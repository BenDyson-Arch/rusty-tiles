//! `rusty-tiles scratch list|clean` and the `doctor` scratch summary, run as a user would,
//! over fake folders: stale, live, ownerless, foreign names and symlinks. The owner
//! records are written here from the documented format, not by the library.
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

/// A PID beyond every supported platform's PID range, so no process has it.
const NO_SUCH_PID: u32 = 2_147_483_000;

fn owner(pid: u32, start: Option<&str>) -> Value {
    json!({
        "format": "rusty-tiles-scratch-owner",
        "version": 1,
        "pid": pid,
        "processStart": start,
        "host": null,
        "command": "rusty-tiles validate big.3tz",
        "rustyTiles": "0.4.1",
        "createdUnixSeconds": 1_700_000_000u64,
    })
}

fn folder(root: &Path, name: &str, bytes: usize, record: Option<Value>) -> PathBuf {
    let path = root.join(name);
    fs::create_dir_all(path.join("sub")).unwrap();
    fs::write(path.join("sub").join("run-0.spill"), vec![3u8; bytes]).unwrap();
    if let Some(record) = record {
        fs::write(path.join("rusty-tiles-owner.json"), record.to_string()).unwrap();
    }
    path
}

/// This test process's start token, read independently of the library.
#[cfg(target_os = "linux")]
fn own_start() -> String {
    let stat = fs::read_to_string("/proc/self/stat").unwrap();
    let rest = &stat[stat.rfind(')').unwrap() + 1..];
    let ticks = rest.split_whitespace().nth(19).unwrap();
    let boot = fs::read_to_string("/proc/sys/kernel/random/boot_id").unwrap();
    format!("linux:{}:{ticks}", boot.trim())
}

struct Env {
    _root: tempfile::TempDir,
    tmp: PathBuf,
    cache: PathBuf,
    extra: PathBuf,
    outside: PathBuf,
}

/// Default locations moved into a private root so the machine's own scratch is not seen.
fn env() -> Env {
    let root = tempfile::tempdir().unwrap();
    let [tmp, cache, extra, outside] = ["tmp", "cache", "extra", "outside"].map(|name| {
        let path = root.path().join(name);
        fs::create_dir(&path).unwrap();
        path
    });
    Env {
        _root: root,
        tmp,
        cache,
        extra,
        outside,
    }
}

fn run(env: &Env, args: &[&str]) -> (Option<i32>, String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_rusty-tiles"))
        .args(args)
        .env("TMPDIR", &env.tmp)
        .env("XDG_CACHE_HOME", &env.cache)
        .env("HOME", &env.cache)
        .env("LOCALAPPDATA", &env.cache)
        .env("TEMP", &env.tmp)
        .env("TMP", &env.tmp)
        .output()
        .unwrap();
    (
        output.status.code(),
        String::from_utf8(output.stdout).unwrap(),
        String::from_utf8(output.stderr).unwrap(),
    )
}

fn json_run(env: &Env, args: &[&str]) -> (Option<i32>, Value) {
    let (code, stdout, stderr) = run(env, args);
    let value = serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("{e}: {stdout}{stderr}"));
    (code, value)
}

fn keys(value: &Value) -> Vec<String> {
    let mut keys: Vec<_> = value.as_object().unwrap().keys().cloned().collect();
    keys.sort();
    keys
}

fn states(report: &Value, key: &str) -> Vec<(String, String, String)> {
    let mut states: Vec<_> = report[key]
        .as_array()
        .unwrap()
        .iter()
        .map(|folder| {
            let path = Path::new(folder["path"].as_str().unwrap());
            (
                path.file_name().unwrap().to_string_lossy().into_owned(),
                folder["state"].as_str().unwrap().to_owned(),
                folder["reason"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    states.sort();
    states
}

fn s(a: &str, b: &str, c: &str) -> (String, String, String) {
    (a.into(), b.into(), c.into())
}

#[test]
fn list_and_clean_remove_only_stale_rusty_tiles_folders() {
    let env = env();
    // The user cache directory's scratch location, as each platform names it.
    let cache = if cfg!(target_os = "macos") {
        env.cache.join("Library").join("Caches")
    } else {
        env.cache.clone()
    };
    let scratch = cache.join("rusty-tiles").join("scratch");
    fs::create_dir_all(&scratch).unwrap();
    let gone = folder(
        &scratch,
        "rusty-tiles-spill-GONE00",
        5000,
        Some(owner(NO_SUCH_PID, Some("x"))),
    );
    folder(
        &env.tmp,
        ".tiles-mesh-GONE01",
        700,
        Some(owner(NO_SUCH_PID, None)),
    );
    folder(&env.extra, ".vector-work-YOUNG0", 11, None);
    // Our own PID with another start time: the record's process is gone.
    folder(
        &env.extra,
        ".point-work-REUSED",
        13,
        Some(owner(std::process::id(), Some("not-this-start"))),
    );
    #[cfg(target_os = "linux")]
    folder(
        &env.extra,
        "rusty-tiles-spill-LIVE00",
        17,
        Some(owner(std::process::id(), Some(&own_start()))),
    );
    // Foreign entries with stale-looking records.
    for name in ["rusty-tiles-spill-TOOLONG7", ".tiles-dir-ABCDEF", "keep"] {
        folder(&env.extra, name, 19, Some(owner(NO_SUCH_PID, None)));
    }
    fs::write(env.extra.join(".mesh-work-AFILE0"), b"not a folder").unwrap();
    fs::write(env.outside.join("precious.bin"), vec![1u8; 2048]).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        symlink(&env.outside, gone.join("escape")).unwrap();
        let decoy = folder(
            &env.outside,
            "rusty-tiles-spill-DECOY0",
            23,
            Some(owner(NO_SUCH_PID, None)),
        );
        symlink(&decoy, env.extra.join("rusty-tiles-spill-LINK00")).unwrap();
    }
    let extra = env.extra.to_str().unwrap();

    let (code, list) = json_run(&env, &["scratch", "list", "--json", "--dir", extra]);
    assert_eq!(code, Some(0), "{list}");
    assert_eq!(
        keys(&list),
        [
            "folders",
            "ok",
            "ownerlessStaleAfterSeconds",
            "roots",
            "stale"
        ]
    );
    assert_eq!(list["ok"], true);
    assert_eq!(list["ownerlessStaleAfterSeconds"], 86400);
    let mut expected = vec![
        s(".point-work-REUSED", "stale", "pid_reused"),
        s(".tiles-mesh-GONE01", "stale", "owner_exited"),
        s(".vector-work-YOUNG0", "unverified", "no_owner_record"),
        s("rusty-tiles-spill-GONE00", "stale", "owner_exited"),
    ];
    if cfg!(target_os = "linux") {
        expected.push(s("rusty-tiles-spill-LIVE00", "live", "owner_running"));
    }
    assert_eq!(states(&list, "folders"), expected);
    let spill = list["folders"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| {
            f["path"]
                .as_str()
                .unwrap()
                .ends_with("rusty-tiles-spill-GONE00")
        })
        .unwrap();
    assert_eq!(
        keys(spill),
        [
            "bytes",
            "kind",
            "modifiedSecondsAgo",
            "owner",
            "ownerRecord",
            "path",
            "reason",
            "sizeComplete",
            "state"
        ]
    );
    assert_eq!(spill["kind"], "spill");
    assert_eq!(spill["ownerRecord"], "present");
    assert_eq!(spill["owner"]["pid"], NO_SUCH_PID);
    assert_eq!(spill["owner"]["command"], "rusty-tiles validate big.3tz");
    let owner_len = fs::metadata(gone.join("rusty-tiles-owner.json"))
        .unwrap()
        .len();
    assert_eq!(spill["bytes"].as_u64().unwrap(), 5000 + owner_len);
    assert!(spill["modifiedSecondsAgo"].as_u64().unwrap() < 3600);
    // Default locations are listed, then --dir.
    let roots: Vec<_> = list["roots"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["path"].clone())
        .collect();
    assert!(
        roots.contains(&json!(env.tmp.to_str().unwrap())),
        "{roots:?}"
    );
    assert!(
        roots.contains(&json!(scratch.to_str().unwrap())),
        "{roots:?}"
    );
    assert_eq!(
        list["roots"].as_array().unwrap().last().unwrap()["explicit"],
        true
    );
    let stale_bytes: u64 = list["folders"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["state"] == "stale")
        .map(|f| f["bytes"].as_u64().unwrap())
        .sum();
    assert_eq!(list["stale"], json!({"folders": 3, "bytes": stale_bytes}));

    let (code, human, _) = run(&env, &["scratch", "list", "--dir", extra]);
    assert_eq!(code, Some(0));
    assert!(human.contains("rusty-tiles-spill-GONE00"), "{human}");
    assert!(human.contains("3 stale"), "{human}");
    assert!(human.contains("rusty-tiles scratch clean"), "{human}");

    // doctor reports the default locations only and stays informational.
    let (code, doctor) = json_run(&env, &["doctor", "--json", "--command", "validate"]);
    assert_eq!(code, Some(0), "{doctor}");
    assert_eq!(
        keys(&doctor["scratch"]),
        [
            "clean",
            "folders",
            "liveFolders",
            "roots",
            "staleBytes",
            "staleFolders"
        ]
    );
    assert_eq!(doctor["scratch"]["staleFolders"], 2);
    assert_eq!(doctor["scratch"]["clean"], "rusty-tiles scratch clean");
    assert!(doctor["probes"]
        .as_array()
        .unwrap()
        .contains(&json!("scratch-folders")));
    let (_, human, _) = run(&env, &["doctor", "--command", "validate"]);
    assert!(human.contains("scratch: 2 stale folders"), "{human}");

    let (code, dry) = json_run(
        &env,
        &["scratch", "clean", "--dry-run", "--json", "--dir", extra],
    );
    assert_eq!(code, Some(0), "{dry}");
    assert_eq!(dry["dryRun"], true);
    assert_eq!(dry["bytesFreed"], stale_bytes);
    assert!(gone.exists());

    // --older-than: nothing here is a day old.
    let (code, young) = json_run(
        &env,
        &[
            "scratch",
            "clean",
            "--older-than",
            "1d",
            "--json",
            "--dir",
            extra,
        ],
    );
    assert_eq!(code, Some(0), "{young}");
    assert_eq!(young["removed"], json!([]));
    assert_eq!(young["olderThanSeconds"], 86400);
    assert!(gone.exists());

    let (code, clean) = json_run(&env, &["scratch", "clean", "--json", "--dir", extra]);
    assert_eq!(code, Some(0), "{clean}");
    assert_eq!(
        keys(&clean),
        [
            "bytesFreed",
            "dryRun",
            "failed",
            "kept",
            "ok",
            "olderThanSeconds",
            "ownerlessStaleAfterSeconds",
            "removed",
            "roots"
        ]
    );
    assert_eq!(clean["ok"], true);
    assert_eq!(clean["bytesFreed"], stale_bytes);
    assert_eq!(clean["olderThanSeconds"], Value::Null);
    assert_eq!(
        states(&clean, "removed"),
        [
            s(".point-work-REUSED", "stale", "pid_reused"),
            s(".tiles-mesh-GONE01", "stale", "owner_exited"),
            s("rusty-tiles-spill-GONE00", "stale", "owner_exited"),
        ]
    );
    assert!(!gone.exists());
    assert!(env.extra.join(".vector-work-YOUNG0").exists());
    #[cfg(target_os = "linux")]
    assert!(env.extra.join("rusty-tiles-spill-LIVE00").exists());
    for name in [
        "rusty-tiles-spill-TOOLONG7",
        ".tiles-dir-ABCDEF",
        "keep",
        ".mesh-work-AFILE0",
    ] {
        assert!(env.extra.join(name).exists(), "{name}");
    }
    assert_eq!(
        fs::read(env.outside.join("precious.bin")).unwrap().len(),
        2048
    );
    #[cfg(unix)]
    {
        assert!(fs::symlink_metadata(env.extra.join("rusty-tiles-spill-LINK00")).is_ok());
        assert!(env
            .outside
            .join("rusty-tiles-spill-DECOY0")
            .join("sub")
            .exists());
    }
    let (_, doctor) = json_run(&env, &["doctor", "--json", "--command", "validate"]);
    assert_eq!(doctor["scratch"]["staleFolders"], 0);

    // An ownerless folder is stale once the threshold has passed; --older-than 0 sets it.
    let (_, ownerless) = json_run(
        &env,
        &[
            "scratch",
            "clean",
            "--older-than",
            "0",
            "--json",
            "--dir",
            extra,
        ],
    );
    assert_eq!(
        states(&ownerless, "removed"),
        [s(".vector-work-YOUNG0", "stale", "no_owner_record")]
    );
}

#[test]
fn scratch_refuses_bad_arguments() {
    let env = env();
    let missing = env.extra.join("missing");
    let (code, value) = json_run(
        &env,
        &[
            "scratch",
            "list",
            "--json",
            "--dir",
            missing.to_str().unwrap(),
        ],
    );
    assert_eq!(code, Some(3), "{value}");
    assert_eq!(value["error"]["code"], "data");
    for bad in ["1y", "-1d", "soon", ""] {
        let (code, _, _) = run(&env, &["scratch", "clean", "--older-than", bad]);
        assert_eq!(code, Some(2), "{bad}");
    }
    let (code, _, _) = run(&env, &["scratch"]);
    assert_eq!(code, Some(2));
}
