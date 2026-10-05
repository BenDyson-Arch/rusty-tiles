//! Staged directory publication with preservation/rollback for explicit replacement.
use crate::Error;
use std::path::Path;

pub(crate) fn publish_directory(staging: &Path, output: &Path, force: bool) -> Result<(), Error> {
    if !output.exists() {
        std::fs::rename(staging, output)?;
        return Ok(());
    }
    if !force {
        return Err(Error::OutputExists(output.into()));
    }
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let backup = tempfile::tempdir_in(parent)?;
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
}
