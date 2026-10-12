//! Versioned native encoder identity, immutable content and subtree invalidation.
use super::*;
use crate::implicit::{ForestOutput, RebaseRecord};
use std::fs::File;
use store::{Counters, Node};

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Cut {
    pub axis: String,
    pub value: f64,
    pub key: String,
    pub id: i64,
}
#[derive(Clone, Serialize, Deserialize)]
struct Record {
    signature: String,
    center: Point,
    padding: f64,
    fragments: usize,
    cut: Option<Cut>,
}
/// How to recover one raw payload that the archive publishes only rebased under
/// `implicit-content/` (design §4.7). Keyed by the raw sha256 in `vector-build.json`.
/// Payloads that explicit tiles publish in place (`t/<sha256>`) have no entry.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Payload {
    member: String,
    delta: [f64; 3],
    root_node: String,
    json_length: u32,
}
pub(super) struct Reuse {
    output: std::path::PathBuf,
    previous: Option<std::path::PathBuf>,
    requested: bool,
    incompatible: Option<String>,
    old: BTreeMap<String, Record>,
    nodes: BTreeMap<String, Value>,
    records: BTreeMap<String, Record>,
    pub cuts: BTreeMap<String, Cut>,
    pub frame: Option<Frame>,
    config: Value,
    old_config: Value,
    old_payloads: Value,
    payloads: BTreeMap<String, Payload>,
    configuration_hash: String,
    reused_subtrees: usize,
    reused_tiles: usize,
    reused_references: usize,
    reused_uris: BTreeSet<String>,
}
#[cfg(feature = "native-geospatial")]
const ENCODER_PREFIX: &str = "rusty-tiles-native-vector-v1:";
#[cfg(not(feature = "native-geospatial"))]
const ENCODER_PREFIX: &str = "rusty-tiles-portable-vector-v1:";

fn encoder() -> String {
    encoder_with_framing(include_str!("../../vector_encoding/framing.rs"))
}
fn encoder_with_framing(framing_source: &str) -> String {
    // Intrinsic source-chart triangulation changes native explicit content too;
    // every hierarchy uses the reviewed source/dependency fingerprint.
    let mut hasher = Sha256::new();
    for source in [
        include_str!("../pipeline.rs"),
        include_str!("../model.rs"),
        include_str!("../source_fields.rs"),
        include_str!("source.rs"),
        include_str!("acceptance.rs"),
        include_str!("../portable.rs"),
        include_str!("../../crs.rs"),
        include_str!("geometry.rs"),
        include_str!("encoding.rs"),
        include_str!("aggregation.rs"),
        include_str!("../../point_sampling.rs"),
        include_str!("store.rs"),
        include_str!("reuse.rs"),
        include_str!("../../vector_encoding.rs"),
        include_str!("../../vector_encoding/limits.rs"),
        include_str!("../../vector_encoding/raw.rs"),
        include_str!("../../vector_encoding/plan.rs"),
        include_str!("../../vector_encoding/rewrite.rs"),
        framing_source,
        include_str!("../../content_integrity.rs"),
        include_str!("../../content_integrity/json.rs"),
        include_str!("../../content_integrity/numbers.rs"),
        include_str!("../../content_integrity/meshopt.rs"),
        include_str!("../../vector_compression.rs"),
        include_str!("../../metadata.rs"),
        include_str!("../../glb_write.rs"),
        include_str!("../../glb.rs"),
        include_str!("../../bbox.rs"),
        include_str!("../../georef.rs"),
        include_str!("../../implicit.rs"),
        include_str!("../../implicit/tileset.rs"),
        include_str!("../../../Cargo.lock"),
    ] {
        hasher.update(source.as_bytes());
        hasher.update([0]);
    }
    #[cfg(feature = "native-geospatial")]
    for source in [
        include_str!("source_native.rs"),
        include_str!("geometry/native.rs"),
        include_str!("../../geospatial.rs"),
    ] {
        hasher.update(source.as_bytes());
        hasher.update([0]);
    }
    #[cfg(not(feature = "native-geospatial"))]
    {
        hasher.update(include_str!("geometry/portable.rs").as_bytes());
        hasher.update([0]);
    }
    format!("{ENCODER_PREFIX}{:x}", hasher.finalize())
}

/// Fragmented polygon fills were b3dm-wrapped before glTF-only fills. Their
/// payloads cannot be reused; the archive must be rebuilt.
const LEGACY_FILL: &str = "previous archive has legacy b3dm polygon fills; fills are now glTF \
     vector content. Run a fresh conversion without --reuse-tileset";

fn has_legacy_fill(node: &Value) -> bool {
    contents(node).iter().any(|content| {
        content["uri"]
            .as_str()
            .is_some_and(|uri| uri.to_ascii_lowercase().ends_with(".b3dm"))
    }) || node["children"]
        .as_array()
        .is_some_and(|children| children.iter().any(has_legacy_fill))
}

pub(super) fn contents(node: &Value) -> Vec<&Value> {
    if let Some(contents) = node["contents"].as_array() {
        contents.iter().collect()
    } else {
        node.get("content").into_iter().collect()
    }
}
impl Reuse {
    pub fn new(output: &Path, options: &VectorOptions) -> Result<Self, Error> {
        let mut result = Self {
            output: output.into(),
            previous: options.reuse_tileset.clone(),
            requested: options.reuse_tileset.is_some(),
            incompatible: None,
            old: BTreeMap::new(),
            nodes: BTreeMap::new(),
            records: BTreeMap::new(),
            cuts: BTreeMap::new(),
            frame: None,
            config: Value::Null,
            old_config: Value::Null,
            old_payloads: Value::Null,
            payloads: BTreeMap::new(),
            configuration_hash: String::new(),
            reused_subtrees: 0,
            reused_tiles: 0,
            reused_references: 0,
            reused_uris: BTreeSet::new(),
        };
        if let Some(previous) = &result.previous {
            let mut archive = zip::ZipArchive::new(File::open(previous)?)?;
            let limit = 16777216u64.max(options.max_tiles.saturating_mul(4096) as u64);
            let mut read = |name: &str| -> Result<Vec<u8>, Error> {
                let mut file = archive.by_name(name)?;
                if file.size() > limit {
                    return Err(data(
                        "previous build metadata exceeds configured hierarchy limit",
                    ));
                }
                let mut bytes = Vec::new();
                std::io::Read::read_to_end(&mut file, &mut bytes)?;
                Ok(bytes)
            };
            let mut manifest: Value = serde_json::from_slice(&read("tileset.json")?)?;
            let raw = read("vector-build.json")?;
            let state: Value = serde_json::from_slice(&raw)?;
            // Python's encoder has different content/canonicalization identities.
            // Reject it explicitly before interpreting native integrity hashes.
            if !state["config"]["encoder"]
                .as_str()
                .is_some_and(|s| s.starts_with(ENCODER_PREFIX))
            {
                #[cfg(feature = "native-geospatial")]
                let message =
                    "previous encoder differs; run a fresh native conversion without reuseTileset";
                #[cfg(not(feature = "native-geospatial"))]
                let message = "previous encoder differs; run a fresh portable conversion without reuseTileset";
                return Err(data(message));
            }
            if state["version"] != 1
                || state["records"]
                    .as_object()
                    .is_none_or(|r| r.len() > options.max_tiles)
            {
                return Err(data(
                    "unsupported previous vector build state; run a fresh conversion",
                ));
            }
            let expected = manifest["asset"]["extras"]
                .as_object_mut()
                .and_then(|o| o.remove("vectorBuildStateSha256"));
            if manifest["asset"]["extras"]
                .as_object()
                .is_some_and(|o| o.is_empty())
            {
                manifest["asset"].as_object_mut().unwrap().remove("extras");
            }
            if expected != Some(json!(hash(&raw)))
                || state["manifestSha256"] != json!(digest(&manifest)?)
            {
                return Err(data("previous manifest/build state integrity check failed"));
            }
            if state["config"]["where"] != json!(options.where_clause) {
                result.incompatible = Some("attribute filter changed".into());
                result.previous = None;
                return Ok(result);
            }
            let frame = Frame {
                anchor: serde_json::from_value(state["anchor"].clone())?,
                axes: serde_json::from_value(state["frame"].clone())?,
            };
            if !frame
                .anchor
                .iter()
                .chain(frame.axes.iter().flatten())
                .all(|v| v.is_finite())
                || (0..3).any(|i| {
                    (0..3).any(|j| {
                        (dot(frame.axes[i], frame.axes[j]) - if i == j { 1. } else { 0. }).abs()
                            > 1e-12
                    })
                })
            {
                return Err(data("invalid previous local frame"));
            }
            fn index(node: &Value, nodes: &mut BTreeMap<String, Value>) -> Result<(), Error> {
                if let Some(path) = node["extras"]["buildPath"].as_str() {
                    if nodes.insert(path.into(), node.clone()).is_some() {
                        return Err(data("duplicate cached subtree path"));
                    }
                }
                if let Some(children) = node["children"].as_array() {
                    for child in children {
                        index(child, nodes)?;
                    }
                }
                Ok(())
            }
            let previous_root = state.get("explicitRoot").unwrap_or(&manifest["root"]);
            if has_legacy_fill(previous_root) {
                return Err(data(LEGACY_FILL));
            }
            index(previous_root, &mut result.nodes)?;
            result.old = serde_json::from_value(state["records"].clone())?;
            result.cuts = result
                .old
                .iter()
                .filter_map(|(p, r)| r.cut.clone().map(|cut| (p.clone(), cut)))
                .collect();
            result.frame = Some(frame);
            result.old_config = state["config"].clone();
            // Interpreted only after `configure` has matched the encoder identity:
            // archives from another encoder are refused there with "run a fresh
            // conversion", before their (absent or different) payload map matters.
            result.old_payloads = state.get("payloads").cloned().unwrap_or(Value::Null);
        }
        Ok(result)
    }
    pub fn has_previous(&self) -> bool {
        self.previous.is_some()
    }
    pub fn configure(
        &mut self,
        max_features: usize,
        repair: bool,
        ambiguous: bool,
        options: &VectorOptions,
        reader: &source::Reader,
    ) -> Result<(), Error> {
        #[cfg(feature = "native-geospatial")]
        let versions = crate::geospatial::versions()?;
        #[cfg(not(feature = "native-geospatial"))]
        // The encoder fingerprint covers the locked Rust dependencies. Portable
        // reuse never records or queries installed GDAL, GEOS or PROJ versions.
        let versions = json!({"backend":"portable", "rustyTiles":env!("CARGO_PKG_VERSION")});
        let layers: Vec<_> = reader
            .layer_reports()
            .iter()
            .map(|layer| {
                let mut layer = layer.clone();
                for key in ["features", "invalidFeatures", "featuresWithoutGeometry"] {
                    layer.as_object_mut().unwrap().remove(key);
                }
                layer
            })
            .collect();
        self.config = json!({"encoder":encoder(),"versions":versions,"driver":reader.driver(),"schemas":reader.schemas(),"layers":layers,
            "quantize":options.quantize,"meshopt":options.meshopt,"maxFeatures":max_features,"maxParentFeatures":options.max_parent_features,"maxVertices":options.max_vertices,"maxBytes":options.max_bytes,
            "lodTolerance":options.lod.tolerance_metres,"lodLevels":options.lod.levels,"parentRepair":options.parent_repair,"aggregatePoints":options.aggregate_points,"skipInvalid":options.skip_invalid,"repair":repair,"ambiguousOutlines":ambiguous,
            "sourceCrs":options.source_crs,"where":options.where_clause,"heightOffset":options.height_offset,"listFields":options.list_fields,"fields":options.fields,"dropFields":options.drop_fields});
        if !options.explicit {
            self.config["tiling"] = json!("implicit-forest-v3");
        }
        // Accepted schema/list inventory is derived from this source revision.
        // It belongs in cache signatures, but changes must rebuild content rather
        // than refusing deletions or an empty accepted inventory.
        fn settings(config: &Value) -> Value {
            let mut settings = config.clone();
            settings.as_object_mut().unwrap().remove("schemas");
            if let Some(layers) = settings["layers"].as_array_mut() {
                for layer in layers {
                    layer.as_object_mut().unwrap().remove("jsonFields");
                }
            }
            settings
        }
        if self.previous.is_some() && settings(&self.config) != settings(&self.old_config) {
            return Err(data("previous encoder, CRS or conversion settings differ; run a fresh conversion without reuseTileset"));
        }
        self.configuration_hash = digest(&self.config)?;
        if self.previous.is_some() {
            self.payloads = previous_payloads(std::mem::take(&mut self.old_payloads))?;
        }
        Ok(())
    }
    pub fn signature(&self, db: &rusqlite::Connection, path: &str) -> Result<String, Error> {
        let mut hasher = Sha256::new();
        hasher.update(self.configuration_hash.as_bytes());
        let mut stmt = db
            .prepare(
                "SELECT fingerprint FROM features WHERE path>=?1 AND path<?2 ORDER BY fingerprint",
            )
            .map_err(sql)?;
        let values = stmt
            .query_map(rusqlite::params![path, format!("{path}~")], |r| {
                r.get::<_, String>(0)
            })
            .map_err(sql)?;
        for value in values {
            let value = value.map_err(sql)?;
            if value.len() != 64 {
                return Err(data("invalid feature fingerprint"));
            }
            let bytes = (0..32)
                .map(|i| {
                    u8::from_str_radix(&value[i * 2..i * 2 + 2], 16)
                        .map_err(|_| data("invalid feature fingerprint"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            hasher.update(bytes);
        }
        Ok(format!("{:x}", hasher.finalize()))
    }
    pub fn restore(
        &mut self,
        path: &str,
        signature: &str,
        counters: &mut Counters,
        current_fragments: usize,
        max_tiles: usize,
    ) -> Result<Option<Node>, Error> {
        let Some(record) = self
            .old
            .get(path)
            .filter(|r| r.signature == signature)
            .cloned()
        else {
            return Ok(None);
        };
        let mut value = self
            .nodes
            .get(path)
            .cloned()
            .ok_or_else(|| data("cached subtree is absent from previous manifest"))?;
        value.as_object_mut().unwrap().remove("transform");
        let mut archive = zip::ZipArchive::new(File::open(self.previous.as_ref().unwrap())?)?;
        fn visit(
            node: &Value,
            reuse: &mut Reuse,
            archive: &mut zip::ZipArchive<File>,
            counters: &mut Counters,
            max_tiles: usize,
        ) -> Result<(), Error> {
            counters.tiles += 1;
            if counters.tiles > max_tiles {
                return Err(data("reused hierarchy exceeds maxTiles"));
            }
            reuse.reused_tiles += 1;
            let content = contents(node);
            if !node["children"].as_array().is_some_and(|c| !c.is_empty()) && !content.is_empty() {
                counters.leaf_tiles += 1;
            }
            if node["extras"]["routing"] == true {
                counters.routing_tiles += 1;
            }
            for content in content {
                let uri = content["uri"]
                    .as_str()
                    .ok_or_else(|| data("invalid previous content URI"))?;
                let name = uri.strip_prefix("t/").ok_or_else(|| {
                    data("previous content URI is not an immutable vector content path")
                })?;
                let (stem, suffix) = name
                    .rsplit_once('.')
                    .ok_or_else(|| data("invalid previous content path"))?;
                if stem.len() != 64
                    || !stem
                        .bytes()
                        .all(|v| v.is_ascii_digit() || (b'a'..=b'f').contains(&v))
                    || suffix != "glb"
                {
                    return Err(data(
                        "previous content URI is not an immutable vector content path",
                    ));
                }
                let bytes = previous_payload(archive, reuse.payloads.get(stem), uri, stem, suffix)?;
                // Staged privately in the build workspace under its raw name; this
                // build's writer rebases or publishes it per the new layout.
                let target = reuse.output.join(uri);
                if !target.exists() {
                    std::fs::write(target, bytes)?;
                }
                reuse.reused_references += 1;
                reuse.reused_uris.insert(uri.into());
            }
            if !contents(node).is_empty() {
                counters.maximum_tile_vertices = counters
                    .maximum_tile_vertices
                    .max(node["extras"]["vertices"].as_u64().unwrap_or(0) as usize);
                counters.maximum_tile_bytes = counters
                    .maximum_tile_bytes
                    .max(node["extras"]["encodedBytes"].as_u64().unwrap_or(0) as usize);
            }
            if let Some(children) = node["children"].as_array() {
                for child in children {
                    visit(child, reuse, archive, counters, max_tiles)?;
                }
            }
            Ok(())
        }
        visit(&value, self, &mut archive, counters, max_tiles)?;
        for (key, record) in self.old.range(path.to_string()..format!("{path}~")) {
            self.records.insert(key.clone(), record.clone());
        }
        counters.fragments = counters.fragments + record.fragments - current_fragments;
        self.reused_subtrees += 1;
        Ok(Some(Node {
            value,
            center: record.center,
            padding: record.padding,
        }))
    }
    pub fn remember(&mut self, path: &str, signature: String, node: &mut Node, fragments: usize) {
        node.value["extras"]["buildPath"] = json!(path);
        self.records.insert(
            path.into(),
            Record {
                signature,
                center: node.center,
                padding: node.padding,
                fragments,
                cut: self.cuts.get(path).cloned(),
            },
        );
    }
    /// Record build state and publish only referenced raw payloads. `forest` is the
    /// producer's explicit hierarchy and the writer's output for implicit archives.
    pub fn publish(
        &mut self,
        manifest: &mut Value,
        frame: &Frame,
        forest: Option<(&Value, &ForestOutput)>,
    ) -> Result<Value, Error> {
        fn used(node: &Value, result: &mut BTreeSet<String>) {
            for content in contents(node) {
                if let Some(uri) = content["uri"].as_str() {
                    result.insert(uri.into());
                }
            }
            if let Some(children) = node["children"].as_array() {
                for child in children {
                    used(child, result);
                }
            }
        }
        let mut contents = BTreeSet::new();
        used(
            forest.map_or(&manifest["root"], |(root, _)| root),
            &mut contents,
        );
        // Raw payloads stay published only where a tile names them in place: every
        // content of an explicit archive, a forest's explicit-tile payloads. The
        // forest writer has already removed the raw sources it rebased.
        let published: BTreeSet<&str> = match forest {
            Some((_, output)) => output.members.iter().map(String::as_str).collect(),
            None => contents.iter().map(String::as_str).collect(),
        };
        let raw = self.output.join("t");
        if raw.is_dir() {
            for entry in std::fs::read_dir(raw)? {
                let entry = entry?;
                let name = format!("t/{}", entry.file_name().to_string_lossy());
                if !published.contains(name.as_str()) {
                    std::fs::remove_file(entry.path())?;
                }
            }
        }
        let mut state = json!({"version":1,"config":self.config,"anchor":frame.anchor,"frame":frame.axes,"records":self.records,
            "payloads":payload_map(forest.map(|(_, output)| output)),"manifestSha256":digest(manifest)?});
        if let Some((root, _)) = forest {
            state["explicitRoot"] = root.clone();
        }
        let raw = canonical(&state)?;
        std::fs::write(self.output.join("vector-build.json"), &raw)?;
        manifest["asset"]["extras"] = json!({"vectorBuildStateSha256":hash(&raw)});
        Ok(
            json!({"previousTileset":self.previous.is_some(),"requestedPreviousTileset":self.requested,"incompatibleReason":self.incompatible,
            "reusedSubtrees":self.reused_subtrees,"reusedTiles":self.reused_tiles,"reusedContents":self.reused_uris.len(),"reusedContentReferences":self.reused_references,
            "publishedContents":contents.len(),"rebuiltContents":contents.difference(&self.reused_uris).count()}),
        )
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Option<[u8; 32]> {
    let mut result = [0; 32];
    if text.len() != 64
        || !text
            .bytes()
            .all(|v| v.is_ascii_digit() || (b'a'..=b'f').contains(&v))
    {
        return None;
    }
    for (i, byte) in result.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(result)
}

/// One entry per distinct raw payload the forest publishes only rebased, first in
/// tile preorder. Shared raw contents need one location; any one restores exactly.
fn payload_map(forest: Option<&ForestOutput>) -> BTreeMap<String, Payload> {
    let mut map = BTreeMap::new();
    for content in forest
        .into_iter()
        .flat_map(|output| &output.tiles)
        .flat_map(|tile| &tile.contents)
    {
        if let Some(record) = &content.rebase {
            map.entry(hex(&record.raw_sha256))
                .or_insert_with(|| Payload {
                    member: content.member.clone(),
                    delta: record.delta,
                    root_node: record.raw_root_node.clone(),
                    json_length: record.raw_json_length,
                });
        }
    }
    map
}

/// Validate the shape of the recorded payload map once, at the reuse boundary.
fn previous_payloads(value: Value) -> Result<BTreeMap<String, Payload>, Error> {
    let map: BTreeMap<String, Payload> = serde_json::from_value(value)
        .map_err(|error| data(format!("invalid previous payload record: {error}")))?;
    for (key, entry) in &map {
        let name = entry.member.strip_prefix("implicit-content/");
        if unhex(key).is_none()
            || !entry.delta.iter().all(|v| v.is_finite())
            || name.is_none_or(|name| {
                name.is_empty()
                    || name.contains('/')
                    || name.contains('\\')
                    || name.starts_with('.')
            })
        {
            return Err(data(format!("invalid previous payload record: {key}")));
        }
    }
    Ok(map)
}

fn read_member(archive: &mut zip::ZipArchive<File>, name: &str) -> Result<Option<Vec<u8>>, Error> {
    let mut file = match archive.by_name(name) {
        Ok(file) => file,
        Err(zip::result::ZipError::FileNotFound) => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(&mut file, &mut bytes)?;
    Ok(Some(bytes))
}

/// Recover the exact raw bytes of `uri` (`t/<stem>.<suffix>`) from the previous
/// archive: in place, or by inverting the recorded rebase of the published member.
/// Every failure, including a missing member or a record that does not invert, is
/// an integrity error: the result must hash to its immutable name.
fn previous_payload(
    archive: &mut zip::ZipArchive<File>,
    entry: Option<&Payload>,
    uri: &str,
    stem: &str,
    suffix: &str,
) -> Result<Vec<u8>, Error> {
    let failed =
        |reason: String| data(format!("previous content checksum failed: {uri}: {reason}"));
    let bytes = match entry {
        None => read_member(archive, uri)?
            .ok_or_else(|| failed("the archive has no such member".into()))?,
        Some(entry) => {
            if !entry.member.ends_with(&format!(".{suffix}")) {
                return Err(data(format!("invalid previous payload record: {stem}")));
            }
            let published = read_member(archive, &entry.member)?
                .ok_or_else(|| failed(format!("published member {} is absent", entry.member)))?;
            let record = RebaseRecord {
                raw_sha256: unhex(stem).ok_or_else(|| failed("invalid content name".into()))?,
                delta: entry.delta,
                raw_root_node: entry.root_node.clone(),
                raw_json_length: entry.json_length,
            };
            crate::implicit::restore_payload(&published, &record)
                .map_err(|error| failed(format!("{} does not restore: {error}", entry.member)))?
        }
    };
    if hash(&bytes) != stem {
        return Err(failed("sha256 differs".into()));
    }
    Ok(bytes)
}

#[cfg(test)]
mod codec_reuse_tests {
    use super::*;

    /// An intact earlier archive whose explicit hierarchy still names a
    /// b3dm fill is refused up front with the actionable rebuild message.
    #[test]
    fn previous_b3dm_fill_archive_refuses_reuse() {
        let root = tempfile::tempdir().unwrap();
        for (uri, refused) in [("t/aa.b3dm", true), ("t/aa.glb", false)] {
            let tile = json!({"geometricError":0,"children":[{"geometricError":0,
                "contents":[{"uri":"t/bb.glb"},{"uri":uri}]}]});
            let manifest = json!({"asset":{"version":"1.1"},"geometricError":1,"root":tile});
            let state = json!({"version":1,"records":{},"config":{"encoder":encoder(),"where":null},
                "manifestSha256":digest(&manifest).unwrap(),"anchor":[0.,0.,0.],
                "frame":[[1.,0.,0.],[0.,1.,0.],[0.,0.,1.]]});
            let raw = serde_json::to_vec(&state).unwrap();
            let mut published = manifest.clone();
            published["asset"]["extras"] = json!({"vectorBuildStateSha256":hash(&raw)});
            let path = root.path().join(format!("previous-{refused}.3tz"));
            let mut zip = zip::ZipWriter::new(File::create(&path).unwrap());
            let options = zip::write::SimpleFileOptions::default();
            zip.start_file("tileset.json", options).unwrap();
            std::io::Write::write_all(&mut zip, &serde_json::to_vec(&published).unwrap()).unwrap();
            zip.start_file("vector-build.json", options).unwrap();
            std::io::Write::write_all(&mut zip, &raw).unwrap();
            zip.finish().unwrap();
            let options = VectorOptions {
                reuse_tileset: Some(path),
                ..VectorOptions::default()
            };
            match Reuse::new(root.path(), &options) {
                Err(Error::Data(message)) if refused => assert_eq!(message, LEGACY_FILL),
                Ok(_) if !refused => {}
                Err(error) => panic!("unexpected refusal for {uri}: {error}"),
                Ok(_) => panic!("b3dm fill archive must not be reused"),
            }
        }
        assert!(LEGACY_FILL.contains("without --reuse-tileset"));
    }

    /// Exercise the real subtree signature and restore decision: an unchanged
    /// encoder identity restores the cached node, a changed framing dependency
    /// rebuilds it without opening or copying old content.
    #[test]
    fn selected_framing_dependency_change_invalidates_cached_subtree() {
        let root = tempfile::tempdir().unwrap();
        let archive_path = root.path().join("previous.3tz");
        zip::ZipWriter::new(File::create(&archive_path).unwrap())
            .finish()
            .unwrap();
        let db = rusqlite::Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE features(path TEXT,fingerprint TEXT); INSERT INTO features VALUES('', 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa');").unwrap();
        let source = include_str!("../../vector_encoding/framing.rs");
        let original = encoder_with_framing(source);
        assert_eq!(original, encoder());
        let mut same = Reuse::new(root.path(), &VectorOptions::default()).unwrap();
        same.configuration_hash = original.clone();
        let signature = same.signature(&db, "").unwrap();
        same.previous = Some(archive_path);
        same.old.insert(
            "".into(),
            Record {
                signature: signature.clone(),
                center: [0.; 3],
                padding: 0.,
                fragments: 1,
                cut: None,
            },
        );
        same.nodes.insert(
            "".into(),
            json!({"extras":{"routing":true},"geometricError":0}),
        );
        let mut counters = Counters {
            fragments: 1,
            ..Counters::default()
        };
        assert_eq!(same.signature(&db, "").unwrap(), signature);
        assert!(same
            .restore("", &signature, &mut counters, 1, 8)
            .unwrap()
            .is_some());
        assert_eq!(same.reused_subtrees, 1);
        // An actual byte change in a codec framing dependency changes the
        // configuration input of the real signature, not feature fingerprints.
        let changed_source = format!("{source}\n// selected framing policy changed\n");
        same.configuration_hash = encoder_with_framing(&changed_source);
        let changed = same.signature(&db, "").unwrap();
        assert_ne!(same.configuration_hash, original);
        assert_ne!(changed, signature);
        let before = (same.reused_subtrees, counters.tiles);
        assert!(same
            .restore("", &changed, &mut counters, 1, 8)
            .unwrap()
            .is_none());
        assert_eq!((same.reused_subtrees, counters.tiles), before);
    }
}
