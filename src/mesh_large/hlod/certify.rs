//! Publication in the captured local metre frame. A complete cover in each
//! direction bounds the Hausdorff distance between two closed triangle supports.
use super::{resource, state, store::TileKey, TileGeometry};
use crate::{mesh_large::phases::STEP_HEARTBEAT, JobError, RunEvent};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::{
    ffi::{c_int, c_void},
    panic::{catch_unwind, AssertUnwindSafe},
    sync::Mutex,
    time::Instant,
};

type Face = [[f32; 3]; 3];
pub(crate) const POLICY: &str = "hlod-certified-decoded-surface-v1";
const DIRECT_LIMIT: u64 = 131_072;

/// Both face arrays and both immutable directional indexes can coexist.
/// The native layout is checked at compile time. Eight MiB covers traversal
/// stacks, allocator rounding, the callback and fixed job state.
pub(super) fn workspace(max_children: u64, proxy_cap: usize, source_faces: u64) -> u64 {
    let reference_faces = if source_faces <= DIRECT_LIMIT {
        source_faces
    } else {
        DIRECT_LIMIT.max(max_children)
    };
    reference_faces
        .saturating_add(proxy_cap as u64)
        .saturating_mul(512)
        .saturating_add(8 << 20)
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum Reference {
    Source,
    Children,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct Certificate {
    reference: Reference,
    reference_triangles: u64,
    /// Directed prices of complete covers, before a possible child addition.
    to_proxy: f64,
    from_proxy: f64,
    inherited: f64,
}

impl Certificate {
    pub(super) fn bound(self) -> f64 {
        let local = self.to_proxy.max(self.from_proxy);
        match self.reference {
            Reference::Source => local,
            Reference::Children => (local + self.inherited).next_up(),
        }
    }

    pub(crate) fn check(
        self,
        source_triangles: u64,
        merged: u64,
        published: f64,
    ) -> Result<(), JobError> {
        let reference_ok = match self.reference {
            Reference::Source => {
                source_triangles <= DIRECT_LIMIT
                    && self.reference_triangles == source_triangles
                    && self.inherited == 0.
            }
            Reference::Children => {
                // Construction can discard repeated-corner faces after welding. The
                // certificate retains those closed supports in its complete frontier.
                source_triangles > DIRECT_LIMIT
                    && self.reference_triangles >= merged
                    && self.reference_triangles
                        <= 8 * crate::mesh_large::config::CONSUMER_TABLE_ROWS
            }
        };
        if !reference_ok
            || self.reference_triangles == 0
            || [self.to_proxy, self.from_proxy, self.inherited, published]
                .iter()
                .any(|v| !v.is_finite() || *v < 0.)
            || published < self.bound()
        {
            return Err(state("invalid or understated decoded-surface certificate"));
        }
        Ok(())
    }
}

/// Kept while construction's required S2 pass streams the original ranks.
/// The rank map also turns existing S2 nearest hits into candidate face IDs.
/// Candidates only steer the search. The native kernel prices actual witnesses.
pub(super) struct SourceRows {
    first: u64,
    faces: Vec<Face>,
    ordinal: Vec<u32>,
}

impl SourceRows {
    pub(super) fn new(key: TileKey, original: bool) -> Result<Option<Self>, JobError> {
        let count = key
            .end
            .checked_sub(key.first)
            .filter(|&n| n > 0)
            .ok_or_else(|| state("empty certificate source range"))?;
        if !original || count > DIRECT_LIMIT {
            return Ok(None);
        }
        let count =
            usize::try_from(count).map_err(|_| resource("certificate rank count overflows"))?;
        Ok(Some(Self {
            first: key.first,
            faces: face_buffer(count)?,
            ordinal: vec![u32::MAX; count],
        }))
    }

    pub(super) fn seeds(&self) -> Vec<u32> {
        vec![u32::MAX; self.ordinal.len()]
    }

    pub(super) fn record(&mut self, t: &super::source_index::Triangle) -> Result<(), JobError> {
        let index = t
            .rank
            .checked_sub(self.first)
            .and_then(|k| usize::try_from(k).ok())
            .and_then(|i| self.ordinal.get_mut(i))
            .ok_or_else(|| state("certificate rank outside source range"))?;
        if *index != u32::MAX {
            return Err(state("duplicate certificate source rank"));
        }
        *index = face_count(&self.faces)?;
        self.faces.push(t.bits.map(|p| p.map(f32::from_bits)));
        Ok(())
    }

    pub(super) fn finish(
        self,
        by_rank: Vec<u32>,
        centroids: &[super::error::Centroid],
    ) -> Result<Comparison, JobError> {
        if self.faces.len() != self.ordinal.len() || by_rank.len() != self.ordinal.len() {
            return Err(state("incomplete certificate source range"));
        }
        let mut forward = vec![u32::MAX; by_rank.len()];
        for (&ordinal, hint) in self.ordinal.iter().zip(by_rank) {
            forward[ordinal as usize] = hint;
        }
        let backward = centroids
            .iter()
            .map(|h| {
                h.rank
                    .checked_sub(self.first)
                    .and_then(|r| usize::try_from(r).ok())
                    .and_then(|r| self.ordinal.get(r))
                    .copied()
                    .unwrap_or(u32::MAX)
            })
            .collect();
        Ok(Comparison {
            reference: Reference::Source,
            faces: self.faces,
            inherited: 0.,
            seeds: Seeds { forward, backward },
        })
    }
}

#[derive(Default)]
struct Seeds {
    forward: Vec<u32>,
    backward: Vec<u32>,
}

/// Reference geometry prepared while the children are still resident. Promotion
/// and recovery may skip tree levels, but never a rank or a leaf of this tile.
pub(super) struct Comparison {
    seeds: Seeds,
    reference: Reference,
    faces: Vec<Face>,
    inherited: f64,
}

impl Comparison {
    pub(super) fn new(
        key: TileKey,
        captured: &crate::mesh_large::full::FullCaptured,
        retained: Option<Self>,
        children: &[TileGeometry],
        check: &dyn Fn() -> Result<(), JobError>,
    ) -> Result<Self, JobError> {
        let n = key
            .end
            .checked_sub(key.first)
            .filter(|&n| n > 0)
            .ok_or_else(|| state("empty certificate source range"))?;
        if let Some(comparison) = retained {
            if captured.original_positions.is_some()
                || n > DIRECT_LIMIT
                || comparison.faces.len() as u64 != n
            {
                return Err(state(
                    "retained certificate range differs from the original source",
                ));
            }
            return Ok(comparison);
        }
        if n <= DIRECT_LIMIT {
            let original = captured
                .original_positions
                .as_ref()
                .ok_or_else(|| state("missing original certificate source"))?;
            let count =
                usize::try_from(n).map_err(|_| resource("certificate rank count overflows"))?;
            let mut faces = face_buffer(count)?;
            for (i, t) in captured
                .triangles
                .range(key.first, key.end, 65_536)?
                .enumerate()
            {
                if i.is_multiple_of(256) {
                    check()?;
                }
                faces.push(original.face(&t?)?);
            }
            if faces.len() as u64 != n {
                return Err(state("incomplete pre-weld certificate source"));
            }
            return Ok(Self {
                reference: Reference::Source,
                faces,
                inherited: 0.,
                seeds: Seeds::default(),
            });
        }
        Self::frontier(key, children, check)
    }

    fn frontier(
        key: TileKey,
        children: &[TileGeometry],
        check: &dyn Fn() -> Result<(), JobError>,
    ) -> Result<Self, JobError> {
        let mut next = (key.first, key.leaf_begin);
        let mut inherited = 0.0_f64;
        let merged = children
            .iter()
            .try_fold(0usize, |sum, c| sum.checked_add(c.triangles.len()))
            .ok_or_else(|| resource("certificate frontier size overflows"))?;
        let mut faces = face_buffer(merged)?;
        for child in children {
            let k = child.key;
            if (k.first, k.leaf_begin) != next
                || k.end <= k.first
                || k.leaf_end <= k.leaf_begin
                || child.triangles.is_empty()
                || !child.surface_error.is_finite()
                || child.surface_error < 0.
            {
                return Err(state("certificate children do not partition the parent"));
            }
            next = (k.end, k.leaf_end);
            inherited = inherited.max(child.surface_error);
            let positions = child.delivered_positions();
            for (i, t) in child.triangles.iter().enumerate() {
                if i.is_multiple_of(256) {
                    check()?;
                }
                faces.push(t.map(|v| positions[v as usize]));
            }
        }
        if next != (key.end, key.leaf_end) || faces.is_empty() {
            return Err(state("incomplete certificate child frontier"));
        }
        Ok(Self {
            reference: Reference::Children,
            seeds: Seeds::default(),
            faces,
            inherited,
        })
    }

    pub(super) fn certify(
        self,
        decoded: &[[f32; 3]],
        indices: &[[u32; 3]],
        construction: f64,
        tick: &Tick<'_>,
    ) -> Result<(Certificate, f64), JobError> {
        let start = super::error::thread_cpu();
        let mut target = face_buffer(indices.len())?;
        for t in indices {
            target.push(t.map(|v| decoded[v as usize]));
        }
        let guide = (construction - self.inherited).max(construction * 0.25);
        let preparation = super::error::elapsed(start);
        let ([to_proxy, from_proxy], cpu) = cover(&self.faces, &target, &self.seeds, guide, tick)?;
        let cert = Certificate {
            reference: self.reference,
            reference_triangles: self.faces.len() as u64,
            to_proxy,
            from_proxy,
            inherited: self.inherited,
        };
        if !cert.bound().is_finite() {
            return Err(state("nonfinite accumulated surface certificate"));
        }
        Ok((cert, preparation + cpu))
    }
}

fn face_buffer(count: usize) -> Result<Vec<Face>, JobError> {
    let mut faces = Vec::new();
    faces
        .try_reserve_exact(count)
        .map_err(|_| resource("could not allocate certificate faces"))?;
    Ok(faces)
}

type Tick<'a> = dyn Fn(u32) -> Result<(), JobError> + Sync + 'a;
type Poll = extern "C" fn(*mut c_void, u32) -> c_int;

// SAFETY: The definitions use contiguous binary32 triples and fixed-width
// counts. Calls are synchronous. A successful index construction transfers
// one immutable native allocation, released exactly once by NativeIndex.
unsafe extern "C" {
    fn rt_cert_index(
        faces: *const Face,
        count: u32,
        out: *mut *mut c_void,
        poll: Poll,
        context: *mut c_void,
    ) -> c_int;
    fn rt_cert_free(index: *mut c_void);
    fn rt_cert_range(
        index: *const c_void,
        faces: *const Face,
        hints: *const u32,
        count: u32,
        credits: u64,
        guide: f64,
        upper: *mut f64,
        poll: Poll,
        context: *mut c_void,
    ) -> c_int;
}

struct Callback<'a> {
    tick: &'a Tick<'a>,
    error: Option<JobError>,
}

extern "C" fn poll(context: *mut c_void, done: u32) -> c_int {
    // SAFETY: Each synchronous call has an exclusive local Callback. The shared
    // observer implements Sync and native code never retains the context.
    let callback = unsafe { &mut *context.cast::<Callback<'_>>() };
    callback.error = match catch_unwind(AssertUnwindSafe(|| (callback.tick)(done))) {
        Ok(r) => r.err(),
        Err(_) => Some(JobError::new(
            crate::JobErrorKind::ObserverFailure,
            "surface certificate observer panicked",
        )),
    };
    i32::from(callback.error.is_some())
}

fn checked_call(tick: &Tick<'_>, call: impl FnOnce(*mut c_void) -> c_int) -> Result<(), JobError> {
    tick(0)?;
    let mut callback = Callback { tick, error: None };
    let status = call((&raw mut callback).cast());
    if let Some(error) = callback.error {
        return Err(error);
    }
    match status {
        0 => Ok(()),
        2 => Err(resource("could not allocate surface certificate workspace")),
        _ => Err(state("could not certify both finite decoded surfaces")),
    }
}

struct NativeIndex(std::ptr::NonNull<c_void>);
// SAFETY: After construction, the index holds only immutable face, BVH and edge
// tables. Each range has its own subdivision stack, visit stamps and callback.
unsafe impl Sync for NativeIndex {}

impl Drop for NativeIndex {
    fn drop(&mut self) {
        // SAFETY: This owner holds one successful construction. Scoped range
        // borrows, including any stolen by the operation pool, have ended.
        unsafe { rt_cert_free(self.0.as_ptr()) }
    }
}

fn face_count(faces: &[Face]) -> Result<u32, JobError> {
    u32::try_from(faces.len()).map_err(|_| resource("surface certificate face count overflows"))
}

impl NativeIndex {
    fn new(target: &[Face], tick: &Tick<'_>) -> Result<Self, JobError> {
        let count = face_count(target)?;
        let mut ptr = std::ptr::null_mut();
        checked_call(tick, |context| {
            // SAFETY: The nested arrays are contiguous. The kernel copies their
            // coordinates into its index before returning and initializes ptr on success.
            unsafe { rt_cert_index(target.as_ptr(), count, &raw mut ptr, poll, context) }
        })?;
        std::ptr::NonNull::new(ptr)
            .map(Self)
            .ok_or_else(|| state("missing certificate index"))
    }

    fn range(
        &self,
        faces: &[Face],
        hints: Option<&[u32]>,
        credits: u64,
        guide: f64,
        tick: &Tick<'_>,
    ) -> Result<(f64, f64), JobError> {
        let count = face_count(faces)?;
        let mut upper = 0.;
        let start = super::error::thread_cpu();
        checked_call(tick, |context| {
            // SAFETY: All inputs live for this call. The index is read-only and
            // the exclusive output and callback belong to the current range.
            unsafe {
                rt_cert_range(
                    self.0.as_ptr(),
                    faces.as_ptr(),
                    hints.map_or(std::ptr::null(), <[u32]>::as_ptr),
                    count,
                    credits,
                    guide,
                    &raw mut upper,
                    poll,
                    context,
                )
            }
        })?;
        Ok((upper, super::error::elapsed(start)))
    }
}

fn direction(
    domain: &[Face],
    target: &[Face],
    hints: &[u32],
    guide: f64,
    tick: &Tick<'_>,
) -> Result<(f64, f64), JobError> {
    let start = super::error::thread_cpu();
    if !hints.is_empty() && hints.len() != domain.len() {
        return Err(state("certificate hint count differs from its domain"));
    }
    let index = NativeIndex::new(target, tick)?;
    let build_cpu = super::error::elapsed(start);
    // The same contiguous ranges and split budgets are used at every job count.
    // The operation pool shares work within a large direction when a tile
    // frontier narrows. A direct caller has the same sequential partition.
    let parts = domain.len().div_ceil(8192).clamp(1, 4);
    let total_credits =
        65_536_u64.max(256 * u64::from(face_count(domain)?.max(face_count(target)?)));
    let run = |part: usize| {
        let first = part * domain.len() / parts;
        let end = (part + 1) * domain.len() / parts;
        let split = |i: usize| total_credits * i as u64 / parts as u64;
        index.range(
            &domain[first..end],
            hints.get(first..end),
            split(part + 1) - split(part),
            guide,
            tick,
        )
    };
    let results: Vec<_> = if parts > 1 && rayon::current_thread_index().is_some() {
        (0..parts).into_par_iter().map(run).collect()
    } else {
        (0..parts).map(run).collect()
    };
    let mut price = 0.0_f64;
    let mut cpu = build_cpu;
    for result in results {
        let (bound, used) = result?;
        price = price.max(bound);
        cpu += used;
    }
    Ok((price, cpu))
}

fn cover(
    a: &[Face],
    b: &[Face],
    seeds: &Seeds,
    guide: f64,
    tick: &Tick<'_>,
) -> Result<([f64; 2], f64), JobError> {
    let forward = || direction(a, b, &seeds.forward, guide, tick);
    let backward = || direction(b, a, &seeds.backward, guide, tick);
    let (a, b) = if rayon::current_thread_index().is_some() {
        rayon::join(forward, backward)
    } else {
        (forward(), backward())
    };
    let (a, ac) = a?;
    let (b, bc) = b?;
    Ok(([a, b], ac + bc))
}

/// All workers advance one monotone count of completed domain faces, with a
/// heartbeat for observers even during a long BVH build or a difficult face.
pub(super) struct Progress<'a> {
    check: &'a (dyn Fn() -> Result<(), JobError> + Sync),
    emit: Option<&'a crate::mesh_large::phases::Emit<'a>>,
    state: Mutex<(u64, Option<Instant>)>,
}
impl<'a> Progress<'a> {
    pub(super) fn new(
        check: &'a (dyn Fn() -> Result<(), JobError> + Sync),
        emit: Option<&'a crate::mesh_large::phases::Emit<'a>>,
    ) -> Self {
        Self {
            check,
            emit,
            state: Mutex::new((0, None)),
        }
    }
    pub(super) fn tick(&self, faces: u32) -> Result<(), JobError> {
        (self.check)()?;
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.0 = state
            .0
            .checked_add(u64::from(faces))
            .ok_or_else(|| resource("certificate progress overflows"))?;
        if state.1.is_none_or(|last| last.elapsed() >= STEP_HEARTBEAT) {
            state.1 = Some(Instant::now());
            self.send(state.0, None)?;
        }
        Ok(())
    }
    pub(super) fn finish(&self) -> Result<(), JobError> {
        let (done, last) = *self.state.lock().unwrap_or_else(|e| e.into_inner());
        if last.is_some() {
            self.send(done, Some(done))
        } else {
            Ok(())
        }
    }
    fn send(&self, done: u64, total: Option<u64>) -> Result<(), JobError> {
        self.emit.map_or(Ok(()), |emit| {
            emit(&RunEvent::Progress {
                phase: "hlod_certify",
                done,
                total,
            })
        })
    }
}

#[cfg(test)]
pub(super) fn direct_control(source: &[Face], proxy: &[Face], construction: f64) -> Certificate {
    let comparison = Comparison {
        reference: Reference::Source,
        seeds: Seeds::default(),
        faces: source.to_vec(),
        inherited: 0.,
    };
    let positions: Vec<_> = proxy.iter().flatten().copied().collect();
    let indices: Vec<_> = (0..proxy.len() as u32)
        .map(|i| [3 * i, 3 * i + 1, 3 * i + 2])
        .collect();
    comparison
        .certify(&positions, &indices, construction, &|_| Ok(()))
        .unwrap()
        .0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::JobErrorKind;
    #[test]
    fn adjacent_binary32_levels_remain_enclosed_at_small_and_large_coordinates() {
        for low in [f32::from_bits(1), 999_999.0, -999_999.0] {
            let high = low.next_up();
            let a = [[0., 0., low]; 3];
            let b = [[0., 0., high]; 3];
            // Adjacent binary32 differences are exactly representable in binary64.
            let expected = f64::from(high) - f64::from(low);
            let cert = direct_control(&[a], &[b], 0.);
            assert!(cert.to_proxy >= expected && cert.from_proxy >= expected);
            assert!(cert.bound() < expected * 1.00001);
        }
    }

    #[test]
    fn displacements_of_points_segments_and_parallel_layers_are_covered() {
        let point = [[0., 0., 0.]; 3];
        let segment = [[0., 0., 0.], [3., 0., 0.], [0., 0., 0.]];
        let triangle = [[0., 0., 0.], [3., 0., 0.], [0., 1., 0.]];
        for face in [point, segment, triangle] {
            let above = face.map(|[x, y, z]| [x, y, z + 0.5]);
            let direct = direct_control(&[face], &[above], 0.);
            for distance in [direct.to_proxy, direct.from_proxy] {
                assert!((0.5..0.500001).contains(&distance));
            }
        }
    }

    #[test]
    fn opposite_diagonals_cover_a_flat_sheet_and_its_parallel_offset() {
        let a = [0., 0., 0.];
        let b = [4., 0., 0.];
        let c = [4., 4., 0.];
        let d = [0., 4., 0.];
        let original = [[a, b, c], [a, c, d]];
        let alternative = [[a, b, d], [b, c, d]];
        for dz in [0., 0.125] {
            let delivered = alternative.map(|face| face.map(|[x, y, z]| [x, y, z + dz]));
            let result = direct_control(&original, &delivered, f64::from(dz) + 1e-6);
            for upper in [result.to_proxy, result.from_proxy] {
                assert!(
                    (f64::from(dz)..f64::from(dz) + 1e-6).contains(&upper),
                    "{upper}"
                );
            }
        }
    }

    #[test]
    fn interior_hole_and_folded_neighbor_leave_a_positive_directed_error() {
        let a = [0., 0., 0.];
        let b = [6., 0., 0.];
        let c = [0., 6., 0.];
        let p = [1., 1., 0.];
        let q = [4., 1., 0.];
        let r = [1., 4., 0.];
        let domain = [[a, b, c]];
        // An interior point (2,2) is 1/sqrt(2) from the inner hypotenuse.
        // The second copy of an incident face folds back onto the same side.
        let ring = [
            [a, b, q],
            [a, q, p],
            [b, c, r],
            [b, r, q],
            [c, a, p],
            [c, p, r],
            [q, p, a],
        ];
        let expected = std::f64::consts::FRAC_1_SQRT_2;
        let result = direct_control(&domain, &ring, 0.001);
        assert!(result.to_proxy >= expected);
        assert!(result.from_proxy < 0.001);
        let swapped = direct_control(&ring, &domain, 0.001);
        assert!(swapped.from_proxy >= expected);
        assert!(swapped.to_proxy < 0.001);
    }

    #[test]
    fn incomplete_or_repeated_source_ranks_cannot_be_retained() {
        let key = TileKey {
            first: 7,
            end: 9,
            depth: 1,
            leaf_begin: 0,
            leaf_end: 1,
        };
        let point = [[0.; 3]; 3];
        let mut rows = SourceRows::new(key, true).unwrap().unwrap();
        let t = super::super::source_index::Triangle::new(point, 7);
        rows.record(&t).unwrap();
        assert!(rows.record(&t).is_err());
        assert!(rows.finish(vec![u32::MAX; 2], &[]).is_err());
        assert!(SourceRows::new(key, false).unwrap().is_none());
    }

    #[test]
    fn progress_interrupts_inside_native_work_and_preserves_the_error_kind() {
        let face = [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]];
        let geometry = vec![face; 2048];
        use std::sync::atomic::{AtomicUsize, Ordering};
        let calls = AtomicUsize::new(0);
        let stopped = cover(&geometry, &geometry, &Seeds::default(), 0., &|_| {
            if calls.fetch_add(1, Ordering::Relaxed) >= 3 {
                Err(JobError::new(JobErrorKind::Cancelled, "cancel certificate"))
            } else {
                Ok(())
            }
        })
        .unwrap_err();
        assert_eq!(stopped.kind(), JobErrorKind::Cancelled);
        assert!(calls.load(Ordering::Relaxed) >= 4);
    }

    #[test]
    fn frontier_uses_all_delivered_children_and_adds_their_largest_bound() {
        let key = TileKey {
            first: 0,
            end: DIRECT_LIMIT + 1,
            depth: 0,
            leaf_begin: 0,
            leaf_end: 2,
        };
        let original = [[0., 0., 0.], [2., 0., 0.], [0., 2., 0.]];
        let shifted = |z| original.map(|[x, y, _]| [x, y, z]);
        let child = |first, end, leaf, err| TileGeometry {
            key: TileKey {
                first,
                end,
                depth: 1,
                leaf_begin: leaf,
                leaf_end: leaf + 1,
            },
            ells: vec![],
            positions: original.to_vec(),
            decoded_override: shifted(1.).to_vec(),
            triangles: vec![[0, 1, 2]],
            representative: vec![],
            primitives: vec![],
            interface: vec![],
            geometric_error: 0.,
            surface_error: err,
            published_error: 10.,
            texture: Default::default(),
            lease: None,
        };
        let children = [
            child(0, DIRECT_LIMIT, 0, 1.),
            child(DIRECT_LIMIT, DIRECT_LIMIT + 1, 1, 0.5),
        ];
        let comparison = Comparison::frontier(key, &children, &|| Ok(())).unwrap();
        let cert = comparison
            .certify(&shifted(2.), &[[0, 1, 2]], 0., &|_| Ok(()))
            .unwrap()
            .0;
        assert_eq!(cert.inherited, 1.);
        assert!((1.0..1.000001).contains(&cert.to_proxy));
        assert!((2.0..2.000001).contains(&cert.bound()));
        assert!(Comparison::frontier(key, &children[..1], &|| Ok(())).is_err());
        assert!(Comparison::frontier(key, &children[1..], &|| Ok(())).is_err());
    }

    #[test]
    fn staged_certificate_refuses_a_rounded_down_sum_or_wrong_reference() {
        let cert = Certificate {
            reference: Reference::Children,
            reference_triangles: 2,
            to_proxy: 0.5,
            from_proxy: 0.25,
            inherited: 1.,
        };
        assert!(cert.check(DIRECT_LIMIT + 1, 2, 1.5).is_err());
        assert!(cert.check(DIRECT_LIMIT + 1, 2, cert.bound()).is_ok());
        assert!(cert.check(DIRECT_LIMIT, 2, cert.bound()).is_err());
        assert!(cert.check(DIRECT_LIMIT + 1, 3, cert.bound()).is_err());
        assert!(Certificate {
            inherited: f64::NAN,
            ..cert
        }
        .check(DIRECT_LIMIT + 1, 2, 2.)
        .is_err());
    }
}
