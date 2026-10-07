use std::process::Command;
fn main() {
    #[cfg(feature = "native-geospatial")]
    {
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
    if std::env::var_os("RUSTY_TILES_DISABLE_NATIVE_JPEG").is_some() {
        return;
    }
    if std::env::var("HOST").ok() != std::env::var("TARGET").ok() {
        return;
    }
    let Ok(output) = Command::new("pkg-config")
        .args(["--libs", "libturbojpeg"])
        .output()
    else {
        return;
    };
    if !output.status.success() {
        return;
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
