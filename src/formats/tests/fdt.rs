use xivl_formats::fdt::{self, GlyphChecks, GlyphLayout, GlyphRecord, Texture};
use xivl_formats::Span;

const V4: &[u8] = include_bytes!("../../../tests/fixtures/public/fdt/v4.bin");
const NO_VERSION: &[u8] = include_bytes!("../../../tests/fixtures/public/fdt/no-version.bin");

#[test]
fn versioned_records_decode_their_consumed_fields() {
    let parsed = fdt::parse(V4).unwrap();
    assert_eq!(parsed.layout, GlyphLayout::Versioned);
    assert_eq!(parsed.texture_offset, 1);
    assert_eq!(
        parsed.textures,
        vec![
            Texture {
                chunk: 1,
                dimensions: None
            },
            Texture {
                chunk: 2,
                dimensions: Some((32, 16))
            },
            Texture {
                chunk: 3,
                dimensions: Some((16, 8))
            },
        ]
    );
    assert_eq!(
        parsed.glyphs[2],
        GlyphRecord {
            span: Span::new(1476, 14),
            resource_index: 1,
            selector: 3,
            offset_x: 1,
            offset_y: -1,
            width: 7,
            height: 9,
            advance: 6,
            byte_07: 0,
            flag_02_advance_addend: Some(1),
            byte_09: Some(0),
            source_x: 20,
            source_y: 7,
            unread: None,
        }
    );
    assert_eq!(
        parsed.checks,
        GlyphChecks {
            source_unchecked: 1,
            ..GlyphChecks::default()
        }
    );
}

#[test]
fn keys_reach_records_through_both_tables() {
    let parsed = fdt::parse(V4).unwrap();
    assert_eq!(parsed.glyph(0x0041), Some(&parsed.glyphs[0]));
    assert_eq!(parsed.glyph(0x0042), Some(&parsed.glyphs[2]));
    assert_eq!(parsed.glyph(0x3000), Some(&parsed.glyphs[2]));
    // Unmapped LTBL word, then unmapped HTBL page.
    assert_eq!(parsed.glyph(0x0043), None);
    assert_eq!(parsed.glyph(0x4100), None);
}

#[test]
fn a_zero_version_moves_the_source_origin_and_leaves_two_bytes_unread() {
    let parsed = fdt::parse(NO_VERSION).unwrap();
    assert_eq!(parsed.layout, GlyphLayout::Unversioned);
    let record = parsed.glyphs[0];
    assert_eq!(record.span, Span::new(1352, 14));
    assert_eq!((record.source_x, record.source_y), (4, 2));
    assert_eq!(record.flag_02_advance_addend, None);
    assert_eq!(record.byte_09, None);
    assert_eq!(record.unread, Some(Span::new(1364, 2)));
    assert_eq!(parsed.checks, GlyphChecks::default());
}

#[test]
fn embedded_gtex_payloads_parse_standalone_with_errors_rebased_onto_the_font() {
    let data = include_bytes!("../../../tests/fixtures/public/fdt/gtex-payloads.bin");
    let parsed = fdt::parse(data).unwrap();
    let payloads = parsed.gtex_payloads();
    // Texture 0 is DDSF, which has no GTEX payload.
    assert_eq!(
        payloads
            .iter()
            .map(|payload| payload.texture)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
    let (bytes, gtex) = fdt::parse_gtex_payload(data, payloads[0]).unwrap();
    assert_eq!(bytes.len() as u64, payloads[0].payload.length);
    assert_eq!(gtex.surfaces.len(), 2);
    let standalone = fdt::parse_gtex_payload(data, payloads[1]).unwrap_err();
    let relative = xivl_formats::gtex_pwib::parse(
        &data[payloads[1].payload.offset as usize..payloads[1].payload.end() as usize],
        xivl_formats::gtex_pwib::TaggedResourceKind::Gtex,
    )
    .unwrap_err();
    assert_eq!(standalone.kind(), relative.kind());
    assert_eq!(
        standalone.offset(),
        payloads[1].payload.offset + relative.offset()
    );
}
