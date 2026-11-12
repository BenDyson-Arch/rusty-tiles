//! Decoded views of published tile content, for consumers that read neither
//! `EXT_meshopt_compression` nor `KHR_texture_basisu` (issue #329).
//!
//! A view is for display and inspection. It is not a reversible round trip:
//! nothing here re-encodes a tile, and an edited view cannot be published back.
//!
//! Contract:
//! - Input is a `.3tz` archive (admitted by the stored-archive reader) or an
//!   unpacked tileset directory. Its `tileset.json`, external documents and
//!   implicit subtrees are expanded once by [`crate::implicit::expand_tileset`];
//!   a member is addressed by the content URI that expansion yields.
//! - Content is one GLB with an embedded binary chunk. Meshopt views are
//!   decoded by the linked meshopt decoder (modes ATTRIBUTES, TRIANGLES and
//!   INDICES; filters NONE and OCTAHEDRAL). Basis Universal KTX2 pages are
//!   transcoded at level 0 by the vendored transcoder; other levels are dropped.
//!   Any other codec, external resource or legacy tile format is `Unsupported`.
//! - The plain GLB keeps every other part of the document: accessors (so
//!   `KHR_mesh_quantization` stays), samplers, materials, `EXT_mesh_features`
//!   and `EXT_structural_metadata`.
//! - Arrays are in the tile frame: node transforms applied, then glTF Y-up
//!   turned to 3D Tiles Z-up (`x, -z, y`). The tile's accumulated
//!   `transform` maps that frame to the tileset's world (ECEF for placed output).
//!   Indices keep counter-clockwise front faces: under a node transform with a
//!   negative determinant, corners 1 and 2 of every triangle are swapped.
//! - Malformed content is `InvalidInput`; an unknown member is `InvalidRequest`.
//! - Every tile is bounded before allocation by fixed limits ([`TileLimit`]):
//!   member bytes, JSON text, the bytes held at once by one decode (see
//!   [`Budget`]), scene work (node visits and primitive instances), and page edge
//!   and texels. A refusal is `ResourceLimit` with the [`TileLimit`] as its cause.
//! - Directory input reads regular files only, never through a symlink below
//!   the given root.
use crate::{
    archive3tz::{ReadError, ReadLimits, StoredArchive},
    content_integrity::meshopt::{self, Filter, Mode},
    JobError, JobErrorKind,
};
use image::ImageEncoder as _;
use serde_json::{Map, Value};
use std::{
    collections::BTreeSet,
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
};

/// Largest member read from a tileset, in bytes.
const MEMBER_LIMIT: u64 = 1 << 30;
/// Largest total of bytes held at once for one tile's decode (see [`Budget`]).
const DECODED_LIMIT: u64 = 1 << 30;
/// Largest JSON text parsed at once: a GLB JSON chunk or one tileset document. Real
/// tiles hold kilobytes; producer documents stay under 2 MiB.
const JSON_LIMIT: u64 = 64 << 20;
/// Parsed `serde_json::Value` bytes per JSON text byte, worst case: `[0,0,...]`
/// spends a 32-byte value on every two bytes.
const JSON_EXPANSION: u64 = 16;
/// Largest number of node visits plus primitive instances in one scene walk.
const SCENE_WORK_LIMIT: u64 = 1 << 16;
/// Deepest node nesting walked in one GLB scene.
const NODE_DEPTH: usize = 64;
/// Alignment of every view in a written plain GLB (covers 8-byte metadata columns).
const VIEW_ALIGN: usize = 8;
const MESHOPT: &str = "EXT_meshopt_compression";
const BASISU: &str = "KHR_texture_basisu";
/// glTF Y-up to 3D Tiles Z-up, column-major.
const Y_UP_TO_Z_UP: [f64; 16] = [
    1., 0., 0., 0., 0., 0., 1., 0., 0., -1., 0., 0., 0., 0., 0., 1.,
];
const IDENTITY: [f64; 16] = [
    1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
];

fn error(kind: JobErrorKind, message: impl Into<String>) -> JobError {
    JobError::new(kind, message)
}
fn invalid(message: impl Into<String>) -> JobError {
    error(JobErrorKind::InvalidInput, message)
}
fn unsupported(message: impl Into<String>) -> JobError {
    error(JobErrorKind::Unsupported, message)
}
fn limit(message: impl Into<String>) -> JobError {
    error(JobErrorKind::ResourceLimit, message)
}

/// A fixed per-tile limit that refused a decode, attached as the `ResourceLimit`
/// error's cause. `limit` is `memberBytes`, `jsonBytes`, `decodedBytes`,
/// `sceneWork`, `pageEdge` or `pageTexels`; `observed` is the refused quantity
/// (for `decodedBytes`, what would have been held).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TileLimit {
    pub limit: &'static str,
    pub value: u64,
    pub observed: u64,
}

impl std::fmt::Display for TileLimit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "tile view {} limit {} exceeded ({})",
            self.limit, self.value, self.observed
        )
    }
}

impl std::error::Error for TileLimit {}

fn exceeded(limit: &'static str, value: u64, observed: u64) -> JobError {
    let cause = TileLimit {
        limit,
        value,
        observed,
    };
    JobError::with_cause(JobErrorKind::ResourceLimit, cause.to_string(), cause)
}

/// What one decode may hold at once and walk. Every allocation is charged before
/// it is made and refused when the bytes held would pass the limit; transient
/// buffers (the member, widened accessor values, decoder scratch, pages that are
/// re-encoded) are credited when they are freed. Held for the whole decode: the
/// parsed document ([`JSON_EXPANSION`] times its JSON), the decoded views and the
/// outputs. The caller's copy is charged as it happens for a GLB, and for arrays
/// as the largest single primitive, page or property table (the Python binding
/// releases each after copying it).
struct Budget {
    limit: u64,
    held: u64,
    peak: u64,
    work: u64,
    unit: u64,
    largest_unit: u64,
}

impl Default for Budget {
    fn default() -> Self {
        Self::with_limit(DECODED_LIMIT)
    }
}

impl Budget {
    fn with_limit(limit: u64) -> Self {
        Self {
            limit,
            held: 0,
            peak: 0,
            work: 0,
            unit: 0,
            largest_unit: 0,
        }
    }

    fn bytes(&mut self, bytes: u64) -> Result<(), JobError> {
        let total = self.held.saturating_add(bytes);
        if total > self.limit {
            return Err(exceeded("decodedBytes", self.limit, total));
        }
        self.held = total;
        self.peak = self.peak.max(total);
        Ok(())
    }

    /// Returns `bytes` freed by the decode.
    fn credit(&mut self, bytes: u64) {
        self.held = self.held.saturating_sub(bytes);
    }

    /// `count * width` bytes, overflow included in the refusal.
    fn elements(&mut self, count: usize, width: usize) -> Result<(), JobError> {
        self.bytes((count as u64).saturating_mul(width as u64))
    }

    /// An output of `count` elements of `width` bytes, part of the current unit.
    fn output(&mut self, count: usize, width: usize) -> Result<(), JobError> {
        let bytes = (count as u64).saturating_mul(width as u64);
        self.bytes(bytes)?;
        self.unit = self.unit.saturating_add(bytes);
        Ok(())
    }

    /// Counts already charged bytes as output of the current unit.
    fn adopt(&mut self, bytes: u64) {
        self.unit = self.unit.saturating_add(bytes);
    }

    /// Closes one primitive, page or table: the caller copies and frees units one by one.
    fn end_unit(&mut self) {
        self.largest_unit = self.largest_unit.max(self.unit);
        self.unit = 0;
    }

    /// The caller's copy in flight: its largest unit.
    fn caller_copy(&mut self) -> Result<(), JobError> {
        self.end_unit();
        self.bytes(self.largest_unit)
    }

    fn work(&mut self) -> Result<(), JobError> {
        self.work += 1;
        if self.work > SCENE_WORK_LIMIT {
            return Err(exceeded("sceneWork", SCENE_WORK_LIMIT, self.work));
        }
        Ok(())
    }
}

/// Refuses a page with an edge over [`crate::basisu_ktx2::MAX_EDGE`] or more than
/// [`crate::basisu_ktx2::MAX_TEXELS`] texels, from its header size.
fn check_page(width: u32, height: u32) -> Result<u64, JobError> {
    let edge = width.max(height);
    if edge > crate::basisu_ktx2::MAX_EDGE {
        return Err(exceeded(
            "pageEdge",
            u64::from(crate::basisu_ktx2::MAX_EDGE),
            u64::from(edge),
        ));
    }
    let texels = u64::from(width) * u64::from(height);
    let max = crate::basisu_ktx2::MAX_TEXELS;
    if texels > max {
        return Err(exceeded("pageTexels", max, texels));
    }
    Ok(texels)
}

/// Refuses a JSON text over [`JSON_LIMIT`].
fn check_json(length: usize) -> Result<(), JobError> {
    if length as u64 > JSON_LIMIT {
        return Err(exceeded("jsonBytes", JSON_LIMIT, length as u64));
    }
    Ok(())
}

/// How a content-bearing tile sits in the expanded hierarchy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TileRole {
    /// No child tiles. For mesh output, an exact leaf with `source_triangle` IDs.
    Leaf,
    /// Refined by child tiles. For mesh output, a coarse (HLOD) tile.
    Coarse,
}

impl TileRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Leaf => "leaf",
            Self::Coarse => "coarse",
        }
    }
}

/// One content URI of the expanded tileset.
#[derive(Clone, Debug, PartialEq)]
pub struct TileEntry {
    pub uri: String,
    /// Hops from the root of the expanded hierarchy (the root is 0). This is not the
    /// cell level a producer may write into member names.
    pub depth: u32,
    pub role: TileRole,
    pub geometric_error: f64,
    /// Product of the tile transforms from the root down to this tile, column-major.
    pub transform: [f64; 16],
}

enum Source {
    Archive(Box<StoredArchive<File>>),
    Directory(PathBuf),
}

fn read_error(failure: ReadError) -> JobError {
    match failure {
        ReadError::InvalidInput(message) => invalid(message),
        ReadError::Unsupported(message) => unsupported(message),
        ReadError::ResourceLimit(message) => limit(message),
        ReadError::Cap { message, .. } => limit(message),
        ReadError::MissingMember(name) => invalid(format!("missing archive member: {name}")),
        ReadError::Io(cause) => error_io(cause),
        ReadError::Scratch(cause) => error(JobErrorKind::Io, format!("archive catalog: {cause}")),
    }
}
fn error_io(cause: std::io::Error) -> JobError {
    error(JobErrorKind::Io, cause.to_string())
}

/// A member URI as relative path segments: no scheme, query, fragment, escape,
/// absolute path, empty, `.` or `..` segment.
fn segments(uri: &str) -> Result<Vec<&str>, JobError> {
    let parts: Vec<&str> = uri.split('/').collect();
    let plain = !uri.contains(['\\', ':', '?', '#', '%', '\0'])
        && parts
            .iter()
            .all(|p| !p.is_empty() && *p != "." && *p != "..");
    if plain {
        Ok(parts)
    } else {
        Err(unsupported(format!("unsupported content URI {uri:?}")))
    }
}

impl Source {
    fn open(path: &Path) -> Result<Self, JobError> {
        let metadata = fs::metadata(path).map_err(|e| JobError::io("open tileset", path, e))?;
        if metadata.is_dir() {
            return Ok(Self::Directory(path.to_owned()));
        }
        let file = File::open(path).map_err(|e| JobError::io("open tileset", path, e))?;
        let length = file
            .metadata()
            .map_err(|e| JobError::io("open tileset", path, e))?
            .len();
        let limits = ReadLimits {
            source_archive_bytes: None,
            central_directory_bytes: None,
            archive_entries: None,
            member_bytes: length,
            archive_stored_bytes: None,
        };
        StoredArchive::new(file, length, &limits)
            .map(|archive| Self::Archive(Box::new(archive)))
            .map_err(read_error)
    }

    fn read(&mut self, uri: &str) -> Result<Vec<u8>, JobError> {
        let parts = segments(uri)?;
        match self {
            Self::Archive(archive) => {
                let size = archive.member(uri).map_err(read_error)?.size;
                if size > MEMBER_LIMIT {
                    return Err(exceeded("memberBytes", MEMBER_LIMIT, size));
                }
                archive.read_member(uri, MEMBER_LIMIT).map_err(read_error)
            }
            Self::Directory(root) => read_regular(root, &parts, uri),
        }
    }
}

/// Reads `root/parts...` as a regular file. No component below `root` may be a
/// symlink, and the member must be a regular file (never a FIFO, device or
/// directory). On Unix the open itself refuses a final symlink and cannot block.
fn read_regular(root: &Path, parts: &[&str], uri: &str) -> Result<Vec<u8>, JobError> {
    let special = || unsupported(format!("member {uri:?} is a symlink or not a regular file"));
    let mut path = root.to_owned();
    for part in parts {
        path.push(part);
        let metadata =
            fs::symlink_metadata(&path).map_err(|e| JobError::io("read member", &path, e))?;
        if metadata.file_type().is_symlink() {
            return Err(special());
        }
    }
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options
        .open(&path)
        .map_err(|e| JobError::io("read member", &path, e))?;
    let metadata = file
        .metadata()
        .map_err(|e| JobError::io("read member", &path, e))?;
    if !metadata.is_file() {
        return Err(special());
    }
    if metadata.len() > MEMBER_LIMIT {
        return Err(exceeded("memberBytes", MEMBER_LIMIT, metadata.len()));
    }
    let mut bytes = Vec::new();
    file.take(MEMBER_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| JobError::io("read member", &path, e))?;
    if bytes.len() as u64 > MEMBER_LIMIT {
        return Err(exceeded("memberBytes", MEMBER_LIMIT, bytes.len() as u64));
    }
    Ok(bytes)
}

/// An opened tileset: its source and expanded content listing.
pub struct TileSet {
    source: Source,
    tiles: Vec<TileEntry>,
    root_transform: [f64; 16],
}

impl TileSet {
    /// Opens a `.3tz` archive or an unpacked tileset directory and expands its hierarchy.
    pub fn open(path: &Path) -> Result<Self, JobError> {
        let mut source = Source::open(path)?;
        let manifest = source.read("tileset.json")?;
        check_json(manifest.len())?;
        let manifest: Value =
            serde_json::from_slice(&manifest).map_err(|e| invalid(format!("tileset.json: {e}")))?;
        // The expander reports through the crate error; keep the first typed read failure.
        let mut failure = None;
        let expanded = crate::implicit::expand_tileset(&manifest, |uri| {
            let document = uri.to_ascii_lowercase().ends_with(".json");
            let read = source.read(uri).and_then(|bytes| {
                if document {
                    check_json(bytes.len())?;
                }
                Ok(bytes)
            });
            read.map_err(|error| {
                let message = error.to_string();
                failure.get_or_insert(error);
                crate::Error::Message(message)
            })
        });
        let expanded = match (expanded, failure) {
            (Ok(expanded), _) => expanded,
            (Err(_), Some(error)) => return Err(error),
            (Err(error), None) => return Err(expansion_error(error)),
        };
        let root = &expanded["root"];
        let root_transform = transform(root)?;
        let mut tiles = Vec::new();
        collect(root, 0, &IDENTITY, &mut tiles)?;
        Ok(Self {
            source,
            tiles,
            root_transform,
        })
    }

    /// Content-bearing tiles in hierarchy order (parent before children).
    pub fn tiles(&self) -> &[TileEntry] {
        &self.tiles
    }

    /// The root tile's own transform, column-major (identity when absent).
    pub fn root_transform(&self) -> [f64; 16] {
        self.root_transform
    }

    fn entry(&self, uri: &str) -> Result<TileEntry, JobError> {
        self.tiles
            .iter()
            .find(|tile| tile.uri == uri)
            .cloned()
            .ok_or_else(|| {
                error(
                    JobErrorKind::InvalidRequest,
                    format!("{uri:?} is not a content URI of this tileset"),
                )
            })
    }

    fn plain(&mut self, uri: &str, budget: &mut Budget) -> Result<(TileEntry, Plain), JobError> {
        let entry = self.entry(uri)?;
        let bytes = self.source.read(uri)?;
        // The member is held until its views are decoded.
        budget.elements(bytes.len(), 1)?;
        let plain = Plain::decode(&bytes, budget)?;
        budget.credit(bytes.len() as u64);
        Ok((entry, plain))
    }

    /// The member as a plain glTF 2.0 GLB: meshopt views decoded, Basis pages
    /// transcoded to PNG at level 0, everything else kept.
    pub fn plain_glb(&mut self, uri: &str) -> Result<Vec<u8>, JobError> {
        let mut budget = Budget::default();
        self.plain(uri, &mut budget)?.1.into_glb(&mut budget)
    }

    /// The member's primitives, pages and property tables as typed arrays.
    pub fn arrays(&mut self, uri: &str) -> Result<DecodedTile, JobError> {
        let mut budget = Budget::default();
        let (entry, plain) = self.plain(uri, &mut budget)?;
        plain.into_arrays(entry, self.root_transform, &mut budget)
    }
}

fn expansion_error(failure: crate::Error) -> JobError {
    use crate::validate::ValidationFailure;
    match failure {
        crate::Error::Validation(ValidationFailure::ResourceLimit(refusal)) => {
            limit(refusal.to_string())
        }
        crate::Error::Validation(ValidationFailure::Unsupported(message)) => {
            unsupported(message.to_string())
        }
        crate::Error::Io(error) => error_io(error),
        other => invalid(format!("tileset hierarchy: {other}")),
    }
}

fn finite_matrix(value: &Value) -> Option<[f64; 16]> {
    let values = value.as_array().filter(|v| v.len() == 16)?;
    let mut matrix = [0.; 16];
    for (out, value) in matrix.iter_mut().zip(values) {
        *out = value.as_f64().filter(|v| v.is_finite())?;
    }
    Some(matrix)
}

fn transform(tile: &Value) -> Result<[f64; 16], JobError> {
    match tile.get("transform") {
        None => Ok(IDENTITY),
        Some(value) => {
            finite_matrix(value).ok_or_else(|| invalid("tile transform must be 16 finite numbers"))
        }
    }
}

/// Column-major `a * b`, summing products in index order.
fn multiply(a: &[f64; 16], b: &[f64; 16]) -> [f64; 16] {
    let mut out = [0.; 16];
    for (i, value) in out.iter_mut().enumerate() {
        let (row, column) = (i % 4, i / 4);
        *value = (0..4).fold(0., |sum, k| sum + a[row + k * 4] * b[k + column * 4]);
    }
    out
}

fn collect(
    tile: &Value,
    depth: u32,
    parent: &[f64; 16],
    out: &mut Vec<TileEntry>,
) -> Result<(), JobError> {
    let placed = multiply(parent, &transform(tile)?);
    let children = match tile.get("children") {
        None => &[][..],
        Some(children) => children
            .as_array()
            .ok_or_else(|| invalid("tile children must be an array"))?,
    };
    let contents = match (tile.get("content"), tile.get("contents")) {
        (Some(content), None) => vec![content],
        (None, Some(Value::Array(contents))) => contents.iter().collect(),
        (None, None) => Vec::new(),
        _ => {
            return Err(invalid(
                "tile content must be one content or a contents array",
            ))
        }
    };
    if !contents.is_empty() {
        let geometric_error = tile["geometricError"]
            .as_f64()
            .filter(|v| v.is_finite() && *v >= 0.)
            .ok_or_else(|| invalid("geometricError must be finite and nonnegative"))?;
        let role = if children.is_empty() {
            TileRole::Leaf
        } else {
            TileRole::Coarse
        };
        for content in contents {
            let uri = content["uri"]
                .as_str()
                .ok_or_else(|| invalid("tile content without a URI"))?;
            out.push(TileEntry {
                uri: uri.to_owned(),
                depth,
                role,
                geometric_error,
                transform: placed,
            });
        }
    }
    for child in children {
        collect(child, depth + 1, &placed, out)?;
    }
    Ok(())
}

// ---- One member: GLB framing, decoded views, plain document ----

fn word(bytes: &[u8], at: usize) -> Option<usize> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?) as usize)
}

/// The JSON document and binary chunk (empty when absent) of a GLB 2.0.
fn glb_parts(bytes: &[u8]) -> Result<(Value, &[u8]), JobError> {
    if bytes.len() >= 4 && &bytes[..4] != b"glTF" {
        let magic = String::from_utf8_lossy(&bytes[..4]).into_owned();
        return Err(unsupported(format!(
            "tile content is not GLB (magic {magic:?})"
        )));
    }
    let header = |at| word(bytes, at).ok_or_else(|| invalid("truncated GLB header"));
    if header(4)? != 2 || header(8)? != bytes.len() || header(16)? != 0x4E4F_534A {
        return Err(invalid(
            "GLB header is not version 2 with a leading JSON chunk",
        ));
    }
    check_json(header(12)?)?;
    let json_end = header(12)?
        .checked_add(20)
        .filter(|&end| end <= bytes.len())
        .ok_or_else(|| invalid("GLB JSON chunk exceeds the file"))?;
    let doc: Value = serde_json::from_slice(&bytes[20..json_end])
        .map_err(|e| invalid(format!("GLB JSON: {e}")))?;
    if !doc.is_object() {
        return Err(invalid("GLB JSON must be an object"));
    }
    if json_end == bytes.len() {
        return Ok((doc, &[]));
    }
    let length = word(bytes, json_end).ok_or_else(|| invalid("truncated GLB chunk"))?;
    if word(bytes, json_end + 4) != Some(0x004E_4942) {
        return Err(invalid("second GLB chunk is not BIN"));
    }
    let bin = json_end
        .checked_add(8)
        .and_then(|start| bytes.get(start..start.checked_add(length)?))
        .ok_or_else(|| invalid("GLB BIN chunk exceeds the file"))?;
    Ok((doc, bin))
}

fn index(value: &Value, what: &str) -> Result<usize, JobError> {
    value
        .as_u64()
        .and_then(|v| usize::try_from(v).ok())
        .ok_or_else(|| invalid(format!("{what} must be a nonnegative integer")))
}

fn optional_index(value: &Value, what: &str) -> Result<usize, JobError> {
    if value.is_null() {
        Ok(0)
    } else {
        index(value, what)
    }
}

fn array<'a>(doc: &'a Value, key: &str) -> Result<&'a [Value], JobError> {
    match doc.get(key) {
        None => Ok(&[]),
        Some(Value::Array(values)) => Ok(values),
        Some(_) => Err(invalid(format!("{key} must be an array"))),
    }
}

/// A member with every buffer view decoded to plain bytes.
struct Plain {
    doc: Value,
    views: Vec<Vec<u8>>,
}

fn decode_view(view: &Value, bin: &[u8], buffers: &[Value]) -> Result<Vec<u8>, JobError> {
    let length = index(&view["byteLength"], "bufferView.byteLength")?;
    let slice = |buffer: &Value, offset: &Value, length: usize| -> Result<&[u8], JobError> {
        if index(buffer, "buffer index")? != 0 {
            return Err(unsupported("buffer view data outside the GLB binary chunk"));
        }
        let start = optional_index(offset, "byteOffset")?;
        start
            .checked_add(length)
            .and_then(|end| bin.get(start..end))
            .ok_or_else(|| invalid("buffer view exceeds the GLB binary chunk"))
    };
    let Some(ext) = view.get("extensions").and_then(|e| e.get(MESHOPT)) else {
        let buffer = index(&view["buffer"], "bufferView.buffer")?;
        if buffers.get(buffer).is_some_and(|b| b.get("uri").is_some()) {
            return Err(unsupported("external buffers are not read"));
        }
        return Ok(slice(&view["buffer"], &view["byteOffset"], length)?.to_vec());
    };
    let count = index(&ext["count"], "meshopt count")?;
    let stride = index(&ext["byteStride"], "meshopt byteStride")?;
    if count.checked_mul(stride) != Some(length)
        || view
            .get("byteStride")
            .is_some_and(|s| s.as_u64() != Some(stride as u64))
    {
        return Err(invalid(
            "meshopt count and byteStride disagree with the view",
        ));
    }
    let mode = match ext["mode"].as_str() {
        Some("ATTRIBUTES") => Mode::Attributes,
        Some("TRIANGLES") => Mode::Triangles,
        Some("INDICES") => Mode::Indices,
        _ => return Err(invalid("unknown meshopt mode")),
    };
    let filter = match ext.get("filter") {
        None => Filter::None,
        Some(name) => name
            .as_str()
            .and_then(Filter::parse)
            .ok_or_else(|| unsupported(format!("meshopt filter {name} is not decoded")))?,
    };
    let compressed = slice(
        &ext["buffer"],
        &ext["byteOffset"],
        index(&ext["byteLength"], "meshopt byteLength")?,
    )?;
    let decode = || -> Result<Vec<u8>, crate::content_integrity::FormatError> {
        meshopt::check_header(compressed, mode)?;
        meshopt::decode_filtered(compressed, count, stride, mode, filter)
    };
    decode().map_err(|e| {
        use crate::content_integrity::FormatError;
        match e {
            FormatError::InvalidInput(m) => invalid(m),
            FormatError::Unsupported(m) => unsupported(m),
            FormatError::ResourceLimit(m) => limit(m),
        }
    })
}

impl Plain {
    /// Decodes a member the caller holds (and charges). The parsed document is
    /// charged at [`JSON_EXPANSION`] times its JSON chunk, which is capped first.
    fn decode(bytes: &[u8], budget: &mut Budget) -> Result<Self, JobError> {
        if let Some(json) = word(bytes, 12).filter(|_| bytes.starts_with(b"glTF")) {
            check_json(json)?;
            budget.elements(json, JSON_EXPANSION as usize)?;
        }
        let (doc, bin) = glb_parts(bytes)?;
        let buffers = array(&doc, "buffers")?;
        if let Some(first) = buffers.first() {
            if first.get("uri").is_some() {
                return Err(unsupported("the first buffer must be the GLB binary chunk"));
            }
        }
        let mut views = Vec::new();
        for view in array(&doc, "bufferViews")? {
            let length = index(&view["byteLength"], "bufferView.byteLength")?;
            // The view, plus the meshopt decoder's word buffer while it decodes.
            budget.elements(length, 2)?;
            views.push(decode_view(view, bin, buffers)?);
            budget.credit(length as u64);
        }
        Ok(Self { doc, views })
    }

    /// The view bytes of image `i`, with its declared MIME type.
    fn image(&self, i: usize) -> Result<(&[u8], &str), JobError> {
        let image = &self.doc["images"][i];
        if image.get("uri").is_some() {
            return Err(unsupported("external images are not read"));
        }
        let view = index(&image["bufferView"], "image.bufferView")?;
        let bytes = self
            .views
            .get(view)
            .ok_or_else(|| invalid("image bufferView does not exist"))?;
        Ok((bytes, image["mimeType"].as_str().unwrap_or("")))
    }

    /// Level 0 of image `i` as RGBA8. The size comes from the page header and is
    /// refused or charged before anything is decoded; the page stays charged (the
    /// caller credits it when freed), decoder scratch is credited here.
    fn page(&self, i: usize, budget: &mut Budget) -> Result<Page, JobError> {
        let (bytes, mime) = self.image(i)?;
        let (width, height, rgba) = match mime {
            "image/ktx2" => {
                // The raw header size first, so an oversized page names its limit.
                if let (Some(width), Some(height)) = (word(bytes, 20), word(bytes, 24)) {
                    check_page(width as u32, height as u32)?;
                }
                let (width, height) = crate::basisu_ktx2::base_dimensions(bytes)?;
                let texels = check_page(width, height)?;
                budget.bytes(texels * 4)?;
                crate::basisu_ktx2::transcode_base_rgba(bytes)?
            }
            "image/png" | "image/jpeg" => {
                let format = if mime == "image/png" {
                    image::ImageFormat::Png
                } else {
                    image::ImageFormat::Jpeg
                };
                use image::ImageDecoder as _;
                let decode_error = |e: image::ImageError| invalid(format!("image {i}: {e}"));
                let mut decoder =
                    image::ImageReader::with_format(std::io::Cursor::new(bytes), format)
                        .into_decoder()
                        .map_err(decode_error)?;
                let (width, height) = decoder.dimensions();
                let texels = check_page(width, height)?;
                // The decoder's own image; an RGBA8 source becomes the page without a
                // copy, any other is converted into a new RGBA8 page and then freed.
                let decoded = decoder.total_bytes();
                let rgba8 = decoder.color_type() == image::ColorType::Rgba8;
                budget.bytes(decoded)?;
                if !rgba8 {
                    budget.bytes(texels * 4)?;
                }
                let mut limits = image::Limits::default();
                limits.max_alloc = Some(decoded + (16 << 20));
                decoder.set_limits(limits).map_err(decode_error)?;
                let rgba = image::DynamicImage::from_decoder(decoder)
                    .map_err(decode_error)?
                    .into_rgba8()
                    .into_raw();
                if !rgba8 {
                    budget.credit(decoded);
                }
                (width, height, rgba)
            }
            other => return Err(unsupported(format!("image {i} has MIME type {other:?}"))),
        };
        Ok(Page {
            width,
            height,
            rgba,
        })
    }

    /// Points every `KHR_texture_basisu` texture at its page and drops the codec
    /// extensions this view removes. Returns the KTX2 image indices.
    fn retarget(&mut self) -> Result<BTreeSet<usize>, JobError> {
        let mut ktx2 = BTreeSet::new();
        for (i, image) in array(&self.doc, "images")?.iter().enumerate() {
            if image["mimeType"] == "image/ktx2" {
                ktx2.insert(i);
            }
        }
        if let Some(Value::Array(textures)) = self.doc.get_mut("textures") {
            for texture in textures {
                let Some(source) = texture
                    .get("extensions")
                    .and_then(|e| e.get(BASISU))
                    .map(|e| index(&e["source"], "KHR_texture_basisu.source"))
                    .transpose()?
                else {
                    continue;
                };
                texture["source"] = Value::from(source);
                remove_extension(texture, BASISU);
            }
        }
        for key in ["extensionsUsed", "extensionsRequired"] {
            if let Some(Value::Array(names)) = self.doc.get_mut(key) {
                names.retain(|name| name != MESHOPT && name != BASISU);
                if names.is_empty() {
                    self.doc.as_object_mut().unwrap().remove(key);
                }
            }
        }
        Ok(ktx2)
    }

    fn into_glb(mut self, budget: &mut Budget) -> Result<Vec<u8>, JobError> {
        let ktx2 = self.retarget()?;
        let mut replaced = BTreeSet::new();
        for &i in &ktx2 {
            let view = index(&self.doc["images"][i]["bufferView"], "image.bufferView")?;
            if !replaced.insert(view) {
                self.doc["images"][i]["mimeType"] = Value::from("image/png");
                continue;
            }
            // The RGBA page is transient: it is credited once its PNG replaces the view.
            let page = self.page(i, budget)?;
            self.doc["images"][i]["mimeType"] = Value::from("image/png");
            let mut png = Vec::new();
            // A view for display: fast deflate, not the smallest file.
            image::codecs::png::PngEncoder::new_with_quality(
                &mut png,
                image::codecs::png::CompressionType::Fast,
                image::codecs::png::FilterType::Adaptive,
            )
            .write_image(
                &page.rgba,
                page.width,
                page.height,
                image::ExtendedColorType::Rgba8,
            )
            .map_err(|e| error(JobErrorKind::InvalidState, format!("PNG encode: {e}")))?;
            budget.elements(png.len(), 1)?;
            let ktx2 = std::mem::replace(&mut self.views[view], png);
            budget.credit(ktx2.len() as u64 + page.rgba.len() as u64);
        }
        // The new binary chunk and the framed GLB, then (views and chunk freed) the
        // caller's copy of the GLB.
        let views: usize = self.views.iter().map(Vec::len).sum();
        let padded = views.saturating_add(self.views.len().saturating_mul(VIEW_ALIGN));
        budget.elements(padded, 1)?;
        let mut bin = Vec::new();
        if let Some(Value::Array(views)) = self.doc.get_mut("bufferViews") {
            for (view, bytes) in views.iter_mut().zip(&self.views) {
                let offset = crate::glb::append_aligned(&mut bin, VIEW_ALIGN, bytes);
                remove_extension(view, MESHOPT);
                view["buffer"] = Value::from(0);
                view["byteOffset"] = Value::from(offset);
                view["byteLength"] = Value::from(bytes.len());
            }
        }
        let doc = self.doc.as_object_mut().unwrap();
        if bin.is_empty() {
            doc.remove("buffers");
        } else {
            doc.insert(
                "buffers".into(),
                serde_json::json!([{ "byteLength": bin.len() }]),
            );
        }
        let json = crate::glb::json_len(&self.doc).map_err(|e| invalid(e.to_string()))?;
        // Header, chunk headers and padding: under 64 bytes.
        let framed = padded.saturating_add(json).saturating_add(64);
        budget.elements(framed, 1)?;
        let glb = crate::glb::encode_glb(&self.doc, &bin).map_err(|e| limit(e.to_string()))?;
        drop(bin);
        self.views = Vec::new();
        budget.credit((views + padded + framed.saturating_sub(glb.len())) as u64);
        budget.elements(glb.len(), 1)?;
        Ok(glb)
    }
}

fn remove_extension(owner: &mut Value, name: &str) {
    let Some(object) = owner.as_object_mut() else {
        return;
    };
    if let Some(Value::Object(extensions)) = object.get_mut("extensions") {
        extensions.remove(name);
        if extensions.is_empty() {
            object.remove("extensions");
        }
    }
}

// ---- Arrays ----

/// One RGBA8 page, rows tightly packed, top row first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Page {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// One `EXT_mesh_features` feature ID set of a primitive.
#[derive(Clone, Debug, PartialEq)]
pub struct FeatureIds {
    pub label: Option<String>,
    pub feature_count: u64,
    pub null_feature_id: Option<u64>,
    pub property_table: Option<usize>,
    /// One ID per vertex (`_FEATURE_ID_n`, or the vertex index when implicit).
    pub values: Vec<u32>,
}

/// One triangle primitive in the tile frame.
#[derive(Clone, Debug, PartialEq)]
pub struct Primitive {
    pub node: usize,
    pub mesh: usize,
    pub primitive: usize,
    pub material: Option<usize>,
    pub positions: Vec<[f64; 3]>,
    pub indices: Vec<[u32; 3]>,
    pub normals: Option<Vec<[f32; 3]>>,
    /// `TEXCOORD_0`, `TEXCOORD_1`, ... in set order.
    pub uvs: Vec<Vec<[f32; 2]>>,
    /// `COLOR_0` as linear RGBA (alpha 1 for VEC3).
    pub colors: Option<Vec<[f32; 4]>>,
    pub feature_ids: Vec<FeatureIds>,
}

/// One numeric scalar property column: the stored little-endian values of
/// `component_type` (`UINT8`, `INT8`, ... `FLOAT64`, as in the metadata schema).
/// The value transform is returned, not applied: a property's value is
/// `offset + scale * v`, where `v` is the stored value, normalized first when
/// `normalized` (an omitted offset is 0, an omitted scale 1). `noData` and
/// `default` are not applied either.
#[derive(Clone, Debug, PartialEq)]
pub struct Column {
    pub name: String,
    pub component_type: String,
    pub bytes: Vec<u8>,
    pub normalized: bool,
    /// The property's override, else the class's, if any.
    pub offset: Option<f64>,
    pub scale: Option<f64>,
}

/// One `EXT_structural_metadata` property table. Properties that are not
/// numeric scalars (strings, enums, booleans, arrays, vectors and matrices)
/// are listed in `skipped` with the reason, not decoded.
#[derive(Clone, Debug, PartialEq)]
pub struct PropertyTable {
    pub class: String,
    pub count: u64,
    pub columns: Vec<Column>,
    pub skipped: Vec<(String, String)>,
}

/// A decoded member. `materials`, `textures` and `samplers` are the plain
/// document's arrays; a texture's `source` is an index into `pages`.
#[derive(Clone, Debug, PartialEq)]
pub struct DecodedTile {
    pub tile: TileEntry,
    pub root_transform: [f64; 16],
    pub primitives: Vec<Primitive>,
    pub pages: Vec<Page>,
    pub property_tables: Vec<PropertyTable>,
    pub materials: Value,
    pub textures: Value,
    pub samplers: Value,
}

fn component_width(component_type: u64) -> Option<usize> {
    match component_type {
        5120 | 5121 => Some(1),
        5122 | 5123 => Some(2),
        5125 | 5126 => Some(4),
        _ => None,
    }
}

/// Accessor values widened to f64, normalized integers mapped per glTF.
fn accessor(
    plain: &Plain,
    i: usize,
    width: &[usize],
    budget: &mut Budget,
) -> Result<(Vec<f64>, usize), JobError> {
    let a = &plain.doc["accessors"][i];
    if !a.is_object() {
        return Err(invalid(format!("accessor {i} does not exist")));
    }
    if a.get("sparse").is_some() {
        return Err(unsupported("sparse accessors are not read"));
    }
    let components = match a["type"].as_str() {
        Some("SCALAR") => 1,
        Some("VEC2") => 2,
        Some("VEC3") => 3,
        Some("VEC4") => 4,
        _ => return Err(unsupported(format!("accessor {i} type is not read"))),
    };
    if !width.contains(&components) {
        return Err(invalid(format!("accessor {i} has {components} components")));
    }
    let kind = a["componentType"].as_u64().unwrap_or(0);
    let size =
        component_width(kind).ok_or_else(|| invalid(format!("accessor {i} componentType")))?;
    let count = index(&a["count"], "accessor.count")?;
    let normalized = a["normalized"] == true;
    // The widened values, charged (overflow included) before anything is allocated.
    budget.elements(count.saturating_mul(components), 8)?;
    let values = count
        .checked_mul(components)
        .ok_or_else(|| limit("accessor size overflows"))?;
    let Some(view_index) = a.get("bufferView") else {
        let mut zeros = Vec::new();
        zeros
            .try_reserve_exact(values)
            .map_err(|_| limit("accessor allocation failed"))?;
        zeros.resize(values, 0.);
        return Ok((zeros, components));
    };
    let view_index = index(view_index, "accessor.bufferView")?;
    let view = plain
        .views
        .get(view_index)
        .ok_or_else(|| invalid("accessor bufferView does not exist"))?;
    let element = size * components;
    let stride = match plain.doc["bufferViews"][view_index].get("byteStride") {
        None => element,
        Some(stride) => index(stride, "byteStride")?,
    };
    let offset = optional_index(&a["byteOffset"], "accessor.byteOffset")?;
    let fits = count == 0
        || (count - 1)
            .checked_mul(stride)
            .and_then(|n| n.checked_add(offset))
            .and_then(|n| n.checked_add(element))
            .is_some_and(|end| end <= view.len());
    if !fits || stride < element {
        return Err(invalid(format!("accessor {i} exceeds its buffer view")));
    }
    let mut out = Vec::new();
    out.try_reserve_exact(values)
        .map_err(|_| limit("accessor allocation failed"))?;
    for e in 0..count {
        for c in 0..components {
            let at = offset + e * stride + c * size;
            let b = &view[at..at + size];
            let (value, max) = match kind {
                5120 => (f64::from(b[0] as i8), 127.),
                5121 => (f64::from(b[0]), 255.),
                5122 => (f64::from(i16::from_le_bytes([b[0], b[1]])), 32767.),
                5123 => (f64::from(u16::from_le_bytes([b[0], b[1]])), 65535.),
                5125 => (
                    f64::from(u32::from_le_bytes(b.try_into().unwrap())),
                    4294967295.,
                ),
                _ => (f64::from(f32::from_le_bytes(b.try_into().unwrap())), 1.),
            };
            out.push(if normalized && kind != 5126 {
                (value / max).max(-1.)
            } else {
                value
            });
        }
    }
    if out.iter().any(|v| !v.is_finite()) {
        return Err(invalid(format!("accessor {i} holds a non-finite value")));
    }
    Ok((out, components))
}

fn node_matrix(node: &Value) -> Result<[f64; 16], JobError> {
    if let Some(matrix) = node.get("matrix") {
        return finite_matrix(matrix)
            .ok_or_else(|| invalid("node matrix must be 16 finite numbers"));
    }
    let numbers = |key: &str, default: &[f64]| -> Result<Vec<f64>, JobError> {
        match node.get(key) {
            None => Ok(default.to_vec()),
            Some(Value::Array(values)) if values.len() == default.len() => values
                .iter()
                .map(|v| v.as_f64().filter(|v| v.is_finite()))
                .collect::<Option<Vec<_>>>()
                .ok_or_else(|| invalid(format!("node {key} must be finite numbers"))),
            Some(_) => Err(invalid(format!("node {key} has the wrong length"))),
        }
    };
    let t = numbers("translation", &[0., 0., 0.])?;
    let q = numbers("rotation", &[0., 0., 0., 1.])?;
    let s = numbers("scale", &[1., 1., 1.])?;
    let (x, y, z, w) = (q[0], q[1], q[2], q[3]);
    Ok([
        (1. - 2. * y * y - 2. * z * z) * s[0],
        (2. * x * y + 2. * z * w) * s[0],
        (2. * x * z - 2. * y * w) * s[0],
        0.,
        (2. * x * y - 2. * z * w) * s[1],
        (1. - 2. * x * x - 2. * z * z) * s[1],
        (2. * y * z + 2. * x * w) * s[1],
        0.,
        (2. * x * z + 2. * y * w) * s[2],
        (2. * y * z - 2. * x * w) * s[2],
        (1. - 2. * x * x - 2. * y * y) * s[2],
        0.,
        t[0],
        t[1],
        t[2],
        1.,
    ])
}

/// Determinant of the linear part of a column-major affine matrix.
fn determinant(m: &[f64; 16]) -> f64 {
    m[0] * (m[5] * m[10] - m[6] * m[9]) - m[4] * (m[1] * m[10] - m[2] * m[9])
        + m[8] * (m[1] * m[6] - m[2] * m[5])
}

fn point(m: &[f64; 16], p: &[f64]) -> [f64; 3] {
    [0, 1, 2].map(|a| m[a] * p[0] + m[a + 4] * p[1] + m[a + 8] * p[2] + m[a + 12])
}

/// Inverse transpose of the linear part (cofactors over the determinant), row-major 3x3.
fn normal_matrix(m: &[f64; 16]) -> Result<[f64; 9], JobError> {
    let (a, b, c) = ([m[0], m[1], m[2]], [m[4], m[5], m[6]], [m[8], m[9], m[10]]);
    let cross = |u: [f64; 3], v: [f64; 3]| {
        [
            u[1] * v[2] - u[2] * v[1],
            u[2] * v[0] - u[0] * v[2],
            u[0] * v[1] - u[1] * v[0],
        ]
    };
    let (ca, cb, cc) = (cross(b, c), cross(c, a), cross(a, b));
    let det = a[0] * ca[0] + a[1] * ca[1] + a[2] * ca[2];
    if !det.is_finite() || det == 0. {
        return Err(invalid("singular node transform"));
    }
    let mut out = [0.; 9];
    for (o, v) in out.iter_mut().zip(ca.iter().chain(&cb).chain(&cc)) {
        *o = v / det;
    }
    Ok(out)
}

fn integers(values: &[f64], what: &str) -> Result<Vec<u32>, JobError> {
    values
        .iter()
        .map(|&v| {
            (v.fract() == 0. && (0. ..=f64::from(u32::MAX)).contains(&v))
                .then_some(v as u32)
                .ok_or_else(|| invalid(format!("{what} holds a non-integer or out-of-range value")))
        })
        .collect()
}

impl Plain {
    fn primitive(
        &self,
        node: usize,
        mesh: usize,
        index_in_mesh: usize,
        world: &[f64; 16],
        budget: &mut Budget,
    ) -> Result<Primitive, JobError> {
        let p = &self.doc["meshes"][mesh]["primitives"][index_in_mesh];
        if p["mode"].as_u64().unwrap_or(4) != 4 {
            return Err(unsupported("only triangle-list primitives are read"));
        }
        let attributes = p["attributes"]
            .as_object()
            .ok_or_else(|| invalid("primitive attributes must be an object"))?;
        let attribute = |name: &str| attributes.get(name).map(|v| index(v, name)).transpose();
        let position =
            attribute("POSITION")?.ok_or_else(|| invalid("primitive has no POSITION"))?;
        let (local, _) = accessor(self, position, &[3], budget)?;
        let tile = multiply(&Y_UP_TO_Z_UP, world);
        budget.output(local.len() / 3, 24)?;
        let positions: Vec<[f64; 3]> = local
            .as_chunks::<3>()
            .0
            .iter()
            .map(|p| {
                let [x, y, z] = point(world, p);
                [x, -z, y]
            })
            .collect();
        // Widened accessor values are freed once converted.
        let free = |budget: &mut Budget, values: Vec<f64>| budget.credit(values.len() as u64 * 8);
        free(budget, local);
        let vertices = positions.len();
        // The flat index list is scratch for the triangle output below.
        let indices = match p.get("indices") {
            None => {
                budget.elements(vertices, 4)?;
                (0..vertices as u32).collect()
            }
            Some(i) => {
                let (values, _) = accessor(self, index(i, "indices")?, &[1], budget)?;
                budget.elements(values.len(), 4)?;
                let indices = integers(&values, "indices");
                free(budget, values);
                indices?
            }
        };
        budget.output(indices.len(), 4)?;
        if indices.len() % 3 != 0 || indices.iter().any(|&i| i as usize >= vertices) {
            return Err(invalid(
                "primitive indices are not whole triangles within its vertices",
            ));
        }
        // glTF: a negative determinant reverses the front-face winding. Keep
        // counter-clockwise front faces in the tile frame.
        let mirrored = determinant(world) < 0.;
        let indices: Vec<[u32; 3]> = indices
            .as_chunks::<3>()
            .0
            .iter()
            .map(|&[a, b, c]| if mirrored { [a, c, b] } else { [a, b, c] })
            .collect();
        budget.credit(indices.len() as u64 * 12);
        let per_vertex = |name: &str, values: &[f64], n: usize| -> Result<(), JobError> {
            if values.len() != vertices * n {
                return Err(invalid(format!("{name} count differs from POSITION")));
            }
            Ok(())
        };
        let normals = match attribute("NORMAL")? {
            None => None,
            Some(i) => {
                let (values, _) = accessor(self, i, &[3], budget)?;
                per_vertex("NORMAL", &values, 3)?;
                budget.output(vertices, 12)?;
                let n = normal_matrix(&tile)?;
                let mut out = Vec::with_capacity(vertices);
                for v in values.as_chunks::<3>().0 {
                    let d = [0, 1, 2].map(|r| n[r] * v[0] + n[r + 3] * v[1] + n[r + 6] * v[2]);
                    let length = d[0].hypot(d[1]).hypot(d[2]);
                    if length == 0. || !length.is_finite() {
                        return Err(invalid("NORMAL holds a zero vector"));
                    }
                    out.push(d.map(|c| (c / length) as f32));
                }
                free(budget, values);
                Some(out)
            }
        };
        let mut uvs = Vec::new();
        while let Some(i) = attribute(&format!("TEXCOORD_{}", uvs.len()))? {
            let (values, _) = accessor(self, i, &[2], budget)?;
            per_vertex("TEXCOORD", &values, 2)?;
            budget.output(vertices, 8)?;
            uvs.push(
                values
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|v| [v[0] as f32, v[1] as f32])
                    .collect(),
            );
            free(budget, values);
        }
        let colors = match attribute("COLOR_0")? {
            None => None,
            Some(i) => {
                let (values, n) = accessor(self, i, &[3, 4], budget)?;
                per_vertex("COLOR_0", &values, n)?;
                budget.output(vertices, 16)?;
                let colors = values
                    .chunks_exact(n)
                    .map(|c| [c[0], c[1], c[2], if n == 4 { c[3] } else { 1. }].map(|v| v as f32))
                    .collect();
                free(budget, values);
                Some(colors)
            }
        };
        let mut feature_ids = Vec::new();
        let sets = &p["extensions"]["EXT_mesh_features"]["featureIds"];
        for set in sets.as_array().map(Vec::as_slice).unwrap_or_default() {
            if set.get("texture").is_some() {
                return Err(unsupported("feature ID textures are not read"));
            }
            budget.output(vertices, 4)?;
            let values = match set.get("attribute") {
                None => (0..vertices as u32).collect(),
                Some(n) => {
                    let name = format!("_FEATURE_ID_{}", index(n, "featureIds.attribute")?);
                    let i = attribute(&name)?
                        .ok_or_else(|| invalid(format!("primitive has no {name}")))?;
                    let (values, _) = accessor(self, i, &[1], budget)?;
                    per_vertex(&name, &values, 1)?;
                    let ids = integers(&values, &name);
                    free(budget, values);
                    ids?
                }
            };
            feature_ids.push(FeatureIds {
                label: set["label"].as_str().map(str::to_owned),
                feature_count: set["featureCount"]
                    .as_u64()
                    .ok_or_else(|| invalid("featureCount must be a nonnegative integer"))?,
                null_feature_id: set["nullFeatureId"].as_u64(),
                property_table: set
                    .get("propertyTable")
                    .map(|t| index(t, "propertyTable"))
                    .transpose()?,
                values,
            });
        }
        Ok(Primitive {
            node,
            mesh,
            primitive: index_in_mesh,
            material: p
                .get("material")
                .map(|m| index(m, "material"))
                .transpose()?,
            positions,
            indices,
            normals,
            uvs,
            colors,
            feature_ids,
        })
    }

    fn walk(
        &self,
        node: usize,
        parent: &[f64; 16],
        path: &mut Vec<usize>,
        out: &mut Vec<Primitive>,
        budget: &mut Budget,
    ) -> Result<(), JobError> {
        // Shared children make a DAG whose paths grow exponentially; every visit and
        // every primitive instance is charged, so the walk is bounded either way.
        budget.work()?;
        if path.contains(&node) || path.len() >= NODE_DEPTH {
            return Err(invalid("scene nodes are cyclic or nested too deeply"));
        }
        let value = &self.doc["nodes"][node];
        if !value.is_object() {
            return Err(invalid(format!("node {node} does not exist")));
        }
        let world = multiply(parent, &node_matrix(value)?);
        if let Some(mesh) = value.get("mesh") {
            let mesh = index(mesh, "node.mesh")?;
            let primitives = self.doc["meshes"][mesh]["primitives"]
                .as_array()
                .ok_or_else(|| invalid(format!("mesh {mesh} has no primitives")))?;
            for i in 0..primitives.len() {
                budget.work()?;
                out.push(self.primitive(node, mesh, i, &world, budget)?);
                budget.end_unit();
            }
        }
        path.push(node);
        for child in array(value, "children")? {
            self.walk(index(child, "node.children")?, &world, path, out, budget)?;
        }
        path.pop();
        Ok(())
    }

    fn property_tables(&self, budget: &mut Budget) -> Result<Vec<PropertyTable>, JobError> {
        let Some(metadata) = self.doc["extensions"].get("EXT_structural_metadata") else {
            return Ok(Vec::new());
        };
        if metadata.get("schemaUri").is_some() {
            return Err(unsupported("external metadata schemas are not read"));
        }
        let classes = &metadata["schema"]["classes"];
        let mut tables = Vec::new();
        for table in array(metadata, "propertyTables")? {
            let class = table["class"]
                .as_str()
                .ok_or_else(|| invalid("property table without a class"))?;
            if !classes[class].is_object() {
                return Err(invalid(format!(
                    "property table class {class:?} is not in the schema"
                )));
            }
            let count = table["count"]
                .as_u64()
                .ok_or_else(|| invalid("property table count must be a nonnegative integer"))?;
            let (mut columns, mut skipped) = (Vec::new(), Vec::new());
            let properties = table["properties"].as_object().cloned().unwrap_or_default();
            for (name, property) in &properties {
                let schema = &classes[class]["properties"][name];
                let component = schema["componentType"].as_str().unwrap_or("");
                let width = match component {
                    "UINT8" | "INT8" => 1,
                    "UINT16" | "INT16" => 2,
                    "UINT32" | "INT32" | "FLOAT32" => 4,
                    "UINT64" | "INT64" | "FLOAT64" => 8,
                    _ => 0,
                };
                let reason = if !schema.is_object() {
                    Some("not declared by the class".to_owned())
                } else if schema["type"] != "SCALAR" || schema["array"] == true {
                    Some(format!(
                        "{} {}is not a numeric scalar",
                        schema["type"].as_str().unwrap_or("untyped"),
                        if schema["array"] == true {
                            "array "
                        } else {
                            ""
                        }
                    ))
                } else if width == 0 {
                    Some(format!("component type {component:?} is not numeric"))
                } else {
                    None
                };
                if let Some(reason) = reason {
                    skipped.push((name.clone(), reason));
                    continue;
                }
                let number = |key: &str| -> Result<Option<f64>, JobError> {
                    match property.get(key).or_else(|| schema.get(key)) {
                        None => Ok(None),
                        Some(v) => {
                            v.as_f64()
                                .filter(|v| v.is_finite())
                                .map(Some)
                                .ok_or_else(|| {
                                    invalid(format!(
                                        "property {class}.{name} {key} must be a finite number"
                                    ))
                                })
                        }
                    }
                };
                let view = index(&property["values"], "property values")?;
                let length = usize::try_from(count)
                    .ok()
                    .and_then(|c| c.checked_mul(width))
                    .ok_or_else(|| limit("property column size overflows"))?;
                let bytes = self
                    .views
                    .get(view)
                    .and_then(|v| v.get(..length))
                    .ok_or_else(|| invalid(format!("property {class}.{name} is truncated")))?;
                budget.output(length, 1)?;
                columns.push(Column {
                    name: name.clone(),
                    component_type: component.to_owned(),
                    bytes: bytes.to_vec(),
                    normalized: schema["normalized"] == true,
                    offset: number("offset")?,
                    scale: number("scale")?,
                });
            }
            tables.push(PropertyTable {
                class: class.to_owned(),
                count,
                columns,
                skipped,
            });
            budget.end_unit();
        }
        Ok(tables)
    }

    fn into_arrays(
        mut self,
        tile: TileEntry,
        root_transform: [f64; 16],
        budget: &mut Budget,
    ) -> Result<DecodedTile, JobError> {
        self.retarget()?;
        let scene = optional_index(&self.doc["scene"], "scene")?;
        let roots = match self.doc["scenes"].get(scene) {
            Some(scene) => array(scene, "nodes")?.to_vec(),
            None if array(&self.doc, "scenes")?.is_empty() => Vec::new(),
            None => return Err(invalid("selected scene does not exist")),
        };
        let mut primitives = Vec::new();
        for root in &roots {
            self.walk(
                index(root, "scene.nodes")?,
                &IDENTITY,
                &mut Vec::new(),
                &mut primitives,
                budget,
            )?;
        }
        // One page per image; images sharing a view decode once and are copied.
        let mut pages: Vec<Page> = Vec::new();
        let mut decoded = std::collections::BTreeMap::new();
        for (i, image) in array(&self.doc, "images")?.iter().enumerate() {
            let view = image.get("bufferView").and_then(Value::as_u64);
            let page = match view.and_then(|v| decoded.get(&v)) {
                Some(&first) => {
                    let page: &Page = &pages[first];
                    budget.elements(page.rgba.len(), 1)?;
                    page.clone()
                }
                None => self.page(i, budget)?,
            };
            if let Some(view) = view {
                decoded.entry(view).or_insert(i);
            }
            budget.adopt(page.rgba.len() as u64);
            budget.end_unit();
            pages.push(page);
        }
        let property_tables = self.property_tables(budget)?;
        // The views are freed before the caller copies the outputs.
        let views: usize = self.views.iter().map(Vec::len).sum();
        self.views = Vec::new();
        budget.credit(views as u64);
        budget.caller_copy()?;
        let take = |doc: &mut Map<String, Value>, key: &str| {
            doc.remove(key).unwrap_or_else(|| Value::Array(Vec::new()))
        };
        let doc = self.doc.as_object_mut().unwrap();
        Ok(DecodedTile {
            tile,
            root_transform,
            primitives,
            pages,
            property_tables,
            materials: take(doc, "materials"),
            textures: take(doc, "textures"),
            samplers: take(doc, "samplers"),
        })
    }
}

#[cfg(test)]
#[path = "tile_view/tests.rs"]
mod tests;
