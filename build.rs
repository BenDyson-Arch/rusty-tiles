use std::process::Command;
fn main() {
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
    println!("cargo:rustc-check-cfg=cfg(rusty_tiles_native_jpeg)");
    println!("cargo:rerun-if-env-changed=RUSTY_TILES_DISABLE_NATIVE_JPEG");
    println!("cargo:rerun-if-env-changed=PKG_CONFIG_PATH");
    if !cfg!(feature = "native-jpeg")
        || std::env::var_os("RUSTY_TILES_DISABLE_NATIVE_JPEG").is_some()
    {
        return;
    }
    if std::env::var("HOST").ok() != std::env::var("TARGET").ok() {
        return;
    }
    let Ok(output) = Command::new("pkg-config")
        .args(["--libs", "libturbojpeg"])
        .output()
    else {
        panic!(
            "native-jpeg requires pkg-config and libjpeg-turbo; omit the feature for portable JPEG"
        );
    };
    if !output.status.success() {
        panic!("native-jpeg requires libjpeg-turbo; omit the feature for portable JPEG");
    }
    for flag in String::from_utf8_lossy(&output.stdout).split_whitespace() {
        if let Some(path) = flag.strip_prefix("-L") {
            println!("cargo:rustc-link-search=native={path}");
        }
        if let Some(lib) = flag.strip_prefix("-l") {
            println!("cargo:rustc-link-lib={lib}");
        }
    }
    println!("cargo:rustc-cfg=rusty_tiles_native_jpeg");
}
