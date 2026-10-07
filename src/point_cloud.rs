//! General-purpose LAS/LAZ point-cloud tiling, with disk-backed spatial LOD.
use crate::error::Error;
use std::path::Path;
use std::{
    fs::File,
    io::{BufWriter, Write},
};

mod source;
mod tiles;
use source::{header_crs, las_error, read_source, Layout, RAW};

#[derive(Clone, Debug)]
pub struct PointCloudOptions {
    pub force: bool,
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
    if output.exists() && !options.force {
        return Err(Error::OutputExists(output.into()));
    }
    if options.max_points == 0 {
        return Err(Error::msg("--maxPoints must be at least 1"));
    }
    if options.chunk_points == 0 {
        return Err(Error::msg("--chunkPoints must be at least 1"));
    }
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)?;
    let work = tempfile::tempdir_in(parent)?;
    let staging = work.path().join("tiles");
    convert(input, &staging, options)?;
    crate::pack::convert_to_3tz(
        &staging,
        output,
        &crate::pack::PackOptions {
            force: options.force,
        },
    )
}

enum Coordinates {
    Local,
    #[cfg(feature = "native-geospatial")]
    Ecef(crate::geospatial::EcefTransform),
}

impl Coordinates {
    fn new(
        header: &las::Header,
        options: &PointCloudOptions,
    ) -> Result<(Self, Option<String>), Error> {
        if options.source_crs == "local" {
            if options.height_offset.is_some() {
                return Err(Error::Data(
                    "heightOffset applies only to geospatial CRS; local XYZ is in metres".into(),
                ));
            }
            return Ok((Self::Local, None));
        }
        if !options.height_offset.is_some_and(f64::is_finite) {
            return Err(Error::Data("geospatial input requires explicit finite --heightOffset to ellipsoidal metres (0 if established)".into()));
        }
        let definition = if options.source_crs == "header" {
            header_crs(header)?
        } else {
            options.source_crs.clone()
        };
        #[cfg(feature = "native-geospatial")]
        {
            let source = crate::geospatial::Crs::from_definition(&definition)?;
            if !source.is_horizontal() {
                return Err(Error::Data("use a 2D horizontal CRS and explicit ellipsoidal height offset; compound/geocentric CRS is unsupported".into()));
            }
            let transform = crate::geospatial::EcefTransform::new(source, options.height_offset)?;
            Ok((Self::Ecef(transform), Some(definition)))
        }
        #[cfg(not(feature = "native-geospatial"))]
        {
            let _ = definition;
            Err(Error::Environment("geospatial point clouds require a build with --features native-geospatial (GDAL >= 3.12, PROJ >= 9.2); local XYZ needs no GDAL or Python".into()))
        }
    }

    fn transform(&mut self, positions: Vec<[f64; 3]>) -> Result<Vec<[f64; 3]>, Error> {
        match self {
            Self::Local => Ok(positions),
            #[cfg(feature = "native-geospatial")]
            Self::Ecef(transform) => transform.transform(&positions),
        }
    }
}

fn convert(input: &Path, output: &Path, options: &PointCloudOptions) -> Result<(), Error> {
    std::fs::create_dir_all(output.join("scratch"))?;
    std::fs::create_dir(output.join("t"))?;
    let path = output.join("scratch/source.bin");
    let (layout, origin, count, resolved, scales, offsets) = {
        let (mut reader, header) = read_source(input)?;
        let header = &header;
        let layout = Layout::new(header)?;
        let (mut coordinates, resolved) = Coordinates::new(header, options)?;
        let expected = header.number_of_points();
        let transforms = header.transforms();
        let scales = [transforms.x.scale, transforms.y.scale, transforms.z.scale];
        let offsets = [
            transforms.x.offset,
            transforms.y.offset,
            transforms.z.offset,
        ];
        if !scales.iter().chain(&offsets).all(|v| v.is_finite()) {
            return Err(Error::Data("nonfinite LAS source scale/offset".into()));
        }
        let chunk = (options.chunk_points as u64).min(expected);
        if usize::try_from(chunk)
            .ok()
            .and_then(|n| n.checked_mul(layout.record_len))
            .is_none_or(|n| n > isize::MAX as usize)
        {
            return Err(Error::Data("point chunk exceeds platform limits".into()));
        }
        let mut points = las::PointDataBuilder::new().for_header(header).build();
        let mut origin = None;
        let mut count = 0_u64;
        let mut file = BufWriter::new(File::create(&path)?);
        progress("ingestion", 0, expected);
        loop {
            let n = reader
                .fill_points(options.chunk_points as u64, &mut points)
                .map_err(las_error)?;
            if n == 0 {
                break;
            }
            let raw_len = layout.record_len - RAW;
            let mut xyz = Vec::with_capacity(points.len());
            for raw in points.raw_bytes().chunks_exact(raw_len) {
                layout.validate(raw)?;
                let source: [f64; 3] = std::array::from_fn(|i| {
                    i32::from_le_bytes(raw[i * 4..i * 4 + 4].try_into().unwrap()) as f64 * scales[i]
                        + offsets[i]
                });
                if !source.iter().all(|v| v.is_finite()) {
                    return Err(Error::Data("nonfinite source coordinates".into()));
                }
                xyz.push(source);
            }
            let projected = coordinates.transform(xyz.clone())?;
            let origin = *origin.get_or_insert(projected[0]);
            for ((raw, source), point) in points
                .raw_bytes()
                .chunks_exact(raw_len)
                .zip(&xyz)
                .zip(projected)
            {
                for i in 0..3 {
                    let relative = point[i] - origin[i];
                    if !relative.is_finite() {
                        return Err(Error::Data("nonfinite projected coordinates".into()));
                    }
                    file.write_all(&relative.to_le_bytes())?;
                }
                file.write_all(&count.to_le_bytes())?;
                for value in source {
                    file.write_all(&value.to_le_bytes())?;
                }
                file.write_all(raw)?;
                count += 1;
            }
            progress("ingestion", count, expected);
        }
        file.flush()?;
        if count != expected {
            return Err(Error::Data("LAS point count differs from header".into()));
        }
        (
            layout,
            origin.ok_or_else(|| Error::Data("empty point cloud".into()))?,
            count,
            resolved,
            scales,
            offsets,
        )
    };
    let mut tree = tiles::Tree {
        layout: &layout,
        options,
        output,
        tiles: 0,
        max_rounding: 0.,
        total_points: count,
        leaf_points: 0,
    };
    progress("tiling", 0, count);
    let mut root = tree.build(&path, [0.; 3], 0)?;
    for i in 0..3 {
        let value = root["transform"][12 + i].as_f64().unwrap() + origin[i];
        if !value.is_finite() {
            return Err(Error::Data("nonfinite tileset origin".into()));
        }
        root["transform"][12 + i] = value.into();
    }
    std::fs::remove_dir(output.join("scratch"))?;
    let error = tiles::tileset_error(&root);
    if !error.is_finite() {
        return Err(Error::Data(
            "tileset extent exceeds finite coordinate range".into(),
        ));
    }
    std::fs::write(
        output.join("tileset.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"asset":{"version":"1.1"},"geometricError":error,"root":root}),
        )?,
    )?;
    let report = serde_json::json!({"points":count,"tiles":tree.tiles,"sourceCrs":options.source_crs,
        "resolvedCrs":resolved,"sourceScales":scales,"sourceOffsets":offsets,"heightOffset":options.height_offset,
        "properties":layout.dimensions.iter().map(|d| &d.name).collect::<Vec<_>>(),
        "maxPositionRoundingMetres":tree.max_rounding,
        "sampling":"first source point per voxel; celldiagonal bounds source-to-sample distance",
        "maxPoints":options.max_points,"chunkPoints":options.chunk_points,"encoder":"rusty-tiles-native-las-v1"});
    std::fs::write(
        output.join("conversion.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    if std::env::var_os("RUSTY_TILES_JSON_STDOUT").is_none()
        && std::env::var_os("RUSTY_TILES_PROGRESS_JSON").is_none()
    {
        eprintln!(
            "point cloud: {count} points, {} tiles; max local float32 rounding {:.6} m",
            tree.tiles, tree.max_rounding
        );
    }
    Ok(())
}

fn progress(phase: &str, done: u64, total: u64) {
    if std::env::var_os("RUSTY_TILES_PROGRESS_JSON").is_some() {
        eprintln!(
            "{}",
            serde_json::json!({"event":"progress","phase":phase,"done":done,"total":total})
        );
    }
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
            force: false,
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
