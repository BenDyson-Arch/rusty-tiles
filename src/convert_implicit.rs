//! Conservative, byte-preserving conversion of this crate's explicit archives.
use crate::{implicit::SubdivisionScheme, output::Job, ConversionResult, Error, Reporter};
use serde_json::{json, Value};
use std::{collections::BTreeSet, fs, path::Path};

/// Only an existing output is replaced when `force` is true. Input is never modified.
#[derive(Clone, Debug, Default)]
pub struct ConvertToImplicitOptions {
    pub force: bool,
}

fn invalid(message: impl Into<String>) -> Error {
    Error::Data(format!("convert-to-implicit: {}", message.into()))
}
fn irregular(message: &str, command: &str) -> Error {
    invalid(format!(
        "irregular explicit hierarchy ({message}); re-run {command} from source without --explicit"
    ))
}
fn read_json(zip: &mut zip::ZipArchive<fs::File>, name: &str) -> Result<Value, Error> {
    let file = zip
        .by_name(name)
        .map_err(|_| invalid(format!("missing {name}; foreign archives are unsupported")))?;
    if file.size() > 64 * 1024 * 1024 {
        return Err(invalid(format!("{name} exceeds 64 MiB")));
    }
    Ok(serde_json::from_reader(file)?)
}
fn safe_name(name: &str) -> bool {
    !name.is_empty()
        && !name.contains(['\\', ':', '?', '#', '%', '{', '}'])
        && !name.split('/').any(|p| matches!(p, "" | "." | ".."))
}

/// Convert an eligible rusty-tiles point/vector `.3tz` without decoding or
/// changing its GLB/b3dm bytes. Binary median trees are accepted only when each
/// child fits one distinct regular cell. General kd-trees and LOD chains fail.
pub fn convert_to_implicit(
    input: &Path,
    output: &Path,
    options: &ConvertToImplicitOptions,
) -> Result<(), Error> {
    convert_to_implicit_reported(input, output, options, &Reporter::human_stderr()).map(|_| ())
}

pub fn convert_to_implicit_reported(
    input: &Path,
    output: &Path,
    options: &ConvertToImplicitOptions,
    reporter: &Reporter,
) -> Result<ConversionResult, Error> {
    crate::output::require_file(input)?;
    crate::output::check_output(output, options.force)?;
    if fs::canonicalize(input).ok() == fs::canonicalize(output).ok() && output.exists() {
        return Err(invalid("input and output must be different paths"));
    }
    if input.extension().and_then(|s| s.to_str()) != Some("3tz")
        || output.extension().and_then(|s| s.to_str()) != Some("3tz")
    {
        return Err(invalid("input and output must have the .3tz extension"));
    }
    crate::pack::validate_3tz(input)?;
    let mut zip = zip::ZipArchive::new(fs::File::open(input)?)?;
    let mut manifest = read_json(&mut zip, "tileset.json")?;
    if manifest["root"].get("implicitTiling").is_some() {
        return Err(invalid("archive is already implicit"));
    }
    if zip
        .file_names()
        .any(|name| name.starts_with("implicit-") || name.starts_with("subtrees/"))
    {
        return Err(invalid("archive contains reserved implicit output paths"));
    }
    if manifest.get("schemaUri").is_some() {
        return Err(invalid(
            "external tileset schemaUri is unsupported; use an inline schema before conversion",
        ));
    }
    if manifest["schema"]["classes"].get("rustyTile").is_some() {
        return Err(invalid("source schema uses reserved rustyTile class"));
    }
    let source_report = if zip.file_names().any(|name| name == "conversion.json") {
        read_json(&mut zip, "conversion.json")?
    } else {
        Value::Null
    };
    let (scheme, command) = if source_report["encoder"] == "rusty-tiles-native-las-v1" {
        (SubdivisionScheme::Octree, "point-cloud")
    } else if manifest["asset"]["extras"]["vectorBuildStateSha256"].is_string() {
        let state = read_json(&mut zip, "vector-build.json")?;
        if !state["config"]["encoder"].as_str().is_some_and(|s| {
            s.starts_with("rusty-tiles-native-vector-v1:")
                || s.starts_with("rusty-tiles-portable-vector-v1:")
        }) {
            return Err(invalid(
                "foreign vector encoder; only rusty-tiles archives are supported",
            ));
        }
        (SubdivisionScheme::Quadtree, "vector")
    } else if manifest["asset"]["generator"] == "rusty-tiles"
        || source_report["encoder"]
            .as_str()
            .is_some_and(|s| s.contains("mesh"))
    {
        return Err(invalid(
            "mesh archives are unsupported; re-run mesh-to-3tz from source without --explicit",
        ));
    } else {
        return Err(invalid(
            "foreign or unsupported archive; only rusty-tiles point-cloud and vector archives with conversion provenance are supported; for mesh archives, re-run mesh-to-3tz from source without --explicit",
        ));
    };
    // Validate checks index, duplicate/safe paths, checksums, bounds, resource
    // references and existing metadata before anything is staged.
    let original_root = manifest["root"].clone();
    let cell = bounds(&original_root, [0.; 3], command)?;
    let mut count = 0;
    let padding = if scheme == SubdivisionScheme::Octree {
        source_report["maxPositionRoundingMetres"]
            .as_f64()
            .unwrap_or(0.)
            + 1e-6
    } else {
        0.001
    };
    check_tree(
        &mut manifest["root"],
        cell,
        [0.; 3],
        scheme,
        command,
        padding,
        0,
        &mut count,
    )?;
    crate::validate::archive(input, None)?;
    let job = Job::begin(output, options.force)?;
    let staging = job.staging("tileset")?;
    let mut names = BTreeSet::new();
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index)?;
        let name = entry.name().to_owned();
        if name == crate::TZ_INDEX_NAME {
            continue;
        }
        if !safe_name(&name) || !names.insert(name.clone()) {
            return Err(invalid(format!("unsafe or duplicate archive path: {name}")));
        }
        if name.starts_with("implicit-") || name.starts_with("subtrees/") {
            return Err(invalid("archive contains reserved implicit output paths"));
        }
        let target = staging.join(&name);
        fs::create_dir_all(target.parent().unwrap())?;
        std::io::copy(&mut entry, &mut fs::File::create(target)?)?;
    }
    reporter.progress("implicit", 0, count as u64);
    let header = manifest.clone();
    let mut wrappers = 0;
    wrap_contents(
        &mut manifest["root"],
        &original_root,
        [0.; 3],
        true,
        &header,
        &staging,
        &mut wrappers,
    )?;
    crate::implicit::write_tileset(&mut manifest, &staging, scheme, false)?;
    // Original tile metadata schemas remain in external content wrappers. Merge
    // their classes into the main schema too, without replacing semantic classes.
    if header.get("schema").is_some() {
        let class = manifest["schema"]["classes"]["rustyTile"].clone();
        manifest["schema"] = header["schema"].clone();
        manifest["schema"]["classes"]["rustyTile"] = class;
    }
    fs::write(
        staging.join("tileset.json"),
        serde_json::to_vec_pretty(&manifest)?,
    )?;
    let mut report = json!({
        "operation":"convert-to-implicit", "encoder":"rusty-tiles-explicit-to-implicit-v1",
        "tiling":"implicit", "subdivisionScheme":scheme.as_str(), "tiles":count,
        "contentWrappers":wrappers, "contentBytesPreserved":true,
        "sourceReport":source_report,
    });
    if let Some(name) = source_report.get("geometryReports") {
        report["geometryReports"] = name.clone();
    }
    let report = crate::output::write_report(&staging, report, true)?;
    reporter.progress("implicit", count as u64, count as u64);
    // Validate the fully packed candidate before publication, including external
    // content bounds and every untouched metadata/resource reference.
    let candidate = job.path().join("candidate.3tz");
    let files = crate::pack::tree_members(&staging, &candidate)?;
    crate::pack::pack_named_files(&files, &candidate, &crate::pack::PackOptions::default())?;
    crate::validate::archive(&candidate, None)?;
    job.publish_tree_3tz(&staging, Some(report))
}

fn bounds(node: &Value, delta: [f64; 3], command: &str) -> Result<[f64; 12], Error> {
    let mut b: [f64; 12] = serde_json::from_value(node["boundingVolume"]["box"].clone())
        .map_err(|_| irregular("requires axis-aligned box bounds", command))?;
    if b.iter().any(|n| !n.is_finite())
        || [4, 5, 6, 8, 9, 10].iter().any(|&i| b[i] != 0.)
        || [3, 7, 11].iter().any(|&i| b[i] <= 0.)
    {
        return Err(irregular(
            "requires finite, positive axis-aligned box bounds",
            command,
        ));
    }
    for i in 0..3 {
        b[i] += delta[i];
    }
    Ok(b)
}
fn offset(node: &Value, command: &str) -> Result<[f64; 3], Error> {
    if node.get("transform").is_none() {
        return Ok([0.; 3]);
    }
    let matrix: [f64; 16] = serde_json::from_value(node["transform"].clone())
        .map_err(|_| irregular("invalid child transform", command))?;
    for (i, value) in matrix.iter().enumerate() {
        if !value.is_finite() || (i < 12 || i == 15) && *value != if i % 5 == 0 { 1. } else { 0. } {
            return Err(irregular("child transform must be a translation", command));
        }
    }
    Ok([matrix[12], matrix[13], matrix[14]])
}

#[allow(clippy::too_many_arguments)]
fn check_tree(
    node: &mut Value,
    cell: [f64; 12],
    delta: [f64; 3],
    scheme: SubdivisionScheme,
    command: &str,
    padding: f64,
    depth: u32,
    count: &mut usize,
) -> Result<(), Error> {
    *count += 1;
    if *count > 1_000_000 || depth > 31 {
        return Err(irregular("exceeds implicit coordinate limits", command));
    }
    if node.get("implicitTiling").is_some() || node["refine"] != "REPLACE" {
        return Err(irregular("requires explicit REPLACE tiles", command));
    }
    if node.get("viewerRequestVolume").is_some() {
        return Err(irregular("viewer request volumes are unsupported", command));
    }
    let axes = if scheme == SubdivisionScheme::Octree {
        3
    } else {
        2
    };
    let mut used = BTreeSet::new();
    if let Some(children) = node.get_mut("children").and_then(Value::as_array_mut) {
        for child in children {
            let translation = offset(child, command)?;
            let child_delta = std::array::from_fn(|i| delta[i] + translation[i]);
            let b = bounds(child, child_delta, command)?;
            let slot = (0..axes).fold(0usize, |s, i| s | (usize::from(b[i] >= cell[i]) << i));
            if !used.insert(slot) {
                return Err(irregular(
                    "multiple children occupy the same regular cell",
                    command,
                ));
            }
            let mut child_cell = cell;
            for i in 0..axes {
                child_cell[3 + 4 * i] /= 2.;
                child_cell[i] += if slot & (1 << i) == 0 {
                    -child_cell[3 + 4 * i]
                } else {
                    child_cell[3 + 4 * i]
                };
            }
            let extra_padding = child["extras"]["positionRoundingMetres"]
                .as_f64()
                .unwrap_or(0.)
                + child["extras"]["quantizationErrorMetres"]
                    .as_f64()
                    .unwrap_or(0.);
            if (0..3).any(|i| {
                (b[i] - child_cell[i]).abs() + b[3 + 4 * i]
                    > child_cell[3 + 4 * i] + padding + extra_padding + 1e-8 * cell[3 + 4 * i]
            }) {
                return Err(irregular(
                    "child bounds cross their midpoint cell beyond recorded padding",
                    command,
                ));
            }
            if let Some(value) = child["extras"]["implicitChildIndex"].as_u64() {
                if value != slot as u64 {
                    return Err(irregular("child index disagrees with bounds", command));
                }
            }
            child["_rustyImplicitChildIndex"] = json!(slot);
            check_tree(
                child,
                child_cell,
                child_delta,
                scheme,
                command,
                padding,
                depth + 1,
                count,
            )?;
        }
    }
    Ok(())
}

fn wrap_contents(
    node: &mut Value,
    original: &Value,
    delta: [f64; 3],
    root: bool,
    header: &Value,
    directory: &Path,
    count: &mut usize,
) -> Result<(), Error> {
    let command = "point-cloud/vector";
    let delta = if root {
        delta
    } else {
        let offset = offset(original, command)?;
        std::array::from_fn(|i| delta[i] + offset[i])
    };
    if original.get("content").is_some()
        || original.get("contents").is_some()
        || original.get("metadata").is_some()
        || original.get("extensions").is_some()
        || original["extras"].get("vertices").is_some()
        || original["extras"].get("encodedBytes").is_some()
    {
        let mut wrapper = header.clone();
        let mut tile = original.clone();
        tile.as_object_mut().unwrap().remove("children");
        tile["transform"] = crate::tileset_node::translation(delta);
        let contents: Vec<Value> = original
            .get("contents")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_else(|| original.get("content").cloned().into_iter().collect());
        let mut fixed = Vec::new();
        for mut content in contents {
            let uri = content["uri"]
                .as_str()
                .ok_or_else(|| invalid("content has no URI"))?;
            if !safe_name(uri)
                || !uri.starts_with("t/")
                || !matches!(
                    Path::new(uri).extension().and_then(|s| s.to_str()),
                    Some("glb" | "b3dm")
                )
            {
                return Err(invalid(
                    "only original local t/ GLB/b3dm content is supported",
                ));
            }
            content["uri"] = json!(format!("../{uri}"));
            fixed.push(content);
        }
        tile.as_object_mut().unwrap().remove("content");
        tile.as_object_mut().unwrap().remove("contents");
        if fixed.is_empty() {
            // Routing metadata can be carried by an empty external root.
        } else if fixed.len() == 1 {
            tile["content"] = fixed.remove(0);
        } else {
            tile["contents"] = json!(fixed);
        }
        wrapper["root"] = tile;
        wrapper["geometricError"] = original["geometricError"].clone();
        wrapper["asset"].as_object_mut().unwrap().remove("extras");
        let name = format!("implicit-source-{count}.json");
        *count += 1;
        fs::write(directory.join(&name), serde_json::to_vec(&wrapper)?)?;
        node.as_object_mut().unwrap().remove("contents");
        node["content"] = json!({"uri":name});
    }
    node["boundingVolume"]["box"] = json!(bounds(original, delta, command)?);
    if !root {
        node.as_object_mut().unwrap().remove("transform");
    }
    if let Some(extras) = node.get_mut("extras").and_then(Value::as_object_mut) {
        // Payload statistics on the external-content root refer to the original
        // bytes; implicit routing nodes have JSON content instead.
        extras.remove("vertices");
        extras.remove("encodedBytes");
    }
    if let Some(children) = node.get_mut("children").and_then(Value::as_array_mut) {
        for (child, original) in children
            .iter_mut()
            .zip(original["children"].as_array().unwrap())
        {
            wrap_contents(child, original, delta, false, header, directory, count)?;
        }
    }
    Ok(())
}
