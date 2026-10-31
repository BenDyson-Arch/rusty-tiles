//! Opt-in real-data acceptance and release benchmark driver. Downloads nothing.
#[path = "demodata/runner.rs"]
mod runner;

use clap::Parser;
use std::path::PathBuf;

#[derive(Parser)]
#[command(about = "Test or benchmark the pinned, openly licensed demodata corpus")]
struct Args {
    /// Existing data directory, normally ../demodata. Sources are read-only.
    #[arg(long)]
    data_root: PathBuf,
    /// Absolute or relative native CLI binary to exercise.
    #[arg(long)]
    bin: Option<PathBuf>,
    /// Optional release CLI to compare with the current binary.
    #[arg(long)]
    baseline_bin: Option<PathBuf>,
    /// Private output directory; must not exist or overlap the input corpus.
    #[arg(long)]
    output: Option<PathBuf>,
    /// smoke, core (includes smoke), scale, or all.
    #[arg(long, default_value = "smoke", value_parser = ["smoke", "core", "scale", "all"])]
    profile: String,
    /// Select named cases instead of a profile. Repeat for several cases.
    #[arg(long)]
    case: Vec<String>,
    /// Run one warmup and at least three rotated measured repetitions.
    #[arg(long)]
    benchmark: bool,
    #[arg(long, default_value_t = 3)]
    repeats: usize,
    /// Print recipes and the reference fixture inventory, without running tools.
    #[arg(long)]
    list: bool,
    /// Check every pinned corpus hash; no CLI binary or output is needed.
    #[arg(long)]
    verify: bool,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let raw: Vec<_> = std::env::args().collect();
    if raw.get(1).is_some_and(|arg| arg == "__measure") {
        let sample = runner::measure_process(&raw[3..], std::path::Path::new(&raw[2]))?;
        println!("{}", serde_json::to_string(&sample)?);
        return Ok(());
    }
    let args = Args::parse();
    let manifest = runner::Manifest::load()?;
    if args.list {
        println!("{}", serde_json::to_string_pretty(&manifest)?);
        return Ok(());
    }
    if args.verify {
        let verified = runner::verify_all(&manifest, &args.data_root)?;
        println!(
            "{}",
            serde_json::json!({"ok":true,"verifiedFiles":verified})
        );
        return Ok(());
    }
    let bin = args
        .bin
        .ok_or("--bin is required for acceptance or benchmarking")?;
    let output = args
        .output
        .ok_or("--output is required for acceptance or benchmarking")?;
    runner::run(
        &manifest,
        runner::Options {
            data_root: args.data_root,
            binary: bin,
            baseline: args.baseline_bin,
            output,
            profile: args.profile,
            cases: args.case,
            benchmark: args.benchmark,
            repeats: args.repeats,
        },
    )?;
    Ok(())
}
