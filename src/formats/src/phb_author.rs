//! Experimental template-based PHB collision authoring.
//!
//! The supported profile and retail consumer locators are documented in
//! `docs/formats/phb-collision.md`. Client walking acceptance is separate.

use crate::error::{ErrorKind, FormatError, Result};
use crate::zone::Vec3;

const GBD: usize = 0xc0;
const TBC: usize = GBD + 0x200;
const TBD: usize = GBD + 0x270;
const NODE: usize = TBD + 0x50;
const REFS: usize = TBD + 0x70;
const TRIANGLES: usize = GBD + 0x300;
const VERTICES: usize = GBD + 0x380;
const MAX_TEMPLATE_BYTES: usize = 64 * 1024 * 1024;

/// Author a retained-allocation single-leaf PHB. Unsupported templates fail
/// before any bytes are changed. Unresolved bytes and all offsets are retained.
pub fn author_phb(template: &[u8], geometry: &CollisionGeometry) -> Result<Vec<u8>> {
    let profile = SingleLeaf::parse(template)?;
    if geometry.triangles.len() > 4 || geometry.vertices.len() > profile.vertex_capacity {
        return Err(FormatError::new(
            ErrorKind::ResourceLimitExceeded,
            0,
            "single-leaf profile permits at most four triangles and the retained vertex capacity",
        ));
    }
    let (min, max) = geometry_bounds(geometry)?;
    let (min, max) = enclosing_bounds(min, max, profile.padding)?;
    let triangle_count = geometry.triangles.len() as u16;
    let mut output = template.to_vec();
    put_u32(&mut output, GBD + 0x3c, geometry.vertices.len() as u32);
    put_u32(&mut output, GBD + 0x64, u32::from(triangle_count));
    put_u32(&mut output, TBD + 0x14, u32::from(triangle_count));
    let mut surface_min = u16::MAX;
    let mut surface_max = 0;
    for (index, triangle) in geometry.triangles.iter().enumerate() {
        surface_min = surface_min.min(triangle.surface);
        surface_max = surface_max.max(triangle.surface);
        for (lane, value) in triangle
            .indices
            .into_iter()
            .chain([triangle.surface])
            .enumerate()
        {
            put_u16(&mut output, TRIANGLES + index * 8 + lane * 2, value);
        }
        put_u16(&mut output, REFS + index * 2, index as u16);
    }
    let aggregate = u32::from(surface_min) | (u32::from(surface_max) << 16);
    for at in [GBD + 0x68, TBC + 0x3c, TBD + 0x40] {
        put_u32(&mut output, at, aggregate);
    }
    for (index, vertex) in geometry.vertices.iter().enumerate() {
        for (axis, value) in components(*vertex).into_iter().enumerate() {
            put_u32(
                &mut output,
                VERTICES + index * 12 + axis * 4,
                value.to_bits(),
            );
        }
    }
    for offset in [GBD + 0x40, TBC + 0x10, TBD + 0x20] {
        put_bounds(&mut output, offset, min, max);
    }
    // Unsigned packed endpoints cover the complete TBD box, not just the
    // template's original plane. The single leaf references every triangle.
    for axis in 0..3 {
        put_u16(&mut output, NODE + axis * 2, 0x4000);
        put_u16(&mut output, NODE + 8 + axis * 2, 0xc000);
    }
    put_u16(&mut output, NODE + 14, (triangle_count - 1) << 14);
    put_u16(&mut output, NODE + 16 + 6, triangle_count + 1);
    crate::zone::parse_phb(&output)?;
    Ok(output)
}

struct SingleLeaf {
    vertex_capacity: usize,
    padding: Vec3,
}

impl SingleLeaf {
    fn parse(template: &[u8]) -> Result<Self> {
        if template.len() > MAX_TEMPLATE_BYTES {
            return Err(FormatError::new(
                ErrorKind::ResourceLimitExceeded,
                0,
                "PHB template exceeds 64 MiB",
            ));
        }
        for (offset, expected) in [
            (8, 0x120),
            (0x10, 0x48),
            (GBD + 0xc, 0x212),
            (GBD + 0x10, 1),
            (GBD + 0x14, 0x80),
            (GBD + 0x18, 0x80),
            (GBD + 0x1c, 0x100),
            (GBD + 0x20, 0x80),
            (GBD + 0x24, 0x180),
            (GBD + 0x28, 0x80),
            (GBD + 0x2c, 0x200),
            (GBD + 0x34, 1),
            (GBD + 0x6c, 0),
            (GBD + 0x7c, 0),
        ] {
            require_u32(template, offset, expected)?;
        }
        if read_u16(template, 0xc)? != 0x400 || read_u16(template, 0xe)? != 0x48 {
            return Err(author_error(0xc, "unsupported SEDB PHB header profile"));
        }
        let payload_end = read_u32(template, GBD + 8)? as usize;
        require_u32(template, GBD + 8, 0x400)?;
        for offset in [0x70, 0x74, 0x78] {
            require_u32(template, GBD + offset, payload_end as u32)?;
        }
        for (offset, expected) in [
            (GBD + 0x30, 0xf0),
            (GBD + 0x38, 0x380),
            (GBD + 0x60, 0x300),
            (GBD + 0x180, 0),
            (GBD + 0x184, 0xf0),
            (TBC + 8, 0x100),
            (TBC + 0xc, 0xf0),
            (TBC + 0x30, 1),
            (TBC + 0x34, 0x50),
            (TBC + 0x38, 0x60),
            (TBC + 0x50, 0x70),
            (TBC + 0x60, 0x80),
            (TBD + 8, 0x80),
            (TBD + 0xc, 1),
            (TBD + 0x10, 0x50),
            (TBD + 0x18, 0x70),
            (TBD + 0x1c, 0),
        ] {
            require_u32(template, offset, expected)?;
        }
        for (offset, tag) in [(TBC, b"PHB.TBC\0"), (TBD, b"PHB.TBD\0")] {
            if template.get(offset..offset + 8) != Some(tag.as_slice()) {
                return Err(author_error(offset as u64, "unsupported PHB nested tag"));
            }
        }
        let triangle_count = read_u32(template, GBD + 0x64)? as usize;
        if triangle_count == 0 || triangle_count > 4 {
            return Err(author_error(
                GBD as u64 + 0x64,
                "unsupported single-leaf triangle count",
            ));
        }
        let vertex_capacity = (payload_end - 0x380) / 12;
        let vertex_count = read_u32(template, GBD + 0x3c)? as usize;
        if vertex_count == 0 || vertex_count > vertex_capacity {
            return Err(author_error(
                GBD as u64 + 0x3c,
                "template vertices exceed the retained allocation",
            ));
        }
        let phb = crate::zone::parse_phb(template)?;
        let hull = &phb.hulls[0];
        require_u32(template, TBD + 0x14, triangle_count as u32)?;
        if read_u16(template, NODE + 6)? != 1
            || read_u16(template, NODE + 14)? != ((triangle_count as u16 - 1) << 14)
            || read_u16(template, NODE + 16 + 6)? != triangle_count as u16 + 1
            || read_u16(template, NODE + 16 + 14)? != u16::MAX
        {
            return Err(author_error(
                NODE as u64,
                "unsupported PHB leaf or terminal record",
            ));
        }
        let mut references = Vec::with_capacity(triangle_count);
        for index in 0..triangle_count {
            references.push(read_u16(template, REFS + index * 2)?);
        }
        references.sort_unstable();
        if references.iter().copied().ne(0..triangle_count as u16) {
            return Err(author_error(
                REFS as u64,
                "PHB leaf references must cover each triangle exactly once",
            ));
        }
        let min_surface = *hull
            .surfaces
            .iter()
            .min()
            .expect("nonempty template triangles");
        let max_surface = *hull
            .surfaces
            .iter()
            .max()
            .expect("nonempty template triangles");
        for at in [GBD + 0x68, TBC + 0x3c, TBD + 0x40] {
            require_u32(template, at, min_surface | (max_surface << 16))?;
        }
        let source_bounds = (hull.decoded_min, hull.decoded_max);
        for offset in [GBD + 0x40, TBC + 0x10, TBD + 0x20] {
            let (min, max) = read_bounds(template, offset)?;
            require_u32(template, offset + 12, 1.0f32.to_bits())?;
            require_u32(template, offset + 28, 1.0f32.to_bits())?;
            for axis in 0..3 {
                if components(min)[axis] > components(max)[axis] {
                    return Err(author_error(
                        offset as u64,
                        "PHB template bounds are inverted",
                    ));
                }
            }
        }
        let (tbc_min, tbc_max) = read_bounds(template, TBC + 0x10)?;
        let mut padding = [0.0; 3];
        for (axis, pad) in padding.iter_mut().enumerate() {
            let lower = components(source_bounds.0)[axis] - components(tbc_min)[axis];
            let upper = components(tbc_max)[axis] - components(source_bounds.1)[axis];
            if !lower.is_finite() || !upper.is_finite() || lower < 0.0 || upper < 0.0 {
                return Err(author_error(
                    TBC as u64 + 0x10,
                    "TBC bounds do not enclose template geometry",
                ));
            }
            *pad = lower.max(upper);
            if *pad <= 0.0 {
                return Err(author_error(
                    TBC as u64 + 0x10,
                    "TBC profile requires positive bounds padding on each axis",
                ));
            }
        }
        Ok(Self {
            vertex_capacity,
            padding: Vec3 {
                x: padding[0],
                y: padding[1],
                z: padding[2],
            },
        })
    }
}

fn components(vertex: Vec3) -> [f32; 3] {
    [vertex.x, vertex.y, vertex.z]
}

fn enclosing_bounds(min: Vec3, max: Vec3, padding: Vec3) -> Result<(Vec3, Vec3)> {
    let mut lower = [0.0; 3];
    let mut upper = [0.0; 3];
    for axis in 0..3 {
        // Round away from the geometry when a template margin is below one ULP.
        lower[axis] = (components(min)[axis] - components(padding)[axis])
            .min(components(min)[axis].next_down());
        upper[axis] = (components(max)[axis] + components(padding)[axis])
            .max(components(max)[axis].next_up());
        let width = upper[axis] - lower[axis];
        let sum = upper[axis] + lower[axis];
        if !lower[axis].is_finite()
            || !upper[axis].is_finite()
            || !width.is_finite()
            || !sum.is_finite()
            || width <= 0.0
            || !(1.0 / width).is_finite()
        {
            return Err(author_error(
                0,
                "collision bounds are not representable by finite retail f32 box arithmetic",
            ));
        }
    }
    Ok((
        Vec3 {
            x: lower[0],
            y: lower[1],
            z: lower[2],
        },
        Vec3 {
            x: upper[0],
            y: upper[1],
            z: upper[2],
        },
    ))
}

fn read_bounds(data: &[u8], offset: usize) -> Result<(Vec3, Vec3)> {
    let mut values = [0.0; 6];
    for (index, at) in [
        offset,
        offset + 4,
        offset + 8,
        offset + 16,
        offset + 20,
        offset + 24,
    ]
    .into_iter()
    .enumerate()
    {
        values[index] = f32::from_bits(read_u32(data, at)?);
        if !values[index].is_finite() {
            return Err(author_error(at as u64, "PHB bounds are not finite"));
        }
    }
    Ok((
        Vec3 {
            x: values[0],
            y: values[1],
            z: values[2],
        },
        Vec3 {
            x: values[3],
            y: values[4],
            z: values[5],
        },
    ))
}

fn put_u32(data: &mut [u8], at: usize, value: u32) {
    data[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

fn put_u16(data: &mut [u8], at: usize, value: u16) {
    data[at..at + 2].copy_from_slice(&value.to_le_bytes());
}

fn put_bounds(data: &mut [u8], at: usize, min: Vec3, max: Vec3) {
    for (relative, value) in [
        (0, min.x),
        (4, min.y),
        (8, min.z),
        (16, max.x),
        (20, max.y),
        (24, max.z),
    ] {
        put_u32(data, at + relative, value.to_bits());
    }
}

fn require_u32(data: &[u8], offset: usize, expected: u32) -> Result<()> {
    if read_u32(data, offset)? != expected {
        return Err(author_error(
            offset as u64,
            "unsupported PHB template field",
        ));
    }
    Ok(())
}

fn read_u32(data: &[u8], offset: usize) -> Result<u32> {
    let end = offset
        .checked_add(4)
        .ok_or_else(|| author_error(offset as u64, "PHB field end overflows"))?;
    let bytes = data
        .get(offset..end)
        .ok_or_else(|| author_error(offset as u64, "PHB field is outside input"))?;
    Ok(u32::from_le_bytes(
        bytes.try_into().expect("validated four-byte slice"),
    ))
}

fn read_u16(data: &[u8], offset: usize) -> Result<u16> {
    let end = offset
        .checked_add(2)
        .ok_or_else(|| author_error(offset as u64, "PHB field end overflows"))?;
    let bytes = data
        .get(offset..end)
        .ok_or_else(|| author_error(offset as u64, "PHB field is outside input"))?;
    Ok(u16::from_le_bytes(
        bytes.try_into().expect("validated two-byte slice"),
    ))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollisionTriangle {
    /// Zero-based vertex indices. Order and winding are retained.
    pub indices: [u16; 3],
    /// Raw retail surface word, without an inferred material classification.
    pub surface: u16,
}

/// Coordinates are local to the PHB resource, in the caller's Y-up scene units.
#[derive(Debug, Clone, PartialEq)]
pub struct CollisionGeometry {
    pub vertices: Vec<Vec3>,
    pub triangles: Vec<CollisionTriangle>,
}

fn geometry_bounds(geometry: &CollisionGeometry) -> Result<(Vec3, Vec3)> {
    if geometry.vertices.is_empty() || geometry.triangles.is_empty() {
        return Err(author_error(
            0,
            "collision geometry requires vertices and triangles",
        ));
    }
    if geometry.vertices.len() > u16::MAX as usize + 1 {
        return Err(FormatError::new(
            ErrorKind::ResourceLimitExceeded,
            0,
            "collision vertices exceed the u16 index space",
        ));
    }
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
    for (index, vertex) in geometry.vertices.iter().enumerate() {
        if !vertex.x.is_finite() || !vertex.y.is_finite() || !vertex.z.is_finite() {
            return Err(author_error(
                index as u64 * 12,
                "collision vertex is not finite",
            ));
        }
        min.x = min.x.min(vertex.x);
        min.y = min.y.min(vertex.y);
        min.z = min.z.min(vertex.z);
        max.x = max.x.max(vertex.x);
        max.y = max.y.max(vertex.y);
        max.z = max.z.max(vertex.z);
    }
    for (index, triangle) in geometry.triangles.iter().enumerate() {
        let [a, b, c] = triangle.indices.map(usize::from);
        if a >= geometry.vertices.len()
            || b >= geometry.vertices.len()
            || c >= geometry.vertices.len()
        {
            return Err(author_error(
                index as u64 * 8,
                "collision triangle index is outside vertices",
            ));
        }
        let a = geometry.vertices[a];
        let b = geometry.vertices[b];
        let c = geometry.vertices[c];
        let ab = [
            f64::from(b.x) - f64::from(a.x),
            f64::from(b.y) - f64::from(a.y),
            f64::from(b.z) - f64::from(a.z),
        ];
        let ac = [
            f64::from(c.x) - f64::from(a.x),
            f64::from(c.y) - f64::from(a.y),
            f64::from(c.z) - f64::from(a.z),
        ];
        let cross = [
            ab[1] * ac[2] - ab[2] * ac[1],
            ab[2] * ac[0] - ab[0] * ac[2],
            ab[0] * ac[1] - ab[1] * ac[0],
        ];
        if cross == [0.0; 3] {
            return Err(author_error(
                index as u64 * 8,
                "collision triangle has zero area",
            ));
        }
    }
    Ok((min, max))
}

fn author_error(offset: u64, detail: impl Into<String>) -> FormatError {
    FormatError::new(ErrorKind::InvalidAttributeValue, offset, detail)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vertex(x: f32, y: f32, z: f32) -> Vec3 {
        Vec3 { x, y, z }
    }

    fn quad(vertices: [Vec3; 4], surfaces: [u16; 2]) -> CollisionGeometry {
        CollisionGeometry {
            vertices: vertices.into(),
            triangles: vec![
                CollisionTriangle {
                    indices: [0, 1, 2],
                    surface: surfaces[0],
                },
                CollisionTriangle {
                    indices: [0, 2, 3],
                    surface: surfaces[1],
                },
            ],
        }
    }

    fn floor() -> CollisionGeometry {
        quad(
            [
                vertex(-4.0, 0.0, -3.0),
                vertex(4.0, 0.0, -3.0),
                vertex(4.0, 0.0, 3.0),
                vertex(-4.0, 0.0, 3.0),
            ],
            [0, 7],
        )
    }

    fn wall() -> CollisionGeometry {
        quad(
            [
                vertex(0.0, 0.0, -2.0),
                vertex(0.0, 3.0, -2.0),
                vertex(0.0, 3.0, 2.0),
                vertex(0.0, 0.0, 2.0),
            ],
            [4, 5],
        )
    }

    fn stairs() -> CollisionGeometry {
        let mut geometry = quad(
            [
                vertex(-1.0, 0.5, 0.0),
                vertex(1.0, 0.5, 0.0),
                vertex(1.0, 0.5, 1.0),
                vertex(-1.0, 0.5, 1.0),
            ],
            [1, 2],
        );
        geometry.vertices.extend([
            vertex(-1.0, 1.0, 1.0),
            vertex(1.0, 1.0, 1.0),
            vertex(1.0, 1.0, 2.0),
            vertex(-1.0, 1.0, 2.0),
        ]);
        geometry.triangles.extend([
            CollisionTriangle {
                indices: [4, 5, 6],
                surface: 3,
            },
            CollisionTriangle {
                indices: [4, 6, 7],
                surface: 6,
            },
        ]);
        geometry
    }

    fn put_u32(data: &mut [u8], at: usize, value: u32) {
        data[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn put_u16(data: &mut [u8], at: usize, value: u16) {
        data[at..at + 2].copy_from_slice(&value.to_le_bytes());
    }

    fn put_bounds(data: &mut [u8], at: usize, min: Vec3, max: Vec3) {
        for (relative, value) in [
            (0, min.x),
            (4, min.y),
            (8, min.z),
            (16, max.x),
            (20, max.y),
            (24, max.z),
        ] {
            put_u32(data, at + relative, value.to_bits());
        }
    }

    fn template() -> Vec<u8> {
        let mut data = vec![0; GBD + 0x400 + 16];
        data[..8].copy_from_slice(b"SEDBPHB\0");
        put_u32(&mut data, 8, 0x120);
        put_u16(&mut data, 0xc, 0x400);
        put_u16(&mut data, 0xe, 0x48);
        put_u32(&mut data, 0x10, 0x48);
        data[0x20..0x24].copy_from_slice(b"KEEP");
        data[GBD..GBD + 8].copy_from_slice(b"PHB.GBD\0");
        for (at, value) in [
            (8, 0x400),
            (0xc, 0x212),
            (0x10, 1),
            (0x14, 0x80),
            (0x18, 0x80),
            (0x1c, 0x100),
            (0x20, 0x80),
            (0x24, 0x180),
            (0x28, 0x80),
            (0x2c, 0x200),
            (0x30, 0xf0),
            (0x34, 1),
            (0x38, 0x380),
            (0x3c, 4),
            (0x60, 0x300),
            (0x64, 2),
            (0x70, 0x400),
            (0x74, 0x400),
            (0x78, 0x400),
        ] {
            put_u32(&mut data, GBD + at, value);
        }
        for at in [0x4c, 0x5c, 0x21c, 0x22c, 0x29c, 0x2ac] {
            put_u32(&mut data, GBD + at, 1.0f32.to_bits());
        }
        put_bounds(
            &mut data,
            GBD + 0x40,
            vertex(0.0, 0.0, -0.00005),
            vertex(2.0, 2.0, 0.00005),
        );
        put_u32(&mut data, GBD + 0x100, 0xffff0000);
        put_u32(&mut data, GBD + 0x184, 0xf0);
        data[GBD + 0x200..GBD + 0x208].copy_from_slice(b"PHB.TBC\0");
        for (at, value) in [
            (0x208, 0x100),
            (0x20c, 0xf0),
            (0x230, 1),
            (0x234, 0x50),
            (0x238, 0x60),
            (0x250, 0x70),
            (0x260, 0x80),
        ] {
            put_u32(&mut data, GBD + at, value);
        }
        put_bounds(
            &mut data,
            GBD + 0x210,
            vertex(-0.01, -0.01, -0.01),
            vertex(2.01, 2.01, 0.01),
        );
        data[GBD + 0x270..GBD + 0x278].copy_from_slice(b"PHB.TBD\0");
        for (at, value) in [
            (0x278, 0x80),
            (0x27c, 1),
            (0x280, 0x50),
            (0x284, 2),
            (0x288, 0x70),
        ] {
            put_u32(&mut data, GBD + at, value);
        }
        put_bounds(
            &mut data,
            GBD + 0x290,
            vertex(0.0, 0.0, -0.005),
            vertex(2.0, 2.0, 0.005),
        );
        for (index, value) in [
            0x4000, 0x4000, 0x8000, 1, 0xc000, 0xc000, 0x8000, 0x4000, 0xffff, 0xffff, 0xffff, 3,
            0, 0, 0, 0xffff, 0, 1,
        ]
        .into_iter()
        .enumerate()
        {
            put_u16(&mut data, GBD + 0x2c0 + index * 2, value);
        }
        let geometry = quad(
            [
                vertex(0.0, 0.0, 0.0),
                vertex(2.0, 0.0, 0.0),
                vertex(2.0, 2.0, 0.0),
                vertex(0.0, 2.0, 0.0),
            ],
            [0, 0],
        );
        for (index, position) in geometry.vertices.iter().enumerate() {
            for (axis, value) in [position.x, position.y, position.z].into_iter().enumerate() {
                put_u32(
                    &mut data,
                    GBD + 0x380 + index * 12 + axis * 4,
                    value.to_bits(),
                );
            }
        }
        for (index, triangle) in geometry.triangles.iter().enumerate() {
            for (lane, value) in triangle
                .indices
                .into_iter()
                .chain([triangle.surface])
                .enumerate()
            {
                put_u16(&mut data, GBD + 0x300 + index * 8 + lane * 2, value);
            }
        }
        data[GBD + 0x2f0..GBD + 0x300].fill(0xa5);
        data[GBD + 0x400..].copy_from_slice(b"OPAQUE_PHB_TAIL_");
        data
    }

    #[test]
    fn geometry_rejects_nonfinite_indices_and_zero_area() {
        let mut geometry = floor();
        geometry.vertices[1].x = f32::NAN;
        assert!(geometry_bounds(&geometry)
            .unwrap_err()
            .detail()
            .contains("not finite"));
        geometry = floor();
        geometry.triangles[0].indices[2] = u16::MAX;
        assert!(geometry_bounds(&geometry)
            .unwrap_err()
            .detail()
            .contains("outside vertices"));
        geometry = floor();
        geometry.triangles[0].indices = [0, 1, 1];
        assert!(geometry_bounds(&geometry)
            .unwrap_err()
            .detail()
            .contains("zero area"));
        geometry.triangles.clear();
        assert!(geometry_bounds(&geometry)
            .unwrap_err()
            .detail()
            .contains("requires vertices"));
    }

    #[test]
    fn recognizes_single_leaf_and_rejects_stale_references_and_counts() {
        let source = template();
        assert_eq!(SingleLeaf::parse(&source).unwrap().vertex_capacity, 10);
        for (at, value) in [
            (GBD + 0x10, 8),
            (GBD + 0x6c, 1),
            (GBD + 0x38, 0x300),
            (TBD + 0xc, 7),
            (TBD + 0x14, 4),
            (GBD + 0x68, 0x10001),
        ] {
            let mut malformed = source.clone();
            put_u32(&mut malformed, at, value);
            assert!(
                SingleLeaf::parse(&malformed).is_err(),
                "accepted changed field at {at:#x}"
            );
        }
        let mut duplicate = source.clone();
        put_u16(&mut duplicate, REFS + 2, 0);
        assert!(SingleLeaf::parse(&duplicate).is_err());
        let mut stale_leaf = source.clone();
        put_u16(&mut stale_leaf, NODE + 14, 0xc000);
        assert!(SingleLeaf::parse(&stale_leaf).is_err());
        for cut in [0, 1, 48, 192, 320, 1000, source.len() - 1] {
            assert!(SingleLeaf::parse(&source[..cut]).is_err());
        }
    }

    #[test]
    fn synthetic_floor_wall_and_stair_bounds_are_local_geometry() {
        assert_eq!(
            geometry_bounds(&floor()).unwrap(),
            (vertex(-4.0, 0.0, -3.0), vertex(4.0, 0.0, 3.0))
        );
        assert_eq!(
            geometry_bounds(&wall()).unwrap(),
            (vertex(0.0, 0.0, -2.0), vertex(0.0, 3.0, 2.0))
        );
        assert_eq!(
            geometry_bounds(&stairs()).unwrap(),
            (vertex(-1.0, 0.5, 0.0), vertex(1.0, 1.0, 2.0))
        );
    }

    #[test]
    fn serializes_floor_wall_and_stairs_with_exact_geometry_and_surfaces() {
        for geometry in [floor(), wall(), stairs()] {
            let source = template();
            let original = source.clone();
            let output = author_phb(&source, &geometry).unwrap();
            assert_eq!(source, original);
            assert_eq!(output.len(), source.len());
            let parsed = crate::zone::parse_phb(&output).unwrap();
            let hull = &parsed.hulls[0];
            assert_eq!(hull.vertices, geometry.vertices);
            assert_eq!(
                hull.indices,
                geometry
                    .triangles
                    .iter()
                    .flat_map(|triangle| triangle.indices.map(u32::from))
                    .collect::<Vec<_>>()
            );
            assert_eq!(
                hull.surfaces,
                geometry
                    .triangles
                    .iter()
                    .map(|triangle| u32::from(triangle.surface))
                    .collect::<Vec<_>>()
            );
            assert_eq!(
                (hull.decoded_min, hull.decoded_max),
                geometry_bounds(&geometry).unwrap()
            );
            let declared = read_bounds(&output, GBD + 0x40).unwrap();
            assert_eq!(read_bounds(&output, TBC + 0x10).unwrap(), declared);
            assert_eq!(read_bounds(&output, TBD + 0x20).unwrap(), declared);
            let mut covered = Vec::new();
            let word = read_u16(&output, NODE + 14).unwrap();
            let leaf_count = (word >> 14) + 1;
            let leaf_start = word & 0x3fff;
            assert_eq!(usize::from(leaf_count), geometry.triangles.len());
            for index in leaf_start..leaf_start + leaf_count {
                covered.push(read_u16(&output, REFS + usize::from(index) * 2).unwrap());
            }
            assert_eq!(
                covered,
                (0..geometry.triangles.len() as u16).collect::<Vec<_>>()
            );
            assert_eq!(read_u16(&output, NODE + 6).unwrap(), 1);
            assert_eq!(read_u16(&output, NODE + 22).unwrap(), leaf_count + 1);
            for axis in 0..3 {
                let min = f64::from(components(declared.0)[axis]);
                let max = f64::from(components(declared.1)[axis]);
                let decode = |at| {
                    min + (f64::from(read_u16(&output, at).unwrap()) - 16384.0) / 32768.0
                        * (max - min)
                };
                let node_min = decode(NODE + axis * 2);
                let node_max = decode(NODE + 8 + axis * 2);
                for vertex in &geometry.vertices {
                    let value = f64::from(components(*vertex)[axis]);
                    assert!(node_min <= value && value <= node_max);
                }
            }
            let surface_min = *hull.surfaces.iter().min().unwrap();
            let surface_max = *hull.surfaces.iter().max().unwrap();
            assert_eq!(
                read_u32(&output, GBD + 0x68).unwrap(),
                surface_min | (surface_max << 16)
            );
            assert_eq!(
                read_u32(&output, TBC + 0x3c).unwrap(),
                surface_min | (surface_max << 16)
            );
            assert_eq!(
                read_u32(&output, TBD + 0x40).unwrap(),
                surface_min | (surface_max << 16)
            );
            assert_eq!(SingleLeaf::parse(&output).unwrap().vertex_capacity, 10);
        }
    }

    #[test]
    fn preserves_every_byte_outside_owned_fields_and_used_arrays() {
        let source = template();
        let geometry = stairs();
        let output = author_phb(&source, &geometry).unwrap();
        let mut changed_ranges = vec![
            GBD + 0x3c..GBD + 0x40,
            GBD + 0x64..GBD + 0x6c,
            TBD + 0x14..TBD + 0x18,
            TBC + 0x3c..TBC + 0x40,
            TBD + 0x40..TBD + 0x44,
            NODE..NODE + 6,
            NODE + 8..NODE + 16,
            NODE + 22..NODE + 24,
            REFS..REFS + geometry.triangles.len() * 2,
            TRIANGLES..TRIANGLES + geometry.triangles.len() * 8,
            VERTICES..VERTICES + geometry.vertices.len() * 12,
        ];
        for at in [GBD + 0x40, TBC + 0x10, TBD + 0x20] {
            changed_ranges.extend([at..at + 12, at + 16..at + 28]);
        }
        for (offset, (before, after)) in source.iter().zip(&output).enumerate() {
            if !changed_ranges.iter().any(|range| range.contains(&offset)) {
                assert_eq!(before, after, "changed unresolved byte at {offset:#x}");
            }
        }
    }

    #[test]
    fn handles_one_and_three_triangle_leaves_and_full_raw_surface_words() {
        let mut geometry = floor();
        geometry.triangles.truncate(1);
        geometry.triangles[0].surface = u16::MAX;
        let output = author_phb(&template(), &geometry).unwrap();
        assert_eq!(read_u16(&output, NODE + 14).unwrap(), 0);
        assert_eq!(read_u32(&output, GBD + 0x68).unwrap(), u32::MAX);
        assert_eq!(
            crate::zone::parse_phb(&output).unwrap().hulls[0].surfaces,
            [65535]
        );
        geometry = stairs();
        geometry.triangles.truncate(3);
        let output = author_phb(&template(), &geometry).unwrap();
        assert_eq!(read_u16(&output, NODE + 14).unwrap(), 0x8000);
        assert_eq!(read_u16(&output, NODE + 22).unwrap(), 4);
    }

    #[test]
    fn rejects_capacity_overflow_and_unrepresentable_bounds() {
        let source = template();
        let mut geometry = stairs();
        geometry.vertices.extend([Vec3::ZERO; 3]);
        assert_eq!(
            author_phb(&source, &geometry).unwrap_err().kind(),
            ErrorKind::ResourceLimitExceeded
        );
        geometry = stairs();
        geometry.triangles.push(geometry.triangles[0].clone());
        assert_eq!(
            author_phb(&source, &geometry).unwrap_err().kind(),
            ErrorKind::ResourceLimitExceeded
        );
        geometry = wall();
        for vertex in &mut geometry.vertices {
            vertex.x = 3.3e38;
        }
        assert!(author_phb(&source, &geometry)
            .unwrap_err()
            .detail()
            .contains("f32 box arithmetic"));
        let mut malformed = source.clone();
        put_u32(&mut malformed, TBD + 0x20, f32::NAN.to_bits());
        assert!(author_phb(&malformed, &floor())
            .unwrap_err()
            .detail()
            .contains("not finite"));
        put_u32(&mut malformed, GBD + 0x3c, 1_000_000);
        assert!(author_phb(&malformed, &floor()).is_err());
    }
}
