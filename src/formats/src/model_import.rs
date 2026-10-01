//! Experimental OBJ-to-native model blockout import.
//!
//! The importer is deliberately narrow. It accepts one RES resource with one
//! WRB, one MDL, and one MESH, and uses that native resource as the byte-level
//! template for a position/index replacement. Unknown chunks and unresolved
//! vertex fields stay in the template. This is a blockout writer, not a
//! general model or material exporter.

use std::ops::Range;

use crate::error::{ErrorKind, FormatError, Result};
use crate::reader::Span;
use crate::sedb::{self, Container, EntryBody};
use crate::zone::{self, ModelChunkInspection, Vec3};

const MAX_OBJ_VERTICES: usize = 1_000_000;
const MAX_OBJ_TRIANGLES: usize = 1_000_000;
const MAX_MODEL_VERTICES: usize = u16::MAX as usize;
const MAX_MODEL_TRIANGLES: usize = u16::MAX as usize;
const MAX_OUTPUT_BYTES: usize = 256 * 1024 * 1024;
const POSITION_USAGE: u32 = 0;
const NORMAL_USAGE: u32 = 0x0002_0000;
const INDEX_USAGE: u32 = 0x00ff_0000;

/// Rebuild one native model resource from an explicitly supplied OBJ text.
///
/// The native input is the complete RES resource. The result retains the
/// template's material, shader, and opaque chunks while replacing the one
/// supported MESH's position and index data. Every rejection carries a typed
/// error and an offset in the native or OBJ input.
pub fn import_obj_to_model(native: &[u8], obj: &str) -> Result<Vec<u8>> {
    let geometry = parse_obj(obj)?;
    let root = sedb::parse_container(native, 0)?;
    let profile = TemplateProfile::from_native(native, &root)?;
    let output = profile.rewrite(native, &geometry)?;

    let parsed = zone::parse_model(&output)?;
    if parsed.parts.len() != 1
        || parsed.parts[0].vertices.len() != geometry.vertices.len()
        || parsed.parts[0].indices.len() != geometry.indices.len()
    {
        return Err(import_error(
            0,
            "generated model counts do not match the imported OBJ",
        ));
    }
    Ok(output)
}

#[derive(Debug, Clone)]
struct ObjGeometry {
    vertices: Vec<Vec3>,
    indices: Vec<u32>,
    normals: Vec<Option<Vec3>>,
    min: Vec3,
    max: Vec3,
}

fn parse_obj(text: &str) -> Result<ObjGeometry> {
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    let mut line_offset = 0usize;

    for raw_line in text.split_inclusive('\n') {
        let line = raw_line.trim_end_matches(['\n', '\r']);
        let leading = line.len() - line.trim_start().len();
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            line_offset = line_offset
                .checked_add(raw_line.len())
                .ok_or_else(|| import_error(line_offset as u64, "OBJ offset overflows"))?;
            continue;
        }

        let mut words = trimmed.split_whitespace();
        let record = words.next().ok_or_else(|| {
            import_error((line_offset + leading) as u64, "OBJ record has no type")
        })?;
        let record_offset = (line_offset + leading) as u64;
        match record {
            "v" => {
                if vertices.len() == MAX_OBJ_VERTICES {
                    return Err(FormatError::new(
                        ErrorKind::ResourceLimitExceeded,
                        record_offset,
                        format!("OBJ has more than {MAX_OBJ_VERTICES} vertices"),
                    ));
                }
                let values = words
                    .map(|word| parse_float(word, record_offset))
                    .collect::<Result<Vec<_>>>()?;
                if values.len() != 3 {
                    return Err(import_error(
                        record_offset,
                        "OBJ vertex must have exactly three coordinates",
                    ));
                }
                vertices.push(Vec3 {
                    x: values[0],
                    y: values[1],
                    z: values[2],
                });
            }
            "f" => {
                let face_words = words.collect::<Vec<_>>();
                if face_words.len() != 3 {
                    return Err(import_error(
                        record_offset,
                        "OBJ faces must contain exactly three vertices",
                    ));
                }
                if indices.len() / 3 == MAX_OBJ_TRIANGLES {
                    return Err(FormatError::new(
                        ErrorKind::ResourceLimitExceeded,
                        record_offset,
                        format!("OBJ has more than {MAX_OBJ_TRIANGLES} triangles"),
                    ));
                }
                for word in face_words {
                    indices.push(parse_face_index(word, vertices.len(), record_offset)?);
                }
            }
            "l" | "o" | "s" => {}
            _ => {
                return Err(import_error(
                    record_offset,
                    format!("unsupported OBJ record '{record}'"),
                ));
            }
        }
        line_offset = line_offset
            .checked_add(raw_line.len())
            .ok_or_else(|| import_error(line_offset as u64, "OBJ offset overflows"))?;
    }

    if vertices.is_empty() {
        return Err(import_error(0, "OBJ has no vertices"));
    }
    if indices.is_empty() {
        return Err(import_error(0, "OBJ has no triangular faces"));
    }
    let (min, max) = bounds(&vertices)?;
    let normals = geometry_normals(&vertices, &indices)?;
    Ok(ObjGeometry {
        vertices,
        indices,
        normals,
        min,
        max,
    })
}

fn parse_float(word: &str, offset: u64) -> Result<f32> {
    let value = word.parse::<f32>().map_err(|_| {
        import_error(
            offset,
            format!("OBJ coordinate '{word}' is not a finite decimal"),
        )
    })?;
    if !value.is_finite() {
        return Err(import_error(
            offset,
            format!("OBJ coordinate '{word}' is not finite"),
        ));
    }
    Ok(value)
}

fn parse_face_index(word: &str, vertex_count: usize, offset: u64) -> Result<u32> {
    let position = word.split('/').next().unwrap_or_default();
    if position.is_empty() {
        return Err(import_error(offset, "OBJ face has an empty position index"));
    }
    let raw = position.parse::<i64>().map_err(|_| {
        import_error(
            offset,
            format!("OBJ face index '{position}' is not an integer"),
        )
    })?;
    if raw == 0 {
        return Err(import_error(offset, "OBJ face indices cannot be zero"));
    }
    let index = if raw > 0 {
        raw.checked_sub(1)
            .and_then(|value| usize::try_from(value).ok())
    } else {
        i64::try_from(vertex_count)
            .ok()
            .and_then(|count| count.checked_add(raw))
            .and_then(|value| usize::try_from(value).ok())
    };
    let Some(index) = index.filter(|index| *index < vertex_count) else {
        return Err(import_error(
            offset,
            format!("OBJ face index '{position}' is outside the vertex array"),
        ));
    };
    u32::try_from(index).map_err(|_| import_error(offset, "OBJ face index exceeds u32"))
}

fn bounds(vertices: &[Vec3]) -> Result<(Vec3, Vec3)> {
    let first = vertices[0];
    let mut min = first;
    let mut max = first;
    for vertex in &vertices[1..] {
        min.x = min.x.min(vertex.x);
        min.y = min.y.min(vertex.y);
        min.z = min.z.min(vertex.z);
        max.x = max.x.max(vertex.x);
        max.y = max.y.max(vertex.y);
        max.z = max.z.max(vertex.z);
    }
    if [min.x, min.y, min.z, max.x, max.y, max.z]
        .into_iter()
        .any(|value| !value.is_finite())
    {
        return Err(import_error(0, "OBJ bounds are not finite"));
    }
    Ok((min, max))
}

fn geometry_normals(vertices: &[Vec3], indices: &[u32]) -> Result<Vec<Option<Vec3>>> {
    let mut sums = vec![Vec3::ZERO; vertices.len()];
    let mut referenced = vec![false; vertices.len()];
    for triangle in indices.chunks_exact(3) {
        let a = vertices[triangle[0] as usize];
        let b = vertices[triangle[1] as usize];
        let c = vertices[triangle[2] as usize];
        let normal = cross(subtract(b, a), subtract(c, a));
        let length = length(normal);
        if !length.is_finite() || length <= f32::EPSILON {
            return Err(import_error(0, "OBJ contains a degenerate triangle"));
        }
        for index in triangle {
            let index = *index as usize;
            sums[index] = add(sums[index], normal);
            referenced[index] = true;
        }
    }
    Ok(sums
        .into_iter()
        .zip(referenced)
        .map(|(normal, referenced)| {
            if !referenced {
                None
            } else {
                let length = length(normal);
                (length > f32::EPSILON && length.is_finite()).then(|| scale(normal, 1.0 / length))
            }
        })
        .collect())
}

fn add(left: Vec3, right: Vec3) -> Vec3 {
    Vec3 {
        x: left.x + right.x,
        y: left.y + right.y,
        z: left.z + right.z,
    }
}

fn subtract(left: Vec3, right: Vec3) -> Vec3 {
    Vec3 {
        x: left.x - right.x,
        y: left.y - right.y,
        z: left.z - right.z,
    }
}

fn scale(value: Vec3, factor: f32) -> Vec3 {
    Vec3 {
        x: value.x * factor,
        y: value.y * factor,
        z: value.z * factor,
    }
}

fn cross(left: Vec3, right: Vec3) -> Vec3 {
    Vec3 {
        x: left.y * right.z - left.z * right.y,
        y: left.z * right.x - left.x * right.z,
        z: left.x * right.y - left.y * right.x,
    }
}

fn length(value: Vec3) -> f32 {
    (value.x * value.x + value.y * value.y + value.z * value.z).sqrt()
}

#[derive(Debug, Clone)]
struct StreamProfile {
    chunk: Span,
    padded: Span,
    data: Span,
    item_count: usize,
    stride: usize,
    field_offset: usize,
    normal_offset: Option<usize>,
}

#[derive(Debug, Clone)]
struct TemplateProfile {
    selected_start: usize,
    selected_end: usize,
    selected_slot: u32,
    root_total_size: usize,
    wrb_chunk: ModelChunkInspection,
    head_chunk: ModelChunkInspection,
    model_aabb_leaf: ModelChunkInspection,
    aabb_leaf: ModelChunkInspection,
    comp_chunk: ModelChunkInspection,
    position: StreamProfile,
    index: StreamProfile,
}

impl TemplateProfile {
    fn from_native(native: &[u8], root: &Container) -> Result<Self> {
        let (_, inspection) = zone::parse_model_with_structure(native)?;
        if root.subtype != "RES " || root.res.is_none() {
            return Err(import_error(4, "native input is not a RES resource"));
        }
        let records = directory_records(native, root)?;
        let mut wrb = Vec::new();
        for record in &records {
            if record.end > native.len() {
                continue;
            }
            let bytes = native.get(record.start..record.end).ok_or_else(|| {
                import_error(record.start as u64, "RES directory entry is outside input")
            })?;
            if bytes.get(0..4) != Some(b"SEDB") || bytes.get(4..8) != Some(b"wrb\0") {
                continue;
            }
            let child = sedb::parse_container(bytes, record.start as u64)?;
            if child.span.offset as usize != record.start
                || child.span.length as usize != record.size as usize
                || child.subtype != "wrb\\x00"
            {
                return Err(import_error(
                    record.start as u64,
                    "WRB directory entry has an unsupported slack extent",
                ));
            }
            wrb.push((record.clone(), child));
        }
        if wrb.len() != 1 {
            return Err(import_error(
                0,
                format!(
                    "native input must contain exactly one direct WRB resource, found {}",
                    wrb.len()
                ),
            ));
        }
        if inspection.resources.len() != 1 {
            return Err(import_error(
                0,
                format!(
                    "native input must contain exactly one parsed WRB resource, found {}",
                    inspection.resources.len()
                ),
            ));
        }
        let (record, child) = wrb
            .into_iter()
            .next()
            .ok_or_else(|| import_error(0, "WRB resource selection was empty"))?;
        if inspection.resources[0].span.offset as usize != record.start {
            return Err(import_error(
                record.start as u64,
                "WRB resource span does not match its RES directory entry",
            ));
        }
        reject_selected_overlaps(root, record.start, record.end)?;

        let wrb_chunks = find_chunks(&inspection.resources[0].chunks, *b"WRB\0");
        let mdls = find_chunks(&inspection.resources[0].chunks, *b"MDL\0");
        let meshes = find_chunks(&inspection.resources[0].chunks, *b"MESH");
        let comps = find_chunks(&inspection.resources[0].chunks, *b"COMP");
        if wrb_chunks.len() != 1 || mdls.len() != 1 || meshes.len() != 1 || comps.len() != 1 {
            return Err(import_error(
                record.start as u64,
                "native template must contain one WRB, one MDL, one MESH, and one COMP",
            ));
        }
        let mdl_chunk = mdls
            .first()
            .ok_or_else(|| import_error(record.start as u64, "native MDL selection was empty"))?;
        let model_aabb_roots = mdl_chunk
            .children
            .iter()
            .filter(|chunk| chunk.tag == *b"AABB")
            .collect::<Vec<_>>();
        if model_aabb_roots.len() != 1 {
            return Err(import_error(
                mdl_chunk.span.offset,
                "native MDL must contain one direct AABB branch",
            ));
        }
        let model_aabb_leaves = find_leaf_chunks(model_aabb_roots[0], *b"AABB");
        if model_aabb_leaves.len() != 1 {
            return Err(import_error(
                model_aabb_roots[0].span.offset,
                "native MDL AABB branch must contain one leaf",
            ));
        }
        let mesh_chunk = meshes[0].clone();
        let head_chunks = mesh_chunk
            .children
            .iter()
            .filter(|chunk| chunk.tag == *b"HEAD")
            .cloned()
            .collect::<Vec<_>>();
        let aabb_leaves = mesh_chunk
            .children
            .iter()
            .flat_map(|chunk| find_leaf_chunks(chunk, *b"AABB"))
            .collect::<Vec<_>>();
        if head_chunks.len() != 1 || aabb_leaves.len() != 1 {
            return Err(import_error(
                mesh_chunk.span.offset,
                "native MESH must contain one HEAD and one AABB leaf",
            ));
        }
        let streams = mesh_chunk
            .children
            .iter()
            .filter(|chunk| chunk.tag == *b"STMS")
            .cloned()
            .collect::<Vec<_>>();
        let mut positions = Vec::new();
        let mut indices = Vec::new();
        for stream in streams {
            let metadata = stream.stream.as_ref().ok_or_else(|| {
                import_error(stream.span.offset, "MESH STMS stream has no metadata")
            })?;
            let descriptors = metadata
                .descriptors
                .iter()
                .map(|field| field.words)
                .collect::<Vec<_>>();
            if is_index_stream(metadata.stride, &descriptors) {
                indices.push(make_stream_profile(&stream, 0, None, false)?);
                continue;
            }
            let position_fields = descriptors
                .iter()
                .filter(|field| field[1] == 4 && field[2] == 4 && field[3] == POSITION_USAGE)
                .collect::<Vec<_>>();
            if position_fields.len() != 1 {
                return Err(import_error(
                    stream.span.offset,
                    "native MESH contains an unsupported extra STMS stream",
                ));
            }
            let field = *position_fields[0];
            let field_offset = usize::try_from(field[0]).map_err(|_| {
                import_error(
                    stream.span.offset,
                    "native position field offset does not fit usize",
                )
            })?;
            let normal_fields = descriptors
                .iter()
                .filter(|field| field[1] == 3 && field[2] == 4 && field[3] == NORMAL_USAGE)
                .collect::<Vec<_>>();
            if normal_fields.len() > 1 {
                return Err(import_error(
                    stream.span.offset,
                    "native position stream has ambiguous normal fields",
                ));
            }
            let normal_field = normal_fields.first().map(|field| **field);
            let normal_offset = normal_field
                .map(|field| usize::try_from(field[0]))
                .transpose()
                .map_err(|_| {
                    import_error(
                        stream.span.offset,
                        "native normal field offset does not fit usize",
                    )
                })?;
            let stride = usize::try_from(metadata.stride).map_err(|_| {
                import_error(
                    stream.span.offset,
                    "native stream stride does not fit usize",
                )
            })?;
            let position_range =
                descriptor_range(field, stride, stream.span.offset, "native position field")?;
            let normal_range = normal_field
                .map(|field| {
                    descriptor_range(field, stride, stream.span.offset, "native normal field")
                })
                .transpose()?;
            if normal_range
                .as_ref()
                .is_some_and(|normal| ranges_overlap(normal, &position_range))
            {
                return Err(import_error(
                    stream.span.offset,
                    "native normal field overlaps the position field",
                ));
            }
            for descriptor in &descriptors {
                if *descriptor == field || Some(*descriptor) == normal_field {
                    continue;
                }
                let range = descriptor_range(
                    *descriptor,
                    stride,
                    stream.span.offset,
                    "native vertex field",
                )?;
                if ranges_overlap(&range, &position_range)
                    || normal_range
                        .as_ref()
                        .is_some_and(|normal| ranges_overlap(&range, normal))
                {
                    return Err(import_error(
                        stream.span.offset,
                        "native unresolved vertex field overlaps generated data",
                    ));
                }
            }
            positions.push(make_stream_profile(
                &stream,
                field_offset,
                normal_offset,
                true,
            )?);
        }
        if positions.len() != 1 || indices.len() != 1 {
            return Err(import_error(
                mesh_chunk.span.offset,
                "native MESH must contain one supported position and one index stream",
            ));
        }
        let position = positions.pop().ok_or_else(|| {
            import_error(
                mesh_chunk.span.offset,
                "position stream selection was empty",
            )
        })?;
        let index = indices.pop().ok_or_else(|| {
            import_error(mesh_chunk.span.offset, "index stream selection was empty")
        })?;
        if position.item_count == 0 || position.stride < position.field_offset + 8 {
            return Err(import_error(
                position.chunk.offset,
                "native position stream has an invalid item layout",
            ));
        }
        if index.item_count % 3 != 0 {
            return Err(import_error(
                index.chunk.offset,
                "native index stream is not a triangle list",
            ));
        }
        if child.total_size as usize != record.size as usize {
            return Err(import_error(
                record.start as u64,
                "native WRB resource has an unsupported trailing extent",
            ));
        }
        Ok(Self {
            selected_start: record.start,
            selected_end: record.end,
            selected_slot: record.slot,
            root_total_size: root.total_size as usize,
            wrb_chunk: wrb_chunks[0].clone(),
            head_chunk: head_chunks[0].clone(),
            model_aabb_leaf: model_aabb_leaves[0].clone(),
            aabb_leaf: aabb_leaves[0].clone(),
            comp_chunk: comps[0].clone(),
            position,
            index,
        })
    }

    fn rewrite(&self, native: &[u8], geometry: &ObjGeometry) -> Result<Vec<u8>> {
        if geometry.vertices.len() > MAX_MODEL_VERTICES {
            return Err(import_error(
                self.position.chunk.offset,
                format!("OBJ vertex count exceeds native u16 limit {MAX_MODEL_VERTICES}"),
            ));
        }
        if geometry.indices.len() / 3 > MAX_MODEL_TRIANGLES {
            return Err(import_error(
                self.index.chunk.offset,
                format!("OBJ triangle count exceeds native u16 limit {MAX_MODEL_TRIANGLES}"),
            ));
        }
        validate_native_bounds(geometry.min, geometry.max, self.comp_chunk.span.offset)?;
        preflight_output_budget(native.len(), &self.position, &self.index)?;
        let position_data = build_position_data(native, &self.position, geometry)?;
        let index_data = build_index_data(&self.index, geometry)?;
        let head = replace_head(native, &self.head_chunk, geometry)?;
        let model_aabb = replace_bounds(native, &self.model_aabb_leaf, geometry.min, geometry.max)?;
        let aabb = replace_bounds(native, &self.aabb_leaf, geometry.min, geometry.max)?;
        let comp = replace_bounds(native, &self.comp_chunk, geometry.min, geometry.max)?;
        let position = replace_stream_data(native, &self.position, position_data)?;
        let index = replace_stream_data(native, &self.index, index_data)?;
        let replacements = Replacements {
            head: (self.head_chunk.span, head),
            model_aabb: (self.model_aabb_leaf.span, model_aabb),
            aabb: (self.aabb_leaf.span, aabb),
            comp: (self.comp_chunk.span, comp),
            position: (self.position.chunk, position),
            index: (self.index.chunk, index),
        };
        let wrb = rewrite_chunk(native, &self.wrb_chunk, &replacements)?;
        let root_bytes = replace_wrb_resource(native, self, &wrb)?;
        if root_bytes.len() < self.root_total_size {
            return Err(import_error(
                0,
                "generated RES is shorter than its original extent",
            ));
        }
        Ok(root_bytes)
    }
}

#[derive(Debug, Clone)]
struct DirectoryRecord {
    slot: u32,
    size: u32,
    start: usize,
    end: usize,
}

fn directory_records(native: &[u8], root: &Container) -> Result<Vec<DirectoryRecord>> {
    let res = root
        .res
        .as_ref()
        .ok_or_else(|| import_error(4, "root is not RES"))?;
    let count = usize::try_from(res.subresource_count)
        .map_err(|_| import_error(0x30, "RES subresource count does not fit usize"))?;
    let directory_start = usize::from(root.header_size);
    let directory_bytes = count
        .checked_mul(16)
        .ok_or_else(|| import_error(directory_start as u64, "RES directory size overflows"))?;
    let payload_base = directory_start
        .checked_add(directory_bytes)
        .ok_or_else(|| import_error(directory_start as u64, "RES payload base overflows"))?;
    let mut records = Vec::with_capacity(count);
    for slot in 0..count {
        let at = directory_start + slot * 16;
        let _index = read_u32_le(native, at)?;
        let offset = read_u32_le(native, at + 4)?;
        let size = read_u32_le(native, at + 8)?;
        let _kind = read_u32_le(native, at + 12)?;
        let start = payload_base
            .checked_add(usize::try_from(offset).map_err(|_| {
                import_error((at + 4) as u64, "RES subresource offset does not fit usize")
            })?)
            .ok_or_else(|| import_error((at + 4) as u64, "RES subresource offset overflows"))?;
        let end = start
            .checked_add(usize::try_from(size).map_err(|_| {
                import_error((at + 8) as u64, "RES subresource size does not fit usize")
            })?)
            .ok_or_else(|| import_error((at + 8) as u64, "RES subresource size overflows"))?;
        if start > native.len() {
            return Err(import_error(
                at as u64,
                "RES subresource starts outside input",
            ));
        }
        records.push(DirectoryRecord {
            slot: slot as u32,
            size,
            start,
            end,
        });
    }
    Ok(records)
}

fn reject_selected_overlaps(
    root: &Container,
    selected_start: usize,
    selected_end: usize,
) -> Result<()> {
    for entry in &root.entries {
        if let EntryBody::Subresource { .. } = entry.body {
            let start = entry.span.offset as usize;
            let end = entry.span.end() as usize;
            if start < selected_end
                && selected_start < end
                && !(start == selected_start && end == selected_end)
            {
                return Err(import_error(
                    entry.span.offset,
                    "RES directory extent overlaps the selected WRB resource",
                ));
            }
        }
    }
    Ok(())
}

fn find_chunks(chunks: &[ModelChunkInspection], tag: [u8; 4]) -> Vec<ModelChunkInspection> {
    let mut found = Vec::new();
    for chunk in chunks {
        if chunk.tag == tag {
            found.push(chunk.clone());
        }
        found.extend(find_chunks(&chunk.children, tag));
    }
    found
}

fn find_leaf_chunks(chunk: &ModelChunkInspection, tag: [u8; 4]) -> Vec<ModelChunkInspection> {
    let mut found = Vec::new();
    if chunk.tag == tag && chunk.children.is_empty() {
        found.push(chunk.clone());
    }
    for child in &chunk.children {
        found.extend(find_leaf_chunks(child, tag));
    }
    found
}

fn is_index_stream(stride: u32, descriptors: &[[u32; 4]]) -> bool {
    stride == 2 && descriptors == [[0, 0, 1, INDEX_USAGE]]
}

fn descriptor_range(
    field: [u32; 4],
    stride: usize,
    offset: u64,
    label: &str,
) -> Result<Range<usize>> {
    let start = usize::try_from(field[0])
        .map_err(|_| import_error(offset, format!("{label} offset does not fit usize")))?;
    if start >= stride {
        return Err(import_error(
            offset,
            format!("{label} offset is outside its stream stride"),
        ));
    }
    let Some(bytes_per_component) = (match field[1] {
        3 => Some(1usize),
        4 => Some(2usize),
        _ => None,
    }) else {
        return Ok(start..stride);
    };
    let count = usize::try_from(field[2])
        .map_err(|_| import_error(offset, format!("{label} count does not fit usize")))?;
    let byte_count = count
        .checked_mul(bytes_per_component)
        .ok_or_else(|| import_error(offset, format!("{label} byte range overflows")))?;
    let end = start
        .checked_add(byte_count)
        .ok_or_else(|| import_error(offset, format!("{label} byte range overflows")))?;
    if end > stride {
        return Err(import_error(
            offset,
            format!("{label} extends beyond its stream stride"),
        ));
    }
    Ok(start..end)
}

fn ranges_overlap(left: &Range<usize>, right: &Range<usize>) -> bool {
    left.start < right.end && right.start < left.end
}

fn make_stream_profile(
    chunk: &ModelChunkInspection,
    field_offset: usize,
    normal_offset: Option<usize>,
    validate_position: bool,
) -> Result<StreamProfile> {
    let metadata = chunk
        .stream
        .as_ref()
        .ok_or_else(|| import_error(chunk.span.offset, "stream metadata is missing"))?;
    let stride = usize::try_from(metadata.stride).map_err(|_| {
        import_error(
            metadata.header.offset + 8,
            "stream stride does not fit usize",
        )
    })?;
    let item_count = usize::try_from(metadata.item_count).map_err(|_| {
        import_error(
            metadata.header.offset + 4,
            "stream item count does not fit usize",
        )
    })?;
    if stride == 0 && item_count != 0 {
        return Err(import_error(
            metadata.header.offset + 8,
            "stream stride is zero",
        ));
    }
    let data_length = item_count
        .checked_mul(stride)
        .ok_or_else(|| import_error(metadata.data.offset, "stream data length overflows"))?;
    if data_length as u64 != metadata.data.length {
        return Err(import_error(
            metadata.data.offset,
            "stream data span is inconsistent",
        ));
    }
    if validate_position && field_offset.checked_add(8).is_none_or(|end| end > stride) {
        return Err(import_error(
            metadata.header.offset,
            "stream field extends beyond stride",
        ));
    }
    if let Some(normal_offset) = normal_offset {
        if normal_offset.checked_add(4).is_none_or(|end| end > stride) {
            return Err(import_error(
                metadata.header.offset,
                "normal field extends beyond stride",
            ));
        }
    }
    Ok(StreamProfile {
        chunk: chunk.span,
        padded: chunk.padded_span,
        data: metadata.data,
        item_count,
        stride,
        field_offset,
        normal_offset,
    })
}

struct Replacements {
    head: (Span, Vec<u8>),
    model_aabb: (Span, Vec<u8>),
    aabb: (Span, Vec<u8>),
    comp: (Span, Vec<u8>),
    position: (Span, Vec<u8>),
    index: (Span, Vec<u8>),
}

impl Replacements {
    fn get(&self, span: Span) -> Option<&[u8]> {
        for (candidate, bytes) in [
            &self.head,
            &self.model_aabb,
            &self.aabb,
            &self.comp,
            &self.position,
            &self.index,
        ] {
            if *candidate == span {
                return Some(bytes);
            }
        }
        None
    }

    fn intersects(&self, span: Span) -> bool {
        [
            &self.head.0,
            &self.model_aabb.0,
            &self.aabb.0,
            &self.comp.0,
            &self.position.0,
            &self.index.0,
        ]
        .into_iter()
        .any(|candidate| spans_overlap(*candidate, span))
    }
}

fn rewrite_chunk(
    native: &[u8],
    chunk: &ModelChunkInspection,
    replacements: &Replacements,
) -> Result<Vec<u8>> {
    if let Some(replacement) = replacements.get(chunk.span) {
        return Ok(replacement.to_vec());
    }
    if !replacements.intersects(chunk.span) {
        return copy_span(native, chunk.padded_span);
    }
    let start = span_start(chunk.span)?;
    let logical_end = span_end(chunk.span)?;
    let padded_end = span_end(chunk.padded_span)?;
    if start + 16 > logical_end || padded_end > native.len() {
        return Err(import_error(
            chunk.span.offset,
            "chunk header span is outside input",
        ));
    }
    let mut output = native[start..start + 16].to_vec();
    let mut cursor = start + 16;
    for child in &chunk.children {
        let child_start = span_start(child.span)?;
        let child_padded_end = span_end(child.padded_span)?;
        if child_start < cursor || child_padded_end > logical_end {
            return Err(import_error(
                child.span.offset,
                "chunk child span is outside its parent",
            ));
        }
        output.extend_from_slice(&native[cursor..child_start]);
        if replacements.intersects(child.span) {
            output.extend_from_slice(&rewrite_chunk(native, child, replacements)?);
        } else {
            output.extend_from_slice(&native[child_start..child_padded_end]);
        }
        cursor = child_padded_end;
    }
    output.extend_from_slice(&native[cursor..logical_end]);
    finish_chunk(native, output, chunk)
}

fn finish_chunk(native: &[u8], mut output: Vec<u8>, old: &ModelChunkInspection) -> Result<Vec<u8>> {
    let logical_size = output.len();
    let minimum_padded_size = round_up_16(logical_size)?;
    let old_logical_end = span_end(old.span)?;
    let old_padded_end = span_end(old.padded_span)?;
    let old_padding = native
        .get(old_logical_end..old_padded_end)
        .ok_or_else(|| import_error(old.span.offset, "chunk padding is outside input"))?;
    let preserved_padded_size = logical_size
        .checked_add(old_padding.len())
        .ok_or_else(|| import_error(old.span.offset, "chunk padded size overflows"))?;
    let padded_size = minimum_padded_size.max(preserved_padded_size);
    output[8..12].copy_from_slice(
        &u32::try_from(logical_size)
            .map_err(|_| import_error(old.span.offset + 8, "chunk size exceeds u32"))?
            .to_be_bytes(),
    );
    output[12..16].copy_from_slice(
        &u32::try_from(padded_size)
            .map_err(|_| import_error(old.span.offset + 12, "chunk padded size exceeds u32"))?
            .to_be_bytes(),
    );
    let padding_length = padded_size - logical_size;
    output.extend_from_slice(&old_padding[..padding_length.min(old_padding.len())]);
    output.resize(padded_size, 0);
    Ok(output)
}

fn replace_head(
    native: &[u8],
    chunk: &ModelChunkInspection,
    geometry: &ObjGeometry,
) -> Result<Vec<u8>> {
    let logical_length = usize::try_from(chunk.span.length)
        .map_err(|_| import_error(chunk.span.offset, "HEAD chunk length does not fit usize"))?;
    if logical_length < 16 + 12 {
        return Err(import_error(
            chunk.span.offset,
            "MESH HEAD logical payload is too short",
        ));
    }
    let mut output = copy_span(native, chunk.padded_span)?;
    let payload = 16usize;
    output[payload + 8..payload + 10].copy_from_slice(
        &u16::try_from(geometry.vertices.len())
            .map_err(|_| import_error(chunk.span.offset + 24, "vertex count exceeds HEAD u16"))?
            .to_be_bytes(),
    );
    output[payload + 10..payload + 12].copy_from_slice(
        &u16::try_from(geometry.indices.len() / 3)
            .map_err(|_| import_error(chunk.span.offset + 26, "triangle count exceeds HEAD u16"))?
            .to_be_bytes(),
    );
    Ok(output)
}

fn replace_bounds(
    native: &[u8],
    chunk: &ModelChunkInspection,
    min: Vec3,
    max: Vec3,
) -> Result<Vec<u8>> {
    let logical_length = usize::try_from(chunk.span.length)
        .map_err(|_| import_error(chunk.span.offset, "bounds chunk length does not fit usize"))?;
    if logical_length < 16 + 24 {
        return Err(import_error(
            chunk.span.offset,
            "bounds chunk logical payload is too short",
        ));
    }
    let mut output = copy_span(native, chunk.padded_span)?;
    for (index, value) in [min.x, min.y, min.z, max.x, max.y, max.z]
        .into_iter()
        .enumerate()
    {
        let at = 16 + index * 4;
        output[at..at + 4].copy_from_slice(&value.to_bits().to_be_bytes());
    }
    Ok(output)
}

fn validate_native_bounds(min: Vec3, max: Vec3, offset: u64) -> Result<()> {
    for (axis, (low, high)) in [
        ("x", (min.x, max.x)),
        ("y", (min.y, max.y)),
        ("z", (min.z, max.z)),
    ] {
        let sum = low + high;
        let extent = high - low;
        let center = sum * 0.5;
        let half_extent = extent * 0.5;
        if !sum.is_finite()
            || !extent.is_finite()
            || !center.is_finite()
            || !half_extent.is_finite()
        {
            return Err(import_error(
                offset,
                format!("OBJ {axis} bounds are not representable by native decode"),
            ));
        }
    }
    Ok(())
}

fn preflight_output_budget(
    native_length: usize,
    position: &StreamProfile,
    index: &StreamProfile,
) -> Result<()> {
    let max_position_bytes = MAX_MODEL_VERTICES
        .checked_mul(position.stride)
        .ok_or_else(|| resource_error(position.chunk.offset, "position output size overflows"))?;
    let max_index_items = MAX_MODEL_TRIANGLES
        .checked_mul(3)
        .ok_or_else(|| resource_error(index.chunk.offset, "index output item count overflows"))?;
    let max_index_bytes = max_index_items
        .checked_mul(index.stride)
        .ok_or_else(|| resource_error(index.chunk.offset, "index output size overflows"))?;
    let required = native_length
        .checked_add(max_position_bytes)
        .and_then(|size| size.checked_add(max_index_bytes))
        .ok_or_else(|| resource_error(position.chunk.offset, "maximum output size overflows"))?;
    if required > MAX_OUTPUT_BYTES {
        return Err(resource_error(
            position.chunk.offset,
            format!("native template exceeds the {MAX_OUTPUT_BYTES}-byte output budget"),
        ));
    }
    Ok(())
}

fn resource_error(offset: u64, detail: impl Into<String>) -> FormatError {
    FormatError::new(ErrorKind::ResourceLimitExceeded, offset, detail)
}

fn build_position_data(
    native: &[u8],
    stream: &StreamProfile,
    geometry: &ObjGeometry,
) -> Result<Vec<u8>> {
    let old_data = copy_span(native, stream.data)?;
    let mut output = Vec::with_capacity(geometry.vertices.len() * stream.stride);
    for (index, vertex) in geometry.vertices.iter().copied().enumerate() {
        let source_index = index.min(stream.item_count - 1);
        let source_start = source_index * stream.stride;
        let mut record = old_data[source_start..source_start + stream.stride].to_vec();
        for (component, value) in [vertex.x, vertex.y, vertex.z].into_iter().enumerate() {
            let raw = quantize_component(value, geometry.min, geometry.max, component);
            let decoded = decode_component(raw, geometry.min, geometry.max, component);
            if !decoded.is_finite() {
                return Err(import_error(
                    stream.data.offset,
                    "quantized vertex is not finite under native decode",
                ));
            }
            let at = stream.field_offset + component * 2;
            record[at..at + 2].copy_from_slice(&raw.to_be_bytes());
        }
        if let (Some(normal_offset), Some(normal)) = (stream.normal_offset, geometry.normals[index])
        {
            for (component, value) in [normal.x, normal.y, normal.z].into_iter().enumerate() {
                let encoded = ((value + 1.0) * 127.5).round().clamp(0.0, 255.0) as u8;
                record[normal_offset + component] = encoded;
            }
        }
        output.extend_from_slice(&record);
    }
    Ok(output)
}

fn decode_component(raw: i16, min: Vec3, max: Vec3, component: usize) -> f32 {
    let (low, high) = match component {
        0 => (min.x, max.x),
        1 => (min.y, max.y),
        _ => (min.z, max.z),
    };
    (low + high) * 0.5 + (raw as f32 / 32767.0) * (high - low) * 0.5
}

fn quantize_component(value: f32, min: Vec3, max: Vec3, component: usize) -> i16 {
    let (low, high) = match component {
        0 => (min.x, max.x),
        1 => (min.y, max.y),
        _ => (min.z, max.z),
    };
    let center = (low + high) * 0.5;
    let half = (high - low) * 0.5;
    if half == 0.0 {
        return 0;
    }
    ((value - center) / half * 32767.0)
        .round()
        .clamp(i16::MIN as f32, i16::MAX as f32) as i16
}

fn build_index_data(stream: &StreamProfile, geometry: &ObjGeometry) -> Result<Vec<u8>> {
    let mut output = Vec::with_capacity(geometry.indices.len() * 2);
    for index in &geometry.indices {
        let index = u16::try_from(*index)
            .map_err(|_| import_error(stream.chunk.offset, "OBJ index exceeds native u16"))?;
        output.extend_from_slice(&index.to_be_bytes());
    }
    Ok(output)
}

fn replace_stream_data(native: &[u8], stream: &StreamProfile, data: Vec<u8>) -> Result<Vec<u8>> {
    let start = span_start(stream.chunk)?;
    let data_start = span_start(stream.data)?;
    let old_end = span_end(stream.chunk)?;
    if data_start < start + 16 || data_start > old_end {
        return Err(import_error(
            stream.chunk.offset,
            "stream data span is outside chunk",
        ));
    }
    let header_and_descriptors = native
        .get(start..data_start)
        .ok_or_else(|| import_error(stream.chunk.offset, "stream header is outside input"))?;
    let mut output = header_and_descriptors.to_vec();
    let item_count = data.len() / stream.stride;
    if !data.len().is_multiple_of(stream.stride) {
        return Err(import_error(
            stream.data.offset,
            "stream data is not stride aligned",
        ));
    }
    output[16 + 4..16 + 8].copy_from_slice(
        &u32::try_from(item_count)
            .map_err(|_| import_error(stream.chunk.offset + 20, "stream item count exceeds u32"))?
            .to_be_bytes(),
    );
    output.extend_from_slice(&data);
    let old = ModelChunkInspection {
        tag: *b"STMS",
        span: stream.chunk,
        declared_size: 0,
        declared_padded_size: 0,
        padded_span: stream.padded,
        payload: Span::new(
            stream.chunk.offset + 16,
            stream.chunk.length.saturating_sub(16),
        ),
        padding: None,
        descriptor: false,
        stream: None,
        mesh: None,
        children: Vec::new(),
        opaque: Vec::new(),
    };
    finish_chunk(native, output, &old)
}

fn replace_wrb_resource(native: &[u8], profile: &TemplateProfile, wrb: &[u8]) -> Result<Vec<u8>> {
    let selected_start = profile.selected_start;
    let selected_end = profile.selected_end;
    let child_header_size = read_u16_le(native, selected_start + 0x0e)? as usize;
    let child_header_end = selected_start.checked_add(child_header_size);
    if child_header_size < 0x14 || child_header_end.is_none_or(|end| end > selected_end) {
        return Err(import_error(
            (selected_start + 0x0e) as u64,
            "WRB child header is invalid",
        ));
    }
    let old_wrb_end = span_end(profile.wrb_chunk.span)?;
    let old_wrb_padded_end = span_end(profile.wrb_chunk.padded_span)?;
    if old_wrb_end > selected_end || old_wrb_padded_end > selected_end {
        return Err(import_error(
            profile.wrb_chunk.span.offset,
            "WRB chunk escapes its resource",
        ));
    }
    let mut child = Vec::new();
    child.extend_from_slice(
        native
            .get(selected_start..span_start(profile.wrb_chunk.span)?)
            .ok_or_else(|| import_error(selected_start as u64, "WRB header is outside input"))?,
    );
    child.extend_from_slice(wrb);
    child.extend_from_slice(
        native
            .get(old_wrb_padded_end..selected_end)
            .ok_or_else(|| {
                import_error(
                    selected_start as u64,
                    "WRB trailing bytes are outside input",
                )
            })?,
    );
    let child_size = u32::try_from(child.len())
        .map_err(|_| import_error((selected_start + 0x10) as u64, "WRB child size exceeds u32"))?;
    child[0x10..0x14].copy_from_slice(&child_size.to_le_bytes());
    let delta = i64::try_from(child.len())
        .ok()
        .and_then(|new| {
            i64::try_from(selected_end - selected_start)
                .ok()
                .map(|old| new - old)
        })
        .ok_or_else(|| import_error(selected_start as u64, "WRB relocation delta overflows"))?;
    let output_length = add_delta(native.len(), delta)?;
    if output_length > MAX_OUTPUT_BYTES {
        return Err(resource_error(
            selected_start as u64,
            format!("generated RES exceeds the {MAX_OUTPUT_BYTES}-byte output budget"),
        ));
    }
    let mut output = Vec::with_capacity(output_length);
    output.extend_from_slice(&native[..selected_start]);
    output.extend_from_slice(&child);
    output.extend_from_slice(&native[selected_end..]);
    let new_root_total = add_delta(profile.root_total_size, delta)?;
    output[0x10..0x14].copy_from_slice(
        &u32::try_from(new_root_total)
            .map_err(|_| import_error(0x10, "RES total size exceeds u32"))?
            .to_le_bytes(),
    );
    let root = sedb::parse_container(native, 0)?;
    let records = directory_records(native, &root)?;
    let res = root
        .res
        .as_ref()
        .ok_or_else(|| import_error(4, "root is not RES"))?;
    let directory_bytes = usize::try_from(res.subresource_count)
        .ok()
        .and_then(|count| count.checked_mul(16))
        .ok_or_else(|| import_error(root.header_size as u64, "RES directory size overflows"))?;
    let payload_base = usize::from(root.header_size)
        .checked_add(directory_bytes)
        .ok_or_else(|| import_error(root.header_size as u64, "RES payload base overflows"))?;
    for record in records {
        let at = usize::from(root.header_size) + record.slot as usize * 16;
        let new_start = if record.slot == profile.selected_slot {
            selected_start
        } else if record.start >= selected_end {
            map_after(record.start, delta)?
        } else {
            record.start
        };
        let relative = new_start
            .checked_sub(payload_base)
            .ok_or_else(|| import_error(at as u64 + 4, "RES subresource offset underflows"))?;
        output[at + 4..at + 8].copy_from_slice(
            &u32::try_from(relative)
                .map_err(|_| import_error(at as u64 + 4, "RES subresource offset exceeds u32"))?
                .to_le_bytes(),
        );
        let size = if record.slot == profile.selected_slot {
            child.len()
        } else {
            usize::try_from(record.size)
                .map_err(|_| import_error(at as u64 + 8, "RES size does not fit usize"))?
        };
        output[at + 8..at + 12].copy_from_slice(
            &u32::try_from(size)
                .map_err(|_| import_error(at as u64 + 8, "RES subresource size exceeds u32"))?
                .to_le_bytes(),
        );
    }
    Ok(output)
}

fn map_after(start: usize, delta: i64) -> Result<usize> {
    let start =
        i64::try_from(start).map_err(|_| import_error(start as u64, "offset exceeds i64"))?;
    let mapped = start
        .checked_add(delta)
        .ok_or_else(|| import_error(start as u64, "relocated offset overflows"))?;
    usize::try_from(mapped)
        .map_err(|_| import_error(start as u64, "relocated offset does not fit usize"))
}

fn add_delta(value: usize, delta: i64) -> Result<usize> {
    let value_i64 =
        i64::try_from(value).map_err(|_| import_error(value as u64, "size exceeds i64"))?;
    let adjusted = value_i64
        .checked_add(delta)
        .ok_or_else(|| import_error(value as u64, "relocated size overflows"))?;
    usize::try_from(adjusted).map_err(|_| import_error(value as u64, "relocated size is negative"))
}

fn copy_span(data: &[u8], span: Span) -> Result<Vec<u8>> {
    let range = span_range(span)?;
    data.get(range.clone())
        .map(ToOwned::to_owned)
        .ok_or_else(|| import_error(span.offset, "span is outside input"))
}

fn span_range(span: Span) -> Result<Range<usize>> {
    let start = usize::try_from(span.offset)
        .map_err(|_| import_error(span.offset, "span offset does not fit usize"))?;
    let length = usize::try_from(span.length)
        .map_err(|_| import_error(span.offset, "span length does not fit usize"))?;
    let end = start
        .checked_add(length)
        .ok_or_else(|| import_error(span.offset, "span end overflows"))?;
    Ok(start..end)
}

fn span_start(span: Span) -> Result<usize> {
    usize::try_from(span.offset)
        .map_err(|_| import_error(span.offset, "span offset does not fit usize"))
}

fn span_end(span: Span) -> Result<usize> {
    let range = span_range(span)?;
    Ok(range.end)
}

fn round_up_16(value: usize) -> Result<usize> {
    value
        .checked_add(15)
        .map(|value| value & !15)
        .ok_or_else(|| import_error(value as u64, "chunk size alignment overflows"))
}

fn spans_overlap(left: Span, right: Span) -> bool {
    left.offset < right.end() && right.offset < left.end()
}

fn read_u16_le(data: &[u8], offset: usize) -> Result<u16> {
    let bytes = data
        .get(offset..offset + 2)
        .ok_or_else(|| import_error(offset as u64, "u16 field is outside input"))?;
    Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn read_u32_le(data: &[u8], offset: usize) -> Result<u32> {
    let bytes = data
        .get(offset..offset + 4)
        .ok_or_else(|| import_error(offset as u64, "u32 field is outside input"))?;
    Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn import_error(offset: u64, detail: impl Into<String>) -> FormatError {
    FormatError::new(ErrorKind::InvalidAttributeValue, offset, detail)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::zone::parse_model;

    fn put_be_u32(bytes: &mut [u8], offset: usize, value: u32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
    }

    fn put_be_f32(bytes: &mut [u8], offset: usize, value: f32) {
        put_be_u32(bytes, offset, value.to_bits());
    }

    fn chunk(tag: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        chunk_with_padding(tag, payload, 0)
    }

    fn chunk_with_padding(tag: &[u8; 4], payload: &[u8], pad_extra: usize) -> Vec<u8> {
        let size = 16 + payload.len();
        let padded = (size + 15) & !15;
        let mut bytes = vec![0u8; padded + pad_extra];
        bytes[..4].copy_from_slice(tag);
        put_be_u32(&mut bytes, 8, size as u32);
        put_be_u32(&mut bytes, 12, (padded + pad_extra) as u32);
        bytes[16..16 + payload.len()].copy_from_slice(payload);
        let sentinel = b"PAD_SENTINEL_16_";
        for (index, byte) in bytes[padded..].iter_mut().enumerate() {
            *byte = sentinel[index % sentinel.len()];
        }
        bytes
    }

    fn container_chunk(tag: &[u8; 4], children: &[Vec<u8>]) -> Vec<u8> {
        let mut payload = vec![0u8; 16];
        for child in children {
            payload.extend_from_slice(child);
        }
        chunk(tag, &payload)
    }

    fn stms(fields: &[[u32; 4]], stride: usize, data: &[u8], pad_extra: usize) -> Vec<u8> {
        let mut payload = vec![0u8; 16 + fields.len() * 16];
        put_be_u32(&mut payload, 0, fields.len() as u32);
        put_be_u32(&mut payload, 4, (data.len() / stride) as u32);
        put_be_u32(&mut payload, 8, stride as u32);
        for (index, field) in fields.iter().enumerate() {
            for (word, value) in field.iter().enumerate() {
                put_be_u32(&mut payload, 16 + index * 16 + word * 4, *value);
            }
        }
        payload.extend_from_slice(data);
        chunk_with_padding(b"STMS", &payload, pad_extra)
    }

    fn stms_position_with_opaque(
        normal_offset: u32,
        stride: usize,
        opaque_offset: Option<u32>,
    ) -> Vec<u8> {
        let mut data = vec![0u8; 3 * stride];
        for (index, values) in [
            [0i16, 0, 0, 32767],
            [32767, 0, 0, 32767],
            [0, 32767, 0, 32767],
        ]
        .into_iter()
        .enumerate()
        {
            let at = index * stride;
            for (component, value) in values.into_iter().enumerate() {
                data[at + component * 2..at + component * 2 + 2]
                    .copy_from_slice(&value.to_be_bytes());
            }
        }
        let mut fields = vec![
            [normal_offset, 3, 4, NORMAL_USAGE],
            [0, 4, 4, POSITION_USAGE],
        ];
        if let Some(opaque_offset) = opaque_offset {
            fields.push([opaque_offset, 3, 4, 0x0001_0000]);
        }
        stms(&fields, stride, &data, 0)
    }

    fn stms_indices_with_padding(pad_extra: usize) -> Vec<u8> {
        let mut data = Vec::with_capacity(6);
        for value in [0u16, 1, 2] {
            data.extend_from_slice(&value.to_be_bytes());
        }
        stms(&[[0, 0, 1, INDEX_USAGE]], 2, &data, pad_extra)
    }

    fn template_bytes(with_tail_overlap: bool) -> Vec<u8> {
        template_bytes_with_options(with_tail_overlap, 0, false, 12)
    }

    fn template_bytes_with_opaque(opaque_offset: u32) -> Vec<u8> {
        template_bytes_with_opaque_options(false, 0, false, 16, Some(opaque_offset))
    }

    fn template_bytes_with_options(
        with_tail_overlap: bool,
        index_padding: usize,
        extra_stream: bool,
        stride: usize,
    ) -> Vec<u8> {
        template_bytes_with_opaque_options(
            with_tail_overlap,
            index_padding,
            extra_stream,
            stride,
            None,
        )
    }

    fn template_bytes_with_opaque_options(
        with_tail_overlap: bool,
        index_padding: usize,
        extra_stream: bool,
        stride: usize,
        opaque_offset: Option<u32>,
    ) -> Vec<u8> {
        let mut head = vec![0u8; 12];
        head[0..4].copy_from_slice(b"OPAQ");
        put_be_u32(&mut head, 8, 3);
        let head = chunk(b"HEAD", &head);
        let mut aabb = vec![0u8; 24];
        for (offset, value) in [
            (0, -1.0),
            (4, -1.0),
            (8, -1.0),
            (12, 1.0),
            (16, 1.0),
            (20, 1.0),
        ] {
            put_be_f32(&mut aabb, offset, value);
        }
        let mesh_aabb = container_chunk(b"AABB", &[chunk(b"AABB", &aabb)]);
        let mut mesh_children = vec![
            head,
            stms_indices_with_padding(index_padding),
            stms_position_with_opaque(8, stride, opaque_offset),
        ];
        if extra_stream {
            mesh_children.push(stms(
                &[[0, 3, 4, 0x0001_0000]],
                4,
                &(0..12).collect::<Vec<_>>(),
                0,
            ));
        }
        mesh_children.push(mesh_aabb);
        let mesh = container_chunk(b"MESH", &mesh_children);
        let comp = chunk(b"COMP", &aabb);
        let model_aabb = container_chunk(b"AABB", &[chunk(b"AABB", &aabb)]);
        let mdl = container_chunk(b"MDL\0", &[model_aabb, comp, mesh]);
        let mdlc = container_chunk(b"MDLC", &[mdl]);
        let wrb = container_chunk(b"WRB\0", &[mdlc]);
        let mut child = vec![0u8; 0x30];
        child[..4].copy_from_slice(b"SEDB");
        child[4..8].copy_from_slice(b"wrb\0");
        child[0x0e..0x10].copy_from_slice(&0x30u16.to_le_bytes());
        child[0x10..0x14].copy_from_slice(&((0x30 + wrb.len()) as u32).to_le_bytes());
        child.extend_from_slice(&wrb);
        let mut root = vec![0u8; 0x40 + 3 * 16];
        root[..4].copy_from_slice(b"SEDB");
        root[4..8].copy_from_slice(b"RES ");
        root[0x0e..0x10].copy_from_slice(&0x40u16.to_le_bytes());
        root[0x30..0x34].copy_from_slice(&3u32.to_le_bytes());
        root[0x38..0x3c].copy_from_slice(&3u32.to_le_bytes());
        root[0x3c..0x40].copy_from_slice(b"brt\0");
        let payload_base = root.len();
        let tail = vec![0xA5u8; 48];
        let first_tail_offset = child.len() as u32 + 8;
        let second_tail_offset = child.len() as u32 + 16;
        let total = payload_base + child.len() + 48;
        root[0x10..0x14].copy_from_slice(&(total as u32).to_le_bytes());
        root[0x40..0x44].copy_from_slice(&7u32.to_le_bytes());
        root[0x44..0x48].copy_from_slice(&0u32.to_le_bytes());
        root[0x48..0x4c].copy_from_slice(&(child.len() as u32).to_le_bytes());
        root[0x4c..0x50].copy_from_slice(&2u32.to_le_bytes());
        root[0x50..0x54].copy_from_slice(&8u32.to_le_bytes());
        root[0x54..0x58].copy_from_slice(&first_tail_offset.to_le_bytes());
        root[0x58..0x5c]
            .copy_from_slice(&(if with_tail_overlap { 40u32 } else { 16u32 }).to_le_bytes());
        root[0x5c..0x60].copy_from_slice(&0u32.to_le_bytes());
        root[0x60..0x64].copy_from_slice(&9u32.to_le_bytes());
        root[0x64..0x68].copy_from_slice(&second_tail_offset.to_le_bytes());
        root[0x68..0x6c].copy_from_slice(&16u32.to_le_bytes());
        root[0x6c..0x70].copy_from_slice(&0u32.to_le_bytes());
        root.extend_from_slice(&child);
        root.extend_from_slice(&tail);
        root
    }

    #[test]
    fn imports_changed_topology_and_quantizes_positions_and_normals() {
        let obj = "o blockout\ns 1\nv 0 0 0\nv 2 0 0\nv 0 2 0\nv 0 0 2\nf 1 2 3\nf 1 3 4\nl 1 2\n";
        let output = import_obj_to_model(&template_bytes(false), obj).unwrap();
        let model = parse_model(&output).unwrap();
        assert_eq!(model.parts.len(), 1);
        assert_eq!(model.parts[0].vertices.len(), 4);
        assert_eq!(model.parts[0].indices, vec![0, 1, 2, 0, 2, 3]);
        assert!((model.parts[0].vertices[1].x - 2.0).abs() < 0.001);
        assert!(output.windows(4).any(|window| window == b"OPAQ"));
        assert_eq!(&output[output.len() - 24..], &[0xA5; 24]);
        let root = sedb::parse_container(&output, 0).unwrap();
        let profile = TemplateProfile::from_native(&output, &root).unwrap();
        let normal_start =
            profile.position.data.offset as usize + profile.position.normal_offset.unwrap();
        assert_eq!(&output[normal_start..normal_start + 3], &[218, 128, 218]);
    }

    fn bounds_from_leaf(data: &[u8], leaf: &ModelChunkInspection) -> (Vec3, Vec3) {
        let start = leaf.span.offset as usize + 16;
        let read = |index: usize| {
            let at = start + index * 4;
            f32::from_bits(u32::from_be_bytes(data[at..at + 4].try_into().unwrap()))
        };
        (
            Vec3 {
                x: read(0),
                y: read(1),
                z: read(2),
            },
            Vec3 {
                x: read(3),
                y: read(4),
                z: read(5),
            },
        )
    }

    #[test]
    fn updates_model_and_mesh_bounds_with_zero_axis() {
        let obj = "v -10 0 -4\nv 20 0 -4\nv -10 0 12\nf 1 2 3\n";
        let output = import_obj_to_model(&template_bytes(false), obj).unwrap();
        let root = sedb::parse_container(&output, 0).unwrap();
        let profile = TemplateProfile::from_native(&output, &root).unwrap();
        let expected_min = Vec3 {
            x: -10.0,
            y: 0.0,
            z: -4.0,
        };
        let expected_max = Vec3 {
            x: 20.0,
            y: 0.0,
            z: 12.0,
        };
        for leaf in [&profile.model_aabb_leaf, &profile.aabb_leaf] {
            let (min, max) = bounds_from_leaf(&output, leaf);
            assert_eq!(min, expected_min);
            assert_eq!(max, expected_max);
            assert!(min.x <= expected_min.x && max.x >= expected_max.x);
            assert!(min.y <= expected_min.y && max.y >= expected_max.y);
            assert!(min.z <= expected_min.z && max.z >= expected_max.z);
        }
        let model = parse_model(&output).unwrap();
        assert!(model.parts[0].vertices.iter().all(|vertex| vertex.y == 0.0));
    }

    #[test]
    fn rejects_overlapping_normal_field() {
        let mut native = template_bytes(false);
        let streams = native
            .windows(4)
            .enumerate()
            .filter_map(|(offset, bytes)| (bytes == b"STMS").then_some(offset))
            .collect::<Vec<_>>();
        native[streams[1] + 32..streams[1] + 36].copy_from_slice(&0u32.to_be_bytes());
        let error =
            import_obj_to_model(&native, "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n").unwrap_err();
        assert!(
            error.detail().contains("overlaps the position"),
            "{}",
            error.detail()
        );
    }

    #[test]
    fn rejects_unsupported_extra_vertex_stream() {
        let native = template_bytes_with_options(false, 0, true, 12);
        let error = import_obj_to_model(
            &native,
            "v 0 0 0\nv 1 0 0\nv 0 1 0\nv 0 0 1\nf 1 2 3\nf 1 3 4\n",
        )
        .unwrap_err();
        assert!(error.detail().contains("unsupported extra STMS"));
    }

    #[test]
    fn preserves_disjoint_opaque_fields_and_fourth_lanes() {
        let native = template_bytes_with_opaque(12);
        let output = import_obj_to_model(
            &native,
            "v 0 0 0\nv 1 0 0\nv 0 1 0\nv 0 0 1\nf 1 2 3\nf 1 3 4\n",
        )
        .unwrap();
        let native_root = sedb::parse_container(&native, 0).unwrap();
        let output_root = sedb::parse_container(&output, 0).unwrap();
        let native_profile = TemplateProfile::from_native(&native, &native_root).unwrap();
        let output_profile = TemplateProfile::from_native(&output, &output_root).unwrap();
        let native_data = copy_span(&native, native_profile.position.data).unwrap();
        let output_data = copy_span(&output, output_profile.position.data).unwrap();
        for index in 0..4 {
            let source = index.min(2);
            let output_start = index * 16;
            let source_start = source * 16;
            assert_eq!(
                &output_data[output_start + 6..output_start + 8],
                &native_data[source_start + 6..source_start + 8]
            );
            assert_eq!(
                output_data[output_start + 11],
                native_data[source_start + 11]
            );
            assert_eq!(
                &output_data[output_start + 12..output_start + 16],
                &native_data[source_start + 12..source_start + 16]
            );
        }
    }

    #[test]
    fn rejects_opaque_fields_aliasing_generated_ranges() {
        for offset in [0, 8] {
            let error = import_obj_to_model(
                &template_bytes_with_opaque(offset),
                "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n",
            )
            .unwrap_err();
            assert!(error.detail().contains("unresolved vertex field overlaps"));
        }
    }

    #[test]
    fn rejects_bounds_that_native_decode_cannot_represent() {
        let error = import_obj_to_model(
            &template_bytes(false),
            "v 3.3e38 0 0\nv 3.3e38 1 0\nv 3.3e38 0 1\nf 1 2 3\n",
        )
        .unwrap_err();
        assert!(error
            .detail()
            .contains("not representable by native decode"));
    }

    #[test]
    fn rejects_logically_short_replaced_chunks() {
        let mut short_head = template_bytes(false);
        let head = short_head
            .windows(4)
            .position(|bytes| bytes == b"HEAD")
            .unwrap();
        put_be_u32(&mut short_head, head + 8, 20);
        let error =
            import_obj_to_model(&short_head, "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n").unwrap_err();
        assert!(error.detail().contains("HEAD logical payload"));

        let mut short_aabb = template_bytes(false);
        let aabb = short_aabb
            .windows(4)
            .enumerate()
            .filter_map(|(offset, bytes)| (bytes == b"AABB").then_some(offset))
            .next_back()
            .unwrap();
        put_be_u32(&mut short_aabb, aabb + 8, 36);
        let error =
            import_obj_to_model(&short_aabb, "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n").unwrap_err();
        assert!(error.detail().contains("bounds chunk logical payload"));
    }

    #[test]
    fn preserves_explicit_stream_padding() {
        let output = import_obj_to_model(
            &template_bytes_with_options(false, 16, false, 12),
            "v 0 0 0\nv 1 0 0\nv 0 1 0\nv 0 0 1\nf 1 2 3\nf 1 3 4\n",
        )
        .unwrap();
        assert!(output
            .windows(b"PAD_SENTINEL_16_".len())
            .any(|window| { window == b"PAD_SENTINEL_16_" }));
    }

    #[test]
    fn rejects_templates_over_output_budget_before_vertex_allocation() {
        let native = template_bytes_with_options(false, 0, false, 4097);
        let error =
            import_obj_to_model(&native, "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n").unwrap_err();
        assert_eq!(error.kind(), ErrorKind::ResourceLimitExceeded);
        assert!(error.detail().contains("output budget"));
    }

    #[test]
    fn relocates_overlapping_tail_directory_entries() {
        let output = import_obj_to_model(
            &template_bytes(true),
            "v 0 0 0\nv 1 0 0\nv 0 1 0\nv 0 0 1\nv 1 1 0\nf 1 2 3\nf 1 3 4\nf 1 4 5\n",
        )
        .unwrap();
        let root = sedb::parse_container(&output, 0).unwrap();
        assert_eq!(root.total_size as usize, output.len());
        assert!(output.len() > template_bytes(true).len());
        assert!(root
            .anomalies
            .iter()
            .any(|anomaly| anomaly.kind == "subresource-overlap"));
        assert_eq!(output[output.len() - 24], 0xA5);
    }

    #[test]
    fn rejects_unsupported_faces_and_stream_profiles() {
        let error = import_obj_to_model(
            &template_bytes(false),
            "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3 1\n",
        )
        .unwrap_err();
        assert!(error.detail().contains("exactly three"));
        let mut unsupported = template_bytes(false);
        let streams = unsupported
            .windows(4)
            .enumerate()
            .filter_map(|(offset, bytes)| (bytes == b"STMS").then_some(offset))
            .collect::<Vec<_>>();
        let position = streams[1];
        unsupported[position + 16 + 32 + 4..position + 16 + 32 + 8]
            .copy_from_slice(&2u32.to_be_bytes());
        let error =
            import_obj_to_model(&unsupported, "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n").unwrap_err();
        assert!(error.offset() <= unsupported.len() as u64);
        let mut truncated = template_bytes(false);
        truncated.truncate(truncated.len() - 1);
        let error =
            import_obj_to_model(&truncated, "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n").unwrap_err();
        assert!(error.offset() <= truncated.len() as u64);
    }
}
