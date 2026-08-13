//! SEDB containers and the RES subresource directory.
//!
//! Bounded enumeration only: the parser resolves the container header, the
//! RES directory, and the extent of every subresource, and it accounts for
//! every payload byte. It does not interpret a payload. Payload internals
//! are outside this parser's scope. See `docs/formats/sedb-res.md` for the byte
//! layout evidence and for what this parser does not claim to understand.
//!
//! The accounting rule is the point: entries tile the payload region with
//! no holes and no overlap in the output, so a byte this parser does not
//! understand appears as an unknown entry rather than disappearing.

use crate::digest::sha256_hex;
use crate::error::{ErrorKind, FormatError, Result};
use crate::reader::{escape_tag, Reader, Span};

/// Container signature.
pub const MAGIC: &[u8; 4] = b"SEDB";

/// Subtype of a composite container carrying a subresource directory.
pub const SUBTYPE_RES: &[u8; 4] = b"RES ";

/// Offset just past the fixed header fields. A header may not be shorter.
pub const FIXED_HEADER_SIZE: u16 = 0x14;

/// Offset of the RES extended header fields inside the container header.
pub const RES_EXTENDED_HEADER_OFFSET: usize = 0x30;

/// Header size a RES container is observed to declare. The directory
/// begins there.
pub const RES_HEADER_SIZE: u16 = 0x40;

/// Bytes per RES subresource directory entry.
pub const RES_DIRECTORY_ENTRY_SIZE: u64 = 16;

/// How many nested containers the parser will follow before refusing.
pub const MAX_NESTING_DEPTH: u32 = 8;

/// The RES extended header, present only on `RES ` containers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResHeader {
    pub subresource_count: u32,
    /// Points near the trailing name table, relative to the directory
    /// start. Semantics unresolved. It is carried so it is not dropped.
    pub unknown_b: u32,
    pub type_name: String,
}

/// What a payload entry is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntryBody {
    /// The whole payload of a container this parser does not decompose.
    Payload,
    /// The RES subresource directory itself.
    Directory { entry_count: u32 },
    /// One subresource, as the directory declares it.
    Subresource {
        index: u32,
        declared_offset: u32,
        declared_size: u32,
        kind: u32,
        child: Option<Box<Container>>,
    },
    /// Payload bytes no directory entry claims.
    Gap,
}

impl EntryBody {
    pub fn kind_name(&self) -> &'static str {
        match self {
            EntryBody::Payload => "payload",
            EntryBody::Directory { .. } => "subresource-directory",
            EntryBody::Subresource { .. } => "subresource",
            EntryBody::Gap => "unknown-gap",
        }
    }
}

/// A span of the input with its digest and its role.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub span: Span,
    pub sha256: String,
    pub body: EntryBody,
}

pub use crate::anomaly::Anomaly;

/// A parsed SEDB container.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Container {
    /// The container as resolved: header start, `total_size` bytes.
    pub span: Span,
    pub subtype: String,
    pub unknown_a: u32,
    pub flags: u16,
    pub header_size: u16,
    /// The 0x10 field verbatim. Advisory: it is the file size for most
    /// subtypes, the header size for PHB, and zero for mtb.
    pub declared_size: u32,
    /// The extent the parser resolved, which is `declared_size` unless
    /// that was below the header.
    pub total_size: u32,
    /// Header bytes past the fixed fields that this parser does not read.
    pub header_unknown: Vec<Entry>,
    pub res: Option<ResHeader>,
    /// Payload entries, ordered by offset, tiling `[header_size, total_size)`.
    pub entries: Vec<Entry>,
    pub anomalies: Vec<Anomaly>,
}

impl Container {
    /// The support-matrix format id this container belongs to.
    pub fn format_id(&self) -> &'static str {
        if self.res.is_some() {
            "res"
        } else {
            "sedb"
        }
    }
}

/// Does `data` start with a SEDB container signature?
pub fn has_magic(data: &[u8]) -> bool {
    data.len() >= MAGIC.len() && &data[..MAGIC.len()] == MAGIC
}

/// Parse the container that begins at `data[0]`, which sits at absolute
/// offset `base` in the original input.
///
/// `data` may extend past the container. The caller owns whatever follows.
pub fn parse_container(data: &[u8], base: u64) -> Result<Container> {
    parse_container_at_depth(data, base, 0)
}

fn parse_container_at_depth(data: &[u8], base: u64, depth: u32) -> Result<Container> {
    if depth > MAX_NESTING_DEPTH {
        return Err(FormatError::new(
            ErrorKind::NestingTooDeep,
            base,
            format!("nested containers exceed the depth limit of {MAX_NESTING_DEPTH}"),
        ));
    }

    let mut reader = Reader::with_base(data, base);
    let magic = reader.take(MAGIC.len())?;
    if magic != MAGIC {
        return Err(FormatError::new(
            ErrorKind::BadMagic,
            base,
            format!("expected a SEDB signature, found '{}'", escape_tag(magic)),
        ));
    }

    let subtype_bytes = reader.take(4)?;
    let is_res = subtype_bytes == SUBTYPE_RES;
    let subtype = escape_tag(subtype_bytes);
    let unknown_a = reader.u32_le()?;
    let flags = reader.u16_le()?;
    let header_size_offset = reader.offset();
    let header_size = reader.u16_le()?;
    let declared_size_offset = reader.offset();
    let declared_size = reader.u32_le()?;

    if header_size < FIXED_HEADER_SIZE {
        return Err(FormatError::new(
            ErrorKind::HeaderTooSmall,
            header_size_offset,
            format!(
                "header size {header_size} is below the {FIXED_HEADER_SIZE} bytes the fixed fields occupy"
            ),
        ));
    }
    if header_size as usize > data.len() {
        return Err(FormatError::new(
            ErrorKind::HeaderSizeOutOfRange,
            header_size_offset,
            format!(
                "header size {} exceeds the {} byte(s) available",
                header_size,
                data.len()
            ),
        ));
    }
    if declared_size as usize > data.len() {
        return Err(FormatError::new(
            ErrorKind::DeclaredSizeOutOfRange,
            declared_size_offset,
            format!(
                "declared size {} exceeds the {} byte(s) available",
                declared_size,
                data.len()
            ),
        ));
    }

    let mut anomalies = Vec::new();

    // The 0x10 field is advisory, not an authoritative extent. Across the
    // 1.23b install it equals the file size for 92303 of 97749 SEDB
    // resources, equals headerSize for the 5203 PHB, and reads zero for
    // the 145 mtb. See docs/formats/sedb-res.md, "The 0x10 field". A value
    // below the header cannot describe a container, so the parser falls
    // back to the header extent and says so rather than rejecting a file
    // the client reads happily.
    let total_size = if declared_size < u32::from(header_size) {
        anomalies.push(Anomaly {
            kind: "declared-size-below-header",
            span: Span::new(declared_size_offset, 4),
            detail: format!(
                "declared size {declared_size} is below the {header_size} byte header; the container extent falls back to the header and the remainder is reported as trailing bytes"
            ),
        });
        u32::from(header_size)
    } else {
        declared_size
    };

    let mut header_unknown = Vec::new();
    let mut res = None;

    if is_res {
        if header_size < RES_HEADER_SIZE {
            return Err(FormatError::new(
                ErrorKind::HeaderTooSmall,
                header_size_offset,
                format!(
                    "a RES container needs a header of at least {RES_HEADER_SIZE} bytes, found {header_size}"
                ),
            ));
        }
        if header_size != RES_HEADER_SIZE {
            anomalies.push(Anomaly {
                kind: "unexpected-res-header-size",
                span: Span::new(header_size_offset, 2),
                detail: format!(
                    "RES header size {header_size} is not the observed {RES_HEADER_SIZE}"
                ),
            });
        }

        reader.seek(RES_EXTENDED_HEADER_OFFSET)?;
        let count_offset = reader.offset();
        let subresource_count = reader.u32_le()?;
        let unknown_b = reader.u32_le()?;
        let repeat_offset = reader.offset();
        let repeated_count = reader.u32_le()?;
        let type_name = reader.tag4()?;

        if subresource_count != repeated_count {
            return Err(FormatError::new(
                ErrorKind::SubresourceCountMismatch,
                repeat_offset,
                format!(
                    "subresource count {subresource_count} at 0x30 disagrees with {repeated_count} at 0x38"
                ),
            ));
        }

        let directory_bytes = u64::from(subresource_count) * RES_DIRECTORY_ENTRY_SIZE;
        if u64::from(header_size) + directory_bytes > u64::from(total_size) {
            return Err(FormatError::new(
                ErrorKind::SubresourceCountOutOfRange,
                count_offset,
                format!(
                    "{subresource_count} subresource entries do not fit in the {total_size} byte container"
                ),
            ));
        }

        push_unknown(
            &mut header_unknown,
            data,
            base,
            FIXED_HEADER_SIZE as usize,
            RES_EXTENDED_HEADER_OFFSET,
        )?;
        push_unknown(
            &mut header_unknown,
            data,
            base,
            RES_HEADER_SIZE as usize,
            header_size as usize,
        )?;

        res = Some(ResHeader {
            subresource_count,
            unknown_b,
            type_name,
        });
    } else {
        push_unknown(
            &mut header_unknown,
            data,
            base,
            FIXED_HEADER_SIZE as usize,
            header_size as usize,
        )?;
    }

    let payload_region = header_size as usize..total_size as usize;
    let entries = match &res {
        Some(header) => parse_res_payload(
            data,
            base,
            header_size,
            total_size,
            header.subresource_count,
            depth,
            &mut anomalies,
        )?,
        None => vec![entry_for(
            data,
            base,
            payload_region.clone(),
            EntryBody::Payload,
        )?],
    };

    let container = Container {
        span: Span::new(base, u64::from(total_size)),
        subtype,
        unknown_a,
        flags,
        header_size,
        declared_size,
        total_size,
        header_unknown,
        res,
        entries,
        anomalies,
    };
    debug_assert!(container.entries_tile_the_payload());
    Ok(container)
}

impl Container {
    /// The accounting invariant: payload entries are ordered, contiguous
    /// from the end of the header, and reach at least the end of the
    /// container. They reach past it only when a flagged anomaly says so.
    pub fn entries_tile_the_payload(&self) -> bool {
        let mut cursor = self.span.offset + u64::from(self.header_size);
        for entry in &self.entries {
            if entry.span.offset != cursor {
                return false;
            }
            cursor = entry.span.end();
        }
        cursor >= self.span.offset + u64::from(self.total_size)
    }
}

/// Enumerate a RES payload: the directory, every subresource it declares,
/// and every payload byte no entry claims.
fn parse_res_payload(
    data: &[u8],
    base: u64,
    header_size: u16,
    total_size: u32,
    subresource_count: u32,
    depth: u32,
    anomalies: &mut Vec<Anomaly>,
) -> Result<Vec<Entry>> {
    let directory_start = header_size as usize;
    let directory_bytes = subresource_count as usize * RES_DIRECTORY_ENTRY_SIZE as usize;
    let payload_base = directory_start + directory_bytes;
    let container_end = total_size as usize;
    let available_end = data.len();

    let mut reader = Reader::with_base(data, base);
    reader.seek(directory_start)?;

    // Resolved extents in directory order. The tiling pass sorts them.
    let mut resolved: Vec<(usize, usize, EntryBody)> = Vec::new();
    for slot in 0..subresource_count {
        let entry_offset = reader.offset();
        let index = reader.u32_le()?;
        let declared_offset = reader.u32_le()?;
        let declared_size = reader.u32_le()?;
        let kind = reader.u32_le()?;

        let start = payload_base.saturating_add(declared_offset as usize);
        let declared_end = start.saturating_add(declared_size as usize);
        if start > available_end {
            anomalies.push(Anomaly {
                kind: "subresource-start-out-of-range",
                span: Span::new(entry_offset, RES_DIRECTORY_ENTRY_SIZE),
                detail: format!(
                    "subresource {slot} starts at payload offset {declared_offset}, past the end of the input"
                ),
            });
            continue;
        }
        // Directory sizes carry alignment slack and can declare an extent
        // past the end of the file. Clamp and say so rather than read out
        // of bounds or drop the entry.
        let end = declared_end.min(available_end);
        if declared_end > available_end {
            anomalies.push(Anomaly {
                kind: "subresource-extent-clamped",
                span: Span::new(base + start as u64, (end - start) as u64),
                detail: format!(
                    "subresource {slot} declares {declared_size} byte(s) ending at {declared_end}, clamped to the {available_end} byte input"
                ),
            });
        } else if declared_end > container_end {
            anomalies.push(Anomaly {
                kind: "subresource-past-container-end",
                span: Span::new(base + start as u64, (end - start) as u64),
                detail: format!(
                    "subresource {slot} ends at {declared_end}, past the declared container size {container_end}"
                ),
            });
        }

        let child = maybe_parse_child(data, base, start, end, depth, slot, anomalies);
        resolved.push((
            start,
            end,
            EntryBody::Subresource {
                index,
                declared_offset,
                declared_size,
                kind,
                child,
            },
        ));
    }

    resolved.sort_by_key(|(start, end, _)| (*start, *end));

    let mut entries = Vec::with_capacity(resolved.len() + 2);
    entries.push(entry_for(
        data,
        base,
        directory_start..payload_base,
        EntryBody::Directory {
            entry_count: subresource_count,
        },
    )?);

    let mut cursor = payload_base;
    for (start, end, body) in resolved {
        if start < cursor {
            anomalies.push(Anomaly {
                kind: "subresource-overlap",
                span: Span::new(base + start as u64, (cursor.min(end) - start) as u64),
                detail: format!(
                    "subresource extent starting at {start} overlaps the preceding entry ending at {cursor}"
                ),
            });
            // Report the overlapping bytes once, under the later entry, so
            // the payload still tiles.
            if end <= cursor {
                continue;
            }
            entries.push(entry_for(data, base, cursor..end, body)?);
            cursor = end;
            continue;
        }
        if start > cursor {
            entries.push(entry_for(data, base, cursor..start, EntryBody::Gap)?);
        }
        entries.push(entry_for(data, base, start..end, body)?);
        cursor = end;
    }

    if cursor < container_end {
        entries.push(entry_for(
            data,
            base,
            cursor..container_end,
            EntryBody::Gap,
        )?);
    } else if cursor > container_end {
        // A clamped or overlong subresource already carries its own
        // anomaly. The container span grows to keep the tiling honest.
        anomalies.push(Anomaly {
            kind: "payload-past-container-end",
            span: Span::new(base + container_end as u64, (cursor - container_end) as u64),
            detail: format!(
                "resolved subresource extents reach {cursor}, past the declared container size {container_end}"
            ),
        });
    }

    Ok(entries)
}

/// Follow a nested container. A malformed child is an anomaly on the
/// parent, not a failure of the whole parse: the bytes stay accounted for.
fn maybe_parse_child(
    data: &[u8],
    base: u64,
    start: usize,
    end: usize,
    depth: u32,
    slot: u32,
    anomalies: &mut Vec<Anomaly>,
) -> Option<Box<Container>> {
    let bytes = data.get(start..end)?;
    if !has_magic(bytes) {
        return None;
    }
    match parse_container_at_depth(bytes, base + start as u64, depth + 1) {
        Ok(child) => Some(Box::new(child)),
        Err(error) => {
            anomalies.push(Anomaly {
                kind: "nested-parse-error",
                span: Span::new(base + start as u64, (end - start) as u64),
                detail: format!(
                    "subresource {slot} carries a SEDB signature but did not parse: {error}"
                ),
            });
            None
        }
    }
}

/// Record an unread header range, skipping it when it is empty.
fn push_unknown(
    into: &mut Vec<Entry>,
    data: &[u8],
    base: u64,
    start: usize,
    end: usize,
) -> Result<()> {
    if start < end {
        into.push(entry_for(data, base, start..end, EntryBody::Gap)?);
    }
    Ok(())
}

fn entry_for(
    data: &[u8],
    base: u64,
    range: std::ops::Range<usize>,
    body: EntryBody,
) -> Result<Entry> {
    let bytes = data.get(range.clone()).ok_or_else(|| {
        FormatError::new(
            ErrorKind::UnexpectedEndOfInput,
            base + range.start as u64,
            format!(
                "span {}..{} is outside the {} byte input",
                range.start,
                range.end,
                data.len()
            ),
        )
    })?;
    Ok(Entry {
        span: Span::new(base + range.start as u64, bytes.len() as u64),
        sha256: sha256_hex(bytes),
        body,
    })
}
