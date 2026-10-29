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
    /// Preserve range order and boundaries, ignoring source spans and anomalies.
    pub fn encode(&self) -> Vec<u8> {
        let ranges: Vec<_> = self
            .ranges
            .iter()
            .map(|range| (range.first_row, range.count))
            .collect();
        encode_enable_file(&ranges)
    }

    /// Total row identifiers named, counting a repeat twice. Overlaps are
    /// reported as anomalies rather than folded away here.
    pub fn row_count(&self) -> u64 {
        self.ranges.iter().map(|range| u64::from(range.count)).sum()
    }
}

/// Write enable ranges verbatim, without sorting or merging them.
pub fn encode_enable_file(ranges: &[(u32, u32)]) -> Vec<u8> {
    let mut data = Vec::new();
    for &(first_row, count) in ranges {
        data.extend_from_slice(&first_row.to_le_bytes());
        data.extend_from_slice(&count.to_le_bytes());
    }
    data
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

/// Write non-empty slots as `(slot index, row end offset)` pairs in increasing
/// order. Pair each end from [`encode_rows`] with its [`RowSlot::index`], not
/// [`Row::index`]. A row identifier is the slot index plus the block's `begin`;
/// enable ranges name those identifiers. Unlisted slots repeat the prior end.
pub fn encode_row_offsets(slot_count: usize, slots: &[(u64, u32)]) -> Result<Vec<u8>> {
    let mut next_slot = 0;
    let mut previous_end = 0;
    for &(index, end) in slots {
        let slot = usize::try_from(index)
            .ok()
            .filter(|&slot| slot >= next_slot && slot < slot_count)
            .ok_or_else(|| {
                FormatError::new(
                    ErrorKind::TableIndexOutOfRange,
                    index.saturating_mul(ROW_OFFSET_SIZE as u64),
                    "row slot indices must increase strictly and fit the slot count",
                )
            })?;
        if end <= previous_end {
            return Err(FormatError::new(
                ErrorKind::TableOutOfRange,
                index.saturating_mul(ROW_OFFSET_SIZE as u64),
                "non-empty row slots must have strictly increasing end offsets",
            ));
        }
        next_slot = slot + 1;
        previous_end = end;
    }

    let size = slot_count.checked_mul(ROW_OFFSET_SIZE).ok_or_else(|| {
        FormatError::new(
            ErrorKind::TableOutOfRange,
            0,
            "row-offset file size overflows usize",
        )
    })?;
    let mut data = Vec::new();
    data.try_reserve_exact(size).map_err(|_| {
        FormatError::new(
            ErrorKind::TableOutOfRange,
            0,
            "row-offset file is too large to allocate",
        )
    })?;
    let mut next = 0;
    let mut end = 0u32;
    for index in 0..slot_count {
        if let Some(&(slot, row_end)) = slots.get(next) {
            if slot == index as u64 {
                end = row_end;
                next += 1;
            }
        }
        data.extend_from_slice(&end.to_le_bytes());
    }
    Ok(data)
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
    /// Build a value from decoded bytes, retaining the rich-token framing.
    /// Source spans use zero as a placeholder until the written rows are parsed.
    pub fn from_decoded(decoded: &[u8], scrambled: bool) -> Result<Self> {
        let declared_length = sheet_string_body_length(decoded.len(), scrambled)?;
        if !scrambled {
            validate_plain_sheet_string(decoded, 0)?;
        }
        Ok(Self {
            span: Span::new(0, 0),
            declared_length,
            scrambled,
            rich: RichString::parse(decoded, 0)?,
            sha256: sha256_hex(decoded),
        })
    }

    /// Length of the decoded bytes, terminator excluded.
    pub fn decoded_length(&self) -> u64 {
        u64::from(self.declared_length) - 1 - u64::from(self.scrambled)
    }
}

fn sheet_string_body_length(decoded_length: usize, scrambled: bool) -> Result<u16> {
    decoded_length
        .checked_add(1 + usize::from(scrambled))
        .and_then(|length| u16::try_from(length).ok())
        .ok_or_else(|| {
            FormatError::new(
                ErrorKind::MalformedSheetString,
                0,
                "a sheet string body exceeds the u16 length field",
            )
        })
}

/// The length prefix frames the body, so interior NULs are unambiguous and
/// retail token payloads carry them. Only a leading marker byte would read
/// back as a scrambled string.
fn validate_plain_sheet_string(decoded: &[u8], base: u64) -> Result<()> {
    if decoded.first() == Some(&SCRAMBLE_MARKER) {
        return Err(FormatError::new(
            ErrorKind::MalformedSheetString,
            base,
            "a plain sheet string cannot begin with the scramble marker",
        ));
    }
    Ok(())
}

/// Recompute the length prefix and framing from the rich bytes and cipher state.
pub fn encode_sheet_string(value: &SheetString) -> Result<Vec<u8>> {
    let decoded = value.rich.encode();
    let length = sheet_string_body_length(decoded.len(), value.scrambled)?;
    if !value.scrambled {
        validate_plain_sheet_string(&decoded, 2)?;
    }
    RichString::parse(&decoded, 2 + u64::from(value.scrambled))?;
    let mut data = Vec::with_capacity(2 + usize::from(length));
    data.extend_from_slice(&length.to_le_bytes());
    if value.scrambled {
        data.push(SCRAMBLE_MARKER);
        data.extend(decoded.iter().map(|byte| byte ^ SCRAMBLE_KEY));
        data.push(SCRAMBLE_KEY);
    } else {
        data.extend_from_slice(&decoded);
        data.push(0);
    }
    Ok(data)
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

/// Append a column, ignoring its source span. Boolean and binary16 values use
/// their raw fields; binary32 values retain their exact bit patterns.
pub fn encode_column_value(value: &ColumnValue, out: &mut Vec<u8>) -> Result<()> {
    match value {
        ColumnValue::Text(string) => out.extend_from_slice(&encode_sheet_string(string)?),
        ColumnValue::Unsigned8 { value, .. } => out.push(*value),
        ColumnValue::Signed8 { value, .. } => out.extend_from_slice(&value.to_le_bytes()),
        ColumnValue::Boolean { raw, .. } => out.push(*raw),
        ColumnValue::Unsigned16 { value, .. } => out.extend_from_slice(&value.to_le_bytes()),
        ColumnValue::Signed16 { value, .. } => out.extend_from_slice(&value.to_le_bytes()),
        ColumnValue::Half16 { raw, .. } => out.extend_from_slice(&raw.to_le_bytes()),
        ColumnValue::Unsigned32 { value, .. } => out.extend_from_slice(&value.to_le_bytes()),
        ColumnValue::Signed32 { value, .. } => out.extend_from_slice(&value.to_le_bytes()),
        ColumnValue::Float32 { value, .. } => out.extend_from_slice(&value.to_bits().to_le_bytes()),
    }
    Ok(())
}

/// One row of a sheet data file.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub index: u64,
    pub span: Span,
    pub values: Vec<ColumnValue>,
}

/// Data bytes and cumulative row ends in input order, without empty slots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodedRows {
    pub data: Vec<u8>,
    pub row_ends: Vec<u32>,
}

/// Write rows and columns in input order, ignoring stored indices and spans.
pub fn encode_rows(rows: &[Row]) -> Result<EncodedRows> {
    let mut data = Vec::new();
    let mut row_ends = Vec::with_capacity(rows.len());
    for row in rows {
        if row.values.is_empty() {
            return Err(FormatError::new(
                ErrorKind::UnknownColumnType,
                data.len() as u64,
                "a row needs at least one column",
            ));
        }
        for value in &row.values {
            encode_column_value(value, &mut data)?;
        }
        row_ends.push(row_end_offset(data.len())?);
    }
    Ok(EncodedRows { data, row_ends })
}

fn row_end_offset(length: usize) -> Result<u32> {
    u32::try_from(length).map_err(|_| {
        FormatError::new(
            ErrorKind::DeclaredSizeOutOfRange,
            length as u64,
            "sheet data exceeds the u32 row-offset range",
        )
    })
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

    #[test]
    fn enable_writer_preserves_range_boundaries_and_order() {
        let data = [10, 0, 0, 0, 2, 0, 0, 0, 12, 0, 0, 0, 1, 0, 0, 0];
        let parsed = parse_enable_file(&data).unwrap();
        assert!(parsed.anomalies.is_empty());
        assert_eq!(parsed.encode(), data);
        assert_eq!(encode_enable_file(&[(10, 2), (12, 1)]), data);
        assert!(encode_enable_file(&[]).is_empty());
    }

    #[test]
    fn row_offset_writer_preserves_sparse_slots() {
        let data = [
            0, 0, 0, 0, 3, 0, 0, 0, 3, 0, 0, 0, 3, 0, 0, 0, 8, 0, 0, 0, 8, 0, 0, 0,
        ];
        let parsed = parse_row_offsets(&data).unwrap();
        assert!(parsed.anomalies.is_empty());
        let slots: Vec<_> = parsed
            .rows
            .iter()
            .map(|row| (row.index, row.span.end() as u32))
            .collect();
        assert_eq!(
            encode_row_offsets(parsed.slot_count(), &slots).unwrap(),
            data
        );
        assert_eq!(encode_row_offsets(2, &[]).unwrap(), [0; 8]);
        assert!(encode_row_offsets(0, &[]).unwrap().is_empty());
    }

    #[test]
    fn row_offset_writer_rejects_invalid_slots_and_ends() {
        for slots in [
            &[(3, 1)][..],
            &[(1, 1), (1, 2)],
            &[(2, 1), (1, 2)],
            &[(u64::MAX, 1)],
        ] {
            let error = encode_row_offsets(3, slots).unwrap_err();
            assert_eq!(error.kind(), ErrorKind::TableIndexOutOfRange);
        }
        for slots in [&[(0, 0)][..], &[(0, 2), (2, 2)], &[(0, 2), (2, 1)]] {
            let error = encode_row_offsets(3, slots).unwrap_err();
            assert_eq!(error.kind(), ErrorKind::TableOutOfRange);
            assert_eq!(error.offset(), slots.last().unwrap().0 * 4);
        }
        let error = encode_row_offsets(usize::MAX, &[]).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::TableOutOfRange);
    }

    #[test]
    fn string_constructor_and_writer_preserve_framing_and_tokens() {
        let decoded = b"ab\x02\x10\x01\x03cd\x02\x09\xf0\x02ef\x03";
        for (source, scrambled, expected) in [
            (&decoded[..], false, plain(decoded)),
            (&decoded[..], true, scramble(decoded)),
            (&b""[..], false, vec![1, 0, 0]),
            (&b""[..], true, vec![2, 0, 0xff, 0x73]),
        ] {
            let mut value = SheetString::from_decoded(source, scrambled).unwrap();
            assert_eq!(value.span, Span::new(0, 0));
            assert_eq!(value.rich.encode(), source);
            assert_eq!(value.sha256, sha256_hex(source));
            value.declared_length = 0;
            assert_eq!(encode_sheet_string(&value).unwrap(), expected);
            let parsed = parse_sheet_string(&expected, 0, 0).unwrap();
            assert_eq!(encode_sheet_string(&parsed).unwrap(), expected);
        }
        let abc = SheetString::from_decoded(b"abc", false).unwrap();
        assert_eq!(
            abc.sha256,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn a_plain_string_cannot_open_on_the_scramble_marker() {
        let error = SheetString::from_decoded(b"\xffx", false).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::MalformedSheetString);
        assert_eq!(error.offset(), 0);
        // Interior NULs are framed by the length prefix, plain or scrambled.
        for scrambled in [false, true] {
            let value = SheetString::from_decoded(b"a\0b", scrambled).unwrap();
            let encoded = encode_sheet_string(&value).unwrap();
            let parsed = parse_sheet_string(&encoded, 0, 0).unwrap();
            assert_eq!(parsed.rich.encode(), b"a\0b");
            assert_eq!(parsed.scrambled, scrambled);
        }
    }

    #[test]
    fn sheet_string_writer_checks_length_limits_and_edited_tokens() {
        for scrambled in [false, true] {
            let maximum = usize::from(u16::MAX) - 1 - usize::from(scrambled);
            let mut decoded = vec![b'a'; maximum];
            let mut value = SheetString::from_decoded(&decoded, scrambled).unwrap();
            let encoded = encode_sheet_string(&value).unwrap();
            assert_eq!(&encoded[..2], &[0xff, 0xff]);
            assert_eq!(encoded.len(), usize::from(u16::MAX) + 2);
            decoded.push(b'b');
            assert_eq!(
                SheetString::from_decoded(&decoded, scrambled)
                    .unwrap_err()
                    .kind(),
                ErrorKind::MalformedSheetString
            );
            value.rich = RichString::parse(&decoded, 0).unwrap();
            assert_eq!(
                encode_sheet_string(&value).unwrap_err().kind(),
                ErrorKind::MalformedSheetString
            );
        }
        let mut value = SheetString::from_decoded(b"\x02\x10\x01\x03", false).unwrap();
        if let crate::richstring::Segment::Token(token) = &mut value.rich.segments[0] {
            token.payload.push(b'x');
        }
        let mut out = vec![42];
        let error = encode_column_value(&ColumnValue::Text(Box::new(value)), &mut out).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::MalformedRichStringToken);
        assert_eq!(out, [42]);
    }

    #[test]
    fn row_writer_round_trips_all_ten_column_types_exactly() {
        let columns = parse_column_list("str,u8,s8,bool,u16,s16,f16,u32,s32,float").unwrap();
        let mut data = plain(b"ab\x02\x10\x01\x03cd\x02\x09\xf0\x02ef\x03");
        let mut fixed = [
            0xfe, 0x80, 2, 0x34, 0x12, 0xfe, 0xff, 1, 0x7e, 0x78, 0x56, 0x34, 0x12, 0xfe, 0xff,
            0xff, 0xff, 0x45, 0x23, 0x81, 0x7f,
        ];
        data.extend_from_slice(&fixed);
        let first_end = data.len() as u32;
        data.extend(scramble(b"\x02\x0a\xf2\x00\x03e\0g\x03"));
        fixed[8] = 0;
        data.extend_from_slice(&fixed);
        let mut rows = parse_rows(&data, &columns).unwrap();
        assert_eq!(rows.len(), 2);
        assert!(matches!(
            rows[0].values[3],
            ColumnValue::Boolean {
                raw: 2,
                value: true,
                ..
            }
        ));
        assert!(matches!(
            rows[0].values[6],
            ColumnValue::Half16 { raw: 0x7e01, .. }
        ));
        assert!(matches!(
            rows[1].values[6],
            ColumnValue::Half16 { raw: 1, .. }
        ));
        if let ColumnValue::Float32 { value, .. } = rows[0].values[9] {
            assert_eq!(value.to_bits(), 0x7f81_2345);
        } else {
            panic!("expected a binary32 column");
        }
        for row in &mut rows {
            row.span = Span::new(u64::MAX, 0);
            row.index = u64::MAX;
            if let ColumnValue::Boolean { value, .. } = &mut row.values[3] {
                *value = false;
            }
            if let ColumnValue::Half16 { value, .. } = &mut row.values[6] {
                *value = 0.0;
            }
        }
        let encoded = encode_rows(&rows).unwrap();
        assert_eq!(encoded.data, data);
        assert_eq!(encoded.row_ends, [first_end, data.len() as u32]);
    }

    #[test]
    fn edited_rows_rebuild_data_and_sparse_row_spans() {
        let columns = parse_column_list("u32,str").unwrap();
        let mut data = Vec::new();
        for (number, text) in [(1u32, plain(b"a")), (2, scramble(b"b")), (3, plain(b"c"))] {
            data.extend_from_slice(&number.to_le_bytes());
            data.extend(text);
        }
        let mut rows = parse_rows(&data, &columns).unwrap();
        if let ColumnValue::Unsigned32 { value, .. } = &mut rows[1].values[0] {
            *value = 0x1234_5678;
        }
        rows[1].values[1] = ColumnValue::Text(Box::new(
            SheetString::from_decoded(b"longer", true).unwrap(),
        ));
        let encoded = encode_rows(&rows).unwrap();
        assert_eq!(encoded.row_ends, [8, 22, 30]);
        let mut expected = data[..8].to_vec();
        expected.extend_from_slice(&[
            0x78, 0x56, 0x34, 0x12, 8, 0, 0xff, 0x1f, 0x1c, 0x1d, 0x14, 0x16, 1, 0x73,
        ]);
        expected.extend_from_slice(&data[17..]);
        assert_eq!(encoded.data, expected);
        let slots: Vec<_> = [1, 3, 4].into_iter().zip(encoded.row_ends).collect();
        let offsets = encode_row_offsets(7, &slots).unwrap();
        let offsets = parse_row_offsets(&offsets).unwrap();
        assert_eq!(offsets.offsets, [0, 8, 8, 22, 30, 30, 30]);
        assert!(offsets.anomalies.is_empty());
        let reparsed = parse_rows(&encoded.data, &columns).unwrap();
        for (row, slot) in reparsed.iter().zip(&offsets.rows) {
            assert_eq!(row.span, slot.span);
        }
        assert_eq!(reparsed.len(), offsets.rows.len());
        assert_eq!(reparsed[0], rows[0]);
        match &reparsed[1].values[0] {
            ColumnValue::Unsigned32 { value, .. } => assert_eq!(*value, 0x1234_5678),
            other => panic!("expected u32, got {other:?}"),
        }
        match &reparsed[1].values[1] {
            ColumnValue::Text(value) => assert_eq!(value.rich.encode(), b"longer"),
            other => panic!("expected text, got {other:?}"),
        }
    }

    #[test]
    fn row_writer_checks_offset_range_and_empty_rows() {
        assert_eq!(row_end_offset(u32::MAX as usize).unwrap(), u32::MAX);
        if let Ok(length) = usize::try_from(u64::from(u32::MAX) + 1) {
            let error = row_end_offset(length).unwrap_err();
            assert_eq!(error.kind(), ErrorKind::DeclaredSizeOutOfRange);
            assert_eq!(error.offset(), u64::from(u32::MAX) + 1);
        }
        let empty = encode_rows(&[]).unwrap();
        assert!(empty.data.is_empty());
        assert!(empty.row_ends.is_empty());
        let error = encode_rows(&[Row {
            index: 0,
            span: Span::new(0, 0),
            values: vec![],
        }])
        .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::UnknownColumnType);
    }
}
