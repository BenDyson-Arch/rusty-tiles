//! Scale evidence driver (VS-A, VS-B): inspects one archive with the absolute volume caps
//! raised (source, directory, entries and stored bytes) and every other limit at its default,
//! with the amplification budgets scaled from the stored bytes (VS-B-2), as VS-C will. Run
//! explicitly in a release test binary:
//! `RUSTY_TILES_SCALE_INPUT=ARCHIVE BINARY --exact validate::scale_driver::drive --ignored`.
use super::*;

#[test]
#[ignore = "scale evidence: run explicitly with RUSTY_TILES_SCALE_INPUT"]
fn drive() {
    let Some(path) = std::env::var_os("RUSTY_TILES_SCALE_INPUT") else {
        return;
    };
    let limits = ValidationLimits {
        source_archive_bytes: u64::MAX,
        central_directory_bytes: u64::MAX,
        archive_entries: u64::MAX,
        archive_stored_bytes: u64::MAX,
        ..ValidationLimits::default()
    };
    // The test-only seams would hold one record per tile and per payload.
    SEAMS.with(|seams| seams.set(false));
    let start = std::time::Instant::now();
    let result = inspect_facts(ValidationRequest::new(&path), Budgets::Scaled(limits))
        .map(|inspection| inspection.report);
    let seconds = start.elapsed().as_secs_f64();
    let outcome = match &result {
        Ok(report) => format!(
            "ok tiles={} payloads={} contained={} notInspected={}",
            report.tiles,
            report.payloads.len(),
            report
                .checks
                .iter()
                .any(|check| check == "decodedContentContainment"),
            report.not_inspected.join("|")
        ),
        Err(error) => format!("{}: {error}", error.category().0),
    };
    println!("SCALE_RESULT seconds={seconds:.3} {outcome}");
}
