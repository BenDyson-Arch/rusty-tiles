//! Publication in the captured local metre frame. A complete cover in each
//! direction bounds the Hausdorff distance between two closed triangle supports.
use super::{resource, source_index::SourceQuery, state, store::TileKey, TileGeometry};
use crate::{mesh_large::phases::STEP_HEARTBEAT, JobError, RunEvent};
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

/// Both face arrays and the larger of the two sequential directional BVHs.
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

/// Reference geometry prepared while the children are still resident. Promotion
/// and recovery may skip tree levels, but never a rank or a leaf of this tile.
pub(super) struct Comparison {
    reference: Reference,
    faces: Vec<Face>,
    inherited: f64,
}

impl Comparison {
    pub(super) fn new(
        key: TileKey,
        captured: &crate::mesh_large::full::FullCaptured,
        query: &SourceQuery<'_>,
        children: &[TileGeometry],
        check: &dyn Fn() -> Result<(), JobError>,
    ) -> Result<Self, JobError> {
        let n = key
            .end
            .checked_sub(key.first)
            .filter(|&n| n > 0)
            .ok_or_else(|| state("empty certificate source range"))?;
        if n <= DIRECT_LIMIT {
            let len =
                usize::try_from(n).map_err(|_| resource("certificate source range too large"))?;
            let mut faces = face_buffer(len)?;
            if let Some(original) = &captured.original_positions {
                // This sequence is in tree rank order, before any degenerate
                // faces are discarded for simplification. Attribute/donor queries
                // deliberately keep using their welded geometry.
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
            } else {
                let mut seen = vec![0u64; len.div_ceil(64)];
                for (i, t) in query.surface()?.enumerate() {
                    if i.is_multiple_of(256) {
                        check()?;
                    }
                    if faces.len() == len {
                        return Err(state("excess certificate source ranks"));
                    }
                    let t = t?;
                    let rank = t
                        .rank
                        .checked_sub(key.first)
                        .filter(|&r| r < n)
                        .ok_or_else(|| state("certificate source rank outside tile"))?;
                    let rank =
                        usize::try_from(rank).map_err(|_| resource("certificate rank overflow"))?;
                    let bit = 1u64 << (rank % 64);
                    if seen[rank / 64] & bit != 0 {
                        return Err(state("duplicate certificate source rank"));
                    }
                    seen[rank / 64] |= bit;
                    faces.push(t.bits.map(|p| p.map(f32::from_bits)));
                }
            }
            if faces.len() != len {
                return Err(state("incomplete certificate source range"));
            }
            return Ok(Self {
                reference: Reference::Source,
                faces,
                inherited: 0.,
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
            faces,
            inherited,
        })
    }

    pub(super) fn certify(
        self,
        decoded: &[[f32; 3]],
        indices: &[[u32; 3]],
        construction: f64,
        tick: &dyn Fn(u32) -> Result<(), JobError>,
    ) -> Result<Certificate, JobError> {
        let mut target = face_buffer(indices.len())?;
        for t in indices {
            target.push(t.map(|v| decoded[v as usize]));
        }
        let guide = (construction - self.inherited).max(construction * 0.25);
        let [to_proxy, from_proxy] = cover(&self.faces, &target, guide, tick)?;
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
        Ok(cert)
    }
}

fn face_buffer(count: usize) -> Result<Vec<Face>, JobError> {
    let mut faces = Vec::new();
    faces
        .try_reserve_exact(count)
        .map_err(|_| resource("could not allocate certificate faces"))?;
    Ok(faces)
}

// SAFETY: The C++ definition uses fixed-width face counts, contiguous float
// triples and a synchronous callback. cover maintains the pointer lifetimes.
unsafe extern "C" {
    fn rt_surface_cert(
        a: *const Face,
        na: u32,
        b: *const Face,
        nb: u32,
        guide: f64,
        forward: *mut f64,
        backward: *mut f64,
        callback: extern "C" fn(*mut c_void, u32) -> c_int,
        context: *mut c_void,
    ) -> c_int;
}

struct Callback<'a> {
    tick: &'a dyn Fn(u32) -> Result<(), JobError>,
    error: Option<JobError>,
}

extern "C" fn poll(context: *mut c_void, done: u32) -> c_int {
    // SAFETY: cover passes its live, exclusively borrowed Callback. The kernel
    // calls synchronously on this thread and retains no pointer after return.
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

fn cover(
    a: &[Face],
    b: &[Face],
    guide: f64,
    tick: &dyn Fn(u32) -> Result<(), JobError>,
) -> Result<[f64; 2], JobError> {
    let count =
        |n| u32::try_from(n).map_err(|_| resource("surface certificate face count overflows"));
    let (na, nb) = (count(a.len())?, count(b.len())?);
    let mut callback = Callback { tick, error: None };
    let (mut forward, mut backward) = (0., 0.);
    tick(0)?;
    // SAFETY: nested arrays have contiguous C layout. Both input slices, the
    // two distinct outputs, and the callback outlive this synchronous invocation.
    // Native allocation and cancellation exceptions are caught inside the ABI.
    let status = unsafe {
        rt_surface_cert(
            a.as_ptr(),
            na,
            b.as_ptr(),
            nb,
            guide,
            &mut forward,
            &mut backward,
            poll,
            (&raw mut callback).cast(),
        )
    };
    if let Some(error) = callback.error {
        return Err(error);
    }
    match status {
        0 => Ok([forward, backward]),
        2 => Err(resource("could not allocate surface certificate workspace")),
        _ => Err(state("could not certify both finite decoded surfaces")),
    }
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
    fn progress_interrupts_inside_native_work_and_preserves_the_error_kind() {
        let face = [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]];
        let geometry = vec![face; 2048];
        let calls = std::cell::Cell::new(0);
        let stopped = cover(&geometry, &geometry, 0., &|_| {
            calls.set(calls.get() + 1);
            if calls.get() == 4 {
                Err(JobError::new(JobErrorKind::Cancelled, "cancel certificate"))
            } else {
                Ok(())
            }
        })
        .unwrap_err();
        assert_eq!(stopped.kind(), JobErrorKind::Cancelled);
        assert_eq!(calls.get(), 4);
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
            .unwrap();
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
