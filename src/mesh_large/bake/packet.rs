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

const MAGIC: &[u8; 8] = b"H27CHD\x02\0";

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

struct Writer<'c> {
    check: Option<&'c mut dyn FnMut() -> Result<(), JobError>>,
    checkpoint: usize,
    bytes: Option<Vec<u8>>,
    /// A streaming sink (and its path, for errors) instead of `bytes`.
    io: Option<(&'c mut dyn Write, &'c std::path::Path)>,
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
        if let Some((out, path)) = &mut self.io {
            out.write_all(bytes)
                .map_err(|e| JobError::io("write pre-encode store", path, e))?;
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
            io: None,
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

/// Texels per tile edge in a packet body: a parent reads child texels a tile at a time.
pub(super) const TILE: u32 = 16;
/// Wire bytes of one [`TerminalSample`] (the field order of its `Wire` encoding).
const SAMPLE_WIRE: usize = 8 + 4 * 24 + 4 + 1;
/// Wire bytes of one body texel: its RGBA, then its terminal sample.
pub(super) const TEXEL_WIRE: usize = 4 + SAMPLE_WIRE;

/// What a parent reads of one child page besides its texels.
pub(super) struct PageHead {
    pub width: u32,
    pub height: u32,
    pub slot: TextureSlot,
    pub sampler: Value,
    pub material: u32,
    pub normal_scale: f64,
    /// Every level's dimensions, the base first and the terminal 1x1 last.
    pub levels: Vec<[u32; 2]>,
}
fields!(
    PageHead,
    width,
    height,
    slot,
    sampler,
    material,
    normal_scale,
    levels
);
struct Head {
    node: NodeRecord,
    texel_size: f64,
    faces: Vec<ChildFace>,
    pages: Vec<PageHead>,
}
fields!(Head, node, texel_size, faces, pages);

/// The same bytes as `Head::write` of `child`'s head, without building it.
fn write_head(child: &PreencodeChild, w: &mut Writer<'_>) -> Result<(), JobError> {
    child.node.write(w)?;
    child.texel_size.write(w)?;
    child.faces.write(w)?;
    child.pages.len().write(w)?;
    for p in &child.pages {
        PageHead {
            width: p.width,
            height: p.height,
            slot: p.slot,
            sampler: p.sampler.clone(),
            material: p.material,
            normal_scale: p.normal_scale,
            levels: p.levels.iter().map(|l| [l.width, l.height]).collect(),
        }
        .write(w)?;
    }
    Ok(())
}
fn head_len(
    child: &PreencodeChild,
    check: &mut dyn FnMut() -> Result<(), JobError>,
) -> Result<u64, JobError> {
    let mut count = Writer {
        check: Some(check),
        checkpoint: 0,
        bytes: None,
        io: None,
        count: 0,
        limit: u64::MAX,
    };
    write_head(child, &mut count)?;
    Ok(count.count as u64)
}
fn body_len(mut levels: impl Iterator<Item = [u32; 2]>) -> Result<u64, JobError> {
    levels.try_fold(0u64, |sum, [w, h]| {
        (u64::from(w) * u64::from(h))
            .checked_mul(TEXEL_WIRE as u64)
            .and_then(|n| sum.checked_add(n))
            .ok_or_else(|| resource("child packet body length"))
    })
}
/// Tile `(tx, ty)` of a `width` x `height` level: the position of its first texel in
/// the level's body, and its clipped width and height. Tiles are stored row-major, and
/// the texels of a tile row-major.
pub(super) fn tile_span(width: u32, height: u32, tx: u32, ty: u32) -> (u64, u32, u32) {
    let th = TILE.min(height - ty * TILE);
    let tw = TILE.min(width - tx * TILE);
    (
        u64::from(ty) * u64::from(TILE) * u64::from(width)
            + u64::from(tx) * u64::from(TILE) * u64::from(th),
        tw,
        th,
    )
}
fn put_sample(b: &mut Vec<u8>, s: &TerminalSample) {
    b.extend_from_slice(&s.source_rank.to_le_bytes());
    for v in s
        .barycentric
        .iter()
        .chain(&s.centre)
        .chain(&s.support_min)
        .chain(&s.support_max)
    {
        b.extend_from_slice(&v.to_bits().to_le_bytes());
    }
    b.extend_from_slice(&s.weight.to_le_bytes());
    b.push(u8::from(s.centre_is_centroid));
}
fn get_sample(b: &[u8]) -> Result<TerminalSample, JobError> {
    let u64_at = |o: usize| u64::from_le_bytes(b[o..o + 8].try_into().unwrap());
    let v3 = |o: usize| std::array::from_fn(|a| f64::from_bits(u64_at(o + 8 * a)));
    Ok(TerminalSample {
        source_rank: u64_at(0),
        barycentric: v3(8),
        centre: v3(32),
        support_min: v3(56),
        support_max: v3(80),
        weight: u32::from_le_bytes(b[104..108].try_into().unwrap()),
        centre_is_centroid: match b[108] {
            0 => false,
            1 => true,
            _ => return Err(invalid("child packet boolean")),
        },
    })
}

/// The exact byte length of `child`'s packet (a size known before any write).
pub(super) fn size(
    child: &PreencodeChild,
    check: &mut dyn FnMut() -> Result<(), JobError>,
) -> Result<u64, JobError> {
    let body = body_len(
        child
            .pages
            .iter()
            .flat_map(|p| p.levels.iter().map(|l| [l.width, l.height])),
    )?;
    (16 + head_len(child, check)?)
        .checked_add(body)
        .ok_or_else(|| resource("child packet length"))
}

/// Stream `child`'s packet to `out` (`path` names it in errors) after validating the
/// whole child; returns the bytes written. The packet holds what a parent samples and
/// nothing else: the magic, the head length and head (node, density, faces and page
/// metadata), then each page level's texels, RGBA and terminal sample, in
/// [`TILE`]-square tiles ([`tile_span`]). Its parent decodes the head and reads texels
/// a tile at a time ([`open`], [`read_tile`]), so neither side holds it whole.
pub(super) fn write(
    child: &PreencodeChild,
    classes: &MaterialClasses,
    out: &mut dyn Write,
    path: &std::path::Path,
    check: &mut dyn FnMut() -> Result<(), JobError>,
) -> Result<u64, JobError> {
    validate(child, classes, check)?;
    let head = head_len(child, check)?;
    let mut w = Writer {
        check: Some(&mut *check),
        checkpoint: 0,
        bytes: None,
        io: Some((&mut *out, path)),
        count: 0,
        limit: u64::MAX,
    };
    w.put(MAGIC)?;
    head.write(&mut w)?;
    write_head(child, &mut w)?;
    if w.count as u64 != 16 + head {
        return Err(invalid("child packet head length"));
    }
    drop(w);
    let mut total = 16 + head;
    let mut buf = Vec::with_capacity((TILE * TILE) as usize * TEXEL_WIRE);
    for page in &child.pages {
        for level in &page.levels {
            let (width, height) = (level.width, level.height);
            for ty in 0..height.div_ceil(TILE) {
                check()?;
                for tx in 0..width.div_ceil(TILE) {
                    let (_, tw, th) = tile_span(width, height, tx, ty);
                    buf.clear();
                    for y in ty * TILE..ty * TILE + th {
                        for x in tx * TILE..tx * TILE + tw {
                            let i = y as usize * width as usize + x as usize;
                            buf.extend_from_slice(&level.rgba[i * 4..i * 4 + 4]);
                            put_sample(&mut buf, &level.samples[i]);
                        }
                    }
                    out.write_all(&buf)
                        .map_err(|e| JobError::io("write pre-encode store", path, e))?;
                    total += buf.len() as u64;
                }
            }
        }
    }
    Ok(total)
}

fn read_exact_at(
    file: &std::fs::File,
    path: &std::path::Path,
    mut offset: u64,
    mut out: &mut [u8],
) -> Result<(), JobError> {
    while !out.is_empty() {
        #[cfg(unix)]
        let read = std::os::unix::fs::FileExt::read_at(file, out, offset);
        #[cfg(windows)]
        let read = std::os::windows::fs::FileExt::seek_read(file, out, offset);
        match read {
            Ok(0) => return Err(invalid("truncated child packet")),
            Ok(n) => {
                offset += n as u64;
                out = &mut out[n..];
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(JobError::io("read pre-encode store", path, e)),
        }
    }
    Ok(())
}

/// A child packet's decoded head, as its parent holds it while texels are read.
pub(super) struct Opened {
    pub node: NodeRecord,
    pub texel_size: f64,
    pub faces: Vec<ChildFace>,
    pub pages: Vec<PageHead>,
    /// Byte offset in the packet of each page level's body, by page then level.
    pub offsets: Vec<Vec<u64>>,
    /// Bytes charged for what stays owned (the decoded head and offsets): the lease
    /// minus the transient head bytes.
    pub owned: u64,
}

/// The head checks of [`validate`] that need no texel: node and density, face geometry,
/// frames and bindings, and each page's level chain.
fn validate_head(
    head: &Head,
    classes: &MaterialClasses,
    check: &mut dyn FnMut() -> Result<(), JobError>,
) -> Result<(), JobError> {
    let node = &head.node;
    if node.first >= node.end
        || node.cell_depth > 21
        || node.path_len > 21
        || node.last_octant > 7
        || !head.texel_size.is_finite()
        || head.texel_size < 0.
    {
        return Err(invalid("child packet node/density"));
    }
    for face in &head.faces {
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
                let page = head
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
    for page in &head.pages {
        check()?;
        if page.width == 0
            || page.height == 0
            || !page.normal_scale.is_finite()
            || page.levels.first() != Some(&[page.width, page.height])
            || page.levels.last() != Some(&[1, 1])
        {
            return Err(invalid("child page base shape/scale"));
        }
        let mut dims = [page.width, page.height];
        for (i, level) in page.levels.iter().enumerate() {
            if *level != dims || (*level == [1, 1] && i + 1 != page.levels.len()) {
                return Err(invalid("child mip semantic shape"));
            }
            dims = [(dims[0] / 2).max(1), (dims[1] / 2).max(1)];
        }
    }
    Ok(())
}

/// Decode the head of the packet in `file` within `memory` bytes (the transient head
/// bytes and every decoded owner are charged before allocation), validate it, and check
/// the body has exactly the length the head implies. No texel is read.
pub(super) fn open(
    file: &std::fs::File,
    path: &std::path::Path,
    classes: &MaterialClasses,
    memory: u64,
    check: &mut dyn FnMut() -> Result<(), JobError>,
) -> Result<Opened, JobError> {
    check()?;
    let len = file
        .metadata()
        .map_err(|e| JobError::io("inspect pre-encode store", path, e))?
        .len();
    if len < 16 {
        return Err(invalid("truncated child packet"));
    }
    let mut fixed = [0u8; 16];
    read_exact_at(file, path, 0, &mut fixed)?;
    if &fixed[..8] != MAGIC {
        return Err(invalid("child packet magic"));
    }
    let head_len = u64::from_le_bytes(fixed[8..].try_into().unwrap());
    if head_len > len - 16 {
        return Err(invalid("truncated child packet"));
    }
    if head_len > memory {
        return Err(resource("child packet decoded ownership lease"));
    }
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(head_len as usize)
        .map_err(|_| resource("child packet head reserve"))?;
    bytes.resize(head_len as usize, 0);
    for (i, chunk) in bytes.chunks_mut(1 << 20).enumerate() {
        check()?;
        read_exact_at(file, path, 16 + (i as u64) * (1 << 20), chunk)?;
    }
    let (head, owned) = {
        let mut r = Reader {
            check: &mut *check,
            checkpoint: 0,
            bytes: &bytes,
            offset: 0,
            used: 0,
            limit: memory,
        };
        r.charge(head_len)?;
        r.charge(std::mem::size_of::<Opened>() as u64)?;
        let head = Head::read(&mut r)?;
        if r.offset != bytes.len() {
            return Err(invalid("child packet trailing head bytes"));
        }
        let levels: u64 = head.pages.iter().map(|p| p.levels.len() as u64).sum();
        r.charge(levels * 8 + (head.pages.len() as u64) * std::mem::size_of::<Vec<u64>>() as u64)?;
        let owned = r.used - head_len;
        (head, owned)
    };
    drop(bytes);
    validate_head(&head, classes, check)?;
    let mut at = 16 + head_len;
    let mut offsets = Vec::with_capacity(head.pages.len());
    for page in &head.pages {
        let mut levels = Vec::with_capacity(page.levels.len());
        for &level in &page.levels {
            levels.push(at);
            at = at
                .checked_add(body_len(std::iter::once(level))?)
                .ok_or_else(|| resource("child packet body length"))?;
        }
        offsets.push(levels);
    }
    if at != len {
        return Err(invalid("child packet body length differs from its head"));
    }
    Ok(Opened {
        node: head.node,
        texel_size: head.texel_size,
        faces: head.faces,
        pages: head.pages,
        offsets,
        owned,
    })
}

/// Read tile `(tx, ty)` of a `level` (`[width, height]`) whose body starts at byte
/// `offset` of `file`: its RGBA into `rgba` and its terminal samples into `samples`
/// (row-major inside the tile; `buf` is reused wire space). Every sample must name a
/// source rank of `node` (or be the empty sample), as [`validate`] requires.
#[allow(clippy::too_many_arguments)]
pub(super) fn read_tile(
    file: &std::fs::File,
    path: &std::path::Path,
    node: &NodeRecord,
    level: [u32; 2],
    offset: u64,
    tile: [u32; 2],
    buf: &mut Vec<u8>,
    rgba: &mut [u8],
    samples: &mut Vec<TerminalSample>,
) -> Result<(), JobError> {
    let (first, tw, th) = tile_span(level[0], level[1], tile[0], tile[1]);
    let n = (tw * th) as usize;
    buf.resize(n * TEXEL_WIRE, 0);
    read_exact_at(file, path, offset + first * TEXEL_WIRE as u64, buf)?;
    samples.clear();
    for (i, texel) in buf.as_chunks::<TEXEL_WIRE>().0.iter().enumerate() {
        rgba[i * 4..i * 4 + 4].copy_from_slice(&texel[..4]);
        let s = get_sample(&texel[4..])?;
        if !sample_ok(&s, node) {
            return Err(invalid("child terminal source witness"));
        }
        samples.push(s);
    }
    Ok(())
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crate::mesh_large::records::{Bounds, NodeKind};
    use std::path::Path;
    pub(in crate::mesh_large::bake) fn child() -> PreencodeChild {
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
    fn try_encode(child: &PreencodeChild) -> Result<Vec<u8>, JobError> {
        let mut bytes = Vec::new();
        let n = write(
            child,
            &MaterialClasses::EMPTY,
            &mut bytes,
            Path::new("packet"),
            &mut || Ok(()),
        )?;
        assert_eq!(n, bytes.len() as u64);
        assert_eq!(size(child, &mut || Ok(())).unwrap(), n);
        Ok(bytes)
    }
    pub(in crate::mesh_large::bake) fn encode(child: &PreencodeChild) -> Vec<u8> {
        try_encode(child).unwrap()
    }
    /// The packet `bytes` in a file of its own.
    pub(in crate::mesh_large::bake) fn file(
        bytes: &[u8],
    ) -> (tempfile::TempDir, std::path::PathBuf, std::fs::File) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("child.pre");
        std::fs::write(&path, bytes).unwrap();
        let file = std::fs::File::open(&path).unwrap();
        (dir, path, file)
    }
    fn open_bytes(bytes: &[u8], memory: u64) -> Result<Opened, JobError> {
        let (_dir, path, file) = file(bytes);
        open(
            &file,
            &path,
            &MaterialClasses::EMPTY,
            memory,
            &mut || Ok(()),
        )
    }
    /// A valid `width` x `height` page with every level down to 1x1, whose texels are
    /// distinct (RGBA and terminal samples derived from `seed`, level and position).
    pub(in crate::mesh_large::bake) fn page(
        width: u32,
        height: u32,
        slot: TextureSlot,
        seed: u64,
    ) -> SignalPage {
        let frame = Frame {
            tangent: [1., 0., 0.],
            bitangent: [0., 1., 0.],
            normal: [0., 0., 1.],
        };
        let mut levels = Vec::new();
        let (mut w, mut h) = (width, height);
        loop {
            let n = (w * h) as usize;
            let mut rgba = Vec::with_capacity(n * 4);
            let mut samples = Vec::with_capacity(n);
            for i in 0..n as u64 {
                let k = seed
                    .wrapping_mul(0x9e37_79b9_7f4a_7c15)
                    .wrapping_add((levels.len() as u64) << 40 | i);
                rgba.extend_from_slice(&k.to_le_bytes()[..4]);
                let b = (k % 1000) as f64 / 1000.;
                samples.push(TerminalSample {
                    source_rank: 4,
                    barycentric: [b, 1. - b, 0.],
                    centre: [i as f64, -(levels.len() as f64), b],
                    support_min: [-1., -1., -(k as f64)],
                    support_max: [1., 1., k as f64],
                    weight: (k % 7) as u32,
                    centre_is_centroid: k.is_multiple_of(2),
                });
            }
            levels.push(SignalLevel {
                width: w,
                height: h,
                rgba,
                support: vec![1; n],
                frames: vec![frame; n],
                samples,
                world_normals: if slot == TextureSlot::Normal {
                    vec![[0., 0., 1.]; n]
                } else {
                    Vec::new()
                },
            });
            if (w, h) == (1, 1) {
                break;
            }
            (w, h) = ((w / 2).max(1), (h / 2).max(1));
        }
        SignalPage {
            width,
            height,
            rgba: levels[0].rgba.clone(),
            support: levels[0].support.clone(),
            slot,
            samples: levels[0].samples.clone(),
            levels,
            sampler: serde_json::json!({"minFilter":9987,"magFilter":9729,"wrapS":33071,"wrapT":33071}),
            material: 9,
            receipt: serde_json::json!({"fixture":seed}),
            normal_samples: Vec::new(),
            normal_scale: 1.,
        }
    }
    fn same(a: &TerminalSample, b: &TerminalSample) -> bool {
        sample_equal(a, b)
    }
    #[test]
    fn child_packet_exact_bits_determinism_and_bounded_counts() {
        let original = child();
        let bytes = encode(&original);
        assert_eq!(bytes, encode(&original));
        let opened = open_bytes(&bytes, 4096).unwrap();
        assert_eq!(opened.texel_size.to_bits(), original.texel_size.to_bits());
        assert_eq!(opened.node, original.node);
        assert_eq!(
            open_bytes(&bytes, bytes.len() as u64 - 16)
                .err()
                .unwrap()
                .kind(),
            crate::JobErrorKind::ResourceLimit
        );
        let mut corrupt = bytes.clone();
        let faces = 16 + NodeRecord::SIZE + 8;
        corrupt[faces..faces + 8].copy_from_slice(&u64::MAX.to_le_bytes());
        assert!(open_bytes(&corrupt, 4096).is_err());
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(open_bytes(&trailing, 4096).is_err());
        let mut short = bytes;
        short.pop();
        assert!(open_bytes(&short, 4096).is_err());
    }

    #[test]
    fn opened_owner_is_exactly_the_lease_beyond_the_head_bytes() {
        let mut c = child();
        c.pages.push(page(5, 3, TextureSlot::BaseColor, 1));
        let bytes = encode(&c);
        let head = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
        let owned = open_bytes(&bytes, 1 << 20).unwrap().owned;
        assert!(owned >= std::mem::size_of::<Opened>() as u64);
        assert_eq!(open_bytes(&bytes, head + owned).unwrap().owned, owned);
        assert_eq!(
            open_bytes(&bytes, head + owned - 1).err().unwrap().kind(),
            crate::JobErrorKind::ResourceLimit
        );
    }

    #[test]
    fn tiled_body_returns_every_texel_of_every_level_exactly() {
        let mut c = child();
        // Edges that are and are not tile multiples, and a tall thin page.
        for (i, (w, h)) in [(37u32, 21u32), (32, 16), (3, 70)].into_iter().enumerate() {
            c.pages.push(page(w, h, TextureSlot::BaseColor, i as u64));
        }
        let bytes = encode(&c);
        let (_dir, path, file) = file(&bytes);
        let opened = open(&file, &path, &MaterialClasses::EMPTY, 1 << 20, &mut || {
            Ok(())
        })
        .unwrap();
        let (mut buf, mut rgba, mut samples) = (Vec::new(), vec![0; 4 * 256], Vec::new());
        for (p, page) in c.pages.iter().enumerate() {
            assert_eq!(opened.pages[p].levels.len(), page.levels.len());
            for (l, level) in page.levels.iter().enumerate() {
                let dims = [level.width, level.height];
                assert_eq!(opened.pages[p].levels[l], dims);
                for ty in 0..level.height.div_ceil(TILE) {
                    for tx in 0..level.width.div_ceil(TILE) {
                        read_tile(
                            &file,
                            &path,
                            &opened.node,
                            dims,
                            opened.offsets[p][l],
                            [tx, ty],
                            &mut buf,
                            &mut rgba,
                            &mut samples,
                        )
                        .unwrap();
                        let (_, tw, th) = tile_span(level.width, level.height, tx, ty);
                        assert_eq!(samples.len(), (tw * th) as usize);
                        for y in 0..th {
                            for x in 0..tw {
                                let j = (y * tw + x) as usize;
                                let i = ((ty * TILE + y) * level.width + tx * TILE + x) as usize;
                                assert_eq!(rgba[j * 4..j * 4 + 4], level.rgba[i * 4..i * 4 + 4]);
                                assert!(same(&samples[j], &level.samples[i]));
                            }
                        }
                    }
                }
            }
        }
        // A sample naming a rank outside the child is refused when its tile is read.
        let mut node = opened.node;
        node.first = 5;
        node.end = 6;
        assert!(read_tile(
            &file,
            &path,
            &node,
            [18, 10],
            opened.offsets[0][1],
            [0, 0],
            &mut buf,
            &mut rgba,
            &mut samples
        )
        .is_err());
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
        let bytes = encode(&original);
        let (_dir, path, file) = file(&bytes);
        let opened = open(&file, &path, &MaterialClasses::EMPTY, 1 << 20, &mut || {
            Ok(())
        })
        .unwrap();
        assert_eq!(
            opened.faces[0].positions[0][0].to_bits(),
            (-0.0f32).to_bits()
        );
        assert_eq!(opened.faces[0].source.instance, u64::from(u32::MAX) + 1);
        assert_eq!(opened.faces[0].bindings[4].unwrap().page, 4);
        assert_eq!(opened.pages[2].slot, TextureSlot::Normal);
        assert_eq!(opened.pages[3].sampler["minFilter"], 9728);
        let (mut buf, mut rgba, mut samples) = (Vec::new(), vec![0; 4 * 256], Vec::new());
        read_tile(
            &file,
            &path,
            &opened.node,
            [1, 1],
            opened.offsets[4][0],
            [0, 0],
            &mut buf,
            &mut rgba,
            &mut samples,
        )
        .unwrap();
        assert_eq!(rgba[..4], [129, 87, 255, 213]);
        assert!(same(&samples[0], &sample));
        assert_eq!(samples[0].centre[1].to_bits(), (-0.0f64).to_bits());
        original.faces[0].frames[0].tangent = [0.6, 0., 0.8];
        assert!(try_encode(&original).is_ok());
        original.faces[0].frames[0].tangent = [2., 0., 0.];
        assert!(try_encode(&original).is_err());
        original.faces[0].frames[0].tangent = [0., 0., 1.];
        assert!(try_encode(&original).is_err());
        original.faces[0].frames[0] = frame;
        original.pages[0].sampler = Value::String("x".repeat(65536));
        assert_eq!(
            try_encode(&original).err().unwrap().kind(),
            crate::JobErrorKind::ResourceLimit
        );
        original.pages[0].sampler =
            serde_json::json!({"minFilter":9728,"magFilter":9728,"wrapS":33071,"wrapT":33071});
        original.pages[0].samples[0].barycentric = [0.5, 0.25, 0.25];
        assert!(try_encode(&original).is_err());
    }
}
