//! Lossless CSV views for SSD sheet values.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use crate::error::{ErrorKind, FormatError, Result};
use crate::sheet::{self, ColumnType, ColumnValue};
use crate::InspectAs;

/// One merged sheet table. Definitions place their columns at the indexes
/// declared by the SSD document, so language variants share one row.
#[derive(Debug, Default)]
pub struct CsvTable {
    pub columns: Vec<Option<ColumnType>>,
    pub rows: BTreeMap<u32, Vec<Option<String>>>,
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
            let Some(value) = value else {
                let slot = &mut row[index as usize];
                if let Some(existing) = slot {
                    existing.push_str("[@missing]");
                } else {
                    *slot = Some("[@missing]".into());
                }
                continue;
            };
            let text = value_text(&value);
            let slot = &mut row[index as usize];
            if let Some(existing) = slot {
                if *existing != text {
                    conflicts += 1;
                    existing.push_str("[@duplicate:");
                    for byte in text.as_bytes() {
                        let _ = write!(existing, "{byte:02X}");
                    }
                    existing.push(']');
                }
            } else {
                *slot = Some(text);
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
/// empty cells. This is the bounded anomaly used by the 1.23b Chinese
/// `xtx/quest` blocks.
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
    fn csv_quotes_only_fields_that_need_it() {
        let mut table = CsvTable {
            columns: vec![Some(ColumnType::Unsigned8), None],
            ..CsvTable::default()
        };
        table.rows.insert(7, vec![Some("9".into()), None]);
        assert_eq!(table.render(), " ,0,1\n ,u8,\n7,9,\n");
    }

    #[test]
    fn a_missing_trailing_column_is_an_explicit_empty_value() {
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
        assert_eq!(table.render(), " ,0\n ,u8\n1,7[@duplicate:38]\n");
    }

    #[test]
    fn a_missing_duplicate_stays_distinct_from_an_empty_value() {
        let mut table = CsvTable::default();
        table.insert_columns(&[0], &[ColumnType::Text]).unwrap();
        table.insert_row(1, &[0], vec![None]).unwrap();
        assert_eq!(table.render(), " ,0\n ,str\n1,[@missing]\n");
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
