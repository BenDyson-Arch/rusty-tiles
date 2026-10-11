//! Conservative, byte-preserving conversion of this crate's explicit archives.
use crate::{
    implicit::SubdivisionScheme,
    output::Job,
    package::{package, PackageMember, PackageRequest},
    ConversionResult, Error, Reporter, RunControl,
};
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
/// changing its GLB/b3dm bytes. Each content tile owns its replacement descendants.
/// Binary median trees are accepted only when each
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
    crate::archive3tz::validate_3tz(input)?;
    let mut zip = zip::ZipArchive::new(fs::File::open(input)?)?;
    let mut manifest = read_json(&mut zip, "tileset.json")?;
    if manifest["root"].get("implicitTiling").is_some() {
        return Err(invalid("archive is already implicit"));
    }
    if zip.file_names().any(|name| {
        name.starts_with("implicit-")
            || name.starts_with("subtrees/")
            || name.starts_with("t/owned-")
    }) {
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
    crate::validate::inspect(crate::validate::ValidationRequest::new(input))?;
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
    let mut header = manifest.clone();
    header.as_object_mut().unwrap().remove("root");
    let mut roots = 0;
    let mut retained = BTreeSet::new();
    manifest = owned_document(
        &header,
        &manifest["root"],
        &original_root,
        &staging,
        scheme,
        &mut roots,
        &mut retained,
    )?;
    fs::write(
        staging.join("tileset.json"),
        serde_json::to_vec_pretty(&manifest)?,
    )?;
    let mut report = json!({
        "operation":"convert-to-implicit", "encoder":"rusty-tiles-explicit-to-implicit-v1",
        "tiling":"implicit", "subdivisionScheme":scheme.as_str(), "tiles":count,
        "externalTilesetRoots":roots.saturating_sub(1), "contentBytesPreserved":true,
        "retainedContentUris":retained,
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
    let members = files
        .into_iter()
        .map(|(name, source)| PackageMember::new(name, source))
        .collect();
    package(
        PackageRequest::members(members, &candidate),
        &RunControl::default(),
    )?;
    crate::validate::inspect(crate::validate::ValidationRequest::new(&candidate))?;
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
    if node.get("metadata").is_some() {
        return Err(irregular(
            "explicit tile metadata requires unsupported subtree property tables",
            command,
        ));
    }
    let headers = node["contents"]
        .as_array()
        .cloned()
        .unwrap_or_else(|| node.get("content").cloned().into_iter().collect());
    if headers.iter().any(|c| c.get("boundingVolume").is_some()) {
        return Err(irregular(
            "content bounding volumes require unsupported subtree content metadata",
            command,
        ));
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

/// Each original content tile owns its complete replacement descendants. A
/// separate content-only external root would be a sibling of implicit children,
/// and Cesium would continue selecting the parent proxy after refinement.
/// The 1.1 implicit-root external-content restriction applies at level zero:
/// its JSON slot is unavailable, and actual links only occupy terminal level-one
/// cells. They have neither implicit descendants nor child subtree availability.
#[allow(clippy::too_many_arguments)]
fn owned_document(
    header: &Value,
    checked: &Value,
    original: &Value,
    directory: &Path,
    scheme: SubdivisionScheme,
    roots: &mut usize,
    retained: &mut BTreeSet<String>,
) -> Result<Value, Error> {
    use crate::implicit::{Coordinates, Subtree, TileMetadata};
    let id = *roots;
    *roots += 1;
    let children = original["children"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let has_children = !children.is_empty();
    let mut contents = original["contents"]
        .as_array()
        .cloned()
        .unwrap_or_else(|| original.get("content").cloned().into_iter().collect());
    let template = if scheme == SubdivisionScheme::Octree {
        "{level}-{x}-{y}-{z}"
    } else {
        "{level}-{x}-{y}"
    };
    let zero_key = if scheme == SubdivisionScheme::Octree {
        "0-0-0-0"
    } else {
        "0-0-0"
    };
    for (slot, content) in contents.iter_mut().enumerate() {
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
        retained.insert(uri.to_owned());
        let suffix = Path::new(uri).extension().unwrap().to_str().unwrap();
        let parent = uri.rsplit_once('/').unwrap().0;
        let alias = format!("{parent}/owned-{id}-{zero_key}-{slot}.{suffix}");
        let target = directory.join(&alias);
        if target.exists() {
            return Err(invalid(format!(
                "generated content alias collides with source member: {alias}"
            )));
        }
        fs::copy(directory.join(uri), target)?;
        content["uri"] = json!(format!("{parent}/owned-{id}-{template}-{slot}.{suffix}"));
    }
    let payloads = contents.len();
    if has_children {
        contents.push(json!({"uri":format!("implicit-owned-{id}-{template}.json")}));
    }
    let levels = if has_children { 2 } else { 1 };
    let mut tree = Subtree::new(scheme, levels, contents.len())?;
    let zero = Coordinates {
        level: 0,
        x: 0,
        y: 0,
        z: 0,
    };
    let mut flags = vec![true; payloads];
    if has_children {
        flags.push(false);
    }
    tree.set_tile(zero, &flags)?;
    tree.set_metadata(
        zero,
        TileMetadata {
            bounding_box: bounds(original, [0.; 3], "point-cloud/vector")?,
            geometric_error: original["geometricError"].as_f64().unwrap(),
            extras: original.get("extras").cloned().unwrap_or_else(|| json!({})),
        },
    )?;
    for (index, child) in children.iter().enumerate() {
        let checked_child = &checked["children"][index];
        let slot = checked_child["_rustyImplicitChildIndex"].as_u64().unwrap() as u32;
        let coord = Coordinates {
            level: 1,
            x: slot & 1,
            y: (slot >> 1) & 1,
            z: if scheme == SubdivisionScheme::Octree {
                (slot >> 2) & 1
            } else {
                0
            },
        };
        let mut flags = vec![false; contents.len()];
        flags[payloads] = true;
        tree.set_tile(coord, &flags)?;
        tree.set_metadata(
            coord,
            TileMetadata {
                bounding_box: bounds(
                    child,
                    offset(child, "point-cloud/vector")?,
                    "point-cloud/vector",
                )?,
                geometric_error: child["geometricError"].as_f64().unwrap(),
                // Actual tile extras, payload statistics and metadata remain on its
                // external root, whose content is the original unmodified bytes.
                extras: json!({}),
            },
        )?;
        let child_document = owned_document(
            header,
            checked_child,
            child,
            directory,
            scheme,
            roots,
            retained,
        )?;
        let coordinate = if scheme == SubdivisionScheme::Octree {
            format!("1-{}-{}-{}", coord.x, coord.y, coord.z)
        } else {
            format!("1-{}-{}", coord.x, coord.y)
        };
        fs::write(
            directory.join(format!("implicit-owned-{id}-{coordinate}.json")),
            serde_json::to_vec(&child_document)?,
        )?;
    }
    fs::create_dir_all(directory.join("subtrees"))?;
    fs::write(
        directory.join(format!("subtrees/owned-{id}-{zero_key}.subtree")),
        tree.to_bytes()?,
    )?;
    let mut root = Value::Object(
        original
            .as_object()
            .unwrap()
            .iter()
            .filter(|(key, _)| key.as_str() != "children")
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect(),
    );
    let root_object = root.as_object_mut().unwrap();
    root_object.remove("children");
    root_object.remove("content");
    root_object.remove("contents");
    root["implicitTiling"] = json!({"subdivisionScheme":scheme.as_str(),"subtreeLevels":levels,"availableLevels":levels,"subtrees":{"uri":format!("subtrees/owned-{id}-{template}.subtree")}});
    if contents.len() == 1 {
        root["content"] = contents.remove(0);
    } else if !contents.is_empty() {
        root["contents"] = json!(contents);
    }
    let mut document = header.clone();
    document["root"] = root;
    if id > 0 {
        document["geometricError"] = original["geometricError"].clone();
        document["asset"].as_object_mut().unwrap().remove("extras");
    }
    let semantic_schema = serde_json::to_value(crate::metadata::tile_schema())?;
    if document.get("schema").is_none() {
        document["schema"] = semantic_schema.clone();
    }
    document["schema"]["classes"]["rustyTile"] = semantic_schema["classes"]["rustyTile"].clone();
    Ok(document)
}
