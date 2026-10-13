//! Scale evidence driver (VS-A, VS-B, VS-C): inspects one archive with the default limit plan
//! (no volume cap, budgets `max(floor, ⌊S·p/q⌋)`), exactly as the CLI with no flags, and prints
//! the outcome and each budget's use per stored byte. Run explicitly in a release test binary:
//! `RUSTY_TILES_SCALE_INPUT=ARCHIVE BINARY --exact validate::scale_driver::drive --ignored`.
use super::*;

#[test]
#[ignore = "scale evidence: run explicitly with RUSTY_TILES_SCALE_INPUT"]
fn drive() {
    let Some(path) = std::env::var_os("RUSTY_TILES_SCALE_INPUT") else {
        return;
    };
    // The test-only seams would hold one record per tile and per payload.
    SEAMS.with(|seams| seams.set(false));
    let start = std::time::Instant::now();
    let result = inspect_facts(
        ValidationRequest::new(&path),
        Budgets::Plan(LimitPlan::default()),
    );
    let seconds = start.elapsed().as_secs_f64();
    let outcome = match &result {
        Ok(inspection) => {
            let report = &inspection.report;
            let stored = report.stored_bytes.max(1) as f64;
            let [visits, references, bytes_read] = inspection.counters;
            format!(
                "ok tiles={} payloads={} contained={} notInspected={} storedBytes={} \
                 units={} unitsBudget={} unitsPerS={:.3} bytesRead={} bytesReadPerS={:.3} visits={} \
                 visitsPerS={:.6} references={} referencesPerS={:.6} elements={} \
                 elementsPerS={:.3}",
                report.tiles,
                report.payloads.len(),
                report
                    .checks
                    .iter()
                    .any(|check| check == "decodedContentContainment"),
                report.not_inspected.join("|"),
                report.stored_bytes,
                inspection.units,
                report.limits.containment_work_units,
                inspection.units as f64 / stored,
                bytes_read,
                bytes_read as f64 / stored,
                visits,
                visits as f64 / stored,
                references,
                references as f64 / stored,
                inspection.elements,
                inspection.elements as f64 / stored,
            )
        }
        Err(error) => format!("{}: {error}", error.category().0),
    };
    println!("SCALE_RESULT seconds={seconds:.3} {outcome}");
}
