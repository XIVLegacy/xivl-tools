//! Lossless CSV views for SSD sheet values.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use crate::error::{ErrorKind, FormatError, Result};
use crate::sheet::{self, ColumnType, ColumnValue};
use crate::InspectAs;

/// Read extract's two headers and LF-terminated CSV records without changing cells.
pub fn read_table(
    text: &str,
    columns: &[Option<ColumnType>],
) -> Result<BTreeMap<u32, Vec<String>>> {
    let bad = |offset, message| {
        FormatError::new(ErrorKind::InvalidAttributeValue, offset as u64, message)
    };
    let mut records = Vec::new();
    let mut record = Vec::new();
    let mut position = 0;
    let bytes = text.as_bytes();
    while position < bytes.len() {
        let mut field = Vec::new();
        if bytes[position] == b'"' {
            position += 1;
            loop {
                let Some(&byte) = bytes.get(position) else {
                    return Err(bad(position, "unterminated quoted CSV field"));
                };
                position += 1;
                if byte == b'"' {
                    if bytes.get(position) == Some(&b'"') {
                        field.push(b'"');
                        position += 1;
                    } else {
                        break;
                    }
                } else {
                    field.push(byte);
                }
            }
        } else {
            while let Some(&byte) = bytes.get(position) {
                if matches!(byte, b',' | b'\n') {
                    break;
                }
                if matches!(byte, b'"' | b'\r') {
                    return Err(bad(position, "invalid unquoted CSV field"));
                }
                field.push(byte);
                position += 1;
            }
        }
        record.push(String::from_utf8(field).map_err(|_| bad(position, "invalid CSV UTF-8"))?);
        match bytes.get(position) {
            Some(b',') => position += 1,
            Some(b'\n') => {
                position += 1;
                records.push(std::mem::take(&mut record));
            }
            _ => {
                return Err(bad(
                    position,
                    "CSV records require LF endings and comma separators",
                ))
            }
        }
    }
    if !record.is_empty() {
        return Err(bad(position, "incomplete CSV record"));
    }
    let indexes: Vec<String> = std::iter::once(" ".into())
        .chain((0..columns.len()).map(|i| i.to_string()))
        .collect();
    let types: Vec<String> = std::iter::once(" ".into())
        .chain(
            columns
                .iter()
                .map(|c| c.map_or("", ColumnType::name).into()),
        )
        .collect();
    if records.first() != Some(&indexes) || records.get(1) != Some(&types) {
        return Err(bad(0, "CSV headers disagree with the client sheet columns"));
    }
    let mut rows = BTreeMap::new();
    for record in records.into_iter().skip(2) {
        if record.len() != columns.len() + 1 {
            return Err(bad(0, "CSV row width disagrees with the headers"));
        }
        let id: u32 = record[0]
            .parse()
            .map_err(|_| bad(0, "invalid CSV row id"))?;
        if id.to_string() != record[0] {
            return Err(bad(0, "noncanonical CSV row id"));
        }
        if rows
            .insert(id, record.into_iter().skip(1).collect())
            .is_some()
        {
            return Err(bad(0, "duplicate CSV row id"));
        }
    }
    Ok(rows)
}

/// Parse one ordinary cell, refusing any spelling that would render differently.
/// Missing and duplicate markers need the caller's original logical cell.
pub fn parse_cell(text: &str, column: ColumnType, scrambled: bool) -> Result<ColumnValue> {
    let bad = || {
        FormatError::new(
            ErrorKind::InvalidAttributeValue,
            0,
            format!("invalid or inexact {} cell", column.name()),
        )
    };
    let span = crate::Span::new(0, column.width().unwrap_or(0) as u64);
    let value = match column {
        ColumnType::Text => ColumnValue::Text(Box::new(sheet::SheetString::from_decoded(
            &crate::RichString::from_lossless_text(text)?.encode(),
            scrambled,
        )?)),
        ColumnType::Unsigned8 => ColumnValue::Unsigned8 {
            span,
            value: text.parse().map_err(|_| bad())?,
        },
        ColumnType::Signed8 => ColumnValue::Signed8 {
            span,
            value: text.parse().map_err(|_| bad())?,
        },
        ColumnType::Unsigned16 => ColumnValue::Unsigned16 {
            span,
            value: text.parse().map_err(|_| bad())?,
        },
        ColumnType::Signed16 => ColumnValue::Signed16 {
            span,
            value: text.parse().map_err(|_| bad())?,
        },
        ColumnType::Unsigned32 => ColumnValue::Unsigned32 {
            span,
            value: text.parse().map_err(|_| bad())?,
        },
        ColumnType::Signed32 => ColumnValue::Signed32 {
            span,
            value: text.parse().map_err(|_| bad())?,
        },
        ColumnType::Boolean => {
            let raw = match text {
                "false" => 0,
                "true" => 1,
                _ => u8::from_str_radix(
                    text.strip_prefix("bool:0x")
                        .and_then(|s| s.split_once(':'))
                        .ok_or_else(bad)?
                        .0,
                    16,
                )
                .map_err(|_| bad())?,
            };
            ColumnValue::Boolean {
                span,
                raw,
                value: raw != 0,
            }
        }
        ColumnType::Float32 => {
            let value = if let Some(hex) = text.strip_prefix("nan:0x") {
                let value = f32::from_bits(u32::from_str_radix(hex, 16).map_err(|_| bad())?);
                if !value.is_nan() {
                    return Err(bad());
                }
                value
            } else {
                text.parse().map_err(|_| bad())?
            };
            ColumnValue::Float32 { span, value }
        }
        ColumnType::Half16 => {
            let raw = if let Some(hex) = text.strip_prefix("nan:0x") {
                let raw = u16::from_str_radix(hex, 16).map_err(|_| bad())?;
                if !sheet::half_to_f32(raw).is_nan() {
                    return Err(bad());
                }
                raw
            } else {
                let value: f32 = text.parse().map_err(|_| bad())?;
                if value.is_nan() {
                    return Err(bad());
                }
                let mut low = 0u16;
                let mut high = 0x7C00u16;
                while low < high {
                    let mid = low + (high - low) / 2;
                    if sheet::half_to_f32(mid) < value.abs() {
                        low = mid + 1;
                    } else {
                        high = mid;
                    }
                }
                let raw = low | if value.is_sign_negative() { 0x8000 } else { 0 };
                if sheet::half_to_f32(raw).to_bits() != value.to_bits() {
                    return Err(bad());
                }
                raw
            };
            ColumnValue::Half16 {
                span,
                raw,
                value: sheet::half_to_f32(raw),
            }
        }
    };
    if value_text(&value) != text {
        return Err(bad());
    }
    Ok(value)
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum LogicalValue {
    Missing,
    Present(String),
}

/// One merged sheet table. Definitions place their columns at the indexes
/// declared by the SSD document, so language variants share one row.
#[derive(Debug, Default)]
pub struct CsvTable {
    pub columns: Vec<Option<ColumnType>>,
    pub rows: BTreeMap<u32, Vec<Option<String>>>,
    /// Values seen per cell, kept separately so duplicate markers do not
    /// become part of the next comparison.
    logical_values: BTreeMap<(u32, usize), BTreeSet<LogicalValue>>,
}

impl CsvTable {
    pub fn ensure_width(&mut self, width: usize) {
        if self.columns.len() < width {
            self.columns.resize(width, None);
        }
        for row in self.rows.values_mut() {
            if row.len() < width {
                row.resize(width, None);
            }
        }
    }

    pub fn insert_columns(&mut self, indexes: &[u32], types: &[ColumnType]) -> Result<()> {
        if indexes.len() != types.len() {
            return Err(FormatError::new(
                ErrorKind::InvalidAttributeValue,
                0,
                format!(
                    "a sheet maps {} column type(s) through {} index value(s)",
                    types.len(),
                    indexes.len()
                ),
            ));
        }
        let width = indexes
            .iter()
            .copied()
            .max()
            .map_or(0, |index| index as usize + 1);
        self.ensure_width(width);
        for (&index, &column) in indexes.iter().zip(types) {
            let slot = &mut self.columns[index as usize];
            if let Some(existing) = slot {
                if *existing != column {
                    return Err(FormatError::new(
                        ErrorKind::InvalidAttributeValue,
                        0,
                        format!(
                            "column {index} is declared as both '{}' and '{}'",
                            existing.name(),
                            column.name()
                        ),
                    ));
                }
            } else {
                *slot = Some(column);
            }
        }
        Ok(())
    }

    pub fn insert_row(
        &mut self,
        row_id: u32,
        indexes: &[u32],
        values: Vec<Option<ColumnValue>>,
    ) -> Result<usize> {
        if indexes.len() != values.len() {
            return Err(FormatError::new(
                ErrorKind::InvalidAttributeValue,
                0,
                "a row value count does not match its index list",
            ));
        }
        let row = self
            .rows
            .entry(row_id)
            .or_insert_with(|| vec![None; self.columns.len()]);
        row.resize(self.columns.len(), None);
        let mut conflicts = 0;
        for (&index, value) in indexes.iter().zip(values) {
            let slot = &mut row[index as usize];
            let logical = self
                .logical_values
                .entry((row_id, index as usize))
                .or_default();
            if logical.is_empty() {
                if let Some(existing) = slot.as_ref() {
                    logical.insert(LogicalValue::Present(existing.clone()));
                }
            }
            let Some(value) = value else {
                if !logical.contains(&LogicalValue::Missing) {
                    if let Some(existing) = slot {
                        if existing.is_empty() {
                            // Keep an authored empty value visible when a
                            // missing value shares its cell.
                            existing.push_str("[@duplicate:]");
                        }
                        existing.push_str("[@missing]");
                    } else {
                        *slot = Some("[@missing]".into());
                    }
                    logical.insert(LogicalValue::Missing);
                }
                continue;
            };
            let text = value_text(&value);
            let logical_value = LogicalValue::Present(text.clone());
            if !logical.contains(&logical_value) {
                let has_other_present = logical.iter().any(
                    |entry| matches!(entry, LogicalValue::Present(existing) if existing != &text),
                );
                if has_other_present {
                    conflicts += 1;
                }
                if let Some(existing) = slot {
                    existing.push_str("[@duplicate:");
                    for byte in text.as_bytes() {
                        let _ = write!(existing, "{byte:02X}");
                    }
                    existing.push(']');
                } else {
                    *slot = Some(text.clone());
                }
                logical.insert(logical_value);
            }
        }
        Ok(conflicts)
    }

    pub fn render(&self) -> String {
        let mut output = String::new();
        output.push(' ');
        for index in 0..self.columns.len() {
            let _ = write!(output, ",{index}");
        }
        output.push('\n');

        output.push(' ');
        for column in &self.columns {
            output.push(',');
            if let Some(column) = column {
                output.push_str(column.name());
            }
        }
        output.push('\n');

        for (row_id, values) in &self.rows {
            let _ = write!(output, "{row_id}");
            for value in values {
                output.push(',');
                if let Some(value) = value {
                    push_csv_field(&mut output, value);
                }
            }
            output.push('\n');
        }
        output
    }
}

/// Read exactly one row span. Missing trailing values are represented as
/// `None` for the caller; `CsvTable` renders them as `[@missing]`. This is
/// the bounded anomaly used by the 1.23b Chinese `xtx/quest` blocks.
pub fn parse_row_span(data: &[u8], columns: &[ColumnType]) -> Result<Vec<Option<ColumnValue>>> {
    let shortest = columns.len().saturating_sub(1).max(1);
    for count in (shortest..=columns.len()).rev() {
        if let Ok(rows) = sheet::parse_rows(data, &columns[..count]) {
            if rows.len() == 1 && rows[0].span.length == data.len() as u64 {
                let mut values: Vec<Option<ColumnValue>> = rows
                    .into_iter()
                    .next()
                    .unwrap()
                    .values
                    .into_iter()
                    .map(Some)
                    .collect();
                values.resize_with(columns.len(), || None);
                return Ok(values);
            }
        }
    }
    Err(FormatError::new(
        ErrorKind::UnexpectedEndOfInput,
        data.len() as u64,
        "a row span does not contain a complete prefix of its declared columns",
    ))
}

pub fn value_text(value: &ColumnValue) -> String {
    match value {
        ColumnValue::Text(string) => string.rich.to_lossless_text(),
        ColumnValue::Unsigned8 { value, .. } => value.to_string(),
        ColumnValue::Signed8 { value, .. } => value.to_string(),
        ColumnValue::Boolean { value, raw, .. } => match raw {
            0 => "false".into(),
            1 => "true".into(),
            other => format!(
                "bool:0x{other:02X}:{}",
                if *value { "true" } else { "false" }
            ),
        },
        ColumnValue::Unsigned16 { value, .. } => value.to_string(),
        ColumnValue::Signed16 { value, .. } => value.to_string(),
        ColumnValue::Half16 { raw, value, .. } => float_text(*value, u32::from(*raw), 4),
        ColumnValue::Unsigned32 { value, .. } => value.to_string(),
        ColumnValue::Signed32 { value, .. } => value.to_string(),
        ColumnValue::Float32 { value, .. } => float_text(*value, value.to_bits(), 8),
    }
}

fn float_text(value: f32, raw: u32, hex_width: usize) -> String {
    if value.is_nan() {
        format!("nan:0x{raw:0hex_width$X}")
    } else if value == f32::INFINITY {
        "inf".into()
    } else if value == f32::NEG_INFINITY {
        "-inf".into()
    } else {
        value.to_string()
    }
}

fn push_csv_field(output: &mut String, value: &str) {
    if value.contains([',', '"', '\n', '\r']) {
        output.push('"');
        for character in value.chars() {
            if character == '"' {
                output.push('"');
            }
            output.push(character);
        }
        output.push('"');
    } else {
        output.push_str(value);
    }
}

/// Export one standalone sheet-data resource through the same typed view
/// accepted by `inspect --as sheet-data`.
pub fn export_sheet_data(data: &[u8], how: &InspectAs) -> Result<serde_json::Value> {
    let InspectAs::SheetData(selected) = how else {
        return Err(FormatError::new(
            ErrorKind::InvalidAttributeValue,
            0,
            "CSV export currently accepts only --as sheet-data",
        ));
    };
    let columns = if selected.is_empty() {
        vec![ColumnType::Text]
    } else {
        selected.clone()
    };
    let rows = sheet::parse_rows(data, &columns)?;
    let indexes: Vec<u32> = (0..columns.len() as u32).collect();
    let mut table = CsvTable::default();
    table.insert_columns(&indexes, &columns)?;
    for (index, row) in rows.into_iter().enumerate() {
        table.insert_row(
            index as u32,
            &indexes,
            row.values.into_iter().map(Some).collect(),
        )?;
    }
    Ok(serde_json::json!({
        "schemaVersion": crate::inspect::DOCUMENT_SCHEMA_VERSION,
        "operation": "extract",
        "format": "ssd-sheet",
        "input": { "length": data.len() as u64 },
        "columns": columns.iter().map(|column| column.name()).collect::<Vec<_>>(),
        "rowCount": table.rows.len() as u64,
        "csv": table.render(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_reader_preserves_quoted_multiline_fields_and_headers() {
        let mut table = CsvTable {
            columns: vec![Some(ColumnType::Text)],
            ..CsvTable::default()
        };
        table.rows.insert(
            3,
            vec![Some("comma, quote\" LF\nCR\r backslash\\ bracket[".into())],
        );
        let rows = read_table(&table.render(), &table.columns).unwrap();
        assert_eq!(rows[&3][0], "comma, quote\" LF\nCR\r backslash\\ bracket[");
        for text in [
            " ,0\r\n ,str\r\n",
            " ,1\n ,str\n",
            " ,0\n ,u8\n",
            " ,0\n ,str\n3,\"open\n",
            " ,0\n ,str\n3,a\"b\n",
            " ,0\n ,str\n3,\"a\"b\n",
            " ,0\n ,str\n3,x",
            " ,0\n ,str\n3,x,\n",
            " ,0\n ,str\n3,x\n3,y\n",
            " ,0\n ,str\n03,x\n",
        ] {
            assert!(read_table(text, &table.columns).is_err(), "{text:?}");
        }
    }

    #[test]
    fn cells_preserve_all_half_patterns_and_reject_rounding() {
        for raw in 0..=u16::MAX {
            let value = ColumnValue::Half16 {
                span: crate::Span::new(0, 2),
                raw,
                value: sheet::half_to_f32(raw),
            };
            let text = value_text(&value);
            let parsed = parse_cell(&text, ColumnType::Half16, false).unwrap();
            let mut bytes = Vec::new();
            sheet::encode_column_value(&parsed, &mut bytes).unwrap();
            assert_eq!(bytes, raw.to_le_bytes(), "{text}");
        }
        for (kind, text) in [
            (ColumnType::Half16, "0.1"),
            (ColumnType::Half16, "1.00001"),
            (ColumnType::Float32, "1.00000001"),
            (ColumnType::Unsigned8, "01"),
            (ColumnType::Unsigned8, "256"),
            (ColumnType::Boolean, "bool:0x02:false"),
            (ColumnType::Boolean, "1"),
            (ColumnType::Text, "[@missing]"),
            (ColumnType::Text, "[@duplicate:61]"),
            (ColumnType::Float32, "nan:0x00000000"),
        ] {
            assert!(parse_cell(text, kind, false).is_err(), "{text}");
        }
    }

    #[test]
    fn cells_preserve_numeric_bits_and_rich_tokens() {
        for raw in [
            0,
            1,
            0x8000_0000,
            0x7F80_0000,
            0xFF80_0000,
            0x7FC0_0001,
            0xFFA0_0002,
            0x3F80_0000,
            0x0080_0000,
        ] {
            let text = value_text(&ColumnValue::Float32 {
                span: crate::Span::new(0, 4),
                value: f32::from_bits(raw),
            });
            let mut bytes = Vec::new();
            sheet::encode_column_value(
                &parse_cell(&text, ColumnType::Float32, false).unwrap(),
                &mut bytes,
            )
            .unwrap();
            assert_eq!(bytes, raw.to_le_bytes());
        }
        for raw in 0..=u8::MAX {
            let text = value_text(&ColumnValue::Boolean {
                span: crate::Span::new(0, 1),
                value: raw != 0,
                raw,
            });
            assert_eq!(
                value_text(&parse_cell(&text, ColumnType::Boolean, false).unwrap()),
                text
            );
        }
        for (kind, text) in [
            (ColumnType::Signed8, "-128"),
            (ColumnType::Unsigned16, "65535"),
            (ColumnType::Signed16, "-32768"),
            (ColumnType::Unsigned32, "4294967295"),
            (ColumnType::Signed32, "-2147483648"),
            (ColumnType::Text, "\\[literal]\\\\[@newline:02100103]"),
        ] {
            assert_eq!(value_text(&parse_cell(text, kind, true).unwrap()), text);
        }
    }

    #[test]
    fn csv_quotes_only_fields_that_need_it() {
        let mut table = CsvTable {
            columns: vec![Some(ColumnType::Unsigned8), None],
            ..CsvTable::default()
        };
        table.rows.insert(7, vec![Some("9".into()), None]);
        assert_eq!(table.render(), " ,0,1\n ,u8,\n7,9,\n");
    }

    #[test]
    fn an_omitted_trailing_column_is_an_explicit_missing_value() {
        let data = [1u8, 2u8];
        let columns = [
            ColumnType::Unsigned8,
            ColumnType::Unsigned8,
            ColumnType::Text,
        ];
        let values = parse_row_span(&data, &columns).unwrap();
        assert_eq!(values.len(), 3);
        assert!(values[2].is_none());

        let error = parse_row_span(&[1u8], &columns).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::UnexpectedEndOfInput);
    }

    #[test]
    fn a_conflicting_duplicate_keeps_both_values() {
        let mut table = CsvTable::default();
        table
            .insert_columns(&[0], &[ColumnType::Unsigned8])
            .unwrap();
        let value = |number| {
            vec![Some(ColumnValue::Unsigned8 {
                span: crate::Span::new(0, 1),
                value: number,
            })]
        };
        assert_eq!(table.insert_row(1, &[0], value(7)).unwrap(), 0);
        assert_eq!(table.insert_row(1, &[0], value(8)).unwrap(), 1);
        assert_eq!(table.insert_row(1, &[0], value(7)).unwrap(), 0);
        assert_eq!(table.insert_row(1, &[0], value(8)).unwrap(), 0);
        assert_eq!(table.render(), " ,0\n ,u8\n1,7[@duplicate:38]\n");
    }

    #[test]
    fn a_missing_duplicate_stays_distinct_from_an_empty_value() {
        let mut table = CsvTable::default();
        table.insert_columns(&[0], &[ColumnType::Text]).unwrap();
        table.insert_row(1, &[0], vec![None]).unwrap();
        table.insert_row(1, &[0], vec![None]).unwrap();
        assert_eq!(table.render(), " ,0\n ,str\n1,[@missing]\n");
    }

    #[test]
    fn missing_and_empty_render_as_distinct_lossless_states() {
        let empty = || parse_row_span(&[1, 0, 0], &[ColumnType::Text]).unwrap();

        let mut value_then_missing = CsvTable::default();
        value_then_missing
            .insert_columns(&[0], &[ColumnType::Unsigned8])
            .unwrap();
        let value = || {
            vec![Some(ColumnValue::Unsigned8 {
                span: crate::Span::new(0, 1),
                value: 7,
            })]
        };
        assert_eq!(value_then_missing.insert_row(1, &[0], value()).unwrap(), 0);
        assert_eq!(
            value_then_missing.insert_row(1, &[0], vec![None]).unwrap(),
            0
        );
        assert_eq!(
            value_then_missing.insert_row(1, &[0], vec![None]).unwrap(),
            0
        );
        assert_eq!(value_then_missing.render(), " ,0\n ,u8\n1,7[@missing]\n");

        let mut empty_then_missing = CsvTable::default();
        empty_then_missing
            .insert_columns(&[0], &[ColumnType::Text])
            .unwrap();
        assert_eq!(empty_then_missing.insert_row(1, &[0], empty()).unwrap(), 0);
        assert_eq!(
            empty_then_missing.insert_row(1, &[0], vec![None]).unwrap(),
            0
        );
        assert_eq!(
            empty_then_missing.insert_row(1, &[0], vec![None]).unwrap(),
            0
        );
        assert_eq!(
            empty_then_missing.render(),
            " ,0\n ,str\n1,[@duplicate:][@missing]\n"
        );

        let mut missing_then_empty = CsvTable::default();
        missing_then_empty
            .insert_columns(&[0], &[ColumnType::Text])
            .unwrap();
        assert_eq!(
            missing_then_empty.insert_row(1, &[0], vec![None]).unwrap(),
            0
        );
        assert_eq!(missing_then_empty.insert_row(1, &[0], empty()).unwrap(), 0);
        assert_eq!(missing_then_empty.insert_row(1, &[0], empty()).unwrap(), 0);
        assert_eq!(
            missing_then_empty.render(),
            " ,0\n ,str\n1,[@missing][@duplicate:]\n"
        );

        let mut missing_only = CsvTable::default();
        missing_only
            .insert_columns(&[0], &[ColumnType::Text])
            .unwrap();
        assert_eq!(missing_only.insert_row(1, &[0], vec![None]).unwrap(), 0);
        assert_eq!(missing_only.insert_row(1, &[0], vec![None]).unwrap(), 0);
        assert_eq!(missing_only.render(), " ,0\n ,str\n1,[@missing]\n");
    }

    #[test]
    fn non_finite_floats_keep_distinguishing_bits() {
        let value = ColumnValue::Half16 {
            span: crate::Span::new(0, 2),
            raw: 0x7E01,
            value: f32::NAN,
        };
        assert_eq!(value_text(&value), "nan:0x7E01");
    }
}
