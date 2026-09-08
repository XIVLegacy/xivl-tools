//! Bounded retail 1.23b world-layout and collision geometry.
//!
//! The supported path is the documented MapLayout -> SEDB `lyb` graph, the
//! SEDB RES/WRB model stream reader, and the fixed-offset SEDB PHB hull.  The
//! exporter consumes collision only: Attribute-bound PHB hulls and
//! CollisionBox unit primitives.  Render meshes are decoded for source
//! association but are never added to the collision OBJ.

use std::collections::BTreeMap;

use crate::error::{ErrorKind, FormatError, Result};
use crate::resource::ResourceId;
use serde_json::{json, Value};

pub const GEOMETRY_SCHEMA_VERSION: u32 = 2;
const MAX_RECORDS: u32 = 1_000_000;
const MAX_GRAPH_DEPTH: usize = 128;
const MAX_MODEL_CHUNK_DEPTH: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vec3 {
    pub const ZERO: Self = Self {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };
    pub const ONE: Self = Self {
        x: 1.0,
        y: 1.0,
        z: 1.0,
    };
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform {
    pub translation: Vec3,
    pub rotation: Vec3,
    pub scale: Vec3,
}

impl Default for Transform {
    fn default() -> Self {
        Self {
            translation: Vec3::ZERO,
            rotation: Vec3::ZERO,
            scale: Vec3::ONE,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Matrix4 {
    pub values: [[f32; 4]; 4],
}

impl Matrix4 {
    pub const IDENTITY: Self = Self {
        values: [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
    };

    pub fn multiply(self, other: Self) -> Self {
        let mut values = [[0.0; 4]; 4];
        for (row, output_row) in values.iter_mut().enumerate() {
            for (column, output) in output_row.iter_mut().enumerate() {
                *output = (0..4)
                    .map(|index| self.values[row][index] * other.values[index][column])
                    .sum();
            }
        }
        Self { values }
    }

    pub fn transform_point(self, point: Vec3) -> Vec3 {
        Vec3 {
            x: self.values[0][0] * point.x
                + self.values[0][1] * point.y
                + self.values[0][2] * point.z
                + self.values[0][3],
            y: self.values[1][0] * point.x
                + self.values[1][1] * point.y
                + self.values[1][2] * point.z
                + self.values[1][3],
            z: self.values[2][0] * point.x
                + self.values[2][1] * point.y
                + self.values[2][2] * point.z
                + self.values[2][3],
        }
    }
}

/// The client scene order is Y-up `T * Rz * Ry * Rx * S`.
pub fn transform_matrix(transform: Transform) -> Matrix4 {
    let (sx, cx) = transform.rotation.x.sin_cos();
    let (sy, cy) = transform.rotation.y.sin_cos();
    let (sz, cz) = transform.rotation.z.sin_cos();
    let translation = Matrix4 {
        values: [
            [1.0, 0.0, 0.0, transform.translation.x],
            [0.0, 1.0, 0.0, transform.translation.y],
            [0.0, 0.0, 1.0, transform.translation.z],
            [0.0, 0.0, 0.0, 1.0],
        ],
    };
    let rz = Matrix4 {
        values: [
            [cz, -sz, 0.0, 0.0],
            [sz, cz, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
    };
    let ry = Matrix4 {
        values: [
            [cy, 0.0, sy, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [-sy, 0.0, cy, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
    };
    let rx = Matrix4 {
        values: [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, cx, -sx, 0.0],
            [0.0, sx, cx, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
    };
    let scale = Matrix4 {
        values: [
            [transform.scale.x, 0.0, 0.0, 0.0],
            [0.0, transform.scale.y, 0.0, 0.0],
            [0.0, 0.0, transform.scale.z, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
    };
    translation
        .multiply(rz)
        .multiply(ry)
        .multiply(rx)
        .multiply(scale)
}

pub fn compose_transforms(transforms: &[Transform]) -> Matrix4 {
    transforms
        .iter()
        .fold(Matrix4::IDENTITY, |world, transform| {
            world.multiply(transform_matrix(*transform))
        })
}

#[derive(Debug, Clone, PartialEq)]
pub struct LayoutResource {
    pub name: String,
    pub type_tag: String,
    pub resource_id: ResourceId,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LayoutObject {
    pub model: Option<ResourceId>,
    pub phb: Option<ResourceId>,
    pub source_node: u32,
    pub source_block: u32,
    pub resource_key: Option<String>,
    pub instance: Transform,
    pub unit_tree: Transform,
    pub group_chain: Vec<Transform>,
    pub child: Transform,
    pub boxes: Vec<CollisionPrimitive>,
    pub world_matrix: Matrix4,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    pub zone_name: String,
    pub zone: Transform,
    pub objects: Vec<LayoutObject>,
    pub resources: Vec<LayoutResource>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeshClassification {
    Render,
    Collision,
}

impl MeshClassification {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Render => "render",
            Self::Collision => "collision",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MeshPart {
    pub classification: MeshClassification,
    pub vertices: Vec<Vec3>,
    pub indices: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Model {
    pub parts: Vec<MeshPart>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PhbHull {
    pub surface: u32,
    pub surfaces: Vec<u32>,
    pub vertices: Vec<Vec3>,
    pub indices: Vec<u32>,
    pub declared_min: Vec3,
    pub declared_max: Vec3,
    pub decoded_min: Vec3,
    pub decoded_max: Vec3,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CollisionPrimitive {
    pub kind: u32,
    pub transform: Transform,
    pub center: Vec3,
    pub half_extents: Vec3,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Phb {
    pub hulls: Vec<PhbHull>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SourceFace {
    pub kind: String,
    pub resource_id: Option<ResourceId>,
    pub object_index: usize,
    pub part_index: Option<usize>,
    pub triangle_index: usize,
    pub classification: String,
    pub classification_raw: Option<u32>,
    pub raw_surface: Option<u32>,
    pub model_resource_id: Option<ResourceId>,
    pub placement_index: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OutputFace {
    pub indices: [u32; 3],
    pub source: SourceFace,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlacementTrace {
    pub object_index: usize,
    pub source_node: u32,
    pub source_block: u32,
    pub resource_key: Option<String>,
    pub model_resource_id: Option<ResourceId>,
    pub phb_resource_id: Option<ResourceId>,
    pub classifications: Vec<MeshClassification>,
    pub zone: Transform,
    pub instance: Transform,
    pub unit_tree: Transform,
    pub group_chain: Vec<Transform>,
    pub child: Transform,
    pub world_matrix: Matrix4,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CollisionResourceInfo {
    pub resource_id: ResourceId,
    pub hulls: Vec<CollisionHullInfo>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CollisionHullInfo {
    pub surface: u32,
    pub vertex_count: usize,
    pub triangle_count: usize,
    pub declared_min: Vec3,
    pub declared_max: Vec3,
    pub decoded_min: Vec3,
    pub decoded_max: Vec3,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FlattenedZone {
    pub vertices: Vec<Vec3>,
    pub faces: Vec<OutputFace>,
    pub placements: Vec<PlacementTrace>,
    pub collision_resources: Vec<CollisionResourceInfo>,
}

impl FlattenedZone {
    pub fn obj(&self) -> String {
        let mut text = String::new();
        for vertex in &self.vertices {
            text.push_str(&format!(
                "v {} {} {}\n",
                obj_float(vertex.x),
                obj_float(vertex.y),
                obj_float(vertex.z)
            ));
        }
        for face in &self.faces {
            text.push_str(&format!(
                "f {} {} {}\n",
                face.indices[0] + 1,
                face.indices[1] + 1,
                face.indices[2] + 1
            ));
        }
        text
    }

    pub fn metadata(&self, zone_name: &str, sources: &[SourceInfo]) -> Value {
        let face_ranges = self.face_ranges();
        let placements: Vec<Value> = self
            .placements
            .iter()
            .map(|placement| {
                json!({
                    "objectIndex": placement.object_index,
                    "source": {
                        "nodeOffset": placement.source_node,
                        "blockOffset": placement.source_block,
                        "resourceKey": placement.resource_key,
                        "modelResourceId": placement.model_resource_id.map(ResourceId::to_hex),
                        "phbResourceId": placement.phb_resource_id.map(ResourceId::to_hex),
                    },
                    "classifications": placement.classifications.iter().map(|classification| classification.as_str()).collect::<Vec<_>>(),
                    "transformChain": {
                        "zone": transform_json(placement.zone),
                        "instance": transform_json(placement.instance),
                        "unitTree": transform_json(placement.unit_tree),
                        "groups": placement.group_chain.iter().map(|group| transform_json(*group)).collect::<Vec<_>>(),
                        "child": transform_json(placement.child),
                    },
                    "worldMatrix": placement.world_matrix.values,
                })
            })
            .collect();
        json!({
            "schemaVersion": GEOMETRY_SCHEMA_VERSION,
            "format": "xivl-zone-geometry",
            "zone": { "name": zone_name },
            "settings": {
                "coordinateSystem": "y-up",
                "transformOrder": "T*Rz*Ry*Rx*S",
                "collisionWinding": "two-sided-source-order",
                "geometry": "collision-only"
            },
            "counts": { "vertices": self.vertices.len(), "faces": self.faces.len() },
            "sources": sources.iter().map(SourceInfo::json).collect::<Vec<_>>(),
            "collisionResources": self
                .collision_resources
                .iter()
                .map(collision_resource_json)
                .collect::<Vec<_>>(),
            "placements": placements,
            "faceRanges": face_ranges
        })
    }

    fn face_ranges(&self) -> Vec<Value> {
        let mut ranges = Vec::new();
        let mut current: Option<(usize, usize, SourceFace)> = None;
        for (face_index, face) in self.faces.iter().enumerate() {
            let can_extend = current.as_ref().is_some_and(|(_, count, source)| {
                source.kind == face.source.kind
                    && source.resource_id == face.source.resource_id
                    && source.object_index == face.source.object_index
                    && source.part_index == face.source.part_index
                    && source.classification == face.source.classification
                    && source.classification_raw == face.source.classification_raw
                    && source.raw_surface == face.source.raw_surface
                    && source.model_resource_id == face.source.model_resource_id
                    && source.placement_index == face.source.placement_index
                    && source.triangle_index + *count == face.source.triangle_index
            });
            if !can_extend {
                if let Some((start, count, source)) = current.take() {
                    ranges.push(face_range_json(start, count, &source));
                }
                current = Some((face_index, 1, face.source.clone()));
            } else if let Some((_, count, _)) = current.as_mut() {
                *count += 1;
            }
        }
        if let Some((start, count, source)) = current {
            ranges.push(face_range_json(start, count, &source));
        }
        ranges
    }
}

fn transform_json(transform: Transform) -> Value {
    json!({
        "translation": [transform.translation.x, transform.translation.y, transform.translation.z],
        "rotation": [transform.rotation.x, transform.rotation.y, transform.rotation.z],
        "scale": [transform.scale.x, transform.scale.y, transform.scale.z],
    })
}

fn vec3_json(value: Vec3) -> Value {
    json!([value.x, value.y, value.z])
}

fn collision_resource_json(resource: &CollisionResourceInfo) -> Value {
    json!({
        "resourceId": resource.resource_id.to_hex(),
        "classification": "collision",
        "hulls": resource.hulls.iter().map(|hull| json!({
            "surface": hull.surface,
            "vertexCount": hull.vertex_count,
            "triangleCount": hull.triangle_count,
            "declaredMin": vec3_json(hull.declared_min),
            "declaredMax": vec3_json(hull.declared_max),
            "decodedMin": vec3_json(hull.decoded_min),
            "decodedMax": vec3_json(hull.decoded_max),
            "boundsMismatch": hull.declared_min != hull.decoded_min || hull.declared_max != hull.decoded_max,
        })).collect::<Vec<_>>(),
    })
}

fn face_range_json(start: usize, count: usize, source: &SourceFace) -> Value {
    json!({
        "faceStart": start + 1,
        "faceCount": count,
        "winding": "source-order",
        "source": {
            "kind": source.kind,
            "resourceId": source.resource_id.map(ResourceId::to_hex),
            "objectIndex": source.object_index,
            "placementIndex": source.placement_index,
            "partIndex": source.part_index,
            "triangleStart": source.triangle_index,
            "triangleCount": count,
            "classification": source.classification,
            "classificationRaw": source.classification_raw,
            "rawSurface": source.raw_surface,
            "modelResourceId": source.model_resource_id.map(ResourceId::to_hex),
        }
    })
}

#[derive(Debug, Clone, PartialEq)]
pub struct SourceInfo {
    pub kind: String,
    pub resource_id: ResourceId,
    pub path: String,
    pub sha256: String,
}

impl SourceInfo {
    pub fn json(&self) -> Value {
        let classification = match self.kind.as_str() {
            "model" => "render",
            "phb" => "collision",
            "layout" => "layout",
            _ => "unresolved",
        };
        json!({ "kind": self.kind, "classification": classification, "resourceId": self.resource_id.to_hex(), "path": self.path, "sha256": self.sha256 })
    }
}

/// Read a MapLayout outer resource and its embedded SEDB `lyb` graph.
pub fn parse_layout(data: &[u8]) -> Result<Layout> {
    if data.len() < 0x40 || data.get(..24) != Some(b"MapLayoutResourceData\0\0\0") {
        return Err(zone_error(0, "expected MapLayoutResourceData header"));
    }
    let scene_offset = u32_at(data, 0x20)? as usize;
    let entry_count = bounded_u32(u32_at(data, 0x24)?, 0x24, "MapLayout entry")?;
    let table_end = 0x40usize
        .checked_add(
            (entry_count as usize)
                .checked_mul(32)
                .ok_or_else(|| zone_error(0x24, "MapLayout table overflows"))?,
        )
        .ok_or_else(|| zone_error(0x24, "MapLayout table overflows"))?;
    if table_end != scene_offset || scene_offset > data.len() {
        return Err(zone_error(
            0x20,
            "MapLayout scene offset does not follow its resource table",
        ));
    }
    let mut resources = Vec::with_capacity(entry_count as usize);
    for index in 0..entry_count as usize {
        let offset = 0x40 + index * 32;
        resources.push(LayoutResource {
            name: fixed_text(data, offset, 16)?,
            type_tag: fixed_text(data, offset + 16, 4)?,
            resource_id: ResourceId::new(u32_at(data, offset + 20)?),
        });
    }
    let scene = data
        .get(scene_offset..)
        .ok_or_else(|| zone_error(scene_offset as u64, "MapLayout scene is outside input"))?;
    let container = crate::sedb::parse_container(scene, scene_offset as u64)?;
    if container.subtype != "lyb\\x00" {
        return Err(zone_error(
            scene_offset as u64 + 4,
            "MapLayout scene is not an lyb SEDB container",
        ));
    }
    let payload_start = scene_offset + usize::from(container.header_size);
    let payload_end = scene_offset + container.total_size as usize;
    let payload = data
        .get(payload_start..payload_end)
        .ok_or_else(|| zone_error(payload_start as u64, "lyb payload is outside MapLayout"))?;
    parse_lyb_payload(payload, resources, container.total_size)
}

/// Decode the documented SEDB model path (RES -> WRB -> MDL -> MESH).
pub fn parse_model(data: &[u8]) -> Result<Model> {
    let root = crate::sedb::parse_container(data, 0)?;
    if root.res.is_none() {
        return Err(zone_error(4, "model resource is not a RES container"));
    }
    reject_model_res_anomalies(&root)?;
    let mut parts = Vec::new();
    let mut found_wrb = false;
    for entry in &root.entries {
        if let crate::sedb::EntryBody::Subresource {
            child: Some(child), ..
        } = &entry.body
        {
            if child.subtype == "wrb\\x00" {
                found_wrb = true;
                let start = child.span.offset as usize + usize::from(child.header_size);
                let end = child.span.end() as usize;
                if let Some(bytes) = data.get(start..end) {
                    parse_wrb_chunks(bytes, None, &mut parts)?;
                }
            }
        }
    }
    if !found_wrb {
        return Err(zone_error(0, "model RES contains no WRB resource"));
    }
    Ok(Model { parts })
}

fn reject_model_res_anomalies(container: &crate::sedb::Container) -> Result<()> {
    for entry in &container.entries {
        if let crate::sedb::EntryBody::Subresource {
            child: Some(child), ..
        } = &entry.body
        {
            if child.subtype == "wrb\\x00" {
                if let Some(anomaly) = container
                    .anomalies
                    .iter()
                    .find(|anomaly| spans_overlap(anomaly.span, entry.span))
                {
                    return Err(zone_error(
                        anomaly.span.offset,
                        format!("model RES has unsupported anomaly {}", anomaly.kind),
                    ));
                }
                reject_model_child_anomalies(child)?;
            }
        }
    }
    Ok(())
}

fn reject_model_child_anomalies(container: &crate::sedb::Container) -> Result<()> {
    if let Some(anomaly) = container.anomalies.first() {
        return Err(zone_error(
            anomaly.span.offset,
            format!("model RES has unsupported anomaly {}", anomaly.kind),
        ));
    }
    for entry in &container.entries {
        if let crate::sedb::EntryBody::Subresource {
            child: Some(child), ..
        } = &entry.body
        {
            reject_model_child_anomalies(child)?;
        }
    }
    Ok(())
}

fn spans_overlap(left: crate::reader::Span, right: crate::reader::Span) -> bool {
    left.offset < right.end() && right.offset < left.end()
}

/// Decode a PHB hull. Retail PHB files have `PHB.GBD` at file offset 0xC0,
/// even though the SEDB total extent ends at the 0x48 header.
pub fn parse_phb(data: &[u8]) -> Result<Phb> {
    let root = crate::sedb::parse_container(data, 0)?;
    if root.subtype != "PHB\\x00" {
        return Err(zone_error(4, "collision resource is not a PHB container"));
    }
    let chunk_start = 0xC0usize;
    let chunk = data
        .get(chunk_start..)
        .ok_or_else(|| zone_error(chunk_start as u64, "PHB.GBD is outside input"))?;
    if chunk.get(..8) != Some(b"PHB.GBD\0") {
        return Err(zone_error(
            chunk_start as u64,
            "expected PHB.GBD at file offset 0xC0",
        ));
    }
    let gbd_size = u32_at(chunk, 8)? as usize;
    let payload_end = gbd_size;
    let expected_end = payload_end
        .checked_add(16)
        .ok_or_else(|| zone_error(chunk_start as u64 + 8, "PHB.GBD trailer size overflows"))?;
    if chunk.len() != expected_end {
        return Err(zone_error(
            chunk_start as u64 + 8,
            "PHB.GBD must have a 16-byte trailer after payloadEnd",
        ));
    }
    let chunk = chunk.get(..payload_end).ok_or_else(|| {
        zone_error(
            chunk_start as u64 + 8,
            "PHB.GBD declared size is outside the resource payload",
        )
    })?;
    let declared_min = Vec3 {
        x: f32_le(chunk, 0x40)?,
        y: f32_le(chunk, 0x44)?,
        z: f32_le(chunk, 0x48)?,
    };
    let declared_max = Vec3 {
        x: f32_le(chunk, 0x50)?,
        y: f32_le(chunk, 0x54)?,
        z: f32_le(chunk, 0x58)?,
    };
    let vertex_offset = u32_at(chunk, 0x38)? as usize;
    let vertex_count = bounded_u32(
        u32_at(chunk, 0x3C)?,
        chunk_start as u64 + 0x3C,
        "PHB vertex",
    )?;
    let triangle_offset = u32_at(chunk, 0x60)? as usize;
    let triangle_count = bounded_u32(
        u32_at(chunk, 0x64)?,
        chunk_start as u64 + 0x64,
        "PHB triangle",
    )?;
    if vertex_offset < 0x80 || triangle_offset < 0x80 {
        return Err(zone_error(
            chunk_start as u64 + 0x38,
            "PHB geometry offsets must follow the 0x80 header",
        ));
    }
    let vertex_bytes = (vertex_count as usize)
        .checked_mul(12)
        .ok_or_else(|| zone_error(chunk_start as u64 + 0x3C, "PHB vertices overflow"))?;
    let vertex_data = chunk
        .get(vertex_offset..vertex_offset + vertex_bytes)
        .ok_or_else(|| {
            zone_error(
                chunk_start as u64 + vertex_offset as u64,
                "PHB vertices are outside chunk",
            )
        })?;
    let mut vertices = Vec::with_capacity(vertex_count as usize);
    for index in 0..vertex_count as usize {
        let at = index * 12;
        vertices.push(Vec3 {
            x: f32_le(vertex_data, at)?,
            y: f32_le(vertex_data, at + 4)?,
            z: f32_le(vertex_data, at + 8)?,
        });
    }
    let triangle_bytes = (triangle_count as usize)
        .checked_mul(8)
        .ok_or_else(|| zone_error(chunk_start as u64 + 0x64, "PHB triangles overflow"))?;
    let triangle_data = chunk
        .get(triangle_offset..triangle_offset + triangle_bytes)
        .ok_or_else(|| {
            zone_error(
                chunk_start as u64 + triangle_offset as u64,
                "PHB triangles are outside chunk",
            )
        })?;
    let mut indices = Vec::with_capacity(triangle_count as usize * 3);
    let mut surfaces = Vec::with_capacity(triangle_count as usize);
    for index in 0..triangle_count as usize {
        let at = index * 8;
        let i0 = u16_le(triangle_data, at)? as u32;
        let i1 = u16_le(triangle_data, at + 2)? as u32;
        let i2 = u16_le(triangle_data, at + 4)? as u32;
        if i0 >= vertex_count || i1 >= vertex_count || i2 >= vertex_count {
            return Err(zone_error(
                chunk_start as u64 + triangle_offset as u64 + at as u64,
                "PHB triangle index is outside vertices",
            ));
        }
        indices.extend([i0, i1, i2]);
        surfaces.push(u16_le(triangle_data, at + 6)? as u32);
    }
    let (decoded_min, decoded_max) = bounds(&vertices);
    Ok(Phb {
        hulls: vec![PhbHull {
            surface: surfaces.first().copied().unwrap_or(0),
            surfaces,
            vertices,
            indices,
            declared_min,
            declared_max,
            decoded_min,
            decoded_max,
        }],
    })
}

fn bounds(vertices: &[Vec3]) -> (Vec3, Vec3) {
    let mut min = Vec3 {
        x: f32::INFINITY,
        y: f32::INFINITY,
        z: f32::INFINITY,
    };
    let mut max = Vec3 {
        x: f32::NEG_INFINITY,
        y: f32::NEG_INFINITY,
        z: f32::NEG_INFINITY,
    };
    for vertex in vertices {
        min.x = min.x.min(vertex.x);
        min.y = min.y.min(vertex.y);
        min.z = min.z.min(vertex.z);
        max.x = max.x.max(vertex.x);
        max.y = max.y.max(vertex.y);
        max.z = max.z.max(vertex.z);
    }
    if vertices.is_empty() {
        (Vec3::ZERO, Vec3::ZERO)
    } else {
        (min, max)
    }
}

/// Flatten collision only. Model meshes are intentionally not appended.
pub fn flatten_zone(layout: &Layout, phbs: &[(ResourceId, Phb)]) -> Result<FlattenedZone> {
    let mut output = FlattenedZone {
        vertices: Vec::new(),
        faces: Vec::new(),
        placements: Vec::new(),
        collision_resources: phbs
            .iter()
            .map(|(resource_id, phb)| CollisionResourceInfo {
                resource_id: *resource_id,
                hulls: phb
                    .hulls
                    .iter()
                    .map(|hull| CollisionHullInfo {
                        surface: hull.surface,
                        vertex_count: hull.vertices.len(),
                        triangle_count: hull.indices.len() / 3,
                        declared_min: hull.declared_min,
                        declared_max: hull.declared_max,
                        decoded_min: hull.decoded_min,
                        decoded_max: hull.decoded_max,
                    })
                    .collect(),
            })
            .collect(),
    };
    for (object_index, object) in layout.objects.iter().enumerate() {
        let world = object.world_matrix;
        let placement_index = output.placements.len();
        output.placements.push(PlacementTrace {
            object_index,
            source_node: object.source_node,
            source_block: object.source_block,
            resource_key: object.resource_key.clone(),
            model_resource_id: object.model,
            phb_resource_id: object.phb,
            classifications: placement_classifications(object),
            zone: layout.zone,
            instance: object.instance,
            unit_tree: object.unit_tree,
            group_chain: object.group_chain.clone(),
            child: object.child,
            world_matrix: world,
        });
        if let Some(phb_id) = object.phb {
            let phb = phbs
                .iter()
                .find(|(id, _)| *id == phb_id)
                .map(|(_, value)| value)
                .ok_or_else(|| zone_error(0, "layout references a missing PHB"))?;
            append_phb(
                &mut output,
                object_index,
                placement_index,
                phb_id,
                object.model,
                world,
                phb,
            );
        }
        for (primitive_index, primitive) in object.boxes.iter().enumerate() {
            append_box(
                &mut output,
                object_index,
                placement_index,
                primitive_index,
                world.multiply(transform_matrix(primitive.transform)),
                primitive,
            );
        }
    }
    Ok(output)
}

fn placement_classifications(object: &LayoutObject) -> Vec<MeshClassification> {
    let mut classifications = Vec::new();
    if object.model.is_some() {
        classifications.push(MeshClassification::Render);
    }
    if object.phb.is_some() || !object.boxes.is_empty() {
        classifications.push(MeshClassification::Collision);
    }
    classifications
}

fn append_phb(
    output: &mut FlattenedZone,
    object_index: usize,
    placement_index: usize,
    resource_id: ResourceId,
    model_id: Option<ResourceId>,
    world: Matrix4,
    phb: &Phb,
) {
    for (hull_index, hull) in phb.hulls.iter().enumerate() {
        let base = output.vertices.len() as u32;
        output.vertices.extend(
            hull.vertices
                .iter()
                .map(|vertex| world.transform_point(*vertex)),
        );
        for (triangle_index, triangle) in hull.indices.chunks_exact(3).enumerate() {
            output.faces.push(OutputFace {
                indices: [base + triangle[0], base + triangle[1], base + triangle[2]],
                source: SourceFace {
                    kind: "phb-hull".into(),
                    resource_id: Some(resource_id),
                    object_index,
                    part_index: Some(hull_index),
                    triangle_index,
                    classification: "collision".into(),
                    classification_raw: None,
                    raw_surface: hull.surfaces.get(triangle_index).copied(),
                    model_resource_id: model_id,
                    placement_index,
                },
            });
        }
    }
}

fn append_box(
    output: &mut FlattenedZone,
    object_index: usize,
    placement_index: usize,
    primitive_index: usize,
    world: Matrix4,
    primitive: &CollisionPrimitive,
) {
    let c = primitive.center;
    let h = primitive.half_extents;
    let corners = [
        Vec3 {
            x: c.x - h.x,
            y: c.y - h.y,
            z: c.z + h.z,
        },
        Vec3 {
            x: c.x + h.x,
            y: c.y - h.y,
            z: c.z + h.z,
        },
        Vec3 {
            x: c.x - h.x,
            y: c.y + h.y,
            z: c.z + h.z,
        },
        Vec3 {
            x: c.x + h.x,
            y: c.y + h.y,
            z: c.z + h.z,
        },
        Vec3 {
            x: c.x - h.x,
            y: c.y + h.y,
            z: c.z - h.z,
        },
        Vec3 {
            x: c.x + h.x,
            y: c.y + h.y,
            z: c.z - h.z,
        },
        Vec3 {
            x: c.x - h.x,
            y: c.y - h.y,
            z: c.z - h.z,
        },
        Vec3 {
            x: c.x + h.x,
            y: c.y - h.y,
            z: c.z - h.z,
        },
    ];
    let base = output.vertices.len() as u32;
    output.vertices.extend(
        corners
            .into_iter()
            .map(|vertex| world.transform_point(vertex)),
    );
    const TRIANGLES: [[u32; 3]; 12] = [
        [0, 1, 3],
        [0, 3, 2],
        [2, 3, 5],
        [2, 5, 4],
        [4, 5, 7],
        [4, 7, 6],
        [6, 7, 1],
        [6, 1, 0],
        [1, 7, 5],
        [1, 5, 3],
        [6, 0, 2],
        [6, 2, 4],
    ];
    for (triangle_index, triangle) in TRIANGLES.into_iter().enumerate() {
        output.faces.push(OutputFace {
            indices: [base + triangle[0], base + triangle[1], base + triangle[2]],
            source: SourceFace {
                kind: "collision-box".into(),
                resource_id: None,
                object_index,
                part_index: Some(primitive_index),
                triangle_index,
                classification: "collision".into(),
                classification_raw: Some(primitive.kind),
                raw_surface: Some(0),
                model_resource_id: None,
                placement_index,
            },
        });
    }
}

#[derive(Debug, Clone)]
struct LyNodeType {
    data_offset: u32,
    node_count: u32,
    node_size: u32,
    fields: Vec<(u16, u16)>,
}

#[derive(Debug, Clone)]
struct LyGraph<'a> {
    payload: &'a [u8],
    node_types: Vec<LyNodeType>,
    objects: Vec<u32>,
    type_index: BTreeMap<u32, usize>,
}

struct WalkPath {
    transforms: [Transform; 5],
    groups: Vec<Transform>,
}

const CLASS_NODE_FIELDS: &[(u16, u16)] = &[(4, 0), (8, 2), (16, 0)];
const INSTANCE_CHILD_RANGE_FIELDS: &[(u16, u16)] = &[(0, 0)];
const INSTANCE_FIELDS: &[(u16, u16)] =
    &[(4, 0), (8, 2), (16, 0), (44, 0), (48, 0), (60, 0), (52, 0)];
const UNIT_TREE_FIELDS: &[(u16, u16)] =
    &[(4, 0), (8, 2), (28, 0), (32, 0), (44, 2), (36, 0), (48, 0)];
const CHILD_FIELDS: &[(u16, u16)] = &[(20, 2), (12, 0), (16, 0), (24, 0), (32, 0)];
const GROUP_FIELDS: &[(u16, u16)] = &[(0x0C, 0), (0x10, 0), (0x14, 0), (0x1C, 2), (0x20, 0)];
// Some 1.23b layouts carry the same group payload with the surrounding
// serializer's nine-descriptor declaration; both shapes are exact.
const GROUP_FIELDS_EXTENDED: &[(u16, u16)] = &[
    (0, 0),
    (8, 2),
    (12, 2),
    (16, 2),
    (20, 0),
    (28, 2),
    (32, 2),
    (36, 2),
    (40, 2),
];
const ATTRIBUTE_FIELDS: &[(u16, u16)] = &[(4, 0), (8, 2), (36, 2), (20, 0), (28, 0)];
const COLLISION_BOX_FIELDS: &[(u16, u16)] = &[(4, 0), (8, 2)];
const LAY_SETTINGS_FIELDS: &[(u16, u16)] = &[
    (4, 0),
    (8, 2),
    (76, 0),
    (80, 0),
    (92, 0),
    (28, 2),
    (84, 0),
    (96, 0),
];

fn exact_node_shape(kind: &LyNodeType, node_size: u32, fields: &[(u16, u16)]) -> bool {
    kind.node_size == node_size
        && kind.fields.len() == fields.len()
        && kind.fields.iter().all(|field| fields.contains(field))
        && fields.iter().all(|field| kind.fields.contains(field))
}

fn parse_lyb_payload(
    payload: &[u8],
    resources: Vec<LayoutResource>,
    containing_size: u32,
) -> Result<Layout> {
    if payload.len() < 0x18 || payload.get(..4) != Some(b"lyb\0") {
        return Err(zone_error(0, "expected lyb scene payload"));
    }
    if u32_at(payload, 4)? != containing_size {
        return Err(zone_error(
            4,
            "lyb declared size does not match its containing SEDB size",
        ));
    }
    let node_table = u32_at(payload, 0x10)? as usize;
    let object_list = u32_at(payload, 0x14)? as usize;
    if object_list < 0x18 || object_list > node_table || node_table > payload.len() {
        return Err(zone_error(0x10, "lyb graph offsets are outside payload"));
    }
    let object_count = (node_table - object_list) / 4;
    if object_count > MAX_RECORDS as usize || (node_table - object_list) % 4 != 0 {
        return Err(zone_error(0x14, "lyb object list is invalid"));
    }
    let mut objects = Vec::with_capacity(object_count);
    for index in 0..object_count {
        objects.push(u32_at(payload, object_list + index * 4)?);
    }
    let first_fields = u32_at(payload, node_table + 4)? as usize;
    if first_fields < node_table
        || first_fields > payload.len()
        || (first_fields - node_table) % 16 != 0
    {
        return Err(zone_error(
            node_table as u64 + 4,
            "lyb node type table is invalid",
        ));
    }
    let type_count = (first_fields - node_table) / 16;
    if type_count == 0 || type_count > MAX_RECORDS as usize {
        return Err(zone_error(
            node_table as u64,
            "lyb node type table is empty",
        ));
    }
    let mut node_types = Vec::with_capacity(type_count);
    for index in 0..type_count {
        let at = node_table + index * 16;
        let extent_offset = u32_at(payload, at)? as usize;
        let field_list = u32_at(payload, at + 4)? as usize;
        let field_count = bounded_u32(u32_at(payload, at + 12)?, at as u64 + 12, "lyb field")?;
        let field_bytes = (field_count as usize)
            .checked_mul(4)
            .ok_or_else(|| zone_error(field_list as u64, "lyb field list overflows"))?;
        if field_list
            .checked_add(field_bytes)
            .map_or(true, |end| end > payload.len())
        {
            return Err(zone_error(
                field_list as u64,
                "lyb field list is outside payload",
            ));
        }
        let mut fields = Vec::with_capacity(field_count as usize);
        for field in 0..field_count as usize {
            let at = field_list + field * 4;
            fields.push((u16_at(payload, at)?, u16_at(payload, at + 2)?));
        }
        let data_offset = u32_at(payload, extent_offset)?;
        let node_count = bounded_u32(
            u32_at(payload, extent_offset + 4)?,
            extent_offset as u64 + 4,
            "lyb node",
        )?;
        let node_size = u32_at(payload, extent_offset + 8)?;
        node_types.push(LyNodeType {
            data_offset,
            node_count,
            node_size,
            fields,
        });
    }
    let mut type_index = BTreeMap::new();
    for (index, kind) in node_types.iter().enumerate() {
        let span = (kind.node_count as usize)
            .checked_mul(kind.node_size as usize)
            .ok_or_else(|| zone_error(0, "lyb node extent overflows"))?;
        let end = kind.data_offset as usize + span;
        if end > payload.len() || kind.node_size == 0 {
            return Err(zone_error(
                kind.data_offset as u64,
                "lyb node extent is outside payload",
            ));
        }
        for node in 0..kind.node_count {
            type_index.insert(kind.data_offset + node * kind.node_size, index);
        }
    }
    let graph = LyGraph {
        payload,
        node_types,
        objects,
        type_index,
    };
    let mut zone = Transform::default();
    let mut zone_name = String::new();
    for &object in &graph.objects {
        if let Some(path) = graph.class_path(object)? {
            if path.starts_with("MiscObjects/LaySettings/") {
                graph.require_shape(object, 0x70, LAY_SETTINGS_FIELDS, "LaySettings")?;
                zone_name = graph
                    .node_name(object)?
                    .trim_end_matches("/Test")
                    .to_string();
                zone = graph.read_transform(object, 0x40, 0x4C, 0x50)?;
                break;
            }
        }
    }
    if zone_name.is_empty() {
        zone_name = graph.node_name(
            *graph
                .objects
                .first()
                .ok_or_else(|| zone_error(0, "lyb object list is empty"))?,
        )?;
    }
    if graph.objects.len() < 2 {
        return Err(zone_error(0, "lyb object list has no scene objects"));
    }
    let mut objects_out = Vec::new();
    let mut path = WalkPath {
        transforms: [Transform::default(); 5],
        groups: Vec::new(),
    };
    for &object in graph.objects.iter().skip(1) {
        if graph.class_path(object)?.as_deref() == Some("RefObjects/InstanceObject") {
            graph.walk_instance(object, zone, &resources, &mut path, &mut objects_out, 0)?;
        }
    }
    Ok(Layout {
        zone_name,
        zone,
        objects: objects_out,
        resources,
    })
}

impl<'a> LyGraph<'a> {
    fn node_kind(&self, offset: u32) -> Result<(&LyNodeType, usize)> {
        let index = *self
            .type_index
            .get(&offset)
            .ok_or_else(|| zone_error(offset as u64, "lyb reference does not name a node"))?;
        Ok((&self.node_types[index], index))
    }
    fn node_bytes(&self, offset: u32) -> Result<&[u8]> {
        let (kind, _) = self.node_kind(offset)?;
        self.payload
            .get(offset as usize..offset as usize + kind.node_size as usize)
            .ok_or_else(|| zone_error(offset as u64, "lyb node is outside payload"))
    }
    fn class_path(&self, offset: u32) -> Result<Option<String>> {
        let node = self.node_bytes(offset)?;
        let (kind, _) = self.node_kind(offset)?;
        if !kind.fields.contains(&(4, 0)) {
            return Ok(None);
        }
        if node.len() < 8 {
            return Ok(None);
        }
        let class = u32_le(node, 4)?;
        if class == 0 {
            return Ok(None);
        }
        let class_node = self.node_bytes(class)?;
        let (class_kind, _) = self.node_kind(class)?;
        if !exact_node_shape(class_kind, 0x14, CLASS_NODE_FIELDS) {
            return Err(zone_error(
                class as u64,
                "lyb class node has an unsupported width or field descriptor set",
            ));
        }
        if class_node.len() < 12 {
            return Ok(None);
        }
        Ok(Some(self.string(u32_le(class_node, 8)?)?))
    }
    fn require_shape(
        &self,
        node: u32,
        node_size: u32,
        fields: &[(u16, u16)],
        name: &str,
    ) -> Result<()> {
        let (kind, _) = self.node_kind(node)?;
        if !exact_node_shape(kind, node_size, fields) {
            return Err(zone_error(
                node as u64,
                format!("lyb {name} node has an unsupported width or field descriptor set"),
            ));
        }
        Ok(())
    }
    fn require_group_shape(&self, node: u32) -> Result<()> {
        let (kind, _) = self.node_kind(node)?;
        if !exact_node_shape(kind, 0x30, GROUP_FIELDS)
            && !exact_node_shape(kind, 0x30, GROUP_FIELDS_EXTENDED)
        {
            return Err(zone_error(
                node as u64,
                "lyb group node has an unsupported width or field descriptor set",
            ));
        }
        Ok(())
    }
    fn node_name(&self, offset: u32) -> Result<String> {
        let node = self.node_bytes(offset)?;
        if node.len() < 12 {
            return Ok(String::new());
        }
        self.string(u32_le(node, 8)?)
    }
    fn string(&self, offset: u32) -> Result<String> {
        if offset == 0 {
            return Ok(String::new());
        }
        let start = offset as usize;
        let bytes = self
            .payload
            .get(start..)
            .ok_or_else(|| zone_error(offset as u64, "lyb string is outside payload"))?;
        let end = bytes
            .iter()
            .position(|byte| *byte == 0)
            .ok_or_else(|| zone_error(offset as u64, "lyb string is unterminated"))?;
        std::str::from_utf8(&bytes[..end])
            .map(str::to_string)
            .map_err(|_| FormatError::new(ErrorKind::InvalidUtf8, offset as u64, "lyb string"))
    }
    fn read_transform(
        &self,
        node: u32,
        translation: usize,
        rotation: usize,
        scale: usize,
    ) -> Result<Transform> {
        let bytes = self.node_bytes(node)?;
        let translation = Vec3 {
            x: f32_le(bytes, translation)?,
            y: f32_le(bytes, translation + 4)?,
            z: f32_le(bytes, translation + 8)?,
        };
        let rotation_ref = u32_le(bytes, rotation)?;
        let scale_ref = u32_le(bytes, scale)?;
        let rotation = self.pool_vec(rotation_ref, Vec3::ZERO)?;
        let scale = self.pool_vec(scale_ref, Vec3::ONE)?;
        Ok(Transform {
            translation,
            rotation,
            scale,
        })
    }
    fn pool_vec(&self, offset: u32, default: Vec3) -> Result<Vec3> {
        if offset == 0 {
            return Ok(default);
        }
        let bytes = self
            .payload
            .get(offset as usize..offset as usize + 16)
            .ok_or_else(|| {
                zone_error(
                    offset as u64,
                    "lyb transform pool record is outside payload",
                )
            })?;
        Ok(Vec3 {
            x: f32_le(bytes, 0)?,
            y: f32_le(bytes, 4)?,
            z: f32_le(bytes, 8)?,
        })
    }
    fn node_run(&self, offset: u32, count: u32) -> Result<Vec<u32>> {
        let count = bounded_u32(count, offset as u64, "lyb reference")?;
        if offset == 0 && count != 0 {
            return Err(zone_error(
                offset as u64,
                format!("lyb null reference has nonzero count {count}"),
            ));
        }
        if count == 0 {
            return Ok(Vec::new());
        }
        let (kind, _) = self.node_kind(offset)?;
        let span = (count as usize)
            .checked_mul(kind.node_size as usize)
            .ok_or_else(|| zone_error(offset as u64, "lyb node run overflows"))?;
        let run_end = (offset as usize)
            .checked_add(span)
            .ok_or_else(|| zone_error(offset as u64, "lyb node run overflows"))?;
        let extent_end = (kind.data_offset as usize)
            .checked_add(
                (kind.node_count as usize)
                    .checked_mul(kind.node_size as usize)
                    .ok_or_else(|| zone_error(offset as u64, "lyb node extent overflows"))?,
            )
            .ok_or_else(|| zone_error(offset as u64, "lyb node extent overflows"))?;
        if (offset as usize) < kind.data_offset as usize || run_end > extent_end {
            return Err(zone_error(
                offset as u64,
                "lyb node run is outside its extent",
            ));
        }
        (0..count)
            .map(|index| {
                let node = offset + index * kind.node_size;
                self.node_kind(node).map(|_| node)
            })
            .collect()
    }
    fn walk_instance(
        &self,
        node: u32,
        zone: Transform,
        resources: &[LayoutResource],
        path: &mut WalkPath,
        output: &mut Vec<LayoutObject>,
        depth: usize,
    ) -> Result<()> {
        if depth > MAX_GRAPH_DEPTH {
            return Err(zone_error(node as u64, "lyb scene graph is too deep"));
        }
        let class = self.class_path(node)?.unwrap_or_default();
        if class == "RefObjects/InstanceObject" {
            self.require_shape(node, 0x40, INSTANCE_FIELDS, "InstanceObject")?;
            path.groups.clear();
            path.transforms[1] = Transform::default();
            path.transforms[2] = Transform::default();
            path.transforms[4] = Transform::default();
            path.transforms[0] = self.read_transform(node, 0x20, 0x2C, 0x30)?;
            let bytes = self.node_bytes(node)?;
            let base = u32_le(bytes, 0x3C)?;
            self.walk_base(base, zone, resources, path, output, depth + 1)?;
            let children = self.node_run(u32_le(bytes, 0x34)?, u32_le(bytes, 0x38)?)?;
            for child in children {
                if self.is_instance_child_range(child)? {
                    continue;
                }
                self.require_shape(child, 0x30, CHILD_FIELDS, "child")?;
                self.walk_child(child, zone, resources, path, output, depth + 1)?;
            }
        }
        Ok(())
    }
    fn is_instance_child_range(&self, node: u32) -> Result<bool> {
        let (kind, _) = self.node_kind(node)?;
        if !exact_node_shape(kind, 0x0C, INSTANCE_CHILD_RANGE_FIELDS) {
            return Ok(false);
        }
        let bytes = self.node_bytes(node)?;
        let target = u32_le(bytes, 0)? as usize;
        let count = bounded_u32(u32_le(bytes, 4)?, node as u64 + 4, "lyb child range")?;
        if (count != 0 && target >= self.payload.len()) || target > self.payload.len() {
            return Err(zone_error(
                node as u64,
                "lyb child range points outside payload",
            ));
        }
        Ok(true)
    }
    fn walk_child(
        &self,
        node: u32,
        zone: Transform,
        resources: &[LayoutResource],
        path: &mut WalkPath,
        output: &mut Vec<LayoutObject>,
        depth: usize,
    ) -> Result<()> {
        if depth > MAX_GRAPH_DEPTH {
            return Err(zone_error(node as u64, "lyb scene graph is too deep"));
        }
        let bytes = self.node_bytes(node)?;
        self.require_shape(node, 0x30, CHILD_FIELDS, "child")?;
        let transform = self.read_transform(node, 0, 0x0C, 0x10)?;
        let previous = path.transforms[4];
        path.transforms[4] = transform;
        self.walk_base(
            u32_le(bytes, 0x18)?,
            zone,
            resources,
            path,
            output,
            depth + 1,
        )?;
        path.transforms[4] = previous;
        Ok(())
    }
    fn walk_base(
        &self,
        node: u32,
        zone: Transform,
        resources: &[LayoutResource],
        path: &mut WalkPath,
        output: &mut Vec<LayoutObject>,
        depth: usize,
    ) -> Result<()> {
        if node == 0 {
            return Ok(());
        }
        if depth > MAX_GRAPH_DEPTH {
            return Err(zone_error(node as u64, "lyb scene graph is too deep"));
        }
        let class = self.class_path(node)?.unwrap_or_default();
        if class == "RefObjects/UnitTree/UnitTreeObject" {
            self.require_shape(node, 0x50, UNIT_TREE_FIELDS, "UnitTree")?;
            let bytes = self.node_bytes(node)?;
            let previous = path.transforms[1];
            path.groups.clear();
            path.transforms[2] = Transform::default();
            path.transforms[1] = self.read_transform(node, 0x10, 0x1C, 0x20)?;
            for group in self.node_run(u32_le(bytes, 0x24)?, u32_le(bytes, 0x28)?)? {
                self.require_group_shape(group)?;
                self.walk_group(group, zone, resources, path, output, depth + 1)?;
            }
            for child in self.node_run(u32_le(bytes, 0x30)?, u32_le(bytes, 0x34)?)? {
                self.require_shape(child, 0x30, CHILD_FIELDS, "child")?;
                self.walk_child(child, zone, resources, path, output, depth + 1)?;
            }
            path.transforms[1] = previous;
        } else if class == "BaseObjects/Attribute/AttributeBaseObject" {
            self.require_shape(node, 0x2C, ATTRIBUTE_FIELDS, "Attribute")?;
            let bytes = self.node_bytes(node)?;
            let key = self.string(u32_le(bytes, 0x24)?)?;
            let (model, phb) = resolve_resource_key(resources, &key);
            if phb.is_none() && model.is_none() {
                return Err(zone_error(
                    node as u64,
                    format!(
                        "Attribute resource key '{key}' has no documented model or PHB binding"
                    ),
                ));
            }
            if phb.is_some() || model.is_some() {
                let source_block = self.node_kind(node)?.0.data_offset;
                output.push(LayoutObject {
                    model,
                    phb,
                    source_node: node,
                    source_block,
                    resource_key: Some(key),
                    instance: path.transforms[0],
                    unit_tree: path.transforms[1],
                    child: path.transforms[4],
                    boxes: Vec::new(),
                    world_matrix: placement_matrix(zone, path),
                    group_chain: path.groups.clone(),
                });
            }
        } else if class == "BaseObjects/CollisionBox/CollisionBoxBaseObject" {
            self.require_shape(node, 0x14, COLLISION_BOX_FIELDS, "CollisionBox")?;
            let bytes = self.node_bytes(node)?;
            let kind = u32_le(bytes, 0x0C)?;
            let source_block = self.node_kind(node)?.0.data_offset;
            output.push(LayoutObject {
                model: None,
                phb: None,
                source_node: node,
                source_block,
                resource_key: None,
                instance: path.transforms[0],
                unit_tree: path.transforms[1],
                child: path.transforms[4],
                boxes: vec![CollisionPrimitive {
                    kind,
                    transform: Transform::default(),
                    center: Vec3 {
                        x: 0.0,
                        y: 0.5,
                        z: 0.0,
                    },
                    half_extents: Vec3 {
                        x: 0.5,
                        y: 0.5,
                        z: 0.5,
                    },
                }],
                world_matrix: placement_matrix(zone, path),
                group_chain: path.groups.clone(),
            });
        }
        Ok(())
    }
    fn walk_group(
        &self,
        node: u32,
        zone: Transform,
        resources: &[LayoutResource],
        path: &mut WalkPath,
        output: &mut Vec<LayoutObject>,
        depth: usize,
    ) -> Result<()> {
        if depth > MAX_GRAPH_DEPTH {
            return Err(zone_error(node as u64, "lyb scene graph is too deep"));
        }
        let bytes = self.node_bytes(node)?;
        self.require_group_shape(node)?;
        let transform = self.read_transform(node, 0, 0x0C, 0x10)?;
        let previous = path.transforms[2];
        path.transforms[2] = transform;
        path.groups.push(transform);
        for group in self.node_run(u32_le(bytes, 0x14)?, u32_le(bytes, 0x18)?)? {
            self.walk_group(group, zone, resources, path, output, depth + 1)?;
        }
        for child in self.node_run(u32_le(bytes, 0x20)?, u32_le(bytes, 0x24)?)? {
            self.require_shape(child, 0x30, CHILD_FIELDS, "child")?;
            self.walk_child(child, zone, resources, path, output, depth + 1)?;
        }
        path.groups.pop();
        path.transforms[2] = previous;
        Ok(())
    }
}

fn placement_matrix(zone: Transform, path: &WalkPath) -> Matrix4 {
    let mut transforms = vec![zone, path.transforms[0], path.transforms[1]];
    transforms.extend(path.groups.iter().copied());
    transforms.push(path.transforms[4]);
    compose_transforms(&transforms)
}

fn resolve_resource_key(
    resources: &[LayoutResource],
    key: &str,
) -> (Option<ResourceId>, Option<ResourceId>) {
    let mut model = None;
    let mut phb = None;
    for resource in resources.iter().filter(|resource| resource.name == key) {
        match resource.type_tag.as_str() {
            "brt" => model = Some(resource.resource_id),
            "bhp" => phb = Some(resource.resource_id),
            _ => {}
        }
    }
    (model, phb)
}

#[derive(Debug, Clone)]
struct Chunk<'a> {
    tag: [u8; 4],
    bytes: &'a [u8],
    children: Vec<Chunk<'a>>,
}

fn parse_wrb_chunks(
    bytes: &[u8],
    comp: Option<(Vec3, Vec3)>,
    parts: &mut Vec<MeshPart>,
) -> Result<()> {
    let chunks = parse_chunks(bytes)?;
    for chunk in &chunks {
        let next_comp = if chunk.tag == *b"COMP" {
            Some(read_box(chunk.bytes)?)
        } else {
            comp
        };
        if chunk.tag == *b"MESH" {
            parse_mesh_chunk(chunk, next_comp.or(comp), parts)?;
        } else if chunk.tag == *b"MDL\0" || chunk.tag == *b"MDLC" || chunk.tag == *b"WRB\0" {
            parse_chunk_children(chunk, next_comp, parts)?;
        }
    }
    Ok(())
}

fn parse_chunk_children<'a>(
    chunk: &Chunk<'a>,
    comp: Option<(Vec3, Vec3)>,
    parts: &mut Vec<MeshPart>,
) -> Result<()> {
    let current_comp = find_comp(chunk)?.or(comp);
    for child in &chunk.children {
        if child.tag == *b"COMP" {
            continue;
        }
        let next_comp = current_comp;
        if child.tag == *b"MESH" {
            parse_mesh_chunk(child, next_comp, parts)?;
        } else if !child.children.is_empty() {
            parse_chunk_children(child, next_comp, parts)?;
        }
    }
    Ok(())
}

fn parse_mesh_chunk(
    chunk: &Chunk<'_>,
    comp: Option<(Vec3, Vec3)>,
    parts: &mut Vec<MeshPart>,
) -> Result<()> {
    let mut vertices = None;
    let mut indices = None;
    for child in &chunk.children {
        if child.tag != *b"STMS" {
            continue;
        }
        let (fields, item_count, stride, data) = parse_stms(child.bytes)?;
        if stride == 2
            && fields.len() == 1
            && fields[0].0 == 0
            && fields[0].1 == 0
            && fields[0].2 == 1
            && fields[0].3 >> 16 == 0xFF
        {
            let mut values = Vec::with_capacity(item_count as usize);
            for index in 0..item_count as usize {
                values.push(u16_be(data, index * 2)? as u32);
            }
            indices = Some(values);
        } else if let Some(position) = fields
            .iter()
            .find(|field| field.3 >> 16 == 0 && field.1 == 4 && field.2 == 4)
        {
            let Some((min, max)) = comp else {
                return Err(zone_error(0, "model position stream has no COMP box"));
            };
            let field_offset = position.0 as usize;
            if field_offset
                .checked_add(8)
                .map_or(true, |end| end > stride as usize)
            {
                return Err(zone_error(
                    0,
                    "model position field extends beyond stream stride",
                ));
            }
            let mut values = Vec::with_capacity(item_count as usize);
            for index in 0..item_count as usize {
                let at = index * stride as usize + field_offset;
                let raw = [
                    i16_be(data, at)?,
                    i16_be(data, at + 2)?,
                    i16_be(data, at + 4)?,
                ];
                values.push(Vec3 {
                    x: (min.x + max.x) * 0.5 + raw[0] as f32 / 32767.0 * (max.x - min.x) * 0.5,
                    y: (min.y + max.y) * 0.5 + raw[1] as f32 / 32767.0 * (max.y - min.y) * 0.5,
                    z: (min.z + max.z) * 0.5 + raw[2] as f32 / 32767.0 * (max.z - min.z) * 0.5,
                });
            }
            vertices = Some(values);
        }
    }
    let (Some(vertices), Some(indices)) = (vertices, indices) else {
        return Err(zone_error(
            0,
            "model MESH lacks a supported position or index stream",
        ));
    };
    if indices.len() % 3 != 0
        || indices
            .iter()
            .any(|index| *index as usize >= vertices.len())
    {
        return Err(zone_error(0, "model index stream is not a triangle list"));
    }
    parts.push(MeshPart {
        classification: MeshClassification::Render,
        vertices,
        indices,
    });
    Ok(())
}

fn find_comp(chunk: &Chunk<'_>) -> Result<Option<(Vec3, Vec3)>> {
    for child in &chunk.children {
        if child.tag == *b"COMP" {
            return Ok(Some(read_box(child.bytes)?));
        }
        if let Some(comp) = find_comp(child)? {
            return Ok(Some(comp));
        }
    }
    Ok(None)
}

#[derive(Debug, Clone, Copy)]
struct StreamField(u32, u32, u32, u32);

fn parse_stms(bytes: &[u8]) -> Result<(Vec<StreamField>, u32, u32, &[u8])> {
    let fields_count = bounded_u32(u32_be(bytes, 0)?, 0, "STMS field")?;
    let item_count = bounded_u32(u32_be(bytes, 4)?, 4, "STMS item")?;
    let stride = u32_be(bytes, 8)?;
    let fields_start = 16usize;
    let fields_bytes = (fields_count as usize)
        .checked_mul(16)
        .ok_or_else(|| zone_error(0, "STMS fields overflow"))?;
    let data_start = fields_start
        .checked_add(fields_bytes)
        .ok_or_else(|| zone_error(0, "STMS data offset overflows"))?;
    let data_bytes = (item_count as usize)
        .checked_mul(stride as usize)
        .ok_or_else(|| zone_error(0, "STMS data size overflows"))?;
    if data_start + data_bytes != bytes.len() {
        return Err(zone_error(0, "STMS size does not match its stream"));
    }
    let mut fields = Vec::with_capacity(fields_count as usize);
    for index in 0..fields_count as usize {
        let at = fields_start + index * 16;
        fields.push(StreamField(
            u32_be(bytes, at)?,
            u32_be(bytes, at + 4)?,
            u32_be(bytes, at + 8)?,
            u32_be(bytes, at + 12)?,
        ));
    }
    Ok((fields, item_count, stride, &bytes[data_start..]))
}

fn parse_chunks(bytes: &[u8]) -> Result<Vec<Chunk<'_>>> {
    parse_chunks_at_depth(bytes, 0)
}

fn parse_chunks_at_depth(bytes: &[u8], depth: usize) -> Result<Vec<Chunk<'_>>> {
    if depth > MAX_MODEL_CHUNK_DEPTH {
        return Err(zone_error(
            depth as u64,
            "model chunk nesting exceeds the depth limit",
        ));
    }
    let mut chunks = Vec::new();
    let mut offset = 0usize;
    while offset + 16 <= bytes.len() {
        if bytes[offset..offset + 4].iter().all(|byte| *byte == 0) {
            break;
        }
        let tag = bytes[offset..offset + 4]
            .try_into()
            .map_err(|_| zone_error(offset as u64, "chunk tag"))?;
        reject_known_tag_variant(&tag, offset)?;
        let size = u32_be(bytes, offset + 8)? as usize;
        let padded = u32_be(bytes, offset + 12)? as usize;
        if size < 16 || offset + size > bytes.len() {
            return Err(zone_error(
                offset as u64 + 8,
                "chunk size is outside payload",
            ));
        }
        let child_bytes = &bytes[offset + 16..offset + size];
        let children = if tag == *b"WRB\0"
            || tag == *b"MDLC"
            || tag == *b"MDL\0"
            || tag == *b"MESH"
            || tag == *b"AABB"
        {
            let nested = child_bytes
                .get(16..)
                .ok_or_else(|| zone_error(offset as u64, "container chunk has no info block"))?;
            parse_chunks_at_depth(nested, depth + 1)?
        } else {
            Vec::new()
        };
        chunks.push(Chunk {
            tag,
            bytes: child_bytes,
            children,
        });
        let advance = if padded >= size {
            padded
        } else {
            (size + 15) & !15
        };
        if advance == 0 || offset + advance > bytes.len() {
            return Err(zone_error(
                offset as u64 + 12,
                "chunk padded size is outside payload",
            ));
        }
        offset += advance;
    }
    Ok(chunks)
}

fn reject_known_tag_variant(tag: &[u8; 4], offset: usize) -> Result<()> {
    let malformed = (tag.starts_with(b"WRB") && *tag != *b"WRB\0")
        || (tag.starts_with(b"MDL") && *tag != *b"MDL\0" && *tag != *b"MDLC")
        || (tag.starts_with(b"PHB") && *tag != *b"PHB\0");
    if malformed {
        return Err(zone_error(offset as u64, "unsupported non-exact model tag"));
    }
    Ok(())
}

fn read_box(bytes: &[u8]) -> Result<(Vec3, Vec3)> {
    Ok((
        Vec3 {
            x: f32_be(bytes, 0)?,
            y: f32_be(bytes, 4)?,
            z: f32_be(bytes, 8)?,
        },
        Vec3 {
            x: f32_be(bytes, 12)?,
            y: f32_be(bytes, 16)?,
            z: f32_be(bytes, 20)?,
        },
    ))
}

fn fixed_text(data: &[u8], offset: usize, length: usize) -> Result<String> {
    let bytes = data
        .get(offset..offset + length)
        .ok_or_else(|| zone_error(offset as u64, "fixed text is outside input"))?;
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    std::str::from_utf8(&bytes[..end])
        .map(str::to_string)
        .map_err(|_| FormatError::new(ErrorKind::InvalidUtf8, offset as u64, "fixed text"))
}

fn bounded_u32(value: u32, offset: u64, field: &str) -> Result<u32> {
    if value > MAX_RECORDS {
        return Err(zone_error(offset, format!("{field} count is too large")));
    }
    Ok(value)
}

fn u32_at(data: &[u8], offset: usize) -> Result<u32> {
    u32_le(data, offset)
}
fn u32_le(data: &[u8], offset: usize) -> Result<u32> {
    let bytes = data
        .get(offset..offset + 4)
        .ok_or_else(|| zone_error(offset as u64, "u32 is outside input"))?;
    Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}
fn u16_le(data: &[u8], offset: usize) -> Result<u16> {
    let bytes = data
        .get(offset..offset + 2)
        .ok_or_else(|| zone_error(offset as u64, "u16 is outside input"))?;
    Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
}
fn u16_at(data: &[u8], offset: usize) -> Result<u16> {
    u16_le(data, offset)
}
fn u32_be(data: &[u8], offset: usize) -> Result<u32> {
    let bytes = data
        .get(offset..offset + 4)
        .ok_or_else(|| zone_error(offset as u64, "u32 is outside input"))?;
    Ok(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}
fn u16_be(data: &[u8], offset: usize) -> Result<u16> {
    let bytes = data
        .get(offset..offset + 2)
        .ok_or_else(|| zone_error(offset as u64, "u16 is outside input"))?;
    Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
}
fn i16_be(data: &[u8], offset: usize) -> Result<i16> {
    Ok(u16_be(data, offset)? as i16)
}
fn f32_le(data: &[u8], offset: usize) -> Result<f32> {
    Ok(f32::from_le_bytes(u32_le(data, offset)?.to_le_bytes()))
}
fn f32_be(data: &[u8], offset: usize) -> Result<f32> {
    Ok(f32::from_bits(u32_be(data, offset)?))
}

fn zone_error(offset: u64, detail: impl Into<String>) -> FormatError {
    FormatError::new(ErrorKind::InvalidAttributeValue, offset, detail)
}
fn obj_float(value: f32) -> String {
    if value == 0.0 {
        "0".into()
    } else {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(x: f32, y: f32, z: f32) -> Transform {
        Transform {
            translation: Vec3 { x, y, z },
            ..Transform::default()
        }
    }

    #[test]
    fn transform_order_is_y_up_and_complete() {
        let transform = Transform {
            translation: Vec3 {
                x: 10.0,
                y: 20.0,
                z: 30.0,
            },
            rotation: Vec3 {
                x: 0.0,
                y: 0.0,
                z: std::f32::consts::FRAC_PI_2,
            },
            scale: Vec3 {
                x: 2.0,
                y: 3.0,
                z: 4.0,
            },
        };
        let point = transform_matrix(transform).transform_point(Vec3 {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        });
        assert!((point.x - 10.0).abs() < 1e-5);
        assert!((point.y - 22.0).abs() < 1e-5);
        assert!((point.z - 30.0).abs() < 1e-5);
    }

    #[test]
    fn flatten_collision_only_preserves_phb_surface_and_winding() {
        let layout = Layout {
            zone_name: "test".into(),
            zone: Transform::default(),
            resources: Vec::new(),
            objects: vec![
                LayoutObject {
                    model: Some(ResourceId::new(9)),
                    phb: Some(ResourceId::new(2)),
                    source_node: 0x100,
                    source_block: 0x80,
                    resource_key: Some("mesh".into()),
                    instance: t(1.0, 0.0, 0.0),
                    unit_tree: Transform::default(),
                    group_chain: Vec::new(),
                    child: Transform::default(),
                    boxes: Vec::new(),
                    world_matrix: transform_matrix(t(1.0, 0.0, 0.0)),
                },
                LayoutObject {
                    model: None,
                    phb: None,
                    source_node: 0x200,
                    source_block: 0x180,
                    resource_key: None,
                    instance: Transform::default(),
                    unit_tree: Transform::default(),
                    group_chain: Vec::new(),
                    child: Transform::default(),
                    boxes: vec![CollisionPrimitive {
                        kind: 0x02030000,
                        transform: Transform::default(),
                        center: Vec3 {
                            x: 0.0,
                            y: 0.5,
                            z: 0.0,
                        },
                        half_extents: Vec3 {
                            x: 0.5,
                            y: 0.5,
                            z: 0.5,
                        },
                    }],
                    world_matrix: transform_matrix(Transform::default()),
                },
            ],
        };
        let phb = Phb {
            hulls: vec![PhbHull {
                surface: 0xCAFE,
                surfaces: vec![0xCAFE, 0xBEEF],
                vertices: vec![
                    Vec3::ZERO,
                    Vec3 {
                        x: 1.0,
                        y: 0.0,
                        z: 0.0,
                    },
                    Vec3 {
                        x: 0.0,
                        y: 1.0,
                        z: 0.0,
                    },
                ],
                indices: vec![0, 2, 1, 0, 1, 2],
                declared_min: Vec3::ZERO,
                declared_max: Vec3 {
                    x: 1.0,
                    y: 1.0,
                    z: 0.0,
                },
                decoded_min: Vec3::ZERO,
                decoded_max: Vec3 {
                    x: 1.0,
                    y: 1.0,
                    z: 0.0,
                },
            }],
        };
        let flattened = flatten_zone(&layout, &[(ResourceId::new(2), phb)]).unwrap();
        assert_eq!(flattened.faces[0].indices, [0, 2, 1]);
        assert_eq!(flattened.faces[0].source.raw_surface, Some(0xCAFE));
        assert_eq!(
            flattened.faces[0].source.model_resource_id,
            Some(ResourceId::new(9))
        );
        assert_eq!(flattened.faces[2].source.kind, "collision-box");
        assert_eq!(
            flattened.faces[2].source.classification_raw,
            Some(0x02030000)
        );
        assert_eq!(flattened.faces[2].source.raw_surface, Some(0));
        assert_eq!(
            flattened.faces[2..]
                .iter()
                .map(|face| face.indices)
                .collect::<Vec<_>>(),
            vec![
                [3, 4, 6],
                [3, 6, 5],
                [5, 6, 8],
                [5, 8, 7],
                [7, 8, 10],
                [7, 10, 9],
                [9, 10, 4],
                [9, 4, 3],
                [4, 10, 8],
                [4, 8, 6],
                [9, 3, 5],
                [9, 5, 7],
            ]
        );
        assert!(flattened.obj().contains("f 1 3 2\n"));
        let metadata = flattened.metadata("test", &[]);
        let ranges = metadata["faceRanges"].as_array().unwrap();
        assert_eq!(ranges.len(), 3);
        assert_eq!(ranges[0]["faceCount"], 1);
        assert_eq!(ranges[1]["faceCount"], 1);
        assert_eq!(ranges[0]["source"]["rawSurface"], 0xCAFE);
        assert_eq!(ranges[1]["source"]["rawSurface"], 0xBEEF);
        assert_eq!(ranges[2]["faceCount"], 12);
        assert!(ranges[2]["source"]["resourceId"].is_null());
        assert_eq!(ranges[2]["source"]["classificationRaw"], 0x02030000);
        assert_eq!(ranges[2]["source"]["rawSurface"], 0);
        assert_eq!(
            metadata["collisionResources"][0]["resourceId"],
            "0x00000002"
        );
        assert_eq!(
            metadata["collisionResources"][0]["hulls"][0]["vertexCount"],
            3
        );
        assert_eq!(
            metadata["collisionResources"][0]["hulls"][0]["triangleCount"],
            2
        );
        assert_eq!(
            metadata["collisionResources"][0]["hulls"][0]["declaredMin"],
            json!([0.0, 0.0, 0.0])
        );
        assert_eq!(
            ranges
                .iter()
                .map(|range| range["faceCount"].as_u64().unwrap())
                .sum::<u64>(),
            flattened.faces.len() as u64
        );
    }

    #[test]
    fn nested_group_transforms_accumulate_in_order() {
        let layout = Layout {
            zone_name: "nested".into(),
            zone: Transform::default(),
            resources: Vec::new(),
            objects: vec![LayoutObject {
                model: None,
                phb: Some(ResourceId::new(2)),
                source_node: 1,
                source_block: 1,
                resource_key: Some("mesh".into()),
                instance: Transform::default(),
                unit_tree: Transform::default(),
                group_chain: vec![
                    t(1.0, 0.0, 0.0),
                    Transform {
                        translation: Vec3::ZERO,
                        rotation: Vec3 {
                            x: 0.0,
                            y: 0.0,
                            z: std::f32::consts::FRAC_PI_2,
                        },
                        scale: Vec3::ONE,
                    },
                ],
                child: Transform::default(),
                boxes: Vec::new(),
                world_matrix: compose_transforms(&[
                    t(1.0, 0.0, 0.0),
                    Transform {
                        translation: Vec3::ZERO,
                        rotation: Vec3 {
                            x: 0.0,
                            y: 0.0,
                            z: std::f32::consts::FRAC_PI_2,
                        },
                        scale: Vec3::ONE,
                    },
                ]),
            }],
        };
        let phb = Phb {
            hulls: vec![PhbHull {
                surface: 1,
                surfaces: vec![1],
                vertices: vec![
                    Vec3::ZERO,
                    Vec3 {
                        x: 1.0,
                        y: 0.0,
                        z: 0.0,
                    },
                    Vec3 {
                        x: 0.0,
                        y: 1.0,
                        z: 0.0,
                    },
                ],
                indices: vec![0, 1, 2],
                declared_min: Vec3::ZERO,
                declared_max: Vec3::ONE,
                decoded_min: Vec3::ZERO,
                decoded_max: Vec3::ONE,
            }],
        };
        let flattened = flatten_zone(&layout, &[(ResourceId::new(2), phb)]).unwrap();
        assert!((flattened.vertices[1].x - 1.0).abs() < 1e-5);
        assert!((flattened.vertices[1].y - 1.0).abs() < 1e-5);
    }

    fn put_le_u32(bytes: &mut [u8], offset: usize, value: u32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn put_be_u32(bytes: &mut [u8], offset: usize, value: u32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
    }

    fn put_be_f32(bytes: &mut [u8], offset: usize, value: f32) {
        put_be_u32(bytes, offset, value.to_bits());
    }

    fn chunk(tag: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let size = 16 + payload.len();
        let padded = (size + 15) & !15;
        let mut bytes = vec![0u8; padded];
        bytes[..4].copy_from_slice(tag);
        put_be_u32(&mut bytes, 8, size as u32);
        put_be_u32(&mut bytes, 12, padded as u32);
        bytes[16..16 + payload.len()].copy_from_slice(payload);
        bytes
    }

    fn container_chunk(tag: &[u8; 4], children: &[Vec<u8>]) -> Vec<u8> {
        let mut payload = vec![0u8; 16];
        for child in children {
            payload.extend_from_slice(child);
        }
        chunk(tag, &payload)
    }

    fn stms_position_with_field_offset(field_offset: u32) -> Vec<u8> {
        let mut payload = vec![0u8; 16 + 32 + 3 * 12];
        put_be_u32(&mut payload, 0, 2);
        put_be_u32(&mut payload, 4, 3);
        put_be_u32(&mut payload, 8, 12);
        put_be_u32(&mut payload, 16 + 4, 3);
        put_be_u32(&mut payload, 16 + 8, 4);
        put_be_u32(&mut payload, 16 + 12, 0x02000000);
        put_be_u32(&mut payload, 32, field_offset);
        put_be_u32(&mut payload, 32 + 4, 4);
        put_be_u32(&mut payload, 32 + 8, 4);
        for (index, values) in [
            [0i16, 0, 0, 32767],
            [32767, 0, 0, 32767],
            [0, 32767, 0, 32767],
        ]
        .into_iter()
        .enumerate()
        {
            let at = 48 + index * 12;
            for (component, value) in values.into_iter().enumerate() {
                payload[at + component * 2..at + component * 2 + 2]
                    .copy_from_slice(&value.to_be_bytes());
            }
        }
        chunk(b"STMS", &payload)
    }

    fn stms_position() -> Vec<u8> {
        stms_position_with_field_offset(0)
    }

    fn stms_indices() -> Vec<u8> {
        let mut payload = vec![0u8; 16 + 16 + 6];
        put_be_u32(&mut payload, 0, 1);
        put_be_u32(&mut payload, 4, 3);
        put_be_u32(&mut payload, 8, 2);
        put_be_u32(&mut payload, 16 + 4, 0);
        put_be_u32(&mut payload, 16 + 8, 1);
        put_be_u32(&mut payload, 16 + 12, 0x00ff0000);
        for (index, value) in [0u16, 1, 2].into_iter().enumerate() {
            payload[32 + index * 2..34 + index * 2].copy_from_slice(&value.to_be_bytes());
        }
        chunk(b"STMS", &payload)
    }

    fn model_bytes_with_streams(position: Vec<u8>, indices: Vec<u8>) -> Vec<u8> {
        let mut comp = vec![0u8; 24];
        for (offset, value) in [
            (0, -1.0),
            (4, -1.0),
            (8, -1.0),
            (12, 1.0),
            (16, 1.0),
            (20, 1.0),
        ] {
            put_be_f32(&mut comp, offset, value);
        }
        let mesh = container_chunk(b"MESH", &[position, indices]);
        let mdl = container_chunk(b"MDL\0", &[chunk(b"COMP", &comp), mesh]);
        let mdlc = container_chunk(b"MDLC", &[mdl]);
        let wrb = container_chunk(b"WRB\0", &[mdlc]);
        let mut wrb_sedb = vec![0u8; 0x30];
        wrb_sedb[..4].copy_from_slice(b"SEDB");
        wrb_sedb[4..8].copy_from_slice(b"wrb\0");
        wrb_sedb[0x0e..0x10].copy_from_slice(&0x30u16.to_le_bytes());
        wrb_sedb[0x10..0x14].copy_from_slice(&(0x30u32 + wrb.len() as u32).to_le_bytes());
        wrb_sedb.extend_from_slice(&wrb);

        let total = 0x50 + wrb_sedb.len();
        let mut root = vec![0u8; 0x50];
        root[..4].copy_from_slice(b"SEDB");
        root[4..8].copy_from_slice(b"RES ");
        root[0x0e..0x10].copy_from_slice(&0x40u16.to_le_bytes());
        root[0x10..0x14].copy_from_slice(&(total as u32).to_le_bytes());
        put_le_u32(&mut root, 0x30, 1);
        put_le_u32(&mut root, 0x38, 1);
        root[0x3c..0x40].copy_from_slice(b"brt\0");
        put_le_u32(&mut root, 0x48, wrb_sedb.len() as u32);
        put_le_u32(&mut root, 0x4c, 2);
        root.extend_from_slice(&wrb_sedb);
        root
    }

    fn model_bytes_with_position(position: Vec<u8>) -> Vec<u8> {
        model_bytes_with_streams(position, stms_indices())
    }

    fn model_bytes() -> Vec<u8> {
        model_bytes_with_position(stms_position())
    }

    fn phb_bytes() -> Vec<u8> {
        let chunk_start = 0xc0;
        let mut bytes = vec![0u8; chunk_start + 0xc0 + 0x10];
        bytes[..4].copy_from_slice(b"SEDB");
        bytes[4..8].copy_from_slice(b"PHB\0");
        bytes[0x0e..0x10].copy_from_slice(&0x48u16.to_le_bytes());
        bytes[0x10..0x14].copy_from_slice(&0x48u32.to_le_bytes());
        bytes[chunk_start..chunk_start + 8].copy_from_slice(b"PHB.GBD\0");
        put_le_u32(&mut bytes, chunk_start + 8, 0xc0);
        for (offset, value) in [
            (0x40, 0.0f32),
            (0x44, 0.0),
            (0x48, 0.0),
            (0x50, 1.0),
            (0x54, 1.0),
            (0x58, 0.0),
        ] {
            bytes[chunk_start + offset..chunk_start + offset + 4]
                .copy_from_slice(&value.to_le_bytes());
        }
        put_le_u32(&mut bytes, chunk_start + 0x38, 0x80);
        put_le_u32(&mut bytes, chunk_start + 0x3c, 3);
        put_le_u32(&mut bytes, chunk_start + 0x60, 0xb0);
        put_le_u32(&mut bytes, chunk_start + 0x64, 1);
        for (index, value) in [0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0]
            .into_iter()
            .enumerate()
        {
            bytes[chunk_start + 0x80 + index * 4..chunk_start + 0x84 + index * 4]
                .copy_from_slice(&value.to_le_bytes());
        }
        bytes[chunk_start + 0xb0..chunk_start + 0xb8]
            .copy_from_slice(&[0, 0, 1, 0, 2, 0, 0xfe, 0xca]);
        bytes
    }

    #[test]
    fn parse_retail_phb_offset_and_surface() {
        let phb = parse_phb(&phb_bytes()).unwrap();
        assert_eq!(phb.hulls[0].vertices[1].x, 1.0);
        assert_eq!(phb.hulls[0].indices, vec![0, 1, 2]);
        assert_eq!(phb.hulls[0].surfaces, vec![0xCAFE]);
        assert_eq!(phb.hulls[0].declared_min, Vec3::ZERO);
        assert_eq!(
            phb.hulls[0].decoded_max,
            Vec3 {
                x: 1.0,
                y: 1.0,
                z: 0.0
            }
        );
    }

    #[test]
    fn phb_requires_header_offsets_and_trailer() {
        let mut short = phb_bytes();
        short.truncate(short.len() - 0x10);
        assert!(parse_phb(&short).is_err());

        let mut early_vertex = phb_bytes();
        put_le_u32(&mut early_vertex, 0xc0 + 0x38, 0x70);
        assert!(parse_phb(&early_vertex).is_err());

        let mut early_triangle = phb_bytes();
        put_le_u32(&mut early_triangle, 0xc0 + 0x60, 0x70);
        assert!(parse_phb(&early_triangle).is_err());
    }

    #[test]
    fn parse_retail_res_wrb_mesh_streams() {
        let model = parse_model(&model_bytes()).unwrap();
        assert_eq!(model.parts.len(), 1);
        assert_eq!(model.parts[0].indices, vec![0, 1, 2]);
        assert_eq!(model.parts[0].vertices[1].x, 1.0);
        assert_eq!(model.parts[0].classification, MeshClassification::Render);
    }

    #[test]
    fn model_wrb_without_mesh_is_valid() {
        let mut bytes = model_bytes();
        let mesh = bytes
            .windows(4)
            .position(|window| window == b"MESH")
            .unwrap();
        bytes[mesh..mesh + 4].copy_from_slice(b"HEAD");
        assert!(parse_model(&bytes).unwrap().parts.is_empty());
    }

    #[test]
    fn model_position_descriptor_must_fit_stride() {
        assert!(
            parse_model(&model_bytes_with_position(stms_position_with_field_offset(
                4
            )))
            .is_ok()
        );
        assert!(
            parse_model(&model_bytes_with_position(stms_position_with_field_offset(
                8
            )))
            .is_err()
        );
    }

    #[test]
    fn model_index_stream_requires_one_exact_descriptor() {
        let mut payload = vec![0u8; 16 + 32 + 6];
        put_be_u32(&mut payload, 0, 2);
        put_be_u32(&mut payload, 4, 3);
        put_be_u32(&mut payload, 8, 2);
        put_be_u32(&mut payload, 16 + 4, 0);
        put_be_u32(&mut payload, 16 + 8, 1);
        put_be_u32(&mut payload, 16 + 12, 0x00ff0000);
        put_be_u32(&mut payload, 32, 0);
        put_be_u32(&mut payload, 32 + 4, 0);
        put_be_u32(&mut payload, 32 + 8, 0);
        put_be_u32(&mut payload, 32 + 12, 0);
        for (index, value) in [0u16, 1, 2].into_iter().enumerate() {
            payload[48 + index * 2..50 + index * 2].copy_from_slice(&value.to_be_bytes());
        }
        let indices = chunk(b"STMS", &payload);
        assert!(parse_model(&model_bytes_with_streams(stms_position(), indices)).is_err());
    }

    #[test]
    fn model_and_phb_tags_must_be_exact() {
        let mut wrb_subtype = model_bytes();
        wrb_subtype[0x54..0x58].copy_from_slice(b"wrbX");
        assert!(parse_model(&wrb_subtype).is_err());

        let mut wrb_chunk = model_bytes();
        wrb_chunk[0x80..0x84].copy_from_slice(b"WRBX");
        assert!(parse_model(&wrb_chunk).is_err());

        let mut phb = phb_bytes();
        phb[4..8].copy_from_slice(b"PHBX");
        assert!(parse_phb(&phb).is_err());
    }

    #[test]
    fn model_chunk_depth_is_bounded() {
        let mut nested = chunk(b"STMS", &[]);
        for _ in 0..(MAX_MODEL_CHUNK_DEPTH + 2) {
            nested = container_chunk(b"AABB", &[nested]);
        }
        assert!(parse_chunks(&nested).is_err());
    }
}
