//! Bounded, endian-explicit reads over a byte slice.
//!
//! The client formats mix endianness within one file (SEDB headers are
//! little-endian, the chunk trees inside some payloads are big-endian), so
//! there is no default: every integer read names its byte order at the call
//! site.
//!
//! A reader carries the absolute offset of its slice within the original
//! input, so a nested reader over a subresource still reports offsets a
//! reader of the whole file would recognize. Every read is bounds-checked
//! and returns `UnexpectedEndOfInput` at the offset the read started from.
//! nothing here can panic on hostile input.

use crate::error::{ErrorKind, FormatError, Result};

/// A half-open byte range of the input, in the normalized JSON form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub offset: u64,
    pub length: u64,
}

impl Span {
    pub fn new(offset: u64, length: u64) -> Self {
        Self { offset, length }
    }

    pub fn end(&self) -> u64 {
        self.offset.saturating_add(self.length)
    }

    pub fn to_json(self) -> serde_json::Value {
        serde_json::json!({ "offset": self.offset, "length": self.length })
    }
}

/// A cursor over a bounded slice that knows where that slice sits in the
/// original input.
#[derive(Debug, Clone)]
pub struct Reader<'a> {
    data: &'a [u8],
    base: u64,
    position: usize,
}

impl<'a> Reader<'a> {
    /// A reader over a whole input, whose slice starts at absolute offset 0.
    pub fn new(data: &'a [u8]) -> Self {
        Self::with_base(data, 0)
    }

    /// A reader over a slice that begins at `base` in the original input.
    pub fn with_base(data: &'a [u8], base: u64) -> Self {
        Self {
            data,
            base,
            position: 0,
        }
    }

    /// Absolute offset of the cursor within the original input.
    pub fn offset(&self) -> u64 {
        self.base + self.position as u64
    }

    /// Absolute offset of the first byte of this reader's slice.
    pub fn base(&self) -> u64 {
        self.base
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    pub fn remaining(&self) -> usize {
        self.data.len() - self.position
    }

    /// The whole slice this reader covers.
    pub fn as_slice(&self) -> &'a [u8] {
        self.data
    }

    /// Span of this reader's whole slice in the original input.
    pub fn span(&self) -> Span {
        Span::new(self.base, self.data.len() as u64)
    }

    /// Move the cursor to a slice-relative position.
    pub fn seek(&mut self, position: usize) -> Result<()> {
        if position > self.data.len() {
            return Err(self.out_of_range(self.base + position as u64, position, 0));
        }
        self.position = position;
        Ok(())
    }

    /// Read `count` bytes and advance.
    pub fn take(&mut self, count: usize) -> Result<&'a [u8]> {
        let start = self.position;
        let end = start.checked_add(count).ok_or_else(|| {
            FormatError::new(
                ErrorKind::UnexpectedEndOfInput,
                self.offset(),
                "read length overflows the address space",
            )
        })?;
        if end > self.data.len() {
            return Err(self.out_of_range(self.offset(), start, count));
        }
        self.position = end;
        Ok(&self.data[start..end])
    }

    /// Borrow a slice-relative range without moving the cursor.
    pub fn slice_at(&self, position: usize, count: usize) -> Result<&'a [u8]> {
        let end = position.checked_add(count).ok_or_else(|| {
            FormatError::new(
                ErrorKind::UnexpectedEndOfInput,
                self.base + position as u64,
                "read length overflows the address space",
            )
        })?;
        if end > self.data.len() {
            return Err(self.out_of_range(self.base + position as u64, position, count));
        }
        Ok(&self.data[position..end])
    }

    /// A reader over a slice-relative range, carrying the right absolute base.
    pub fn sub_reader(&self, position: usize, count: usize) -> Result<Reader<'a>> {
        let bytes = self.slice_at(position, count)?;
        Ok(Reader::with_base(bytes, self.base + position as u64))
    }

    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    pub fn u16_le(&mut self) -> Result<u16> {
        let bytes = self.take(2)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    pub fn u16_be(&mut self) -> Result<u16> {
        let bytes = self.take(2)?;
        Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
    }

    pub fn u32_le(&mut self) -> Result<u32> {
        let bytes = self.take(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    pub fn u32_be(&mut self) -> Result<u32> {
        let bytes = self.take(4)?;
        Ok(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    /// A four-byte tag, rendered with non-printable bytes escaped so it is
    /// always safe to put in ASCII JSON.
    pub fn tag4(&mut self) -> Result<String> {
        let bytes = self.take(4)?;
        Ok(escape_tag(bytes))
    }

    fn out_of_range(&self, offset: u64, start: usize, count: usize) -> FormatError {
        FormatError::new(
            ErrorKind::UnexpectedEndOfInput,
            offset,
            format!(
                "wanted {} byte(s) at slice position {}, {} available",
                count,
                start,
                self.data.len().saturating_sub(start)
            ),
        )
    }
}

/// Render a tag as ASCII: printable bytes verbatim, everything else as
/// `\xNN`. Tags reach JSON, and JSON here is ASCII-only.
pub fn escape_tag(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len());
    for &byte in bytes {
        if (0x20..0x7F).contains(&byte) && byte != b'\\' {
            text.push(byte as char);
        } else {
            text.push_str(&format!("\\x{byte:02x}"));
        }
    }
    text
}

/// Lowercase hex, the normalized form for binary values.
pub fn to_hex(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        text.push_str(&format!("{byte:02x}"));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_both_byte_orders() {
        let data = [0x01u8, 0x02, 0x03, 0x04];
        let mut reader = Reader::new(&data);
        assert_eq!(reader.u32_le().unwrap(), 0x0403_0201);
        reader.seek(0).unwrap();
        assert_eq!(reader.u32_be().unwrap(), 0x0102_0304);
        reader.seek(0).unwrap();
        assert_eq!(reader.u16_le().unwrap(), 0x0201);
        assert_eq!(reader.u16_be().unwrap(), 0x0304);
    }

    #[test]
    fn short_read_reports_the_offset_it_started_from() {
        let data = [0x01u8, 0x02, 0x03];
        let mut reader = Reader::new(&data);
        assert_eq!(reader.u16_le().unwrap(), 0x0201);
        let error = reader.u32_le().unwrap_err();
        assert_eq!(error.kind(), ErrorKind::UnexpectedEndOfInput);
        assert_eq!(error.offset(), 2);
    }

    #[test]
    fn nested_readers_report_absolute_offsets() {
        let data = [0u8; 16];
        let outer = Reader::new(&data);
        let mut inner = outer.sub_reader(8, 4).unwrap();
        assert_eq!(inner.offset(), 8);
        inner.take(4).unwrap();
        assert_eq!(inner.offset(), 12);
        let error = inner.u8().unwrap_err();
        assert_eq!(error.offset(), 12);
    }

    #[test]
    fn oversized_reads_do_not_overflow() {
        let data = [0u8; 4];
        let mut reader = Reader::new(&data);
        let error = reader.take(usize::MAX).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::UnexpectedEndOfInput);
        let error = reader.slice_at(1, usize::MAX).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::UnexpectedEndOfInput);
    }

    #[test]
    fn seek_past_the_end_fails_instead_of_moving() {
        let data = [0u8; 4];
        let mut reader = Reader::new(&data);
        assert!(reader.seek(5).is_err());
        assert_eq!(reader.offset(), 0);
        assert!(reader.seek(4).is_ok());
    }

    #[test]
    fn tags_and_hex_stay_ascii() {
        assert_eq!(escape_tag(b"RES "), "RES ");
        assert_eq!(escape_tag(&[0x00, 0xFF, b'a', b'\\']), "\\x00\\xffa\\x5c");
        assert_eq!(to_hex(&[0x00, 0x0f, 0xff]), "000fff");
    }
}
