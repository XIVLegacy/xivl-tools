//! Bounded SKL framing from xivl-decomp:docs/resource/skeleton-bone-framing.md.
//! Transform bytes and string encodings remain opaque. See docs/formats/skeleton.md.

use std::collections::HashSet;

use serde_json::{json, Value};

use crate::digest::sha256_hex;
use crate::error::{ErrorKind, FormatError, Result};
use crate::gtex_pwib::{self, TaggedResource, TaggedResourceKind};
use crate::reader::{Reader, Span};
use crate::sedb;

pub const RECORD_LENGTH: usize = 176;
pub const NO_LINK: u32 = u32::MAX;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoneRecord {
    pub span: Span,
    pub name_index: u32,
    pub auxiliary_string_indices: [u32; 2],
    pub parent: u32,
    pub first_child: u32,
    pub next_sibling: u32,
    pub self_index: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skeleton {
    pub span: Span,
    /// Physical RES directory ordinals, without a native selection claim.
    pub entry_path: Vec<u32>,
    pub records: Vec<BoneRecord>,
    /// Includes each NUL terminator. No character encoding is assumed.
    pub strings: Vec<Span>,
    pub record_array: Span,
    pub string_pool: Span,
    pub opaque: Vec<Span>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkeletonFile {
    pub skeletons: Vec<Skeleton>,
    /// Complement of the selected SKL extents, including outer containers.
    pub opaque: Vec<Span>,
}

fn error(kind: ErrorKind, offset: u64, detail: &str) -> FormatError {
    FormatError::new(kind, offset, detail)
}

fn word(reader: &mut Reader<'_>, at: usize) -> Result<u32> {
    reader.seek(at)?;
    reader.u32_le()
}

/// Parse one exact selected SKL extent, preserving absolute source offsets.
pub fn parse_block(data: &[u8], base: u64) -> Result<Skeleton> {
    let mut reader = Reader::with_base(data, base);
    if reader.take(8)? != b"SEDBSKL " {
        return Err(error(ErrorKind::BadMagic, base, "expected SEDBSKL "));
    }
    if reader.u32_le()? != 7000 {
        return Err(error(
            ErrorKind::UnexpectedChunk,
            base + 8,
            "SKL scalar outside the evidenced shape",
        ));
    }
    if reader.u8()? != 0 {
        return Err(error(
            ErrorKind::UnexpectedChunk,
            base + 12,
            "SKL conversion path is unsupported",
        ));
    }
    reader.u8()?;
    if reader.u16_le()? != 0x30 {
        return Err(error(
            ErrorKind::InvalidChunkLength,
            base + 14,
            "SKL requires a 0x30 header",
        ));
    }
    if reader.u32_le()? as usize != data.len() {
        return Err(error(
            ErrorKind::DeclaredSizeOutOfRange,
            base + 16,
            "SKL declared size must equal the selected extent",
        ));
    }
    if word(&mut reader, 0x30)? != 1 {
        return Err(error(
            ErrorKind::UnexpectedChunk,
            base + 0x30,
            "SKL requires one directory row",
        ));
    }
    let string_offset = word(&mut reader, 0x34)? as usize;
    let string_count = reader.u32_le()? as usize;
    if word(&mut reader, 0x44)? != 0 {
        return Err(error(
            ErrorKind::UnexpectedChunk,
            base + 0x44,
            "SKL directory offset outside the evidenced shape",
        ));
    }
    let record_bytes = reader.u32_le()? as usize;
    reader.slice_at(0, 0x50)?;
    let record_base = 0x50;
    if !record_bytes.is_multiple_of(RECORD_LENGTH) {
        return Err(error(
            ErrorKind::TrailingPartialRecord,
            base + 0x48,
            "SKL record length is not a multiple of 176",
        ));
    }
    // R = B + 0x30 + 16 * (count + 1); the accepted directory count is one.
    if string_offset != record_bytes || record_bytes > data.len() - record_base {
        return Err(error(
            ErrorKind::TableOutOfRange,
            base + 0x34,
            "SKL strings must immediately follow the bounded record array",
        ));
    }
    let string_base = record_base + record_bytes;
    if string_count > data.len() - string_base {
        return Err(error(
            ErrorKind::TableOutOfRange,
            base + 0x38,
            "SKL counted strings cannot fit",
        ));
    }
    let mut strings = Vec::with_capacity(string_count);
    let mut cursor = string_base;
    for _ in 0..string_count {
        let length = data[cursor..]
            .iter()
            .position(|byte| *byte == 0)
            .ok_or_else(|| {
                error(
                    ErrorKind::UnexpectedEndOfInput,
                    base + cursor as u64,
                    "SKL string has no bounded NUL terminator",
                )
            })?
            + 1;
        strings.push(Span::new(base + cursor as u64, length as u64));
        cursor += length;
    }
    let record_count = record_bytes / RECORD_LENGTH;
    let mut records = Vec::with_capacity(record_count);
    let mut opaque = vec![
        Span::new(base + 13, 1),
        Span::new(base + 20, 28),
        Span::new(base + 0x3c, 8),
        Span::new(base + 0x4c, 4),
    ];
    for index in 0..record_count {
        let at = record_base + index * RECORD_LENGTH;
        let name_index = word(&mut reader, at)?;
        let auxiliary_string_indices = [reader.u32_le()?, reader.u32_le()?];
        for (field, value) in [
            name_index,
            auxiliary_string_indices[0],
            auxiliary_string_indices[1],
        ]
        .iter()
        .enumerate()
        {
            if *value as usize >= string_count {
                return Err(error(
                    ErrorKind::TableIndexOutOfRange,
                    base + (at + field * 4) as u64,
                    "SKL string index is out of range",
                ));
            }
        }
        let parent = word(&mut reader, at + 0x38)?;
        let first_child = reader.u32_le()?;
        let next_sibling = reader.u32_le()?;
        let self_index = reader.u32_le()?;
        for (field, value) in [parent, first_child, next_sibling].iter().enumerate() {
            if *value != NO_LINK && *value as usize >= record_count {
                return Err(error(
                    ErrorKind::TableIndexOutOfRange,
                    base + (at + 0x38 + field * 4) as u64,
                    "SKL hierarchy index is out of range",
                ));
            }
        }
        if self_index as usize != index {
            return Err(error(
                ErrorKind::TableIndexOutOfRange,
                base + (at + 0x44) as u64,
                "SKL self index must equal its record ordinal",
            ));
        }
        records.push(BoneRecord {
            span: Span::new(base + at as u64, RECORD_LENGTH as u64),
            name_index,
            auxiliary_string_indices,
            parent,
            first_child,
            next_sibling,
            self_index,
        });
        opaque.push(Span::new(base + (at + 0x0c) as u64, 0x2c));
        opaque.push(Span::new(base + (at + 0x48) as u64, 0x68));
    }
    validate_parents(&records)?;
    if cursor < data.len() {
        opaque.push(Span::new(
            base + cursor as u64,
            (data.len() - cursor) as u64,
        ));
    }
    Ok(Skeleton {
        span: Span::new(base, data.len() as u64),
        entry_path: Vec::new(),
        records,
        strings,
        record_array: Span::new(base + record_base as u64, record_bytes as u64),
        string_pool: Span::new(base + string_base as u64, (cursor - string_base) as u64),
        opaque,
    })
}

fn validate_parents(records: &[BoneRecord]) -> Result<()> {
    // States are unseen, on the current parent chain, and complete.
    let mut states = vec![0u8; records.len()];
    for start in 0..records.len() {
        let mut index = start;
        while states[index] == 0 {
            states[index] = 1;
            let parent = records[index].parent;
            if parent == NO_LINK {
                break;
            }
            if states[parent as usize] == 1 {
                return Err(error(
                    ErrorKind::UnexpectedChunk,
                    records[index].span.offset + 0x38,
                    "SKL parent chain contains a cycle",
                ));
            }
            index = parent as usize;
        }
        index = start;
        while states[index] == 1 {
            states[index] = 2;
            let parent = records[index].parent;
            if parent == NO_LINK {
                break;
            }
            index = parent as usize;
        }
    }
    Ok(())
}

struct SelectedExtents {
    visited: HashSet<(u64, u32)>,
    // Deeper claims may replace ancestors, but siblings cannot share bytes.
    depths: Vec<u8>,
}

impl SelectedExtents {
    fn new(length: usize) -> Self {
        Self {
            visited: HashSet::new(),
            depths: vec![0; length],
        }
    }

    fn claim(&mut self, start: u64, length: u32, depth: u8, field: u64) -> Result<()> {
        let bytes = &mut self.depths[start as usize..start as usize + length as usize];
        if !self.visited.insert((start, length)) || bytes.iter().any(|claimed| *claimed >= depth) {
            return Err(error(
                ErrorKind::AmbiguousPayloadSpan,
                field,
                "selected SKL or RES extents alias or overlap",
            ));
        }
        // Each input byte is claimed at most once per bounded nesting level.
        bytes.fill(depth);
        Ok(())
    }
}

fn collect(
    data: &[u8],
    base: u64,
    path: &mut Vec<u32>,
    found: &mut Vec<Skeleton>,
    selected: &mut SelectedExtents,
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
            "expected a RES tree containing SKL",
        ));
    }
    reader.seek(14)?;
    let header = reader.u16_le()?;
    if !matches!(header, 0x30 | 0x40) {
        return Err(error(
            ErrorKind::InvalidChunkLength,
            base + 14,
            "RES header outside the evidenced skeleton containers",
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
        if tag != b"SEDBSKL " && tag != b"SEDBRES " {
            continue;
        }
        if u64::from(length) > (extent - start) as u64 {
            return Err(error(
                ErrorKind::AmbiguousPayloadSpan,
                base + (row + 8) as u64,
                "selected SKL or RES escapes its containing extent",
            ));
        }
        selected.claim(
            base + start as u64,
            length,
            (path.len() + 1) as u8,
            base + (row + 8) as u64,
        )?;
        children.push((ordinal, start, length, tag == b"SEDBSKL "));
    }
    for (ordinal, start, length, is_skeleton) in children {
        let bytes = &data[start..start + length as usize];
        path.push(ordinal);
        if is_skeleton {
            let mut skeleton = parse_block(bytes, base + start as u64)?;
            skeleton.entry_path = path.clone();
            found.push(skeleton);
        } else {
            collect(bytes, base + start as u64, path, found, selected)?;
        }
        path.pop();
    }
    Ok(())
}

/// Enumerate explicit SKL blocks in a standalone block, RES tree, or PWIB first segment.
pub fn parse(data: &[u8]) -> Result<SkeletonFile> {
    let mut skeletons = Vec::new();
    if data.starts_with(b"SEDBSKL ") {
        skeletons.push(parse_block(data, 0)?);
    } else {
        let span = if data.starts_with(b"PWIB") {
            let TaggedResource::Pwib(pwib) = gtex_pwib::parse(data, TaggedResourceKind::Pwib)?
            else {
                unreachable!()
            };
            pwib.first_segment
        } else {
            Span::new(0, data.len() as u64)
        };
        let bytes = Reader::new(data).slice_at(span.offset as usize, span.length as usize)?;
        collect(
            bytes,
            span.offset,
            &mut Vec::new(),
            &mut skeletons,
            &mut SelectedExtents::new(data.len()),
        )?;
        if skeletons.is_empty() {
            return Err(error(
                ErrorKind::UnexpectedChunk,
                span.offset,
                "RES tree contains no SKL block",
            ));
        }
    }
    skeletons.sort_by_key(|skeleton| skeleton.span.offset);
    let mut opaque = Vec::new();
    let mut end = 0;
    for skeleton in &skeletons {
        if end < skeleton.span.offset {
            opaque.push(Span::new(end, skeleton.span.offset - end));
        }
        end = skeleton.span.end();
    }
    if end < data.len() as u64 {
        opaque.push(Span::new(end, data.len() as u64 - end));
    }
    Ok(SkeletonFile { skeletons, opaque })
}

pub(crate) fn inspect(data: &[u8]) -> Result<Value> {
    let file = parse(data)?;
    let digest = |span: Span| json!({"span": span.to_json(), "sha256": sha256_hex(&data[span.offset as usize..span.end() as usize])});
    let skeletons: Vec<Value> = file.skeletons.iter().map(|skeleton| json!({
        "source": digest(skeleton.span), "entryPath": skeleton.entry_path,
        "scalar": 7000, "recordStride": RECORD_LENGTH,
        "recordArray": digest(skeleton.record_array), "recordCount": skeleton.records.len(),
        "stringPool": digest(skeleton.string_pool), "stringCount": skeleton.strings.len(),
        "strings": skeleton.strings.iter().map(|span| span.to_json()).collect::<Vec<_>>(),
        "rootCount": skeleton.records.iter().filter(|record| record.parent == NO_LINK).count(),
        "records": skeleton.records.iter().map(|record| json!({"span": record.span.to_json(),
            "nameIndex": record.name_index, "auxiliaryStringIndices": record.auxiliary_string_indices,
            "parent": record.parent, "firstChild": record.first_child,
            "nextSibling": record.next_sibling, "selfIndex": record.self_index,
        })).collect::<Vec<_>>(),
        "opaque": skeleton.opaque.iter().copied().map(digest).collect::<Vec<_>>(),
    })).collect();
    Ok(
        json!({"schemaVersion": 1, "operation": "inspect", "format": "skeleton",
            "input": {"length": data.len()}, "skeletons": skeletons,
            "opaque": file.opaque.iter().copied().map(digest).collect::<Vec<_>>(),
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const RECORDS: &[u8] = include_bytes!("../../../tests/fixtures/public/skeleton/records.bin");

    #[test]
    fn skeleton_decodes_indices_without_interpreting_opaque_bytes() {
        let skeleton = parse_block(RECORDS, 0).unwrap();
        assert_eq!(skeleton.record_array, Span::new(80, 528));
        assert_eq!(skeleton.string_pool, Span::new(608, 17));
        assert_eq!(
            skeleton.strings,
            vec![
                Span::new(608, 4),
                Span::new(612, 5),
                Span::new(617, 4),
                Span::new(621, 4)
            ]
        );
        assert_eq!(
            skeleton.records[0],
            BoneRecord {
                span: Span::new(80, 176),
                name_index: 0,
                auxiliary_string_indices: [3, 3],
                parent: NO_LINK,
                first_child: 1,
                next_sibling: NO_LINK,
                self_index: 0,
            }
        );
        assert_eq!(skeleton.records[1].name_index, 0);
        assert_eq!(skeleton.records[2].parent, 1);
        assert_eq!(skeleton.records[2].first_child, NO_LINK);
        assert_eq!(skeleton.opaque[4], Span::new(92, 44));
        assert_eq!(skeleton.opaque[5], Span::new(152, 104));
        assert_eq!(skeleton.opaque.last(), Some(&Span::new(625, 3)));
        let mut unknown_encoding = RECORDS.to_vec();
        unknown_encoding[608] = 0xff;
        assert!(parse_block(&unknown_encoding, 0).is_ok());
    }

    #[test]
    fn skeleton_uses_physical_paths_and_absolute_offsets() {
        let input = include_bytes!("../../../tests/fixtures/public/skeleton/pwib.bin");
        let file = parse(input).unwrap();
        assert_eq!(file.skeletons.len(), 1);
        assert_eq!(file.skeletons[0].entry_path, [0, 0]);
        assert_eq!(file.skeletons[0].span, Span::new(176, 628));
        assert_eq!(file.skeletons[0].records[0].span, Span::new(256, 176));
        assert_eq!(file.opaque, [Span::new(0, 176)]);
        let mut bad = input.to_vec();
        bad[260..264].copy_from_slice(&4u32.to_le_bytes());
        let failure = parse(&bad).unwrap_err();
        assert_eq!(
            (failure.kind(), failure.offset()),
            (ErrorKind::TableIndexOutOfRange, 260)
        );
    }

    #[test]
    fn skeleton_never_uses_a_terminator_outside_the_selected_slice() {
        let mut bytes =
            include_bytes!("../../../tests/fixtures/public/skeleton/unterminated.bin").to_vec();
        bytes.push(0);
        let failure = parse_block(&bytes[..628], 4096).unwrap_err();
        assert_eq!(
            (failure.kind(), failure.offset()),
            (ErrorKind::UnexpectedEndOfInput, 4717)
        );
        let mut short = RECORDS[..72].to_vec();
        short[16..20].copy_from_slice(&72u32.to_le_bytes());
        let failure = parse_block(&short, 4096).unwrap_err();
        assert_eq!(
            (failure.kind(), failure.offset()),
            (ErrorKind::UnexpectedEndOfInput, 4168)
        );
    }

    #[test]
    fn skeleton_explicit_dispatch_keeps_strings_out_of_reports() {
        let how = crate::InspectAs::from_arguments(&["--as".into(), "skeleton".into()]).unwrap();
        let report = crate::inspect_bytes_as(RECORDS, &how).unwrap();
        assert_eq!(report["format"], "skeleton");
        assert_eq!(report["skeletons"][0]["recordCount"], 3);
        assert_eq!(report["skeletons"][0]["rootCount"], 1);
        assert!(!report.to_string().contains("oak"));
        assert_ne!(crate::inspect_bytes(RECORDS).unwrap()["format"], "skeleton");
    }

    #[test]
    fn skeleton_preserves_opaque_aliases_without_normalizing_selected_rows() {
        let bytes = include_bytes!("../../../tests/fixtures/public/skeleton/opaque-alias.bin");
        let file = parse(bytes).unwrap();
        assert_eq!(file.skeletons[0].entry_path, [1]);
        assert_eq!(file.skeletons[0].span, Span::new(112, 628));
        assert_eq!(file.opaque, [Span::new(0, 112), Span::new(740, 16)]);
    }

    #[test]
    fn skeleton_bounds_work_before_following_aliased_selected_rows() {
        let input = include_bytes!("../../../tests/fixtures/public/skeleton/selected-alias.bin");
        let mut selected = SelectedExtents::new(input.len());
        let mut found = Vec::new();
        let failure = collect(input, 0, &mut Vec::new(), &mut found, &mut selected).unwrap_err();
        assert_eq!(
            (failure.kind(), failure.offset()),
            (ErrorKind::AmbiguousPayloadSpan, 88)
        );
        assert_eq!(selected.visited.len(), 1);
        assert!(found.is_empty());

        let mut overlapping = input.to_vec();
        overlapping[88..92].copy_from_slice(&3535u32.to_le_bytes());
        let failure = parse(&overlapping).unwrap_err();
        assert_eq!(
            (failure.kind(), failure.offset()),
            (ErrorKind::AmbiguousPayloadSpan, 88)
        );

        let mut wrapped =
            include_bytes!("../../../tests/fixtures/public/skeleton/nested-res.bin")[..80].to_vec();
        wrapped[16..20].copy_from_slice(&4192u32.to_le_bytes());
        wrapped[72..76].copy_from_slice(&4112u32.to_le_bytes());
        wrapped.extend_from_slice(input);
        let failure = parse(&wrapped).unwrap_err();
        assert_eq!(
            (failure.kind(), failure.offset()),
            (ErrorKind::AmbiguousPayloadSpan, 168)
        );
    }

    #[test]
    fn skeleton_accepts_disjoint_selected_siblings_in_reverse_order() {
        let nested = include_bytes!("../../../tests/fixtures/public/skeleton/nested-res.bin");
        let mut bytes = nested[..64].to_vec();
        bytes.extend_from_slice(&[0; 32]);
        for (offset, value) in [
            (16, 1432u32),
            (48, 2),
            (56, 2),
            (68, 708),
            (72, 628),
            (88, 708),
        ] {
            bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes.extend_from_slice(&nested[80..]);
        bytes.extend_from_slice(RECORDS);
        let file = parse(&bytes).unwrap();
        assert_eq!(file.skeletons.len(), 2);
        assert_eq!(file.skeletons[0].entry_path, [1, 0]);
        assert_eq!(file.skeletons[0].span, Span::new(176, 628));
        assert_eq!(file.skeletons[1].entry_path, [0]);
        assert_eq!(file.skeletons[1].span, Span::new(804, 628));
    }

    #[test]
    fn skeleton_refuses_parent_cycles_at_the_closing_parent_field() {
        for (input, offset) in [
            (
                include_bytes!("../../../tests/fixtures/public/skeleton/parent-self.bin"),
                4232,
            ),
            (
                include_bytes!("../../../tests/fixtures/public/skeleton/parent-cycle.bin"),
                4408,
            ),
        ] {
            let failure = parse_block(input, 4096).unwrap_err();
            assert_eq!(
                (failure.kind(), failure.offset()),
                (ErrorKind::UnexpectedChunk, offset)
            );
        }
        let mut forward = RECORDS.to_vec();
        forward[136..140].copy_from_slice(&2u32.to_le_bytes());
        forward[488..492].copy_from_slice(&NO_LINK.to_le_bytes());
        assert!(parse_block(&forward, 4096).is_ok());
    }
}
