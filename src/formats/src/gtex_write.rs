//! Exact GTEX rebuilding and canonical 2D container construction.

use crate::dds::max_mip_levels;
use crate::error::{ErrorKind, FormatError, Result};
use crate::gtex_pwib::{gtex_format, GtexResource, GTEX_MAGIC};
use crate::reader::Span;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GtexBuild {
    pub format_index: u8,
    pub width: u16,
    pub height: u16,
    pub mips: Vec<Vec<u8>>,
}

/// Build a 2D GTEX with a table at 24, zero padding to 16-byte alignment,
/// and contiguous surfaces in supplied mip order. No mip levels are generated.
pub fn build_gtex(build: &GtexBuild) -> Result<Vec<u8>> {
    let format = gtex_format(build.format_index).ok_or_else(|| {
        FormatError::new(
            ErrorKind::InvalidAttributeValue,
            6,
            "unmapped GTEX format index",
        )
    })?;
    if build.width == 0 || build.height == 0 {
        return Err(FormatError::new(
            ErrorKind::InvalidAttributeValue,
            if build.width == 0 { 10 } else { 12 },
            "GTEX dimensions must be nonzero",
        ));
    }
    if build.mips.is_empty()
        || build.mips.len()
            > max_mip_levels(u32::from(build.width), u32::from(build.height)) as usize
    {
        return Err(FormatError::new(
            ErrorKind::InvalidAttributeValue,
            7,
            "GTEX mip count is outside the geometric chain",
        ));
    }
    let data_base = (24 + build.mips.len() * 8).next_multiple_of(16);
    let mut sizes = Vec::with_capacity(build.mips.len());
    let mut total = data_base as u64;
    for level in 0..build.mips.len() {
        let width = u64::from((build.width >> level).max(1));
        let height = u64::from((build.height >> level).max(1));
        let size = if let Some(block_bytes) = format.block_bytes {
            width.div_ceil(4) * height.div_ceil(4) * u64::from(block_bytes)
        } else {
            width * height * u64::from(format.bits_per_pixel) / 8
        };
        total += size;
        sizes.push(size);
    }
    if total > u64::from(u32::MAX) || usize::try_from(total).is_err() {
        return Err(FormatError::new(
            ErrorKind::DeclaredSizeOutOfRange,
            20,
            "GTEX total size exceeds the addressable u32 range",
        ));
    }
    for (level, (mip, size)) in build.mips.iter().zip(&sizes).enumerate() {
        if mip.len() as u64 != *size {
            return Err(FormatError::new(
                ErrorKind::AmbiguousPayloadSpan,
                (28 + level * 8) as u64,
                format!("GTEX mip {level} has {} bytes, expected {size}", mip.len()),
            ));
        }
    }

    let mut bytes = vec![0; data_base];
    bytes[..4].copy_from_slice(GTEX_MAGIC);
    bytes[4..10].copy_from_slice(&[1, 1, build.format_index, build.mips.len() as u8, 2, 0]);
    bytes[10..12].copy_from_slice(&build.width.to_be_bytes());
    bytes[12..14].copy_from_slice(&build.height.to_be_bytes());
    bytes[14..16].copy_from_slice(&1u16.to_be_bytes());
    bytes[16..20].copy_from_slice(&24u32.to_be_bytes());
    bytes[20..24].copy_from_slice(&(data_base as u32).to_be_bytes());
    bytes.reserve(total as usize - data_base);
    for (level, mip) in build.mips.iter().enumerate() {
        let entry = 24 + level * 8;
        let relative = (bytes.len() - data_base) as u32;
        bytes[entry..entry + 4].copy_from_slice(&relative.to_be_bytes());
        bytes[entry + 4..entry + 8].copy_from_slice(&(mip.len() as u32).to_be_bytes());
        bytes.extend_from_slice(mip);
    }
    Ok(bytes)
}

/// Rebuild a parsed standalone GTEX, retaining opaque header bytes, gaps,
/// and encoded surfaces from `data` at their recorded offsets.
pub fn rebuild_gtex(data: &[u8], gtex: &GtexResource) -> Result<Vec<u8>> {
    let mut bytes = vec![0; data.len()];
    for span in gtex
        .header_unknown
        .iter()
        .chain(&gtex.data_gaps)
        .copied()
        .chain(gtex.surfaces.iter().map(|surface| surface.source_span))
    {
        let range = span_range(span, data.len())?;
        bytes[range.clone()].copy_from_slice(&data[range]);
    }
    write_at(&mut bytes, 0, GTEX_MAGIC)?;
    write_at(&mut bytes, 6, &[gtex.format_index, gtex.mip_levels])?;
    write_at(&mut bytes, 9, &[gtex.flags])?;
    write_at(&mut bytes, 10, &gtex.width.to_be_bytes())?;
    write_at(&mut bytes, 12, &gtex.height.to_be_bytes())?;
    write_at(&mut bytes, 14, &gtex.depth.to_be_bytes())?;
    write_at(&mut bytes, 16, &gtex.offset_table_base.to_be_bytes())?;
    write_at(&mut bytes, 20, &gtex.data_base.to_be_bytes())?;
    for surface in &gtex.surfaces {
        write_at(
            &mut bytes,
            surface.offset_field_span.offset,
            &surface.relative_offset.to_be_bytes(),
        )?;
        write_at(
            &mut bytes,
            surface.size_field_span.offset,
            &surface.declared_size.to_be_bytes(),
        )?;
    }
    Ok(bytes)
}

fn span_range(span: Span, length: usize) -> Result<std::ops::Range<usize>> {
    let end = span
        .offset
        .checked_add(span.length)
        .filter(|end| *end <= length as u64)
        .ok_or_else(|| {
            FormatError::new(
                ErrorKind::DeclaredSizeOutOfRange,
                span.offset,
                "GTEX rebuild span escapes the input",
            )
        })?;
    Ok(span.offset as usize..end as usize)
}

fn write_at(bytes: &mut [u8], offset: u64, value: &[u8]) -> Result<()> {
    let range = span_range(Span::new(offset, value.len() as u64), bytes.len())?;
    bytes[range].copy_from_slice(value);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gtex_pwib::{parse, TaggedResource, TaggedResourceKind, TextureKind};
    use crate::inspect::{validate_named_bytes_as, InspectAs};

    fn parsed(bytes: &[u8]) -> GtexResource {
        let TaggedResource::Gtex(gtex) = parse(bytes, TaggedResourceKind::Gtex).unwrap() else {
            panic!("expected GTEX")
        };
        gtex
    }

    fn build() -> GtexBuild {
        GtexBuild {
            format_index: 4,
            width: 5,
            height: 3,
            mips: vec![vec![0x31; 60], vec![0x72; 8], vec![0xa4; 4]],
        }
    }

    #[test]
    fn builds_each_mapping_with_literal_fields_and_spans() {
        for (index, sizes) in [
            (4, [60, 8, 4]),
            (22, [30, 4, 2]),
            (24, [16, 8, 8]),
            (26, [32, 16, 16]),
        ] {
            for count in [1, 3] {
                let build = GtexBuild {
                    format_index: index,
                    width: 5,
                    height: 3,
                    mips: sizes[..count]
                        .iter()
                        .enumerate()
                        .map(|(level, size)| vec![0x31 + level as u8; *size])
                        .collect(),
                };
                let bytes = build_gtex(&build).unwrap();
                let data_base = if count == 1 { 32 } else { 48 };
                assert_eq!(
                    &bytes[..24],
                    &[
                        b'G',
                        b'T',
                        b'E',
                        b'X',
                        1,
                        1,
                        index,
                        count as u8,
                        2,
                        0,
                        0,
                        5,
                        0,
                        3,
                        0,
                        1,
                        0,
                        0,
                        0,
                        24,
                        0,
                        0,
                        0,
                        data_base as u8
                    ]
                );
                let gtex = parsed(&bytes);
                assert_eq!((gtex.format_index, gtex.mip_levels), (index, count as u8));
                assert_eq!(
                    (gtex.width, gtex.height, gtex.depth, gtex.flags),
                    (5, 3, 1, 0)
                );
                assert_eq!(gtex.texture_kind, TextureKind::Texture2d);
                assert_eq!(gtex.offset_table_base, 24);
                assert_eq!(gtex.header, Span::new(0, data_base));
                assert_eq!(gtex.header_unknown, vec![Span::new(4, 2), Span::new(8, 1)]);
                assert!(gtex.data_gaps.is_empty());
                let mut offset = 0;
                for (level, surface) in gtex.surfaces.iter().enumerate() {
                    assert_eq!((surface.face, surface.mip_level), (0, level as u8));
                    assert_eq!(surface.relative_offset, offset as u32);
                    assert_eq!(surface.declared_size, sizes[level] as u32);
                    assert_eq!(
                        surface.source_span,
                        Span::new(data_base + offset, sizes[level] as u64)
                    );
                    assert_eq!(
                        surface.offset_field_span,
                        Span::new(24 + level as u64 * 8, 4)
                    );
                    assert_eq!(surface.size_field_span, Span::new(28 + level as u64 * 8, 4));
                    assert_eq!(surface.calculated_size, Some(sizes[level] as u64));
                    assert_eq!(
                        &bytes[(data_base + offset) as usize
                            ..(data_base + offset) as usize + sizes[level]],
                        &build.mips[level]
                    );
                    offset += sizes[level] as u64;
                }
                assert_eq!(gtex.data, Span::new(data_base, offset));
                assert_eq!(rebuild_gtex(&bytes, &gtex).unwrap(), bytes);
                let report = validate_named_bytes_as(&bytes, "", &InspectAs::Auto).unwrap();
                assert_eq!(report["checks"][1]["status"], "pass");
                assert_eq!(report["checks"][2]["status"], "pass");
            }
        }
    }

    #[test]
    fn zero_pads_the_canonical_header() {
        let mut build = build();
        build.mips.truncate(2);
        let bytes = build_gtex(&build).unwrap();
        assert_eq!(&bytes[40..48], &[0; 8]);
        assert_eq!(parsed(&bytes).header_unknown[2], Span::new(40, 8));
        let mut nonzero = bytes.clone();
        nonzero[43] = 9;
        let report = validate_named_bytes_as(&nonzero, "", &InspectAs::Gtex).unwrap();
        assert_eq!(report["checks"][1]["status"], "pass");
        assert_eq!(report["checks"][2]["status"], "not-applicable");
        let mut changed_header = bytes;
        changed_header[4] = 9;
        let report = validate_named_bytes_as(&changed_header, "", &InspectAs::Gtex).unwrap();
        assert_eq!(report["checks"][2]["status"], "fail");
    }

    #[test]
    fn rebuilds_opaque_header_leading_internal_and_trailing_gaps() {
        let mut bytes = build_gtex(&build()).unwrap();
        bytes.splice(48..48, [0x83; 8]);
        bytes.splice(116..116, [0xe1; 8]);
        bytes.extend_from_slice(&[0x59; 7]);
        bytes[4..6].copy_from_slice(&[0x56, 0x78]);
        bytes[8] = 0xab;
        for (field, offset) in [(24, 8u32), (32, 76), (40, 84)] {
            bytes[field..field + 4].copy_from_slice(&offset.to_be_bytes());
        }
        let gtex = parsed(&bytes);
        assert_eq!(
            gtex.data_gaps,
            vec![Span::new(48, 8), Span::new(116, 8), Span::new(136, 7)]
        );
        assert_eq!(rebuild_gtex(&bytes, &gtex).unwrap(), bytes);
        let mut edited = gtex.clone();
        edited.width = 6;
        let rebuilt = rebuild_gtex(&bytes, &edited).unwrap();
        bytes[10..12].copy_from_slice(&6u16.to_be_bytes());
        assert_eq!(rebuilt, bytes);
        edited.header_unknown.push(Span::new(u64::MAX, 1));
        let error = rebuild_gtex(&bytes, &edited).unwrap_err();
        assert_eq!(
            (error.kind(), error.offset()),
            (ErrorKind::DeclaredSizeOutOfRange, u64::MAX)
        );
    }

    #[test]
    fn rebuilds_noncanonical_models_without_imposing_build_limits() {
        let original = build_gtex(&build()).unwrap();
        let mut no_table = original.clone();
        no_table[6..10].copy_from_slice(&[3, 255, 0x9a, 3]);
        no_table[10..20].fill(0);
        assert_eq!(
            rebuild_gtex(&no_table, &parsed(&no_table)).unwrap(),
            no_table
        );
        let mut moved_table = original.clone();
        moved_table.splice(24..24, [0x93; 8]);
        moved_table[16..20].copy_from_slice(&32u32.to_be_bytes());
        moved_table[20..24].copy_from_slice(&56u32.to_be_bytes());
        assert_eq!(
            rebuild_gtex(&moved_table, &parsed(&moved_table)).unwrap(),
            moved_table
        );
        let mut zero_mips = original;
        zero_mips[7] = 0;
        assert_eq!(
            rebuild_gtex(&zero_mips, &parsed(&zero_mips)).unwrap(),
            zero_mips
        );
    }

    #[test]
    fn refuses_unmapped_dimensions_mip_counts_sizes_and_total_overflow() {
        for (index, width, height, mips, kind, offset) in [
            (
                3,
                5,
                3,
                vec![vec![0; 60]],
                ErrorKind::InvalidAttributeValue,
                6,
            ),
            (4, 0, 3, vec![vec![]], ErrorKind::InvalidAttributeValue, 10),
            (4, 5, 0, vec![vec![]], ErrorKind::InvalidAttributeValue, 12),
            (4, 5, 3, vec![], ErrorKind::InvalidAttributeValue, 7),
            (
                4,
                5,
                3,
                vec![vec![]; 4],
                ErrorKind::InvalidAttributeValue,
                7,
            ),
            (
                4,
                5,
                3,
                vec![vec![0; 59]],
                ErrorKind::AmbiguousPayloadSpan,
                28,
            ),
            (
                24,
                5,
                3,
                vec![vec![0; 17]],
                ErrorKind::AmbiguousPayloadSpan,
                28,
            ),
            (
                4,
                5,
                3,
                vec![vec![0; 60], vec![0; 7]],
                ErrorKind::AmbiguousPayloadSpan,
                36,
            ),
            (
                4,
                u16::MAX,
                u16::MAX,
                vec![vec![]],
                ErrorKind::DeclaredSizeOutOfRange,
                20,
            ),
        ] {
            let error = build_gtex(&GtexBuild {
                format_index: index,
                width,
                height,
                mips,
            })
            .unwrap_err();
            assert_eq!((error.kind(), error.offset()), (kind, offset));
        }
    }
}
