//! The H8 producer publishes leaf GLBs and diagnostic manifests, without a hierarchy.
use super::super::{
    config::{self, Jobs, Profile, TreeConfig, TILE_BYTES},
    full,
    records::NodeKind,
};
use super::{atlas, writer};
use crate::{
    runtime::directory::DirectoryTarget, CleanupDiagnostic, JobError, JobErrorKind, JobFailure,
    OutputPolicy, RunControl, RunEvent,
};
use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{BufWriter, Write},
    num::NonZeroUsize,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LeafProfile {
    Fast,
    Balanced,
    Quality,
    Lossless,
}
impl From<LeafProfile> for Profile {
    fn from(value: LeafProfile) -> Self {
        match value {
            LeafProfile::Fast => Self::Fast,
            LeafProfile::Balanced => Self::Balanced,
            LeafProfile::Quality => Self::Quality,
            LeafProfile::Lossless => Self::Lossless,
        }
    }
}
#[derive(Clone, Debug)]
pub struct LeafOutputRequest {
    input: PathBuf,
    output: PathBuf,
    profile: LeafProfile,
    jobs: Jobs,
    scratch_dir: Option<PathBuf>,
}
impl LeafOutputRequest {
    pub fn new(
        input: impl Into<PathBuf>,
        output: impl Into<PathBuf>,
        profile: LeafProfile,
    ) -> Self {
        Self {
            input: input.into(),
            output: output.into(),
            profile,
            jobs: Jobs::Auto,
            scratch_dir: None,
        }
    }
    pub fn with_jobs(mut self, jobs: NonZeroUsize) -> Self {
        self.jobs = Jobs::Fixed(jobs);
        self
    }
    pub fn with_scratch_dir(mut self, path: impl Into<PathBuf>) -> Self {
        self.scratch_dir = Some(path.into());
        self
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct LeafOutputReport {
    pub schema_version: u32,
    pub profile: LeafProfile,
    pub triangles: u64,
    pub leaves: u64,
    pub glb_bytes: u64,
    pub maximum_glb_bytes: u64,
    pub maximum_geometry_error: f64,
    pub density_reduced_leaves: u64,
}
#[derive(Debug)]
pub struct LeafOutputResult {
    pub output: PathBuf,
    pub report: LeafOutputReport,
    pub cleanup_diagnostics: Vec<CleanupDiagnostic>,
}
fn invariant(message: &str) -> JobError {
    JobError::new(JobErrorKind::InvalidState, message)
}
fn resource(message: &str) -> JobError {
    JobError::new(JobErrorKind::ResourceLimit, message)
}
fn close_failed_capture(captured: full::FullCaptured, failure: &mut JobFailure) {
    let path = captured.scratch.path().to_owned();
    if let Err(error) = captured.close() {
        failure.secondary.push(error);
        if path.exists() {
            failure.retained_paths.push(path);
        }
    }
}
fn hex(bytes: impl AsRef<[u8]>) -> String {
    bytes.as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

/// Serial delivery is an explicit stage policy. Capture may use the requested jobs.
/// Reserve the largest supported page lease plus source/geometry/encoded closures.
fn stage_memory(profile: Profile, captured: &full::FullCaptured) -> Result<u64, JobError> {
    let edge: u64 = match profile {
        Profile::Fast => 2048,
        Profile::Lossless => 8192,
        _ => 4096,
    };
    let pixels = edge
        .checked_mul(edge)
        .ok_or_else(|| resource("leaf page memory overflow"))?;
    let texture = if profile == Profile::Lossless {
        pixels.checked_mul(40)
    } else {
        pixels.checked_mul(176)
    }
    .ok_or_else(|| resource("leaf texture workspace overflow"))?;
    let global = captured
        .report
        .memory
        .owned_metadata_bytes
        .checked_add(captured.report.memory.geometry.document_bytes)
        .ok_or_else(|| resource("leaf global ownership overflow"))?;
    let mut leaf_owned = 0;
    for plan in captured.leaf_plans.reader()? {
        leaf_owned = leaf_owned.max(plan?.owned_bytes);
    }
    let working = texture
        .checked_add(512 << 20)
        // Delivery retains independently decoded levels. MIP bindings also
        // retain a bounded original source class chain and its decode owner.
        .and_then(|n| {
            n.checked_add(if profile == Profile::Lossless {
                0
            } else {
                128 << 20
            })
        })
        .and_then(|n| {
            n.checked_add(if profile == Profile::Lossless {
                0
            } else {
                pixels.checked_mul(6)?
            })
        })
        .and_then(|n| n.checked_add(global))
        .and_then(|n| n.checked_add(leaf_owned))
        .ok_or_else(|| resource("leaf workspace overflow"))?;
    let tmpfs = if captured.scratch.memory_backed() {
        captured.report.scratch.need_bytes
    } else {
        0
    };
    if config::available_memory()
        .is_some_and(|available| working.checked_add(tmpfs).is_none_or(|n| n > available))
    {
        return Err(resource(
            "one leaf job cannot fit available memory including owned scratch",
        ));
    }
    Ok(working)
}
fn write_file(path: &Path, bytes: &[u8]) -> Result<(), JobError> {
    let mut file = File::create(path).map_err(|e| JobError::io("create leaf member", path, e))?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| JobError::io("write leaf member", path, e))
}
fn produce(
    path: &Path,
    captured: &full::FullCaptured,
    config: &TreeConfig,
    memory: u64,
    profile: LeafProfile,
    check: &mut dyn FnMut() -> Result<(), JobError>,
) -> Result<LeafOutputReport, JobError> {
    let leaves = path.join("leaves");
    fs::create_dir(&leaves).map_err(|e| JobError::io("create leaf directory", &leaves, e))?;
    let manifest_path = path.join("leaves.jsonl");
    let file = File::create(&manifest_path)
        .map_err(|e| JobError::io("create leaf manifest", &manifest_path, e))?;
    let mut manifest = BufWriter::new(file);
    let mut triangles = captured.triangles.reader()?;
    let mut depths = captured.corner_depths.reader()?;
    let mut plans = captured.leaf_plans.reader()?;
    let mut report = LeafOutputReport {
        schema_version: 1,
        profile,
        triangles: 0,
        leaves: 0,
        glb_bytes: 0,
        maximum_glb_bytes: 0,
        maximum_geometry_error: 0.,
        density_reduced_leaves: 0,
    };
    for node in captured.tree.nodes.reader()? {
        let node = node?;
        if node.kind != NodeKind::Leaf {
            continue;
        }
        check()?;
        crate::spill::preflight(
            &leaves,
            TILE_BYTES + (1 << 20),
            crate::spill::SpaceRole::Output,
            crate::spill::PreflightStage::Texture,
        )?;
        if node.first != report.triangles || node.triangles() > config.leaf_triangles() {
            return Err(invariant(
                "leaf ranges or triangle limit differ from frozen tree",
            ));
        }
        let count =
            usize::try_from(node.triangles()).map_err(|_| resource("leaf count overflow"))?;
        let mut records = Vec::new();
        records
            .try_reserve_exact(count)
            .map_err(|_| resource("leaf triangle allocation"))?;
        let mut corner_depths = Vec::new();
        corner_depths
            .try_reserve_exact(count)
            .map_err(|_| resource("leaf depth allocation"))?;
        for i in 0..count {
            if i.is_multiple_of(4096) {
                check()?;
            }
            records.push(
                triangles
                    .next()
                    .transpose()?
                    .ok_or_else(|| invariant("truncated full leaf triangles"))?,
            );
            corner_depths.push(
                depths
                    .next()
                    .transpose()?
                    .ok_or_else(|| invariant("truncated leaf corner depths"))?
                    .depths,
            );
        }
        let plan = plans
            .next()
            .transpose()?
            .ok_or_else(|| invariant("truncated frozen leaf plans"))?;
        if plan.first != node.first || plan.end != node.end || u32::from(plan.depth) != node.depth()
        {
            return Err(invariant("leaf payload plan differs from frozen tree"));
        }
        let mut density_drop = u32::from(plan.density_drop);
        let mut density_reasons = Vec::new();
        let (encoded, appearance) = loop {
            check()?;
            let attempt = atlas::prepare(
                (records.clone(), corner_depths.clone()),
                captured,
                config.profile,
                node.depth() as u8,
                density_drop,
                memory,
                check,
            )
            .and_then(|(atlas, receipt)| {
                writer::encode(&atlas, &captured.lattice, node.depth() as u8, TILE_BYTES)
                    .map_err(|error| match error {
                        writer::WriteError::Budget(e) => atlas::PrepareError::Budget(e),
                        writer::WriteError::Failure(e) => atlas::PrepareError::Failure(e),
                    })
                    .map(|leaf| (leaf, receipt))
            });
            match attempt {
                Ok(leaf) => break leaf,
                Err(atlas::PrepareError::Budget(e))
                    if config.profile != Profile::Lossless && density_drop < 12 =>
                {
                    density_reasons
                        .push(json!({"densityDrop":density_drop,"reason":e.to_string()}));
                    density_drop += 1;
                }
                Err(atlas::PrepareError::Budget(e) | atlas::PrepareError::Failure(e)) => {
                    return Err(e)
                }
            }
        };
        let name = format!("{:012}.glb", report.leaves);
        let member = leaves.join(&name);
        write_file(&member, &encoded.glb)?;
        let receipt = json!({"schemaVersion":1,"uri":format!("leaves/{name}"),"sha256":hex(Sha256::digest(&encoded.glb)),"bytes":encoded.glb.len(),"first":node.first,"end":node.end,"cellDepth":node.cell_depth,"cell":node.cell,"pathLength":node.path_len,"path":node.path,"lastOctant":node.last_octant,"depth":node.depth(),"packed":node.is_packed(),"bounds":{"min":encoded.bounds.min.map(f64::from),"max":encoded.bounds.max.map(f64::from)},"geometryError":encoded.geometry_error,"densityDrop":density_drop,"densityReasons":density_reasons,"plannedFootprint":{"densityDrop":plan.density_drop,"pages":plan.pages,"baseTexels":plan.base_texels,"chainTexels":plan.chain_texels,"geometryBytes":plan.geometry_bytes,"planningOwnedBytes":plan.owned_bytes},"appearance":appearance});
        serde_json::to_writer(&mut manifest, &receipt)
            .map_err(|e| invariant(&format!("leaf manifest serialization: {e}")))?;
        manifest
            .write_all(b"\n")
            .map_err(|e| JobError::io("write leaf manifest", &manifest_path, e))?;
        let bytes = encoded.glb.len() as u64;
        report.triangles = report
            .triangles
            .checked_add(node.triangles())
            .ok_or_else(|| resource("triangle report overflow"))?;
        report.leaves += 1;
        report.glb_bytes = report
            .glb_bytes
            .checked_add(bytes)
            .ok_or_else(|| resource("leaf output size overflow"))?;
        report.maximum_glb_bytes = report.maximum_glb_bytes.max(bytes);
        report.maximum_geometry_error = report.maximum_geometry_error.max(encoded.geometry_error);
        report.density_reduced_leaves += u64::from(density_drop != 0);
    }
    if report.triangles != captured.scan.triangles
        || report.leaves != captured.tree.stats.leaves
        || triangles.next().transpose()?.is_some()
        || depths.next().transpose()?.is_some()
        || plans.next().transpose()?.is_some()
    {
        return Err(invariant(
            "full leaf output does not cover captured source exactly",
        ));
    }
    manifest
        .flush()
        .and_then(|_| manifest.get_ref().sync_all())
        .map_err(|e| JobError::io("finish leaf manifest", &manifest_path, e))?;
    let doc = json!({"schemaVersion":1,"kind":"full_detail_leaf_payloads","report":report,"lattice":captured.lattice,"provenanceSchema":"rusty_tiles_large_source_v1","leafManifest":"leaves.jsonl","losslessMipGuarantee":"PNG level0 exact; viewer-generated minification levels outside guarantee"});
    write_file(
        &path.join("manifest.json"),
        &serde_json::to_vec_pretty(&doc).map_err(|e| invariant(&e.to_string()))?,
    )?;
    let resources = json!({"capture":captured.report,"deliveryJobs":1,"declaredStage2MemoryBytes":memory,"nativeWorkspace":"conservative provisional charge; measured scale acceptance recorded separately"});
    write_file(
        &path.join("resources.json"),
        &serde_json::to_vec_pretty(&resources).map_err(|e| invariant(&e.to_string()))?,
    )?;
    Ok(report)
}

pub fn mesh_to_leaf_payloads(
    request: LeafOutputRequest,
    control: &RunControl,
) -> Result<LeafOutputResult, JobFailure> {
    let attempt = control.begin()?;
    let target = DirectoryTarget::prepare(&request.output, OutputPolicy::CreateNew)
        .map_err(|e| attempt.fail(e))?;
    let mut config = TreeConfig::new(request.profile.into());
    config.jobs = request.jobs;
    config.scratch_dir = request.scratch_dir;
    let captured = match full::capture(&request.input, &config, &mut || attempt.check()) {
        Ok(captured) => captured,
        Err(failure) => {
            let mut finished = attempt.fail(failure.error);
            finished.secondary.extend(failure.secondary);
            finished.retained_paths.extend(failure.retained_paths);
            finished.recovery = failure.recovery;
            return Err(finished);
        }
    };
    let memory = match stage_memory(config.profile, &captured) {
        Ok(memory) => memory,
        Err(error) => {
            let mut failed = attempt.fail(error);
            close_failed_capture(captured, &mut failed);
            return Err(failed);
        }
    };
    let stage = match target.stage_pending(&attempt) {
        Ok(stage) => stage,
        Err(error) => {
            let mut failed = attempt.fail(error);
            close_failed_capture(captured, &mut failed);
            return Err(failed);
        }
    };
    let result = produce(
        stage.path(),
        &captured,
        &config,
        memory,
        request.profile,
        &mut || attempt.check(),
    );
    let cleanup_path = captured.scratch.path().to_owned();
    let cleanup = captured.close();
    let report = match result {
        Ok(report) => report,
        Err(error) => {
            let mut failed = stage.fail(error);
            if let Err(error) = cleanup {
                failed.secondary.push(error);
                if cleanup_path.exists() {
                    failed.retained_paths.push(cleanup_path);
                }
            }
            return Err(failed);
        }
    };
    if let Err(error) = cleanup {
        let mut failed = stage.fail(error);
        if cleanup_path.exists() {
            failed.retained_paths.push(cleanup_path);
        }
        return Err(failed);
    }
    if let Err(error) = attempt
        .emit(&RunEvent::Progress {
            phase: "mesh_leaf_payloads",
            done: report.leaves,
            total: Some(report.leaves),
        })
        .and_then(|_| attempt.close_events())
    {
        return Err(stage.fail(error));
    }
    let published = stage.seal()?.publish()?;
    Ok(LeafOutputResult {
        output: published.output,
        report,
        cleanup_diagnostics: published.cleanup_diagnostics,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn real_leaf_output_all_profiles_and_exact_member_limit() {
        let work = tempfile::tempdir().unwrap();
        let input = full::tests::fixture(work.path(), 2);
        for (name, profile) in [
            ("fast", LeafProfile::Fast),
            ("balanced", LeafProfile::Balanced),
            ("quality", LeafProfile::Quality),
            ("lossless", LeafProfile::Lossless),
        ] {
            let output = work.path().join(name);
            let result = mesh_to_leaf_payloads(
                LeafOutputRequest::new(&input, &output, profile)
                    .with_jobs(NonZeroUsize::new(1).unwrap()),
                &RunControl::default(),
            )
            .unwrap();
            assert_eq!(result.report.triangles, 2);
            assert_eq!(result.report.leaves, 1);
            assert!(result.report.maximum_glb_bytes <= TILE_BYTES);
            let manifest: serde_json::Value =
                serde_json::from_slice(&fs::read(output.join("manifest.json")).unwrap()).unwrap();
            assert_eq!(manifest["kind"], "full_detail_leaf_payloads");
            assert!(!output.join("tileset.json").exists());
            let leaf = fs::read(output.join("leaves/000000000000.glb")).unwrap();
            assert_eq!(&leaf[..4], b"glTF");
            assert_eq!(
                u32::from_le_bytes(leaf[8..12].try_into().unwrap()) as usize,
                leaf.len()
            );
        }
    }
    #[test]
    fn synthetic_depth_payloads_are_identical_across_jobs_and_paths() {
        let work = tempfile::tempdir().unwrap();
        let input = full::tests::fixture(work.path(), 8193);
        let mut previous = None;
        for jobs in [1, 2] {
            let scratch = work.path().join(format!("scratch{jobs}"));
            fs::create_dir(&scratch).unwrap();
            let output = work.path().join(format!("output{jobs}"));
            let result = mesh_to_leaf_payloads(
                LeafOutputRequest::new(&input, &output, LeafProfile::Lossless)
                    .with_jobs(NonZeroUsize::new(jobs).unwrap())
                    .with_scratch_dir(&scratch),
                &RunControl::default(),
            )
            .unwrap();
            assert_eq!(result.report.triangles, 8193);
            assert!(result.report.leaves >= 2);
            assert!(fs::read_dir(scratch).unwrap().next().is_none());
            let rows = fs::read_to_string(output.join("leaves.jsonl")).unwrap();
            let mut files = vec![
                fs::read(output.join("manifest.json")).unwrap(),
                rows.as_bytes().to_vec(),
            ];
            for row in rows.lines() {
                let row: serde_json::Value = serde_json::from_str(row).unwrap();
                assert!(row["depth"].as_u64().unwrap() > 21);
                files.push(fs::read(output.join(row["uri"].as_str().unwrap())).unwrap());
            }
            if let Some(ref prior) = previous {
                assert_eq!(prior, &files);
            } else {
                previous = Some(files);
            }
        }
    }
    #[test]
    fn cancelled_output_does_not_create_a_private_or_public_tree() {
        let work = tempfile::tempdir().unwrap();
        let input = full::tests::fixture(work.path(), 1);
        let output = work.path().join("cancelled");
        let control = RunControl::default();
        control.cancellation_handle().cancel();
        let failure = mesh_to_leaf_payloads(
            LeafOutputRequest::new(input, &output, LeafProfile::Lossless),
            &control,
        )
        .unwrap_err();
        assert_eq!(failure.error.kind(), JobErrorKind::Cancelled);
        assert!(!output.exists());
        assert!(!fs::read_dir(work.path()).unwrap().any(|entry| entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".tiles-dir-")));
    }
}
