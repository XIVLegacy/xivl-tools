//! Bounded animation bank directories and MTB section framing.
//!
//! Evidence and opaque payload limits are in `docs/formats/animation.md`.

use crate::error::{ErrorKind, FormatError, Result};
use crate::gtex_pwib::{self, PwibResource, TaggedResource, TaggedResourceKind};
use crate::reader::{Reader, Span};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OffsetTable {
    pub span: Span,
    pub offsets: Vec<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SectionKind {
    Header,
    Curve,
    SpuBinary,
    Opaque,
}

impl SectionKind {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Header => "Header",
            Self::Curve => "Curve",
            Self::SpuBinary => "SpuBinary",
            Self::Opaque => "opaque",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub name_index: u32,
    pub kind: SectionKind,
    /// A bounded view ending at the next section, name start, or input end.
    /// MTB has no evidenced section-length field.
    pub view: Span,
    pub opaque: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Motion {
    pub version: u32,
    pub total_properties: u16,
    pub tables: Vec<OffsetTable>,
    pub section_names: Vec<Span>,
    pub group_names: Vec<Span>,
    pub sections: Vec<Section>,
    pub opaque: Vec<Span>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BankEntry {
    pub span: Span,
    pub selected: bool,
    pub clamped: bool,
    pub subtype: Option<[u8; 4]>,
    pub motion: Option<Motion>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnimationFile {
    pub wrapper: Option<PwibResource>,
    pub directory: Option<Span>,
    pub entries: Vec<BankEntry>,
    pub opaque: Vec<Span>,
}

fn failure(kind: ErrorKind, offset: u64, detail: &str) -> FormatError {
    FormatError::new(kind, offset, detail)
}

fn is_animation(tag: &[u8]) -> bool {
    matches!(
        tag,
        b"mtb\0" | b"MTB " | b"MCB\0" | b"MCB " | b"SCB\0" | b"SCB "
    )
}

/// Reads a standalone MTB/MCB/SCB or a RES bank, optionally inside PWIB.
/// Curves, motion blobs, scheduler commands, and nested banks remain opaque.
pub fn parse(data: &[u8]) -> Result<AnimationFile> {
    let wrapper = if data.starts_with(b"PWIB") {
        match gtex_pwib::parse(data, TaggedResourceKind::Pwib)? {
            TaggedResource::Pwib(value) => Some(value),
            TaggedResource::Gtex(_) => unreachable!("requested PWIB"),
        }
    } else {
        None
    };
    let root = wrapper
        .as_ref()
        .map_or(Span::new(0, data.len() as u64), |value| value.first_segment);
    let mut reader = Reader::new(data).sub_reader(root.offset as usize, root.length as usize)?;
    let bytes = reader.as_slice();
    let tag = header(&mut reader)?;
    let mut directory = None;
    let mut spans = Vec::new();
    let mut known = Vec::new();
    if tag == *b"RES " {
        let header_size = u16::from_le_bytes([bytes[14], bytes[15]]) as usize;
        if header_size < 0x40 && header_size != 0x30 {
            return Err(failure(
                ErrorKind::HeaderTooSmall,
                root.offset + 14,
                "RES extended fields require header size 0x30 or at least 0x40",
            ));
        }
        reader.seek(0x30)?;
        let count_offset = reader.offset();
        let count = reader.u32_le()? as usize;
        reader.u32_le()?;
        let repeat_offset = reader.offset();
        if reader.u32_le()? as usize != count {
            return Err(failure(
                ErrorKind::SubresourceCountMismatch,
                repeat_offset,
                "RES counts differ",
            ));
        }
        reader.take(4)?;
        let start = header_size.max(0x40);
        if start > bytes.len() || count > (bytes.len() - start) / 16 {
            return Err(failure(
                ErrorKind::SubresourceCountOutOfRange,
                count_offset,
                "RES directory exceeds the bounded bank",
            ));
        }
        let payload = start + 16 * count;
        directory = Some(Span::new(root.offset + start as u64, (16 * count) as u64));
        known.push(Span::new(root.offset, 8));
        known.push(Span::new(root.offset + 14, 6));
        known.push(Span::new(root.offset + 0x30, 4));
        known.push(Span::new(root.offset + 0x38, 4));
        known.push(Span::new(root.offset + start as u64, (16 * count) as u64));
        reader.seek(start)?;
        for _ in 0..count {
            reader.u32_le()?;
            let field = reader.offset();
            let offset = u64::from(reader.u32_le()?);
            let length = u64::from(reader.u32_le()?);
            reader.u32_le()?;
            let at = payload as u64 + offset;
            let available = at.min(root.length);
            let span = Span::new(root.offset + available, length.min(root.length - available));
            let selected = root.length - available >= 8
                && &bytes[available as usize..available as usize + 4] == b"SEDB"
                && is_animation(&bytes[available as usize + 4..available as usize + 8]);
            let clamped = at + length > root.length;
            if selected && clamped {
                return Err(failure(
                    ErrorKind::AmbiguousPayloadSpan,
                    field,
                    "RES entry exceeds the first segment",
                ));
            }
            spans.push((span, field, selected, clamped));
        }
    } else if is_animation(&tag) {
        spans.push((root, root.offset + 4, true, false));
    } else {
        return Err(failure(
            ErrorKind::UnexpectedChunk,
            root.offset + 4,
            "expected an animation subtype or RES bank",
        ));
    }
    let mut entries = Vec::new();
    let mut animations = 0;
    let mut selected_bytes = vec![false; data.len()];
    for (span, field, selected, clamped) in spans {
        if selected {
            let claimed = &mut selected_bytes[span.offset as usize..span.end() as usize];
            if claimed.iter().any(|&byte| byte) {
                return Err(failure(
                    ErrorKind::AmbiguousPayloadSpan,
                    field,
                    "selected animation entries alias or overlap",
                ));
            }
            claimed.fill(true);
        }
        let mut child = Reader::new(data).sub_reader(span.offset as usize, span.length as usize)?;
        let bytes = child.as_slice();
        let subtype: Option<[u8; 4]> = if bytes.starts_with(b"SEDB") && bytes.len() >= 8 {
            Some(bytes[4..8].try_into().expect("four bytes"))
        } else {
            None
        };
        let motion = if selected {
            let tag = header(&mut child)?;
            animations += 1;
            if matches!(&tag, b"mtb\0" | b"MTB ") {
                Some(parse_motion(bytes, span.offset)?)
            } else {
                None
            }
        } else {
            None
        };
        if selected {
            known.push(span);
        }
        entries.push(BankEntry {
            span,
            selected,
            clamped,
            subtype,
            motion,
        });
    }
    if animations == 0 {
        return Err(failure(
            ErrorKind::UnexpectedChunk,
            root.offset + 4,
            "bank has no direct MTB, MCB, or SCB entry",
        ));
    }
    Ok(AnimationFile {
        wrapper,
        directory,
        entries,
        opaque: complement(Span::new(0, data.len() as u64), &known),
    })
}

fn header(reader: &mut Reader<'_>) -> Result<[u8; 4]> {
    let base = reader.base();
    if reader.take(4)? != b"SEDB" {
        return Err(failure(ErrorKind::BadMagic, base, "expected SEDB"));
    }
    let tag = reader.take(4)?.try_into().expect("four bytes");
    reader.u32_le()?;
    reader.u16_le()?;
    let size = reader.u16_le()? as usize;
    let declared = reader.u32_le()? as usize;
    if size < 0x14 {
        return Err(failure(
            ErrorKind::HeaderTooSmall,
            base + 14,
            "SEDB header is below 0x14 bytes",
        ));
    }
    if size > reader.len() {
        return Err(failure(
            ErrorKind::HeaderSizeOutOfRange,
            base + 14,
            "SEDB header exceeds input",
        ));
    }
    if declared > reader.len() {
        return Err(failure(
            ErrorKind::DeclaredSizeOutOfRange,
            base + 16,
            "SEDB declared size exceeds input",
        ));
    }
    Ok(tag)
}

fn parse_motion(data: &[u8], base: u64) -> Result<Motion> {
    let mut reader = Reader::with_base(data, base);
    reader.seek(8)?;
    let version = reader.u32_le()?;
    if version != 0x43 {
        return Err(failure(
            ErrorKind::UnexpectedChunk,
            base + 8,
            "only MTB version 0x43 is read",
        ));
    }
    reader.seek(14)?;
    if reader.u16_le()? != 0x40 {
        return Err(failure(
            ErrorKind::InvalidChunkLength,
            base + 14,
            "MTB tables require the evidenced 0x40-byte header",
        ));
    }
    reader.seek(0x30)?;
    let total_properties = reader.u16_le()?;
    reader.seek(0x40)?;
    let mut tables = Vec::new();
    let mut known = vec![
        Span::new(base, 12),
        Span::new(base + 14, 6),
        Span::new(base + 0x30, 2),
    ];
    for _ in 0..3 {
        let start = reader.offset();
        let count = reader.u32_le()? as usize;
        if count > reader.remaining() / 4 {
            return Err(failure(
                ErrorKind::TableOutOfRange,
                start,
                "MTB offset table exceeds the bounded resource",
            ));
        }
        let mut offsets = Vec::with_capacity(count);
        for _ in 0..count {
            offsets.push(reader.u32_le()?);
        }
        let span = Span::new(start, 4 + 4 * count as u64);
        known.push(span);
        tables.push(OffsetTable { span, offsets });
    }
    let table_end = reader.offset() - base;
    let mut names = Vec::new();
    let mut name_bytes = vec![false; data.len()];
    for table in &tables[1..] {
        let mut spans = Vec::new();
        for (index, &offset) in table.offsets.iter().enumerate() {
            let at = offset as usize;
            let field = table.span.offset + 4 + 4 * index as u64;
            if u64::from(offset) < table_end || at >= data.len() {
                return Err(failure(
                    ErrorKind::TableOutOfRange,
                    field,
                    "MTB name offset is outside the name storage",
                ));
            }
            let length = name_length(data, &mut name_bytes, at, base, field)?;
            let span = Span::new(base + u64::from(offset), length as u64);
            known.push(span);
            spans.push(span);
        }
        names.push(spans);
    }
    let mut boundaries: Vec<u64> = tables[0]
        .offsets
        .iter()
        .map(|&offset| u64::from(offset))
        .collect();
    boundaries.extend(names.iter().flatten().map(|span| span.offset - base));
    boundaries.push(data.len() as u64);
    boundaries.sort_unstable();
    // The client selects the first exact table-2 match, not later duplicates.
    let first_name = |wanted: &[u8]| {
        names[0].iter().position(|span| {
            let at = (span.offset - base) as usize;
            &data[at..at + span.length as usize - 1] == wanted
        })
    };
    let header_index = first_name(b"Header");
    let curve_index = first_name(b"Curve");
    let spu_index = first_name(b"SpuBinary");
    let mut starts = tables[0].offsets.clone();
    starts.sort_unstable();
    if starts.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(failure(
            ErrorKind::AmbiguousPayloadSpan,
            tables[0].span.offset + 4,
            "MTB section offsets alias",
        ));
    }
    let mut sections = Vec::new();
    for (index, &offset) in tables[0].offsets.iter().enumerate() {
        let field = tables[0].span.offset + 4 + 4 * index as u64;
        let next = boundaries.partition_point(|&end| end <= u64::from(offset));
        let end = boundaries.get(next).copied().unwrap_or(0);
        if offset % 16 != 0
            || u64::from(offset) < table_end
            || end < u64::from(offset) + 16
            || end > data.len() as u64
            || name_bytes.get(offset as usize).copied().unwrap_or(false)
        {
            return Err(failure(
                ErrorKind::TableOutOfRange,
                field,
                "MTB section header has no independent bounded view",
            ));
        }
        reader.seek(offset as usize)?;
        let name_index = reader.u32_le()?;
        names[0].get(name_index as usize).ok_or_else(|| {
            failure(
                ErrorKind::TableIndexOutOfRange,
                base + u64::from(offset),
                "MTB section name index exceeds table 2",
            )
        })?;
        let index = Some(name_index as usize);
        let kind = if index == header_index {
            SectionKind::Header
        } else if index == curve_index {
            SectionKind::Curve
        } else if index == spu_index {
            SectionKind::SpuBinary
        } else {
            SectionKind::Opaque
        };
        known.push(Span::new(base + u64::from(offset), 4));
        sections.push(Section {
            name_index,
            kind,
            view: Span::new(base + u64::from(offset), end - u64::from(offset)),
            opaque: Span::new(base + u64::from(offset) + 4, end - u64::from(offset) - 4),
        });
    }
    if !sections
        .iter()
        .any(|section| section.kind == SectionKind::Header)
    {
        return Err(failure(
            ErrorKind::UnexpectedChunk,
            tables[0].span.offset,
            "MTB lacks a Header section",
        ));
    }
    Ok(Motion {
        version,
        total_properties,
        tables,
        section_names: names.remove(0),
        group_names: names.remove(0),
        sections,
        opaque: complement(Span::new(base, data.len() as u64), &known),
    })
}

fn name_length(
    data: &[u8],
    claimed: &mut [bool],
    at: usize,
    base: u64,
    field: u64,
) -> Result<usize> {
    // Stop before revisiting claimed storage, including a prior terminator.
    for end in at..data.len() {
        #[cfg(test)]
        tests::NAME_BYTES_VISITED.with(|count| count.set(count.get() + 1));
        if claimed[end] {
            return Err(failure(
                ErrorKind::AmbiguousPayloadSpan,
                field,
                "MTB names alias or overlap",
            ));
        }
        if data[end] == 0 {
            claimed[at..=end].fill(true);
            return Ok(end - at + 1);
        }
    }
    Err(failure(
        ErrorKind::UnexpectedEndOfInput,
        base + at as u64,
        "MTB name has no bounded NUL terminator",
    ))
}

fn complement(whole: Span, known: &[Span]) -> Vec<Span> {
    let mut ordered = known.to_vec();
    ordered.sort_by_key(|span| span.offset);
    let mut cursor = whole.offset;
    let mut result = Vec::new();
    for span in ordered {
        if span.offset > cursor {
            result.push(Span::new(cursor, span.offset - cursor));
        }
        cursor = cursor.max(span.end());
    }
    if cursor < whole.end() {
        result.push(Span::new(cursor, whole.end() - cursor));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{inspect_bytes_as, InspectAs};

    const SECTIONS: &[u8] = include_bytes!("../../../tests/fixtures/public/animation/sections.bin");
    const BANK: &[u8] = include_bytes!("../../../tests/fixtures/public/animation/bank.bin");

    thread_local! {
        pub(super) static NAME_BYTES_VISITED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    }

    #[test]
    fn animation_aliased_names_visit_storage_once() {
        let count = 12_000u32;
        let section = (0x50 + count * 4 + 15) & !15;
        let first = section + 16;
        let long = first + 7;
        let mut data = SECTIONS[..64].to_vec();
        for word in [1, section, count, first] {
            data.extend_from_slice(&word.to_le_bytes());
        }
        for _ in 1..count {
            data.extend_from_slice(&long.to_le_bytes());
        }
        data.extend_from_slice(&0u32.to_le_bytes());
        data.resize(first as usize, 0);
        data.extend_from_slice(b"Header\0");
        data.resize(data.len() + 200_000, b'A');
        data.push(0);
        NAME_BYTES_VISITED.with(|visits| visits.set(0));
        let error = inspect_bytes_as(&data, &InspectAs::Animation).unwrap_err();
        assert_eq!(
            (error.kind(), error.offset()),
            (ErrorKind::AmbiguousPayloadSpan, 84)
        );
        NAME_BYTES_VISITED.with(|visits| assert_eq!(visits.get(), 200_009));
    }

    #[test]
    fn animation_opaque_aliases_remain_unclaimed() {
        let mut bank = BANK[16..80].to_vec();
        bank[48..52].copy_from_slice(&3u32.to_le_bytes());
        bank[56..60].copy_from_slice(&3u32.to_le_bytes());
        for (offset, length) in [(0u32, 180u32), (180, 64), (180, 64)] {
            for word in [0, offset, length, 0] {
                bank.extend_from_slice(&word.to_le_bytes());
            }
        }
        bank.extend_from_slice(&BANK[112..356]);
        bank[296..300].copy_from_slice(b"XTRA");
        let file = parse(&bank).unwrap();
        assert_eq!(file.entries[1].span, Span::new(292, 64));
        assert_eq!(file.entries[1].span, file.entries[2].span);
        assert!(!file.entries[1].selected);
        assert!(!file.entries[2].selected);
        assert!(file.opaque.contains(&Span::new(292, 64)));
        let report = inspect_bytes_as(&bank, &InspectAs::Animation).unwrap();
        assert!(report["entries"][1]["sha256"].is_null());
        assert!(report["entries"][2]["sha256"].is_null());
    }

    #[test]
    fn animation_reads_literal_section_and_bank_bounds() {
        let file = parse(SECTIONS).unwrap();
        let motion = file.entries[0].motion.as_ref().unwrap();
        assert_eq!(motion.version, 67);
        assert_eq!(motion.total_properties, 3);
        assert_eq!(motion.tables[0].span, Span::new(64, 12));
        assert_eq!(motion.tables[0].offsets, [96, 128]);
        assert_eq!(motion.tables[1].offsets, [160, 167]);
        assert_eq!(motion.tables[2].offsets, [173]);
        assert_eq!(motion.section_names, [Span::new(160, 7), Span::new(167, 6)]);
        assert_eq!(motion.group_names, [Span::new(173, 7)]);
        assert_eq!(motion.sections[0].kind, SectionKind::Header);
        assert_eq!(motion.sections[0].view, Span::new(96, 32));
        assert_eq!(motion.sections[0].opaque, Span::new(100, 28));
        assert_eq!(motion.sections[1].kind, SectionKind::Curve);
        assert_eq!(motion.sections[1].view, Span::new(128, 32));
        assert_eq!(
            motion.opaque,
            [
                Span::new(12, 2),
                Span::new(20, 28),
                Span::new(50, 14),
                Span::new(100, 28),
                Span::new(132, 28)
            ]
        );

        let bank = parse(BANK).unwrap();
        assert_eq!(bank.directory, Some(Span::new(80, 32)));
        assert_eq!(bank.entries.len(), 2);
        assert_eq!(bank.entries[0].span, Span::new(112, 180));
        assert_eq!(bank.entries[1].span, Span::new(292, 64));
        assert_eq!(bank.entries[1].subtype, Some(*b"SCB "));
        assert!(bank.entries[1].motion.is_none());
        assert_eq!(
            bank.opaque,
            [
                Span::new(0, 16),
                Span::new(24, 6),
                Span::new(36, 28),
                Span::new(68, 4),
                Span::new(76, 4),
                Span::new(356, 16),
            ]
        );
        assert_eq!(
            bank.entries[0].motion.as_ref().unwrap().sections[0].view,
            Span::new(208, 32)
        );
        assert_eq!(
            bank.wrapper.as_ref().unwrap().second_segment,
            Span::new(356, 16)
        );

        let document = inspect_bytes_as(BANK, &InspectAs::Animation).unwrap();
        assert_eq!(document["format"], "animation");
        assert_eq!(document["entries"][0]["motion"]["tables"][0]["count"], 2);
        let text = document.to_string();
        assert!(!text.contains("group0"));
        assert!(!text.contains("offsets"));
        assert_eq!(
            InspectAs::from_arguments(&["--as".into(), "animation".into()]).unwrap(),
            InspectAs::Animation
        );

        let mut opaque = SECTIONS.to_vec();
        opaque[167..172].copy_from_slice(b"Extra");
        assert_eq!(
            parse(&opaque).unwrap().entries[0]
                .motion
                .as_ref()
                .unwrap()
                .sections[1]
                .kind,
            SectionKind::Opaque
        );
        let mut spu = SECTIONS[..167].to_vec();
        spu.extend_from_slice(b"SpuBinary\0group0\0");
        spu[92..96].copy_from_slice(&177u32.to_le_bytes());
        assert_eq!(
            parse(&spu).unwrap().entries[0]
                .motion
                .as_ref()
                .unwrap()
                .sections[1]
                .kind,
            SectionKind::SpuBinary
        );
        let mut uppercase = SECTIONS.to_vec();
        uppercase[4..8].copy_from_slice(b"MTB ");
        assert_eq!(
            parse(&uppercase).unwrap().entries[0]
                .motion
                .as_ref()
                .unwrap()
                .tables[0]
                .offsets,
            [96, 128]
        );
        let opaque_entry =
            include_bytes!("../../../tests/fixtures/public/animation/opaque-entry.bin");
        let file = parse(opaque_entry).unwrap();
        assert_eq!(file.entries[1].subtype, Some(*b"XTRA"));
        assert_eq!(file.entries[1].span, Span::new(292, 64));
        assert!(file.entries[1].motion.is_none());
        assert!(!file.entries[1].selected);
        for (bytes, span) in [
            (
                include_bytes!("../../../tests/fixtures/public/animation/opaque-overrun.bin")
                    .as_slice(),
                Span::new(292, 64),
            ),
            (
                include_bytes!("../../../tests/fixtures/public/animation/opaque-outside.bin")
                    .as_slice(),
                Span::new(356, 0),
            ),
        ] {
            let file = parse(bytes).unwrap();
            assert_eq!(file.entries[1].span, span);
            assert!(file.entries[1].clamped);
            assert!(!file.entries[1].selected);
            assert!(file.opaque.contains(&Span::new(292, 80)));
            let report = inspect_bytes_as(bytes, &InspectAs::Animation).unwrap();
            assert!(report["entries"][1]["sha256"].is_null());
        }
        let duplicate =
            include_bytes!("../../../tests/fixtures/public/animation/duplicate-header.bin");
        let file = parse(duplicate).unwrap();
        let sections = &file.entries[0].motion.as_ref().unwrap().sections;
        assert_eq!(sections[0].kind, SectionKind::Header);
        assert_eq!(sections[1].kind, SectionKind::Opaque);
    }

    #[test]
    fn animation_refuses_malformed_fields_at_literal_offsets() {
        for (data, kind, offset) in [
            (&SECTIONS[..12], ErrorKind::UnexpectedEndOfInput, 12),
            (
                include_bytes!("../../../tests/fixtures/public/animation/table-overrun.bin")
                    .as_slice(),
                ErrorKind::TableOutOfRange,
                64,
            ),
            (
                include_bytes!("../../../tests/fixtures/public/animation/name-overrun.bin")
                    .as_slice(),
                ErrorKind::TableOutOfRange,
                80,
            ),
            (
                include_bytes!("../../../tests/fixtures/public/animation/section-overrun.bin")
                    .as_slice(),
                ErrorKind::TableOutOfRange,
                68,
            ),
            (
                include_bytes!("../../../tests/fixtures/public/animation/section-index.bin")
                    .as_slice(),
                ErrorKind::TableIndexOutOfRange,
                96,
            ),
            (
                include_bytes!("../../../tests/fixtures/public/animation/section-alias.bin")
                    .as_slice(),
                ErrorKind::AmbiguousPayloadSpan,
                68,
            ),
            (
                include_bytes!("../../../tests/fixtures/public/animation/section-unaligned.bin")
                    .as_slice(),
                ErrorKind::TableOutOfRange,
                68,
            ),
            (
                include_bytes!("../../../tests/fixtures/public/animation/old-version.bin")
                    .as_slice(),
                ErrorKind::UnexpectedChunk,
                8,
            ),
            (
                include_bytes!("../../../tests/fixtures/public/animation/unterminated-name.bin")
                    .as_slice(),
                ErrorKind::UnexpectedEndOfInput,
                173,
            ),
            (
                include_bytes!("../../../tests/fixtures/public/animation/missing-header.bin")
                    .as_slice(),
                ErrorKind::UnexpectedChunk,
                64,
            ),
            (
                include_bytes!("../../../tests/fixtures/public/animation/entry-overrun.bin")
                    .as_slice(),
                ErrorKind::AmbiguousPayloadSpan,
                84,
            ),
            (
                include_bytes!("../../../tests/fixtures/public/animation/wrapper-overrun.bin")
                    .as_slice(),
                ErrorKind::DeclaredSizeOutOfRange,
                4,
            ),
        ] {
            let error = parse(data).unwrap_err();
            assert_eq!((error.kind(), error.offset()), (kind, offset));
        }
        let mut malformed = BANK.to_vec();
        malformed[112 + 96..112 + 100].copy_from_slice(&2u32.to_le_bytes());
        let error = parse(&malformed).unwrap_err();
        assert_eq!(
            (error.kind(), error.offset()),
            (ErrorKind::TableIndexOutOfRange, 208)
        );
        let mut short_entry = BANK.to_vec();
        short_entry[88..92].copy_from_slice(&4u32.to_le_bytes());
        let error = parse(&short_entry).unwrap_err();
        assert_eq!(
            (error.kind(), error.offset()),
            (ErrorKind::UnexpectedEndOfInput, 116)
        );
        let mut short_res = BANK.to_vec();
        short_res[30..32].copy_from_slice(&0x20u16.to_le_bytes());
        let error = parse(&short_res).unwrap_err();
        assert_eq!(
            (error.kind(), error.offset()),
            (ErrorKind::HeaderTooSmall, 30)
        );
        let control_only =
            include_bytes!("../../../tests/fixtures/public/animation/control-only-bank.bin");
        let error = parse(control_only).unwrap_err();
        assert_eq!(
            (error.kind(), error.offset()),
            (ErrorKind::UnexpectedChunk, 20)
        );
        for (fixture, offset) in [
            (
                include_bytes!("../../../tests/fixtures/public/animation/name-alias.bin")
                    .as_slice(),
                84,
            ),
            (
                include_bytes!("../../../tests/fixtures/public/animation/name-overlap.bin")
                    .as_slice(),
                84,
            ),
            (
                include_bytes!("../../../tests/fixtures/public/animation/group-name-alias.bin")
                    .as_slice(),
                92,
            ),
            (
                include_bytes!("../../../tests/fixtures/public/animation/entry-alias.bin")
                    .as_slice(),
                100,
            ),
            (
                include_bytes!("../../../tests/fixtures/public/animation/entry-overlap.bin")
                    .as_slice(),
                100,
            ),
        ] {
            let error = parse(fixture).unwrap_err();
            assert_eq!(
                (error.kind(), error.offset()),
                (ErrorKind::AmbiguousPayloadSpan, offset)
            );
        }
        let later = include_bytes!("../../../tests/fixtures/public/animation/later-header.bin");
        let error = parse(later).unwrap_err();
        assert_eq!(
            (error.kind(), error.offset()),
            (ErrorKind::UnexpectedChunk, 64)
        );
    }

    #[test]
    fn animation_truncations_and_mutations_stay_bounded() {
        for fixture in [SECTIONS, BANK] {
            for end in 0..fixture.len() {
                let _ = parse(&fixture[..end]);
            }
            for field in (0..fixture.len() - 4).step_by(4) {
                let mut data = fixture.to_vec();
                data[field..field + 4].copy_from_slice(&u32::MAX.to_le_bytes());
                if let Ok(file) = parse(&data) {
                    for entry in file.entries {
                        assert!(entry.span.end() <= data.len() as u64);
                        if let Some(motion) = entry.motion {
                            for section in motion.sections {
                                assert!(section.view.offset >= entry.span.offset);
                                assert!(section.view.end() <= entry.span.end());
                            }
                        }
                    }
                }
            }
        }
    }
}
