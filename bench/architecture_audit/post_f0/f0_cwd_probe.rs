use rusty_tiles::{package::{package, PackageMember, PackageRequest}, JobError, Observer, OutputPolicy, RunControl, RunEvent};
use std::{fs, path::PathBuf, sync::{Arc, atomic::{AtomicBool, Ordering}}};
struct ChangeDirectory { target: PathBuf, changed: AtomicBool }
impl Observer for ChangeDirectory {
    fn observe(&self, _: &RunEvent<'_>) -> Result<(), JobError> {
        if !self.changed.swap(true, Ordering::SeqCst) { std::env::set_current_dir(&self.target).unwrap(); }
        Ok(())
    }
}
fn main() {
    let original_cwd = std::env::current_dir().unwrap();
    let root = std::env::temp_dir().join(format!("f0-cwd-probe-{}", std::process::id()));
    let initial = root.join("initial"); let source_dir = root.join("selected-source");
    fs::create_dir_all(&initial).unwrap(); fs::create_dir(&source_dir).unwrap();
    let source = source_dir.join("out.3tz"); let original_source = b"unchanged opaque source bytes";
    fs::write(&source, original_source).unwrap(); std::env::set_current_dir(&initial).unwrap();
    let control = RunControl::new(Some(Arc::new(ChangeDirectory { target: source_dir.clone(), changed: AtomicBool::new(false) })));
    let result = package(PackageRequest::members(vec![PackageMember::new("tileset.json", &source)], "out.3tz").with_policy(OutputPolicy::Replace), &control);
    let after = fs::read(&source).unwrap();
    println!("result_ok={} initial_destination_exists={} selected_source_preserved={} selected_source_now_zip={} result_output={:?}", result.is_ok(), initial.join("out.3tz").exists(), after == original_source, after.starts_with(b"PK"), result.as_ref().map(|r| &r.output));
    std::env::set_current_dir(original_cwd).unwrap(); fs::remove_dir_all(root).unwrap();
}
