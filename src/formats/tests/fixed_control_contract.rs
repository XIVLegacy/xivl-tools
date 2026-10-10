use xivl_formats::error::ErrorKind;
use xivl_formats::reader::Span;
use xivl_formats::richstring::fixed::decode;

// Independent literal fixtures follow AT-COMP-007's authenticated numeric codec.
#[test]
fn source_derived_examples_decode_without_assigning_semantic_ids() {
    for (bytes, fields) in [
        (&[0x02, 0x2E, 0x03, 0x01, 0x01, 0x03][..], [0, 0]),
        (&[0x02, 0x2E, 0x04, 0x02, 0xF0, 0xCF, 0x03], [1, 207]),
        (&[0x02, 0x2E, 0x04, 0x02, 0xF1, 0x01, 0x03], [1, 256]),
        (&[0x02, 0x2E, 0x03, 0xCF, 0xD7, 0x03], [206, 214]),
    ] {
        let control = decode(bytes, 0).unwrap();
        assert_eq!(control.numeric_fields, Some(fields));
        assert_eq!(control.selector, 0x2E);
        assert_eq!(control.span, Span::new(0, bytes.len() as u64));
        assert_eq!(control.raw, bytes);
    }
}

#[test]
fn every_presence_mask_has_an_independently_specified_value() {
    for (numeric, expected) in [
        (&[0xF0, 0xCF][..], 0xCF),
        (&[0xF1, 0x12], 0x1200),
        (&[0xF2, 0x12, 0x34], 0x1234),
        (&[0xF3, 0x12], 0x120000),
        (&[0xF4, 0x12, 0x34], 0x120034),
        (&[0xF5, 0x12, 0x34], 0x123400),
        (&[0xF6, 0x12, 0x34, 0x56], 0x123456),
        (&[0xF7, 0x12], 0x12000000),
        (&[0xF8, 0x12, 0x34], 0x12000034),
        (&[0xF9, 0x12, 0x34], 0x12003400),
        (&[0xFA, 0x12, 0x34, 0x56], 0x12003456),
        (&[0xFB, 0x12, 0x34], 0x12340000),
        (&[0xFC, 0x12, 0x34, 0x56], 0x12340056),
        (&[0xFD, 0x12, 0x34, 0x56], 0x12345600),
        (&[0xFE, 0xFF, 0xFF, 0xFF, 0xFF], u32::MAX),
    ] {
        // Only framing is assembled here; neither the numeric bytes nor expected value is encoded.
        for first in [true, false] {
            let mut bytes = vec![0x02, 0x2E, (numeric.len() + 2) as u8];
            if !first {
                bytes.push(0x01);
            }
            bytes.extend_from_slice(numeric);
            if first {
                bytes.push(0x01);
            }
            bytes.push(0x03);
            let fields = if first { [expected, 0] } else { [0, expected] };
            assert_eq!(decode(&bytes, 0).unwrap().numeric_fields, Some(fields));
        }
    }
}

#[test]
fn extended_length_and_offsets_preserve_only_the_control_span() {
    let bytes = [0x41, 0x02, 0x2E, 0xF0, 0x02, 0x01, 0x01, 0x03, 0xFF];
    let control = decode(&bytes, 1).unwrap();
    assert_eq!(control.span, Span::new(1, 7));
    assert_eq!(control.raw, &bytes[1..8]);
    assert_eq!(control.numeric_fields, Some([0, 0]));
    let error = decode(&[0x02, 0x2E, 0xFE, 0xFF, 0xFF, 0xFF, 0xFF], 0).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::UnexpectedEndOfInput);
    assert_eq!(error.offset(), 7);
}

#[test]
fn malformed_numeric_fields_and_lengths_report_exact_offsets() {
    for lead in [0x00, 0xD8, 0xEF, 0xFF] {
        for bytes in [
            vec![0x02, 0x2E, lead, 0x01, 0x01, 0x03],
            vec![0x02, 0x2E, 0x03, lead, 0x01, 0x03],
            vec![0x02, 0x2E, 0x03, 0x01, lead, 0x03],
        ] {
            let wanted = bytes.iter().position(|byte| *byte == lead).unwrap();
            let error = decode(&bytes, 0).unwrap_err();
            assert_eq!(error.kind(), ErrorKind::MalformedRichStringToken);
            assert_eq!(error.offset(), wanted as u64);
        }
    }
    for (bytes, kind, offset) in [
        (&[0x00][..], ErrorKind::MalformedRichStringToken, 0),
        (
            &[0x02, 0x2E, 0x03, 0x01, 0x01, 0x04],
            ErrorKind::MalformedRichStringToken,
            5,
        ),
        (
            &[0x02, 0x2E, 0x04, 0x01, 0x01, 0x01, 0x03],
            ErrorKind::MalformedRichStringToken,
            5,
        ),
        (
            &[0x02, 0x2E, 0x04, 0xF0, 0x00, 0x01, 0x03],
            ErrorKind::MalformedRichStringToken,
            4,
        ),
        (
            &[0x02, 0x2E, 0x02, 0x01, 0x03],
            ErrorKind::UnexpectedEndOfInput,
            4,
        ),
        (
            &[0x02, 0x2E, 0x03, 0x01, 0xF1, 0x03, 0x01],
            ErrorKind::UnexpectedEndOfInput,
            5,
        ),
    ] {
        let error = decode(bytes, 0).unwrap_err();
        assert_eq!((error.kind(), error.offset()), (kind, offset));
    }
}

#[test]
fn truncated_controls_never_borrow_bytes_outside_the_slice() {
    let bytes = [
        0x02, 0x2E, 0x0B, 0xFE, 0x12, 0x34, 0x56, 0x78, 0xFE, 0x87, 0x65, 0x43, 0x21, 0x03,
    ];
    for end in 0..bytes.len() {
        let error = decode(&bytes[..end], 0).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::UnexpectedEndOfInput);
        assert!(error.offset() <= end as u64);
    }
    assert_eq!(
        decode(&bytes, 0).unwrap().numeric_fields,
        Some([0x12345678, 0x87654321])
    );
    assert_eq!(
        decode(&bytes, usize::MAX).unwrap_err().offset(),
        usize::MAX as u64
    );
}

#[test]
fn other_selectors_remain_opaque_even_when_their_length_is_invalid() {
    for selector in [0x10, 0x2D, 0x37, 0xFF] {
        let bytes = [0x41, 0x02, selector, 0x00, 0x02, 0x2E, 0xFF];
        let control = decode(&bytes, 1).unwrap();
        assert_eq!(control.numeric_fields, None);
        assert_eq!(control.selector, selector);
        assert_eq!(control.raw, &bytes[1..]);
        assert_eq!(control.span, Span::new(1, 6));
    }
}
