//! Owner-approved root-only competition. One extra proposal, one source S2 pass,
//! strict published-G selection, and no appearance work on the loser.
use super::*;

/// The incumbent survives while the candidate uses the ordinary job workspace.
/// Its controls/change are released first. Geometry has its standard capped charge;
/// decoded + stored coordinates and locks need at most 3C * (12 + 12 + 1) bytes.
pub(super) fn retained_bytes(cap: u64) -> u64 {
    TileGeometry::charge(cap)
        .saturating_add(cap.saturating_mul(75))
        .saturating_add(4096)
}

/// Worker CPU seconds, kept out of deterministic archive/sidecar publication.
#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct Timing {
    pub reduction: f64,
    pub measurement: f64,
    pub appearance: f64,
}
impl Timing {
    pub(super) fn add(&mut self, other: Self) {
        self.reduction += other.reduction;
        self.measurement += other.measurement;
        self.appearance += other.appearance;
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Receipt {
    pub baseline_g: Option<f64>,
    pub baseline_m: Option<f64>,
    pub candidate_g: Option<f64>,
    pub candidate_m: Option<f64>,
    pub accepted: bool,
    pub reason: &'static str,
    #[serde(skip)]
    pub cpu_seconds: f64,
    #[serde(skip)]
    pub timing: Timing,
}

fn improves(old: Option<f64>, new: f64) -> bool {
    new.is_finite() && old.is_none_or(|g| new < g)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn select(
    job: &Job<'_, '_>,
    key: TileKey,
    children: &[TileGeometry],
    contract: &borders::Contract,
    outcome: &mut geometry::Outcome,
    prepared: &mut Option<Prepared>,
    query: &mut source_index::SourceQuery<'_>,
    nearest: &mut error::Nearest,
    audit: Option<&audit::Writer<'_>>,
    attempts: &mut u32,
    published_attempt: &mut Option<audit::Index>,
) -> Result<Receipt, JobError> {
    let start = error::thread_cpu();
    let mut receipt = Receipt {
        baseline_g: prepared.as_ref().map(|p| p.error.geometric_error),
        baseline_m: prepared.as_ref().map(|p| p.measurement.sampled_max),
        candidate_g: None,
        candidate_m: None,
        accepted: false,
        reason: "geometry_refused",
        cpu_seconds: 0.,
        timing: Timing::default(),
    };
    if let Some(built) = &mut outcome.built {
        built.locked.shrink_to_fit();
        built.change = Default::default();
        built.controls = Default::default();
    }
    let candidate =
        geometry::root_candidate(key, children, job.input.settings.cap, contract, job.check)?;
    receipt.timing.reduction = error::thread_cpu().zip(start).map_or(0., |(b, a)| b - a);
    if let Some(mut built) = candidate {
        let measure_start = error::thread_cpu();
        let measured = prepare(job, &mut built, contract, query, nearest)?;
        receipt.timing.measurement = error::thread_cpu()
            .zip(measure_start)
            .map_or(0., |(b, a)| b - a);
        let eligible = measured.is_some();
        let index = if let Some(audit) = audit {
            *attempts += 1;
            Some(audit.topology_attempt(&built, *attempts, eligible)?)
        } else {
            None
        };
        outcome.attempted_topology.add(built.change.receipt);
        receipt.reason = "prepare_refused";
        if let Some(mut measured) = measured {
            receipt.candidate_g = Some(measured.error.geometric_error);
            receipt.candidate_m = Some(measured.measurement.sampled_max);
            receipt.accepted = improves(receipt.baseline_g, measured.error.geometric_error);
            receipt.reason = if receipt.accepted {
                "lower_g_or_missing"
            } else {
                "incumbent_not_worse"
            };
            // Both complete measurements consumed CPU, even though only one is published.
            if let Some(old) = prepared.as_mut() {
                if receipt.accepted {
                    measured.error_cpu.add(old.error_cpu);
                    measured.error_cpu_seconds += old.error_cpu_seconds;
                } else {
                    old.error_cpu.add(measured.error_cpu);
                    old.error_cpu_seconds += measured.error_cpu_seconds;
                }
            }
            if receipt.accepted {
                *prepared = Some(measured);
                outcome.built = Some(built);
                *published_attempt = index;
            }
        }
    }
    receipt.cpu_seconds = error::thread_cpu().zip(start).map_or(0., |(b, a)| b - a);
    Ok(receipt)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn strict_published_g_keeps_ties_and_can_fill_a_missing_root() {
        assert!(improves(Some(6.), 5.));
        assert!(!improves(Some(6.), 6.));
        assert!(!improves(Some(6.), 7.));
        assert!(improves(None, 6.));
        assert!(!improves(None, f64::INFINITY));
        assert!(!improves(None, f64::NAN));
    }
}
