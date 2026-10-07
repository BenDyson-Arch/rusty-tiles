//! Conversion job lifecycle: input/output preflight, a private work directory
//! beside the output, no-clobber publication and cleanup of failed jobs.
use crate::{report::ConversionResult, Error};
use serde_json::Value;
use std::{
    fs,
    io::{BufWriter, Write},
    path::{Path, PathBuf},
};

/// Name of the machine-readable report every reporting converter publishes.
pub(crate) const REPORT_NAME: &str = "conversion.json";

/// The directory that will contain `output` ("." for a bare file name).
pub(crate) fn parent_dir(output: &Path) -> &Path {
    output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
}

/// Converters that read one source file reject anything else up front.
pub(crate) fn require_file(input: &Path) -> Result<(), Error> {
    if input.is_file() {
        Ok(())
    } else {
        Err(Error::InputNotFound(input.into()))
    }
}

/// Refuse to touch an existing output unless replacement was requested.
pub(crate) fn check_output(output: &Path, force: bool) -> Result<(), Error> {
    if output.exists() && !force {
        return Err(Error::OutputExists(output.into()));
    }
    Ok(())
}

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

/// One conversion: a `.tiles-work-*` directory created next to the output (so
/// publication is a same-filesystem rename) that is removed when the job is
/// dropped, whether it was published or failed.
pub(crate) struct Job {
    work: tempfile::TempDir,
    output: PathBuf,
    force: bool,
}

impl Job {
    /// Check the output and create the work directory. Callers validate their
    /// options and input before beginning a job.
    pub fn begin(output: &Path, force: bool) -> Result<Self, Error> {
        check_output(output, force)?;
        let parent = parent_dir(output);
        fs::create_dir_all(parent)?;
        let work = tempfile::Builder::new()
            .prefix(".tiles-work-")
            .tempdir_in(parent)?;
        Ok(Self {
            work,
            output: output.into(),
            force,
        })
    }

    /// Private scratch space for the whole job.
    pub fn path(&self) -> &Path {
        self.work.path()
    }

    /// A fresh directory inside the job, to be published or packed as a tree.
    pub fn staging(&self, name: &str) -> Result<PathBuf, Error> {
        let path = self.work.path().join(name);
        fs::create_dir(&path)?;
        Ok(path)
    }

    fn result(&self, archive: bool, report: Option<Value>) -> ConversionResult {
        ConversionResult {
            output: self.output.clone(),
            archive,
            report,
        }
    }

    /// Publish a staged directory as the output directory.
    #[cfg_attr(not(feature = "native-geospatial"), allow(dead_code))]
    pub fn publish_dir(
        self,
        staging: &Path,
        report: Option<Value>,
    ) -> Result<ConversionResult, Error> {
        publish_directory(staging, &self.output, self.force)?;
        Ok(self.result(false, report))
    }

    /// Pack named member files into a `.3tz` and publish it.
    pub fn publish_3tz(
        self,
        files: &[(String, PathBuf)],
        report: Option<Value>,
    ) -> Result<ConversionResult, Error> {
        crate::pack::check_members(files, &self.output)?;
        let mut temp = crate::pack::temp_archive(self.work.path())?;
        crate::pack::write_archive(files, temp.as_file_mut())?;
        temp.as_file().sync_all()?;
        let persisted = if self.force {
            temp.persist(&self.output)
        } else {
            // Another writer may have created the output while we worked.
            temp.persist_noclobber(&self.output)
        };
        persisted.map_err(|e| match e.error.kind() {
            std::io::ErrorKind::AlreadyExists if !self.force => {
                Error::OutputExists(self.output.clone())
            }
            _ => Error::Io(e.error),
        })?;
        Ok(self.result(true, report))
    }

    /// Pack every file below a staged tileset directory and publish it.
    pub fn publish_tree_3tz(
        self,
        root: &Path,
        report: Option<Value>,
    ) -> Result<ConversionResult, Error> {
        let files = crate::pack::tree_members(root, &self.output)?;
        self.publish_3tz(&files, report)
    }
}

/// Rename `staging` to `output`. An existing output is moved aside first and
/// restored if the rename fails.
#[cfg_attr(not(feature = "native-geospatial"), allow(dead_code))]
pub(crate) fn publish_directory(staging: &Path, output: &Path, force: bool) -> Result<(), Error> {
    if !output.exists() {
        std::fs::rename(staging, output)?;
        return Ok(());
    }
    if !force {
        return Err(Error::OutputExists(output.into()));
    }
    let backup = tempfile::tempdir_in(parent_dir(output))?;
    let previous = backup.path().join("previous");
    std::fs::rename(output, &previous)?;
    if let Err(error) = std::fs::rename(staging, output) {
        if let Err(restore) = std::fs::rename(&previous, output) {
            let retained = backup.keep();
            return Err(Error::msg(format!("publication failed ({error}); restore failed ({restore}); previous output retained at {}", retained.join("previous").display())));
        }
        return Err(Error::Io(error));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failed_directory_publication_restores_original() {
        let tmp = tempfile::tempdir().unwrap();
        let output = tmp.path().join("output");
        std::fs::create_dir(&output).unwrap();
        std::fs::write(output.join("original"), b"keep me").unwrap();
        assert!(publish_directory(&tmp.path().join("missing"), &output, true).is_err());
        assert_eq!(std::fs::read(output.join("original")).unwrap(), b"keep me");
        assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), 1);
    }

    #[test]
    fn dropped_job_removes_work_and_keeps_output() {
        let tmp = tempfile::tempdir().unwrap();
        let output = tmp.path().join("out.3tz");
        std::fs::write(&output, b"original").unwrap();
        assert!(matches!(
            Job::begin(&output, false),
            Err(Error::OutputExists(_))
        ));
        let job = Job::begin(&output, true).unwrap();
        let work = job.path().to_path_buf();
        assert!(work
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with(".tiles-work-"));
        std::fs::write(job.staging("tree").unwrap().join("partial"), b"x").unwrap();
        drop(job);
        assert!(!work.exists());
        assert_eq!(std::fs::read(&output).unwrap(), b"original");
        assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), 1);
    }

    #[test]
    fn archive_publication_does_not_clobber_a_concurrent_output() {
        let tmp = tempfile::tempdir().unwrap();
        let output = tmp.path().join("out.3tz");
        let job = Job::begin(&output, false).unwrap();
        let manifest = job.path().join("tileset.json");
        std::fs::write(&manifest, b"{}").unwrap();
        std::fs::write(&output, b"raced").unwrap();
        let error = job
            .publish_3tz(&[("tileset.json".into(), manifest)], None)
            .unwrap_err();
        assert!(matches!(error, Error::OutputExists(_)), "{error}");
        assert_eq!(std::fs::read(&output).unwrap(), b"raced");
        assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), 1);
    }

    /// Entry name → bytes of a published archive.
    fn archive_entries(path: &Path) -> Vec<(String, Vec<u8>)> {
        let mut zip = zip::ZipArchive::new(fs::File::open(path).unwrap()).unwrap();
        (0..zip.len())
            .map(|i| {
                let mut entry = zip.by_index(i).unwrap();
                let mut bytes = Vec::new();
                std::io::Read::read_to_end(&mut entry, &mut bytes).unwrap();
                (entry.name().to_string(), bytes)
            })
            .collect()
    }

    /// Names in `dir` other than the given ones (work dirs must not linger).
    fn leftovers(dir: &Path, keep: &[&str]) -> Vec<String> {
        fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| !keep.contains(&n.as_str()))
            .collect()
    }

    /// A job whose staged tree holds `tileset.json` (when `manifest`) and a tile.
    fn staged_tree(output: &Path, force: bool, manifest: bool) -> (Job, PathBuf) {
        let job = Job::begin(output, force).unwrap();
        let root = job.staging("tree").unwrap();
        if manifest {
            fs::write(root.join("tileset.json"), b"{\"new\":true}").unwrap();
        }
        fs::create_dir(root.join("tiles")).unwrap();
        fs::write(root.join("tiles/0.glb"), b"tile").unwrap();
        (job, root)
    }

    #[test]
    fn tree_archive_publication_packs_every_staged_file() {
        let tmp = tempfile::tempdir().unwrap();
        let output = tmp.path().join("out.3tz");
        let (job, root) = staged_tree(&output, false, true);
        let report = serde_json::json!({"ok": true});
        let result = job.publish_tree_3tz(&root, Some(report.clone())).unwrap();
        assert!(result.archive);
        assert_eq!(result.output, output);
        assert_eq!(result.report, Some(report));
        let entries = archive_entries(&output);
        let names: Vec<&str> = entries.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names.first(), Some(&"tileset.json"));
        assert!(names.contains(&"tiles/0.glb"), "{names:?}");
        assert!(entries.contains(&("tiles/0.glb".into(), b"tile".to_vec())));
        assert_eq!(leftovers(tmp.path(), &["out.3tz"]), Vec::<String>::new());
    }

    #[test]
    fn failed_tree_archive_publication_keeps_previous_output_and_no_work_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let output = tmp.path().join("out.3tz");
        fs::write(&output, b"previous").unwrap();
        // No tileset.json: packing must fail before anything is published.
        let (job, root) = staged_tree(&output, true, false);
        let error = job.publish_tree_3tz(&root, None).unwrap_err();
        assert!(matches!(error, Error::MissingTilesetJson), "{error}");
        assert_eq!(fs::read(&output).unwrap(), b"previous");
        assert_eq!(leftovers(tmp.path(), &["out.3tz"]), Vec::<String>::new());
    }

    #[test]
    fn tree_archive_publication_does_not_clobber_without_force() {
        let tmp = tempfile::tempdir().unwrap();
        let output = tmp.path().join("out.3tz");
        let (job, root) = staged_tree(&output, false, true);
        // Another writer created the output while the job was running.
        fs::write(&output, b"raced").unwrap();
        let error = job.publish_tree_3tz(&root, None).unwrap_err();
        assert!(matches!(error, Error::OutputExists(_)), "{error}");
        assert_eq!(fs::read(&output).unwrap(), b"raced");
        assert_eq!(leftovers(tmp.path(), &["out.3tz"]), Vec::<String>::new());
    }

    #[test]
    fn tree_archive_publication_overwrites_with_force() {
        let tmp = tempfile::tempdir().unwrap();
        let output = tmp.path().join("out.3tz");
        fs::write(&output, b"previous").unwrap();
        let (job, root) = staged_tree(&output, true, true);
        job.publish_tree_3tz(&root, None).unwrap();
        let entries = archive_entries(&output);
        assert!(entries.contains(&("tileset.json".into(), b"{\"new\":true}".to_vec())));
        assert_eq!(leftovers(tmp.path(), &["out.3tz"]), Vec::<String>::new());
    }
}
