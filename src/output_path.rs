//! Resolve destination identity without creating missing parent directories.
use crate::JobError;
use std::{
    fs,
    path::{Path, PathBuf},
};

pub(crate) fn resolve(path: &Path) -> Result<PathBuf, JobError> {
    let absolute =
        std::path::absolute(path).map_err(|error| JobError::io("resolve output", path, error))?;
    let mut ancestor = absolute.as_path();
    let mut missing = Vec::new();
    loop {
        match fs::canonicalize(ancestor) {
            Ok(mut resolved) => {
                for name in missing.into_iter().rev() {
                    resolved.push(name);
                }
                return Ok(resolved);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let name = ancestor
                    .file_name()
                    .ok_or_else(|| JobError::io("resolve output", path, error))?;
                missing.push(name.to_os_string());
                ancestor = ancestor
                    .parent()
                    .expect("an absolute path with a filename has a parent");
            }
            Err(error) => return Err(JobError::io("resolve output", path, error)),
        }
    }
}
