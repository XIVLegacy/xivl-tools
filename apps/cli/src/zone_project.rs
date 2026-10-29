//! Explicit, identity-pinned inputs for an offline floor project.

use std::collections::BTreeSet;
use std::fmt;
use std::fs::{self, File, Metadata};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use serde::de::{value::MapAccessDeserializer, MapAccess, Visitor};
use serde::{Deserialize, Deserializer};
use xivl_formats::collision_json::{parse_geometry_json, MAX_GEOMETRY_JSON_BYTES};
use xivl_formats::digest::sha256_hex;
use xivl_formats::phb_author::CollisionGeometry;
use xivl_formats::zone::{parse_layout_with_structure, Layout, LayoutInspection};
use xivl_formats::{ErrorKind, FormatError, ResourceId};

/// Limits apply before parsing or allocating the declared input size.
pub const MAX_PROJECT_BYTES: u64 = 1024 * 1024;
pub const MAX_LAYOUT_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_RESOURCE_BYTES: u64 = 256 * 1024 * 1024;
pub const MAX_PHB_TEMPLATE_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_OBJ_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_TOTAL_INPUT_BYTES: u64 = 512 * 1024 * 1024;

// Serde's derived struct visitor accepts sequences as well as maps.
struct ObjectOnly<T>(T);

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
    fn new(kind: ProjectFailureKind, role: &str, detail: impl Into<String>) -> Self {
        Self {
            kind,
            role: role.into(),
            offset: None,
            detail: detail.into(),
        }
    }
    fn format(role: &str, error: FormatError) -> Self {
        Self {
            kind: ProjectFailureKind::Format(error.kind()),
            role: role.into(),
            offset: Some(error.offset()),
            detail: error.detail().into(),
        }
    }
    pub fn code(&self) -> u8 {
        if self.kind == ProjectFailureKind::Io {
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

fn validate_reference(
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
    let project_bytes = read_bounded(&project_path, "project", MAX_PROJECT_BYTES)?;
    let project = parse_project(&project_bytes)?;
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

fn load_input(
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

fn checked_root(root: &Path, role: &str) -> Result<PathBuf, ProjectFailure> {
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
            ProjectFailureKind::InvalidPath,
            role,
            "expected a directory",
        ));
    }
    fs::canonicalize(root)
        .map_err(|error| ProjectFailure::new(ProjectFailureKind::Io, role, error.to_string()))
}

fn checked_path(root: &Path, relative: &Path, role: &str) -> Result<PathBuf, ProjectFailure> {
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

fn read_bounded(path: &Path, role: &str, limit: u64) -> Result<Vec<u8>, ProjectFailure> {
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
