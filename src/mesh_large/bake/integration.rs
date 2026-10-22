//! H17's immutable handoff adapter and H27's real complete-member protocol.
//! Missing owner leases/retained leaf frames are explicit integration errors;
//! no untextured or geometry-only substitute can satisfy this entry point.
use super::{
    assembler, bake_node, child_bridge, invalid, resource, types::*, CapturedSource, ChildView,
    SourceImages,
};
use crate::{
    mesh_large::{
        bake_pass as root,
        lod::handoff,
        records::{Bounds, NodeKind, NodeRecord},
    },
    JobError,
};

/// Policy resolved by the caller from the actual stage-two allowance. Profile
/// and complete-member budget are owned by the live pass and set from it.
#[derive(Debug)]
pub(crate) struct ProxyTextureBaker {
    limits: BakeLimits,
    /// Child texel tiles parents read from retained packets (diagnostics).
    child_tile_reads: std::sync::atomic::AtomicU64,
}
#[cfg(test)]
thread_local! {
    /// Test seam: resolve the bake allowance on this thread as on a host that reports
    /// no available memory (macOS and Windows today), i.e. the fixed 1 GiB fallback.
    pub(crate) static UNKNOWN_MEMORY_HOST: std::cell::Cell<bool> =
        const { std::cell::Cell::new(false) };
}
impl ProxyTextureBaker {
    pub(crate) fn new(limits: BakeLimits) -> Result<Self, JobError> {
        if limits.memory_bytes == 0
            || limits.source_cache_bytes == 0
            || limits.source_cache_bytes >= limits.memory_bytes
            || limits.query_work == 0
            || limits.max_page_edge < 4
            || !limits.max_page_edge.is_power_of_two()
        {
            return Err(invalid("resolved proxy bake resource policy"));
        }
        Ok(Self {
            limits,
            child_tile_reads: Default::default(),
        })
    }
    /// Child texel tiles parents have read from retained packets so far.
    #[cfg(test)]
    pub(crate) fn child_tile_reads(&self) -> u64 {
        self.child_tile_reads
            .load(std::sync::atomic::Ordering::Relaxed)
    }
    /// The production policy, resolved once from the actual run before the bake stage:
    /// - memory: half of what the host reports available after the captured state's
    ///   owned scratch (memory-backed), metadata and the stage reserve; a host that cannot
    ///   report its memory gets a fixed 1 GiB, never an unbounded allowance;
    /// - source cache: the source sampler's peak for the largest image (24 bytes per
    ///   pixel, its encoded bytes and 64 MiB), at most a quarter of the memory, and for
    ///   the workers' shared cache of decoded images, room for every image's sampling
    ///   chain (4 bytes per pixel, a third more for its mips), at most an eighth;
    /// - query work: a finite runaway guard of 2^40 index visits per tile;
    /// - page edge: the profile's leaf page edge; eight complete-member density drops.
    pub(crate) fn for_stage(
        profile: crate::mesh_large::config::Profile,
        captured: &crate::mesh_large::full::FullCaptured,
    ) -> Result<Self, JobError> {
        use crate::mesh_large::config::{available_memory, Profile};
        const RESERVE: u64 = 256 << 20;
        let tmpfs = if captured.scratch.memory_backed() {
            captured.report.scratch.need_bytes
        } else {
            0
        };
        let owned = captured
            .report
            .memory
            .owned_metadata_bytes
            .saturating_add(captured.report.memory.geometry.document_bytes);
        let available = available_memory();
        #[cfg(test)]
        let available = available.filter(|_| !UNKNOWN_MEMORY_HOST.with(std::cell::Cell::get));
        let memory_bytes = match available {
            Some(a) => {
                a.saturating_sub(tmpfs)
                    .saturating_sub(owned)
                    .saturating_sub(RESERVE)
                    / 2
            }
            None => 1 << 30,
        };
        // The source sampler's own peak for its largest image: decoded chain at 24 bytes
        // per pixel, the encoded bytes and a fixed 64 MiB codec allowance.
        let source_cache_bytes = captured
            .image_info
            .iter()
            .zip(&captured.images)
            .map(|(i, snapshot)| {
                (u64::from(i.width) * u64::from(i.height))
                    .saturating_mul(24)
                    .saturating_add(snapshot.identity.encoded_bytes)
                    .saturating_add(64 << 20)
            })
            .max()
            .unwrap_or(1 << 20)
            .min(memory_bytes / 4);
        let shared_image_bytes = captured
            .image_info
            .iter()
            .map(|i| {
                (u64::from(i.width) * u64::from(i.height))
                    .saturating_mul(4)
                    .saturating_mul(4)
                    / 3
                    + 4096
            })
            .fold(0u64, u64::saturating_add)
            .min(memory_bytes / 8);
        let child_texel_bytes = super::CHILD_TEXEL_CACHE;
        if memory_bytes < 64 << 20 {
            return Err(resource(
                "available memory cannot hold the proxy texture bake stage",
            ));
        }
        Self::new(BakeLimits {
            profile,
            memory_bytes,
            member_bytes: crate::mesh_large::config::TILE_BYTES,
            source_cache_bytes,
            shared_image_bytes,
            child_texel_bytes,
            query_work: 1 << 40,
            max_page_edge: match profile {
                Profile::Fast => 2048,
                Profile::Lossless => 8192,
                Profile::Balanced | Profile::Quality => 4096,
            },
            max_density_drops: 8,
            id_textures: false,
        })
    }
    pub(crate) fn limits(&self) -> BakeLimits {
        self.limits
    }
    pub(super) fn limits_for(&self, pass: &root::BakePass<'_>) -> BakeLimits {
        BakeLimits {
            profile: pass.profile,
            member_bytes: pass.tile_bytes,
            ..self.limits
        }
    }
}
impl root::ProxyBaker for ProxyTextureBaker {
    /// Bake every coarse tile, children before parents. Tiles of one depth are independent
    /// (each reads only its own packet, certified member and retained children), so each
    /// depth runs on up to `pass.jobs` workers, deepest first; every worker owns its own
    /// source index and an equal share of the memory lease.
    fn bake(&self, pass: &root::BakePass<'_>) -> Result<(), JobError> {
        (pass.check)()?;
        let limits = self.limits_for(pass);
        if limits.member_bytes == 0 || pass.jobs == 0 {
            return Err(invalid("live proxy bake pass policy"));
        }
        // (first, depth) of every coarse tile: 12 bytes each, grouped by depth.
        let mut levels: std::collections::BTreeMap<u32, Vec<u64>> = Default::default();
        for next in pass.nodes()? {
            (pass.check)()?;
            let (first, depth) = next?;
            if first >= pass.captured.triangles.len() {
                return Err(invalid("proxy row outside captured hierarchy"));
            }
            levels.entry(depth).or_default().push(first);
        }
        let Some(widest) = levels.values().map(Vec::len).max() else {
            return Ok(());
        };
        pass.phases
            .begin_bake(levels.values().map(|l| l.len() as u64).sum())?;
        let workers = pass.jobs.min(widest).max(1);
        // Decoded source images are shared by every worker of every depth.
        let images = std::sync::Arc::new(SourceImages::new(limits.source_cache_bytes));
        for (depth, firsts) in levels.iter().rev() {
            let started = std::time::Instant::now();
            let retried = run_depth(
                firsts,
                workers,
                limits,
                pass.check,
                &|capacity| images.set_capacity(capacity),
                |limits, check| {
                    CapturedSource::shared(
                        pass.captured,
                        limits.source_cache_bytes,
                        images.clone(),
                        check,
                    )
                },
                |source, share, first, check| {
                    bake_tile(
                        pass,
                        share,
                        source,
                        first,
                        *depth,
                        check,
                        &self.child_tile_reads,
                    )
                },
            )?;
            pass.phases.baked_depth(
                *depth,
                firsts.len() as u64,
                retried,
                started.elapsed().as_secs_f64(),
            )?;
        }
        Ok(())
    }
}

/// How one tile's bake ended.
#[derive(Debug)]
pub(super) enum TileOutcome {
    Done,
    /// A resource refusal before anything was committed or released: the tile may run
    /// again with a larger share of the allowance.
    Retry(JobError),
}

/// A bake worker consults the run's cancellation state on one call in this many, and
/// on any call once [`CHECK_AFTER`] has passed since its last consultation.
const CHECK_EVERY: u32 = 64;
/// The longest a worker goes without consulting the run's cancellation state. The call
/// count alone would let coarse sites (a density drop, a page, a chart pass) wait about
/// `CHECK_EVERY` steps; the clock bounds that wait whatever the step length.
const CHECK_AFTER: std::time::Duration = std::time::Duration::from_millis(50);

/// One worker's throttle on the run's cancellation check.
pub(super) struct CheckThrottle {
    calls: u32,
    last: std::time::Instant,
}
impl CheckThrottle {
    pub(super) fn new() -> Self {
        Self {
            calls: 0,
            last: std::time::Instant::now(),
        }
    }
    /// Whether this call should consult the run's check: the first, every
    /// `CHECK_EVERY`-th, and any after `CHECK_AFTER` without one.
    pub(super) fn due(&mut self) -> bool {
        self.calls = self.calls.wrapping_add(1);
        if self.calls % CHECK_EVERY == 1 || self.last.elapsed() >= CHECK_AFTER {
            self.last = std::time::Instant::now();
            return true;
        }
        false
    }
}

/// Bake the tiles of one depth: first on up to `workers` threads, each with an equal
/// share of `limits.memory_bytes`; then every tile that refused its share runs again on
/// half as many threads, each share twice as large, and so on until a tile that still
/// refuses runs alone with the whole allowance. Admission therefore does not depend on
/// the worker count: a tile that fits the whole allowance succeeds at any `--jobs`, and
/// tiles too large for the first share still bake in parallel when two or more of them
/// fit a larger one. Any other failure stops the depth; siblings stop at their next
/// cancellation check and the first real failure is reported. Returns how many tiles
/// were retried with a larger share.
pub(super) fn run_depth<S>(
    firsts: &[u64],
    workers: usize,
    limits: BakeLimits,
    check: &(dyn Fn() -> Result<(), JobError> + Sync),
    images: &(dyn Fn(u64) + Sync),
    state: impl Fn(BakeLimits, &mut dyn FnMut() -> Result<(), JobError>) -> Result<S, JobError> + Sync,
    bake: impl Fn(
            &mut S,
            BakeLimits,
            u64,
            &mut dyn FnMut() -> Result<(), JobError>,
        ) -> Result<TileOutcome, JobError>
        + Sync,
) -> Result<u64, JobError> {
    let mut running = workers.min(firsts.len()).max(1);
    let mut pending = run_pass(firsts, running, limits, check, images, &state, &bake)?;
    let retried = pending.len() as u64;
    while !pending.is_empty() {
        pending.sort_unstable();
        running = (running / 2).min(pending.len()).max(1);
        pending = run_pass(&pending, running, limits, check, images, &state, &bake)?;
    }
    Ok(retried)
}

/// One pass of [`run_depth`] over `firsts` on `running` threads, each with an equal
/// share of the allowance. Returns the tiles that refused their share; with one thread
/// (the whole allowance) a refusal is the depth's failure.
fn run_pass<S>(
    firsts: &[u64],
    running: usize,
    limits: BakeLimits,
    check: &(dyn Fn() -> Result<(), JobError> + Sync),
    images: &(dyn Fn(u64) + Sync),
    state: &(impl Fn(BakeLimits, &mut dyn FnMut() -> Result<(), JobError>) -> Result<S, JobError>
          + Sync),
    bake: &(impl Fn(
        &mut S,
        BakeLimits,
        u64,
        &mut dyn FnMut() -> Result<(), JobError>,
    ) -> Result<TileOutcome, JobError>
          + Sync),
) -> Result<Vec<u64>, JobError> {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    // The source cache reservation (a decode peak, and a part of the shared cache of
    // decoded images) is part of each tile's charge, so it is capped by the worker's
    // share too; otherwise a cache sized for the whole allowance alone could make every
    // tile refuse its share. A smaller cache changes only residency, never results.
    let memory_bytes = limits.memory_bytes / running as u64;
    let share = BakeLimits {
        memory_bytes,
        source_cache_bytes: limits
            .source_cache_bytes
            .saturating_add(limits.shared_image_bytes / running as u64)
            .min(memory_bytes / 4),
        ..limits
    };
    // The shared source image cache holds what the running workers reserve for it.
    images(share.source_cache_bytes.saturating_mul(running as u64));
    let failed = AtomicBool::new(false);
    // Source, colour and child queries check for cancellation at every visited node and
    // candidate, and the run's check takes its shared state lock. Four workers convoyed
    // on it: a sixth of bake CPU on a 10M-triangle textured run, and on a 0.79M-triangle
    // vertex-colour crop 712 s of system time and 3232 worker seconds of raster (243 s
    // without it). A worker consults it on every `CHECK_EVERY`-th call (the first
    // included) and after every `CHECK_AFTER` of calls without one, and the
    // sibling-failure flag on every call. Only cancellation latency changes, and the
    // clock bounds it for coarse sites whose calls are few and far apart.
    let cancelled = |throttle: &mut CheckThrottle| -> Result<(), JobError> {
        if failed.load(Ordering::Relaxed) {
            return Err(JobError::new(
                crate::JobErrorKind::Cancelled,
                "another proxy bake worker failed",
            ));
        }
        if throttle.due() {
            check()
        } else {
            Ok(())
        }
    };
    let next = AtomicUsize::new(0);
    let deferred = std::sync::Mutex::new(Vec::new());
    let results: Vec<Result<(), JobError>> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..running)
            .map(|_| {
                scope.spawn(|| -> Result<(), JobError> {
                    let mut throttle = CheckThrottle::new();
                    let mut check = || cancelled(&mut throttle);
                    let mut worker = None;
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(&first) = firsts.get(i) else {
                            return Ok(());
                        };
                        let worker = match &mut worker {
                            Some(s) => s,
                            None => match state(share, &mut check) {
                                Ok(s) => worker.insert(s),
                                Err(e) => {
                                    failed.store(true, Ordering::Relaxed);
                                    return Err(e);
                                }
                            },
                        };
                        match bake(worker, share, first, &mut check) {
                            Ok(TileOutcome::Done) => {}
                            Ok(TileOutcome::Retry(_)) if running > 1 => deferred
                                .lock()
                                .map_err(|_| invalid("proxy bake retry list poisoned"))?
                                .push(first),
                            Ok(TileOutcome::Retry(e)) | Err(e) => {
                                failed.store(true, Ordering::Relaxed);
                                return Err(e);
                            }
                        }
                    }
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| {
                h.join()
                    .unwrap_or_else(|_| Err(invalid("a proxy bake worker panicked")))
            })
            .collect()
    });
    // Report the first real failure, not a sibling's induced cancellation.
    let induced = |e: &JobError| {
        e.kind() == crate::JobErrorKind::Cancelled
            && e.message() == "another proxy bake worker failed"
    };
    let errors: Vec<JobError> = results.into_iter().filter_map(Result::err).collect();
    if let Some(e) = errors
        .iter()
        .position(|e| !induced(e))
        .or((!errors.is_empty()).then_some(0))
    {
        return Err(errors.into_iter().nth(e).unwrap());
    }
    deferred
        .into_inner()
        .map_err(|_| invalid("proxy bake retry list poisoned"))
}

/// One coarse tile: read the frozen packet and the certified member under exact leases,
/// load the retained children, run the core with the owner's leased assembler, commit
/// the verified bytes, retain this tile's exact packet for its parent, then release its
/// children and packet.
fn bake_tile(
    pass: &root::BakePass<'_>,
    limits: BakeLimits,
    source: &mut CapturedSource<'_>,
    first: u64,
    depth: u32,
    check: &mut dyn FnMut() -> Result<(), JobError>,
    reads: &std::sync::atomic::AtomicU64,
) -> Result<TileOutcome, JobError> {
    // Nothing is written before the commit's own write: every memory-leased step (the
    // packet and member reads, the bake and the commit's decoded readback) comes first,
    // so a resource refusal anywhere in them may be retried with a larger share. Only
    // the scratch writes (the member, then this tile's packet for its parent) come after.
    let retryable = |e: JobError| {
        if e.kind() == crate::JobErrorKind::ResourceLimit {
            pass.phases
                .refused_tile(first, depth, limits.memory_bytes, e.message())?;
            Ok(TileOutcome::Retry(e))
        } else {
            Err(e)
        }
    };
    check()?;
    // The decoded packet stays resident beside the core, so its exact lease comes off
    // the worker's allowance first; the core owns the remainder.
    let prepared = (|| {
        let lease = pass.frozen_lease_bytes(first, depth)?;
        let core = limits
            .memory_bytes
            .checked_sub(lease)
            .filter(|core| *core >= lease)
            .ok_or_else(|| resource("frozen tile packet exceeds the bake memory lease"))?;
        let frozen = pass.frozen(first, depth, lease)?;
        let parent = node_record(&frozen)?;
        let glb = pass.read_certified_proxy(first, depth, frozen.glb_bytes)?;
        let children = child_bridge::load_children(pass, &frozen, &parent, core / 2, check)?;
        let accepted = bake_one(
            &frozen,
            glb,
            source,
            &children,
            BakeLimits {
                memory_bytes: core,
                ..limits
            },
            &mut |prepared, memory| pass.assemble(&frozen, prepared, memory),
            check,
        )?;
        drop(children);
        Ok((core, frozen, accepted, std::time::Instant::now()))
    })();
    let (core, frozen, accepted, commit_started) = match prepared {
        Ok(p) => p,
        Err(e) => return retryable(e),
    };
    let AcceptedNode {
        assembly,
        preencode,
        stats,
        ..
    } = accepted;
    // Its readback runs (and may refuse) before the member is written.
    if let Err(e) = pass.commit(&frozen, assembly, core) {
        return retryable(e);
    }
    // The parent's packet is streamed from the baked appearance after the commit, so
    // it is never held whole in memory beside it.
    if depth > 0 {
        child_bridge::persist_packet(
            pass.preencode,
            &preencode,
            &pass.captured.material_classes,
            check,
        )?;
    }
    drop(preencode);
    child_bridge::release_children(pass.preencode, &frozen, core, pass.check)?;
    pass.release(first, depth)?;
    reads.fetch_add(stats.child_tile_reads, std::sync::atomic::Ordering::Relaxed);
    pass.phases
        .baked_tile(first, depth, &stats, commit_started.elapsed().as_secs_f64())?;
    Ok(TileOutcome::Done)
}

fn validate_transforms(frozen: &handoff::FrozenNode) -> Result<(), JobError> {
    for gltf in &frozen.gltf {
        let scale = f32::from_bits(gltf.scale[0]);
        if !scale.is_finite()
            || scale <= 0.
            || gltf.scale != [gltf.scale[0]; 3]
            || gltf
                .translation
                .iter()
                .any(|&word| !f32::from_bits(word).is_finite())
        {
            return Err(invalid(
                "immutable H17 transform must be finite positive uniform",
            ));
        }
    }
    Ok(())
}

pub(super) fn node_record(frozen: &handoff::FrozenNode) -> Result<NodeRecord, JobError> {
    let id = &frozen.identity;
    if id.depth != u32::from(id.cell_depth) + u32::from(id.path_len) {
        return Err(invalid("handoff node depth/identity disagreement"));
    }
    let mut bounds = Bounds::EMPTY;
    for face in &frozen.faces {
        for corner in &face.corners {
            bounds.include(corner.decoded.map(f32::from_bits));
        }
    }
    Ok(NodeRecord {
        cell_depth: id.cell_depth,
        cell: id.cell,
        path_len: id.path_len,
        path: id.path,
        last_octant: id.last_octant,
        kind: NodeKind::Internal,
        children: u8::try_from(frozen.children.len())
            .map_err(|_| invalid("handoff child count"))?,
        first: id.first,
        end: id.end,
        bytes: frozen.glb_bytes,
        bounds,
    })
}

fn words(raw: [u32; 3], quantized: bool) -> Result<PositionWord, JobError> {
    if quantized {
        Ok(PositionWord::U16([
            u16::try_from(raw[0]).map_err(|_| invalid("frozen u16 POSITION range"))?,
            u16::try_from(raw[1]).map_err(|_| invalid("frozen u16 POSITION range"))?,
            u16::try_from(raw[2]).map_err(|_| invalid("frozen u16 POSITION range"))?,
        ]))
    } else {
        Ok(PositionWord::Float(raw))
    }
}

fn features(
    frozen: &handoff::FrozenNode,
    face: usize,
    values: &[(u32, Option<u32>)],
) -> Result<Vec<FeatureIdentity>, JobError> {
    let face = frozen
        .faces
        .get(face)
        .ok_or_else(|| invalid("decoded face index"))?;
    let gltf = frozen
        .gltf
        .get(face.gltf as usize)
        .ok_or_else(|| invalid("decoded glTF primitive index"))?;
    let declarations = gltf
        .feature_ids
        .as_array()
        .ok_or_else(|| invalid("frozen feature declaration array"))?;
    let mut result = Vec::with_capacity(values.len());
    let mut values = values.iter();
    for declaration in declarations {
        let Some(attribute) = declaration
            .get("attribute")
            .and_then(serde_json::Value::as_u64)
        else {
            continue;
        };
        let &(value, table) = values
            .next()
            .ok_or_else(|| invalid("decoded feature declaration/value closure"))?;
        result.push(FeatureIdentity {
            set: u32::try_from(attribute)
                .map_err(|_| invalid("frozen feature attribute number"))?,
            table,
            value: u64::from(value),
        });
    }
    if values.next().is_some() {
        return Err(invalid("undeclared frozen feature value"));
    }
    Ok(result)
}

/// The caller has leased the immutable owner packet, certified GLB and children
/// before invoking this conversion. Core packet_charge checks their combined
/// resident ownership again, including cloned dynamic features/ranges.
pub(super) fn frozen_node(
    frozen: &handoff::FrozenNode,
    original_glb: Vec<u8>,
    children: &[ChildView],
    memory_bytes: u64,
    check: &mut dyn FnMut() -> Result<(), JobError>,
) -> Result<FrozenNode, JobError> {
    frozen.validate()?;
    validate_transforms(frozen)?;
    if original_glb.len() as u64 != frozen.glb_bytes {
        return Err(invalid("certified proxy byte count"));
    }
    let mut charge = original_glb.capacity() as u64;
    for face in &frozen.faces {
        charge = charge
            .checked_add(std::mem::size_of::<FrozenFace>() as u64)
            .and_then(|n| {
                n.checked_add(
                    face.features.len() as u64 * std::mem::size_of::<FeatureIdentity>() as u64,
                )
            })
            .and_then(|n| {
                n.checked_add(
                    frozen.donor_ranges(face).len() as u64
                        * std::mem::size_of::<std::ops::Range<u64>>() as u64,
                )
            })
            .ok_or_else(|| resource("frozen bake adapter ownership"))?;
    }
    if charge > memory_bytes {
        return Err(resource("frozen bake adapter lease"));
    }
    let mut faces = Vec::with_capacity(frozen.faces.len());
    for (ordinal, face) in frozen.faces.iter().enumerate() {
        check()?;
        let gltf = &frozen.gltf[face.gltf as usize];
        let source = &frozen.primitives[face.source as usize];
        let fill_bound = face.fill.map(|fill| frozen.fills[fill as usize].bound);
        let mut position_words = [PositionWord::Float([0; 3]); 3];
        for (word, corner) in position_words.iter_mut().zip(&face.corners) {
            *word = words(corner.raw, gltf.quantized)?;
        }
        faces.push(FrozenFace {
            ordinal: ordinal as u64,
            source: SourceKey {
                node: source.node,
                mesh: source.mesh,
                primitive: source.primitive,
                instance: source.expanded_ordinal,
            },
            source_triangle: face.origin.triangle,
            material: source.material,
            positions: face.corners.map(|c| c.decoded.map(f32::from_bits)),
            words: position_words,
            transform: PositionTransform {
                translation: gltf.translation,
                scale: gltf.scale,
            },
            welds: face.corners.map(|c| c.weld),
            features: features(frozen, ordinal, &face.features)?,
            fill_bound,
            donor_ranges: frozen.donor_ranges(face).to_vec(),
        });
    }
    Ok(FrozenNode {
        node: node_record(frozen)?,
        original_glb,
        original_sha256: frozen.glb_sha256,
        geometric_error: frozen.geometric_error,
        child_texel_size: children
            .iter()
            .map(|child| child.texel_size)
            .fold(0., f64::max),
        faces,
    })
}

fn decoded_identity(
    frozen: &handoff::FrozenNode,
    face: &root::FaceIdentity,
) -> Result<FaceIdentity, JobError> {
    let original = frozen
        .faces
        .get(face.face as usize)
        .ok_or_else(|| invalid("assembled face identity index"))?;
    let gltf = &frozen.gltf[original.gltf as usize];
    let mut raw = [PositionWord::Float([0; 3]); 3];
    for (word, values) in raw.iter_mut().zip(face.raw) {
        *word = words(values, gltf.quantized)?;
    }
    Ok(FaceIdentity {
        ordinal: u64::from(face.face),
        source: SourceKey {
            node: face.source.0,
            mesh: face.source.1,
            primitive: face.source.2,
            instance: face.source.3,
        },
        source_triangle: face.origin.triangle,
        material: face.material,
        position_bits: face.decoded,
        words: raw,
        // The root independently verified actual assembled TRS against these
        // exact frozen bits before producing the private Verified token.
        transform: PositionTransform {
            translation: gltf.translation,
            scale: gltf.scale,
        },
        features: features(frozen, face.face as usize, &face.features)?,
    })
}

pub(super) struct AcceptedNode {
    pub assembly: root::Assembly,
    pub preencode: PreencodeChild,
    pub receipts: serde_json::Value,
    pub stats: BakeStats,
}
/// A real, owner-leased complete assembler. Its allowance includes translated
/// packet, native meshopt/codec, document, decoded readback and output ownership.
pub(super) type LeasedAssembler<'a> =
    dyn FnMut(&root::PreparedNode, u64) -> Result<root::Assembly, JobError> + 'a;

/// Exercise the unchanged core with the actual verified root Assembly. Only
/// the GLB Vec is moved through core validation, then reunited with the exact
/// original private Verified token. The caller can pass this to pass.commit.
pub(super) fn bake_one(
    frozen: &handoff::FrozenNode,
    original_glb: Vec<u8>,
    source: &mut CapturedSource<'_>,
    children: &[ChildView],
    limits: BakeLimits,
    assemble: &mut LeasedAssembler<'_>,
    check: &mut dyn FnMut() -> Result<(), JobError>,
) -> Result<AcceptedNode, JobError> {
    let core = frozen_node(frozen, original_glb, children, limits.memory_bytes, check)?;
    let checks = std::cell::RefCell::new(check);
    let mut accepted = None;
    let mut callback = |prepared: &PreparedNode| {
        let packet = assembler::prepare(frozen, prepared, &mut || (checks.borrow_mut())())?;
        (checks.borrow_mut())()?;
        match assemble(&packet.node, packet.writer_memory_bytes)? {
            root::Assembly::PhysicalBudget { actual_bytes } => {
                Ok(Assembly::PhysicalBudget { actual_bytes })
            }
            root::Assembly::Fit {
                glb,
                decoded,
                verified,
            } => {
                let identities = decoded
                    .iter()
                    .map(|face| decoded_identity(frozen, face))
                    .collect::<Result<Vec<_>, _>>()?;
                accepted = Some((decoded, verified));
                Ok(Assembly::Fit {
                    glb,
                    decoded: identities,
                })
            }
        }
    };
    let baked = bake_node(&core, source, children, limits, &mut callback, &mut || {
        (checks.borrow_mut())()
    })?;
    let (decoded, verified) =
        accepted.ok_or_else(|| invalid("bake core accepted without real verified assembly"))?;
    Ok(AcceptedNode {
        assembly: root::Assembly::Fit {
            glb: baked.glb,
            decoded,
            verified,
        },
        preencode: baked.preencode,
        receipts: baked.receipts,
        stats: baked.stats,
    })
}

#[cfg(test)]
pub(super) fn handoff_fixture() -> handoff::FrozenNode {
    use crate::mesh_large::lod::mesh::Origin;
    use serde_json::json;
    let primitives = (0..2)
        .map(|row| handoff::FrozenPrimitive {
            node: 10 + row,
            mesh: 20 + row,
            primitive: 30 + row,
            expanded_ordinal: 40 + u64::from(row),
            material: 5,
            row,
            error: 0.25,
            donor_ranges: std::iter::once(u64::from(row)..u64::from(row) + 1).collect(),
        })
        .collect();
    let faces = (0..2)
        .map(|source| {
            let positions: [[f32; 3]; 3] = if source == 0 {
                [[-0., 0., 0.], [1., 0., 0.], [0., 1., 0.]]
            } else {
                [[0.; 3]; 3]
            };
            handoff::FrozenFace {
                gltf: 0,
                source,
                corners: positions.map(|position| handoff::FrozenCorner {
                    raw: position.map(f32::to_bits),
                    decoded: position.map(|v| if v == 0. { 0 } else { v.to_bits() }),
                    weld: position.map(|v| if v == 0. { 0 } else { v.to_bits() }),
                    ell: 0,
                }),
                origin: Origin {
                    node: 10 + source,
                    mesh: 20 + source,
                    primitive: 30 + source,
                    triangle: u64::from(source) + 100,
                },
                fill: None,
                features: vec![(source, Some(0)), (99 + source, Some(3))],
            }
        })
        .collect();
    handoff::FrozenNode {
        identity: handoff::NodeIdentity {
            cell_depth: 0,
            cell: 0,
            path_len: 0,
            path: 0,
            last_octant: 0,
            depth: 0,
            first: 0,
            end: 2,
        },
        profile: "balanced".into(),
        geometric_error: 0.25,
        proxy_to_source: 0.25,
        source_to_proxy: 0.25,
        lattice_sha256: String::new(),
        glb_sha256: [0; 32],
        glb_bytes: 0,
        primitives,
        gltf: vec![handoff::FrozenGltfPrimitive {
            node: 0,
            mesh: 0,
            index: 0,
            sources: 0..2,
            material: Some(0),
            translation: [0; 3],
            scale: [1.0f32.to_bits(); 3],
            quantized: false,
            feature_ids: json!([
                {"attribute":7,"featureCount":2,"propertyTable":0,"label":"source_primitive"},
                {"attribute":3,"featureCount":101,"propertyTable":3,"label":"custom"}
            ]),
            faces: 0..2,
        }],
        faces,
        fills: Vec::new(),
        vanished: Vec::new(),
        children: Vec::new(),
        tables: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn depth_limits(memory_bytes: u64) -> BakeLimits {
        BakeLimits {
            profile: crate::mesh_large::config::Profile::Balanced,
            memory_bytes,
            member_bytes: 1,
            source_cache_bytes: 1,
            shared_image_bytes: 0,
            child_texel_bytes: 256 << 20,
            query_work: 1,
            max_page_edge: 4,
            max_density_drops: 0,
            id_textures: false,
        }
    }

    /// Per attempt: tile, share of memory it got, and whether it fitted.
    type Attempts = Vec<(u64, u64, bool)>;
    /// A fake tile bake whose tile `first` needs `need(first)` bytes; records the share
    /// each attempt got.
    fn run(
        tiles: &[u64],
        workers: usize,
        memory: u64,
        need: impl Fn(u64) -> u64 + Sync,
        fail: Option<u64>,
    ) -> (Result<(), JobError>, Attempts) {
        let log = std::sync::Mutex::new(Vec::new());
        let result = run_depth(
            tiles,
            workers,
            depth_limits(memory),
            &|| Ok(()),
            &|_| {},
            |_, _| Ok(()),
            |_, share, first, check| {
                check()?;
                if fail == Some(first) {
                    return Err(invalid("fake tile failure"));
                }
                let fits = need(first) <= share.memory_bytes;
                log.lock().unwrap().push((first, share.memory_bytes, fits));
                Ok(if fits {
                    TileOutcome::Done
                } else {
                    TileOutcome::Retry(resource("fake tile needs more memory"))
                })
            },
        )
        .map(|retried| {
            // Distinct tiles that refused a share (a tile may refuse more than one).
            let refused: std::collections::BTreeSet<u64> = log
                .lock()
                .unwrap()
                .iter()
                .filter(|e| !e.2)
                .map(|e| e.0)
                .collect();
            assert_eq!(retried, refused.len() as u64);
        });
        let mut log = log.into_inner().unwrap();
        log.sort_unstable();
        (result, log)
    }

    #[test]
    fn depth_admission_does_not_depend_on_worker_count() {
        let tiles: Vec<u64> = (0..8).collect();
        // Tiles 3 and 6 need more than an eighth of the allowance but fit all of it.
        let need = |first: u64| {
            if first.is_multiple_of(3) && first > 0 {
                500
            } else {
                100
            }
        };
        for workers in [1, 2, 4, 8, 16] {
            let (result, log) = run(&tiles, workers, 1000, need, None);
            result.unwrap();
            let done: Vec<u64> = log.iter().filter(|e| e.2).map(|e| e.0).collect();
            assert_eq!(done, tiles, "workers {workers}");
            // A refused tile is retried with a larger share and then fits.
            for e in log.iter().filter(|e| !e.2) {
                assert!(
                    log.iter().any(|f| f.0 == e.0 && f.1 > e.1 && f.2),
                    "workers {workers}: {e:?}"
                );
            }
        }
        // With eight workers each share is 125 bytes: tiles 3 and 6 are refused, then
        // both bake together on two workers with 500 bytes each.
        let (_, log) = run(&tiles, 8, 1000, need, None);
        assert!(log.contains(&(3, 125, false)) && log.contains(&(6, 125, false)));
        assert!(log.contains(&(3, 500, true)) && log.contains(&(6, 500, true)));
        // A tile that fits only the whole allowance halves its way down to it alone.
        let (result, log) = run(&tiles, 8, 1000, |f| if f == 3 { 900 } else { 100 }, None);
        result.unwrap();
        assert!(log.contains(&(3, 125, false)) && log.contains(&(3, 1000, true)));
    }

    #[test]
    fn throttled_cancellation_check_is_bounded_by_calls_and_by_time() {
        let mut throttle = CheckThrottle::new();
        let due = (0..200).filter(|_| throttle.due()).count();
        // Calls 1, 65, 129 and 193; this loop is far shorter than CHECK_AFTER.
        assert!(due >= 4, "{due} consultations");
        assert!(due <= 4 + 1, "{due} consultations");
        // A single call after a long pause consults at once.
        std::thread::sleep(CHECK_AFTER * 2);
        assert!(throttle.due());
        assert!(!throttle.due());
    }

    #[test]
    fn a_coarse_step_sees_cancellation_once_the_time_bound_has_passed() {
        let cancelled = std::sync::atomic::AtomicBool::new(false);
        let calls_after = std::sync::atomic::AtomicU32::new(0);
        let result = run_depth(
            &[1],
            1,
            depth_limits(1000),
            &|| {
                if cancelled.load(std::sync::atomic::Ordering::SeqCst) {
                    Err(JobError::new(crate::JobErrorKind::Cancelled, "stop"))
                } else {
                    Ok(())
                }
            },
            &|_| {},
            |_, _| Ok(()),
            |_, _, _, check| {
                // The first call consults, so the run is not yet cancelled.
                check()?;
                cancelled.store(true, std::sync::atomic::Ordering::SeqCst);
                // Hot-path calls in a burst are batched and miss the cancellation.
                for _ in 0..10 {
                    check()?;
                }
                // A coarse step that took longer than the bound does not wait for 64
                // calls: the very next check sees it.
                std::thread::sleep(CHECK_AFTER * 2);
                calls_after.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                check()?;
                calls_after.fetch_add(100, std::sync::atomic::Ordering::SeqCst);
                Ok(TileOutcome::Done)
            },
        );
        let e = result.unwrap_err();
        assert_eq!(e.kind(), crate::JobErrorKind::Cancelled);
        assert_eq!(calls_after.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[test]
    fn worker_state_and_tiles_get_a_source_cache_within_their_share() {
        let mut limits = depth_limits(4000);
        limits.source_cache_bytes = 3000;
        let seen = std::sync::Mutex::new(Vec::new());
        run_depth(
            &[1, 2, 3, 4],
            4,
            limits,
            &|| Ok(()),
            &|_| {},
            |share, _| {
                seen.lock()
                    .unwrap()
                    .push(("state", share.memory_bytes, share.source_cache_bytes));
                Ok(())
            },
            |_, share, _, _| {
                seen.lock()
                    .unwrap()
                    .push(("tile", share.memory_bytes, share.source_cache_bytes));
                Ok(TileOutcome::Done)
            },
        )
        .unwrap();
        let seen = seen.into_inner().unwrap();
        // One state per worker that took a tile (scheduling decides how many).
        assert_eq!(seen.iter().filter(|e| e.0 == "tile").count(), 4);
        assert!((1..=4).contains(&seen.iter().filter(|e| e.0 == "state").count()));
        assert!(seen.iter().all(|e| e.1 == 1000 && e.2 == 250));
    }

    #[test]
    fn depth_refuses_a_tile_too_large_for_the_whole_allowance_and_reports_real_failures() {
        let tiles: Vec<u64> = (0..6).collect();
        for workers in [1, 4] {
            let (result, _) = run(
                &tiles,
                workers,
                1000,
                |f| if f == 2 { 2000 } else { 1 },
                None,
            );
            let e = result.unwrap_err();
            assert_eq!(e.kind(), crate::JobErrorKind::ResourceLimit);
            assert_eq!(e.message(), "fake tile needs more memory");
            // A non-resource failure is reported as itself, not as a sibling cancellation.
            let (result, _) = run(&tiles, workers, 1000, |_| 1, Some(4));
            assert_eq!(result.unwrap_err().message(), "fake tile failure");
        }
    }
    #[test]
    fn handoff_maps_each_source_with_shared_material_gltf_and_original_attribute_numbers() {
        let frozen = handoff_fixture();
        let mapped = frozen_node(&frozen, Vec::new(), &[], 1 << 20, &mut || Ok(())).unwrap();
        assert_eq!(mapped.faces[0].source.instance, 40);
        assert_eq!(mapped.faces[1].source.instance, 41);
        assert_eq!(mapped.faces[1].source.node, 11);
        assert_eq!(mapped.faces[1].source_triangle, 101);
        assert_eq!(mapped.faces[1].donor_ranges, vec![1..2]);
        assert_eq!(mapped.faces[1].features[0].set, 7);
        assert_eq!(mapped.faces[1].features[1].set, 3);
        assert_eq!(mapped.faces[1].features[1].table, Some(3));
        assert_eq!(
            mapped.faces[0].words[0],
            PositionWord::Float([(-0.0f32).to_bits(), 0, 0])
        );
        assert_eq!(mapped.faces[0].positions[0][0].to_bits(), 0);
    }
    #[test]
    fn handoff_transform_rejects_nonuniform_reflected_and_nonfinite_before_bake() {
        for scale in [[1., 2., 1.], [-1.; 3], [f32::INFINITY; 3]] {
            let mut frozen = handoff_fixture();
            frozen.gltf[0].scale = scale.map(f32::to_bits);
            assert_eq!(
                frozen_node(&frozen, Vec::new(), &[], 1 << 20, &mut || Ok(()))
                    .err()
                    .unwrap()
                    .kind(),
                crate::JobErrorKind::InvalidState
            );
        }
        let frozen = handoff_fixture();
        assert_eq!(
            frozen_node(&frozen, Vec::new(), &[], 1, &mut || Ok(()))
                .err()
                .unwrap()
                .kind(),
            crate::JobErrorKind::ResourceLimit
        );
    }
}
