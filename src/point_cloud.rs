//! General-purpose LAS/LAZ point-cloud tiling, with disk-backed spatial LOD.
use crate::error::Error;
use std::path::Path;

#[derive(Clone, Debug)]
pub struct PointCloudOptions {
    /// `local` for XYZ metres, `header` for LAS CRS, or an explicit horizontal CRS.
    pub source_crs: String,
    /// Explicit metre offset from source Z to ellipsoidal height, for geospatial input.
    pub height_offset: Option<f64>,
    pub max_points: usize,
    pub chunk_points: usize,
}

pub fn point_cloud_to_3tz(
    input: &Path,
    output: &Path,
    options: &PointCloudOptions,
) -> Result<(), Error> {
    if !input.is_file() {
        return Err(Error::InputNotFound(input.into()));
    }
    if output.exists() {
        return Err(Error::OutputExists(output.into()));
    }
    if options.max_points == 0 || options.chunk_points == 0 {
        return Err(Error::msg("point budgets must be positive"));
    }
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)?;
    let work = tempfile::tempdir_in(parent)?;
    let staging = work.path().join("tiles");
    let mut command = std::process::Command::new("python3");
    command
        .arg("-c")
        .arg(crate::python::script(
            include_str!("../scripts/point_cloud.py"),
            "point-cloud",
            "NumPy, laspy[lazrs] and pyproj",
        )?)
        .arg(input)
        .arg(&staging)
        .arg("--source-crs")
        .arg(&options.source_crs)
        .arg("--max-points")
        .arg(options.max_points.to_string())
        .arg("--chunk-points")
        .arg(options.chunk_points.to_string());
    if let Some(height) = options.height_offset {
        command.arg("--height-offset").arg(height.to_string());
    }
    crate::python::run(&mut command, "point-cloud")?;
    crate::pack::convert_to_3tz(&staging, output, &crate::pack::PackOptions::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_budgets_and_existing_outputs_are_safe_without_python() {
        let work = tempfile::tempdir().unwrap();
        let input = work.path().join("input.las");
        let output = work.path().join("cloud.3tz");
        std::fs::write(&input, b"invalid input").unwrap();
        let options = PointCloudOptions {
            source_crs: "local".into(),
            height_offset: None,
            max_points: 0,
            chunk_points: 100,
        };
        assert!(point_cloud_to_3tz(&input, &output, &options).is_err());
        assert!(!output.exists());
        std::fs::write(&output, b"original").unwrap();
        assert!(point_cloud_to_3tz(&input, &output, &options).is_err());
        assert_eq!(std::fs::read(&output).unwrap(), b"original");
    }
}
