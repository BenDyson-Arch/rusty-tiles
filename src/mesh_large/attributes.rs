//! Bounded full vertex-attribute capture. Images must be captured separately before the
//! caller releases source ownership. At most six accessor windows are live in the join;
//! callers must additionally charge the typed sorter runs and merge buffers.
use super::{
    capture::{canonical, source_position, triangle_bytes, vertices, SourceScan},
    input::{ElementReader, StreamSource},
    records::{Bounds, CornerPosition, CornerRef, Cube, Provenance, TriangleRecord},
    source_crs::{Jacobian, SourceFrame},
};
use crate::{
    gltf_source::source::ScenePrimitive,
    spill::{ExternalSorter, Layout, Record, Scratch, SortConfig, SortedStream},
    JobError, JobErrorKind,
};
use std::cmp::Ordering;
type Result<T> = std::result::Result<T, JobError>;
fn invalid(message: impl Into<String>) -> JobError {
    JobError::new(JobErrorKind::InvalidInput, message)
}
fn invariant(message: &'static str) -> JobError {
    JobError::new(JobErrorKind::InvalidState, message)
}
fn overflow() -> JobError {
    JobError::new(
        JobErrorKind::ResourceLimit,
        "full attribute capture count or byte sum overflow",
    )
}
pub(crate) const NORMAL: u8 = 1;
pub(crate) const TANGENT: u8 = 2;
pub(crate) const UV0: u8 = 4;
pub(crate) const UV1: u8 = 8;
pub(crate) const COLOR: u8 = 16;
/// Every attribute flag; a layout's other bits are ignored.
const ALL: u8 = NORMAL | TANGENT | UV0 | UV1 | COLOR;
/// The run-level attribute layout of [`FullTriangle`] and [`FullCorner`]: the union of the
/// selected scene's primitive attribute flags. An attribute no primitive carries is the
/// default (zero) in every record, so the run's sorts and sequences omit it; decode restores
/// it, and the records consumers read are unchanged.
pub(crate) fn attribute_layout(source: &StreamSource) -> Layout {
    Layout(
        source
            .document
            .scene_primitives()
            .fold(0, |union, p| union | flags(&p)),
    )
}
/// Encoded bytes of one corner's attributes under `layout`.
pub(crate) fn corner_attribute_bytes(layout: Layout) -> usize {
    let bits = layout.0;
    [(NORMAL, 12), (TANGENT, 16), (UV0, 8), (UV1, 8), (COLOR, 16)]
        .iter()
        .filter(|(flag, _)| bits & flag != 0)
        .map(|(_, bytes)| bytes)
        .sum()
}
fn flags(p: &ScenePrimitive<'_>) -> u8 {
    (u8::from(p.normal.is_some()) * NORMAL)
        | (u8::from(p.tangent.is_some()) * TANGENT)
        | (u8::from(p.texcoords[0].is_some()) * UV0)
        | (u8::from(p.texcoords[1].is_some()) * UV1)
        | (u8::from(p.color.is_some()) * COLOR)
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct CornerAttributes {
    pub normal: [f32; 3],
    pub tangent: [f32; 4],
    pub texcoords: [[f32; 2]; 2],
    pub color: [f32; 4],
}
impl CornerAttributes {
    /// The fields `layout` keeps, in wire order (normal, tangent, UV0, UV1, color).
    fn fields(&self, layout: Layout) -> impl Iterator<Item = &f32> {
        fn keep(layout: Layout, flag: u8, values: &[f32]) -> std::slice::Iter<'_, f32> {
            let kept: &[f32] = if layout.0 & flag != 0 { values } else { &[] };
            kept.iter()
        }
        let keep = |flag, values| keep(layout, flag, values);
        keep(NORMAL, &self.normal)
            .chain(keep(TANGENT, &self.tangent))
            .chain(keep(UV0, &self.texcoords[0]))
            .chain(keep(UV1, &self.texcoords[1]))
            .chain(keep(COLOR, &self.color))
    }
    /// Writes the fields `layout` keeps (`out.len() == corner_attribute_bytes(layout)`); the
    /// omitted fields must be zero, which capture guarantees by flag.
    fn encode(&self, layout: Layout, out: &mut [u8]) {
        // Decode restores omitted fields as +0.0: any other bits would not round-trip.
        debug_assert!(
            self.fields(Layout(!layout.0)).all(|v| v.to_bits() == 0),
            "attribute layout omits a nonzero field"
        );
        for (dst, v) in out
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .zip(self.fields(layout))
        {
            dst.copy_from_slice(&v.to_le_bytes());
        }
    }
    fn decode(layout: Layout, bytes: &[u8]) -> Self {
        let mut values = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f32::from_le_bytes(*b));
        let mut read = |flag: u8, out: &mut [f32]| {
            if layout.0 & flag != 0 {
                for v in out {
                    *v = values.next().unwrap();
                }
            }
        };
        let mut out = Self::default();
        read(NORMAL, &mut out.normal);
        read(TANGENT, &mut out.tangent);
        read(UV0, &mut out.texcoords[0]);
        read(UV1, &mut out.texcoords[1]);
        read(COLOR, &mut out.color);
        out
    }
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct FullTriangle {
    pub triangle: TriangleRecord,
    pub expanded_ordinal: u64,
    /// u32::MAX means no material.
    pub material: u32,
    pub flags: u8,
    pub corners: [CornerAttributes; 3],
}
impl PartialEq for FullTriangle {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}
impl Eq for FullTriangle {}
impl PartialOrd for FullTriangle {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for FullTriangle {
    fn cmp(&self, other: &Self) -> Ordering {
        (
            self.triangle.key,
            self.triangle.source,
            self.expanded_ordinal,
        )
            .cmp(&(
                other.triangle.key,
                other.triangle.source,
                other.expanded_ordinal,
            ))
    }
}
/// The triangle, expanded ordinal, material and flags; three corners' attributes follow.
const FULL_TRIANGLE_HEAD: usize = 106;
impl Record for FullTriangle {
    const SIZE: usize = 286;
    fn encode(&self, out: &mut [u8]) {
        self.encode_in(Layout::FULL, out);
    }
    fn decode(bytes: &[u8]) -> Self {
        Self::decode_in(Layout::FULL, bytes)
    }
    fn size_in(layout: Layout) -> usize {
        FULL_TRIANGLE_HEAD + 3 * corner_attribute_bytes(layout)
    }
    fn encode_in(&self, layout: Layout, out: &mut [u8]) {
        debug_assert_eq!(self.flags & ALL & !layout.0, 0);
        self.triangle.encode(&mut out[..93]);
        out[93..101].copy_from_slice(&self.expanded_ordinal.to_le_bytes());
        out[101..105].copy_from_slice(&self.material.to_le_bytes());
        out[105] = self.flags;
        let width = corner_attribute_bytes(layout);
        for (a, b) in self
            .corners
            .iter()
            .zip(out[FULL_TRIANGLE_HEAD..].chunks_exact_mut(width.max(1)))
        {
            a.encode(layout, b);
        }
    }
    fn decode_in(layout: Layout, bytes: &[u8]) -> Self {
        let width = corner_attribute_bytes(layout);
        Self {
            triangle: TriangleRecord::decode(&bytes[..93]),
            expanded_ordinal: u64::from_le_bytes(bytes[93..101].try_into().unwrap()),
            material: u32::from_le_bytes(bytes[101..105].try_into().unwrap()),
            flags: bytes[105],
            corners: std::array::from_fn(|i| {
                let start = FULL_TRIANGLE_HEAD + i * width;
                CornerAttributes::decode(layout, &bytes[start..start + width])
            }),
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct FullCorner {
    pub corner: CornerPosition,
    pub attributes: CornerAttributes,
}
impl PartialEq for FullCorner {
    fn eq(&self, other: &Self) -> bool {
        self.corner == other.corner
    }
}
impl Eq for FullCorner {}
impl PartialOrd for FullCorner {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for FullCorner {
    fn cmp(&self, other: &Self) -> Ordering {
        self.corner.cmp(&other.corner)
    }
}
impl Record for FullCorner {
    const SIZE: usize = 97;
    fn encode(&self, out: &mut [u8]) {
        self.encode_in(Layout::FULL, out);
    }
    fn decode(bytes: &[u8]) -> Self {
        Self::decode_in(Layout::FULL, bytes)
    }
    fn size_in(layout: Layout) -> usize {
        37 + corner_attribute_bytes(layout)
    }
    fn encode_in(&self, layout: Layout, out: &mut [u8]) {
        self.corner.encode(&mut out[..37]);
        self.attributes.encode(layout, &mut out[37..]);
    }
    fn decode_in(layout: Layout, bytes: &[u8]) -> Self {
        Self {
            corner: CornerPosition::decode(&bytes[..37]),
            attributes: CornerAttributes::decode(layout, &bytes[37..]),
        }
    }
}
fn normalize(mapped: [f64; 3], role: &str) -> Result<[f32; 3]> {
    let norm = mapped.into_iter().map(|x| x * x).sum::<f64>().sqrt();
    if !norm.is_finite() || norm == 0. {
        return Err(JobError::new(
            JobErrorKind::Unsupported,
            format!("{role} transform outside numeric domain"),
        ));
    }
    Ok(mapped.map(|v| (v / norm) as f32))
}
fn read_attributes(
    p: &ScenePrimitive<'_>,
    vertex: u64,
    jacobian: Option<&Jacobian>,
    normals: &mut Option<ElementReader<'_>>,
    tangents: &mut Option<ElementReader<'_>>,
    uvs: &mut [Option<ElementReader<'_>>; 2],
    colors: &mut Option<ElementReader<'_>>,
) -> Result<CornerAttributes> {
    let vertex = usize::try_from(vertex).map_err(|_| overflow())?;
    let mut out = CornerAttributes::default();
    // With a source CRS, the f64 unit normal that a mapped tangent is orthogonalized against.
    let mut unit_normal = None;
    if let Some(reader) = normals {
        let n = reader.vec3(vertex)?;
        let mut mapped: [f64; 3] =
            std::array::from_fn(|r| (0..3).map(|c| p.normal_matrix[r][c] * n[c]).sum());
        if let Some(jacobian) = jacobian {
            mapped = jacobian.normal(mapped)?;
            let norm = mapped.iter().map(|v| v * v).sum::<f64>().sqrt();
            unit_normal = Some(mapped.map(|v| v / norm));
        }
        out.normal = normalize(mapped, "normal")?;
    }
    if let Some(reader) = tangents {
        let t = reader.values(vertex)?;
        let mut mapped = std::array::from_fn(|r| (0..3).map(|c| p.world[r][c] * t[c]).sum());
        let mut handedness = p.determinant.signum();
        if let Some(jacobian) = jacobian {
            mapped = jacobian.tangent(mapped);
            handedness *= jacobian.handedness();
            // A CRS map is not conformal: restore orthogonality to the mapped normal
            // (Gram-Schmidt) before normalizing; the handedness sign is kept.
            if let Some(n) = unit_normal {
                let along = (0..3).map(|i| mapped[i] * n[i]).sum::<f64>();
                mapped = std::array::from_fn(|i| mapped[i] - along * n[i]);
            }
        }
        let xyz = normalize(mapped, "tangent")?;
        out.tangent = [xyz[0], xyz[1], xyz[2], (t[3] * handedness) as f32];
    }
    for (reader, uv) in uvs.iter_mut().zip(&mut out.texcoords) {
        if let Some(reader) = reader {
            let v = reader.values(vertex)?;
            *uv = [v[0] as f32, v[1] as f32];
        }
    }
    if let Some(reader) = colors {
        let c = reader.values(vertex)?;
        out.color = [
            c[0] as f32,
            c[1] as f32,
            c[2] as f32,
            if p.color.as_ref().unwrap().width == 3 {
                1.
            } else {
                c[3] as f32
            },
        ];
    }
    Ok(out)
}
/// Captures all selected geometry and vertex attributes, retaining every triangle and
/// degenerate. The returned stream reads only scratch; image bytes are not captured here.
pub(crate) fn capture_full<'s>(
    source: &StreamSource,
    scratch: &'s Scratch,
    sort: SortConfig,
    scan: &SourceScan,
    check: &mut dyn FnMut() -> Result<()>,
) -> Result<SortedStream<'s, FullTriangle>> {
    let mut scan = *scan;
    capture_full_in(
        source,
        &mut SourceFrame::Local,
        scratch,
        sort,
        attribute_layout(source),
        &mut scan,
        check,
    )
}
/// [`capture_full`] in a resolved frame. A source CRS frame checks the used post-node source
/// bounds against the scan's and replaces `scan.bounds` with the mapped box before the cube
/// keys are assigned; the scan never maps positions. Both sorts spill under `layout`, which
/// must hold every selected primitive's attributes ([`attribute_layout`]).
pub(crate) fn capture_full_in<'s>(
    source: &StreamSource,
    frame: &mut SourceFrame,
    scratch: &'s Scratch,
    sort: SortConfig,
    layout: Layout,
    scan: &mut SourceScan,
    check: &mut dyn FnMut() -> Result<()>,
) -> Result<SortedStream<'s, FullTriangle>> {
    source.validate_payload(sort.buffer_bytes, check)?;
    // Only a primitive's own attributes are ever nonzero, so a layout holding them all drops
    // nothing but zeros. Checked once per primitive, never per record.
    if source
        .document
        .scene_primitives()
        .any(|p| flags(&p) & !layout.0 != 0)
    {
        return Err(invariant(
            "attribute layout omits a selected primitive's attribute",
        ));
    }
    let mut corners = ExternalSorter::<CornerRef>::new(scratch, sort)?;
    for (ordinal, p) in source.document.scene_primitives().enumerate() {
        check()?;
        let ordinal = u64::try_from(ordinal).map_err(|_| overflow())?;
        let mut indices = p.indices.map(|v| source.reader(&v, sort.buffer_bytes));
        for triangle in 0..p.triangles {
            if triangle.is_multiple_of(1024) {
                check()?;
            }
            for (corner, vertex) in vertices(&p, &mut indices, triangle)?
                .into_iter()
                .enumerate()
            {
                corners.push(CornerRef {
                    primitive: ordinal,
                    vertex,
                    triangle: u64::try_from(triangle).map_err(|_| overflow())?,
                    corner: corner as u8,
                })?;
            }
        }
    }
    let mut corners = corners.finish()?;
    let mut next = corners.next().transpose()?;
    let mut positioned = ExternalSorter::<FullCorner>::with_layout(scratch, sort, layout)?;
    let mut mapped_bounds = Bounds::EMPTY;
    let mut source_bounds = [[f64::INFINITY; 3], [f64::NEG_INFINITY; 3]];
    for (ordinal, p) in source.document.scene_primitives().enumerate() {
        let ordinal = u64::try_from(ordinal).map_err(|_| overflow())?;
        let mut positions = source.reader(&p.position, sort.buffer_bytes);
        let mut normals = p.normal.map(|v| source.reader(&v, sort.buffer_bytes));
        let mut tangents = p.tangent.map(|v| source.reader(&v, sort.buffer_bytes));
        let mut uvs = p
            .texcoords
            .map(|v| v.map(|v| source.reader(&v, sort.buffer_bytes)));
        let mut colors = p.color.map(|v| source.reader(&v, sort.buffer_bytes));
        let mut cached = None;
        let mut records = 0u64;
        while let Some(corner) = next.filter(|c| c.primitive == ordinal) {
            if records.is_multiple_of(3072) {
                check()?;
            }
            let (position, attributes) =
                if let Some((_, value)) = cached.filter(|(v, _)| *v == corner.vertex) {
                    value
                } else {
                    let world = source_position(&p, &mut positions, corner.vertex)?;
                    let (position, jacobian) =
                        frame.vertex(world, normals.is_some() || tangents.is_some())?;
                    let position = canonical(position);
                    mapped_bounds.include(position);
                    for axis in 0..3 {
                        source_bounds[0][axis] = source_bounds[0][axis].min(world[axis]);
                        source_bounds[1][axis] = source_bounds[1][axis].max(world[axis]);
                    }
                    let attributes = read_attributes(
                        &p,
                        corner.vertex,
                        jacobian.as_ref(),
                        &mut normals,
                        &mut tangents,
                        &mut uvs,
                        &mut colors,
                    )?;
                    cached = Some((corner.vertex, (position, attributes)));
                    (position, attributes)
                };
            positioned.push(FullCorner {
                corner: CornerPosition {
                    primitive: corner.primitive,
                    triangle: corner.triangle,
                    corner: corner.corner,
                    vertex: corner.vertex,
                    position,
                },
                attributes,
            })?;
            records = records.checked_add(1).ok_or_else(overflow)?;
            next = corners.next().transpose()?;
        }
        if records
            != u64::try_from(p.triangles)
                .map_err(|_| overflow())?
                .checked_mul(3)
                .ok_or_else(overflow)?
        {
            return Err(invariant(
                "corner reference stream disagrees with admitted primitive",
            ));
        }
    }
    if next.is_some() {
        return Err(invariant("unmatched corner reference after POSITION join"));
    }
    drop(corners);
    if let Some(expected) = frame.source_bounds() {
        if source_bounds != expected {
            return Err(invalid(
                "mesh source geometry changed between preflight and capture",
            ));
        }
        scan.bounds = mapped_bounds;
    }
    let mut positioned = positioned.finish()?;
    let mut triangles = ExternalSorter::<FullTriangle>::with_layout(scratch, sort, layout)?;
    let cube = Cube::around(&scan.bounds);
    let mut count = 0u64;
    let mut bounds = Bounds::EMPTY;
    let mut geometry_bytes = 0u64;
    for (ordinal, p) in source.document.scene_primitives().enumerate() {
        let ordinal = u64::try_from(ordinal).map_err(|_| overflow())?;
        let bytes = triangle_bytes(&p);
        for triangle in 0..p.triangles {
            if triangle.is_multiple_of(1024) {
                check()?;
            }
            let mut positions = [[0.; 3]; 3];
            let mut vertices = [0; 3];
            let mut attributes = [CornerAttributes::default(); 3];
            for corner in 0..3 {
                let value = positioned
                    .next()
                    .transpose()?
                    .ok_or_else(|| invariant("missing positioned corner"))?;
                if (
                    value.corner.primitive,
                    value.corner.triangle,
                    value.corner.corner,
                ) != (ordinal, triangle as u64, corner as u8)
                {
                    return Err(invariant(
                        "positioned corner stream order disagrees with provenance",
                    ));
                }
                positions[corner] = value.corner.position;
                vertices[corner] = value.corner.vertex;
                bounds.include(value.corner.position);
                attributes[corner] = value.attributes;
            }
            let degenerate = positions[0] == positions[1]
                || positions[0] == positions[2]
                || positions[1] == positions[2];
            triangles.push(FullTriangle {
                triangle: TriangleRecord {
                    key: cube.key(&positions),
                    source: Provenance {
                        node: p.node,
                        mesh: p.mesh,
                        primitive: p.primitive,
                        triangle: triangle as u64,
                    },
                    vertices,
                    positions,
                    bytes,
                    degenerate,
                },
                expanded_ordinal: ordinal,
                material: p.material.unwrap_or(u32::MAX),
                flags: flags(&p),
                corners: attributes,
            })?;
            count = count.checked_add(1).ok_or_else(overflow)?;
            geometry_bytes = geometry_bytes
                .checked_add(u64::from(bytes))
                .ok_or_else(overflow)?;
        }
    }
    if positioned.next().transpose()?.is_some() {
        return Err(invariant("extra positioned corner"));
    }
    drop(positioned);
    source.verify_unchanged()?;
    if count != scan.triangles || geometry_bytes != scan.geometry_bytes || bounds != scan.bounds {
        return Err(invalid(
            "mesh source geometry changed between preflight and capture",
        ));
    }
    Ok(triangles.finish()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;

    fn wire<T: Record>(row: &T) -> Vec<u8> {
        let mut bytes = vec![0; T::SIZE];
        row.encode(&mut bytes);
        bytes
    }

    #[test]
    fn full_wire_preserves_attributes_and_wide_identity() {
        let large = u64::from(u32::MAX) + 17;
        let attributes = CornerAttributes {
            normal: [-0., 0.25, 1.],
            tangent: [1., 0., 0., -1.],
            texcoords: [[-2., 7.], [0.25, 0.75]],
            color: [0., 0.5, 1., 1.],
        };
        let triangle = FullTriangle {
            triangle: TriangleRecord {
                key: 17,
                source: Provenance {
                    node: 2,
                    mesh: 3,
                    primitive: 4,
                    triangle: large,
                },
                vertices: [large, large + 1, large + 2],
                positions: [[0., 0., 0.]; 3],
                bytes: 236,
                degenerate: true,
            },
            expanded_ordinal: large + 3,
            material: u32::MAX,
            flags: NORMAL | TANGENT | UV0 | UV1 | COLOR,
            corners: [attributes; 3],
        };
        let bytes = wire(&triangle);
        assert_eq!(bytes.len(), 286);
        assert_eq!(wire(&FullTriangle::decode(&bytes)), bytes);
        let corner = FullCorner {
            corner: CornerPosition {
                primitive: large,
                triangle: large + 1,
                corner: 2,
                vertex: large + 2,
                position: [1., 2., 3.],
            },
            attributes,
        };
        let bytes = wire(&corner);
        assert_eq!(bytes.len(), 97);
        assert_eq!(wire(&FullCorner::decode(&bytes)), bytes);
        let mut later = triangle;
        later.expanded_ordinal += 1;
        assert!(triangle < later);
    }

    /// A record whose present attributes are all nonzero (a normal of -0.0 included) and
    /// whose absent ones are zero, as capture builds them.
    fn layout_row(flags: u8) -> FullTriangle {
        let large = u64::from(u32::MAX) + 17;
        let full = CornerAttributes {
            normal: [-0., 0.25, 1.],
            tangent: [1., -0.5, 0.125, -1.],
            texcoords: [[-2., 7.], [0.25, 0.75]],
            color: [0.1, 0.5, 1., 0.75],
        };
        let mut corners = [CornerAttributes::default(); 3];
        for (i, corner) in corners.iter_mut().enumerate() {
            let shift = i as f32;
            if flags & NORMAL != 0 {
                corner.normal = full.normal.map(|v| v * (shift + 1.));
            }
            if flags & TANGENT != 0 {
                corner.tangent = full.tangent.map(|v| v + shift);
            }
            if flags & UV0 != 0 {
                corner.texcoords[0] = full.texcoords[0].map(|v| v - shift);
            }
            if flags & UV1 != 0 {
                corner.texcoords[1] = full.texcoords[1].map(|v| v * 3. + shift);
            }
            if flags & COLOR != 0 {
                corner.color = full.color.map(|v| v / (shift + 2.));
            }
        }
        FullTriangle {
            triangle: TriangleRecord {
                key: 17,
                source: Provenance {
                    node: 2,
                    mesh: 3,
                    primitive: 4,
                    triangle: large,
                },
                vertices: [large, large + 1, large + 2],
                positions: [[0., -0., 1.], [2., 3., 4.], [5., 6., 7.]],
                bytes: 236,
                degenerate: false,
            },
            expanded_ordinal: large + 3,
            material: 9,
            flags,
            corners,
        }
    }

    /// Every attribute layout round-trips every record whose attributes it holds, byte for
    /// byte against the full encoding, at exactly the documented width; the full layout is
    /// the unchanged 286/97-byte wire.
    #[test]
    fn attribute_layouts_round_trip_at_their_widths() {
        assert_eq!(corner_attribute_bytes(Layout::FULL), 60);
        assert_eq!(FullTriangle::size_in(Layout::FULL), FullTriangle::SIZE);
        assert_eq!(FullCorner::size_in(Layout::FULL), FullCorner::SIZE);
        // The evidence inputs: crop3 (NORMAL, COLOR_0), tex3 (TEXCOORD_0), positions only.
        assert_eq!(FullTriangle::size_in(Layout(NORMAL | COLOR)), 190);
        assert_eq!(FullTriangle::size_in(Layout(UV0)), 130);
        assert_eq!(FullTriangle::size_in(Layout(0)), 106);
        assert_eq!(FullCorner::size_in(Layout(UV0)), 45);
        for flags in 0..=ALL {
            let row = layout_row(flags);
            let full = wire(&row);
            for extra in [0, ALL, !0] {
                let layout = Layout(flags | extra);
                let size = FullTriangle::size_in(layout);
                assert_eq!(size, 106 + 3 * corner_attribute_bytes(layout));
                let mut bytes = vec![0; size];
                row.encode_in(layout, &mut bytes);
                assert_eq!(wire(&FullTriangle::decode_in(layout, &bytes)), full);
                let corner = FullCorner {
                    corner: CornerPosition {
                        primitive: 1,
                        triangle: 2,
                        corner: 1,
                        vertex: 3,
                        position: [1., 2., 3.],
                    },
                    attributes: row.corners[1],
                };
                let mut bytes = vec![0; FullCorner::size_in(layout)];
                corner.encode_in(layout, &mut bytes);
                assert_eq!(wire(&FullCorner::decode_in(layout, &bytes)), wire(&corner));
            }
        }
    }

    /// A compact sequence reads back the same records sequentially, by range and by
    /// position, and its file holds only the layout's bytes.
    #[test]
    fn compact_sequences_read_back_every_access_path() {
        use crate::spill::SequenceWriter;
        let scratch = Scratch::create(None, 0).unwrap();
        let layout = Layout(UV0 | COLOR);
        let rows: Vec<_> = (0..7)
            .map(|i| {
                let mut row = layout_row(if i % 2 == 0 { UV0 } else { UV0 | COLOR });
                row.expanded_ordinal = i;
                row
            })
            .collect();
        let mut writer = SequenceWriter::with_layout(&scratch, "compact", 16, layout).unwrap();
        for row in &rows {
            writer.push(row).unwrap();
        }
        let sequence = writer.finish().unwrap();
        assert_eq!(sequence.layout(), layout);
        assert_eq!(
            scratch.live_bytes(),
            7 * FullTriangle::size_in(layout) as u64
        );
        let expected: Vec<_> = rows.iter().map(wire).collect();
        let read = |it: crate::spill::SequenceReader<'_, FullTriangle>| -> Vec<Vec<u8>> {
            it.map(|row| wire(&row.unwrap())).collect()
        };
        assert_eq!(read(sequence.reader().unwrap()), expected);
        assert_eq!(read(sequence.range(2, 5, 16).unwrap()), expected[2..5]);
        let (mut encoded, mut out) = (Vec::new(), Vec::new());
        sequence
            .random()
            .unwrap()
            .read(3, 4, &mut encoded, &mut out)
            .unwrap();
        assert_eq!(out.iter().map(wire).collect::<Vec<_>>(), expected[3..7]);
    }

    fn fixture() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("scene.gltf");
        let mut bytes = Vec::new();
        let mut views = Vec::new();
        let mut accessors = Vec::new();
        let mut add =
            |data: Vec<u8>, component: u32, kind: &str, normalized: bool, extrema: bool| {
                let i = views.len();
                views.push(json!({"buffer":0,"byteOffset":bytes.len(),"byteLength":data.len()}));
                bytes.extend(data);
                let mut a = json!({"bufferView":i,"componentType":component,"count":3,"type":kind});
                if normalized {
                    a["normalized"] = json!(true);
                }
                if extrema {
                    a["min"] = json!([0, 0, 0]);
                    a["max"] = json!([1, 1, 0]);
                }
                accessors.push(a);
            };
        let floats = |values: &[f32]| {
            values
                .iter()
                .flat_map(|v| v.to_le_bytes())
                .collect::<Vec<_>>()
        };
        add(
            floats(&[0., 0., 0., 1., 0., 0., 0., 1., 0.]),
            5126,
            "VEC3",
            false,
            true,
        );
        let n = std::f32::consts::FRAC_1_SQRT_2;
        add(
            floats(&[n, n, 0., n, n, 0., n, n, 0.]),
            5126,
            "VEC3",
            false,
            false,
        );
        add(
            floats(&[n, -n, 0., 1., n, -n, 0., 1., n, -n, 0., 1.]),
            5126,
            "VEC4",
            false,
            false,
        );
        add(
            floats(&[0., 0., 1., 0., 0., 1.]),
            5126,
            "VEC2",
            false,
            false,
        );
        add(
            [0u16, 65535, 32768, 0, 65535, 32768]
                .iter()
                .flat_map(|v| v.to_le_bytes())
                .collect(),
            5123,
            "VEC2",
            true,
            false,
        );
        add(
            vec![0, 128, 255, 0, 255, 0, 128, 0, 128, 255, 0, 0],
            5121,
            "VEC3",
            true,
            false,
        );
        views[5]["byteStride"] = json!(4);
        let doc = json!({"asset":{"version":"2.0"},"buffers":[{"uri":"data.bin","byteLength":bytes.len()}],"bufferViews":views,"accessors":accessors,"materials":[{}],"meshes":[{"primitives":[{"attributes":{"POSITION":0,"NORMAL":1,"TANGENT":2,"TEXCOORD_0":3,"TEXCOORD_1":4,"COLOR_0":5},"material":0}]}],"nodes":[{"mesh":0,"scale":[-2,2,2]},{"mesh":0},{"mesh":0},{"mesh":0}],"scenes":[{"nodes":[0,1,2,3]}]});
        fs::write(&input, serde_json::to_vec(&doc).unwrap()).unwrap();
        fs::write(dir.path().join("data.bin"), bytes).unwrap();
        (dir, input)
    }

    #[test]
    fn reflected_nonuniform_normal_uses_inverse_transpose() {
        let (dir, input) = fixture();
        let mut doc: serde_json::Value =
            serde_json::from_slice(&fs::read(&input).unwrap()).unwrap();
        doc["nodes"][0]["scale"] = json!([-2, 1, 3]);
        doc["meshes"][0]["primitives"][0]["attributes"] = json!({"POSITION":0,"NORMAL":1});
        fs::write(&input, serde_json::to_vec(&doc).unwrap()).unwrap();
        let source = StreamSource::open(&input, &mut || Ok(())).unwrap();
        let scan = super::super::capture::scan(&source, 16, &mut || Ok(())).unwrap();
        let scratch = Scratch::create(Some(dir.path()), 0).unwrap();
        let captured = capture_full(
            &source,
            &scratch,
            SortConfig {
                run_bytes: 512,
                fan_in: 2,
                buffer_bytes: 16,
                jobs: 1,
            },
            &scan,
            &mut || Ok(()),
        )
        .unwrap();
        let rows = captured
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap();
        let row = rows.iter().find(|r| r.triangle.source.node == 0).unwrap();
        assert_eq!(row.flags, NORMAL);
        assert_eq!(row.triangle.vertices, [0, 2, 1]);
        assert_eq!(
            row.triangle.positions,
            [[0., 0., 0.], [0., 1., 0.], [-2., 0., 0.]]
        );
        for corner in row.corners {
            assert!((corner.normal[0] + 1. / 5f32.sqrt()).abs() < 1e-6);
            assert!((corner.normal[1] - 2. / 5f32.sqrt()).abs() < 1e-6);
            assert_eq!(corner.normal[2], 0.);
            assert_eq!(corner.tangent, [0.; 4]);
        }
    }

    /// Capture under the run's attribute layout yields the records the full layout does,
    /// through spilled sorts, while spilling fewer bytes; a layout missing a selected
    /// primitive's attribute is refused before any record is written.
    #[test]
    fn compact_attribute_layout_captures_identical_records() {
        let (dir, input) = fixture();
        let mut doc: serde_json::Value =
            serde_json::from_slice(&fs::read(&input).unwrap()).unwrap();
        doc["meshes"][0]["primitives"][0]["attributes"] =
            json!({"POSITION":0,"NORMAL":1,"COLOR_0":5});
        fs::write(&input, serde_json::to_vec(&doc).unwrap()).unwrap();
        let source = StreamSource::open(&input, &mut || Ok(())).unwrap();
        let scan = super::super::capture::scan(&source, 16, &mut || Ok(())).unwrap();
        let scratch = Scratch::create(Some(dir.path()), 0).unwrap();
        let sort = SortConfig {
            run_bytes: 512,
            fan_in: 2,
            buffer_bytes: 16,
            jobs: 1,
        };
        let layout = attribute_layout(&source);
        assert_eq!(layout, Layout(NORMAL | COLOR));
        let run = |layout: Layout| {
            let mut scan = scan;
            let sorted = capture_full_in(
                &source,
                &mut SourceFrame::Local,
                &scratch,
                sort,
                layout,
                &mut scan,
                &mut || Ok(()),
            )?;
            let spilled = sorted.stats().spilled_bytes;
            let rows = sorted.map(|row| row.map(|row| wire(&row)));
            Ok::<_, JobError>((rows.collect::<std::result::Result<Vec<_>, _>>()?, spilled))
        };
        let (full, full_spilled) = run(Layout::FULL).unwrap();
        let (compact, compact_spilled) = run(layout).unwrap();
        assert_eq!(compact, full);
        assert!(compact_spilled > 0);
        assert!(compact_spilled < full_spilled);
        let error = run(Layout(NORMAL)).unwrap_err();
        assert_eq!(error.kind(), JobErrorKind::InvalidState);
        assert_eq!(scratch.live_bytes(), 0);
    }

    #[test]
    fn nonuniform_authored_tangent_remains_unsupported() {
        let (_dir, input) = fixture();
        let mut doc: serde_json::Value =
            serde_json::from_slice(&fs::read(&input).unwrap()).unwrap();
        doc["nodes"][0]["scale"] = json!([-2, 1, 3]);
        fs::write(&input, serde_json::to_vec(&doc).unwrap()).unwrap();
        let error = match StreamSource::open(&input, &mut || Ok(())) {
            Ok(_) => panic!("nonuniform authored tangent must be refused"),
            Err(error) => error,
        };
        assert_eq!(error.kind(), JobErrorKind::Unsupported);
        assert!(error
            .message()
            .contains("authored TANGENT requires a conformal accumulated transform"));
    }

    #[test]
    fn reflected_attributes_are_owned_and_sort_configuration_independent() {
        let (dir, input) = fixture();
        let source = StreamSource::open(&input, &mut || Ok(())).unwrap();
        let scan = super::super::capture::scan(&source, 16, &mut || Ok(())).unwrap();
        let scratch = Scratch::create(Some(dir.path()), 0).unwrap();
        let mut baseline = None;
        for (run_bytes, fan_in) in [(512, 2), (1024, 3)] {
            let sorted = capture_full(
                &source,
                &scratch,
                SortConfig {
                    run_bytes,
                    fan_in,
                    buffer_bytes: 16,
                    jobs: 1,
                },
                &scan,
                &mut || Ok(()),
            )
            .unwrap();
            let rows = sorted.collect::<std::result::Result<Vec<_>, _>>().unwrap();
            let encoded: Vec<_> = rows.iter().flat_map(wire).collect();
            if let Some(baseline) = &baseline {
                assert_eq!(&encoded, baseline);
            } else {
                baseline = Some(encoded);
            }
        }
        let captured = capture_full(
            &source,
            &scratch,
            SortConfig {
                run_bytes: 512,
                fan_in: 2,
                buffer_bytes: 16,
                jobs: 1,
            },
            &scan,
            &mut || Ok(()),
        )
        .unwrap();
        drop(source);
        fs::remove_file(&input).unwrap();
        fs::remove_file(dir.path().join("data.bin")).unwrap();
        let rows = captured
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(rows.len(), 4);
        let row = rows.iter().find(|r| r.triangle.source.node == 0).unwrap();
        assert_eq!(row.expanded_ordinal, 0);
        assert_eq!(row.material, 0);
        assert_eq!(row.flags, NORMAL | TANGENT | UV0 | UV1 | COLOR);
        assert_eq!(row.triangle.vertices, [0, 2, 1]);
        assert_eq!(
            row.triangle.positions,
            [[0., 0., 0.], [0., 2., 0.], [-2., 0., 0.]]
        );
        let a = row.corners[1];
        let near = |actual: f32, expected: f32| {
            assert!((actual - expected).abs() < 1e-6, "{actual} != {expected}")
        };
        near(a.normal[0], -std::f32::consts::FRAC_1_SQRT_2);
        near(a.normal[1], std::f32::consts::FRAC_1_SQRT_2);
        near(a.tangent[0], -std::f32::consts::FRAC_1_SQRT_2);
        near(a.tangent[1], -std::f32::consts::FRAC_1_SQRT_2);
        assert_eq!(a.tangent[3], -1.);
        assert_eq!(a.texcoords[0], [0., 1.]);
        near(a.texcoords[1][0], 1.);
        near(a.texcoords[1][1], 32768. / 65535.);
        assert_eq!(a.color[3], 1.);
        near(a.color[0], 128. / 255.);
        assert_eq!(a.color[1..3], [1., 0.]);
    }
}

#[cfg(test)]
mod position_only_tests {
    use super::*;
    use serde_json::json;
    use std::fs;

    #[test]
    fn absent_attributes_and_degenerates_keep_h1_geometry() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("plain.gltf");
        let doc = json!({"asset":{"version":"2.0"},"buffers":[{"uri":"data.bin","byteLength":36}],"bufferViews":[{"buffer":0,"byteLength":36}],"accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,0,0]}],"meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}],"nodes":[{"mesh":0}],"scenes":[{"nodes":[0]}]});
        fs::write(&input, serde_json::to_vec(&doc).unwrap()).unwrap();
        let bytes: Vec<_> = [0f32, 0., 0., 0., 0., 0., 1., 0., 0.]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        fs::write(dir.path().join("data.bin"), bytes).unwrap();
        let source = StreamSource::open(&input, &mut || Ok(())).unwrap();
        let scan = super::super::capture::scan(&source, 16, &mut || Ok(())).unwrap();
        let scratch = Scratch::create(Some(dir.path()), 0).unwrap();
        let sort = SortConfig {
            run_bytes: 512,
            fan_in: 2,
            buffer_bytes: 16,
            jobs: 1,
        };
        let h1 = super::super::capture::capture(&source, &scratch, sort, &scan, &mut || Ok(()))
            .unwrap()
            .next()
            .unwrap()
            .unwrap();
        let full = capture_full(&source, &scratch, sort, &scan, &mut || Ok(()))
            .unwrap()
            .next()
            .unwrap()
            .unwrap();
        let mut original = [0; TriangleRecord::SIZE];
        let mut projected = [0; TriangleRecord::SIZE];
        h1.encode(&mut original);
        full.triangle.encode(&mut projected);
        assert_eq!(original, projected);
        assert!(full.triangle.degenerate);
        assert_eq!(full.triangle.vertices, [0, 1, 2]);
        assert_eq!(full.flags, 0);
        assert_eq!(full.material, u32::MAX);
        assert_eq!(full.corners, [CornerAttributes::default(); 3]);
    }
}
