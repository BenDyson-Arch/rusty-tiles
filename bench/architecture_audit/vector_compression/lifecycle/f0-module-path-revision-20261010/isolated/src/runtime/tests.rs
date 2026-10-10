use super::*;
use std::io::Write;

fn candidate<'a>(output: &Path, attempt: &'a Attempt) -> Staging<'a> {
    let mut staging = Staging::create(output, attempt).unwrap();
    staging.writer().write_all(b"complete candidate").unwrap();
    staging
}


include!("../../probes/p5_f0_seams.rs");
