//! Bounded material sampler framing. See docs/formats/material.md.

use serde_json::{json, Value};

use crate::digest::sha256_hex;
use crate::error::{ErrorKind, Result};
use crate::reader::{Reader, Span};
use crate::res_extents::{self, error, word, Leaf, SelectedExtents};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sampler {
    /// Minimum evidenced record prefix, not the complete variable record.
    pub prefix: Span,
    pub flags: u8,
    pub argument: u8,
    pub environment_slot: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parameters {
    pub span: Span,
    pub offset_table: Span,
    pub samplers: Vec<Sampler>,
    pub opaque: Vec<Span>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Material {
    pub span: Span,
    /// Physical RES directory ordinals, without native selection or ownership.
    pub entry_path: Vec<u32>,
    pub parameters: Vec<Parameters>,
    pub opaque: Vec<Span>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterialFile {
    pub materials: Vec<Material>,
    pub opaque: Vec<Span>,
}

fn complement(span: Span, mut known: Vec<Span>) -> Vec<Span> {
    known.sort_by_key(|part| part.offset);
    let mut end = span.offset;
    let mut opaque = Vec::new();
    for part in known {
        if end < part.offset {
            opaque.push(Span::new(end, part.offset - end));
        }
        end = end.max(part.end());
    }
    if end < span.end() {
        opaque.push(Span::new(end, span.end() - end));
    }
    opaque
}

fn parameters(data: &[u8], base: u64) -> Result<Parameters> {
    let mut reader = Reader::with_base(data, base);
    reader.slice_at(0, 0x20)?;
    let count = word(&mut reader, 0x14)? as usize;
    let table = reader.u32_le()? as usize;
    if table < 0x20 || table > data.len() || count as u64 * 4 > (data.len() - table) as u64 {
        return Err(error(
            ErrorKind::TableOutOfRange,
            base + 0x18,
            "PRAM sampler offset table escapes its body",
        ));
    }
    let table_end = table + count * 4;
    // Claim all prefixes before reading records; aliases never multiply work.
    let mut claimed = vec![false; data.len()];
    claimed[..0x20].fill(true);
    claimed[table..table_end].fill(true);
    let mut offsets = Vec::with_capacity(count);
    for index in 0..count {
        let field = table + index * 4;
        let offset = word(&mut reader, field)? as usize;
        if offset > data.len() || 0x20 > data.len() - offset {
            return Err(error(
                ErrorKind::TableOutOfRange,
                base + field as u64,
                "PRAM sampler prefix escapes its body",
            ));
        }
        if claimed[offset..offset + 0x20].iter().any(|value| *value) {
            return Err(error(
                ErrorKind::AmbiguousPayloadSpan,
                base + field as u64,
                "PRAM sampler prefixes overlap each other or the header or offset table",
            ));
        }
        claimed[offset..offset + 0x20].fill(true);
        offsets.push(offset);
    }
    let offset_table = Span::new(base + table as u64, (count * 4) as u64);
    let mut known = vec![Span::new(base + 0x14, 8), offset_table];
    let mut samplers = Vec::with_capacity(count);
    for offset in offsets {
        samplers.push(Sampler {
            prefix: Span::new(base + offset as u64, 0x20),
            flags: data[offset + 3],
            argument: data[offset + 0x0c],
            environment_slot: data[offset + 0x0d],
        });
        known.push(Span::new(base + offset as u64 + 3, 1));
        known.push(Span::new(base + offset as u64 + 0x0c, 2));
    }
    let span = Span::new(base, data.len() as u64);
    Ok(Parameters {
        span,
        offset_table,
        samplers,
        opaque: complement(span, known),
    })
}

/// Read a selected shd container, leaving shader bytecode and parameter values opaque.
pub fn parse_block(data: &[u8], base: u64) -> Result<Material> {
    let mut reader = Reader::with_base(data, base);
    if reader.take(8)? != b"SEDBshd\0" {
        return Err(error(ErrorKind::BadMagic, base, "expected SEDBshd"));
    }
    if reader.u32_le()? != 29 {
        return Err(error(
            ErrorKind::UnexpectedChunk,
            base + 8,
            "shd version outside the evidenced subset",
        ));
    }
    if reader.u8()? != 0 {
        return Err(error(
            ErrorKind::UnexpectedChunk,
            base + 12,
            "shd conversion path is unsupported",
        ));
    }
    reader.u8()?;
    if reader.u16_le()? != 0x30 {
        return Err(error(
            ErrorKind::InvalidChunkLength,
            base + 14,
            "shd header must be 0x30 bytes",
        ));
    }
    let extent = reader.u32_le()? as usize;
    if extent < 0x50 || extent > data.len() {
        return Err(error(
            ErrorKind::DeclaredSizeOutOfRange,
            base + 16,
            "shd extent escapes its selected slice",
        ));
    }
    let mut chunks = Reader::with_base(&data[0x30..extent], base + 0x30);
    if chunks.take(4)? != b"SHD\0" {
        return Err(error(
            ErrorKind::UnexpectedChunk,
            base + 0x30,
            "expected exact SHD root",
        ));
    }
    chunks.take(4)?;
    let root_size = chunks.u32_be()? as usize;
    if root_size < 32 || root_size != extent - 0x30 {
        return Err(error(
            ErrorKind::ChunkLengthOutOfRange,
            base + 0x38,
            "SHD root must fill the declared shd payload",
        ));
    }
    let mut at = 0x50;
    let mut found = Vec::new();
    while at < extent {
        let mut child = reader.sub_reader(at, extent - at)?;
        let tag = child.take(4)?;
        child.take(4)?;
        let size = child.u32_be()? as usize;
        let padded = child.u32_be()? as usize;
        if size < 16 || size > extent - at {
            return Err(error(
                ErrorKind::ChunkLengthOutOfRange,
                base + at as u64 + 8,
                "SHD child escapes its parent",
            ));
        }
        if tag == b"PRAM" {
            found.push(parameters(
                &data[at + 16..at + size],
                base + at as u64 + 16,
            )?);
        }
        let advance = if padded >= size {
            padded
        } else if at + size == extent {
            size
        } else {
            (size + 15) & !15
        };
        if advance > extent - at {
            return Err(error(
                ErrorKind::ChunkLengthOutOfRange,
                base + at as u64 + 12,
                "SHD child padding escapes its parent",
            ));
        }
        at += advance;
    }
    if found.is_empty() {
        return Err(error(
            ErrorKind::UnexpectedChunk,
            base + 0x30,
            "SHD contains no PRAM body",
        ));
    }
    let span = Span::new(base, extent as u64);
    let mut known = vec![
        Span::new(base, 13),
        Span::new(base + 14, 6),
        Span::new(base + 0x30, 4),
        Span::new(base + 0x38, 4),
    ];
    known.extend(found.iter().map(|pram| pram.span));
    Ok(Material {
        span,
        entry_path: Vec::new(),
        parameters: found,
        opaque: complement(span, known),
    })
}

const LEAF: Leaf = Leaf {
    magic: b"SEDBshd\0",
    word: "shd",
    header_error: "RES header outside the evidenced material containers",
    extent_error: "selected shd or RES escapes its parent",
};

fn collect(
    data: &[u8],
    base: u64,
    path: &mut Vec<u32>,
    found: &mut Vec<Material>,
    selected: &mut SelectedExtents,
) -> Result<()> {
    res_extents::collect(
        data,
        base,
        path,
        selected,
        &LEAF,
        &mut |bytes, base, path| {
            let mut material = parse_block(bytes, base)?;
            material.entry_path = path.to_vec();
            found.push(material);
            Ok(())
        },
    )
}

/// Enumerate explicit shd blocks in a standalone container, RES tree, or PWIB first segment.
pub fn parse(data: &[u8]) -> Result<MaterialFile> {
    let mut materials = Vec::new();
    if data.starts_with(b"SEDBshd\0") {
        materials.push(parse_block(data, 0)?);
    } else {
        let span = res_extents::first_segment(data)?;
        let bytes = Reader::new(data).slice_at(span.offset as usize, span.length as usize)?;
        collect(
            bytes,
            span.offset,
            &mut Vec::new(),
            &mut materials,
            &mut SelectedExtents::new(data.len()),
        )?;
        if materials.is_empty() {
            return Err(error(
                ErrorKind::UnexpectedChunk,
                span.offset,
                "RES tree contains no shd block",
            ));
        }
    }
    materials.sort_by_key(|material| material.span.offset);
    let opaque = complement(
        Span::new(0, data.len() as u64),
        materials.iter().map(|material| material.span).collect(),
    );
    Ok(MaterialFile { materials, opaque })
}

pub(crate) fn inspect(data: &[u8]) -> Result<Value> {
    let file = parse(data)?;
    let digest = |span: Span| json!({"span": span.to_json(), "sha256": sha256_hex(&data[span.offset as usize..span.end() as usize])});
    let materials: Vec<Value> = file.materials.iter().map(|material| json!({
        "source": digest(material.span), "entryPath": material.entry_path, "version": 29,
        "parameters": material.parameters.iter().map(|pram| json!({
            "source": digest(pram.span), "samplerCount": pram.samplers.len(), "offsetTable": digest(pram.offset_table),
            "samplers": pram.samplers.iter().map(|sampler| json!({"prefix": digest(sampler.prefix),
                "flags": sampler.flags, "argument": sampler.argument, "environmentSlot": sampler.environment_slot,
            })).collect::<Vec<_>>(), "opaque": pram.opaque.iter().copied().map(digest).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(), "opaque": material.opaque.iter().copied().map(digest).collect::<Vec<_>>(),
    })).collect();
    Ok(
        json!({"schemaVersion": 1, "operation": "inspect", "format": "material", "input": {"length": data.len()},
        "materials": materials, "opaque": file.opaque.iter().copied().map(digest).collect::<Vec<_>>()}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &[u8] = include_bytes!("../../../tests/fixtures/public/material/samplers.bin");

    #[test]
    fn material_reports_literal_sampler_fields_and_opaque_spans() {
        let file = parse(SAMPLE).unwrap();
        let material = &file.materials[0];
        assert_eq!(material.span, Span::new(0, 248));
        let pram = &material.parameters[0];
        assert_eq!(pram.span, Span::new(128, 120));
        assert_eq!(pram.offset_table, Span::new(160, 8));
        assert_eq!(
            pram.samplers,
            [
                Sampler {
                    prefix: Span::new(168, 32),
                    flags: 0xa0,
                    argument: 0,
                    environment_slot: 0
                },
                Sampler {
                    prefix: Span::new(208, 32),
                    flags: 0x21,
                    argument: 0xff,
                    environment_slot: 0xe1
                },
            ]
        );
        assert_eq!(
            pram.opaque,
            [
                Span::new(128, 20),
                Span::new(156, 4),
                Span::new(168, 3),
                Span::new(172, 8),
                Span::new(182, 29),
                Span::new(212, 8),
                Span::new(222, 26)
            ]
        );
        assert!(file.opaque.is_empty());
        let empty = parse(include_bytes!(
            "../../../tests/fixtures/public/material/empty.bin"
        ))
        .unwrap();
        assert!(empty.materials[0].parameters[0].samplers.is_empty());
        let padded = parse(include_bytes!(
            "../../../tests/fixtures/public/material/padded.bin"
        ))
        .unwrap();
        assert_eq!(padded.materials[0].parameters[0].span, Span::new(144, 120));
        assert_eq!(padded.materials[0].span, Span::new(0, 264));
    }

    #[test]
    fn material_nested_paths_and_errors_use_absolute_offsets() {
        let input = include_bytes!("../../../tests/fixtures/public/material/pwib.bin");
        let file = parse(input).unwrap();
        assert_eq!(file.materials[0].entry_path, [0, 0]);
        assert_eq!(file.materials[0].span, Span::new(176, 248));
        assert_eq!(
            file.materials[0].parameters[0].samplers[0].prefix,
            Span::new(344, 32)
        );
        assert_eq!(file.opaque, [Span::new(0, 176)]);
        let mut bad = input.to_vec();
        bad[336..340].copy_from_slice(&100u32.to_le_bytes());
        let failure = parse(&bad).unwrap_err();
        assert_eq!(
            (failure.kind(), failure.offset()),
            (ErrorKind::TableOutOfRange, 336)
        );
    }

    #[test]
    fn material_preflights_selected_aliases_before_recursion() {
        let input = include_bytes!("../../../tests/fixtures/public/material/selected-alias.bin");
        let mut selected = SelectedExtents::new(input.len());
        let mut found = Vec::new();
        let failure = collect(input, 0, &mut Vec::new(), &mut found, &mut selected).unwrap_err();
        assert_eq!(
            (failure.kind(), failure.offset()),
            (ErrorKind::AmbiguousPayloadSpan, 88)
        );
        assert_eq!(selected.visited.len(), 1);
        assert!(found.is_empty());
        let failure = parse(include_bytes!(
            "../../../tests/fixtures/public/material/selected-overlap.bin"
        ))
        .unwrap_err();
        assert_eq!(
            (failure.kind(), failure.offset()),
            (ErrorKind::AmbiguousPayloadSpan, 88)
        );
        let file = parse(include_bytes!(
            "../../../tests/fixtures/public/material/opaque-alias.bin"
        ))
        .unwrap();
        assert_eq!(file.materials[0].entry_path, [1]);
        assert_eq!(file.materials[0].span, Span::new(112, 248));
        assert_eq!(file.opaque, [Span::new(0, 112), Span::new(360, 16)]);
    }

    #[test]
    fn material_accepts_disjoint_selected_siblings_in_reverse_order() {
        let nested = include_bytes!("../../../tests/fixtures/public/material/nested-res.bin");
        let mut bytes = nested[..64].to_vec();
        bytes.extend_from_slice(&[0; 32]);
        for (offset, value) in [
            (16, 672u32),
            (48, 2),
            (56, 2),
            (68, 328),
            (72, 248),
            (88, 328),
        ] {
            bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes.extend_from_slice(&nested[80..]);
        bytes.extend_from_slice(SAMPLE);
        let file = parse(&bytes).unwrap();
        assert_eq!(file.materials.len(), 2);
        assert_eq!(file.materials[0].entry_path, [1, 0]);
        assert_eq!(file.materials[0].span, Span::new(176, 248));
        assert_eq!(file.materials[1].entry_path, [0]);
        assert_eq!(file.materials[1].span, Span::new(424, 248));
    }

    #[test]
    fn material_bounds_sampler_prefixes_and_preserves_opaque_data() {
        for (bytes, kind, offset) in [
            (
                include_bytes!("../../../tests/fixtures/public/material/sampler-offset.bin")
                    .as_slice(),
                ErrorKind::TableOutOfRange,
                160,
            ),
            (
                include_bytes!("../../../tests/fixtures/public/material/sampler-alias.bin")
                    .as_slice(),
                ErrorKind::AmbiguousPayloadSpan,
                164,
            ),
            (
                include_bytes!("../../../tests/fixtures/public/material/sampler-overlap.bin")
                    .as_slice(),
                ErrorKind::AmbiguousPayloadSpan,
                164,
            ),
            (
                include_bytes!("../../../tests/fixtures/public/material/short-body.bin").as_slice(),
                ErrorKind::UnexpectedEndOfInput,
                128,
            ),
        ] {
            let failure = parse_block(bytes, 4096).unwrap_err();
            assert_eq!((failure.kind(), failure.offset()), (kind, 4096 + offset));
        }
        let mut changed = SAMPLE.to_vec();
        changed[156..160].copy_from_slice(&u32::MAX.to_le_bytes());
        changed[188] ^= 0xff;
        let file = parse(&changed).unwrap();
        assert_eq!(file.materials[0].parameters[0].samplers[0].flags, 0xa0);
        assert_ne!(inspect(&changed).unwrap(), inspect(SAMPLE).unwrap());
    }

    #[test]
    fn material_explicit_dispatch_and_bounded_input_sweeps() {
        let how = crate::InspectAs::from_arguments(&["--as".into(), "material".into()]).unwrap();
        let report = crate::inspect_bytes_as(SAMPLE, &how).unwrap();
        assert_eq!(report["format"], "material");
        assert_eq!(report["materials"][0]["parameters"][0]["samplerCount"], 2);
        assert_ne!(crate::inspect_bytes(SAMPLE).unwrap()["format"], "material");
        for length in 0..SAMPLE.len() {
            assert!(parse(&SAMPLE[..length]).is_err());
        }
        for at in [148, 152, 160, 164] {
            for value in [0, 1, 31, 32, 40, 64, 80, 100, 120, u32::MAX] {
                let mut bytes = SAMPLE.to_vec();
                bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
                let _ = parse(&bytes);
            }
        }
    }
}
