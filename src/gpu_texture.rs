//! Native UASTC encoding with a measured quality floor. Intermediate textures
//! remain lossless; GPU compression is applied only to emitted tile content.
use crate::Error;
use std::{fs, path::Path, process::Command};

pub fn check(executable: &Path) -> Result<(), Error> {
    let out=Command::new(executable).arg("-version").output().map_err(|e|Error::msg(format!("Basis Universal encoder {} is unavailable ({e}); install basisu, set --basisu, or explicitly choose --textureFormat lossless",executable.display())))?;
    if !out.status.success() {
        return Err(Error::msg("basisu -version failed"));
    }
    Ok(())
}

fn psnr(log: &str, label: &str) -> Option<f64> {
    log.lines()
        .find(|line| line.starts_with(label))?
        .split("PSNR:")
        .nth(1)?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

pub fn encode(
    executable: &Path,
    dir: &Path,
    name: &str,
    png: &[u8],
    opaque: bool,
) -> Result<Vec<u8>, Error> {
    let input = dir.join(format!("{name}.png"));
    let output = dir.join(format!("{name}.ktx2"));
    fs::write(&input, png)?;
    for effort in ["0", "1"] {
        let mut cmd = Command::new(executable);
        cmd.args([
            "-uastc",
            "-uastc_level",
            effort,
            "-ktx2",
            "-ktx2_zstandard_level",
            "3",
            "-mipmap",
            "-mip_filter",
            "box",
            "-no_multithreading",
            "-stats",
        ]);
        if opaque {
            cmd.arg("-no_alpha");
        }
        let result = cmd
            .arg("-file")
            .arg(&input)
            .arg("-output_file")
            .arg(&output)
            .output()?;
        if !result.status.success() {
            return Err(Error::msg(format!(
                "basisu failed: {} {}",
                String::from_utf8_lossy(&result.stdout),
                String::from_utf8_lossy(&result.stderr)
            )));
        }
        let log = String::from_utf8_lossy(&result.stdout);
        let rgb = psnr(&log, ".basis RGB Avg:")
            .ok_or_else(|| Error::msg("basisu did not report source-level RGB quality"))?;
        let alpha = if opaque {
            100.0
        } else {
            psnr(&log, ".basis A   Avg:")
                .ok_or_else(|| Error::msg("basisu did not report alpha quality"))?
        };
        if rgb >= 42.0 && alpha >= 50.0 {
            let bytes = fs::read(&output)?;
            fs::remove_file(&input)?;
            fs::remove_file(&output)?;
            if !bytes.starts_with(b"\xabKTX 20\xbb\r\n\x1a\n") {
                return Err(Error::msg("basisu did not produce KTX2"));
            }
            return Ok(bytes);
        }
    }
    // No silent quality reduction on difficult charts. PNG is a valid glTF
    // fallback and retains the same full-resolution source pixels.
    fs::remove_file(input)?;
    fs::remove_file(output)?;
    eprintln!("mesh-to-3tz: {name} retained as PNG to meet texture quality floor");
    Ok(png.to_vec())
}

#[cfg(test)]
mod tests {
    #[test]
    fn quality_parser_uses_base_level_and_rejects_missing_metrics() {
        assert_eq!(
            super::psnr(
                ".basis RGB Avg: Max: 2 PSNR: 47.5 dB\n.basis RGB Avg: PSNR: 30 dB",
                ".basis RGB Avg:"
            ),
            Some(47.5)
        );
        assert_eq!(super::psnr("encoder failed", ".basis RGB Avg:"), None);
    }
}
