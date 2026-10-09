//! Resolve only declared local resources, keeping glTF and resource bytes intact.
use std::{collections::BTreeMap, fs::File, path::PathBuf};

use serde_json::Value;

use crate::Error;

pub(super) fn members(input: &std::path::Path) -> Result<Vec<(String, PathBuf)>, Error> {
    resolve(input, true)
}

/// Declared dependencies only: the implicit writer stages and renames the model.
/// A declared reference to the source document still belongs in this list.
pub(super) fn dependencies(input: &std::path::Path) -> Result<Vec<(String, PathBuf)>, Error> {
    resolve(input, false)
}

fn resolve(
    input: &std::path::Path,
    include_document: bool,
) -> Result<Vec<(String, PathBuf)>, Error> {
    let file = File::open(input)?;
    // SAFETY: Read-only mapping used only while parsing. Source files must remain
    // unchanged during conversion, as they must during archive publication.
    let bytes = unsafe { memmap2::Mmap::map(&file)? };
    // Borrow the JSON chunk without copying a potentially very large GLB BIN.
    let glb;
    let json = if input
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("glb"))
    {
        glb = gltf::binary::Glb::from_slice(&bytes)?;
        glb.json.as_ref()
    } else {
        &bytes
    };
    let document: Value = serde_json::from_slice(json)?;
    let parent = input.parent().filter(|p| !p.as_os_str().is_empty());
    let root = parent.unwrap_or(std::path::Path::new(".")).canonicalize()?;
    let input_name = super::file_name(input)?;
    local_name(&input_name)?;
    let mut files = BTreeMap::new();
    if include_document {
        files.insert(input_name, input.to_path_buf());
    }
    let mut uris = Vec::new();
    for key in ["buffers", "images"] {
        for item in document[key].as_array().into_iter().flatten() {
            if let Some(uri) = item.get("uri") {
                uris.push((uri, true));
            }
        }
    }
    if let Some(uri) = document["extensions"]["EXT_structural_metadata"].get("schemaUri") {
        uris.push((uri, false));
    }
    for (uri, allow_embedded) in uris {
        let uri = uri
            .as_str()
            .ok_or_else(|| Error::Data("glTF resource URI must be a string".into()))?;
        if allow_embedded && uri.starts_with("data:") {
            continue;
        }
        let name = local_name(uri)?;
        if name == "tileset.json" || name == crate::pack::TZ_INDEX_NAME {
            return Err(Error::Data(format!("reserved glTF resource path: {uri}")));
        }
        let path = root
            .join(&name)
            .canonicalize()
            .map_err(|err| Error::Data(format!("cannot read glTF resource {uri}: {err}")))?;
        if !path.starts_with(&root) || !path.is_file() {
            return Err(Error::Data(format!(
                "glTF resource must be a file inside the input directory: {uri}"
            )));
        }
        files.entry(name).or_insert(path);
    }
    Ok(files.into_iter().collect())
}

// Match the archive validator's local URI rules. Keep the document untouched:
// relative dot segments resolve to these same canonical archive member names.
fn local_name(uri: &str) -> Result<String, Error> {
    if uri.is_empty() || uri.contains(['\\', ':', '?', '#', '%']) || uri.starts_with('/') {
        return Err(Error::Data(format!(
            "non-local or unsupported glTF resource URI: {uri}"
        )));
    }
    let mut parts = Vec::new();
    for part in uri.split('/') {
        match part {
            "" | "." => (),
            ".." => {
                if parts.pop().is_none() {
                    return Err(Error::Data(format!(
                        "glTF resource URI escapes the input directory: {uri}"
                    )));
                }
            }
            other => parts.push(other),
        }
    }
    if parts.is_empty() {
        return Err(Error::Data(format!("invalid glTF resource URI: {uri}")));
    }
    Ok(parts.join("/"))
}
