//! Conditional fixedPhrase source-cell lookup, with caller-supplied family and locale.

use std::collections::BTreeSet;

use csv::StringRecord;
use serde_json::{json, Value};
use xivl_formats::digest::sha256_hex;
use xivl_formats::error::{ErrorKind, FormatError, Result};

#[derive(Clone, Copy)]
pub enum Locale {
    Ja,
    En,
    De,
    Fr,
}

impl Locale {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "ja" => Some(Self::Ja),
            "en" => Some(Self::En),
            "de" => Some(Self::De),
            "fr" => Some(Self::Fr),
            _ => None,
        }
    }

    pub(super) fn columns(self) -> (&'static str, [usize; 2]) {
        match self {
            Self::Ja => ("ja", [2, 3]),
            Self::En => ("en", [4, 5]),
            Self::De => ("de", [6, 7]),
            Self::Fr => ("fr", [8, 9]),
        }
    }
}

pub fn report(data: &[u8], numbers: [u32; 2], locale: Locale) -> (Value, bool) {
    let (locale_name, columns) = locale.columns();
    let [category, key] = numbers;
    let mut document = json!({
        "family": "fixedPhrase",
        "familySource": "explicit-option",
        "association": "conditional-ordinary-producer",
        "locale": locale_name,
        "tableSha256": sha256_hex(data),
        "rowKey": key,
        "category": category,
        "textEncoding": "ssd-lossless-csv",
    });
    match selected_row(data, key) {
        Err(error) => {
            document["status"] = json!("invalid-table");
            document["errorKind"] = json!(error.kind().as_str());
            document["errorOffset"] = json!(error.offset());
            document["detail"] = json!(error.detail());
        }
        Ok(None) => document["status"] = json!("not-found"),
        Ok(Some(row)) => {
            let field0 = row[1].parse::<u32>().expect("validated field 0");
            document["field0"] = json!(field0);
            if category >= 64 || !(1..=64).contains(&field0) {
                document["status"] = json!("unsupported-category");
            } else if field0 - 1 != category {
                document["status"] = json!("category-mismatch");
            } else if !row[2].is_empty() {
                // Descriptor rows prepare other categories or target tables;
                // their own key does not identify an ordinary phrase.
                document["status"] = json!("unsupported-descriptor");
            } else {
                document["status"] = json!("matched");
                document["fields"] = json!(columns.map(|column| json!({
                    "column": column,
                    "value": &row[column + 1],
                })));
                return (document, false);
            }
        }
    }
    (document, true)
}

fn invalid(offset: u64, detail: impl Into<String>) -> FormatError {
    FormatError::new(ErrorKind::InvalidAttributeValue, offset, detail)
}

fn selected_row(data: &[u8], key: u32) -> Result<Option<StringRecord>> {
    validate_quotes(data)?;
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .from_reader(data);
    let mut record = StringRecord::new();
    let mut read = |record: &mut StringRecord| -> Result<bool> {
        reader.read_record(record).map_err(|error| {
            invalid(
                error.position().map_or(0, |position| position.byte()),
                format!("invalid fixedPhrase CSV: {error}"),
            )
        })
    };
    if !read(&mut record)?
        || record.len() != 13
        || !record[0].trim().is_empty()
        || record.iter().skip(1).ne((0..12).map(|n| n.to_string()))
    {
        return Err(invalid(0, "expected fixedPhrase columns 0 through 11"));
    }
    if !read(&mut record)?
        || !record[0].trim().is_empty()
        || &record[1] != "u32"
        || !record.iter().skip(2).all(|value| value == "str")
    {
        return Err(invalid(
            record.position().map_or(data.len() as u64, |p| p.byte()),
            "expected fixedPhrase types u32 followed by eleven str fields",
        ));
    }
    let mut keys = BTreeSet::new();
    let mut selected = None;
    while read(&mut record)? {
        let offset = record.position().map_or(0, |p| p.byte());
        let row_key = record[0]
            .parse::<u32>()
            .map_err(|_| invalid(offset, "fixedPhrase row key must be unsigned decimal"))?;
        if !(100..3026).contains(&row_key) {
            return Err(invalid(
                offset,
                "fixedPhrase row key is outside block (100,2926)",
            ));
        }
        if !keys.insert(row_key) {
            return Err(invalid(offset, "duplicate fixedPhrase row key"));
        }
        record[1]
            .parse::<u32>()
            .map_err(|_| invalid(offset, "fixedPhrase field 0 must be unsigned decimal"))?;
        if row_key == key {
            selected = Some(record.clone());
        }
    }
    Ok(selected)
}

fn validate_quotes(data: &[u8]) -> Result<()> {
    #[derive(Clone, Copy)]
    enum State {
        Start,
        Unquoted,
        Quoted,
        Closed,
    }
    use State::*;
    let mut state = Start;
    for (offset, &byte) in data.iter().enumerate() {
        state = match (state, byte) {
            (Start, b'"') | (Closed, b'"') => Quoted,
            (Quoted, b'"') => Closed,
            (Quoted, _) => Quoted,
            (_, b',' | b'\r' | b'\n') => Start,
            (Start | Unquoted, byte) if byte != b'"' => Unquoted,
            _ => return Err(invalid(offset as u64, "invalid fixedPhrase CSV quoting")),
        };
    }
    if matches!(state, Quoted) {
        return Err(invalid(
            data.len() as u64,
            "unterminated fixedPhrase CSV quoted field",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEADER: &str =
        " ,0,1,2,3,4,5,6,7,8,9,10,11\n ,u32,str,str,str,str,str,str,str,str,str,str,str\n";

    fn table(rows: &str) -> Vec<u8> {
        format!("{HEADER}{rows}").into_bytes()
    }

    #[test]
    fn matches_key_and_category_and_preserves_separate_locale_cells() {
        let data = table(
            "101,3,,Ja0,Ja1,\"Authored, \"\"word\"\"\",,De0,De1,Fr0,Fr1,,\n102,2,,Wrong,,,,,,,,,\n",
        );
        for (locale, columns, expected) in [
            (Locale::Ja, [2, 3], ["Ja0", "Ja1"]),
            (Locale::En, [4, 5], ["Authored, \"word\"", ""]),
            (Locale::De, [6, 7], ["De0", "De1"]),
            (Locale::Fr, [8, 9], ["Fr0", "Fr1"]),
        ] {
            let (result, failed) = report(&data, [2, 101], locale);
            assert!(!failed, "{result}");
            assert_eq!(result["status"], "matched");
            for i in 0..2 {
                assert_eq!(result["fields"][i]["column"], columns[i]);
                assert_eq!(result["fields"][i]["value"], expected[i]);
            }
        }
    }

    #[test]
    fn missing_mismatched_and_descriptor_rows_do_not_resolve() {
        let data = table("101,3,,First,,Second,,,,,,,\n102,3,xtx/other,First,,Second,,,,,,,\n103,65,,First,,Second,,,,,,,\n");
        for (numbers, expected) in [
            ([2, 104], "not-found"),
            ([1, 101], "category-mismatch"),
            ([2, 102], "unsupported-descriptor"),
            ([64, 103], "unsupported-category"),
            ([u32::MAX, 101], "unsupported-category"),
        ] {
            let (result, failed) = report(&data, numbers, Locale::En);
            assert!(failed);
            assert_eq!(result["status"], expected);
            assert!(result.get("fields").is_none());
        }
    }

    #[test]
    fn validates_the_whole_table_before_returning_a_match() {
        let row = "101,3,,Ja0,Ja1,En0,En1,De0,De1,Fr0,Fr1,,\n";
        for suffix in [
            row,
            "102,invalid,,,,,,,,,,,\n",
            "short,row\n",
            "99,3,,,,,,,,,,,\n",
        ] {
            let data = table(&format!("{row}{suffix}"));
            let error = selected_row(&data, 101).unwrap_err();
            assert_eq!(error.offset(), (HEADER.len() + row.len()) as u64);
        }
        for data in [b"".as_slice(), b"wrong,header\n", b"101,3,wrong\n"] {
            assert!(selected_row(data, 101).is_err());
        }
    }

    #[test]
    fn rejects_unclosed_or_misplaced_quotes_and_keeps_quoted_newlines() {
        let row = "101,3,,Ja0,Ja1,En0,En1,De0,De1,Fr0,Fr1,,";
        let data = table(&format!("{row}\"unterminated"));
        let error = selected_row(&data, 101).unwrap_err();
        assert_eq!(error.offset(), data.len() as u64);
        for suffix in ["unquoted\"", "\"closed\"junk"] {
            assert!(selected_row(&table(&format!("{row}{suffix}")), 101).is_err());
        }
        let data = table("101,3,,Ja0,Ja1,\"First\nSecond\",,De0,De1,Fr0,Fr1,,\n");
        let (result, failed) = report(&data, [2, 101], Locale::En);
        assert!(!failed);
        assert_eq!(result["fields"][0]["value"], "First\nSecond");
    }
}
