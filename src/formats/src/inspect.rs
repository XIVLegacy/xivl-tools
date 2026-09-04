//! `inspect`: a structural report of one input, in the normalized form.
//!
//! The report is a report, not an export: it never carries payload bytes,
//! sheet-row text, or configuration values. The explicit SSD document view
//! preserves sheet names and attribute values because they are part of that
//! format's structural contract. Retail expectations use the redacted
//! scrambled-container view instead. Decoded sheet strings reach a caller
//! through the library types in `sheet` and `richstring`.

use std::collections::BTreeMap;

use serde_json::{json, Map, Value};

use crate::config::{self, ConfigFile, ConfigKind};
use crate::digest::{sha256_hex, sha256_xor_hex};
use crate::error::Result;
use crate::gtex_pwib::{self, TaggedResourceKind};
use crate::lpb;
use crate::lua51::{
    self, Lua51Instruction, Lua51Operand, Lua51Operands, Lua51Prototype, LuaConstant, LuaString,
};
use crate::reader::Span;
use crate::richstring::{payload_hex, RichString, Segment};
use crate::scrambled;
use crate::sedb::{self, Container, Entry, EntryBody};
use crate::sheet::{self, ColumnType, ColumnValue, EnableFile, Row, RowOffsets, SheetString};
use crate::sqwt;
use crate::ssd::{self, SheetBody, SsdDocument};
use crate::staticactor;
use crate::xml;

/// Version of the inspect document shape.
pub const DOCUMENT_SCHEMA_VERSION: u64 = 1;

/// How the caller wants an input read.
///
/// An enable file and a row-offset array are headerless arrays of 32-bit
/// values, so no amount of sniffing tells them apart. The caller names the
/// format for those. SEDB containers and SSD documents carry a signature
/// and are recognized.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum InspectAs {
    #[default]
    Auto,
    Sedb,
    Ssd,
    /// The scrambled container, reported without its document's content:
    /// spans, keys, a digest, and a census of element and attribute names.
    /// `--as ssd` on the same file reads the document itself.
    ScrambledXml,
    /// The SQEX container, whose key is the file's own name.
    Sqwt,
    /// An LPB wrapper around compiled Lua 5.1 bytecode.
    Lpb,
    /// An LPB wrapper plus bounded structure from its Lua 5.1 payload.
    LpbBytecode,
    /// The XOR-0x73 static-actor SAN record table.
    StaticActorSan,
    /// A GTEX-tagged resource with no evidenced post-signature layout.
    Gtex,
    /// A PWIB-tagged resource with no evidenced post-signature layout.
    Pwib,
    EnableFile,
    RowOffsets,
    /// A sheet data file. With no columns it is read as a stream of string
    /// values, which is how an all-string sheet is stored. With columns it
    /// is read as typed rows.
    SheetData(Vec<ColumnType>),
    /// One of the client's configuration files. Which one is the caller's
    /// to say: nothing in the bytes distinguishes them.
    Config(ConfigKind),
}

impl InspectAs {
    /// Names accepted by the `--as` option.
    pub const NAMES: [&'static str; 16] = [
        "sedb",
        "ssd",
        "scrambled-xml",
        "sqwt",
        "lpb",
        "lpb-bytecode",
        "staticactor-san",
        "gtex",
        "pwib",
        "enable-file",
        "row-offsets",
        "sheet-data",
        "config-sys",
        "config-pad",
        "config-lng",
        "config-rgn",
    ];

    /// Parse `inspect` arguments after the input path.
    ///
    /// Shared by the command line and the conformance runner so a case and
    /// a shell invocation cannot drift apart.
    pub fn from_arguments(arguments: &[String]) -> std::result::Result<Self, String> {
        let mut named: Option<String> = None;
        let mut columns: Option<Vec<ColumnType>> = None;
        let mut index = 0;
        while index < arguments.len() {
            let flag = arguments[index].as_str();
            let value = arguments.get(index + 1).cloned();
            match flag {
                "--as" => {
                    named = Some(value.ok_or_else(|| "--as needs a format name".to_string())?);
                    index += 2;
                }
                "--columns" => {
                    let text = value.ok_or_else(|| "--columns needs a column list".to_string())?;
                    columns =
                        Some(sheet::parse_column_list(&text).map_err(|error| error.to_string())?);
                    index += 2;
                }
                other => return Err(format!("unknown option '{other}'")),
            }
        }

        let selected = match named.as_deref() {
            None => Self::Auto,
            Some("sedb") => Self::Sedb,
            Some("ssd") => Self::Ssd,
            Some("scrambled-xml") => Self::ScrambledXml,
            Some("sqwt") => Self::Sqwt,
            Some("lpb") => Self::Lpb,
            Some("lpb-bytecode") => Self::LpbBytecode,
            Some("staticactor-san") => Self::StaticActorSan,
            Some("gtex") => Self::Gtex,
            Some("pwib") => Self::Pwib,
            Some("enable-file") => Self::EnableFile,
            Some("row-offsets") => Self::RowOffsets,
            Some("sheet-data") => Self::SheetData(columns.clone().unwrap_or_default()),
            Some(other) => match ConfigKind::from_name(other) {
                Some(kind) => Self::Config(kind),
                None => {
                    return Err(format!(
                        "unknown format '{other}'; expected one of {}",
                        Self::NAMES.join(", ")
                    ))
                }
            },
        };
        if columns.is_some() && !matches!(selected, Self::SheetData(_)) {
            return Err("--columns applies to --as sheet-data".to_string());
        }
        Ok(selected)
    }
}

/// Inspect an input, recognizing the formats that carry a signature.
pub fn inspect_bytes(data: &[u8]) -> Result<Value> {
    inspect_bytes_as(data, &InspectAs::Auto)
}

/// Inspect an input the way the caller asked for.
///
/// The SQEX container's key is the file's own name, which bytes alone do
/// not carry. Reading one through this entry point fails with
/// `missing-container-name`. [`inspect_named_bytes_as`] is the one that
/// takes a name.
pub fn inspect_bytes_as(data: &[u8], how: &InspectAs) -> Result<Value> {
    inspect_named_bytes_as(data, "", how)
}

/// Inspect an input whose name is part of its reading.
///
/// `name` is the input's base name as the caller knows it. Nothing here
/// derives it from a path: this crate never resolves one.
pub fn inspect_named_bytes_as(data: &[u8], name: &str, how: &InspectAs) -> Result<Value> {
    match how {
        InspectAs::Auto => {
            if let Some(kind) = gtex_pwib::detect(data) {
                inspect_tagged_resource(data, kind)
            } else if staticactor::has_signature(data) {
                inspect_staticactor(data)
            } else if ssd::has_document_signature(data) {
                inspect_ssd(data)
            } else if sqwt::has_signature(data) {
                inspect_sqwt(data, name)
            } else if scrambled::has_signature(data) {
                // Recognizing this needs the whole decode: the trailer byte
                // alone is not a signature, and resources exist that carry
                // it without being documents.
                inspect_scrambled(data)
            } else {
                // Not a document either way, so it is read as a container. A
                // file that is neither fails with bad-magic, which is the
                // honest answer: this crate does not read it.
                inspect_sedb(data)
            }
        }
        InspectAs::Sedb => inspect_sedb(data),
        InspectAs::Ssd => inspect_ssd(data),
        InspectAs::ScrambledXml => inspect_scrambled(data),
        InspectAs::Sqwt => inspect_sqwt(data, name),
        InspectAs::Lpb => inspect_lpb(data),
        InspectAs::LpbBytecode => inspect_lpb_bytecode(data),
        InspectAs::StaticActorSan => inspect_staticactor(data),
        InspectAs::Gtex => inspect_tagged_resource(data, TaggedResourceKind::Gtex),
        InspectAs::Pwib => inspect_tagged_resource(data, TaggedResourceKind::Pwib),
        InspectAs::EnableFile => inspect_enable_file(data),
        InspectAs::RowOffsets => inspect_row_offsets(data),
        InspectAs::SheetData(columns) => inspect_sheet_data(data, columns),
        InspectAs::Config(kind) => inspect_config(data, *kind),
    }
}

/// Read an input and check the invariants its format promises.
///
/// `inspect` answers what an input holds. `validate` answers whether this
/// crate's reading of it is sound. The one check that is not free is the
/// round trip: for a format this crate can write, the model is encoded back
/// and required to reproduce the input byte for byte. That is what a write
/// claim in `data/support-matrix.json` means, so it is a checked property
/// of every case rather than a statement in a document.
pub fn validate_named_bytes_as(data: &[u8], name: &str, how: &InspectAs) -> Result<Value> {
    let format = match how {
        InspectAs::Config(kind) => kind.format_id().to_string(),
        other => {
            let document = inspect_named_bytes_as(data, name, other)?;
            string_field(&document, "format")
        }
    };

    let mut checks = vec![json!({ "name": "parse", "status": "pass" })];
    match how {
        InspectAs::Config(kind) => {
            let parsed = config::parse(data, *kind)?;
            let encoded = parsed.encode();
            let matched = encoded == data;
            checks.push(json!({
                "name": "round-trip",
                "status": if matched { "pass" } else { "fail" },
                "encodedLength": encoded.len() as u64,
                "encodedSha256": sha256_hex(&encoded),
            }));
        }
        _ => checks.push(json!({
            // No writer, so nothing to round trip. Saying so beats an
            // absent check a reader could mistake for a passed one.
            "name": "round-trip",
            "status": "not-applicable",
        })),
    }

    let mut object = envelope(&format, data);
    object.insert("operation".into(), json!("validate"));
    object.insert("sha256".into(), json!(sha256_hex(data)));
    object.insert("checks".into(), Value::Array(checks));
    Ok(Value::Object(object))
}

fn string_field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn envelope(format: &str, data: &[u8]) -> Map<String, Value> {
    let mut object = Map::new();
    object.insert("schemaVersion".into(), json!(DOCUMENT_SCHEMA_VERSION));
    object.insert("operation".into(), json!("inspect"));
    object.insert("format".into(), json!(format));
    object.insert("input".into(), json!({ "length": data.len() as u64 }));
    object
}

fn inspect_sedb(data: &[u8]) -> Result<Value> {
    let root = sedb::parse_container(data, 0)?;
    let container_end = root.total_size as usize;

    // Bytes after the resolved container extent, preserved as their own
    // entries. They are ordinary for the subtypes whose 0x10 field is not
    // a file size (PHB, mtb, vins, leaf); for the rest they mean something
    // is unaccounted for. Either way, dropping them would hide it.
    let mut trailing = Vec::new();
    if container_end < data.len() {
        let bytes = &data[container_end..];
        trailing.push(json!({
            "kind": "trailing-bytes",
            "span": { "offset": container_end as u64, "length": bytes.len() as u64 },
            "sha256": sha256_hex(bytes),
        }));
    }

    let mut object = envelope(root.format_id(), data);
    object.insert("root".into(), container_to_json(&root));
    object.insert("trailing".into(), Value::Array(trailing));
    Ok(Value::Object(object))
}

fn container_to_json(container: &Container) -> Value {
    let mut object = Map::new();
    object.insert("kind".into(), json!("sedb-container"));
    object.insert("span".into(), container.span.to_json());
    object.insert("subtype".into(), json!(container.subtype));
    object.insert("unknownA".into(), json!(container.unknown_a));
    object.insert("flags".into(), json!(container.flags));
    object.insert("headerSize".into(), json!(container.header_size));
    object.insert("declaredSize".into(), json!(container.declared_size));
    object.insert("totalSize".into(), json!(container.total_size));
    object.insert(
        "headerUnknown".into(),
        Value::Array(container.header_unknown.iter().map(entry_to_json).collect()),
    );
    if let Some(res) = &container.res {
        object.insert(
            "res".into(),
            json!({
                "subresourceCount": res.subresource_count,
                "unknownB": res.unknown_b,
                "typeName": res.type_name,
            }),
        );
    }
    object.insert(
        "entries".into(),
        Value::Array(container.entries.iter().map(entry_to_json).collect()),
    );
    object.insert(
        "anomalies".into(),
        Value::Array(
            container
                .anomalies
                .iter()
                .map(|item| item.to_json())
                .collect(),
        ),
    );
    Value::Object(object)
}

fn entry_to_json(entry: &Entry) -> Value {
    let mut object = Map::new();
    object.insert("kind".into(), json!(entry.body.kind_name()));
    object.insert("span".into(), entry.span.to_json());
    object.insert("sha256".into(), json!(entry.sha256));
    match &entry.body {
        EntryBody::Payload | EntryBody::Gap => {}
        EntryBody::Directory { entry_count } => {
            object.insert("entryCount".into(), json!(entry_count));
        }
        EntryBody::Subresource {
            index,
            declared_offset,
            declared_size,
            kind,
            child,
        } => {
            object.insert("index".into(), json!(index));
            object.insert("declaredOffset".into(), json!(declared_offset));
            object.insert("declaredSize".into(), json!(declared_size));
            object.insert("declaredKind".into(), json!(kind));
            if let Some(child) = child {
                object.insert("child".into(), container_to_json(child));
            }
        }
    }
    Value::Object(object)
}

fn inspect_ssd(data: &[u8]) -> Result<Value> {
    // A scrambled resource is decoded first and then read by exactly the
    // code a plaintext document is read by, so the two cannot drift. Spans
    // inside the document are relative to the decoded bytes, which the
    // report states rather than leaving to be inferred.
    let container = scrambled::decode(data).ok();
    let body: &[u8] = match &container {
        Some(decoded) => &decoded.document,
        None => data,
    };
    let document = ssd::parse_document(body)?;
    let mut object = envelope(document.format_id(), data);
    object.insert(
        "container".into(),
        match &container {
            None => Value::Null,
            Some(decoded) => container_facts(decoded),
        },
    );
    object.insert(
        "spanBase".into(),
        json!(if container.is_some() {
            "decoded"
        } else {
            "input"
        }),
    );
    object.insert("document".into(), ssd_to_json(&document));
    Ok(Value::Object(object))
}

fn container_facts(decoded: &scrambled::ScrambledXml) -> Value {
    json!({
        "kind": "scrambled-xml",
        "encoded": decoded.encoded.to_json(),
        "trailer": decoded.trailer.to_json(),
        "keyA": decoded.key_a,
        "keyB": decoded.key_b,
        "finalByteCorrected": decoded.final_byte_corrected,
        "decodedLength": decoded.document.len() as u64,
        "decodedSha256": sha256_hex(&decoded.document),
    })
}

/// The container view: what the decode establishes, and a census of the
/// document's shape. Element and attribute names and their counts are retained.
/// attribute values and element text are omitted so the report stays
/// committable for a retail fixture. Reading the document itself is `--as ssd`.
fn inspect_scrambled(data: &[u8]) -> Result<Value> {
    let decoded = scrambled::decode(data)?;
    let document = xml::parse_document(&decoded.document)?;
    let mut elements: BTreeMap<&str, u64> = BTreeMap::new();
    let mut attributes: BTreeMap<&str, u64> = BTreeMap::new();
    census(&document.root, &mut elements, &mut attributes);

    let mut object = envelope("scrambled-xml", data);
    object.insert("container".into(), container_facts(&decoded));
    object.insert(
        "document".into(),
        json!({
            "byteOrderMark": document.byte_order_mark,
            "root": document.root.name,
            "elementCounts": counts_to_json(&elements),
            "attributeCounts": counts_to_json(&attributes),
        }),
    );
    Ok(Value::Object(object))
}

/// The container view, plus a census of the widget document's shape.
///
/// Deliberately no attribute value and no element text: a widget document
/// is client content, so what a retail expectation may record is its
/// structure - spans, block count, digest, and the names its elements and
/// attributes are drawn from - and not the document.
fn inspect_sqwt(data: &[u8], name: &str) -> Result<Value> {
    let decoded = sqwt::decode(data, name)?;
    let document = xml::parse_document_with(&decoded.document, xml::Profile::SqwtWidget)?;
    let mut elements: BTreeMap<&str, u64> = BTreeMap::new();
    let mut attributes: BTreeMap<&str, u64> = BTreeMap::new();
    census(&document.root, &mut elements, &mut attributes);

    let mut object = envelope("sqwt", data);
    object.insert(
        "container".into(),
        json!({
            "kind": "sqwt",
            "header": decoded.header.to_json(),
            "enciphered": decoded.enciphered.to_json(),
            "plaintextTail": decoded.plaintext_tail.to_json(),
            "blockCount": decoded.block_count,
            // The name is the key, so a report that did not state it would
            // not say what was decoded. It is the caller's own input.
            "keyName": decoded.key_name,
            "decodedLength": decoded.document.len() as u64,
            "decodedSha256": sha256_hex(&decoded.document),
        }),
    );
    object.insert(
        "document".into(),
        json!({
            "byteOrderMark": document.byte_order_mark,
            "root": document.root.name,
            "comments": document.comments,
            "elementCounts": counts_to_json(&elements),
            "attributeCounts": counts_to_json(&attributes),
        }),
    );
    Ok(Value::Object(object))
}

fn inspect_lpb(data: &[u8]) -> Result<Value> {
    let file = lpb::extract(data)?;
    let mut object = envelope("lpb", data);
    object.insert("variant".into(), json!(file.variant.name()));
    object.insert("header".into(), file.header.to_json());
    object.insert(
        "unknownHeader".into(),
        Value::Array(
            file.unknown_header
                .iter()
                .map(|field| {
                    json!({
                        "span": field.span.to_json(),
                        "sha256": sha256_hex(&field.bytes),
                    })
                })
                .collect(),
        ),
    );
    object.insert("advisorySize".into(), json!(file.advisory_size));
    object.insert(
        "encodedPrefix".into(),
        file.encoded_prefix
            .map(Span::to_json)
            .unwrap_or(Value::Null),
    );
    object.insert("encodedPayload".into(), file.encoded_payload.to_json());
    object.insert("decodedLength".into(), json!(file.decoded.len() as u64));
    object.insert("decodedSha256".into(), json!(sha256_hex(&file.decoded)));
    Ok(Value::Object(object))
}

fn inspect_staticactor(data: &[u8]) -> Result<Value> {
    let file = staticactor::parse(data)?;
    let mut object = envelope("staticactor-san", data);
    object.insert("header".into(), file.header.to_json());
    object.insert(
        "unknownHeader".into(),
        json!({
            "span": file.unknown_header.to_json(),
            "encodedSha256": sha256_hex(span_bytes(data, file.unknown_header)),
            "decodedSha256": sha256_xor_hex(
                span_bytes(data, file.unknown_header),
                staticactor::XOR_KEY,
            ),
        }),
    );
    object.insert(
        "encoding".into(),
        json!({
            "kind": "xor",
            "key": staticactor::XOR_KEY,
            "span": {
                "offset": 4,
                "length": (data.len() - 4) as u64,
            },
        }),
    );
    object.insert(
        "recordCount".into(),
        json!({
            "span": file.count_span.to_json(),
            "byteOrder": "big",
            "value": file.declared_count,
        }),
    );
    object.insert("encodedBody".into(), file.encoded_body.to_json());
    object.insert(
        "records".into(),
        Value::Array(
            file.records
                .iter()
                .map(|record| {
                    let encoded = span_bytes(data, record.string_span);
                    let decoded_ascii = encoded
                        .iter()
                        .all(|byte| (byte ^ staticactor::XOR_KEY).is_ascii());
                    let decoded_starts_with_slash = encoded
                        .first()
                        .is_some_and(|byte| byte ^ staticactor::XOR_KEY == b'/');
                    json!({
                        "index": record.index,
                        "span": record.span.to_json(),
                        "field0": {
                            "span": record.value_span.to_json(),
                            "byteOrder": "big",
                            "value": record.value_be,
                            "meaning": "unknown",
                        },
                        "string": {
                            "encodedSpan": record.string_span.to_json(),
                            "terminatorSpan": record.terminator_span.to_json(),
                            "decodedLength": record.string_span.length,
                            "decodedSha256": sha256_xor_hex(encoded, staticactor::XOR_KEY),
                            "decodedAscii": decoded_ascii,
                            "decodedStartsWithSlash": decoded_starts_with_slash,
                            "meaning": "unknown",
                        },
                    })
                })
                .collect(),
        ),
    );
    Ok(Value::Object(object))
}

fn inspect_tagged_resource(data: &[u8], kind: TaggedResourceKind) -> Result<Value> {
    let resource = gtex_pwib::parse(data, kind)?;
    let header_unknown = span_bytes(data, resource.header_unknown);
    let declared_extent = span_bytes(data, resource.declared_extent);
    let mut object = envelope(resource.kind.format_id(), data);
    object.insert(
        "signature".into(),
        json!({
            "ascii": String::from_utf8_lossy(resource.kind.magic()),
            "span": resource.signature.to_json(),
        }),
    );
    object.insert(
        "header".into(),
        json!({
            "span": resource.header.to_json(),
            "unknown": [{
                "kind": "unknown-gap",
                "span": resource.header_unknown.to_json(),
                "sha256": sha256_hex(header_unknown),
            }],
        }),
    );
    if resource.kind == TaggedResourceKind::Gtex {
        object.insert(
            "extentSize".into(),
            json!({
                "byteOrder": "big",
                "span": { "offset": 0x1c, "length": 4 },
                "value": resource.declared_extent_size,
            }),
        );
    }
    let mut extent_object = Map::new();
    extent_object.insert(
        "kind".into(),
        json!(if resource.kind == TaggedResourceKind::Pwib {
            "sedb-container"
        } else {
            "opaque-extent"
        }),
    );
    extent_object.insert("span".into(), resource.declared_extent.to_json());
    extent_object.insert("sha256".into(), json!(sha256_hex(declared_extent)));
    if let Some(nested) = &resource.nested_sedb {
        extent_object.insert("child".into(), container_to_json(nested));
    }
    object.insert("declaredExtent".into(), Value::Object(extent_object));
    object.insert(
        "trailing".into(),
        if resource.trailing.length == 0 {
            json!([])
        } else {
            json!([{
                "kind": "trailing-bytes",
                "span": resource.trailing.to_json(),
                "sha256": sha256_hex(span_bytes(data, resource.trailing)),
            }])
        },
    );
    object.insert("layoutStatus".into(), json!("bounded"));
    object.insert("anomalies".into(), json!([]));
    Ok(Value::Object(object))
}

fn span_bytes(data: &[u8], span: Span) -> &[u8] {
    &data[span.offset as usize..span.end() as usize]
}

fn inspect_lpb_bytecode(data: &[u8]) -> Result<Value> {
    let file = lpb::extract(data)?;
    let chunk = lua51::parse(&file.decoded)?;
    let mut object = envelope("client-lua", data);
    object.insert(
        "spanBase".into(),
        json!({ "wrapper": "input", "bytecode": "decoded" }),
    );
    object.insert(
        "wrapper".into(),
        json!({
            "variant": file.variant.name(),
            "header": file.header.to_json(),
            "unknownHeader": Value::Array(file.unknown_header.iter().map(|field| json!({
                "span": field.span.to_json(),
                "sha256": sha256_hex(&field.bytes),
            })).collect()),
            "advisorySize": file.advisory_size,
            "encodedPrefix": file.encoded_prefix.map(Span::to_json),
            "encodedPayload": file.encoded_payload.to_json(),
            "decodedLength": file.decoded.len() as u64,
            "decodedSha256": sha256_hex(&file.decoded),
        }),
    );
    object.insert(
        "bytecode".into(),
        json!({
            "header": {
                "span": chunk.header.span.to_json(),
                "version": chunk.header.version,
                "format": chunk.header.format,
                "endianness": if chunk.header.little_endian { "little" } else { "big" },
                "intSize": chunk.header.int_size,
                "sizeTSize": chunk.header.size_t_size,
                "instructionSize": chunk.header.instruction_size,
                "numberSize": chunk.header.number_size,
                "integralNumbers": chunk.header.integral_numbers,
            },
            "root": lua_prototype_to_json(&chunk.root, &file.decoded),
        }),
    );
    Ok(Value::Object(object))
}

fn lua_prototype_to_json(prototype: &Lua51Prototype, decoded: &[u8]) -> Value {
    json!({
        "span": prototype.span.to_json(),
        "source": prototype.source.as_ref().map(lua_string_to_json),
        "lineDefined": prototype.line_defined,
        "lastLineDefined": prototype.last_line_defined,
        "upvalueCount": prototype.upvalue_count,
        "parameterCount": prototype.parameter_count,
        "varargFlags": prototype.vararg_flags,
        "maxStackSize": prototype.max_stack_size,
        "instructions": {
            "span": prototype.instructions.to_json(),
            "count": prototype.instruction_count,
            "sha256": span_sha256(decoded, prototype.instructions),
            "items": Value::Array(prototype.decoded_instructions.iter().map(
                lua_instruction_to_json
            ).collect()),
            "setlistExtraWords": Value::Array(prototype.setlist_extra_words.iter().map(|word| json!({
                "index": word.index,
                "offset": word.span.offset,
                "span": word.span.to_json(),
                "rawWord": word.raw_word,
            })).collect()),
        },
        "constants": Value::Array(prototype.constants.iter().map(|constant| {
            let mut value = json!({
                "type": constant.kind_name(),
                "span": constant.span().to_json(),
                "sha256": span_sha256(decoded, constant.span()),
            });
            if let LuaConstant::String { value: string, .. } = constant {
                value["length"] = json!(string.bytes.len() as u64);
            }
            value
        }).collect()),
        "nested": Value::Array(prototype.nested.iter().map(|child| {
            lua_prototype_to_json(child, decoded)
        }).collect()),
        "debug": {
            "lineInfo": {
                "span": prototype.line_info.to_json(),
                "count": prototype.line_info_count,
            },
            "localCount": prototype.local_count,
            "upvalueNameCount": prototype.upvalue_name_count,
        },
    })
}

fn lua_instruction_to_json(instruction: &Lua51Instruction) -> Value {
    let operands = match instruction.operands {
        Lua51Operands::Abc { a, b, c } => json!({
            "A": a,
            "B": lua_operand_to_json(b),
            "C": lua_operand_to_json(c),
        }),
        Lua51Operands::Abx { a, bx } => json!({
            "A": a,
            "Bx": lua_operand_to_json(bx),
        }),
        Lua51Operands::Asbx { a, sbx } => json!({
            "A": a,
            "sBx": sbx,
        }),
    };
    json!({
        "index": instruction.index,
        "offset": instruction.span.offset,
        "span": instruction.span.to_json(),
        "rawWord": instruction.raw_word,
        "opcode": {
            "number": instruction.opcode.number,
            "name": instruction.opcode.name,
        },
        "mode": instruction.opcode.mode.name(),
        "operands": operands,
    })
}

fn lua_operand_to_json(operand: Lua51Operand) -> Value {
    match operand {
        Lua51Operand::Unused { raw } => json!({ "kind": "unused", "raw": raw }),
        Lua51Operand::Value { value } => json!({ "kind": "value", "value": value }),
        Lua51Operand::Register { index, raw, rk } => {
            json!({ "kind": "register", "index": index, "raw": raw, "rk": rk })
        }
        Lua51Operand::Constant { index, raw, rk } => {
            json!({ "kind": "constant", "index": index, "raw": raw, "rk": rk })
        }
    }
}

fn lua_string_to_json(value: &LuaString) -> Value {
    json!({
        "span": value.span.to_json(),
        "length": value.bytes.len() as u64,
        "sha256": sha256_hex(&value.bytes),
    })
}

fn span_sha256(data: &[u8], span: Span) -> String {
    let start = span.offset as usize;
    let end = start + span.length as usize;
    sha256_hex(&data[start..end])
}

fn census<'a>(
    element: &'a xml::Element,
    elements: &mut BTreeMap<&'a str, u64>,
    attributes: &mut BTreeMap<&'a str, u64>,
) {
    *elements.entry(element.name.as_str()).or_insert(0) += 1;
    for attribute in &element.attributes {
        *attributes.entry(attribute.name.as_str()).or_insert(0) += 1;
    }
    for child in &element.children {
        census(child, elements, attributes);
    }
}

fn counts_to_json(counts: &BTreeMap<&str, u64>) -> Value {
    Value::Array(
        counts
            .iter()
            .map(|(name, count)| json!({ "name": name, "count": count }))
            .collect(),
    )
}

fn ssd_to_json(document: &SsdDocument) -> Value {
    let mut object = Map::new();
    object.insert("byteOrderMark".into(), json!(document.byte_order_mark));
    object.insert(
        "declaration".into(),
        match &document.declaration {
            None => Value::Null,
            Some(declaration) => json!({
                "span": declaration.span.to_json(),
                "version": declaration.version,
                "encoding": declaration.encoding,
            }),
        },
    );
    object.insert("version".into(), json!(document.version));
    object.insert(
        "sheets".into(),
        Value::Array(document.sheets.iter().map(sheet_to_json).collect()),
    );
    Value::Object(object)
}

fn sheet_to_json(sheet: &ssd::Sheet) -> Value {
    let mut object = Map::new();
    object.insert("kind".into(), json!(sheet.kind_name()));
    object.insert("span".into(), sheet.span.to_json());
    object.insert("name".into(), json!(sheet.name));
    // Every attribute verbatim and in document order, so an attribute this
    // crate does not interpret is still visible in the report.
    object.insert(
        "attributes".into(),
        Value::Array(
            sheet
                .attributes
                .iter()
                .map(|attribute| {
                    json!({
                        "name": attribute.name,
                        "value": attribute.value,
                        "span": attribute.span.to_json(),
                    })
                })
                .collect(),
        ),
    );
    match &sheet.body {
        SheetBody::Reference { infofile } => {
            object.insert(
                "infofile".into(),
                json!({
                    "id": infofile.value(),
                    "hex": infofile.to_hex(),
                    "path": infofile.dat_path(),
                }),
            );
        }
        SheetBody::Definition {
            columns,
            index,
            blocks,
        } => {
            object.insert(
                "columns".into(),
                Value::Array(
                    columns
                        .iter()
                        .map(|column| {
                            json!({
                                "type": column.name(),
                                "width": column.width().map(|width| width as u64),
                            })
                        })
                        .collect(),
                ),
            );
            object.insert("index".into(), json!(index));
            object.insert(
                "blocks".into(),
                Value::Array(
                    blocks
                        .iter()
                        .map(|block| {
                            json!({
                                "span": block.span.to_json(),
                                "begin": block.begin,
                                "count": block.count,
                                "data": resource_to_json(block.data),
                                "enable": resource_to_json(block.enable),
                                "offsets": resource_to_json(block.offsets),
                            })
                        })
                        .collect(),
                ),
            );
        }
    }
    Value::Object(object)
}

fn resource_to_json(id: crate::resource::ResourceId) -> Value {
    json!({ "id": id.value(), "hex": id.to_hex(), "path": id.dat_path() })
}

fn inspect_enable_file(data: &[u8]) -> Result<Value> {
    let parsed: EnableFile = sheet::parse_enable_file(data)?;
    let mut object = envelope("enable-file", data);
    object.insert(
        "ranges".into(),
        Value::Array(
            parsed
                .ranges
                .iter()
                .map(|range| {
                    json!({
                        "span": range.span.to_json(),
                        "firstRow": range.first_row,
                        "count": range.count,
                        "endRow": range.end(),
                    })
                })
                .collect(),
        ),
    );
    object.insert("rowCount".into(), json!(parsed.row_count()));
    object.insert(
        "anomalies".into(),
        Value::Array(parsed.anomalies.iter().map(|item| item.to_json()).collect()),
    );
    Ok(Value::Object(object))
}

fn inspect_row_offsets(data: &[u8]) -> Result<Value> {
    let parsed: RowOffsets = sheet::parse_row_offsets(data)?;
    let mut object = envelope("ssd-sheet", data);
    object.insert("view".into(), json!("row-offsets"));
    object.insert("slotCount".into(), json!(parsed.slot_count() as u64));
    object.insert("dataLength".into(), json!(parsed.data_length()));
    // Only the slots with a span: every other entry repeats its
    // predecessor, so this list plus the slot count is the whole array.
    object.insert(
        "rows".into(),
        Value::Array(
            parsed
                .rows
                .iter()
                .map(|row| json!({ "index": row.index, "span": row.span.to_json() }))
                .collect(),
        ),
    );
    object.insert("rowCount".into(), json!(parsed.rows.len() as u64));
    object.insert(
        "anomalies".into(),
        Value::Array(parsed.anomalies.iter().map(|item| item.to_json()).collect()),
    );
    Ok(Value::Object(object))
}

fn inspect_sheet_data(data: &[u8], columns: &[ColumnType]) -> Result<Value> {
    let mut object = envelope("ssd-sheet", data);
    let mut inventory = BTreeMap::new();

    if columns.is_empty() {
        let strings = sheet::parse_string_stream(data)?;
        for string in &strings {
            count_tokens(&string.rich, &mut inventory);
        }
        object.insert("view".into(), json!("strings"));
        object.insert("stringCount".into(), json!(strings.len() as u64));
        object.insert(
            "scrambledCount".into(),
            json!(strings.iter().filter(|string| string.scrambled).count() as u64),
        );
        object.insert(
            "strings".into(),
            Value::Array(strings.iter().map(string_to_json).collect()),
        );
    } else {
        let rows: Vec<Row> = sheet::parse_rows(data, columns)?;
        for row in &rows {
            for value in &row.values {
                if let ColumnValue::Text(string) = value {
                    count_tokens(&string.rich, &mut inventory);
                }
            }
        }
        object.insert("view".into(), json!("rows"));
        object.insert(
            "columns".into(),
            Value::Array(
                columns
                    .iter()
                    .map(|column| {
                        json!({
                            "type": column.name(),
                            "width": column.width().map(|width| width as u64),
                        })
                    })
                    .collect(),
            ),
        );
        object.insert("rowCount".into(), json!(rows.len() as u64));
        object.insert(
            "rows".into(),
            Value::Array(rows.iter().map(row_to_json).collect()),
        );
    }

    object.insert("tokenInventory".into(), inventory_to_json(&inventory));
    Ok(Value::Object(object))
}

fn string_to_json(string: &SheetString) -> Value {
    json!({
        "kind": "sheet-string",
        "span": string.span.to_json(),
        "declaredLength": string.declared_length,
        "scrambled": string.scrambled,
        "decodedLength": string.decoded_length(),
        "sha256": string.sha256,
        "segments": Value::Array(string.rich.segments.iter().map(segment_to_json).collect()),
    })
}

fn segment_to_json(segment: &Segment) -> Value {
    match segment {
        Segment::Text { span, text } => json!({
            "kind": "text",
            "span": span.to_json(),
            // The character count, not the text. A report never carries
            // decoded client strings; see the module comment.
            "characters": text.chars().count() as u64,
        }),
        Segment::Token(token) => json!({
            "kind": "token",
            "span": token.span.to_json(),
            "code": token.code,
            "lengthEncoding": token.encoding.name(),
            "lengthBytes": payload_hex(&token.length_bytes),
            "payloadLength": token.payload.len() as u64,
            "payloadSha256": sha256_hex(&token.payload),
        }),
    }
}

fn row_to_json(row: &Row) -> Value {
    json!({
        "index": row.index,
        "span": row.span.to_json(),
        "values": Value::Array(row.values.iter().map(value_to_json).collect()),
    })
}

fn value_to_json(value: &ColumnValue) -> Value {
    match value {
        ColumnValue::Text(string) => string_to_json(string),
        // A fixed-width column's bytes are sheet content, so the report
        // names the column and its span and stops there.
        other => json!({
            "kind": "column",
            "type": other.type_name(),
            "span": other.span().to_json(),
        }),
    }
}

/// The structural view of a configuration file.
///
/// No word value and no run content appear here, deliberately. These files
/// contain user-written settings, so a faithful report of their values would
/// be the file. What the report carries is the shape:
/// the stamp, which is a compiled-in constant and not this install's, the
/// grid's extent and how much of it any install has ever written, and where
/// the printable runs sit. A caller that wants the values reads them from
/// `config::ConfigFile`.
fn inspect_config(data: &[u8], kind: ConfigKind) -> Result<Value> {
    let parsed: ConfigFile = config::parse(data, kind)?;
    let mut object = envelope(kind.format_id(), data);
    object.insert("sha256".into(), json!(sha256_hex(data)));
    object.insert(
        "stamp".into(),
        match parsed.stamp {
            None => Value::Null,
            Some(stamp) => json!({ "span": stamp.span.to_json(), "value": stamp.value }),
        },
    );
    object.insert(
        "grid".into(),
        if kind.is_word_grid() {
            json!({
                "span": parsed.grid.to_json(),
                "wordCount": parsed.words.len() as u64,
                "zeroWordCount": parsed.zero_word_count(),
                "nonZeroWordOffsets": parsed.non_zero_word_offsets(),
            })
        } else {
            Value::Null
        },
    );
    object.insert(
        "body".into(),
        if kind.is_word_grid() {
            Value::Null
        } else {
            json!({
                "span": Span::new(0, parsed.body.len() as u64).to_json(),
                "sha256": sha256_hex(&parsed.body),
            })
        },
    );
    object.insert(
        "textRuns".into(),
        Value::Array(
            parsed
                .runs
                .iter()
                .map(|run| {
                    json!({
                        "encoding": run.encoding.name(),
                        "span": run.span.to_json(),
                        "units": run.units,
                    })
                })
                .collect(),
        ),
    );
    Ok(Value::Object(object))
}

fn count_tokens(rich: &RichString, inventory: &mut BTreeMap<u8, u64>) {
    for token in rich.tokens() {
        *inventory.entry(token.code).or_insert(0) += 1;
    }
}

fn inventory_to_json(inventory: &BTreeMap<u8, u64>) -> Value {
    Value::Array(
        inventory
            .iter()
            .map(|(code, count)| json!({ "code": code, "count": count }))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_select_a_reading() {
        let arguments =
            |parts: &[&str]| -> Vec<String> { parts.iter().map(|part| part.to_string()).collect() };
        assert_eq!(
            InspectAs::from_arguments(&arguments(&[])).unwrap(),
            InspectAs::Auto
        );
        assert_eq!(
            InspectAs::from_arguments(&arguments(&["--as", "enable-file"])).unwrap(),
            InspectAs::EnableFile
        );
        assert_eq!(
            InspectAs::from_arguments(&arguments(&["--as", "lpb-bytecode"])).unwrap(),
            InspectAs::LpbBytecode
        );
        assert_eq!(
            InspectAs::from_arguments(&arguments(&["--as", "staticactor-san"])).unwrap(),
            InspectAs::StaticActorSan
        );
        assert_eq!(
            InspectAs::from_arguments(&arguments(&["--as", "gtex"])).unwrap(),
            InspectAs::Gtex
        );
        assert_eq!(
            InspectAs::from_arguments(&arguments(&["--as", "sheet-data", "--columns", "str,u8"]))
                .unwrap(),
            InspectAs::SheetData(vec![ColumnType::Text, ColumnType::Unsigned8])
        );
        for (parts, needle) in [
            (vec!["--as", "not-a-format"], "unknown format"),
            (vec!["--as"], "needs a format name"),
            (vec!["--columns", "str"], "applies to --as sheet-data"),
            (vec!["--as", "sheet-data", "--columns", "s64"], "s64"),
            (vec!["-x"], "unknown option"),
        ] {
            let error = InspectAs::from_arguments(&arguments(&parts)).unwrap_err();
            assert!(error.contains(needle), "{parts:?}: {error}");
        }
    }

    #[test]
    fn auto_recognizes_a_document_and_a_container() {
        let document = inspect_bytes(b"\xEF\xBB\xBF<ssd version=\"0.1\"></ssd>").unwrap();
        assert_eq!(document["format"], "ssd-master");
        let mut san = b"sane".to_vec();
        san.extend([0u8; 9].map(|byte| byte ^ staticactor::XOR_KEY));
        let document = inspect_bytes(&san).unwrap();
        assert_eq!(document["format"], "staticactor-san");
        let mut gtex = vec![0u8; 0x20];
        gtex[0..4].copy_from_slice(b"GTEX");
        assert_eq!(inspect_bytes(&gtex).unwrap()["format"], "gtex");
        let mut pwib = vec![0u8; 0x24];
        pwib[0..4].copy_from_slice(b"PWIB");
        pwib[0x10..0x18].copy_from_slice(b"SEDBsyn\0");
        pwib[0x1e..0x20].copy_from_slice(&0x14u16.to_le_bytes());
        pwib[0x20..0x24].copy_from_slice(&0x14u32.to_le_bytes());
        assert_eq!(inspect_bytes(&pwib).unwrap()["format"], "pwib");
        let error = inspect_bytes(b"not a container at all").unwrap_err();
        assert_eq!(error.kind(), crate::error::ErrorKind::BadMagic);
    }

    #[test]
    fn a_sheet_data_report_carries_no_decoded_text() {
        let mut data = vec![];
        let body: Vec<u8> = b"secret"
            .iter()
            .map(|byte| byte ^ sheet::SCRAMBLE_KEY)
            .collect();
        data.extend(((body.len() + 2) as u16).to_le_bytes());
        data.push(sheet::SCRAMBLE_MARKER);
        data.extend(body);
        data.push(sheet::SCRAMBLE_KEY);
        let document = inspect_bytes_as(&data, &InspectAs::SheetData(Vec::new())).unwrap();
        let text = crate::normalize::to_canonical_json(&document);
        assert!(!text.contains("secret"), "{text}");
        assert_eq!(document["stringCount"], 1);
        assert_eq!(document["strings"][0]["decodedLength"], 6);
    }
}
