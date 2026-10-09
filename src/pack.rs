//! Compatibility entry points delegate opaque packaging to the foundation API.
pub use crate::archive3tz::TZ_INDEX_NAME;
use crate::{
    archive3tz::{self, CodecError},
    package::{package, PackageMember, PackageRequest},
    report::ConversionResult,
    runtime::{OutputPolicy, RunControl},
    Error,
};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default)]
pub struct PackOptions {
    pub force: bool,
}
fn policy(options: &PackOptions) -> OutputPolicy {
    if options.force {
        OutputPolicy::Replace
    } else {
        OutputPolicy::CreateNew
    }
}
pub fn convert_to_3tz(input: &Path, output: &Path, options: &PackOptions) -> Result<(), Error> {
    convert_to_3tz_reported(input, output, options).map(drop)
}
pub fn convert_to_3tz_reported(
    input: &Path,
    output: &Path,
    options: &PackOptions,
) -> Result<ConversionResult, Error> {
    let result = package(
        PackageRequest::directory(input, output).with_policy(policy(options)),
        &RunControl::default(),
    )?;
    Ok(ConversionResult {
        output: result.output,
        archive: true,
        report: None,
    })
}
pub fn pack_named_files(
    files: &[(String, PathBuf)],
    output: &Path,
    options: &PackOptions,
) -> Result<(), Error> {
    let members = files
        .iter()
        .map(|(name, path)| PackageMember::new(name, path))
        .collect();
    package(
        PackageRequest::members(members, output).with_policy(policy(options)),
        &RunControl::default(),
    )
    .map(drop)
    .map_err(Error::from)
}

// Only legacy converters call this helper; the public directory request uses
// strict inventory resolution and never skips previous outputs or reserved names.
pub(crate) fn tree_members(root: &Path, output: &Path) -> Result<Vec<(String, PathBuf)>, Error> {
    crate::output::tree_members(root, output)
}
pub fn list_zip_names(path: &Path) -> Result<Vec<String>, Error> {
    archive3tz::list_zip_names(path).map_err(Error::from)
}
pub fn validate_3tz(path: &Path) -> Result<(), Error> {
    archive3tz::validate_3tz(path).map_err(Error::from)
}

// This legacy error bridge does not introduce a codec dependency on jobs/runtime.
impl From<CodecError> for Error {
    fn from(error: CodecError) -> Self {
        match error {
            CodecError::Io(error) => Self::Io(error),
            CodecError::SourceIo { source, .. } => Self::Io(source),
            CodecError::Zip(error) => Self::Zip(error),
            CodecError::Invalid(message) => Self::Message(message),
            CodecError::MissingManifest => Self::MissingTilesetJson,
            CodecError::SourceChanged(path) => {
                Self::Data(format!("source changed while packing: {}", path.display()))
            }
        }
    }
}

#[cfg(test)]
mod reproducibility_tests {
    use super::*;
    use std::fs;
    #[test]
    fn packing_is_independent_of_caller_entry_order() {
        let work = tempfile::tempdir().unwrap();
        let manifest = work.path().join("tileset.json");
        let content = work.path().join("tile.glb");
        fs::write(&manifest, b"{}").unwrap();
        fs::write(&content, b"same payload").unwrap();
        let mut files = vec![
            ("tile.glb".into(), content),
            ("tileset.json".into(), manifest),
        ];
        let a = work.path().join("a.3tz");
        let b = work.path().join("b.3tz");
        pack_named_files(&files, &a, &PackOptions::default()).unwrap();
        files.reverse();
        pack_named_files(&files, &b, &PackOptions::default()).unwrap();
        assert_eq!(fs::read(&a).unwrap(), fs::read(&b).unwrap());
        validate_3tz(&a).unwrap();
    }
}
