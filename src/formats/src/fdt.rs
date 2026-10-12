//! Bounded bitmap font container (fdt). See docs/formats/fdt.md.
//!
//! The chunk order, version gates, and table roles follow the two retail
//! FontData readers recorded in
//! `xivl-decomp:docs/resource/bitmap-font-container.md`. Where the client
//! reports a mismatch and continues, this reader refuses the input instead.
//! GLYP record fields follow the consumers in
//! `xivl-decomp:docs/resource/bitmap-font-glyph-consumers.md`.

use crate::error::{ErrorKind, FormatError, Result};
use crate::reader::{Reader, Span};

pub const CHUNK_HEADER_LENGTH: usize = 8;
pub const SCALAR_LENGTH: u64 = 4;
pub const PAGE_TABLE_LENGTH: u64 = 256;
pub const INDEX_PAGE_LENGTH: u64 = 512;
pub const GLYPH_RECORD_LENGTH: u64 = 14;
pub const UNMAPPED_PAGE: u8 = 0xFF;
pub const UNMAPPED_RECORD: u16 = 0xFFFF;
pub const SELECTOR_VECTORS: i8 = 4;

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

/// The copy layout glyph lookup applies, selected by a zero or nonzero version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlyphLayout {
    /// Nonzero version: bytes 0x08 and 0x09 are copied and the source origin
    /// words are at 0x0A and 0x0C.
    Versioned,
    /// Zero version: bytes 0x08 and 0x09 are cleared, the source origin words
    /// are at 0x08 and 0x0A, and bytes 0x0C and 0x0D are not read.
    Unversioned,
}

impl GlyphLayout {
    pub fn name(self) -> &'static str {
        match self {
            GlyphLayout::Versioned => "versioned",
            GlyphLayout::Unversioned => "unversioned",
        }
    }
}

/// One GLYP record, decoded as the client's lookup and draw path consume it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GlyphRecord {
    pub span: Span,
    /// Byte 0: index into the DDSF and GTEX resource list.
    pub resource_index: i8,
    /// Byte 1: index into the four selector vectors.
    pub selector: i8,
    pub offset_x: i8,
    pub offset_y: i8,
    pub width: i8,
    pub height: i8,
    /// Byte 6: horizontal advance before caller-flag adjustments.
    pub advance: i8,
    /// Byte 7: copied by lookup with no established consumer.
    pub byte_07: u8,
    /// Versioned byte 8: added to the advance, modulo 256, under caller flag 0x02.
    pub flag_02_advance_addend: Option<u8>,
    /// Versioned byte 9: copied by lookup with no established consumer.
    pub byte_09: Option<u8>,
    pub source_x: u16,
    pub source_y: u16,
    /// Unversioned bytes 0x0C and 0x0D, which lookup does not read.
    pub unread: Option<Span>,
}

/// A DDSF or GTEX chunk in the client's resource-list order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Texture {
    /// Index into `FdtFile::chunks`.
    pub chunk: usize,
    /// Width and height from a GTEX header at the start of the payload.
    pub dimensions: Option<(u16, u16)>,
}

/// Record counts that fail a consistency check. None of these is an error:
/// the client's bounds behavior for them is not a format rule.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GlyphChecks {
    pub resource_outside_textures: u64,
    pub offset_resource_outside_textures: u64,
    pub selector_outside_vectors: u64,
    pub negative_extent: u64,
    pub source_outside_texture: u64,
    /// Records whose source rectangle has no texture dimensions to compare.
    pub source_unchecked: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FdtFile {
    /// Every chunk in file order, including the chunks after GLYP.
    pub chunks: Vec<Chunk>,
    /// The VERS value, or zero when the file has no VERS chunk.
    pub version: u32,
    pub has_version_chunk: bool,
    /// DDSF and GTEX chunks, which the client appends in file order.
    pub textures: Vec<Texture>,
    /// The TEXC value added to a record's resource index, or zero without TEXC.
    pub texture_offset: u32,
    /// Index into `chunks` of the GLYP chunk.
    pub glyph_chunk: usize,
    pub mapped_pages: u32,
    pub index_pages: u64,
    pub mapped_keys: u64,
    pub glyph_records: u64,
    pub referenced_records: u64,
    pub page_table: Vec<u8>,
    pub index_table: Vec<u16>,
    pub layout: GlyphLayout,
    pub glyphs: Vec<GlyphRecord>,
    pub checks: GlyphChecks,
    /// Bytes after the last complete chunk that follows GLYP.
    pub trailing: Span,
}

impl FdtFile {
    /// The record a 16-bit key reaches through HTBL and LTBL.
    pub fn glyph(&self, key: u16) -> Option<&GlyphRecord> {
        let page = self.page_table[usize::from(key >> 8)];
        if page == UNMAPPED_PAGE {
            return None;
        }
        let record = self.index_table[usize::from(page) * 256 + usize::from(key & 0xFF)];
        if record == UNMAPPED_RECORD {
            return None;
        }
        self.glyphs.get(usize::from(record))
    }
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

    let mut textures = Vec::new();
    for texture in [DDSF, GTEX] {
        while walker.peek_tag() == Some(texture) {
            let chunk = walker.take(texture)?;
            textures.push(Texture {
                chunk: walker.chunks.len() - 1,
                dimensions: gtex_dimensions(payload(data, chunk)),
            });
        }
    }

    // The client compares these gates as unsigned values.
    let mut texture_offset = 0;
    if version >= 4 {
        let chunk = walker.take(TEXC)?;
        texture_offset = scalar(data, chunk)?;
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
    let index_table: Vec<u16> = index_bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
        .collect();
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

    let layout = if version == 0 {
        GlyphLayout::Unversioned
    } else {
        GlyphLayout::Versioned
    };
    let glyph_bytes = payload(data, glyphs);
    let records: Vec<GlyphRecord> = glyph_bytes
        .as_chunks::<14>()
        .0
        .iter()
        .enumerate()
        .map(|(index, record)| {
            let offset = glyphs.payload.offset + index as u64 * GLYPH_RECORD_LENGTH;
            decode_record(record, offset, layout)
        })
        .collect();
    let checks = check_records(&records, &textures, texture_offset);

    Ok(FdtFile {
        chunks: walker.chunks,
        version,
        has_version_chunk,
        textures,
        texture_offset,
        glyph_chunk,
        mapped_pages,
        index_pages,
        mapped_keys,
        glyph_records,
        referenced_records,
        page_table: page_bytes.to_vec(),
        index_table,
        layout,
        glyphs: records,
        checks,
        trailing,
    })
}

fn decode_record(record: &[u8; 14], offset: u64, layout: GlyphLayout) -> GlyphRecord {
    let signed = |index: usize| record[index] as i8;
    let word = |index: usize| u16::from_le_bytes([record[index], record[index + 1]]);
    let (flag_02_advance_addend, byte_09, source, unread) = match layout {
        GlyphLayout::Versioned => (Some(record[8]), Some(record[9]), 0x0A, None),
        GlyphLayout::Unversioned => (None, None, 0x08, Some(Span::new(offset + 0x0C, 2))),
    };
    GlyphRecord {
        span: Span::new(offset, GLYPH_RECORD_LENGTH),
        resource_index: signed(0),
        selector: signed(1),
        offset_x: signed(2),
        offset_y: signed(3),
        width: signed(4),
        height: signed(5),
        advance: signed(6),
        byte_07: record[7],
        flag_02_advance_addend,
        byte_09,
        source_x: word(source),
        source_y: word(source + 2),
        unread,
    }
}

fn check_records(glyphs: &[GlyphRecord], textures: &[Texture], offset: u32) -> GlyphChecks {
    let mut checks = GlyphChecks::default();
    let texture = |index: i64| {
        usize::try_from(index)
            .ok()
            .and_then(|index| textures.get(index))
    };
    for record in glyphs {
        let resource = i64::from(record.resource_index);
        let selected = texture(resource);
        if selected.is_none() {
            checks.resource_outside_textures += 1;
        }
        if texture(resource + i64::from(offset)).is_none() {
            checks.offset_resource_outside_textures += 1;
        }
        if !(0..SELECTOR_VECTORS).contains(&record.selector) {
            checks.selector_outside_vectors += 1;
        }
        let negative = record.width < 0 || record.height < 0;
        if negative {
            checks.negative_extent += 1;
        }
        match selected.and_then(|texture| texture.dimensions) {
            Some((width, height)) if !negative => {
                let right = u32::from(record.source_x) + record.width as u32;
                let bottom = u32::from(record.source_y) + record.height as u32;
                if right > u32::from(width) || bottom > u32::from(height) {
                    checks.source_outside_texture += 1;
                }
            }
            _ => checks.source_unchecked += 1,
        }
    }
    checks
}

/// GTEX width and height are big-endian words at 0x0A and 0x0C; see
/// docs/formats/gtex-pwib.md.
fn gtex_dimensions(payload: &[u8]) -> Option<(u16, u16)> {
    if payload.len() < 0x0E || !payload.starts_with(b"GTEX") {
        return None;
    }
    Some((
        u16::from_be_bytes([payload[0x0A], payload[0x0B]]),
        u16::from_be_bytes([payload[0x0C], payload[0x0D]]),
    ))
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
