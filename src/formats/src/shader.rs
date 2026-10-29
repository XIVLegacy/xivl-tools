//! Bounded SHD FILE envelopes. See docs/formats/shader.md.

use serde_json::{json, Value};

use crate::digest::sha256_hex;
use crate::error::{ErrorKind, FormatError, Result};
use crate::material;
use crate::reader::{Reader, Span};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChunk {
    pub span: Span,
    pub body: Span,
    pub padded_size: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shader {
    pub span: Span,
    /// Physical RES directory ordinals, without native selection or ownership.
    pub entry_path: Vec<u32>,
    pub files: Vec<FileChunk>,
    pub opaque: Vec<Span>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShaderFile {
    pub shaders: Vec<Shader>,
    pub opaque: Vec<Span>,
}

/// Read FILE envelopes in the material reader's bounded shd subset.
/// FILE bodies remain opaque, including descriptors and any inline programs.
pub fn parse(data: &[u8]) -> Result<ShaderFile> {
    // Material selection claims all selected sibling extents before recursion.
    let selected = material::parse(data)?;
    let mut shaders = Vec::with_capacity(selected.materials.len());
    for material in selected.materials {
        let span = material.span;
        let bytes = Reader::new(data).slice_at(span.offset as usize, span.length as usize)?;
        let reader = Reader::with_base(bytes, span.offset);
        let mut at = 0x50;
        let mut files = Vec::new();
        let mut opaque = vec![
            Span::new(span.offset + 13, 1),
            Span::new(span.offset + 20, 28),
            Span::new(span.offset + 0x34, 4),
            Span::new(span.offset + 0x3c, 20),
        ];
        while at < bytes.len() {
            let mut child = reader.sub_reader(at, bytes.len() - at)?;
            let tag = child.take(4)?;
            child.take(4)?;
            let size = child.u32_be()? as usize;
            let padded_size = child.u32_be()?;
            // The material walk already checked each logical and padded extent.
            let advance = if padded_size as usize >= size {
                padded_size as usize
            } else if at + size == bytes.len() {
                size
            } else {
                (size + 15) & !15
            };
            let base = span.offset + at as u64;
            if tag == b"FILE" {
                files.push(FileChunk {
                    span: Span::new(base, size as u64),
                    body: Span::new(base + 16, (size - 16) as u64),
                    padded_size,
                });
                opaque.push(Span::new(base + 4, 4));
                if size > 16 {
                    opaque.push(Span::new(base + 16, (size - 16) as u64));
                }
                if advance > size {
                    opaque.push(Span::new(base + size as u64, (advance - size) as u64));
                }
            } else {
                opaque.push(Span::new(base, advance as u64));
            }
            at += advance;
        }
        if files.is_empty() {
            return Err(FormatError::new(
                ErrorKind::UnexpectedChunk,
                span.offset + 0x30,
                "SHD contains no FILE child",
            ));
        }
        shaders.push(Shader {
            span,
            entry_path: material.entry_path,
            files,
            opaque,
        });
    }
    Ok(ShaderFile {
        shaders,
        opaque: selected.opaque,
    })
}

pub(crate) fn inspect(data: &[u8]) -> Result<Value> {
    let file = parse(data)?;
    let digest = |span: Span| json!({"span": span.to_json(), "sha256": sha256_hex(&data[span.offset as usize..span.end() as usize])});
    Ok(json!({
        "schemaVersion": 1, "operation": "inspect", "format": "shader", "input": {"length": data.len()},
        "shaders": file.shaders.iter().map(|shader| json!({
            "source": digest(shader.span), "entryPath": shader.entry_path, "version": 29,
            "files": shader.files.iter().map(|chunk| json!({
                "source": digest(chunk.span), "body": digest(chunk.body), "paddedSize": chunk.padded_size,
            })).collect::<Vec<_>>(),
            "opaque": shader.opaque.iter().copied().map(digest).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "opaque": file.opaque.iter().copied().map(digest).collect::<Vec<_>>(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &[u8] = include_bytes!("../../../tests/fixtures/public/shader/files.bin");

    #[test]
    fn shader_reports_literal_file_envelopes_and_opaque_bodies() {
        let file = parse(SAMPLE).unwrap();
        let shader = &file.shaders[0];
        assert_eq!(shader.span, Span::new(0, 216));
        assert_eq!(
            shader.files,
            [
                FileChunk {
                    span: Span::new(80, 23),
                    body: Span::new(96, 7),
                    padded_size: 32
                },
                FileChunk {
                    span: Span::new(112, 29),
                    body: Span::new(128, 13),
                    padded_size: 0
                },
            ]
        );
        assert_eq!(
            shader.opaque,
            [
                Span::new(13, 1),
                Span::new(20, 28),
                Span::new(52, 4),
                Span::new(60, 20),
                Span::new(84, 4),
                Span::new(96, 7),
                Span::new(103, 9),
                Span::new(116, 4),
                Span::new(128, 13),
                Span::new(141, 3),
                Span::new(144, 24),
                Span::new(168, 48),
            ]
        );
        assert!(file.opaque.is_empty());
        let mut opaque = SAMPLE.to_vec();
        opaque[96..103].fill(0xff);
        assert_eq!(parse(&opaque).unwrap(), file);
    }

    #[test]
    fn shader_nested_selection_retains_paths_and_absolute_offsets() {
        let file = parse(include_bytes!(
            "../../../tests/fixtures/public/shader/pwib.bin"
        ))
        .unwrap();
        assert_eq!(file.shaders[0].entry_path, [0, 0]);
        assert_eq!(file.shaders[0].span, Span::new(176, 216));
        assert_eq!(file.shaders[0].files[0].body, Span::new(272, 7));
        assert_eq!(file.opaque, [Span::new(0, 176)]);
    }

    #[test]
    fn shader_refuses_missing_files_aliases_and_escaping_framing() {
        for (data, kind, offset) in [
            (
                include_bytes!("../../../tests/fixtures/public/shader/no-file.bin").as_slice(),
                ErrorKind::UnexpectedChunk,
                48,
            ),
            (
                include_bytes!("../../../tests/fixtures/public/shader/selected-alias.bin")
                    .as_slice(),
                ErrorKind::AmbiguousPayloadSpan,
                88,
            ),
            (
                include_bytes!("../../../tests/fixtures/public/shader/child-size.bin").as_slice(),
                ErrorKind::ChunkLengthOutOfRange,
                88,
            ),
            (
                include_bytes!("../../../tests/fixtures/public/shader/child-padding.bin")
                    .as_slice(),
                ErrorKind::ChunkLengthOutOfRange,
                92,
            ),
            (
                include_bytes!("../../../tests/fixtures/public/shader/pwib-boundary.bin")
                    .as_slice(),
                ErrorKind::DeclaredSizeOutOfRange,
                4,
            ),
            (
                include_bytes!("../../../tests/fixtures/public/shader/nested-child-size.bin")
                    .as_slice(),
                ErrorKind::ChunkLengthOutOfRange,
                248,
            ),
        ] {
            let failure = parse(data).unwrap_err();
            assert_eq!((failure.kind(), failure.offset()), (kind, offset));
        }
    }

    #[test]
    fn shader_truncated_prefixes_return_typed_errors() {
        for end in 0..SAMPLE.len() {
            let failure = parse(&SAMPLE[..end]).unwrap_err();
            assert!(matches!(
                failure.kind(),
                ErrorKind::UnexpectedEndOfInput | ErrorKind::DeclaredSizeOutOfRange
            ));
        }
    }
}
