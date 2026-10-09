//! A finite RGB raster consumer of the D1 directory publication lifecycle.
#[cfg(feature = "native-geospatial")]
use crate::{
    runtime::{directory::DirectoryTarget, Attempt},
    RunEvent,
};
use crate::{CleanupDiagnostic, JobError, JobErrorKind, JobFailure, OutputPolicy, RunControl};
use serde::Serialize;
#[cfg(feature = "native-geospatial")]
use serde_json::json;
#[cfg(feature = "native-geospatial")]
use std::io::Write;
use std::path::PathBuf;
#[cfg(feature = "native-geospatial")]
use std::{
    fs::{self, File, Metadata},
    io::{Read, Seek, SeekFrom},
    path::Path,
};
#[cfg(feature = "native-geospatial")]
mod native;
#[cfg(feature = "native-geospatial")]
const MAX_SOURCE_BYTES: u64 = 32 * 1024 * 1024;
#[derive(Clone, Debug)]
pub struct RasterDirectoryRequest {
    input: PathBuf,
    output: PathBuf,
    z: u8,
    x: u32,
    y: u32,
    policy: OutputPolicy,
}
impl RasterDirectoryRequest {
    pub fn web_mercator_rgb(
        input: impl Into<PathBuf>,
        output: impl Into<PathBuf>,
        z: u8,
        x: u32,
        y: u32,
    ) -> Self {
        Self {
            input: input.into(),
            output: output.into(),
            z,
            x,
            y,
            policy: OutputPolicy::CreateNew,
        }
    }
    pub fn with_policy(mut self, policy: OutputPolicy) -> Self {
        self.policy = policy;
        self
    }
}
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RasterDirectoryReport {
    pub schema_version: u64,
    pub profile: &'static str,
    pub source_bytes: u64,
    pub width: u32,
    pub height: u32,
    pub z: u8,
    pub x: u32,
    pub y: u32,
}
#[derive(Debug)]
pub struct RasterDirectoryResult {
    pub output: PathBuf,
    pub report: RasterDirectoryReport,
    pub cleanup_diagnostics: Vec<CleanupDiagnostic>,
}
fn error(kind: JobErrorKind, message: impl Into<String>) -> JobError {
    JobError::new(kind, message)
}
#[cfg(feature = "native-geospatial")]
fn inspect(path: &Path) -> Result<Metadata, JobError> {
    let m =
        fs::symlink_metadata(path).map_err(|e| JobError::io("inspect raster source", path, e))?;
    if !m.is_file() {
        return Err(error(
            JobErrorKind::InvalidInput,
            "raster source must be a regular file without a symlink leaf",
        ));
    }
    if m.len() > MAX_SOURCE_BYTES {
        return Err(error(
            JobErrorKind::Unsupported,
            "D1 raster source exceeds 32 MiB",
        ));
    }
    Ok(m)
}
#[cfg(feature = "native-geospatial")]
fn same_source(a: &Metadata, b: &Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if a.dev() != b.dev()
            || a.ino() != b.ino()
            || a.ctime() != b.ctime()
            || a.ctime_nsec() != b.ctime_nsec()
        {
            return false;
        }
    }
    a.len() == b.len() && a.modified().ok() == b.modified().ok()
}
// Narrow TIFF tag admission, not a pixel or metadata decoder. GDAL does not
// expose standalone transfer functions in its COLOR_PROFILE domain.
#[cfg(feature = "native-geospatial")]
fn admit_tiff_tags(path: &Path, source_bytes: u64) -> Result<(), JobError> {
    let mut file =
        File::open(path).map_err(|e| JobError::io("open raster tag directory", path, e))?;
    let mut read = |at: u64, bytes: &mut [u8]| -> Result<(), JobError> {
        if at
            .checked_add(bytes.len() as u64)
            .is_none_or(|end| end > source_bytes)
        {
            return Err(error(
                JobErrorKind::InvalidInput,
                "TIFF requested byte range outside source",
            ));
        }
        file.seek(SeekFrom::Start(at))
            .and_then(|_| file.read_exact(bytes))
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::UnexpectedEof {
                    error(JobErrorKind::InvalidInput, "truncated TIFF tag directory")
                } else {
                    JobError::io("read raster tag directory", path, e)
                }
            })
    };
    let mut header = [0u8; 8];
    read(0, &mut header)?;
    let little = &header[..2] == b"II";
    let uint = |bytes: &[u8]| -> u64 {
        if little {
            bytes.iter().rev().fold(0, |n, b| (n << 8) | u64::from(*b))
        } else {
            bytes.iter().fold(0, |n, b| (n << 8) | u64::from(*b))
        }
    };
    let (offset, count_size, entry_size) = if uint(&header[2..4]) == 43 {
        if uint(&header[4..6]) != 8 || uint(&header[6..8]) != 0 {
            return Err(error(JobErrorKind::InvalidInput, "invalid BigTIFF header"));
        }
        let mut offset = [0u8; 8];
        read(8, &mut offset)?;
        (uint(&offset), 8u64, 20u64)
    } else {
        (uint(&header[4..8]), 2u64, 12u64)
    };
    if offset < if count_size == 8 { 16 } else { 8 } {
        return Err(error(
            JobErrorKind::InvalidInput,
            "invalid first TIFF directory offset",
        ));
    }
    let mut count = [0u8; 8];
    read(offset, &mut count[..count_size as usize])?;
    let count = uint(&count[..count_size as usize]);
    if count > 4096 {
        return Err(error(
            JobErrorKind::Unsupported,
            "D1 raster first TIFF directory exceeds 4096 tags",
        ));
    }
    let length = count * entry_size;
    let start = offset
        .checked_add(count_size)
        .ok_or_else(|| error(JobErrorKind::InvalidInput, "TIFF directory offset overflow"))?;
    if start.checked_add(length).is_none_or(|end| {
        end.checked_add(if count_size == 8 { 8 } else { 4 })
            .is_none_or(|end| end > source_bytes)
    }) {
        return Err(error(
            JobErrorKind::InvalidInput,
            "TIFF tag directory outside source",
        ));
    }
    let mut entries = vec![0u8; length as usize];
    read(start, &mut entries)?;
    let mut next = [0u8; 8];
    read(
        start + length,
        &mut next[..if count_size == 8 { 8 } else { 4 }],
    )?;
    if uint(&next[..if count_size == 8 { 8 } else { 4 }]) != 0 {
        return Err(error(
            JobErrorKind::Unsupported,
            "D1 raster requires a single TIFF image without overviews",
        ));
    }
    for entry in entries.chunks_exact(entry_size as usize) {
        let tag = uint(&entry[..2]);
        if tag == 330 {
            return Err(error(
                JobErrorKind::Unsupported,
                "D1 raster forbids TIFF sub-IFDs",
            ));
        }
        if [259, 274, 284, 317].contains(&tag) {
            let value = inline_short(entry, count_size == 8, &uint)?;
            let admitted = match tag {
                259 => [1, 8].contains(&value),
                284 => [1, 2].contains(&value),
                274 | 317 => value == 1,
                _ => unreachable!("selected scalar TIFF tag"),
            };
            if !admitted {
                return Err(error(
                    JobErrorKind::Unsupported,
                    format!("D1 raster does not support TIFF tag {tag} value {value}"),
                ));
            }
        }
        if [301, 318, 319, 342, 34675].contains(&uint(&entry[..2])) {
            return Err(error(
                JobErrorKind::Unsupported,
                "D1 raster forbids TIFF color profiles and transfer functions",
            ));
        }
    }
    Ok(())
}

#[cfg(feature = "native-geospatial")]
fn inline_short(entry: &[u8], big: bool, uint: &impl Fn(&[u8]) -> u64) -> Result<u64, JobError> {
    let value_at = if big { 12 } else { 8 };
    if uint(&entry[2..4]) != 3 || uint(&entry[4..value_at]) != 1 {
        return Err(error(
            JobErrorKind::InvalidInput,
            "TIFF layout scalar must be a single SHORT",
        ));
    }
    Ok(uint(&entry[value_at..value_at + 2]))
}

#[cfg(feature = "native-geospatial")]
fn write_file(path: &Path, bytes: &[u8]) -> Result<(), JobError> {
    let mut file = File::options()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| JobError::io("create raster member", path, e))?;
    finish_member(&mut file, bytes, |file| file.sync_all())
        .map_err(|e| JobError::io("finish raster member", path, e))
}
#[cfg(feature = "native-geospatial")]
fn finish_member<W: Write>(
    writer: &mut W,
    bytes: &[u8],
    finalize: impl FnOnce(&mut W) -> std::io::Result<()>,
) -> std::io::Result<()> {
    writer.write_all(bytes)?;
    writer.flush()?;
    finalize(writer)
}

#[cfg(feature = "native-geospatial")]
fn produce(
    path: &Path,
    request: &RasterDirectoryRequest,
    pixels: &[u8],
    report: &RasterDirectoryReport,
    attempt: &Attempt,
) -> Result<(), JobError> {
    produce_with(path, request, pixels, report, attempt, write_file)
}
#[cfg(feature = "native-geospatial")]
fn produce_with(
    path: &Path,
    request: &RasterDirectoryRequest,
    pixels: &[u8],
    report: &RasterDirectoryReport,
    attempt: &Attempt,
    mut write: impl FnMut(&Path, &[u8]) -> Result<(), JobError>,
) -> Result<(), JobError> {
    use image::ImageEncoder;
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(pixels, 256, 256, image::ExtendedColorType::Rgb8)
        .map_err(|e| {
            error(
                JobErrorKind::InvalidInput,
                format!("encode raster PNG: {e}"),
            )
        })?;
    attempt.check()?;
    let tile = format!("tiles/{}/{}/{}.png", request.z, request.x, request.y);
    let destination = path.join(&tile);
    fs::create_dir_all(destination.parent().unwrap())
        .map_err(|e| JobError::io("create raster tile parent", &destination, e))?;
    write(&destination, &png)?;
    let n = f64::from(1u32 << request.z);
    let longitude = |x: u32| f64::from(x) / n * 360. - 180.;
    let latitude = |y: u32| {
        (std::f64::consts::PI * (1. - 2. * f64::from(y) / n))
            .sinh()
            .atan()
            .to_degrees()
    };
    let tilejson = json!({"tilejson":"3.0.0", "scheme":"xyz", "tiles":["tiles/{z}/{x}/{y}.png"], "minzoom":request.z, "maxzoom":request.z,
        "bounds":[longitude(request.x),latitude(request.y+1),longitude(request.x+1),latitude(request.y)]});
    write(
        &path.join("tilejson.json"),
        &serde_json::to_vec_pretty(&tilejson).unwrap(),
    )?;
    attempt.check()?;
    write(
        &path.join("report.json"),
        &serde_json::to_vec_pretty(report).unwrap(),
    )?;
    attempt.emit(&RunEvent::Progress {
        phase: "raster_directory",
        done: 1,
        total: Some(1),
    })?;
    attempt.close_events()
}
pub fn raster_to_directory(
    request: RasterDirectoryRequest,
    control: &RunControl,
) -> Result<RasterDirectoryResult, JobFailure> {
    let attempt = control.begin()?;
    let validate = || -> Result<(), JobError> {
        attempt.check()?;
        if request.input.as_os_str().is_empty()
            || request.output.as_os_str().is_empty()
            || request.z > 24
            || request.x >= 1u32 << request.z
            || request.y >= 1u32 << request.z
        {
            return Err(error(
                JobErrorKind::InvalidRequest,
                "D1 raster requires source/output paths and a valid XYZ address with z <= 24",
            ));
        }
        Ok(())
    };
    validate().map_err(|e| attempt.fail(e))?;
    #[cfg(not(feature = "native-geospatial"))]
    {
        Err(attempt.fail(error(
            JobErrorKind::Unsupported,
            "D1 raster requires native-geospatial GDAL capability",
        )))
    }
    #[cfg(feature = "native-geospatial")]
    {
        let prepare = || -> Result<_, JobError> {
            let before = inspect(&request.input)?;
            let source = fs::canonicalize(&request.input)
                .map_err(|e| JobError::io("resolve raster source", &request.input, e))?;
            let target = DirectoryTarget::prepare(&request.output, request.policy)?;
            for suffix in [".aux.xml", ".msk", ".ovr"] {
                let mut companion = source.as_os_str().to_os_string();
                companion.push(suffix);
                let companion = PathBuf::from(companion);
                match fs::symlink_metadata(&companion) {
                    Ok(_) => {
                        return Err(error(
                            JobErrorKind::Unsupported,
                            "D1 raster forbids external sidecars",
                        ))
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                    Err(e) => return Err(JobError::io("inspect raster sidecar", &companion, e)),
                }
            }
            let mut header = [0u8; 4];
            File::open(&source)
                .and_then(|mut file| file.read_exact(&mut header))
                .map_err(|e| {
                    if e.kind() == std::io::ErrorKind::UnexpectedEof {
                        error(
                            JobErrorKind::InvalidInput,
                            "truncated raster TIFF signature",
                        )
                    } else {
                        JobError::io("read raster signature", &source, e)
                    }
                })?;
            if ![b"II\x2a\x00", b"MM\x00\x2a", b"II\x2b\x00", b"MM\x00\x2b"].contains(&&header) {
                return Err(error(
                    JobErrorKind::Unsupported,
                    "D1 raster requires a TIFF signature",
                ));
            }
            admit_tiff_tags(&source, before.len())?;
            if !same_source(&before, &inspect(&source)?) {
                return Err(error(
                    JobErrorKind::InvalidInput,
                    "raster source changed during admission",
                ));
            }
            Ok((source, before, target))
        };
        let (source, before, target) = prepare().map_err(|e| attempt.fail(e))?;
        let pixels = native::read(&source, request.z, request.x, request.y, &attempt)?;
        if !same_source(&before, &inspect(&source).map_err(|e| attempt.fail(e))?) {
            return Err(attempt.fail(error(
                JobErrorKind::InvalidInput,
                "raster source changed during reading",
            )));
        }
        attempt
            .emit(&RunEvent::Progress {
                phase: "raster_read",
                done: 1,
                total: Some(1),
            })
            .map_err(|e| attempt.fail(e))?;
        let report = RasterDirectoryReport {
            schema_version: 1,
            profile: "d1-web-mercator-rgb",
            source_bytes: before.len(),
            width: 256,
            height: 256,
            z: request.z,
            x: request.x,
            y: request.y,
        };
        let staging = target.stage(&attempt)?;
        if let Err(e) = produce(staging.path(), &request, &pixels, &report, &attempt) {
            return Err(staging.fail(e));
        }
        let published = staging.seal()?.publish()?;
        Ok(RasterDirectoryResult {
            output: published.output,
            cleanup_diagnostics: published.cleanup_diagnostics,
            report,
        })
    }
}

#[cfg(all(test, feature = "native-geospatial"))]
mod tests {
    use super::*;
    struct FaultWriter {
        bytes: Vec<u8>,
        writes: usize,
        flush_fault: bool,
    }
    impl Write for FaultWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.writes += 1;
            if self.writes > 1 {
                return Err(std::io::Error::other("persistent write fault"));
            }
            self.bytes.extend_from_slice(&bytes[..1]);
            Ok(1)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            if self.flush_fault {
                Err(std::io::Error::other("flush fault"))
            } else {
                Ok(())
            }
        }
    }
    #[test]
    fn partial_write_failure_never_finalizes() {
        let mut writer = FaultWriter {
            bytes: vec![],
            writes: 0,
            flush_fault: false,
        };
        let mut finalized = false;
        let e = finish_member(&mut writer, b"rgb", |_| {
            finalized = true;
            Ok(())
        })
        .unwrap_err();
        assert_eq!(writer.bytes, b"r");
        assert!(!finalized);
        assert!(e.to_string().contains("write fault"));
    }
    #[test]
    fn flush_failure_never_finalizes() {
        let mut writer = FaultWriter {
            bytes: vec![],
            writes: 0,
            flush_fault: true,
        };
        let mut finalized = false;
        assert!(finish_member(&mut writer, b"r", |_| {
            finalized = true;
            Ok(())
        })
        .unwrap_err()
        .to_string()
        .contains("flush fault"));
        assert!(!finalized);
    }
    #[test]
    fn finalizer_failure_is_returned() {
        let mut writer = Vec::new();
        assert!(
            finish_member(&mut writer, b"rgb", |_| Err(std::io::Error::other(
                "sync fault"
            )))
            .unwrap_err()
            .to_string()
            .contains("sync fault")
        );
        assert_eq!(writer, b"rgb");
    }
    #[test]
    fn composed_partial_member_failure_cleans_real_staging() {
        for failing_member in ["0.png", "report.json"] {
            let parent = crate::runtime::directory::test_directory();
            let output = parent.path().join("published");
            let request = RasterDirectoryRequest::web_mercator_rgb("unused", &output, 0, 0, 0);
            let report = RasterDirectoryReport {
                schema_version: 1,
                profile: "d1-web-mercator-rgb",
                source_bytes: 1,
                width: 256,
                height: 256,
                z: 0,
                x: 0,
                y: 0,
            };
            let control = RunControl::default();
            let attempt = control.begin().unwrap();
            let staging = DirectoryTarget::prepare(&output, OutputPolicy::CreateNew)
                .unwrap()
                .stage(&attempt)
                .unwrap();
            let work = staging.path().to_owned();
            let mut injected = false;
            let e = produce_with(
                staging.path(),
                &request,
                &vec![17; 256 * 256 * 3],
                &report,
                &attempt,
                |path, bytes| {
                    if path.file_name().unwrap() == failing_member {
                        fs::write(path, &bytes[..3]).unwrap();
                        injected = true;
                        return Err(JobError::io(
                            "write injected partial raster member",
                            path,
                            std::io::Error::other("persistent writer fault"),
                        ));
                    }
                    write_file(path, bytes)
                },
            )
            .unwrap_err();
            assert!(injected);
            assert!(work.exists());
            let failure = staging.fail(e);
            assert_eq!(failure.error.kind(), JobErrorKind::Io);
            assert!(failure.error.message().contains("persistent writer fault"));
            assert!(failure.retained_paths.is_empty());
            assert!(!output.exists());
            assert!(!work.exists());
            assert_eq!(fs::read_dir(parent.path()).unwrap().count(), 0);
        }
    }
}
