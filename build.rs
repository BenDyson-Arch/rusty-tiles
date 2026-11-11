use std::path::PathBuf;

fn main() {
    compile_basisu();
    #[cfg(feature = "native-geospatial")]
    {
        // PROJ/GDAL also load SQLite. Mixing that shared library with a bundled
        // copy can interpose only part of SQLite's API and corrupt its state.
        // libsqlite3-sys provides this override even when `bundled` is enabled;
        // require it for native builds, including downstream library consumers.
        println!("cargo:rerun-if-env-changed=LIBSQLITE3_SYS_USE_PKG_CONFIG");
        assert!(
            std::env::var("LIBSQLITE3_SYS_USE_PKG_CONFIG").as_deref() == Ok("1"),
            "native-geospatial requires system SQLite shared with GDAL/PROJ: set LIBSQLITE3_SYS_USE_PKG_CONFIG=1 before running cargo; portable builds bundle SQLite without this variable"
        );
        for variable in ["SQLITE3_STATIC", "PKG_CONFIG_ALL_STATIC", "SQLITE3_LIB_DIR"] {
            println!("cargo:rerun-if-env-changed={variable}");
            assert!(
                std::env::var_os(variable).is_none(),
                "native-geospatial requires shared SQLite from the GDAL/PROJ pkg-config environment; unset {variable} and configure PKG_CONFIG_PATH instead"
            );
        }
        if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
            println!("cargo:rerun-if-env-changed=VCPKGRS_DYNAMIC");
            assert!(
                std::env::var("VCPKGRS_DYNAMIC").as_deref() == Ok("1"),
                "native-geospatial on MSVC requires VCPKGRS_DYNAMIC=1 for shared SQLite"
            );
        }
        pkg_config::Config::new()
            .atleast_version("3.12")
            .cargo_metadata(false)
            .probe("gdal")
            .expect("native-geospatial requires GDAL >= 3.12 headers/libraries and pkg-config");
        pkg_config::Config::new()
            .atleast_version("9.2")
            .cargo_metadata(false)
            .probe("proj")
            .expect("native-geospatial requires PROJ >= 9.2 for strict only-best transformations");
    }
}

/// Vendored Basis Universal v2.50 (see vendor/README.md):
/// the encoder sources listed by upstream CMake's ENCODER_LIB_SRC_LIST plus the
/// Android ASTC decompressor it enables by default. OpenCL and SSE are off.
/// The two files in BASISU_GUARDED_SOURCES are the rest of that list.
const BASISU_CXX_SOURCES: &[&str] = &[
    "encoder/basisu_basis_file.cpp",
    "encoder/basisu_comp.cpp",
    "encoder/basisu_enc.cpp",
    "encoder/basisu_etc.cpp",
    "encoder/basisu_gpu_texture.cpp",
    "encoder/basisu_pvrtc1_4.cpp",
    "encoder/basisu_resampler.cpp",
    "encoder/basisu_resample_filters.cpp",
    "encoder/basisu_ssim.cpp",
    "encoder/basisu_uastc_enc.cpp",
    "encoder/basisu_bc7e_scalar.cpp",
    "encoder/basisu_dds_export.cpp",
    "encoder/basisu_bc7enc.cpp",
    "encoder/jpgd.cpp",
    "encoder/basisu_kernels_sse.cpp",
    "encoder/basisu_bc15_spmd.cpp",
    "encoder/basisu_bc15_spmd_sse.cpp",
    "encoder/basisu_opencl.cpp",
    "encoder/pvpngreader.cpp",
    "encoder/basisu_uastc_hdr_4x4_enc.cpp",
    "encoder/basisu_astc_hdr_6x6_enc.cpp",
    "encoder/basisu_astc_hdr_common.cpp",
    "encoder/basisu_astc_ldr_common.cpp",
    "encoder/basisu_astc_ldr_encode.cpp",
    "encoder/basisu_astc_ldr_fencode.cpp",
    "encoder/basisu_xbc7_encode.cpp",
    "encoder/basisu_tinyexr.cpp",
    "encoder/3rdparty/android_astc_decomp.cpp",
];

/// The rest of the source list: translation units that end the process on
/// failure, each compiled with a second force-included header that turns
/// that into a C++ exception the shim reports as a status:
/// - the encoder's internal consistency checks call abort();
/// - the one definition of basisu::container_abort(), called on allocation
///   failure, size overflow or a failed span check, calls std::terminate().
const BASISU_GUARDED_SOURCES: &[(&[&str], &str)] = &[
    (
        &["encoder/basisu_backend.cpp", "encoder/basisu_frontend.cpp"],
        "rt_basisu_invariant.h",
    ),
    (
        &["transcoder/basisu_transcoder.cpp"],
        "rt_basisu_container.h",
    ),
];

/// Compiles the vendored encoder, its Zstandard copy and the rusty-tiles shim
/// with one fixed configuration on every target and profile:
/// - optimized and NDEBUG even in dev builds, so tests exercise the shipped code;
/// - no floating-point contraction, so FMA availability does not change bytes;
/// - every vendored translation unit force-includes rt_basisu_silence.h, which
///   routes its stdio output to the capturing sinks in rt_basisu_sink.cpp;
/// - C++ exceptions are enabled on every target (MSVC needs /EHsc), so the
///   shim can catch them and the guarded sources' checks can unwind to it.
fn compile_basisu() {
    let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let vendor = manifest.join("vendor/basis_universal");
    let native = manifest.join("src/basisu_ktx2/native");
    println!("cargo:rerun-if-changed=vendor/basis_universal");
    println!("cargo:rerun-if-changed=src/basisu_ktx2/native");
    let msvc = std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    let silence = native.join("rt_basisu_silence.h");

    let configure = |build: &mut cc::Build, cpp: bool, force_includes: &[&PathBuf]| {
        build
            .cpp(cpp)
            .include(&vendor)
            .opt_level(2)
            .debug(false)
            .warnings(false)
            .extra_warnings(false)
            .cargo_warnings(false)
            .define("NDEBUG", None)
            .define("BASISU_SUPPORT_SSE", "0")
            .define("BASISU_SUPPORT_OPENCL", "0")
            .define("BASISD_SUPPORT_KTX2", "1")
            .define("BASISD_SUPPORT_KTX2_ZSTD", "1")
            .define("BASISU_DISABLE_ANDROID_ASTC_DECOMP", "0");
        if cpp {
            build.std("c++17");
        }
        if msvc {
            build.flag("/bigobj").flag("/fp:precise").flag("/utf-8");
            if cpp {
                build.flag("/EHsc");
            }
            for header in force_includes {
                build.flag(format!("/FI{}", header.display()));
            }
        } else {
            build
                .flag("-fno-strict-aliasing")
                .flag("-ffp-contract=off")
                .flag("-fvisibility=hidden");
            if cpp {
                build.flag("-fexceptions");
            }
            for header in force_includes {
                build.flag("-include").flag(header.as_os_str());
            }
        }
    };

    // Objects only; they go into the rt_basisu_vendor archive below.
    let mut guarded_objects = Vec::new();
    for (sources, guard) in BASISU_GUARDED_SOURCES {
        let mut guarded = cc::Build::new();
        configure(&mut guarded, true, &[&silence, &native.join(guard)]);
        guarded_objects.extend(
            guarded
                .files(sources.iter().map(|source| vendor.join(source)))
                .compile_intermediates(),
        );
    }

    let mut cxx = cc::Build::new();
    configure(&mut cxx, true, &[&silence]);
    cxx.files(BASISU_CXX_SOURCES.iter().map(|source| vendor.join(source)))
        .file(native.join("rt_basisu_shim.cpp"))
        .objects(guarded_objects)
        .compile("rt_basisu_vendor");

    let mut zstd = cc::Build::new();
    configure(&mut zstd, false, &[&silence]);
    zstd.file(vendor.join("zstd/zstd.c"))
        .compile("rt_basisu_zstd");

    let mut sink = cc::Build::new();
    configure(&mut sink, true, &[]);
    sink.file(native.join("rt_basisu_sink.cpp"))
        .compile("rt_basisu_sink");
}
