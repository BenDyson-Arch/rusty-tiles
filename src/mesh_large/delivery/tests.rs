//! H16 tests. Leaf directories are generated here with known H1 addresses (spatial octants,
//! packed leaves, a depth-21 chain into a synthetic run hierarchy), so every expectation on
//! slots, availability, members and bounds comes from the generator, not from the module.
use super::{
    plan::{tile_box, Builder, Limits, Linked, Node, Sink, LIMITS, MIN_HALF_LENGTH},
    publish_with,
    source::{glb_bounds, payload_bounds, Aabb, Digits, LeafRow, MAX_GLB_JSON},
    *,
};
use crate::validate::{inspect, ValidationRequest};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
};

/// One generated leaf: H1 address and rank count.
#[derive(Clone, Debug)]
struct Spec {
    digits: Vec<u8>,
    cell_depth: u8,
    last_octant: u8,
    triangles: u64,
}

impl Spec {
    fn path_len(&self) -> u8 {
        self.digits.len() as u8 - self.cell_depth
    }
    fn packed(&self) -> bool {
        self.path_len() == 0 && self.digits.last().is_some_and(|s| *s != self.last_octant)
    }
}

struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }
}

/// A random spatial subtree below `digits` (depth < `max`), in H1 preorder, with packed leaves.
fn spatial(digits: &mut Vec<u8>, max: usize, rng: &mut Lcg, out: &mut Vec<Spec>) {
    let mut mask = rng.next() as u8;
    if mask == 0 {
        mask = 1 << (rng.next() % 8);
    }
    let chosen: Vec<u8> = (0..8).filter(|s| mask & (1 << s) != 0).collect();
    let mut k = 0;
    while k < chosen.len() {
        let slot = chosen[k];
        digits.push(slot);
        if digits.len() < max && !rng.next().is_multiple_of(3) {
            spatial(digits, max, rng, out);
            k += 1;
        } else {
            // A leaf, sometimes packed over the next chosen octants.
            let take = if rng.next().is_multiple_of(3) {
                (rng.next() as usize % 3).min(chosen.len() - 1 - k)
            } else {
                0
            };
            out.push(Spec {
                digits: digits.clone(),
                cell_depth: digits.len() as u8,
                last_octant: chosen[k + take],
                triangles: 1 + rng.next() % 50,
            });
            k += 1 + take;
        }
        digits.pop();
    }
}

/// A mixed tree: a random spatial subtree under octant 0 and, under octant 7, a chain to
/// depth 21 whose degenerate cell holds a three-level synthetic run (70 leaves, depth 24).
fn mixed_tree(seed: u64) -> Vec<Spec> {
    let mut rng = Lcg(seed);
    let mut out = Vec::new();
    let mut digits = vec![0u8];
    spatial(&mut digits, 7, &mut rng, &mut out);
    let chain: Vec<u8> = std::iter::once(7)
        .chain((1..21).map(|i| (i * 3 % 8) as u8))
        .collect();
    for index in 0..70u8 {
        let mut d = chain.clone();
        d.extend([index >> 6, (index >> 3) & 7, index & 7]);
        out.push(Spec {
            digits: d,
            cell_depth: 21,
            last_octant: index & 7,
            triangles: 8192,
        });
    }
    out
}

/// Every H1 tile address (proper prefixes of leaves, and the leaves).
fn nodes(specs: &[Spec]) -> BTreeSet<Vec<u8>> {
    let mut all = BTreeSet::new();
    for spec in specs {
        for len in 0..=spec.digits.len() {
            all.insert(spec.digits[..len].to_vec());
        }
    }
    all
}

fn axes(digits: &[u8]) -> [u32; 3] {
    let mut out = [0u32; 3];
    for &d in digits {
        for (a, v) in out.iter_mut().enumerate() {
            *v = (*v << 1) | u32::from((d >> a) & 1);
        }
    }
    out
}

/// A one-triangle GLB inside the leaf's cell of a 1024 m cube (glTF y-up).
fn glb(spec: &Spec) -> Vec<u8> {
    let depth = spec.digits.len() as i32;
    let size = 1024f64 / 2f64.powi(depth.min(21));
    let a = axes(&spec.digits[..spec.digits.len().min(21)]);
    let lo: [f64; 3] = std::array::from_fn(|i| f64::from(a[i]) * size);
    // z-up (X, Y, Z) -> glTF (X, Z, -Y)
    let corner = |t: [f64; 3]| [t[0] as f32, t[2] as f32, -t[1] as f32];
    let points = [
        corner([lo[0] + size * 0.25, lo[1] + size * 0.25, lo[2] + size * 0.5]),
        corner([
            lo[0] + size * 0.75,
            lo[1] + size * 0.25,
            lo[2] + size * 0.25,
        ]),
        corner([lo[0] + size * 0.5, lo[1] + size * 0.75, lo[2] + size * 0.75]),
    ];
    let min: Vec<f64> = (0..3)
        .map(|i| {
            points
                .iter()
                .map(|p| f64::from(p[i]))
                .fold(f64::MAX, f64::min)
        })
        .collect();
    let max: Vec<f64> = (0..3)
        .map(|i| {
            points
                .iter()
                .map(|p| f64::from(p[i]))
                .fold(f64::MIN, f64::max)
        })
        .collect();
    let bin: Vec<u8> = points
        .iter()
        .flatten()
        .flat_map(|c| c.to_le_bytes())
        .collect();
    let doc = json!({"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],"nodes":[{"mesh":0}],
        "meshes":[{"primitives":[{"attributes":{"POSITION":0},"mode":4}]}],
        "accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":min,"max":max}],
        "bufferViews":[{"buffer":0,"byteLength":36}],"buffers":[{"byteLength":36}]});
    let mut text = serde_json::to_vec(&doc).unwrap();
    while !text.len().is_multiple_of(4) {
        text.push(b' ');
    }
    let mut out = b"glTF".to_vec();
    out.extend(2u32.to_le_bytes());
    out.extend((28 + text.len() as u32 + 36).to_le_bytes());
    out.extend((text.len() as u32).to_le_bytes());
    out.extend(b"JSON");
    out.extend(&text);
    out.extend(36u32.to_le_bytes());
    out.extend(b"BIN\0");
    out.extend(bin);
    out
}

fn row_json(index: usize, spec: &Spec, first: u64, bytes: &[u8]) -> Value {
    let cell = spec.digits[..spec.cell_depth as usize]
        .iter()
        .fold(0u64, |acc, d| (acc << 3) | u64::from(*d));
    let path = spec.digits[spec.cell_depth as usize..]
        .iter()
        .fold(0u64, |acc, d| (acc << 3) | u64::from(*d));
    json!({"schemaVersion":1,"uri":format!("leaves/{index:012}.glb"),
        "sha256":format!("{:x}", Sha256::digest(bytes)),"bytes":bytes.len(),
        "first":first,"end":first + spec.triangles,"cellDepth":spec.cell_depth,"cell":cell,
        "pathLength":spec.path_len(),"path":path,"lastOctant":spec.last_octant,
        "depth":spec.digits.len(),"packed":spec.packed(),"geometryError":0.001})
}

/// Write an H8-shaped leaf directory.
fn leaf_dir(parent: &Path, specs: &[Spec]) -> PathBuf {
    let root = parent.join("leaves-dir");
    std::fs::create_dir_all(root.join("leaves")).unwrap();
    let mut rows = String::new();
    let mut first = 0u64;
    for (index, spec) in specs.iter().enumerate() {
        let bytes = glb(spec);
        std::fs::write(root.join(format!("leaves/{index:012}.glb")), &bytes).unwrap();
        rows.push_str(&row_json(index, spec, first, &bytes).to_string());
        rows.push('\n');
        first += spec.triangles;
    }
    std::fs::write(root.join("leaves.jsonl"), rows).unwrap();
    let manifest = json!({"schemaVersion":1,"kind":"full_detail_leaf_payloads",
        "report":{"leaves":specs.len(),"triangles":first},"leafManifest":"leaves.jsonl",
        "provenanceSchema":"rusty_tiles_large_source_v1"});
    std::fs::write(root.join("manifest.json"), manifest.to_string()).unwrap();
    root
}

fn run(
    leaves: &Path,
    output: &Path,
    limits: Limits,
    request: impl FnOnce(MeshTilesetRequest) -> MeshTilesetRequest,
) -> Result<MeshTilesetReport, JobError> {
    let request = request(MeshTilesetRequest::new(leaves, output));
    let parent = output.parent().unwrap();
    let mut file = File::create(output).unwrap();
    let result = publish_with(&request, limits, &mut || Ok(()), &mut file, parent);
    drop(file);
    if result.is_err() {
        std::fs::remove_file(output).unwrap();
    }
    result.map(|(report, _)| report)
}

/// Small limits that force external documents, levels splits and implicit roots.
const TIGHT: Limits = Limits {
    objects: 4,
    records: 24,
    levels: 3,
};

fn members(path: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut zip = zip::ZipArchive::new(File::open(path).unwrap()).unwrap();
    let mut out = BTreeMap::new();
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).unwrap();
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).unwrap();
        out.insert(entry.name().to_owned(), bytes);
    }
    out
}

/// An independent subtree reader: the available tiles and contents of a one-subtree,
/// `availableLevels == subtreeLevels` OCTREE subtree, from its bitstreams.
pub(crate) fn subtree_availability(
    bytes: &[u8],
    levels: u32,
) -> (BTreeSet<[u32; 4]>, BTreeSet<[u32; 4]>) {
    assert_eq!(&bytes[..4], b"subt");
    let json_len = u64::from_le_bytes(bytes[8..16].try_into().unwrap()) as usize;
    let bin_len = u64::from_le_bytes(bytes[16..24].try_into().unwrap()) as usize;
    let doc: Value = serde_json::from_slice(&bytes[24..24 + json_len]).unwrap();
    let bin_start = 24 + json_len;
    assert_eq!(bin_start % 8, 0, "8-byte subtree framing");
    let bin = &bytes[bin_start..bin_start + bin_len];
    let total: u64 = (0..levels).map(|l| 1u64 << (3 * l)).sum();
    let bits = |availability: &Value| -> Vec<bool> {
        if let Some(constant) = availability.get("constant") {
            return vec![constant == 1; total as usize];
        }
        let view = &doc["bufferViews"][availability["bitstream"].as_u64().unwrap() as usize];
        let start = view["byteOffset"].as_u64().unwrap_or(0) as usize;
        (0..total as usize)
            .map(|i| bin[start + i / 8] >> (i % 8) & 1 == 1)
            .collect()
    };
    let decode = |flags: Vec<bool>| {
        let mut set = BTreeSet::new();
        let mut offset = 0usize;
        for level in 0..levels {
            let count = 1usize << (3 * level);
            for m in 0..count {
                if flags[offset + m] {
                    let (mut x, mut y, mut z) = (0u32, 0u32, 0u32);
                    for bit in 0..level {
                        x |= ((m >> (3 * bit)) as u32 & 1) << bit;
                        y |= ((m >> (3 * bit + 1)) as u32 & 1) << bit;
                        z |= ((m >> (3 * bit + 2)) as u32 & 1) << bit;
                    }
                    set.insert([level, x, y, z]);
                }
            }
            offset += count;
        }
        set
    };
    assert_eq!(doc["childSubtreeAvailability"]["constant"], 0);
    (
        decode(bits(&doc["tileAvailability"])),
        decode(bits(&doc["contentAvailability"][0])),
    )
}

/// Walk every document from `tileset.json`, collecting explicit content members, implicit
/// roots (key, levels) and links. Panics on an unknown member.
struct Walk {
    explicit: Vec<String>,
    implicit: Vec<(String, u32, Value)>,
    documents: usize,
    /// Explicit tile objects (not links, not implicit roots).
    explicit_tiles: usize,
    /// Per document: (name, tile objects, tile levels).
    shapes: Vec<(String, usize, usize)>,
}

fn walk(archive: &BTreeMap<String, Vec<u8>>) -> Walk {
    let mut walk = Walk {
        explicit: Vec::new(),
        implicit: Vec::new(),
        documents: 0,
        explicit_tiles: 0,
        shapes: Vec::new(),
    };
    let mut pending = vec!["tileset.json".to_owned()];
    while let Some(name) = pending.pop() {
        walk.documents += 1;
        let doc: Value = serde_json::from_slice(&archive[&name]).unwrap();
        let (mut objects, mut levels) = (0, 0);
        let mut tiles = vec![(doc["root"].clone(), 1)];
        while let Some((tile, level)) = tiles.pop() {
            objects += 1;
            levels = levels.max(level);
            if let Some(tiling) = tile.get("implicitTiling") {
                let uri = tile["content"]["uri"].as_str().unwrap();
                let key = uri
                    .strip_prefix("implicit-content/")
                    .unwrap()
                    .strip_suffix("-{level}-{x}-{y}-{z}.glb")
                    .unwrap()
                    .to_owned();
                walk.implicit.push((
                    key,
                    tiling["subtreeLevels"].as_u64().unwrap() as u32,
                    tile.clone(),
                ));
                continue;
            }
            if let Some(uri) = tile["content"]["uri"].as_str() {
                if uri.ends_with(".json") {
                    assert!(
                        !uri.contains('/'),
                        "external documents sit at the archive root"
                    );
                    pending.push(uri.to_owned());
                    continue;
                }
                walk.explicit.push(uri.to_owned());
            }
            walk.explicit_tiles += 1;
            if let Some(children) = tile["children"].as_array() {
                tiles.extend(children.iter().map(|c| (c.clone(), level + 1)));
            }
        }
        walk.shapes.push((name, objects, levels));
    }
    walk
}

fn key(digits: &[u8], specs: &[Spec]) -> String {
    // A tile's (first rank, depth): the first rank of its first leaf.
    let mut first = 0u64;
    for spec in specs {
        if spec.digits.starts_with(digits) {
            return format!("f{first}-d{}", digits.len());
        }
        first += spec.triangles;
    }
    unreachable!("every tile has a leaf")
}

fn heights(specs: &[Spec]) -> BTreeMap<Vec<u8>, u32> {
    let mut out = BTreeMap::new();
    for spec in specs {
        for len in 0..=spec.digits.len() {
            let h = (spec.digits.len() - len) as u32;
            let entry = out.entry(spec.digits[..len].to_vec()).or_insert(0);
            *entry = (*entry).max(h);
        }
    }
    out
}

/// Middle limits: some shallow trees stay explicit, others are demoted, and a few documents.
const MID: Limits = Limits {
    objects: 40,
    records: 400,
    levels: 6,
};

/// Check a published forest against the generator: every implicit root is a shallow tree
/// (height 1 to 3) whose parent is a skeleton tile (or the root); every leaf outside an
/// implicit root is an explicit member, listed once; every implicit root's availability is
/// the generator's; documents are within the caps; the report counts what was written; the
/// merged validator accepts the archive with decoded containment. Returns the walk.
fn check_forest(
    specs: &[Spec],
    dir: &Path,
    output: &Path,
    report: &MeshTilesetReport,
    limits: Limits,
) -> Walk {
    let all = nodes(specs);
    let height = heights(specs);
    let by_key: BTreeMap<String, Vec<u8>> =
        all.iter().map(|d| (key(d, specs), d.clone())).collect();
    assert_eq!(report.leaves, specs.len() as u64);
    assert_eq!(report.tiles, all.len() as u64);
    let archive = members(output);
    assert_eq!(archive.len() as u64, report.members);
    let names: Vec<String> = {
        let zip = zip::ZipArchive::new(File::open(output).unwrap()).unwrap();
        zip.file_names().map(str::to_owned).collect()
    };
    let walk = walk(&archive);
    assert_eq!(walk.documents as u64, report.documents);
    assert_eq!(walk.explicit_tiles as u64, report.explicit_tiles);
    assert_eq!(walk.implicit.len() as u64, report.implicit_roots);
    assert_eq!(report.explicit_tiles + report.implicit_tiles, report.tiles);

    // Roles: implicit roots are shallow trees under a skeleton tile (or the whole tree).
    let mut roots = BTreeMap::new();
    for (k, ..) in &walk.implicit {
        let d = by_key[k].clone();
        let h = height[&d];
        assert!((1..4).contains(&h), "implicit root {k} has height {h}");
        assert!(d.is_empty() || height[&d[..d.len() - 1]] >= 4, "{k}");
        roots.insert(k.clone(), d);
    }
    let inside = |d: &Vec<u8>| roots.values().any(|r| d.starts_with(r));
    let explicit_leaves: BTreeSet<String> = specs
        .iter()
        .filter(|s| !inside(&s.digits))
        .map(|s| format!("content/{}.glb", key(&s.digits, specs)))
        .collect();
    assert_eq!(
        walk.explicit.iter().cloned().collect::<BTreeSet<_>>(),
        explicit_leaves
    );
    assert_eq!(walk.explicit.len(), explicit_leaves.len(), "listed once");

    let mut implicit_payloads = 0;
    for (k, levels, tile) in &walk.implicit {
        let root = &roots[k];
        assert_eq!(*levels, height[root] + 1);
        assert_eq!(tile["implicitTiling"]["availableLevels"], *levels);
        // Expected availability from the H1 addresses below this root.
        let mut tiles = BTreeSet::new();
        let mut contents = BTreeSet::new();
        for d in all.iter().filter(|d| d.starts_with(root)) {
            let level = (d.len() - root.len()) as u32;
            let a = axes(&d[root.len()..]);
            tiles.insert([level, a[0], a[1], a[2]]);
            if height[d] == 0 {
                contents.insert([level, a[0], a[1], a[2]]);
                let member = format!(
                    "implicit-content/{k}-{level}-{}-{}-{}.glb",
                    a[0], a[1], a[2]
                );
                assert!(archive.contains_key(&member), "{member}");
                implicit_payloads += 1;
            }
        }
        let subtree = &archive[&format!("subtrees/{k}-0-0-0-0.subtree")];
        let (available, content) = subtree_availability(subtree, *levels);
        assert_eq!(available, tiles, "tile availability of {k}");
        assert_eq!(content, contents, "content availability of {k}");
    }
    // Documents are within the caps (objects at least 1 + 8 when only units remain).
    for (name, objects, levels) in &walk.shapes {
        assert!(
            *objects as u64 <= limits.objects.max(9),
            "{name}: {objects} objects"
        );
        assert!(
            *levels as u32 <= limits.levels.max(2),
            "{name}: {levels} levels"
        );
    }
    // Every member is accounted for, tileset.json is first and the index last.
    assert_eq!(names.first().unwrap(), "tileset.json");
    assert_eq!(names.last().unwrap(), crate::TZ_INDEX_NAME);
    assert_eq!(
        archive.len(),
        walk.documents + explicit_leaves.len() + implicit_payloads + walk.implicit.len() + 1
    );
    // The leaves are byte-identical to their H8 payloads.
    for (index, spec) in specs.iter().enumerate() {
        let bytes = std::fs::read(dir.join(format!("leaves/{index:012}.glb"))).unwrap();
        let explicit = format!("content/{}.glb", key(&spec.digits, specs));
        if let Some(found) = archive.get(&explicit) {
            assert_eq!(found, &bytes);
        } else {
            assert!(archive.values().any(|v| v == &bytes));
        }
    }
    // The merged validator decodes every payload and accepts containment.
    let checked = inspect(ValidationRequest::new(output)).unwrap();
    assert!(
        checked
            .checks
            .iter()
            .any(|c| c == "decodedContentContainment"),
        "{:?}",
        checked.not_inspected
    );
    assert!(checked.content_references >= report.leaves);
    walk
}

/// The published forest of a mixed tree (spatial, packed, synthetic depth 24) is accepted by
/// the merged validator with decoded containment, and its roles, members, slots and
/// availability equal what the generator's H1 addresses predict, under the default, middle
/// and tight partitions.
#[test]
fn mixed_tree_publishes_exact_roles_slots_availability_and_validates() {
    let work = tempfile::tempdir().unwrap();
    let specs = mixed_tree(11);
    assert!(specs.iter().any(Spec::packed), "the fixture packs leaves");
    let dir = leaf_dir(work.path(), &specs);
    for (tag, limits) in [("default", LIMITS), ("mid", MID), ("tight", TIGHT)] {
        let output = work.path().join(format!("{tag}.3tz"));
        let report = run(&dir, &output, limits, |r| r).unwrap();
        assert_eq!(report.max_depth, 24);
        assert_eq!(report.synthetic_leaves, 70);
        let walk = check_forest(&specs, &dir, &output, &report, limits);
        match tag {
            // Everything fits a few documents as explicit tiles.
            "default" => assert_eq!(walk.implicit.len(), 0, "{report:?}"),
            "mid" => {
                // Both forms of shallow tree: implicit roots, and explicit leaves whose parent
                // is not a skeleton tile.
                assert!(!walk.implicit.is_empty(), "{report:?}");
                let height = heights(&specs);
                let explicit_shallow = walk.explicit.iter().any(|member| {
                    let spec = specs
                        .iter()
                        .find(|s| format!("content/{}.glb", key(&s.digits, &specs)) == *member)
                        .unwrap();
                    height[&spec.digits[..spec.digits.len() - 1]] < 4
                });
                assert!(explicit_shallow, "{report:?}");
            }
            _ => {
                assert!(
                    report.documents > 3,
                    "tight limits split documents: {report:?}"
                );
                assert!(!walk.implicit.is_empty(), "{report:?}");
            }
        }
    }
}

/// Work files this module or the runtime leave next to an output (none after any run).
fn leftovers(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with(".tiles-"))
        .collect()
}

/// Same input, different work locations and repeated runs: identical archive bytes, for the
/// default partition and for the tight one that writes several external documents. The two
/// partitions give different (both valid) layouts.
#[test]
fn archives_are_deterministic_across_work_locations() {
    let work = tempfile::tempdir().unwrap();
    let specs = mixed_tree(23);
    let dir = leaf_dir(work.path(), &specs);
    let mut layouts = Vec::new();
    for (tag, limits) in [("default", LIMITS), ("tight", TIGHT)] {
        let mut previous: Option<Vec<u8>> = None;
        for location in ["a", "b/c"] {
            let parent = work.path().join(tag).join(location);
            std::fs::create_dir_all(&parent).unwrap();
            let output = parent.join("out.3tz");
            let report = run(&dir, &output, limits, |r| r).unwrap();
            if tag == "tight" {
                assert!(report.documents > 3, "{report:?}");
            }
            let bytes = std::fs::read(&output).unwrap();
            if let Some(prior) = &previous {
                assert_eq!(prior, &bytes, "{tag}");
            }
            previous = Some(bytes);
            assert!(leftovers(&parent).is_empty());
        }
        layouts.push(previous.unwrap());
    }
    assert_ne!(layouts[0], layouts[1]);
}

/// The 3TZ ceiling refuses early (a leaf-count lower bound) and exactly (the planned count,
/// index included) before anything is staged; the exact count itself is admitted.
#[test]
fn member_ceiling_refuses_before_staging_and_admits_the_exact_count() {
    let work = tempfile::tempdir().unwrap();
    let specs = mixed_tree(5);
    let dir = leaf_dir(work.path(), &specs);
    let output = work.path().join("probe.3tz");
    let exact = run(&dir, &output, TIGHT, |r| r).unwrap().members;
    std::fs::remove_file(&output).unwrap();
    let ok = work.path().join("ok.3tz");
    let report = run(&dir, &ok, TIGHT, |r| r.with_member_ceiling(exact)).unwrap();
    assert_eq!(report.members, exact);
    for (ceiling, why) in [(exact - 1, "exactly"), (specs.len() as u64 + 1, "at least")] {
        let refused = work.path().join("refused.3tz");
        let error = run(&dir, &refused, TIGHT, |r| r.with_member_ceiling(ceiling)).unwrap_err();
        assert_eq!(error.kind(), JobErrorKind::Unsupported);
        assert!(error.message().contains(why), "{}", error.message());
        assert!(error.message().contains("3TZ member ceiling"));
        assert!(!std::fs::read_dir(work.path()).unwrap().any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".tiles-h16-")));
    }
    // Before staging: with a work parent that does not exist, the ceiling refusal still
    // comes first, while the admitted count fails only when staging begins.
    let missing = work.path().join("no-such-parent");
    for (ceiling, kind) in [
        (exact - 1, JobErrorKind::Unsupported),
        (exact, JobErrorKind::Io),
    ] {
        let target = work.path().join("direct.3tz");
        let mut file = File::create(&target).unwrap();
        let request = MeshTilesetRequest::new(&dir, &target).with_member_ceiling(ceiling);
        let error = publish_with(&request, TIGHT, &mut || Ok(()), &mut file, &missing).unwrap_err();
        assert_eq!(error.kind(), kind, "{}", error.message());
    }
    // The production ceiling is the archive codec's.
    assert_eq!(
        MeshTilesetRequest::new("a", "b").member_ceiling,
        crate::archive3tz::MEMBER_CEILING
    );
}

/// Contiguous ranks from 0 in row order, keeping each row's triangle count.
fn renumber(rows: &mut [Value]) {
    let mut first = 0u64;
    for row in rows {
        let count = row["end"].as_u64().unwrap() - row["first"].as_u64().unwrap();
        row["first"] = json!(first);
        row["end"] = json!(first + count);
        first += count;
    }
}

/// Malformed leaf directories are typed refusals: rows out of H1 preorder, a duplicated
/// octant, a packed leaf overlapping its next sibling, a rank gap, a tampered payload, a
/// leaf wider than the consumer table limit, and a totals mismatch.
#[test]
fn malformed_leaf_directories_are_refused() {
    let base = mixed_tree(3);
    type Tamper = (&'static str, JobErrorKind, fn(&Path, &mut Vec<Value>));
    let cases: [Tamper; 11] = [
        ("preorder", JobErrorKind::InvalidInput, |_, rows| {
            rows.swap(0, 1);
            renumber(rows);
        }),
        ("octant order", JobErrorKind::InvalidInput, |_, rows| {
            let last = rows.len() - 1;
            rows.swap(last - 1, last);
            renumber(rows);
        }),
        ("contiguous", JobErrorKind::InvalidInput, |_, rows| {
            rows[1]["first"] = json!(rows[1]["first"].as_u64().unwrap() + 1);
        }),
        ("file length", JobErrorKind::InvalidInput, |dir, rows| {
            let uri = rows[2]["uri"].as_str().unwrap().to_owned();
            let mut bytes = std::fs::read(dir.join(&uri)).unwrap();
            bytes.push(0);
            std::fs::write(dir.join(uri), bytes).unwrap();
        }),
        (
            "rows CesiumJS can load",
            JobErrorKind::Unsupported,
            |_, rows| {
                rows[0]["end"] = json!(rows[0]["first"].as_u64().unwrap() + 8193);
            },
        ),
        (
            "manifest.json reports",
            JobErrorKind::InvalidInput,
            |_, rows| {
                rows.pop();
            },
        ),
        (
            "invalid H1 address",
            JobErrorKind::InvalidInput,
            |_, rows| {
                rows[0]["pathLength"] = json!(1);
            },
        ),
        (
            "invalid H1 address",
            JobErrorKind::InvalidInput,
            |_, rows| {
                // Depth 32: one level beyond the 32-level hierarchy.
                let last = rows.len() - 1;
                rows[last]["pathLength"] = json!(11);
                rows[last]["depth"] = json!(32);
            },
        ),
        (
            "invalid H1 address",
            JobErrorKind::InvalidInput,
            |_, rows| {
                let depth = rows[0]["cellDepth"].as_u64().unwrap();
                rows[0]["cell"] = json!(1u64 << (3 * depth));
            },
        ),
        (
            "invalid H1 address",
            JobErrorKind::InvalidInput,
            |_, rows| {
                rows[0]["depth"] = json!(rows[0]["depth"].as_u64().unwrap() + 1);
            },
        ),
        (
            "lastOctant/packed disagree",
            JobErrorKind::InvalidInput,
            |_, rows| {
                // A synthetic run leaf never packs octants.
                let last = rows.len() - 1;
                let slot = rows[last]["path"].as_u64().unwrap() & 7;
                rows[last]["lastOctant"] = json!((slot + 1) % 8);
            },
        ),
    ];
    for (why, kind, tamper) in cases {
        let work = tempfile::tempdir().unwrap();
        let dir = leaf_dir(work.path(), &base);
        let text = std::fs::read_to_string(dir.join("leaves.jsonl")).unwrap();
        let mut rows: Vec<Value> = text
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        tamper(&dir, &mut rows);
        let text: String = rows.iter().map(|r| format!("{r}\n")).collect();
        std::fs::write(dir.join("leaves.jsonl"), text).unwrap();
        let output = work.path().join("out.3tz");
        let error = run(&dir, &output, TIGHT, |r| r).unwrap_err();
        assert_eq!(error.kind(), kind, "{why}: {}", error.message());
        assert!(error.message().contains(why), "{why}: {}", error.message());
        assert!(leftovers(work.path()).is_empty());
    }
    // A packed leaf overlapping its next sibling: two leaves under the root, the first
    // packed over octants 0..=3, the second at octant 2.
    let overlap = vec![
        Spec {
            digits: vec![0],
            cell_depth: 1,
            last_octant: 3,
            triangles: 1,
        },
        Spec {
            digits: vec![2],
            cell_depth: 1,
            last_octant: 2,
            triangles: 1,
        },
    ];
    let work = tempfile::tempdir().unwrap();
    let dir = leaf_dir(work.path(), &overlap);
    let error = run(&dir, &work.path().join("o.3tz"), TIGHT, |r| r).unwrap_err();
    assert!(
        error.message().contains("octant order"),
        "{}",
        error.message()
    );
}

/// A payload whose bytes contradict its row at the same length passes the bounds pass (which
/// reads only the JSON chunk) and is refused when it is staged, after documents have been
/// written: InvalidInput, nothing published, no work files left.
#[test]
fn payload_tampered_at_the_same_length_is_refused_while_staging() {
    let work = tempfile::tempdir().unwrap();
    let specs = mixed_tree(9);
    let dir = leaf_dir(work.path(), &specs);
    let last = format!("leaves/{:012}.glb", specs.len() - 1);
    let mut bytes = std::fs::read(dir.join(&last)).unwrap();
    *bytes.last_mut().unwrap() ^= 0x40;
    std::fs::write(dir.join(&last), bytes).unwrap();
    let out = work.path().join("published");
    std::fs::create_dir(&out).unwrap();
    let output = out.join("out.3tz");
    let failure = mesh_leaves_to_3tz(
        MeshTilesetRequest::new(&dir, &output),
        &RunControl::default(),
    )
    .unwrap_err();
    assert_eq!(failure.error.kind(), JobErrorKind::InvalidInput);
    assert!(
        failure.error.message().contains("payload bytes differ"),
        "{}",
        failure.error.message()
    );
    assert!(!output.exists());
    assert!(leftovers(&out).is_empty(), "{:?}", leftovers(&out));
}

/// Root forms: a single leaf, a short tree (explicit tiles at tileset.json, or one implicit
/// root when it exceeds the document caps), and the rigid placement, which is exactly the
/// shared placement boundary's matrix on the root and nowhere else. Local output has no
/// transform.
#[test]
fn root_forms_and_rigid_placement() {
    let placement = MeshPlacement::Wgs84 {
        anchor_degrees_metres: [133.057, -12.354, 100.],
        orientation_xyzw: [0., 0., 0.258_819_045_102_520_74, 0.965_925_826_289_068_3],
        scene_offset_metres: [3., 2., -1.],
    };
    let expected = ResolvedPlacement::resolve(&placement).unwrap().transform();
    let single = vec![Spec {
        digits: Vec::new(),
        cell_depth: 0,
        last_octant: 0,
        triangles: 4,
    }];
    let short = vec![
        Spec {
            digits: vec![1, 2],
            cell_depth: 2,
            last_octant: 2,
            triangles: 3,
        },
        Spec {
            digits: vec![1, 5],
            cell_depth: 2,
            last_octant: 7,
            triangles: 3,
        },
        Spec {
            digits: vec![6],
            cell_depth: 1,
            last_octant: 6,
            triangles: 3,
        },
    ];
    let small = Limits {
        objects: 4,
        ..LIMITS
    };
    for (specs, limits, implicit) in [
        (&single, LIMITS, false),
        (&short, LIMITS, false),
        (&short, small, true),
    ] {
        for placed in [false, true] {
            let work = tempfile::tempdir().unwrap();
            let dir = leaf_dir(work.path(), specs);
            let output = work.path().join("out.3tz");
            let report = run(&dir, &output, limits, |r| {
                if placed {
                    r.with_placement(placement.clone())
                } else {
                    r
                }
            })
            .unwrap();
            let archive = members(&output);
            let top: Value = serde_json::from_slice(&archive["tileset.json"]).unwrap();
            assert_eq!(top["asset"]["version"], "1.1");
            assert_eq!(top["root"].get("implicitTiling").is_some(), implicit);
            assert_eq!(report.implicit_roots, u64::from(implicit));
            assert_eq!(archive.keys().any(|k| k.ends_with(".subtree")), implicit);
            assert!(
                top["geometricError"].as_f64().unwrap()
                    >= top["root"]["geometricError"].as_f64().unwrap()
            );
            match top["root"].get("transform") {
                None => assert!(!placed),
                Some(transform) => {
                    assert!(placed);
                    let values: Vec<f64> = transform
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|v| v.as_f64().unwrap())
                        .collect();
                    assert_eq!(values, expected);
                    assert!(matches!(
                        report.placement,
                        MeshPlacementReport::Wgs84 { .. }
                    ));
                }
            }
            let text = String::from_utf8(archive["tileset.json"].clone()).unwrap();
            assert_eq!(text.matches("\"transform\"").count(), usize::from(placed));
            let checked = inspect(ValidationRequest::new(&output)).unwrap();
            assert!(checked
                .checks
                .iter()
                .any(|c| c == "decodedContentContainment"));
        }
    }
}

/// An unpacked spatial leaf at `digits`.
fn leaf(digits: &[u8]) -> Spec {
    Spec {
        digits: digits.to_vec(),
        cell_depth: digits.len() as u8,
        last_octant: *digits.last().unwrap_or(&0),
        triangles: 2,
    }
}

/// The first `count` leaves of a complete octree of `depth` levels below `prefix`, in
/// preorder.
fn complete(prefix: &[u8], depth: u32, count: usize) -> Vec<Spec> {
    (0..8usize.pow(depth))
        .take(count)
        .map(|index| {
            let mut d = prefix.to_vec();
            d.extend(
                (0..depth)
                    .rev()
                    .map(|level| ((index >> (3 * level)) & 7) as u8),
            );
            leaf(&d)
        })
        .collect()
}

fn publish(
    specs: &[Spec],
    limits: Limits,
) -> (tempfile::TempDir, PathBuf, MeshTilesetReport, Walk) {
    let work = tempfile::tempdir().unwrap();
    let dir = leaf_dir(work.path(), specs);
    let output = work.path().join("out.3tz");
    let report = run(&dir, &output, limits, |r| r).unwrap();
    let walk = check_forest(specs, &dir, &output, &report, limits);
    (work, output, report, walk)
}

/// A single-level shallow tree (height 1: a tile and its leaves) is written as explicit tiles
/// with no subtree, both as the whole tree and under a skeleton tile; with a cap below its
/// size it is one implicit root of two levels.
#[test]
fn single_level_shallow_trees_are_explicit_until_over_budget() {
    let whole = vec![leaf(&[0]), leaf(&[3]), leaf(&[5])];
    let (_w, _o, report, walk) = publish(&whole, LIMITS);
    assert_eq!((report.implicit_roots, report.explicit_tiles), (0, 4));
    assert_eq!((walk.documents, walk.shapes[0].2), (1, 2));

    // Under a skeleton tile: [0] has height 4 through a chain, [1] is a height-1 tree.
    let mut under = vec![leaf(&[0, 1, 2, 3])];
    under.push(leaf(&[0, 1, 2, 4, 0]));
    under.extend([leaf(&[1, 0]), leaf(&[1, 6])]);
    let (_w, _o, report, _) = publish(&under, LIMITS);
    assert_eq!(report.implicit_roots, 0, "{report:?}");
    let tight = Limits {
        objects: 3,
        ..LIMITS
    };
    let (_w, _o, report, walk) = publish(&under, tight);
    assert!(report.implicit_roots >= 1, "{report:?}");
    assert!(walk.implicit.iter().any(|(_, levels, _)| *levels == 2));
}

/// The deepest shallow tree (height 3, four levels) is written explicitly while it fits the
/// object cap and as one four-level implicit root from the first tile beyond it.
#[test]
fn deepest_shallow_tree_is_explicit_up_to_the_cap_and_implicit_beyond() {
    let cap = LIMITS.objects as usize;
    let mut fits = 0;
    while nodes(&complete(&[], 3, fits + 1)).len() <= cap {
        fits += 1;
    }
    let at = complete(&[], 3, fits);
    let beyond = complete(&[], 3, fits + 1);
    assert_eq!(nodes(&at).len(), cap);
    assert_eq!(nodes(&beyond).len(), cap + 1);

    let (_w, _o, report, walk) = publish(&at, LIMITS);
    assert_eq!(report.implicit_roots, 0);
    assert_eq!(report.explicit_tiles as usize, cap);
    assert_eq!(walk.shapes, vec![("tileset.json".to_owned(), cap, 4)]);

    let (_w, output, report, walk) = publish(&beyond, LIMITS);
    assert_eq!(report.implicit_roots, 1);
    assert_eq!(walk.implicit[0].1, 4);
    let top: Value = serde_json::from_slice(&members(&output)["tileset.json"]).unwrap();
    assert_eq!(top["root"]["implicitTiling"]["subtreeLevels"], 4);

    // The same trees under a skeleton tile next to a chain of height 4.
    let mut under = complete(&[0], 3, fits);
    under.push(leaf(&[7, 0, 0, 0, 0]));
    let (_w, _o, report, _) = publish(&under, LIMITS);
    // The tree and the chain together pass the cap: the tree is demoted, the chain stays.
    assert_eq!(report.implicit_roots, 1, "{report:?}");
}

/// A skeleton tile's children mix every role: an explicit leaf, an explicit shallow tree, an
/// implicit root (the largest shallow tree, demoted first) and a link to an external
/// document; the smaller trees stay explicit.
#[test]
fn mixed_explicit_and_implicit_siblings() {
    let mut specs = vec![leaf(&[0])];
    specs.extend([leaf(&[1, 0]), leaf(&[1, 2])]); // height 1, 3 tiles
    specs.extend(complete(&[2], 2, 24)); // height 2, 1 + 3 + 24 = 28 tiles
    specs.extend(complete(&[3], 1, 5)); // height 1, 6 tiles
                                        // A chain of height 5 under [4] with its own shallow sibling trees: a skeleton child.
    specs.extend(complete(&[4, 0, 0, 0], 2, 16));
    specs.push(leaf(&[4, 1, 0, 0, 0, 0]));
    // 44 tile objects at first: [2] is demoted for objects; the five levels below the root
    // through [4] then move [4] into its own document (its shallow trees demoted for levels).
    let limits = Limits {
        objects: 30,
        records: 4096,
        levels: 3,
    };
    let (_w, output, report, walk) = publish(&specs, limits);
    let archive = members(&output);
    let top: Value = serde_json::from_slice(&archive["tileset.json"]).unwrap();
    let children = top["root"]["children"].as_array().unwrap();
    let role = |c: &Value| {
        if c.get("implicitTiling").is_some() {
            "implicit"
        } else if c["content"]["uri"]
            .as_str()
            .is_some_and(|u| u.ends_with(".json"))
        {
            "link"
        } else if c.get("children").is_some() {
            "explicit tree"
        } else {
            "leaf"
        }
    };
    let roles: Vec<&str> = children.iter().map(role).collect();
    assert_eq!(
        roles,
        ["leaf", "explicit tree", "implicit", "explicit tree", "link"]
    );
    // The demoted tree is the largest one, [2].
    assert_eq!(
        walk.implicit
            .iter()
            .filter(|(k, ..)| k == &key(&[2], &specs))
            .count(),
        1
    );
    assert!(report.documents >= 2, "{report:?}");
}

/// Planning only (nothing written): a 2^18-leaf tree with a depth-21 chain holds no more
/// tile records at once than the fixed partition allows, the same as a 2^12-leaf tree.
#[test]
fn planner_memory_is_bounded_by_the_partition_not_the_tree() {
    struct Count {
        documents: u64,
    }
    impl Sink for Count {
        fn external(&mut self, _: Node) -> Result<Linked, JobError> {
            self.documents += 1;
            Ok(None)
        }
        fn root(&mut self, _: Node) -> Result<(), JobError> {
            Ok(())
        }
    }
    fn plan(leaves: u64) -> super::plan::Shape {
        let mut sink = Count { documents: 0 };
        let mut builder = Builder::new(&mut sink, LIMITS);
        for index in 0..leaves {
            // A complete octree of 6 levels under a 14-level chain, 8 leaves per bottom cell.
            let mut digits = Digits::ROOT;
            for level in 0..14 {
                digits.push((level % 8) as u8);
            }
            for level in (0..6).rev() {
                digits.push(((index >> (3 * level)) & 7) as u8);
            }
            let bounds = Aabb {
                lo: [index as f64, 0., 0.],
                hi: [index as f64 + 1., 1., 1.],
            };
            let row = LeafRow {
                uri: format!("leaves/{index:012}.glb"),
                sha256: [0; 32],
                bytes: 64,
                first: index * 8192,
                end: (index + 1) * 8192,
                digits,
                last_octant: (index & 7) as u8,
                packed: false,
                synthetic: 0,
                geometry_error: 0.,
            };
            builder.push(row, bounds).unwrap();
        }
        builder.finish().unwrap()
    }
    let small = plan(1 << 12);
    let large = plan(1 << 18);
    assert_eq!(large.leaves, 1 << 18);
    assert!(large.tiles > 1 << 18);
    let bound = 32 * 8 * LIMITS.records;
    assert!(large.peak_held_records <= bound, "{large:?}");
    // 64 times the leaves, the same resident records within a handful of tiles.
    assert!(
        large.peak_held_records <= small.peak_held_records + 16,
        "{small:?} {large:?}"
    );
}

/// The H8 fixture without its texture: the same triangles, no material texture.
fn untextured(input: &Path) -> PathBuf {
    let mut doc: Value = serde_json::from_slice(&std::fs::read(input).unwrap()).unwrap();
    for key in ["images", "textures", "samplers"] {
        doc.as_object_mut().unwrap().remove(key);
    }
    doc["materials"][0]["pbrMetallicRoughness"]
        .as_object_mut()
        .unwrap()
        .remove("baseColorTexture");
    let output = input.with_file_name("untextured.gltf");
    std::fs::write(&output, serde_json::to_vec(&doc).unwrap()).unwrap();
    output
}

/// Every payload's property tables are within the consumer limit; returns the number of
/// leaves (two tables: source_primitive and source_triangle). HLOD coarse tiles (one
/// representative source_primitive table, D4) are checked for the limit too but not counted.
fn leaf_tables_within_consumer_limit(archive: &BTreeMap<String, Vec<u8>>) -> usize {
    let mut leaves = 0;
    for (name, bytes) in archive {
        if !name.ends_with(".glb") {
            continue;
        }
        let length = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
        let doc: Value = serde_json::from_slice(&bytes[20..20 + length]).unwrap();
        let tables = doc["extensions"]["EXT_structural_metadata"]["propertyTables"]
            .as_array()
            .unwrap();
        let classes: Vec<_> = tables
            .iter()
            .map(|t| t["class"].as_str().unwrap())
            .collect();
        if classes.contains(&"source_triangle") {
            leaves += 1;
            assert_eq!(classes, ["source_primitive", "source_triangle"], "{name}");
        } else {
            assert_eq!(classes, ["source_primitive"], "{name}");
        }
        for table in tables {
            assert!(table["count"].as_u64().unwrap() <= 8192, "{name}");
        }
    }
    leaves
}

/// The real chain: H8 leaves (balanced, quantized positions, a synthetic run below depth
/// 21) with jobs 1 and 2 and separate scratch, published by H16: identical archives, every
/// property table within the consumer limit, accepted by the merged validator. The textured
/// balanced leaves (KTX2, required `KHR_texture_basisu`) publish too and validate with
/// containment, their KTX2 containers checked by C1.
#[test]
fn real_leaves_from_h8_publish_deterministically_and_validate() {
    use crate::mesh_large::leaves::producer::{LeafOutputRequest, LeafProfile};
    use std::num::NonZeroUsize;
    let work = tempfile::tempdir().unwrap();
    let textured = super::super::full::tests::fixture(&work.path().join("src"), 8193);
    let plain = untextured(&textured);
    let placement = MeshPlacement::Wgs84 {
        anchor_degrees_metres: [133.06, -12.32, 100.],
        orientation_xyzw: [0., 0., 0., 1.],
        scene_offset_metres: [0.; 3],
    };
    let publish = |input: &Path, jobs: usize, tag: &str| {
        let scratch = work.path().join(format!("scratch-{tag}"));
        std::fs::create_dir(&scratch).unwrap();
        let leaves = work.path().join(format!("leaves-{tag}"));
        crate::mesh_large::leaves::producer::mesh_to_leaf_payloads(
            LeafOutputRequest::new(input, &leaves, LeafProfile::Balanced)
                .with_hlod_settings(crate::mesh_large::hlod::test_settings(
                    crate::mesh_large::hlod::TRIANGLE_CAP,
                ))
                .with_jobs(NonZeroUsize::new(jobs).unwrap())
                .with_scratch_dir(&scratch),
            &RunControl::default(),
        )
        .unwrap();
        let output = work.path().join(format!("{tag}.3tz"));
        let result = mesh_leaves_to_3tz(
            MeshTilesetRequest::new(&leaves, &output).with_placement(placement.clone()),
            &RunControl::default(),
        )
        .unwrap();
        assert_eq!(result.report.triangles, 8193);
        assert_eq!(result.report.leaves, 2);
        assert!(result.report.max_depth > 21);
        output
    };
    let mut previous: Option<Vec<u8>> = None;
    for jobs in [1, 2] {
        let output = publish(&plain, jobs, &format!("plain{jobs}"));
        let archive = members(&output);
        assert_eq!(leaf_tables_within_consumer_limit(&archive), 2);
        for (name, bytes) in &archive {
            if name.ends_with(".glb") {
                let length = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
                let doc: Value = serde_json::from_slice(&bytes[20..20 + length]).unwrap();
                assert!(doc["extensionsUsed"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("KHR_mesh_quantization")));
            }
        }
        let checked = inspect(ValidationRequest::new(&output)).unwrap();
        assert!(checked
            .checks
            .iter()
            .any(|c| c == "decodedContentContainment"));
        let bytes = std::fs::read(&output).unwrap();
        if let Some(prior) = &previous {
            assert_eq!(prior, &bytes, "jobs and scratch do not change the archive");
        }
        previous = Some(bytes);
    }
    let output = publish(&textured, 1, "textured");
    assert_eq!(leaf_tables_within_consumer_limit(&members(&output)), 2);
    let checked = inspect(ValidationRequest::new(&output)).unwrap();
    assert!(checked
        .checks
        .iter()
        .any(|c| c == "decodedContentContainment"));
    assert!(checked
        .not_inspected
        .iter()
        .any(|c| c == "KTX2 level data and transcoding"));
}

/// Real H8 leaves (textured: KTX2 leaves and HLOD coarse tiles) under the tight partition must come
/// out with implicit roots; the archive validates with decoded containment, and three
/// corruptions of an implicit root's content or metadata are each refused by the validator
/// (an error, not a panic): a tile-availability bit cleared on a tile that has content, a
/// box in the subtree's `boundingBox` property moved off its parent, and one implicit
/// content GLB deleted.
#[test]
fn real_h8_leaves_under_low_limits_publish_implicit_roots_and_reject_corruption() {
    use crate::mesh_large::leaves::producer::{LeafOutputRequest, LeafProfile};
    let work = tempfile::tempdir().unwrap();
    let textured = super::super::full::tests::fixture(&work.path().join("src"), 8193);
    let scratch = work.path().join("scratch");
    std::fs::create_dir(&scratch).unwrap();
    let leaves = work.path().join("leaves");
    crate::mesh_large::leaves::producer::mesh_to_leaf_payloads(
        LeafOutputRequest::new(&textured, &leaves, LeafProfile::Balanced)
            .with_hlod_settings(crate::mesh_large::hlod::test_settings(
                crate::mesh_large::hlod::TRIANGLE_CAP,
            ))
            .with_scratch_dir(&scratch),
        &RunControl::default(),
    )
    .unwrap();
    let output = work.path().join("low.3tz");
    let report = run(&leaves, &output, TIGHT, |r| r).unwrap();
    assert!(report.implicit_roots >= 1, "{report:?}");
    assert_eq!(report.leaves, 2);
    assert!(report.max_depth > 21);
    let archive = members(&output);
    let forest = walk(&archive);
    assert!(!forest.implicit.is_empty(), "{report:?}");
    assert_eq!(forest.implicit.len() as u64, report.implicit_roots);
    // Real payloads: the KTX2 leaf is among the published GLBs.
    assert!(archive.iter().any(|(name, bytes)| {
        let needle = b"KHR_texture_basisu";
        name.ends_with(".glb") && bytes.windows(needle.len()).any(|w| w == needle)
    }));
    let checked = inspect(ValidationRequest::new(&output)).unwrap();
    assert!(checked
        .checks
        .iter()
        .any(|c| c == "decodedContentContainment"));

    let (root_key, levels, _) = &forest.implicit[0];
    let subtree_name = format!("subtrees/{root_key}-0-0-0-0.subtree");
    let subtree = &archive[&subtree_name];
    let json_len = u64::from_le_bytes(subtree[8..16].try_into().unwrap()) as usize;
    let doc: Value = serde_json::from_slice(&subtree[24..24 + json_len]).unwrap();
    let bin_start = 24 + json_len;
    let view = |index: u64| {
        let v = &doc["bufferViews"][index as usize];
        let start = bin_start + v["byteOffset"].as_u64().unwrap_or(0) as usize;
        start..start + v["byteLength"].as_u64().unwrap() as usize
    };
    let (available, contents) = subtree_availability(subtree, *levels);
    // A tile's position in the level-order (Morton within each level) bitstream, which is
    // also the order of the subtree's property-table rows.
    let index = |t: &[u32; 4]| -> usize {
        let morton = (0..t[0]).fold(0usize, |m, bit| {
            m | (((t[1] >> bit) & 1) as usize) << (3 * bit)
                | (((t[2] >> bit) & 1) as usize) << (3 * bit + 1)
                | (((t[3] >> bit) & 1) as usize) << (3 * bit + 2)
        });
        ((1usize << (3 * t[0])) - 1) / 7 + morton
    };
    // A non-root available tile with an available child (under HLOD it may be a content-less
    // single-child cell), and a non-root tile with content.
    let has_child = |t: &[u32; 4]| {
        available.iter().any(|c| {
            c[0] == t[0] + 1 && c[1] >> 1 == t[1] && c[2] >> 1 == t[2] && c[3] >> 1 == t[3]
        })
    };
    let victim = *available
        .iter()
        .find(|t| t[0] >= 1 && has_child(t))
        .unwrap();
    let content = *contents.iter().find(|t| t[0] >= 1).unwrap();
    let bit = index(&victim);

    type Archive = BTreeMap<String, Vec<u8>>;
    let damage = |case: &str, change: &dyn Fn(&mut Archive)| -> String {
        let mut edited = archive.clone();
        edited.remove("@3dtilesIndex1@");
        change(&mut edited);
        let directory = work.path().join(case);
        for (name, bytes) in &edited {
            let path = directory.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        }
        let damaged = work.path().join(format!("{case}.3tz"));
        crate::package::package(
            crate::package::PackageRequest::directory(&directory, &damaged),
            &RunControl::default(),
        )
        .unwrap();
        inspect(ValidationRequest::new(&damaged))
            .expect_err(case)
            .to_string()
    };
    let availability = damage("availability", &|edited| {
        let bytes = edited.get_mut(&subtree_name).unwrap();
        let at = view(doc["tileAvailability"]["bitstream"].as_u64().unwrap()).start;
        bytes[at + bit / 8] &= !(1 << (bit % 8));
        // Keep the declared count equal to the popcount so only the missing parent remains.
        let text = &mut bytes[24..24 + json_len];
        assert!(available.len() < 11);
        let needle = format!(
            r#""tileAvailability":{{"availableCount":{},"#,
            available.len()
        );
        let found = text
            .windows(needle.len())
            .position(|w| w == needle.as_bytes())
            .unwrap();
        text[found + needle.len() - 2] = b'0' + (available.len() as u8 - 1);
    });
    let boxed = damage("box", &|edited| {
        let bytes = edited.get_mut(&subtree_name).unwrap();
        let property = doc["propertyTables"][0]["properties"]["boundingBox"]["values"]
            .as_u64()
            .unwrap();
        // The victim's box centre x (12 f64 per row, rows in availability order).
        let row = available
            .iter()
            .filter(|t| index(t) < index(&victim))
            .count();
        let at = view(property).start + row * 96;
        let x = f64::from_le_bytes(bytes[at..at + 8].try_into().unwrap());
        bytes[at..at + 8].copy_from_slice(&(x + 1.0e6).to_le_bytes());
    });
    let missing = damage("missing", &|edited| {
        let name = format!(
            "implicit-content/{root_key}-{}-{}-{}-{}.glb",
            content[0], content[1], content[2], content[3]
        );
        edited.remove(&name).unwrap();
    });
    assert!(
        availability.contains("tile has an unavailable parent"),
        "{availability}"
    );
    assert!(boxed.contains("child bounds escape parent"), "{boxed}");
    assert!(
        missing.contains("missing archive entry: implicit-content/"),
        "{missing}"
    );
}

/// GLB bounds: quantized TRS nodes are bounded outward in exact arithmetic and taken to the
/// z-up tile frame once; a rotated node is refused.
#[test]
fn glb_bounds_are_outward_and_z_up() {
    let work = tempfile::tempdir().unwrap();
    let write = |doc: Value| {
        let mut text = serde_json::to_vec(&doc).unwrap();
        while !text.len().is_multiple_of(4) {
            text.push(b' ');
        }
        let mut out = b"glTF".to_vec();
        out.extend(2u32.to_le_bytes());
        out.extend((20 + text.len() as u32).to_le_bytes());
        out.extend((text.len() as u32).to_le_bytes());
        out.extend(b"JSON");
        out.extend(&text);
        let path = work.path().join("leaf.glb");
        std::fs::write(&path, &out).unwrap();
        (path, out.len() as u64)
    };
    let row = |bytes: u64| LeafRow {
        uri: "leaves/000000000000.glb".into(),
        sha256: [0; 32],
        bytes,
        first: 0,
        end: 1,
        digits: Digits::ROOT,
        last_octant: 0,
        packed: false,
        synthetic: 0,
        geometry_error: 0.,
    };
    let node = json!({"mesh":0,"translation":[0.1, -5.0, 1e7],"scale":[0.3, 0.001, 2.0]});
    let doc = |node: Value| {
        json!({"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],"nodes":[node],
        "meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}],
        "accessors":[{"componentType":5123,"count":2,"type":"VEC3","min":[0,7,1],"max":[65535,9,3]}]})
    };
    let (path, bytes) = write(doc(node));
    let b = glb_bounds(&path, &row(bytes)).unwrap();
    // A JSON chunk above the caller's cap is refused before it is read.
    let refused = payload_bounds(&path, "leaf", bytes, 64).unwrap_err();
    assert!(
        refused.to_string().contains("exceeds 64 bytes"),
        "{refused}"
    );
    // Exact y-up bounds: x in [0.1, 0.1 + 0.3·65535], y in [-5 + 0.007, -5 + 0.009],
    // z in [1e7 + 2, 1e7 + 6]; z-up is (x, -z, y).
    let exact_lo_x = 0.1f64;
    let exact_hi_x = 0.1 + 0.3 * 65535.;
    assert!(b.lo[0] <= exact_lo_x && b.hi[0] >= exact_hi_x - 1e-9);
    assert!(b.lo[1] <= -(1e7 + 6.) && b.hi[1] >= -(1e7 + 2.));
    assert!(b.lo[2] <= -5. + 0.007 + 1e-12 && b.hi[2] >= -5. + 0.009 - 1e-12);
    let (tile, extent) = tile_box(b).unwrap();
    for i in 0..3 {
        assert!(extent.lo[i] <= b.lo[i] && extent.hi[i] >= b.hi[i]);
        let c = tile.center()[i];
        let h = (tile.lanes()[3 + 4 * i]).abs();
        assert!(c - h <= b.lo[i] && c + h >= b.hi[i]);
    }
    let (path, bytes) = write(doc(json!({"mesh":0,"rotation":[0,0,0,1]})));
    assert!(glb_bounds(&path, &row(bytes)).is_err());
    // A flat tile (tex3's one-triangle leaves) gets a minimal, non-zero half length.
    let flat = Aabb {
        lo: [685.0, 55.25, 27.5],
        hi: [685.0009765625, 55.25, 27.5009765625],
    };
    let (tile, extent) = tile_box(flat).unwrap();
    let lanes = tile.lanes();
    assert!(
        [3, 7, 11].iter().all(|&i| lanes[i] >= MIN_HALF_LENGTH),
        "{lanes:?}"
    );
    assert!((0..3).all(|i| extent.lo[i] <= flat.lo[i] && extent.hi[i] >= flat.hi[i]));
}

// Independently authored conversion-member ceiling control.
#[test]
fn reviewer_tiler_conversion_member_is_in_early_and_exact_ceiling() {
    let one = tempfile::tempdir().unwrap();
    let specs = [Spec {
        digits: vec![],
        cell_depth: 0,
        last_octant: 0,
        triangles: 1,
    }];
    let leaves = leaf_dir(one.path(), &specs);
    let output = one.path().join("unused.3tz");
    let request = MeshTilesetRequest::new(&leaves, &output).with_member_ceiling(3);
    let mut report_called = false;
    let refused = prepare_tiler(
        &request,
        ResolvedPlacement::resolve(&request.placement).unwrap(),
        &mut || Ok(()),
        |_| {
            report_called = true;
            Ok(b"{}".to_vec())
        },
        MAX_GLB_JSON,
    );
    let error = match refused {
        Err(error) => error,
        Ok(_) => panic!("omitted conversion.json from early ceiling"),
    };
    assert_eq!(error.kind(), JobErrorKind::Unsupported);
    assert!(error.message().contains("at least"));
    assert!(!report_called);
    assert!(!output.exists());
    let request = MeshTilesetRequest::new(&leaves, &output).with_member_ceiling(4);
    let admitted = prepare_tiler(
        &request,
        ResolvedPlacement::resolve(&request.placement).unwrap(),
        &mut || Ok(()),
        |facts| {
            assert_eq!(facts.members, 4); // content + tileset + conversion + archive index
            Ok(b"{}".to_vec())
        },
        MAX_GLB_JSON,
    )
    .unwrap();
    assert_eq!(admitted.facts().members, 4);
    assert!(admitted.conversion.is_some());
    assert!(!output.exists());

    // The established mixed-tree fixture creates extra forest documents under TIGHT.
    // Baseline count excludes conversion; adding it must refuse at the exact pass.
    let mixed = tempfile::tempdir().unwrap();
    let specs = mixed_tree(5);
    let leaves = leaf_dir(mixed.path(), &specs);
    let output = mixed.path().join("unused.3tz");
    let baseline = prepare(
        &MeshTilesetRequest::new(&leaves, &output),
        TIGHT,
        &mut || Ok(()),
        false,
    )
    .unwrap();
    let exact = baseline.facts().members;
    assert!(exact >= specs.len() as u64 + 3);
    let request = MeshTilesetRequest::new(&leaves, &output).with_member_ceiling(exact);
    let refused = prepare(&request, TIGHT, &mut || Ok(()), true);
    let error = match refused {
        Err(error) => error,
        Ok(_) => panic!("omitted conversion.json from exact ceiling"),
    };
    assert_eq!(error.kind(), JobErrorKind::Unsupported);
    assert!(error.message().contains("exactly"));
    let admitted = prepare(
        &request.with_member_ceiling(exact + 1),
        TIGHT,
        &mut || Ok(()),
        true,
    )
    .unwrap();
    assert_eq!(admitted.facts().members, exact + 1);
    assert!(!output.exists());
}

/// Round 4: a chain cell that repeats its coarse payload (here inside an implicit root,
/// forced by tight caps) holds that payload's bounds, which reach past its children's.
#[test]
fn repeated_chain_content_stays_inside_its_tile() {
    struct Capture(Option<Node>);
    impl Sink for Capture {
        fn external(&mut self, _: Node) -> Result<Linked, JobError> {
            panic!("no external document expected")
        }
        fn root(&mut self, node: Node) -> Result<(), JobError> {
            self.0 = Some(node);
            Ok(())
        }
    }
    let (fine, coarse) = (3. / 4096., 4. / 4096.);
    let row = super::source::ProxyRow {
        uri: "proxies/f0-d0.glb".into(),
        sha256: [0; 32],
        bytes: 64,
        first: 0,
        end: 2,
        depth: 0,
        geometric_error: 1.,
        primitives: 1,
        sampling: (0, 0, crate::mesh_large::hlod::error::Schedule::PRODUCTION),
        encoded_depth: 0,
    };
    let payload = Aabb {
        lo: [-1., -coarse, -1.],
        hi: [1., coarse, 1.],
    };
    let proxies: super::plan::Proxies<'_> = Box::new(std::iter::once(Ok((row, payload))));
    let mut sink = Capture(None);
    let tight = Limits {
        objects: 3,
        records: 6,
        levels: 2,
    };
    let mut builder = Builder::with_proxies(&mut sink, tight, Some(proxies));
    for i in 0..2u64 {
        let mut digits = Digits::ROOT;
        for d in [0, 0, i as u8] {
            digits.push(d);
        }
        let leaf = LeafRow {
            uri: format!("leaves/{i:012}.glb"),
            sha256: [0; 32],
            bytes: 64,
            first: i,
            end: i + 1,
            digits,
            last_octant: i as u8,
            packed: false,
            synthetic: 0,
            geometry_error: 0.,
        };
        let bounds = Aabb {
            lo: [-1., -fine, -1.],
            hi: [1., fine, 1.],
        };
        builder.push(leaf, bounds).unwrap();
    }
    builder.finish().unwrap();
    let root = sink.0.unwrap();
    assert!(root.is_implicit_root());
    let chain = &root.children[0];
    assert!(chain.is_chain_cell());
    for i in 0..3 {
        assert!(chain.extent.lo[i] <= payload.lo[i] && chain.extent.hi[i] >= payload.hi[i]);
    }
}

/// D8: every entry staged under a metered work directory is admitted before it is written.
/// A run within its charge passes, and its stored model (hard-linked members counted once) is
/// within what was admitted; a tight charge is refused at the write, never past it; a write
/// that bypasses the meter is caught by the stored-model cross-check; and a charged archive
/// is admitted to its exact length.
#[test]
fn memory_backed_work_is_admitted_before_it_is_written() {
    use crate::storage_meter::{register, stored_bytes, Meter};
    let work = tempfile::tempdir().unwrap();
    let dir = leaf_dir(work.path(), &mixed_tree(23));
    let attempt = |name: &str, work_limit: u64, archive: Option<u64>| {
        let meter = Meter::new("the work directory", "test", work_limit, 4096);
        // The leaf stage's files were written before this registration: admit them.
        meter
            .admit(stored_bytes(&meter, &dir, &dir.join("none")).unwrap())
            .unwrap();
        let _registration = register(&dir, meter.clone());
        let charges = StorageCharges {
            work: Some(meter.clone()),
            archive: archive.map(|limit| Meter::new("the archive staged", "test", limit, 1)),
        };
        let output = work.path().join(name);
        let mut file = File::create(&output).unwrap();
        let request = MeshTilesetRequest::new(&dir, &output);
        let prepared = prepare(&request, LIMITS, &mut || Ok(()), false).unwrap();
        let result = write_prepared(prepared, &mut || Ok(()), &mut file, &dir, &charges);
        assert!(
            !std::fs::read_dir(&dir).unwrap().any(|e| e
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".tiles-h16-")),
            "{name}: forest staging left behind"
        );
        assert!(meter.used() <= work_limit, "{name}");
        (
            result,
            meter,
            charges.archive,
            std::fs::read(&output).unwrap(),
        )
    };
    let (result, meter, _, bytes) = attempt("free.3tz", u64::MAX, None);
    let (_, length, measured) = result.unwrap();
    let measured = measured.unwrap();
    let admitted = meter.used();
    let payloads: u64 = std::fs::read_dir(dir.join("leaves"))
        .unwrap()
        .map(|e| meter.file_bytes(e.unwrap().metadata().unwrap().len()))
        .sum();
    // Payloads once (their forest links are entries), manifests and forest documents.
    assert!(
        measured > payloads && measured <= admitted,
        "{measured} {admitted}"
    );
    let exact = Meter::new("x", "x", 0, 1).file_bytes(length);
    let (result, _, archive, again) = attempt("exact.3tz", admitted, Some(exact));
    assert_eq!(result.unwrap().1, length);
    assert_eq!((again, archive.unwrap().used()), (bytes.clone(), exact));
    // Tight charges: refused at the write that would pass them.
    for limit in [admitted - 1, measured - 4096] {
        let (result, meter, _, _) = attempt("tight.3tz", limit, None);
        assert!(result.is_err());
        let refusal = meter.refusal().expect("the meter refused");
        assert_eq!(refusal.kind(), JobErrorKind::ResourceLimit);
        assert!(refusal.to_string().contains("memory-backed"), "{refusal}");
    }
    let (result, _, _, _) = attempt("archive.3tz", admitted, Some(exact - 1));
    assert_eq!(result.unwrap_err().kind(), JobErrorKind::ResourceLimit);
    // Sensitivity: a file written around the meter is caught by the cross-check.
    let meter = Meter::new("the work directory", "test", u64::MAX, 4096);
    meter
        .admit(stored_bytes(&meter, &dir, &dir.join("none")).unwrap())
        .unwrap();
    let _registration = register(&dir, meter.clone());
    std::fs::write(dir.join("unmetered.bin"), vec![0u8; 1 << 20]).unwrap();
    let output = work.path().join("bypass.3tz");
    let mut file = File::create(&output).unwrap();
    let request = MeshTilesetRequest::new(&dir, &output);
    let prepared = prepare(&request, LIMITS, &mut || Ok(()), false).unwrap();
    let charges = StorageCharges {
        work: Some(meter),
        archive: None,
    };
    let error = write_prepared(prepared, &mut || Ok(()), &mut file, &dir, &charges).unwrap_err();
    assert!(error.to_string().contains("bypassed the meter"), "{error}");
}
