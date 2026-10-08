//! Versioned native encoder identity, immutable content and subtree invalidation.
use super::*;
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
    // Intrinsic source-chart triangulation changes native explicit content too;
    // every hierarchy uses the reviewed source/dependency fingerprint.
    let mut hasher = Sha256::new();
    for source in [
        include_str!("../pipeline.rs"),
        include_str!("../model.rs"),
        include_str!("../source_fields.rs"),
        include_str!("geometry.rs"),
        include_str!("encoding.rs"),
        include_str!("aggregation.rs"),
        include_str!("../../point_sampling.rs"),
        include_str!("store.rs"),
        include_str!("reuse.rs"),
        include_str!("../../vector_encoding.rs"),
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
    for source in [
        include_str!("../portable.rs"),
        include_str!("geometry/portable.rs"),
        include_str!("../../crs.rs"),
    ] {
        hasher.update(source.as_bytes());
        hasher.update([0]);
    }
    format!("{ENCODER_PREFIX}{:x}", hasher.finalize())
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
            index(
                state.get("explicitRoot").unwrap_or(&manifest["root"]),
                &mut result.nodes,
            )?;
            result.old = serde_json::from_value(state["records"].clone())?;
            result.cuts = result
                .old
                .iter()
                .filter_map(|(p, r)| r.cut.clone().map(|cut| (p.clone(), cut)))
                .collect();
            result.frame = Some(frame);
            result.old_config = state["config"].clone();
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
            .layer_reports
            .iter()
            .map(|layer| {
                let mut layer = layer.clone();
                for key in ["features", "invalidFeatures", "featuresWithoutGeometry"] {
                    layer.as_object_mut().unwrap().remove(key);
                }
                layer
            })
            .collect();
        self.config = json!({"encoder":encoder(),"versions":versions,"driver":reader.driver,"schemas":reader.schemas,"layers":layers,
            "quantize":options.quantize,"meshopt":options.meshopt,"maxFeatures":max_features,"maxParentFeatures":options.max_parent_features,"maxVertices":options.max_vertices,"maxBytes":options.max_bytes,
            "lodTolerance":options.lod.tolerance_metres,"lodLevels":options.lod.levels,"parentRepair":options.parent_repair,"aggregatePoints":options.aggregate_points,"skipInvalid":options.skip_invalid,"repair":repair,"ambiguousOutlines":ambiguous,
            "sourceCrs":options.source_crs,"where":options.where_clause,"heightOffset":options.height_offset,"listFields":options.list_fields,"fields":options.fields,"dropFields":options.drop_fields});
        if !options.explicit {
            self.config["tiling"] = json!("implicit-quadtree-v2");
        }
        if self.previous.is_some() && self.config != self.old_config {
            return Err(data("previous encoder, schema, CRS or conversion settings differ; run a fresh conversion without reuseTileset"));
        }
        self.configuration_hash = digest(&self.config)?;
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
                    || !matches!(suffix, "glb" | "b3dm")
                {
                    return Err(data(
                        "previous content URI is not an immutable vector content path",
                    ));
                }
                let mut file = archive.by_name(uri)?;
                let mut bytes = Vec::new();
                std::io::Read::read_to_end(&mut file, &mut bytes)?;
                if hash(&bytes) != stem {
                    return Err(data(format!("previous content checksum failed: {uri}")));
                }
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
    pub fn publish(
        &mut self,
        manifest: &mut Value,
        frame: &Frame,
        explicit_root: Option<&Value>,
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
        used(explicit_root.unwrap_or(&manifest["root"]), &mut contents);
        for entry in std::fs::read_dir(self.output.join("t"))? {
            let entry = entry?;
            if !contents.contains(&format!("t/{}", entry.file_name().to_string_lossy())) {
                std::fs::remove_file(entry.path())?;
            }
        }
        let mut state = json!({"version":1,"config":self.config,"anchor":frame.anchor,"frame":frame.axes,"records":self.records,"manifestSha256":digest(manifest)?});
        if let Some(root) = explicit_root {
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
