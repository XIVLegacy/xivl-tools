//! Loader-backed GTEX fields and PWIB segment boundaries.

use crate::error::{ErrorKind, FormatError, Result};
use crate::reader::{escape_tag, Reader, Span};
use crate::sedb;

pub const GTEX_MAGIC: &[u8; 4] = b"GTEX";
pub const PWIB_MAGIC: &[u8; 4] = b"PWIB";
pub const GTEX_FIXED_FIELDS_SIZE: usize = 0x18;
pub const PWIB_HEADER_SIZE: usize = 0x10;
pub const SURFACE_OFFSET_ENTRY_SIZE: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaggedResourceKind {
    Gtex,
    Pwib,
}

impl TaggedResourceKind {
    pub const fn format_id(self) -> &'static str {
        match self {
            Self::Gtex => "gtex",
            Self::Pwib => "pwib",
        }
    }

    pub const fn magic(self) -> &'static [u8; 4] {
        match self {
            Self::Gtex => GTEX_MAGIC,
            Self::Pwib => PWIB_MAGIC,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextureKind {
    Texture2d,
    Cube,
    Volume,
}

impl TextureKind {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Texture2d => "2d",
            Self::Cube => "cube",
            Self::Volume => "volume",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GtexFormat {
    pub index: u8,
    pub d3d_value: u32,
    pub d3d_name: &'static str,
    pub bits_per_pixel: u8,
    pub block_bytes: Option<u8>,
}

pub const fn gtex_format(index: u8) -> Option<GtexFormat> {
    match index {
        4 => Some(GtexFormat {
            index,
            d3d_value: 21,
            d3d_name: "D3DFMT_A8R8G8B8",
            bits_per_pixel: 32,
            block_bytes: None,
        }),
        24 => Some(GtexFormat {
            index,
            d3d_value: 0x3154_5844,
            d3d_name: "D3DFMT_DXT1",
            bits_per_pixel: 4,
            block_bytes: Some(8),
        }),
        26 => Some(GtexFormat {
            index,
            d3d_value: 0x3554_5844,
            d3d_name: "D3DFMT_DXT5",
            bits_per_pixel: 8,
            block_bytes: Some(16),
        }),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurfaceEntry {
    pub index: u32,
    pub face: u8,
    pub mip_level: u8,
    pub offset_field_span: Span,
    pub size_field_span: Span,
    pub relative_offset: u32,
    pub declared_size: u32,
    pub source_span: Span,
    pub calculated_size: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GtexResource {
    pub signature: Span,
    pub header: Span,
    pub format_index: u8,
    pub mip_levels: u8,
    pub flags: u8,
    pub texture_kind: TextureKind,
    pub width: u16,
    pub height: u16,
    pub depth: u16,
    pub offset_table_base: u32,
    pub data_base: u32,
    pub format: Option<GtexFormat>,
    pub surfaces: Vec<SurfaceEntry>,
    pub header_unknown: Vec<Span>,
    pub data_gaps: Vec<Span>,
    pub data: Span,
}

impl GtexResource {
    pub fn materialization_refusal(&self) -> Option<&'static str> {
        if self.offset_table_base == 0 {
            return Some("GTEX has no surface table");
        }
        if self.format.is_none() {
            return Some("GTEX client format index is not mapped");
        }
        if self.flags != 0 {
            return Some("GTEX materialization supports only flags 0");
        }
        if self.texture_kind != TextureKind::Texture2d {
            return Some("GTEX materialization supports only 2D textures");
        }
        if self.depth != 1 {
            return Some("GTEX materialization supports only depth 1");
        }
        if self.mip_levels == 0 || self.width == 0 || self.height == 0 {
            return Some("GTEX materialization requires nonzero mip count, width, and height");
        }
        if self.surfaces.len() != usize::from(self.mip_levels) {
            return Some("GTEX surface table does not contain exactly one entry per mip");
        }
        None
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SedbPrefix {
    pub span: Span,
    pub subtype: String,
    pub unknown_a: u32,
    pub flags: u16,
    pub header_size: u16,
    pub declared_size: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PwibResource {
    pub signature: Span,
    pub header: Span,
    pub total_size: u32,
    pub first_offset: u32,
    pub second_offset: u32,
    pub first_segment: Span,
    pub second_segment: Span,
    pub trailing: Span,
    pub sedb_prefix: SedbPrefix,
}

/// One bounded entry in the first RES segment. `visible_index` is absent for
/// the RESOURCE_TYPE and RESOURCE_ID metadata blocks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResEntry {
    pub physical_index: u32,
    pub visible_index: Option<u32>,
    pub name_index: u32,
    pub name: String,
    pub declared_offset: u32,
    pub declared_size: u32,
    pub kind: u32,
    pub source_span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DescriptorOffsetBranch {
    DirectWord,
    IndirectDword,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PwibSelection {
    pub visible_index: u32,
    pub pwib: PwibResource,
    pub first_directory: Span,
    pub directory_count: u32,
    pub names: Span,
    pub names_count: u32,
    pub payload_base: u64,
    pub entries: Vec<ResEntry>,
    pub metadata_count: u32,
    pub visible_count: u32,
    pub resource_type: ResEntry,
    pub resource_id: Option<ResEntry>,
    pub selected_entry: ResEntry,
    pub selected_type: u32,
    pub txb: Span,
    pub descriptor_offset_field: Span,
    pub descriptor_pointer_field: Option<Span>,
    pub descriptor_offset_word: u16,
    pub descriptor_offset_branch: DescriptorOffsetBranch,
    pub descriptor_offset: u64,
    pub descriptor_fixed: Span,
    pub descriptor: Span,
    pub descriptor_table: Span,
    pub gtex: GtexResource,
    pub surface: Span,
    pub surface_relative_offset: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaggedResource {
    Gtex(GtexResource),
    Pwib(PwibResource),
}

pub fn detect(data: &[u8]) -> Option<TaggedResourceKind> {
    if data.starts_with(GTEX_MAGIC) {
        Some(TaggedResourceKind::Gtex)
    } else if data.starts_with(PWIB_MAGIC) {
        Some(TaggedResourceKind::Pwib)
    } else {
        None
    }
}

pub fn parse(data: &[u8], expected: TaggedResourceKind) -> Result<TaggedResource> {
    if data.len() < 4 {
        return Err(FormatError::new(
            ErrorKind::UnexpectedEndOfInput,
            0,
            format!("{} signature is truncated", expected.format_id()),
        ));
    }
    if !data.starts_with(expected.magic()) {
        return Err(FormatError::new(
            ErrorKind::BadMagic,
            0,
            format!("expected {} signature", expected.magic().escape_ascii()),
        ));
    }
    match expected {
        TaggedResourceKind::Gtex => parse_gtex(data).map(TaggedResource::Gtex),
        TaggedResourceKind::Pwib => parse_pwib(data).map(TaggedResource::Pwib),
    }
}

/// Resolve one explicitly selected visible RES entry through the proven
/// little-endian RES/txb one-mip external-source subset.
pub fn parse_selected_pwib(data: &[u8], visible_index: u32) -> Result<PwibSelection> {
    let TaggedResource::Pwib(pwib) = parse(data, TaggedResourceKind::Pwib)? else {
        unreachable!("the requested parser returns PWIB");
    };
    let first_start = usize::try_from(pwib.first_offset).map_err(|_| {
        FormatError::new(
            ErrorKind::InvalidPwibStructure,
            8,
            "PWIB first segment offset does not fit this platform",
        )
    })?;
    let first_end = usize::try_from(pwib.second_offset).map_err(|_| {
        FormatError::new(
            ErrorKind::InvalidPwibStructure,
            12,
            "PWIB second segment offset does not fit this platform",
        )
    })?;
    let first = data.get(first_start..first_end).ok_or_else(|| {
        FormatError::new(
            ErrorKind::InvalidPwibStructure,
            pwib.first_offset as u64,
            "PWIB first segment is outside the input",
        )
    })?;
    if first.len() < 0x40 || first.get(0..8) != Some(b"SEDBRES ") {
        return Err(FormatError::new(
            ErrorKind::UnsupportedPwibSelection,
            pwib.first_offset as u64,
            "selected PWIB extraction requires an SEDBRES first segment",
        ));
    }
    if first[0x0c] != 0 {
        return Err(FormatError::new(
            ErrorKind::UnsupportedPwibSelection,
            pwib.first_offset as u64 + 0x0c,
            "selected RES extraction supports little-endian first segments only",
        ));
    }

    let count =
        usize::try_from(read_le32(first, 0x30, pwib.first_offset as u64)?).map_err(|_| {
            FormatError::new(
                ErrorKind::InvalidPwibStructure,
                pwib.first_offset as u64 + 0x30,
                "RES entry count does not fit this platform",
            )
        })?;
    let directory_len = count.checked_mul(16).ok_or_else(|| {
        FormatError::new(
            ErrorKind::InvalidPwibStructure,
            pwib.first_offset as u64 + 0x30,
            "RES directory length overflows",
        )
    })?;
    let directory_end = 0x40usize.checked_add(directory_len).ok_or_else(|| {
        FormatError::new(
            ErrorKind::InvalidPwibStructure,
            pwib.first_offset as u64 + 0x30,
            "RES directory end overflows",
        )
    })?;
    if directory_end > first.len() {
        return Err(FormatError::new(
            ErrorKind::InvalidPwibStructure,
            pwib.first_offset as u64 + 0x30,
            "RES directory escapes the first segment",
        ));
    }
    let payload_base = directory_end;
    let names_offset =
        usize::try_from(read_le32(first, 0x34, pwib.first_offset as u64)?).map_err(|_| {
            FormatError::new(
                ErrorKind::InvalidPwibStructure,
                pwib.first_offset as u64 + 0x34,
                "RES names offset does not fit this platform",
            )
        })?;
    let names_count =
        usize::try_from(read_le32(first, 0x38, pwib.first_offset as u64)?).map_err(|_| {
            FormatError::new(
                ErrorKind::InvalidPwibStructure,
                pwib.first_offset as u64 + 0x38,
                "RES names count does not fit this platform",
            )
        })?;
    let names_start = payload_base.checked_add(names_offset).ok_or_else(|| {
        FormatError::new(
            ErrorKind::InvalidPwibStructure,
            pwib.first_offset as u64 + 0x34,
            "RES names start overflows",
        )
    })?;
    if names_start > first.len() {
        return Err(FormatError::new(
            ErrorKind::InvalidPwibStructure,
            pwib.first_offset as u64 + 0x34,
            "RES names start escapes the first segment",
        ));
    }
    if names_count > first.len() - names_start {
        return Err(FormatError::new(
            ErrorKind::InvalidPwibStructure,
            pwib.first_offset as u64 + 0x38,
            "RES name count exceeds the bounded names view",
        ));
    }
    let mut names = Vec::with_capacity(names_count);
    let mut names_cursor = names_start;
    for index in 0..names_count {
        let Some(relative_end) = first[names_cursor..].iter().position(|byte| *byte == 0) else {
            return Err(FormatError::new(
                ErrorKind::InvalidPwibStructure,
                pwib.first_offset as u64 + names_cursor as u64,
                format!("RES name {index} is not NUL terminated"),
            ));
        };
        let end = names_cursor + relative_end;
        let value = std::str::from_utf8(&first[names_cursor..end]).map_err(|_| {
            FormatError::new(
                ErrorKind::InvalidPwibStructure,
                pwib.first_offset as u64 + names_cursor as u64,
                format!("RES name {index} is not UTF-8"),
            )
        })?;
        names.push(value.to_string());
        names_cursor = end + 1;
    }

    let directory_span = Span::new(pwib.first_offset as u64 + 0x40, directory_len as u64);
    let names_span = Span::new(
        pwib.first_offset as u64 + names_start as u64,
        (names_cursor - names_start) as u64,
    );
    let payload_base_absolute = pwib.first_offset as u64 + payload_base as u64;
    let mut entries = Vec::with_capacity(count);
    for physical in 0..count {
        let field = 0x40 + physical * 16;
        let name_index = read_le32(first, field, pwib.first_offset as u64)?;
        let name = names.get(name_index as usize).ok_or_else(|| {
            FormatError::new(
                ErrorKind::InvalidPwibStructure,
                pwib.first_offset as u64 + field as u64,
                format!("RES entry {physical} name index {name_index} is outside the name table"),
            )
        })?;
        let declared_offset = read_le32(first, field + 4, pwib.first_offset as u64)?;
        let declared_size = read_le32(first, field + 8, pwib.first_offset as u64)?;
        let kind = read_le32(first, field + 12, pwib.first_offset as u64)?;
        let start = payload_base
            .checked_add(usize::try_from(declared_offset).map_err(|_| {
                FormatError::new(
                    ErrorKind::InvalidPwibStructure,
                    pwib.first_offset as u64 + (field + 4) as u64,
                    "RES entry offset does not fit this platform",
                )
            })?)
            .ok_or_else(|| {
                FormatError::new(
                    ErrorKind::InvalidPwibStructure,
                    pwib.first_offset as u64 + (field + 4) as u64,
                    "RES entry start overflows",
                )
            })?;
        let end = start
            .checked_add(usize::try_from(declared_size).map_err(|_| {
                FormatError::new(
                    ErrorKind::InvalidPwibStructure,
                    pwib.first_offset as u64 + (field + 8) as u64,
                    "RES entry size does not fit this platform",
                )
            })?)
            .ok_or_else(|| {
                FormatError::new(
                    ErrorKind::InvalidPwibStructure,
                    pwib.first_offset as u64 + (field + 8) as u64,
                    "RES entry end overflows",
                )
            })?;
        if end > first.len() {
            return Err(FormatError::new(
                ErrorKind::InvalidPwibStructure,
                pwib.first_offset as u64 + (field + 8) as u64,
                format!("RES entry {physical} span escapes the first segment"),
            ));
        }
        entries.push(ResEntry {
            physical_index: physical as u32,
            visible_index: None,
            name_index,
            name: name.clone(),
            declared_offset,
            declared_size,
            kind,
            source_span: Span::new(
                payload_base_absolute + u64::from(declared_offset),
                u64::from(declared_size),
            ),
        });
    }

    let metadata_type = entries
        .iter()
        .find(|entry| entry.name == "RESOURCE_TYPE")
        .cloned();
    let metadata_id = entries
        .iter()
        .find(|entry| entry.name == "RESOURCE_ID")
        .cloned();
    if entries
        .iter()
        .filter(|entry| entry.name == "RESOURCE_TYPE")
        .count()
        > 1
        || entries
            .iter()
            .filter(|entry| entry.name == "RESOURCE_ID")
            .count()
            > 1
        || metadata_type.is_none()
    {
        return Err(FormatError::new(
            ErrorKind::UnsupportedPwibSelection,
            pwib.first_offset as u64 + 0x40,
            "RES metadata does not establish one RESOURCE_TYPE block",
        ));
    }
    let metadata_type = metadata_type.expect("checked above");
    let expected_type_length = count.checked_mul(4).ok_or_else(|| {
        FormatError::new(
            ErrorKind::InvalidPwibStructure,
            metadata_type.source_span.offset,
            "RESOURCE_TYPE length overflows",
        )
    })?;
    if usize::try_from(metadata_type.source_span.length).unwrap_or(usize::MAX)
        < expected_type_length
    {
        return Err(FormatError::new(
            ErrorKind::InvalidPwibStructure,
            metadata_type.source_span.offset,
            "RESOURCE_TYPE metadata is shorter than the RES entry table",
        ));
    }
    if let Some(resource_id) = &metadata_id {
        let expected_id_length = count.checked_mul(16).ok_or_else(|| {
            FormatError::new(
                ErrorKind::InvalidPwibStructure,
                resource_id.source_span.offset,
                "RESOURCE_ID length overflows",
            )
        })?;
        if usize::try_from(resource_id.source_span.length).unwrap_or(usize::MAX)
            < expected_id_length
        {
            return Err(FormatError::new(
                ErrorKind::InvalidPwibStructure,
                resource_id.source_span.offset,
                "RESOURCE_ID metadata is shorter than the RES entry table",
            ));
        }
    }

    let metadata_count = entries
        .iter()
        .filter(|entry| entry.name == "RESOURCE_TYPE" || entry.name == "RESOURCE_ID")
        .count();
    let visible_count = count.checked_sub(metadata_count).ok_or_else(|| {
        FormatError::new(
            ErrorKind::InvalidPwibStructure,
            pwib.first_offset as u64 + 0x30,
            "RES metadata count exceeds the entry count",
        )
    })?;
    for entry in &mut entries {
        if entry.name != "RESOURCE_TYPE" && entry.name != "RESOURCE_ID" {
            // The native reader indexes the physical table and only uses
            // N-minus-metadata as an upper bound. Keep that identity here;
            // metadata entries are refused when directly selected.
            entry.visible_index = Some(entry.physical_index);
        }
    }
    if usize::try_from(visible_index).map_or(true, |index| index >= count)
        || usize::try_from(visible_index).map_or(true, |index| index >= visible_count)
    {
        return Err(FormatError::new(
            ErrorKind::InvalidPwibSelection,
            pwib.first_offset as u64 + 0x30,
            format!(
                "physical RES entry index {visible_index} is outside {visible_count} exposed entries"
            ),
        ));
    }
    let selected_entry = entries[visible_index as usize].clone();
    if selected_entry.name == "RESOURCE_TYPE" || selected_entry.name == "RESOURCE_ID" {
        return Err(FormatError::new(
            ErrorKind::UnsupportedPwibSelection,
            selected_entry.source_span.offset,
            "metadata entries are not selectable PWIB resources",
        ));
    }
    let type_offset = usize::try_from(metadata_type.source_span.offset).map_err(|_| {
        FormatError::new(
            ErrorKind::InvalidPwibStructure,
            0x30,
            "type offset does not fit",
        )
    })?;
    let type_end = type_offset
        .checked_add(4 * usize::try_from(visible_index).unwrap_or(usize::MAX))
        .and_then(|value| value.checked_add(4))
        .ok_or_else(|| {
            FormatError::new(
                ErrorKind::InvalidPwibStructure,
                metadata_type.source_span.offset,
                "type index overflows",
            )
        })?;
    let type_bytes = data.get(type_offset..type_end).ok_or_else(|| {
        FormatError::new(
            ErrorKind::InvalidPwibStructure,
            metadata_type.source_span.offset,
            "type metadata escapes the input",
        )
    })?;
    let selected_type = u32::from_le_bytes(
        type_bytes[4 * usize::try_from(visible_index).unwrap_or(0)..][..4]
            .try_into()
            .expect("four type bytes"),
    );
    if selected_type != 0x0074_7862 {
        return Err(FormatError::new(
            ErrorKind::UnsupportedPwibSelection,
            metadata_type.source_span.offset + u64::from(visible_index) * 4,
            format!("selected RESOURCE_TYPE is 0x{selected_type:08x}, expected txb"),
        ));
    }

    let txb_start = usize::try_from(selected_entry.source_span.offset).map_err(|_| {
        FormatError::new(
            ErrorKind::InvalidPwibStructure,
            selected_entry.source_span.offset,
            "txb offset does not fit",
        )
    })?;
    let txb_size = usize::try_from(selected_entry.declared_size).map_err(|_| {
        FormatError::new(
            ErrorKind::InvalidPwibStructure,
            selected_entry.source_span.offset,
            "txb size does not fit this platform",
        )
    })?;
    let txb_end = txb_start.checked_add(txb_size).ok_or_else(|| {
        FormatError::new(
            ErrorKind::InvalidPwibStructure,
            selected_entry.source_span.offset,
            "txb end overflows",
        )
    })?;
    let txb_bytes = data.get(txb_start..txb_end).ok_or_else(|| {
        FormatError::new(
            ErrorKind::InvalidPwibStructure,
            selected_entry.source_span.offset,
            "txb span escapes the input",
        )
    })?;
    if txb_bytes.len() < 0x30 || txb_bytes.get(0..8) != Some(b"SEDBtxb\0") {
        return Err(FormatError::new(
            ErrorKind::UnsupportedPwibSelection,
            selected_entry.source_span.offset,
            "selected entry is not a supported txb block",
        ));
    }
    if read_le32(txb_bytes, 0x08, selected_entry.source_span.offset)? != 1 {
        return Err(FormatError::new(
            ErrorKind::UnsupportedPwibSelection,
            selected_entry.source_span.offset + 0x08,
            "selected txb version is not one",
        ));
    }
    if txb_bytes[0x0c] != 0 {
        return Err(FormatError::new(
            ErrorKind::UnsupportedPwibSelection,
            selected_entry.source_span.offset + 0x0c,
            "selected txb extraction supports little-endian blocks only",
        ));
    }
    let word = read_le16(txb_bytes, 0x0e, selected_entry.source_span.offset)?;
    let descriptor_offset_branch = if word <= 0x30 {
        DescriptorOffsetBranch::DirectWord
    } else {
        DescriptorOffsetBranch::IndirectDword
    };
    let descriptor_offset = if word <= 0x30 {
        0x30usize.checked_add(usize::from(word)).ok_or_else(|| {
            FormatError::new(
                ErrorKind::InvalidPwibStructure,
                selected_entry.source_span.offset + 0x0e,
                "txb descriptor offset overflows",
            )
        })?
    } else {
        usize::try_from(read_le32(
            txb_bytes,
            0x30,
            selected_entry.source_span.offset,
        )?)
        .map_err(|_| {
            FormatError::new(
                ErrorKind::InvalidPwibStructure,
                selected_entry.source_span.offset + 0x30,
                "txb descriptor offset does not fit",
            )
        })?
    };
    let descriptor_pointer_field =
        (word > 0x30).then(|| Span::new(selected_entry.source_span.offset + 0x30, 4));
    let descriptor_fixed_end = descriptor_offset
        .checked_add(GTEX_FIXED_FIELDS_SIZE)
        .ok_or_else(|| {
            FormatError::new(
                ErrorKind::InvalidPwibStructure,
                selected_entry.source_span.offset + descriptor_offset as u64,
                "txb descriptor end overflows",
            )
        })?;
    if descriptor_fixed_end > txb_bytes.len() {
        return Err(FormatError::new(
            ErrorKind::InvalidPwibStructure,
            selected_entry.source_span.offset + descriptor_offset as u64,
            "txb descriptor escapes the selected block",
        ));
    }
    let descriptor_absolute = selected_entry.source_span.offset + descriptor_offset as u64;
    let descriptor_fixed_bytes = &txb_bytes[descriptor_offset..descriptor_fixed_end];
    let descriptor_table_base = u32::from_be_bytes(
        descriptor_fixed_bytes[0x10..0x14]
            .try_into()
            .expect("four table-base bytes"),
    );
    let descriptor_mips = descriptor_fixed_bytes[0x07];
    let descriptor_flags = descriptor_fixed_bytes[0x09];
    let descriptor_face_count = if descriptor_flags & 1 != 0 { 6 } else { 1 };
    let descriptor_entry_count = usize::from(descriptor_mips) * descriptor_face_count;
    let descriptor_table_length = descriptor_entry_count
        .checked_mul(SURFACE_OFFSET_ENTRY_SIZE)
        .ok_or_else(|| {
            FormatError::new(
                ErrorKind::InvalidPwibStructure,
                descriptor_absolute + 0x10,
                "GTEX descriptor table length overflows",
            )
        })?;
    if descriptor_table_base == 0 {
        return Err(FormatError::new(
            ErrorKind::UnsupportedPwibSelection,
            descriptor_absolute + 0x10,
            "selected GTEX descriptor has no surface table",
        ));
    }
    let descriptor_table_end = usize::try_from(descriptor_table_base)
        .ok()
        .and_then(|base| base.checked_add(descriptor_table_length))
        .ok_or_else(|| {
            FormatError::new(
                ErrorKind::InvalidPwibStructure,
                descriptor_absolute + 0x10,
                "GTEX descriptor table end overflows",
            )
        })?;
    let descriptor_end = descriptor_table_end.max(GTEX_FIXED_FIELDS_SIZE);
    let descriptor_view_end = descriptor_offset
        .checked_add(descriptor_end)
        .ok_or_else(|| {
            FormatError::new(
                ErrorKind::InvalidPwibStructure,
                descriptor_absolute,
                "GTEX descriptor view end overflows",
            )
        })?;
    if descriptor_view_end > txb_bytes.len() {
        return Err(FormatError::new(
            ErrorKind::InvalidPwibStructure,
            descriptor_absolute,
            "GTEX descriptor and table escape the selected block",
        ));
    }
    let descriptor_bytes = &txb_bytes[descriptor_offset..descriptor_view_end];
    let source_len = pwib.second_segment.length;
    let descriptor_format = descriptor_fixed_bytes[0x06];
    let descriptor_width =
        u16::from_be_bytes([descriptor_fixed_bytes[0x0a], descriptor_fixed_bytes[0x0b]]);
    let descriptor_height =
        u16::from_be_bytes([descriptor_fixed_bytes[0x0c], descriptor_fixed_bytes[0x0d]]);
    let descriptor_depth =
        u16::from_be_bytes([descriptor_fixed_bytes[0x0e], descriptor_fixed_bytes[0x0f]]);
    if descriptor_format != 24
        || descriptor_mips != 1
        || descriptor_flags != 0
        || descriptor_depth != 1
        || descriptor_width == 0
        || descriptor_height == 0
    {
        return Err(FormatError::new(
            ErrorKind::UnsupportedPwibSelection,
            descriptor_absolute,
            "GTEX descriptor is outside the supported one-mip DXT1 subset",
        ));
    }
    let gtex = parse_gtex_external(descriptor_bytes, source_len)?;
    if gtex.format_index != 24
        || gtex.mip_levels != 1
        || gtex.flags != 0
        || gtex.texture_kind != TextureKind::Texture2d
        || gtex.depth != 1
        || gtex.width == 0
        || gtex.height == 0
        || gtex.surfaces.len() != 1
    {
        return Err(FormatError::new(
            ErrorKind::UnsupportedPwibSelection,
            descriptor_absolute,
            "GTEX descriptor is outside the supported one-mip DXT1 subset",
        ));
    }
    let surface = gtex.surfaces[0].source_span;
    let surface_absolute = (pwib.second_offset as u64)
        .checked_add(surface.offset)
        .ok_or_else(|| {
            FormatError::new(
                ErrorKind::InvalidPwibStructure,
                pwib.second_offset as u64,
                "PWIB surface absolute offset overflows",
            )
        })?;
    let descriptor_offset_field = Span::new(selected_entry.source_span.offset + 0x0e, 2);
    Ok(PwibSelection {
        visible_index,
        pwib,
        first_directory: directory_span,
        directory_count: count as u32,
        names: names_span,
        names_count: names_count as u32,
        payload_base: payload_base_absolute,
        entries,
        metadata_count: metadata_count as u32,
        visible_count: visible_count as u32,
        resource_type: metadata_type,
        resource_id: metadata_id,
        selected_entry,
        selected_type,
        txb: Span::new(txb_start as u64, txb_bytes.len() as u64),
        descriptor_offset_field,
        descriptor_pointer_field,
        descriptor_offset_word: word,
        descriptor_offset_branch,
        descriptor_offset: descriptor_offset as u64,
        descriptor_fixed: Span::new(descriptor_absolute, GTEX_FIXED_FIELDS_SIZE as u64),
        descriptor: Span::new(descriptor_absolute, descriptor_bytes.len() as u64),
        descriptor_table: Span::new(
            descriptor_absolute + u64::from(descriptor_table_base),
            descriptor_table_length as u64,
        ),
        gtex,
        surface: Span::new(surface_absolute, surface.length),
        surface_relative_offset: surface.offset as u32,
    })
}

fn read_le16(data: &[u8], offset: usize, base: u64) -> Result<u16> {
    let bytes = data.get(offset..offset + 2).ok_or_else(|| {
        FormatError::new(
            ErrorKind::UnexpectedEndOfInput,
            base + offset as u64,
            "u16 field is truncated",
        )
    })?;
    Ok(u16::from_le_bytes(bytes.try_into().expect("two bytes")))
}

fn read_le32(data: &[u8], offset: usize, base: u64) -> Result<u32> {
    let bytes = data.get(offset..offset + 4).ok_or_else(|| {
        FormatError::new(
            ErrorKind::UnexpectedEndOfInput,
            base + offset as u64,
            "u32 field is truncated",
        )
    })?;
    Ok(u32::from_le_bytes(bytes.try_into().expect("four bytes")))
}

fn parse_gtex(data: &[u8]) -> Result<GtexResource> {
    parse_gtex_with_source(data, None)
}

/// Parse a GTEX descriptor whose encoded surface bytes live in a separately
/// bounded source view. Standalone GTEX parsing keeps its existing nonzero
/// data-base requirement; this path is only for the selected PWIB consumer.
pub fn parse_gtex_external(data: &[u8], source_len: u64) -> Result<GtexResource> {
    if data.len() < GTEX_MAGIC.len() {
        return Err(FormatError::new(
            ErrorKind::UnexpectedEndOfInput,
            data.len() as u64,
            "GTEX descriptor signature is truncated",
        ));
    }
    if !data.starts_with(GTEX_MAGIC) {
        return Err(FormatError::new(
            ErrorKind::BadMagic,
            0,
            "expected GTEX descriptor signature",
        ));
    }
    parse_gtex_with_source(data, Some(source_len))
}

fn parse_gtex_with_source(data: &[u8], external_source_len: Option<u64>) -> Result<GtexResource> {
    if data.len() < GTEX_FIXED_FIELDS_SIZE {
        return Err(FormatError::new(
            ErrorKind::UnexpectedEndOfInput,
            data.len() as u64,
            format!(
                "GTEX fixed fields are truncated: wanted {GTEX_FIXED_FIELDS_SIZE} bytes, found {}",
                data.len()
            ),
        ));
    }

    let mut reader = Reader::new(data);
    reader.seek(0x06)?;
    let format_index = reader.u8()?;
    let mip_levels = reader.u8()?;
    reader.seek(0x09)?;
    let flags = reader.u8()?;
    let width = reader.u16_be()?;
    let height = reader.u16_be()?;
    let depth = reader.u16_be()?;
    let offset_table_base = reader.u32_be()?;
    let data_base = reader.u32_be()?;

    let data_start = usize::try_from(data_base).map_err(|_| {
        FormatError::new(
            ErrorKind::DeclaredSizeOutOfRange,
            0x14,
            "GTEX data base does not fit this platform",
        )
    })?;
    if external_source_len.is_none()
        && (data_start < GTEX_FIXED_FIELDS_SIZE || data_start > data.len())
    {
        return Err(FormatError::new(
            ErrorKind::DeclaredSizeOutOfRange,
            0x14,
            format!("GTEX data base {data_base} is outside the input"),
        ));
    }
    if external_source_len.is_some() && data_base != 0 {
        return Err(FormatError::new(
            ErrorKind::InvalidPwibStructure,
            0x14,
            "external GTEX descriptors require a zero data base",
        ));
    }

    let texture_kind = if flags & 1 != 0 {
        TextureKind::Cube
    } else if flags & 2 != 0 {
        TextureKind::Volume
    } else {
        TextureKind::Texture2d
    };
    let face_count = if texture_kind == TextureKind::Cube {
        6
    } else {
        1
    };
    let entry_count = usize::from(mip_levels) * face_count;
    let format = gtex_format(format_index);
    let mut surfaces = Vec::new();
    let mut header_unknown = vec![Span::new(4, 2), Span::new(8, 1)];
    let mut data_gaps = Vec::new();
    if offset_table_base != 0 {
        let table_start = usize::try_from(offset_table_base).map_err(|_| {
            FormatError::new(
                ErrorKind::DeclaredSizeOutOfRange,
                0x10,
                "GTEX offset-table base does not fit this platform",
            )
        })?;
        let table_length = entry_count
            .checked_mul(SURFACE_OFFSET_ENTRY_SIZE)
            .ok_or_else(|| {
                FormatError::new(
                    ErrorKind::DeclaredSizeOutOfRange,
                    0x10,
                    "GTEX offset-table length overflows the address space",
                )
            })?;
        let table_end = table_start.checked_add(table_length).ok_or_else(|| {
            FormatError::new(
                ErrorKind::DeclaredSizeOutOfRange,
                0x10,
                "GTEX offset-table end overflows the address space",
            )
        })?;
        let header_end = if external_source_len.is_some() {
            data.len()
        } else {
            data_start
        };
        if table_start < GTEX_FIXED_FIELDS_SIZE || table_end > header_end {
            return Err(FormatError::new(
                ErrorKind::DeclaredSizeOutOfRange,
                0x10,
                format!(
                    "GTEX offset table [{table_start}, {table_end}) is outside the header ending at {header_end}"
                ),
            ));
        }
        if table_start > GTEX_FIXED_FIELDS_SIZE {
            header_unknown.push(Span::new(
                GTEX_FIXED_FIELDS_SIZE as u64,
                (table_start - GTEX_FIXED_FIELDS_SIZE) as u64,
            ));
        }
        for index in 0..entry_count {
            let field_offset = table_start + index * SURFACE_OFFSET_ENTRY_SIZE;
            reader.seek(field_offset)?;
            let relative_offset = reader.u32_be()?;
            let declared_size = reader.u32_be()?;
            let source = if external_source_len.is_some() {
                usize::try_from(relative_offset).map_err(|_| {
                    FormatError::new(
                        ErrorKind::DeclaredSizeOutOfRange,
                        field_offset as u64,
                        "GTEX external surface offset does not fit this platform",
                    )
                })?
            } else {
                data_start
                    .checked_add(relative_offset as usize)
                    .ok_or_else(|| {
                        FormatError::new(
                            ErrorKind::DeclaredSizeOutOfRange,
                            field_offset as u64,
                            "GTEX surface source offset overflows the address space",
                        )
                    })?
            };
            let source_limit = external_source_len.unwrap_or(data.len() as u64);
            if source as u64 > source_limit {
                return Err(FormatError::new(
                    ErrorKind::DeclaredSizeOutOfRange,
                    field_offset as u64,
                    format!("GTEX surface source offset {relative_offset} escapes the input"),
                ));
            }
            let end = source.checked_add(declared_size as usize).ok_or_else(|| {
                FormatError::new(
                    ErrorKind::DeclaredSizeOutOfRange,
                    (field_offset + 4) as u64,
                    "GTEX surface end overflows the address space",
                )
            })?;
            if end as u64 > source_limit {
                return Err(FormatError::new(
                    ErrorKind::DeclaredSizeOutOfRange,
                    (field_offset + 4) as u64,
                    format!("GTEX surface span [{source}, {end}) escapes the input"),
                ));
            }
            if let Some(previous) = surfaces.last() {
                let previous: &SurfaceEntry = previous;
                let previous_end = previous.source_span.offset + previous.source_span.length;
                if (source as u64) < previous_end {
                    return Err(FormatError::new(
                        ErrorKind::AmbiguousPayloadSpan,
                        field_offset as u64,
                        "GTEX surface spans overlap or run out of table order",
                    ));
                }
            }
            let mip_level = (index % usize::from(mip_levels.max(1))) as u8;
            let face = (index / usize::from(mip_levels.max(1))) as u8;
            let calculated_size = format.and_then(|format| {
                encoded_surface_size(
                    format,
                    u32::from(width)
                        .checked_shr(u32::from(mip_level))
                        .unwrap_or(0)
                        .max(1),
                    u32::from(height)
                        .checked_shr(u32::from(mip_level))
                        .unwrap_or(0)
                        .max(1),
                    if texture_kind == TextureKind::Volume {
                        u32::from(depth)
                            .checked_shr(u32::from(mip_level))
                            .unwrap_or(0)
                            .max(1)
                    } else {
                        1
                    },
                )
            });
            if let Some(calculated) = calculated_size {
                if calculated != u64::from(declared_size) {
                    return Err(FormatError::new(
                        ErrorKind::AmbiguousPayloadSpan,
                        (field_offset + 4) as u64,
                        format!("GTEX declared surface size {declared_size} differs from calculated size {calculated}"),
                    ));
                }
            }
            surfaces.push(SurfaceEntry {
                index: index as u32,
                face,
                mip_level,
                offset_field_span: Span::new(field_offset as u64, 4),
                size_field_span: Span::new((field_offset + 4) as u64, 4),
                relative_offset,
                declared_size,
                source_span: Span::new(source as u64, declared_size as u64),
                calculated_size,
            });
        }
        if table_end < data_start {
            header_unknown.push(Span::new(table_end as u64, (data_start - table_end) as u64));
        }
    } else if external_source_len.is_none() && data_start > GTEX_FIXED_FIELDS_SIZE {
        header_unknown.push(Span::new(
            GTEX_FIXED_FIELDS_SIZE as u64,
            (data_start - GTEX_FIXED_FIELDS_SIZE) as u64,
        ));
    }

    let mut cursor = data_start as u64;
    for surface in &surfaces {
        if surface.source_span.offset > cursor {
            data_gaps.push(Span::new(cursor, surface.source_span.offset - cursor));
        }
        cursor = surface.source_span.offset + surface.source_span.length;
    }
    if external_source_len.is_none() && cursor < data.len() as u64 {
        data_gaps.push(Span::new(cursor, data.len() as u64 - cursor));
    }

    Ok(GtexResource {
        signature: Span::new(0, 4),
        header: Span::new(
            0,
            if external_source_len.is_some() {
                data.len() as u64
            } else {
                data_base as u64
            },
        ),
        format_index,
        mip_levels,
        flags,
        texture_kind,
        width,
        height,
        depth,
        offset_table_base,
        data_base,
        format,
        surfaces,
        header_unknown,
        data_gaps,
        data: if external_source_len.is_some() {
            Span::new(0, 0)
        } else {
            Span::new(data_base as u64, (data.len() - data_start) as u64)
        },
    })
}

fn encoded_surface_size(format: GtexFormat, width: u32, height: u32, depth: u32) -> Option<u64> {
    let plane = if let Some(block_bytes) = format.block_bytes {
        u64::from(width.div_ceil(4))
            .checked_mul(u64::from(height.div_ceil(4)))?
            .checked_mul(u64::from(block_bytes))?
    } else {
        u64::from(width)
            .checked_mul(u64::from(height))?
            .checked_mul(u64::from(format.bits_per_pixel))?
            .checked_div(8)?
    };
    plane.checked_mul(u64::from(depth))
}

fn parse_pwib(data: &[u8]) -> Result<PwibResource> {
    if data.len() < PWIB_HEADER_SIZE {
        return Err(FormatError::new(
            ErrorKind::UnexpectedEndOfInput,
            data.len() as u64,
            format!(
                "PWIB header is truncated: wanted {PWIB_HEADER_SIZE} bytes, found {}",
                data.len()
            ),
        ));
    }
    let mut reader = Reader::new(data);
    reader.seek(4)?;
    let total_size = reader.u32_be()?;
    let first_offset = reader.u32_be()?;
    let second_offset = reader.u32_be()?;
    let total = total_size as usize;
    let first = first_offset as usize;
    let second = second_offset as usize;
    if first < PWIB_HEADER_SIZE || first > second || second > total || total > data.len() {
        return Err(FormatError::new(
            ErrorKind::DeclaredSizeOutOfRange,
            4,
            format!(
                "PWIB boundaries must satisfy 16 <= first ({first}) <= second ({second}) <= total ({total}) <= input ({})",
                data.len()
            ),
        ));
    }
    let first_length = second - first;
    if first_length < sedb::FIXED_HEADER_SIZE as usize {
        return Err(FormatError::new(
            ErrorKind::UnexpectedEndOfInput,
            first_offset as u64,
            "PWIB first segment cannot hold the fixed SEDB header",
        ));
    }
    let mut prefix = Reader::with_base(&data[first..second], first_offset as u64);
    let magic = prefix.take(4)?;
    if magic != sedb::MAGIC {
        return Err(FormatError::new(
            ErrorKind::BadMagic,
            first_offset as u64,
            format!("expected a SEDB signature, found '{}'", escape_tag(magic)),
        ));
    }
    let subtype = escape_tag(prefix.take(4)?);
    let unknown_a = prefix.u32_le()?;
    let flags = prefix.u16_le()?;
    let header_size = prefix.u16_le()?;
    let declared_size = prefix.u32_le()?;

    Ok(PwibResource {
        signature: Span::new(0, 4),
        header: Span::new(0, PWIB_HEADER_SIZE as u64),
        total_size,
        first_offset,
        second_offset,
        first_segment: Span::new(first_offset as u64, (second - first) as u64),
        second_segment: Span::new(second_offset as u64, (total - second) as u64),
        trailing: Span::new(total_size as u64, (data.len() - total) as u64),
        sedb_prefix: SedbPrefix {
            span: Span::new(first_offset as u64, sedb::FIXED_HEADER_SIZE as u64),
            subtype,
            unknown_a,
            flags,
            header_size,
            declared_size,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gtex() -> Vec<u8> {
        let mut bytes = vec![0x5a; 0x30];
        bytes[0..4].copy_from_slice(GTEX_MAGIC);
        bytes[6] = 4;
        bytes[7] = 2;
        bytes[9] = 1;
        bytes[0x0a..0x0c].copy_from_slice(&64u16.to_be_bytes());
        bytes[0x0c..0x0e].copy_from_slice(&32u16.to_be_bytes());
        bytes[0x0e..0x10].copy_from_slice(&1u16.to_be_bytes());
        bytes[0x10..0x14].copy_from_slice(&0u32.to_be_bytes());
        bytes[0x14..0x18].copy_from_slice(&0x20u32.to_be_bytes());
        bytes
    }

    fn pwib() -> Vec<u8> {
        let mut bytes = vec![0x3c; 0x35];
        bytes[0..4].copy_from_slice(PWIB_MAGIC);
        bytes[4..8].copy_from_slice(&0x30u32.to_be_bytes());
        bytes[8..12].copy_from_slice(&0x10u32.to_be_bytes());
        bytes[12..16].copy_from_slice(&0x28u32.to_be_bytes());
        bytes[0x10..0x18].copy_from_slice(b"SEDBsyn\0");
        bytes[0x18..0x1c].copy_from_slice(&7u32.to_le_bytes());
        bytes[0x1c..0x1e].copy_from_slice(&2u16.to_le_bytes());
        bytes[0x1e..0x20].copy_from_slice(&0x14u16.to_le_bytes());
        bytes[0x20..0x24].copy_from_slice(&0x20u32.to_le_bytes());
        bytes
    }

    fn surface_gtex(format: u8, width: u16, height: u16, entries: &[(u32, u32)]) -> Vec<u8> {
        let data_base = 0x18 + entries.len() * 8;
        let data_size = entries
            .iter()
            .map(|(offset, size)| offset + size)
            .max()
            .unwrap_or(0);
        let mut bytes = vec![0u8; data_base + data_size as usize];
        bytes[0..4].copy_from_slice(GTEX_MAGIC);
        bytes[6] = format;
        bytes[7] = entries.len() as u8;
        bytes[0x0a..0x0c].copy_from_slice(&width.to_be_bytes());
        bytes[0x0c..0x0e].copy_from_slice(&height.to_be_bytes());
        bytes[0x0e..0x10].copy_from_slice(&1u16.to_be_bytes());
        bytes[0x10..0x14].copy_from_slice(&0x18u32.to_be_bytes());
        bytes[0x14..0x18].copy_from_slice(&(data_base as u32).to_be_bytes());
        for (index, (offset, size)) in entries.iter().enumerate() {
            let start = 0x18 + index * 8;
            bytes[start..start + 4].copy_from_slice(&offset.to_be_bytes());
            bytes[start + 4..start + 8].copy_from_slice(&size.to_be_bytes());
        }
        bytes
    }

    #[test]
    fn recognizes_only_the_two_exact_tags() {
        assert_eq!(detect(b"GTEXbody"), Some(TaggedResourceKind::Gtex));
        assert_eq!(detect(b"PWIBbody"), Some(TaggedResourceKind::Pwib));
        assert_eq!(detect(b"GTExbody"), None);
    }

    #[test]
    fn reads_loader_backed_gtex_fields() {
        let TaggedResource::Gtex(parsed) = parse(&gtex(), TaggedResourceKind::Gtex).unwrap() else {
            panic!("expected GTEX");
        };
        assert_eq!(parsed.header, Span::new(0, 0x20));
        assert_eq!(parsed.data, Span::new(0x20, 0x10));
        assert_eq!(parsed.texture_kind, TextureKind::Cube);
        assert_eq!((parsed.width, parsed.height, parsed.depth), (64, 32, 1));
    }

    #[test]
    fn calculates_linear_and_block_sizes_and_preserves_gaps() {
        let TaggedResource::Gtex(linear) = parse(
            &surface_gtex(4, 4, 2, &[(0, 32), (40, 8)]),
            TaggedResourceKind::Gtex,
        )
        .unwrap() else {
            panic!("expected GTEX")
        };
        assert_eq!(linear.surfaces[0].calculated_size, Some(32));
        assert_eq!(
            linear.data_gaps,
            vec![Span::new(linear.data_base as u64 + 32, 8)]
        );

        let TaggedResource::Gtex(dxt1) = parse(
            &surface_gtex(24, 8, 8, &[(0, 32), (32, 8)]),
            TaggedResourceKind::Gtex,
        )
        .unwrap() else {
            panic!("expected GTEX")
        };
        assert_eq!(
            dxt1.surfaces
                .iter()
                .map(|entry| entry.calculated_size)
                .collect::<Vec<_>>(),
            vec![Some(32), Some(8)]
        );
    }

    #[test]
    fn rejects_surface_size_mismatch_and_overlap() {
        let mismatch =
            parse(&surface_gtex(26, 4, 4, &[(0, 8)]), TaggedResourceKind::Gtex).unwrap_err();
        assert_eq!(mismatch.kind(), ErrorKind::AmbiguousPayloadSpan);
        let overlap = parse(
            &surface_gtex(3, 4, 4, &[(0, 8), (4, 8)]),
            TaggedResourceKind::Gtex,
        )
        .unwrap_err();
        assert_eq!(overlap.kind(), ErrorKind::AmbiguousPayloadSpan);
    }

    #[test]
    fn reads_loader_backed_pwib_segments() {
        let TaggedResource::Pwib(parsed) = parse(&pwib(), TaggedResourceKind::Pwib).unwrap() else {
            panic!("expected PWIB");
        };
        assert_eq!(parsed.first_segment, Span::new(0x10, 0x18));
        assert_eq!(parsed.second_segment, Span::new(0x28, 8));
        assert_eq!(parsed.trailing, Span::new(0x30, 5));
        assert_eq!(parsed.sedb_prefix.subtype, "syn\\x00");
    }

    #[test]
    fn reports_truncation_wrong_magic_and_bad_boundaries() {
        let truncated = parse(b"GTE", TaggedResourceKind::Gtex).unwrap_err();
        assert_eq!(truncated.kind(), ErrorKind::UnexpectedEndOfInput);
        let wrong = parse(b"PWIB", TaggedResourceKind::Gtex).unwrap_err();
        assert_eq!(wrong.kind(), ErrorKind::BadMagic);
        let short = parse(b"GTEX", TaggedResourceKind::Gtex).unwrap_err();
        assert_eq!(short.kind(), ErrorKind::UnexpectedEndOfInput);
        assert_eq!(short.offset(), 4);
        let mut bad = pwib();
        bad[12..16].copy_from_slice(&0x31u32.to_be_bytes());
        let error = parse(&bad, TaggedResourceKind::Pwib).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::DeclaredSizeOutOfRange);
        assert_eq!(error.offset(), 4);
    }
}
