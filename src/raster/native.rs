//! Narrow synchronous native owners. Explicit close precedes root publication.
use super::{
    display,
    grid::{GridPlan, StaticCrs},
    source::{
        self, AreaPoint, BandFacts, Driver, LeafKind, SampleType, ScalarFact, SourceFacts,
        SourceLeaf, SourcePlan, TextSpan,
    },
    tile, RasterCapabilities,
};
use crate::{
    geospatial::QuietErrors, runtime::Attempt, JobError, JobErrorKind, JobFailure, RunEvent,
};
use source::{exact, invalid, limit, unsupported};
use std::{
    ffi::{c_char, c_int, c_void, CStr, CString},
    path::Path,
    ptr::{null, null_mut, NonNull},
    sync::Mutex,
};
pub(super) fn failure(error: JobError) -> JobFailure {
    JobFailure {
        error,
        secondary: Vec::new(),
        retained_paths: Vec::new(),
        recovery: None,
    }
}
pub(super) fn secondary(f: &mut JobFailure, r: Result<(), JobError>) {
    if let Err(e) = r {
        f.secondary.push(e);
    }
}
pub(super) fn init() -> Result<(), JobError> {
    crate::geospatial::native::init().map_err(|e| {
        JobError::with_cause(
            JobErrorKind::Unsupported,
            "native raster initialization failed",
            e,
        )
    })
}
pub(super) fn path(p: &Path) -> Result<CString, JobError> {
    #[cfg(windows)]
    let b = p
        .to_str()
        .ok_or_else(|| unsupported("native raster path requires UTF8 on Windows"))?
        .as_bytes();
    #[cfg(not(windows))]
    let b = p.as_os_str().as_encoded_bytes();
    cstring(b)
}
/// All external native text is length-scanned before copying or formatting.
pub(super) unsafe fn bytes<'a>(p: *const c_char, max: usize) -> Result<&'a [u8], JobError> {
    if p.is_null() {
        return Ok(&[]);
    }
    for n in 0..=max {
        if unsafe { *p.add(n) } == 0 {
            return Ok(unsafe { std::slice::from_raw_parts(p.cast(), n) });
        }
    }
    Err(unsupported("native raster metadata text exceeds profile"))
}
pub(super) fn version() -> Result<String, JobError> {
    let b = unsafe { bytes(gdal_sys::GDALVersionInfo(c"RELEASE_NAME".as_ptr()), 64) }?;
    let mut owned = exact(b.len())?;
    owned.extend_from_slice(b);
    String::from_utf8(owned).map_err(|_| unsupported("native version is not UTF8"))
}
#[derive(Debug)]
struct NativeDiagnostic {
    number: i32,
    category: u32,
    text: std::borrow::Cow<'static, str>,
}
impl std::fmt::Display for NativeDiagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "native error {} category {}: {}",
            self.number, self.category, self.text
        )
    }
}
impl std::error::Error for NativeDiagnostic {}
pub(super) fn diagnostic_owner_bytes() -> usize {
    std::mem::size_of::<NativeDiagnostic>() + 2 * std::mem::size_of::<usize>() + 512
}
pub(super) fn error(stage: &'static str) -> JobError {
    let number = unsafe { gdal_sys::CPLGetLastErrorNo() };
    let category = unsafe { gdal_sys::CPLGetLastErrorType() };
    let pointer = unsafe { gdal_sys::CPLGetLastErrorMsg() };
    let mut copied = [0u8; 512];
    let mut length = 0;
    if !pointer.is_null() {
        for (i, slot) in copied.iter_mut().enumerate() {
            let value = unsafe { *pointer.add(i) } as u8;
            if value == 0 {
                break;
            }
            *slot = value;
            length = i + 1;
        }
    }
    let text = diagnostic_text(&copied[..length]);
    JobError::with_cause(
        match number {
            2 => JobErrorKind::ResourceLimit,
            3 | 4 => JobErrorKind::Io,
            6 => JobErrorKind::Unsupported,
            9 => JobErrorKind::Cancelled,
            0 => JobErrorKind::InvalidState,
            _ => JobErrorKind::InvalidInput,
        },
        stage,
        NativeDiagnostic {
            number,
            category,
            text,
        },
    )
}

fn diagnostic_text(mut bytes: &[u8]) -> std::borrow::Cow<'static, str> {
    let mut owned = String::new();
    if owned.try_reserve_exact(512).is_err() || owned.capacity() != 512 {
        drop(owned);
        return std::borrow::Cow::Borrowed("native diagnostic copy unavailable");
    }
    while !bytes.is_empty() {
        let (valid, skip) = match std::str::from_utf8(bytes) {
            Ok(s) => (s, 0),
            Err(e) => {
                let prefix =
                    std::str::from_utf8(&bytes[..e.valid_up_to()]).expect("validated UTF8 prefix");
                (
                    prefix,
                    e.error_len().unwrap_or(bytes.len() - e.valid_up_to()),
                )
            }
        };
        for character in valid.chars() {
            if owned.len() + character.len_utf8() > 512 {
                return std::borrow::Cow::Owned(owned);
            }
            owned.push(character);
        }
        if skip == 0 {
            break;
        }
        if owned.len() + 3 > 512 {
            break;
        }
        owned.push('\u{fffd}');
        bytes = &bytes[valid.len() + skip..];
    }
    std::borrow::Cow::Owned(owned)
}

pub(super) struct Dataset(Option<NonNull<c_void>>);
impl Dataset {
    pub(super) fn raw(&self) -> gdal_sys::GDALDatasetH {
        self.0.expect("live native dataset").as_ptr()
    }
    pub(super) fn owned(p: gdal_sys::GDALDatasetH) -> Result<Self, JobError> {
        NonNull::new(p)
            .map(|p| Self(Some(p)))
            .ok_or_else(|| error("native dataset failed"))
    }
    pub(super) fn open(p: &Path) -> Result<Self, JobError> {
        let _quiet = QuietErrors::new();
        {
            use std::io::Read;
            let mut header = [0u8; 2];
            let mut file =
                std::fs::File::open(p).map_err(|e| JobError::io("open raster signature", p, e))?;
            let n = file
                .read(&mut header)
                .map_err(|e| JobError::io("read raster signature", p, e))?;
            if n == 2
                && header == [0xff, 0xd8]
                && unsafe { gdal_sys::GDALGetDriverByName(c"JPEG".as_ptr()) }.is_null()
            {
                return Err(unsupported("requested JPEG source driver unavailable"));
            }
        }
        let p = path(p)?;
        let drivers = [
            c"GTiff".as_ptr(),
            c"PNG".as_ptr(),
            c"JPEG".as_ptr(),
            c"AAIGrid".as_ptr(),
            null(),
        ];
        let opts = [
            c"GEOREF_SOURCES=INTERNAL,PAM,WORLDFILE".as_ptr(),
            c"COLOR_TABLE_MULTIPLIER=257".as_ptr(),
            null(),
        ];
        unsafe {
            Self::owned(gdal_sys::GDALOpenEx(
                p.as_ptr(),
                0x02,
                drivers.as_ptr(),
                opts.as_ptr(),
                null(),
            ))
        }
    }
    pub(super) fn close(mut self) -> Result<(), JobError> {
        let p = self.0.take().expect("live native dataset");
        if unsafe { gdal_sys::GDALClose(p.as_ptr()) } != 0 {
            Err(error("close native raster"))
        } else {
            Ok(())
        }
    }
    pub(super) fn band(&self, index: usize) -> gdal_sys::GDALRasterBandH {
        unsafe { gdal_sys::GDALGetRasterBand(self.raw(), index as c_int) }
    }
}
impl Drop for Dataset {
    fn drop(&mut self) {
        if let Some(p) = self.0.take() {
            unsafe {
                gdal_sys::GDALClose(p.as_ptr());
            }
        }
    }
}

const CONFIG_KEYS: [&CStr; 3] = [c"CPL_TMPDIR", c"GDAL_PAM_ENABLED", c"GDAL_NUM_THREADS"];
pub(super) struct Config {
    old: [Option<CString>; 3],
    debug: [Option<CString>; 2],
    active: bool,
}
impl Config {
    pub(super) fn capture() -> Result<Self, JobError> {
        let mut total = 0;
        let mut read = |key: &CStr, local: bool| -> Result<Option<CString>, JobError> {
            let p = unsafe {
                if local {
                    gdal_sys::CPLGetThreadLocalConfigOption(key.as_ptr(), null())
                } else {
                    gdal_sys::CPLGetConfigOption(key.as_ptr(), null())
                }
            };
            if p.is_null() {
                return Ok(None);
            }
            let b = unsafe { bytes(p, 4096) }?;
            total += b.len() + 1;
            if total > 4096 {
                return Err(unsupported("native configuration exceeds4096 bytes"));
            }
            Ok(Some(cstring(b)?))
        };
        let old = [
            read(CONFIG_KEYS[0], true)?,
            read(CONFIG_KEYS[1], true)?,
            read(CONFIG_KEYS[2], true)?,
        ];
        let debug = [
            read(c"COG_TMP_COMPRESSION", false)?,
            read(c"COG_DELETE_TEMP_FILES", false)?,
        ];
        if unsafe {
            !gdal_sys::CPLGetConfigOption(c"GDAL_PAM_PROXY_DIR".as_ptr(), null()).is_null()
        } {
            return Err(unsupported("proxy PAM authority is unsupported"));
        }
        let point = unsafe {
            bytes(
                gdal_sys::CPLGetConfigOption(c"GTIFF_POINT_GEO_IGNORE".as_ptr(), null()),
                16,
            )
        }?;
        if !point.is_empty() && !point.eq_ignore_ascii_case(b"NO") {
            return Err(unsupported("inherited GTiff Point interpretation override"));
        }
        Ok(Self {
            old,
            debug,
            active: true,
        })
    }
    pub(super) fn take(&mut self) -> Self {
        Self {
            old: std::mem::take(&mut self.old),
            debug: std::mem::take(&mut self.debug),
            active: std::mem::replace(&mut self.active, false),
        }
    }
    pub(super) fn owned_bytes(&self) -> usize {
        self.old
            .iter()
            .chain(&self.debug)
            .flatten()
            .map(|s| s.as_bytes_with_nul().len())
            .sum()
    }
    pub(super) fn set(&mut self, key: &CStr, value: &str) -> Result<(), JobError> {
        let v = cstring(value.as_bytes())?;
        unsafe { gdal_sys::CPLSetThreadLocalConfigOption(key.as_ptr(), v.as_ptr()) };
        Ok(())
    }
    pub(super) fn restore(&mut self) -> Result<(), JobError> {
        if self.active {
            for (k, v) in CONFIG_KEYS.iter().zip(&self.old) {
                unsafe {
                    gdal_sys::CPLSetThreadLocalConfigOption(
                        k.as_ptr(),
                        v.as_ref().map_or(null(), |s| s.as_ptr()),
                    )
                }
            }
            self.active = false;
        }
        Ok(())
    }
    pub(super) fn recheck_debug(&self) -> Result<(), JobError> {
        for (i, k) in [c"COG_TMP_COMPRESSION", c"COG_DELETE_TEMP_FILES"]
            .iter()
            .enumerate()
        {
            let p = unsafe { gdal_sys::CPLGetConfigOption(k.as_ptr(), null()) };
            let current = if p.is_null() {
                None
            } else {
                Some(unsafe { bytes(p, 4096) }?)
            };
            if current != self.debug[i].as_ref().map(|s| s.as_bytes()) {
                return Err(JobError::new(
                    JobErrorKind::Conflict,
                    "native COG configuration changed",
                ));
            }
        }
        Ok(())
    }
    pub(super) fn validate_cog(&self) -> Result<(), JobError> {
        self.recheck_debug()?;
        let metadata = unsafe {
            gdal_sys::GDALGetMetadataItem(
                gdal_sys::GDALGetDriverByName(c"GTiff".as_ptr()),
                c"DMD_CREATIONOPTIONLIST".as_ptr(),
                null(),
            )
        };
        let xml = unsafe { bytes(metadata, 65536) }?;
        let codecs = codec_values(xml)?;
        let compression = self.debug[0].as_ref().map(|s| s.as_bytes());
        let chosen = match compression {
            None => {
                if codecs.1 {
                    b"ZSTD".as_slice()
                } else {
                    b"LZW".as_slice()
                }
            }
            Some(v) if v.eq_ignore_ascii_case(b"ZSTD") => b"ZSTD",
            Some(v) if v.eq_ignore_ascii_case(b"LZW") => b"LZW",
            _ => return Err(unsupported("COG temporary compression must be ZSTD or LZW")),
        };
        if (chosen == b"ZSTD" && !codecs.1) || (chosen == b"LZW" && !codecs.0) {
            return Err(unsupported("COG temporary codec unavailable"));
        }
        if self.debug[1]
            .as_ref()
            .is_some_and(|s| !s.as_bytes().eq_ignore_ascii_case(b"YES"))
        {
            return Err(unsupported("COG temporary deletion must be YES"));
        }
        Ok(())
    }
}
impl Drop for Config {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

/// A finite argv owner; every CString and pointer survives the synchronous call.
pub(super) struct Args {
    _strings: Vec<CString>,
    pointers: Vec<*mut c_char>,
}
impl Args {
    pub(super) fn new(values: &[String]) -> Result<Self, JobError> {
        if values.len() > ARG_SLOTS {
            return Err(invalid("native raster argv count"));
        }
        let mut strings = exact(values.len())?;
        for s in values {
            strings.push(cstring(s.as_bytes())?);
        }
        let mut pointers = exact(strings.len() + 1)?;
        for s in &strings {
            pointers.push(s.as_ptr().cast_mut());
        }
        pointers.push(null_mut());
        Ok(Self {
            _strings: strings,
            pointers,
        })
    }
    pub(super) fn ptr(&self) -> *mut *mut c_char {
        self.pointers.as_ptr().cast_mut()
    }
    pub(super) fn const_ptr(&self) -> *const *const c_char {
        self.pointers.as_ptr().cast()
    }
}

pub(super) struct Progress<'a> {
    attempt: &'a Attempt,
    phase: &'static str,
    state: Mutex<(u64, Option<JobError>)>,
}
impl<'a> Progress<'a> {
    pub(super) fn new(attempt: &'a Attempt, phase: &'static str) -> Self {
        Self {
            attempt,
            phase,
            state: Mutex::new((0, None)),
        }
    }
    pub(super) fn finish(&self, ok: bool) -> Result<(), JobError> {
        // Event admission records an observer unwind before the native callback
        // catches it. Preserve that causal abort before inspecting its mutex.
        self.attempt.check()?;
        let s = self.state.lock().map_err(|_| {
            JobError::new(JobErrorKind::InvalidState, "native progress lock poisoned")
        })?;
        if let Some(e) = &s.1 {
            return Err(e.clone());
        }
        if !ok {
            Err(error(self.phase))
        } else {
            self.attempt.check()
        }
    }
}
pub(super) unsafe extern "C" fn callback(f: f64, _msg: *const c_char, data: *mut c_void) -> c_int {
    let p = unsafe { &*data.cast::<Progress<'_>>() };
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut s = p.state.lock().map_err(|_| {
            JobError::new(JobErrorKind::InvalidState, "native progress lock poisoned")
        })?;
        if s.1.is_some() {
            return Ok(false);
        }
        if let Err(e) = p.attempt.check() {
            s.1 = Some(e);
            return Ok(false);
        }
        let done = if f.is_finite() {
            (f.clamp(0., 1.) * 1_000_000.) as u64
        } else {
            0
        };
        if done > s.0 {
            if let Err(e) = p.attempt.emit(&RunEvent::Progress {
                phase: p.phase,
                done,
                total: Some(1_000_000),
            }) {
                s.1 = Some(e);
                return Ok(false);
            }
            s.0 = done;
        }
        Ok::<bool, JobError>(true)
    }));
    match result {
        Ok(Ok(true)) => 1,
        Ok(Ok(false)) => 0,
        Ok(Err(e)) => {
            if let Ok(mut s) = p.state.lock() {
                if s.1.is_none() {
                    s.1 = Some(e);
                }
            }
            0
        }
        Err(_) => {
            let mut s = p.state.lock().unwrap_or_else(|poison| poison.into_inner());
            if s.1.is_none() {
                s.1 = Some(p.attempt.check().err().unwrap_or_else(|| {
                    JobError::new(
                        JobErrorKind::ObserverFailure,
                        "panic during native raster progress",
                    )
                }));
            }
            0
        }
    }
}

#[repr(C)]
struct XmlNode {
    kind: c_int,
    value: *mut c_char,
    next: *mut XmlNode,
    child: *mut XmlNode,
}
unsafe extern "C" {
    fn CPLParseXMLString(text: *const c_char) -> *mut XmlNode;
    fn CPLDestroyXMLNode(node: *mut XmlNode);
}
struct Xml(*mut XmlNode);
impl Drop for Xml {
    fn drop(&mut self) {
        unsafe { CPLDestroyXMLNode(self.0) }
    }
}
fn xml(text: &[u8]) -> Result<Xml, JobError> {
    let text = CString::new(text).map_err(|_| invalid("NUL XML"))?;
    let p = unsafe { CPLParseXMLString(text.as_ptr()) };
    if p.is_null() {
        Err(invalid("invalid native XML"))
    } else {
        Ok(Xml(p))
    }
}
fn walk_xml(
    root: *mut XmlNode,
    mut f: impl FnMut(c_int, &[u8], *mut XmlNode) -> Result<(), JobError>,
) -> Result<(), JobError> {
    let mut stack = [null_mut(); 64];
    let mut depth = 0;
    let mut p = root;
    let mut nodes = 0;
    while !p.is_null() {
        nodes += 1;
        if nodes > 65536 {
            return Err(limit("raster XML node limit"));
        }
        let n = unsafe { &*p };
        let name = unsafe { bytes(n.value, 1_048_576) }?;
        f(n.kind, name, p)?;
        if !n.child.is_null() {
            if depth == 64 {
                return Err(limit("raster XML depth limit"));
            }
            stack[depth] = n.next;
            depth += 1;
            p = n.child;
        } else if !n.next.is_null() {
            p = n.next;
        } else {
            p = null_mut();
            while depth > 0 && p.is_null() {
                depth -= 1;
                p = stack[depth];
            }
        }
    }
    Ok(())
}
fn codec_values(text: &[u8]) -> Result<(bool, bool), JobError> {
    let tree = xml(text)?;
    let mut result = (false, false);
    walk_xml(tree.0, |kind, name, node| {
        if kind == 0 && name == b"Option" {
            let mut child = unsafe { (*node).child };
            let mut compress = false;
            while !child.is_null() {
                let n = unsafe { &*child };
                if n.kind == 2 && unsafe { bytes(n.value, 64) }? == b"name" && !n.child.is_null() {
                    compress = unsafe { bytes((*n.child).value, 64) }? == b"COMPRESS";
                }
                child = n.next;
            }
            if compress {
                let mut child = unsafe { (*node).child };
                while !child.is_null() {
                    let n = unsafe { &*child };
                    if n.kind == 0
                        && unsafe { bytes(n.value, 64) }? == b"Value"
                        && !n.child.is_null()
                    {
                        let v = unsafe { bytes((*n.child).value, 64) }?;
                        result.0 |= v == b"LZW";
                        result.1 |= v == b"ZSTD";
                    }
                    child = n.next;
                }
            }
        }
        Ok(())
    })?;
    Ok(result)
}
pub(super) fn screen_pam(leaf: &SourceLeaf) -> Result<(), JobError> {
    let text = leaf.read_text()?;
    screen_pam_text(&text)
}
fn screen_pam_text(text: &[u8]) -> Result<(), JobError> {
    let tree = xml(text)?;
    walk_xml(tree.0, |kind, name, node| {
        if kind == 0 && name == b"SRS" {
            screen_srs(xml_text(node)?)?;
        }
        if kind == 4 || name.starts_with(b"?") {
            return Err(unsupported("PAM instruction/literal"));
        }
        if kind == 0
            && !matches!(
                name,
                b"PAMDataset"
                    | b"SRS"
                    | b"GeoTransform"
                    | b"PAMRasterBand"
                    | b"NoDataValue"
                    | b"Scale"
                    | b"Offset"
                    | b"UnitType"
                    | b"ColorInterp"
                    | b"ColorTable"
                    | b"Entry"
                    | b"Metadata"
                    | b"MDI"
            )
        {
            return Err(unsupported("PAM field outside finite profile"));
        }
        if kind == 2 {
            let val = if unsafe { (*node).child }.is_null() {
                &[][..]
            } else {
                unsafe { bytes((*(*node).child).value, 1_048_576) }?
            };
            if name == b"domain" && !val.is_empty() || name == b"key" && val == b"NODATA_VALUES" {
                return Err(unsupported("PAM reference/joint metadata authority"));
            }
            if !matches!(
                name,
                b"band"
                    | b"domain"
                    | b"key"
                    | b"c1"
                    | b"c2"
                    | b"c3"
                    | b"c4"
                    | b"dataAxisToSRSAxisMapping"
            ) {
                return Err(unsupported("PAM attribute outside finite profile"));
            }
        }
        Ok(())
    })
}
fn add_text(out: &mut Vec<u8>, text: &[u8]) -> Result<TextSpan, JobError> {
    if out.len() + text.len() > 1_048_576 {
        return Err(limit("critical raster metadata text limit"));
    }
    let span = TextSpan {
        start: out.len() as u32,
        len: text.len() as u32,
    };
    out.extend_from_slice(text);
    Ok(span)
}
fn check_domains(ds: &Dataset) -> Result<(), JobError> {
    unsafe {
        if gdal_sys::GDALGetGCPCount(ds.raw()) != 0 {
            return Err(unsupported("raster GCP authority"));
        }
        for d in [c"RPC", c"GEOLOCATION", c"SUBDATASETS"] {
            let p = gdal_sys::GDALGetMetadata(ds.raw(), d.as_ptr());
            if !p.is_null() && !(*p).is_null() {
                return Err(unsupported("raster reference domain"));
            }
        }
        if !gdal_sys::GDALGetMetadataItem(ds.raw(), c"NODATA_VALUES".as_ptr(), null()).is_null() {
            return Err(unsupported("joint NoData metadata"));
        }
    }
    Ok(())
}
fn crs(ds: &Dataset, required: bool) -> Result<StaticCrs, JobError> {
    unsafe {
        let s = gdal_sys::GDALGetSpatialRef(ds.raw());
        if s.is_null() {
            return if required {
                Err(invalid("raster source CRS missing"))
            } else {
                Ok(StaticCrs::Geographic4326)
            };
        }
        for (epsg, kind) in [
            (4326, StaticCrs::Geographic4326),
            (3857, StaticCrs::WebMercator3857),
        ] {
            let expected = gdal_sys::OSRNewSpatialReference(null());
            if expected.is_null() {
                return Err(error("allocate spatial reference"));
            }
            let imported = gdal_sys::OSRImportFromEPSG(expected, epsg) == 0;
            let opts = [c"IGNORE_DATA_AXIS_TO_SRS_AXIS_MAPPING=YES".as_ptr(), null()];
            let same = imported && gdal_sys::OSRIsSameEx(s, expected, opts.as_ptr()) != 0;
            gdal_sys::OSRDestroySpatialReference(expected);
            if same {
                let mut count = 0;
                let mapping = gdal_sys::OSRGetDataAxisToSRSAxisMapping(s, &mut count);
                let expected_mapping = match kind {
                    StaticCrs::Geographic4326 => [2, 1],
                    StaticCrs::WebMercator3857 => [1, 2],
                };
                if count != 2
                    || mapping.is_null()
                    || std::slice::from_raw_parts(mapping, 2) != expected_mapping
                {
                    return Err(unsupported("source CRS data axes are outside x/y profile"));
                }
                return Ok(kind);
            }
        }
    }
    Err(unsupported(
        "raster display CRS must be static EPSG4326 or3857",
    ))
}
pub(super) fn block(
    band: gdal_sys::GDALRasterBandH,
    ty: SampleType,
) -> Result<(u32, u32), JobError> {
    let (mut w, mut h) = (0, 0);
    unsafe { gdal_sys::GDALGetBlockSize(band, &mut w, &mut h) };
    if w <= 0 || h <= 0 {
        return Err(invalid("raster block dimensions"));
    }
    let pixels = (w as u64)
        .checked_mul(h as u64)
        .ok_or_else(|| limit("raster block overflow"))?;
    if pixels > 1_048_576
        || pixels
            .checked_mul(ty.bytes() as u64)
            .is_none_or(|n| n > 16_777_216)
    {
        return Err(limit("raster block admission limit"));
    }
    Ok((w as u32, h as u32))
}
fn checked_mask(
    band: gdal_sys::GDALRasterBandH,
    w: i32,
    h: i32,
) -> Result<gdal_sys::GDALRasterBandH, JobError> {
    let mask = unsafe { gdal_sys::GDALGetMaskBand(band) };
    if mask.is_null() {
        return Err(invalid("raster mask missing"));
    }
    if unsafe { gdal_sys::GDALGetRasterDataType(mask) } != 1
        || unsafe { gdal_sys::GDALGetRasterBandXSize(mask) } != w
        || unsafe { gdal_sys::GDALGetRasterBandYSize(mask) } != h
    {
        return Err(unsupported("raster mask shape/type"));
    }
    block(mask, SampleType::Byte)?;
    Ok(mask)
}
pub(super) fn admit_mask(p: &Path) -> Result<(u32, u32), JobFailure> {
    let ds = Dataset::open(p).map_err(failure)?;
    let result = (|| {
        let w = unsafe { gdal_sys::GDALGetRasterXSize(ds.raw()) };
        let h = unsafe { gdal_sys::GDALGetRasterYSize(ds.raw()) };
        let count = unsafe { gdal_sys::GDALGetRasterCount(ds.raw()) };
        if w <= 0 || h <= 0 || w > 65536 || h > 65536 || !(1..=32).contains(&count) {
            return Err(unsupported("external mask shape"));
        }
        for i in 1..=count {
            let b = ds.band(i as usize);
            if unsafe { gdal_sys::GDALGetRasterDataType(b) } != 1 {
                return Err(unsupported("external mask must be Byte"));
            }
            block(b, SampleType::Byte)?;
        }
        check_domains(&ds)?;
        check_standalone(&ds, p)?;
        Ok((w as u32, h as u32))
    })();
    let close = ds.close();
    match result {
        Ok(dims) => {
            close.map_err(failure)?;
            Ok(dims)
        }
        Err(e) => {
            let mut f = failure(e);
            secondary(&mut f, close);
            Err(f)
        }
    }
}
pub(super) fn inspect(
    ds: &Dataset,
    source_bytes: u64,
    required: bool,
) -> Result<SourceFacts, JobError> {
    let _quiet = QuietErrors::new();
    check_domains(ds)?;
    let w = unsafe { gdal_sys::GDALGetRasterXSize(ds.raw()) };
    let h = unsafe { gdal_sys::GDALGetRasterYSize(ds.raw()) };
    let count = unsafe { gdal_sys::GDALGetRasterCount(ds.raw()) };
    if w <= 0 || h <= 0 || !(1..=32).contains(&count) {
        return Err(unsupported("raster shape/band profile"));
    }
    let d = unsafe {
        bytes(
            gdal_sys::GDALGetDriverShortName(gdal_sys::GDALGetDatasetDriver(ds.raw())),
            32,
        )
    }?;
    let driver = match d {
        b"GTiff" => Driver::GTiff,
        b"PNG" => Driver::Png,
        b"JPEG" => Driver::Jpeg,
        b"AAIGrid" => Driver::AAIGrid,
        _ => return Err(unsupported("raster source driver")),
    };
    let mut affine = [0.; 6];
    if unsafe { gdal_sys::GDALGetGeoTransform(ds.raw(), affine.as_mut_ptr()) } != 0 {
        if required {
            return Err(invalid("raster affine missing"));
        }
        affine = [0.; 6];
    }
    if affine.iter().any(|v| !v.is_finite()) {
        return Err(invalid("nonfinite raster affine"));
    }
    let crs = crs(ds, required)?;
    let proj = unsafe { bytes(gdal_sys::GDALGetProjectionRef(ds.raw()), 1_048_576) }?;
    let mut text_size = proj.len();
    let mut palette_size = 0;
    for i in 1..=count {
        let b = ds.band(i as usize);
        text_size = text_size
            .checked_add(unsafe { bytes(gdal_sys::GDALGetRasterUnitType(b), 1_048_576) }?.len())
            .ok_or_else(|| limit("metadata overflow"))?;
        let ct = unsafe { gdal_sys::GDALGetRasterColorTable(b) };
        if !ct.is_null() {
            let n = unsafe { gdal_sys::GDALGetColorEntryCount(ct) };
            if !(0..=256).contains(&n) {
                return Err(unsupported("palette size"));
            }
            palette_size += n as usize;
        }
    }
    if text_size > 1_048_576 {
        return Err(limit("critical metadata text limit"));
    }
    let mut text = exact(text_size)?;
    let projection = add_text(&mut text, proj)?;
    let mut palettes = exact(palette_size)?;
    let mut bands = exact(count as usize)?;
    for i in 1..=count {
        let b = ds.band(i as usize);
        let datatype = SampleType::from_native(unsafe { gdal_sys::GDALGetRasterDataType(b) })?;
        block(b, datatype)?;
        let mut present = 0;
        let nd = unsafe { gdal_sys::GDALGetRasterNoDataValue(b, &mut present) };
        let nodata = ScalarFact::new(nd, present != 0);
        if nodata.present {
            datatype.nodata(nd)?;
        }
        let sc = unsafe { gdal_sys::GDALGetRasterScale(b, &mut present) };
        let scale = ScalarFact::new(sc, present != 0);
        let of = unsafe { gdal_sys::GDALGetRasterOffset(b, &mut present) };
        let offset = ScalarFact::new(of, present != 0);
        if (scale.present && !sc.is_finite()) || (offset.present && !of.is_finite()) {
            return Err(unsupported("nonfinite scale/offset"));
        }
        let unit = add_text(&mut text, unsafe {
            bytes(gdal_sys::GDALGetRasterUnitType(b), 1_048_576)
        }?)?;
        let color = unsafe { gdal_sys::GDALGetRasterColorInterpretation(b) };
        if color > 6 {
            return Err(unsupported("raster color role"));
        }
        let color_interp = color as u8;
        let raw_mask = unsafe { gdal_sys::GDALGetMaskFlags(b) };
        if !matches!(raw_mask, 1 | 2 | 6 | 8 | 10) {
            return Err(unsupported("unknown raster mask flag bits"));
        }
        let mask_class = raw_mask as u8;
        if !matches!(mask_class, 1 | 2 | 6 | 8 | 10) {
            return Err(unsupported("nonshared/unknown raster mask"));
        }
        if matches!(mask_class, 8 | 10) && !nodata.present {
            return Err(unsupported("derived mask lacks independent NoData"));
        }
        let ct = unsafe { gdal_sys::GDALGetRasterColorTable(b) };
        let palette = TextSpan {
            start: palettes.len() as u32,
            len: if ct.is_null() {
                0
            } else {
                (unsafe { gdal_sys::GDALGetColorEntryCount(ct) }) as u32
            },
        };
        for j in 0..palette.len {
            let entry = unsafe { gdal_sys::GDALGetColorEntry(ct, j as c_int) };
            if entry.is_null() {
                return Err(invalid("palette entry missing"));
            }
            let e = unsafe { &*entry };
            if !(0..=255).contains(&e.c1)
                || !(0..=255).contains(&e.c2)
                || !(0..=255).contains(&e.c3)
                || e.c4 != 255
            {
                return Err(unsupported("nonopaque/nonbyte normalized palette"));
            }
            palettes.push([e.c1 as u8, e.c2 as u8, e.c3 as u8, 255]);
        }
        bands.push(BandFacts {
            nodata,
            scale,
            offset,
            unit,
            palette,
            datatype,
            color_interp,
            mask_class,
        });
    }
    let first = bands[0];
    if bands
        .iter()
        .any(|b| b.datatype != first.datatype || !b.nodata.same(first.nodata))
    {
        return Err(unsupported("mixed sample type or unequal band NoData"));
    }
    let alpha = bands.iter().filter(|b| b.color_interp == 6).count();
    if alpha > 1 || bands.iter().any(|b| b.mask_class == 6) && alpha != 1 {
        return Err(unsupported("ambiguous inherited alpha association"));
    }
    let alpha_index = bands
        .iter()
        .position(|b| b.color_interp == 6)
        .map(|i| i + 1);
    let mut ordinary: gdal_sys::GDALRasterBandH = null_mut();
    for (i, b) in bands.iter().enumerate() {
        // The guard covers the actual internal/derived/virtual RasterIO handle.
        let mask = checked_mask(ds.band(i + 1), w, h)?;
        if b.mask_class == 6
            && b.datatype == SampleType::Byte
            && Some(mask) != alpha_index.map(|a| ds.band(a))
        {
            return Err(unsupported("alpha mask is not the declared alpha band"));
        }
        if b.mask_class == 2 {
            if ordinary.is_null() {
                ordinary = mask;
            } else if ordinary != mask {
                return Err(unsupported("nonshared ordinary mask associations"));
            }
        }
    }
    let area = unsafe {
        bytes(
            gdal_sys::GDALGetMetadataItem(ds.raw(), c"AREA_OR_POINT".as_ptr(), null()),
            16,
        )
    }?;
    let area_point = match area {
        b"" | b"Area" => AreaPoint::Area,
        b"Point" => AreaPoint::Point,
        _ => return Err(unsupported("unknown Area/Point")),
    };
    Ok(SourceFacts {
        width: w as u32,
        height: h as u32,
        affine,
        crs,
        source_bytes,
        driver,
        area_point,
        area_present: !area.is_empty(),
        projection,
        text,
        bands,
        palettes,
    })
}
pub(super) fn compare_primary(p: &SourceFacts, e: &SourceFacts) -> Result<(), JobError> {
    if p.width != e.width || p.height != e.height || p.bands.len() != e.bands.len() {
        return Err(invalid("primary source shape changed by companion"));
    }
    if p.affine != [0.; 6] && p.affine.map(f64::to_bits) != e.affine.map(f64::to_bits) {
        return Err(invalid("companion conflicts with primary affine"));
    }
    if p.projection.len != 0 && p.crs != e.crs {
        return Err(invalid("companion conflicts with primary CRS"));
    }
    if p.area_present && p.area_point != e.area_point {
        return Err(invalid("companion conflicts with primary Area/Point"));
    }
    for (a, b) in p.bands.iter().zip(&e.bands) {
        if a.datatype != b.datatype
            || (a.nodata.present && !a.nodata.same(b.nodata))
            || (a.scale.present && !a.scale.same(b.scale))
            || (a.offset.present && !a.offset.same(b.offset))
            || (a.color_interp != 0 && a.color_interp != b.color_interp)
            || (!p.text(a.unit).is_empty() && p.text(a.unit) != e.text(b.unit))
            || (!p.palette(a.palette).is_empty() && p.palette(a.palette) != e.palette(b.palette))
        {
            return Err(invalid("companion conflicts with primary band authority"));
        }
    }
    Ok(())
}
pub(super) fn check_file_list(ds: &Dataset, leaves: &[SourceLeaf]) -> Result<(), JobError> {
    let p = unsafe { gdal_sys::GDALGetFileList(ds.raw()) };
    if p.is_null() {
        return Err(invalid("native source file list missing"));
    }
    let result = (|| {
        for i in 0..=5 {
            let item = unsafe { *p.add(i) };
            if item.is_null() {
                return Ok(());
            }
            if i == 5 {
                return Err(unsupported(
                    "native source dependencies exceed admitted inventory",
                ));
            }
            let b = unsafe { bytes(item, 65536) }?;
            #[cfg(unix)]
            let path = {
                use std::os::unix::ffi::OsStrExt;
                Path::new(std::ffi::OsStr::from_bytes(b))
            };
            #[cfg(not(unix))]
            let path = Path::new(
                std::str::from_utf8(b).map_err(|_| unsupported("native dependency path UTF8"))?,
            );
            let actual = std::fs::canonicalize(path)
                .map_err(|e| JobError::io("resolve native dependency", path, e))?;
            if !leaves.iter().any(|l| l.path == actual) {
                return Err(unsupported(
                    "native consumed an unadmitted source dependency",
                ));
            }
        }
        Err(unsupported("native source file list limit"))
    })();
    unsafe { gdal_sys::CSLDestroy(p) };
    result
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ColorRoleDelta {
    Exact,
    SingleUndefinedGray,
}
impl ColorRoleDelta {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::SingleUndefinedGray => "single_undefined_gray",
        }
    }
}
pub(super) struct NativeProduced {
    pub source_cog_bytes: u64,
    pub tiles: u64,
    pub finest_rgba_bytes: u64,
    pub source_color_interp: u8,
    pub target_color_interp: u8,
    pub role_delta: ColorRoleDelta,
    pub native_version: String,
}
fn gray_delta(f: &SourceFacts) -> ColorRoleDelta {
    if f.bands.len() == 1 && f.bands[0].color_interp == 0 && f.bands[0].palette.len == 0 {
        ColorRoleDelta::SingleUndefinedGray
    } else {
        ColorRoleDelta::Exact
    }
}
fn create(p: &Path, w: u32, h: u32, bands: i32, ty: u32) -> Result<Dataset, JobError> {
    let driver = unsafe { gdal_sys::GDALGetDriverByName(c"GTiff".as_ptr()) };
    if driver.is_null() {
        return Err(unsupported("GTiff output driver unavailable"));
    }
    let p = path(p)?;
    let opts = [
        c"TILED=YES".as_ptr().cast_mut(),
        c"COMPRESS=NONE".as_ptr().cast_mut(),
        c"BIGTIFF=IF_SAFER".as_ptr().cast_mut(),
        null_mut(),
    ];
    unsafe {
        Dataset::owned(gdal_sys::GDALCreate(
            driver,
            p.as_ptr(),
            w as i32,
            h as i32,
            bands,
            ty,
            opts.as_ptr().cast_mut().cast(),
        ))
    }
}
pub(super) fn create_rgba(
    p: &Path,
    w: u32,
    h: u32,
    gt: [f64; 6],
    source: Option<&Dataset>,
) -> Result<Dataset, JobFailure> {
    let ds = create(p, w, h, 4, 1).map_err(failure)?;
    let result = (|| {
        if unsafe { gdal_sys::GDALSetGeoTransform(ds.raw(), gt.as_ptr().cast_mut()) } != 0 {
            return Err(error("set exact RGBA geotransform"));
        }
        let s = unsafe { gdal_sys::OSRNewSpatialReference(null()) };
        if s.is_null() {
            return Err(error("allocate RGBA CRS"));
        }
        let result = unsafe {
            if let Some(src) = source {
                gdal_sys::GDALSetSpatialRef(ds.raw(), gdal_sys::GDALGetSpatialRef(src.raw()))
            } else {
                let code = gdal_sys::OSRImportFromEPSG(s, 3857);
                if code == 0 {
                    gdal_sys::GDALSetSpatialRef(ds.raw(), s)
                } else {
                    code
                }
            }
        };
        unsafe { gdal_sys::OSRDestroySpatialReference(s) };
        if result != 0 {
            return Err(error("set RGBA CRS"));
        }
        for i in 1..=4 {
            if unsafe {
                gdal_sys::GDALSetRasterColorInterpretation(ds.band(i), [3, 4, 5, 6][i - 1])
            } != 0
                || unsafe { gdal_sys::GDALFillRaster(ds.band(i), 0., 0.) } != 0
            {
                return Err(error("initialize canonical RGBA raster"));
            }
        }
        Ok(())
    })();
    match result {
        Ok(()) => Ok(ds),
        Err(e) => {
            let mut f = failure(e);
            secondary(&mut f, ds.close());
            Err(f)
        }
    }
}
struct TranslateOptions(*mut gdal_sys::GDALTranslateOptions);
impl Drop for TranslateOptions {
    fn drop(&mut self) {
        unsafe { gdal_sys::GDALTranslateOptionsFree(self.0) }
    }
}
struct WarpOptions(*mut gdal_sys::GDALWarpAppOptions);
impl Drop for WarpOptions {
    fn drop(&mut self) {
        unsafe { gdal_sys::GDALWarpAppOptionsFree(self.0) }
    }
}
fn cog(
    source: &Dataset,
    p: &Path,
    workers: u8,
    delta: ColorRoleDelta,
    attempt: &Attempt,
) -> Result<(), JobFailure> {
    #[cfg(test)]
    super::enter_phase(0);
    let result = (|| {
        let mut args = exact(17 + usize::from(delta == ColorRoleDelta::SingleUndefinedGray) * 2)?;
        args.extend([
            "-strict".into(),
            "-of".into(),
            "COG".into(),
            "-co".into(),
            "COMPRESS=DEFLATE".into(),
            "-co".into(),
            "PREDICTOR=NO".into(),
            "-co".into(),
            "BLOCKSIZE=512".into(),
            "-co".into(),
            "BIGTIFF=IF_SAFER".into(),
            "-co".into(),
            "OVERVIEWS=IGNORE_EXISTING".into(),
            "-co".into(),
            "OVERVIEW_RESAMPLING=NEAREST".into(),
            "-co".into(),
            format!("NUM_THREADS={workers}"),
        ]);
        if delta == ColorRoleDelta::SingleUndefinedGray {
            args.extend(["-colorinterp".into(), "gray".into()]);
        }
        let owned_args = Args::new(&args)?;
        drop(args);
        let args = owned_args;
        let options = unsafe { gdal_sys::GDALTranslateOptionsNew(args.ptr(), null_mut()) };
        if options.is_null() {
            return Err(error("prepare COG options"));
        }
        let options = TranslateOptions(options);
        let progress = Progress::new(attempt, "source_cog");
        unsafe {
            gdal_sys::GDALTranslateOptionsSetProgress(
                options.0,
                Some(callback),
                std::ptr::from_ref(&progress).cast_mut().cast(),
            )
        };
        let p = path(p)?;
        let mut usage = 0;
        let raw =
            unsafe { gdal_sys::GDALTranslate(p.as_ptr(), source.raw(), options.0, &mut usage) };
        let result = progress.finish(!raw.is_null() && usage == 0);
        let close = if raw.is_null() {
            Ok(())
        } else {
            Dataset::owned(raw)?.close()
        };
        drop(options);
        match (result, close) {
            (Err(e), Err(c)) => {
                let mut f = failure(e);
                f.secondary.push(c);
                Ok(Err(f))
            }
            (Err(e), _) => Err(e),
            (Ok(()), Err(e)) => Err(e),
            (Ok(()), Ok(())) => Ok(Ok(())),
        }
    })();
    match result {
        Ok(r) => r,
        Err(e) => Err(failure(e)),
    }
}
fn compare_closed(ds: &Dataset, f: &SourceFacts, delta: ColorRoleDelta) -> Result<(), JobError> {
    let mut gt = [0.; 6];
    if unsafe { gdal_sys::GDALGetGeoTransform(ds.raw(), gt.as_mut_ptr()) } != 0
        || gt.map(f64::to_bits) != f.affine.map(f64::to_bits)
        || crs(ds, true)? != f.crs
        || unsafe { gdal_sys::GDALGetRasterXSize(ds.raw()) } != f.width as i32
        || unsafe { gdal_sys::GDALGetRasterYSize(ds.raw()) } != f.height as i32
        || unsafe { gdal_sys::GDALGetRasterCount(ds.raw()) } != f.bands.len() as i32
    {
        return Err(invalid("closed COG changed source georeference/shape"));
    }
    let area = unsafe {
        bytes(
            gdal_sys::GDALGetMetadataItem(ds.raw(), c"AREA_OR_POINT".as_ptr(), null()),
            16,
        )
    }?;
    if (area == b"Point") != (f.area_point == AreaPoint::Point)
        || !matches!(area, b"" | b"Area" | b"Point")
    {
        return Err(invalid("closed COG changed Area/Point"));
    }
    for (i, a) in f.bands.iter().enumerate() {
        let band = ds.band(i + 1);
        if SampleType::from_native(unsafe { gdal_sys::GDALGetRasterDataType(band) })? != a.datatype
        {
            return Err(invalid("closed COG changed sample type"));
        }
        block(band, a.datatype)?;
        checked_mask(band, f.width as i32, f.height as i32)?;
        let mut present = 0;
        let nd = ScalarFact::new(
            unsafe { gdal_sys::GDALGetRasterNoDataValue(band, &mut present) },
            present != 0,
        );
        let sc = ScalarFact::new(
            unsafe { gdal_sys::GDALGetRasterScale(band, &mut present) },
            present != 0,
        );
        let of = ScalarFact::new(
            unsafe { gdal_sys::GDALGetRasterOffset(band, &mut present) },
            present != 0,
        );
        let color = if i == 0 && delta == ColorRoleDelta::SingleUndefinedGray {
            1
        } else {
            a.color_interp
        };
        if !a.nodata.same(nd)
            || !a.scale.same(sc)
            || !a.offset.same(of)
            || unsafe { gdal_sys::GDALGetRasterColorInterpretation(band) } != u32::from(color)
            || unsafe { bytes(gdal_sys::GDALGetRasterUnitType(band), 1_048_576) }? != f.text(a.unit)
        {
            return Err(invalid("closed COG changed critical band facts"));
        }
        let mask = unsafe { gdal_sys::GDALGetMaskFlags(band) } as u8;
        if mask != a.mask_class {
            return Err(invalid("closed COG changed mask association"));
        }
        let table = unsafe { gdal_sys::GDALGetRasterColorTable(band) };
        let count = if table.is_null() {
            0
        } else {
            unsafe { gdal_sys::GDALGetColorEntryCount(table) }
        };
        let palette = f.palette(a.palette);
        if count < palette.len() as i32 || count > 256 {
            return Err(invalid("closed COG changed palette count"));
        }
        for j in 0..count {
            let p = unsafe { gdal_sys::GDALGetColorEntry(table, j) };
            if p.is_null() {
                return Err(invalid("closed COG palette entry missing"));
            }
            let e = unsafe { &*p };
            let expected = palette.get(j as usize).copied().unwrap_or([0, 0, 0, 255]);
            if [e.c1, e.c2, e.c3, e.c4] != expected.map(i16::from) {
                return Err(invalid("closed COG changed normalized palette"));
            }
        }
    }
    Ok(())
}
fn warp(
    input: &Path,
    output: &Path,
    grid: &GridPlan,
    workers: u8,
    attempt: &Attempt,
) -> Result<(), JobFailure> {
    let source = Dataset::open(input).map_err(failure)?;
    let target = grid.target();
    let dest = match create_rgba(
        output,
        target.width,
        target.height,
        target.geotransform,
        None,
    ) {
        Ok(d) => d,
        Err(mut f) => {
            secondary(&mut f, source.close());
            return Err(f);
        }
    };
    let run = (|| {
        let args = Args::new(&[
            "-r".into(),
            "near".into(),
            "-ovr".into(),
            "NONE".into(),
            "-et".into(),
            "0".into(),
            "-novshift".into(),
            "-wm".into(),
            "64".into(),
            "-srcalpha".into(),
            "-dstalpha".into(),
            "-wo".into(),
            format!("NUM_THREADS={workers}"),
        ])?;
        let opt = unsafe { gdal_sys::GDALWarpAppOptionsNew(args.ptr(), null_mut()) };
        if opt.is_null() {
            return Err(error("prepare aligned warp options"));
        }
        let options = WarpOptions(opt);
        let progress = Progress::new(attempt, "aligned_warp");
        unsafe {
            gdal_sys::GDALWarpAppOptionsSetProgress(
                options.0,
                Some(callback),
                std::ptr::from_ref(&progress).cast_mut().cast(),
            )
        };
        let mut usage = 0;
        let mut source_raw = source.raw();
        let raw = unsafe {
            gdal_sys::GDALWarp(
                null(),
                dest.raw(),
                1,
                &mut source_raw,
                options.0,
                &mut usage,
            )
        };
        let result = progress.finish(!raw.is_null() && raw == dest.raw() && usage == 0);
        if !raw.is_null() && raw != dest.raw() {
            unsafe { gdal_sys::GDALClose(raw) };
        }
        drop(options);
        result
    })();
    let dc = dest.close();
    let sc = source.close();
    let mut failure_record = match run {
        Err(e) => Some(failure(e)),
        Ok(()) => None,
    };
    for r in [dc, sc] {
        if let Err(e) = r {
            if let Some(f) = &mut failure_record {
                f.secondary.push(e)
            } else {
                failure_record = Some(failure(e));
            }
        }
    }
    if let Some(f) = failure_record {
        return Err(f);
    }
    let closed = Dataset::open(output).map_err(failure)?;
    let verify = (|| {
        let mut gt = [0.; 6];
        if unsafe { gdal_sys::GDALGetGeoTransform(closed.raw(), gt.as_mut_ptr()) } != 0
            || gt.map(f64::to_bits) != target.geotransform.map(f64::to_bits)
            || unsafe { gdal_sys::GDALGetRasterXSize(closed.raw()) } != target.width as i32
            || unsafe { gdal_sys::GDALGetRasterYSize(closed.raw()) } != target.height as i32
            || crs(&closed, true)? != StaticCrs::WebMercator3857
        {
            return Err(invalid("closed aligned target differs from exact GridPlan"));
        }
        for i in 1..=4 {
            if unsafe { gdal_sys::GDALGetOverviewCount(closed.band(i)) } != 0 {
                return Err(invalid("aligned target unexpectedly has overviews"));
            }
        }
        Ok(())
    })();
    let c = closed.close();
    match verify {
        Err(e) => {
            let mut f = failure(e);
            secondary(&mut f, c);
            Err(f)
        }
        Ok(()) => c.map_err(failure),
    }
}
/// Logical workspace + requested Rust owners, not opaque GDAL allocations/RSS.
const ARG_SLOTS: usize = 32;
const SMALL_ARG_BYTES: usize = 64;
fn pyramid(mut w: u32, mut h: u32) -> Result<u64, JobError> {
    let mut q = u64::from(w) * u64::from(h);
    while w > 1 || h > 1 {
        w = w.div_ceil(2);
        h = h.div_ceil(2);
        q = q
            .checked_add(u64::from(w) * u64::from(h))
            .ok_or_else(|| limit("raster pyramid work overflow"))?;
    }
    Ok(q)
}
pub(super) fn planned_work(
    source: &SourcePlan,
    grid: &GridPlan,
    stage: &Path,
) -> Result<u64, JobError> {
    let p = u64::from(source.facts.width) * u64::from(source.facts.height);
    let q = pyramid(source.facts.width, source.facts.height)?;
    let t = pyramid(grid.target().width, grid.target().height)?;
    let b = source.facts.bands.len() as u64 * source.facts.bands[0].datatype.bytes() as u64;
    let c = source.facts.bands.len() as u64;
    let m = display::window_pixels(&source.facts);
    let coherence = 2 * display::words(m, source.facts.bands[0].datatype.bytes()) * 8 + 2 * m;
    let render = display::words(m, source.roles.width(&source.facts)) * 8 + 6 * m;
    let path_bound = stage
        .as_os_str()
        .as_encoded_bytes()
        .len()
        .checked_add(33)
        .ok_or_else(|| limit("raster path estimate overflow"))?;
    // Three retained paths; one current output path; at most two temporary argv
    // String paths + two CString paths; one scoped-setting CString. Other phases
    // have fewer path owners. Numeric/static argv payloads fit64 bytes each.
    let paths = path_bound
        .checked_mul(9)
        .ok_or_else(|| limit("raster path owners overflow"))?;
    let argv = ARG_SLOTS
        * (std::mem::size_of::<String>() + std::mem::size_of::<CString>() + SMALL_ARG_BYTES * 2)
        + (ARG_SLOTS + 1) * std::mem::size_of::<*mut c_char>();
    let fixed = std::mem::size_of::<GridPlan>()
        + std::mem::size_of::<Args>()
        + std::mem::size_of::<Progress<'_>>()
        + std::mem::size_of::<Dataset>() * 4
        + std::mem::size_of::<TranslateOptions>()
        + std::mem::size_of::<WarpOptions>()
        + std::mem::size_of::<display::Window>()
        + 4 * std::mem::size_of::<Vec<u8>>()
        + 4 * std::mem::size_of::<std::path::PathBuf>()
        + std::mem::size_of::<Xml>()
        + tile::owner_bytes()
        + 64 * std::mem::size_of::<*mut XmlNode>();
    let owners = source
        .owned_bytes()?
        .checked_add(
            (coherence
                .max(render)
                .checked_add(paths)
                .and_then(|n| n.checked_add(argv))
                .and_then(|n| n.checked_add(fixed))
                .ok_or_else(|| limit("raster requested owner overflow"))?) as u64,
        )
        .ok_or_else(|| limit("raster owner estimate overflow"))?;
    let terms = [
        p.checked_mul(4),
        t.checked_mul(4),
        q.checked_mul(b + c),
        grid.total_tiles().checked_mul(262144),
        Some(owners),
        Some(source.companion_bytes()?),
    ];
    terms.into_iter().try_fold(0u64, |n, t| {
        n.checked_add(t.ok_or_else(|| limit("raster work multiplication overflow"))?)
            .ok_or_else(|| limit("raster work addition overflow"))
    })
}
pub(super) fn output_path(stage: &Path, name: &str) -> Result<std::path::PathBuf, JobError> {
    let count = stage
        .as_os_str()
        .as_encoded_bytes()
        .len()
        .checked_add(1 + name.len())
        .ok_or_else(|| limit("output path overflow"))?;
    let mut path = std::path::PathBuf::new();
    path.try_reserve_exact(count)
        .map_err(|_| limit("output path allocation failed"))?;
    if path.capacity() != count {
        return Err(limit("output path requested capacity mismatch"));
    }
    path.push(stage);
    path.push(name);
    Ok(path)
}

pub(super) fn produce(
    mut plan: SourcePlan,
    grid: &GridPlan,
    stage: &Path,
    attempt: &Attempt,
) -> Result<NativeProduced, JobFailure> {
    let _quiet = QuietErrors::new();
    let source_color = plan.facts.bands[0].color_interp;
    let delta = gray_delta(&plan.facts);
    let paths = (|| {
        Ok::<_, JobError>((
            output_path(stage, "source.cog.tif")?,
            output_path(stage, "display-source.tif")?,
            output_path(stage, "display-finest.tif")?,
        ))
    })();
    let (cog_path, rgba_path, finest_path) = match paths {
        Ok(p) => p,
        Err(e) => return Err(plan.finish_failure(e)),
    };
    let mut closed: Option<Dataset> = None;
    let result = (|| {
        attempt.check().map_err(failure)?;
        plan.config
            .set(
                c"CPL_TMPDIR",
                stage
                    .to_str()
                    .ok_or_else(|| failure(unsupported("native temporary stage requires UTF8")))?,
            )
            .map_err(failure)?;
        plan.config.recheck_debug().map_err(failure)?;
        cog(
            plan.source.as_ref().expect("source plan owns dataset"),
            &cog_path,
            plan.workers,
            delta,
            attempt,
        )?;
        super::check_closed_workspace(stage, grid.total_tiles(), plan.limits.max_working_bytes)
            .map_err(failure)?;
        let cog_size = std::fs::metadata(&cog_path)
            .map_err(|e| failure(JobError::io("inspect closed source COG", &cog_path, e)))?
            .len();
        if cog_size > plan.limits.max_output_bytes {
            return Err(failure(limit("closed COG exceeds output byte limit")));
        }
        plan.config
            .set(c"GDAL_PAM_ENABLED", "NO")
            .map_err(failure)?;
        closed = Some(Dataset::open(&cog_path).map_err(failure)?);
        compare_closed(closed.as_ref().unwrap(), &plan.facts, delta).map_err(failure)?;
        check_standalone(closed.as_ref().unwrap(), &cog_path).map_err(failure)?;
        display::coherence(
            plan.source.as_ref().unwrap(),
            closed.as_ref().unwrap(),
            &plan.facts,
            attempt,
        )
        .map_err(failure)?;
        plan.source.take().unwrap().close().map_err(failure)?;
        plan.recheck().map_err(failure)?;
        plan.leaves = Vec::new();
        plan.facts.text = Vec::new();
        display::render(
            closed.as_ref().unwrap(),
            &plan.facts,
            plan.roles,
            &rgba_path,
            attempt,
        )?;
        closed.take().unwrap().close().map_err(failure)?;
        super::check_closed_workspace(stage, grid.total_tiles(), plan.limits.max_working_bytes)
            .map_err(failure)?;
        plan.facts.bands = Vec::new();
        plan.facts.palettes = Vec::new();
        warp(&rgba_path, &finest_path, grid, plan.workers, attempt)?;
        super::check_closed_workspace(stage, grid.total_tiles(), plan.limits.max_working_bytes)
            .map_err(failure)?;
        std::fs::remove_file(&rgba_path)
            .map_err(|e| failure(JobError::io("remove display source", &rgba_path, e)))?;
        tile::run(
            &finest_path,
            &output_path(stage, "tiles").map_err(failure)?,
            grid,
            plan.workers,
            attempt,
            stage,
            plan.limits.max_working_bytes,
        )?;
        std::fs::remove_file(&finest_path)
            .map_err(|e| failure(JobError::io("remove finest target", &finest_path, e)))?;
        let cog_bytes = std::fs::metadata(&cog_path)
            .map_err(|e| failure(JobError::io("inspect closed COG", &cog_path, e)))?
            .len();
        Ok(NativeProduced {
            source_cog_bytes: cog_bytes,
            tiles: grid.total_tiles(),
            finest_rgba_bytes: grid.finest_rgba_bytes(),
            source_color_interp: source_color,
            target_color_interp: if delta == ColorRoleDelta::SingleUndefinedGray {
                1
            } else {
                source_color
            },
            role_delta: delta,
            native_version: std::mem::take(&mut plan.version),
        })
    })();
    match result {
        Ok(produced) => {
            plan.config.restore().map_err(failure)?;
            Ok(produced)
        }
        Err(mut f) => {
            if let Some(ds) = closed.take() {
                secondary(&mut f, ds.close());
            }
            if let Some(ds) = plan.source.take() {
                secondary(&mut f, ds.close());
            }
            secondary(&mut f, plan.config.restore());
            Err(f)
        }
    }
}
pub(super) fn capabilities() -> Result<RasterCapabilities, JobError> {
    let _quiet = QuietErrors::new();
    unsafe { gdal_sys::GDALAllRegister() };
    let has = |n: &CStr| unsafe { !gdal_sys::GDALGetDriverByName(n.as_ptr()).is_null() };
    Ok(RasterCapabilities {
        native_version: version()?,
        gtiff: has(c"GTiff"),
        png: has(c"PNG"),
        jpeg: has(c"JPEG"),
        aaigrid: has(c"AAIGrid"),
        cog: has(c"COG"),
        // Doctor reports capability readiness; conversions retain the complete
        // causal failure through require_outputs instead of this projection.
        tile: tile::available().map_err(|failure| failure.error)?,
    })
}

fn xml_text(node: *mut XmlNode) -> Result<&'static [u8], JobError> {
    let mut p = unsafe { (*node).child };
    while !p.is_null() {
        let n = unsafe { &*p };
        if n.kind == 1 {
            return unsafe { bytes(n.value, 1_048_576) };
        }
        p = n.next;
    }
    Ok(&[])
}
fn number(text: &[u8]) -> Result<f64, JobError> {
    std::str::from_utf8(text)
        .map_err(|_| invalid("critical companion scalar UTF8"))?
        .trim()
        .parse()
        .map_err(|_| invalid("critical companion scalar"))
}
fn attr(node: *mut XmlNode, key: &[u8]) -> Result<Option<&'static [u8]>, JobError> {
    let mut p = unsafe { (*node).child };
    while !p.is_null() {
        let n = unsafe { &*p };
        if n.kind == 2 && unsafe { bytes(n.value, 64) }? == key {
            return Ok(Some(xml_text(p)?));
        }
        p = n.next;
    }
    Ok(None)
}
fn affine_text(text: &[u8]) -> Result<[f64; 6], JobError> {
    let text = std::str::from_utf8(text).map_err(|_| invalid("companion affine UTF8"))?;
    let mut out = [0.0_f64; 6];
    let mut n = 0;
    for s in text
        .split(|c: char| c.is_ascii_whitespace() || c == ',')
        .filter(|s| !s.is_empty())
    {
        if n == 6 {
            return Err(invalid("companion affine coefficient count"));
        }
        out[n] = s
            .parse()
            .map_err(|_| invalid("companion affine coefficient"))?;
        if !out[n].is_finite() {
            return Err(invalid("nonfinite companion affine"));
        }
        n += 1;
    }
    if n != 6 {
        return Err(invalid("companion affine coefficient count"));
    }
    Ok(out)
}
// Both pre-open screening and post-open authority comparison consume this
// restricted parser; companion text cannot delegate to another file or URL.
fn parse_srs(text: &[u8]) -> Result<gdal_sys::OGRSpatialReferenceH, JobError> {
    let text = cstring(text)?;
    let s = unsafe { gdal_sys::OSRNewSpatialReference(null()) };
    if s.is_null() {
        return Err(error("allocate companion SRS"));
    }
    let options = [
        c"ALLOW_FILE_ACCESS=NO".as_ptr(),
        c"ALLOW_NETWORK_ACCESS=NO".as_ptr(),
        null(),
    ];
    // CSLConstList is read-only in the C++ implementation; its C-compatible
    // typedef generates char** in bindings despite those const semantics.
    // SAFETY: owned input, finite terminated read-only options, fresh owned SRS.
    if unsafe {
        gdal_sys::OSRSetFromUserInputEx(s, text.as_ptr(), options.as_ptr().cast_mut().cast())
    } != 0
    {
        unsafe { gdal_sys::OSRDestroySpatialReference(s) };
        return Err(unsupported(
            "companion SRS must be an inline supported definition",
        ));
    }
    Ok(s)
}
pub(super) fn screen_srs(text: &[u8]) -> Result<(), JobError> {
    let _quiet = QuietErrors::new();
    let s = parse_srs(text)?;
    unsafe { gdal_sys::OSRDestroySpatialReference(s) };
    Ok(())
}
fn verify_srs(text: &[u8], f: &SourceFacts) -> Result<(), JobError> {
    let _quiet = QuietErrors::new();
    let s = parse_srs(text)?;
    let result = (|| {
        let expected = unsafe { gdal_sys::OSRNewSpatialReference(null()) };
        if expected.is_null() {
            return Err(error("allocate expected SRS"));
        }
        let epsg = match f.crs {
            StaticCrs::Geographic4326 => 4326,
            StaticCrs::WebMercator3857 => 3857,
        };
        let same = unsafe {
            gdal_sys::OSRImportFromEPSG(expected, epsg) == 0
                && gdal_sys::OSRIsSameEx(
                    s,
                    expected,
                    [c"IGNORE_DATA_AXIS_TO_SRS_AXIS_MAPPING=YES".as_ptr(), null()].as_ptr(),
                ) != 0
        };
        unsafe { gdal_sys::OSRDestroySpatialReference(expected) };
        if same {
            Ok(())
        } else {
            Err(invalid("companion CRS conflicts with resolved authority"))
        }
    })();
    unsafe { gdal_sys::OSRDestroySpatialReference(s) };
    result
}
/// Declared companion critical facts must agree even when INTERNAL precedence
/// would hide a conflicting declaration from the effective native dataset.
pub(super) fn verify_companions(leaves: &[SourceLeaf], f: &SourceFacts) -> Result<(), JobError> {
    for leaf in leaves.iter().skip(1) {
        if leaf.kind == LeafKind::Mask {
            continue;
        }
        let text = leaf.read_text()?;
        if leaf.kind == LeafKind::Prj {
            if f.driver != Driver::AAIGrid {
                return Err(unsupported("PRJ companion requires actual AAIGrid driver"));
            }
            verify_srs(&text, f)?;
        } else if leaf.kind == LeafKind::Pam {
            let tree = xml(&text)?;
            if unsafe { bytes((*tree.0).value, 64) }? != b"PAMDataset" {
                return Err(unsupported("PAM root outside finite profile"));
            }
            let mut p = unsafe { (*tree.0).child };
            while !p.is_null() {
                let n = unsafe { &*p };
                if n.kind == 0 {
                    let tag = unsafe { bytes(n.value, 64) }?;
                    match tag {
                        b"SRS" => verify_srs(xml_text(p)?, f)?,
                        b"GeoTransform" => {
                            if affine_text(xml_text(p)?)?.map(f64::to_bits)
                                != f.affine.map(f64::to_bits)
                            {
                                return Err(invalid("PAM affine conflicts with source authority"));
                            }
                        }
                        b"PAMRasterBand" => {
                            let index = std::str::from_utf8(
                                attr(p, b"band")?
                                    .ok_or_else(|| invalid("PAM band index absent"))?,
                            )
                            .map_err(|_| invalid("PAM band index"))?
                            .parse::<usize>()
                            .map_err(|_| invalid("PAM band index"))?;
                            let band = f
                                .bands
                                .get(
                                    index
                                        .checked_sub(1)
                                        .ok_or_else(|| invalid("PAM band index"))?,
                                )
                                .ok_or_else(|| invalid("PAM band index"))?;
                            let mut q = n.child;
                            while !q.is_null() {
                                let child = unsafe { &*q };
                                if child.kind == 0 {
                                    let field = unsafe { bytes(child.value, 64) }?;
                                    match field {
                                        b"NoDataValue" | b"Scale" | b"Offset" => {
                                            let scalar = match field {
                                                b"NoDataValue" => band.nodata,
                                                b"Scale" => band.scale,
                                                _ => band.offset,
                                            };
                                            let declared =
                                                ScalarFact::new(number(xml_text(q)?)?, true);
                                            if !scalar.same(declared) {
                                                return Err(invalid(
                                                    "PAM scalar conflicts with source authority",
                                                ));
                                            }
                                        }
                                        b"UnitType" => {
                                            if xml_text(q)? != f.text(band.unit) {
                                                return Err(invalid(
                                                    "PAM unit conflicts with source authority",
                                                ));
                                            }
                                        }
                                        b"ColorInterp" => {
                                            let role = match xml_text(q)? {
                                                b"Undefined" => 0,
                                                b"Gray" => 1,
                                                b"Palette" => 2,
                                                b"Red" => 3,
                                                b"Green" => 4,
                                                b"Blue" => 5,
                                                b"Alpha" => 6,
                                                _ => return Err(unsupported("PAM color role")),
                                            };
                                            if role != band.color_interp {
                                                return Err(invalid(
                                                    "PAM color role conflicts with source authority",
                                                ));
                                            }
                                        }
                                        b"ColorTable" => {
                                            let palette = f.palette(band.palette);
                                            let mut e = child.child;
                                            let mut i = 0;
                                            while !e.is_null() {
                                                let en = unsafe { &*e };
                                                if en.kind == 0 {
                                                    if unsafe { bytes(en.value, 64) }? != b"Entry" {
                                                        return Err(unsupported(
                                                            "PAM palette child",
                                                        ));
                                                    }
                                                    let mut rgba = [0u8, 0, 0, 255];
                                                    for (k, key) in [b"c1", b"c2", b"c3", b"c4"]
                                                        .iter()
                                                        .enumerate()
                                                    {
                                                        if let Some(v) = attr(e, *key)? {
                                                            rgba[k] = std::str::from_utf8(v)
                                                                .map_err(|_| {
                                                                    invalid("PAM palette scalar")
                                                                })?
                                                                .parse()
                                                                .map_err(|_| {
                                                                    invalid("PAM palette scalar")
                                                                })?;
                                                        }
                                                    }
                                                    if palette.get(i) != Some(&rgba) {
                                                        return Err(invalid(
                                                            "PAM palette conflicts with source authority",
                                                        ));
                                                    }
                                                    i += 1;
                                                }
                                                e = en.next;
                                            }
                                            if i != palette.len() {
                                                return Err(invalid(
                                                    "PAM palette count conflicts with source authority",
                                                ));
                                            }
                                        }
                                        b"Metadata" => {}
                                        _ => {
                                            return Err(unsupported(
                                                "PAM band authority outside profile",
                                            ));
                                        }
                                    }
                                }
                                q = child.next;
                            }
                        }
                        b"Metadata" => {}
                        _ => return Err(unsupported("PAM dataset authority outside profile")),
                    }
                }
                p = n.next;
            }
        } else {
            let world = affine_text(&text)?;
            let actual = [
                world[4] - 0.5 * world[0] - 0.5 * world[2],
                world[0],
                world[2],
                world[5] - 0.5 * world[1] - 0.5 * world[3],
                world[1],
                world[3],
            ];
            if actual.map(f64::to_bits) != f.affine.map(f64::to_bits) {
                return Err(invalid("worldfile conflicts with resolved source affine"));
            }
        }
    }
    Ok(())
}

fn cstring(bytes: &[u8]) -> Result<CString, JobError> {
    if bytes.contains(&0) {
        return Err(invalid("NUL in native raster bytes"));
    }
    let mut owned = exact(
        bytes
            .len()
            .checked_add(1)
            .ok_or_else(|| limit("native string overflow"))?,
    )?;
    owned.extend_from_slice(bytes);
    owned.push(0);
    CString::from_vec_with_nul(owned).map_err(|_| invalid("native string termination"))
}

fn check_standalone(ds: &Dataset, expected: &Path) -> Result<(), JobError> {
    let list = unsafe { gdal_sys::GDALGetFileList(ds.raw()) };
    if list.is_null() {
        return Err(invalid("standalone dataset inventory missing"));
    }
    let result = (|| {
        let first = unsafe { *list };
        if first.is_null() || !unsafe { *list.add(1) }.is_null() {
            return Err(invalid("generated raster requires external sidecar"));
        }
        let bytes = unsafe { bytes(first, 65536) }?;
        #[cfg(unix)]
        let actual = {
            use std::os::unix::ffi::OsStrExt;
            Path::new(std::ffi::OsStr::from_bytes(bytes))
        };
        #[cfg(not(unix))]
        let actual = Path::new(
            std::str::from_utf8(bytes)
                .map_err(|_| unsupported("standalone native filename UTF8"))?,
        );
        if actual != expected {
            return Err(invalid("standalone generated raster inventory differs"));
        }
        Ok(())
    })();
    unsafe { gdal_sys::CSLDestroy(list) };
    result
}

pub(super) fn require_outputs() -> Result<(), JobFailure> {
    for name in [c"GTiff", c"COG", c"PNG"] {
        if unsafe { gdal_sys::GDALGetDriverByName(name.as_ptr()) }.is_null() {
            return Err(failure(unsupported(
                "requested raster output driver unavailable",
            )));
        }
    }
    if !tile::available()? {
        return Err(failure(unsupported(
            "requested raster tile algorithm unavailable",
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn companion_srs_never_delegates_and_descriptive_urls_stay_opaque() {
        assert!(super::screen_srs(b"EPSG:4326").is_ok());
        let root = tempfile::tempdir().unwrap();
        let referenced = root.path().join("otherwise-valid-srs.txt");
        std::fs::write(&referenced, b"EPSG:4326").unwrap();
        assert_eq!(
            super::screen_srs(referenced.as_os_str().as_encoded_bytes())
                .unwrap_err()
                .kind(),
            crate::JobErrorKind::Unsupported
        );
        assert_eq!(
            super::screen_srs(b"https://example.invalid/srs.txt")
                .unwrap_err()
                .kind(),
            crate::JobErrorKind::Unsupported
        );
        assert!(super::screen_pam_text(br#"<PAMDataset><Metadata><MDI key="source">https://example.invalid/descriptive</MDI></Metadata><SRS>EPSG:4326</SRS></PAMDataset>"#).is_ok());
        assert_eq!(
            super::screen_pam_text(
                b"<PAMDataset><SRS>https://example.invalid/srs.txt</SRS></PAMDataset>"
            )
            .unwrap_err()
            .kind(),
            crate::JobErrorKind::Unsupported
        );
    }

    use super::*;
    #[test]
    fn logical_pyramids_and_actual_owner_types() {
        assert_eq!(pyramid(1, 1).unwrap(), 1);
        assert_eq!(pyramid(2, 3).unwrap(), 9);
        assert_eq!(pyramid(513, 513).unwrap(), 351582);
        assert!(std::mem::size_of::<SourcePlan>() > std::mem::size_of::<SourceFacts>());
        assert_eq!(std::mem::size_of::<display::Window>(), 16);
    }
    #[test]
    fn finite_diagnostic_actual_capacity() {
        let text = diagnostic_text(&[b'a'; 512]);
        assert_eq!(text.len(), 512);
        let std::borrow::Cow::Owned(text) = text else {
            panic!("expected exact owned diagnostic")
        };
        assert_eq!(text.capacity(), 512);
        let text = diagnostic_text(&[0xff; 512]);
        assert_eq!(text.len(), 510);
        assert!(text.chars().all(|c| c == '\u{fffd}'));
        let std::borrow::Cow::Owned(text) = text else {
            panic!("expected exact owned diagnostic")
        };
        assert_eq!(text.capacity(), 512);
        assert_eq!(diagnostic_text(b"a\xffb"), "a\u{fffd}b");
    }
    #[test]
    fn companion_affine_and_native_strings() {
        assert_eq!(
            affine_text(b"1, 2, 3, 4, 5, 6").unwrap(),
            [1., 2., 3., 4., 5., 6.]
        );
        assert!(affine_text(b"1 2 3 4 5").is_err());
        assert!(cstring(b"a\0b").is_err());
        assert_eq!(cstring(b"abc").unwrap().as_bytes_with_nul(), b"abc\0");
    }
}
