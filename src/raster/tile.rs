//! Direct per-zoom tile calls: Run, Finalize, Release, then next zoom.
use super::{
    grid::GridPlan,
    native::{self, Args, Progress},
};
use crate::{runtime::Attempt, JobFailure};
use std::{
    ffi::{c_char, c_void},
    path::Path,
};
unsafe extern "C" {
    fn GDALGetGlobalAlgorithmRegistry() -> *mut c_void;
    fn GDALAlgorithmRegistryRelease(registry: *mut c_void);
    fn GDALAlgorithmRegistryInstantiateAlg(
        registry: *mut c_void,
        name: *const c_char,
    ) -> *mut c_void;
    fn GDALAlgorithmInstantiateSubAlgorithm(
        algorithm: *mut c_void,
        name: *const c_char,
    ) -> *mut c_void;
    fn GDALAlgorithmRelease(algorithm: *mut c_void);
    fn GDALAlgorithmParseCommandLineArguments(
        algorithm: *mut c_void,
        arguments: *const *const c_char,
    ) -> bool;
    fn GDALAlgorithmRun(
        algorithm: *mut c_void,
        progress: gdal_sys::GDALProgressFunc,
        data: *mut c_void,
    ) -> bool;
    fn GDALAlgorithmFinalize(algorithm: *mut c_void) -> bool;
}
struct Algorithm {
    leaf: Option<*mut c_void>,
    parent: Option<*mut c_void>,
}
impl Algorithm {
    fn new() -> Result<Self, JobFailure> {
        unsafe {
            let registry = GDALGetGlobalAlgorithmRegistry();
            if registry.is_null() {
                return Err(native::failure(native::error(
                    "native algorithm registry unavailable",
                )));
            }
            let parent = GDALAlgorithmRegistryInstantiateAlg(registry, c"raster".as_ptr());
            GDALAlgorithmRegistryRelease(registry);
            if parent.is_null() {
                return Err(native::failure(native::error(
                    "native raster algorithms unavailable",
                )));
            }
            let leaf = GDALAlgorithmInstantiateSubAlgorithm(parent, c"tile".as_ptr());
            let algorithm = Self {
                leaf: (!leaf.is_null()).then_some(leaf),
                parent: Some(parent),
            };
            if leaf.is_null() {
                let mut failure =
                    native::failure(native::error("native tile algorithm unavailable"));
                if let Err(closed) = algorithm.finish() {
                    failure.secondary.push(closed.error);
                    failure.secondary.extend(closed.secondary);
                }
                return Err(failure);
            }
            Ok(algorithm)
        }
    }
    fn raw(&self) -> *mut c_void {
        self.leaf.expect("live tile algorithm")
    }
    fn finish(mut self) -> Result<(), JobFailure> {
        let mut failure: Option<JobFailure> = None;
        for slot in [&mut self.leaf, &mut self.parent] {
            if let Some(handle) = slot.take() {
                let finished = unsafe { GDALAlgorithmFinalize(handle) };
                if !finished {
                    let error = native::error("finalize raster tile algorithm");
                    if let Some(failure) = &mut failure {
                        failure.secondary.push(error);
                    } else {
                        failure = Some(native::failure(error));
                    }
                }
                unsafe { GDALAlgorithmRelease(handle) };
            }
        }
        failure.map_or(Ok(()), Err)
    }
}
impl Drop for Algorithm {
    fn drop(&mut self) {
        for slot in [&mut self.leaf, &mut self.parent] {
            if let Some(handle) = slot.take() {
                unsafe {
                    GDALAlgorithmFinalize(handle);
                    GDALAlgorithmRelease(handle);
                }
            }
        }
    }
}
pub(super) fn owner_bytes() -> usize {
    std::mem::size_of::<Algorithm>()
}
pub(super) fn available() -> Result<bool, JobFailure> {
    let a = match Algorithm::new() {
        Ok(a) => a,
        Err(f) if f.error.kind() == crate::JobErrorKind::Unsupported && f.secondary.is_empty() => {
            return Ok(false);
        }
        Err(f) => return Err(f),
    };
    a.finish()?;
    Ok(true)
}
pub(super) fn run(
    input: &Path,
    output: &Path,
    grid: &GridPlan,
    workers: u8,
    attempt: &Attempt,
    stage: &Path,
    working_cap: u64,
) -> Result<(), JobFailure> {
    let input = input.to_str().ok_or_else(|| {
        native::failure(super::source::unsupported(
            "tile input requires native UTF8 path",
        ))
    })?;
    let output = output.to_str().ok_or_else(|| {
        native::failure(super::source::unsupported(
            "tile output requires native UTF8 path",
        ))
    })?;
    for r in grid.rectangles() {
        attempt.check().map_err(native::failure)?;
        let algorithm = Algorithm::new()?;
        let result = (|| {
            let args = Args::new(&[
                "--webviewer=none".into(),
                "--tiling-scheme=WebMercatorQuad".into(),
                "--convention=xyz".into(),
                "--output-format=PNG".into(),
                "--parallel-method=thread".into(),
                "--resampling=nearest".into(),
                format!("--num-threads={workers}"),
                format!("--min-zoom={}", r.z),
                format!("--max-zoom={}", r.z),
                format!("--min-x={}", r.x0),
                format!("--max-x={}", r.x1),
                format!("--min-y={}", r.y0),
                format!("--max-y={}", r.y1),
                input.into(),
                output.into(),
            ])?;
            if !unsafe { GDALAlgorithmParseCommandLineArguments(algorithm.raw(), args.const_ptr()) }
            {
                return Err(native::error("parse tile arguments"));
            }
            let progress = Progress::new(attempt, "tiling");
            let ok = unsafe {
                GDALAlgorithmRun(
                    algorithm.raw(),
                    Some(native::callback),
                    std::ptr::from_ref(&progress).cast_mut().cast(),
                )
            };
            progress.finish(ok)
        })();
        let final_result = algorithm.finish();
        match (result, final_result) {
            (Err(error), final_result) => {
                let mut failure = native::failure(error);
                if let Err(closed) = final_result {
                    failure.secondary.push(closed.error);
                    failure.secondary.extend(closed.secondary);
                }
                return Err(failure);
            }
            (Ok(()), Err(failure)) => return Err(failure),
            (Ok(()), Ok(())) => {}
        }
        super::check_closed_workspace(stage, grid.total_tiles(), working_cap)
            .map_err(native::failure)?;
    }
    Ok(())
}
