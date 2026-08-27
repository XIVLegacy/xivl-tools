//! Format libraries for Final Fantasy XIV 1.23b client files.
//!
//! Every parser here takes a bounded byte slice and returns a `Result`. A
//! malformed input produces a [`FormatError`] carrying the offset the
//! parser stopped at. It never panics, aborts, or truncates silently.
//! `unsafe` is forbidden at the workspace level, so lifting that ban is a
//! visible change rather than a quiet one.
//!
//! Byte-layout evidence, with its retail citations, is in
//! `docs/format-evidence.md`. This crate documents what it reads, not how
//! the evidence was gathered.

pub mod anomaly;
pub mod blowfish;
pub mod config;
pub mod csv;
pub mod digest;
pub mod error;
pub mod inspect;
pub mod lpb;
pub mod lua51;
pub mod lua_path;
pub mod normalize;
pub mod reader;
pub mod resource;
pub mod richstring;
pub mod scrambled;
pub mod sedb;
pub mod sheet;
pub mod sqwt;
pub mod ssd;
pub mod xml;

pub use anomaly::Anomaly;
pub use config::{ConfigFile, ConfigKind};
pub use csv::{export_sheet_data, parse_row_span, value_text, CsvTable};
pub use error::{ErrorKind, FormatError, Result};
pub use inspect::{
    inspect_bytes, inspect_bytes_as, inspect_named_bytes_as, validate_named_bytes_as, InspectAs,
};
pub use lpb::{extract as extract_lpb, LpbFile, LpbVariant, PreservedBytes};
pub use lua51::{
    parse as parse_lua51, Lua51ArgumentMode, Lua51Chunk, Lua51Header, Lua51Instruction,
    Lua51InstructionMode, Lua51Opcode, Lua51Operand, Lua51Operands, Lua51Prototype, LuaConstant,
};
pub use lua_path::transform as transform_lua_path;
pub use normalize::to_canonical_json;
pub use reader::{Reader, Span};
pub use resource::{parse_dat_path, parse_resource_id, ResourceId};
pub use richstring::{Expression, MacroCode, RichString, Segment, Token};
pub use scrambled::ScrambledXml;
pub use sheet::{ColumnType, ColumnValue, EnableFile, Row, RowOffsets, SheetString};
pub use sqwt::SqwtFile;
pub use ssd::SsdDocument;

/// Map every non-empty, non-comment line of a resource-id listing to its
/// DAT path.
///
/// The listing is the public fixture shape for the `resource-path`
/// operation: one identifier per line, `#` starts a comment, blank lines
/// are ignored. A malformed identifier fails the whole listing with the
/// offset of the offending text in the file.
pub fn resource_path_listing(text: &str) -> Result<serde_json::Value> {
    let mut entries = Vec::new();
    let mut offset: u64 = 0;
    for (number, raw_line) in text.split_inclusive('\n').enumerate() {
        let line = raw_line.trim_end_matches(['\n', '\r']);
        let trimmed = line.trim();
        if !trimmed.is_empty() && !trimmed.starts_with('#') {
            let column = line.find(trimmed).unwrap_or(0) as u64;
            let id = parse_resource_id(trimmed, offset + column)?;
            entries.push(serde_json::json!({
                "line": number as u64 + 1,
                "text": trimmed,
                "id": id.value(),
                "path": id.dat_path(),
            }));
        }
        offset += raw_line.len() as u64;
    }
    Ok(serde_json::json!({
        "schemaVersion": inspect::DOCUMENT_SCHEMA_VERSION,
        "operation": "resource-path",
        "format": "resource-path",
        "input": { "length": text.len() as u64 },
        "entries": entries,
    }))
}

/// Normalized document for one Lua path transform.
pub fn lua_path_document(text: &str) -> Result<serde_json::Value> {
    let transformed = transform_lua_path(text)?;
    Ok(serde_json::json!({
        "schemaVersion": inspect::DOCUMENT_SCHEMA_VERSION,
        "operation": "lua-path",
        "format": "lua-path",
        "input": { "length": text.len() as u64, "path": text },
        "transformed": transformed,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn listing_skips_blanks_and_comments() {
        let value = resource_path_listing("# note\n\n0x29D90001\n").unwrap();
        let entries = value["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["line"], 3);
        assert_eq!(entries[0]["path"], "data/29/D9/00/01.DAT");
    }

    #[test]
    fn listing_reports_the_file_offset_of_a_bad_identifier() {
        let error = resource_path_listing("0x00000000\nnope\n").unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidResourceId);
        assert_eq!(error.offset(), 11);
    }
}
