//! The walk's scratch ledger: the contract downstream slices (S2 samples, S3 pages and
//! charts, ...) use to hold scratch during the HLOD walk.
//!
//! - **Admit before allocating.** A component calls [`ScratchLedger::admit`] with the most
//!   bytes it will hold at once, before it writes any. Admission preflights free space (and,
//!   on memory-backed scratch, the memory charged for it) for the stage-wide simultaneous
//!   total: the incidence row stream's walk bound plus every component admitted at the time.
//! - **One owner per name.** A name is admitted once at a time; a second admission while
//!   the first lease lives is refused.
//! - **Check its own bytes.** The component reports its live bytes through
//!   [`ComponentLease::charge`] as they grow; bytes past its admission are refused, and so
//!   is stage-wide live scratch past the ledger's current bound (bytes nobody admitted).
//! - **Release.** Dropping the lease returns its admission.
//!
//! The stage's peak over the walk is then checked against the incidence bound and the
//! ledger's largest simultaneous admission (`peak_bound`). No slice can trip another's
//! bound, and a slice's bytes are checked against its own admission.
use crate::{spill::Scratch, JobError, JobErrorKind};
use std::{collections::BTreeMap, sync::Mutex};

fn resource(message: String) -> JobError {
    JobError::new(JobErrorKind::ResourceLimit, message)
}

#[derive(Default)]
struct State {
    /// Live admissions by component.
    admitted: BTreeMap<&'static str, u64>,
    /// Largest simultaneous sum of admissions.
    peak: u64,
    /// Largest admission per component, for the report.
    largest: BTreeMap<&'static str, u64>,
}

/// See the module notes.
pub(crate) struct ScratchLedger<'a> {
    scratch: &'a Scratch,
    /// The incidence row stream's bound during the walk (live bytes included).
    base: u64,
    admit: &'a (dyn Fn(u64) -> Result<(), JobError> + Sync),
    state: Mutex<State>,
}

/// One component's admission; dropping it releases the admission.
pub(crate) struct ComponentLease<'l, 'a> {
    ledger: &'l ScratchLedger<'a>,
    name: &'static str,
    bytes: u64,
}

impl<'a> ScratchLedger<'a> {
    /// A ledger over `scratch` whose walk starts with `base` bytes declared (the incidence
    /// row stream's walk phase); `admit` preflights a stage-wide total.
    pub(crate) fn new(
        scratch: &'a Scratch,
        base: u64,
        admit: &'a (dyn Fn(u64) -> Result<(), JobError> + Sync),
    ) -> Self {
        Self {
            scratch,
            base,
            admit,
            state: Mutex::new(State::default()),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Admit `bytes` for component `name` before it allocates any.
    pub(crate) fn admit(
        &self,
        name: &'static str,
        bytes: u64,
    ) -> Result<ComponentLease<'_, 'a>, JobError> {
        let mut state = self.lock();
        if state.admitted.contains_key(name) {
            return Err(JobError::new(
                JobErrorKind::InvalidState,
                format!("scratch component {name} is already admitted (one owner per name)"),
            ));
        }
        let total = state
            .admitted
            .values()
            .try_fold(self.base, |a, b| a.checked_add(*b))
            .and_then(|a| a.checked_add(bytes))
            .ok_or_else(|| resource(format!("scratch component {name} overflows")))?;
        (self.admit)(total)?;
        state.admitted.insert(name, bytes);
        state.peak = state.peak.max(total - self.base);
        let largest = state.largest.entry(name).or_default();
        *largest = (*largest).max(bytes);
        Ok(ComponentLease {
            ledger: self,
            name,
            bytes,
        })
    }

    /// The current bound: the walk's base and every live admission.
    pub(crate) fn bound(&self) -> u64 {
        self.base + self.lock().admitted.values().sum::<u64>()
    }

    /// Stage-wide: live scratch within the current bound.
    pub(crate) fn check(&self) -> Result<(), JobError> {
        let (live, bound) = (self.scratch.live_bytes(), self.bound());
        if live > bound {
            return Err(resource(format!(
                "HLOD walk scratch holds {live} bytes, past the {bound} admitted"
            )));
        }
        Ok(())
    }

    /// The walk's largest simultaneous bound.
    pub(crate) fn peak_bound(&self) -> u64 {
        self.base + self.lock().peak
    }

    /// The largest admission of each component.
    pub(crate) fn components(&self) -> BTreeMap<&'static str, u64> {
        self.lock().largest.clone()
    }
}

impl ComponentLease<'_, '_> {
    /// Report the component's own live bytes: past its admission is refused, as is
    /// stage-wide live scratch past the ledger's bound.
    pub(crate) fn charge(&self, live: u64) -> Result<(), JobError> {
        if live > self.bytes {
            return Err(resource(format!(
                "scratch component {} holds {live} bytes, past its admitted {}",
                self.name, self.bytes
            )));
        }
        self.ledger.check()
    }
}

impl Drop for ComponentLease<'_, '_> {
    fn drop(&mut self) {
        self.ledger.lock().admitted.remove(self.name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spill::{Record, SequenceWriter};

    #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    struct Word(u64);
    impl Record for Word {
        const SIZE: usize = 8;
        fn encode(&self, out: &mut [u8]) {
            out.copy_from_slice(&self.0.to_le_bytes());
        }
        fn decode(bytes: &[u8]) -> Self {
            Self(u64::from_le_bytes(bytes.try_into().unwrap()))
        }
    }

    fn write(scratch: &Scratch, stem: &str, bytes: u64) -> crate::spill::Sequence<Word> {
        let mut w = SequenceWriter::<Word>::new(scratch, stem, 1 << 12).unwrap();
        for i in 0..bytes / 8 {
            w.push(&Word(i)).unwrap();
        }
        w.finish().unwrap()
    }

    /// Sol's simulated consumer (round 7): admitted 1 MiB, it writes 2.16 MB. Registration
    /// alone once let that pass; admission now refuses it, both by its own charge and by the
    /// stage-wide check (bytes nobody admitted).
    #[test]
    fn a_component_past_its_admission_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let scratch = Scratch::create(Some(dir.path()), 0).unwrap();
        let preflights = Mutex::new(Vec::new());
        let admit = |total: u64| -> Result<(), JobError> {
            preflights.lock().unwrap().push(total);
            Ok(())
        };
        let ledger = ScratchLedger::new(&scratch, scratch.live_bytes(), &admit);
        let lease = ledger.admit("s3-pages", 1 << 20).unwrap();
        assert_eq!(*preflights.lock().unwrap(), vec![1 << 20]);
        let written = write(&scratch, "pages", 2_160_000);
        let own = written.len() * 8;
        let err = lease.charge(own).unwrap_err();
        assert_eq!(err.kind(), JobErrorKind::ResourceLimit, "{err}");
        // Not reporting does not hide it: the stage-wide check sees the bytes.
        let err = ledger.check().unwrap_err();
        assert_eq!(err.kind(), JobErrorKind::ResourceLimit, "{err}");
        drop(written);
        lease.charge(0).unwrap();
    }

    /// Within admission it passes; admissions add up stage-wide, are preflighted before any
    /// write, belong to one owner at a time, and are released on drop.
    #[test]
    fn admissions_are_stage_wide_owned_and_released() {
        let dir = tempfile::tempdir().unwrap();
        let scratch = Scratch::create(Some(dir.path()), 0).unwrap();
        let preflights = Mutex::new(Vec::new());
        let admit = |total: u64| -> Result<(), JobError> {
            preflights.lock().unwrap().push(total);
            if total > 3 << 20 {
                return Err(resource(format!("no room for {total}")));
            }
            Ok(())
        };
        let ledger = ScratchLedger::new(&scratch, 1 << 20, &admit);
        let samples = ledger.admit("s2-samples", 1 << 20).unwrap();
        let written = write(&scratch, "samples", 1 << 19);
        samples.charge(written.len() * 8).unwrap();
        assert!(ledger.admit("s2-samples", 1).is_err(), "one owner per name");
        // 1 MiB base + 1 MiB samples + 2 MiB pages exceeds the 3 MiB room.
        assert!(ledger.admit("s3-pages", 2 << 20).is_err());
        let pages = ledger.admit("s3-pages", 1 << 20).unwrap();
        assert_eq!(ledger.bound(), 3 << 20);
        drop(samples);
        assert_eq!(ledger.bound(), 2 << 20);
        drop(pages);
        assert_eq!(ledger.peak_bound(), 3 << 20);
        assert_eq!(
            ledger.components(),
            BTreeMap::from([("s2-samples", 1 << 20), ("s3-pages", 1 << 20)])
        );
        assert_eq!(*preflights.lock().unwrap(), vec![2 << 20, 4 << 20, 3 << 20]);
    }
}
