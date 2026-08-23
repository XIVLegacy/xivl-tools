//! No-panic sweep over mutated public fixtures.
//!
//! The parser contract requires no panics on malformed input. Asserting the
//! absence of a crash on hand-written inputs alone would not hold that line,
//! so this walks every truncation and a deterministic set of byte mutations
//! of every committed fixture and requires each one to either parse or return
//! a typed error with an in-range offset.

use std::path::{Path, PathBuf};

use xivl_formats::config::{self, ConfigKind};
use xivl_formats::sheet::ColumnType;
use xivl_formats::{
    inspect_bytes, inspect_named_bytes_as, richstring, scrambled, sedb, sheet, sqwt, ssd,
    validate_named_bytes_as, ErrorKind, InspectAs,
};

/// Every configuration file this crate reads. The four are not
/// interchangeable, so a mutated fixture is walked through all of them.
const CONFIG_KINDS: [ConfigKind; 4] = [
    ConfigKind::Sys,
    ConfigKind::Pad,
    ConfigKind::Lng,
    ConfigKind::Rgn,
];

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("src/formats sits two levels below the checkout root")
        .join("tests/fixtures/public")
}

fn binary_fixtures() -> Vec<(String, Vec<u8>)> {
    fixtures_under(&["sedb", "res"])
}

fn document_fixtures() -> Vec<(String, Vec<u8>)> {
    let mut fixtures = fixtures_under(&["ssd", "sheet", "scrambled", "sqwt", "config", "lpb"]);
    // The 128-level nesting bomb has its own exact limit assertion. Mutating
    // each of its thousands of bytes would multiply the cross-reader sweep
    // without adding a distinct boundary.
    fixtures.retain(|(name, _)| !name.ends_with("bytecode-nesting-bomb.bin"));
    fixtures
}

/// The part of a fixture path after the last separator. It is the key of
/// a SQEX container, so the sweep has to carry it rather than invent one.
fn base_name(path: &str) -> &str {
    match path.rfind(['/', '\\']) {
        Some(index) => &path[index + 1..],
        None => path,
    }
}

fn fixtures_under(directories: &[&str]) -> Vec<(String, Vec<u8>)> {
    let mut fixtures = Vec::new();
    for directory in directories {
        let path = fixture_root().join(directory);
        let mut names: Vec<PathBuf> = std::fs::read_dir(&path)
            .unwrap_or_else(|error| panic!("cannot list {}: {error}", path.display()))
            .filter_map(std::result::Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|value| value == "bin"))
            .collect();
        names.sort();
        for name in names {
            let bytes = std::fs::read(&name).expect("a committed fixture is readable");
            fixtures.push((name.display().to_string(), bytes));
        }
    }
    assert!(!fixtures.is_empty(), "no binary fixtures were found");
    fixtures
}

/// Parse and check the invariants that must hold whatever the input is.
fn parse_and_check(data: &[u8]) {
    match sedb::parse_container(data, 0) {
        Ok(container) => {
            assert!(
                container.entries_tile_the_payload(),
                "payload entries do not tile the container"
            );
            assert!(container.total_size as usize <= data.len());
            for entry in &container.entries {
                assert!(entry.span.end() <= data.len() as u64);
            }
        }
        Err(error) => {
            assert!(
                error.offset() <= data.len() as u64,
                "error offset {} is past the {} byte input",
                error.offset(),
                data.len()
            );
        }
    }
    // The document builder must survive whatever the parser produced.
    if let Ok(document) = inspect_bytes(data) {
        let text = xivl_formats::to_canonical_json(&document);
        assert!(text.is_ascii());
        assert!(text.ends_with('\n'));
    }
}

#[test]
fn every_truncation_of_every_fixture_is_handled() {
    for (name, bytes) in binary_fixtures() {
        for length in 0..=bytes.len() {
            std::panic::catch_unwind(|| parse_and_check(&bytes[..length]))
                .unwrap_or_else(|_| panic!("{name} truncated to {length} byte(s) panicked"));
        }
    }
}

#[test]
fn deterministic_byte_mutations_are_handled() {
    // A fixed sweep rather than a random one: the same inputs on every
    // machine and every run, so a failure is always reproducible.
    for (name, bytes) in binary_fixtures() {
        for position in 0..bytes.len() {
            for value in [0x00u8, 0x01, 0x7F, 0x80, 0xFE, 0xFF] {
                let mut mutated = bytes.clone();
                mutated[position] = value;
                std::panic::catch_unwind(|| parse_and_check(&mutated)).unwrap_or_else(|_| {
                    panic!("{name} with byte {position} set to {value:#04x} panicked")
                });
            }
        }
    }
}

/// Every reading of a document, sheet-stack, or configuration input, plus
/// the invariants that must hold whatever the bytes are: an error offset
/// inside the input, a losslessly re-encodable rich string, a configuration
/// file that writes back to itself, and an ASCII report.
fn read_every_way(data: &[u8], name: &str) {
    let readings = [
        InspectAs::Auto,
        InspectAs::Ssd,
        InspectAs::EnableFile,
        InspectAs::RowOffsets,
        InspectAs::SheetData(Vec::new()),
        InspectAs::ScrambledXml,
        InspectAs::Sqwt,
        InspectAs::Lpb,
        InspectAs::LpbBytecode,
        InspectAs::SheetData(vec![
            ColumnType::Text,
            ColumnType::Signed32,
            ColumnType::Boolean,
            ColumnType::Float32,
            ColumnType::Unsigned8,
        ]),
        // The five widths the scrambled documents added, so a mutation is
        // walked through them too.
        InspectAs::SheetData(vec![
            ColumnType::Text,
            ColumnType::Signed8,
            ColumnType::Signed16,
            ColumnType::Unsigned16,
            ColumnType::Half16,
            ColumnType::Unsigned32,
        ]),
        InspectAs::Config(ConfigKind::Sys),
        InspectAs::Config(ConfigKind::Pad),
        InspectAs::Config(ConfigKind::Lng),
        InspectAs::Config(ConfigKind::Rgn),
    ];
    for reading in &readings {
        for document in [
            inspect_named_bytes_as(data, name, reading),
            validate_named_bytes_as(data, name, reading),
        ] {
            match document {
                Ok(document) => {
                    let text = xivl_formats::to_canonical_json(&document);
                    assert!(text.is_ascii());
                    assert!(text.ends_with('\n'));
                }
                Err(error) => assert!(
                    error.offset() <= data.len() as u64,
                    "error offset {} is past the {} byte input",
                    error.offset(),
                    data.len()
                ),
            }
        }
    }

    // A configuration file that parses must write back to exactly what it
    // came from. That is the write claim, and it is checked on every
    // mutation rather than only on the well-formed fixtures.
    for kind in CONFIG_KINDS {
        if let Ok(parsed) = config::parse(data, kind) {
            assert_eq!(
                parsed.encode(),
                data,
                "{} did not round trip",
                kind.format_id()
            );
        }
    }

    // A rich string that parses must re-encode to exactly what it came
    // from. That is what "lossless" means here, and it is checked on every
    // mutation rather than only on the well-formed fixtures.
    if let Ok(rich) = richstring::RichString::parse(data, 0) {
        assert_eq!(rich.encode(), data, "a rich string did not re-encode");
    }
    if let Ok(strings) = sheet::parse_string_stream(data) {
        for string in &strings {
            assert!(string.span.end() <= data.len() as u64);
        }
    }
    // A decode that succeeds must land inside the input and leave the
    // trailer out of the document, whatever the mutation did to the bytes.
    if let Ok(decoded) = scrambled::decode(data) {
        assert_eq!(decoded.document.len(), data.len() - 1);
        assert_eq!(decoded.trailer.end(), data.len() as u64);
        assert_eq!(decoded.encoded.length, decoded.document.len() as u64);
    }
    // A SQEX decode that succeeds must tile the input and survive a round
    // trip: whatever the mutation did, the container loses nothing.
    if let Ok(decoded) = sqwt::decode(data, name) {
        assert_eq!(decoded.header.end(), decoded.enciphered.offset);
        assert_eq!(decoded.enciphered.end(), decoded.plaintext_tail.offset);
        assert_eq!(decoded.plaintext_tail.end(), data.len() as u64);
        assert_eq!(
            sqwt::encode(&decoded.document, name).expect("the name is not empty"),
            data,
            "a SQEX container did not re-encode"
        );
    }
    let _ = ssd::parse_document(data);
    let _ = sheet::parse_enable_file(data);
    let _ = sheet::parse_row_offsets(data);
}

#[test]
fn every_truncation_of_every_document_fixture_is_handled() {
    for (name, bytes) in document_fixtures() {
        for length in 0..=bytes.len() {
            let name = base_name(&name);
            std::panic::catch_unwind(|| read_every_way(&bytes[..length], name))
                .unwrap_or_else(|_| panic!("{name} truncated to {length} byte(s) panicked"));
        }
    }
}

#[test]
fn deterministic_document_byte_mutations_are_handled() {
    for (name, bytes) in document_fixtures() {
        for position in 0..bytes.len() {
            for value in [0x00u8, 0x02, 0x03, 0x73, 0x80, 0xF1, 0xFF] {
                let mut mutated = bytes.clone();
                mutated[position] = value;
                let short = base_name(&name);
                std::panic::catch_unwind(|| read_every_way(&mutated, short)).unwrap_or_else(|_| {
                    panic!("{name} with byte {position} set to {value:#04x} panicked")
                });
            }
        }
    }
}

#[test]
fn a_deeply_nested_document_stops_at_the_depth_limit() {
    let depth = 4096usize;
    let mut input = String::with_capacity(depth * 7 + 1);
    for _ in 0..depth {
        input.push_str("<a>");
    }
    input.push('x');
    for _ in 0..depth {
        input.push_str("</a>");
    }
    let error = ssd::parse_document(input.as_bytes())
        .expect_err("nesting past the limit is refused, not followed");
    assert_eq!(error.kind(), ErrorKind::NestingTooDeep);
    assert!(error.offset() <= input.len() as u64);
}

#[test]
fn a_deeply_nested_container_stops_at_the_depth_limit() {
    // Each container wraps the next as its single subresource, so a parser
    // that followed children without a limit would recurse to the stack
    // limit on an input this small.
    let depth = (sedb::MAX_NESTING_DEPTH + 4) as usize;
    let mut image: Vec<u8> = Vec::new();
    for _ in 0..depth {
        let child_size = image.len() as u32;
        let mut container = vec![0u8; 0x50];
        container[0x00..0x04].copy_from_slice(b"SEDB");
        container[0x04..0x08].copy_from_slice(b"RES ");
        container[0x0E..0x10].copy_from_slice(&0x40u16.to_le_bytes());
        container[0x10..0x14].copy_from_slice(&(0x50 + child_size).to_le_bytes());
        container[0x30..0x34].copy_from_slice(&1u32.to_le_bytes());
        container[0x38..0x3C].copy_from_slice(&1u32.to_le_bytes());
        container[0x44..0x48].copy_from_slice(&0u32.to_le_bytes()); // payload offset
        container[0x48..0x4C].copy_from_slice(&child_size.to_le_bytes()); // size
        container.extend_from_slice(&image);
        image = container;
    }
    let container = sedb::parse_container(&image, 0).expect("the outer container parses");
    assert!(container.entries_tile_the_payload());
    // The limit shows up as an anomaly on the deepest container the parser
    // agreed to follow, not as a crash and not as a dropped subresource.
    let text = format!("{container:?}");
    assert!(
        text.contains("nesting-too-deep"),
        "no depth anomaly recorded"
    );
}
