//! Bounded RES walk shared by the material and skeleton readers.

use std::collections::HashSet;

use crate::error::{ErrorKind, FormatError, Result};
use crate::gtex_pwib::{self, TaggedResource, TaggedResourceKind};
use crate::reader::{Reader, Span};
use crate::sedb;

pub(crate) struct Leaf {
    pub(crate) magic: &'static [u8; 8],
    pub(crate) word: &'static str,
    pub(crate) header_error: &'static str,
    pub(crate) extent_error: &'static str,
}

pub(crate) fn error(kind: ErrorKind, offset: u64, detail: &str) -> FormatError {
    FormatError::new(kind, offset, detail)
}

pub(crate) fn word(reader: &mut Reader<'_>, at: usize) -> Result<u32> {
    reader.seek(at)?;
    reader.u32_le()
}

pub(crate) fn first_segment(data: &[u8]) -> Result<Span> {
    if data.starts_with(b"PWIB") {
        let TaggedResource::Pwib(pwib) = gtex_pwib::parse(data, TaggedResourceKind::Pwib)? else {
            unreachable!()
        };
        Ok(pwib.first_segment)
    } else {
        Ok(Span::new(0, data.len() as u64))
    }
}

pub(crate) struct SelectedExtents {
    pub(crate) visited: HashSet<(u64, u32)>,
    // Deeper claims may replace ancestors, but siblings cannot share bytes.
    depths: Vec<u8>,
}

impl SelectedExtents {
    pub(crate) fn new(length: usize) -> Self {
        Self {
            visited: HashSet::new(),
            depths: vec![0; length],
        }
    }

    fn claim(&mut self, start: u64, length: u32, depth: u8, field: u64, word: &str) -> Result<()> {
        let bytes = &mut self.depths[start as usize..start as usize + length as usize];
        if !self.visited.insert((start, length)) || bytes.iter().any(|claimed| *claimed >= depth) {
            return Err(error(
                ErrorKind::AmbiguousPayloadSpan,
                field,
                &format!("selected {word} or RES extents alias or overlap"),
            ));
        }
        // Each input byte is claimed at most once per bounded nesting level.
        bytes.fill(depth);
        Ok(())
    }
}

pub(crate) fn collect(
    data: &[u8],
    base: u64,
    path: &mut Vec<u32>,
    selected: &mut SelectedExtents,
    leaf: &Leaf,
    visit: &mut impl FnMut(&[u8], u64, &[u32]) -> Result<()>,
) -> Result<()> {
    if path.len() > sedb::MAX_NESTING_DEPTH as usize {
        return Err(error(
            ErrorKind::NestingTooDeep,
            base,
            "RES nesting exceeds eight",
        ));
    }
    let mut reader = Reader::with_base(data, base);
    if reader.take(8)? != b"SEDBRES " {
        return Err(error(
            ErrorKind::BadMagic,
            base,
            &format!("expected a RES tree containing {}", leaf.word),
        ));
    }
    reader.seek(14)?;
    if !matches!(reader.u16_le()?, 0x30 | 0x40) {
        return Err(error(
            ErrorKind::InvalidChunkLength,
            base + 14,
            leaf.header_error,
        ));
    }
    let extent = reader.u32_le()? as usize;
    if extent < 64 || extent > data.len() {
        return Err(error(
            ErrorKind::DeclaredSizeOutOfRange,
            base + 16,
            "RES extent escapes its selected slice",
        ));
    }
    let count = word(&mut reader, 0x30)?;
    if word(&mut reader, 0x38)? != count {
        return Err(error(
            ErrorKind::SubresourceCountMismatch,
            base + 0x38,
            "RES directory counts disagree",
        ));
    }
    let payload = 64 + u64::from(count) * sedb::RES_DIRECTORY_ENTRY_SIZE;
    if payload > extent as u64 {
        return Err(error(
            ErrorKind::SubresourceCountOutOfRange,
            base + 0x30,
            "RES directory escapes its extent",
        ));
    }
    let mut children = Vec::new();
    for ordinal in 0..count {
        let row = 64 + ordinal as usize * 16;
        let start = payload + u64::from(word(&mut reader, row + 4)?);
        let length = reader.u32_le()?;
        // Opaque entries can alias or carry alignment extents; only selected tags are followed.
        if start + 8 > extent as u64 || length < 8 {
            continue;
        }
        let start = start as usize;
        let tag = &data[start..start + 8];
        if tag != leaf.magic && tag != b"SEDBRES " {
            continue;
        }
        if u64::from(length) > (extent - start) as u64 {
            return Err(error(
                ErrorKind::AmbiguousPayloadSpan,
                base + row as u64 + 8,
                leaf.extent_error,
            ));
        }
        selected.claim(
            base + start as u64,
            length,
            (path.len() + 1) as u8,
            base + row as u64 + 8,
            leaf.word,
        )?;
        children.push((ordinal, start, length, tag == leaf.magic));
    }
    for (ordinal, start, length, is_leaf) in children {
        let bytes = &data[start..start + length as usize];
        path.push(ordinal);
        if is_leaf {
            visit(bytes, base + start as u64, path)?;
        } else {
            collect(bytes, base + start as u64, path, selected, leaf, visit)?;
        }
        path.pop();
    }
    Ok(())
}
