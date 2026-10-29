//! Bounded RegionResourceData 1.1.0 structure. See docs/formats/region.md.

use crate::error::{ErrorKind, FormatError, Result};
use crate::reader::{Reader, Span};

pub const NAME: &[u8; 19] = b"RegionResourceData\0";
pub const VERSION: &[u8; 6] = b"1.1.0\0";
pub const HEADER_SIZE: usize = 0x40;
pub const ROW_STRIDE: usize = 0x30;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegionMembership {
    Root { index: u32, child_count: u32 },
    Child { root_index: u32, index: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegionRow {
    pub span: Span,
    pub membership: RegionMembership,
    pub id: u32,
    pub dat_key: u32,
    /// Raw row words +0x04, +0x0C and +0x20. A child's +0x04 is its type and
    /// +0x0C its stored kind; a root's +0x20 is its layout-world argument.
    pub word_04: u32,
    pub word_0c: u32,
    pub word_20: u32,
    pub token: Span,
    pub unknown: Vec<Span>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegionResourceData {
    pub header: Span,
    pub unknown_header: [Span; 3],
    /// Observed header word, not an established bound on the row walk.
    pub declared_size: u32,
    pub root_count: u32,
    pub rows: Vec<RegionRow>,
    pub rows_span: Span,
    pub trailing: Span,
}

pub fn has_signature(data: &[u8]) -> bool {
    data.starts_with(NAME)
}

pub fn parse(data: &[u8]) -> Result<RegionResourceData> {
    let mut reader = Reader::new(data);
    if reader.take(NAME.len())? != NAME {
        return Err(FormatError::new(
            ErrorKind::BadMagic,
            0,
            "expected RegionResourceData",
        ));
    }
    reader.take(0x18 - NAME.len())?;
    if reader.take(VERSION.len())? != VERSION {
        return Err(FormatError::new(
            ErrorKind::BadMagic,
            0x18,
            "expected RegionResourceData version 1.1.0",
        ));
    }
    reader.take(0x20 - 0x18 - VERSION.len())?;
    let declared_size = reader.u32_le()?;
    let root_count = reader.u32_le()?;
    reader.take(HEADER_SIZE - 0x28)?;

    let available_rows = reader.remaining() / ROW_STRIDE;
    if u64::from(root_count) > available_rows as u64 {
        return Err(count_error(
            0x24,
            "root rows cannot fit in the bounded input",
        ));
    }

    // Counts are checked against complete input rows before loops or allocation.
    let mut rows = Vec::new();
    for root_index in 0..root_count {
        let start = reader.offset();
        let bytes = reader.take(ROW_STRIDE)?;
        let mut root_reader = Reader::with_base(bytes, start);
        root_reader.seek(0x0C)?;
        let child_count = root_reader.u32_le()?;
        let required = u64::from(child_count) + u64::from(root_count - root_index - 1);
        if required > (reader.remaining() / ROW_STRIDE) as u64 {
            return Err(count_error(
                start + 0x0C,
                "child rows and remaining roots cannot fit in the bounded input",
            ));
        }
        rows.push(parse_row(
            bytes,
            start,
            RegionMembership::Root {
                index: root_index,
                child_count,
            },
        )?);
        for index in 0..child_count {
            let start = reader.offset();
            let bytes = reader.take(ROW_STRIDE)?;
            rows.push(parse_row(
                bytes,
                start,
                RegionMembership::Child { root_index, index },
            )?);
        }
    }
    let end = reader.offset();
    Ok(RegionResourceData {
        header: Span::new(0, HEADER_SIZE as u64),
        unknown_header: [Span::new(19, 5), Span::new(30, 2), Span::new(0x28, 0x18)],
        declared_size,
        root_count,
        rows,
        rows_span: Span::new(HEADER_SIZE as u64, end - HEADER_SIZE as u64),
        trailing: Span::new(end, reader.remaining() as u64),
    })
}

/// Name bytes the native constructors copy: at most 15 token bytes, ending
/// at the first NUL. `None` when `data` is not the parsed input.
pub fn effective_name<'a>(data: &'a [u8], row: &RegionRow) -> Option<&'a [u8]> {
    let start = row.token.offset as usize;
    let copied = data.get(start..start + 15)?;
    let end = copied.iter().position(|byte| *byte == 0).unwrap_or(15);
    Some(&copied[..end])
}

/// Whether the 16-byte token holds a NUL. The root's second name copy scans
/// for one when global 0x00F67298 holds its pinned sentinel 0xFFFFFFFF.
pub fn token_terminated(data: &[u8], row: &RegionRow) -> Option<bool> {
    let start = row.token.offset as usize;
    Some(data.get(start..start + 16)?.contains(&0))
}

/// The key of the extra resource request a root derives when its +0x20 word
/// is zero; the request itself is conditional.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuxiliaryKey {
    Derived(u32),
    NonzeroLayoutArgument,
    NoChildren,
    LowIndexOverflow,
}

/// Derives the auxiliary key for the root at `rows[index]`: the first
/// child's high 16 bits with one plus the largest child low index. Children
/// of every type count. Returns `None` when the row is not a root.
pub fn auxiliary_key(table: &RegionResourceData, index: usize) -> Option<AuxiliaryKey> {
    let root = table.rows.get(index)?;
    let RegionMembership::Root { child_count, .. } = root.membership else {
        return None;
    };
    if root.word_20 != 0 {
        return Some(AuxiliaryKey::NonzeroLayoutArgument);
    }
    let children = table
        .rows
        .get(index + 1..index + 1 + child_count as usize)?;
    let Some(first) = children.first() else {
        return Some(AuxiliaryKey::NoChildren);
    };
    let mut highest = 0;
    for child in children {
        let low = child.dat_key & 0xFFFF;
        if low >= 0xFFFF {
            return Some(AuxiliaryKey::LowIndexOverflow);
        }
        highest = highest.max(low);
    }
    Some(AuxiliaryKey::Derived(
        (first.dat_key & 0xFFFF_0000) | (highest + 1),
    ))
}

fn count_error(offset: u64, detail: &str) -> FormatError {
    FormatError::new(ErrorKind::SubresourceCountOutOfRange, offset, detail)
}

fn parse_row(bytes: &[u8], start: u64, membership: RegionMembership) -> Result<RegionRow> {
    let mut reader = Reader::with_base(bytes, start);
    let id = reader.u32_le()?;
    let word_04 = reader.u32_le()?;
    let dat_key = reader.u32_le()?;
    let word_0c = reader.u32_le()?;
    reader.seek(0x20)?;
    let word_20 = reader.u32_le()?;
    let mut unknown = vec![Span::new(start + 4, 4), Span::new(start + 0x20, 0x10)];
    // Only roots have a following-child count. Child +0x0C is opaque.
    if matches!(membership, RegionMembership::Child { .. }) {
        unknown.insert(1, Span::new(start + 0x0C, 4));
    }
    Ok(RegionRow {
        span: Span::new(start, ROW_STRIDE as u64),
        membership,
        id,
        dat_key,
        word_04,
        word_0c,
        word_20,
        token: Span::new(start + 0x10, 16),
        unknown,
    })
}
