//! Scale evidence driver (VS-A): inspects one archive with the absolute volume caps raised
//! (source, directory, entries, stored bytes and read work) and every other limit at its
//! default, as VS-C will. Run explicitly in a release test binary:
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
        total_bytes_read: u64::MAX,
        ..ValidationLimits::default()
    };
    let start = std::time::Instant::now();
    let result = inspect_selected(ValidationRequest::new(&path), limits);
    let seconds = start.elapsed().as_secs_f64();
    let outcome = match &result {
        Ok(report) => format!(
            "ok tiles={} payloads={}",
            report.tiles,
            report.payloads.len()
        ),
        Err(error) => format!("{}: {error}", error.category().0),
    };
    println!("SCALE_RESULT seconds={seconds:.3} {outcome}");
}
