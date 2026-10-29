use xivl_formats::region::{self, RegionMembership};
use xivl_formats::{inspect_bytes, inspect_bytes_as, validate_named_bytes_as, InspectAs, Span};

const ROWS: &[u8] = include_bytes!("../../../tests/fixtures/public/region/rows.bin");

#[test]
fn roots_children_duplicates_and_unknown_bytes_are_retained() {
    let parsed = region::parse(ROWS).unwrap();
    assert_eq!(parsed.root_count, 2);
    assert_eq!(parsed.rows.len(), 4);
    assert_eq!(parsed.rows_span, Span::new(64, 192));
    assert_eq!(parsed.trailing, Span::new(256, 0));
    assert_eq!(
        parsed.rows[0].membership,
        RegionMembership::Root {
            index: 0,
            child_count: 2
        }
    );
    for (index, row) in parsed.rows[1..3].iter().enumerate() {
        assert_eq!(
            row.membership,
            RegionMembership::Child {
                root_index: 0,
                index: index as u32
            }
        );
        assert_eq!(row.id, 17);
        assert_eq!(row.dat_key, 0x1020_3040);
        assert_eq!(row.token, Span::new(row.span.offset + 16, 16));
        assert_eq!(
            row.unknown,
            vec![
                Span::new(row.span.offset + 4, 4),
                Span::new(row.span.offset + 12, 4),
                Span::new(row.span.offset + 32, 16)
            ]
        );
    }
    assert_eq!(
        parsed.rows[3].membership,
        RegionMembership::Root {
            index: 1,
            child_count: 0
        }
    );
    let document = inspect_bytes(ROWS).unwrap();
    assert_eq!(
        document,
        inspect_bytes_as(ROWS, &InspectAs::Region).unwrap()
    );
    assert_eq!(document["rows"][1]["sha256"], document["rows"][2]["sha256"]);
    let text = document.to_string();
    assert!(!text.contains("SyntheticChild"));
    assert!(!text.contains("SyntheticRoot"));
    assert!(document["rows"][1]["token"].get("value").is_none());
    assert!(document["rows"][1]["membership"]
        .get("childCount")
        .is_none());
    let validated = validate_named_bytes_as(ROWS, "rows.bin", &InspectAs::Region).unwrap();
    assert_eq!(validated["format"], "region");
    assert_eq!(validated["checks"][0]["status"], "pass");
    assert_eq!(validated["checks"][1]["status"], "not-applicable");
}

#[test]
fn size_word_does_not_invent_a_row_bound_and_trailing_bytes_remain_opaque() {
    let data = include_bytes!("../../../tests/fixtures/public/region/advisory-size.bin");
    let parsed = region::parse(data).unwrap();
    assert_eq!(parsed.declared_size, 1);
    assert_eq!(parsed.rows.len(), 4);
    let data = include_bytes!("../../../tests/fixtures/public/region/trailing.bin");
    let parsed = region::parse(data).unwrap();
    assert_eq!(parsed.trailing, Span::new(256, 3));
    let document = inspect_bytes(data).unwrap();
    assert_eq!(document["endsAtEof"], false);
    assert_eq!(document["trailing"]["span"]["length"], 3);
}

#[test]
fn allocation_helpers_report_names_words_and_auxiliary_keys() {
    let mut data = vec![0u8; 0x40 + 4 * 0x30];
    data[..19].copy_from_slice(region::NAME);
    data[0x18..0x1E].copy_from_slice(region::VERSION);
    let size = data.len() as u32;
    data[0x20..0x24].copy_from_slice(&size.to_le_bytes());
    data[0x24..0x28].copy_from_slice(&2u32.to_le_bytes());
    let row = |data: &mut Vec<u8>, index: usize, words: [u32; 4], name: &[u8], word_20: u32| {
        let start = 0x40 + index * 0x30;
        for (slot, word) in words.iter().enumerate() {
            data[start + slot * 4..start + slot * 4 + 4].copy_from_slice(&word.to_le_bytes());
        }
        data[start + 0x10..start + 0x10 + name.len()].copy_from_slice(name);
        data[start + 0x20..start + 0x24].copy_from_slice(&word_20.to_le_bytes());
    };
    row(&mut data, 0, [7, 1, 2, 2], b"root\0", 0);
    row(&mut data, 1, [70, 0, 0x1234_0005, 9], b"sky", 0);
    row(
        &mut data,
        2,
        [71, 0x3000, 0x5678_0002, 0x102],
        b"abcdefghijklmnopq",
        0,
    );
    row(&mut data, 3, [8, 0, 0, 0], b"sixteen-bytes-xx", 5);
    let parsed = region::parse(&data).unwrap();

    assert_eq!(parsed.rows[2].word_04, 0x3000);
    assert_eq!(parsed.rows[2].word_0c, 0x102);
    assert_eq!(
        region::effective_name(&data, &parsed.rows[0]),
        Some(&b"root"[..])
    );
    assert_eq!(
        region::effective_name(&data, &parsed.rows[2]),
        Some(&b"abcdefghijklmno"[..])
    );
    assert_eq!(region::token_terminated(&data, &parsed.rows[0]), Some(true));
    assert_eq!(
        region::token_terminated(&data, &parsed.rows[3]),
        Some(false)
    );
    assert_eq!(region::effective_name(&[], &parsed.rows[0]), None);
    assert_eq!(
        region::auxiliary_key(&parsed, 0),
        Some(region::AuxiliaryKey::Derived(0x1234_0006))
    );
    assert_eq!(region::auxiliary_key(&parsed, 1), None);
    assert_eq!(
        region::auxiliary_key(&parsed, 3),
        Some(region::AuxiliaryKey::NonzeroLayoutArgument)
    );
}
