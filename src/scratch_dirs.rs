//! Scratch and work folders: one creation helper that writes each folder's owner record,
//! and the inventory that `rusty-tiles scratch list|clean` and `doctor` read.
//!
//! Every folder rusty-tiles creates for working data goes through [`create`]. Its name is
//! one of the [`Kind`] prefixes followed by six random alphanumerics, and a folder created
//! directly in a scan root holds [`OWNER_FILE`]: the creating process's PID and start
//! token, host, command, rusty-tiles version and creation time, written by rename so a
//! reader sees either the whole record or none. A folder created inside another
//! rusty-tiles folder is covered by that folder's record and removed with it.
//!
//! A folder is `live` while its owner runs, `stale` once the owner is provably gone (no
//! such PID, or the PID now names a process with a different start token), and
//! `unverified` when neither can be proved (another host or user, or no readable start
//! token). A folder without a readable record (written by 0.4.0, or interrupted before the
//! record landed) is stale only when nothing in it changed for [`OWNERLESS_STALE_AFTER`]
//! (or the caller's own threshold). Only stale folders are ever removed.
//!
//! Output staging that becomes the published result (`.tiles-dir-`, `.tiles-stage-`) and
//! the held previous output (`.tiles-previous-`) are not scratch and are never listed.

pub(crate) mod process;

use process::Presence;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The owner record's file name inside a scratch folder.
pub const OWNER_FILE: &str = "rusty-tiles-owner.json";
const OWNER_PARTIAL: &str = ".rusty-tiles-owner.json.partial";
/// The `format` value of an owner record.
pub const OWNER_FORMAT: &str = "rusty-tiles-scratch-owner";
/// The owner record version this build writes and reads.
pub const OWNER_VERSION: u32 = 1;
/// Records larger than this are not owner records.
const OWNER_MAX_BYTES: u64 = 64 * 1024;
/// Bytes of the command line kept in a record.
const COMMAND_MAX_BYTES: usize = 1024;
/// Random characters `tempfile` appends to each prefix.
const SUFFIX_LEN: usize = 6;
/// How long a folder without an owner record must stay unmodified to count as stale.
pub const OWNERLESS_STALE_AFTER: Duration = Duration::from_secs(24 * 60 * 60);

/// What a scratch folder holds, by its name prefix.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    /// Spill files of external sorts and the validator catalog.
    Spill,
    /// The streaming mesh tiler's private intermediate.
    MeshWork,
    /// Mesh forest staging (inside `MeshWork`).
    MeshForest,
    /// `glb-to-3tz` archive workspace.
    ModelWork,
    /// Point-cloud workspace.
    PointCloudWork,
    /// Vector workspace.
    VectorWork,
}

impl Kind {
    pub const ALL: [Kind; 6] = [
        Self::Spill,
        Self::MeshWork,
        Self::MeshForest,
        Self::ModelWork,
        Self::PointCloudWork,
        Self::VectorWork,
    ];

    /// The folder name prefix.
    pub const fn prefix(self) -> &'static str {
        match self {
            Self::Spill => "rusty-tiles-spill-",
            Self::MeshWork => ".tiles-mesh-",
            Self::MeshForest => ".tiles-h16-",
            Self::ModelWork => ".mesh-work-",
            Self::PointCloudWork => ".point-work-",
            Self::VectorWork => ".vector-work-",
        }
    }

    /// The kind whose pattern `name` matches exactly: prefix, then six ASCII alphanumerics.
    pub fn of_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| {
            name.strip_prefix(kind.prefix()).is_some_and(|suffix| {
                suffix.len() == SUFFIX_LEN && suffix.bytes().all(|b| b.is_ascii_alphanumeric())
            })
        })
    }
}

/// Create a private `kind` folder in `parent`, with its owner record unless `parent` is
/// inside another rusty-tiles scratch folder. Creation in a metered directory is metered.
pub(crate) fn create(parent: &Path, kind: Kind) -> io::Result<tempfile::TempDir> {
    let dir = crate::storage_meter::tempdir_in(parent, kind.prefix())?;
    if !nested(parent) {
        write_owner(dir.path(), &OwnerRecord::current())?;
    }
    Ok(dir)
}

/// Charge the owner record already in `dir` to `meter`, for a folder created before its
/// meter was registered.
pub(crate) fn admit_owner_record(
    meter: &crate::storage_meter::Meter,
    dir: &Path,
) -> io::Result<()> {
    match fs::symlink_metadata(dir.join(OWNER_FILE)) {
        Ok(metadata) => meter.admit(meter.file_bytes(metadata.len())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

/// The entries of a scratch folder other than its owner record, in name order.
#[cfg(test)]
pub(crate) fn data_entries(dir: &Path) -> Vec<PathBuf> {
    let mut entries: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| !path.ends_with(OWNER_FILE))
        .collect();
    entries.sort();
    entries
}

/// Whether any ancestor of `path` is a rusty-tiles scratch folder.
fn nested(path: &Path) -> bool {
    path.components().any(|component| match component {
        Component::Normal(name) => name.to_str().and_then(Kind::of_name).is_some(),
        _ => false,
    })
}

/// The process that created a scratch folder.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnerRecord {
    pub format: String,
    pub version: u32,
    pub pid: u32,
    /// Platform start token of `pid`; see [`process`]. `null` where none can be read.
    pub process_start: Option<String>,
    pub host: Option<String>,
    /// The command line, lossily decoded and truncated to 1 KiB.
    pub command: String,
    pub rusty_tiles: String,
    pub created_unix_seconds: u64,
}

impl OwnerRecord {
    /// The record of the running process.
    pub fn current() -> Self {
        let (pid, process_start) = process::current();
        let mut command = std::env::args_os()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(" ");
        if command.len() > COMMAND_MAX_BYTES {
            let mut end = COMMAND_MAX_BYTES;
            while !command.is_char_boundary(end) {
                end -= 1;
            }
            command.truncate(end);
        }
        Self {
            format: OWNER_FORMAT.to_owned(),
            version: OWNER_VERSION,
            pid,
            process_start,
            host: process::host(),
            command,
            rusty_tiles: env!("CARGO_PKG_VERSION").to_owned(),
            created_unix_seconds: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.as_secs()),
        }
    }

    /// Parse a record, refusing other formats and versions.
    pub fn parse(bytes: &[u8]) -> Result<Self, RecordStatus> {
        let record: Self = serde_json::from_slice(bytes).map_err(|_| RecordStatus::Invalid)?;
        if record.format != OWNER_FORMAT {
            return Err(RecordStatus::Invalid);
        }
        if record.version != OWNER_VERSION {
            return Err(RecordStatus::Unsupported);
        }
        Ok(record)
    }
}

/// Write `record` into `dir` atomically: a partial file, then a rename over nothing. Under
/// a metered directory the record is admitted before it is written.
fn write_owner(dir: &Path, record: &OwnerRecord) -> io::Result<()> {
    let mut bytes = serde_json::to_vec_pretty(record).map_err(io::Error::other)?;
    bytes.push(b'\n');
    let partial = dir.join(OWNER_PARTIAL);
    let mut file = crate::storage_meter::create_new(&partial)?;
    file.write_all(&bytes)?;
    drop(file);
    fs::rename(&partial, dir.join(OWNER_FILE))
}

/// Read `dir`'s record. A record that is a symlink or not a regular file is invalid.
fn read_owner(dir: &Path) -> Result<OwnerRecord, RecordStatus> {
    let path = dir.join(OWNER_FILE);
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.is_file() && metadata.len() <= OWNER_MAX_BYTES => {}
        Ok(_) => return Err(RecordStatus::Invalid),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Err(RecordStatus::Missing),
        Err(_) => return Err(RecordStatus::Unreadable),
    }
    let mut bytes = Vec::new();
    fs::File::open(&path)
        .and_then(|file| file.take(OWNER_MAX_BYTES + 1).read_to_end(&mut bytes))
        .map_err(|_| RecordStatus::Unreadable)?;
    OwnerRecord::parse(&bytes)
}

/// Whether a folder's owner record could be used.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordStatus {
    Present,
    Missing,
    Invalid,
    Unsupported,
    Unreadable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    /// The owner is running.
    Live,
    /// The owner is gone; `clean` may remove the folder.
    Stale,
    /// Neither could be proved; never removed.
    Unverified,
}

/// Why a folder has its [`State`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    /// The PID runs with the recorded start token.
    OwnerRunning,
    /// No process has the PID.
    OwnerExited,
    /// The PID names a process with a different start token.
    PidReused,
    /// The record names another host.
    OtherHost,
    /// The PID exists but its start token cannot be compared.
    StartUnverifiable,
    /// The folder belongs to another user.
    OtherUser,
    /// No usable record; the state follows the folder's last modification.
    NoOwnerRecord,
}

/// The state of a folder whose record is `record`, with `presence` the state of its PID
/// and `local_host` this machine's name.
pub(crate) fn judge(
    record: &OwnerRecord,
    local_host: Option<&str>,
    presence: impl FnOnce(u32) -> Presence,
) -> (State, Reason) {
    if let (Some(recorded), Some(local)) = (record.host.as_deref(), local_host) {
        if recorded != local {
            return (State::Unverified, Reason::OtherHost);
        }
    }
    match presence(record.pid) {
        Presence::Absent => (State::Stale, Reason::OwnerExited),
        Presence::Present { start: Some(start) } => match &record.process_start {
            Some(recorded) if *recorded == start => (State::Live, Reason::OwnerRunning),
            Some(_) => (State::Stale, Reason::PidReused),
            None => (State::Unverified, Reason::StartUnverifiable),
        },
        Presence::Present { start: None } => (State::Unverified, Reason::StartUnverifiable),
    }
}

/// One scan root.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Root {
    pub path: String,
    /// The root was given explicitly (`--dir`) rather than being a default location.
    pub explicit: bool,
    pub exists: bool,
    /// Why the root could not be listed.
    pub error: Option<String>,
}

/// One rusty-tiles scratch folder.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Folder {
    pub path: String,
    pub kind: Kind,
    /// Total length of the regular files inside; symlinks are not followed.
    pub bytes: u64,
    /// `false` when part of the folder could not be read, so `bytes` is a lower bound.
    pub size_complete: bool,
    /// Seconds since anything in the folder (or the folder itself) was last modified.
    pub modified_seconds_ago: u64,
    pub owner_record: RecordStatus,
    pub owner: Option<OwnerRecord>,
    pub state: State,
    pub reason: Reason,
    #[serde(skip)]
    location: PathBuf,
    #[serde(skip)]
    modified: Duration,
}

/// The folders found in `roots`, in root order and then by name.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Inventory {
    pub roots: Vec<Root>,
    pub folders: Vec<Folder>,
    /// The ownerless staleness threshold that was applied.
    pub ownerless_stale_after_seconds: u64,
}

impl Inventory {
    /// Count and total bytes of the stale folders.
    pub fn stale(&self) -> (usize, u64) {
        self.folders
            .iter()
            .filter(|folder| folder.state == State::Stale)
            .fold((0, 0), |(count, bytes), folder| {
                (count + 1, bytes.saturating_add(folder.bytes))
            })
    }
}

/// The default scratch locations: `$TMPDIR`, the user cache directory's
/// `rusty-tiles/scratch` and the system temporary directory.
pub fn default_roots() -> Vec<PathBuf> {
    crate::spill::default_scratch_roots()
}

/// The roots to scan: the defaults, then `extra`, without duplicates.
fn roots(extra: &[PathBuf]) -> Vec<(PathBuf, bool)> {
    let mut roots: Vec<(PathBuf, bool)> = Vec::new();
    let mut seen: Vec<PathBuf> = Vec::new();
    let candidates = default_roots()
        .into_iter()
        .map(|path| (path, false))
        .chain(extra.iter().map(|path| (path.clone(), true)));
    for (path, explicit) in candidates {
        let identity = fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
        if let Some(index) = seen.iter().position(|known| *known == identity) {
            roots[index].1 |= explicit;
            continue;
        }
        seen.push(identity);
        roots.push((path, explicit));
    }
    roots
}

/// Inspect `roots` (the defaults plus `extra`) with the ownerless threshold `ownerless`.
pub fn list(extra: &[PathBuf], ownerless: Duration) -> Inventory {
    scan(&roots(extra), ownerless, SystemTime::now(), &process::query)
}

fn scan(
    roots: &[(PathBuf, bool)],
    ownerless: Duration,
    now: SystemTime,
    presence: &dyn Fn(u32) -> Presence,
) -> Inventory {
    let host = process::host();
    let mut listed = Vec::new();
    let mut folders = Vec::new();
    for (root, explicit) in roots {
        let mut entry = Root {
            path: root.to_string_lossy().into_owned(),
            explicit: *explicit,
            exists: false,
            error: None,
        };
        match fs::read_dir(root) {
            Ok(entries) => {
                entry.exists = true;
                let mut found: Vec<_> = entries
                    .flatten()
                    .filter_map(|entry| {
                        let name = entry.file_name().into_string().ok()?;
                        Kind::of_name(&name)?;
                        Some(root.join(name))
                    })
                    .collect();
                found.sort();
                folders.extend(
                    found.iter().filter_map(|path| {
                        inspect(path, ownerless, now, host.as_deref(), presence)
                    }),
                );
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => entry.error = Some(error.to_string()),
        }
        listed.push(entry);
    }
    Inventory {
        roots: listed,
        folders,
        ownerless_stale_after_seconds: ownerless.as_secs(),
    }
}

/// Inspect one candidate. `None` unless `path` is a real directory (not a symlink) whose
/// name is a rusty-tiles pattern.
fn inspect(
    path: &Path,
    ownerless: Duration,
    now: SystemTime,
    host: Option<&str>,
    presence: &dyn Fn(u32) -> Presence,
) -> Option<Folder> {
    let kind = Kind::of_name(path.file_name()?.to_str()?)?;
    let metadata = fs::symlink_metadata(path).ok()?;
    if !metadata.is_dir() {
        return None;
    }
    let (bytes, newest, size_complete) = measure(path, &metadata);
    let modified = newest
        .and_then(|newest| now.duration_since(newest).ok())
        .unwrap_or(Duration::ZERO);
    let (owner_record, owner, (state, reason)) = if !process::owned_by_current_user(&metadata) {
        (
            RecordStatus::Unreadable,
            None,
            (State::Unverified, Reason::OtherUser),
        )
    } else {
        match read_owner(path) {
            Ok(record) => {
                let verdict = judge(&record, host, presence);
                (RecordStatus::Present, Some(record), verdict)
            }
            Err(status) => {
                let state = if modified >= ownerless {
                    State::Stale
                } else {
                    State::Unverified
                };
                (status, None, (state, Reason::NoOwnerRecord))
            }
        }
    };
    Some(Folder {
        path: path.to_string_lossy().into_owned(),
        kind,
        bytes,
        size_complete,
        modified_seconds_ago: modified.as_secs(),
        owner_record,
        owner,
        state,
        reason,
        location: path.to_owned(),
        modified,
    })
}

/// Total regular-file bytes under `dir`, the newest modification time of anything in it,
/// and whether every entry could be read. Symlinks are counted as entries, never followed.
fn measure(dir: &Path, metadata: &fs::Metadata) -> (u64, Option<SystemTime>, bool) {
    let mut bytes = 0u64;
    let mut newest = metadata.modified().ok();
    let mut complete = true;
    let mut pending = vec![dir.to_owned()];
    while let Some(dir) = pending.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(_) => {
                complete = false;
                continue;
            }
        };
        for entry in entries {
            let Ok(entry) = entry else {
                complete = false;
                continue;
            };
            let path = entry.path();
            let metadata = match fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                // Removed while we looked: a live run's file.
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(_) => {
                    complete = false;
                    continue;
                }
            };
            if let Ok(time) = metadata.modified() {
                newest = Some(newest.map_or(time, |known| known.max(time)));
            }
            if metadata.is_dir() {
                pending.push(path);
            } else if metadata.is_file() {
                bytes = bytes.saturating_add(metadata.len());
            }
        }
    }
    (bytes, newest, complete)
}

/// A folder `clean` could not remove.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Failure {
    pub path: String,
    pub message: String,
    /// Bytes freed before the removal failed.
    pub bytes_freed: u64,
}

/// What `clean` did.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanReport {
    pub dry_run: bool,
    pub roots: Vec<Root>,
    /// Removed folders, or with `dry_run` the folders that would be removed.
    pub removed: Vec<Folder>,
    /// Folders left in place, with their state and reason.
    pub kept: Vec<Folder>,
    pub failed: Vec<Failure>,
    /// Bytes removed (with `dry_run`, the bytes that would be).
    pub bytes_freed: u64,
    /// `--older-than` in seconds, when given.
    pub older_than_seconds: Option<u64>,
    pub ownerless_stale_after_seconds: u64,
}

/// Remove the stale folders in the defaults and `extra`. With `older_than`, only folders
/// unmodified for at least that long, and it replaces [`OWNERLESS_STALE_AFTER`] for
/// folders without a record. Each folder is inspected again immediately before removal.
pub fn clean(extra: &[PathBuf], older_than: Option<Duration>, dry_run: bool) -> CleanReport {
    clean_with(
        &roots(extra),
        older_than,
        dry_run,
        SystemTime::now,
        &process::query,
    )
}

fn clean_with(
    roots: &[(PathBuf, bool)],
    older_than: Option<Duration>,
    dry_run: bool,
    now: impl Fn() -> SystemTime,
    presence: &dyn Fn(u32) -> Presence,
) -> CleanReport {
    let ownerless = older_than.unwrap_or(OWNERLESS_STALE_AFTER);
    let selected = |folder: &Folder| {
        folder.state == State::Stale && older_than.is_none_or(|age| folder.modified >= age)
    };
    let inventory = scan(roots, ownerless, now(), presence);
    let host = process::host();
    let mut report = CleanReport {
        dry_run,
        roots: inventory.roots,
        removed: Vec::new(),
        kept: Vec::new(),
        failed: Vec::new(),
        bytes_freed: 0,
        older_than_seconds: older_than.map(|age| age.as_secs()),
        ownerless_stale_after_seconds: ownerless.as_secs(),
    };
    for folder in inventory.folders {
        if !selected(&folder) {
            report.kept.push(folder);
            continue;
        }
        if dry_run {
            report.bytes_freed = report.bytes_freed.saturating_add(folder.bytes);
            report.removed.push(folder);
            continue;
        }
        // The owner or contents may have changed since the scan.
        let Some(current) = inspect(
            &folder.location,
            ownerless,
            now(),
            host.as_deref(),
            presence,
        ) else {
            continue;
        };
        if !selected(&current) {
            report.kept.push(current);
            continue;
        }
        // remove_dir_all removes symlinks themselves and never follows them.
        match fs::remove_dir_all(&current.location) {
            Ok(()) => {
                report.bytes_freed = report.bytes_freed.saturating_add(current.bytes);
                report.removed.push(current);
            }
            Err(error) => {
                let remaining = fs::symlink_metadata(&current.location)
                    .map_or(0, |metadata| measure(&current.location, &metadata).0);
                let freed = current.bytes.saturating_sub(remaining);
                report.bytes_freed = report.bytes_freed.saturating_add(freed);
                report.failed.push(Failure {
                    path: current.path.clone(),
                    message: error.to_string(),
                    bytes_freed: freed,
                });
            }
        }
    }
    report
}

/// Parse a duration such as `90s`, `30m`, `24h`, `7d` or `2w`; a bare number is seconds.
pub fn parse_duration(text: &str) -> Result<Duration, String> {
    let text = text.trim();
    let split = text
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(text.len());
    let (number, unit) = text.split_at(split);
    if number.is_empty() {
        return Err(format!(
            "expected a duration such as 90s, 30m, 24h or 7d, got {text:?}"
        ));
    }
    let scale = match unit {
        "" | "s" => 1,
        "m" => 60,
        "h" => 60 * 60,
        "d" => 24 * 60 * 60,
        "w" => 7 * 24 * 60 * 60,
        _ => {
            return Err(format!(
                "unknown duration unit {unit:?}: use s, m, h, d or w"
            ))
        }
    };
    number
        .parse::<u64>()
        .ok()
        .and_then(|number| number.checked_mul(scale))
        .map(Duration::from_secs)
        .ok_or_else(|| format!("duration {text:?} is too large"))
}

/// A byte count for people: `0 B`, `512 B`, `1.5 KiB`, `143.9 GiB`.
pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests;
