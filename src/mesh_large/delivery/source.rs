//! H8's published leaf directory, as H16 reads it: `manifest.json`, the `leaves.jsonl` rows in
//! rank order, and each leaf GLB's JSON chunk, from which the decoded bounds are computed
//! exactly and rounded outward. Leaf payload bytes are never read here; the forest writer
//! hashes each payload against its row when it publishes it.
use super::super::{config::CONSUMER_TABLE_ROWS, records::KEY_DEPTH};
use crate::{tileset_node::sum_error, JobError, JobErrorKind};
use serde::Deserialize;
use serde_json::Value;
use std::{
    fs::File,
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
};

/// The largest leaf JSON chunk read for bounds (H8's are a few KiB).
const MAX_GLB_JSON: u64 = 64 << 20;

pub(super) fn input(message: impl Into<String>) -> JobError {
    JobError::new(JobErrorKind::InvalidInput, message.into())
}

/// The H1 logical address of a tile: `depth` octant digits, most significant first (the
/// `cell_depth` spatial octants, then the synthetic run octants). A digit is the implicit
/// slot `x + 2y + 4z`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Digits {
    len: u8,
    digits: [u8; 32],
}

impl Digits {
    pub(crate) const ROOT: Self = Self {
        len: 0,
        digits: [0; 32],
    };

    pub(crate) fn len(&self) -> u32 {
        u32::from(self.len)
    }

    pub(crate) fn get(&self, level: u32) -> u8 {
        self.digits[level as usize]
    }

    pub(crate) fn push(&mut self, digit: u8) {
        self.digits[usize::from(self.len)] = digit;
        self.len += 1;
    }

    /// The three axis coordinates of the address at its own depth.
    pub(crate) fn axes(&self) -> [u32; 3] {
        let mut axes = [0u32; 3];
        for &digit in &self.digits[..usize::from(self.len)] {
            for (axis, value) in axes.iter_mut().enumerate() {
                *value = (*value << 1) | u32::from((digit >> axis) & 1);
            }
        }
        axes
    }
}

/// One `leaves.jsonl` row, checked.
#[derive(Clone, Debug)]
pub(crate) struct LeafRow {
    pub uri: String,
    pub sha256: [u8; 32],
    pub bytes: u64,
    pub first: u64,
    pub end: u64,
    pub digits: Digits,
    pub last_octant: u8,
    pub packed: bool,
    pub synthetic: u8,
    pub geometry_error: f64,
}

impl LeafRow {
    pub(crate) fn triangles(&self) -> u64 {
        self.end - self.first
    }
    pub(crate) fn depth(&self) -> u32 {
        self.digits.len()
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RowWire {
    schema_version: u32,
    uri: String,
    sha256: String,
    bytes: u64,
    first: u64,
    end: u64,
    cell_depth: u8,
    cell: u64,
    path_length: u8,
    path: u64,
    last_octant: u8,
    depth: u32,
    packed: bool,
    geometry_error: f64,
}

fn hex32(text: &str) -> Option<[u8; 32]> {
    if text.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(text.get(2 * i..2 * i + 2)?, 16).ok()?;
    }
    Some(out)
}

impl LeafRow {
    /// Check one row as H1/H8 define it (records.rs `NodeRecord`, producer receipt).
    pub(crate) fn parse(line: &str, row: u64) -> Result<Self, JobError> {
        let bad = |why: &str| input(format!("leaves.jsonl row {row}: {why}"));
        let w: RowWire = serde_json::from_str(line).map_err(|e| bad(&e.to_string()))?;
        if w.schema_version != 1 {
            return Err(bad("unsupported schemaVersion"));
        }
        let name_ok = w
            .uri
            .strip_prefix("leaves/")
            .and_then(|rest| rest.strip_suffix(".glb"))
            .is_some_and(|stem| !stem.is_empty() && stem.bytes().all(|b| b.is_ascii_digit()));
        if !name_ok {
            return Err(bad("uri is not a leaves/<digits>.glb member"));
        }
        let sha256 = hex32(&w.sha256).ok_or_else(|| bad("sha256 is not 64 hex digits"))?;
        if w.cell_depth > KEY_DEPTH
            || (w.path_length > 0 && w.cell_depth != KEY_DEPTH)
            || u32::from(w.cell_depth) + u32::from(w.path_length) != w.depth
            || w.depth > 31
            || w.cell >= 1u64 << (3 * u32::from(w.cell_depth))
            || w.path >= 1u64 << (3 * u32::from(w.path_length))
            || w.last_octant > 7
        {
            return Err(bad("invalid H1 address"));
        }
        if w.first >= w.end {
            return Err(bad("empty or reversed rank range"));
        }
        if w.end - w.first > CONSUMER_TABLE_ROWS {
            return Err(JobError::new(
                JobErrorKind::Unsupported,
                format!(
                    "leaves.jsonl row {row}: a leaf of {} triangles has a source_triangle table \
                     wider than the {CONSUMER_TABLE_ROWS} rows CesiumJS can load; re-run the leaf \
                     producer",
                    w.end - w.first
                ),
            ));
        }
        if !w.geometry_error.is_finite() || w.geometry_error < 0. {
            return Err(bad("geometryError must be finite and non-negative"));
        }
        let mut digits = Digits::ROOT;
        for (length, bits) in [(w.cell_depth, w.cell), (w.path_length, w.path)] {
            for level in (0..length).rev() {
                digits.push(((bits >> (3 * u32::from(level))) & 7) as u8);
            }
        }
        let slot = if w.depth == 0 {
            0
        } else {
            digits.get(w.depth - 1)
        };
        let packed = w.path_length == 0 && w.last_octant != slot;
        // Only a spatial leaf packs octants; a run leaf or the root covers its own slot.
        if w.last_octant < slot
            || packed != w.packed
            || ((w.path_length > 0 || w.depth == 0) && w.last_octant != slot)
        {
            return Err(bad("lastOctant/packed disagree with the address"));
        }
        Ok(Self {
            uri: w.uri,
            sha256,
            bytes: w.bytes,
            first: w.first,
            end: w.end,
            digits,
            last_octant: w.last_octant,
            packed,
            synthetic: w.path_length,
            geometry_error: w.geometry_error,
        })
    }
}

/// One `proxies.jsonl` row: an H17 coarse tile, in postorder (H16's tile-closing order).
#[derive(Clone, Debug)]
pub(crate) struct ProxyRow {
    pub uri: String,
    pub sha256: [u8; 32],
    pub bytes: u64,
    pub first: u64,
    pub end: u64,
    pub depth: u32,
    pub geometric_error: f64,
    pub primitives: u32,
    pub sample_count: Option<u64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProxyWire {
    schema_version: u32,
    uri: String,
    sha256: String,
    bytes: u64,
    first: u64,
    end: u64,
    depth: u32,
    geometric_error: f64,
    primitives: u32,
    #[serde(default)]
    error_policy: Option<String>,
    #[serde(default)]
    measurement: Option<crate::mesh_large::hlod::error::Summary>,
    #[serde(default)]
    children_error: Option<f64>,
}

impl ProxyRow {
    pub(crate) fn parse(line: &str, row: u64, hlod: bool) -> Result<Self, JobError> {
        let bad = |why: &str| input(format!("proxies.jsonl row {row}: {why}"));
        let w: ProxyWire = serde_json::from_str(line).map_err(|e| bad(&e.to_string()))?;
        if w.schema_version != 1 {
            return Err(bad("unsupported schemaVersion"));
        }
        match (w.error_policy.as_deref(), w.measurement.as_ref()) {
            (Some(crate::mesh_large::hlod::error::POLICY), Some(summary)) => {
                let children = w
                    .children_error
                    .ok_or_else(|| bad("S2 row has no childrenError"))?;
                summary
                    .validate(w.geometric_error, children)
                    .map_err(|e| bad(&e.to_string()))?;
                if w.depth == 0
                    && summary.area_samples_per_direction
                        < crate::mesh_large::hlod::error::MIN_DEPTH
                {
                    return Err(bad("incomplete root sampling"));
                }
            }
            (None, None) if !hlod => {} // Historical H17 rows, removed with that producer in S6.
            _ => return Err(bad("missing or unsupported sampled-error policy/summary")),
        }
        let expected = format!("proxies/f{}-d{}.glb", w.first, w.depth);
        if w.uri != expected {
            return Err(bad("uri is not proxies/f<first>-d<depth>.glb"));
        }
        if w.first >= w.end || w.depth > 30 {
            return Err(bad("invalid rank range or depth"));
        }
        if !w.geometric_error.is_finite() || w.geometric_error < 0. {
            return Err(bad("geometricError must be finite and non-negative"));
        }
        if u64::from(w.primitives) > CONSUMER_TABLE_ROWS || w.primitives == 0 {
            return Err(bad("primitive table outside 1..=8192 rows"));
        }
        Ok(Self {
            uri: w.uri,
            sha256: hex32(&w.sha256).ok_or_else(|| bad("sha256 is not 64 hex digits"))?,
            bytes: w.bytes,
            first: w.first,
            end: w.end,
            depth: w.depth,
            geometric_error: w.geometric_error,
            primitives: w.primitives,
            sample_count: w.measurement.as_ref().map(|s| s.area_samples_per_direction),
        })
    }
}

/// An axis-aligned box in the tile frame (z-up metres), closed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Aabb {
    pub lo: [f64; 3],
    pub hi: [f64; 3],
}

impl Aabb {
    pub(crate) fn union(self, other: Self) -> Self {
        Self {
            lo: std::array::from_fn(|i| self.lo[i].min(other.lo[i])),
            hi: std::array::from_fn(|i| self.hi[i].max(other.hi[i])),
        }
    }
}

/// `a + b` rounded up / down (exact TwoSum residual).
pub(crate) fn add_up(a: f64, b: f64) -> f64 {
    let sum = a + b;
    if sum_error(a, b) > 0. {
        sum.next_up()
    } else {
        sum
    }
}
pub(crate) fn add_down(a: f64, b: f64) -> f64 {
    let sum = a + b;
    if sum_error(a, b) < 0. {
        sum.next_down()
    } else {
        sum
    }
}
/// `a · b` rounded up / down (exact FMA residual).
fn mul_up(a: f64, b: f64) -> f64 {
    let product = a * b;
    if a.mul_add(b, -product) > 0. {
        product.next_up()
    } else {
        product
    }
}
fn mul_down(a: f64, b: f64) -> f64 {
    let product = a * b;
    if a.mul_add(b, -product) < 0. {
        product.next_down()
    } else {
        product
    }
}

fn numbers<const N: usize>(value: Option<&Value>, default: f64) -> Option<[f64; N]> {
    match value {
        None => Some([default; N]),
        Some(Value::Array(items)) if items.len() == N => {
            let mut out = [0.; N];
            for (slot, item) in out.iter_mut().zip(items) {
                *slot = item.as_f64().filter(|v| v.is_finite())?;
            }
            Some(out)
        }
        _ => None,
    }
}

/// The decoded positions' box of an H8 leaf GLB in the tile frame, from its JSON chunk only.
///
/// H8 writes one scene-root node per primitive with at most `translation` and `scale`, and
/// POSITION accessors whose `min`/`max` bound the stored values (quantized unnormalized
/// `UNSIGNED_SHORT` codes, or `FLOAT`). Each decoded coordinate is `t + s · c`; the bound is
/// evaluated with outward rounding, then taken from glTF y-up to the tile frame's z-up as
/// `(x, y, z) -> (x, -z, y)`, the conversion C2 applies exactly once.
pub(crate) fn glb_bounds(path: &Path, row: &LeafRow) -> Result<Aabb, JobError> {
    payload_bounds(path, &row.uri, row.bytes)
}

/// [`glb_bounds`] for any H8/H17 payload of `bytes` bytes.
pub(crate) fn payload_bounds(path: &Path, uri: &str, bytes: u64) -> Result<Aabb, JobError> {
    let bad = |why: &str| input(format!("payload {uri}: {why}"));
    let io = |e| JobError::io("read leaf GLB", path, e);
    let mut file = File::open(path).map_err(io)?;
    let length = file.metadata().map_err(io)?.len();
    if length != bytes {
        return Err(bad("file length differs from its manifest row"));
    }
    let mut header = [0u8; 20];
    file.read_exact(&mut header).map_err(io)?;
    let u32_at = |offset: usize| u32::from_le_bytes(header[offset..offset + 4].try_into().unwrap());
    if &header[..4] != b"glTF" || u32_at(4) != 2 || u64::from(u32_at(8)) != length {
        return Err(bad("not a GLB 2.0 of its declared length"));
    }
    let json_length = u64::from(u32_at(12));
    if &header[16..20] != b"JSON" || json_length + 20 > length {
        return Err(bad("first GLB chunk is not JSON"));
    }
    if json_length > MAX_GLB_JSON {
        return Err(bad("GLB JSON chunk exceeds 64 MiB"));
    }
    let mut text = vec![0u8; json_length as usize];
    file.read_exact(&mut text).map_err(io)?;
    let document: Value = serde_json::from_slice(&text).map_err(|e| bad(&e.to_string()))?;
    let scene = document["scene"].as_u64().unwrap_or(0) as usize;
    let roots = document["scenes"][scene]["nodes"]
        .as_array()
        .ok_or_else(|| bad("no scene nodes"))?;
    let mut bounds: Option<([f64; 3], [f64; 3])> = None;
    for root in roots {
        let node = &document["nodes"][root.as_u64().ok_or_else(|| bad("bad node index"))? as usize];
        let object = node.as_object().ok_or_else(|| bad("missing node"))?;
        if object
            .keys()
            .any(|k| !matches!(k.as_str(), "mesh" | "translation" | "scale"))
        {
            return Err(bad("a scene node is not an H8 translation/scale node"));
        }
        let translation =
            numbers::<3>(node.get("translation"), 0.).ok_or_else(|| bad("bad node translation"))?;
        let scale = numbers::<3>(node.get("scale"), 1.).ok_or_else(|| bad("bad node scale"))?;
        let Some(mesh) = node.get("mesh") else {
            continue;
        };
        let mesh =
            &document["meshes"][mesh.as_u64().ok_or_else(|| bad("bad mesh index"))? as usize];
        for primitive in mesh["primitives"]
            .as_array()
            .ok_or_else(|| bad("mesh has no primitives"))?
        {
            let accessor = &document["accessors"][primitive["attributes"]["POSITION"]
                .as_u64()
                .ok_or_else(|| bad("primitive has no POSITION"))?
                as usize];
            let float = match accessor["componentType"].as_u64() {
                Some(5126) => true,
                Some(5123) if accessor["normalized"] != Value::Bool(true) => false,
                _ => {
                    return Err(bad(
                        "POSITION is neither FLOAT nor unnormalized UNSIGNED_SHORT",
                    ))
                }
            };
            let min = numbers::<3>(accessor.get("min"), f64::NAN)
                .filter(|v| v.iter().all(|c| c.is_finite()))
                .ok_or_else(|| bad("POSITION has no min"))?;
            let max = numbers::<3>(accessor.get("max"), f64::NAN)
                .filter(|v| v.iter().all(|c| c.is_finite()))
                .ok_or_else(|| bad("POSITION has no max"))?;
            if !float && (0..3).any(|a| min[a].fract() != 0. || max[a].fract() != 0.) {
                return Err(bad("quantized POSITION bounds are not integers"));
            }
            let mut lo = [0.; 3];
            let mut hi = [0.; 3];
            for a in 0..3 {
                if min[a] > max[a] {
                    return Err(bad("POSITION min exceeds max"));
                }
                let low = mul_down(scale[a], min[a]).min(mul_down(scale[a], max[a]));
                let high = mul_up(scale[a], min[a]).max(mul_up(scale[a], max[a]));
                lo[a] = add_down(translation[a], low);
                hi[a] = add_up(translation[a], high);
            }
            bounds = Some(match bounds {
                None => (lo, hi),
                Some((l, h)) => (
                    std::array::from_fn(|a| l[a].min(lo[a])),
                    std::array::from_fn(|a| h[a].max(hi[a])),
                ),
            });
        }
    }
    let (lo, hi) = bounds.ok_or_else(|| bad("leaf draws no positions"))?;
    if lo.iter().chain(&hi).any(|v| !v.is_finite()) {
        return Err(bad("leaf bounds are not finite"));
    }
    Ok(Aabb {
        lo: [lo[0], -hi[2], lo[1]],
        hi: [hi[0], -lo[2], hi[1]],
    })
}

/// The `manifest.json` totals of a leaf directory.
#[derive(Clone, Copy, Debug)]
pub(crate) struct LeafTotals {
    pub leaves: u64,
    pub triangles: u64,
}

/// A published H8 leaf directory.
pub(crate) struct LeafDirectory {
    root: PathBuf,
    pub totals: LeafTotals,
    /// H17 coarse tiles are present (`proxyManifest`).
    pub proxies: bool,
    hlod: bool,
}

impl LeafDirectory {
    pub(crate) fn open(root: &Path) -> Result<Self, JobError> {
        let path = root.join("manifest.json");
        let bytes =
            std::fs::read(&path).map_err(|e| JobError::io("read leaf manifest", &path, e))?;
        let manifest: Value =
            serde_json::from_slice(&bytes).map_err(|e| input(format!("manifest.json: {e}")))?;
        if manifest["schemaVersion"] != 1
            || manifest["kind"] != "full_detail_leaf_payloads"
            || manifest["leafManifest"] != "leaves.jsonl"
            || manifest["provenanceSchema"] != "rusty_tiles_large_source_v1"
        {
            return Err(input(
                "manifest.json is not an H8 full_detail_leaf_payloads manifest (schema 1)",
            ));
        }
        let count = |key: &str| {
            manifest["report"][key]
                .as_u64()
                .ok_or_else(|| input(format!("manifest.json report has no {key}")))
        };
        let totals = LeafTotals {
            leaves: count("leaves")?,
            triangles: count("triangles")?,
        };
        if totals.leaves == 0 {
            return Err(input("the leaf directory has no leaves"));
        }
        let proxies = match manifest.get("proxyManifest") {
            None => false,
            Some(v) if v == "proxies.jsonl" => true,
            Some(_) => return Err(input("manifest.json proxyManifest is not proxies.jsonl")),
        };
        Ok(Self {
            root: root.to_owned(),
            totals,
            proxies,
            hlod: manifest.get("hlod").is_some(),
        })
    }

    pub(crate) fn path(&self, row: &LeafRow) -> PathBuf {
        self.root.join(&row.uri)
    }

    pub(crate) fn member(&self, uri: &str) -> PathBuf {
        self.root.join(uri)
    }

    /// The H17 proxy rows with their decoded bounds, in file (post)order; empty when the
    /// directory has none.
    pub(crate) fn proxy_rows(&self) -> Result<super::plan::Proxies<'_>, JobError> {
        if !self.proxies {
            return Ok(Box::new(std::iter::empty()));
        }
        let path = self.root.join("proxies.jsonl");
        let file = File::open(&path).map_err(|e| JobError::io("open proxy rows", &path, e))?;
        let mut lines = BufReader::new(file).lines();
        let mut row = 0u64;
        let mut samples = [0u64; 32];
        let mut finished = false;
        Ok(Box::new(std::iter::from_fn(move || {
            if finished {
                return None;
            }
            let line = match lines.next() {
                Some(line) => line,
                None => {
                    finished = true;
                    return samples
                        .iter()
                        .enumerate()
                        .find(|(_, n)| **n > 0 && **n < crate::mesh_large::hlod::error::MIN_DEPTH)
                        .map(|(depth, n)| {
                            Err(input(format!(
                                "incomplete HLOD depth {depth} sampling: {n}/200000 per direction"
                            )))
                        });
                }
            };
            row += 1;
            Some(
                line.map_err(|e| JobError::io("read proxy rows", &path, e))
                    .and_then(|line| ProxyRow::parse(&line, row, self.hlod))
                    .and_then(|proxy| {
                        if let Some(n) = proxy.sample_count {
                            samples[proxy.depth as usize] = samples[proxy.depth as usize]
                                .checked_add(n)
                                .ok_or_else(|| input("HLOD sample count overflow"))?;
                        }
                        let bounds =
                            payload_bounds(&self.member(&proxy.uri), &proxy.uri, proxy.bytes)?;
                        Ok((proxy, bounds))
                    }),
            )
        })))
    }

    /// The rows with their decoded bounds, streamed in file order.
    pub(crate) fn leaves(
        &self,
    ) -> Result<impl Iterator<Item = Result<(LeafRow, Aabb), JobError>> + '_, JobError> {
        let path = self.root.join("leaves.jsonl");
        let file = File::open(&path).map_err(|e| JobError::io("open leaf rows", &path, e))?;
        let mut lines = BufReader::new(file).lines();
        let mut row = 0u64;
        Ok(std::iter::from_fn(move || {
            let line = lines.next()?;
            row += 1;
            Some(
                line.map_err(|e| JobError::io("read leaf rows", &path, e))
                    .and_then(|line| LeafRow::parse(&line, row))
                    .and_then(|leaf| {
                        let bounds = glb_bounds(&self.path(&leaf), &leaf)?;
                        Ok((leaf, bounds))
                    }),
            )
        }))
    }
}
