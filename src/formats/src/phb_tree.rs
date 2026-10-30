//! Independent bounded validation and conservative candidate queries for PHB trees.

use crate::error::{ErrorKind, FormatError, Result};
use crate::zone::Vec3;

const GBD: usize = 0xc0;

#[derive(Debug, Clone, Copy)]
struct Box3 {
    min: [f32; 3],
    max: [f32; 3],
}

impl Box3 {
    fn union(self, other: Self) -> Self {
        Self {
            min: std::array::from_fn(|a| self.min[a].min(other.min[a])),
            max: std::array::from_fn(|a| self.max[a].max(other.max[a])),
        }
    }

    fn encloses(self, other: Self, tolerance: f64) -> bool {
        (0..3).all(|a| {
            f64::from(self.min[a]) <= f64::from(other.min[a]) + tolerance
                && f64::from(self.max[a]) + tolerance >= f64::from(other.max[a])
        })
    }
}

#[derive(Debug)]
struct Tree {
    bounds: Box3,
    nodes: Vec<[u16; 8]>,
    references: Vec<u16>,
}

/// Validated structure, retaining tree data for conservative AABB queries.
#[derive(Debug)]
pub struct PhbTreeReport {
    pub vertex_count: usize,
    pub triangle_count: usize,
    pub tbc_count: usize,
    pub node_count: usize,
    pub reference_count: usize,
    trees: Vec<Tree>,
}

impl PhbTreeReport {
    pub fn tree_count(&self) -> usize {
        self.trees.len()
    }

    /// Return triangle IDs in traversal order, without surface filtering.
    /// All registered buckets are visited, so grid selection can add candidates.
    pub fn candidates(&self, min: Vec3, max: Vec3) -> Result<Vec<u16>> {
        let query = Box3 {
            min: xyz(min),
            max: xyz(max),
        };
        check_box(query, 0)?;
        let mut result = Vec::new();
        for tree in &self.trees {
            let (lower, upper) = query_coordinates(query, tree.bounds);
            let mut index = 0;
            while index < tree.nodes.len() {
                let node = tree.nodes[index];
                if (0..3).any(|a| upper[a] < node[a] || lower[a] > node[a + 4]) {
                    index = usize::from(node[3]);
                } else {
                    if node[7] != u16::MAX {
                        let start = usize::from(node[7] & 0x3fff);
                        let count = usize::from((node[7] >> 14) + 1);
                        result.extend_from_slice(&tree.references[start..start + count]);
                    }
                    index += 1;
                }
            }
        }
        Ok(result)
    }
}

fn xyz(v: Vec3) -> [f32; 3] {
    [v.x, v.y, v.z]
}

fn error(at: usize, detail: &str) -> FormatError {
    FormatError::new(ErrorKind::InvalidAttributeValue, at as u64, detail)
}

fn add(a: usize, b: usize, at: usize) -> Result<usize> {
    a.checked_add(b)
        .ok_or_else(|| error(at, "PHB extent overflows"))
}

fn mul(a: usize, b: usize, at: usize) -> Result<usize> {
    a.checked_mul(b)
        .ok_or_else(|| error(at, "PHB count overflows"))
}

fn span(data: &[u8], at: usize, size: usize, end: usize) -> Result<&[u8]> {
    let finish = add(at, size, at)?;
    if finish > end {
        return Err(error(at, "PHB span escapes its containing chunk"));
    }
    data.get(at..finish)
        .ok_or_else(|| error(at, "PHB span is outside input"))
}

fn u32_at(data: &[u8], at: usize) -> Result<usize> {
    Ok(u32::from_le_bytes(span(data, at, 4, data.len())?.try_into().unwrap()) as usize)
}

fn u16_at(data: &[u8], at: usize) -> Result<u16> {
    Ok(u16::from_le_bytes(
        span(data, at, 2, data.len())?.try_into().unwrap(),
    ))
}

fn tag(data: &[u8], at: usize, expected: &[u8; 8]) -> Result<()> {
    if span(data, at, 8, data.len())? != expected {
        return Err(error(at, "unexpected PHB chunk tag"));
    }
    Ok(())
}

fn check_box(bounds: Box3, at: usize) -> Result<()> {
    if (0..3).any(|a| {
        !bounds.min[a].is_finite() || !bounds.max[a].is_finite() || bounds.min[a] > bounds.max[a]
    }) {
        return Err(error(at, "non-finite or inverted PHB bounds"));
    }
    Ok(())
}

fn read_box(data: &[u8], at: usize) -> Result<Box3> {
    let mut bounds = Box3 {
        min: [0.0; 3],
        max: [0.0; 3],
    };
    for a in 0..3 {
        bounds.min[a] = f32::from_bits(u32_at(data, at + a * 4)? as u32);
        bounds.max[a] = f32::from_bits(u32_at(data, at + 16 + a * 4)? as u32);
    }
    check_box(bounds, at)?;
    Ok(bounds)
}

fn tree_box(bounds: Box3, at: usize) -> Result<()> {
    for a in 0..3 {
        let extent = bounds.max[a] - bounds.min[a];
        if !extent.is_finite() || extent <= 0.0 || !(1.0 / extent).is_finite() {
            return Err(error(at, "invalid PHB float extent"));
        }
    }
    Ok(())
}

fn coordinate(position: f32, min: f32, max: f32, bias: f32) -> u16 {
    // Client operation order and biases: docs/formats/phb-collision.md, quantization oracle.
    let delta = position - min;
    let product = delta * 32768.0;
    let scaled = product / (max - min);
    (scaled + bias).clamp(0.0, 65535.5) as u16
}

fn query_coordinates(query: Box3, bounds: Box3) -> ([u16; 3], [u16; 3]) {
    (
        std::array::from_fn(|a| coordinate(query.min[a], bounds.min[a], bounds.max[a], 16383.0)),
        std::array::from_fn(|a| coordinate(query.max[a], bounds.min[a], bounds.max[a], 16385.0)),
    )
}

fn aggregate(data: &[u8], at: usize, surfaces: impl Iterator<Item = u16>) -> Result<()> {
    let word = u32_at(data, at)?;
    let low = word as u16;
    let high = (word >> 16) as u16;
    if low > high || surfaces.into_iter().any(|s| s < low || s > high) {
        return Err(error(
            at,
            "PHB surface aggregate does not enclose references",
        ));
    }
    Ok(())
}

fn disjoint(spans: &mut [(usize, usize)]) -> Result<()> {
    spans.sort_unstable();
    for pair in spans.windows(2) {
        if pair[0].1 > pair[1].0 {
            return Err(error(pair[1].0, "overlapping PHB allocations"));
        }
    }
    Ok(())
}

fn aligned(at: usize, size: usize) -> Result<()> {
    if !at.is_multiple_of(16) || !size.is_multiple_of(16) {
        return Err(error(at, "misaligned PHB allocation"));
    }
    Ok(())
}

/// Re-read a complete PHB, including retail dimension-eight grids and opaque tails.
/// Surface aggregates may be wider than the referenced range. Retail float boxes
/// allow the measured 0.001 world-unit excursion; node coverage uses query biases.
pub fn validate_phb_tree(data: &[u8]) -> Result<PhbTreeReport> {
    tag(data, 0, b"SEDBPHB\0")?;
    tag(data, GBD, b"PHB.GBD\0")?;
    let end = add(GBD, u32_at(data, GBD + 8)?, GBD + 8)?;
    if add(end, 16, GBD + 8)? != data.len() {
        return Err(error(GBD + 8, "invalid PHB payload end or trailer"));
    }
    span(data, GBD, 0x80, end)?;
    let vc = u32_at(data, GBD + 0x3c)?;
    let tc = u32_at(data, GBD + 0x64)?;
    if vc == 0 || vc > 65536 || tc == 0 || tc > 65536 {
        return Err(error(GBD + 0x3c, "geometry exceeds global u16 index space"));
    }
    let vo = add(GBD, u32_at(data, GBD + 0x38)?, GBD + 0x38)?;
    let to = add(GBD, u32_at(data, GBD + 0x60)?, GBD + 0x60)?;
    let vb = mul(vc, 12, GBD + 0x3c)?;
    let tb = mul(tc, 8, GBD + 0x64)?;
    span(data, vo, vb, end)?;
    span(data, to, tb, end)?;
    let mut vertices = Vec::with_capacity(vc);
    for i in 0..vc {
        let at = vo + i * 12;
        let v = std::array::from_fn(|a| f32::from_bits(u32_at(data, at + a * 4).unwrap() as u32));
        if v.iter().any(|v| !v.is_finite()) {
            return Err(error(at, "non-finite PHB vertex"));
        }
        vertices.push(v);
    }
    let mut boxes = Vec::with_capacity(tc);
    let mut surfaces = Vec::with_capacity(tc);
    for i in 0..tc {
        let at = to + i * 8;
        let mut bounds = Box3 {
            min: [f32::INFINITY; 3],
            max: [f32::NEG_INFINITY; 3],
        };
        for lane in 0..3 {
            let index = usize::from(u16_at(data, at + lane * 2)?);
            let v = vertices
                .get(index)
                .ok_or_else(|| error(at + lane * 2, "invalid PHB vertex index"))?;
            bounds = bounds.union(Box3 { min: *v, max: *v });
        }
        boxes.push(bounds);
        surfaces.push(u16_at(data, at + 6)?);
    }
    let gbd_bounds = read_box(data, GBD + 0x40)?;
    tree_box(gbd_bounds, GBD + 0x40)?;
    for a in 0..3 {
        if !(1.0 / (gbd_bounds.max[a] - gbd_bounds.min[a])).is_finite()
            || !(gbd_bounds.max[a] + gbd_bounds.min[a]).is_finite()
        {
            return Err(error(GBD + 0x40, "invalid GBD grid arithmetic"));
        }
    }
    for v in &vertices {
        if !gbd_bounds.encloses(Box3 { min: *v, max: *v }, 0.001) {
            return Err(error(GBD + 0x40, "GBD bounds exclude geometry"));
        }
    }
    aggregate(data, GBD + 0x68, surfaces.iter().copied())?;
    let count = u32_at(data, GBD + 0x34)?;
    if count == 0 || count > 512 {
        return Err(error(GBD + 0x34, "invalid PHB TBC count"));
    }
    let start = add(GBD, u32_at(data, GBD + 0x2c)?, GBD + 0x2c)?;
    let size = u32_at(data, GBD + 0x30)?;
    span(data, start, size, end)?;
    let chain_end = add(start, size, GBD + 0x30)?;
    let mut allocations = vec![
        (GBD, GBD + 0x80),
        (vo, vo + vb),
        (to, to + tb),
        (start, chain_end),
    ];
    let mut tables = Vec::new();
    for field in [0x14, 0x1c, 0x24] {
        let at = add(GBD, u32_at(data, GBD + field)?, GBD + field)?;
        let size = u32_at(data, GBD + field + 4)?;
        span(data, at, size, end)?;
        allocations.push((at, add(at, size, at)?));
        tables.push((at, size));
    }
    disjoint(&mut allocations)?;
    for field in [0x70, 0x74, 0x78] {
        let tail = add(GBD, u32_at(data, GBD + field)?, GBD + field)?;
        if tail < GBD + 0x80 || tail > end {
            return Err(error(GBD + field, "invalid optional tail offset"));
        }
        if tail < end && allocations.iter().any(|&(a, b)| tail >= a && tail < b) {
            return Err(error(GBD + field, "optional tail overlaps core allocation"));
        }
    }
    let dimension = u32_at(data, GBD + 0x10)?;
    let cells = mul(
        mul(dimension, dimension, GBD + 0x10)?,
        dimension,
        GBD + 0x10,
    )?;
    if cells == 0
        || mul(cells, 4, tables[0].0)? > tables[0].1
        || mul(count, 16, tables[2].0)? > tables[2].1
    {
        return Err(error(
            GBD + 0x10,
            "PHB grid or descriptors exceed allocation",
        ));
    }
    let mut listed = vec![false; count];
    let mut visited_halfwords = vec![false; tables[1].1 / 2];
    for cell in 0..cells {
        let field = tables[0].0 + cell * 4;
        let offset = mul(u32_at(data, field)?, 2, field)?;
        let mut at = add(tables[1].0, offset, field)?;
        loop {
            span(data, at, 2, tables[1].0 + tables[1].1)?;
            let slot = (at - tables[1].0) / 2;
            // A previously visited suffix already reached a valid terminator.
            if visited_halfwords[slot] {
                break;
            }
            visited_halfwords[slot] = true;
            let id = u16_at(data, at)?;
            if id == u16::MAX {
                break;
            }
            if usize::from(id) >= count {
                return Err(error(at, "invalid PHB bucket TBC ID"));
            }
            listed[usize::from(id)] = true;
            at += 2;
        }
    }
    if listed.contains(&false) {
        return Err(error(tables[1].0, "bucket lists omit a TBC"));
    }
    let mut report = PhbTreeReport {
        vertex_count: vc,
        triangle_count: tc,
        tbc_count: count,
        node_count: 0,
        reference_count: 0,
        trees: Vec::new(),
    };
    let mut reference_uses = vec![false; tc];
    let mut cursor = start;
    for id in 0..count {
        span(data, cursor, 0x50, chain_end)?;
        tag(data, cursor, b"PHB.TBC\0")?;
        let size = u32_at(data, cursor + 0xc)?;
        aligned(cursor, size)?;
        span(data, cursor, size, chain_end)?;
        let tbc_end = add(cursor, size, cursor + 0xc)?;
        let descriptor = tables[2].0 + id * 16;
        if u32_at(data, descriptor)? != cursor - start || u32_at(data, descriptor + 4)? != size {
            return Err(error(descriptor, "stale PHB TBC descriptor"));
        }
        let bounds = read_box(data, cursor + 0x10)?;
        if !gbd_bounds.encloses(bounds, 0.001) {
            return Err(error(cursor + 0x10, "GBD bounds exclude TBC bounds"));
        }
        let n = u32_at(data, cursor + 0x30)?;
        if n == 0 || n > size / 0x50 {
            return Err(error(cursor + 0x30, "invalid PHB TBD count"));
        }
        let offsets = add(cursor, u32_at(data, cursor + 0x34)?, cursor + 0x34)?;
        let sizes = add(cursor, u32_at(data, cursor + 0x38)?, cursor + 0x38)?;
        let bytes = mul(n, 4, cursor + 0x30)?;
        span(data, offsets, bytes, tbc_end)?;
        span(data, sizes, bytes, tbc_end)?;
        let mut nested = vec![
            (cursor, cursor + 0x50),
            (offsets, offsets + bytes),
            (sizes, sizes + bytes),
        ];
        let mut selected = Vec::new();
        for i in 0..n {
            let at = add(cursor, u32_at(data, offsets + i * 4)?, offsets + i * 4)?;
            let extent = u32_at(data, sizes + i * 4)?;
            span(data, at, extent, tbc_end)?;
            aligned(at, extent)?;
            nested.push((at, add(at, extent, at)?));
            let tree = validate_tree(data, at, extent, &boxes)?;
            if !bounds.encloses(tree.bounds, 0.001) {
                return Err(error(at + 0x20, "TBC bounds exclude TBD bounds"));
            }
            for &r in &tree.references {
                let r = usize::from(r);
                if reference_uses[r] {
                    return Err(error(at, "duplicate PHB triangle reference"));
                }
                reference_uses[r] = true;
                if !bounds.encloses(boxes[r], 0.001) {
                    return Err(error(cursor + 0x10, "TBC bounds exclude geometry"));
                }
                selected.push(surfaces[r]);
            }
            aggregate(
                data,
                at + 0x40,
                tree.references.iter().map(|&r| surfaces[usize::from(r)]),
            )?;
            report.node_count += tree.nodes.len();
            report.reference_count += tree.references.len();
            report.trees.push(tree);
        }
        disjoint(&mut nested)?;
        if nested.last().map(|s| s.1) != Some(tbc_end) {
            return Err(error(cursor, "TBC extent has unconsumed bytes"));
        }
        aggregate(data, cursor + 0x3c, selected.into_iter())?;
        cursor = tbc_end;
    }
    if cursor != chain_end {
        return Err(error(cursor, "TBC chain does not consume extent"));
    }
    if reference_uses.contains(&false) {
        return Err(error(to, "PHB references omit a triangle"));
    }
    Ok(report)
}

fn validate_tree(data: &[u8], at: usize, size: usize, boxes: &[Box3]) -> Result<Tree> {
    let end = add(at, size, at)?;
    span(data, at, 0x50, end)?;
    tag(data, at, b"PHB.TBD\0")?;
    if u32_at(data, at + 8)? != size {
        return Err(error(at + 8, "TBD extent differs from table"));
    }
    let n = u32_at(data, at + 0xc)?;
    let rc = u32_at(data, at + 0x14)?;
    if n == 0 || n > 65535 || rc == 0 || add(n, rc, at + 0xc)? > 65535 {
        return Err(error(at + 0xc, "invalid TBD node/reference count"));
    }
    let nodes_at = add(at, u32_at(data, at + 0x10)?, at + 0x10)?;
    let refs_at = add(at, u32_at(data, at + 0x18)?, at + 0x18)?;
    aligned(nodes_at, 16)?;
    let nb = mul(add(n, 1, at)?, 16, at)?;
    let rb = mul(rc, 2, at)?;
    span(data, nodes_at, nb, end)?;
    span(data, refs_at, rb, end)?;
    disjoint(&mut [
        (at, at + 0x50),
        (nodes_at, nodes_at + nb),
        (refs_at, refs_at + rb),
    ])?;
    let bounds = read_box(data, at + 0x20)?;
    tree_box(bounds, at + 0x20)?;
    let mut references = Vec::with_capacity(rc);
    for i in 0..rc {
        let r = u16_at(data, refs_at + i * 2)?;
        if usize::from(r) >= boxes.len() {
            return Err(error(refs_at + i * 2, "invalid PHB triangle reference"));
        }
        if !bounds.encloses(boxes[usize::from(r)], 0.001) {
            return Err(error(at + 0x20, "TBD bounds exclude geometry"));
        }
        references.push(r);
    }
    let mut nodes = Vec::with_capacity(n);
    for i in 0..=n {
        let pos = nodes_at + i * 16;
        let mut node = [0; 8];
        for (a, lane) in node.iter_mut().enumerate() {
            *lane = u16_at(data, pos + a * 2)?;
        }
        if i == n {
            if node != [65535, 65535, 65535, (n + rc) as u16, 0, 0, 0, 65535] {
                return Err(error(pos, "invalid PHB terminal record"));
            }
        } else {
            nodes.push(node);
        }
    }
    let mut used = vec![false; rc];
    let mut parent_count = vec![0usize; n];
    for (i, node) in nodes.iter().enumerate().rev() {
        let pos = nodes_at + i * 16;
        let skip = usize::from(node[3]);
        if skip <= i || skip > n {
            return Err(error(pos + 6, "backward or past-end PHB skip"));
        }
        if (0..3).any(|a| node[a] > node[a + 4]) {
            return Err(error(pos, "inverted PHB integer bounds"));
        }
        if node[7] == 65535 {
            let first = i + 1;
            if first >= skip {
                return Err(error(pos + 6, "branch lacks children"));
            }
            let second = usize::from(nodes[first][3]);
            if second >= skip || usize::from(nodes[second][3]) != skip {
                return Err(error(pos + 6, "invalid binary PHB child partition"));
            }
            for child in [first, second] {
                parent_count[child] += 1;
                if (0..3).any(|a| node[a] > nodes[child][a] || node[a + 4] < nodes[child][a + 4]) {
                    return Err(error(pos, "parent PHB box excludes child"));
                }
            }
        } else {
            if skip != i + 1 {
                return Err(error(pos + 6, "leaf skip is not consecutive"));
            }
            let start = usize::from(node[7] & 0x3fff);
            let count = usize::from((node[7] >> 14) + 1);
            let finish = add(start, count, pos + 14)?;
            if finish > rc {
                return Err(error(pos + 14, "PHB leaf span exceeds references"));
            }
            for slot in start..finish {
                if used[slot] {
                    return Err(error(pos + 14, "overlapping PHB leaf spans"));
                }
                used[slot] = true;
                let b = boxes[usize::from(references[slot])];
                // Extremal point queries establish coverage under the biased monotone quantizer.
                let (_, low_upper) = query_coordinates(
                    Box3 {
                        min: b.min,
                        max: b.min,
                    },
                    bounds,
                );
                let (high_lower, _) = query_coordinates(
                    Box3 {
                        min: b.max,
                        max: b.max,
                    },
                    bounds,
                );
                if (0..3).any(|a| node[a] > low_upper[a] || node[a + 4] < high_lower[a]) {
                    return Err(error(pos, "PHB leaf excludes triangle query extrema"));
                }
            }
        }
    }
    if usize::from(nodes[0][3]) != n
        || parent_count[0] != 0
        || parent_count[1..].iter().any(|&n| n != 1)
    {
        return Err(error(nodes_at, "unreachable PHB node"));
    }
    if used.contains(&false) {
        return Err(error(refs_at, "unused PHB reference slot"));
    }
    Ok(Tree {
        bounds,
        nodes,
        references,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put32(b: &mut [u8], p: usize, v: u32) {
        b[p..p + 4].copy_from_slice(&v.to_le_bytes());
    }
    fn put16(b: &mut [u8], p: usize, v: u16) {
        b[p..p + 2].copy_from_slice(&v.to_le_bytes());
    }
    fn bounds(b: &mut [u8], p: usize) {
        for a in 0..3 {
            put32(b, p + a * 4, 0f32.to_bits());
            put32(b, p + 16 + a * 4, 2f32.to_bits());
        }
        put32(b, p + 12, 1f32.to_bits());
        put32(b, p + 28, 1f32.to_bits());
    }
    fn v(x: f32, y: f32, z: f32) -> Vec3 {
        Vec3 { x, y, z }
    }

    // Literal independent layout: GBD 0xc0, TBC 0x2c0, TBD 0x330, nodes 0x380.
    fn oracle() -> Vec<u8> {
        let mut b = vec![0; 0x5d0];
        b[..8].copy_from_slice(b"SEDBPHB\0");
        put32(&mut b, 8, 0x120);
        put16(&mut b, 0xc, 0x400);
        put16(&mut b, 0xe, 0x48);
        put32(&mut b, 0x10, 0x48);
        b[0xc0..0xc8].copy_from_slice(b"PHB.GBD\0");
        for (p, w) in [
            (8, 0x500),
            (0xc, 0x212),
            (0x10, 1),
            (0x14, 0x80),
            (0x18, 0x80),
            (0x1c, 0x100),
            (0x20, 0x80),
            (0x24, 0x180),
            (0x28, 0x80),
            (0x2c, 0x200),
            (0x30, 0x110),
            (0x34, 1),
            (0x38, 0x400),
            (0x3c, 12),
            (0x60, 0x380),
            (0x64, 4),
            (0x70, 0x500),
            (0x74, 0x500),
            (0x78, 0x500),
        ] {
            put32(&mut b, 0xc0 + p, w);
        }
        bounds(&mut b, 0x100);
        put16(&mut b, 0x1c0, 0);
        put16(&mut b, 0x1c2, 65535);
        put32(&mut b, 0x244, 0x110);
        b[0x2c0..0x2c8].copy_from_slice(b"PHB.TBC\0");
        for (p, w) in [
            (8, 0x100),
            (0xc, 0x110),
            (0x30, 1),
            (0x34, 0x50),
            (0x38, 0x60),
            (0x50, 0x70),
            (0x60, 0xa0),
        ] {
            put32(&mut b, 0x2c0 + p, w);
        }
        bounds(&mut b, 0x2d0);
        b[0x330..0x338].copy_from_slice(b"PHB.TBD\0");
        for (p, w) in [(8, 0xa0), (0xc, 3), (0x10, 0x50), (0x14, 4), (0x18, 0x90)] {
            put32(&mut b, 0x330 + p, w);
        }
        bounds(&mut b, 0x350);
        let nodes = [
            [0x4000, 0x4000, 0x4000, 3, 0xc000, 0xc000, 0xc000, 0xffff],
            [0x4000, 0x4000, 0x4000, 2, 0x8000, 0xc000, 0xc000, 0x4000],
            [0x8000, 0x4000, 0x4000, 3, 0xc000, 0xc000, 0xc000, 0x4002],
            [65535, 65535, 65535, 7, 0, 0, 0, 65535],
        ];
        for (i, node) in nodes.iter().enumerate() {
            for (a, &w) in node.iter().enumerate() {
                put16(&mut b, 0x380 + i * 16 + a * 2, w);
            }
        }
        for i in 0..4 {
            put16(&mut b, 0x3c0 + i * 2, i as u16);
        }
        let vertices = [
            [0., 0., 0.],
            [1., 0., 0.],
            [1., 2., 2.],
            [0., 0., 0.],
            [1., 2., 2.],
            [0., 2., 2.],
            [1., 0., 0.],
            [2., 0., 0.],
            [2., 2., 2.],
            [1., 0., 0.],
            [2., 2., 2.],
            [1., 2., 2.],
        ];
        for (i, vertex) in vertices.iter().enumerate() {
            for (a, &w) in vertex.iter().enumerate() {
                put32(&mut b, 0x4c0 + i * 12 + a * 4, f32::to_bits(w));
            }
        }
        for i in 0..4 {
            for a in 0..3 {
                put16(&mut b, 0x440 + i * 8 + a * 2, (i * 3 + a) as u16);
            }
        }
        b
    }

    #[test]
    fn literal_two_leaf_queries_and_reference_identity() {
        let mut b = oracle();
        let report = validate_phb_tree(&b).unwrap();
        assert_eq!(
            (
                report.node_count,
                report.reference_count,
                report.tree_count()
            ),
            (3, 4, 1)
        );
        for (min, max, expected) in [
            (v(0.2, 0.5, 0.5), v(0.8, 1.5, 1.5), vec![0, 1]),
            (v(1.2, 0.5, 0.5), v(1.8, 1.5, 1.5), vec![2, 3]),
            (v(1., 0.5, 0.5), v(1., 1.5, 1.5), vec![0, 1, 2, 3]),
            (v(3., 0., 0.), v(4., 1., 1.), vec![]),
        ] {
            assert_eq!(report.candidates(min, max).unwrap(), expected);
        }
        // Swap identities within a leaf without moving triangle storage.
        put16(&mut b, 0x3c0, 1);
        put16(&mut b, 0x3c2, 0);
        assert_eq!(
            validate_phb_tree(&b)
                .unwrap()
                .candidates(v(0.2, 0.5, 0.5), v(0.8, 1.5, 1.5))
                .unwrap(),
            [1, 0]
        );
        assert_eq!(&b[0x440..0x460], &oracle()[0x440..0x460]);
    }

    #[test]
    fn bucket_lists_with_many_overlapping_suffixes_are_bounded() {
        let original = oracle();
        let cells = 64usize.pow(3);
        let grid_bytes = cells * 4;
        let list_bytes = (cells * 2 + 2 + 15) & !15;
        let list_at = 0x140 + grid_bytes;
        let shift = grid_bytes + list_bytes - 0x100;
        let mut b = vec![0; original.len() + shift];
        b[..0x140].copy_from_slice(&original[..0x140]);
        b[0x240 + shift..].copy_from_slice(&original[0x240..]);
        for (field, value) in [
            (8, 0x500 + shift),
            (0x10, 64),
            (0x18, grid_bytes),
            (0x1c, list_at - GBD),
            (0x20, list_bytes),
            (0x24, 0x180 + shift),
            (0x2c, 0x200 + shift),
            (0x38, 0x400 + shift),
            (0x60, 0x380 + shift),
            (0x70, 0x500 + shift),
            (0x74, 0x500 + shift),
            (0x78, 0x500 + shift),
        ] {
            put32(&mut b, GBD + field, value as u32);
        }
        for cell in 0..cells {
            put32(&mut b, 0x140 + cell * 4, cell as u32);
        }
        put16(&mut b, list_at + cells * 2, u16::MAX);
        let report = validate_phb_tree(&b).unwrap();
        assert_eq!((report.tbc_count, report.reference_count), (1, 4));
        assert_eq!(
            report.candidates(v(0., 0., 0.), v(2., 2., 2.)).unwrap(),
            [0, 1, 2, 3]
        );

        put16(&mut b, list_at + cells * 2, 0);
        let e = validate_phb_tree(&b).unwrap_err();
        assert_eq!(e.kind(), ErrorKind::InvalidAttributeValue);
        assert_eq!(e.offset(), (list_at + list_bytes) as u64);
        assert_eq!(e.detail(), "PHB span escapes its containing chunk");
    }

    #[test]
    fn validator_checks_tbc_box_inside_gbd_with_tolerance() {
        for (at, tolerated, excluded) in
            [(0x2d8, -0.0005f32, -0.002f32), (0x2e8, 2.0005f32, 2.002f32)]
        {
            let mut b = oracle();
            put32(&mut b, at, tolerated.to_bits());
            validate_phb_tree(&b).unwrap();
            put32(&mut b, at, excluded.to_bits());
            let e = validate_phb_tree(&b).unwrap_err();
            assert_eq!(e.kind(), ErrorKind::InvalidAttributeValue);
            assert_eq!(e.offset(), 0x2d0);
            assert_eq!(e.detail(), "GBD bounds exclude TBC bounds");
        }
    }

    #[test]
    fn validator_checks_tbd_box_inside_tbc_with_tolerance() {
        for (at, tolerated, excluded) in
            [(0x358, -0.0005f32, -0.002f32), (0x368, 2.0005f32, 2.002f32)]
        {
            let mut b = oracle();
            put32(&mut b, at, tolerated.to_bits());
            validate_phb_tree(&b).unwrap();
            put32(&mut b, at, excluded.to_bits());
            let e = validate_phb_tree(&b).unwrap_err();
            assert_eq!(e.kind(), ErrorKind::InvalidAttributeValue);
            assert_eq!(e.offset(), 0x350);
            assert_eq!(e.detail(), "TBC bounds exclude TBD bounds");
        }
    }

    #[test]
    fn validator_rejects_missing_triangle_with_consistent_terminal() {
        let mut b = oracle();
        put32(&mut b, 0x344, 3);
        put16(&mut b, 0x3b6, 6);
        put16(&mut b, 0x3ae, 2);
        let e = validate_phb_tree(&b).unwrap_err();
        assert_eq!(e.kind(), ErrorKind::InvalidAttributeValue);
        assert_eq!(e.offset(), 0x440);
        assert_eq!(e.detail(), "PHB references omit a triangle");
    }

    #[test]
    fn validator_rejects_duplicate_triangle_with_consistent_terminal() {
        let mut b = oracle();
        put16(&mut b, 0x3c2, 0);
        let e = validate_phb_tree(&b).unwrap_err();
        assert_eq!(e.kind(), ErrorKind::InvalidAttributeValue);
        assert_eq!(e.offset(), 0x330);
        assert_eq!(e.detail(), "duplicate PHB triangle reference");
    }

    #[test]
    fn validator_rejects_bounded_structural_mutations() {
        let cases = [
            (0x386, 0, 2, 0x386),
            (0x386, 4, 2, 0x386),                 // backward and past-end skip
            (0x33c, 4, 4, 0x3c0), // declared fourth node consumes terminal allocation
            (0x344, 3, 4, 0x3b0), // terminal count mismatch
            (0x3c0, 4, 2, 0x3c0), // triangle reference
            (0x440, 12, 2, 0x440), // vertex index
            (0x380, 0xc001, 2, 0x380), // inverted integer box
            (0x360, (-1f32).to_bits(), 4, 0x350), // inverted float box
            (0x338, u32::MAX, 4, 0x338), // overflowing chunk extent
            (0x340, 0x52, 4, 0x382), // misaligned nodes
            (0xf0, u32::MAX, 4, 0x2c0), // overflowing TBC chain extent
        ];
        for (at, value, width, expected_offset) in cases {
            let mut b = oracle();
            if width == 2 {
                put16(&mut b, at, value as u16);
            } else {
                put32(&mut b, at, value);
            }
            let e = validate_phb_tree(&b).unwrap_err();
            assert_eq!(
                e.kind(),
                ErrorKind::InvalidAttributeValue,
                "mutation {at:x}"
            );
            assert_eq!(e.offset(), expected_offset, "mutation {at:x}: {e}");
        }
        let mut b = oracle();
        b[0x3b0..0x3c0].fill(0);
        assert_eq!(validate_phb_tree(&b).unwrap_err().offset(), 0x3b0);
        for cut in [0, 7, 0xc0, 0x330, 0x3b8, 0x3c6, 0x5cf] {
            assert!(validate_phb_tree(&oracle()[..cut]).is_err());
        }
    }

    #[test]
    fn retail_enclosure_accepts_wider_aggregates_and_biased_nodes() {
        let mut b = oracle();
        for at in [0x128, 0x2fc, 0x370] {
            put32(&mut b, at, 0xffff0000);
        }
        put16(&mut b, 0x398, 0x7fff);
        let report = validate_phb_tree(&b).unwrap();
        assert_eq!(
            report.candidates(v(1., 1., 1.), v(1., 1., 1.)).unwrap(),
            [0, 1, 2, 3]
        );
        put16(&mut b, 0x398, 0x7ffe);
        assert_eq!(validate_phb_tree(&b).unwrap_err().offset(), 0x390);
    }

    #[test]
    fn retail_dimension_eight_grid_and_opaque_optional_tails() {
        let original = oracle();
        let mut b = vec![0; 0xd70];
        b[..0x1c0].copy_from_slice(&original[..0x1c0]);
        b[0x940..0xd40].copy_from_slice(&original[0x1c0..0x5c0]);
        b[0xd40..0xd60].fill(0xa5);
        b[0xd60..].copy_from_slice(&original[0x5c0..]);
        for (field, value) in [
            (8, 0xca0),
            (0x10, 8),
            (0x18, 0x800),
            (0x1c, 0x880),
            (0x24, 0x900),
            (0x2c, 0x980),
            (0x38, 0xb80),
            (0x60, 0xb00),
            (0x6c, 1),
            (0x70, 0xc80),
            (0x74, 0xc90),
            (0x78, 0xca0),
        ] {
            put32(&mut b, GBD + field, value);
        }
        let report = validate_phb_tree(&b).unwrap();
        assert_eq!((report.tbc_count, report.reference_count), (1, 4));
        assert_eq!(
            report.candidates(v(0., 0., 0.), v(2., 2., 2.)).unwrap(),
            [0, 1, 2, 3]
        );
    }

    #[test]
    fn query_quantization_endpoints_biases_and_clamps() {
        let b = Box3 {
            min: [0.; 3],
            max: [2.; 3],
        };
        assert_eq!(
            query_coordinates(
                Box3 {
                    min: [0.; 3],
                    max: [2.; 3]
                },
                b
            ),
            ([16383; 3], [49153; 3])
        );
        assert_eq!(
            query_coordinates(
                Box3 {
                    min: [-100.; 3],
                    max: [-50.; 3]
                },
                b
            ),
            ([0; 3], [0; 3])
        );
        assert_eq!(
            query_coordinates(
                Box3 {
                    min: [50.; 3],
                    max: [100.; 3]
                },
                b
            ),
            ([65535; 3], [65535; 3])
        );
        let report = validate_phb_tree(&oracle()).unwrap();
        assert_eq!(
            report.candidates(v(0., 0., 0.), v(0., 0., 0.)).unwrap(),
            [0, 1]
        );
    }

    #[test]
    fn deep_unbalanced_tree_skip_map_and_iterative_termination() {
        let nodes = vec![
            [0x4000, 0x4000, 0x4000, 7, 0xc000, 0xc000, 0xc000, 65535],
            [0x4000, 0x4000, 0x4000, 2, 0x5000, 0xc000, 0xc000, 0],
            [0x6000, 0x4000, 0x4000, 7, 0xc000, 0xc000, 0xc000, 65535],
            [0x6000, 0x4000, 0x4000, 4, 0x7000, 0xc000, 0xc000, 1],
            [0x8000, 0x4000, 0x4000, 7, 0xc000, 0xc000, 0xc000, 65535],
            [0x8000, 0x4000, 0x4000, 6, 0x9000, 0xc000, 0xc000, 2],
            [0xa000, 0x4000, 0x4000, 7, 0xc000, 0xc000, 0xc000, 3],
        ];
        let mut bytes = vec![0; 0xe0];
        bytes[..8].copy_from_slice(b"PHB.TBD\0");
        for (p, w) in [(8, 0xe0), (0xc, 7), (0x10, 0x50), (0x14, 4), (0x18, 0xd0)] {
            put32(&mut bytes, p, w);
        }
        bounds(&mut bytes, 0x20);
        for (i, node) in nodes
            .iter()
            .chain(std::iter::once(&[65535, 65535, 65535, 11, 0, 0, 0, 65535]))
            .enumerate()
        {
            for (a, &v) in node.iter().enumerate() {
                put16(&mut bytes, 0x50 + i * 16 + a * 2, v);
            }
        }
        for i in 0..4 {
            put16(&mut bytes, 0xd0 + i * 2, i as u16);
        }
        let boxes = [0.125, 0.625, 1.125, 1.625].map(|x| Box3 {
            min: [x, 0.5, 0.5],
            max: [x, 1.5, 1.5],
        });
        let tree = validate_tree(&bytes, 0, bytes.len(), &boxes).unwrap();
        assert_eq!(tree.nodes, nodes);
        assert_eq!(
            tree.nodes.iter().map(|n| n[3]).collect::<Vec<_>>(),
            [7, 2, 7, 4, 7, 6, 7]
        );
        let report = PhbTreeReport {
            vertex_count: 0,
            triangle_count: 4,
            tbc_count: 1,
            node_count: 7,
            reference_count: 4,
            trees: vec![tree],
        };
        assert_eq!(
            report
                .candidates(v(-10., -10., -10.), v(10., 10., 10.))
                .unwrap(),
            [0, 1, 2, 3]
        );
        assert_eq!(
            report
                .candidates(v(0.1, 0.5, 0.5), v(0.2, 1.5, 1.5))
                .unwrap(),
            [0]
        );
        assert_eq!(
            report
                .candidates(v(0.5, 0.5, 0.5), v(0.75, 1.5, 1.5))
                .unwrap(),
            [1]
        );
        assert_eq!(u16_at(&bytes, 0xc6).unwrap(), 11);
        assert_eq!(u16_at(&bytes, 0xce).unwrap(), 65535);

        let mut deep = vec![0; 0x4450];
        deep[..8].copy_from_slice(b"PHB.TBD\0");
        for (p, w) in [
            (8, 0x4450),
            (0xc, 1023),
            (0x10, 0x50),
            (0x14, 512),
            (0x18, 0x4050),
        ] {
            put32(&mut deep, p, w);
        }
        bounds(&mut deep, 0x20);
        for i in 0..1023 {
            let branch = i % 2 == 0 && i < 1021;
            let skip = if branch { 1023 } else { i + 1 };
            let word = if branch { 65535 } else { (i / 2) as u16 };
            let node = [
                0x4000,
                0x4000,
                0x4000,
                skip as u16,
                0xc000,
                0xc000,
                0xc000,
                word,
            ];
            for (a, v) in node.into_iter().enumerate() {
                put16(&mut deep, 0x50 + i * 16 + a * 2, v);
            }
        }
        for (a, v) in [65535, 65535, 65535, 1535, 0, 0, 0, 65535]
            .into_iter()
            .enumerate()
        {
            put16(&mut deep, 0x4040 + a * 2, v);
        }
        for i in 0..512 {
            put16(&mut deep, 0x4050 + i * 2, i as u16);
        }
        let boxes = vec![
            Box3 {
                min: [0.5; 3],
                max: [1.5; 3]
            };
            512
        ];
        let tree = validate_tree(&deep, 0, deep.len(), &boxes).unwrap();
        let report = PhbTreeReport {
            vertex_count: 0,
            triangle_count: 512,
            tbc_count: 1,
            node_count: 1023,
            reference_count: 512,
            trees: vec![tree],
        };
        assert_eq!(
            report.candidates(v(0., 0., 0.), v(2., 2., 2.)).unwrap(),
            (0..512).collect::<Vec<u16>>()
        );
    }
}
