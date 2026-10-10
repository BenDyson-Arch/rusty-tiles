//! Finite, stable local source inventory and the once-resolved display recipe.
use super::{display::RoleRecipe, grid::StaticCrs, native, RasterDisplay, RasterLimits};
use crate::{runtime::Attempt, JobError, JobErrorKind, JobFailure};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Driver {
    GTiff,
    Png,
    Jpeg,
    AAIGrid,
}
impl Driver {
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::GTiff => "GTiff",
            Self::Png => "PNG",
            Self::Jpeg => "JPEG",
            Self::AAIGrid => "AAIGrid",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AreaPoint {
    Area,
    Point,
}
impl AreaPoint {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Area => "Area",
            Self::Point => "Point",
        }
    }
}
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SampleType {
    Byte = 1,
    U16 = 2,
    I16 = 3,
    U32 = 4,
    I32 = 5,
    F32 = 6,
    F64 = 7,
    I8 = 14,
}
impl SampleType {
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Byte => "Byte",
            Self::U16 => "UInt16",
            Self::I16 => "Int16",
            Self::U32 => "UInt32",
            Self::I32 => "Int32",
            Self::F32 => "Float32",
            Self::F64 => "Float64",
            Self::I8 => "Int8",
        }
    }
    pub(super) fn bytes(self) -> usize {
        match self {
            Self::Byte | Self::I8 => 1,
            Self::U16 | Self::I16 => 2,
            Self::U32 | Self::I32 | Self::F32 => 4,
            Self::F64 => 8,
        }
    }
    pub(super) fn from_native(n: u32) -> Result<Self, JobError> {
        match n {
            1 => Ok(Self::Byte),
            2 => Ok(Self::U16),
            3 => Ok(Self::I16),
            4 => Ok(Self::U32),
            5 => Ok(Self::I32),
            6 => Ok(Self::F32),
            7 => Ok(Self::F64),
            14 => Ok(Self::I8),
            _ => Err(unsupported(
                "source sample type is outside the real display/COG profile",
            )),
        }
    }
    pub(super) fn nodata(self, v: f64) -> Result<(), JobError> {
        let valid = match self {
            Self::Byte => (0. ..=255.).contains(&v) && v.fract() == 0.,
            Self::I8 => (-128. ..=127.).contains(&v) && v.fract() == 0.,
            Self::U16 => (0. ..=65535.).contains(&v) && v.fract() == 0.,
            Self::I16 => (-32768. ..=32767.).contains(&v) && v.fract() == 0.,
            Self::U32 => v >= 0. && v <= u32::MAX as f64 && v.fract() == 0.,
            Self::I32 => v >= i32::MIN as f64 && v <= i32::MAX as f64 && v.fract() == 0.,
            Self::F32 => v.is_nan() || (v.is_finite() && (v as f32) as f64 == v),
            Self::F64 => v.is_nan() || v.is_finite(),
        };
        if valid {
            Ok(())
        } else {
            Err(unsupported("unrepresentable typed NoData"))
        }
    }
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct TextSpan {
    pub start: u32,
    pub len: u32,
}
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(super) struct ScalarFact {
    pub bits: u64,
    pub present: bool,
}
impl ScalarFact {
    pub(super) fn new(v: f64, present: bool) -> Self {
        Self {
            bits: v.to_bits(),
            present,
        }
    }
    pub(super) fn value(self) -> f64 {
        f64::from_bits(self.bits)
    }
    pub(super) fn same(self, o: Self) -> bool {
        self.present == o.present
            && (!self.present
                || self.bits == o.bits
                || (self.value().is_nan() && o.value().is_nan()))
    }
}
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(super) struct BandFacts {
    pub nodata: ScalarFact,
    pub scale: ScalarFact,
    pub offset: ScalarFact,
    pub unit: TextSpan,
    pub palette: TextSpan,
    pub datatype: SampleType,
    pub color_interp: u8,
    pub mask_class: u8,
}
#[derive(Debug)]
pub(super) struct SourceFacts {
    pub width: u32,
    pub height: u32,
    pub affine: [f64; 6],
    pub crs: StaticCrs,
    pub source_bytes: u64,
    pub driver: Driver,
    pub area_point: AreaPoint,
    pub(super) area_present: bool,
    pub(super) projection: TextSpan,
    pub(super) text: Vec<u8>,
    pub(super) bands: Vec<BandFacts>,
    pub(super) palettes: Vec<[u8; 4]>,
}
impl SourceFacts {
    pub(super) fn bands(&self) -> &[BandFacts] {
        &self.bands
    }
    pub(super) fn text(&self, s: TextSpan) -> &[u8] {
        &self.text[s.start as usize..(s.start + s.len) as usize]
    }
    pub(super) fn palette(&self, s: TextSpan) -> &[[u8; 4]] {
        &self.palettes[s.start as usize..(s.start + s.len) as usize]
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Stamp {
    identity: [u64; 3],
    length: u64,
    seconds: i64,
    nanos: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum LeafKind {
    Main,
    Pam,
    Mask,
    World,
    Prj,
}
#[derive(Debug)]
pub(super) struct SourceLeaf {
    pub(super) path: PathBuf,
    stamp: Stamp,
    pub(super) kind: LeafKind,
}
impl SourceLeaf {
    /// Reads exactly the captured extent; growth/shrinkage is a source conflict.
    pub(super) fn read_text(&self) -> Result<Vec<u8>, JobError> {
        if self.stamp.length > 1_048_576 {
            return Err(limit("source companion text limit"));
        }
        let length = usize::try_from(self.stamp.length)
            .map_err(|_| limit("source companion text length"))?;
        let mut text = exact(length)?;
        text.resize(length, 0);
        let mut file = fs::File::open(&self.path)
            .map_err(|e| JobError::io("open source companion", &self.path, e))?;
        match file.read_exact(&mut text) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                return Err(JobError::new(
                    JobErrorKind::Conflict,
                    "source companion shrank",
                ));
            }
            Err(e) => return Err(JobError::io("read source companion", &self.path, e)),
        }
        let mut extra = [0_u8; 1];
        loop {
            match file.read(&mut extra) {
                Ok(0) => break,
                Ok(_) => {
                    return Err(JobError::new(
                        JobErrorKind::Conflict,
                        "source companion grew",
                    ))
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(JobError::io("probe source companion extent", &self.path, e)),
            }
        }
        if stamp(&self.path)? != self.stamp {
            return Err(JobError::new(
                JobErrorKind::Conflict,
                "source companion changed",
            ));
        }
        Ok(text)
    }
}
pub(super) struct SourcePlan {
    pub(super) source: Option<native::Dataset>,
    pub(super) config: native::Config,
    pub(super) facts: SourceFacts,
    pub(super) roles: RoleRecipe,
    pub(super) leaves: Vec<SourceLeaf>,
    pub(super) limits: RasterLimits,
    pub(super) workers: u8,
    pub(super) version: String,
}
impl SourcePlan {
    pub(super) fn facts(&self) -> &SourceFacts {
        &self.facts
    }
    pub(super) fn finish_failure(mut self, error: JobError) -> JobFailure {
        let mut failure = native::failure(error);
        if let Some(ds) = self.source.take() {
            native::secondary(&mut failure, ds.close());
        }
        native::secondary(&mut failure, self.config.restore());
        failure
    }
    pub(super) fn recheck(&self) -> Result<(), JobError> {
        for leaf in &self.leaves {
            if stamp(&leaf.path)? != leaf.stamp {
                return Err(JobError::new(
                    JobErrorKind::Conflict,
                    "raster source dependency changed",
                ));
            }
        }
        Ok(())
    }
    pub(super) fn companion_bytes(&self) -> Result<u64, JobError> {
        self.leaves
            .iter()
            .skip(1)
            .filter(|leaf| leaf.kind != LeafKind::Mask)
            .try_fold(0u64, |bytes, leaf| {
                bytes
                    .checked_add(leaf.stamp.length)
                    .ok_or_else(|| limit("companion work bytes overflow"))
            })
    }
    pub(super) fn owned_bytes(&self) -> Result<u64, JobError> {
        let mut n = std::mem::size_of::<Self>() as u64;
        n = n
            .checked_add((self.leaves.capacity() * std::mem::size_of::<SourceLeaf>()) as u64)
            .ok_or_else(|| limit("source owner overflow"))?;
        for leaf in &self.leaves {
            n = n
                .checked_add(leaf.path.capacity() as u64)
                .ok_or_else(|| limit("source path overflow"))?;
        }
        for bytes in [
            self.facts.text.capacity(),
            self.facts.bands.capacity() * std::mem::size_of::<BandFacts>(),
            self.facts.palettes.capacity() * 4,
            self.version.capacity(),
            self.config.owned_bytes(),
        ] {
            n = n
                .checked_add(bytes as u64)
                .ok_or_else(|| limit("source owner overflow"))?;
        }
        Ok(n)
    }
}
pub(super) fn invalid(s: &'static str) -> JobError {
    JobError::new(JobErrorKind::InvalidInput, s)
}
pub(super) fn unsupported(s: &'static str) -> JobError {
    JobError::new(JobErrorKind::Unsupported, s)
}
pub(super) fn limit(s: &'static str) -> JobError {
    JobError::new(JobErrorKind::ResourceLimit, s)
}
pub(super) fn exact<T>(count: usize) -> Result<Vec<T>, JobError> {
    let mut v = Vec::new();
    v.try_reserve_exact(count)
        .map_err(|_| limit("raster requested allocation failed"))?;
    if v.capacity() != count {
        return Err(limit(
            "raster requested capacity differs from admitted capacity",
        ));
    }
    Ok(v)
}
fn stamp(path: &Path) -> Result<Stamp, JobError> {
    let m = fs::symlink_metadata(path).map_err(|e| JobError::io("inspect raster leaf", path, e))?;
    if !m.is_file() || m.file_type().is_symlink() {
        return Err(unsupported(
            "raster dependencies must be regular nonsymlink leaves",
        ));
    }
    #[cfg(unix)]
    let identity = {
        use std::os::unix::fs::MetadataExt;
        [m.dev(), m.ino(), 0]
    };
    #[cfg(windows)]
    let identity = windows_identity(path)?;
    #[cfg(not(any(unix, windows)))]
    return Err(unsupported("raster stable file identity unavailable"));
    let mt = m
        .modified()
        .map_err(|e| JobError::io("inspect raster modification", path, e))?;
    let (seconds, nanos) = match mt.duration_since(UNIX_EPOCH) {
        Ok(d) => (
            i64::try_from(d.as_secs()).map_err(|_| invalid("raster timestamp range"))?,
            d.subsec_nanos(),
        ),
        Err(e) => {
            let d = e.duration();
            (
                -(i64::try_from(d.as_secs()).map_err(|_| invalid("raster timestamp range"))?)
                    - i64::from(d.subsec_nanos() != 0),
                if d.subsec_nanos() == 0 {
                    0
                } else {
                    1_000_000_000 - d.subsec_nanos()
                },
            )
        }
    };
    Ok(Stamp {
        identity,
        length: m.len(),
        seconds,
        nanos,
    })
}
#[cfg(windows)]
fn windows_identity(path: &Path) -> Result<[u64; 3], JobError> {
    use std::os::windows::io::AsRawHandle;
    #[repr(C)]
    struct Info {
        volume: u64,
        id: [u64; 2],
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetFileInformationByHandleEx(
            h: *mut std::ffi::c_void,
            class: i32,
            info: *mut std::ffi::c_void,
            size: u32,
        ) -> i32;
    }
    let f = fs::File::open(path).map_err(|e| JobError::io("open raster identity", path, e))?;
    let mut info = Info {
        volume: 0,
        id: [0; 2],
    };
    if unsafe {
        GetFileInformationByHandleEx(
            f.as_raw_handle().cast(),
            18,
            (&mut info as *mut Info).cast(),
            std::mem::size_of::<Info>() as u32,
        )
    } == 0
    {
        return Err(JobError::io(
            "read raster identity",
            path,
            std::io::Error::last_os_error(),
        ));
    }
    Ok([info.volume, info.id[0], info.id[1]])
}
fn companion(path: &Path, suffix: &str, stem: bool) -> PathBuf {
    let mut p = path.to_path_buf();
    let base = if stem {
        path.file_stem()
    } else {
        path.file_name()
    }
    .unwrap_or_default();
    let mut name = base.to_os_string();
    name.push(suffix);
    p.set_file_name(name);
    p
}
fn present(p: &Path) -> Result<bool, JobError> {
    match fs::symlink_metadata(p) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(JobError::io("inspect raster companion", p, e)),
    }
}
// Enumerate the finite ASCII spellings, without a retained list or directory scan.
fn case_spellings(
    suffix: &str,
    mut visit: impl FnMut(&str) -> Result<(), JobError>,
) -> Result<(), JobError> {
    let mut bytes = [0_u8; 8];
    if suffix.len() > bytes.len() || !suffix.is_ascii() {
        return Err(invalid("internal companion suffix"));
    }
    let n = suffix.len();
    bytes[..n].copy_from_slice(suffix.as_bytes());
    let letters = bytes[..n]
        .iter()
        .filter(|c| c.is_ascii_alphabetic())
        .count();
    for spelling in 0..(1usize << letters) {
        let mut bit = 0;
        for c in &mut bytes[..n] {
            if c.is_ascii_alphabetic() {
                *c = if spelling & (1 << bit) == 0 {
                    c.to_ascii_lowercase()
                } else {
                    c.to_ascii_uppercase()
                };
                bit += 1;
            }
        }
        let spelling = std::str::from_utf8(&bytes[..n]).expect("ASCII companion suffix");
        visit(spelling)?;
    }
    Ok(())
}
fn reject_companions(path: &Path) -> Result<(), JobError> {
    for stem in [false, true] {
        for suffix in [
            ".ovr", ".rrd", ".aux", ".tab", ".xml", ".imd", ".rpb", "_RPC.txt",
        ] {
            case_spellings(suffix, |spelling| {
                if present(&companion(path, spelling, stem))? {
                    return Err(unsupported(
                        "external overview/RPC/auxiliary authority is outside raster profile",
                    ));
                }
                Ok(())
            })?;
        }
    }
    Ok(())
}
fn push_leaf(
    leaves: &mut Vec<SourceLeaf>,
    p: PathBuf,
    kind: LeafKind,
    output: &Path,
) -> Result<(), JobError> {
    let s = stamp(&p)?;
    let p = fs::canonicalize(&p).map_err(|e| JobError::io("resolve raster leaf", &p, e))?;
    if p == output || p.starts_with(output) || output.starts_with(&p) {
        return Err(invalid("raster output aliases a source dependency"));
    }
    if leaves.iter().any(|l| l.stamp.identity == s.identity) {
        return Err(invalid("raster dependency aliases another source leaf"));
    }
    leaves.push(SourceLeaf {
        path: p,
        stamp: s,
        kind,
    });
    Ok(())
}
// Only case spellings of the same authority can deduplicate. Different
// authority suffixes remain ambiguous even when they are hard links.
fn same_case_authority(
    found: &[Option<(PathBuf, LeafKind)>; 5],
    candidate: &Path,
    kind: LeafKind,
) -> Result<bool, JobError> {
    for (path, role) in found.iter().flatten() {
        if *role == kind
            && path
                .as_os_str()
                .as_encoded_bytes()
                .eq_ignore_ascii_case(candidate.as_os_str().as_encoded_bytes())
            && stamp(path)?.identity == stamp(candidate)?.identity
        {
            return Ok(true);
        }
    }
    Ok(false)
}
fn refuse_aai_world(driver: Driver, leaves: &[SourceLeaf]) -> Result<(), JobError> {
    if driver == Driver::AAIGrid && leaves.iter().any(|leaf| leaf.kind == LeafKind::World) {
        return Err(unsupported(
            "AAIGrid worldfile authority is outside source profile",
        ));
    }
    Ok(())
}
/// Opens only after all finite external authorities have been screened.
pub(super) fn admit(
    input: &Path,
    output: &Path,
    limits: &RasterLimits,
    display: &RasterDisplay,
    workers: u8,
    attempt: &Attempt,
) -> Result<SourcePlan, JobFailure> {
    let mut config = native::Config::capture().map_err(native::failure)?;
    let mut cleanup = Vec::new();
    let result = (|| {
        attempt.check()?;
        reject_companions(input)?;
        let mut found: [Option<(PathBuf, LeafKind)>; 5] = std::array::from_fn(|_| None);
        found[0] = Some((input.to_path_buf(), LeafKind::Main));
        let mut count = 1;
        for (suffix, kind) in [(".aux.xml", LeafKind::Pam), (".msk", LeafKind::Mask)] {
            case_spellings(suffix, |spelling| {
                let p = companion(input, spelling, false);
                if !present(&p)? || same_case_authority(&found, &p, kind)? {
                    return Ok(());
                }
                if found.iter().flatten().any(|(_, role)| *role == kind) {
                    return Err(invalid("ambiguous PAM/mask authority"));
                }
                if kind == LeafKind::Mask {
                    reject_companions(&p)?;
                    for nested in [".aux.xml", ".msk"] {
                        case_spellings(nested, |s| {
                            if present(&companion(&p, s, false))? {
                                return Err(unsupported("nested mask authority"));
                            }
                            Ok(())
                        })?;
                    }
                }
                if count == 5 {
                    return Err(unsupported("too many raster dependencies"));
                }
                found[count] = Some((p, kind));
                count += 1;
                Ok(())
            })?;
        }
        let ext = input
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let world = match ext.as_str() {
            "tif" | "tiff" => ".tfw",
            "png" => ".pgw",
            "jpg" | "jpeg" => ".jgw",
            _ => ".wld",
        };
        let mut world_seen = false;
        for (suffix, stem) in [(world, true), (".wld", true)] {
            case_spellings(suffix, |spelling| {
                let p = companion(input, spelling, stem);
                if !present(&p)? || same_case_authority(&found, &p, LeafKind::World)? {
                    return Ok(());
                }
                if world_seen {
                    return Err(invalid("ambiguous worldfile authority"));
                }
                world_seen = true;
                if count == 5 {
                    return Err(unsupported("too many raster dependencies"));
                }
                found[count] = Some((p, LeafKind::World));
                count += 1;
                Ok(())
            })?;
        }
        for suffix in ["w", "W"] {
            let p = companion(input, suffix, false);
            if present(&p)? {
                if same_case_authority(&found, &p, LeafKind::World)? {
                    continue;
                }
                if world_seen {
                    return Err(invalid("ambiguous worldfile authority"));
                }
                world_seen = true;
                if count == 5 {
                    return Err(unsupported("too many raster dependencies"));
                }
                found[count] = Some((p, LeafKind::World));
                count += 1;
            }
        }
        case_spellings(".prj", |suffix| {
            let p = companion(input, suffix, true);
            if present(&p)? {
                if same_case_authority(&found, &p, LeafKind::Prj)? {
                    return Ok(());
                }
                if found
                    .iter()
                    .flatten()
                    .any(|(_, role)| *role == LeafKind::Prj)
                {
                    return Err(invalid("ambiguous PRJ authority"));
                }
                if count == 5 {
                    return Err(unsupported("too many raster dependencies"));
                }
                found[count] = Some((p, LeafKind::Prj));
                count += 1;
            }
            Ok(())
        })?;
        let mut leaves = exact(count)?;
        for (p, kind) in found.into_iter().flatten() {
            push_leaf(&mut leaves, p, kind, output)?;
        }
        let companion_bytes = leaves
            .iter()
            .skip(1)
            .filter(|l| l.kind != LeafKind::Mask)
            .try_fold(0u64, |n, l| {
                n.checked_add(l.stamp.length)
                    .ok_or_else(|| limit("companion text overflow"))
            })?;
        if companion_bytes > 1_048_576 {
            return Err(limit("aggregate companion text limit"));
        }
        let bytes = leaves.iter().try_fold(0u64, |n, l| {
            n.checked_add(l.stamp.length)
                .ok_or_else(|| limit("source bytes overflow"))
        })?;
        if bytes > limits.max_source_bytes {
            return Err(limit("raster source byte limit"));
        }
        native::init()?;
        native::require_outputs().map_err(|mut failure| {
            cleanup.append(&mut failure.secondary);
            failure.error
        })?;
        config.validate_cog()?;
        config.set(c"GDAL_NUM_THREADS", &workers.to_string())?;
        config.set(c"GDAL_PAM_ENABLED", "NO")?;
        let mut mask_dims = None;
        for leaf in &leaves {
            if leaf.kind == LeafKind::Mask {
                mask_dims = Some(native::admit_mask(&leaf.path).map_err(|mut f| {
                    cleanup.append(&mut f.secondary);
                    f.error
                })?);
            }
            if leaf.kind == LeafKind::Pam {
                native::screen_pam(leaf)?;
            }
            if leaf.kind == LeafKind::Prj {
                native::screen_srs(&leaf.read_text()?)?;
            }
        }
        let main = &leaves[0].path;
        let primary = native::Dataset::open(main)?;
        let inspected = native::inspect(&primary, bytes, false);
        let close = primary.close();
        let primary_facts = match inspected {
            Ok(f) => {
                close?;
                f
            }
            Err(e) => {
                if let Err(c) = close {
                    cleanup.push(c);
                }
                return Err(e);
            }
        };
        refuse_aai_world(primary_facts.driver, &leaves)?;
        config.set(c"GDAL_PAM_ENABLED", "YES")?;
        let ds = native::Dataset::open(main)?;
        let admission = (|| {
            let facts = native::inspect(&ds, bytes, true)?;
            if mask_dims.is_some_and(|dims| dims != (facts.width, facts.height)) {
                return Err(invalid("external mask/source dimensions disagree"));
            }
            native::compare_primary(&primary_facts, &facts)?;
            native::verify_companions(&leaves, &facts)?;
            native::check_file_list(&ds, &leaves)?;
            if facts.width > 65536 || facts.height > 65536 {
                return Err(limit("raster source dimension limit"));
            }
            let pixels = u64::from(facts.width) * u64::from(facts.height);
            let decoded = pixels
                .checked_mul(
                    facts.bands.len() as u64 * (facts.bands[0].datatype.bytes() as u64 + 1),
                )
                .ok_or_else(|| limit("source decoded bytes overflow"))?;
            if pixels > limits.max_source_pixels || decoded > limits.max_decoded_bytes {
                return Err(limit("raster source pixel/decoded byte limit"));
            }
            let roles = RoleRecipe::resolve(&facts, display)?;
            let version = native::version()?;
            Ok((facts, roles, version))
        })();
        match admission {
            Ok((facts, roles, version)) => Ok(SourcePlan {
                source: Some(ds),
                config: config.take(),
                facts,
                roles,
                leaves,
                limits: *limits,
                workers,
                version,
            }),
            Err(e) => {
                let f = native::failure(e);
                if let Err(c) = ds.close() {
                    cleanup.push(c);
                }
                Err(f.error)
            }
        }
    })();
    match result {
        Ok(p) => Ok(p),
        Err(e) => {
            let mut f = native::failure(e);
            f.secondary.append(&mut cleanup);
            native::secondary(&mut f, config.restore());
            Err(f)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn finite_mixed_case_spellings_and_authority_identity() {
        let mut seen = Vec::new();
        case_spellings(".tfw", |s| {
            seen.push(s.to_owned());
            Ok(())
        })
        .unwrap();
        assert_eq!(seen.len(), 8);
        assert!(seen.iter().any(|s| s == ".TfW"));
        let root = tempfile::tempdir().unwrap();
        let input = root.path().join("image.tif");
        fs::write(&input, b"literal").unwrap();
        fs::write(root.path().join("image.tif.OvR"), b"literal").unwrap();
        assert_eq!(
            reject_companions(&input).unwrap_err().kind(),
            JobErrorKind::Unsupported
        );
        let lower = root.path().join("image.tfw");
        let mixed = root.path().join("image.TfW");
        fs::write(&lower, b"literal").unwrap();
        if !present(&mixed).unwrap() {
            fs::hard_link(&lower, &mixed).unwrap();
        }
        let mut found = std::array::from_fn(|_| None);
        found[0] = Some((lower.clone(), LeafKind::World));
        assert!(same_case_authority(&found, &mixed, LeafKind::World).unwrap());
        let distinct = root.path().join("image.wld");
        fs::hard_link(&lower, &distinct).unwrap();
        assert!(!same_case_authority(&found, &distinct, LeafKind::World).unwrap());
    }
    #[test]
    fn companion_exact_extent_and_aai_world_refusal() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("source.wld");
        fs::write(&path, b"123456").unwrap();
        let leaf = SourceLeaf {
            stamp: stamp(&path).unwrap(),
            path: path.clone(),
            kind: LeafKind::World,
        };
        let text = leaf.read_text().unwrap();
        assert_eq!(text, b"123456");
        assert_eq!(text.capacity(), 6);
        assert!(refuse_aai_world(Driver::GTiff, std::slice::from_ref(&leaf)).is_ok());
        assert_eq!(
            refuse_aai_world(Driver::AAIGrid, std::slice::from_ref(&leaf))
                .unwrap_err()
                .kind(),
            JobErrorKind::Unsupported
        );
        fs::write(&path, b"1234567").unwrap();
        assert_eq!(leaf.read_text().unwrap_err().kind(), JobErrorKind::Conflict);
        fs::write(&path, b"12345").unwrap();
        assert_eq!(leaf.read_text().unwrap_err().kind(), JobErrorKind::Conflict);
        fs::write(&path, b"").unwrap();
        let empty = SourceLeaf {
            stamp: stamp(&path).unwrap(),
            path,
            kind: LeafKind::Prj,
        };
        assert_eq!(empty.read_text().unwrap().capacity(), 0);
    }
    #[test]
    fn typed_nodata_refusals() {
        assert!(SampleType::Byte.nodata(255.).is_ok());
        assert!(SampleType::Byte.nodata(256.).is_err());
        assert!(SampleType::I16.nodata(-32768.).is_ok());
        assert!(SampleType::I16.nodata(0.5).is_err());
        assert!(SampleType::F32.nodata(1.1).is_err());
        assert!(SampleType::F32.nodata(f64::NAN).is_ok());
        assert!(SampleType::F64.nodata(f64::INFINITY).is_err());
        assert!(SampleType::from_native(12).is_err());
        assert!(SampleType::from_native(13).is_err());
    }
    #[test]
    fn exact_actual_counts_and_layout() {
        assert_eq!(exact::<BandFacts>(2).unwrap().capacity(), 2);
        assert_eq!(exact::<SourceLeaf>(1).unwrap().capacity(), 1);
        assert_eq!(exact::<u64>(0).unwrap().capacity(), 0);
        println!(
            "R2 owner sizes leaf={} stamp={} band={} facts={} plan={} role={} window={}",
            std::mem::size_of::<SourceLeaf>(),
            std::mem::size_of::<Stamp>(),
            std::mem::size_of::<BandFacts>(),
            std::mem::size_of::<SourceFacts>(),
            std::mem::size_of::<SourcePlan>(),
            std::mem::size_of::<RoleRecipe>(),
            std::mem::size_of::<super::super::display::Window>()
        );
    }
    fn facts() -> SourceFacts {
        let band = BandFacts {
            nodata: ScalarFact::new(0., false),
            scale: ScalarFact::new(1., false),
            offset: ScalarFact::new(0., false),
            unit: TextSpan::default(),
            palette: TextSpan::default(),
            datatype: SampleType::Byte,
            color_interp: 1,
            mask_class: 1,
        };
        SourceFacts {
            width: 2,
            height: 3,
            affine: [0., 1., 0., 0., 0., -1.],
            crs: StaticCrs::WebMercator3857,
            source_bytes: 42,
            driver: Driver::GTiff,
            area_point: AreaPoint::Area,
            area_present: false,
            projection: TextSpan::default(),
            text: Vec::new(),
            bands: vec![band],
            palettes: Vec::new(),
        }
    }
    #[test]
    fn selected_roles_and_numeric_endpoints() {
        let mut f = facts();
        assert_eq!(
            RoleRecipe::resolve(&f, &RasterDisplay::Image { alpha_band: None })
                .unwrap()
                .data,
            [1, 0, 0]
        );
        assert!(RoleRecipe::resolve(
            &f,
            &RasterDisplay::Image {
                alpha_band: Some(0)
            }
        )
        .is_err());
        let mut rgb = facts();
        let mut undefined = rgb.bands[0];
        undefined.color_interp = 0;
        rgb.bands = vec![undefined; 3];
        assert_eq!(
            RoleRecipe::resolve(&rgb, &RasterDisplay::Image { alpha_band: None })
                .unwrap()
                .data,
            [1, 2, 3]
        );
        rgb.bands[0].color_interp = 1;
        assert!(RoleRecipe::resolve(&rgb, &RasterDisplay::Image { alpha_band: None }).is_err());
        f.bands[0].datatype = SampleType::F32;
        assert!(RoleRecipe::resolve(&f, &RasterDisplay::Image { alpha_band: None }).is_err());
        assert!(RoleRecipe::resolve(
            &f,
            &RasterDisplay::Gray {
                band: 1,
                low: 0.,
                high: 1.,
                alpha_band: None
            }
        )
        .is_ok());
        assert!(RoleRecipe::resolve(
            &f,
            &RasterDisplay::Gray {
                band: 1,
                low: -f64::MAX,
                high: f64::MAX,
                alpha_band: None
            }
        )
        .is_err());
        let mut alpha = f.bands[0];
        alpha.color_interp = 6;
        f.bands.push(alpha);
        let original = RoleRecipe::resolve(
            &f,
            &RasterDisplay::Gray {
                band: 1,
                low: 0.,
                high: 1.,
                alpha_band: None,
            },
        )
        .unwrap();
        assert_eq!(original.alpha, 2);
        let explicit = RoleRecipe::resolve(
            &f,
            &RasterDisplay::Gray {
                band: 1,
                low: 0.,
                high: 1.,
                alpha_band: Some(1),
            },
        )
        .unwrap();
        assert_eq!(explicit.alpha, 1);
    }
}
