//! Draft vector GLBs, typed metadata, quantization and direct Rust meshopt.
use super::*;
use crate::tileset_node::{
    GeometricError, NodePlacement, Refine, SourceIdentity, TileBox, TileRecord,
};
use crate::{
    content_integrity::FormatError,
    glb,
    metadata::MetadataGlb,
    runtime::Attempt,
    vector_encoding::{self as codec, CodecError, GeneratedCounts, ValidatedCompressionLimits},
    JobError,
};
use std::io::Write;

/// The producer resolves its existing compression choice and core limits once.
/// Fitting and final encoding receive this same policy and the same Attempt.
#[derive(Clone, Copy)]
pub(super) struct ProducerPolicy {
    pub(super) limits: ValidatedCompressionLimits,
    compress: bool,
}
impl ProducerPolicy {
    pub(super) fn new(compress: bool) -> Result<Self, FormatError> {
        Ok(Self {
            limits: codec::CompressionLimits::default().validate()?,
            compress,
        })
    }
}

pub(super) fn codec_failure(error: CodecError<JobError>, attempt: &Attempt) -> Error {
    Error::Job(attempt.fail(crate::vector_compression::authored_codec_error(error)))
}
fn resource(reason: &'static str) -> FormatError {
    FormatError::ResourceLimit(reason.into())
}
fn invalid(reason: &'static str) -> FormatError {
    FormatError::InvalidInput(reason.into())
}
fn add_storage(a: usize, b: usize) -> Result<usize, FormatError> {
    a.checked_add(b)
        .filter(|n| *n <= isize::MAX as usize)
        .ok_or_else(|| resource("vector bridge storage overflow"))
}
fn array_storage<T>(capacity: usize) -> Result<usize, FormatError> {
    capacity
        .checked_mul(std::mem::size_of::<T>())
        .filter(|n| *n <= isize::MAX as usize)
        .ok_or_else(|| resource("vector bridge capacity overflow"))
}

/// Conservative retained BTreeMap node storage for the matched std source:
/// B=6, eleven key/value slots and twelve internal edges. A nonempty node
/// contains at least one entry, so max(entries,1) full internal nodes bounds
/// retained backing (including an empty retained root). This is not allocator
/// usable-size, startup storage or an RSS bound. Arrays/strings use real capacity.
#[repr(C)]
struct ValueMapNodeUpper {
    _parent: usize,
    _parent_index: u16,
    _len: u16,
    _keys: [std::mem::MaybeUninit<String>; 11],
    _values: [std::mem::MaybeUninit<Value>; 11],
    _edges: [usize; 12],
}

fn value_storage(
    value: &Value,
    depth: u64,
    nodes: &mut usize,
    limits: &codec::CompressionLimits,
) -> Result<usize, FormatError> {
    if depth > limits.json_depth {
        return Err(resource("vector bridge JSON depth limit"));
    }
    *nodes = nodes
        .checked_add(1)
        .filter(|n| *n <= limits.json_value_nodes)
        .ok_or_else(|| resource("vector bridge JSON node limit"))?;
    match value {
        Value::String(text) => Ok(text.capacity()),
        Value::Array(values) => {
            let mut size = array_storage::<Value>(values.capacity())?;
            for child in values {
                let child_depth = depth
                    .checked_add(1)
                    .ok_or_else(|| resource("vector bridge JSON depth overflow"))?;
                size = add_storage(size, value_storage(child, child_depth, nodes, limits)?)?;
            }
            Ok(size)
        }
        Value::Object(values) => {
            let mut size = array_storage::<ValueMapNodeUpper>(values.len().max(1))?;
            for (key, child) in values {
                size = add_storage(size, key.capacity())?;
                let child_depth = depth
                    .checked_add(1)
                    .ok_or_else(|| resource("vector bridge JSON depth overflow"))?;
                size = add_storage(size, value_storage(child, child_depth, nodes, limits)?)?;
            }
            Ok(size)
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => Ok(0),
    }
}

struct BridgeFacts {
    counts: GeneratedCounts,
    document_storage: usize,
    metadata: bool,
}
fn bridge_facts(
    document: &Value,
    binary: &[u8],
    limits: &codec::CompressionLimits,
) -> Result<BridgeFacts, FormatError> {
    let document_storage = value_storage(document, 0, &mut 0, limits)?;
    let mut metadata = false;
    for name in ["extensionsUsed", "extensionsRequired"] {
        if let Some(declarations) = document.get(name) {
            let declarations = declarations
                .as_array()
                .ok_or_else(|| invalid("generated vector declarations shape"))?;
            for declaration in declarations {
                let declaration = declaration
                    .as_str()
                    .ok_or_else(|| invalid("generated vector declaration shape"))?;
                if !matches!(
                    declaration,
                    "EXT_mesh_features"
                        | "EXT_structural_metadata"
                        | "EXT_mesh_polygon"
                        | "KHR_mesh_primitive_restart"
                        | "KHR_mesh_quantization"
                ) {
                    return Err(FormatError::Unsupported(
                        "generated vector declaration outside profile".into(),
                    ));
                }
                metadata |= declaration == "EXT_structural_metadata";
            }
        }
    }
    let views = document
        .get("bufferViews")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("generated vector views shape"))?;
    let accessors = document
        .get("accessors")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("generated vector accessors shape"))?;
    if views.len() > limits.buffer_views || accessors.len() > limits.accessors {
        return Err(resource("vector bridge cardinality limit"));
    }
    let unsigned = |value: Option<&Value>| -> Result<usize, FormatError> {
        value
            .and_then(Value::as_u64)
            .and_then(|n| usize::try_from(n).ok())
            .ok_or_else(|| invalid("generated vector range dimension"))
    };
    let buffers = document
        .get("buffers")
        .and_then(Value::as_array)
        .filter(|buffers| buffers.len() == 1)
        .ok_or_else(|| invalid("generated vector buffer shape"))?;
    let buffer = buffers[0]
        .as_object()
        .ok_or_else(|| invalid("generated vector buffer shape"))?;
    if buffer.contains_key("uri") || unsigned(buffer.get("byteLength"))? != binary.len() {
        return Err(invalid("generated vector buffer length or resource"));
    }
    let mut logical_bytes = 0usize;
    for view in views {
        if unsigned(view.get("buffer"))? != 0 {
            return Err(invalid("generated vector source buffer"));
        }
        let offset = view
            .get("byteOffset")
            .map_or(Ok(0), |v| unsigned(Some(v)))?;
        let length = unsigned(view.get("byteLength"))?;
        let end = offset
            .checked_add(length)
            .ok_or_else(|| invalid("generated vector view range overflow"))?;
        if length == 0 || end > binary.len() || offset % 8 != 0 {
            return Err(invalid("generated vector view range or alignment"));
        }
        logical_bytes = add_storage(logical_bytes, length)?;
        if logical_bytes > limits.logical_bytes {
            return Err(resource("vector bridge logical byte limit"));
        }
    }
    Ok(BridgeFacts {
        counts: GeneratedCounts {
            views: views.len(),
            accessors: accessors.len(),
            logical_bytes,
        },
        document_storage,
        metadata,
    })
}

fn bridge_fixed_storage() -> usize {
    std::mem::size_of::<ProducerPolicy>()
        + std::mem::size_of::<Value>()
        + std::mem::size_of::<Vec<u8>>()
        + std::mem::size_of::<BridgeFacts>()
        + std::mem::size_of::<codec::CompressionReceipt>()
        + 2 * std::mem::size_of::<&Attempt>()
}

#[cfg(test)]
#[derive(Clone, Copy)]
enum GeneratedFault {
    ViewRange,
    UnknownDeclaration,
}
#[cfg(test)]
thread_local! {
    static GENERATED_FAULT: std::cell::Cell<Option<GeneratedFault>> = const { std::cell::Cell::new(None) };
    static GENERATED_FAULT_REACHED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
#[cfg(test)]
fn with_generated_fault<T>(fault: GeneratedFault, f: impl FnOnce() -> T) -> (T, usize) {
    struct Reset(Option<GeneratedFault>);
    impl Drop for Reset {
        fn drop(&mut self) {
            GENERATED_FAULT.with(|slot| slot.set(self.0));
        }
    }
    let previous = GENERATED_FAULT.with(|slot| slot.replace(Some(fault)));
    let _reset = Reset(previous);
    GENERATED_FAULT_REACHED.with(|count| count.set(0));
    let result = f();
    (result, GENERATED_FAULT_REACHED.with(|count| count.get()))
}
#[cfg(test)]
fn fault_generated_document(mut document: Value) -> Value {
    GENERATED_FAULT.with(|slot| {
        if let Some(fault) = slot.get() {
            GENERATED_FAULT_REACHED.with(|count| count.set(count.get() + 1));
            match fault {
                GeneratedFault::ViewRange => {
                    document["bufferViews"][0]["byteLength"] = json!(u64::MAX)
                }
                GeneratedFault::UnknownDeclaration => {
                    document["extensionsUsed"] = json!(["VENDOR_unknown"])
                }
            }
        }
    });
    document
}

/// The checked bridge source shared by the writer and the counted fitting path:
/// generated-profile facts, declared metadata padding and retained storage.
fn bridge_source(
    document: Value,
    binary: &[u8],
    capacity: usize,
    policy: ProducerPolicy,
    attempt: &Attempt,
) -> Result<(Value, BridgeFacts, usize), Error> {
    attempt
        .check()
        .map_err(|cause| codec_failure(CodecError::Checkpoint(cause), attempt))?;
    #[cfg(test)]
    let document = fault_generated_document(document);
    let facts = bridge_facts(&document, binary, &policy.limits.values)
        .map_err(|error| codec_failure(CodecError::Format(error), attempt))?;
    let mut document = document;
    if facts.metadata {
        // Selected8 framing emits these zeros already. Declare them as buffer
        // data: published glTF permits at most3 undeclared BIN padding bytes.
        // The original existing slot and source length were checked above.
        let byte_length = document
            .get_mut("buffers")
            .and_then(Value::as_array_mut)
            .and_then(|buffers| buffers.first_mut())
            .and_then(Value::as_object_mut)
            .and_then(|buffer| buffer.get_mut("byteLength"))
            .ok_or_else(|| {
                codec_failure(
                    CodecError::Format(invalid("generated vector buffer slot")),
                    attempt,
                )
            })?;
        *byte_length = Value::from(
            add_storage(binary.len(), 7)
                .map_err(|error| codec_failure(CodecError::Format(error), attempt))?
                & !7,
        );
    }
    let retained = add_storage(facts.document_storage, capacity)
        .and_then(|n| add_storage(n, bridge_fixed_storage()))
        .map_err(|error| codec_failure(CodecError::Format(error), attempt))?;
    Ok((document, facts, retained))
}

/// Emit the actual selected-framed raw source once. Old Value/BIN owners drop
/// before codec preparation. Caller feature/model/worker state H is outside
/// this codec-owner estimate; duplicate emit geometry has already been dropped.
fn bridge(
    document: Value,
    binary: Vec<u8>,
    policy: ProducerPolicy,
    attempt: &Attempt,
) -> Result<(Vec<u8>, usize), Error> {
    let mut checkpoint = || attempt.check();
    let (document, facts, retained) =
        bridge_source(document, &binary, binary.capacity(), policy, attempt)?;
    let raw = codec::write_generated_glb(
        &document,
        &binary,
        facts.metadata,
        facts.counts,
        retained,
        &policy.limits,
        &mut checkpoint,
    )
    .map_err(|error| codec_failure(error, attempt))?;
    let before_bytes = raw.bytes.len();
    drop(document);
    drop(binary);
    // There is one owning branch; raw output does not allocate a Plan or streams.
    let bytes = if policy.compress {
        let retained = add_storage(raw.bytes.capacity(), bridge_fixed_storage())
            .map_err(|error| codec_failure(CodecError::Format(error), attempt))?;
        let plan = codec::prepare(&raw.bytes, retained, &policy.limits, &mut checkpoint)
            .map_err(|error| codec_failure(error, attempt))?;
        let encoded =
            codec::encode(plan, &mut checkpoint).map_err(|error| codec_failure(error, attempt))?;
        debug_assert_eq!(encoded.receipt.before_bytes, before_bytes);
        encoded.bytes
    } else {
        raw.bytes
    };
    Ok((bytes, before_bytes))
}

/// The exact length `bridge` returns for an uncompressed policy, with the same
/// refusals, without writing the GLB or re-admitting its JSON.
fn bridge_len(
    document: Value,
    binary: Vec<u8>,
    policy: ProducerPolicy,
    attempt: &Attempt,
) -> Result<usize, Error> {
    debug_assert!(!policy.compress);
    let mut checkpoint = || attempt.check();
    let (document, facts, retained) =
        bridge_source(document, &binary, binary.capacity(), policy, attempt)?;
    codec::generated_glb_len(
        &document,
        binary.len(),
        facts.metadata,
        facts.counts,
        retained,
        &policy.limits,
        &mut checkpoint,
    )
    .map_err(|error| codec_failure(error, attempt))
}

/// Quantization on the frame lattice: per axis, the points `k * 2^e` of the
/// local vector frame (node translation `k_lo * 2^e - center`, scale
/// `65535 * 2^e`, both before the forest adds the tile offset).
///
/// A vertex that more than one spool row holds (fragments of split polygons
/// and lines, borders between features) can be drawn by several tiles at
/// any depth. A two-pass build makes it decode identically wherever two of
/// them can be drawn together: the survey pass encodes every content on its
/// own range, as admitted, and records each accepted tile's finest
/// power-of-two exponent against its shared vertices. Tiles that hold a
/// vertex and can render together (neither is the other's ancestor, as
/// REPLACE refinement never draws both) are joined; each joined group snaps
/// that vertex at the largest exponent among its tiles. The final pass
/// replays the same hierarchy and snaps each shared vertex, from its exact
/// source position, at its group's exponent: a multiple of every member's
/// step.
///
/// A u16 window holds 65,536 steps, so a tile spanning a large area (a
/// fragment with long triangles) has a coarse step, and every tile that
/// shares a vertex with it would inherit that step. Instead, a tile whose
/// own step is coarser, on any axis, than that of a tile it can render
/// together with and shares a vertex with stores float32 lattice values
/// ([`Storage::Float32`], 2^24 steps, so 2^8 times finer). The u16 tiles
/// of a group then all have the same step, which the group snaps at, so
/// each keeps its own step unless a float32 member's finest step is coarser
/// still (more than 2^8 times coarser than the u16 tile): the group then
/// snaps there, and the measured bound includes it.
///
/// The tile's other vertices keep its own finest step. Deriving the range
/// per payload instead rounds a shared vertex differently on each side of a
/// seam (cracks).
pub(super) mod lattice {
    use super::*;
    /// Lowest exponent: keeps `65535 * 2^e` and every float32 lattice value
    /// normal.
    const MIN_EXPONENT: i32 = -120;
    /// How a payload on the lattice stores its positions.
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub(in super::super) enum Storage {
        /// Normalized u16 codes (`KHR_mesh_quantization`).
        U16,
        /// Float32 multiples of the step: every integer up to 2^24 is exact.
        Float32,
    }
    impl Storage {
        /// Lattice indices one window spans (its highest index).
        pub fn window(self) -> i64 {
            match self {
                Self::U16 => 65535,
                Self::Float32 => 1 << 24,
            }
        }
    }
    /// Per axis, the finest exponent whose `storage` window (with one cell
    /// of rounding slack) spans `lo..hi`, with every index exact in f64.
    pub(in super::super) fn enclosing(lo: Point, hi: Point, storage: Storage) -> [i32; 3] {
        let cells = (storage.window() - 1) as f64;
        std::array::from_fn(|i| {
            let span = hi[i] - lo[i];
            let magnitude = lo[i].abs().max(hi[i].abs());
            // |k| < 2^53 so k * 2^e and the index arithmetic are exact; a
            // zero-span axis (every value equal) then decodes exactly.
            let mut e = if magnitude > 0. && magnitude.is_finite() {
                magnitude.log2().floor() as i32 - 52
            } else {
                MIN_EXPONENT
            };
            if span > 0. && span.is_finite() {
                e = e.max((span / cells).log2().floor() as i32 - 1);
                while cells * 2f64.powi(e) < span {
                    e += 1;
                }
            }
            e.max(MIN_EXPONENT)
        })
    }
    /// One payload's exponent and lowest index per axis. It starts at the
    /// payload's finest exponent and doubles until every position and every
    /// line-strip end mirror `2c - n` fits one `storage` window.
    /// `shared[b][v]` gives a vertex's recorded exponents: it snaps there
    /// first.
    pub(in super::super) fn place(
        sources: &[&[Point]],
        shared: &[Vec<Option<[i32; 3]>>],
        ends: &[(usize, u32, u32)],
        storage: Storage,
    ) -> Placement {
        let all = || sources.iter().flat_map(|s| s.iter());
        let own = bounds(all())
            .map(|(lo, hi)| enclosing(lo, hi, storage))
            .unwrap_or([0; 3]);
        let mut placement = Placement {
            exponent: own,
            low: [0; 3],
        };
        for i in 0..3 {
            loop {
                let index = |b: usize, v: usize| {
                    let snap = shared.get(b).and_then(|s| s[v]).map(|e| e[i]);
                    placement.index(sources[b][v][i], snap, i)
                };
                let mut low = i64::MAX;
                let mut high = i64::MIN;
                for (b, points) in sources.iter().enumerate() {
                    for v in 0..points.len() {
                        let k = index(b, v);
                        low = low.min(k);
                        high = high.max(k);
                    }
                }
                for (b, c, n) in ends {
                    let k = 2 * index(*b, *c as usize) - index(*b, *n as usize);
                    low = low.min(k);
                    high = high.max(k);
                }
                if low > high || high - low <= storage.window() {
                    placement.low[i] = if low > high { 0 } else { low };
                    break;
                }
                placement.exponent[i] += 1;
            }
        }
        placement
    }
    pub(in super::super) struct Placement {
        pub exponent: [i32; 3],
        pub low: [i64; 3],
    }
    impl Placement {
        /// The (absolute) lattice index of one coordinate at this payload's
        /// step; a shared vertex first snaps at its recorded exponent.
        pub fn index(&self, v: f64, snap: Option<i32>, axis: usize) -> i64 {
            let e = self.exponent[axis];
            match snap {
                // Exact when snap >= e: a multiple of 2^snap is one of 2^e.
                Some(snap) => lattice_index(lattice_index(v, snap) as f64 * 2f64.powi(snap), e),
                None => lattice_index(v, e),
            }
        }
    }
}

use lattice::Storage;
/// How a payload quantizes its positions.
#[derive(Clone, Copy)]
pub(super) enum Quantize<'a> {
    /// Its own range, as admitted (fitting, the survey pass, and payloads
    /// without shared vertices).
    Own,
    /// The frame lattice, stored as `Storage`; shared vertices snap at
    /// their recorded exponents.
    Shared(&'a BTreeMap<(Cell, Kind), [i32; 3]>, Storage),
}
/// Each position accessor's values: normalized u16 codes, or float32
/// lattice values.
enum Codes {
    U16(Vec<Vec<[u16; 3]>>),
    Float32(Vec<Vec<[f32; 3]>>),
}
/// A payload's finest lattice exponents on its own range: with u16 codes
/// (strip-end mirrors included) and with float32 values.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct OwnExponents {
    pub u16: [i32; 3],
    pub float: [i32; 3],
}
/// The nearest lattice index (ties to even): `v / 2^e` is exact.
fn lattice_index(v: f64, exponent: i32) -> i64 {
    (v * 2f64.powi(-exponent)).round_ties_even() as i64
}
/// `(batch, end, neighbour)` for both ends of every line strip.
fn strip_ends(batch: usize, indices: &[u32], ends: &mut Vec<(usize, u32, u32)>) {
    for strip in indices.split(|i| *i == u32::MAX).filter(|s| s.len() >= 2) {
        ends.push((batch, strip[0], strip[1]));
        ends.push((batch, strip[strip.len() - 1], strip[strip.len() - 2]));
    }
}

#[derive(Default)]
struct Batch {
    points: Vec<Point>,
    /// The exact source (frame) position of each point, for the shared grid.
    sources: Vec<Point>,
    ids: Vec<usize>,
    indices: Vec<u32>,
    loops: Vec<u32>,
    triangles: Vec<u32>,
    loop_offsets: Vec<u32>,
}
struct Encoded {
    bytes: Vec<u8>,
    vertices: usize,
    primitives: usize,
    rounding: f64,
    quantization: f64,
    /// This payload's finest own lattice exponents (quantized only).
    survey: Option<OwnExponents>,
    /// A shared vertex was re-rounded at a coarser step than recorded.
    inexact: bool,
    before_bytes: usize,
}
pub(super) struct Candidate {
    pub node: Option<records::VectorRecord>,
    pub reports: Vec<Value>,
    pub reason: Option<&'static str>,
    pub worker: usize,
    pub survey: Option<Survey>,
}
fn view(glb: &mut MetadataGlb, bytes: &[u8]) -> usize {
    glb.view(if bytes.is_empty() { &[0] } else { bytes })
}
fn u32_accessor(glb: &mut MetadataGlb, values: &[u32]) -> usize {
    let bytes: Vec<_> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
    let v = view(glb, &bytes);
    glb.accessor(json!({"bufferView":v,"componentType":5125,"count":values.len(),"type":"SCALAR"}))
}
/// An EXT_mesh_polygon accessor: UNSIGNED_SHORT when every value fits below
/// the 16-bit restart value, otherwise UNSIGNED_INT. A u32 restart value
/// (`u32::MAX`) becomes the 16-bit restart value 65535.
fn polygon_accessor(glb: &mut MetadataGlb, values: &[u32]) -> usize {
    if values
        .iter()
        .any(|v| *v != u32::MAX && *v >= u32::from(u16::MAX))
    {
        return u32_accessor(glb, values);
    }
    let bytes: Vec<_> = values
        .iter()
        .flat_map(|v| (if *v == u32::MAX { u16::MAX } else { *v as u16 }).to_le_bytes())
        .collect();
    let v = view(glb, &bytes);
    glb.accessor(json!({"bufferView":v,"componentType":5123,"count":values.len(),"type":"SCALAR"}))
}
/// CesiumJS 1.146.0 (`appendBufferPolygons`) sizes its vertex remap by the
/// loopIndices count but indexes it by triangle vertex number, so a triangle
/// corner numbered at or beyond that count silently loses its triangle.
/// `polygon_for` refuses a polygon with a vertex outside its loops when the
/// feature is admitted, and `simplify` re-runs it in the parent tile's frame,
/// keeping the source polygon when it refuses a simplified one. Each polygon or
/// fill triangle owns its vertices, so the loop list (with restarts) is never
/// shorter than the vertex list here. This is the final guard before publication.
fn cesium_loop_remap_fits(vertices: usize, indices: &[u32], loops: &[u32]) -> FeatureResult<()> {
    if vertices > loops.len() || indices.iter().any(|i| *i as usize >= loops.len()) {
        return Err(FeatureFailure::reject(
            "polygon vertices outnumber their loop indices; CesiumJS would drop triangles",
        ));
    }
    Ok(())
}
fn metadata(
    glb: &mut MetadataGlb,
    items: &[&Feature],
    schemas: &BTreeMap<String, String>,
) -> Result<(), Error> {
    let aggregate = items
        .iter()
        .all(|f| !f.properties.contains_key("_source_id"));
    crate::metadata::encode_property_table(
        glb,
        &items.iter().map(|f| &f.properties).collect::<Vec<_>>(),
        schemas,
        "rusty_tiles_vector",
        if aggregate {
            "pointAggregate"
        } else {
            "feature"
        },
        if aggregate {
            "pointAggregates"
        } else {
            "features"
        },
    )
}

// Keep geometry, schema and report inputs explicit alongside the once-validated
// codec policy and the borrowed original Attempt. They have distinct owners.
#[allow(clippy::too_many_arguments)]
fn emit(
    items: &[&Feature],
    center: Point,
    quantize: Quantize<'_>,
    repair: bool,
    schemas: &BTreeMap<String, String>,
    options: &VectorOptions,
    reports: &mut Vec<Value>,
    policy: ProducerPolicy,
    attempt: &Attempt,
) -> FeatureResult<Encoded> {
    let source = emit_source(
        items, center, quantize, repair, schemas, options, reports, attempt,
    )?;
    let (bytes, before_bytes) = bridge(source.document, source.binary, policy, attempt)?;
    Ok(Encoded {
        bytes,
        vertices: source.vertices,
        primitives: source.primitives,
        rounding: source.rounding,
        quantization: source.quantization,
        survey: source.survey,
        inexact: source.inexact,
        before_bytes,
    })
}

/// The generated glTF document and BIN of one tile, before the codec bridge.
struct Source {
    document: Value,
    binary: Vec<u8>,
    vertices: usize,
    primitives: usize,
    rounding: f64,
    quantization: f64,
    survey: Option<OwnExponents>,
    inexact: bool,
}
/// See [`Quantize`] for how quantized positions are placed.
#[allow(clippy::too_many_arguments)]
fn emit_source(
    items: &[&Feature],
    center: Point,
    quantize: Quantize<'_>,
    repair: bool,
    schemas: &BTreeMap<String, String>,
    options: &VectorOptions,
    reports: &mut Vec<Value>,
    attempt: &Attempt,
) -> FeatureResult<Source> {
    attempt
        .check()
        .map_err(|cause| codec_failure(CodecError::Checkpoint(cause), attempt))?;
    if items.len() > 16777217 {
        return Err(FeatureFailure::reject(
            "too many exact feature IDs in one tile",
        ));
    }
    let mut glb = MetadataGlb::new("rusty-tiles native vector");
    glb.document["extensionsUsed"] = json!(["EXT_mesh_features", "EXT_structural_metadata"]);
    glb.document["meshes"] = json!([{"primitives":[]}]);
    metadata(&mut glb, items, schemas)?;
    let mut batches: BTreeMap<u32, Batch> = BTreeMap::new();
    let mut line_restart = false;
    let mut all_positions = Vec::new();
    for (fid, feature) in items.iter().enumerate() {
        let mut parts = Vec::new();
        match &feature.geometry {
            Geometry::Point(p) => parts.push((0, vec![*p], None, None)),
            Geometry::MultiPoint(p) => parts.push((0, p.clone(), None, None)),
            Geometry::LineString(p) => parts.push((3, p.clone(), None, None)),
            Geometry::MultiLineString(p) => {
                parts.extend(p.iter().map(|p| (3, p.clone(), None, None)))
            }
            geometry => {
                let polygons = match geometry {
                    Geometry::Polygon(r) => vec![r],
                    Geometry::MultiPolygon(p) => p.iter().collect(),
                    _ => unreachable!(),
                };
                for (index, rings) in polygons.into_iter().enumerate() {
                    // Each polygon vertex is one of these local ring points;
                    // the shared grid snaps its exact source position.
                    let mut origin = BTreeMap::new();
                    let local: Vec<Vec<_>> = rings
                        .iter()
                        .map(|r| {
                            r.iter()
                                .map(|p| {
                                    let local = sub(*p, center);
                                    origin.entry(key(local)).or_insert(*p);
                                    local
                                })
                                .collect()
                        })
                        .collect();
                    let polygon = geometry::polygon_for(feature, &local, index, center, repair)?;
                    let mut report = polygon.report.clone();
                    report["sourceId"] = feature.source_id().clone();
                    reports.push(report);
                    let sources = polygon
                        .positions
                        .iter()
                        .map(|p| origin.get(&key(*p)).copied().unwrap_or(add(*p, center)))
                        .collect();
                    parts.push((
                        4,
                        polygon.positions.iter().map(|p| add(*p, center)).collect(),
                        Some(sources),
                        Some(polygon),
                    ));
                }
            }
        }
        for (mode, positions, sources, polygon) in parts {
            let points: Vec<_> = positions.iter().map(|p| sub(*p, center)).collect();
            let batch = batches.entry(mode).or_default();
            let base = u32::try_from(batch.points.len())
                .map_err(|_| FeatureFailure::reject("tile vertex index overflow"))?;
            if mode == 3 && !batch.indices.is_empty() {
                batch.indices.push(u32::MAX);
                line_restart = true;
            }
            if let Some(polygon) = polygon {
                // Every polygon, including each triangle of a fragmented
                // fill, is one EXT_mesh_polygon polygon with its own loops.
                if !batch.loops.is_empty() {
                    batch.loops.push(u32::MAX);
                }
                batch.triangles.extend(
                    polygon
                        .triangle_offsets
                        .iter()
                        .map(|offset| batch.indices.len() as u32 + offset),
                );
                batch.loop_offsets.extend(
                    polygon
                        .loop_offsets
                        .iter()
                        .map(|offset| batch.loops.len() as u32 + offset),
                );
                batch.loops.extend(polygon.loops.iter().map(|i| {
                    if *i == u32::MAX {
                        *i
                    } else {
                        base + i
                    }
                }));
                batch
                    .indices
                    .extend(polygon.indices.iter().map(|i| base + i));
            } else {
                batch
                    .indices
                    .extend((0..points.len() as u32).map(|i| base + i));
            }
            batch.ids.extend(std::iter::repeat_n(fid, points.len()));
            batch.points.extend_from_slice(&points);
            batch.sources.extend(sources.unwrap_or(positions));
            all_positions.extend(points);
        }
    }
    if line_restart {
        glb::add_extension(&mut glb.document, "KHR_mesh_primitive_restart", true)?;
    }
    let mut position_accessors = Vec::new();
    let mut accessor_modes = Vec::new();
    let mut line_ends = Vec::new();
    for (mode, batch) in &batches {
        let mut ext = json!({"EXT_mesh_features":crate::metadata::MeshFeatures::attribute(items.len(), 0, Some(0))});
        if *mode == 4 {
            cesium_loop_remap_fits(batch.points.len(), &batch.indices, &batch.loops)?;
            ext["EXT_mesh_polygon"] = json!({"count":batch.triangles.len(),"indicesOffsets":polygon_accessor(&mut glb,&batch.triangles),
                "loopIndices":polygon_accessor(&mut glb,&batch.loops),"loopIndicesOffsets":polygon_accessor(&mut glb,&batch.loop_offsets)});
            glb::add_extension(&mut glb.document, "EXT_mesh_polygon", false)?;
        }
        let values: Vec<Point> = batch
            .points
            .iter()
            .map(|p| p.map(|v| v as f32 as f64))
            .collect();
        if !values.iter().flatten().all(|v| v.is_finite()) {
            return Err(FeatureFailure::reject("positions exceed float32 range"));
        }
        let bytes: Vec<_> = values
            .iter()
            .flat_map(|p| p.iter().flat_map(|v| (*v as f32).to_le_bytes()))
            .collect();
        let position_view = view(&mut glb, &bytes);
        let (lo, hi) = bounds(values.iter())?;
        if *mode == 3 {
            strip_ends(position_accessors.len(), &batch.indices, &mut line_ends);
        }
        let position=glb.accessor(json!({"bufferView":position_view,"componentType":5126,"count":values.len(),"type":"VEC3","min":lo,"max":hi}));
        position_accessors.push((position, position_view, values, &batch.sources));
        accessor_modes.push(*mode);
        let mut ids = Vec::new();
        for id in &batch.ids {
            if items.len() <= 65536 {
                ids.extend((*id as u16).to_le_bytes());
                ids.extend(0u16.to_le_bytes());
            } else {
                ids.extend((*id as f32).to_le_bytes());
            }
        }
        let id_view = view(&mut glb, &ids);
        if items.len() <= 65536 {
            glb.document["bufferViews"][id_view]["byteStride"] = json!(4);
        }
        let id_accessor=glb.accessor(json!({"bufferView":id_view,"componentType":if items.len()<=65536 {5123}else{5126},"count":batch.ids.len(),"type":"SCALAR"}));
        let index = u32_accessor(&mut glb, &batch.indices);
        glb.document["meshes"][0]["primitives"].as_array_mut().unwrap().push(json!({"mode":mode,"attributes":{"POSITION":position,"_FEATURE_ID_0":id_accessor},"indices":index,"extensions":ext}));
    }
    let rounding = all_positions
        .iter()
        .map(|p| norm(sub(*p, p.map(|v| v as f32 as f64))))
        .fold(0., f64::max);
    let mut quantization: f64 = 0.;
    let mut survey = None;
    let mut inexact = false;
    if options.quantize {
        // The finest lattice exponent of this payload on its own (exact
        // positions and strip-end mirrors): what the survey pass records.
        let sources: Vec<&[Point]> = position_accessors
            .iter()
            .map(|(_, _, _, s)| s.as_slice())
            .collect();
        survey = Some(OwnExponents {
            u16: lattice::place(&sources, &[], &line_ends, Storage::U16).exponent,
            float: lattice::place(&sources, &[], &[], Storage::Float32).exponent,
        });
        // Workaround for a CesiumJS bug (seen in 1.146,
        // renderBufferPolylineCollection.js): a line strip's missing end
        // neighbour is synthesised as 2c-n and stored in prev/next buffers of
        // the position component type, so with u16 positions an end on the
        // range edge wraps (e.g. -65535 -> 1) into a sheared cap or long
        // spike. Both u16 ranges below keep every end's mirror in [0, 65535]
        // (float32 positions do not wrap).
        //
        // `lo`, `scale`, half a step per axis, and each accessor's codes.
        let (lo, scale, half_step, codes): (Point, Point, Point, Codes) = match quantize {
            Quantize::Shared(exponents, storage) => {
                // The frame lattice, from exact source positions; the window
                // holds the written mirrors 2c-n exactly.
                let coarse: Vec<Vec<Option<[i32; 3]>>> = sources
                    .iter()
                    .zip(&accessor_modes)
                    .map(|(points, mode)| {
                        let kind = Kind::of_mode(*mode);
                        points
                            .iter()
                            .map(|p| kind.and_then(|k| exponents.get(&(cell(*p), k)).copied()))
                            .collect()
                    })
                    .collect();
                let ends: &[_] = match storage {
                    Storage::U16 => &line_ends,
                    Storage::Float32 => &[],
                };
                let placement = lattice::place(&sources, &coarse, ends, storage);
                // A shared vertex recorded finer than this payload's final
                // step (its snapped positions widened the window) is
                // re-rounded here: that seam is not exact.
                inexact = coarse
                    .iter()
                    .flatten()
                    .flatten()
                    .any(|e| (0..3).any(|i| e[i] < placement.exponent[i]));
                let step: Point = placement.exponent.map(|e| 2f64.powi(e));
                // Exact: |k_lo| < 2^53 and the step is a power of two.
                let lo = std::array::from_fn(|i| placement.low[i] as f64 * step[i] - center[i]);
                // Window offsets: `place` sized the window from these same indices.
                let offsets = sources.iter().zip(&coarse).map(|(points, coarse)| {
                    points.iter().zip(coarse).map(|(p, snap)| -> [i64; 3] {
                        std::array::from_fn(|i| {
                            placement.index(p[i], snap.map(|e| e[i]), i) - placement.low[i]
                        })
                    })
                });
                let codes = match storage {
                    Storage::U16 => Codes::U16(
                        offsets
                            .map(|points| {
                                points
                                    .map(|k| {
                                        k.map(|k| {
                                            u16::try_from(k)
                                                .expect("lattice index inside its u16 window")
                                        })
                                    })
                                    .collect()
                            })
                            .collect(),
                    ),
                    // Exact: an integer below 2^24 times a power of two
                    // no smaller than 2^MIN_EXPONENT. Checked all the same:
                    // a value that float32 rounds would open the seam.
                    Storage::Float32 => {
                        let mut values = Vec::new();
                        for points in offsets {
                            let mut accessor = Vec::new();
                            for k in points {
                                let mut value = [0f32; 3];
                                for i in 0..3 {
                                    let exact = k[i] as f64 * step[i];
                                    if k[i] > Storage::Float32.window()
                                        || f64::from(exact as f32) != exact
                                    {
                                        return Err(FeatureFailure::reject(
                                            "lattice position not exact in float32",
                                        ));
                                    }
                                    value[i] = exact as f32;
                                }
                                accessor.push(value);
                            }
                            values.push(accessor);
                        }
                        Codes::Float32(values)
                    }
                };
                let scale = match storage {
                    Storage::U16 => step.map(|s| s * 65535.),
                    Storage::Float32 => [1.; 3],
                };
                (lo, scale, step.map(|s| s / 2.), codes)
            }
            Quantize::Own => {
                // This payload's own range (as admitted), padded by the
                // strip-end reach plus two
                // quanta (rounding of c and n):
                // pad >= reach + 2 (extent + 2 pad) / 65535.
                let values = || position_accessors.iter().flat_map(|(_, _, p, _)| p.iter());
                let (lo, hi) = bounds(values())?;
                let mut reach = [0f64; 3];
                for (batch, c, n) in &line_ends {
                    let points = &position_accessors[*batch].2;
                    for i in 0..3 {
                        reach[i] =
                            reach[i].max((points[*c as usize][i] - points[*n as usize][i]).abs());
                    }
                }
                let pad: Point = std::array::from_fn(|i| {
                    if reach[i] > 0. {
                        (reach[i] + 2. * (hi[i] - lo[i]) / 65535.) / (1. - 4. / 65535.)
                    } else {
                        0.
                    }
                });
                // Outward to float32 so the node translation stays exact and short.
                let lo: Point = std::array::from_fn(|i| match pad[i] > 0. {
                    true => ((lo[i] - pad[i]) as f32).next_down() as f64,
                    false => lo[i],
                });
                let hi: Point = std::array::from_fn(|i| match pad[i] > 0. {
                    true => ((hi[i] + pad[i]) as f32).next_up() as f64,
                    false => hi[i],
                });
                let extent = sub(hi, lo);
                let scale = extent.map(|v| if v > 0. { v } else { 1. });
                let codes = position_accessors
                    .iter()
                    .map(|(_, _, points, _)| {
                        points
                            .iter()
                            .map(|p| {
                                std::array::from_fn(|i| {
                                    ((p[i] - lo[i]) / scale[i] * 65535.)
                                        .round_ties_even()
                                        .clamp(0., 65535.)
                                        as u16
                                })
                            })
                            .collect()
                    })
                    .collect();
                (lo, scale, mul(extent, 1. / 131070.), Codes::U16(codes))
            }
        };
        // From the position that was snapped: exact source on the build's
        // lattice, the float32 position on a payload range.
        let snapped = |values: &[Point], sources: &[Point], v: usize| match quantize {
            Quantize::Shared(..) => sub(sources[v], center),
            Quantize::Own => values[v],
        };
        let mut largest: f64 = 0.;
        match &codes {
            Codes::U16(codes) => {
                for ((accessor, view, values, sources), codes) in
                    position_accessors.iter().zip(codes)
                {
                    let mut bytes = Vec::new();
                    let mut low = [u16::MAX; 3];
                    let mut high = [0u16; 3];
                    for (v, q) in codes.iter().enumerate() {
                        let decoded: Point =
                            std::array::from_fn(|i| q[i] as f64 / 65535. * scale[i] + lo[i]);
                        quantization =
                            quantization.max(norm(sub(decoded, snapped(values, sources, v))));
                        for i in 0..3 {
                            low[i] = low[i].min(q[i]);
                            high[i] = high[i].max(q[i]);
                            bytes.extend(q[i].to_le_bytes());
                        }
                        bytes.extend(0u16.to_le_bytes());
                    }
                    glb.replace_view(*view, &bytes);
                    glb.document["bufferViews"][*view]["byteStride"] = json!(8);
                    let ac = &mut glb.document["accessors"][*accessor];
                    ac["componentType"] = json!(5123);
                    ac["normalized"] = json!(true);
                    ac["min"] = json!(low);
                    ac["max"] = json!(high);
                }
                glb.document["nodes"][0]["scale"] = json!(scale);
                glb::add_extension(&mut glb.document, "KHR_mesh_quantization", true)?;
            }
            Codes::Float32(codes) => {
                for ((accessor, view, values, sources), codes) in
                    position_accessors.iter().zip(codes)
                {
                    let mut bytes = Vec::new();
                    let mut low = [f32::INFINITY; 3];
                    let mut high = [f32::NEG_INFINITY; 3];
                    for (v, q) in codes.iter().enumerate() {
                        let decoded: Point = std::array::from_fn(|i| f64::from(q[i]) + lo[i]);
                        quantization =
                            quantization.max(norm(sub(decoded, snapped(values, sources, v))));
                        largest = largest.max(norm(q.map(f64::from)));
                        for i in 0..3 {
                            low[i] = low[i].min(q[i]);
                            high[i] = high[i].max(q[i]);
                            bytes.extend(q[i].to_le_bytes());
                        }
                    }
                    glb.replace_view(*view, &bytes);
                    let ac = &mut glb.document["accessors"][*accessor];
                    ac["min"] = json!(low);
                    ac["max"] = json!(high);
                }
            }
        }
        glb.compact_views();
        glb.document["nodes"][0]["translation"] = json!(lo);
        // Source-to-decoded displacement (measured, and at least half a
        // step), plus the float32 placement allowance of a client that holds
        // the node translation, scale and position in float32 and rounds the
        // normalization, product and sum. (`positionRoundingMetres`, still
        // added to the tile error, double-counts float32 rounding on the
        // shared lattice, which snaps exact source positions: conservative.)
        quantization = quantization.max(norm(half_step));
        quantization += norm(sub(lo, lo.map(|v| v as f32 as f64)));
        quantization += match codes {
            Codes::U16(_) => {
                norm(sub(scale, scale.map(|v| v as f32 as f64)))
                    + (norm(lo) + 3. * norm(scale)) * 2f64.powi(-24)
            }
            // Exact float32 values: only the sum rounds.
            Codes::Float32(_) => (norm(lo) + largest) * 2f64.powi(-24),
        };
    }
    let vertices = all_positions.len();
    let primitives = batches.len();
    // These duplicate geometry owners have served their numeric/report purpose.
    // They do not survive into raw-source emission or codec materialization.
    drop(position_accessors);
    drop(all_positions);
    drop(batches);
    let (document, binary) = glb.into_parts();
    Ok(Source {
        document,
        binary,
        vertices,
        primitives,
        rounding,
        quantization,
        survey,
        inexact,
    })
}

/// The feature's own metadata schema used by full-detail fitting.
fn feature_schemas(feature: &Feature) -> BTreeMap<String, String> {
    feature
        .properties
        .iter()
        .filter_map(|(name, value)| {
            let kind = if value.is_boolean() {
                "boolean"
            } else if value.is_i64() || value.is_u64() {
                "integer"
            } else if value.is_number() {
                "real"
            } else if value.is_string() {
                "string"
            } else {
                return None;
            };
            Some((name.clone(), kind.to_string()))
        })
        .collect()
}

/// Bytes held back from `maxBytes` for the JSON that implicit scene-root
/// placement adds to a content when the forest rebases it.
pub(super) const IMPLICIT_FRAMING_RESERVE: usize = 128;

/// Test the exact full-detail encodings before admitting a source fragment.
/// Later cross-feature schema changes remain a fatal hierarchy constraint.
/// Implicit scene framing uses the same reservation as final candidate encoding.
pub(super) fn fits_feature(
    feature: &Feature,
    repair: bool,
    options: &VectorOptions,
    policy: ProducerPolicy,
    attempt: &Attempt,
) -> FeatureResult<bool> {
    attempt
        .check()
        .map_err(|cause| codec_failure(CodecError::Checkpoint(cause), attempt))?;
    let (lo, hi) = bounds(feature.geometry.points())?;
    let center = mul(add(lo, hi), 0.5);
    let schemas = feature_schemas(feature);
    // A fill fragment and its polygon's outline are separate spool rows,
    // each fitted alone; a tile encodes them together in one GLB. Admission
    // precedes the build's shared grid (same u16 layout); final encoding
    // re-checks the budgets.
    let source = emit_source(
        &[feature],
        center,
        Quantize::Own,
        repair,
        &schemas,
        options,
        &mut Vec::new(),
        attempt,
    )?;
    let vertices = source.vertices;
    let bytes = if policy.compress {
        // The compressed length is only known by encoding.
        bridge(source.document, source.binary, policy, attempt)?
            .0
            .len()
    } else {
        bridge_len(source.document, source.binary, policy, attempt)?
    };
    Ok(vertices <= options.max_vertices
        && bytes <= options.max_bytes.saturating_sub(IMPLICIT_FRAMING_RESERVE))
}

/// Inputs shared by every candidate encoded in one build.
#[derive(Clone, Copy)]
pub(super) struct Encoder<'a> {
    pub spool: &'a Path,
    pub max_features: usize,
    pub repair: bool,
    pub options: &'a VectorOptions,
    pub schemas: &'a BTreeMap<String, String>,
    pub output: &'a Path,
    pub policy: ProducerPolicy,
    pub attempt: &'a Attempt,
    /// Which pass of a quantized build encodes this candidate.
    pub pass: GridPass,
}

/// The passes of a quantized build (see [`lattice`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum GridPass {
    /// Unquantized builds: one pass.
    Single,
    /// Own ranges; accepted tiles report their exponents and shared vertices.
    Survey,
    /// Shared vertices snap at the exponents recorded in the spool's `grid`.
    Final,
}

/// What the survey pass learns from one candidate: the tile (prefix and
/// level), its finest exponents and the vertices other spool rows also hold.
pub(super) struct Survey {
    pub prefix: String,
    pub level: u32,
    pub exponent: OwnExponents,
    pub shared: Vec<(Cell, Kind)>,
}

/// Shared vertices are identified by cell, 1/1024 m per axis, so points
/// equal up to rounding are one vertex, and by the kind of geometry drawing
/// them.
pub(super) type Cell = [i64; 3];
/// Fill (polygon) or line copies of a vertex: each must agree with its own
/// kind in other tiles. Points draw no seams.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(super) enum Kind {
    Fill = 0,
    Line = 1,
}
impl Kind {
    fn of(geometry: &Geometry) -> Option<Self> {
        match geometry {
            Geometry::Polygon(_) | Geometry::MultiPolygon(_) => Some(Self::Fill),
            Geometry::LineString(_) | Geometry::MultiLineString(_) => Some(Self::Line),
            _ => None,
        }
    }
    fn of_mode(mode: u32) -> Option<Self> {
        match mode {
            4 => Some(Self::Fill),
            3 => Some(Self::Line),
            _ => None,
        }
    }
}
const CELLS_PER_METRE: f64 = 1024.;
pub(super) fn cell(p: Point) -> Cell {
    p.map(|v| (v * CELLS_PER_METRE).round() as i64)
}

fn strip_holes(geometry: &mut Geometry) {
    match geometry {
        Geometry::Polygon(rings) => rings.truncate(1),
        Geometry::MultiPolygon(polygons) => polygons.iter_mut().for_each(|r| r.truncate(1)),
        _ => {}
    }
}

pub(super) fn encode(
    encoder: Encoder<'_>,
    prefix: &str,
    center: Point,
    level: u32,
    tolerance: f64,
) -> Result<Candidate, Error> {
    let Encoder {
        spool,
        max_features,
        repair,
        options,
        schemas,
        output,
        policy,
        attempt,
        pass,
    } = encoder;
    attempt
        .check()
        .map_err(|cause| codec_failure(CodecError::Checkpoint(cause), attempt))?;
    let db =
        rusqlite::Connection::open_with_flags(spool, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(sql)?;
    db.execute_batch("PRAGMA cache_size=-32768; PRAGMA temp_store=FILE;")
        .map_err(sql)?;
    let end = format!("{prefix}~");
    let cap = if level > 0 {
        options.max_parent_features
    } else {
        max_features
    };
    // A whole unsplit line or polygon whose box diagonal fits a coarse tolerance is
    // within the tile's error: the `max_features` largest become stand-ins, the rest
    // are omitted.
    let extent = "COALESCE((hx-lx)*(hx-lx)+(hy-ly)*(hy-ly)+(hz-lz)*(hz-lz),0)";
    let small = format!("COALESCE(?3>0 AND omissible AND {extent}<=?3*?3,0)");
    let tolerance = if level > 0 { tolerance } else { 0. };
    if !tolerance.is_finite() {
        return Err(data("vector LOD tolerance overflows; reduce lodTolerance"));
    }
    let range = rusqlite::params![prefix, end, if level > 0 { tolerance } else { -1. }];
    let (count, minor): (usize, usize) = db
        .query_row(
            &format!(
                "SELECT COUNT(*),COALESCE(SUM({small}),0) FROM features WHERE path>=?1 AND path<?2"
            ),
            range,
            |r| Ok((r.get::<_, i64>(0)? as usize, r.get::<_, i64>(1)? as usize)),
        )
        .map_err(sql)?;
    let count = count - minor + minor.min(max_features);
    let mut stand_ins = BTreeSet::new();
    if minor > 0 {
        let mut stmt = db.prepare(&format!("SELECT id FROM features WHERE path>=?1 AND path<?2 AND {small} ORDER BY {extent} DESC,id LIMIT {max_features}")).map_err(sql)?;
        let ids = stmt.query_map(range, |r| r.get::<_, i64>(0)).map_err(sql)?;
        for id in ids {
            stand_ins.insert(id.map_err(sql)?);
        }
    }
    let mut result = Candidate {
        node: None,
        reports: Vec::new(),
        reason: None,
        worker: rayon::current_thread_index().unwrap_or(0),
        survey: None,
    };
    let mut aggregation = None;
    let mut items = Vec::new();
    let mut aggregation_reason = None;
    if level > 0 && options.aggregate_points {
        match aggregation::collect(&db, prefix, tolerance, options)? {
            aggregation::Outcome::Ready(value) => {
                aggregation = Some(value.summary);
                items = value.features;
            }
            aggregation::Outcome::Rejected(reason) => aggregation_reason = Some(reason),
            aggregation::Outcome::Unchanged => {}
        }
    }
    if aggregation.is_none() && count > cap {
        result.reason = Some(aggregation_reason.unwrap_or(if level > 0 {
            "parentFeatures"
        } else {
            "features"
        }));
        return Ok(result);
    }
    let mut vertices = 0;
    let mut estimate = 0;
    // The requested tolerance also promotes coincident aggregates to their
    // original identities on near refinement, even when spatial error is zero.
    let mut error: f64 = if aggregation.is_some() { tolerance } else { 0. };
    let mut stmt = db
        .prepare(&format!(
            "SELECT data,id,{small},{extent} FROM features WHERE path>=?1 AND path<?2 ORDER BY id"
        ))
        .map_err(sql)?;
    let mut rows = stmt.query(range).map_err(sql)?;
    let mut lock_query = db
        .prepare_cached("SELECT shared FROM vertices WHERE x=?1 AND y=?2 AND z=?3")
        .map_err(sql)?;
    // Cells of vertices other spool rows also hold (or that hold another
    // spooled vertex): the only ones another tile can render, so the only
    // ones that must decode identically everywhere (quantization only).
    let mut shared = BTreeSet::new();
    let mut shared_here = BTreeSet::new();
    let mut coincident_query = db
        .prepare_cached("SELECT COUNT(*) FROM vertices WHERE x BETWEEN ?1 AND ?2 AND y BETWEEN ?3 AND ?4 AND z BETWEEN ?5 AND ?6")
        .map_err(sql)?;
    while aggregation.is_none() {
        let Some(row) = rows.next().map_err(sql)? else {
            break;
        };
        // Load one original at a time; retain only budgeted simplified candidates.
        let minor = row.get::<_, bool>(2).map_err(sql)?;
        let extent = row.get::<_, f64>(3).map_err(sql)?.sqrt();
        if minor && !stand_ins.contains(&row.get::<_, i64>(1).map_err(sql)?) {
            error = error.max(extent);
            continue;
        }
        let mut feature: Feature = serde_json::from_str(&row.get::<_, String>(0).map_err(sql)?)?;
        if minor {
            // A stand-in keeps its exterior rings, reduced as far as they validly go.
            strip_holes(&mut feature.geometry);
            if let Some(intrinsic) = &mut feature.intrinsic {
                strip_holes(&mut intrinsic.geometry);
            }
        }
        let mut locked = BTreeSet::new();
        for p in feature.geometry.points() {
            // Quantization needs every shared vertex, fragments included
            // (a fragment vertex without a row counts as shared).
            let is_shared = if !feature.surface_fragment || options.quantize {
                use rusqlite::OptionalExtension;
                match lock_query
                    .query_row(rusqlite::params![p[0], p[1], p[2]], |r| r.get::<_, i32>(0))
                    .optional()
                    .map_err(sql)?
                {
                    Some(flag) => flag != 0,
                    None if feature.surface_fragment => true,
                    None => return Err(sql(rusqlite::Error::QueryReturnedNoRows)),
                }
            } else {
                false
            };
            if is_shared || feature.surface_fragment {
                locked.insert(key(*p));
            }
            // Another spooled vertex in the same cell (e.g. the two sides of
            // an antimeridian cut, equal in 3D up to rounding) is drawn at
            // the same place: it must snap the same way too.
            let coincident = options.quantize && !is_shared && {
                let r = 1. / CELLS_PER_METRE;
                coincident_query
                    .query_row(
                        rusqlite::params![
                            p[0] - r,
                            p[0] + r,
                            p[1] - r,
                            p[1] + r,
                            p[2] - r,
                            p[2] + r
                        ],
                        |row| row.get::<_, i64>(0),
                    )
                    .map_err(sql)?
                    > 1
            };
            if options.quantize && (is_shared || coincident) {
                shared_here.insert(cell(*p));
            }
        }
        let mut staged = Vec::new();
        let feature = if level > 0 {
            let (simplified, e) = geometry::simplify(
                &feature,
                if minor { extent } else { tolerance },
                &locked,
                if minor {
                    &mut staged
                } else {
                    &mut result.reports
                },
                options.parent_repair,
                center,
                repair,
            )
            .map_err(FeatureFailure::into_error)?;
            if minor {
                // An unreduced stand-in is omitted unless the tile is empty; its diagonal counts.
                let size = simplified.geometry.size();
                let paths = feature.geometry.paths().len();
                if size >= feature.geometry.size() && size > 4 * paths && !items.is_empty() {
                    error = error.max(extent);
                    continue;
                }
                result.reports.append(&mut staged);
            }
            error = error.max(if minor { extent } else { e });
            simplified
        } else {
            feature
        };
        // Fill copies must meet fill copies, line copies line copies; a
        // line over a fill edge is not a crack.
        if let Some(kind) = Kind::of(&feature.geometry) {
            for p in feature.geometry.points() {
                let c = cell(*p);
                if shared_here.contains(&c) {
                    shared.insert((c, kind));
                }
            }
        }
        shared_here.clear();
        vertices += feature.geometry.size();
        estimate += feature.estimate() - if level > 0 { 2048 } else { 0 };
        if vertices > options.max_vertices {
            result.reason = Some("vertices");
            return Ok(result);
        }
        if estimate > options.max_bytes.saturating_mul(2) {
            result.reason = Some("estimatedBytes");
            return Ok(result);
        }
        items.push(feature);
    }
    // Fragmented fills, their polygons' outlines and every other feature
    // share one glTF vector content: each fill triangle is an EXT_mesh_polygon
    // polygon, so Cesium's vector decoder accepts the whole tile. The outline
    // pieces of one source feature in this tile are chained into maximal ring
    // runs, so a strip ends only where its ring leaves the tile.
    let (pieces, others): (Vec<&Feature>, Vec<&Feature>) =
        items.iter().partition(|f| outline::is_outline(f));
    let outlines = outline::fragment_outlines(pieces);
    let features: Vec<&Feature> = others.into_iter().chain(&outlines).collect();
    let mut reports = Vec::new();
    let aggregate_schemas = BTreeMap::new();
    // A vertex spooled in more than one row (fragments of split polygons
    // and lines, borders between features) can be rendered by other tiles,
    // at any depth. The final pass snaps those vertices at their recorded
    // exponents so they decode identically and fills meet without cracks;
    // every other vertex keeps this payload's own finest step.
    let byte_limit = options.max_bytes.saturating_sub(IMPLICIT_FRAMING_RESERVE);
    let metadata_schemas = if aggregation.is_some() {
        &aggregate_schemas
    } else {
        schemas
    };
    let mut exponents = BTreeMap::new();
    // Whether the resolved grid stores this tile as float32 (the same on
    // every row of the tile).
    let mut storage = Storage::U16;
    // Shared vertices the survey did not record for this tile (the spool's
    // shared set can grow during the survey): kept on this tile's own step.
    // A vertex no co-rendering tile holds (only ancestors or descendants)
    // needs no shared step: a tile with only such vertices keeps its own
    // range.
    let mut unrecorded = 0usize;
    if options.quantize && pass == GridPass::Final && !shared.is_empty() {
        let mut query = db
            .prepare_cached(
                "SELECT ex,ey,ez,fl,al FROM grid WHERE cx=?1 AND cy=?2 AND cz=?3 AND kind=?4 AND prefix=?5 AND level=?6",
            )
            .map_err(sql)?;
        for k in &shared {
            let (p, kind) = k;
            use rusqlite::OptionalExtension;
            if let Some((e, float, alone)) = query
                .query_row(
                    rusqlite::params![p[0], p[1], p[2], *kind as i64, prefix, level],
                    |r| {
                        Ok((
                            [r.get(0)?, r.get(1)?, r.get(2)?],
                            r.get::<_, bool>(3)?,
                            r.get::<_, bool>(4)?,
                        ))
                    },
                )
                .optional()
                .map_err(sql)?
            {
                if alone {
                    continue;
                }
                exponents.insert(*k, e);
                if float {
                    storage = Storage::Float32;
                }
            } else {
                unrecorded += 1;
            }
        }
    }
    let on_grid = !exponents.is_empty();
    let quantize = if on_grid {
        Quantize::Shared(&exponents, storage)
    } else {
        Quantize::Own
    };
    let mut encoded = emit(
        &features,
        center,
        quantize,
        repair,
        metadata_schemas,
        options,
        &mut reports,
        policy,
        attempt,
    )?;
    // The survey pass fitted this content on its own range. Should the
    // lattice compress worse past the budget, keep the content on its own
    // range instead of refusing it (the hierarchy stays the survey's), and
    // say so.
    let mut grid_fallback = false;
    if on_grid && encoded.bytes.len() > byte_limit {
        let mut own_reports = Vec::new();
        let own = emit(
            &features,
            center,
            Quantize::Own,
            repair,
            metadata_schemas,
            options,
            &mut own_reports,
            policy,
            attempt,
        )?;
        if own.bytes.len() <= byte_limit {
            encoded = own;
            reports = own_reports;
            grid_fallback = true;
        }
    }
    if pass == GridPass::Survey && !shared.is_empty() {
        if let Some(exponent) = encoded.survey {
            result.survey = Some(Survey {
                prefix: prefix.to_string(),
                level,
                exponent,
                shared: shared.iter().copied().collect(),
            });
        }
    }
    drop(features);
    drop(outlines);
    if level == 0 {
        result.reports.extend(reports);
    }
    let primitives = encoded.primitives;
    let vertex_count = encoded.vertices;
    let before_bytes = encoded.before_bytes;
    let rounding = encoded.rounding;
    let quantization = encoded.quantization;
    let inexact = encoded.inexact;
    let bytes = encoded.bytes;
    let sources = if aggregation.is_some() {
        SourceIdentity::Aggregate
    } else {
        SourceIdentity::Features {
            count: u32::try_from(items.len())
                .map_err(|_| data("a vector content holds more than 2^32 features"))?,
            digest: records::identity_digest(&items)?,
        }
    };
    let (content, uri) = records::content(&bytes, sources)?;
    let byte_count = bytes.len();
    let mut publication = tempfile::NamedTempFile::new_in(output.join("t"))?;
    publication.write_all(&bytes)?;
    match publication.persist_noclobber(output.join(&uri)) {
        Ok(_) => {}
        Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.error.into()),
    }
    drop(bytes);
    // Implicit scene-root placement adds JSON to the content (reserved in
    // `byte_limit`) before accepting a content candidate.
    if vertex_count > options.max_vertices || byte_count > byte_limit {
        result.reason = Some(if vertex_count > options.max_vertices {
            "vertices"
        } else {
            "bytes"
        });
        return Ok(result);
    }
    // A coarse tolerance bounds every simplification, stand-in and omission error.
    let own_error = if level > 0 || options.quantize {
        error.max(tolerance) + rounding + quantization
    } else {
        0.
    };
    let extras = records::VectorTileExtras::Content(records::Statistics {
        feature_fragments: items.len(),
        vertices: vertex_count,
        primitives,
        encoded_bytes: byte_count,
        geometry_error_metres: error,
        position_rounding_metres: rounding,
        quantization_error_metres: quantization,
        uncompressed_bytes: before_bytes,
        tolerance_metres: tolerance,
        point_aggregation: aggregation.clone(),
        build_path: None,
    });
    if inexact {
        // A shared vertex's snapped position widened this content's window
        // past its recorded exponent, so that vertex is re-rounded here.
        result.reports.push(json!({"buildPrefix":prefix,"level":level,"quantizationGrid":"coarsened",
            "reason":"a shared vertex is re-rounded at a coarser step than its other tiles use; its seams are not exact"}));
    }
    if unrecorded > 0 {
        result.reports.push(json!({"buildPrefix":prefix,"level":level,"quantizationGrid":"unrecorded","vertices":unrecorded,
            "reason":"shared vertices without a surveyed step keep this tile's own step; their seams are not exact"}));
    }
    if grid_fallback {
        // Exceeded maxBytes on the shared lattice: quantized on its own
        // range, as admitted, so its shared vertices are not exact.
        result.reports.push(json!({"buildPrefix":prefix,"level":level,"quantizationGridFallback":"bytes",
            "reason":"content exceeds maxBytes on the shared quantization grid; kept on its own range, seams are not exact"}));
    }
    if let Some(mut report) = aggregation {
        report["substitution"] = json!("pointAggregates");
        report["buildPrefix"] = json!(prefix);
        report["toleranceMetres"] = json!(tolerance);
        result.reports.push(report);
    }
    // Attach places, bounds and refines the record within its hierarchy.
    result.node = Some(TileRecord::new(
        NodePlacement::INHERIT,
        TileBox::centred([0.; 3])?,
        GeometricError::new(own_error)?,
        Refine::Replace,
        vec![content],
        Some(extras),
    )?);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A candidate's tile object as the producer writes it.
    fn tile_json(record: records::VectorRecord) -> Value {
        Value::Object(
            record
                .explicit_object(record.bounds(), crate::tileset_node::ObjectRole::Tile)
                .unwrap(),
        )
    }

    fn point_feature() -> Feature {
        Feature {
            intrinsic: None,
            properties: BTreeMap::from([
                ("_source_id".into(), json!("p0")),
                ("flag".into(), json!(true)),
                ("signed".into(), json!(9007199254740993i64)),
                ("text".into(), json!("λ🦉")),
                ("list".into(), json!("[9007199254740993,null,[\"λ\",2]]")),
            ]),
            geometry: Geometry::Point([0., 0., 0.]),
            surface_fragment: false,
            fragment_path: String::new(),
        }
    }

    // This fixture enters through the actual final producer and its SQL reader,
    // rather than calling the mapper or manufacturing a CodecError.
    fn final_point(
        options: &VectorOptions,
        policy: ProducerPolicy,
        attempt: &Attempt,
    ) -> (Result<Candidate, Error>, Vec<Vec<u8>>) {
        let root = tempfile::tempdir().unwrap();
        let spool = root.path().join("features.sqlite");
        let db = rusqlite::Connection::open(&spool).unwrap();
        db.execute_batch("CREATE TABLE features(id INTEGER PRIMARY KEY,path TEXT,data TEXT,lx,ly,lz,hx,hy,hz,omissible DEFAULT 0); CREATE TABLE vertices(x REAL,y REAL,z REAL,shared INTEGER); INSERT INTO vertices VALUES(0,0,0,0);").unwrap();
        db.execute(
            "INSERT INTO features(id,path,data) VALUES(1,'',?1)",
            [serde_json::to_string(&point_feature()).unwrap()],
        )
        .unwrap();
        drop(db);
        std::fs::create_dir(root.path().join("t")).unwrap();
        let schemas = BTreeMap::new();
        let encoder = Encoder {
            spool: &spool,
            pass: GridPass::Single,
            max_features: 4,
            repair: false,
            options,
            schemas: &schemas,
            output: root.path(),
            policy,
            attempt,
        };
        let result = encode(encoder, "", [0.; 3], 0, 0.);
        let files = std::fs::read_dir(root.path().join("t"))
            .unwrap()
            .map(|entry| std::fs::read(entry.unwrap().path()).unwrap())
            .collect();
        (result, files)
    }

    fn final_point_error(
        options: &VectorOptions,
        policy: ProducerPolicy,
        attempt: &Attempt,
    ) -> Error {
        let (result, files) = final_point(options, policy, attempt);
        let error = match result {
            Err(error) => error,
            Ok(_) => panic!("controlled infrastructure failure cannot produce a candidate"),
        };
        assert!(
            files.is_empty(),
            "refusal must occur before candidate file publication"
        );
        error
    }

    fn fatal_job(error: Error, kind: crate::JobErrorKind, attempt: &Attempt) -> crate::JobFailure {
        let Error::Job(failure) = error else {
            panic!("infrastructure failure must retain a job cause")
        };
        assert_eq!(failure.error.kind(), kind);
        assert!(failure.error.same_cause(&attempt.check().unwrap_err()));
        assert!(failure.secondary.is_empty());
        failure
    }
    fn read_views(bytes: &[u8]) -> (Value, Vec<Vec<u8>>) {
        assert_eq!(&bytes[..4], b"glTF");
        assert_eq!(
            u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize,
            bytes.len()
        );
        let j = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
        assert_eq!((20 + j) % 8, 0);
        assert_eq!((28 + j) % 8, 0);
        assert_eq!(bytes.len() % 8, 0);
        let document: Value = serde_json::from_slice(&bytes[20..20 + j]).unwrap();
        let binary = &bytes[28 + j..];
        assert_eq!(document["buffers"][0]["byteLength"], binary.len());
        let views = document["bufferViews"]
            .as_array()
            .unwrap()
            .iter()
            .map(|view| {
                if let Some(e) = view["extensions"].get("EXT_meshopt_compression") {
                    let off = e["byteOffset"].as_u64().unwrap() as usize;
                    let len = e["byteLength"].as_u64().unwrap() as usize;
                    assert_eq!(e["mode"], "ATTRIBUTES");
                    crate::content_integrity::meshopt::decode(
                        &binary[off..off + len],
                        e["count"].as_u64().unwrap() as usize,
                        e["byteStride"].as_u64().unwrap() as usize,
                        crate::content_integrity::meshopt::Mode::Attributes,
                    )
                    .unwrap()
                } else {
                    let off = view["byteOffset"].as_u64().unwrap() as usize;
                    let len = view["byteLength"].as_u64().unwrap() as usize;
                    binary[off..off + len].to_vec()
                }
            })
            .collect();
        (document, views)
    }

    fn inspect_payload(
        kind: crate::content_integrity::payload::PayloadKind,
        bytes: &[u8],
    ) -> Result<
        crate::content_integrity::payload::PayloadInspection,
        crate::content_integrity::PayloadError<FormatError>,
    > {
        let limits = codec::CompressionLimits::default();
        crate::content_integrity::payload::inspect(
            kind,
            bytes,
            crate::content_integrity::PayloadLimits {
                member_bytes: limits.candidate_bytes,
                decoded_bytes: limits.candidate_bytes,
                accessor_components: 4_000_000,
                json: crate::content_integrity::JsonLimits {
                    bytes: limits.json_bytes,
                    depth: limits.json_depth,
                    value_nodes: limits.json_value_nodes,
                },
            },
            4_000_000,
            |_uri, _remaining| panic!("generated payload cannot need an external resource"),
        )
    }

    fn reencode(bytes: &[u8], retained_source_capacity: usize) -> Vec<u8> {
        let run = crate::RunControl::default();
        let attempt = run.begin().unwrap();
        let limits = codec::CompressionLimits::default().validate().unwrap();
        let mut checkpoint = || attempt.check();
        let plan =
            codec::prepare(bytes, retained_source_capacity, &limits, &mut checkpoint).unwrap();
        codec::encode(plan, &mut checkpoint).unwrap().bytes
    }

    #[test]
    fn odd_index_counts_declare_selected_bin_and_pass_c1_and_identity() {
        for count in [1, 2, 3, 5] {
            let mut feature = point_feature();
            feature.geometry =
                Geometry::MultiPoint((0..count).map(|i| [i as f64, (i * 2) as f64, 0.]).collect());
            for quantize in [false, true] {
                let mut logical = None;
                for meshopt in [false, true] {
                    let options = VectorOptions {
                        meshopt,
                        quantize,
                        ..Default::default()
                    };
                    let run = crate::RunControl::default();
                    let attempt = run.begin().unwrap();
                    let encoded = emit(
                        &[&feature],
                        [0.; 3],
                        Quantize::Own,
                        false,
                        &BTreeMap::new(),
                        &options,
                        &mut Vec::new(),
                        ProducerPolicy::new(meshopt).unwrap(),
                        &attempt,
                    )
                    .unwrap();
                    let (document, views) = read_views(&encoded.bytes);
                    if !meshopt {
                        let last = document["bufferViews"].as_array().unwrap().last().unwrap();
                        assert_eq!(last["byteLength"], count * 4);
                        let original_end =
                            last["byteOffset"].as_u64().unwrap() as usize + count * 4;
                        assert_eq!(original_end % 8, if count % 2 == 0 { 0 } else { 4 });
                        assert_eq!(
                            document["buffers"][0]["byteLength"],
                            original_end + if count % 2 == 0 { 0 } else { 4 }
                        );
                        logical = Some(views.clone());
                        // Reproduce the obsolete writer declaration while using
                        // the real selected emitter. Independent C1 must reject
                        // exactly the four undeclared zeros for odd counts.
                        if count % 2 != 0 {
                            let mut old_document = document.clone();
                            old_document["buffers"][0]["byteLength"] = json!(original_end);
                            let j = u32::from_le_bytes(encoded.bytes[12..16].try_into().unwrap())
                                as usize;
                            let counts = GeneratedCounts {
                                views: views.len(),
                                accessors: document["accessors"].as_array().unwrap().len(),
                                logical_bytes: views.iter().map(Vec::len).sum(),
                            };
                            let limits = codec::CompressionLimits::default().validate().unwrap();
                            let document_storage =
                                value_storage(&old_document, 0, &mut 0, &limits.values).unwrap();
                            let retained = add_storage(document_storage, encoded.bytes.capacity())
                                .and_then(|n| add_storage(n, bridge_fixed_storage()))
                                .unwrap();
                            let mut checkpoint = || attempt.check();
                            let old = codec::write_generated_glb(
                                &old_document,
                                &encoded.bytes[28 + j..],
                                true,
                                counts,
                                retained,
                                &limits,
                                &mut checkpoint,
                            )
                            .unwrap();
                            let error = inspect_payload(
                                crate::content_integrity::payload::PayloadKind::Glb,
                                &old.bytes,
                            )
                            .unwrap_err();
                            let crate::content_integrity::PayloadError::Format(
                                FormatError::InvalidInput(reason),
                            ) = error
                            else {
                                panic!(
                                    "obsolete declaration must fail independent C1 range admission"
                                )
                            };
                            assert_eq!(reason, "buffer byteLength does not match actual resource");
                        }
                    } else {
                        assert_eq!(logical.as_ref().unwrap(), &views);
                    }
                    let inspection = inspect_payload(
                        crate::content_integrity::payload::PayloadKind::Glb,
                        &encoded.bytes,
                    )
                    .unwrap();
                    assert_eq!(inspection.primitives_checked, 1);
                    assert_eq!(inspection.vertices, count as u64);
                    let compressed = reencode(&encoded.bytes, encoded.bytes.capacity());
                    assert_eq!(read_views(&compressed).1, views);
                    inspect_payload(
                        crate::content_integrity::payload::PayloadKind::Glb,
                        &compressed,
                    )
                    .unwrap();
                    assert_eq!(
                        reencode(&compressed, compressed.capacity()),
                        compressed,
                        "second compressed pass preserves exact bytes"
                    );
                    if meshopt {
                        assert_eq!(compressed, encoded.bytes);
                    }
                }
            }
        }
    }

    #[test]
    fn generated_bridge_checks_buffer_before_metadata_padding_and_preserves_core_length() {
        for buffers in [
            json!([{"byteLength":8}]),
            json!([{"byteLength":"4"}]),
            json!([{"byteLength":4,"uri":null}]),
            json!([4]),
            json!([]),
        ] {
            for meshopt in [false, true] {
                let run = crate::RunControl::default();
                let attempt = run.begin().unwrap();
                let document = json!({"asset":{"version":"2.0"},"buffers":buffers,
                    "extensionsUsed":["EXT_structural_metadata"],"accessors":[],
                    "bufferViews":[{"buffer":0,"byteOffset":0,"byteLength":4}]});
                let error = bridge(
                    document,
                    vec![0; 4],
                    ProducerPolicy::new(meshopt).unwrap(),
                    &attempt,
                )
                .unwrap_err();
                let failure = fatal_job(error, crate::JobErrorKind::InvalidState, &attempt);
                assert!(matches!(
                    std::error::Error::source(&failure.error)
                        .unwrap()
                        .downcast_ref::<FormatError>()
                        .unwrap(),
                    FormatError::InvalidInput(_)
                ));
            }
        }
        let run = crate::RunControl::default();
        let attempt = run.begin().unwrap();
        let document = json!({"asset":{"version":"2.0"},"buffers":[{"byteLength":4}],
            "accessors":[],"bufferViews":[{"buffer":0,"byteOffset":0,"byteLength":4}]});
        let (bytes, before) = bridge(
            document,
            vec![0; 4],
            ProducerPolicy::new(false).unwrap(),
            &attempt,
        )
        .unwrap();
        let j = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
        let document: Value = serde_json::from_slice(&bytes[20..20 + j]).unwrap();
        assert_eq!(document["buffers"][0]["byteLength"], 4);
        assert_eq!(
            u32::from_le_bytes(bytes[20 + j..24 + j].try_into().unwrap()),
            4
        );
        assert_eq!(before, bytes.len());
        inspect_payload(crate::content_integrity::payload::PayloadKind::Glb, &bytes).unwrap();

        // A declaration crossing a decimal digit boundary must be counted
        // after alignment in the actual emitted document.
        let run = crate::RunControl::default();
        let attempt = run.begin().unwrap();
        let document = json!({"asset":{"version":"2.0"},"buffers":[{"byteLength":996}],
            "extensionsUsed":["EXT_structural_metadata"],"accessors":[],
            "bufferViews":[{"buffer":0,"byteOffset":0,"byteLength":996}]});
        let original_json_length = serde_json::to_vec(&document).unwrap().len();
        let (bytes, before) = bridge(
            document,
            vec![0; 996],
            ProducerPolicy::new(false).unwrap(),
            &attempt,
        )
        .unwrap();
        let (document, views) = read_views(&bytes);
        assert_eq!(document["buffers"][0]["byteLength"], 1000);
        assert_eq!(views, vec![vec![0; 996]]);
        assert_eq!(
            serde_json::to_vec(&document).unwrap().len(),
            original_json_length + 1
        );
        assert_eq!(before, bytes.len());
        inspect_payload(crate::content_integrity::payload::PayloadKind::Glb, &bytes).unwrap();
    }

    #[test]
    fn producer_raw_and_compressed_bridge_preserve_whole_views_and_metadata() {
        for quantize in [false, true] {
            let feature = point_feature();
            let mut outputs = Vec::new();
            for meshopt in [false, true] {
                let options = VectorOptions {
                    meshopt,
                    quantize,
                    ..VectorOptions::default()
                };
                let run = crate::RunControl::default();
                let attempt = run.begin().unwrap();
                let encoded = emit(
                    &[&feature],
                    [0.; 3],
                    Quantize::Own,
                    false,
                    &BTreeMap::new(),
                    &options,
                    &mut Vec::new(),
                    ProducerPolicy::new(meshopt).unwrap(),
                    &attempt,
                )
                .unwrap();
                if !meshopt {
                    assert_eq!(encoded.before_bytes, encoded.bytes.len());
                }
                outputs.push(encoded);
            }
            assert_eq!(outputs[1].before_bytes, outputs[0].bytes.len());
            let (raw, raw_views) = read_views(&outputs[0].bytes);
            let (compressed, compressed_views) = read_views(&outputs[1].bytes);
            assert_eq!(raw_views, compressed_views);
            for key in ["extensions", "meshes", "nodes", "accessors"] {
                assert_eq!(raw[key], compressed[key], "{key}");
            }
            assert!(raw["bufferViews"]
                .as_array()
                .unwrap()
                .iter()
                .all(|v| v["extensions"].get("EXT_meshopt_compression").is_none()));
            assert!(compressed["bufferViews"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v["extensions"].get("EXT_meshopt_compression").is_some()));
            // INT64 and JSON-list exact-integer source bytes are independent literals.
            assert!(raw_views
                .iter()
                .any(|v| v.as_slice() == 9007199254740993i64.to_le_bytes().as_slice()));
            assert!(raw_views
                .iter()
                .any(|v| v.as_slice() == "[9007199254740993,null,[\"λ\",2]]".as_bytes()));
        }
    }

    /// CesiumJS mirrors a strip end over its neighbour (2c-n) in the u16
    /// position type; every quantized end must keep that inside [0, 65535].
    /// Unpadded per-tile bboxes put ends on the range edge and wrap.
    #[test]
    fn quantized_line_strip_ends_mirror_inside_u16_range() {
        let ring: Vec<Point> = (0..7)
            .map(|i| {
                let a = i as f64 / 7. * std::f64::consts::TAU;
                [100. * a.cos(), 60. * a.sin(), 3. * (i % 2) as f64]
            })
            .collect();
        let mut piece = point_feature();
        piece.geometry =
            Geometry::MultiLineString(outline::source_rings(&ring, &[0, 1, 2, 3, 4, 5, 6]));
        piece.surface_fragment = true;
        let mut open = point_feature();
        open.properties.insert("_source_id".into(), json!("l1"));
        open.geometry = Geometry::MultiLineString(vec![
            vec![[-150., -90., 0.], [40., 10., 0.], [-20., 80., 5.]],
            vec![[150., 90., -5.], [149., -90., 0.]],
        ]);
        // Without padding the first end is the x and y minimum (0 in u16)
        // with a neighbour above it, so 2c-n would be negative and wrap.
        let all: Vec<Point> = [&piece.geometry, &open.geometry]
            .iter()
            .flat_map(|g| g.points())
            .copied()
            .collect();
        assert!(all.iter().all(|p| p[0] >= -150. && p[1] >= -90.));
        let outlines = outline::fragment_outlines([&piece]);
        // As in `encode`: other features first, then the joined outlines.
        let items: Vec<&Feature> = std::iter::once(&open).chain(&outlines).collect();
        for meshopt in [false, true] {
            let options = VectorOptions {
                meshopt,
                quantize: true,
                ..VectorOptions::default()
            };
            let run = crate::RunControl::default();
            let attempt = run.begin().unwrap();
            let encoded = emit(
                &items,
                [0.; 3],
                Quantize::Own,
                false,
                &BTreeMap::new(),
                &options,
                &mut Vec::new(),
                ProducerPolicy::new(meshopt).unwrap(),
                &attempt,
            )
            .unwrap();
            let (document, views) = read_views(&encoded.bytes);
            let primitive = &document["meshes"][0]["primitives"][0];
            assert_eq!(primitive["mode"], 3);
            let accessor = |i: &Value| &document["accessors"][i.as_u64().unwrap() as usize];
            let position = accessor(&primitive["attributes"]["POSITION"]);
            assert_eq!(position["componentType"], 5123);
            let data = &views[position["bufferView"].as_u64().unwrap() as usize];
            let q = |v: u32, axis: usize| {
                let at = v as usize * 8 + axis * 2;
                i64::from(u16::from_le_bytes([data[at], data[at + 1]]))
            };
            let index = &views[accessor(&primitive["indices"])["bufferView"]
                .as_u64()
                .unwrap() as usize];
            let indices: Vec<u32> = index
                .chunks(4)
                .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
                .collect();
            let strips: Vec<_> = indices.split(|i| *i == u32::MAX).collect();
            // The outline ring (seam vertex added) is one closed strip; the
            // open lines keep theirs.
            assert_eq!(
                strips.iter().map(|s| s.len()).collect::<Vec<_>>(),
                [3, 2, 9]
            );
            let closed = strips[2];
            assert!((0..3).all(|axis| q(closed[0], axis) == q(closed[8], axis)));
            for strip in &strips {
                let ends = [
                    (strip[0], strip[1]),
                    (strip[strip.len() - 1], strip[strip.len() - 2]),
                ];
                for (c, n) in ends {
                    for axis in 0..3 {
                        let mirrored = 2 * q(c, axis) - q(n, axis);
                        assert!((0..=65535).contains(&mirrored), "axis {axis}: {mirrored}");
                    }
                }
            }
            // Decoded positions still match within the reported error.
            let translation = &document["nodes"][0]["translation"];
            let scale = &document["nodes"][0]["scale"];
            let decoded = |v: u32| -> Point {
                std::array::from_fn(|a| {
                    translation[a].as_f64().unwrap()
                        + q(v, a) as f64 / 65535. * scale[a].as_f64().unwrap()
                })
            };
            for (k, p) in [&open.geometry].iter().flat_map(|g| g.points()).enumerate() {
                let v = strips[k / 3][k % 3];
                assert!(norm(sub(decoded(v), *p)) <= encoded.quantization, "{k}");
            }
        }
    }

    /// Fill vertices of one payload as a client decodes them into the frame
    /// (meshopt stream, normalized u16 or float32, node translation and
    /// scale, tile center): in f64, and with float32 node values and product.
    fn decoded_fills(encoded: &Encoded, center: Point) -> (Vec<Point>, Vec<Point>) {
        let (document, views) = read_views(&encoded.bytes);
        let node = &document["nodes"][0];
        let t: Point = std::array::from_fn(|i| node["translation"][i].as_f64().unwrap());
        let primitive = document["meshes"][0]["primitives"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["mode"] == 4)
            .unwrap();
        let position =
            &document["accessors"][primitive["attributes"]["POSITION"].as_u64().unwrap() as usize];
        let data = &views[position["bufferView"].as_u64().unwrap() as usize];
        let (mut wide, mut narrow) = (Vec::new(), Vec::new());
        if position["componentType"] == 5126 {
            // Float32 lattice values, without a node scale.
            assert!(node.get("scale").is_none());
            let (vertices, rest) = data.as_chunks::<12>();
            assert!(rest.is_empty());
            for v in vertices {
                let q: [f32; 3] = std::array::from_fn(|i| {
                    f32::from_le_bytes(v[i * 4..i * 4 + 4].try_into().unwrap())
                });
                wide.push(std::array::from_fn(|i| center[i] + t[i] + f64::from(q[i])));
                narrow.push(std::array::from_fn(|i| {
                    center[i] + f64::from(t[i] as f32 + q[i])
                }));
            }
            return (wide, narrow);
        }
        assert_eq!(position["componentType"], 5123);
        let s: Point = std::array::from_fn(|i| node["scale"][i].as_f64().unwrap());
        for v in 0..position["count"].as_u64().unwrap() as usize {
            let q: [u16; 3] = std::array::from_fn(|i| {
                u16::from_le_bytes([data[v * 8 + i * 2], data[v * 8 + i * 2 + 1]])
            });
            wide.push(std::array::from_fn(|i| {
                center[i] + t[i] + f64::from(q[i]) / 65535. * s[i]
            }));
            narrow.push(std::array::from_fn(|i| {
                center[i] + f64::from(t[i] as f32 + (f32::from(q[i]) / 65535.) * s[i] as f32)
            }));
        }
        (wide, narrow)
    }

    /// Two fill fragments share a cut edge across adjacent tiles of
    /// different size and center, at negative coordinates; the large one also
    /// carries a padded outline strip. With the seam vertices snapped at the
    /// larger of the two payloads' survey exponents (what the build records
    /// for tiles that render together), each, including one just past a
    /// rounding tie, decodes to one frame position in both payloads, within
    /// each certified bound (also through float32 placement), while the small
    /// payload keeps its own finer step. Stored as float32, the large payload
    /// instead holds the seam at the small payload's step, within a quarter
    /// of its own u16 half step; as u16 it cannot. Each payload's own range
    /// opens the seam.
    #[test]
    fn adjacent_payloads_decode_shared_fill_vertices_identically() {
        let (x, h) = (-12345.678, -20.25);
        let (z0, z1) = (-4000.3, -1500.7);
        let fixture = |zt: f64| {
            let shared = [[x, h, z0], [x, h, zt], [x, h, -2200.9], [x, h, z1]];
            let mut a = point_feature();
            a.properties.insert("_source_id".into(), json!("a"));
            a.surface_fragment = true;
            let mut ring_a = vec![[x - 5000.5, h, z0]];
            ring_a.extend(shared);
            ring_a.extend([[x - 5000.5, h, z1], [x - 5000.5, h, z0]]);
            a.geometry = Geometry::Polygon(vec![ring_a]);
            let mut b = point_feature();
            b.properties.insert("_source_id".into(), json!("b"));
            b.surface_fragment = true;
            let far = [
                [x + 60000.25, h, z0 - 1000.],
                [x + 60000.25, h, z1 + 30000.],
            ];
            let mut ring_b = vec![shared[0], far[0], far[1]];
            ring_b.extend(shared.iter().rev().copied());
            b.geometry = Geometry::Polygon(vec![ring_b]);
            let mut outline = point_feature();
            outline.properties.insert("_source_id".into(), json!("o"));
            outline.geometry = Geometry::MultiLineString(vec![vec![shared[0], far[0], far[1]]]);
            (shared, a, b, outline)
        };
        let center_b = [x + 30000., 0., z1 + 13000.];
        let run = crate::RunControl::default();
        let attempt = run.begin().unwrap();
        let options = VectorOptions {
            quantize: true,
            ..VectorOptions::default()
        };
        let own = |items: &[&Feature], center: Point| {
            emit(
                items,
                center,
                Quantize::Own,
                false,
                &BTreeMap::new(),
                &options,
                &mut Vec::new(),
                ProducerPolicy::new(false).unwrap(),
                &attempt,
            )
            .unwrap()
            .survey
            .unwrap()
            .u16
        };
        // The survey exponents, and the seam's: the larger per axis.
        let (_, a, b, outline) = fixture(-2996.);
        let (small, large) = (own(&[&a], [0.; 3]), own(&[&b, &outline], center_b));
        let seam: [i32; 3] = std::array::from_fn(|i| small[i].max(large[i]));
        assert!((0..3).any(|i| small[i] < large[i]), "{small:?} {large:?}");
        let step = 2f64.powi(seam[2]);
        // Just past the tie between two seam lattice indices on z.
        let tie = ((-3000. / step).floor() + 0.5) * step;
        let zt = tie + step * 2f64.powi(-30);
        let (shared, a, b, outline) = fixture(zt);
        assert_eq!(own(&[&a], [0.; 3]), small);
        // Tile A's center puts the tie vertex where a float32 local position
        // would round below the tie: snapping recentred float32 values (not
        // exact source positions) would split this vertex across the seam.
        let v = 98304f64;
        let center_a = [x - 2500., 0., tie - v - 2f64.powi(-7) / 4.];
        let recentred = ((zt - center_a[2]) as f32) as f64 + center_a[2];
        assert_ne!(
            lattice_index(recentred, seam[2]),
            lattice_index(zt, seam[2])
        );
        let at = |e: [i32; 3]| -> BTreeMap<(Cell, Kind), [i32; 3]> {
            shared
                .iter()
                .flat_map(|p| [((cell(*p), Kind::Fill), e), ((cell(*p), Kind::Line), e)])
                .collect()
        };
        let (exponents, fine) = (at(seam), at(small));
        for meshopt in [false, true] {
            let options = VectorOptions {
                meshopt,
                quantize: true,
                ..VectorOptions::default()
            };
            let encode = |items: &[&Feature], center: Point, quantize: Quantize<'_>| {
                emit(
                    items,
                    center,
                    quantize,
                    false,
                    &BTreeMap::new(),
                    &options,
                    &mut Vec::new(),
                    ProducerPolicy::new(meshopt).unwrap(),
                    &attempt,
                )
                .unwrap()
            };
            let nearest = |points: &[Point], p: Point| -> Point {
                *points
                    .iter()
                    .min_by(|u, w| norm(sub(**u, p)).total_cmp(&norm(sub(**w, p))))
                    .unwrap()
            };
            let seam_gap = |quantize: Quantize<'_>, large_quantize: Quantize<'_>| -> f64 {
                let first = encode(&[&a], center_a, quantize);
                let second = encode(&[&b, &outline], center_b, large_quantize);
                if let Quantize::Shared(_, Storage::Float32) = large_quantize {
                    // Float32 holds the small payload's step: the large
                    // payload's bound is far below half its own u16 step.
                    let half = norm(large.map(|e| 2f64.powi(e - 1)));
                    assert!(
                        second.quantization < half / 4.,
                        "{} vs {half}",
                        second.quantization
                    );
                }
                if let Quantize::Shared(..) = quantize {
                    assert!(!first.inexact && !second.inexact);
                    // The small payload keeps its own finer step.
                    let (document, _) = read_views(&first.bytes);
                    let scale = document["nodes"][0]["scale"][0].as_f64().unwrap();
                    assert_eq!(scale, 65535. * 2f64.powi(small[0]));
                }
                let (wide_a, narrow_a) = decoded_fills(&first, center_a);
                let (wide_b, narrow_b) = decoded_fills(&second, center_b);
                for (encoded, wide, narrow, ring) in [
                    (&first, &wide_a, &narrow_a, &a),
                    (&second, &wide_b, &narrow_b, &b),
                ] {
                    for p in ring.geometry.points() {
                        for decoded in [wide, narrow] {
                            let d = norm(sub(nearest(decoded, *p), *p));
                            assert!(d <= encoded.quantization, "{d} > {}", encoded.quantization);
                        }
                    }
                }
                shared
                    .iter()
                    .map(|p| norm(sub(nearest(&wide_a, *p), nearest(&wide_b, *p))))
                    .fold(0., f64::max)
            };
            let both = Quantize::Shared(&exponents, Storage::U16);
            assert!(seam_gap(both, both) < 1e-9, "shared seam");
            // The large payload stores float32 and holds the seam at the
            // small payload's own step instead.
            let fine = Quantize::Shared(&fine, Storage::U16);
            let float = match fine {
                Quantize::Shared(exponents, _) => Quantize::Shared(exponents, Storage::Float32),
                Quantize::Own => unreachable!(),
            };
            assert!(seam_gap(fine, float) < 1e-9, "float32 seam");
            // At the small step, a u16 large payload cannot hold the seam.
            assert!(
                encode(&[&b, &outline], center_b, fine).inexact,
                "u16 re-rounds the seam"
            );
            // Each payload's own range (the behaviour before the fix).
            assert!(
                seam_gap(Quantize::Own, Quantize::Own) > 1e-3,
                "per-payload ranges crack"
            );
        }
    }

    /// The final pass reads each shared vertex's step and the tile's storage
    /// from `grid`. Without rows the tile keeps its own range and reports
    /// the unrecorded vertices. With float32 rows, its fill and line strip
    /// positions are float32 lattice values (no node scale, no strip-end
    /// padding): shared vertices exactly on their recorded step, every
    /// vertex within the bound.
    #[test]
    fn final_pass_reads_grid_storage_and_reports_unrecorded_vertices() {
        let ring = vec![
            [0., 0., 0.],
            [1000.3, 0., 0.],
            [1000.3, 0., 700.7],
            [0., 0., 0.],
        ];
        let mut fill = point_feature();
        fill.properties.insert("_source_id".into(), json!("f"));
        fill.geometry = Geometry::Polygon(vec![ring.clone()]);
        let mut line = point_feature();
        line.properties.insert("_source_id".into(), json!("l"));
        line.geometry = Geometry::LineString(vec![ring[0], [400.1, 0., 900.9], ring[2]]);
        let root = tempfile::tempdir().unwrap();
        let spool = root.path().join("features.sqlite");
        let db = rusqlite::Connection::open(&spool).unwrap();
        db.execute_batch("CREATE TABLE features(id INTEGER PRIMARY KEY,path TEXT,data TEXT,lx,ly,lz,hx,hy,hz,omissible DEFAULT 0); CREATE TABLE vertices(x REAL,y REAL,z REAL,shared INTEGER); CREATE TABLE grid(cx INTEGER,cy INTEGER,cz INTEGER,kind INTEGER,prefix TEXT,level INTEGER,ex INTEGER,ey INTEGER,ez INTEGER,fl INTEGER,al INTEGER);").unwrap();
        for (id, feature) in [(1, &fill), (2, &line)] {
            db.execute(
                "INSERT INTO features(id,path,data) VALUES(?1,'',?2)",
                rusqlite::params![id, serde_json::to_string(feature).unwrap()],
            )
            .unwrap();
        }
        for p in [ring[0], ring[1], ring[2], [400.1, 0., 900.9]] {
            let shared = p != [400.1, 0., 900.9];
            db.execute(
                "INSERT INTO vertices VALUES(?1,?2,?3,?4)",
                rusqlite::params![p[0], p[1], p[2], shared],
            )
            .unwrap();
        }
        let run = crate::RunControl::default();
        let attempt = run.begin().unwrap();
        let options = VectorOptions {
            quantize: true,
            ..VectorOptions::default()
        };
        let schemas = BTreeMap::new();
        let center = [500., 0., 450.];
        let encode_tile = || {
            let t = root.path().join("t");
            let _ = std::fs::remove_dir_all(&t);
            std::fs::create_dir(&t).unwrap();
            let encoder = Encoder {
                spool: &spool,
                pass: GridPass::Final,
                max_features: 4,
                repair: false,
                options: &options,
                schemas: &schemas,
                output: root.path(),
                policy: ProducerPolicy::new(false).unwrap(),
                attempt: &attempt,
            };
            let candidate = encode(encoder, "", center, 0, 0.).unwrap();
            let files: Vec<_> = std::fs::read_dir(&t).unwrap().collect();
            assert_eq!(files.len(), 1);
            let bytes = std::fs::read(files[0].as_ref().unwrap().path()).unwrap();
            (candidate, bytes)
        };
        // Shared (cell, kind): three fill corners and the line's two ends.
        let (candidate, _) = encode_tile();
        let unrecorded: Vec<_> = candidate
            .reports
            .iter()
            .filter(|r| r["quantizationGrid"] == "unrecorded")
            .collect();
        assert_eq!(unrecorded.len(), 1, "{:?}", candidate.reports);
        assert_eq!(unrecorded[0]["vertices"], 5);
        // Every shared copy recorded on a 1/16 m step, stored as float32.
        let e = -4;
        for (p, kind) in [
            (ring[0], Kind::Fill),
            (ring[1], Kind::Fill),
            (ring[2], Kind::Fill),
            (ring[0], Kind::Line),
            (ring[2], Kind::Line),
        ] {
            let c = cell(p);
            db.execute(
                "INSERT INTO grid VALUES(?1,?2,?3,?4,'',0,?5,?5,?5,1,0)",
                rusqlite::params![c[0], c[1], c[2], kind as i64, e],
            )
            .unwrap();
        }
        let (candidate, bytes) = encode_tile();
        assert!(
            candidate
                .reports
                .iter()
                .all(|r| r.get("quantizationGrid").is_none()),
            "{:?}",
            candidate.reports
        );
        let error = candidate
            .node
            .as_ref()
            .unwrap()
            .extras()
            .and_then(records::VectorTileExtras::statistics)
            .unwrap()
            .quantization_error_metres;
        let (document, views) = read_views(&bytes);
        assert!(!document["extensionsUsed"]
            .as_array()
            .unwrap()
            .contains(&json!("KHR_mesh_quantization")));
        let node = &document["nodes"][0];
        assert!(node.get("scale").is_none());
        let t: Point = std::array::from_fn(|i| node["translation"][i].as_f64().unwrap());
        let mut modes = Vec::new();
        for primitive in document["meshes"][0]["primitives"].as_array().unwrap() {
            modes.push(primitive["mode"].as_u64().unwrap());
            let position = &document["accessors"]
                [primitive["attributes"]["POSITION"].as_u64().unwrap() as usize];
            assert_eq!(position["componentType"], 5126);
            let data = &views[position["bufferView"].as_u64().unwrap() as usize];
            let (vertices, _) = data.as_chunks::<12>();
            let decoded: Vec<Point> = vertices
                .iter()
                .map(|v| {
                    std::array::from_fn(|i| {
                        let q = f32::from_le_bytes(v[i * 4..i * 4 + 4].try_into().unwrap());
                        center[i] + t[i] + f64::from(q)
                    })
                })
                .collect();
            for p in [ring[0], ring[1], ring[2]] {
                if primitive["mode"] == 3 && p == ring[1] {
                    continue;
                }
                let snapped = p.map(|v| lattice_index(v, e) as f64 * 2f64.powi(e));
                assert!(
                    decoded.iter().any(|d| norm(sub(*d, snapped)) < 1e-9),
                    "{p:?} not at {snapped:?}"
                );
            }
            for d in &decoded {
                let near = ring
                    .iter()
                    .chain([&[400.1, 0., 900.9]])
                    .map(|p| norm(sub(*d, *p)))
                    .fold(f64::MAX, f64::min);
                assert!(near <= error, "{near} > {error}");
            }
        }
        modes.sort();
        assert_eq!(modes, [3, 4], "fill and line strip in one float32 content");
        // No co-rendering tile holds any of them: its own u16 range, unreported.
        db.execute("UPDATE grid SET al=1", []).unwrap();
        let (candidate, bytes) = encode_tile();
        assert!(candidate
            .reports
            .iter()
            .all(|r| r.get("quantizationGrid").is_none()));
        let (document, _) = read_views(&bytes);
        assert!(document["extensionsUsed"]
            .as_array()
            .unwrap()
            .contains(&json!("KHR_mesh_quantization")));
        let scale = document["nodes"][0]["scale"][0].as_f64().unwrap();
        assert_ne!(
            (scale / 65535.).log2().fract(),
            0.,
            "a range, not a lattice step"
        );
    }

    /// The u16 window holds every strip end's mirror `2c - n`: a two-point
    /// line needs two doublings of its own exponent.
    #[test]
    fn line_strip_mirrors_fit_the_lattice_window() {
        let points = [[-100., 0., 0.], [100., 0., 0.]];
        let own = lattice::enclosing(points[0], points[1], Storage::U16);
        let placement = lattice::place(&[&points], &[], &[(0, 0, 1), (0, 1, 0)], Storage::U16);
        // Mirrors reach +-300 against a 256-unit window: two doublings.
        assert_eq!(placement.exponent[0], own[0] + 2);
        let mut line = point_feature();
        line.properties.insert("_source_id".into(), json!("l"));
        line.geometry = Geometry::LineString(points.to_vec());
        let options = VectorOptions {
            quantize: true,
            ..VectorOptions::default()
        };
        let run = crate::RunControl::default();
        let attempt = run.begin().unwrap();
        let encoded = emit(
            &[&line],
            [0.; 3],
            Quantize::Shared(&BTreeMap::new(), Storage::U16),
            false,
            &BTreeMap::new(),
            &options,
            &mut Vec::new(),
            ProducerPolicy::new(false).unwrap(),
            &attempt,
        )
        .unwrap();
        let (document, views) = read_views(&encoded.bytes);
        let position = &document["accessors"][document["meshes"][0]["primitives"][0]["attributes"]
            ["POSITION"]
            .as_u64()
            .unwrap() as usize];
        let data = &views[position["bufferView"].as_u64().unwrap() as usize];
        let q = |v: usize| i64::from(u16::from_le_bytes([data[v * 8], data[v * 8 + 1]]));
        for (c, n) in [(0, 1), (1, 0)] {
            assert!((0..=65535).contains(&(2 * q(c) - q(n))));
        }
    }

    #[test]
    fn nullable_metadata_ids_and_node_association_survive_compression() {
        let first = point_feature();
        let mut second = point_feature();
        second.geometry = Geometry::Point([1., 2., 3.]);
        second.properties.insert("_source_id".into(), json!("p1"));
        second.properties.insert("flag".into(), json!(false));
        second.properties.remove("signed");
        second.properties.remove("text");
        let mut outputs = Vec::new();
        for meshopt in [false, true] {
            let options = VectorOptions {
                meshopt,
                ..Default::default()
            };
            let run = crate::RunControl::default();
            let attempt = run.begin().unwrap();
            let encoded = emit(
                &[&first, &second],
                [0.; 3],
                Quantize::Own,
                false,
                &BTreeMap::new(),
                &options,
                &mut Vec::new(),
                ProducerPolicy::new(meshopt).unwrap(),
                &attempt,
            )
            .unwrap();
            outputs.push(read_views(&encoded.bytes));
        }
        assert_eq!(outputs[0].1, outputs[1].1);
        for key in ["extensions", "meshes", "nodes", "accessors"] {
            assert_eq!(outputs[0].0[key], outputs[1].0[key]);
        }
        let doc = &outputs[1].0;
        let metadata = &doc["extensions"]["EXT_structural_metadata"];
        let properties = &metadata["schema"]["classes"]["feature"]["properties"];
        assert_eq!(properties["signed"]["componentType"], "INT64");
        assert_eq!(properties["signed"]["noData"], json!(-(1i64 << 53)));
        assert_eq!(properties["text"]["noData"], "__RUSTY_TILES_MISSING__");
        assert_eq!(metadata["propertyTables"][0]["count"], 2);
        let primitive = &doc["meshes"][0]["primitives"][0];
        let id_accessor = primitive["attributes"]["_FEATURE_ID_0"].as_u64().unwrap() as usize;
        let id_view = doc["accessors"][id_accessor]["bufferView"]
            .as_u64()
            .unwrap() as usize;
        assert_eq!(outputs[1].1[id_view], [0, 0, 0, 0, 1, 0, 0, 0]);
        assert_eq!(
            primitive["extensions"]["EXT_mesh_features"]["featureIds"][0]["propertyTable"],
            0
        );
        assert_eq!(doc["nodes"][0]["mesh"], 0);
        assert_eq!(doc["scenes"][0]["nodes"], json!([0]));
    }

    #[test]
    fn selected_output_bytes_drive_fitting_final_budget_and_node_reports() {
        for meshopt in [false, true] {
            for quantize in [false, true] {
                let base = VectorOptions {
                    meshopt,
                    quantize,
                    max_bytes: 1_000_000,
                    ..Default::default()
                };
                let run = crate::RunControl::default();
                let attempt = run.begin().unwrap();
                let (result, files) =
                    final_point(&base, ProducerPolicy::new(meshopt).unwrap(), &attempt);
                let candidate = result.unwrap();
                let node = tile_json(candidate.node.unwrap());
                assert_eq!(files.len(), 1);
                let actual = files[0].len();
                let (document, _) = read_views(&files[0]);
                let last = document["bufferViews"].as_array().unwrap().last().unwrap();
                assert_eq!(
                    last["byteLength"], 4,
                    "one-point index view supplies the odd-count control"
                );
                inspect_payload(
                    crate::content_integrity::payload::PayloadKind::Glb,
                    &files[0],
                )
                .unwrap();
                assert_eq!(node["extras"]["encodedBytes"], actual);
                let raw_run = crate::RunControl::default();
                let raw_attempt = raw_run.begin().unwrap();
                let raw = emit(
                    &[&point_feature()],
                    [0.; 3],
                    Quantize::Own,
                    false,
                    &BTreeMap::new(),
                    &base,
                    &mut Vec::new(),
                    ProducerPolicy::new(false).unwrap(),
                    &raw_attempt,
                )
                .unwrap();
                assert_eq!(node["extras"]["uncompressedBytes"], raw.bytes.len());
                if !meshopt {
                    assert_eq!(actual, raw.bytes.len());
                }
                let limit = actual + IMPLICIT_FRAMING_RESERVE;
                for (limit, accepted) in [(limit, true), (limit - 1, false)] {
                    let options = VectorOptions {
                        max_bytes: limit,
                        ..base.clone()
                    };
                    let fit_run = crate::RunControl::default();
                    let fit_attempt = fit_run.begin().unwrap();
                    assert_eq!(
                        fits_feature(
                            &point_feature(),
                            false,
                            &options,
                            ProducerPolicy::new(meshopt).unwrap(),
                            &fit_attempt
                        )
                        .unwrap(),
                        accepted
                    );
                    let final_run = crate::RunControl::default();
                    let final_attempt = final_run.begin().unwrap();
                    let (result, _) = final_point(
                        &options,
                        ProducerPolicy::new(meshopt).unwrap(),
                        &final_attempt,
                    );
                    let result = result.unwrap();
                    assert_eq!(result.node.is_some(), accepted);
                    assert_eq!(result.reason, if accepted { None } else { Some("bytes") });
                }
            }
        }
    }

    fn fitting_fixtures() -> Vec<Feature> {
        let mut line = point_feature();
        line.geometry = Geometry::MultiLineString(vec![
            vec![[0., 0., 0.], [3., 1., 0.], [5., -2., 1.]],
            vec![[1., 1., 1.], [2., 2., 2.]],
        ]);
        let mut polygon = point_feature();
        polygon.geometry = Geometry::Polygon(vec![
            vec![
                [0., 0., 0.],
                [10., 0., 0.],
                [10., 10., 0.],
                [0., 10., 0.],
                [0., 0., 0.],
            ],
            vec![
                [2., 2., 0.],
                [2., 4., 0.],
                [4., 4., 0.],
                [4., 2., 0.],
                [2., 2., 0.],
            ],
        ]);
        let mut wide = polygon.clone();
        for index in 0..40 {
            wide.properties
                .insert(format!("p{index}"), json!(format!("v{index}")));
        }
        let mut surface = polygon.clone();
        surface.geometry = Geometry::Polygon(vec![vec![
            [0., 0., 0.],
            [10., 0., 0.],
            [0., 10., 0.],
            [0., 0., 0.],
        ]]);
        surface.surface_fragment = true;
        // A partitioned polygon's outline piece: its closed source ring.
        let mut outline = surface.clone();
        outline.geometry = Geometry::MultiLineString(outline::source_rings(
            &[[0., 0., 0.], [10., 0., 0.], [0., 10., 0.]],
            &[0, 1, 2],
        ));
        vec![point_feature(), line, polygon, wide, surface, outline]
    }

    /// The fitting source of features sharing one GLB: one spool row, as
    /// `fits_feature` measures it, or a tile's fill and outline, as `encode`
    /// writes them.
    fn fitting_source(items: &[&Feature], options: &VectorOptions) -> Source {
        let run = crate::RunControl::default();
        let attempt = run.begin().unwrap();
        let schemas = BTreeMap::from([
            ("flag".to_string(), "boolean".to_string()),
            ("signed".to_string(), "integer".to_string()),
            ("text".to_string(), "string".to_string()),
        ]);
        emit_source(
            items,
            [1., 2., 0.],
            Quantize::Own,
            false,
            &schemas,
            options,
            &mut Vec::new(),
            &attempt,
        )
        .unwrap()
    }

    fn written_len(source: Source, policy: ProducerPolicy) -> Result<usize, Error> {
        let run = crate::RunControl::default();
        let attempt = run.begin().unwrap();
        let (bytes, _) = bridge(source.document, source.binary, policy, &attempt)?;
        Ok(bytes.len())
    }

    fn counted_len(source: Source, policy: ProducerPolicy) -> Result<usize, Error> {
        let run = crate::RunControl::default();
        let attempt = run.begin().unwrap();
        bridge_len(source.document, source.binary, policy, &attempt)
    }

    #[test]
    fn counted_fitting_length_equals_written_and_admitted_content() {
        let policy = ProducerPolicy::new(false).unwrap();
        for quantize in [false, true] {
            let options = VectorOptions {
                quantize,
                ..Default::default()
            };
            let fixtures = fitting_fixtures();
            let mut sources: Vec<Vec<&Feature>> = fixtures.iter().map(|f| vec![f]).collect();
            // A tile's fill fragment and its polygon's outline share one GLB
            // (with quantization, in one frame padded for the line ends).
            let tile = vec![&fixtures[4], &fixtures[5]];
            assert_eq!(fitting_source(&tile, &options).primitives, 2);
            sources.push(tile);
            for items in sources {
                let written = written_len(fitting_source(&items, &options), policy).unwrap();
                let counted = counted_len(fitting_source(&items, &options), policy).unwrap();
                assert_eq!(
                    counted,
                    written,
                    "quantize {quantize} items {}",
                    items.len()
                );
            }
        }
    }

    #[test]
    fn counted_fitting_refuses_exactly_where_writing_and_admission_refuse() {
        // The writer additionally serializes the bytes and runs json::admit.
        // At and around each JSON threshold that admission enforces (bytes,
        // value nodes, depth), the counted path must decide identically.
        fn nodes(value: &Value) -> usize {
            1 + match value {
                Value::Array(values) => values.iter().map(nodes).sum(),
                Value::Object(values) => values.values().map(nodes).sum(),
                _ => 0,
            }
        }
        fn depth(value: &Value) -> u64 {
            match value {
                Value::Array(values) => 1 + values.iter().map(depth).max().unwrap_or(0),
                Value::Object(values) => 1 + values.values().map(depth).max().unwrap_or(0),
                _ => 0,
            }
        }
        let options = VectorOptions::default();
        let feature = fitting_fixtures()[3].clone();
        let probe = fitting_source(&[&feature], &options);
        let (n, d) = (nodes(&probe.document), depth(&probe.document));
        let total = written_len(probe, ProducerPolicy::new(false).unwrap()).unwrap();
        let mut decided = [0usize; 2];
        let mut check = |limits: codec::CompressionLimits| {
            let policy = ProducerPolicy {
                limits: limits.validate().unwrap(),
                compress: false,
            };
            let written = written_len(fitting_source(&[&feature], &options), policy);
            let counted = counted_len(fitting_source(&[&feature], &options), policy);
            match (&written, &counted) {
                (Ok(a), Ok(b)) => {
                    assert_eq!(a, b);
                    decided[0] += 1;
                }
                (Err(Error::Job(a)), Err(Error::Job(b))) => {
                    assert_eq!(a.error.kind(), b.error.kind());
                    assert_eq!(a.error.to_string(), b.error.to_string());
                    decided[1] += 1;
                }
                _ => panic!("written {written:?} and counted {counted:?} disagree"),
            }
        };
        let base = codec::CompressionLimits::default();
        for json_bytes in total.saturating_sub(64)..=total {
            check(codec::CompressionLimits { json_bytes, ..base });
        }
        for json_value_nodes in n - 2..=n + 1 {
            check(codec::CompressionLimits {
                json_value_nodes,
                ..base
            });
        }
        for json_depth in d - 1..=d + 1 {
            check(codec::CompressionLimits { json_depth, ..base });
        }
        for source_bytes in total - 1..=total {
            check(codec::CompressionLimits {
                source_bytes,
                ..base
            });
        }
        assert!(decided[0] > 0 && decided[1] > 0, "{decided:?}");
    }

    /// The pre-counting fitting oracle: encode the trial content in full
    /// (bridge writer, JSON admission, meshopt when requested) and return
    /// (vertices, content bytes).
    fn trial_encode(
        feature: &Feature,
        options: &VectorOptions,
        policy: ProducerPolicy,
    ) -> (usize, usize) {
        let run = crate::RunControl::default();
        let attempt = run.begin().unwrap();
        let (lo, hi) = bounds(feature.geometry.points()).unwrap();
        let center = mul(add(lo, hi), 0.5);
        let schemas = feature_schemas(feature);
        let encoded = emit(
            &[feature],
            center,
            Quantize::Own,
            false,
            &schemas,
            options,
            &mut Vec::new(),
            policy,
            &attempt,
        )
        .unwrap();
        (encoded.vertices, encoded.bytes.len())
    }

    #[test]
    fn feature_at_exact_size_limit_fits_with_and_without_trial_encode() {
        // Uncompressed fitting counts bytes (no trial GLB, no admission);
        // meshopt fitting still encodes the trial content. Against the full
        // trial-encode oracle, both accept a feature exactly at the byte and
        // vertex limits (after the implicit framing reservation) and reject it
        // one below, including a fill fragment and an outline piece.
        let fixtures = fitting_fixtures();
        for meshopt in [false, true] {
            let policy = ProducerPolicy::new(meshopt).unwrap();
            let base = VectorOptions {
                meshopt,
                ..Default::default()
            };
            for feature in &fixtures {
                let (vertices, bytes) = trial_encode(feature, &base, policy);
                let limit = bytes + IMPLICIT_FRAMING_RESERVE;
                let fits = |max_bytes: usize, max_vertices: usize| {
                    let options = VectorOptions {
                        max_bytes,
                        max_vertices,
                        ..base.clone()
                    };
                    let run = crate::RunControl::default();
                    let attempt = run.begin().unwrap();
                    fits_feature(feature, false, &options, policy, &attempt).unwrap()
                };
                let case = format!("meshopt {meshopt} {:?}", feature.geometry);
                assert!(fits(limit, vertices), "{case}");
                assert!(!fits(limit - 1, vertices), "{case}");
                assert!(!fits(limit, vertices - 1), "{case}");
            }
        }
    }

    /// Unsigned scalar values of one accessor from `read_views` output.
    fn scalars(document: &Value, views: &[Vec<u8>], accessor: &Value) -> (u64, Vec<u32>) {
        let accessor = &document["accessors"][accessor.as_u64().unwrap() as usize];
        let view = &views[accessor["bufferView"].as_u64().unwrap() as usize];
        let count = accessor["count"].as_u64().unwrap() as usize;
        let component = accessor["componentType"].as_u64().unwrap();
        let values = (0..count)
            .map(|i| match component {
                5123 => u16::from_le_bytes(view[i * 2..i * 2 + 2].try_into().unwrap()) as u32,
                5125 => u32::from_le_bytes(view[i * 4..i * 4 + 4].try_into().unwrap()),
                other => panic!("unexpected index component {other}"),
            })
            .collect();
        (component, values)
    }

    /// A fragmented fill through the actual final producer: one GLB with the
    /// vector content marker, each fill triangle an EXT_mesh_polygon polygon
    /// with its own counterclockwise 3-index loop, 16-bit loops, the outline
    /// in the same GLB (its two pieces joined into one closed strip), and
    /// Cesium's loop-remap invariant.
    #[test]
    fn fragmented_fill_is_one_vector_glb_with_a_polygon_per_triangle() {
        let mut fill = point_feature();
        // Four triangles of a 2x2 square at constant height (best-fit plane).
        let square = |x: f64, z: f64| {
            vec![
                vec![
                    [x, 0., z],
                    [x + 1., 0., z],
                    [x + 1., 0., z + 1.],
                    [x, 0., z],
                ],
                vec![
                    [x, 0., z],
                    [x + 1., 0., z + 1.],
                    [x, 0., z + 1.],
                    [x, 0., z],
                ],
            ]
        };
        fill.geometry = Geometry::MultiPolygon(
            [square(0., 0.), square(1., 0.)]
                .into_iter()
                .flatten()
                .map(|ring| vec![ring])
                .collect(),
        );
        fill.surface_fragment = true;
        // The polygon's outline, cut by the line splitter into two pieces.
        let ring = outline::source_rings(
            &[[0., 0., 0.], [2., 0., 0.], [2., 0., 1.], [0., 0., 1.]],
            &[0, 1, 2, 3],
        )
        .remove(0);
        assert_eq!(ring.len(), 6);
        let pieces = [&ring[..=3], &ring[3..]].map(|run| {
            let mut piece = fill.clone();
            piece.geometry = Geometry::LineString(run.to_vec());
            piece
        });
        for (quantize, meshopt) in [(false, false), (true, false), (true, true)] {
            let options = VectorOptions {
                quantize,
                meshopt,
                ..VectorOptions::default()
            };
            let root = tempfile::tempdir().unwrap();
            let spool = root.path().join("features.sqlite");
            let db = rusqlite::Connection::open(&spool).unwrap();
            db.execute_batch("CREATE TABLE features(id INTEGER PRIMARY KEY,path TEXT,data TEXT,lx,ly,lz,hx,hy,hz,omissible DEFAULT 0); CREATE TABLE vertices(x REAL,y REAL,z REAL,shared INTEGER);").unwrap();
            for (id, feature) in std::iter::once(&fill).chain(&pieces).enumerate() {
                db.execute(
                    "INSERT INTO features(id,path,data) VALUES(?1,'',?2)",
                    rusqlite::params![id as i64 + 1, serde_json::to_string(feature).unwrap()],
                )
                .unwrap();
            }
            drop(db);
            std::fs::create_dir(root.path().join("t")).unwrap();
            let schemas = BTreeMap::new();
            let run = crate::RunControl::default();
            let attempt = run.begin().unwrap();
            let encoder = Encoder {
                spool: &spool,
                pass: GridPass::Single,
                max_features: 4,
                repair: false,
                options: &options,
                schemas: &schemas,
                output: root.path(),
                policy: ProducerPolicy::new(meshopt).unwrap(),
                attempt: &attempt,
            };
            let node = tile_json(encode(encoder, "", [0.; 3], 0, 0.).unwrap().node.unwrap());
            assert!(node.get("contents").is_none());
            let content = &node["content"];
            assert_eq!(
                content["extensions"],
                json!({"3DTILES_content_gltf_vector":{"vector":true}})
            );
            let uri = content["uri"].as_str().unwrap();
            assert!(uri.starts_with("t/") && uri.ends_with(".glb"), "{uri}");
            let names: Vec<_> = std::fs::read_dir(root.path().join("t"))
                .unwrap()
                .map(|e| e.unwrap().file_name().into_string().unwrap())
                .collect();
            assert_eq!(names, [uri.strip_prefix("t/").unwrap()]);
            let bytes = std::fs::read(root.path().join(uri)).unwrap();
            inspect_payload(crate::content_integrity::payload::PayloadKind::Glb, &bytes).unwrap();
            if !meshopt {
                let (document, views) = read_views(&bytes);
                assert!(document.get("materials").is_none());
                let primitives = document["meshes"][0]["primitives"].as_array().unwrap();
                let modes: Vec<_> = primitives.iter().map(|p| p["mode"].clone()).collect();
                assert_eq!(
                    modes,
                    [json!(3), json!(4)],
                    "the outline shares the fill GLB"
                );
                let line = &primitives[0];
                let (_, line_indices) = scalars(&document, &views, &line["indices"]);
                assert_eq!(line_indices.len(), 6, "one closed strip, no restart");
                assert!(!line_indices.contains(&u32::MAX));
                let line_view = &views[document["accessors"]
                    [line["attributes"]["POSITION"].as_u64().unwrap() as usize]["bufferView"]
                    .as_u64()
                    .unwrap() as usize];
                let stride = if quantize { 8 } else { 12 };
                let vertex = |i: u32| &line_view[i as usize * stride..][..stride];
                assert_eq!(vertex(line_indices[0]), vertex(line_indices[5]), "closed");
                let fill = &primitives[1];
                let polygon = &fill["extensions"]["EXT_mesh_polygon"];
                assert_eq!(polygon["count"], 4);
                let (_, indices) = scalars(&document, &views, &fill["indices"]);
                let (loop_type, loops) = scalars(&document, &views, &polygon["loopIndices"]);
                let (_, offsets) = scalars(&document, &views, &polygon["indicesOffsets"]);
                let (_, loop_offsets) = scalars(&document, &views, &polygon["loopIndicesOffsets"]);
                assert_eq!(loop_type, 5123, "16-bit loops when vertices fit");
                assert_eq!(offsets, [0, 3, 6, 9]);
                assert_eq!(loop_offsets, [0, 4, 8, 12]);
                let vertices = document["accessors"]
                    [fill["attributes"]["POSITION"].as_u64().unwrap() as usize]["count"]
                    .as_u64()
                    .unwrap() as usize;
                // CesiumJS 1.146.0 remaps triangle vertices through a table
                // sized by the loop list: every vertex must index inside it.
                assert!(indices.iter().all(|i| (*i as usize) < loops.len()));
                assert!(vertices <= loops.len());
                let rings: Vec<_> = loops.split(|i| *i == 0xFFFF).collect();
                assert_eq!(rings.len(), 4);
                let position = document["accessors"]
                    [fill["attributes"]["POSITION"].as_u64().unwrap() as usize]["bufferView"]
                    .as_u64()
                    .unwrap() as usize;
                let point = |i: u32| -> [f64; 3] {
                    let view = &views[position];
                    if quantize {
                        std::array::from_fn(|k| {
                            u16::from_le_bytes(
                                view[i as usize * 8 + k * 2..i as usize * 8 + k * 2 + 2]
                                    .try_into()
                                    .unwrap(),
                            ) as f64
                        })
                    } else {
                        std::array::from_fn(|k| {
                            f32::from_le_bytes(
                                view[i as usize * 12 + k * 4..i as usize * 12 + k * 4 + 4]
                                    .try_into()
                                    .unwrap(),
                            ) as f64
                        })
                    }
                };
                for (n, ring) in rings.iter().enumerate() {
                    let triangle = &indices[n * 3..n * 3 + 3];
                    let mut a = ring.to_vec();
                    let mut b = triangle.to_vec();
                    a.sort();
                    b.sort();
                    assert_eq!(a, b, "polygon {n} loop is its own triangle");
                    // Counterclockwise seen from glTF +Y (local up).
                    let area: f64 = (0..3)
                        .map(|k| {
                            let (p, q) = (point(ring[k]), point(ring[(k + 1) % 3]));
                            p[2] * q[0] - p[0] * q[2]
                        })
                        .sum();
                    assert!(area > 0., "polygon {n} exterior is counterclockwise");
                }
            }
        }
    }

    #[test]
    fn cesium_loop_remap_check_refuses_uncovered_vertices() {
        assert!(cesium_loop_remap_fits(3, &[0, 1, 2], &[0, 1, 2]).is_ok());
        let two = [0, 1, 2, u32::MAX, 3, 4, 5];
        assert!(cesium_loop_remap_fits(6, &[0, 1, 2, 3, 4, 5], &two).is_ok());
        assert!(cesium_loop_remap_fits(4, &[0, 1, 2], &[0, 1, 2]).is_err());
        // A triangle corner at or beyond the loop count is refused even when
        // the vertex count alone would pass.
        assert!(cesium_loop_remap_fits(3, &[0, 1, 3], &[0, 1, 2]).is_err());
    }

    #[test]
    fn polygon_accessors_use_16_bits_only_below_the_restart_value() {
        for (values, component) in [
            (vec![0, 65534, u32::MAX], 5123),
            (vec![u32::MAX], 5123),
            (vec![0, 65535], 5125),
            (vec![70000, u32::MAX], 5125),
        ] {
            let mut glb = MetadataGlb::new("test");
            let accessor = polygon_accessor(&mut glb, &values);
            let (document, binary) = glb.into_parts();
            let accessor = &document["accessors"][accessor];
            assert_eq!(accessor["componentType"], component, "{values:?}");
            assert_eq!(accessor["count"], values.len());
            let view = &document["bufferViews"][accessor["bufferView"].as_u64().unwrap() as usize];
            let offset = view["byteOffset"].as_u64().unwrap_or(0) as usize;
            let decoded: Vec<u32> = (0..values.len())
                .map(|i| match component {
                    5123 => match u16::from_le_bytes(
                        binary[offset + 2 * i..offset + 2 * i + 2]
                            .try_into()
                            .unwrap(),
                    ) {
                        u16::MAX => u32::MAX,
                        v => v as u32,
                    },
                    _ => u32::from_le_bytes(
                        binary[offset + 4 * i..offset + 4 * i + 4]
                            .try_into()
                            .unwrap(),
                    ),
                })
                .collect();
            assert_eq!(decoded, values);
        }
    }

    /// A tile mixing a fragmented fill and an ordinary polygon with a hole:
    /// one triangle primitive whose polygons keep separate offsets, loops
    /// that cover their own triangles, and Cesium's remap invariant.
    #[test]
    fn merged_fill_and_holed_polygon_share_one_polygon_primitive() {
        let mut fill = point_feature();
        fill.geometry = Geometry::MultiPolygon(vec![
            vec![vec![[0., 0., 0.], [1., 0., 0.], [1., 0., 1.], [0., 0., 0.]]],
            vec![vec![[0., 0., 0.], [1., 0., 1.], [0., 0., 1.], [0., 0., 0.]]],
        ]);
        fill.surface_fragment = true;
        let mut holed = point_feature();
        holed.properties.insert("_source_id".into(), json!("h"));
        holed.geometry = Geometry::Polygon(vec![
            vec![
                [10., 0., 0.],
                [14., 0., 0.],
                [14., 0., 4.],
                [10., 0., 4.],
                [10., 0., 0.],
            ],
            vec![
                [11., 0., 1.],
                [11., 0., 2.],
                [12., 0., 2.],
                [12., 0., 1.],
                [11., 0., 1.],
            ],
        ]);
        let options = VectorOptions::default();
        let run = crate::RunControl::default();
        let attempt = run.begin().unwrap();
        let encoded = emit(
            &[&fill, &holed],
            [0.; 3],
            Quantize::Own,
            false,
            &BTreeMap::new(),
            &options,
            &mut Vec::new(),
            ProducerPolicy::new(false).unwrap(),
            &attempt,
        )
        .unwrap();
        let (document, views) = read_views(&encoded.bytes);
        let primitives = document["meshes"][0]["primitives"].as_array().unwrap();
        assert_eq!(primitives.len(), 1);
        let primitive = &primitives[0];
        let polygon = &primitive["extensions"]["EXT_mesh_polygon"];
        assert_eq!(polygon["count"], 3);
        let (_, indices) = scalars(&document, &views, &primitive["indices"]);
        let (_, offsets) = scalars(&document, &views, &polygon["indicesOffsets"]);
        let (_, loops) = scalars(&document, &views, &polygon["loopIndices"]);
        let (_, loop_offsets) = scalars(&document, &views, &polygon["loopIndicesOffsets"]);
        assert_eq!(offsets[..2], [0, 3]);
        assert_eq!(loop_offsets[..2], [0, 4]);
        assert!(indices.iter().all(|i| (*i as usize) < loops.len()));
        let ends =
            |list: &[u32], n: usize, all: usize| list.get(n + 1).map_or(all, |v| *v as usize);
        for n in 0..3 {
            let triangles = &indices[offsets[n] as usize..ends(&offsets, n, indices.len())];
            let edges: BTreeSet<_> = triangles
                .as_chunks::<3>()
                .0
                .iter()
                .flat_map(|t| [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])])
                .collect();
            let own = &loops[loop_offsets[n] as usize..ends(&loop_offsets, n, loops.len())];
            let rings: Vec<_> = own
                .split(|i| *i == 0xFFFF)
                .filter(|r| !r.is_empty())
                .collect();
            assert_eq!(rings.len(), if n == 2 { 2 } else { 1 }, "polygon {n}");
            for ring in rings {
                for k in 0..ring.len() {
                    let edge = (ring[k], ring[(k + 1) % ring.len()]);
                    assert!(edges.contains(&edge), "polygon {n} loop edge {edge:?}");
                }
            }
        }
    }

    #[test]
    fn actual_bridge_resource_refusal_is_fatal_during_fitting() {
        for meshopt in [false, true] {
            let options = VectorOptions {
                meshopt,
                skip_invalid: true,
                ..VectorOptions::default()
            };
            let run = crate::RunControl::default();
            let attempt = run.begin().unwrap();
            let limits = codec::CompressionLimits {
                source_bytes: 1,
                ..Default::default()
            };
            let policy = ProducerPolicy {
                limits: limits.validate().unwrap(),
                compress: meshopt,
            };
            let error =
                fits_feature(&point_feature(), false, &options, policy, &attempt).unwrap_err();
            let FeatureFailure::Fatal(Error::Job(failure)) = error else {
                panic!("resource refusal cannot become a feature rejection or false fitting result")
            };
            assert_eq!(failure.error.kind(), crate::JobErrorKind::ResourceLimit);
            assert!(failure.error.same_cause(&attempt.check().unwrap_err()));
            assert!(std::error::Error::source(&failure.error).is_some());
        }
    }

    #[test]
    fn actual_bridge_resource_refusal_stops_final_encoding_before_publication() {
        for meshopt in [false, true] {
            let options = VectorOptions {
                meshopt,
                skip_invalid: true,
                ..Default::default()
            };
            let run = crate::RunControl::default();
            let attempt = run.begin().unwrap();
            let limits = codec::CompressionLimits {
                source_bytes: 1,
                ..Default::default()
            };
            let policy = ProducerPolicy {
                limits: limits.validate().unwrap(),
                compress: meshopt,
            };
            let failure = fatal_job(
                final_point_error(&options, policy, &attempt),
                crate::JobErrorKind::ResourceLimit,
                &attempt,
            );
            assert!(std::error::Error::source(&failure.error).is_some());
            assert!(
                run.begin().is_err(),
                "the producer cannot begin a retry Attempt"
            );
        }
    }

    #[test]
    fn actual_native_refusal_stops_fitting_and_final_without_raw_fallback() {
        for fitting in [true, false] {
            let options = VectorOptions {
                meshopt: true,
                skip_invalid: true,
                ..Default::default()
            };
            let run = crate::RunControl::default();
            let attempt = run.begin().unwrap();
            let policy = ProducerPolicy::new(true).unwrap();
            let (error, reached) = codec::with_capacity_one(|| {
                if fitting {
                    let error = fits_feature(&point_feature(), false, &options, policy, &attempt)
                        .unwrap_err();
                    let FeatureFailure::Fatal(error) = error else {
                        panic!("native refusal cannot reject a feature")
                    };
                    error
                } else {
                    final_point_error(&options, policy, &attempt)
                }
            });
            assert_eq!(
                reached, 1,
                "first actual encoder refusal stops without a second stream or fallback"
            );
            let failure = fatal_job(error, crate::JobErrorKind::InvalidState, &attempt);
            assert!(matches!(
                std::error::Error::source(&failure.error)
                    .unwrap()
                    .downcast_ref::<CodecError<JobError>>()
                    .unwrap(),
                CodecError::EncodingFailure("ATTRIBUTES encoder")
            ));
            assert!(run.begin().is_err());
        }
    }

    #[test]
    fn actual_authored_format_faults_stop_fitting_and_final_with_typed_causes() {
        for fault in [
            GeneratedFault::ViewRange,
            GeneratedFault::UnknownDeclaration,
        ] {
            for meshopt in [false, true] {
                for fitting in [true, false] {
                    let options = VectorOptions {
                        meshopt,
                        skip_invalid: true,
                        ..Default::default()
                    };
                    let run = crate::RunControl::default();
                    let attempt = run.begin().unwrap();
                    let policy = ProducerPolicy::new(meshopt).unwrap();
                    let (error, reached) = with_generated_fault(fault, || {
                        if fitting {
                            let error =
                                fits_feature(&point_feature(), false, &options, policy, &attempt)
                                    .unwrap_err();
                            let FeatureFailure::Fatal(error) = error else {
                                panic!("authored bytes cannot reject a feature")
                            };
                            error
                        } else {
                            final_point_error(&options, policy, &attempt)
                        }
                    });
                    assert_eq!(reached, 1);
                    let failure = fatal_job(error, crate::JobErrorKind::InvalidState, &attempt);
                    let source = std::error::Error::source(&failure.error)
                        .unwrap()
                        .downcast_ref::<FormatError>()
                        .unwrap();
                    match fault {
                        GeneratedFault::ViewRange => {
                            assert!(matches!(source, FormatError::InvalidInput(_)))
                        }
                        GeneratedFault::UnknownDeclaration => {
                            assert!(matches!(source, FormatError::Unsupported(_)))
                        }
                    }
                    assert!(run.begin().is_err());
                }
            }
        }
    }

    #[test]
    fn authored_invalid_and_unsupported_bridge_bytes_have_typed_internal_cause() {
        for (bad_view, declaration) in
            [(true, "EXT_structural_metadata"), (false, "VENDOR_unknown")]
        {
            let run = crate::RunControl::default();
            let attempt = run.begin().unwrap();
            let document = json!({"extensionsUsed":[declaration],"buffers":[{"byteLength":8}],"accessors":[],
                "bufferViews":[{"buffer":0,"byteOffset":0,"byteLength":if bad_view {9}else{8}}]});
            let error = bridge(
                document,
                vec![0; 8],
                ProducerPolicy::new(false).unwrap(),
                &attempt,
            )
            .unwrap_err();
            let Error::Job(failure) = error else {
                panic!("authored format failure must retain a job cause")
            };
            assert_eq!(failure.error.kind(), crate::JobErrorKind::InvalidState);
            let source = std::error::Error::source(&failure.error)
                .unwrap()
                .downcast_ref::<FormatError>()
                .unwrap();
            assert!(matches!(source, FormatError::InvalidInput(_)) == bad_view);
            assert!(matches!(source, FormatError::Unsupported(_)) != bad_view);
            if let FormatError::InvalidInput(reason) = source {
                assert_eq!(reason, "generated vector view range or alignment");
            }
        }
    }

    #[test]
    fn fitting_preserves_selected_checkpoint_cause_before_format_work() {
        let run = crate::RunControl::default();
        let attempt = run.begin().unwrap();
        assert!(run.cancellation_handle().cancel());
        let original = attempt.check().unwrap_err();
        let options = VectorOptions {
            skip_invalid: true,
            ..Default::default()
        };
        let error = fits_feature(
            &point_feature(),
            false,
            &options,
            ProducerPolicy::new(options.meshopt).unwrap(),
            &attempt,
        )
        .unwrap_err();
        let FeatureFailure::Fatal(Error::Job(failure)) = error else {
            panic!("checkpoint must remain fatal")
        };
        assert!(failure.error.same_cause(&original));
        assert!(failure.secondary.is_empty());
        assert!(
            run.begin().is_err(),
            "the original control remains claimed once"
        );
    }

    #[test]
    fn final_preserves_checkpoint_priority_before_generated_or_native_work() {
        let run = crate::RunControl::default();
        let attempt = run.begin().unwrap();
        assert!(run.cancellation_handle().cancel());
        let original = attempt.check().unwrap_err();
        let options = VectorOptions {
            meshopt: true,
            skip_invalid: true,
            ..Default::default()
        };
        let ((error, native_reached), generated_reached) =
            with_generated_fault(GeneratedFault::ViewRange, || {
                codec::with_capacity_one(|| {
                    final_point_error(&options, ProducerPolicy::new(true).unwrap(), &attempt)
                })
            });
        assert_eq!((native_reached, generated_reached), (0, 0));
        let failure = fatal_job(error, crate::JobErrorKind::Cancelled, &attempt);
        assert!(failure.error.same_cause(&original));
        assert!(run.begin().is_err());
    }

    #[test]
    fn float64_metadata_checks_original_signed_and_unsigned_integer_bounds() {
        let feature = |value| Feature {
            intrinsic: None,
            properties: BTreeMap::from([
                ("_source_id".into(), json!("0")),
                ("value".into(), value),
            ]),
            geometry: Geometry::Point([0.; 3]),
            surface_fragment: false,
            fragment_path: String::new(),
        };
        for schemas in [
            BTreeMap::new(),
            BTreeMap::from([("value".into(), "real".into())]),
        ] {
            let real = feature(json!(1.5));
            for integer in [
                json!((1i64 << 53) + 1),
                json!(-((1i64 << 53) + 1)),
                json!(i64::MIN),
                json!(i64::MAX),
                json!(u64::MAX),
            ] {
                let source = feature(integer);
                let mut glb = MetadataGlb::new("test");
                let error = metadata(&mut glb, &[&source, &real], &schemas).unwrap_err();
                assert!(error
                    .to_string()
                    .contains("large integers as float64 without loss"));
            }
            for integer in [
                -(1i64 << 53),
                -(1i64 << 53) + 1,
                (1i64 << 53) - 1,
                1i64 << 53,
            ] {
                let source = feature(json!(integer));
                let mut glb = MetadataGlb::new("test");
                metadata(&mut glb, &[&source, &real], &schemas).unwrap();
                let column = &glb.document["extensions"]["EXT_structural_metadata"]
                    ["propertyTables"][0]["properties"]["value"];
                let view =
                    &glb.document["bufferViews"][column["values"].as_u64().unwrap() as usize];
                let offset = view["byteOffset"].as_u64().unwrap() as usize;
                let bytes = glb.finish().unwrap();
                let json_length = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
                let start = 28 + json_length + offset;
                let decoded = f64::from_le_bytes(bytes[start..start + 8].try_into().unwrap());
                assert_eq!(decoded, integer as f64);
                assert_eq!(decoded as i64, integer);
            }
        }
    }
    #[test]
    fn parent_budget_stops_before_decoding_the_next_original() {
        for (name, label, max_vertices, expected) in [
            ("vertices", "", 4, "vertices"),
            ("bytes", "x", 64, "estimatedBytes"),
        ] {
            let root = tempfile::tempdir().unwrap();
            let spool = root.path().join(format!("{name}.sqlite"));
            let db = rusqlite::Connection::open(&spool).unwrap();
            db.execute_batch("CREATE TABLE features(id INTEGER PRIMARY KEY,path TEXT,data TEXT,lx,ly,lz,hx,hy,hz,omissible DEFAULT 0); CREATE TABLE vertices(x REAL,y REAL,z REAL,shared INTEGER);").unwrap();
            let points: Vec<_> = (0..5).map(|i| [i as f64, 0., 0.]).collect();
            let feature = json!({"properties":{"_source_id":"1","name":label.repeat(10000)},"geometry":{"type":"LineString","coordinates":points}});
            db.execute(
                "INSERT INTO features(id,path,data) VALUES(1,'',?1)",
                [feature.to_string()],
            )
            .unwrap();
            // This row would fail JSON decoding if the guard retained/loading
            // loop continued past the first candidate's budget failure.
            db.execute(
                "INSERT INTO features(id,path,data) VALUES(2,'','deliberately invalid JSON')",
                [],
            )
            .unwrap();
            for p in points {
                db.execute(
                    "INSERT INTO vertices VALUES(?1,?2,?3,1)",
                    rusqlite::params![p[0], p[1], p[2]],
                )
                .unwrap();
            }
            drop(db);
            let options = VectorOptions {
                max_vertices,
                max_bytes: 4096,
                ..Default::default()
            };
            let schemas = BTreeMap::new();
            let run = crate::RunControl::default();
            let attempt = run.begin().unwrap();
            let encoder = Encoder {
                spool: &spool,
                pass: GridPass::Single,
                max_features: 1,
                repair: false,
                options: &options,
                schemas: &schemas,
                output: root.path(),
                policy: ProducerPolicy::new(options.meshopt).unwrap(),
                attempt: &attempt,
            };
            let candidate = encode(encoder, "", [0.; 3], 1, options.lod.tolerance_metres).unwrap();
            assert!(candidate.node.is_none());
            assert_eq!(candidate.reason, Some(expected));
        }
    }

    /// A parent simplification that the tile encoder would refuse keeps the
    /// source polygon instead of failing the build at level > 0. Removing the
    /// raised exterior vertex flattens the best-fit plane, and in that plane the
    /// raised hole crosses the exterior: valid before simplification, refused
    /// by `polygon_for` (topology, or loop coverage after repair) after it.
    #[test]
    fn parent_simplification_the_encoder_refuses_keeps_the_source_polygon() {
        let exterior = vec![
            [0., 0., 0.],
            [50., 6., 0.],
            [100., 0., 0.],
            [100., 0., 100.],
            [0., 0., 100.],
            [0., 0., 0.],
        ];
        let hole = vec![
            [40., 30., 90.],
            [40., 30., 100.3],
            [60., 30., 100.3],
            [60., 30., 90.],
            [40., 30., 90.],
        ];
        let flattened: Vec<_> = exterior.iter().copied().filter(|p| p[1] == 0.).collect();
        let mut source = point_feature();
        source.geometry = Geometry::Polygon(vec![exterior.clone(), hole.clone()]);
        for repair in [false, true] {
            geometry::polygon_for(
                &source,
                &[exterior.clone(), hole.clone()],
                0,
                [0.; 3],
                repair,
            )
            .unwrap();
            let refused = geometry::polygon_for(
                &source,
                &[flattened.clone(), hole.clone()],
                0,
                [0.; 3],
                repair,
            );
            assert!(
                matches!(refused, Err(FeatureFailure::Rejected(_))),
                "repair {repair}: the flattened candidate must be refused"
            );
            let root = tempfile::tempdir().unwrap();
            std::fs::create_dir(root.path().join("t")).unwrap();
            let spool = root.path().join("spool.sqlite");
            let db = rusqlite::Connection::open(&spool).unwrap();
            db.execute_batch("CREATE TABLE features(id INTEGER PRIMARY KEY,path TEXT,data TEXT,lx,ly,lz,hx,hy,hz,omissible DEFAULT 0); CREATE TABLE vertices(x REAL,y REAL,z REAL,shared INTEGER);").unwrap();
            let feature = json!({"properties":{"_source_id":"raised"},"geometry":{"type":"Polygon","coordinates":[exterior, hole]}});
            db.execute(
                "INSERT INTO features(id,path,data) VALUES(1,'',?1)",
                [feature.to_string()],
            )
            .unwrap();
            // Lock every vertex except the raised one, which RDP then removes.
            for p in exterior.iter().chain(&hole) {
                db.execute(
                    "INSERT INTO vertices VALUES(?1,?2,?3,?4)",
                    rusqlite::params![p[0], p[1], p[2], i32::from(p[1] != 6.)],
                )
                .unwrap();
            }
            drop(db);
            let options = VectorOptions {
                lod: crate::vector::VectorLodOptions {
                    tolerance_metres: 80.,
                    levels: 3,
                },
                max_bytes: 1 << 20,
                ..Default::default()
            };
            let schemas = BTreeMap::new();
            let run = crate::RunControl::default();
            let attempt = run.begin().unwrap();
            let encoder = Encoder {
                spool: &spool,
                pass: GridPass::Single,
                max_features: 1,
                repair,
                options: &options,
                schemas: &schemas,
                output: root.path(),
                policy: ProducerPolicy::new(false).unwrap(),
                attempt: &attempt,
            };
            let candidate = encode(encoder, "", [0.; 3], 1, options.lod.tolerance_metres).unwrap();
            let node = tile_json(candidate.node.expect("the parent keeps the source polygon"));
            assert_eq!(
                node["extras"]["geometryErrorMetres"],
                json!(0.),
                "repair {repair}"
            );
            assert_eq!(node["extras"]["vertices"], json!(9), "repair {repair}");
            assert!(
                candidate.reports.iter().any(|report| report["reason"]
                    == "simplified polygon would not encode; source polygon retained"),
                "repair {repair}: {:?}",
                candidate.reports
            );
        }
    }
}
