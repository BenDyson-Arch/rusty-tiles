use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

#[path = "audit.rs"]
pub(crate) mod audit;
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Deserialize, Serialize)]
pub struct Manifest {
    pub version: u32,
    pub assembled: String,
    pub sources: BTreeMap<String, Value>,
    pub files: BTreeMap<String, Input>,
    pub cases: Vec<Case>,
    #[serde(rename = "referenceCategories")]
    pub reference_categories: Vec<String>,
}
#[derive(Deserialize, Serialize)]
pub struct Input {
    pub bytes: u64,
    pub sha256: String,
    pub source: String,
}
#[derive(Deserialize, Serialize)]
pub struct Case {
    pub id: String,
    pub category: String,
    pub profile: String,
    pub input: String,
    pub command: String,
    pub args: Vec<String>,
    pub audit: String,
    pub expect: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prepare: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub features: Option<u64>,
}

impl Manifest {
    pub fn load() -> Result<Self> {
        let result: Self = serde_json::from_str(include_str!("../demodata_manifest.json"))?;
        result.validate()?;
        Ok(result)
    }
    pub fn validate(&self) -> Result<()> {
        if self.version != 1 {
            return Err("unsupported demo manifest version".into());
        }
        let mut ids = BTreeSet::new();
        for (path, input) in &self.files {
            relative(path)?;
            if input.sha256.len() != 64 || !input.sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(format!("invalid input hash: {path}").into());
            }
            let source = self
                .sources
                .get(&input.source)
                .ok_or("missing input attribution")?;
            for field in ["url", "license", "attribution"] {
                if source[field].as_str().is_none_or(str::is_empty) {
                    return Err(format!("missing {field} for {path}").into());
                }
            }
        }
        for case in &self.cases {
            relative(&case.id)?;
            if case.id.contains('/') || !ids.insert(&case.id) {
                return Err("duplicate or invalid case id".into());
            }
            relative(&case.input)?;
            if self.dependencies(case).is_empty() {
                return Err(format!("unpinned input for {}", case.id).into());
            }
            if !matches!(case.profile.as_str(), "smoke" | "core" | "scale") {
                return Err("invalid case profile".into());
            }
            if !matches!(
                case.command.as_str(),
                "mesh-to-3tz"
                    | "glb-to-3tz"
                    | "point-cloud"
                    | "vector"
                    | "raster"
                    | "terrain"
                    | "convert"
            ) {
                return Err("unsupported demo command".into());
            }
            if !matches!(
                case.audit.as_str(),
                "archive" | "mesh" | "points" | "raster" | "terrain"
            ) {
                return Err("unknown audit".into());
            }
            if case.prepare.as_deref().is_some_and(|p| {
                !matches!(
                    p,
                    "local-feet" | "decompress-las" | "merge-las" | "tileset-resources"
                )
            }) {
                return Err("unknown preparation".into());
            }
            if case
                .args
                .iter()
                .any(|a| matches!(a.as_str(), "-i" | "--input" | "-o" | "--output" | "--force"))
            {
                return Err("recipe must not override private input/output paths".into());
            }
        }
        Ok(())
    }
    pub fn dependencies(&self, case: &Case) -> Vec<&str> {
        let prefix = format!("{}/", case.input);
        let external = case
            .input
            .ends_with(".gltf")
            .then(|| case.input.rsplit_once('/').map_or("", |(parent, _)| parent));
        self.files
            .keys()
            .filter(|path| {
                *path == &case.input
                    || path.starts_with(&prefix)
                    || external.is_some_and(|directory| path.starts_with(&format!("{directory}/")))
            })
            .map(String::as_str)
            .collect()
    }
    pub fn select(&self, profile: &str, ids: &[String]) -> Result<Vec<&Case>> {
        if !matches!(profile, "smoke" | "core" | "scale" | "all") {
            return Err("unknown profile".into());
        }
        for id in ids {
            if !self.cases.iter().any(|c| &c.id == id) {
                return Err(format!("unknown case: {id}").into());
            }
        }
        Ok(self
            .cases
            .iter()
            .filter(|c| {
                if ids.is_empty() {
                    profile == "all"
                        || c.profile == profile
                        || (profile == "core" && c.profile == "smoke")
                } else {
                    ids.contains(&c.id)
                }
            })
            .collect())
    }
}

fn relative(path: &str) -> Result<()> {
    if path.is_empty()
        || path.contains(['\\', ':'])
        || Path::new(path)
            .components()
            .any(|p| !matches!(p, Component::Normal(_)))
    {
        return Err(format!("unsafe demo path: {path}").into());
    }
    Ok(())
}
pub fn hash_file(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hash = Sha256::new();
    let mut bytes = [0; 65536];
    loop {
        let count = file.read(&mut bytes)?;
        if count == 0 {
            break;
        }
        hash.update(&bytes[..count]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
pub fn verify_inputs(manifest: &Manifest, root: &Path, paths: &BTreeSet<&str>) -> Result<()> {
    let root = root.canonicalize()?;
    for path in paths {
        let input = manifest.files.get(*path).ok_or("unregistered input")?;
        let file = root
            .join(path)
            .canonicalize()
            .map_err(|e| format!("missing input {path}: {e}"))?;
        if !file.starts_with(&root) {
            return Err(format!("input escapes corpus: {path}").into());
        }
        if file.metadata()?.len() != input.bytes || hash_file(&file)? != input.sha256 {
            return Err(format!("changed input: {path}").into());
        }
    }
    Ok(())
}
pub fn verify_all(manifest: &Manifest, root: &Path) -> Result<usize> {
    let paths = manifest.files.keys().map(String::as_str).collect();
    verify_inputs(manifest, root, &paths)?;
    Ok(manifest.files.len())
}

pub struct Options {
    pub data_root: PathBuf,
    pub binary: PathBuf,
    pub baseline: Option<PathBuf>,
    pub output: PathBuf,
    pub profile: String,
    pub cases: Vec<String>,
    pub benchmark: bool,
    pub repeats: usize,
}
#[derive(Serialize, Deserialize)]
pub struct Sample {
    #[serde(rename = "wallSeconds")]
    pub wall_seconds: f64,
    #[serde(rename = "cpuSeconds")]
    pub cpu_seconds: Option<f64>,
    #[serde(rename = "peakRssMiB")]
    pub peak_rss_mib: Option<f64>,
    pub exit_code: Option<i32>,
}
pub fn measure_process(command: &[String], log: &Path) -> Result<Sample> {
    if command.is_empty() {
        return Err("missing measured command".into());
    }
    let output = File::create(log)?;
    let start = Instant::now();
    let child = Command::new(&command[0])
        .args(&command[1..])
        .env("PATH", "")
        .env("PROJ_NETWORK", "OFF")
        .stdout(Stdio::from(output.try_clone()?))
        .stderr(Stdio::from(output))
        .spawn()?;
    #[cfg(unix)]
    let (exit_code, cpu_seconds, peak_rss_mib) = {
        use std::os::unix::process::ExitStatusExt;
        let mut status = 0;
        // SAFETY: rusage is a C POD output structure, initialized before use.
        let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
        loop {
            // SAFETY: child is owned by this function. Output pointers are live
            // and wait4 waits for precisely this PID, with no other waiter.
            let result = unsafe { libc::wait4(child.id() as i32, &mut status, 0, &mut usage) };
            if result >= 0 {
                break;
            }
            let error = std::io::Error::last_os_error();
            if error.kind() != std::io::ErrorKind::Interrupted {
                return Err(error.into());
            }
        }
        let cpu = usage.ru_utime.tv_sec as f64
            + usage.ru_stime.tv_sec as f64
            + (usage.ru_utime.tv_usec + usage.ru_stime.tv_usec) as f64 / 1e6;
        let divisor = if cfg!(target_os = "macos") {
            1048576.
        } else {
            1024.
        };
        (
            std::process::ExitStatus::from_raw(status).code(),
            Some(cpu),
            Some(usage.ru_maxrss as f64 / divisor),
        )
    };
    #[cfg(not(unix))]
    let (exit_code, cpu_seconds, peak_rss_mib) = {
        let mut child = child;
        (child.wait()?.code(), None, None)
    };
    Ok(Sample {
        wall_seconds: start.elapsed().as_secs_f64(),
        cpu_seconds,
        peak_rss_mib,
        exit_code,
    })
}
fn measured(command: &[String], log: &Path, isolated: bool) -> Result<Sample> {
    if !isolated {
        return measure_process(command, log);
    }
    // The tiny worker starts before fixture audits allocate memory, so wait4
    // does not mistake the suite's own RSS for the converter's RSS at exec.
    let output = Command::new(std::env::current_exe()?)
        .arg("__measure")
        .arg(log)
        .args(command)
        .output()?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned().into());
    }
    Ok(serde_json::from_slice(&output.stdout)?)
}

fn prepare(case: &Case, data: &Path, directory: &Path) -> Result<PathBuf> {
    let input = data.join(&case.input);
    match case.prepare.as_deref() {
        None => Ok(input),
        Some("tileset-resources") => {
            let stage = directory.join("input");
            fs::create_dir(&stage)?;
            for entry in walkdir::WalkDir::new(&input) {
                let entry = entry?;
                if entry.file_type().is_file()
                    && entry
                        .path()
                        .extension()
                        .and_then(|s| s.to_str())
                        .is_some_and(|e| {
                            matches!(
                                e,
                                "json"
                                    | "glb"
                                    | "gltf"
                                    | "bin"
                                    | "subtree"
                                    | "png"
                                    | "jpg"
                                    | "jpeg"
                                    | "ktx2"
                            )
                        })
                {
                    let path = stage.join(entry.path().strip_prefix(&input)?);
                    fs::create_dir_all(path.parent().unwrap())?;
                    fs::copy(entry.path(), path)?;
                }
            }
            Ok(stage)
        }
        Some(mode) => {
            let mut paths = if mode == "merge-las" {
                fs::read_dir(&input)?
                    .map(|p| p.map(|p| p.path()))
                    .collect::<std::result::Result<Vec<_>, _>>()?
            } else {
                vec![input]
            };
            paths.retain(|p| p.extension().is_some_and(|e| e == "las" || e == "laz"));
            paths.sort();
            let mut reader = las::Reader::from_path(paths.first().ok_or("empty LAS preparation")?)?;
            let mut builder = las::Builder::from(reader.header().clone());
            builder.point_format.is_compressed = false;
            if mode == "local-feet" {
                builder.vlrs.retain(|v| v.user_id != "LASF_Projection");
                builder.evlrs.retain(|v| v.user_id != "LASF_Projection");
                builder.has_wkt_crs = false;
                for (transform, factor) in [
                    (&mut builder.transforms.x, 0.3048),
                    (&mut builder.transforms.y, 0.3048),
                    (&mut builder.transforms.z, 1200. / 3937.),
                ] {
                    transform.scale *= factor;
                    transform.offset *= factor;
                }
            }
            let original = reader.header().clone();
            let path = directory.join("input.las");
            let staged_header = builder.into_header()?;
            let mut writer = las::Writer::from_path(&path, staged_header.clone())?;
            for (index, path) in paths.iter().enumerate() {
                if index > 0 {
                    reader = las::Reader::from_path(path)?;
                }
                if reader.header().point_format() != original.point_format()
                    || reader.header().transforms() != original.transforms()
                    || reader.header().vlrs() != original.vlrs()
                    || reader.header().evlrs() != original.evlrs()
                {
                    return Err(
                        "merged LAS nodes have incompatible record layouts, transforms or VLRs"
                            .into(),
                    );
                }
                loop {
                    let block = reader.read_points(65536)?;
                    if block.is_empty() {
                        break;
                    }
                    // Re-label raw records with the staged header; scaling the
                    // header changes local units without rounding any integer
                    // coordinates or altering flags and Extra Bytes.
                    let staged = las::PointDataBuilder::new()
                        .for_header(&staged_header)
                        .build_from_bytes(block.raw_bytes().to_vec())?;
                    writer.write_points(&staged)?;
                }
            }
            writer.close()?;
            Ok(path)
        }
    }
}

pub fn snapshot(path: &Path) -> Result<String> {
    let mut files = BTreeMap::new();
    if path.is_dir() {
        for entry in walkdir::WalkDir::new(path) {
            let entry = entry?;
            if entry.file_type().is_file() && entry.file_name() != "conversion.json" {
                files.insert(
                    entry
                        .path()
                        .strip_prefix(path)?
                        .to_string_lossy()
                        .into_owned(),
                    hash_file(entry.path())?,
                );
            }
        }
    } else {
        let mut zip = zip::ZipArchive::new(File::open(path)?)?;
        for i in 0..zip.len() {
            let mut file = zip.by_index(i)?;
            if matches!(file.name(), "conversion.json") || file.name() == rusty_tiles::TZ_INDEX_NAME
            {
                continue;
            }
            let mut hash = Sha256::new();
            let mut buffer = [0; 65536];
            loop {
                let count = file.read(&mut buffer)?;
                if count == 0 {
                    break;
                }
                hash.update(&buffer[..count]);
            }
            files.insert(file.name().to_owned(), format!("{:x}", hash.finalize()));
        }
    }
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(&files)?)))
}
fn bytes(path: &Path) -> Result<u64> {
    if path.is_file() {
        return Ok(path.metadata()?.len());
    }
    walkdir::WalkDir::new(path)
        .into_iter()
        .try_fold(0, |sum, e| {
            let e = e?;
            Ok(sum
                + if e.file_type().is_file() {
                    e.metadata()?.len()
                } else {
                    0
                })
        })
}
fn check(case: &Case, sample: &Sample, input: &Path, output: &Path, log: &Path) -> Result<Value> {
    if case.expect != "success" {
        let text = fs::read_to_string(log)?;
        let diagnostic = text
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .find(|doc| doc.get("ok").is_some())
            .ok_or("missing machine error result")?;
        let (category, exit_code, message) = match case.expect.as_str() {
            "unsupported" => ("unsupported", 2, None),
            reason => ("data", 3, Some(reason)),
        };
        if sample.exit_code != Some(exit_code)
            || diagnostic["error"]["code"] != category
            || message.is_some_and(|reason| {
                !diagnostic["error"]["message"]
                    .as_str()
                    .is_some_and(|m| m.contains(reason))
            })
            || output.exists()
        {
            return Err(format!(
                "{}: expected safe rejection, see {}",
                case.id,
                log.display()
            )
            .into());
        }
        return Ok(json!({"expectedRejection":case.expect,"unpublished":true}));
    }
    if sample.exit_code != Some(0) {
        return Err(format!("{} failed, see {}", case.id, log.display()).into());
    }
    let mut result = audit::check(&case.audit, input, output)?;
    if let Some(features) = case.features {
        let report = audit::archive_json(output, "conversion.json")?;
        let encoded = report["features"].as_u64().ok_or("missing feature count")?;
        let skipped = report["skippedFeatures"].as_u64().unwrap_or(0);
        if encoded + skipped != features {
            return Err(format!(
                "{}: source feature accounting {encoded}+{skipped} differs from {features}",
                case.id
            )
            .into());
        }
        result["sourceFeatureAccounting"] =
            json!({"source":features,"encoded":encoded,"skipped":skipped});
    }
    result["outputBytes"] = json!(bytes(output)?);
    Ok(result)
}

pub fn run(manifest: &Manifest, options: Options) -> Result<Value> {
    manifest.validate()?;
    if options.benchmark && options.repeats < 3 {
        return Err("benchmarks require at least three repetitions".into());
    }
    let selected = manifest.select(&options.profile, &options.cases)?;
    if selected.is_empty() {
        return Err("empty demo selection".into());
    }
    let data = options.data_root.canonicalize()?;
    let absolute = std::path::absolute(&options.output)?;
    if absolute.exists() {
        return Err("output already exists; choose a fresh directory".into());
    }
    let mut existing = absolute.as_path();
    let mut tail = Vec::new();
    while !existing.exists() {
        tail.push(existing.file_name().ok_or("output has no name")?.to_owned());
        existing = existing.parent().ok_or("output has no parent")?;
    }
    let mut output = existing.canonicalize()?;
    for component in tail.iter().rev() {
        output.push(component);
    }
    if output.starts_with(&data) || data.starts_with(&output) {
        return Err("output overlaps the corpus".into());
    }
    let paths = selected
        .iter()
        .flat_map(|c| manifest.dependencies(c))
        .collect();
    verify_inputs(manifest, &data, &paths)?;
    let mut binaries = vec![("current", options.binary.canonicalize()?)];
    if let Some(baseline) = options.baseline {
        binaries.push(("baseline", baseline.canonicalize()?));
    }
    fs::create_dir_all(output.parent().ok_or("output has no parent")?)?;
    fs::create_dir(&output)?;
    let mut methods = BTreeMap::new();
    for (name, binary) in &binaries {
        let version = Command::new(binary).arg("--version").output()?;
        if !version.status.success() {
            return Err("cannot inspect CLI version".into());
        }
        let doctor = Command::new(binary)
            .args(["doctor", "--json"])
            .env("PATH", "")
            .env("PROJ_NETWORK", "OFF")
            .output()?;
        let doctor: Value = serde_json::from_slice(&doctor.stdout)?;
        methods.insert(*name,json!({"binarySha256":hash_file(binary)?,"version":String::from_utf8_lossy(&version.stdout).trim(),"doctor":doctor}));
    }
    let mut report = json!({"version":1,"recordedUnixSeconds":SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
        "platform":{"os":std::env::consts::OS,"arch":std::env::consts::ARCH,"availableParallelism":std::thread::available_parallelism().ok().map(usize::from),
            "cpuModel":fs::read_to_string("/proc/cpuinfo").ok().and_then(|s|s.lines().find_map(|l|l.strip_prefix("model name").and_then(|v|v.split_once(':').map(|(_,model)|model.trim().to_owned())))),
            "memoryKiB":fs::read_to_string("/proc/meminfo").ok().and_then(|s|s.lines().find_map(|l|l.strip_prefix("MemTotal:").and_then(|v|v.split_whitespace().next()?.parse::<u64>().ok())))},"methods":methods,
        "manifestSha256":format!("{:x}",Sha256::digest(serde_json::to_vec(manifest)?)),
        "inputFiles":paths.iter().map(|path|(*path,&manifest.files[*path])).collect::<BTreeMap<_,_>>(),
        "attribution":manifest.sources,"benchmark":options.benchmark,"repeats":if options.benchmark {options.repeats} else {1},
        "measurement":"Warm filesystem cache; one warmup and rotated serial runs. Conversion process wall/CPU time and largest-process peak RSS; audits and preparation excluded. Native CLI PATH empty, PROJ_NETWORK OFF. CPU/RSS unavailable on non-Unix.",
        "payloadComparison":"SHA-256 over output names and member/file hashes, excluding conversion.json and the ZIP index; timing diagnostics excluded, geometry and metadata retained.","cases":[]});
    let result_path = output.join("results.json");
    for case in selected {
        let directory = output.join(&case.id);
        fs::create_dir(&directory)?;
        let input = prepare(case, &data, &directory)?;
        let prepared = if case.prepare.is_some() {
            Some(json!({"bytes":bytes(&input)?,
            "sha256":if input.is_file() {hash_file(&input)?} else {snapshot(&input)?}}))
        } else {
            None
        };
        let mut commands = Vec::new();
        for (name, binary) in &binaries {
            let target = directory.join(if matches!(case.command.as_str(), "raster" | "terrain") {
                name.to_string()
            } else {
                format!("{name}.3tz")
            });
            let mut command = vec![
                binary.to_string_lossy().into_owned(),
                "--json".into(),
                case.command.clone(),
                "-i".into(),
                input.to_string_lossy().into_owned(),
                "-o".into(),
                target.to_string_lossy().into_owned(),
                "--force".into(),
            ];
            command.extend(case.args.clone());
            commands.push((*name, command, target));
        }
        let mut samples: BTreeMap<&str, Vec<Sample>> = BTreeMap::new();
        let mut fingerprints = BTreeMap::new();
        let mut checks = BTreeMap::new();
        if options.benchmark {
            for (name, command, target) in &commands {
                let log = directory.join(format!("{name}-warmup.log"));
                let sample = measured(command, &log, true)?;
                check(case, &sample, &input, target, &log)?;
            }
        }
        let repeats = if options.benchmark {
            options.repeats
        } else {
            1
        };
        for repeat in 0..repeats {
            for offset in 0..commands.len() {
                let (name, command, target) = &commands[(offset + repeat) % commands.len()];
                let log = directory.join(format!("{name}-{repeat}.log"));
                let sample = measured(command, &log, options.benchmark)?;
                let checked = check(case, &sample, &input, target, &log)?;
                if case.expect == "success" {
                    let fingerprint = snapshot(target)?;
                    if fingerprints
                        .get(name)
                        .is_some_and(|before| before != &fingerprint)
                    {
                        return Err(format!(
                            "{}: {name} payloads differ between repetitions",
                            case.id
                        )
                        .into());
                    }
                    fingerprints.insert(name, fingerprint);
                }
                checks.insert(name, checked);
                samples.entry(name).or_default().push(sample);
            }
        }
        let medians:BTreeMap<_,_>=samples.iter().map(|(method,samples)| {
            let median=|mut values:Vec<f64>| {values.sort_by(f64::total_cmp);let middle=values.len()/2;if values.len().is_multiple_of(2) {(values[middle-1]+values[middle])/2.} else {values[middle]}};
            (*method,json!({"wallSeconds":median(samples.iter().map(|s|s.wall_seconds).collect()),
                "cpuSeconds":samples.iter().map(|s|s.cpu_seconds).collect::<Option<Vec<_>>>().map(median),
                "peakRssMiB":samples.iter().map(|s|s.peak_rss_mib).collect::<Option<Vec<_>>>().map(median)}))
        }).collect();
        let entry = json!({"id":case.id,"recipe":case,"preparedInput":prepared,"commands":commands.iter().map(|(n,c,_)|(*n,c)).collect::<BTreeMap<_,_>>(),"samples":samples,"medians":medians,"checks":checks,"payloadFingerprints":fingerprints});
        report["cases"].as_array_mut().unwrap().push(entry);
        let mut file = File::create(&result_path)?;
        file.write_all(&serde_json::to_vec_pretty(&report)?)?;
        println!("{}: passed", case.id);
    }
    verify_inputs(manifest, &data, &paths)?;
    report["ok"] = json!(true);
    fs::write(&result_path, serde_json::to_vec_pretty(&report)?)?;
    println!("results: {}", result_path.display());
    Ok(report)
}
