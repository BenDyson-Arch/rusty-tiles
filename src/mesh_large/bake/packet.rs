//! Exact, bounded pre-codec child appearance packets. The format is private;
//! emitted texture codecs are never read back as donor appearance.
use super::{frames::Frame, invalid, resource, types::*};
use crate::{
    gltf_source::source::TextureSlot,
    mesh_large::{batching::MaterialClasses, records::NodeRecord},
    spill::Record,
    JobError,
};
use serde_json::Value;
use std::io::Write;

const MAGIC: &[u8; 8] = b"H27CHD\x01\0";

pub(super) fn page_owned_bytes(page: &SignalPage) -> Result<u64, JobError> {
    fn add(total: &mut u64, n: usize, width: usize) -> Result<(), JobError> {
        let bytes = (n as u64)
            .checked_mul(width as u64)
            .ok_or_else(|| resource("child page owner overflow"))?;
        *total = total
            .checked_add(bytes)
            .ok_or_else(|| resource("child page owner overflow"))?;
        Ok(())
    }
    let mut total = std::mem::size_of::<SignalPage>() as u64;
    add(&mut total, page.rgba.capacity(), 1)?;
    add(&mut total, page.support.capacity(), 4)?;
    add(
        &mut total,
        page.samples.capacity(),
        std::mem::size_of::<TerminalSample>(),
    )?;
    add(
        &mut total,
        page.normal_samples.capacity(),
        std::mem::size_of::<NormalSample>(),
    )?;
    add(
        &mut total,
        page.levels.capacity(),
        std::mem::size_of::<SignalLevel>(),
    )?;
    for level in &page.levels {
        add(&mut total, level.rgba.capacity(), 1)?;
        add(&mut total, level.support.capacity(), 4)?;
        add(
            &mut total,
            level.frames.capacity(),
            std::mem::size_of::<Frame>(),
        )?;
        add(
            &mut total,
            level.samples.capacity(),
            std::mem::size_of::<TerminalSample>(),
        )?;
        add(
            &mut total,
            level.world_normals.capacity(),
            std::mem::size_of::<[f64; 3]>(),
        )?;
    }
    total
        .checked_add(super::json_charge(&page.sampler))
        .and_then(|n| n.checked_add(super::json_charge(&page.receipt)))
        .ok_or_else(|| resource("child JSON owner overflow"))
}

fn resident_bytes(child: &PreencodeChild) -> Result<u64, JobError> {
    let faces = (child.faces.capacity() as u64)
        .checked_mul(std::mem::size_of::<ChildFace>() as u64)
        .ok_or_else(|| resource("child resident face owner overflow"))?;
    let pages = (child.pages.capacity() as u64)
        .checked_mul(std::mem::size_of::<SignalPage>() as u64)
        .ok_or_else(|| resource("child resident page owner overflow"))?;
    let mut total = (std::mem::size_of::<PreencodeChild>() as u64)
        .checked_add(faces)
        .and_then(|n| n.checked_add(pages))
        .ok_or_else(|| resource("child resident owner overflow"))?;
    for page in &child.pages {
        total = total
            .checked_add(page_owned_bytes(page)?)
            .ok_or_else(|| resource("child resident owner overflow"))?;
    }
    Ok(total)
}

struct Writer<'c> {
    check: Option<&'c mut dyn FnMut() -> Result<(), JobError>>,
    checkpoint: usize,
    bytes: Option<Vec<u8>>,
    count: usize,
    limit: u64,
}
impl Writer<'_> {
    fn put(&mut self, bytes: &[u8]) -> Result<(), JobError> {
        self.count = self
            .count
            .checked_add(bytes.len())
            .ok_or_else(|| resource("child packet length"))?;
        if self.count as u64 > self.limit {
            return Err(resource("child packet encode lease"));
        }
        if self.count.saturating_sub(self.checkpoint) >= 65536 {
            if let Some(check) = &mut self.check {
                check()?;
            }
            self.checkpoint = self.count;
        }
        if let Some(out) = &mut self.bytes {
            out.extend_from_slice(bytes);
        }
        Ok(())
    }
}
impl Write for Writer<'_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.put(bytes).map_err(std::io::Error::other)?;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
struct Reader<'a, 'c> {
    check: &'c mut dyn FnMut() -> Result<(), JobError>,
    checkpoint: usize,
    bytes: &'a [u8],
    offset: usize,
    used: u64,
    limit: u64,
}
impl<'a> Reader<'a, '_> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], JobError> {
        let end = self
            .offset
            .checked_add(n)
            .ok_or_else(|| invalid("child packet offset"))?;
        let out = self
            .bytes
            .get(self.offset..end)
            .ok_or_else(|| invalid("truncated child packet"))?;
        self.offset = end;
        if end.saturating_sub(self.checkpoint) >= 65536 {
            (self.check)()?;
            self.checkpoint = end;
        }
        Ok(out)
    }
    fn charge(&mut self, n: u64) -> Result<(), JobError> {
        self.used = self
            .used
            .checked_add(n)
            .ok_or_else(|| resource("child decode owner overflow"))?;
        if self.used > self.limit {
            return Err(resource("child packet decoded ownership lease"));
        }
        Ok(())
    }
    fn count<T>(&mut self) -> Result<usize, JobError> {
        let n = usize::try_from(u64::read(self)?)
            .map_err(|_| resource("child packet vector length"))?;
        self.charge(
            (n as u64)
                .checked_mul(std::mem::size_of::<T>() as u64)
                .ok_or_else(|| resource("child packet vector charge"))?,
        )?;
        // Every element consumes at least one wire byte. Refuse adversarial
        // counts before any reserve, even where the RAM allowance is generous.
        if n > self.bytes.len().saturating_sub(self.offset) {
            return Err(invalid("child packet vector count exceeds bytes"));
        }
        Ok(n)
    }
}
trait Wire: Sized {
    fn write(&self, w: &mut Writer<'_>) -> Result<(), JobError>;
    fn read(r: &mut Reader<'_, '_>) -> Result<Self, JobError>;
    fn write_many(values: &[Self], w: &mut Writer<'_>) -> Result<(), JobError> {
        for value in values {
            value.write(w)?;
        }
        Ok(())
    }
    fn read_many(n: usize, r: &mut Reader<'_, '_>) -> Result<Vec<Self>, JobError> {
        let mut out = Vec::new();
        out.try_reserve_exact(n)
            .map_err(|_| resource("child packet reserve"))?;
        for _ in 0..n {
            out.push(Self::read(r)?);
        }
        Ok(out)
    }
}
macro_rules! integer {
    ($t:ty, $n:expr) => {
        impl Wire for $t {
            fn write(&self, w: &mut Writer<'_>) -> Result<(), JobError> {
                w.put(&self.to_le_bytes())
            }
            fn read(r: &mut Reader<'_, '_>) -> Result<Self, JobError> {
                Ok(<$t>::from_le_bytes(r.take($n)?.try_into().unwrap()))
            }
        }
    };
}
impl Wire for u8 {
    fn write(&self, w: &mut Writer<'_>) -> Result<(), JobError> {
        w.put(&[*self])
    }
    fn read(r: &mut Reader<'_, '_>) -> Result<Self, JobError> {
        Ok(r.take(1)?[0])
    }
    fn write_many(values: &[Self], w: &mut Writer<'_>) -> Result<(), JobError> {
        for chunk in values.chunks(65536) {
            w.put(chunk)?;
        }
        Ok(())
    }
    fn read_many(n: usize, r: &mut Reader<'_, '_>) -> Result<Vec<Self>, JobError> {
        let mut out = Vec::new();
        out.try_reserve_exact(n)
            .map_err(|_| resource("child byte buffer reserve"))?;
        let mut left = n;
        while left > 0 {
            let count = left.min(65536);
            out.extend_from_slice(r.take(count)?);
            left -= count;
        }
        Ok(out)
    }
}
integer!(u32, 4);
integer!(u64, 8);
impl Wire for usize {
    fn write(&self, w: &mut Writer<'_>) -> Result<(), JobError> {
        (*self as u64).write(w)
    }
    fn read(r: &mut Reader<'_, '_>) -> Result<Self, JobError> {
        usize::try_from(u64::read(r)?).map_err(|_| invalid("child packet usize"))
    }
}
macro_rules! float {
    ($t:ty,$bits:ty) => {
        impl Wire for $t {
            fn write(&self, w: &mut Writer<'_>) -> Result<(), JobError> {
                self.to_bits().write(w)
            }
            fn read(r: &mut Reader<'_, '_>) -> Result<Self, JobError> {
                Ok(<$t>::from_bits(<$bits>::read(r)?))
            }
        }
    };
}
float!(f32, u32);
float!(f64, u64);
impl Wire for bool {
    fn write(&self, w: &mut Writer<'_>) -> Result<(), JobError> {
        u8::from(*self).write(w)
    }
    fn read(r: &mut Reader<'_, '_>) -> Result<Self, JobError> {
        match u8::read(r)? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(invalid("child packet boolean")),
        }
    }
}
impl<T: Wire, const N: usize> Wire for [T; N] {
    fn write(&self, w: &mut Writer<'_>) -> Result<(), JobError> {
        for x in self {
            x.write(w)?;
        }
        Ok(())
    }
    fn read(r: &mut Reader<'_, '_>) -> Result<Self, JobError> {
        // Array storage is already charged by the enclosing owner, not a
        // temporary heap vector. Fallible fixed-array construction via Options.
        let mut values: [Option<T>; N] = std::array::from_fn(|_| None);
        for value in &mut values {
            *value = Some(T::read(r)?);
        }
        Ok(values.map(Option::unwrap))
    }
}
impl<T: Wire> Wire for Option<T> {
    fn write(&self, w: &mut Writer<'_>) -> Result<(), JobError> {
        self.is_some().write(w)?;
        if let Some(x) = self {
            x.write(w)?;
        }
        Ok(())
    }
    fn read(r: &mut Reader<'_, '_>) -> Result<Self, JobError> {
        if bool::read(r)? {
            Ok(Some(T::read(r)?))
        } else {
            Ok(None)
        }
    }
}
impl<T: Wire> Wire for Vec<T> {
    fn write(&self, w: &mut Writer<'_>) -> Result<(), JobError> {
        self.len().write(w)?;
        T::write_many(self, w)
    }
    fn read(r: &mut Reader<'_, '_>) -> Result<Self, JobError> {
        let n = r.count::<T>()?;
        T::read_many(n, r)
    }
}
impl Wire for Value {
    fn write(&self, w: &mut Writer<'_>) -> Result<(), JobError> {
        let mut count = Writer {
            check: None,
            checkpoint: 0,
            bytes: None,
            count: 0,
            limit: w.limit.min(65535),
        };
        serde_json::to_writer(&mut count, self).map_err(|e| {
            if e.is_io() {
                resource(format!("child JSON metadata sizing lease: {e}"))
            } else {
                invalid(format!("child JSON encode: {e}"))
            }
        })?;
        if count.count >= 65536 {
            return Err(resource("child packet metadata maximum"));
        }
        count.count.write(w)?;
        if let Some(check) = &mut w.check {
            check()?;
        }
        w.checkpoint = w.count;
        serde_json::to_writer(w, self).map_err(|e| invalid(format!("child JSON encode: {e}")))
    }
    fn read(r: &mut Reader<'_, '_>) -> Result<Self, JobError> {
        let n = usize::read(r)?;
        if n >= 65536 {
            return Err(resource("child packet metadata maximum"));
        }
        // Value's map nodes, strings, arrays and parser temporaries are charged
        // BEFORE serde allocates. The parser's default recursion bound remains.
        r.charge(
            (n as u64)
                .checked_mul(128)
                .ok_or_else(|| resource("child JSON lease"))?,
        )?;
        serde_json::from_slice(r.take(n)?).map_err(|e| invalid(format!("child JSON decode: {e}")))
    }
}
impl Wire for NodeRecord {
    fn write(&self, w: &mut Writer<'_>) -> Result<(), JobError> {
        let mut bytes = [0u8; Self::SIZE];
        self.encode(&mut bytes);
        w.put(&bytes)
    }
    fn read(r: &mut Reader<'_, '_>) -> Result<Self, JobError> {
        let bytes = r.take(Self::SIZE)?;
        if bytes[19] > 1 || bytes[69..].iter().any(|b| *b != 0) {
            return Err(invalid("child node wire kind/reserved bits"));
        }
        Ok(Self::decode(bytes))
    }
}
impl Wire for TextureSlot {
    fn write(&self, w: &mut Writer<'_>) -> Result<(), JobError> {
        (slot_index(*self) as u8).write(w)
    }
    fn read(r: &mut Reader<'_, '_>) -> Result<Self, JobError> {
        match u8::read(r)? {
            0 => Ok(Self::BaseColor),
            1 => Ok(Self::MetallicRoughness),
            2 => Ok(Self::Normal),
            3 => Ok(Self::Occlusion),
            4 => Ok(Self::Emissive),
            _ => Err(invalid("child packet texture slot")),
        }
    }
}
macro_rules! fields {
    ($t:ty, $($field:ident),+ $(,)?) => {impl Wire for $t {
        fn write(&self,w:&mut Writer<'_>)->Result<(),JobError>{$ (self.$field.write(w)?;)+ Ok(())}
        fn read(r:&mut Reader<'_, '_>)->Result<Self,JobError>{Ok(Self{$($field:Wire::read(r)?,)+})}
    }};
}
fields!(Frame, tangent, bitangent, normal);
fields!(SourceKey, node, mesh, primitive, instance);
fields!(ChildBinding, page, uv);
fields!(
    ChildFace,
    source,
    source_triangle,
    material,
    positions,
    frames,
    bindings
);
fields!(
    TerminalSample,
    source_rank,
    barycentric,
    centre,
    support_min,
    support_max,
    weight,
    centre_is_centroid
);
fields!(NormalSample, uv, frame, chart, weight);
fields!(
    SignalLevel,
    width,
    height,
    rgba,
    support,
    frames,
    samples,
    world_normals
);
fields!(
    SignalPage,
    width,
    height,
    rgba,
    support,
    slot,
    samples,
    levels,
    sampler,
    material,
    receipt,
    normal_samples,
    normal_scale
);
fields!(PreencodeChild, node, texel_size, faces, pages);

fn frame_ok(frame: Frame) -> bool {
    [frame.tangent, frame.bitangent, frame.normal]
        .iter()
        .all(|axis| {
            axis.iter().all(|v| v.is_finite())
                && (axis.iter().map(|v| v * v).sum::<f64>() - 1.).abs() <= 1e-6
        })
}
fn frame_invertible(frame: Frame) -> bool {
    let det = super::math::dot(
        frame.tangent,
        super::math::cross(frame.bitangent, frame.normal),
    );
    det.is_finite() && det != 0.
}
fn sample_ok(s: &TerminalSample, node: &NodeRecord) -> bool {
    ((s.source_rank >= node.first && s.source_rank < node.end)
        || (s.weight == 0 && s.source_rank == u64::MAX))
        && s.barycentric
            .iter()
            .chain(s.centre.iter())
            .chain(s.support_min.iter())
            .chain(s.support_max.iter())
            .all(|v| v.is_finite())
        && s.barycentric.iter().all(|v| (0.0..=1.0).contains(v))
        && (s.barycentric.iter().sum::<f64>() - 1.).abs() <= 1e-8
        && (0..3).all(|a| s.support_min[a] <= s.support_max[a])
}
fn sample_equal(a: &TerminalSample, b: &TerminalSample) -> bool {
    a.source_rank == b.source_rank
        && a.barycentric.map(f64::to_bits) == b.barycentric.map(f64::to_bits)
        && a.centre.map(f64::to_bits) == b.centre.map(f64::to_bits)
        && a.support_min.map(f64::to_bits) == b.support_min.map(f64::to_bits)
        && a.support_max.map(f64::to_bits) == b.support_max.map(f64::to_bits)
        && a.weight == b.weight
        && a.centre_is_centroid == b.centre_is_centroid
}
/// `classes` maps a face's source material to the class a page serves (H28).
pub(super) fn validate(
    child: &PreencodeChild,
    classes: &MaterialClasses,
    check: &mut dyn FnMut() -> Result<(), JobError>,
) -> Result<(), JobError> {
    let node = &child.node;
    if node.first >= node.end
        || node.cell_depth > 21
        || node.path_len > 21
        || node.last_octant > 7
        || !child.texel_size.is_finite()
        || child.texel_size < 0.
    {
        return Err(invalid("child packet node/density"));
    }
    for face in &child.faces {
        check()?;
        if face.positions.iter().flatten().any(|v| !v.is_finite())
            || face.frames.iter().any(|f| !frame_ok(*f))
            || (face.bindings[slot_index(TextureSlot::Normal)].is_some()
                && face.frames.iter().any(|f| !frame_invertible(*f)))
        {
            return Err(invalid("child packet decoded geometry/frame"));
        }
        for (slot, binding) in face.bindings.iter().enumerate() {
            if let Some(binding) = binding {
                let page = child
                    .pages
                    .get(binding.page)
                    .ok_or_else(|| invalid("child packet binding page"))?;
                if slot_index(page.slot) != slot
                    || page.material != classes.of(face.material)
                    || binding.uv.iter().flatten().any(|v| !v.is_finite())
                {
                    return Err(invalid("child packet slot/material/UV binding"));
                }
            }
        }
    }
    for page in &child.pages {
        check()?;
        let count = (page.width as usize)
            .checked_mul(page.height as usize)
            .ok_or_else(|| resource("child page shape"))?;
        if count == 0
            || count.checked_mul(4) != Some(page.rgba.len())
            || page.support.len() != count
            || page.samples.len() != count
            || !page.normal_scale.is_finite()
            || page.levels.is_empty()
        {
            return Err(invalid("child page base shape/scale"));
        }
        if page.levels[0].rgba != page.rgba
            || page.levels[0].support != page.support
            || page.levels[0].samples.len() != page.samples.len()
            || page.levels[0]
                .samples
                .iter()
                .zip(&page.samples)
                .any(|(a, b)| !sample_equal(a, b))
        {
            return Err(invalid("child duplicate base differs from semantic chain"));
        }
        if page.samples.iter().any(|s| !sample_ok(s, node)) {
            return Err(invalid("child terminal source witness"));
        }
        let mut dims = (page.width, page.height);
        for (level_index, level) in page.levels.iter().enumerate() {
            let n = (level.width as usize)
                .checked_mul(level.height as usize)
                .ok_or_else(|| resource("child mip shape"))?;
            let normal = page.slot == TextureSlot::Normal;
            if (level.width, level.height) != dims
                || n.checked_mul(4) != Some(level.rgba.len())
                || level.support.len() != n
                || level.samples.len() != n
                || level.frames.len() != n
                || (normal && level.world_normals.len() != n)
                || (!normal && !level.world_normals.is_empty())
                || level
                    .frames
                    .iter()
                    .any(|f| !frame_ok(*f) || (normal && !frame_invertible(*f)))
                || level.world_normals.iter().flatten().any(|v| !v.is_finite())
                || level.samples.iter().any(|s| !sample_ok(s, node))
                || ((level.width, level.height) == (1, 1) && level_index + 1 != page.levels.len())
            {
                return Err(invalid("child mip semantic shape"));
            }
            dims = ((dims.0 / 2).max(1), (dims.1 / 2).max(1));
        }
        if page
            .levels
            .last()
            .is_none_or(|l| l.width != 1 || l.height != 1)
        {
            return Err(invalid("child terminal mip missing"));
        }
        for sample in &page.normal_samples {
            if !sample.weight.is_finite()
                || sample.weight < 0.
                || sample.uv.iter().any(|v| !v.is_finite())
                || !frame_ok(sample.frame)
                || !frame_invertible(sample.frame)
            {
                return Err(invalid("child normal renderer witness"));
            }
        }
    }
    Ok(())
}

/// `memory` covers the resident child AND additional encoded output bytes.
/// Other live caller owners remain under the caller aggregate stage lease.
pub(super) fn encode(
    child: &PreencodeChild,
    classes: &MaterialClasses,
    memory: u64,
    check: &mut dyn FnMut() -> Result<(), JobError>,
) -> Result<Vec<u8>, JobError> {
    validate(child, classes, check)?;
    let memory = memory
        .checked_sub(resident_bytes(child)?)
        .ok_or_else(|| resource("child resident plus serialized output lease"))?;
    let mut count = Writer {
        check: Some(check),
        checkpoint: 0,
        bytes: None,
        count: 0,
        limit: memory,
    };
    count.put(MAGIC)?;
    child.write(&mut count)?;
    let byte_count = count.count;
    drop(count);
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(byte_count)
        .map_err(|_| resource("child packet output reserve"))?;
    let mut out = Writer {
        check: Some(check),
        checkpoint: 0,
        bytes: Some(bytes),
        count: 0,
        limit: memory,
    };
    out.put(MAGIC)?;
    child.write(&mut out)?;
    let bytes = out.bytes.take().unwrap();
    drop(out);
    check()?;
    Ok(bytes)
}
pub(super) fn decode(
    bytes: &[u8],
    classes: &MaterialClasses,
    memory: u64,
    check: &mut dyn FnMut() -> Result<(), JobError>,
) -> Result<PreencodeChild, JobError> {
    decode_owned(bytes, classes, memory, check).map(|(child, _)| child)
}
/// [`decode`], also returning the bytes charged for the decoded child alone (the lease
/// minus the encoded input), which stay owned after `bytes` is dropped.
pub(super) fn decode_owned(
    bytes: &[u8],
    classes: &MaterialClasses,
    memory: u64,
    check: &mut dyn FnMut() -> Result<(), JobError>,
) -> Result<(PreencodeChild, u64), JobError> {
    check()?;
    let mut r = Reader {
        check,
        checkpoint: 0,
        bytes,
        offset: 0,
        used: 0,
        limit: memory,
    };
    r.charge(bytes.len() as u64)?;
    r.charge(std::mem::size_of::<PreencodeChild>() as u64)?;
    if r.take(8)? != MAGIC {
        return Err(invalid("child packet magic"));
    }
    let child = PreencodeChild::read(&mut r)?;
    if r.offset != bytes.len() {
        return Err(invalid("child packet trailing bytes"));
    }
    let owned = r.used - bytes.len() as u64;
    validate(&child, classes, check)?;
    Ok((child, owned))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh_large::records::{Bounds, NodeKind};
    fn child() -> PreencodeChild {
        PreencodeChild {
            node: NodeRecord {
                cell_depth: 0,
                cell: 0,
                path_len: 0,
                path: 0,
                last_octant: 0,
                kind: NodeKind::Leaf,
                children: 0,
                first: 4,
                end: 5,
                bytes: 0,
                bounds: Bounds {
                    min: [0.; 3],
                    max: [1.; 3],
                },
            },
            texel_size: f64::from_bits(0x8000000000000000),
            faces: Vec::new(),
            pages: Vec::new(),
        }
    }
    #[test]
    fn child_packet_exact_bits_determinism_and_bounded_counts() {
        let original = child();
        let bytes = encode(&original, &MaterialClasses::EMPTY, 4096, &mut || Ok(())).unwrap();
        assert_eq!(bytes, encode(&original, &MaterialClasses::EMPTY, 4096, &mut || Ok(())).unwrap());
        let decoded = decode(&bytes, &MaterialClasses::EMPTY, 4096, &mut || Ok(())).unwrap();
        assert_eq!(decoded.texel_size.to_bits(), original.texel_size.to_bits());
        assert_eq!(decoded.node, original.node);
        assert!(decode(&bytes, &MaterialClasses::EMPTY, bytes.len() as u64, &mut || Ok(())).is_err());
        let mut corrupt = bytes.clone();
        let faces = 8 + NodeRecord::SIZE + 8;
        corrupt[faces..faces + 8].copy_from_slice(&u64::MAX.to_le_bytes());
        assert!(decode(&corrupt, &MaterialClasses::EMPTY, 4096, &mut || Ok(())).is_err());
        let mut trailing = bytes;
        trailing.push(0);
        assert!(decode(&trailing, &MaterialClasses::EMPTY, 4096, &mut || Ok(())).is_err());
    }

    #[test]
    fn decoded_owner_is_exactly_the_lease_beyond_the_encoded_bytes() {
        let bytes = encode(&child(), &MaterialClasses::EMPTY, 4096, &mut || Ok(())).unwrap();
        let (_, owned) = decode_owned(&bytes, &MaterialClasses::EMPTY, 1 << 20, &mut || Ok(())).unwrap();
        assert!(owned >= std::mem::size_of::<PreencodeChild>() as u64);
        let exact = bytes.len() as u64 + owned;
        assert_eq!(
            decode_owned(&bytes, &MaterialClasses::EMPTY, exact, &mut || Ok(())).unwrap().1,
            owned
        );
        assert_eq!(
            decode(&bytes, &MaterialClasses::EMPTY, exact - 1, &mut || Ok(()))
                .err()
                .unwrap()
                .kind(),
            crate::JobErrorKind::ResourceLimit
        );
    }

    #[test]
    fn child_packet_roundtrips_all_slots_frames_ledgers_and_bindings() {
        let mut original = child();
        let frame = Frame {
            tangent: [1., -0., 0.],
            bitangent: [0., 1., 0.],
            normal: [0., 0., 1.],
        };
        let sample = TerminalSample {
            source_rank: 4,
            barycentric: [0.25, 0.25, 0.5],
            centre: [0.25, -0., 0.],
            support_min: [0.; 3],
            support_max: [1.; 3],
            weight: 7,
            centre_is_centroid: true,
        };
        for slot in [
            TextureSlot::BaseColor,
            TextureSlot::MetallicRoughness,
            TextureSlot::Normal,
            TextureSlot::Occlusion,
            TextureSlot::Emissive,
        ] {
            original.pages.push(SignalPage{width:1,height:1,rgba:vec![129,87,255,213],support:vec![7],slot,samples:vec![sample],levels:vec![SignalLevel{width:1,height:1,rgba:vec![129,87,255,213],support:vec![7],frames:vec![frame],samples:vec![sample],world_normals:if slot==TextureSlot::Normal{vec![[0.,0.,1.]]}else{Vec::new()}}],sampler:serde_json::json!({"minFilter":9728,"magFilter":9728,"wrapS":33071,"wrapT":33071}),material:9,receipt:serde_json::json!({"immutable":18446744073709551615u64}),normal_samples:if slot==TextureSlot::Normal{vec![NormalSample{uv:[0.25,0.75],frame,chart:17,weight:0.5}]}else{Vec::new()},normal_scale:1.});
        }
        original.faces.push(ChildFace {
            source: SourceKey {
                node: 7,
                mesh: 8,
                primitive: 9,
                instance: u64::from(u32::MAX) + 1,
            },
            source_triangle: u64::from(u32::MAX) + 3,
            material: 9,
            positions: [[-0., 0., 0.], [1., 0., 0.], [0., 1., 0.]],
            frames: [frame; 3],
            bindings: std::array::from_fn(|page| {
                Some(ChildBinding {
                    page,
                    uv: [[-0., 0.], [1., 0.], [0., 1.]],
                })
            }),
        });
        let bytes = encode(&original, &MaterialClasses::EMPTY, 1 << 20, &mut || Ok(())).unwrap();
        let decoded = decode(&bytes, &MaterialClasses::EMPTY, 1 << 20, &mut || Ok(())).unwrap();
        assert_eq!(bytes, encode(&decoded, &MaterialClasses::EMPTY, 1 << 20, &mut || Ok(())).unwrap());
        assert_eq!(
            decoded.faces[0].positions[0][0].to_bits(),
            (-0.0f32).to_bits()
        );
        assert_eq!(decoded.pages[2].normal_samples[0].chart, 17);
        assert_eq!(decoded.pages[4].samples[0].source_rank, 4);
        original.faces[0].frames[0].tangent = [0.6, 0., 0.8];
        assert!(encode(&original, &MaterialClasses::EMPTY, 1 << 20, &mut || Ok(())).is_ok());
        original.faces[0].frames[0].tangent = [2., 0., 0.];
        assert!(encode(&original, &MaterialClasses::EMPTY, 1 << 20, &mut || Ok(())).is_err());
        original.faces[0].frames[0].tangent = [0., 0., 1.];
        assert!(encode(&original, &MaterialClasses::EMPTY, 1 << 20, &mut || Ok(())).is_err());
        original.faces[0].frames[0] = frame;
        original.pages[0].receipt = Value::String("x".repeat(65536));
        assert_eq!(
            encode(&original, &MaterialClasses::EMPTY, 1 << 20, &mut || Ok(()))
                .err()
                .unwrap()
                .kind(),
            crate::JobErrorKind::ResourceLimit
        );
        original.pages[0].receipt = serde_json::json!({"immutable":18446744073709551615u64});
        original.pages[0].samples[0].barycentric = [0.5, 0.25, 0.25];
        assert!(encode(&original, &MaterialClasses::EMPTY, 1 << 20, &mut || Ok(())).is_err());
    }
}
