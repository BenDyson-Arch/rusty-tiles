//! The `conversion.json` report a converter publishes with its output.
use crate::Error;
use serde_json::Value;
use std::{
    fs,
    io::{BufWriter, Write},
    path::Path,
};

/// Name of the machine-readable report every reporting converter publishes.
pub(crate) const REPORT_NAME: &str = "conversion.json";

/// Write `conversion.json` into a staged output and return the value, so
/// callers can hand the exact published report back without rereading it.
/// Raster keeps its historical compact form; every other report is pretty.
pub(crate) fn write_report(dir: &Path, report: Value, pretty: bool) -> Result<Value, Error> {
    let mut file = BufWriter::new(fs::File::create(dir.join(REPORT_NAME))?);
    if pretty {
        serde_json::to_writer_pretty(&mut file, &report)?;
    } else {
        serde_json::to_writer(&mut file, &report)?;
    }
    file.flush()?;
    Ok(report)
}
