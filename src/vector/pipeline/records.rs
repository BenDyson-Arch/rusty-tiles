//! The vector producer's typed tile records: its `extras`, the content record it builds when a
//! payload is persisted, and the strict reader of a previous build's hierarchy
//! (`vector-build.json` `explicitRoot`), the one place this producer reads tile JSON.
use super::*;
use crate::tileset_node::{
    ContentKind, ContentRef, GeometricError, MemberName, NodePlacement, Refine, SourceIdentity,
    TileBox, TileRecord, TileTree, CONTENT_GLTF_VECTOR,
};

/// A content tile's statistics, in metres and bytes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Statistics {
    pub feature_fragments: usize,
    pub vertices: usize,
    pub primitives: usize,
    pub encoded_bytes: usize,
    pub geometry_error_metres: f64,
    pub position_rounding_metres: f64,
    pub quantization_error_metres: f64,
    pub uncompressed_bytes: usize,
    pub tolerance_metres: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub point_aggregation: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build_path: Option<String>,
}

/// A tile without content whose features route to its children.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Routing {
    routing: bool,
    /// The budget that refused a parent content, or `null`.
    routing_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    build_path: Option<String>,
}

/// The empty dataset's root.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct BuildPath {
    build_path: String,
}

/// The vector producer's tile `extras`. Untagged: each variant has a distinct required key
/// set, written exactly as the producer has always written it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub(super) enum VectorTileExtras {
    Content(Statistics),
    Routing(Routing),
    Empty(BuildPath),
}

impl VectorTileExtras {
    pub fn routing(reason: Option<&str>) -> Self {
        Self::Routing(Routing {
            routing: true,
            routing_reason: reason.map(str::to_owned),
            build_path: None,
        })
    }

    pub fn statistics(&self) -> Option<&Statistics> {
        match self {
            Self::Content(statistics) => Some(statistics),
            _ => None,
        }
    }

    pub fn is_routing(&self) -> bool {
        matches!(self, Self::Routing(_))
    }

    /// The content's own position rounding plus quantization error; 0 without content.
    pub fn padding(&self) -> f64 {
        self.statistics().map_or(0., |s| {
            s.position_rounding_metres + s.quantization_error_metres
        })
    }

    fn set_build_path(&mut self, path: &str) {
        match self {
            Self::Content(s) => s.build_path = Some(path.into()),
            Self::Routing(r) => r.build_path = Some(path.into()),
            Self::Empty(e) => e.build_path = path.into(),
        }
    }
}

pub(super) type VectorRecord = TileRecord<VectorTileExtras>;
pub(super) type VectorTree = TileTree<VectorTileExtras>;

/// Record the build path under which reuse finds this tile's subtree.
pub(super) fn set_build_path(record: &mut VectorRecord, path: &str) -> Result<(), Error> {
    let extras = match record.extras().cloned() {
        Some(mut extras) => {
            extras.set_build_path(path);
            extras
        }
        None => VectorTileExtras::Empty(BuildPath {
            build_path: path.into(),
        }),
    };
    record.set_extras(Some(extras))
}

/// The digest of the source identities (`_source_layer`, `_source_id`) of a content's
/// features, in content order. Not serialized; `_source_id` in the GLB is the carrier.
pub(super) fn identity_digest(features: &[Feature]) -> Result<[u8; 32], Error> {
    let mut hasher = Sha256::new();
    for feature in features {
        for key in ["_source_layer", "_source_id"] {
            let value = feature.properties.get(key).unwrap_or(&Value::Null);
            hasher.update(serde_json::to_vec(value)?);
            hasher.update([0]);
        }
    }
    Ok(hasher.finalize().into())
}

/// The record of a persisted vector payload `t/<sha256>.glb`.
pub(super) fn content(
    bytes: &[u8],
    sources: SourceIdentity,
) -> Result<(ContentRef, String), Error> {
    let sha256: [u8; 32] = Sha256::digest(bytes).into();
    let uri = format!("t/{}.glb", hex(&sha256));
    let content = ContentRef::new(
        MemberName::new(uri.clone())?,
        sha256,
        bytes.len() as u64,
        ContentKind::Glb,
        true,
        sources,
    )?;
    Ok((content, uri))
}

/// The source fragments that the full-detail leaves of H hold: the sum of every leaf
/// content's feature count. A build holds each spool fragment in exactly one leaf, so this
/// equals the build's fragment count unless a fragment was dropped or duplicated.
pub(super) fn leaf_fragments(tree: &VectorTree) -> Result<usize, Error> {
    let mut total = 0usize;
    let mut stack = vec![tree];
    while let Some(tile) = stack.pop() {
        if tile.children().is_empty() {
            for content in tile.record().contents() {
                match content.sources() {
                    SourceIdentity::Features { count, .. } | SourceIdentity::Reused { count } => {
                        total += count as usize
                    }
                    SourceIdentity::Aggregate => {
                        return Err(data("a full-detail vector leaf holds point aggregates"))
                    }
                }
            }
        }
        stack.extend(tile.children());
    }
    Ok(total)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The lowercase sha256 stem of an immutable vector payload name `t/<stem>.glb`.
pub(super) fn payload_stem(uri: &str) -> Option<&str> {
    let stem = uri.strip_prefix("t/")?.strip_suffix(".glb")?;
    (stem.len() == 64
        && stem
            .bytes()
            .all(|v| v.is_ascii_digit() || (b'a'..=b'f').contains(&v)))
    .then_some(stem)
}

fn refused(what: impl std::fmt::Display) -> Error {
    data(format!(
        "previous vector hierarchy: {what}; run a fresh conversion without reuseTileset \
         (--reuse-tileset)"
    ))
}

const TILE_KEYS: [&str; 7] = [
    "boundingVolume",
    "children",
    "content",
    "extras",
    "geometricError",
    "refine",
    "transform",
];

fn integer_zero(value: &Value) -> bool {
    value.as_u64() == Some(0) && !value.is_f64()
}

/// Read one tile of a previous build's hierarchy H (`explicitRoot`) as a typed record,
/// strictly: only the members, number forms and extension this producer writes. A `child`
/// has a translation `transform`; the transform of the subtree's top tile is dropped, as its
/// new parent places it. `stage` stages a content's payload by name and sha256 stem and
/// returns its byte length.
pub(super) fn previous_record(
    node: &Value,
    child: bool,
    stage: &mut impl FnMut(&str, &str) -> Result<u64, Error>,
) -> Result<VectorRecord, Error> {
    let object = node
        .as_object()
        .ok_or_else(|| refused("a tile is not an object"))?;
    if let Some(key) = object.keys().find(|k| !TILE_KEYS.contains(&k.as_str())) {
        return Err(refused(format!("a tile has the member {key:?}")));
    }
    let volume = object
        .get("boundingVolume")
        .and_then(Value::as_object)
        .filter(|v| v.len() == 1)
        .and_then(|v| v.get("box"))
        .and_then(Value::as_array)
        .filter(|lanes| lanes.len() == 12)
        .ok_or_else(|| refused("a tile has no 12-number box"))?;
    let mut half = [0.; 3];
    for (k, lane) in volume.iter().enumerate() {
        if k % 4 == 3 {
            half[k / 4] = lane
                .as_f64()
                .filter(|_| lane.is_f64())
                .ok_or_else(|| refused("a box half length is not a number"))?;
        } else if !integer_zero(lane) {
            return Err(refused("a box is not centred on its tile frame"));
        }
    }
    let error = object
        .get("geometricError")
        .filter(|e| e.is_f64())
        .and_then(Value::as_f64)
        .ok_or_else(|| refused("a tile has no measured geometricError"))?;
    if object.get("refine").and_then(Value::as_str) != Some(Refine::Replace.as_str()) {
        return Err(refused("a tile does not refine by REPLACE"));
    }
    let placement = match (child, object.get("transform")) {
        (false, _) => NodePlacement::INHERIT,
        (true, Some(Value::Array(m))) if m.len() == 16 => {
            let structural = (0..12).chain([15]).all(|i| {
                let expected = u64::from(i % 5 == 0);
                m[i].as_u64() == Some(expected) && !m[i].is_f64()
            });
            let offset: Option<Vec<f64>> = m[12..15]
                .iter()
                .map(|v| v.as_f64().filter(|_| v.is_f64()))
                .collect();
            match (structural, offset) {
                (true, Some(o)) => NodePlacement::translation([o[0], o[1], o[2]])?,
                _ => return Err(refused("a child transform is not a translation")),
            }
        }
        (true, _) => return Err(refused("a child tile has no translation transform")),
    };
    let extras = match object.get("extras") {
        None => None,
        Some(value) => Some(
            VectorTileExtras::deserialize(value)
                .map_err(|_| refused("a tile has unexpected extras"))?,
        ),
    };
    let mut contents = Vec::new();
    if let Some(content) = object.get("content") {
        let marker = serde_json::json!({ CONTENT_GLTF_VECTOR: {"vector": true} });
        let uri = content
            .as_object()
            .filter(|c| c.len() == 2 && c.get("extensions") == Some(&marker))
            .and_then(|c| c.get("uri"))
            .and_then(Value::as_str)
            .ok_or_else(|| refused("a content is not a vector glTF content"))?;
        let stem = payload_stem(uri)
            .ok_or_else(|| refused("a content URI is not an immutable vector content path"))?;
        let sources = match extras.as_ref().and_then(VectorTileExtras::statistics) {
            Some(Statistics {
                point_aggregation: Some(_),
                ..
            }) => SourceIdentity::Aggregate,
            Some(statistics) => SourceIdentity::Reused {
                count: u32::try_from(statistics.feature_fragments)
                    .map_err(|_| refused("a content has too many features"))?,
            },
            None => return Err(refused("a content tile has no statistics")),
        };
        let length = stage(uri, stem)?;
        let mut sha256 = [0u8; 32];
        for (i, byte) in sha256.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&stem[2 * i..2 * i + 2], 16)
                .map_err(|_| refused("a content URI is not hexadecimal"))?;
        }
        contents.push(ContentRef::new(
            MemberName::new(uri)?,
            sha256,
            length,
            ContentKind::Glb,
            true,
            sources,
        )?);
    }
    TileRecord::new(
        placement,
        TileBox::centred(half)?,
        GeometricError::new(error)?,
        Refine::Replace,
        contents,
        extras,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tileset_node::ObjectRole;

    fn statistics(aggregation: Option<Value>, path: Option<&str>) -> VectorTileExtras {
        VectorTileExtras::Content(Statistics {
            feature_fragments: 2,
            vertices: 9,
            primitives: 1,
            encoded_bytes: 1234,
            geometry_error_metres: 0.3,
            position_rounding_metres: 1e-5,
            quantization_error_metres: 0.,
            uncompressed_bytes: 2000,
            tolerance_metres: 0.,
            point_aggregation: aggregation,
            build_path: path.map(str::to_owned),
        })
    }

    /// The extras key sets and number forms the producer has always written.
    #[test]
    fn extras_golden_forms() {
        let content = serde_json::to_string(&statistics(None, None)).unwrap();
        assert_eq!(
            content,
            r#"{"featureFragments":2,"vertices":9,"primitives":1,"encodedBytes":1234,"geometryErrorMetres":0.3,"positionRoundingMetres":0.00001,"quantizationErrorMetres":0.0,"uncompressedBytes":2000,"toleranceMetres":0.0}"#
        );
        let mut record = TileRecord::new(
            NodePlacement::INHERIT,
            TileBox::centred([1.; 3]).unwrap(),
            GeometricError::new(0.).unwrap(),
            Refine::Replace,
            Vec::new(),
            Some(statistics(
                Some(serde_json::json!({"method":"voxel"})),
                None,
            )),
        )
        .unwrap();
        set_build_path(&mut record, "01").unwrap();
        let object = record
            .explicit_object(record.bounds(), ObjectRole::Tile)
            .unwrap();
        assert_eq!(
            serde_json::to_string(&object["extras"]).unwrap(),
            r#"{"buildPath":"01","encodedBytes":1234,"featureFragments":2,"geometryErrorMetres":0.3,"pointAggregation":{"method":"voxel"},"positionRoundingMetres":0.00001,"primitives":1,"quantizationErrorMetres":0.0,"toleranceMetres":0.0,"uncompressedBytes":2000,"vertices":9}"#
        );
        for (reason, expected) in [
            (
                None,
                r#"{"buildPath":"0","routing":true,"routingReason":null}"#,
            ),
            (
                Some("parentFeatures"),
                r#"{"buildPath":"0","routing":true,"routingReason":"parentFeatures"}"#,
            ),
        ] {
            let mut record = TileRecord::new(
                NodePlacement::INHERIT,
                TileBox::centred([1.; 3]).unwrap(),
                GeometricError::new(2.).unwrap(),
                Refine::Replace,
                Vec::new(),
                Some(VectorTileExtras::routing(reason)),
            )
            .unwrap();
            set_build_path(&mut record, "0").unwrap();
            let object = record.extras_object().unwrap().unwrap();
            assert_eq!(serde_json::to_string(&object).unwrap(), expected);
        }
        let mut empty = TileRecord::<VectorTileExtras>::empty_root(None).unwrap();
        set_build_path(&mut empty, "").unwrap();
        assert_eq!(
            serde_json::to_string(
                &empty
                    .explicit_object(empty.bounds(), ObjectRole::Tile)
                    .unwrap()
            )
            .unwrap(),
            r#"{"boundingVolume":{"box":[0,0,0,0.001,0,0,0,0.001,0,0,0,0.001]},"extras":{"buildPath":""},"geometricError":0,"refine":"REPLACE"}"#
        );
    }

    fn tree() -> (VectorTree, BTreeMap<String, u64>) {
        let leaf_bytes = b"glTF\x02\0\0\0leaf-payload".to_vec();
        let coarse_bytes = b"glTF\x02\0\0\0coarse-payload".to_vec();
        let mut lengths = BTreeMap::new();
        let mut node = |bytes: &[u8], sources, extras, error, placement| {
            let (content, uri) = content(bytes, sources).unwrap();
            lengths.insert(uri, bytes.len() as u64);
            TileRecord::new(
                placement,
                TileBox::centred([3., 0.001, 1.5]).unwrap(),
                GeometricError::new(error).unwrap(),
                Refine::Replace,
                vec![content],
                Some(extras),
            )
            .unwrap()
        };
        let identity = SourceIdentity::Reused { count: 2 };
        let leaf = node(
            &leaf_bytes,
            identity,
            statistics(None, None),
            0.,
            NodePlacement::translation([0.; 3]).unwrap(),
        );
        let coarse = node(
            &coarse_bytes,
            SourceIdentity::Aggregate,
            statistics(Some(serde_json::json!({"method":"voxel"})), Some("1")),
            0.75,
            NodePlacement::translation([1.25, -2., 0.5]).unwrap(),
        );
        let mut routing = TileRecord::new(
            NodePlacement::INHERIT,
            TileBox::centred([4., 2., 2.]).unwrap(),
            GeometricError::new(5.5).unwrap(),
            Refine::Replace,
            Vec::new(),
            Some(VectorTileExtras::routing(None)),
        )
        .unwrap();
        set_build_path(&mut routing, "").unwrap();
        let tree = TileTree::new(
            routing,
            vec![TileTree::new(coarse, vec![TileTree::new(leaf, Vec::new())])],
        );
        (tree, lengths)
    }

    fn read(
        node: &Value,
        child: bool,
        lengths: &BTreeMap<String, u64>,
    ) -> Result<VectorTree, Error> {
        let record = previous_record(node, child, &mut |uri, _| Ok(lengths[uri]))?;
        let children = node["children"]
            .as_array()
            .map(|children| {
                children
                    .iter()
                    .map(|c| read(c, true, lengths))
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()?
            .unwrap_or_default();
        Ok(TileTree::new(record, children))
    }

    /// Serialize, read strictly, serialize: the identity, byte for byte.
    #[test]
    fn previous_hierarchy_round_trips() {
        let (tree, lengths) = tree();
        let written = tree.to_value().unwrap();
        let again = read(&written, false, &lengths).unwrap();
        assert_eq!(
            serde_json::to_vec(&again.to_value().unwrap()).unwrap(),
            serde_json::to_vec(&written).unwrap()
        );
        assert_eq!(again, tree);
    }

    /// Anything this producer does not write is refused with the rebuild message.
    #[test]
    fn previous_hierarchy_refuses_foreign_members() {
        let (tree, lengths) = tree();
        let written = tree.to_value().unwrap();
        let child = &written["children"][0];
        type Mutation = Box<dyn Fn(&mut Value)>;
        let cases: Vec<(&str, Mutation)> = vec![
            (
                "member",
                Box::new(|v| v["metadata"] = serde_json::json!({})),
            ),
            (
                "contents",
                Box::new(|v| {
                    let c = v["content"].take();
                    v.as_object_mut().unwrap().remove("content");
                    v["contents"] = serde_json::json!([c]);
                }),
            ),
            (
                "extension",
                Box::new(|v| v["content"]["extensions"]["VENDOR_x"] = serde_json::json!({})),
            ),
            (
                "uri",
                Box::new(|v| v["content"]["uri"] = serde_json::json!("../t/x.glb")),
            ),
            (
                "b3dm",
                Box::new(|v| {
                    let uri = v["content"]["uri"]
                        .as_str()
                        .unwrap()
                        .replace(".glb", ".b3dm");
                    v["content"]["uri"] = serde_json::json!(uri);
                }),
            ),
            (
                "extras",
                Box::new(|v| v["extras"]["implicitChildIndex"] = serde_json::json!(1)),
            ),
            (
                "refine",
                Box::new(|v| v["refine"] = serde_json::json!("ADD")),
            ),
            (
                "offset box",
                Box::new(|v| v["boundingVolume"]["box"][0] = serde_json::json!(0.5)),
            ),
            (
                "rotation",
                Box::new(|v| v["transform"][1] = serde_json::json!(0.5)),
            ),
            (
                "no transform",
                Box::new(|v| {
                    v.as_object_mut().unwrap().remove("transform");
                }),
            ),
            (
                "integer error",
                Box::new(|v| v["geometricError"] = serde_json::json!(1)),
            ),
        ];
        for (label, mutate) in cases {
            let mut node = child.clone();
            mutate(&mut node);
            match read(&node, true, &lengths) {
                Err(Error::Data(message)) => {
                    assert!(
                        message.contains("run a fresh conversion"),
                        "{label}: {message}"
                    )
                }
                other => panic!("{label}: {other:?}"),
            }
        }
    }
}
