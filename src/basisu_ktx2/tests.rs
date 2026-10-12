use super::*;
use sha2::{Digest, Sha256};

/// splitmix64, so fixtures are identical on every target.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

/// Smooth photo-like content: gradients, low-frequency waves, a few hard
/// edges and mild texel noise. Integer arithmetic only.
fn photo(width: u32, height: u32, seed: u64, alpha: bool) -> Vec<u8> {
    let mut rng = Rng(seed);
    let mut out = Vec::with_capacity(width as usize * height as usize * 4);
    for y in 0..height {
        for x in 0..width {
            let (fx, fy) = (x * 256 / width, y * 256 / height);
            let wave = |period: u32, phase: u32| {
                let t = (fx * 3 + fy * 2 + phase) % period;
                let tri = if t * 2 < period { t } else { period - t };
                tri * 255 / (period / 2)
            };
            let edge = if (fx / 48 + fy / 64) % 2 == 0 { 40 } else { 0 };
            let noise = (rng.next() % 9) as u32;
            let r = (fx * 3 / 4 + wave(97, 0) / 4 + edge + noise).min(255);
            let g = (fy * 3 / 4 + wave(131, 40) / 4 + noise).min(255);
            let b = ((fx + fy) / 3 + wave(173, 90) / 3 + edge / 2 + noise).min(255);
            let a = if alpha { (fx + fy / 2).min(255) } else { 255 };
            out.extend_from_slice(&[r as u8, g as u8, b as u8, a as u8]);
        }
    }
    out
}

/// Uniform random bytes, the worst case for any block codec.
fn noise(width: u32, height: u32, seed: u64, alpha: bool) -> Vec<u8> {
    let mut rng = Rng(seed);
    (0..width as usize * height as usize)
        .flat_map(|_| {
            let v = rng.next().to_le_bytes();
            [v[0], v[1], v[2], if alpha { v[3] } else { 255 }]
        })
        .collect()
}

fn flat(width: u32, height: u32, seed: u64) -> Vec<u8> {
    let v = Rng(seed).next().to_le_bytes();
    (0..width as usize * height as usize)
        .flat_map(|_| [v[0], v[1], v[2], 255])
        .collect()
}

fn checker(width: u32, height: u32) -> Vec<u8> {
    (0..height)
        .flat_map(|y| (0..width).map(move |x| if (x + y) % 2 == 0 { 0u8 } else { 255 }))
        .flat_map(|v| [v, v, v, 255])
        .collect()
}

/// A structurally different image of the same size: flipped and inverted.
fn other_image(rgba: &[u8], width: u32) -> Vec<u8> {
    rgba.chunks_exact(width as usize * 4)
        .rev()
        .flat_map(|row| {
            row.as_chunks::<4>()
                .0
                .iter()
                .flat_map(|p| [255 - p[0], 255 - p[1], 255 - p[2], p[3]])
        })
        .collect()
}

fn psnr_db(a: &[u8], b: &[u8]) -> f64 {
    let error = sse(a, b);
    if error == 0 {
        return f64::INFINITY;
    }
    10.0 * (255.0f64 * 255.0 * a.len() as f64 / error as f64).log10()
}

fn hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

const COLOUR: Params = Params {
    effort: 0,
    transfer: Transfer::Srgb,
    mipmaps: true,
};
const DATA: Params = Params {
    effort: 0,
    transfer: Transfer::Linear,
    mipmaps: true,
};

/// The profile-like configurations from the design table, plus plain UASTC.
const CASES: [(&str, Mode, Params); 5] = [
    ("etc1s-q40-e0-srgb", Mode::Etc1s { quality: 40 }, COLOUR),
    ("etc1s-q90-e0-linear", Mode::Etc1s { quality: 90 }, DATA),
    ("uastc-e0-zstd-srgb", Mode::Uastc { zstd: true }, COLOUR),
    ("uastc-e0-zstd-linear", Mode::Uastc { zstd: true }, DATA),
    (
        "uastc-e0-linear-base",
        Mode::Uastc { zstd: false },
        Params {
            mipmaps: false,
            ..DATA
        },
    ),
];

#[test]
fn encode_is_byte_deterministic_and_reports_digests() {
    let (width, height) = (192, 128);
    let image = photo(width, height, 1, true);
    for (name, mode, params) in CASES {
        let first = encode_ktx2(&image, width, height, mode, params).unwrap();
        let second = encode_ktx2(&image, width, height, mode, params).unwrap();
        assert!(first == second, "{name}: repeated encodes differ");
        // Logged so CI runs on every target can be compared.
        println!("basisu-ktx2-digest {name} {} {}", first.len(), hex(&first));
    }
}

#[test]
fn concurrent_encodes_match_sequential_bytes() {
    let (width, height) = (96, 80);
    let image = photo(width, height, 3, false);
    let sequential: Vec<Vec<u8>> = CASES
        .iter()
        .map(|&(_, mode, params)| encode_ktx2(&image, width, height, mode, params).unwrap())
        .collect();
    let concurrent: Vec<Vec<Vec<u8>>> = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..4)
            .map(|worker| {
                let image = &image;
                scope.spawn(move || {
                    // Different start offsets so different modes overlap.
                    let mut out = vec![Vec::new(); CASES.len()];
                    for step in 0..CASES.len() {
                        let index = (step + worker) % CASES.len();
                        let (_, mode, params) = CASES[index];
                        out[index] = encode_ktx2(image, width, height, mode, params).unwrap();
                    }
                    out
                })
            })
            .collect();
        workers.into_iter().map(|w| w.join().unwrap()).collect()
    });
    for run in concurrent {
        assert!(
            run == sequential,
            "concurrent encodes differ from sequential"
        );
    }
}

#[test]
fn round_trips_meet_psnr_thresholds() {
    let (width, height) = (256, 256);
    let image = photo(width, height, 1, false);
    // Synthetic-photo thresholds, about 2 dB under the Linux x86_64
    // measurement; not a quality contract.
    for (mode, params, threshold) in [
        (Mode::Etc1s { quality: 40 }, COLOUR, 31.5),
        (Mode::Etc1s { quality: 90 }, COLOUR, 33.5),
        (Mode::Etc1s { quality: 90 }, DATA, 34.5),
        (Mode::Uastc { zstd: true }, COLOUR, 40.5),
        (Mode::Uastc { zstd: false }, DATA, 40.5),
    ] {
        let encoded = encode_ktx2(&image, width, height, mode, params).unwrap();
        let verified = verify_ktx2(&encoded, &image, width, height, mode, params).unwrap();
        assert_eq!(verified.levels, 9);
        let decoded = decode_level_rgba(&encoded, 0, width, height).unwrap();
        let psnr = psnr_db(&image, &decoded);
        println!(
            "basisu-ktx2-psnr {mode:?} {:?} {psnr:.2} dB",
            params.transfer
        );
        assert!(
            psnr >= threshold,
            "{mode:?} {params:?}: {psnr:.2} dB < {threshold} dB"
        );
    }
}

#[test]
fn edge_shapes_encode_and_verify() {
    for (width, height, image) in [
        (1, 1, flat(1, 1, 10)),
        (3, 5, noise(3, 5, 11, true)),
        (37, 19, photo(37, 19, 5, false)),
        (64, 64, checker(64, 64)),
        (64, 64, noise(64, 64, 12, true)),
    ] {
        for (name, mode, params) in CASES {
            encode_ktx2(&image, width, height, mode, params)
                .unwrap_or_else(|error| panic!("{name} {width}x{height}: {error}"));
        }
    }
}

fn level0_range(ktx2: &[u8]) -> std::ops::Range<usize> {
    let level = container::parse(ktx2).unwrap().levels[0];
    level.offset as usize..(level.offset + level.length) as usize
}

fn expect_invalid_state(result: Result<Verified, JobError>, what: &str) {
    match result {
        Err(error) => assert_eq!(error.kind(), JobErrorKind::InvalidState, "{what}: {error}"),
        Ok(verified) => panic!("{what}: corrupted output passed verification: {verified:?}"),
    }
}

#[test]
fn verification_rejects_corrupted_output() {
    let (width, height) = (128, 96);
    let image = photo(width, height, 4, false);
    for (name, mode, params) in CASES {
        let good = encode_ktx2(&image, width, height, mode, params).unwrap();
        let verify =
            |bytes: &[u8], input: &[u8]| verify_ktx2(bytes, input, width, height, mode, params);
        // Control: the untouched output verifies.
        verify(&good, &image).unwrap();

        let mut corrupted = good.clone();
        corrupted[12] = 37; // vkFormat
        expect_invalid_state(verify(&corrupted, &image), &format!("{name} vkFormat"));

        let mut corrupted = good.clone();
        let dfd = u32::from_le_bytes(good[48..52].try_into().unwrap()) as usize;
        corrupted[dfd + 14] ^= 3; // transfer function sRGB <-> linear
        expect_invalid_state(verify(&corrupted, &image), &format!("{name} DFD transfer"));

        let mut corrupted = good.clone();
        corrupted[40] = corrupted[40].wrapping_add(1); // levelCount
        expect_invalid_state(verify(&corrupted, &image), &format!("{name} levelCount"));

        expect_invalid_state(
            verify(&good[..good.len() - 7], &image),
            &format!("{name} truncated"),
        );

        // Scramble the whole base level payload.
        let mut corrupted = good.clone();
        for byte in &mut corrupted[level0_range(&good)] {
            *byte ^= 0x5A;
        }
        expect_invalid_state(
            verify(&corrupted, &image),
            &format!("{name} level 0 payload"),
        );

        // A valid file checked against different content fails the floor.
        expect_invalid_state(
            verify(&good, &other_image(&image, width)),
            &format!("{name} wrong input"),
        );
    }
}

#[test]
fn requests_outside_the_domain_are_refused() {
    let image = flat(4, 4, 1);
    let etc1s = Mode::Etc1s { quality: 40 };
    type Case<'a> = (&'a [u8], u32, u32, Mode, Params, JobErrorKind);
    let cases: [Case; 6] = [
        (&image, 0, 4, etc1s, COLOUR, JobErrorKind::Unsupported),
        (
            &image,
            MAX_EDGE + 1,
            1,
            etc1s,
            COLOUR,
            JobErrorKind::Unsupported,
        ),
        (
            &image[..60],
            4,
            4,
            etc1s,
            COLOUR,
            JobErrorKind::InvalidInput,
        ),
        (
            &image,
            4,
            4,
            Mode::Etc1s { quality: 0 },
            COLOUR,
            JobErrorKind::InvalidRequest,
        ),
        (
            &image,
            4,
            4,
            Mode::Etc1s { quality: 101 },
            COLOUR,
            JobErrorKind::InvalidRequest,
        ),
        (
            &image,
            4,
            4,
            etc1s,
            Params {
                effort: 11,
                ..COLOUR
            },
            JobErrorKind::InvalidRequest,
        ),
    ];
    for (rgba, width, height, mode, params, kind) in cases {
        let error = encode_ktx2(rgba, width, height, mode, params).unwrap_err();
        assert_eq!(
            error.kind(),
            kind,
            "{width}x{height} {mode:?} {params:?}: {error}"
        );
    }
}

const SILENCE_CHILD: &str = "basisu_ktx2::tests::silence_child";
const SILENCE_ENV: &str = "RUSTY_TILES_BASISU_SILENCE_CHILD";

/// Runs the silence child in a fresh process and returns (stdout, stderr).
fn run_silence_child(mode: &str) -> (String, String) {
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", SILENCE_CHILD, "--nocapture", "--test-threads=1"])
        .env(SILENCE_ENV, mode)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        output.status.success(),
        "silence child ({mode}) failed: {}\nstdout:\n{stdout}\nstderr:\n{stderr}",
        output.status
    );
    (stdout, stderr)
}

/// stdout lines that are not the test harness's own.
fn foreign_lines(stdout: &str) -> Vec<&str> {
    let own = format!("test {SILENCE_CHILD} ... ok");
    stdout
        .lines()
        .filter(|line| {
            !(line.is_empty()
                || line.starts_with("running ")
                || line.starts_with("test result: ")
                || *line == own)
        })
        .collect()
}

// A subprocess entry point, not an evidence test on its own; see
// `library_calls_write_nothing_to_stdout_or_stderr`.
#[test]
fn silence_child() {
    let Ok(mode) = std::env::var(SILENCE_ENV) else {
        return;
    };
    if mode == "control" {
        // Sensitivity control for the parent's capture.
        println!("silence-control-stdout");
        eprintln!("silence-control-stderr");
        return;
    }
    let image = photo(64, 48, 6, true);
    for (_, mode, params) in CASES {
        let good = encode_ktx2(&image, 64, 48, mode, params).unwrap();
        let mut corrupted = good.clone();
        for byte in &mut corrupted[level0_range(&good)] {
            *byte ^= 0x5A;
        }
        assert!(verify_ktx2(&corrupted, &image, 64, 48, mode, params).is_err());
        assert!(verify_ktx2(&good[..100], &image, 64, 48, mode, params).is_err());
    }
    assert!(decode_level_rgba(b"not a ktx2 file", 0, 1, 1).is_err());

    // The vendored encoder's own error path (its edge limit) must reach the
    // capturing sink: it writes "basis_compress: Image too large" through
    // error_printf, which upstream sends to stderr.
    let before = unsafe { ffi::rt_basisu_sink_calls() };
    let wide = vec![0u8; (MAX_EDGE as usize + 1) * 4];
    let (mut data, mut length) = (std::ptr::null_mut(), 0usize);
    let status = unsafe {
        ffi::rt_basisu_encode(
            wide.as_ptr(),
            MAX_EDGE + 1,
            1,
            ffi::FORMAT_ETC1S,
            40,
            0,
            0,
            &mut data,
            &mut length,
        )
    };
    assert_ne!(status, ffi::STATUS_OK);
    assert!(data.is_null());
    assert!(unsafe { ffi::rt_basisu_sink_calls() } > before);
    let diagnostics = ffi::diagnostics();
    assert!(
        diagnostics.contains("too large"),
        "captured: {diagnostics:?}"
    );
}

#[test]
fn library_calls_write_nothing_to_stdout_or_stderr() {
    let (stdout, stderr) = run_silence_child("control");
    assert!(
        stdout.contains("silence-control-stdout"),
        "capture missed stdout: {stdout:?}"
    );
    assert!(
        stderr.contains("silence-control-stderr"),
        "capture missed stderr: {stderr:?}"
    );

    let (stdout, stderr) = run_silence_child("library");
    assert_eq!(stderr, "", "library wrote to stderr");
    assert!(
        foreign_lines(&stdout).is_empty(),
        "library wrote to stdout: {stdout:?}"
    );
}

#[test]
#[ignore = "measurement: decoded error against the 4x4 block-mean reconstruction"]
fn measure_floors() {
    let mut modes = vec![];
    for quality in [1, 10, 40, 90, 100] {
        for effort in [0, 5, 10] {
            modes.push((Mode::Etc1s { quality }, effort));
        }
    }
    for effort in [0, 2, 5] {
        modes.push((Mode::Uastc { zstd: true }, effort));
        modes.push((Mode::Uastc { zstd: false }, effort));
    }
    let images = [
        ("photo", 128, 128, photo(128, 128, 1, false)),
        ("photo-alpha", 128, 96, photo(128, 96, 2, true)),
        ("photo-odd", 37, 19, photo(37, 19, 5, false)),
        ("noise", 64, 64, noise(64, 64, 3, false)),
        ("noise-alpha", 64, 64, noise(64, 64, 4, true)),
        ("checker", 64, 64, checker(64, 64)),
        ("flat-a", 32, 32, flat(32, 32, 7)),
        ("flat-b", 32, 32, flat(32, 32, 8)),
        ("flat-c", 32, 32, flat(32, 32, 9)),
        ("one", 1, 1, flat(1, 1, 10)),
        ("strip", 3, 5, noise(3, 5, 11, true)),
    ];
    for (name, width, height, image) in &images {
        let block_mean = block_mean_sse(image, *width, *height);
        let wrong = other_image(image, *width);
        for &(mode, effort) in &modes {
            for transfer in [Transfer::Srgb, Transfer::Linear] {
                let params = Params {
                    effort,
                    transfer,
                    mipmaps: true,
                };
                let encoded = encode_unverified(image, *width, *height, mode, params).unwrap();
                let decoded = decode_level_rgba(&encoded, 0, *width, *height).unwrap();
                let per_byte = |error: u64| error as f64 / image.len() as f64;
                println!(
                    "{name:12} {mode:?} e{effort} {transfer:?}: mse {:.2}, block-mean mse {:.2}, wrong-input mse {:.2}",
                    per_byte(sse(image, &decoded)),
                    per_byte(block_mean),
                    per_byte(sse(&wrong, &decoded)),
                );
            }
        }
    }
}
