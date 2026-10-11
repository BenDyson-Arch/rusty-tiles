//! Read-only readiness for explicitly selected commands.
//!
//! Each command resolves to `ready`, `unavailable` (compiled in, but a runtime
//! probe failed) or `unsupported` (not in this build). Every probe that ran is
//! named in [`Report::probes`]. See `docs/architecture/t1-tooling-contract.md`.
use crate::JobErrorKind;
use serde::{Serialize, Serializer};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};

/// A subcommand with a readiness entry. The CLI parser takes `doctor
/// --command` values from [`Command::ALL`], and a CLI unit test checks them
/// against the clap subcommand definitions so the two cannot drift.
/// `convert-to-implicit` is retired for 0.4.0 and has no entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Command {
    Vector,
    Raster,
    RasterTileToDirectory,
    Terrain,
    PointCloud,
    MeshTo3tz,
    MeshLocalTo3tz,
    GlbTo3tz,
    CreateTilesetJson,
    Convert,
    Validate,
    Preview,
}

impl Command {
    pub const ALL: [Command; 12] = [
        Self::Vector,
        Self::Raster,
        Self::RasterTileToDirectory,
        Self::Terrain,
        Self::PointCloud,
        Self::MeshTo3tz,
        Self::MeshLocalTo3tz,
        Self::GlbTo3tz,
        Self::CreateTilesetJson,
        Self::Convert,
        Self::Validate,
        Self::Preview,
    ];

    /// Canonical CLI subcommand name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Vector => "vector",
            Self::Raster => "raster",
            Self::RasterTileToDirectory => "raster-tile-to-directory",
            Self::Terrain => "terrain",
            Self::PointCloud => "point-cloud",
            Self::MeshTo3tz => "mesh-to-3tz",
            Self::MeshLocalTo3tz => "mesh-local-to-3tz",
            Self::GlbTo3tz => "glb-to-3tz",
            Self::CreateTilesetJson => "createTilesetJson",
            Self::Convert => "convert",
            Self::Validate => "validate",
            Self::Preview => "preview",
        }
    }

    /// Accepted CLI aliases of [`Command::name`].
    pub const fn aliases(self) -> &'static [&'static str] {
        match self {
            Self::MeshTo3tz => &["meshTo3tz"],
            Self::GlbTo3tz => &["glbTo3tz"],
            Self::CreateTilesetJson => &["create-tileset-json"],
            _ => &[],
        }
    }

    /// The command for a canonical name or alias.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|command| command.name() == name || command.aliases().contains(&name))
    }
}

impl Serialize for Command {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.name())
    }
}

/// Readiness of one capability. `Unsupported` needs a different build;
/// `Unavailable` is a runtime dependency failure in a build that has it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Ready,
    Unavailable,
    Unsupported,
}

/// A probe the report ran. Only these touch native libraries or the filesystem.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Probe {
    /// Linked GDAL/PROJ/GEOS version queries.
    LinkedVersions,
    /// Sets GDAL's process-wide PROJ network policy to disabled.
    ProjOfflinePolicy,
    /// Read-only EPSG:4326 lookup in the PROJ database.
    ProjDatabase,
    /// Non-recursive, capped listing of PROJ search directories.
    ProjDirectoryListing,
    /// Process-wide GDAL driver registration and driver lookups.
    GdalDriverRegistry,
    /// One in-memory GEOS triangle triangulation.
    GeosTriangulation,
    /// Stat of `Cesium.js` in the given runtime directory.
    CesiumRuntime,
}

/// A status with its reason when not ready. `refusal` is the failure kind
/// that the command's own execution check returns for the same cause; the
/// CLI presents it with the category and exit status of execution.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Capability {
    pub status: Status,
    pub ready: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip)]
    pub refusal: Option<JobErrorKind>,
}

impl Capability {
    fn ready() -> Self {
        Self {
            status: Status::Ready,
            ready: true,
            reason: None,
            refusal: None,
        }
    }
    fn not_ready(status: Status, refusal: JobErrorKind, reason: impl Into<String>) -> Self {
        Self {
            status,
            ready: false,
            reason: Some(reason.into()),
            refusal: Some(refusal),
        }
    }
    /// A runtime dependency failure classified by the check that found it.
    #[cfg(feature = "native-geospatial")]
    fn from_check(result: Result<(), crate::JobError>) -> Self {
        match result {
            Ok(()) => Self::ready(),
            Err(error) => Self::not_ready(Status::Unavailable, error.kind(), error.message()),
        }
    }
    /// Execution refuses commands missing from this build as unsupported
    /// (`raster.rs`, `terrain.rs`, `raster_directory.rs` portable branches).
    #[cfg(not(feature = "native-geospatial"))]
    fn unsupported() -> Self {
        Self::not_ready(
            Status::Unsupported,
            JobErrorKind::Unsupported,
            NATIVE_REBUILD,
        )
    }
    /// The first capability that is not ready, else ready.
    #[cfg_attr(not(feature = "native-geospatial"), allow(dead_code))]
    fn all<'a>(parts: impl IntoIterator<Item = &'a Capability>) -> Self {
        parts
            .into_iter()
            .find(|part| !part.ready)
            .cloned()
            .unwrap_or_else(Self::ready)
    }
}

#[cfg(not(feature = "native-geospatial"))]
const NATIVE_REBUILD: &str =
    "this build does not include native-geospatial; rebuild with --features native-geospatial";

/// Linked native GDAL/PROJ readiness with the versions actually linked.
#[derive(Clone, Debug, Serialize)]
pub struct NativeGeospatial {
    #[serde(flatten)]
    pub capability: Capability,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub versions: Option<Value>,
}

/// One selected command's readiness. `details` are descriptive data such as
/// CRS classes, limitations and drivers; they never change `status`.
#[derive(Clone, Debug, Serialize)]
pub struct Readiness {
    #[serde(flatten)]
    pub capability: Capability,
    pub backend: &'static str,
    pub requires: Vec<&'static str>,
    #[serde(flatten)]
    pub details: Map<String, Value>,
}

/// Read-only local PROJ data inventory.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjInventory {
    pub database: Capability,
    pub data_directories: Vec<String>,
    pub available_grids: Vec<String>,
    pub inventory_errors: Vec<String>,
    pub network_enabled: bool,
    pub note: &'static str,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    /// All selected commands are ready.
    pub ready: bool,
    /// `unavailable` if any selected command is, else `unsupported` if any
    /// is, else `ready`.
    pub status: Status,
    pub selected_commands: Vec<Command>,
    /// Selected commands keyed by canonical name.
    pub commands: BTreeMap<&'static str, Readiness>,
    pub native_geospatial: NativeGeospatial,
    pub proj: ProjInventory,
    pub probes: BTreeSet<Probe>,
}

impl Report {
    /// The first selected command, in name order, that is not ready, with its
    /// capability. Its `refusal` is what that command's own execution returns.
    pub fn refusal(&self) -> Option<(&'static str, &Capability)> {
        self.commands
            .iter()
            .find(|(_, readiness)| !readiness.capability.ready)
            .map(|(name, readiness)| (*name, &readiness.capability))
    }
}

/// Top-level entries listed per PROJ directory before the listing is truncated.
const LISTING_LIMIT: usize = 4096;

/// Readiness of `selected` commands (all, in name order, when empty). Only
/// selected commands are resolved and listed. `cesium` is an optional preview
/// runtime directory to look for; nothing is checked without one.
pub fn report(selected: &[Command], cesium: Option<&Path>) -> Report {
    let mut selection: Vec<Command> = Vec::new();
    if selected.is_empty() {
        selection.extend(Command::ALL);
        selection.sort_by_key(|command| command.name());
    }
    for command in selected {
        if !selection.contains(command) {
            selection.push(*command);
        }
    }
    let mut probes = BTreeSet::new();
    let native = native_geospatial(&mut probes);
    let mut resolver = Resolver {
        native: &native,
        cesium,
        probes: &mut probes,
        #[cfg(feature = "native-geospatial")]
        drivers: None,
    };
    let commands: BTreeMap<_, _> = selection
        .iter()
        .map(|command| (command.name(), resolver.resolve(*command)))
        .collect();
    let status = [Status::Unavailable, Status::Unsupported]
        .into_iter()
        .find(|status| commands.values().any(|r| r.capability.status == *status))
        .unwrap_or(Status::Ready);
    let proj = proj_inventory(native.capability.clone(), &mut probes);
    Report {
        ready: status == Status::Ready,
        status,
        selected_commands: selection,
        commands,
        native_geospatial: native,
        proj,
        probes,
    }
}

fn native_geospatial(probes: &mut BTreeSet<Probe>) -> NativeGeospatial {
    #[cfg(feature = "native-geospatial")]
    {
        probes.insert(Probe::LinkedVersions);
        match crate::geospatial::versions() {
            Ok(versions) => {
                // Crs::from_definition first disables PROJ networking, then
                // checks the database with a read-only EPSG:4326 lookup.
                probes.extend([Probe::ProjOfflinePolicy, Probe::ProjDatabase]);
                let capability = match crate::geospatial::Crs::from_definition("EPSG:4326") {
                    Ok(_) => Capability::ready(),
                    Err(error) => native_failure(&error),
                };
                NativeGeospatial {
                    capability,
                    versions: serde_json::to_value(versions).ok(),
                }
            }
            Err(error) => NativeGeospatial {
                capability: native_failure(&error),
                versions: None,
            },
        }
    }
    #[cfg(not(feature = "native-geospatial"))]
    {
        let _ = probes;
        NativeGeospatial {
            capability: Capability::unsupported(),
            versions: None,
        }
    }
}

/// Producers classify native initialization and CRS-database failures
/// (`Error::Environment`) as unsupported: `vector.rs` `job_error`,
/// `raster/native.rs` `init`, `terrain/raster.rs` and `raster_directory/native.rs`
/// `native_error`. Those producers hit this top-level dependency during source
/// admission rather than through a separate check, so doctor applies their
/// shared classification here.
#[cfg(feature = "native-geospatial")]
fn native_failure(error: &crate::Error) -> Capability {
    let refusal = match error {
        crate::Error::Environment(_) => JobErrorKind::Unsupported,
        crate::Error::Io(_) => JobErrorKind::Io,
        _ => JobErrorKind::InvalidInput,
    };
    Capability::not_ready(Status::Unavailable, refusal, error.to_string())
}

struct Resolver<'a> {
    native: &'a NativeGeospatial,
    cesium: Option<&'a Path>,
    probes: &'a mut BTreeSet<Probe>,
    /// The raster owner's driver capability, probed once when first needed.
    #[cfg(feature = "native-geospatial")]
    drivers: Option<Result<crate::raster::RasterCapabilities, crate::JobError>>,
}

impl Resolver<'_> {
    /// The exhaustive match is the only place a command gains readiness.
    fn resolve(&mut self, command: Command) -> Readiness {
        match command {
            Command::MeshLocalTo3tz
            | Command::GlbTo3tz
            | Command::CreateTilesetJson
            | Command::Convert
            | Command::Validate => built_in(Map::new()),
            Command::MeshTo3tz => built_in(details(json!({
                "generalCrs": self.placement(),
                "note": "Local/manual placement and legacy geographic/Web Mercator adapters remain available. General mesh CRS uses the shared horizontal resolver and requires explicit xyz/y-up axes after node transforms and an ellipsoidal metre height offset; E/N shifts use horizontal units, A metres. No compound/geocentric input or manual placement/rotation on the general path."
            }))),
            Command::Preview => built_in(self.cesium()),
            Command::PointCloud => Readiness {
                capability: Capability::ready(),
                backend: "native LAS/LAZ",
                requires: vec![],
                details: details(json!({"reader":"native LAS/LAZ","local":Capability::ready(),
                    "geospatial":self.placement(),
                    "note":"Local XYZ and verified grid-free globe placement need no Python or GDAL. Conversion validates the source-specific operation."})),
            },
            Command::Vector => self.vector(),
            Command::Raster => self.raster(),
            Command::RasterTileToDirectory => {
                self.gtiff_product(
                    "GDAL RGB source reader / Rust PNG and directory publisher",
                    &["native-geospatial", "native GDAL >= 3.12 with GTiff", "supported local output filesystem"],
                    "Finite aligned RGB tile profile; destination filesystem support is checked per request.",
                )
            }
            Command::Terrain => self.gtiff_product(
                "native GDAL/Rust",
                &["native-geospatial", "native GDAL >= 3.12 with GTiff", "PROJ >= 9.2", "local PROJ database/grids"],
                "Bounded EPSG:4326 GeoTIFF elevation decoding to 3D Tiles 1.1; source admission is checked during conversion.",
            ),
        }
    }

    /// Informational only: preview always takes an explicit `--cesium`, so a
    /// missing runtime never makes the command unavailable.
    fn cesium(&mut self) -> Map<String, Value> {
        let Some(cesium) = self.cesium else {
            return Map::new();
        };
        self.probes.insert(Probe::CesiumRuntime);
        let found = cesium.join("Cesium.js").is_file();
        details(
            json!({"cesium":{"path":cesium.to_string_lossy(),"found":found,
            "note":if found {"Cesium IIFE runtime found; pass this directory to preview --cesium."}
                else {"No Cesium.js here. Install cesium@1.146.0 as described in the README and pass its Build/Cesium directory to preview --cesium."}}}),
        )
    }

    /// Shared pure-Rust horizontal placement with the native fallback status.
    fn placement(&self) -> Value {
        let classes = [
            "WGS84 geographic (EPSG:4326)",
            "WGS84 UTM north/south (EPSG:32601–32660/32701–32760)",
            "WGS84 Mercator (EPSG:3857/3395)",
            "supported local projections with an explicit WGS84 datum or 3/7-parameter Helmert shift",
        ];
        let tier = if cfg!(feature = "native-geospatial") {
            "pure Rust with strict native fallback"
        } else {
            "pure Rust"
        };
        json!({"status":Status::Ready,"ready":true,"backend":"pure Rust (proj4rs)","tier":tier,
            "crsClasses":classes,"nativeFallback":self.native,
            "limitations":"Grids, geoid/compound heights, coordinate epochs, unverified datum operations, geographic +lon_0 offsets, non-decimal PROJ angles, quoted PROJ values, PROJ +init or spaced assignments, unsupported WKT syntax, incomplete/mismatched WKT method parameters, unverified method-specific parameter names/identities or colliding aliases, spherical transverse Mercator, oblique stereographic origins at or beyond 80 degrees latitude, ordinary stereographic origins from 80 degrees to below 90 degrees, distinct conic parallels less than one degree apart, conic parallels beyond 80 degrees, omitted LCC origins/second parallels, oblique/nonpolar stereographic points at or beyond 80 degrees source latitude, failed coordinate-domain probes, conflicting polar hemispheres or zero polar standard parallels and Lambert azimuthal equal area require --features native-geospatial. UTM requires an explicit zone; projection scales must be positive, latitudes within method-specific domains and projected PROJ units linear. Conflicting polar stereographic scales and conic parallels whose signed sum has magnitude below one degree and Albers points at or beyond 80 degrees source latitude are refused in all builds. Point clouds require a 2D horizontal CRS and explicit ellipsoidal metre height offset."})
    }

    #[cfg(feature = "native-geospatial")]
    fn vector(&mut self) -> Readiness {
        self.probes
            .extend([Probe::GdalDriverRegistry, Probe::GeosTriangulation]);
        // Execution's own check (vector/pipeline/store.rs) with its error
        // classification. It needs no PROJ database: CRS operations are
        // validated per source, as for point clouds.
        let geometry = Capability::from_check(crate::vector::native_available());
        Readiness {
            capability: geometry.clone(),
            backend: "native GDAL/GEOS",
            requires: vec![
                "native GDAL >= 3.12",
                "GEOS >= 3.10",
                "SQLite",
                "PROJ >= 9.2",
            ],
            details: details(json!({"geospatial":self.native,"geometry":geometry,
                "note":"Native OGR ingestion, constrained triangulation, LOD, meshopt and archive reuse; no Python required."})),
        }
    }

    #[cfg(not(feature = "native-geospatial"))]
    fn vector(&mut self) -> Readiness {
        let crs = self.placement()["crsClasses"].clone();
        Readiness {
            capability: Capability::ready(),
            backend: "portable Rust/SQLite",
            requires: vec![],
            details: details(json!({"inputs":["GeoJSON","GeoPackage"],
                "reader":{"status":Status::Ready,"ready":true,"backend":"Rust GeoJSON and bundled SQLite/WKB"},
                "geometry":{"status":Status::Ready,"ready":true,"backend":"Rust polygon validation, repair and triangulation"},
                "geospatial":{"status":Status::Ready,"ready":true,"backend":"pure Rust (proj4rs)","crsClasses":crs,
                    "nativeFallback":self.native,
                    "limitations":"Only verified grid-free horizontal CRS operations are available. Grids, geoid/compound heights, coordinate epochs and unverified datum operations require --features native-geospatial. GeoPackage 3D horizontal-CRS inputs require an explicit ellipsoidal metre height offset; GeoJSON heights use its ellipsoidal convention."},
                "limitations":"GeoJSON and GeoPackage only; use --features native-geospatial for Shapefile and other OGR drivers. Measured geometries, geometry collections, curves and unsupported field types are refused. Conversion validates each selected source and operation.",
                "note":"Local XYZ, grid-free globe placement, polygons with holes, LOD, meshopt and archive reuse need no Python or system GDAL/GEOS/SQLite."})),
        }
    }

    #[cfg(feature = "native-geospatial")]
    fn drivers(&mut self) -> &Result<crate::raster::RasterCapabilities, crate::JobError> {
        if self.drivers.is_none() {
            self.probes.insert(Probe::GdalDriverRegistry);
            self.drivers = Some(crate::raster::capabilities());
        }
        self.drivers.as_ref().expect("probed above")
    }

    fn raster(&mut self) -> Readiness {
        let note = "Bounded R2 source/COG/display/nearest pyramid profile. Optional input drivers are checked per request; output filesystem support is checked before staging. No Python or GDAL executable required.";
        let requires = vec![
            "native-geospatial",
            "native GDAL >= 3.12 with GTiff and COG",
            "PROJ >= 9.2",
            "local PROJ database/grids",
        ];
        #[cfg(feature = "native-geospatial")]
        {
            // Execution's own admission (raster::output_capability), checked
            // first as in source admission; the PROJ database is needed later
            // to admit the source CRS. The driver inventory is informational.
            self.probes.insert(Probe::GdalDriverRegistry);
            let outputs = Capability::from_check(crate::raster::output_capability());
            let capabilities = match self.drivers() {
                Ok(c) => json!({"nativeVersion":c.native_version,
                    "drivers":{"GTiff":c.gtiff,"PNG":c.png,"JPEG":c.jpeg,"AAIGrid":c.aaigrid,"COG":c.cog}}),
                Err(_) => Value::Null,
            };
            Readiness {
                capability: Capability::all([&outputs, &self.native.capability]),
                backend: "native GDAL",
                requires,
                details: details(json!({"geospatial":self.native,"capabilities":capabilities,
                    "tiling":outputs,"note":note})),
            }
        }
        #[cfg(not(feature = "native-geospatial"))]
        Readiness {
            capability: Capability::unsupported(),
            backend: "native GDAL",
            requires,
            details: details(json!({"tiling":Capability::unsupported(),"note":note})),
        }
    }

    /// Terrain and D1 open sources only through the GDAL GTiff driver. Their
    /// producers export no capability check: they run `native::init` and then
    /// fail to open the source. Doctor therefore composes the GTiff driver from
    /// the raster driver inventory and refuses it as unsupported, while those
    /// producers currently report the failed open as invalid_input
    /// (`terrain/raster.rs`, `raster_directory/native.rs`); see the T1 contract.
    fn gtiff_product(
        &mut self,
        backend: &'static str,
        requires: &[&'static str],
        note: &str,
    ) -> Readiness {
        #[cfg(feature = "native-geospatial")]
        let capability = {
            let gtiff = match self.drivers() {
                Ok(c) if c.gtiff => Capability::ready(),
                Ok(_) => Capability::not_ready(
                    Status::Unavailable,
                    JobErrorKind::Unsupported,
                    "linked GDAL lacks the GTiff driver",
                ),
                Err(error) => {
                    Capability::not_ready(Status::Unavailable, error.kind(), error.message())
                }
            };
            Capability::all([&gtiff, &self.native.capability])
        };
        #[cfg(not(feature = "native-geospatial"))]
        let capability = Capability::unsupported();
        Readiness {
            capability,
            backend,
            requires: requires.to_vec(),
            details: details(json!({"geospatial":self.native,"note":note})),
        }
    }
}

/// A command with no runtime dependency: its resolver is this build.
fn built_in(details: Map<String, Value>) -> Readiness {
    Readiness {
        capability: Capability::ready(),
        backend: "native Rust",
        requires: vec![],
        details,
    }
}

fn details(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        _ => unreachable!("details are JSON objects"),
    }
}

fn proj_inventory(database: Capability, probes: &mut BTreeSet<Probe>) -> ProjInventory {
    let mut paths = BTreeSet::new();
    for name in ["PROJ_DATA", "PROJ_LIB"] {
        if let Some(value) = std::env::var_os(name) {
            paths.extend(std::env::split_paths(&value).filter(|path| !path.as_os_str().is_empty()));
        }
    }
    #[cfg(feature = "native-geospatial")]
    paths.extend(crate::geospatial::proj_search_paths());
    if !paths.is_empty() {
        probes.insert(Probe::ProjDirectoryListing);
    }
    let (data_directories, available_grids, inventory_errors) = list_grids(paths, LISTING_LIMIT);
    ProjInventory {
        database,
        data_directories,
        available_grids,
        inventory_errors,
        network_enabled: false,
        note: "Read-only top-level local grid inventory is not proof that a source-specific height operation is available; conversion validates that operation offline.",
    }
}

/// Non-recursive listing of at most `limit` entries per directory.
fn list_grids(paths: BTreeSet<PathBuf>, limit: usize) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut grids = BTreeSet::new();
    let mut inventory_errors = Vec::new();
    for path in &paths {
        match std::fs::read_dir(path) {
            Ok(entries) => {
                for (index, entry) in entries.enumerate() {
                    if index == limit {
                        inventory_errors.push(format!(
                            "{}: listing truncated after {limit} entries",
                            path.display()
                        ));
                        break;
                    }
                    match entry {
                        Ok(entry) => {
                            let file = entry.path();
                            if file.extension().is_some_and(|suffix| {
                                matches!(suffix.to_str(), Some("gtx" | "gsb" | "tif" | "bin"))
                            }) && file.is_file()
                            {
                                grids.insert(file);
                            }
                        }
                        Err(error) => inventory_errors.push(format!("{}: {error}", path.display())),
                    }
                }
            }
            Err(error) => inventory_errors.push(format!("{}: {error}", path.display())),
        }
    }
    // JSON paths are text; retain the inventory instead of panicking on Unix
    // filenames that are not UTF-8, and report any lossy representation.
    let mut text_paths = |paths: BTreeSet<PathBuf>| {
        paths
            .into_iter()
            .map(|path| {
                if path.to_str().is_none() {
                    inventory_errors.push(format!(
                        "{}: path is not UTF-8; displayed with replacement characters",
                        path.display()
                    ));
                }
                path.to_string_lossy().into_owned()
            })
            .collect::<Vec<_>>()
    };
    let directories = text_paths(paths);
    let grids = text_paths(grids);
    (directories, grids, inventory_errors)
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Ready => "ready",
            Self::Unavailable => "unavailable",
            Self::Unsupported => "unsupported",
        })
    }
}

/// Human report: one `name: status` line per selected command.
impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(versions) = &self.native_geospatial.versions {
            writeln!(
                f,
                "GDAL: {}",
                versions["gdal"].as_str().unwrap_or("unavailable")
            )?;
            writeln!(f, "PROJ: {}", versions["proj"])?;
            writeln!(f, "GEOS: {}", versions["geos"])?;
        }
        let native = &self.native_geospatial.capability;
        if let Some(reason) = &native.reason {
            writeln!(f, "Native geospatial: {} ({reason})", native.status)?;
        }
        for (name, readiness) in &self.commands {
            writeln!(f, "{name}: {}", readiness.capability.status)?;
            if let Some(reason) = &readiness.capability.reason {
                writeln!(f, "  reason: {reason}")?;
            }
            let geospatial = readiness.details.get("geospatial");
            if let Some(backend) = geospatial.and_then(|g| g["backend"].as_str()) {
                writeln!(f, "  geospatial: {backend}")?;
                for class in geospatial
                    .and_then(|g| g["crsClasses"].as_array())
                    .into_iter()
                    .flatten()
                {
                    writeln!(f, "    {}", class.as_str().unwrap_or(""))?;
                }
                writeln!(f, "  native fallback: {}", native.status)?;
            }
            if let Some(cesium) = readiness.details.get("cesium") {
                let path = cesium["path"].as_str().unwrap_or("");
                if cesium["found"] == true {
                    writeln!(f, "  cesium: found at {path}")?;
                } else {
                    writeln!(f, "  cesium: not found at {path} (informational; pass preview --cesium <Build/Cesium>)")?;
                }
            }
        }
        for path in &self.proj.data_directories {
            writeln!(f, "PROJ data: {path}")?;
        }
        writeln!(f, "PROJ grids: {}", self.proj.available_grids.len())?;
        for grid in &self.proj.available_grids {
            writeln!(f, "  {grid}")?;
        }
        write!(f, "{}", self.proj.note)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_aliases_and_selection_are_canonical() {
        for command in Command::ALL {
            assert_eq!(Command::from_name(command.name()), Some(command));
            for alias in command.aliases() {
                assert_eq!(Command::from_name(alias), Some(command));
            }
        }
        assert_eq!(Command::from_name("doctor"), None);
        let all = report(&[], None);
        assert_eq!(all.selected_commands.len(), Command::ALL.len());
        assert_eq!(all.commands.len(), Command::ALL.len());
        let names: Vec<_> = all.selected_commands.iter().map(|c| c.name()).collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        assert_eq!(names, sorted);
        let picked = report(
            &[Command::Preview, Command::MeshTo3tz, Command::Preview],
            None,
        );
        assert_eq!(
            picked.selected_commands,
            [Command::Preview, Command::MeshTo3tz]
        );
        assert_eq!(
            picked.commands.keys().copied().collect::<Vec<_>>(),
            ["mesh-to-3tz", "preview"]
        );
    }

    #[test]
    fn built_in_commands_are_ready_and_cesium_is_checked_only_when_given() {
        let built_in = [
            Command::MeshLocalTo3tz,
            Command::GlbTo3tz,
            Command::CreateTilesetJson,
            Command::Convert,
            Command::Validate,
            Command::Preview,
            Command::MeshTo3tz,
            Command::PointCloud,
        ];
        let result = report(&built_in, None);
        assert!(result.ready);
        assert_eq!(result.status, Status::Ready);
        assert!(result.commands.values().all(|r| r.capability.ready));
        assert!(result.commands["preview"].details.get("cesium").is_none());
        assert!(!result.probes.contains(&Probe::CesiumRuntime));
        assert!(!result.probes.contains(&Probe::GdalDriverRegistry));
        assert!(!result.probes.contains(&Probe::GeosTriangulation));
        let root = tempfile::tempdir().unwrap();
        let result = report(&[Command::Preview], Some(root.path()));
        assert!(result.probes.contains(&Probe::CesiumRuntime));
        assert_eq!(result.commands["preview"].details["cesium"]["found"], false);
        assert!(result.ready);
    }

    #[test]
    fn native_only_commands_follow_the_build() {
        let result = report(
            &[
                Command::Raster,
                Command::Terrain,
                Command::RasterTileToDirectory,
            ],
            None,
        );
        for readiness in result.commands.values() {
            assert_eq!(
                readiness.capability.ready,
                readiness.capability.status == Status::Ready
            );
            #[cfg(not(feature = "native-geospatial"))]
            {
                assert_eq!(readiness.capability.status, Status::Unsupported);
                assert!(readiness
                    .capability
                    .reason
                    .as_ref()
                    .unwrap()
                    .contains("--features native-geospatial"));
            }
            #[cfg(feature = "native-geospatial")]
            assert_ne!(readiness.capability.status, Status::Unsupported);
        }
        #[cfg(not(feature = "native-geospatial"))]
        {
            assert_eq!(result.status, Status::Unsupported);
            assert_eq!(
                result.native_geospatial.capability.status,
                Status::Unsupported
            );
            assert!(result
                .probes
                .iter()
                .all(|p| *p == Probe::ProjDirectoryListing));
        }
    }

    #[test]
    fn concurrent_reports_complete() {
        // Without serialized native probes this aborted inside GDAL in 39 of
        // 40 runs on four test threads.
        let workers: Vec<_> = (0..4)
            .map(|_| {
                std::thread::spawn(|| {
                    (0..4)
                        .map(|_| {
                            report(&[Command::Raster, Command::Terrain, Command::Vector], None)
                        })
                        .map(|result| result.commands.len())
                        .sum::<usize>()
                })
            })
            .collect();
        for worker in workers {
            assert_eq!(worker.join().unwrap(), 12);
        }
    }

    #[test]
    fn capability_combination_keeps_the_first_failure() {
        let ready = Capability::ready();
        let missing =
            Capability::not_ready(Status::Unavailable, JobErrorKind::Unsupported, "missing");
        let rebuild =
            Capability::not_ready(Status::Unsupported, JobErrorKind::Unsupported, "rebuild");
        assert_eq!(Capability::all([&ready, &ready]), ready);
        assert_eq!(Capability::all([&ready, &missing, &rebuild]), missing);
        assert_eq!(Capability::all([&rebuild, &missing]), rebuild);
    }

    #[test]
    fn proj_listing_is_capped_per_directory() {
        let root = tempfile::tempdir().unwrap();
        for index in 0..5 {
            std::fs::write(root.path().join(format!("{index}.gtx")), b"grid").unwrap();
        }
        let paths = BTreeSet::from([root.path().to_owned()]);
        let (_, capped, errors) = list_grids(paths.clone(), 3);
        assert_eq!(capped.len(), 3);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].ends_with("listing truncated after 3 entries"));
        let (directories, complete, errors) = list_grids(paths, 5);
        assert_eq!(directories, [root.path().to_string_lossy()]);
        assert_eq!(complete.len(), 5);
        assert!(errors.is_empty());
    }
}
