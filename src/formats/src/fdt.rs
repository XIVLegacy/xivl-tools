//! Bounded bitmap font container (fdt). See docs/formats/fdt.md.
//!
//! The chunk order, version gates, and table roles follow the two retail
//! FontData readers recorded in
//! `xivl-decomp:docs/resource/bitmap-font-container.md`. Where the client
//! reports a mismatch and continues, this reader refuses the input instead.

use crate::error::{ErrorKind, FormatError, Result};
use crate::reader::{Reader, Span};

pub const CHUNK_HEADER_LENGTH: usize = 8;
pub const SCALAR_LENGTH: u64 = 4;
pub const PAGE_TABLE_LENGTH: u64 = 256;
pub const INDEX_PAGE_LENGTH: u64 = 512;
pub const GLYPH_RECORD_LENGTH: u64 = 14;
pub const UNMAPPED_PAGE: u8 = 0xFF;
pub const UNMAPPED_RECORD: u16 = 0xFFFF;

pub const VERS: [u8; 4] = *b"VERS";
pub const DDSF: [u8; 4] = *b"DDSF";
pub const GTEX: [u8; 4] = *b"GTEX";
pub const TEXC: [u8; 4] = *b"TEXC";
pub const FSIZ: [u8; 4] = *b"FSIZ";
pub const FSMX: [u8; 4] = *b"FSMX";
pub const BSLN: [u8; 4] = *b"BSLN";
pub const HMGN: [u8; 4] = *b"HMGN";
pub const HTBL: [u8; 4] = *b"HTBL";
pub const LTBL: [u8; 4] = *b"LTBL";
pub const GLYP: [u8; 4] = *b"GLYP";

/// One tag, little-endian length, and payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chunk {
    pub tag: [u8; 4],
    pub span: Span,
    pub payload: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FdtFile {
    /// Every chunk in file order, including the chunks after GLYP.
    pub chunks: Vec<Chunk>,
    /// The VERS value, or zero when the file has no VERS chunk.
    pub version: u32,
    pub has_version_chunk: bool,
    /// Count of DDSF and GTEX chunks, which the client appends in file order.
    pub texture_chunks: usize,
    /// Index into `chunks` of the GLYP chunk.
    pub glyph_chunk: usize,
    pub mapped_pages: u32,
    pub index_pages: u64,
    pub mapped_keys: u64,
    pub glyph_records: u64,
    pub referenced_records: u64,
    /// Bytes after the last complete chunk that follows GLYP.
    pub trailing: Span,
}

pub fn parse(data: &[u8]) -> Result<FdtFile> {
    let mut walker = Walker {
        data,
        offset: 0,
        chunks: Vec::new(),
    };

    let mut version = 0;
    let has_version_chunk = walker.peek_tag() == Some(VERS);
    if has_version_chunk {
        let chunk = walker.take(VERS)?;
        version = scalar(data, chunk)?;
    }

    let mut texture_chunks = 0;
    for texture in [DDSF, GTEX] {
        while walker.peek_tag() == Some(texture) {
            walker.take(texture)?;
            texture_chunks += 1;
        }
    }

    // The client compares these gates as unsigned values.
    if version >= 4 {
        let chunk = walker.take(TEXC)?;
        scalar(data, chunk)?;
    }
    let chunk = walker.take(FSIZ)?;
    scalar(data, chunk)?;
    if version >= 3 {
        let chunk = walker.take(FSMX)?;
        scalar(data, chunk)?;
    }
    let chunk = walker.take(BSLN)?;
    scalar(data, chunk)?;
    if version >= 2 {
        let chunk = walker.take(HMGN)?;
        scalar(data, chunk)?;
    }

    let pages = walker.take(HTBL)?;
    if pages.payload.length != PAGE_TABLE_LENGTH {
        return Err(FormatError::new(
            ErrorKind::InvalidChunkLength,
            pages.span.offset + 4,
            "HTBL must hold one page byte for each of 256 high key bytes",
        ));
    }
    let index = walker.take(LTBL)?;
    let index_pages = whole_records(index, INDEX_PAGE_LENGTH, "LTBL ends in a partial page")?;
    let glyphs = walker.take(GLYP)?;
    let glyph_records =
        whole_records(glyphs, GLYPH_RECORD_LENGTH, "GLYP ends in a partial record")?;
    let glyph_chunk = walker.chunks.len() - 1;

    let page_bytes = payload(data, pages);
    let mut mapped_pages = 0;
    for (high, &page) in page_bytes.iter().enumerate() {
        if page != UNMAPPED_PAGE {
            mapped_pages += 1;
            if u64::from(page) >= index_pages {
                return Err(FormatError::new(
                    ErrorKind::TableIndexOutOfRange,
                    pages.payload.offset + high as u64,
                    "HTBL page has no LTBL page",
                ));
            }
        }
    }

    let index_bytes = payload(data, index);
    let mut referenced = vec![false; glyph_records as usize];
    for (word, pair) in index_bytes.as_chunks::<2>().0.iter().enumerate() {
        let record = u16::from_le_bytes(*pair);
        if record != UNMAPPED_RECORD && u64::from(record) >= glyph_records {
            return Err(FormatError::new(
                ErrorKind::TableIndexOutOfRange,
                index.payload.offset + 2 * word as u64,
                "LTBL record index has no GLYP record",
            ));
        }
    }

    // A key reaches a record through its high-byte page and low-byte word.
    let mut mapped_keys = 0;
    for &page in page_bytes.iter().filter(|&&page| page != UNMAPPED_PAGE) {
        let start = page as usize * INDEX_PAGE_LENGTH as usize;
        for pair in index_bytes[start..start + INDEX_PAGE_LENGTH as usize]
            .as_chunks::<2>()
            .0
        {
            let record = u16::from_le_bytes(*pair);
            if record != UNMAPPED_RECORD {
                mapped_keys += 1;
                referenced[record as usize] = true;
            }
        }
    }
    let referenced_records = referenced.iter().filter(|&&used| used).count() as u64;

    // The client stops after GLYP. Later chunks are reported, not interpreted.
    while walker.remaining() >= CHUNK_HEADER_LENGTH {
        match walker.read_chunk() {
            Ok(chunk) => {
                walker.offset = chunk.span.end() as usize;
                walker.chunks.push(chunk);
            }
            Err(_) => break,
        }
    }
    let trailing = Span::new(walker.offset as u64, walker.remaining() as u64);

    Ok(FdtFile {
        chunks: walker.chunks,
        version,
        has_version_chunk,
        texture_chunks,
        glyph_chunk,
        mapped_pages,
        index_pages,
        mapped_keys,
        glyph_records,
        referenced_records,
        trailing,
    })
}

struct Walker<'a> {
    data: &'a [u8],
    offset: usize,
    chunks: Vec<Chunk>,
}

impl Walker<'_> {
    fn remaining(&self) -> usize {
        self.data.len() - self.offset
    }

    fn peek_tag(&self) -> Option<[u8; 4]> {
        self.data
            .get(self.offset..self.offset + 4)
            .map(|bytes| [bytes[0], bytes[1], bytes[2], bytes[3]])
    }

    fn read_chunk(&self) -> Result<Chunk> {
        let start = self.offset as u64;
        let mut reader = Reader::with_base(&self.data[self.offset..], start);
        let tag_bytes = reader.take(4)?;
        let tag = [tag_bytes[0], tag_bytes[1], tag_bytes[2], tag_bytes[3]];
        let length = u64::from(reader.u32_le()?);
        if length > reader.remaining() as u64 {
            return Err(FormatError::new(
                ErrorKind::ChunkLengthOutOfRange,
                start + 4,
                "chunk payload runs past the end of the input",
            ));
        }
        Ok(Chunk {
            tag,
            span: Span::new(start, CHUNK_HEADER_LENGTH as u64 + length),
            payload: Span::new(start + CHUNK_HEADER_LENGTH as u64, length),
        })
    }

    fn take(&mut self, expected: [u8; 4]) -> Result<Chunk> {
        let chunk = self.read_chunk()?;
        if chunk.tag != expected {
            return Err(FormatError::new(
                ErrorKind::UnexpectedChunk,
                chunk.span.offset,
                format!(
                    "expected chunk {}",
                    String::from_utf8_lossy(&expected).into_owned()
                ),
            ));
        }
        self.offset = chunk.span.end() as usize;
        self.chunks.push(chunk);
        Ok(chunk)
    }
}

fn payload(data: &[u8], chunk: Chunk) -> &[u8] {
    &data[chunk.payload.offset as usize..chunk.payload.end() as usize]
}

fn scalar(data: &[u8], chunk: Chunk) -> Result<u32> {
    if chunk.payload.length != SCALAR_LENGTH {
        return Err(FormatError::new(
            ErrorKind::InvalidChunkLength,
            chunk.span.offset + 4,
            "scalar chunk payload must be four bytes",
        ));
    }
    let bytes = payload(data, chunk);
    Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn whole_records(chunk: Chunk, stride: u64, detail: &str) -> Result<u64> {
    let whole = chunk.payload.length / stride;
    if !chunk.payload.length.is_multiple_of(stride) {
        return Err(FormatError::new(
            ErrorKind::TrailingPartialRecord,
            chunk.payload.offset + whole * stride,
            detail,
        ));
    }
    Ok(whole)
}
