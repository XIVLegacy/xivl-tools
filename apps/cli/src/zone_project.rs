//! Identity-pinned inputs and in-memory preparation for an offline floor project.

use std::collections::BTreeSet;
use std::fmt;
use std::fs::{self, File, Metadata, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use serde::de::{value::MapAccessDeserializer, MapAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::{json, Value};
use xivl_formats::collision_json::{parse_geometry_json, MAX_GEOMETRY_JSON_BYTES};
use xivl_formats::digest::sha256_hex;
use xivl_formats::layout_edit::{
    edit_layout_binding, LayoutBindingPlacement, LayoutBindingRequest, LayoutBindingRow,
};
use xivl_formats::model_import::import_obj_to_model;
use xivl_formats::phb_author::{author_phb, CollisionGeometry};
use xivl_formats::sedb::{parse_container, Container, EntryBody, RES_DIRECTORY_ENTRY_SIZE};
use xivl_formats::zone::{
    flatten_zone, parse_layout_with_structure, parse_model, parse_phb, FlattenedZone, Layout,
    LayoutInspection, Matrix4, Model, Vec3,
};
use xivl_formats::{to_canonical_json, ErrorKind, FormatError, ResourceId};

/// Limits apply before parsing or allocating the declared input size.
pub const MAX_PROJECT_BYTES: u64 = 1024 * 1024;
pub const MAX_LAYOUT_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_RESOURCE_BYTES: u64 = 256 * 1024 * 1024;
pub const MAX_PHB_TEMPLATE_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_OBJ_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_TOTAL_INPUT_BYTES: u64 = 512 * 1024 * 1024;

// Serde's derived struct visitor accepts sequences as well as maps.
pub struct ObjectOnly<T>(pub T);

impl<'de, T: Deserialize<'de>> Deserialize<'de> for ObjectOnly<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct ObjectVisitor<T>(std::marker::PhantomData<T>);
        impl<'de, T: Deserialize<'de>> Visitor<'de> for ObjectVisitor<T> {
            type Value = ObjectOnly<T>;
            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("an object")
            }
            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Self::Value, A::Error> {
                T::deserialize(MapAccessDeserializer::new(map)).map(ObjectOnly)
            }
        }
        deserializer.deserialize_map(ObjectVisitor(std::marker::PhantomData))
    }
}

macro_rules! project_object {
    ($name:ident { $($field:ident: $kind:ty),* $(,)? }) => {
        #[derive(Debug, Clone)]
        pub struct $name { $(pub $field: $kind),* }
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields, rename_all = "camelCase")]
                struct Fields { $($field: $kind),* }
                let fields = ObjectOnly::<Fields>::deserialize(deserializer)?.0;
                Ok(Self { $($field: fields.$field),* })
            }
        }
    };
}

project_object!(ZoneProject {
    schema_version: u32,
    profile: String,
    layout: FileReference,
    resources: Vec<ProjectResource>,
    collision_template: FileReference,
    geometry: ProjectGeometry,
    binding: ProjectBinding,
    expected_placements: [ExpectedPlacement; 4],
});
project_object!(FileReference {
    path: String,
    size: u64,
    sha256: String
});
project_object!(ProjectResource {
    row: u32,
    id: ProjectId,
    tag: String,
    file: FileReference
});
project_object!(ProjectGeometry {
    render_obj: FileReference,
    collision_json: FileReference
});
project_object!(ProjectBinding {
    resource_key: String,
    model_row: u32,
    collision_row: u32,
    output_model_id: ProjectId,
    output_collision_id: ProjectId,
});
project_object!(ExpectedPlacement {
    source_node: u32,
    source_block: u32,
    world_matrix: [[f32; 4]; 4],
});

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProjectId(pub ResourceId);

impl<'de> Deserialize<'de> for ProjectId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        if text.len() != 10
            || !text.starts_with("0x")
            || !text.as_bytes()[2..]
                .iter()
                .all(|byte| byte.is_ascii_digit() || (b'A'..=b'F').contains(byte))
        {
            return Err(serde::de::Error::custom(
                "expected 0x and eight uppercase hexadecimal digits",
            ));
        }
        let id = xivl_formats::parse_resource_id(&text, 0).map_err(serde::de::Error::custom)?;
        if id.value() == 0 {
            return Err(serde::de::Error::custom("resource ID must be nonzero"));
        }
        Ok(Self(id))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectFailureKind {
    Usage,
    Io,
    InvalidProject,
    IdentityMismatch,
    InvalidPath,
    ResourceLimit,
    InventoryMismatch,
    Format(ErrorKind),
}

#[derive(Debug)]
pub struct ProjectFailure {
    pub kind: ProjectFailureKind,
    pub role: String,
    pub offset: Option<u64>,
    pub detail: String,
}

impl ProjectFailure {
    pub fn new(kind: ProjectFailureKind, role: &str, detail: impl Into<String>) -> Self {
        Self {
            kind,
            role: role.into(),
            offset: None,
            detail: detail.into(),
        }
    }
    pub fn format(role: &str, error: FormatError) -> Self {
        Self {
            kind: ProjectFailureKind::Format(error.kind()),
            role: role.into(),
            offset: Some(error.offset()),
            detail: error.detail().into(),
        }
    }
    pub fn code(&self) -> u8 {
        if matches!(
            self.kind,
            ProjectFailureKind::Usage | ProjectFailureKind::Io
        ) {
            1
        } else {
            2
        }
    }
}

impl fmt::Display for ProjectFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {:?}", self.role, self.kind)?;
        if let Some(offset) = self.offset {
            write!(formatter, " at offset {offset}")?;
        }
        write!(formatter, ": {}", self.detail)
    }
}
impl std::error::Error for ProjectFailure {}

#[derive(Debug)]
pub struct LoadedInput {
    pub reference: FileReference,
    pub bytes: Vec<u8>,
}

#[derive(Debug)]
pub struct LoadedResource {
    pub declaration: ProjectResource,
    pub bytes: Vec<u8>,
}

#[derive(Debug)]
pub struct LoadedZoneProject {
    pub project: ZoneProject,
    pub layout: LoadedInput,
    pub parsed_layout: Layout,
    pub layout_inspection: LayoutInspection,
    /// All nonzero direct rows, ordered by row. Opaque dependencies are not decoded.
    pub resources: Vec<LoadedResource>,
    pub collision_template: LoadedInput,
    pub render_obj: LoadedInput,
    pub collision_json: LoadedInput,
    pub collision_geometry: CollisionGeometry,
}

#[derive(Debug)]
pub struct PreparedZoneProject {
    pub layout: Vec<u8>,
    pub model: Vec<u8>,
    pub collision: Vec<u8>,
    pub report: Value,
}

/// Author and verify all three resources in memory without publishing output files.
pub fn prepare_project(
    project_path: &Path,
    client_root: &Path,
) -> Result<PreparedZoneProject, ProjectFailure> {
    prepare_loaded(load_project(project_path, client_root)?)
}

/// Parse the project commands and return the canonical offline report.
pub fn run(arguments: &[String], build: bool) -> Result<String, ProjectFailure> {
    let usage = || {
        ProjectFailure::new(
            ProjectFailureKind::Usage,
            "zone project command",
            if build {
                "usage: xivl build-zone-project <project.json> --client-root <root> --output <new-directory>"
            } else {
                "usage: xivl validate-zone-project <project.json> --client-root <root>"
            },
        )
    };
    let project = arguments
        .first()
        .filter(|arg| !arg.starts_with("--"))
        .ok_or_else(usage)?;
    let mut client_root = None;
    let mut output = None;
    let (options, remainder) = arguments[1..].as_chunks::<2>();
    for pair in options {
        if pair[1].is_empty() || pair[1].starts_with("--") {
            return Err(usage());
        }
        match pair[0].as_str() {
            "--client-root" if client_root.is_none() => client_root = Some(Path::new(&pair[1])),
            "--output" if build && output.is_none() => output = Some(Path::new(&pair[1])),
            _ => return Err(usage()),
        }
    }
    if !remainder.is_empty() || (build && output.is_none()) {
        return Err(usage());
    }
    let client_root = client_root.ok_or_else(usage)?;
    let prepared = prepare_project(Path::new(project), client_root)?;
    let report = match output {
        Some(output) => write_project(prepared, client_root, output)?,
        None => prepared.report,
    };
    Ok(to_canonical_json(&report))
}

/// Publish prepared resources in a new directory and publish their report last.
pub fn write_project(
    prepared: PreparedZoneProject,
    client_root: &Path,
    output: &Path,
) -> Result<Value, ProjectFailure> {
    write_project_with_writer(prepared, client_root, output, write_new)
}

fn write_project_with_writer(
    mut prepared: PreparedZoneProject,
    client_root: &Path,
    output: &Path,
    mut write: impl FnMut(&Path, &[u8]) -> Result<(), ProjectFailure>,
) -> Result<Value, ProjectFailure> {
    let output = new_output_directory(client_root, output)?;
    prepared.report["status"] = json!("offline-built");
    for (name, bytes) in [
        ("layout.dat", &prepared.layout),
        ("model.dat", &prepared.model),
        ("collision.dat", &prepared.collision),
    ] {
        write(&output.join(name), bytes)?;
    }
    let pending = output.join("build-report.json.pending");
    write(&pending, to_canonical_json(&prepared.report).as_bytes())?;
    let report = output.join("build-report.json");
    match fs::symlink_metadata(&report) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(ProjectFailure::new(
                ProjectFailureKind::Io,
                "output",
                error.to_string(),
            ))
        }
        Ok(_) => {
            return Err(ProjectFailure::new(
                ProjectFailureKind::Usage,
                "output",
                "build-report.json already exists",
            ))
        }
    }
    fs::rename(pending, report).map_err(|error| {
        ProjectFailure::new(ProjectFailureKind::Io, "output", error.to_string())
    })?;
    Ok(prepared.report)
}

/// Create an absent destination beneath an existing parent outside the client tree.
pub fn new_output_directory(client_root: &Path, output: &Path) -> Result<PathBuf, ProjectFailure> {
    let io = |error: std::io::Error| {
        ProjectFailure::new(ProjectFailureKind::Io, "output", error.to_string())
    };
    let refuse = |detail| ProjectFailure::new(ProjectFailureKind::Usage, "output", detail);
    let name = output
        .components()
        .next_back()
        .filter(|component| matches!(component, Component::Normal(_)))
        .ok_or_else(|| refuse("expected a new directory name"))?;
    let parent = output
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let parent = fs::canonicalize(parent).map_err(io)?;
    let output = parent.join(name);
    let client_root = checked_root(client_root, "client root")?;
    if output.starts_with(&client_root) {
        return Err(refuse("output must be outside the client input tree"));
    }
    // create_dir refuses all existing entries, including empty directories and source aliases.
    fs::create_dir(&output).map_err(io)?;
    Ok(output)
}

pub fn write_new(path: &Path, bytes: &[u8]) -> Result<(), ProjectFailure> {
    let io = |error: std::io::Error| {
        ProjectFailure::new(
            ProjectFailureKind::Io,
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("output"),
            error.to_string(),
        )
    };
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(io)?;
    file.write_all(bytes).map_err(io)?;
    file.sync_all().map_err(io)
}

pub fn prepare_loaded(loaded: LoadedZoneProject) -> Result<PreparedZoneProject, ProjectFailure> {
    let project = &loaded.project;
    let binding = &project.binding;
    let selected = |row| {
        loaded
            .resources
            .iter()
            .find(|resource| resource.declaration.row == row)
            .ok_or_else(|| invalid("selected resource is missing from the direct inventory"))
    };
    let model_source = selected(binding.model_row)?;
    let collision_source = selected(binding.collision_row)?;
    let obj = std::str::from_utf8(&loaded.render_obj.bytes).map_err(|error| {
        ProjectFailure::format(
            "renderObj",
            FormatError::new(
                ErrorKind::InvalidAttributeValue,
                error.valid_up_to() as u64,
                "OBJ must be UTF-8",
            ),
        )
    })?;
    let model = import_obj_to_model(&model_source.bytes, obj)
        .map_err(|error| ProjectFailure::format("model/renderObj", error))?;
    let collision = author_phb(&loaded.collision_template.bytes, &loaded.collision_geometry)
        .map_err(|error| ProjectFailure::format("collisionTemplate/collisionJson", error))?;
    let request = LayoutBindingRequest {
        resource_key: binding.resource_key.clone(),
        model: LayoutBindingRow {
            row: binding.model_row,
            expected_id: model_source.declaration.id.0,
            output_id: binding.output_model_id.0,
        },
        collision: LayoutBindingRow {
            row: binding.collision_row,
            expected_id: collision_source.declaration.id.0,
            output_id: binding.output_collision_id.0,
        },
        expected_placements: project.expected_placements.clone().map(|placement| {
            LayoutBindingPlacement {
                source_node: placement.source_node,
                source_block: placement.source_block,
                world_matrix: Matrix4 {
                    values: placement.world_matrix,
                },
            }
        }),
    };
    let layout = edit_layout_binding(&loaded.layout.bytes, &request)
        .map_err(|error| ProjectFailure::format("layout binding", error))?;
    let (output_layout, _) = parse_layout_with_structure(&layout)
        .map_err(|error| ProjectFailure::format("prepared layout", error))?;
    let output_model =
        parse_model(&model).map_err(|error| ProjectFailure::format("prepared model", error))?;
    let output_phb = parse_phb(&collision)
        .map_err(|error| ProjectFailure::format("prepared collision", error))?;
    let (model_allocation, model_offset) =
        verify_allocation(&model_source.bytes, &model, "model", true)?;
    let (collision_allocation, collision_offset) = verify_allocation(
        &loaded.collision_template.bytes,
        &collision,
        "collision",
        false,
    )?;
    let flattened = flatten_zone(
        &output_layout,
        &[(binding.output_collision_id.0, output_phb)],
    )
    .map_err(|error| ProjectFailure::format("prepared collision placements", error))?;
    let bounds = compare_triangles(
        &output_layout,
        &output_model,
        &flattened,
        model_offset,
        collision_offset,
    )?;
    let resources = loaded
        .resources
        .iter()
        .map(|resource| {
            let row = resource.declaration.row;
            json!({
                "row": row,
                "id": resource.declaration.id.0.to_hex(),
                "tag": resource.declaration.tag,
                "input": input_report(&resource.declaration.file),
                "retained": row != binding.model_row && row != binding.collision_row,
            })
        })
        .collect::<Vec<_>>();
    let mut selected_rows = [binding.model_row, binding.collision_row];
    selected_rows.sort();
    let selected_rows = selected_rows.map(|row| {
        let source = &loaded.layout_inspection.rows[row as usize];
        let output_id = if row == binding.model_row {
            binding.output_model_id.0
        } else {
            binding.output_collision_id.0
        };
        json!({
            "row": row, "span": source.span.to_json(),
            "changedSpan": {"offset": source.span.offset + 0x14, "length": 4},
            "sourceId": source.resource_id.to_hex(), "outputId": output_id.to_hex(),
        })
    });
    let mut placements = flattened
        .placements
        .iter()
        .map(|placement| {
            json!({
                "sourceNode": placement.source_node, "sourceBlock": placement.source_block,
                "worldMatrix": placement.world_matrix.values,
            })
        })
        .collect::<Vec<_>>();
    // Sort whole identities, including matrices: repeated source nodes are valid.
    placements.sort_by_cached_key(Value::to_string);
    let report = json!({
        "schemaVersion": 1, "profile": project.profile,
        "toolVersion": env!("CARGO_PKG_VERSION"),
        "status": "offline-validated", "clientVerification": "unverified",
        "inputs": {
            "layout": input_report(&loaded.layout.reference),
            "collisionTemplate": input_report(&loaded.collision_template.reference),
            "renderObj": input_report(&loaded.render_obj.reference),
            "collisionJson": input_report(&loaded.collision_json.reference),
        },
        "resources": resources,
        "outputs": {
            "layout": output_report("layout.dat", &layout, None),
            "model": output_report("model.dat", &model, Some(binding.output_model_id.0)),
            "collision": output_report("collision.dat", &collision, Some(binding.output_collision_id.0)),
        },
        "binding": {"resourceKey": binding.resource_key, "selectedRows": selected_rows},
        "coverage": {
            "directTable": {"status": "complete", "nonzeroRows": loaded.resources.len()},
            "sceneDependencies": "unresolved", "transitiveDependencies": "unresolved",
        },
        "checks": {
            "placementCount": flattened.placements.len(),
            "flattenedVertexCount": flattened.vertices.len(), "triangleCount": flattened.faces.len(),
            "worldBounds": bounds, "orderedTriangles": "exact-finite-equality",
            "placements": placements,
            "modelAllocation": model_allocation, "collisionAllocation": collision_allocation,
            "layoutOutsideBindingFields": "byte-identical",
        },
    });
    Ok(PreparedZoneProject {
        layout,
        model,
        collision,
        report,
    })
}

fn input_report(reference: &FileReference) -> Value {
    json!({"path": reference.path, "size": reference.size, "sha256": reference.sha256})
}

fn output_report(path: &str, bytes: &[u8], id: Option<ResourceId>) -> Value {
    let mut report = json!({"path": path, "size": bytes.len(), "sha256": sha256_hex(bytes)});
    if let Some(id) = id {
        report["id"] = json!(id.to_hex());
    }
    report
}

fn verify_allocation(
    before: &[u8],
    after: &[u8],
    role: &str,
    require_res: bool,
) -> Result<(Value, u64), ProjectFailure> {
    let source = parse_container(before, 0).map_err(|error| ProjectFailure::format(role, error))?;
    let output = parse_container(after, 0).map_err(|error| ProjectFailure::format(role, error))?;
    // Without a RES directory, PHB retention rests on structure plus file length.
    let directory_retained = source.res.as_ref().is_none_or(|res| {
        let start = usize::from(source.directory_start());
        let end = start + res.subresource_count as usize * RES_DIRECTORY_ENTRY_SIZE as usize;
        matches!((before.get(start..end), after.get(start..end)), (Some(a), Some(b)) if a == b)
    });
    if before.len() != after.len()
        || (require_res && source.res.is_none())
        || !directory_retained
        || !same_allocation(&source, &output)
    {
        return Err(invalid_resource(
            role,
            u64::from(source.directory_start()),
            "prepared resource changed its container allocation or RES directory",
        ));
    }
    Ok((
        json!({
            "status": "retained", "fileSize": after.len(),
            "declaredSize": output.declared_size, "resolvedSpan": output.span.to_json(),
            "directoryStart": output.directory_start(),
            "entries": output.entries.iter().map(|entry| json!({
                "span": entry.span.to_json(), "kind": entry.body.kind_name(),
            })).collect::<Vec<_>>(),
        }),
        u64::from(source.directory_start()),
    ))
}

fn same_allocation(before: &Container, after: &Container) -> bool {
    before.span == after.span
        && before.subtype == after.subtype
        && before.unknown_a == after.unknown_a
        && before.flags == after.flags
        && before.header_size == after.header_size
        && before.declared_size == after.declared_size
        && before.total_size == after.total_size
        && before.header_unknown == after.header_unknown
        && before.res == after.res
        && before.anomalies == after.anomalies
        && before.directory_start() == after.directory_start()
        && before.entries.len() == after.entries.len()
        && before.entries.iter().zip(&after.entries).all(|(a, b)| {
            a.span == b.span
                && match (&a.body, &b.body) {
                    (
                        EntryBody::Directory { entry_count: x },
                        EntryBody::Directory { entry_count: y },
                    ) => {
                        x == y
                            && a.sha256 == b.sha256
                            && a.span.offset
                                == before.span.offset + u64::from(before.directory_start())
                    }
                    (
                        EntryBody::Subresource {
                            index: ai,
                            declared_offset: ao,
                            declared_size: az,
                            kind: ak,
                            child: ac,
                        },
                        EntryBody::Subresource {
                            index: bi,
                            declared_offset: bo,
                            declared_size: bz,
                            kind: bk,
                            child: bc,
                        },
                    ) => {
                        ai == bi
                            && ao == bo
                            && az == bz
                            && ak == bk
                            && match (ac, bc) {
                                (Some(a), Some(b)) => same_allocation(a, b),
                                (None, None) => true,
                                _ => false,
                            }
                    }
                    (EntryBody::Payload, EntryBody::Payload) | (EntryBody::Gap, EntryBody::Gap) => {
                        true
                    }
                    _ => false,
                }
        })
}

fn finite(vertex: Vec3) -> bool {
    [vertex.x, vertex.y, vertex.z]
        .iter()
        .all(|value| value.is_finite())
}

fn finite_bounds(
    vertices: impl IntoIterator<Item = Vec3>,
    role: &str,
    offset: u64,
) -> Result<[[f32; 3]; 2], ProjectFailure> {
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    let mut empty = true;
    for vertex in vertices {
        empty = false;
        if !finite(vertex) {
            return Err(invalid_resource(
                role,
                offset,
                "prepared world coordinates must be finite",
            ));
        }
        for (axis, value) in [vertex.x, vertex.y, vertex.z].into_iter().enumerate() {
            min[axis] = min[axis].min(value);
            max[axis] = max[axis].max(value);
        }
    }
    if empty {
        return Err(invalid_resource(role, offset, "prepared geometry is empty"));
    }
    Ok([min, max])
}

fn compare_triangles(
    layout: &Layout,
    model: &Model,
    collision: &FlattenedZone,
    model_offset: u64,
    collision_offset: u64,
) -> Result<[[f32; 3]; 2], ProjectFailure> {
    let vertices_per_placement = model
        .parts
        .iter()
        .map(|part| part.vertices.len())
        .sum::<usize>();
    let triangles_per_placement = model
        .parts
        .iter()
        .map(|part| part.indices.len() / 3)
        .sum::<usize>();
    if triangles_per_placement == 0
        || collision.placements.len() != layout.objects.len()
        || collision.faces.len() != triangles_per_placement * layout.objects.len()
    {
        return Err(invalid_resource(
            "collision",
            collision_offset,
            "render and collision triangle or placement counts differ",
        ));
    }
    if collision.vertices.len() != vertices_per_placement * layout.objects.len()
        || layout.objects.iter().any(|object| {
            collision
                .collision_resources
                .iter()
                .find(|resource| Some(resource.resource_id) == object.phb)
                .is_none_or(|resource| {
                    resource
                        .hulls
                        .iter()
                        .map(|hull| hull.vertex_count)
                        .sum::<usize>()
                        != vertices_per_placement
                })
        })
    {
        return Err(invalid_resource(
            "collision",
            collision_offset,
            "render and collision vertex counts differ per placement",
        ));
    }
    let mut faces = collision.faces.iter();
    for (object_index, object) in layout.objects.iter().enumerate() {
        let placement = &collision.placements[object_index];
        if placement.source_node != object.source_node
            || placement.source_block != object.source_block
            || placement.world_matrix != object.world_matrix
            || placement.resource_key != object.resource_key
            || placement.model_resource_id != object.model
            || placement.phb_resource_id != object.phb
        {
            return Err(invalid_resource(
                "collision",
                collision_offset,
                "render and collision placement identities differ",
            ));
        }
        for (part_index, part) in model.parts.iter().enumerate() {
            let vertices = part
                .vertices
                .iter()
                .map(|vertex| object.world_matrix.transform_point(*vertex))
                .collect::<Vec<_>>();
            finite_bounds(vertices.iter().copied(), "model", model_offset)?;
            for (triangle_index, triangle) in part.indices.as_chunks::<3>().0.iter().enumerate() {
                let face = faces.next().ok_or_else(|| {
                    invalid_resource(
                        "collision",
                        collision_offset,
                        "collision triangle is missing",
                    )
                })?;
                if face.source.object_index != object_index
                    || face.source.placement_index != object_index
                    || face.source.resource_id != object.phb
                    || face.source.model_resource_id != object.model
                    || triangle
                        .iter()
                        .zip(face.indices)
                        .any(|(render, collision_index)| {
                            let render = vertices.get(*render as usize);
                            let collision = collision.vertices.get(collision_index as usize);
                            match (render, collision) {
                                (Some(a), Some(b)) => {
                                    !finite(*a)
                                        || !finite(*b)
                                        || [a.x, a.y, a.z].map(f32::to_bits)
                                            != [b.x, b.y, b.z].map(f32::to_bits)
                                }
                                _ => true,
                            }
                        })
                {
                    return Err(invalid_resource("collision", collision_offset, format!("ordered render/collision triangle mismatch at placement {object_index}, part {part_index}, triangle {triangle_index}")));
                }
            }
        }
    }
    let render_bounds = finite_bounds(
        layout.objects.iter().flat_map(|object| {
            model.parts.iter().flat_map(move |part| {
                part.vertices
                    .iter()
                    .map(move |vertex| object.world_matrix.transform_point(*vertex))
            })
        }),
        "model",
        model_offset,
    )?;
    let collision_bounds = finite_bounds(
        collision.vertices.iter().copied(),
        "collision",
        collision_offset,
    )?;
    if render_bounds != collision_bounds {
        return Err(invalid_resource(
            "collision",
            collision_offset,
            "render and collision world bounds differ",
        ));
    }
    Ok(collision_bounds)
}

/// Parse at most 1 MiB. File paths and identities are validated independently of the schema.
pub fn parse_project(bytes: &[u8]) -> Result<ZoneProject, ProjectFailure> {
    if bytes.len() as u64 > MAX_PROJECT_BYTES {
        return Err(ProjectFailure::new(
            ProjectFailureKind::ResourceLimit,
            "project",
            "project exceeds the 1 MiB limit",
        ));
    }
    let project: ZoneProject = serde_json::from_slice(bytes).map_err(|error| {
        let start = bytes
            .split_inclusive(|byte| *byte == b'\n')
            .take(error.line().saturating_sub(1))
            .map(<[u8]>::len)
            .sum::<usize>();
        let mut failure = ProjectFailure::new(
            ProjectFailureKind::InvalidProject,
            "project",
            error.to_string(),
        );
        failure.offset = Some((start + error.column().saturating_sub(1)).min(bytes.len()) as u64);
        failure
    })?;
    validate_project(&project).map_err(|mut error| {
        error.offset = Some(0);
        error
    })?;
    Ok(project)
}

fn invalid(detail: impl Into<String>) -> ProjectFailure {
    ProjectFailure::new(ProjectFailureKind::InvalidProject, "project", detail)
}

fn invalid_resource(role: &str, offset: u64, detail: impl Into<String>) -> ProjectFailure {
    ProjectFailure {
        kind: ProjectFailureKind::InvalidProject,
        role: role.into(),
        offset: Some(offset),
        detail: detail.into(),
    }
}

fn validate_project(project: &ZoneProject) -> Result<(), ProjectFailure> {
    if project.schema_version != 1 || project.profile != "floor-blockout-v1" {
        return Err(invalid(
            "expected schemaVersion 1 and profile floor-blockout-v1",
        ));
    }
    validate_reference(&project.layout, "layout", MAX_LAYOUT_BYTES)?;
    validate_reference(
        &project.collision_template,
        "collisionTemplate",
        MAX_PHB_TEMPLATE_BYTES,
    )?;
    validate_reference(&project.geometry.render_obj, "renderObj", MAX_OBJ_BYTES)?;
    validate_reference(
        &project.geometry.collision_json,
        "collisionJson",
        MAX_GEOMETRY_JSON_BYTES as u64,
    )?;
    let mut rows = BTreeSet::new();
    for resource in &project.resources {
        if !rows.insert(resource.row) {
            return Err(invalid("duplicate resource row"));
        }
        if resource.tag.len() > 4
            || !resource
                .tag
                .bytes()
                .all(|byte| (0x20..=0x7e).contains(&byte))
        {
            return Err(invalid(
                "resource tag must contain at most four printable ASCII bytes",
            ));
        }
        validate_reference(
            &resource.file,
            &format!("resources[{}]", resource.row),
            MAX_RESOURCE_BYTES,
        )?;
    }
    let binding = &project.binding;
    if binding.resource_key.is_empty()
        || binding.resource_key.len() > 16
        || !binding
            .resource_key
            .bytes()
            .all(|byte| (0x20..=0x7e).contains(&byte))
    {
        return Err(invalid(
            "resourceKey must contain 1 to 16 printable ASCII bytes",
        ));
    }
    if binding.model_row == binding.collision_row
        || binding.output_model_id == binding.output_collision_id
    {
        return Err(invalid("binding rows and output IDs must be distinct"));
    }
    for placement in &project.expected_placements {
        if !placement
            .world_matrix
            .iter()
            .flatten()
            .all(|value| value.is_finite())
        {
            return Err(invalid("placement matrices must be finite f32 values"));
        }
    }
    Ok(())
}

pub fn validate_reference(
    reference: &FileReference,
    role: &str,
    limit: u64,
) -> Result<(), ProjectFailure> {
    validate_relative_path(&reference.path)
        .map_err(|detail| ProjectFailure::new(ProjectFailureKind::InvalidPath, role, detail))?;
    if reference.size == 0 || reference.size > limit {
        return Err(ProjectFailure::new(
            ProjectFailureKind::ResourceLimit,
            role,
            format!("size must be between 1 and {limit} bytes"),
        ));
    }
    if reference.sha256.len() != 64
        || !reference
            .sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(invalid(format!(
            "{role}: sha256 must contain 64 lowercase hexadecimal digits"
        )));
    }
    Ok(())
}

fn validate_relative_path(path: &str) -> Result<(), &'static str> {
    if path.is_empty() || path.len() > 512 || !path.is_ascii() {
        return Err("path must contain 1 to 512 ASCII bytes");
    }
    for part in path.split('/') {
        if part.is_empty()
            || part == "."
            || part == ".."
            || part.ends_with(['.', ' '])
            || part
                .bytes()
                .any(|byte| !(0x20..=0x7e).contains(&byte) || b"\\:<>\"|?*".contains(&byte))
        {
            return Err("path must be relative with normal slash-separated components");
        }
        let stem = part
            .split('.')
            .next()
            .unwrap_or_default()
            .to_ascii_uppercase();
        if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || (stem.len() == 4
                && (stem.starts_with("COM") || stem.starts_with("LPT"))
                && matches!(stem.as_bytes()[3], b'1'..=b'9'))
        {
            return Err("path contains a reserved device name");
        }
    }
    Ok(())
}

/// Load once from an explicit client root and the project's parent directory.
/// The returned byte vectors are the exact bytes whose sizes and hashes were checked.
pub fn load_project(
    project_path: &Path,
    client_root: &Path,
) -> Result<LoadedZoneProject, ProjectFailure> {
    load_project_with_bytes(project_path, client_root, None)
}

/// Use already identity-checked saved project bytes when supplied by an assembler.
pub fn load_project_with_bytes(
    project_path: &Path,
    client_root: &Path,
    bytes: Option<&[u8]>,
) -> Result<LoadedZoneProject, ProjectFailure> {
    if client_root.as_os_str().is_empty() {
        return Err(ProjectFailure::new(
            ProjectFailureKind::InvalidPath,
            "client root",
            "client root must be explicit",
        ));
    }
    let project_root = project_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let project_root = checked_root(project_root, "project")?;
    let project_name = project_path
        .file_name()
        .ok_or_else(|| invalid("project has no file name"))?;
    let project_path = checked_path(&project_root, Path::new(project_name), "project")?;
    let client_root = checked_root(client_root, "client root")?;
    let project_bytes;
    let bytes = match bytes {
        Some(bytes) => bytes,
        None => {
            project_bytes = read_bounded(&project_path, "project", MAX_PROJECT_BYTES)?;
            &project_bytes
        }
    };
    let project = parse_project(bytes)?;
    let mut total = 0;
    let layout = load_input(
        &client_root,
        &project.layout,
        "layout",
        MAX_LAYOUT_BYTES,
        &mut total,
    )?;
    let (parsed_layout, layout_inspection) = parse_layout_with_structure(&layout.bytes)
        .map_err(|error| ProjectFailure::format("layout", error))?;
    validate_inventory(&project, &parsed_layout, &layout_inspection)?;
    validate_binding(&project, &parsed_layout)?;
    let mut declarations = project.resources.clone();
    declarations.sort_by_key(|resource| resource.row);
    let mut resources = Vec::new();
    for declaration in declarations {
        let input = load_input(
            &client_root,
            &declaration.file,
            &format!("resources[{}]", declaration.row),
            MAX_RESOURCE_BYTES,
            &mut total,
        )?;
        resources.push(LoadedResource {
            declaration,
            bytes: input.bytes,
        });
    }
    let collision_template = load_input(
        &client_root,
        &project.collision_template,
        "collisionTemplate",
        MAX_PHB_TEMPLATE_BYTES,
        &mut total,
    )?;
    let render_obj = load_input(
        &project_root,
        &project.geometry.render_obj,
        "renderObj",
        MAX_OBJ_BYTES,
        &mut total,
    )?;
    let collision_json = load_input(
        &project_root,
        &project.geometry.collision_json,
        "collisionJson",
        MAX_GEOMETRY_JSON_BYTES as u64,
        &mut total,
    )?;
    let collision_geometry = parse_geometry_json(&collision_json.bytes)
        .map_err(|error| ProjectFailure::format("collisionJson", error))?;
    Ok(LoadedZoneProject {
        project,
        layout,
        parsed_layout,
        layout_inspection,
        resources,
        collision_template,
        render_obj,
        collision_json,
        collision_geometry,
    })
}

fn validate_inventory(
    project: &ZoneProject,
    layout: &Layout,
    inspection: &LayoutInspection,
) -> Result<(), ProjectFailure> {
    let failure =
        |detail| ProjectFailure::new(ProjectFailureKind::InventoryMismatch, "resources", detail);
    for (row, source) in layout.resources.iter().enumerate() {
        let declaration = project
            .resources
            .iter()
            .find(|resource| resource.row as usize == row);
        if source.resource_id.value() == 0 {
            if declaration.is_some() {
                return Err(failure(format!(
                    "zero-key row {row} must not declare a file"
                )));
            }
            continue;
        }
        let declaration =
            declaration.ok_or_else(|| failure(format!("missing nonzero direct row {row}")))?;
        if declaration.id.0 != source.resource_id || declaration.tag != source.type_tag {
            let mut error = failure(format!("row {row} ID or tag does not match layout"));
            error.offset = Some(inspection.rows[row].span.offset);
            return Err(error);
        }
    }
    if project
        .resources
        .iter()
        .any(|resource| resource.row as usize >= layout.resources.len())
    {
        return Err(failure(
            "declaration names a row outside the layout".to_string(),
        ));
    }
    Ok(())
}

fn validate_binding(project: &ZoneProject, layout: &Layout) -> Result<(), ProjectFailure> {
    let binding = &project.binding;
    let model = layout
        .resources
        .get(binding.model_row as usize)
        .ok_or_else(|| invalid("modelRow is outside layout"))?;
    let collision = layout
        .resources
        .get(binding.collision_row as usize)
        .ok_or_else(|| invalid("collisionRow is outside layout"))?;
    if model.name != binding.resource_key
        || model.type_tag != "brt"
        || collision.name != binding.resource_key
        || collision.type_tag != "bhp"
        || model.resource_id.value() == 0
        || collision.resource_id.value() == 0
    {
        return Err(invalid(
            "binding key, tags or source IDs do not match the selected rows",
        ));
    }
    for tag in ["brt", "bhp"] {
        if layout
            .resources
            .iter()
            .filter(|resource| resource.name == binding.resource_key && resource.type_tag == tag)
            .count()
            != 1
        {
            return Err(invalid("binding key/tag must select exactly one row"));
        }
    }
    if layout.resources.iter().enumerate().any(|(row, resource)| {
        row != binding.model_row as usize
            && row != binding.collision_row as usize
            && [binding.output_model_id.0, binding.output_collision_id.0]
                .contains(&resource.resource_id)
    }) {
        return Err(invalid("output ID collides with an unmodified source row"));
    }
    if layout.objects.len() != 4 {
        return Err(invalid(
            "floor profile requires exactly four paired placements",
        ));
    }
    let mut unmatched = layout.objects.iter().collect::<Vec<_>>();
    for expected in &project.expected_placements {
        let index = unmatched
            .iter()
            .position(|object| {
                object.source_node == expected.source_node
                    && object.source_block == expected.source_block
                    && object.world_matrix.values == expected.world_matrix
            })
            .ok_or_else(|| {
                invalid("expected placement identity or matrix does not match layout")
            })?;
        let object = unmatched.remove(index);
        if object.model != Some(model.resource_id)
            || object.phb != Some(collision.resource_id)
            || object.resource_key.as_deref() != Some(binding.resource_key.as_str())
            || !object.boxes.is_empty()
        {
            return Err(invalid(
                "floor profile requires paired model/PHB placements without collision boxes",
            ));
        }
    }
    Ok(())
}

pub fn load_input(
    root: &Path,
    reference: &FileReference,
    role: &str,
    limit: u64,
    total: &mut u64,
) -> Result<LoadedInput, ProjectFailure> {
    *total = total
        .checked_add(reference.size)
        .filter(|size| *size <= MAX_TOTAL_INPUT_BYTES)
        .ok_or_else(|| {
            ProjectFailure::new(
                ProjectFailureKind::ResourceLimit,
                role,
                "inputs exceed the 512 MiB total limit",
            )
        })?;
    let path = checked_path(root, Path::new(&reference.path), role)?;
    let metadata = fs::metadata(&path)
        .map_err(|error| ProjectFailure::new(ProjectFailureKind::Io, role, error.to_string()))?;
    if !metadata.is_file() {
        return Err(ProjectFailure::new(
            ProjectFailureKind::InvalidPath,
            role,
            "expected a regular file",
        ));
    }
    if metadata.len() != reference.size {
        return Err(ProjectFailure::new(
            ProjectFailureKind::IdentityMismatch,
            role,
            "declared size does not match input",
        ));
    }
    let bytes = read_bounded(&path, role, limit)?;
    if bytes.len() as u64 != reference.size || sha256_hex(&bytes) != reference.sha256 {
        return Err(ProjectFailure::new(
            ProjectFailureKind::IdentityMismatch,
            role,
            "size or SHA-256 does not match input",
        ));
    }
    Ok(LoadedInput {
        reference: reference.clone(),
        bytes,
    })
}

fn is_reparse(metadata: &Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

pub fn checked_root(root: &Path, role: &str) -> Result<PathBuf, ProjectFailure> {
    let root = root.components().collect::<PathBuf>();
    let metadata = fs::symlink_metadata(&root)
        .map_err(|error| ProjectFailure::new(ProjectFailureKind::Io, role, error.to_string()))?;
    if is_reparse(&metadata) {
        return Err(ProjectFailure::new(
            ProjectFailureKind::InvalidPath,
            role,
            "reparse points and symbolic links are not allowed",
        ));
    }
    if !metadata.is_dir() {
        return Err(ProjectFailure::new(
            if role == "client root" {
                ProjectFailureKind::Usage
            } else {
                ProjectFailureKind::InvalidPath
            },
            role,
            "expected a directory",
        ));
    }
    fs::canonicalize(root)
        .map_err(|error| ProjectFailure::new(ProjectFailureKind::Io, role, error.to_string()))
}

pub fn checked_path(root: &Path, relative: &Path, role: &str) -> Result<PathBuf, ProjectFailure> {
    let mut resolved = root.to_path_buf();
    for component in relative.components() {
        if !matches!(component, Component::Normal(_)) {
            return Err(ProjectFailure::new(
                ProjectFailureKind::InvalidPath,
                role,
                "input escapes its root",
            ));
        }
        resolved.push(component.as_os_str());
        let metadata = fs::symlink_metadata(&resolved).map_err(|error| {
            ProjectFailure::new(ProjectFailureKind::Io, role, error.to_string())
        })?;
        if is_reparse(&metadata) {
            return Err(ProjectFailure::new(
                ProjectFailureKind::InvalidPath,
                role,
                "reparse points and symbolic links are not allowed",
            ));
        }
    }
    let resolved = fs::canonicalize(resolved)
        .map_err(|error| ProjectFailure::new(ProjectFailureKind::Io, role, error.to_string()))?;
    if !resolved.starts_with(root) {
        return Err(ProjectFailure::new(
            ProjectFailureKind::InvalidPath,
            role,
            "input escapes its root",
        ));
    }
    Ok(resolved)
}

pub fn read_bounded(path: &Path, role: &str, limit: u64) -> Result<Vec<u8>, ProjectFailure> {
    let io = |error: std::io::Error| {
        ProjectFailure::new(ProjectFailureKind::Io, role, error.to_string())
    };
    let mut file = File::open(path).map_err(io)?;
    let metadata = file.metadata().map_err(io)?;
    if !metadata.is_file() || is_reparse(&metadata) {
        return Err(ProjectFailure::new(
            ProjectFailureKind::InvalidPath,
            role,
            "expected a regular file",
        ));
    }
    if metadata.len() > limit {
        return Err(ProjectFailure::new(
            ProjectFailureKind::ResourceLimit,
            role,
            format!("input exceeds the {limit}-byte limit"),
        ));
    }
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(io)?;
    if bytes.len() as u64 > limit
        || bytes.len() as u64 != metadata.len()
        || file.metadata().map_err(io)?.len() != metadata.len()
    {
        return Err(ProjectFailure::new(
            ProjectFailureKind::IdentityMismatch,
            role,
            "input changed while being read",
        ));
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    use std::time::{SystemTime, UNIX_EPOCH};
    use xivl_formats::model_import::import_obj_to_model;
    use xivl_formats::phb_author::author_phb;
    use xivl_formats::zone::{flatten_zone, parse_model, parse_phb, Vec3};

    const PROJECT: &[u8] =
        include_bytes!("../../../tests/fixtures/public/zone-project/project.json");
    const FIXTURES: &[(&str, &[u8])] = &[
        ("project.json", PROJECT),
        (
            "floor.obj",
            include_bytes!("../../../tests/fixtures/public/zone-project/floor.obj"),
        ),
        (
            "collision.json",
            include_bytes!("../../../tests/fixtures/public/zone-project/collision.json"),
        ),
        (
            "client/layout.bin",
            include_bytes!("../../../tests/fixtures/public/zone-project/client/layout.bin"),
        ),
        (
            "client/model.bin",
            include_bytes!("../../../tests/fixtures/public/zone-project/client/model.bin"),
        ),
        (
            "client/old-collision.bin",
            include_bytes!("../../../tests/fixtures/public/zone-project/client/old-collision.bin"),
        ),
        (
            "client/collision-template.bin",
            include_bytes!(
                "../../../tests/fixtures/public/zone-project/client/collision-template.bin"
            ),
        ),
        (
            "client/unplaced.bin",
            include_bytes!("../../../tests/fixtures/public/zone-project/client/unplaced.bin"),
        ),
    ];

    struct Inputs {
        root: PathBuf,
    }
    impl Inputs {
        fn new() -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let root = std::env::temp_dir()
                .join(format!("xivl-zone-project-{}-{unique}", std::process::id()));
            fs::create_dir(&root).unwrap();
            fs::create_dir(root.join("client")).unwrap();
            for (path, bytes) in FIXTURES {
                fs::write(root.join(path), bytes).unwrap();
            }
            Self { root }
        }
        fn save(&self, value: &Value) {
            fs::write(
                self.root.join("project.json"),
                serde_json::to_vec(value).unwrap(),
            )
            .unwrap();
        }
        fn load(&self) -> Result<LoadedZoneProject, ProjectFailure> {
            load_project(&self.root.join("project.json"), &self.root.join("client"))
        }
    }
    impl Drop for Inputs {
        fn drop(&mut self) {
            for (path, _) in FIXTURES {
                let _ = fs::remove_file(self.root.join(path));
            }
            let _ = fs::remove_dir(self.root.join("client"));
            let _ = fs::remove_dir(&self.root);
        }
    }

    #[test]
    fn zone_project_partial_write_failure_preserves_files_without_publishing_report() {
        for fail_at in [2, 3, 4] {
            let inputs = Inputs::new();
            let prepared = prepare_loaded(inputs.load().unwrap()).unwrap();
            let expected = [
                ("layout.dat", prepared.layout.clone()),
                ("model.dat", prepared.model.clone()),
                ("collision.dat", prepared.collision.clone()),
                ("build-report.json.pending", Vec::new()),
            ];
            let output = inputs.root.join("partial-output");
            let mut writes = 0;
            let error = write_project_with_writer(
                prepared,
                &inputs.root.join("client"),
                &output,
                |path, bytes| {
                    assert_eq!(path.file_name().unwrap(), expected[writes].0);
                    writes += 1;
                    if writes == fail_at {
                        write_new(path, b"partial")?;
                        return Err(ProjectFailure::new(
                            ProjectFailureKind::Io,
                            "output",
                            "injected write failure",
                        ));
                    }
                    write_new(path, bytes)
                },
            )
            .unwrap_err();
            assert_eq!(error.kind, ProjectFailureKind::Io);
            assert_eq!(error.detail, "injected write failure");
            assert_eq!(writes, fail_at);
            assert!(!output.join("build-report.json").exists());
            assert_eq!(fs::read_dir(&output).unwrap().count(), fail_at);
            for (index, (name, bytes)) in expected.iter().take(fail_at).enumerate() {
                let retained = fs::read(output.join(name)).unwrap();
                if index + 1 == fail_at {
                    assert_eq!(retained, b"partial");
                } else {
                    assert_eq!(&retained, bytes);
                }
            }
            for (path, bytes) in FIXTURES {
                assert_eq!(fs::read(inputs.root.join(path)).unwrap(), *bytes);
            }
            for (name, _) in expected.iter().take(fail_at) {
                fs::remove_file(output.join(name)).unwrap();
            }
            fs::remove_dir(output).unwrap();
        }
    }

    #[test]
    fn zone_project_prepares_floor_without_writing_and_reports_deterministically() {
        let inputs = Inputs::new();
        let prepare = || {
            prepare_project(
                &inputs.root.join("project.json"),
                &inputs.root.join("client"),
            )
            .unwrap()
        };
        let prepared = prepare();
        let again = prepare();
        assert_eq!(prepared.layout, again.layout);
        assert_eq!(prepared.model, again.model);
        assert_eq!(prepared.collision, again.collision);
        assert_eq!(prepared.report, again.report);
        let report = &prepared.report;
        assert_eq!(report["status"], "offline-validated");
        assert_eq!(report["checks"]["placementCount"], 4);
        assert_eq!(report["checks"]["flattenedVertexCount"], 16);
        assert_eq!(report["checks"]["triangleCount"], 8);
        assert_eq!(
            report["checks"]["worldBounds"],
            json!([[-32.0, 0.0, -32.0], [32.0, 0.0, 32.0]])
        );
        assert_eq!(
            report["coverage"]["directTable"],
            json!({"status": "complete", "nonzeroRows": 3})
        );
        assert_eq!(report["coverage"]["sceneDependencies"], "unresolved");
        assert_eq!(report["coverage"]["transitiveDependencies"], "unresolved");
        assert_eq!(report["resources"][2]["tag"], "tex");
        assert_eq!(report["resources"][2]["retained"], true);
        assert_eq!(report["outputs"]["model"]["id"], "0xFFFF0001");
        assert_eq!(report["outputs"]["collision"]["id"], "0xFFFF0002");
        for (name, bytes) in [
            ("layout", &prepared.layout),
            ("model", &prepared.model),
            ("collision", &prepared.collision),
        ] {
            assert_eq!(report["outputs"][name]["sha256"], sha256_hex(bytes));
            assert_eq!(report["outputs"][name]["size"], bytes.len());
        }
        let layout = xivl_formats::zone::parse_layout(&prepared.layout).unwrap();
        assert!(layout
            .objects
            .iter()
            .all(|object| object.model == Some(ResourceId::new(0xFFFF0001))
                && object.phb == Some(ResourceId::new(0xFFFF0002))));
        let model = parse_model(&prepared.model).unwrap();
        let collision = flatten_zone(
            &layout,
            &[(
                ResourceId::new(0xFFFF0002),
                parse_phb(&prepared.collision).unwrap(),
            )],
        )
        .unwrap();
        let first_triangle = collision.faces[0]
            .indices
            .map(|index| collision.vertices[index as usize]);
        assert_eq!(
            first_triangle,
            [
                Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: -32.0
                },
                Vec3 {
                    x: 32.0,
                    y: 0.0,
                    z: 0.0
                },
                Vec3 {
                    x: 32.0,
                    y: 0.0,
                    z: -32.0
                },
            ]
        );
        assert_eq!(model.parts[0].indices, [0, 2, 1, 0, 3, 2]);
        let before = inputs.load().unwrap();
        let mut allowed = Vec::new();
        for row in [1, 2] {
            let at = before.layout_inspection.rows[row].span.offset as usize + 0x14;
            allowed.extend(at..at + 4);
        }
        for (at, (a, b)) in before.layout.bytes.iter().zip(&prepared.layout).enumerate() {
            assert!(a == b || allowed.contains(&at));
        }
        for (path, bytes) in FIXTURES {
            assert_eq!(fs::read(inputs.root.join(path)).unwrap(), *bytes);
        }
        assert_eq!(fs::read_dir(&inputs.root).unwrap().count(), 4);
        assert_eq!(fs::read_dir(inputs.root.join("client")).unwrap().count(), 5);
        assert!(!report.to_string().contains(inputs.root.to_str().unwrap()));
        let mut project: Value = serde_json::from_slice(PROJECT).unwrap();
        project["resources"].as_array_mut().unwrap().reverse();
        project["expectedPlacements"]
            .as_array_mut()
            .unwrap()
            .reverse();
        inputs.save(&project);
        assert_eq!(prepare().report, prepared.report);
    }

    #[test]
    fn zone_project_refuses_same_bounds_collision_changes_and_reversed_winding() {
        for reversed in [false, true] {
            let inputs = Inputs::new();
            let mut geometry: Value =
                serde_json::from_slice(&fs::read(inputs.root.join("collision.json")).unwrap())
                    .unwrap();
            if reversed {
                geometry["triangles"][0]["indices"] = json!([0, 1, 2]);
            } else {
                geometry["vertices"][0][0] = json!(f32::from_bits((-16.0f32).to_bits() - 1));
            }
            let bytes = serde_json::to_vec(&geometry).unwrap();
            fs::write(inputs.root.join("collision.json"), &bytes).unwrap();
            let mut project: Value = serde_json::from_slice(PROJECT).unwrap();
            project["geometry"]["collisionJson"]["size"] = json!(bytes.len());
            project["geometry"]["collisionJson"]["sha256"] = json!(sha256_hex(&bytes));
            inputs.save(&project);
            let loaded = inputs.load().unwrap();
            let authored =
                author_phb(&loaded.collision_template.bytes, &loaded.collision_geometry).unwrap();
            let zone = flatten_zone(
                &loaded.parsed_layout,
                &[(ResourceId::new(0x05060708), parse_phb(&authored).unwrap())],
            )
            .unwrap();
            assert_eq!(
                finite_bounds(zone.vertices.iter().copied(), "collision", 0x48).unwrap(),
                [[-32.0, 0.0, -32.0], [32.0, 0.0, 32.0]]
            );
            assert_eq!(zone.faces.len(), 8);
            let error = prepare_project(
                &inputs.root.join("project.json"),
                &inputs.root.join("client"),
            )
            .unwrap_err();
            assert_eq!(error.kind, ProjectFailureKind::InvalidProject);
            assert_eq!(error.role, "collision");
            assert_eq!(error.offset, Some(0x48));
            assert_eq!(
                error.detail,
                "ordered render/collision triangle mismatch at placement 0, part 0, triangle 0"
            );
        }
    }

    #[test]
    fn zone_project_refuses_unreferenced_collision_vertex_outside_render_bounds() {
        let inputs = Inputs::new();
        let mut loaded = inputs.load().unwrap();
        loaded.collision_geometry.vertices.push(Vec3 {
            x: 64.0,
            y: 0.0,
            z: 0.0,
        });
        let error = prepare_loaded(loaded).unwrap_err();
        assert_eq!(error.kind, ProjectFailureKind::InvalidProject);
        assert_eq!(error.role, "collision");
        assert_eq!(error.offset, Some(0x48));
        assert_eq!(
            error.detail,
            "render and collision vertex counts differ per placement"
        );

        let mut loaded = inputs.load().unwrap();
        loaded.parsed_layout.objects.truncate(1);
        loaded.collision_geometry.vertices.push(Vec3 {
            x: 64.0,
            y: 0.0,
            z: 0.0,
        });
        let bytes =
            author_phb(&loaded.collision_template.bytes, &loaded.collision_geometry).unwrap();
        let zone = flatten_zone(
            &loaded.parsed_layout,
            &[(ResourceId::new(0x05060708), parse_phb(&bytes).unwrap())],
        )
        .unwrap();
        let prepared = prepare_loaded(inputs.load().unwrap()).unwrap();
        let mut model = parse_model(&prepared.model).unwrap();
        model.parts[0].vertices.push(Vec3::ZERO);
        let error =
            compare_triangles(&loaded.parsed_layout, &model, &zone, 0x40, 0x48).unwrap_err();
        assert_eq!(error.kind, ProjectFailureKind::InvalidProject);
        assert_eq!(error.role, "collision");
        assert_eq!(error.offset, Some(0x48));
        assert_eq!(error.detail, "render and collision world bounds differ");
    }

    #[test]
    fn zone_project_refuses_signed_zero_triangle_difference() {
        let inputs = Inputs::new();
        let prepared = prepare_loaded(inputs.load().unwrap()).unwrap();
        let (layout, _) = parse_layout_with_structure(&prepared.layout).unwrap();
        let model = parse_model(&prepared.model).unwrap();
        let mut zone = flatten_zone(
            &layout,
            &[(
                ResourceId::new(0xFFFF0002),
                parse_phb(&prepared.collision).unwrap(),
            )],
        )
        .unwrap();
        compare_triangles(&layout, &model, &zone, 0x40, 0x48).unwrap();
        assert_eq!(zone.vertices[0].y.to_bits(), 0.0f32.to_bits());
        zone.vertices[0].y = -0.0;
        let error = compare_triangles(&layout, &model, &zone, 0x40, 0x48).unwrap_err();
        assert_eq!(error.kind, ProjectFailureKind::InvalidProject);
        assert_eq!(error.role, "collision");
        assert_eq!(error.offset, Some(0x48));
        assert_eq!(
            error.detail,
            "ordered render/collision triangle mismatch at placement 0, part 0, triangle 0"
        );
    }

    #[test]
    fn zone_project_preparation_preserves_writer_failure_kinds_and_offsets() {
        for (path, pointer, at, kind, role) in [
            (
                "client/model.bin",
                "/resources/0/file",
                0,
                ErrorKind::BadMagic,
                "model/renderObj",
            ),
            (
                "client/collision-template.bin",
                "/collisionTemplate",
                0xcc,
                ErrorKind::InvalidAttributeValue,
                "collisionTemplate/collisionJson",
            ),
        ] {
            let inputs = Inputs::new();
            let mut bytes = fs::read(inputs.root.join(path)).unwrap();
            bytes[at] ^= 1;
            fs::write(inputs.root.join(path), &bytes).unwrap();
            let mut project: Value = serde_json::from_slice(PROJECT).unwrap();
            project.pointer_mut(pointer).unwrap()["sha256"] = json!(sha256_hex(&bytes));
            inputs.save(&project);
            let error = prepare_project(
                &inputs.root.join("project.json"),
                &inputs.root.join("client"),
            )
            .unwrap_err();
            assert_eq!(error.kind, ProjectFailureKind::Format(kind));
            assert_eq!(error.offset, Some(at as u64));
            assert_eq!(error.role, role);
        }
    }

    #[test]
    fn zone_project_checks_retained_allocations_without_comparing_payload_hashes() {
        let inputs = Inputs::new();
        let loaded = inputs.load().unwrap();
        let prepared = prepare_project(
            &inputs.root.join("project.json"),
            &inputs.root.join("client"),
        )
        .unwrap();
        let original = &loaded.resources[0].bytes;
        let source = parse_container(original, 0).unwrap();
        let authored = parse_container(&prepared.model, 0).unwrap();
        assert_ne!(source, authored);
        assert!(same_allocation(&source, &authored));
        let mut changed = prepared.model.clone();
        let directory = source.directory_start() as usize;
        changed[directory + 12] ^= 1;
        let error = verify_allocation(original, &changed, "model", true).unwrap_err();
        assert_eq!(error.kind, ProjectFailureKind::InvalidProject);
        assert_eq!(error.role, "model");
        assert_eq!(error.offset, Some(0x40));
        assert_eq!(
            error.detail,
            "prepared resource changed its container allocation or RES directory"
        );
        let mut changed = prepared.collision.clone();
        changed.push(0);
        let error = verify_allocation(
            &loaded.collision_template.bytes,
            &changed,
            "collision",
            false,
        )
        .unwrap_err();
        assert_eq!(error.kind, ProjectFailureKind::InvalidProject);
        assert_eq!(error.role, "collision");
        assert_eq!(error.offset, Some(0x48));
        assert_eq!(
            error.detail,
            "prepared resource changed its container allocation or RES directory"
        );
        let mut short_header = original.clone();
        short_header[0x0e..0x10].copy_from_slice(&0x30u16.to_le_bytes());
        let mut output = prepared.model.clone();
        output[0x0e..0x10].copy_from_slice(&0x30u16.to_le_bytes());
        let (report, offset) = verify_allocation(&short_header, &output, "model", true).unwrap();
        assert_eq!(offset, 0x40);
        assert_eq!(report["directoryStart"], 0x40);
        assert_eq!(&short_header[0x40..0x50], &output[0x40..0x50]);
    }

    #[test]
    fn zone_project_loads_exact_direct_inputs_and_both_writers_accept_templates() {
        let inputs = Inputs::new();
        let loaded = inputs.load().unwrap();
        assert_eq!(
            loaded
                .resources
                .iter()
                .map(|resource| (resource.declaration.row, resource.declaration.id.0.value()))
                .collect::<Vec<_>>(),
            [(1, 0x01020304), (2, 0x05060708), (3, 0x090A0B0C)]
        );
        assert_eq!(
            loaded.resources[2].bytes,
            b"authored opaque direct dependency\n"
        );
        assert_eq!(loaded.layout_inspection.rows.len(), 4);
        assert_eq!(loaded.layout_inspection.placements.phb, 4);
        assert_eq!(loaded.layout_inspection.placements.model, 4);
        assert_eq!(loaded.layout_inspection.placements.collision_boxes, 0);
        parse_phb(&loaded.resources[1].bytes).unwrap();
        let old_template_error =
            author_phb(&loaded.resources[1].bytes, &loaded.collision_geometry).unwrap_err();
        assert_eq!(old_template_error.kind(), ErrorKind::InvalidAttributeValue);
        assert_eq!(old_template_error.offset(), 0xCC);
        assert!(loaded
            .parsed_layout
            .objects
            .iter()
            .all(|object| object.model != Some(ResourceId::new(0x090A0B0C))
                && object.phb != Some(ResourceId::new(0x090A0B0C))));
        let model_bytes = import_obj_to_model(
            &loaded.resources[0].bytes,
            std::str::from_utf8(&loaded.render_obj.bytes).unwrap(),
        )
        .unwrap();
        let model = parse_model(&model_bytes).unwrap();
        assert_eq!(model.parts.len(), 1);
        assert_eq!(model.parts[0].indices, [0, 2, 1, 0, 3, 2]);
        assert_eq!(
            model.parts[0].vertices,
            [
                Vec3 {
                    x: -16.0,
                    y: 0.0,
                    z: -16.0
                },
                Vec3 {
                    x: 16.0,
                    y: 0.0,
                    z: -16.0
                },
                Vec3 {
                    x: 16.0,
                    y: 0.0,
                    z: 16.0
                },
                Vec3 {
                    x: -16.0,
                    y: 0.0,
                    z: 16.0
                },
            ]
        );
        assert_eq!(model_bytes.len(), loaded.resources[0].bytes.len());
        let phb_bytes =
            author_phb(&loaded.collision_template.bytes, &loaded.collision_geometry).unwrap();
        assert_eq!(phb_bytes.len(), loaded.collision_template.bytes.len());
        let phb = parse_phb(&phb_bytes).unwrap();
        assert_eq!(phb.hulls[0].vertices, model.parts[0].vertices);
        assert_eq!(phb.hulls[0].indices, [0, 2, 1, 0, 3, 2]);
        assert_eq!(phb.hulls[0].surfaces, [0, 0]);
        let zone =
            flatten_zone(&loaded.parsed_layout, &[(ResourceId::new(0x05060708), phb)]).unwrap();
        assert_eq!(zone.vertices.len(), 16);
        assert_eq!(zone.faces.len(), 8);
        assert_eq!(
            zone.vertices
                .iter()
                .map(|v| v.x)
                .fold(f32::INFINITY, f32::min),
            -32.0
        );
        assert_eq!(
            zone.vertices
                .iter()
                .map(|v| v.x)
                .fold(f32::NEG_INFINITY, f32::max),
            32.0
        );
        assert_eq!(
            zone.vertices
                .iter()
                .map(|v| v.z)
                .fold(f32::INFINITY, f32::min),
            -32.0
        );
        assert_eq!(
            zone.vertices
                .iter()
                .map(|v| v.z)
                .fold(f32::NEG_INFINITY, f32::max),
            32.0
        );
        assert!(zone.vertices.iter().all(|v| v.y == 0.0));
        for (path, bytes) in FIXTURES {
            assert_eq!(fs::read(inputs.root.join(path)).unwrap(), *bytes);
        }
    }

    #[test]
    fn zone_project_requires_the_unplaced_row_declaration_and_file() {
        let inputs = Inputs::new();
        let mut value: Value = serde_json::from_slice(PROJECT).unwrap();
        value["resources"].as_array_mut().unwrap().pop();
        inputs.save(&value);
        let error = inputs.load().unwrap_err();
        assert_eq!(error.kind, ProjectFailureKind::InventoryMismatch);
        assert!(error.detail.contains("missing nonzero direct row 3"));
        fs::write(inputs.root.join("project.json"), PROJECT).unwrap();
        fs::remove_file(inputs.root.join("client/unplaced.bin")).unwrap();
        let error = inputs.load().unwrap_err();
        assert_eq!(error.kind, ProjectFailureKind::Io);
        assert_eq!(error.role, "resources[3]");
    }

    #[test]
    fn zone_project_checks_exact_size_hash_and_format_offset() {
        let inputs = Inputs::new();
        let mut value: Value = serde_json::from_slice(PROJECT).unwrap();
        value["layout"]["size"] = json!(1);
        inputs.save(&value);
        assert_eq!(
            inputs.load().unwrap_err().kind,
            ProjectFailureKind::IdentityMismatch
        );
        fs::write(inputs.root.join("project.json"), PROJECT).unwrap();
        let mut bytes = fs::read(inputs.root.join("client/layout.bin")).unwrap();
        bytes[0] = b'X';
        fs::write(inputs.root.join("client/layout.bin"), &bytes).unwrap();
        assert_eq!(
            inputs.load().unwrap_err().kind,
            ProjectFailureKind::IdentityMismatch
        );
        let mut value: Value = serde_json::from_slice(PROJECT).unwrap();
        value["layout"]["sha256"] = json!(sha256_hex(&bytes));
        inputs.save(&value);
        let error = inputs.load().unwrap_err();
        assert_eq!(
            error.kind,
            ProjectFailureKind::Format(ErrorKind::InvalidAttributeValue)
        );
        assert_eq!(error.offset, Some(0));
        assert_eq!(error.role, "layout");
    }

    #[test]
    fn zone_project_checks_inventory_and_binding_assertions() {
        let inputs = Inputs::new();
        let original: Value = serde_json::from_slice(PROJECT).unwrap();
        for (pointer, replacement, kind, detail) in [
            (
                "/resources/0/id",
                json!("0x01020305"),
                ProjectFailureKind::InventoryMismatch,
                "row 1 ID or tag does not match layout",
            ),
            (
                "/resources/0/tag",
                json!("tex"),
                ProjectFailureKind::InventoryMismatch,
                "row 1 ID or tag does not match layout",
            ),
            (
                "/resources/2/row",
                json!(4),
                ProjectFailureKind::InventoryMismatch,
                "missing nonzero direct row 3",
            ),
            (
                "/resources/2/row",
                json!(0),
                ProjectFailureKind::InventoryMismatch,
                "zero-key row 0 must not declare a file",
            ),
            (
                "/resources/2/row",
                json!(1),
                ProjectFailureKind::InvalidProject,
                "duplicate resource row",
            ),
            (
                "/binding/modelRow",
                json!(3),
                ProjectFailureKind::InvalidProject,
                "binding key, tags or source IDs do not match the selected rows",
            ),
            (
                "/binding/outputModelId",
                json!("0x090A0B0C"),
                ProjectFailureKind::InvalidProject,
                "output ID collides with an unmodified source row",
            ),
            (
                "/binding/outputCollisionId",
                json!("0x090A0B0C"),
                ProjectFailureKind::InvalidProject,
                "output ID collides with an unmodified source row",
            ),
            (
                "/binding/outputModelId",
                json!("0xFFFF0002"),
                ProjectFailureKind::InvalidProject,
                "binding rows and output IDs must be distinct",
            ),
            (
                "/binding/modelRow",
                json!(2),
                ProjectFailureKind::InvalidProject,
                "binding rows and output IDs must be distinct",
            ),
            (
                "/expectedPlacements/0/worldMatrix/0/3",
                json!(17),
                ProjectFailureKind::InvalidProject,
                "expected placement identity or matrix does not match layout",
            ),
            (
                "/expectedPlacements/0/sourceNode",
                json!(1),
                ProjectFailureKind::InvalidProject,
                "expected placement identity or matrix does not match layout",
            ),
        ] {
            let mut value = original.clone();
            *value.pointer_mut(pointer).unwrap() = replacement;
            inputs.save(&value);
            let error = inputs.load().unwrap_err();
            assert_eq!(error.kind, kind, "{pointer}: {error}");
            assert_eq!(error.detail, detail, "{pointer}: {error}");
        }
        let mut value = original.clone();
        value["expectedPlacements"][1] = value["expectedPlacements"][0].clone();
        inputs.save(&value);
        let error = inputs.load().unwrap_err();
        assert_eq!(error.kind, ProjectFailureKind::InvalidProject);
        assert_eq!(
            error.detail,
            "expected placement identity or matrix does not match layout"
        );
        let mut value = original;
        value["resources"].as_array_mut().unwrap().reverse();
        inputs.save(&value);
        assert_eq!(inputs.load().unwrap().resources[0].declaration.row, 1);
    }

    #[test]
    fn zone_project_rejects_duplicate_binding_key_tag_pairs() {
        let inputs = Inputs::new();
        let loaded = inputs.load().unwrap();
        for tag in ["brt", "bhp"] {
            let mut layout = loaded.parsed_layout.clone();
            layout.resources[3].name = "floor".into();
            layout.resources[3].type_tag = tag.into();
            let error = validate_binding(&loaded.project, &layout).unwrap_err();
            assert_eq!(error.kind, ProjectFailureKind::InvalidProject, "{tag}");
            assert_eq!(
                error.detail, "binding key/tag must select exactly one row",
                "{tag}"
            );
        }
    }

    #[test]
    fn zone_project_rejects_duplicate_unknown_and_sequence_objects() {
        let original = std::str::from_utf8(PROJECT).unwrap();
        for (needle, replacement, message) in [
            (
                "\"schemaVersion\": 1",
                "\"schemaVersion\": 1, \"schemaVersion\": 1",
                "duplicate field",
            ),
            (
                "\"modelRow\": 1",
                "\"modelRow\": 1, \"modelRow\": 1",
                "duplicate field",
            ),
            (
                "\"path\": \"layout.bin\"",
                "\"path\": \"layout.bin\", \"path\": \"layout.bin\"",
                "duplicate field",
            ),
            (
                "\"sourceNode\":",
                "\"unknown\": 0, \"sourceNode\":",
                "unknown field",
            ),
            (
                "\"profile\":",
                "\"unknown\": 0, \"profile\":",
                "unknown field",
            ),
        ] {
            let bytes = original.replacen(needle, replacement, 1);
            let error = parse_project(bytes.as_bytes()).unwrap_err();
            assert_eq!(error.kind, ProjectFailureKind::InvalidProject);
            assert!(error.detail.contains(message), "{error}");
            assert!(error.offset.unwrap() > 0);
        }
        let original: Value = serde_json::from_slice(PROJECT).unwrap();
        let error = parse_project(b"[]").unwrap_err();
        assert!(error.detail.contains("expected an object"));
        for pointer in [
            "/layout",
            "/resources/0",
            "/resources/0/file",
            "/geometry",
            "/binding",
            "/expectedPlacements/0",
        ] {
            let mut value = original.clone();
            *value.pointer_mut(pointer).unwrap() = json!([]);
            assert!(
                parse_project(&serde_json::to_vec(&value).unwrap())
                    .unwrap_err()
                    .detail
                    .contains("expected an object"),
                "{pointer}"
            );
        }
    }

    #[test]
    fn zone_project_rejects_invalid_integer_slots_identities_and_paths() {
        let original: Value = serde_json::from_slice(PROJECT).unwrap();
        for (pointer, replacement) in [
            ("/schemaVersion", json!(2)),
            ("/schemaVersion", json!(1.0)),
            ("/layout/size", json!(-1)),
            ("/layout/size", json!(true)),
            ("/layout/size", json!(0)),
            ("/resources/0/row", json!(4294967296u64)),
            ("/resources/0/id", json!("0x00000000")),
            ("/binding/outputModelId", json!("0xffff0001")),
            ("/binding/outputModelId", json!(123)),
            ("/layout/sha256", json!("bad")),
            ("/expectedPlacements/0/worldMatrix/0/0", json!(1e39)),
        ] {
            let mut value = original.clone();
            *value.pointer_mut(pointer).unwrap() = replacement;
            assert!(
                parse_project(&serde_json::to_vec(&value).unwrap()).is_err(),
                "{pointer}"
            );
        }
        for path in [
            "../layout.bin",
            "client/../layout.bin",
            "/layout.bin",
            "C:/layout.bin",
            "C:layout.bin",
            "\\\\host\\share",
            "a\\b",
            "a//b",
            "./layout.bin",
            "a/",
            "layout.bin:stream",
            "NUL.bin",
            "a./b",
            "a /b",
        ] {
            let mut value = original.clone();
            value["layout"]["path"] = json!(path);
            let error = parse_project(&serde_json::to_vec(&value).unwrap()).unwrap_err();
            assert_eq!(error.kind, ProjectFailureKind::InvalidPath, "{path}");
        }
    }

    #[test]
    fn zone_project_bounds_project_bytes_before_json_parsing() {
        let error = parse_project(&vec![b'!'; MAX_PROJECT_BYTES as usize + 1]).unwrap_err();
        assert_eq!(error.kind, ProjectFailureKind::ResourceLimit);
        let inputs = Inputs::new();
        let file = File::create(inputs.root.join("project.json")).unwrap();
        file.set_len(MAX_PROJECT_BYTES + 1).unwrap();
        let error = inputs.load().unwrap_err();
        assert_eq!(error.kind, ProjectFailureKind::ResourceLimit);
        assert_eq!(error.role, "project");
    }

    #[test]
    fn zone_project_refuses_oversized_resource_before_reading_bytes() {
        let inputs = Inputs::new();
        let path = inputs.root.join("client/unplaced.bin");
        let file = File::options().write(true).open(&path).unwrap();
        file.set_len(MAX_RESOURCE_BYTES + 1).unwrap();
        let error = inputs.load().unwrap_err();
        assert_eq!(error.kind, ProjectFailureKind::IdentityMismatch);
        assert_eq!(error.role, "resources[3]");
        assert_eq!(error.detail, "declared size does not match input");

        let error = read_bounded(&path, "resources[3]", MAX_RESOURCE_BYTES).unwrap_err();
        assert_eq!(error.kind, ProjectFailureKind::ResourceLimit);
        assert_eq!(error.role, "resources[3]");
        assert_eq!(error.detail, "input exceeds the 268435456-byte limit");
    }

    #[test]
    fn zone_project_requires_an_explicit_root_and_regular_input_files() {
        let inputs = Inputs::new();
        let error = load_project(&inputs.root.join("project.json"), Path::new("")).unwrap_err();
        assert_eq!(error.kind, ProjectFailureKind::InvalidPath);
        assert_eq!(error.role, "client root");
        let mut value: Value = serde_json::from_slice(PROJECT).unwrap();
        value["geometry"]["renderObj"]["path"] = json!("client");
        inputs.save(&value);
        let error = inputs.load().unwrap_err();
        assert_eq!(error.kind, ProjectFailureKind::InvalidPath);
        assert_eq!(error.role, "renderObj");
    }

    #[cfg(unix)]
    #[test]
    fn zone_project_rejects_symbolic_client_root() {
        let inputs = Inputs::new();
        let alias = inputs.root.join("client-alias");
        std::os::unix::fs::symlink(inputs.root.join("client"), &alias).unwrap();
        let error = load_project(&inputs.root.join("project.json"), &alias).unwrap_err();
        fs::remove_file(alias).unwrap();
        assert_eq!(error.kind, ProjectFailureKind::InvalidPath);
        assert_eq!(error.role, "client root");
        assert_eq!(
            error.detail,
            "reparse points and symbolic links are not allowed"
        );
    }

    #[cfg(windows)]
    #[test]
    fn zone_project_rejects_junction_aliases_in_inputs_and_roots() {
        let inputs = Inputs::new();
        let alias = inputs.root.join("alias");
        let output = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&alias)
            .arg(inputs.root.join("client"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let error = load_project(&inputs.root.join("project.json"), &alias).unwrap_err();
        assert_eq!(error.kind, ProjectFailureKind::InvalidPath);
        let error =
            load_project(&alias.join("layout.bin"), &inputs.root.join("client")).unwrap_err();
        assert_eq!(error.kind, ProjectFailureKind::InvalidPath);
        let mut value: Value = serde_json::from_slice(PROJECT).unwrap();
        value["geometry"]["renderObj"]["path"] = json!("alias/model.bin");
        inputs.save(&value);
        assert_eq!(
            inputs.load().unwrap_err().kind,
            ProjectFailureKind::InvalidPath
        );
        // Remove only the junction itself, without traversing its target.
        fs::remove_dir(alias).unwrap();
        assert!(inputs.root.join("client/model.bin").is_file());
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn zone_project_loads_roots_under_a_linked_ancestor() {
        let inputs = Inputs::new();
        let alias = inputs.root.join("ancestor");
        #[cfg(unix)]
        std::os::unix::fs::symlink(inputs.root.parent().unwrap(), &alias).unwrap();
        #[cfg(windows)]
        {
            let output = std::process::Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                .arg(&alias)
                .arg(inputs.root.parent().unwrap())
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        let root = alias.join(inputs.root.file_name().unwrap());
        let loaded = load_project(&root.join("project.json"), &root.join("client")).unwrap();
        assert_eq!(loaded.resources.len(), 3);
        assert_eq!(
            loaded.render_obj.bytes,
            fs::read(inputs.root.join("floor.obj")).unwrap()
        );
        #[cfg(unix)]
        fs::remove_file(alias).unwrap();
        #[cfg(windows)]
        fs::remove_dir(alias).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn zone_project_rejects_symbolic_parent_directories_below_roots() {
        let inputs = Inputs::new();
        let alias = inputs.root.join("client/alias");
        std::os::unix::fs::symlink(inputs.root.join("client"), &alias).unwrap();
        let mut value: Value = serde_json::from_slice(PROJECT).unwrap();
        value["resources"][2]["file"]["path"] = json!("alias/unplaced.bin");
        inputs.save(&value);
        let error = inputs.load().unwrap_err();
        assert_eq!(error.kind, ProjectFailureKind::InvalidPath);
        assert_eq!(error.role, "resources[3]");
        assert_eq!(
            error.detail,
            "reparse points and symbolic links are not allowed"
        );
        fs::remove_file(alias).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn zone_project_rejects_symbolic_file_aliases() {
        let inputs = Inputs::new();
        let alias = inputs.root.join("alias.obj");
        std::os::unix::fs::symlink(inputs.root.join("floor.obj"), &alias).unwrap();
        let mut value: Value = serde_json::from_slice(PROJECT).unwrap();
        value["geometry"]["renderObj"]["path"] = json!("alias.obj");
        inputs.save(&value);
        assert_eq!(
            inputs.load().unwrap_err().kind,
            ProjectFailureKind::InvalidPath
        );
        fs::remove_file(alias).unwrap();
    }
}
