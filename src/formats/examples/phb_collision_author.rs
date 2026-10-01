//! Explicit-path PHB collision authoring example.
//!
//! The command requires a native PHB template, a strict geometry JSON input,
//! and a new output path. It never searches for a client install and never
//! overwrites an existing file.

use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process;

use serde::de::{value::MapAccessDeserializer, MapAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::Value;
use xivl_formats::digest::sha256_hex;
use xivl_formats::phb_author::{author_phb, CollisionGeometry, CollisionTriangle};
use xivl_formats::zone::{parse_phb, Vec3};

const MAX_TEMPLATE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_GEOMETRY_JSON_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct GeometryJson {
    schema_version: Value,
    vertices: Value,
    triangles: Vec<ObjectOnly<TriangleJson>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TriangleJson {
    indices: Value,
    surface: Value,
}

// Derived struct deserializers also accept sequences. The contract requires
// objects, while the derives reject duplicate and unknown members.
struct ObjectOnly<T>(T);

impl<'de, T: Deserialize<'de>> Deserialize<'de> for ObjectOnly<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct ObjectVisitor<T>(std::marker::PhantomData<T>);

        impl<'de, T: Deserialize<'de>> Visitor<'de> for ObjectVisitor<T> {
            type Value = ObjectOnly<T>;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("an object")
            }

            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Self::Value, A::Error> {
                T::deserialize(MapAccessDeserializer::new(map)).map(ObjectOnly)
            }
        }

        deserializer.deserialize_map(ObjectVisitor(std::marker::PhantomData))
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut arguments = env::args_os();
    let program = arguments
        .next()
        .unwrap_or_else(|| "phb_collision_author".into());
    let template = arguments
        .next()
        .map(PathBuf::from)
        .ok_or_else(|| usage(&program))?;
    let geometry_json = arguments
        .next()
        .map(PathBuf::from)
        .ok_or_else(|| usage(&program))?;
    let output = arguments
        .next()
        .map(PathBuf::from)
        .ok_or_else(|| usage(&program))?;
    if arguments.next().is_some() {
        return Err(usage(&program));
    }

    let template_absolute = fs::canonicalize(&template).map_err(|error| {
        format!(
            "cannot resolve native PHB input {}: {error}",
            template.display()
        )
    })?;
    let geometry_json_absolute = fs::canonicalize(&geometry_json).map_err(|error| {
        format!(
            "cannot resolve geometry JSON input {}: {error}",
            geometry_json.display()
        )
    })?;
    let output_absolute = output_path_for_compare(&output)?;
    if output_absolute == template_absolute || output_absolute == geometry_json_absolute {
        return Err("refusing to overwrite an input file".into());
    }
    if output.exists() {
        return Err(format!(
            "refusing to overwrite existing output {}",
            output.display()
        ));
    }

    let template_bytes = read_bounded(&template, "native PHB input", MAX_TEMPLATE_BYTES)?;
    let geometry_json_bytes = read_bounded(
        &geometry_json,
        "geometry JSON input",
        MAX_GEOMETRY_JSON_BYTES,
    )?;
    let geometry = parse_geometry_json(&geometry_json_bytes)?;
    let output_bytes = author_phb(&template_bytes, &geometry)
        .map_err(|error| format!("PHB authoring rejected: {error}"))?;
    let phb =
        parse_phb(&output_bytes).map_err(|error| format!("generated PHB is invalid: {error}"))?;
    write_new_output(&output, &output_bytes)?;

    let vertices = phb
        .hulls
        .iter()
        .map(|hull| hull.vertices.len())
        .sum::<usize>();
    let triangles = phb
        .hulls
        .iter()
        .map(|hull| hull.indices.len() / 3)
        .sum::<usize>();
    println!("template_length={}", template_bytes.len());
    println!("geometry_json_length={}", geometry_json_bytes.len());
    println!("output_length={}", output_bytes.len());
    println!("template_sha256={}", sha256_hex(&template_bytes));
    println!("geometry_json_sha256={}", sha256_hex(&geometry_json_bytes));
    println!("output_sha256={}", sha256_hex(&output_bytes));
    println!("hulls={}", phb.hulls.len());
    println!("vertices={vertices}");
    println!("triangles={triangles}");
    Ok(())
}

fn usage(program: &std::ffi::OsStr) -> String {
    format!(
        "usage: {} <native-phb-input> <geometry-json-input> <new-output>",
        Path::new(program).display()
    )
}

fn output_path_for_compare(output: &Path) -> Result<PathBuf, String> {
    let parent = output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let parent = fs::canonicalize(parent).map_err(|error| {
        format!(
            "cannot resolve output directory {}: {error}",
            parent.display()
        )
    })?;
    let name = output
        .file_name()
        .ok_or_else(|| "output path has no file name".to_string())?;
    Ok(parent.join(name))
}

fn read_bounded(path: &Path, label: &str, limit: u64) -> Result<Vec<u8>, String> {
    let mut file = File::open(path)
        .map_err(|error| format!("cannot read {label} {}: {error}", path.display()))?;
    let initial_length = file
        .metadata()
        .map_err(|error| format!("cannot inspect {label} {}: {error}", path.display()))?
        .len();
    if initial_length > limit {
        return Err(format!(
            "{label} {} exceeds the {limit}-byte limit",
            path.display()
        ));
    }
    let capacity = usize::try_from(initial_length)
        .map_err(|_| format!("{label} {} is too large for this platform", path.display()))?;
    let mut bytes = Vec::with_capacity(capacity);
    let read_limit = limit.saturating_add(1);
    Read::by_ref(&mut file)
        .take(read_limit)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("cannot read {label} {}: {error}", path.display()))?;
    let final_length = file
        .metadata()
        .map_err(|error| format!("cannot inspect {label} {}: {error}", path.display()))?
        .len();
    if bytes.len() as u64 > limit
        || final_length != initial_length
        || bytes.len() as u64 != initial_length
    {
        return Err(format!(
            "{label} {} changed while being read",
            path.display()
        ));
    }
    Ok(bytes)
}

fn parse_geometry_json(bytes: &[u8]) -> Result<CollisionGeometry, String> {
    let root: ObjectOnly<GeometryJson> = serde_json::from_slice(bytes)
        .map_err(|error| format!("geometry JSON is invalid: {error}"))?;
    let root = root.0;

    let schema_version = parse_u64(&root.schema_version, "schemaVersion")?;
    if schema_version != 1 {
        return Err(format!(
            "unsupported geometry schemaVersion {schema_version}; expected 1"
        ));
    }

    let vertices_array = root
        .vertices
        .as_array()
        .ok_or_else(|| "vertices must be an array".to_string())?;
    let mut vertices = Vec::with_capacity(vertices_array.len());
    for (index, value) in vertices_array.iter().enumerate() {
        vertices.push(parse_vertex(value, index)?);
    }

    let mut triangles = Vec::with_capacity(root.triangles.len());
    for (index, value) in root.triangles.iter().enumerate() {
        triangles.push(parse_triangle(&value.0, index, vertices.len())?);
    }

    Ok(CollisionGeometry {
        vertices,
        triangles,
    })
}

fn parse_vertex(value: &Value, index: usize) -> Result<Vec3, String> {
    let components = value
        .as_array()
        .ok_or_else(|| format!("vertices[{index}] must be an array"))?;
    if components.len() != 3 {
        return Err(format!(
            "vertices[{index}] must contain exactly 3 components"
        ));
    }
    let mut parsed = [0.0_f32; 3];
    for (component_index, component) in components.iter().enumerate() {
        let number = component.as_f64().ok_or_else(|| {
            format!("vertices[{index}][{component_index}] must be a finite number")
        })?;
        let converted = number as f32;
        if !number.is_finite() || !converted.is_finite() {
            return Err(format!(
                "vertices[{index}][{component_index}] must be representable as f32"
            ));
        }
        parsed[component_index] = converted;
    }
    Ok(Vec3 {
        x: parsed[0],
        y: parsed[1],
        z: parsed[2],
    })
}

fn parse_triangle(
    value: &TriangleJson,
    triangle_index: usize,
    vertex_count: usize,
) -> Result<CollisionTriangle, String> {
    let indices_array = value
        .indices
        .as_array()
        .ok_or_else(|| format!("triangles[{triangle_index}].indices must be an array"))?;
    if indices_array.len() != 3 {
        return Err(format!(
            "triangles[{triangle_index}].indices must contain exactly 3 values"
        ));
    }
    let mut indices = [0_u16; 3];
    for (index, value) in indices_array.iter().enumerate() {
        let parsed = parse_u16(
            value,
            &format!("triangles[{triangle_index}].indices[{index}]"),
        )?;
        if usize::from(parsed) >= vertex_count {
            return Err(format!(
                "triangles[{triangle_index}].indices[{index}] is outside vertices"
            ));
        }
        indices[index] = parsed;
    }
    let surface = parse_u16(
        &value.surface,
        &format!("triangles[{triangle_index}].surface"),
    )?;
    Ok(CollisionTriangle { indices, surface })
}

fn parse_u64(value: &Value, context: &str) -> Result<u64, String> {
    value
        .as_u64()
        .ok_or_else(|| format!("{context} must be an unsigned integer"))
}

fn parse_u16(value: &Value, context: &str) -> Result<u16, String> {
    let value = parse_u64(value, context)?;
    u16::try_from(value).map_err(|_| format!("{context} is outside the u16 range"))
}

fn write_new_output(output: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)
        .map_err(|error| format!("cannot create output {}: {error}", output.display()))?;
    file.write_all(bytes)
        .map_err(|error| format!("cannot write output {}: {error}", output.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn valid_geometry() -> &'static [u8] {
        br#"{"schemaVersion":1,"vertices":[[0,0,0],[1,0,0],[0,1,0]],"triangles":[{"indices":[0,1,2],"surface":7}]}"#
    }

    fn unique_test_path(suffix: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        env::temp_dir().join(format!(
            "xivl-phb-author-{}-{unique}.{suffix}",
            process::id()
        ))
    }

    #[test]
    fn geometry_json_requires_the_exact_schema_and_integer_slots() {
        let geometry = parse_geometry_json(valid_geometry()).unwrap();
        assert_eq!(geometry.vertices.len(), 3);
        assert_eq!(geometry.triangles[0].indices, [0, 1, 2]);
        assert_eq!(geometry.triangles[0].surface, 7);

        let unknown_key = br#"{"schemaVersion":1,"vertices":[],"triangles":[],"extra":true}"#;
        assert!(parse_geometry_json(unknown_key)
            .unwrap_err()
            .contains("unknown field"));

        let missing_key = br#"{"schemaVersion":1,"vertices":[]}"#;
        assert!(parse_geometry_json(missing_key)
            .unwrap_err()
            .contains("missing field `triangles`"));

        let float_index = br#"{"schemaVersion":1,"vertices":[[0,0,0]],"triangles":[{"indices":[0.0,0,0],"surface":1}]}"#;
        assert!(parse_geometry_json(float_index)
            .unwrap_err()
            .contains("unsigned integer"));

        let unsupported_version = br#"{"schemaVersion":2,"vertices":[],"triangles":[]}"#;
        assert!(parse_geometry_json(unsupported_version)
            .unwrap_err()
            .contains("unsupported geometry schemaVersion"));

        let wrong_dimensions = br#"{"schemaVersion":1,"vertices":[[0,0]],"triangles":[]}"#;
        assert!(parse_geometry_json(wrong_dimensions)
            .unwrap_err()
            .contains("exactly 3 components"));

        let unrepresentable_vertex =
            br#"{"schemaVersion":1,"vertices":[[1e39,0,0]],"triangles":[]}"#;
        assert!(parse_geometry_json(unrepresentable_vertex)
            .unwrap_err()
            .contains("representable as f32"));
    }

    #[test]
    fn geometry_json_rejects_duplicate_root_and_triangle_members() {
        for bytes in [
            br#"{"schemaVersion":2,"schemaVersion":1,"vertices":[],"triangles":[]}"#.as_slice(),
            br#"{"schemaVersion":1,"vertices":[[0,0,0]],"triangles":[{"indices":[0,0,0],"surface":2,"surface":1}]}"#.as_slice(),
            br#"{"schemaVersion":1,"vertices":[[0,0,0]],"triangles":[{"indices":[1,0,0],"indices":[0,0,0],"surface":1}]}"#.as_slice(),
        ] {
            assert!(parse_geometry_json(bytes).unwrap_err().contains("duplicate field"));
        }
    }

    #[test]
    fn geometry_json_requires_objects_for_root_and_triangle_records() {
        for bytes in [
            br#"[1,[[0,0,0],[1,0,0],[0,1,0]],[{"indices":[0,1,2],"surface":7}]]"#.as_slice(),
            br#"{"schemaVersion":1,"vertices":[[0,0,0],[1,0,0],[0,1,0]],"triangles":[[[0,1,2],7]]}"#.as_slice(),
        ] {
            assert!(parse_geometry_json(bytes).unwrap_err().contains("expected an object"));
        }
    }

    #[test]
    fn geometry_json_rejects_out_of_range_triangle_values() {
        let out_of_range_index = br#"{"schemaVersion":1,"vertices":[[0,0,0]],"triangles":[{"indices":[1,0,0],"surface":1}]}"#;
        assert!(parse_geometry_json(out_of_range_index)
            .unwrap_err()
            .contains("outside vertices"));

        let out_of_range_surface = br#"{"schemaVersion":1,"vertices":[[0,0,0]],"triangles":[{"indices":[0,0,0],"surface":65536}]}"#;
        assert!(parse_geometry_json(out_of_range_surface)
            .unwrap_err()
            .contains("u16 range"));
    }

    #[test]
    fn bounded_reads_preserve_complete_input() {
        let path = unique_test_path("bin");
        fs::write(&path, b"template").unwrap();
        let bytes = read_bounded(&path, "test input", 64).unwrap();
        assert_eq!(bytes, b"template");
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn bounded_reads_reject_oversized_input() {
        let path = unique_test_path("bin");
        fs::write(&path, b"12345").unwrap();
        let error = read_bounded(&path, "test input", 4).unwrap_err();
        assert!(error.contains("exceeds the 4-byte limit"));
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn exclusive_output_creation_preserves_a_preexisting_sentinel() {
        let path = unique_test_path("bin");
        fs::write(&path, b"sentinel").unwrap();
        let error = write_new_output(&path, b"replacement").unwrap_err();
        assert!(error.contains("cannot create output"));
        assert_eq!(fs::read(&path).unwrap(), b"sentinel");
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn bare_output_name_uses_the_current_directory() {
        let expected = fs::canonicalize(".").unwrap().join("phb-test.bin");
        assert_eq!(
            output_path_for_compare(Path::new("phb-test.bin")).unwrap(),
            expected
        );
    }

    #[test]
    fn io_errors_keep_their_context() {
        let path = unique_test_path("missing");
        let error = read_bounded(&path, "test input", 64).unwrap_err();
        assert!(error.contains("cannot read test input"));
    }
}
