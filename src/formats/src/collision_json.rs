//! Strict, bounded JSON input for PHB collision authoring.

use serde::de::{value::MapAccessDeserializer, MapAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::Value;

use crate::error::{ErrorKind, FormatError, Result};
use crate::phb_author::{CollisionGeometry, CollisionTriangle};
use crate::zone::Vec3;

pub const MAX_GEOMETRY_JSON_BYTES: usize = 16 * 1024 * 1024;

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
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        struct ObjectVisitor<T>(std::marker::PhantomData<T>);

        impl<'de, T: Deserialize<'de>> Visitor<'de> for ObjectVisitor<T> {
            type Value = ObjectOnly<T>;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("an object")
            }

            fn visit_map<A: MapAccess<'de>>(
                self,
                map: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                T::deserialize(MapAccessDeserializer::new(map)).map(ObjectOnly)
            }
        }

        deserializer.deserialize_map(ObjectVisitor(std::marker::PhantomData))
    }
}

/// Parse the strict collision document. Semantic errors refer to the document root.
pub fn parse_geometry_json(bytes: &[u8]) -> Result<CollisionGeometry> {
    if bytes.len() > MAX_GEOMETRY_JSON_BYTES {
        return Err(FormatError::new(
            ErrorKind::ResourceLimitExceeded,
            0,
            "geometry JSON exceeds the 16 MiB limit",
        ));
    }
    let root: ObjectOnly<GeometryJson> = serde_json::from_slice(bytes).map_err(|error| {
        let line_start = bytes
            .split_inclusive(|byte| *byte == b'\n')
            .take(error.line().saturating_sub(1))
            .map(<[u8]>::len)
            .sum::<usize>();
        let offset = (line_start + error.column().saturating_sub(1)).min(bytes.len());
        FormatError::new(
            ErrorKind::InvalidAttributeValue,
            offset as u64,
            format!("geometry JSON is invalid: {error}"),
        )
    })?;
    parse_geometry_json_inner(root.0)
        .map_err(|detail| FormatError::new(ErrorKind::InvalidAttributeValue, 0, detail))
}

fn parse_geometry_json_inner(root: GeometryJson) -> std::result::Result<CollisionGeometry, String> {
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

fn parse_vertex(value: &Value, index: usize) -> std::result::Result<Vec3, String> {
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
) -> std::result::Result<CollisionTriangle, String> {
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

fn parse_u64(value: &Value, context: &str) -> std::result::Result<u64, String> {
    value
        .as_u64()
        .ok_or_else(|| format!("{context} must be an unsigned integer"))
}

fn parse_u16(value: &Value, context: &str) -> std::result::Result<u16, String> {
    let value = parse_u64(value, context)?;
    u16::try_from(value).map_err(|_| format!("{context} is outside the u16 range"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_geometry() -> &'static [u8] {
        br#"{"schemaVersion":1,"vertices":[[0,0,0],[1,0,0],[0,1,0]],"triangles":[{"indices":[0,1,2],"surface":7}]}"#
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
            .detail()
            .contains("unknown field"));

        let missing_key = br#"{"schemaVersion":1,"vertices":[]}"#;
        assert!(parse_geometry_json(missing_key)
            .unwrap_err()
            .detail()
            .contains("missing field `triangles`"));

        let float_index = br#"{"schemaVersion":1,"vertices":[[0,0,0]],"triangles":[{"indices":[0.0,0,0],"surface":1}]}"#;
        assert!(parse_geometry_json(float_index)
            .unwrap_err()
            .detail()
            .contains("unsigned integer"));

        let unsupported_version = br#"{"schemaVersion":2,"vertices":[],"triangles":[]}"#;
        assert!(parse_geometry_json(unsupported_version)
            .unwrap_err()
            .detail()
            .contains("unsupported geometry schemaVersion"));

        let wrong_dimensions = br#"{"schemaVersion":1,"vertices":[[0,0]],"triangles":[]}"#;
        assert!(parse_geometry_json(wrong_dimensions)
            .unwrap_err()
            .detail()
            .contains("exactly 3 components"));

        let unrepresentable_vertex =
            br#"{"schemaVersion":1,"vertices":[[1e39,0,0]],"triangles":[]}"#;
        assert!(parse_geometry_json(unrepresentable_vertex)
            .unwrap_err()
            .detail()
            .contains("representable as f32"));
    }

    #[test]
    fn geometry_json_rejects_duplicate_root_and_triangle_members() {
        for bytes in [
            br#"{"schemaVersion":2,"schemaVersion":1,"vertices":[],"triangles":[]}"#.as_slice(),
            br#"{"schemaVersion":1,"vertices":[[0,0,0]],"triangles":[{"indices":[0,0,0],"surface":2,"surface":1}]}"#.as_slice(),
            br#"{"schemaVersion":1,"vertices":[[0,0,0]],"triangles":[{"indices":[1,0,0],"indices":[0,0,0],"surface":1}]}"#.as_slice(),
        ] {
            assert!(parse_geometry_json(bytes).unwrap_err().detail().contains("duplicate field"));
        }
    }

    #[test]
    fn geometry_json_requires_objects_for_root_and_triangle_records() {
        for bytes in [
            br#"[1,[[0,0,0],[1,0,0],[0,1,0]],[{"indices":[0,1,2],"surface":7}]]"#.as_slice(),
            br#"{"schemaVersion":1,"vertices":[[0,0,0],[1,0,0],[0,1,0]],"triangles":[[[0,1,2],7]]}"#.as_slice(),
        ] {
            assert!(parse_geometry_json(bytes).unwrap_err().detail().contains("expected an object"));
        }
    }

    #[test]
    fn geometry_json_rejects_out_of_range_triangle_values() {
        let out_of_range_index = br#"{"schemaVersion":1,"vertices":[[0,0,0]],"triangles":[{"indices":[1,0,0],"surface":1}]}"#;
        assert!(parse_geometry_json(out_of_range_index)
            .unwrap_err()
            .detail()
            .contains("outside vertices"));

        let out_of_range_surface = br#"{"schemaVersion":1,"vertices":[[0,0,0]],"triangles":[{"indices":[0,0,0],"surface":65536}]}"#;
        assert!(parse_geometry_json(out_of_range_surface)
            .unwrap_err()
            .detail()
            .contains("u16 range"));
    }

    #[test]
    fn geometry_json_bounds_input_before_parsing_and_retains_syntax_offsets() {
        let error = parse_geometry_json(&vec![b' '; MAX_GEOMETRY_JSON_BYTES + 1]).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::ResourceLimitExceeded);
        assert_eq!(error.offset(), 0);
        let error = parse_geometry_json(b"{\n  !").unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidAttributeValue);
        assert_eq!(error.offset(), 4);
    }
}
