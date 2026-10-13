//! Versioned native encoder identity, immutable content and subtree invalidation.
use super::*;
use crate::implicit::{ForestOutput, RebaseRecord};
use crate::tileset_node::TileTree;
use records::{VectorTileExtras, VectorTree};
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
    // Intrinsic source-chart triangulation changes native content too; the
    // encoder identity is the reviewed source/dependency fingerprint.
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
        include_str!("records.rs"),
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
        include_str!("../../tileset_node.rs"),
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

/// Archives published by the retired explicit vector mode record no
/// `explicitRoot` and no implicit tiling; their layout cannot be reused.
const RETIRED_EXPLICIT: &str = "previous archive uses the retired explicit vector layout; \
     reuse needs an archive from the default implicit output. Run a fresh conversion without \
     reuseTileset (--reuse-tileset)";

/// Fragmented polygon fills were b3dm-wrapped before glTF-only fills. Their
/// payloads cannot be reused; the archive must be rebuilt.
const LEGACY_FILL: &str = "previous archive has legacy b3dm polygon fills; fills are now glTF \
     vector content. Run a fresh conversion without reuseTileset (--reuse-tileset)";

fn has_legacy_fill(node: &Value) -> bool {
    contents(node).iter().any(|content| {
        content["uri"]
            .as_str()
            .is_some_and(|uri| uri.to_ascii_lowercase().ends_with(".b3dm"))
    }) || node["children"]
        .as_array()
        .is_some_and(|children| children.iter().any(has_legacy_fill))
}

fn contents(node: &Value) -> Vec<&Value> {
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
            if state.get("explicitRoot").is_none() {
                return Err(data(RETIRED_EXPLICIT));
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
            let previous_root = &state["explicitRoot"];
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
        self.config["tiling"] = json!("implicit-forest-v3");
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
        let value = self
            .nodes
            .get(path)
            .cloned()
            .ok_or_else(|| data("cached subtree is absent from previous manifest"))?;
        let mut archive = zip::ZipArchive::new(File::open(self.previous.as_ref().unwrap())?)?;
        /// Read the cached subtree strictly as typed records, staging each content's raw
        /// payload privately in the build workspace under its immutable name; this build's
        /// writer rebases or publishes it per the new layout. The subtree's top tile drops
        /// its transform: its new parent places it.
        fn visit(
            node: &Value,
            child: bool,
            reuse: &mut Reuse,
            archive: &mut zip::ZipArchive<File>,
            counters: &mut Counters,
            max_tiles: usize,
        ) -> Result<VectorTree, Error> {
            counters.tiles += 1;
            if counters.tiles > max_tiles {
                return Err(data("reused hierarchy exceeds maxTiles"));
            }
            reuse.reused_tiles += 1;
            let children = match node.get("children") {
                None => &[][..],
                Some(Value::Array(children)) => children.as_slice(),
                Some(_) => return Err(data("previous vector hierarchy: children must be an array; run a fresh conversion without reuseTileset (--reuse-tileset)")),
            };
            let record = records::previous_record(node, child, &mut |uri, stem| {
                let bytes = previous_payload(archive, reuse.payloads.get(stem), uri, stem, "glb")?;
                let target = reuse.output.join(uri);
                if !target.exists() {
                    std::fs::write(target, &bytes)?;
                }
                reuse.reused_references += 1;
                reuse.reused_uris.insert(uri.into());
                Ok(bytes.len() as u64)
            })?;
            if children.is_empty() && !record.contents().is_empty() {
                counters.leaf_tiles += 1;
            }
            if record.extras().is_some_and(VectorTileExtras::is_routing) {
                counters.routing_tiles += 1;
            }
            if let (false, Some(statistics)) = (
                record.contents().is_empty(),
                record.extras().and_then(VectorTileExtras::statistics),
            ) {
                counters.maximum_tile_vertices =
                    counters.maximum_tile_vertices.max(statistics.vertices);
                counters.maximum_tile_bytes =
                    counters.maximum_tile_bytes.max(statistics.encoded_bytes);
            }
            let children = children
                .iter()
                .map(|child| visit(child, true, reuse, archive, counters, max_tiles))
                .collect::<Result<_, Error>>()?;
            Ok(TileTree::new(record, children))
        }
        let tree = visit(&value, false, self, &mut archive, counters, max_tiles)?;
        for (key, record) in self.old.range(path.to_string()..format!("{path}~")) {
            self.records.insert(key.clone(), record.clone());
        }
        // The restored leaves' own total: a record from an earlier partial reuse can count
        // a restored descendant's split feature as one row.
        let fragments = records::leaf_fragments(&tree)?;
        counters.fragments = counters.fragments + fragments - current_fragments;
        self.reused_subtrees += 1;
        Ok(Some(Node {
            tree,
            center: record.center,
            padding: record.padding,
            fragments,
        }))
    }
    pub fn remember(
        &mut self,
        path: &str,
        signature: String,
        node: &mut Node,
    ) -> Result<(), Error> {
        records::set_build_path(node.tree.record_mut(), path)?;
        self.records.insert(
            path.into(),
            Record {
                signature,
                center: node.center,
                padding: node.padding,
                fragments: node.fragments,
                cut: self.cuts.get(path).cloned(),
            },
        );
        Ok(())
    }
    /// Record build state and publish only referenced raw payloads. `tree` is the
    /// producer's hierarchy H (the forest's input); `manifest` is `tileset.json` as the
    /// forest wrote it, without the build state; `forest` is the writer's output. Returns
    /// the build state's sha256, which `tileset.json` records, and the reuse report.
    pub fn publish(
        &mut self,
        manifest: &Value,
        frame: &Frame,
        tree: &VectorTree,
        forest: &ForestOutput,
    ) -> Result<([u8; 32], Value), Error> {
        let contents: BTreeSet<&str> = tree
            .preorder()
            .into_iter()
            .flat_map(|(record, _, _)| record.contents())
            .map(|content| content.uri().as_str())
            .collect();
        // Raw payloads stay published only where a forest's explicit tile names
        // them in place. The forest writer has already removed the raw sources it
        // rebased.
        let published: BTreeSet<&str> = forest.members.iter().map(String::as_str).collect();
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
            "payloads":payload_map(forest),"manifestSha256":digest(manifest)?});
        state["explicitRoot"] = tree.to_value()?;
        let raw = canonical(&state)?;
        std::fs::write(self.output.join("vector-build.json"), &raw)?;
        Ok((
            Sha256::digest(&raw).into(),
            json!({"previousTileset":self.previous.is_some(),"requestedPreviousTileset":self.requested,"incompatibleReason":self.incompatible,
            "reusedSubtrees":self.reused_subtrees,"reusedTiles":self.reused_tiles,"reusedContents":self.reused_uris.len(),"reusedContentReferences":self.reused_references,
            "publishedContents":contents.len(),"rebuiltContents":contents.iter().filter(|uri| !self.reused_uris.contains(**uri)).count()}),
        ))
    }
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
fn payload_map(forest: &ForestOutput) -> BTreeMap<String, Payload> {
    let mut map = BTreeMap::new();
    for content in forest.tiles.iter().flat_map(|tile| &tile.contents) {
        if let Some(record) = &content.rebase {
            map.entry(records::hex(&record.raw_sha256))
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
    /// b3dm fill, or that the retired explicit mode published (no
    /// `explicitRoot`), is refused up front with the actionable rebuild message.
    #[test]
    fn previous_b3dm_fill_or_explicit_archive_refuses_reuse() {
        let root = tempfile::tempdir().unwrap();
        for (uri, implicit, refusal) in [
            ("t/aa.b3dm", true, Some(LEGACY_FILL)),
            ("t/aa.glb", true, None),
            ("t/aa.glb", false, Some(RETIRED_EXPLICIT)),
        ] {
            let refused = refusal.is_some();
            let tile = json!({"geometricError":0,"children":[{"geometricError":0,
                "contents":[{"uri":"t/bb.glb"},{"uri":uri}]}]});
            let manifest = json!({"asset":{"version":"1.1"},"geometricError":1,"root":tile});
            let mut state = json!({"version":1,"records":{},"config":{"encoder":encoder(),"where":null},
                "manifestSha256":digest(&manifest).unwrap(),"anchor":[0.,0.,0.],
                "frame":[[1.,0.,0.],[0.,1.,0.],[0.,0.,1.]]});
            if implicit {
                state["explicitRoot"] = tile.clone();
            }
            let raw = serde_json::to_vec(&state).unwrap();
            let mut published = manifest.clone();
            published["asset"]["extras"] = json!({"vectorBuildStateSha256":hash(&raw)});
            let path = root
                .path()
                .join(format!("previous-{refused}-{implicit}.3tz"));
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
            match (Reuse::new(root.path(), &options), refusal) {
                (Err(Error::Data(message)), Some(expected)) => assert_eq!(message, expected),
                (Ok(_), None) => {}
                (Err(error), _) => panic!("unexpected refusal for {uri}: {error}"),
                (Ok(_), Some(_)) => panic!("{uri} implicit {implicit} must not be reused"),
            }
        }
        assert!(LEGACY_FILL.contains("without reuseTileset"));
        assert!(RETIRED_EXPLICIT.contains("without reuseTileset"));
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
            json!({"boundingVolume":{"box":[0,0,0,1.0,0,0,0,1.0,0,0,0,1.0]},"refine":"REPLACE",
                "extras":{"routing":true,"routingReason":null},"geometricError":0.0}),
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

    /// A record written by an earlier partial reuse can count a restored leaf's split feature
    /// as one row. Restore must take the fragment total from the stored leaves, correct the
    /// build counter by the difference, and carry it on the node so `remember` records it.
    #[test]
    fn restore_counts_stored_leaf_fragments_and_remembers_them() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("t")).unwrap();
        let glb = b"leaf payload".to_vec();
        let uri = format!("t/{}.glb", hash(&glb));
        let archive_path = root.path().join("previous.3tz");
        let mut zip = zip::ZipWriter::new(File::create(&archive_path).unwrap());
        zip.start_file(&uri, zip::write::SimpleFileOptions::default())
            .unwrap();
        std::io::Write::write_all(&mut zip, &glb).unwrap();
        zip.finish().unwrap();
        let mut reuse = Reuse::new(root.path(), &VectorOptions::default()).unwrap();
        reuse.previous = Some(archive_path);
        reuse.old.insert(
            "".into(),
            Record {
                signature: "signature".into(),
                center: [0.; 3],
                padding: 0.,
                fragments: 1,
                cut: None,
            },
        );
        reuse.nodes.insert(
            "".into(),
            json!({"boundingVolume":{"box":[0,0,0,1.0,0,0,0,1.0,0,0,0,1.0]},"refine":"REPLACE",
                "geometricError":0.0,
                "content":{"uri":uri,"extensions":{"3DTILES_content_gltf_vector":{"vector":true}}},
                "extras":{"featureFragments":5,"vertices":9,"primitives":1,"encodedBytes":1234,
                    "geometryErrorMetres":0.3,"positionRoundingMetres":0.00001,
                    "quantizationErrorMetres":0.0,"uncompressedBytes":2000,"toleranceMetres":0.0}}),
        );
        let start = 10;
        let mut counters = Counters {
            fragments: start,
            ..Counters::default()
        };
        let mut node = reuse
            .restore("", "signature", &mut counters, 1, 8)
            .unwrap()
            .expect("the unchanged subtree is restored");
        assert_eq!(node.fragments, 5);
        assert_eq!(counters.fragments, start + 4);
        reuse.remember("", "signature".into(), &mut node).unwrap();
        assert_eq!(reuse.records[""].fragments, 5);
    }
}
