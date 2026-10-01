use xivl_formats::richstring::LengthEncoding;
use xivl_formats::{export_sheet_data, ErrorKind, Expression, InspectAs, RichString};

const VOCABULARY: &[(u8, &str)] = &[
    (0x07, "set-time"),
    (0x08, "if"),
    (0x09, "switch"),
    (0x10, "newline"),
    (0x11, "wait"),
    (0x12, "icon"),
    (0x13, "color"),
    (0x14, "edge-color"),
    (0x16, "soft-hyphen"),
    (0x19, "bold"),
    (0x1a, "italic"),
    (0x1d, "non-breaking-space"),
    (0x1f, "hyphen"),
    (0x20, "number"),
    (0x22, "kilo"),
    (0x24, "seconds"),
    (0x25, "time"),
    (0x28, "sheet"),
    (0x29, "string"),
    (0x2b, "head"),
    (0x2c, "split"),
    (0x2d, "head-all"),
    (0x2f, "lower"),
    (0x31, "english-noun"),
    (0x32, "german-noun"),
    (0x33, "french-noun"),
];

const ALL_MARKERS: &str = concat!(
    "[@set-time:0207020103][@if:0208020103][@switch:0209020103]",
    "[@newline:0210020103][@wait:0211020103][@icon:0212020103]",
    "[@color:0213020103][@edge-color:0214020103][@soft-hyphen:0216020103]",
    "[@bold:0219020103][@italic:021a020103][@non-breaking-space:021d020103]",
    "[@hyphen:021f020103][@number:0220020103][@kilo:0222020103]",
    "[@seconds:0224020103][@time:0225020103][@sheet:0228020103]",
    "[@string:0229020103][@head:022b020103][@split:022c020103]",
    "[@head-all:022d020103][@lower:022f020103][@english-noun:0231020103]",
    "[@german-noun:0232020103][@french-noun:0233020103]",
);

fn frame(payload: &[u8]) -> Vec<u8> {
    let mut raw = vec![2, 8];
    if payload.len() <= 238 {
        raw.push(payload.len() as u8 + 1);
    } else {
        raw.push(0xf2);
        raw.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    }
    raw.extend_from_slice(payload);
    raw.push(3);
    raw
}

fn expressions(payload: &[u8]) -> Result<Vec<Expression>, usize> {
    let rich = RichString::parse(&frame(payload), 317).unwrap();
    let result = rich.tokens().next().unwrap().expressions();
    result
}

fn integer(value: u32, raw: &[u8]) -> Expression {
    Expression::Integer {
        value,
        raw: raw.to_vec(),
    }
}

#[test]
fn all_macro_names_and_literal_markers_are_public_contracts() {
    let mut bytes = Vec::new();
    for &(code, _) in VOCABULARY {
        bytes.extend_from_slice(&[2, code, 2, 1, 3]);
    }
    let rich = RichString::parse(&bytes, 97).unwrap();
    assert_eq!(rich.encode(), bytes);
    assert_eq!(rich.to_lossless_text(), ALL_MARKERS);
    for (index, (token, &(code, name))) in rich.tokens().zip(VOCABULARY).enumerate() {
        assert_eq!(token.code, code);
        assert_eq!(token.macro_code().name(), name);
        assert_eq!(token.span.offset, 97 + index as u64 * 5);
        assert_eq!(token.raw_bytes(), [2, code, 2, 1, 3]);
        assert_eq!(token.expressions(), Ok(vec![integer(0, &[1])]));
    }
    for code in 0..=u8::MAX {
        let raw = [2, code, 2, 1, 3];
        let rich = RichString::parse(&raw, 0).unwrap();
        let token = rich.tokens().next().unwrap();
        let expected_name = VOCABULARY
            .iter()
            .find(|item| item.0 == code)
            .map_or("unknown", |item| item.1);
        assert_eq!(token.macro_code().name(), expected_name);
        assert_eq!(token.raw_bytes(), raw);
        assert_eq!(rich.encode(), raw);
    }
}

#[test]
fn compact_integers_and_all_packed_integer_masks_have_literal_values() {
    for value in 0..=206u32 {
        let raw = [value as u8 + 1];
        assert_eq!(expressions(&raw), Ok(vec![integer(value, &raw)]));
    }
    let vectors: &[(&[u8], u32)] = &[
        (&[0xf0, 0x7f], 0x0000007f),
        (&[0xf1, 0x12], 0x00001200),
        (&[0xf2, 0x12, 0x34], 0x00001234),
        (&[0xf3, 0x56], 0x00560000),
        (&[0xf4, 0x56, 0x78], 0x00560078),
        (&[0xf5, 0x56, 0x78], 0x00567800),
        (&[0xf6, 0x56, 0x78, 0x9a], 0x0056789a),
        (&[0xf7, 0xab], 0xab000000),
        (&[0xf8, 0xab, 0xcd], 0xab0000cd),
        (&[0xf9, 0xab, 0xcd], 0xab00cd00),
        (&[0xfa, 0xab, 0xcd, 0xef], 0xab00cdef),
        (&[0xfb, 0xab, 0xcd], 0xabcd0000),
        (&[0xfc, 0xab, 0xcd, 0xef], 0xabcd00ef),
        (&[0xfd, 0xab, 0xcd, 0xef], 0xabcdef00),
        (&[0xfe, 0xab, 0xcd, 0xef, 0x12], 0xabcdef12),
    ];
    for &(raw, value) in vectors {
        assert_eq!(expressions(raw), Ok(vec![integer(value, raw)]));
    }
}

#[test]
fn every_placeholder_and_prefix_operator_retains_its_tree() {
    for lead in (0xd0..=0xdf).chain([0xec]) {
        assert_eq!(
            expressions(&[lead]),
            Ok(vec![Expression::Placeholder(lead)])
        );
    }
    for code in 0xe8..=0xeb {
        assert_eq!(
            expressions(&[code, 0xd2]),
            Ok(vec![Expression::Unary {
                code,
                operand: Box::new(Expression::Placeholder(0xd2)),
            }])
        );
    }
    for code in 0xe0..=0xe5 {
        assert_eq!(
            expressions(&[code, 0xe8, 1, 2]),
            Ok(vec![Expression::Binary {
                code,
                left: Box::new(Expression::Unary {
                    code: 0xe8,
                    operand: Box::new(integer(0, &[1]))
                }),
                right: Box::new(integer(1, &[2])),
            }])
        );
    }
}

#[test]
fn strings_use_expression_integer_lengths_and_keep_nested_raw_tokens() {
    for (payload, raw_length, body) in [
        (&b"\xff\x01"[..], &b"\x01"[..], &b""[..]),
        (&b"\xff\x04A\\["[..], &b"\x04"[..], &b"A\\["[..]),
        (&b"\xff\xf0\x03A\\["[..], &b"\xf0\x03"[..], &b"A\\["[..]),
        (
            &b"\xff\x05\x02\x10\x01\x03"[..],
            &b"\x05"[..],
            &b"\x02\x10\x01\x03"[..],
        ),
    ] {
        let decoded = expressions(payload).unwrap();
        let [Expression::String {
            raw_length: actual_length,
            value,
        }] = decoded.as_slice()
        else {
            panic!("expected one string expression");
        };
        assert_eq!(actual_length, raw_length);
        assert_eq!(value.encode(), body);
        for token in value.tokens() {
            assert_eq!(token.expressions(), Ok(Vec::new()));
            assert_eq!(token.raw_bytes(), [2, 0x10, 1, 3]);
        }
    }
}

#[test]
fn malformed_expressions_fail_at_their_top_level_payload_start() {
    let invalid: &[&[u8]] = &[
        &[0],
        &[0xe6],
        &[0xe7],
        &[0xed],
        &[0xee],
        &[0xef],
        &[0xe8],
        &[0xe9, 0],
        &[0xea, 0xf0],
        &[0xeb, 0xff],
        &[0xe0],
        &[0xe1, 1],
        &[0xe2, 1, 0],
        &[0xe3, 0, 1],
        &[0xe4, 1, 0xf0],
        &[0xe5, 0xe8, 1],
        &[0xff],
        &[0xff, 0],
        &[0xff, 4, b'a'],
        &[0xff, 2, 0xff],
        &[0xff, 2, 2],
        &[0xff, 0xfe, 0xff, 0xff, 0xff, 0xff],
    ];
    for &bad in invalid {
        assert_eq!(expressions(bad), Err(0), "{bad:02x?}");
        let mut after_two = vec![1, 0xd0];
        after_two.extend_from_slice(bad);
        let raw = frame(&after_two);
        let rich = RichString::parse(&raw, 200).unwrap();
        assert_eq!(
            rich.tokens().next().unwrap().expressions(),
            Err(2),
            "{bad:02x?}"
        );
        assert_eq!(rich.encode(), raw);
    }
    let packed: &[&[u8]] = &[
        &[0xf0, 1],
        &[0xf1, 1],
        &[0xf2, 1, 1],
        &[0xf3, 1],
        &[0xf4, 1, 1],
        &[0xf5, 1, 1],
        &[0xf6, 1, 1, 1],
        &[0xf7, 1],
        &[0xf8, 1, 1],
        &[0xf9, 1, 1],
        &[0xfa, 1, 1, 1],
        &[0xfb, 1, 1],
        &[0xfc, 1, 1, 1],
        &[0xfd, 1, 1, 1],
        &[0xfe, 1, 1, 1, 1],
    ];
    for &valid in packed {
        for length in 1..valid.len() {
            assert_eq!(expressions(&valid[..length]), Err(0));
        }
        for index in 1..valid.len() {
            let mut zero_lane = valid.to_vec();
            zero_lane[index] = 0;
            assert_eq!(expressions(&zero_lane), Err(0));
        }
    }
}

#[test]
fn framing_forms_preserve_even_noncanonical_length_bytes() {
    let empty_forms = RichString::parse(
        b"\x02\x10\x01\x03\x02\x10\xf0\x00\x03\x02\x10\xf1\x00\x03\x02\x10\xf2\x00\x00\x03",
        0,
    )
    .unwrap();
    assert_eq!(
        empty_forms.to_lossless_text(),
        concat!(
            "[@newline:02100103][@newline:0210f00003]",
            "[@newline:0210f10003][@newline:0210f2000003]",
        )
    );
    for (length_bytes, length, encoding) in [
        (vec![1], 0, LengthEncoding::Direct),
        (vec![0xef], 238, LengthEncoding::Direct),
        (vec![0xf0, 0], 0, LengthEncoding::Byte),
        (vec![0xf0, 0xff], 255, LengthEncoding::Byte),
        (vec![0xf1, 0], 0, LengthEncoding::ByteScaled),
        (vec![0xf1, 1], 256, LengthEncoding::ByteScaled),
        (vec![0xf1, 0xff], 65280, LengthEncoding::ByteScaled),
        (vec![0xf2, 0, 0], 0, LengthEncoding::Word),
        (vec![0xf2, 1, 0], 256, LengthEncoding::Word),
        (vec![0xf2, 0xff, 0xff], 65535, LengthEncoding::Word),
    ] {
        let mut raw = vec![2, 0xaa];
        raw.extend_from_slice(&length_bytes);
        raw.extend(std::iter::repeat_n(b'A', length));
        raw.push(3);
        let rich = RichString::parse(&raw, 123).unwrap();
        let token = rich.tokens().next().unwrap();
        assert_eq!(token.encoding, encoding);
        assert_eq!(token.length_bytes, length_bytes);
        assert_eq!(token.payload.len(), length);
        assert_eq!(token.span.offset, 123);
        assert_eq!(token.span.length, raw.len() as u64);
        assert_eq!(rich.encode(), raw);
    }
}

#[test]
fn malformed_frames_have_absolute_failure_offsets() {
    let vectors: &[(&[u8], u64, ErrorKind)] = &[
        (&[2], 101, ErrorKind::MalformedRichStringToken),
        (&[2, 0x10], 102, ErrorKind::MalformedRichStringToken),
        (&[2, 0x10, 0], 102, ErrorKind::MalformedRichStringToken),
        (&[2, 0x10, 0xf0], 102, ErrorKind::MalformedRichStringToken),
        (&[2, 0x10, 0xf1], 102, ErrorKind::MalformedRichStringToken),
        (
            &[2, 0x10, 0xf2, 1],
            102,
            ErrorKind::MalformedRichStringToken,
        ),
        (
            &[2, 0x10, 3, b'a'],
            103,
            ErrorKind::MalformedRichStringToken,
        ),
        (
            &[2, 0x10, 2, b'a'],
            104,
            ErrorKind::MalformedRichStringToken,
        ),
        (&[2, 0x10, 1, 4], 103, ErrorKind::MalformedRichStringToken),
        (&[b'a', 0xc3, b'('], 101, ErrorKind::InvalidUtf8),
    ];
    for &(raw, offset, kind) in vectors {
        let error = RichString::parse(raw, 100).unwrap_err();
        assert_eq!(error.kind(), kind);
        assert_eq!(error.offset(), offset);
    }
    for lead in 0xf3..=0xff {
        let error = RichString::parse(&[2, 0x10, lead, 3], 100).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::MalformedRichStringToken);
        assert_eq!(error.offset(), 102);
    }
}

#[test]
fn expression_nesting_is_bounded_without_losing_raw_tokens() {
    let mut accepted = vec![0xe8; 64];
    accepted.push(1);
    assert!(expressions(&accepted).is_ok());
    let mut rejected = vec![0xe8; 65];
    rejected.push(1);
    let raw = frame(&rejected);
    let rich = RichString::parse(&raw, 0).unwrap();
    assert_eq!(rich.tokens().next().unwrap().expressions(), Err(0));
    assert_eq!(rich.encode(), raw);
    let mut binary = vec![1];
    for depth in 1..=65 {
        let mut outer = vec![0xe0];
        outer.extend(binary);
        outer.push(1);
        binary = outer;
        if depth == 64 {
            assert!(expressions(&binary).is_ok());
        }
    }
    assert_eq!(expressions(&binary), Err(0));
    let mut right_nested = vec![1];
    for depth in 1..=65 {
        let mut outer = vec![0xe0, 1];
        outer.extend(right_nested);
        right_nested = outer;
        if depth == 64 {
            assert!(expressions(&right_nested).is_ok());
        }
    }
    assert_eq!(expressions(&right_nested), Err(0));
}

#[test]
fn csv_is_literal_and_lossless_even_when_expression_decoding_fails() {
    let mut body = b"q,\"\\[x]\n\r".to_vec();
    for &(code, _) in VOCABULARY {
        body.extend_from_slice(&[2, code, 2, 1, 3]);
    }
    body.extend_from_slice(b"\x02\xaa\x02\x00\x03");
    body.extend_from_slice("\u{00e9}".as_bytes());
    let rich = RichString::parse(&body, 0).unwrap();
    assert_eq!(rich.tokens().last().unwrap().expressions(), Err(0));
    let expected_text = format!("q,\"\\\\\\[x]\n\r{ALL_MARKERS}[@unknown:02aa020003]\u{00e9}");
    assert_eq!(rich.to_lossless_text(), expected_text);
    assert_eq!(rich.encode(), body);
    let length = u16::try_from(body.len() + 1).unwrap();
    let mut sheet = length.to_le_bytes().to_vec();
    sheet.extend_from_slice(&body);
    sheet.push(0);
    assert_eq!(
        sheet.as_slice(),
        include_bytes!("../../../tests/fixtures/public/rich-string/public-csv.bin")
    );
    let report = export_sheet_data(&sheet, &InspectAs::SheetData(Vec::new())).unwrap();
    let expected_csv = format!(
        " ,0\n ,str\n0,\"q,\"\"\\\\\\[x]\n\r{ALL_MARKERS}[@unknown:02aa020003]\u{00e9}\"\n"
    );
    assert_eq!(report["csv"].as_str().unwrap(), expected_csv);
    let case_output: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/conformance/cases/rich-string-public-csv/expected.json"
    ))
    .unwrap();
    assert_eq!(case_output["csv"].as_str().unwrap(), expected_csv);
}
