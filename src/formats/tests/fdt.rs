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

fn unhex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&text[index..index + 2], 16).unwrap())
        .collect()
}

/// Rebuild VERS, the scalars, HTBL, LTBL, and GLYP from glyph metadata alone.
fn assert_glyph_metadata_rebuilds_tables(data: &[u8]) {
    let parsed = fdt::parse(data).unwrap();
    let glyphs = xivl_formats::export_fdt_glyphs(data).unwrap();
    let payload = |tag: [u8; 4]| {
        let chunk = parsed.chunks.iter().find(|chunk| chunk.tag == tag).unwrap();
        &data[chunk.payload.offset as usize..chunk.payload.end() as usize]
    };
    assert_eq!(glyphs["version"]["value"], parsed.version);
    for scalar in glyphs["scalars"].as_array().unwrap() {
        let chunk = parsed.chunks[scalar["chunkIndex"].as_u64().unwrap() as usize];
        let value = scalar["value"].as_u64().unwrap() as u32;
        assert_eq!(
            &data[chunk.payload.offset as usize..chunk.payload.end() as usize],
            value.to_le_bytes()
        );
    }
    let mut pages = vec![0xFF_u8; 256];
    for page in glyphs["pages"].as_array().unwrap() {
        pages[page["highByte"].as_u64().unwrap() as usize] = page["page"].as_u64().unwrap() as u8;
    }
    assert_eq!(pages, payload(fdt::HTBL));
    let page_count = glyphs["indexPageCount"].as_u64().unwrap() as usize;
    let mut index = vec![0xFFFF_u16; page_count * 256];
    let mut records = Vec::new();
    for record in glyphs["records"].as_array().unwrap() {
        records.extend(unhex(record["bytes"].as_str().unwrap()));
        for key in record["keys"].as_array().unwrap() {
            let key = key.as_u64().unwrap() as usize;
            index[usize::from(pages[key >> 8]) * 256 + (key & 0xFF)] =
                record["index"].as_u64().unwrap() as u16;
        }
    }
    let mut index_bytes: Vec<u8> = index.iter().flat_map(|word| word.to_le_bytes()).collect();
    for page in glyphs["unreachableIndexPages"].as_array().unwrap() {
        let start = page["page"].as_u64().unwrap() as usize * 512;
        index_bytes[start..start + 512].copy_from_slice(&unhex(page["bytes"].as_str().unwrap()));
    }
    assert_eq!(index_bytes, payload(fdt::LTBL));
    assert_eq!(records, payload(fdt::GLYP));
}

#[test]
fn glyph_metadata_rebuilds_the_lookup_tables_and_records() {
    for data in [
        V4,
        NO_VERSION,
        &include_bytes!("../../../tests/fixtures/public/fdt/v1.bin")[..],
        &include_bytes!("../../../tests/fixtures/public/fdt/unreachable-page.bin")[..],
    ] {
        assert_glyph_metadata_rebuilds_tables(data);
    }
}

#[test]
fn glyph_metadata_records_decoded_fields_and_the_keys_that_reach_them() {
    let glyphs = xivl_formats::export_fdt_glyphs(V4).unwrap();
    let record = &glyphs["records"][2];
    assert_eq!(record["keys"], serde_json::json!([0x0042, 0x3000]));
    assert_eq!(record["bytes"], "010301ff07090600010014000700");
    assert_eq!(
        (record["resourceIndex"].as_i64(), record["offsetY"].as_i64()),
        (Some(1), Some(-1))
    );
    assert_eq!(record["flag02AdvanceAddend"], 1);
    assert_eq!(
        (record["sourceX"].as_u64(), record["sourceY"].as_u64()),
        (Some(20), Some(7))
    );
    assert_eq!(glyphs["records"][1]["keys"], serde_json::json!([]));
    assert_eq!(
        glyphs["textures"][0]["gtexDimensions"],
        serde_json::Value::Null
    );

    let unversioned = xivl_formats::export_fdt_glyphs(NO_VERSION).unwrap();
    let record = &unversioned["records"][0];
    assert_eq!(unversioned["layout"], "unversioned");
    assert_eq!(record["flag02AdvanceAddend"], serde_json::Value::Null);
    assert_eq!(record["unread"], "a55a");

    let unreachable = xivl_formats::export_fdt_glyphs(include_bytes!(
        "../../../tests/fixtures/public/fdt/unreachable-page.bin"
    ))
    .unwrap();
    assert_eq!(unreachable["unreachableIndexPages"][0]["page"], 2);
    assert_eq!(unreachable["records"][1]["keys"], serde_json::json!([]));
}
