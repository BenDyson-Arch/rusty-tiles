//! Independent payload accounting, performed outside the measured CLI process.
use super::Result;
use serde_json::{json, Value};
use std::{
    collections::{BTreeSet, HashMap},
    fs::File,
    io::Read,
    path::Path,
};

fn member(zip: &mut zip::ZipArchive<File>, name: &str) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    zip.by_name(name)?.read_to_end(&mut bytes)?;
    Ok(bytes)
}
pub fn archive_json(path: &Path, name: &str) -> Result<Value> {
    let mut zip = zip::ZipArchive::new(File::open(path)?)?;
    Ok(serde_json::from_slice(&member(&mut zip, name)?)?)
}
fn leaf_contents(zip: &mut zip::ZipArchive<File>) -> Result<Vec<String>> {
    let doc: Value = serde_json::from_slice(&member(zip, "tileset.json")?)?;
    let expanded = rusty_tiles::implicit::expand_tileset(&doc, |name| {
        let mut bytes = Vec::new();
        zip.by_name(name)?.read_to_end(&mut bytes)?;
        Ok(bytes)
    })?;
    let mut pending = vec![&expanded["root"]];
    let mut names = Vec::new();
    while let Some(node) = pending.pop() {
        if let Some(children) = node["children"].as_array().filter(|c| !c.is_empty()) {
            pending.extend(children);
        } else {
            let contents = node["contents"]
                .as_array()
                .cloned()
                .unwrap_or_else(|| node.get("content").cloned().into_iter().collect());
            for content in contents {
                names.push(
                    content["uri"]
                        .as_str()
                        .ok_or("missing leaf URI")?
                        .to_owned(),
                );
            }
        }
    }
    if names.is_empty() {
        return Err("archive has no leaf content".into());
    }
    Ok(names)
}

type Triangle = [[u32; 3]; 3];
fn triangle(points: [[f32; 3]; 3]) -> Triangle {
    let key = points.map(|p| p.map(|v| if v == 0. { 0 } else { v.to_bits() }));
    (0..3)
        .map(|rotation| {
            let mut candidate = key;
            candidate.rotate_left(rotation);
            candidate
        })
        .min()
        .unwrap()
}
fn decompress_mesh(bytes: &[u8]) -> Result<Vec<u8>> {
    let glb = gltf::Glb::from_slice(bytes)?;
    let mut doc: Value = serde_json::from_slice(&glb.json)?;
    if !doc["bufferViews"].as_array().is_some_and(|views| {
        views
            .iter()
            .any(|v| v["extensions"].get("EXT_meshopt_compression").is_some())
    }) {
        return Ok(bytes.to_vec());
    }
    let raw = glb.bin.ok_or("missing compressed mesh buffer")?;
    let mut bin = Vec::new();
    for view in doc["bufferViews"]
        .as_array_mut()
        .ok_or("missing buffer views")?
    {
        let destination = bin.len();
        if let Some(ext) = view["extensions"].get("EXT_meshopt_compression") {
            let start = ext["byteOffset"].as_u64().unwrap_or(0) as usize;
            let size = ext["byteLength"].as_u64().ok_or("missing meshopt length")? as usize;
            let count = ext["count"].as_u64().ok_or("missing meshopt count")? as usize;
            let stride = ext["byteStride"].as_u64().ok_or("missing meshopt stride")? as usize;
            if ext["filter"].as_str().is_some_and(|f| f != "NONE") {
                return Err("audit does not support meshopt filters".into());
            }
            let input = raw
                .get(start..start + size)
                .ok_or("meshopt buffer out of bounds")?;
            let mut decoded = vec![0u32; (count * stride).div_ceil(4)];
            // SAFETY: destination is aligned and sized for count*stride bytes;
            // the decoder receives the bounds-checked compressed input slice.
            let code = unsafe {
                match ext["mode"].as_str() {
                    Some("TRIANGLES") => meshopt::ffi::meshopt_decodeIndexBuffer(
                        decoded.as_mut_ptr().cast(),
                        count,
                        stride,
                        input.as_ptr(),
                        input.len(),
                    ),
                    Some("ATTRIBUTES") => meshopt::ffi::meshopt_decodeVertexBuffer(
                        decoded.as_mut_ptr().cast(),
                        count,
                        stride,
                        input.as_ptr(),
                        input.len(),
                    ),
                    _ => return Err("unsupported meshopt mode".into()),
                }
            };
            if code != 0 {
                return Err("meshopt payload failed to decode".into());
            }
            bin.extend_from_slice(&bytemuck::cast_slice(&decoded)[..count * stride]);
        } else {
            let start = view["byteOffset"].as_u64().unwrap_or(0) as usize;
            let size = view["byteLength"].as_u64().ok_or("missing view size")? as usize;
            bin.extend_from_slice(
                raw.get(start..start + size)
                    .ok_or("mesh view out of bounds")?,
            );
        }
        view["buffer"] = json!(0);
        view["byteOffset"] = json!(destination);
        if let Some(extensions) = view.get_mut("extensions").and_then(Value::as_object_mut) {
            extensions.remove("EXT_meshopt_compression");
        }
    }
    doc["buffers"] = json!([{"byteLength":bin.len()}]);
    for field in ["extensionsUsed", "extensionsRequired"] {
        if let Some(exts) = doc[field].as_array_mut() {
            exts.retain(|e| e != "EXT_meshopt_compression");
        }
    }
    let mut output = Vec::new();
    gltf::Glb {
        header: gltf::binary::Header {
            magic: *b"glTF",
            version: 2,
            length: 0,
        },
        json: std::borrow::Cow::Owned(serde_json::to_vec(&doc)?),
        bin: Some(std::borrow::Cow::Owned(bin)),
    }
    .to_writer(&mut output)?;
    Ok(output)
}
fn mesh(input: &Path, output: &Path) -> Result<Value> {
    let source = rusty_tiles::mesh::load(input)?;
    let count = source.triangles.len();
    let mut remaining = HashMap::<Triangle, u32>::new();
    for tri in &source.triangles {
        *remaining
            .entry(triangle(tri.verts.map(|i| source.vertices[i as usize].pos)))
            .or_default() += 1;
    }
    drop(source);
    let mut zip = zip::ZipArchive::new(File::open(output)?)?;
    let leaves = leaf_contents(&mut zip)?;
    let work = tempfile::tempdir()?;
    let payload = work.path().join("leaf.glb");
    let mut actual = 0;
    for name in &leaves {
        std::fs::write(&payload, decompress_mesh(&member(&mut zip, name)?)?)?;
        let scene = rusty_tiles::mesh::load(&payload)?;
        for tri in &scene.triangles {
            let key = triangle(tri.verts.map(|i| scene.vertices[i as usize].pos));
            let total = remaining
                .get_mut(&key)
                .ok_or_else(|| format!("changed triangle or winding in {name}"))?;
            if *total == 0 {
                return Err(format!("duplicated triangle in {name}").into());
            }
            *total -= 1;
            actual += 1;
        }
    }
    if actual != count || remaining.values().any(|&n| n != 0) {
        return Err("missing source triangles".into());
    }
    Ok(
        json!({"sourceTriangles":count,"leafTriangles":actual,"leafContents":leaves.len(),
        "fidelity":"Every oriented float32 position triangle exactly once; textures, normals and UVs are covered by the small synthetic fidelity tests, not this corpus audit."}),
    )
}

enum Expected {
    Raw(usize, usize),
    Bits(usize, u8, u8),
    Index,
    Coordinate(usize),
    Scaled(usize, u8, f64, f64),
}
fn extra_width(kind: u8) -> Result<usize> {
    Ok(match kind {
        1 | 2 => 1,
        3 | 4 => 2,
        5 | 6 | 9 => 4,
        7 | 8 | 10 => 8,
        _ => return Err("unsupported Extra Bytes type in audit".into()),
    })
}
fn scalar(raw: &[u8], kind: u8) -> f64 {
    match kind {
        1 => raw[0] as f64,
        2 => (raw[0] as i8) as f64,
        3 => u16::from_le_bytes(raw.try_into().unwrap()) as f64,
        4 => i16::from_le_bytes(raw.try_into().unwrap()) as f64,
        5 => u32::from_le_bytes(raw.try_into().unwrap()) as f64,
        6 => i32::from_le_bytes(raw.try_into().unwrap()) as f64,
        7 => u64::from_le_bytes(raw.try_into().unwrap()) as f64,
        8 => i64::from_le_bytes(raw.try_into().unwrap()) as f64,
        9 => f32::from_le_bytes(raw.try_into().unwrap()) as f64,
        10 => f64::from_le_bytes(raw.try_into().unwrap()),
        _ => unreachable!(),
    }
}
fn point_columns(header: &las::Header) -> Result<HashMap<String, Expected>> {
    use Expected::*;
    let format = header.point_format().to_u8()? & 63;
    let extended = format >= 6;
    let mut columns = HashMap::new();
    columns.insert("source_index".into(), Index);
    for (i, name) in ["source_x", "source_y", "source_z"].into_iter().enumerate() {
        columns.insert(name.into(), Coordinate(i));
    }
    for (name, offset, width) in [
        ("X", 0, 4),
        ("Y", 4, 4),
        ("Z", 8, 4),
        ("intensity", 12, 2),
        ("user_data", 17, 1),
        ("point_source_id", if extended { 20 } else { 18 }, 2),
    ] {
        columns.insert(name.into(), Raw(offset, width));
    }
    for (name, offset, shift, mask) in [
        ("return_number", 14, 0, if extended { 15 } else { 7 }),
        (
            "number_of_returns",
            14,
            if extended { 4 } else { 3 },
            if extended { 15 } else { 7 },
        ),
        ("scan_direction_flag", if extended { 15 } else { 14 }, 6, 1),
        ("edge_of_flight_line", if extended { 15 } else { 14 }, 7, 1),
        ("synthetic", 15, if extended { 0 } else { 5 }, 1),
        ("key_point", 15, if extended { 1 } else { 6 }, 1),
        ("withheld", 15, if extended { 2 } else { 7 }, 1),
    ] {
        columns.insert(name.into(), Bits(offset, shift, mask));
    }
    if extended {
        columns.insert("classification".into(), Raw(16, 1));
        columns.insert("overlap".into(), Bits(15, 3, 1));
        columns.insert("scanner_channel".into(), Bits(15, 4, 3));
        columns.insert("scan_angle".into(), Raw(18, 2));
    } else {
        columns.insert("classification".into(), Bits(15, 0, 31));
        columns.insert("scan_angle_rank".into(), Raw(16, 1));
    }
    if extended || matches!(format, 1 | 3) {
        columns.insert("gps_time".into(), Raw(if extended { 22 } else { 20 }, 8));
    }
    let rgb = match format {
        2 => Some(20),
        3 => Some(28),
        7 | 8 => Some(30),
        _ => None,
    };
    if let Some(offset) = rgb {
        for (i, name) in ["red", "green", "blue"].into_iter().enumerate() {
            columns.insert(name.into(), Raw(offset + i * 2, 2));
        }
    }
    if format == 8 {
        columns.insert("nir".into(), Raw(36, 2));
    }
    let mut offset =
        header.point_format().len() as usize - header.point_format().extra_bytes as usize;
    for vlr in header
        .all_vlrs()
        .filter(|v| v.user_id == "LASF_Spec" && v.record_id == 4)
    {
        for descriptor in vlr.data.as_chunks::<192>().0 {
            let name = String::from_utf8(
                descriptor[4..36]
                    .iter()
                    .copied()
                    .take_while(|&b| b != 0)
                    .collect(),
            )?;
            let kind = descriptor[2];
            let width = extra_width(kind)?;
            let expected = if descriptor[3] & 24 == 0 {
                Raw(offset, width)
            } else {
                let scale = if descriptor[3] & 8 == 0 {
                    1.
                } else {
                    f64::from_le_bytes(descriptor[112..120].try_into()?)
                };
                let shift = if descriptor[3] & 16 == 0 {
                    0.
                } else {
                    f64::from_le_bytes(descriptor[136..144].try_into()?)
                };
                Scaled(offset, kind, scale, shift)
            };
            if columns.insert(name, expected).is_some() {
                return Err("duplicate source point column".into());
            }
            offset += width;
        }
    }
    if offset != header.point_format().len() as usize {
        return Err("undocumented point record tail".into());
    }
    Ok(columns)
}
fn view<'a>(doc: &Value, bin: &'a [u8], id: usize) -> Result<&'a [u8]> {
    let view = &doc["bufferViews"][id];
    let start = view["byteOffset"].as_u64().unwrap_or(0) as usize;
    let length = view["byteLength"]
        .as_u64()
        .ok_or("missing column byte length")? as usize;
    Ok(bin
        .get(start..start + length)
        .ok_or("column outside payload buffer")?)
}
fn points(input: &Path, output: &Path) -> Result<Value> {
    let reader = las::Reader::from_path(input)?;
    let header = reader.header().clone();
    if header.point_format().is_compressed {
        return Err("point audit requires the staged uncompressed LAS".into());
    }
    let count = usize::try_from(header.number_of_points())?;
    let raw_header = header.clone().into_raw()?;
    let file = File::open(input)?;
    // SAFETY: the staged input is immutable for the lifetime of this mapping.
    let raw = unsafe { memmap2::Mmap::map(&file)? };
    let stride = header.point_format().len() as usize;
    let start = raw_header.offset_to_point_data as usize;
    let records = raw
        .get(start..start + count * stride)
        .ok_or("truncated staged LAS")?;
    let expected = point_columns(&header)?;
    let transforms = [
        &header.transforms().x,
        &header.transforms().y,
        &header.transforms().z,
    ];
    let mut seen = vec![false; count];
    let mut total = 0;
    let mut zip = zip::ZipArchive::new(File::open(output)?)?;
    let leaves = leaf_contents(&mut zip)?;
    for name in &leaves {
        let bytes = member(&mut zip, name)?;
        let glb = gltf::Glb::from_slice(&bytes)?;
        let doc: Value = serde_json::from_slice(&glb.json)?;
        let bin = glb.bin.ok_or("missing point payload buffer")?;
        let metadata = &doc["extensions"]["EXT_structural_metadata"];
        let table = &metadata["propertyTables"][0];
        let properties = table["properties"]
            .as_object()
            .ok_or("missing point metadata")?;
        if properties.keys().collect::<BTreeSet<_>>() != expected.keys().collect::<BTreeSet<_>>() {
            return Err(format!("point metadata schema differs from LAS in {name}").into());
        }
        let ids = view(
            &doc,
            &bin,
            properties["source_index"]["values"]
                .as_u64()
                .ok_or("missing source indices")? as usize,
        )?;
        let rows = table["count"].as_u64().ok_or("missing point count")? as usize;
        if ids.len() != rows * 8 {
            return Err("bad point index column length".into());
        }
        let mut indices = Vec::with_capacity(rows);
        for value in ids.as_chunks::<8>().0 {
            let id = usize::try_from(u64::from_le_bytes(*value))?;
            if id >= count || std::mem::replace(&mut seen[id], true) {
                return Err("duplicated or invalid source point index".into());
            }
            indices.push(id);
            total += 1;
        }
        for (column, recipe) in &expected {
            let values = view(
                &doc,
                &bin,
                properties[column]["values"]
                    .as_u64()
                    .ok_or("missing column buffer")? as usize,
            )?;
            let width = match recipe {
                Expected::Raw(_, width) => *width,
                Expected::Bits(..) => 1,
                _ => 8,
            };
            if values.len() != rows * width {
                return Err(format!("bad point column size: {column}").into());
            }
            for (row, &id) in indices.iter().enumerate() {
                let record = &records[id * stride..(id + 1) * stride];
                let mut value = [0u8; 8];
                let expected_bytes = match *recipe {
                    Expected::Raw(at, width) => &record[at..at + width],
                    Expected::Bits(at, shift, mask) => {
                        value[0] = (record[at] >> shift) & mask;
                        &value[..1]
                    }
                    Expected::Index => {
                        value = (id as u64).to_le_bytes();
                        &value
                    }
                    Expected::Coordinate(axis) => {
                        value = transforms[axis]
                            .direct(i32::from_le_bytes(
                                record[axis * 4..axis * 4 + 4].try_into()?,
                            ))
                            .to_le_bytes();
                        &value
                    }
                    Expected::Scaled(at, kind, scale, offset) => {
                        value = (scalar(&record[at..at + extra_width(kind)?], kind) * scale
                            + offset)
                            .to_le_bytes();
                        &value
                    }
                };
                if values[row * width..(row + 1) * width] != *expected_bytes {
                    return Err(format!("changed point {id} column {column} in {name}").into());
                }
            }
        }
    }
    if total != count || seen.iter().any(|&v| !v) {
        return Err("missing source points".into());
    }
    let columns: BTreeSet<_> = expected.keys().collect();
    Ok(
        json!({"sourcePoints":count,"leafPoints":total,"leafContents":leaves.len(),"auditedColumns":columns,
        "fidelity":"Every staged LAS record exactly once, with every declared scalar metadata column byte-exact; world placement is validated separately by the archive validator."}),
    )
}

fn raster(output: &Path) -> Result<Value> {
    let manifest: Value = serde_json::from_reader(File::open(output.join("tilejson.json"))?)?;
    if manifest["scheme"] != "xyz" {
        return Err("imagery is not XYZ".into());
    }
    let bounds: Vec<f64> = manifest["bounds"]
        .as_array()
        .ok_or("missing imagery bounds")?
        .iter()
        .map(|v| v.as_f64().ok_or("invalid imagery bound"))
        .collect::<std::result::Result<_, _>>()?;
    if bounds.len() != 4 || !bounds.iter().all(|v| v.is_finite()) {
        return Err("invalid imagery bounds".into());
    }
    let minimum = manifest["minzoom"]
        .as_u64()
        .ok_or("missing imagery minzoom")?;
    let maximum = manifest["maxzoom"]
        .as_u64()
        .ok_or("missing imagery maxzoom")?;
    let mut expected = BTreeSet::new();
    for z in minimum..=maximum {
        let n = 2f64.powi(z as i32);
        let x = |longitude: f64| ((longitude + 180.) / 360. * n).floor().clamp(0., n - 1.) as u64;
        let y = |latitude: f64| {
            ((1. - latitude.to_radians().tan().asinh() / std::f64::consts::PI) * n / 2.)
                .floor()
                .clamp(0., n - 1.) as u64
        };
        for x in x(bounds[0])..=x(bounds[2]) {
            for y in y(bounds[3])..=y(bounds[1]) {
                expected.insert(format!("tiles/{z}/{x}/{y}.png"));
            }
        }
    }
    let count = expected.len();
    let mut actual = BTreeSet::new();
    for entry in walkdir::WalkDir::new(output.join("tiles")) {
        let entry = entry?;
        if entry.file_type().is_file() && entry.path().extension().is_some_and(|e| e == "png") {
            let image = image::open(entry.path())?;
            if image.width() != 256 || image.height() != 256 {
                return Err("bad imagery tile dimensions".into());
            }
            actual.insert(
                entry
                    .path()
                    .strip_prefix(output)?
                    .to_string_lossy()
                    .replace('\\', "/"),
            );
        }
    }
    if count == 0 || expected != actual {
        return Err("imagery tile coverage differs from geographic XYZ bounds".into());
    }
    Ok(json!({"decodedPngTiles":count,"tileDimensions":[256,256],"scheme":"xyz"}))
}
fn u32_at(bytes: &[u8], at: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(
        bytes
            .get(at..at + 4)
            .ok_or("truncated terrain")?
            .try_into()?,
    ))
}
fn terrain(output: &Path) -> Result<Value> {
    let manifest: Value = serde_json::from_reader(File::open(output.join("layer.json"))?)?;
    let report: Value = serde_json::from_reader(File::open(output.join("conversion.json"))?)?;
    if manifest["scheme"] != "tms" || manifest["heightOverlay"]["rowOrder"] != "south-to-north" {
        return Err("bad terrain orientation".into());
    }
    let mut count = 0;
    let mut triangles = 0u64;
    let mut vertices = 0u64;
    for entry in walkdir::WalkDir::new(output) {
        let entry = entry?;
        if !entry.file_type().is_file() || entry.path().extension().is_none_or(|e| e != "terrain") {
            continue;
        }
        let bytes = std::fs::read(entry.path())?;
        let low = f32::from_bits(u32_at(&bytes, 24)?);
        let high = f32::from_bits(u32_at(&bytes, 28)?);
        if !low.is_finite() || !high.is_finite() || low > high {
            return Err("invalid terrain height range".into());
        }
        let n = u32_at(&bytes, 88)? as usize;
        if n == 0 || n > 65536 {
            return Err("unsupported terrain vertex count".into());
        }
        let attributes = bytes
            .get(92..92 + n * 6)
            .ok_or("truncated terrain vertices")?;
        for values in attributes.chunks_exact(n * 2) {
            let mut previous = 0i32;
            for word in values.as_chunks::<2>().0 {
                let v = u16::from_le_bytes(*word);
                previous += i32::from(v >> 1) ^ -i32::from(v & 1);
                if !(0..=32767).contains(&previous) {
                    return Err("invalid quantized terrain coordinate".into());
                }
            }
        }
        let nt = u32_at(&bytes, 92 + n * 6)? as usize;
        if nt == 0 {
            return Err("empty terrain mesh".into());
        }
        let mut at = 96 + n * 6;
        let indices = bytes
            .get(at..at + nt * 6)
            .ok_or("truncated terrain triangles")?;
        let mut high_water = 0usize;
        for word in indices.as_chunks::<2>().0 {
            let code = u16::from_le_bytes(*word) as usize;
            let index = high_water
                .checked_sub(code)
                .ok_or("bad terrain high water index")?;
            if index >= n {
                return Err("terrain index out of bounds".into());
            }
            if code == 0 {
                high_water += 1;
            }
        }
        at += nt * 6;
        for _ in 0..4 {
            let edge_count = u32_at(&bytes, at)? as usize;
            at += 4;
            if edge_count == 0 {
                return Err("empty terrain edge".into());
            }
            let edge = bytes
                .get(at..at + edge_count * 2)
                .ok_or("truncated terrain edge")?;
            if edge
                .as_chunks::<2>()
                .0
                .iter()
                .any(|v| u16::from_le_bytes(*v) as usize >= n)
            {
                return Err("terrain edge out of bounds".into());
            }
            at += edge_count * 2;
        }
        if at != bytes.len() {
            return Err("unexpected terrain payload tail".into());
        }
        let overlay: Value =
            serde_json::from_reader(File::open(entry.path().with_extension("heights.json"))?)?;
        let grid = manifest["heightOverlay"]["grid"]
            .as_u64()
            .ok_or("missing height overlay grid")?;
        let heights = overlay["heights"]
            .as_array()
            .ok_or("missing height overlay")?;
        if overlay["width"] != grid
            || overlay["height"] != grid
            || heights.len() as u64 != grid * grid
            || heights
                .iter()
                .any(|v| !v.is_null() && v.as_f64().is_none_or(|h| !h.is_finite()))
        {
            return Err("invalid terrain height overlay".into());
        }
        count += 1;
        vertices += n as u64;
        triangles += nt as u64;
    }
    if count == 0 || report["tiles"].as_u64() != Some(count) {
        return Err("terrain tile count differs from report".into());
    }
    Ok(
        json!({"decodedTerrainTiles":count,"vertices":vertices,"triangles":triangles,"heightOverlays":count,"scheme":"tms",
        "fidelity":"Independent binary attribute, triangle/edge index and finite height-overlay checks; these do not claim vertical datum conversion or source resampling equivalence."}),
    )
}
pub fn check(kind: &str, input: &Path, output: &Path) -> Result<Value> {
    if matches!(kind, "archive" | "mesh" | "points") {
        let validation = rusty_tiles::validate::archive(output, None)?;
        let mut result = match kind {
            "mesh" => mesh(input, output)?,
            "points" => points(input, output)?,
            _ => json!({}),
        };
        if kind == "archive" && input.is_dir() {
            let mut zip = zip::ZipArchive::new(File::open(output)?)?;
            let mut count = 0;
            for entry in walkdir::WalkDir::new(input) {
                let entry = entry?;
                if !entry.file_type().is_file() {
                    continue;
                }
                let name = entry
                    .path()
                    .strip_prefix(input)?
                    .to_string_lossy()
                    .replace('\\', "/");
                let original = std::fs::read(entry.path())?;
                if original != member(&mut zip, &name)? {
                    return Err(format!("changed packed reference resource: {name}").into());
                }
                count += 1;
            }
            result["unchangedReferenceMembers"] = json!(count);
        }
        result["archiveValidation"] = validation;
        Ok(result)
    } else {
        match kind {
            "raster" => raster(output),
            "terrain" => terrain(output),
            _ => Err("unknown audit".into()),
        }
    }
}
