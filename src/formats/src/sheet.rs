//! The sheet data stack: enable ranges, row offsets, strings, and rows.
//!
//! A sheet block is four resources. The schema in the `<sheet>` document
//! names the other three: a data file holding the rows back to back, a
//! row-offset array with one entry per row slot, and an enable file listing
//! the row identifiers that carry data. Byte-layout evidence and its retail
//! citation: `docs/formats/ssd-sheet.md`, "The SSD document stack".
//!
//! Nothing in this module reaches for a second file. A caller that has an
//! install root supplies the bytes. This crate never resolves one.

use crate::anomaly::Anomaly;
use crate::digest::sha256_hex;
use crate::error::{ErrorKind, FormatError, Result};
use crate::reader::{Reader, Span};
use crate::richstring::RichString;

/// Marker byte that opens an obfuscated string body.
pub const SCRAMBLE_MARKER: u8 = 0xFF;

/// The substitution key. A scrambled body is the plain body XOR this,
/// which is why the terminating NUL reads as 0x73.
pub const SCRAMBLE_KEY: u8 = 0x73;

/// Bytes per enable-file record.
pub const ENABLE_RECORD_SIZE: usize = 8;

/// Bytes per row-offset entry.
pub const ROW_OFFSET_SIZE: usize = 4;

/// One contiguous run of enabled row identifiers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnableRange {
    pub span: Span,
    pub first_row: u32,
    pub count: u32,
}

impl EnableRange {
    /// One past the last row this range names, widened so a range ending
    /// at the top of the 32-bit space cannot wrap.
    pub fn end(&self) -> u64 {
        u64::from(self.first_row) + u64::from(self.count)
    }
}

/// A parsed enable file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnableFile {
    pub ranges: Vec<EnableRange>,
    /// Structural oddities. None of them stops the parse. A range the
    /// client wrote is a range this crate reports.
    pub anomalies: Vec<Anomaly>,
}

impl EnableFile {
    /// Total row identifiers named, counting a repeat twice. Overlaps are
    /// reported as anomalies rather than folded away here.
    pub fn row_count(&self) -> u64 {
        self.ranges.iter().map(|range| u64::from(range.count)).sum()
    }
}

/// Parse an enable file: pairs of first row identifier and run length.
pub fn parse_enable_file(data: &[u8]) -> Result<EnableFile> {
    let remainder = data.len() % ENABLE_RECORD_SIZE;
    if remainder != 0 {
        return Err(FormatError::new(
            ErrorKind::TrailingPartialRecord,
            (data.len() - remainder) as u64,
            format!(
                "an enable file is {ENABLE_RECORD_SIZE}-byte records; {remainder} byte(s) are left over"
            ),
        ));
    }

    let mut reader = Reader::new(data);
    let mut ranges = Vec::new();
    let mut anomalies = Vec::new();
    while reader.remaining() > 0 {
        let offset = reader.offset();
        let first_row = reader.u32_le()?;
        let count = reader.u32_le()?;
        let range = EnableRange {
            span: Span::new(offset, ENABLE_RECORD_SIZE as u64),
            first_row,
            count,
        };
        if count == 0 {
            anomalies.push(Anomaly {
                kind: "empty-enable-range",
                span: range.span,
                detail: format!("the range at row {first_row} names no rows"),
            });
        }
        if let Some(previous) = ranges.last() {
            let previous: &EnableRange = previous;
            if u64::from(first_row) < previous.end() {
                anomalies.push(Anomaly {
                    kind: "enable-range-out-of-order",
                    span: range.span,
                    detail: format!(
                        "the range at row {first_row} starts before the previous range ends at {}",
                        previous.end()
                    ),
                });
            }
        }
        ranges.push(range);
    }
    Ok(EnableFile { ranges, anomalies })
}

/// One row slot that carries data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowSlot {
    /// Index of the slot in the array, which is the row identifier minus
    /// the block's `begin`.
    pub index: u64,
    /// Where the row sits in the data file.
    pub span: Span,
}

/// A parsed row-offset array.
///
/// Entry `i` is the end offset of row `i` in the data file, so row `i`
/// spans `offsets[i - 1] .. offsets[i]` and row 0 starts at zero. An empty
/// row repeats the previous value, which is how a sparse sheet declares
/// thousands of slots and stores dozens of rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowOffsets {
    pub offsets: Vec<u32>,
    /// Only the slots with a non-empty span, which is the whole content of
    /// the array: every other entry repeats its predecessor.
    pub rows: Vec<RowSlot>,
    pub anomalies: Vec<Anomaly>,
}

impl RowOffsets {
    /// Number of slots the array declares, empty ones included.
    pub fn slot_count(&self) -> usize {
        self.offsets.len()
    }

    /// The data-file length the array implies.
    pub fn data_length(&self) -> u64 {
        self.offsets.last().copied().map_or(0, u64::from)
    }
}

/// Parse a row-offset array.
pub fn parse_row_offsets(data: &[u8]) -> Result<RowOffsets> {
    let remainder = data.len() % ROW_OFFSET_SIZE;
    if remainder != 0 {
        return Err(FormatError::new(
            ErrorKind::TrailingPartialRecord,
            (data.len() - remainder) as u64,
            format!(
                "a row-offset array is {ROW_OFFSET_SIZE}-byte entries; {remainder} byte(s) are left over"
            ),
        ));
    }

    let mut reader = Reader::new(data);
    let mut offsets = Vec::with_capacity(data.len() / ROW_OFFSET_SIZE);
    let mut rows = Vec::new();
    let mut anomalies = Vec::new();
    let mut previous: u32 = 0;
    let mut index: u64 = 0;
    while reader.remaining() > 0 {
        let entry_offset = reader.offset();
        let value = reader.u32_le()?;
        if value < previous {
            // A shorter value cannot be an end offset. The entry is kept
            // verbatim and the slot is reported as empty rather than
            // producing a negative extent.
            anomalies.push(Anomaly {
                kind: "row-offset-out-of-order",
                span: Span::new(entry_offset, ROW_OFFSET_SIZE as u64),
                detail: format!(
                    "slot {index} ends at {value}, before the previous slot ended at {previous}"
                ),
            });
        } else if value > previous {
            rows.push(RowSlot {
                index,
                span: Span::new(u64::from(previous), u64::from(value - previous)),
            });
            previous = value;
        }
        offsets.push(value);
        index += 1;
    }
    Ok(RowOffsets {
        offsets,
        rows,
        anomalies,
    })
}

/// A column type from a sheet schema.
///
/// Only the ten types the 1.23b documents actually declare are modeled. A
/// width this crate has not read out of retail data is a width it will not
/// invent, so any other type name is [`ErrorKind::UnknownColumnType`].
/// `f16` is IEEE-754 binary16 stored little-endian.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnType {
    /// A framed, possibly obfuscated string.
    Text,
    Unsigned8,
    Signed8,
    Boolean,
    Unsigned16,
    Signed16,
    Half16,
    Unsigned32,
    Signed32,
    Float32,
}

impl ColumnType {
    pub fn parse(name: &str, offset: u64) -> Result<Self> {
        match name {
            "str" => Ok(ColumnType::Text),
            "u8" => Ok(ColumnType::Unsigned8),
            "s8" => Ok(ColumnType::Signed8),
            "bool" => Ok(ColumnType::Boolean),
            "u16" => Ok(ColumnType::Unsigned16),
            "s16" => Ok(ColumnType::Signed16),
            "f16" => Ok(ColumnType::Half16),
            "u32" => Ok(ColumnType::Unsigned32),
            "s32" => Ok(ColumnType::Signed32),
            "float" => Ok(ColumnType::Float32),
            other => Err(FormatError::new(
                ErrorKind::UnknownColumnType,
                offset,
                format!("column type '{other}' has no width established against retail data"),
            )),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            ColumnType::Text => "str",
            ColumnType::Unsigned8 => "u8",
            ColumnType::Signed8 => "s8",
            ColumnType::Boolean => "bool",
            ColumnType::Unsigned16 => "u16",
            ColumnType::Signed16 => "s16",
            ColumnType::Half16 => "f16",
            ColumnType::Unsigned32 => "u32",
            ColumnType::Signed32 => "s32",
            ColumnType::Float32 => "float",
        }
    }

    /// Fixed width in bytes, or `None` for the self-delimiting string.
    pub fn width(self) -> Option<usize> {
        match self {
            ColumnType::Text => None,
            ColumnType::Unsigned8 | ColumnType::Signed8 | ColumnType::Boolean => Some(1),
            ColumnType::Unsigned16 | ColumnType::Signed16 | ColumnType::Half16 => Some(2),
            ColumnType::Unsigned32 | ColumnType::Signed32 | ColumnType::Float32 => Some(4),
        }
    }
}

/// Parse a comma-separated column list, as a command line supplies it.
pub fn parse_column_list(text: &str) -> Result<Vec<ColumnType>> {
    text.split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(|part| ColumnType::parse(part, 0))
        .collect()
}

/// One string value: the framing, the cipher state, and the token IR.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SheetString {
    /// The whole value: length prefix through terminator.
    pub span: Span,
    /// The length the prefix declares, terminator included.
    pub declared_length: u16,
    pub scrambled: bool,
    /// Decoded text and tokens. Losslessly re-encodable.
    pub rich: RichString,
    /// SHA-256 of the decoded bytes. Reports carry this instead of the
    /// text, which keeps a private-fixture expectation free of client
    /// content.
    pub sha256: String,
}

impl SheetString {
    /// Length of the decoded bytes, terminator excluded.
    pub fn decoded_length(&self) -> u64 {
        u64::from(self.declared_length) - 1 - u64::from(self.scrambled)
    }
}

/// Parse the string value beginning at `position` in `data`.
///
/// A body whose first byte is [`SCRAMBLE_MARKER`] is obfuscated: the
/// remainder is XOR [`SCRAMBLE_KEY`], so its terminator reads as the key
/// itself. Any other first byte means a plain body ending in NUL. The
/// marker is unambiguous because 0xFF is never a valid UTF-8 lead byte.
pub fn parse_sheet_string(data: &[u8], base: u64, position: usize) -> Result<SheetString> {
    let mut reader = Reader::with_base(data, base);
    reader.seek(position)?;
    let start = reader.offset();
    let declared_length = reader.u16_le()?;
    if declared_length == 0 {
        return Err(FormatError::new(
            ErrorKind::MalformedSheetString,
            start,
            "a sheet string declares zero bytes, too short for a terminator",
        ));
    }
    let body = reader.take(usize::from(declared_length))?;

    let scrambled = body[0] == SCRAMBLE_MARKER;
    let terminator = if scrambled { SCRAMBLE_KEY } else { 0x00 };
    let last = body.len() - 1;
    if body[last] != terminator {
        return Err(FormatError::new(
            ErrorKind::MalformedSheetString,
            start + 2 + last as u64,
            format!(
                "a {} sheet string ends with 0x{:02x}, not 0x{terminator:02x}",
                if scrambled { "scrambled" } else { "plain" },
                body[last]
            ),
        ));
    }

    let decoded: Vec<u8> = if scrambled {
        body[1..last]
            .iter()
            .map(|byte| byte ^ SCRAMBLE_KEY)
            .collect()
    } else {
        body[..last].to_vec()
    };
    let text_base = start + 2 + u64::from(scrambled);
    let rich = RichString::parse(&decoded, text_base)?;

    Ok(SheetString {
        span: Span::new(start, 2 + u64::from(declared_length)),
        declared_length,
        scrambled,
        sha256: sha256_hex(&decoded),
        rich,
    })
}

/// Parse a whole resource as a stream of string values.
///
/// This is the shape of every sheet whose columns are all strings, which is
/// how the text sheets are stored. The stream must tile the input exactly.
/// a leftover byte is a failure, not a truncation.
pub fn parse_string_stream(data: &[u8]) -> Result<Vec<SheetString>> {
    let mut strings = Vec::new();
    let mut position = 0usize;
    while position < data.len() {
        let value = parse_sheet_string(data, 0, position)?;
        position += value.span.length as usize;
        strings.push(value);
    }
    Ok(strings)
}

/// Does this input tile exactly as a stream of string values?
///
/// Used for reporting, never for a support claim: an enable file and a
/// row-offset array carry no signature, so the caller names the format.
pub fn looks_like_string_stream(data: &[u8]) -> bool {
    !data.is_empty() && parse_string_stream(data).is_ok()
}

/// One column value inside a row.
#[derive(Debug, Clone, PartialEq)]
pub enum ColumnValue {
    Text(Box<SheetString>),
    Unsigned8 { span: Span, value: u8 },
    Signed8 { span: Span, value: i8 },
    Boolean { span: Span, value: bool, raw: u8 },
    Unsigned16 { span: Span, value: u16 },
    Signed16 { span: Span, value: i16 },
    Half16 { span: Span, raw: u16, value: f32 },
    Unsigned32 { span: Span, value: u32 },
    Signed32 { span: Span, value: i32 },
    Float32 { span: Span, value: f32 },
}

impl ColumnValue {
    pub fn span(&self) -> Span {
        match self {
            ColumnValue::Text(string) => string.span,
            ColumnValue::Unsigned8 { span, .. }
            | ColumnValue::Signed8 { span, .. }
            | ColumnValue::Boolean { span, .. }
            | ColumnValue::Unsigned16 { span, .. }
            | ColumnValue::Signed16 { span, .. }
            | ColumnValue::Half16 { span, .. }
            | ColumnValue::Unsigned32 { span, .. }
            | ColumnValue::Signed32 { span, .. }
            | ColumnValue::Float32 { span, .. } => *span,
        }
    }

    pub fn type_name(&self) -> &'static str {
        match self {
            ColumnValue::Text(_) => "str",
            ColumnValue::Unsigned8 { .. } => "u8",
            ColumnValue::Signed8 { .. } => "s8",
            ColumnValue::Boolean { .. } => "bool",
            ColumnValue::Unsigned16 { .. } => "u16",
            ColumnValue::Signed16 { .. } => "s16",
            ColumnValue::Half16 { .. } => "f16",
            ColumnValue::Unsigned32 { .. } => "u32",
            ColumnValue::Signed32 { .. } => "s32",
            ColumnValue::Float32 { .. } => "float",
        }
    }
}

/// One row of a sheet data file.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub index: u64,
    pub span: Span,
    pub values: Vec<ColumnValue>,
}

/// Parse a data file as rows of the given columns.
///
/// Row boundaries do not need the row-offset array: a string column is
/// self-delimiting and every other column is fixed width, so the columns
/// alone determine where a row ends. Over the install this reproduces the
/// row-offset array exactly for all 37 blocks, which is what makes reading
/// a data file on its own sound.
pub fn parse_rows(data: &[u8], columns: &[ColumnType]) -> Result<Vec<Row>> {
    if columns.is_empty() {
        return Err(FormatError::new(
            ErrorKind::UnknownColumnType,
            0,
            "a row needs at least one column",
        ));
    }
    let mut rows = Vec::new();
    let mut position = 0usize;
    let mut index: u64 = 0;
    while position < data.len() {
        let start = position;
        let mut values = Vec::with_capacity(columns.len());
        for column in columns {
            let value = parse_column(data, position, *column)?;
            position += value.span().length as usize;
            values.push(value);
        }
        rows.push(Row {
            index,
            span: Span::new(start as u64, (position - start) as u64),
            values,
        });
        index += 1;
    }
    Ok(rows)
}

fn parse_column(data: &[u8], position: usize, column: ColumnType) -> Result<ColumnValue> {
    if column == ColumnType::Text {
        return Ok(ColumnValue::Text(Box::new(parse_sheet_string(
            data, 0, position,
        )?)));
    }
    let mut reader = Reader::new(data);
    reader.seek(position)?;
    let offset = reader.offset();
    let width = column.width().unwrap_or(0) as u64;
    let span = Span::new(offset, width);
    Ok(match column {
        ColumnType::Unsigned8 => ColumnValue::Unsigned8 {
            span,
            value: reader.u8()?,
        },
        ColumnType::Signed8 => ColumnValue::Signed8 {
            span,
            value: reader.u8()? as i8,
        },
        ColumnType::Boolean => {
            let raw = reader.u8()?;
            ColumnValue::Boolean {
                span,
                value: raw != 0,
                raw,
            }
        }
        ColumnType::Unsigned16 => ColumnValue::Unsigned16 {
            span,
            value: reader.u16_le()?,
        },
        ColumnType::Signed16 => ColumnValue::Signed16 {
            span,
            value: reader.u16_le()? as i16,
        },
        ColumnType::Half16 => {
            let raw = reader.u16_le()?;
            ColumnValue::Half16 {
                span,
                raw,
                value: half_to_f32(raw),
            }
        }
        ColumnType::Unsigned32 => ColumnValue::Unsigned32 {
            span,
            value: reader.u32_le()?,
        },
        ColumnType::Signed32 => ColumnValue::Signed32 {
            span,
            value: reader.u32_le()? as i32,
        },
        ColumnType::Float32 => ColumnValue::Float32 {
            span,
            value: f32::from_bits(reader.u32_le()?),
        },
        ColumnType::Text => unreachable!("handled above"),
    })
}

/// Convert an IEEE-754 binary16 bit pattern to its exact binary32 value.
pub fn half_to_f32(raw: u16) -> f32 {
    let sign = u32::from(raw & 0x8000) << 16;
    let exponent = u32::from((raw >> 10) & 0x1F);
    let fraction = u32::from(raw & 0x03FF);
    let bits = match exponent {
        0 if fraction == 0 => sign,
        0 => {
            let shift = fraction.leading_zeros() - 21;
            let normalized = fraction << shift;
            let exponent32 = 113u32 - shift;
            sign | (exponent32 << 23) | ((normalized & 0x03FF) << 13)
        }
        0x1F => sign | 0x7F80_0000 | (fraction << 13),
        _ => sign | ((exponent + 112) << 23) | (fraction << 13),
    };
    f32::from_bits(bits)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scramble(text: &[u8]) -> Vec<u8> {
        let mut body = vec![SCRAMBLE_MARKER];
        body.extend(text.iter().map(|byte| byte ^ SCRAMBLE_KEY));
        body.push(SCRAMBLE_KEY);
        let mut bytes = (body.len() as u16).to_le_bytes().to_vec();
        bytes.extend(body);
        bytes
    }

    fn plain(text: &[u8]) -> Vec<u8> {
        let mut body = text.to_vec();
        body.push(0x00);
        let mut bytes = (body.len() as u16).to_le_bytes().to_vec();
        bytes.extend(body);
        bytes
    }

    #[test]
    fn enable_ranges_read_as_pairs() {
        let mut data = Vec::new();
        for (first, count) in [(10000u32, 7u32), (11000, 39)] {
            data.extend(first.to_le_bytes());
            data.extend(count.to_le_bytes());
        }
        let parsed = parse_enable_file(&data).unwrap();
        assert_eq!(parsed.ranges.len(), 2);
        assert_eq!(parsed.ranges[0].first_row, 10000);
        assert_eq!(parsed.ranges[0].count, 7);
        assert_eq!(parsed.row_count(), 46);
        assert!(parsed.anomalies.is_empty());
    }

    #[test]
    fn enable_oddities_are_reported_not_rejected() {
        let mut data = Vec::new();
        for (first, count) in [(20u32, 0u32), (10, 5)] {
            data.extend(first.to_le_bytes());
            data.extend(count.to_le_bytes());
        }
        let parsed = parse_enable_file(&data).unwrap();
        let kinds: Vec<&str> = parsed
            .anomalies
            .iter()
            .map(|anomaly| anomaly.kind)
            .collect();
        assert_eq!(kinds, ["empty-enable-range", "enable-range-out-of-order"]);
    }

    #[test]
    fn a_partial_record_fails_at_the_start_of_the_remainder() {
        let error = parse_enable_file(&[0u8; 12]).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::TrailingPartialRecord);
        assert_eq!(error.offset(), 8);

        let error = parse_row_offsets(&[0u8; 7]).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::TrailingPartialRecord);
        assert_eq!(error.offset(), 4);
    }

    #[test]
    fn row_offsets_are_end_offsets_and_repeats_are_empty_rows() {
        let mut data = Vec::new();
        for value in [37u32, 74, 74, 74, 111] {
            data.extend(value.to_le_bytes());
        }
        let parsed = parse_row_offsets(&data).unwrap();
        assert_eq!(parsed.slot_count(), 5);
        assert_eq!(parsed.data_length(), 111);
        assert_eq!(
            parsed.rows,
            [
                RowSlot {
                    index: 0,
                    span: Span::new(0, 37)
                },
                RowSlot {
                    index: 1,
                    span: Span::new(37, 37)
                },
                RowSlot {
                    index: 4,
                    span: Span::new(74, 37)
                },
            ]
        );
        assert!(parsed.anomalies.is_empty());
    }

    #[test]
    fn a_backward_row_offset_is_an_anomaly_and_the_entry_survives() {
        let mut data = Vec::new();
        for value in [40u32, 20, 60] {
            data.extend(value.to_le_bytes());
        }
        let parsed = parse_row_offsets(&data).unwrap();
        assert_eq!(parsed.offsets, [40, 20, 60]);
        assert_eq!(parsed.anomalies.len(), 1);
        assert_eq!(parsed.anomalies[0].kind, "row-offset-out-of-order");
        assert_eq!(parsed.rows.len(), 2);
    }

    #[test]
    fn both_string_forms_decode() {
        let bytes = scramble("\u{30A8}\u{30E9}".as_bytes());
        let value = parse_sheet_string(&bytes, 0, 0).unwrap();
        assert!(value.scrambled);
        assert_eq!(value.rich.text_only(), "\u{30A8}\u{30E9}");
        assert_eq!(value.decoded_length(), 6);

        let bytes = plain(b"ok");
        let value = parse_sheet_string(&bytes, 0, 0).unwrap();
        assert!(!value.scrambled);
        assert_eq!(value.rich.text_only(), "ok");
        assert_eq!(value.decoded_length(), 2);
    }

    #[test]
    fn a_string_stream_tiles_the_input() {
        let mut data = scramble(b"one");
        data.extend(plain(b"two"));
        let strings = parse_string_stream(&data).unwrap();
        assert_eq!(strings.len(), 2);
        assert_eq!(strings[1].span.offset, strings[0].span.length);
        assert!(looks_like_string_stream(&data));
        assert!(!looks_like_string_stream(b"SEDB"));
        assert!(!looks_like_string_stream(b""));
    }

    #[test]
    fn a_bad_terminator_names_the_offending_byte() {
        let mut bytes = scramble(b"one");
        let last = bytes.len() - 1;
        bytes[last] = 0x41;
        let error = parse_sheet_string(&bytes, 0, 0).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::MalformedSheetString);
        assert_eq!(error.offset(), last as u64);

        let error = parse_sheet_string(&[0x00, 0x00], 0, 0).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::MalformedSheetString);
        assert_eq!(error.offset(), 0);
    }

    #[test]
    fn a_truncated_string_reports_where_the_read_started() {
        let error = parse_sheet_string(&[0x10, 0x00, 0xFF], 0, 0).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::UnexpectedEndOfInput);
        assert_eq!(error.offset(), 2);
    }

    #[test]
    fn rows_decode_against_the_column_list() {
        let columns = parse_column_list("str, s32, bool, float, u8").unwrap();
        let mut data = scramble(b"first");
        data.extend((-2i32).to_le_bytes());
        data.push(1);
        data.extend(1.5f32.to_bits().to_le_bytes());
        data.push(9);
        let row_length = data.len();
        data.extend(plain(b"second"));
        data.extend(7i32.to_le_bytes());
        data.push(0);
        data.extend(0.0f32.to_bits().to_le_bytes());
        data.push(0);

        let rows = parse_rows(&data, &columns).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].span, Span::new(0, row_length as u64));
        assert_eq!(rows[1].span.offset, row_length as u64);
        assert_eq!(
            rows[0].values[1],
            ColumnValue::Signed32 {
                span: Span::new(scramble(b"first").len() as u64, 4),
                value: -2
            }
        );
        assert_eq!(rows[0].values[3].type_name(), "float");
        match &rows[1].values[2] {
            ColumnValue::Boolean { value, raw, .. } => {
                assert!(!value);
                assert_eq!(*raw, 0);
            }
            other => panic!("expected a boolean, got {other:?}"),
        }
    }

    #[test]
    fn an_unknown_column_type_is_refused() {
        let error = parse_column_list("str,s64").unwrap_err();
        assert_eq!(error.kind(), ErrorKind::UnknownColumnType);
        let error = parse_rows(b"", &[]).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::UnknownColumnType);
    }

    #[test]
    fn a_short_final_row_fails_rather_than_truncating() {
        let columns = parse_column_list("s32,s32").unwrap();
        let error = parse_rows(&[0u8; 6], &columns).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::UnexpectedEndOfInput);
        assert_eq!(error.offset(), 4);
    }

    #[test]
    fn binary16_covers_normal_subnormal_zero_and_special_values() {
        assert_eq!(half_to_f32(0x63D0), 1000.0);
        assert_eq!(half_to_f32(0x3C00), 1.0);
        assert_eq!(half_to_f32(0x0001), 2f32.powi(-24));
        assert_eq!(half_to_f32(0x8000).to_bits(), (-0.0f32).to_bits());
        assert_eq!(half_to_f32(0x7C00), f32::INFINITY);
        assert!(half_to_f32(0x7E01).is_nan());
    }
}
