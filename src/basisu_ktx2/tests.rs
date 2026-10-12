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

/// Mid-grey plus uniform noise of +-`amplitude`, per channel; opaque.
fn low_noise(width: u32, height: u32, seed: u64, amplitude: u8) -> Vec<u8> {
    let mut rng = Rng(seed);
    let span = 2 * u64::from(amplitude) + 1;
    (0..width as usize * height as usize)
        .flat_map(|_| {
            let mut channel = || (128 - u64::from(amplitude) + rng.next() % span) as u8;
            [channel(), channel(), channel(), 255]
        })
        .collect()
}

/// Detail in chroma only: R and G move in opposite directions, so the luma
/// R + G + B is constant. ETC1S cannot represent it.
fn chroma_noise(width: u32, height: u32, seed: u64) -> Vec<u8> {
    let mut rng = Rng(seed);
    (0..width as usize * height as usize)
        .flat_map(|_| {
            let d = (rng.next() % 161) as u8;
            [48 + d, 208 - d, 128, 255]
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

    // A vendored library write aimed at stderr must reach the capturing
    // sink: basisu::container_abort() writes its message with fputs(stderr)
    // (and no longer ends the process; see rt_basisu_container.h).
    let before = unsafe { ffi::rt_basisu_sink_calls() };
    let status = unsafe { ffi::rt_basisu_selftest_failure(2) };
    assert_eq!(status, ffi::STATUS_CONTAINER_FAILED);
    assert!(unsafe { ffi::rt_basisu_sink_calls() } > before);
    let diagnostics = ffi::diagnostics();
    assert!(
        diagnostics.contains("self-test container failure 2"),
        "captured: {diagnostics:?}"
    );
    // The shim refuses an over-limit edge itself, before the library's own
    // "Image too large" error_printf path.
    let wide = vec![0u8; (MAX_EDGE as usize + 1) * 4];
    let status = raw_encode(&wide, MAX_EDGE + 1, 1, ffi::FORMAT_ETC1S, 40, 0, 0);
    assert_eq!(status, ffi::STATUS_INVALID_ARGUMENT);
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
        ("noise-big", 256, 256, noise(256, 256, 13, false)),
        ("noise-low", 64, 64, low_noise(64, 64, 14, 3)),
        ("noise-mid", 64, 64, low_noise(64, 64, 15, 24)),
        ("chroma-noise", 64, 64, chroma_noise(64, 64, 16)),
        ("photo-big", 256, 256, photo(256, 256, 17, true)),
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
                let correlation = |d: Detail| {
                    d.covariance as f64 / ((d.input as f64) * (d.decoded as f64)).sqrt()
                };
                let own = detail(image, &decoded, *width, *height);
                println!(
                    "{name:12} {mode:?} e{effort} {transfer:?}: mse {:.2}, block-mean mse {:.2}, wrong-input mse {:.2}, \
                     detail var {:.2}, corr {:.3}, wrong corr {:.3}",
                    per_byte(sse(image, &decoded)),
                    per_byte(block_mean),
                    per_byte(sse(&wrong, &decoded)),
                    own.input as f64 / own.scale as f64,
                    correlation(own),
                    correlation(detail(&wrong, &decoded, *width, *height)),
                );
            }
        }
    }
}

const HOSTILE_CHILD: &str = "basisu_ktx2::tests::hostile_child";
const HOSTILE_ENV: &str = "RUSTY_TILES_BASISU_HOSTILE_CHILD";

/// Runs the hostile child in a fresh process; returns whether it exited
/// successfully, with its combined output.
fn run_hostile_child(mode: &str) -> (bool, String) {
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", HOSTILE_CHILD, "--nocapture", "--test-threads=1"])
        .env(HOSTILE_ENV, mode)
        .output()
        .unwrap();
    let text = format!(
        "status {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (output.status.success(), text)
}

fn raw_encode(
    rgba: &[u8],
    width: u32,
    height: u32,
    format: u32,
    quality: i32,
    effort: i32,
    flags: u32,
) -> i32 {
    let (mut data, mut length) = (std::ptr::null_mut(), 0usize);
    // SAFETY: the out-pointers are valid; the shim validates every argument
    // before it reads `rgba`, and the accepted cases pass a full buffer.
    let status = unsafe {
        ffi::rt_basisu_encode(
            rgba.as_ptr(),
            width,
            height,
            format,
            quality,
            effort,
            flags,
            &mut data,
            &mut length,
        )
    };
    if !data.is_null() {
        unsafe { ffi::rt_basisu_free(data) };
    }
    status
}

/// Degenerate and extreme images that must encode and verify.
fn extreme_images() -> Vec<(&'static str, u32, u32, Vec<u8>)> {
    let mut rng = Rng(21);
    let mut impulse = vec![0u8; 64 * 64 * 4];
    impulse[(31 * 64 + 17) * 4..(31 * 64 + 17) * 4 + 4].copy_from_slice(&[255, 255, 255, 255]);
    let binary: Vec<u8> = (0..64 * 64)
        .flat_map(|_| {
            let v = rng.next();
            [0, 8, 16, 24].map(|shift| if (v >> shift) & 1 == 1 { 255 } else { 0 })
        })
        .collect();
    let stripes: Vec<u8> = (0..64u32)
        .flat_map(|y| {
            (0..64u32).flat_map(move |x| {
                [
                    if x % 2 == 0 { 255 } else { 0 },
                    if y % 2 == 0 { 255 } else { 0 },
                    if (x / 3 + y) % 2 == 0 { 255 } else { 0 },
                    if (x + y / 5) % 2 == 0 { 0 } else { 255 },
                ]
            })
        })
        .collect();
    let invisible: Vec<u8> = noise(64, 64, 22, false)
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|p| [p[0], p[1], p[2], 0])
        .collect();
    vec![
        ("1x1 transparent black", 1, 1, vec![0; 4]),
        ("1x1 white", 1, 1, vec![255; 4]),
        (
            "2x2 extremes",
            2,
            2,
            [
                [0, 0, 0, 0],
                [255, 255, 255, 255],
                [255, 0, 255, 0],
                [0, 255, 0, 255],
            ]
            .concat(),
        ),
        (
            "16384x1 wide strip",
            MAX_EDGE,
            1,
            photo(MAX_EDGE, 1, 23, true),
        ),
        (
            "1x16384 tall strip",
            1,
            MAX_EDGE,
            noise(1, MAX_EDGE, 24, true),
        ),
        ("all zero", 64, 64, vec![0; 64 * 64 * 4]),
        ("all 255", 64, 64, vec![255; 64 * 64 * 4]),
        ("single impulse", 64, 64, impulse),
        ("binary noise", 64, 64, binary),
        ("max-contrast stripes", 64, 64, stripes),
        ("noise under zero alpha", 64, 64, invisible),
    ]
}

/// Encodes, verifies and decodes hostile and degenerate inputs in this
/// process; the parent checks that the process ends normally.
// A subprocess entry point, not an evidence test on its own; see
// `hostile_inputs_return_errors_without_ending_the_process`.
#[test]
fn hostile_child() {
    let Ok(mode) = std::env::var(HOSTILE_ENV) else {
        return;
    };
    if mode == "abort-control" {
        // Sensitivity control: the parent must see an abort as a failure.
        std::process::abort();
    }
    let etc1s = Mode::Etc1s { quality: 40 };
    let all_modes = CASES.map(|(_, mode, params)| (mode, params));

    // Refused before any library call: typed errors.
    let refused: [(&[u8], u32, u32, JobErrorKind); 7] = [
        (&[], 0, 0, JobErrorKind::Unsupported),
        (&[], 0, 4, JobErrorKind::Unsupported),
        (&[0; 16], 4, 0, JobErrorKind::Unsupported),
        (&[], MAX_EDGE + 1, 1, JobErrorKind::Unsupported),
        (&[], 1, u32::MAX, JobErrorKind::Unsupported),
        (&[], MAX_EDGE, MAX_EDGE, JobErrorKind::Unsupported),
        (&[0; 15], 2, 2, JobErrorKind::InvalidInput),
    ];
    for (rgba, width, height, kind) in refused {
        for (mode, params) in all_modes {
            let error = encode_ktx2(rgba, width, height, mode, params).unwrap_err();
            assert_eq!(error.kind(), kind, "{width}x{height} {mode:?}: {error}");
        }
    }

    // The C ABI refuses the same domain itself, without reading the image.
    let tiny = [0u8; 16];
    let fmt_etc1s = ffi::FORMAT_ETC1S;
    let fmt_uastc = ffi::FORMAT_UASTC_LDR_4X4;
    for (what, width, height, format, quality, effort, flags) in [
        ("width 0", 0, 2, fmt_etc1s, 40, 0, 0),
        ("height 0", 2, 0, fmt_etc1s, 40, 0, 0),
        ("edge over limit", MAX_EDGE + 1, 1, fmt_etc1s, 40, 0, 0),
        (
            "texels over limit",
            MAX_EDGE,
            MAX_EDGE,
            fmt_uastc,
            100,
            0,
            0,
        ),
        ("unknown format", 2, 2, 7, 40, 0, 0),
        ("unknown flag", 2, 2, fmt_etc1s, 40, 0, 1 << 9),
        ("ETC1S quality 0", 2, 2, fmt_etc1s, 0, 0, 0),
        ("ETC1S quality 101", 2, 2, fmt_etc1s, 101, 0, 0),
        ("UASTC with RDO quality", 2, 2, fmt_uastc, 50, 0, 0),
        ("effort -1", 2, 2, fmt_etc1s, 40, -1, 0),
        ("effort 11", 2, 2, fmt_etc1s, 40, 11, 0),
        ("effort i32::MAX", 2, 2, fmt_uastc, 100, i32::MAX, 0),
    ] {
        let status = raw_encode(&tiny, width, height, format, quality, effort, flags);
        assert_eq!(status, ffi::STATUS_INVALID_ARGUMENT, "{what}");
    }
    let (mut data, mut length) = (std::ptr::null_mut(), 0usize);
    // SAFETY: null image pointer, valid out-pointers; the shim must refuse.
    let status = unsafe {
        ffi::rt_basisu_encode(
            std::ptr::null(),
            2,
            2,
            fmt_etc1s,
            40,
            0,
            0,
            &mut data,
            &mut length,
        )
    };
    assert_eq!(status, ffi::STATUS_INVALID_ARGUMENT, "null image");
    // SAFETY: null out-pointers; the shim must refuse without writing.
    let status = unsafe {
        ffi::rt_basisu_encode(
            tiny.as_ptr(),
            2,
            2,
            fmt_etc1s,
            40,
            0,
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    assert_eq!(status, ffi::STATUS_INVALID_ARGUMENT, "null outputs");

    // Degenerate and extreme content inside the domain encodes and verifies.
    for (name, width, height, image) in extreme_images() {
        for (mode, params) in all_modes {
            encode_ktx2(&image, width, height, mode, params)
                .unwrap_or_else(|error| panic!("{name} {mode:?} {params:?}: {error}"));
        }
        for effort in [5, MAX_EFFORT] {
            if u64::from(width) * u64::from(height) <= 4096 {
                encode_ktx2(&image, width, height, etc1s, Params { effort, ..COLOUR })
                    .unwrap_or_else(|error| panic!("{name} effort {effort}: {error}"));
            }
        }
    }

    // The two library failure paths that used to end the process come back
    // as statuses, with their message captured.
    for (which, status, message) in [
        (
            0,
            ffi::STATUS_INVARIANT_FAILED,
            "self-test invariant failure",
        ),
        (
            1,
            ffi::STATUS_CONTAINER_FAILED,
            "self-test container failure 1",
        ),
    ] {
        assert_eq!(unsafe { ffi::rt_basisu_selftest_failure(which) }, status);
        let diagnostics = ffi::diagnostics();
        assert!(diagnostics.contains(message), "captured: {diagnostics:?}");
    }

    // The decoder on malformed containers: any outcome but the end of the
    // process. Corruptions reach the transcoder directly, without the
    // independent container parse that normally runs first.
    let image = photo(64, 48, 25, true);
    let mut rng = Rng(26);
    for (mode, params) in all_modes {
        let good = encode_ktx2(&image, 64, 48, mode, params).unwrap();
        let _ = decode_level_rgba(&[], 0, 1, 1);
        let _ = decode_level_rgba(&good[..12], 0, 64, 48);
        let _ = decode_level_rgba(&good, 99, 1, 1);
        let _ = decode_level_rgba(&good, 0, 1, 1);
        for cut in [13, 48, 80, 104, good.len() / 2, good.len() - 1] {
            let _ = decode_level_rgba(&good[..cut], 0, 64, 48);
        }
        // Every header, level index and DFD byte, set to 0xFF and to 0.
        let dfd_end = u32::from_le_bytes(good[48..52].try_into().unwrap()) as usize
            + u32::from_le_bytes(good[52..56].try_into().unwrap()) as usize;
        for index in 0..dfd_end.min(good.len()) {
            for value in [0xFF, 0x00] {
                let mut bad = good.clone();
                bad[index] = value;
                let _ = decode_level_rgba(&bad, 0, 64, 48);
            }
        }
        // Random byte flips anywhere, including supercompression data and
        // level payloads.
        for _ in 0..300 {
            let mut bad = good.clone();
            for _ in 0..1 + rng.next() % 8 {
                let at = (rng.next() % bad.len() as u64) as usize;
                bad[at] ^= 1 << (rng.next() % 8);
            }
            let _ = decode_level_rgba(&bad, 0, 64, 48);
            let _ = verify_ktx2(&bad, &image, 64, 48, mode, params);
        }
    }
    println!("hostile-child-done");
}

#[test]
fn hostile_inputs_return_errors_without_ending_the_process() {
    let (ok, output) = run_hostile_child("abort-control");
    assert!(!ok, "the harness did not see the control abort:\n{output}");
    let (ok, output) = run_hostile_child("hostile");
    assert!(ok, "hostile inputs ended the process or failed:\n{output}");
    assert!(output.contains("hostile-child-done"), "{output}");
}

#[test]
fn etc1s_detail_floor_rejects_a_wrong_decode_the_error_floor_accepts() {
    // On pure noise the block-mean error floor cannot tell the right input
    // from another noise image; the detail floor must.
    for (width, height, seed) in [(64, 64, 31), (256, 256, 32)] {
        let image = noise(width, height, seed, false);
        let other = noise(width, height, seed + 100, false);
        for quality in [1, 40, 90] {
            let mode = Mode::Etc1s { quality };
            let encoded = encode_ktx2(&image, width, height, mode, COLOUR).unwrap();
            let decoded = decode_level_rgba(&encoded, 0, width, height).unwrap();
            let allowed = ETC1S_FLOOR.ratio * block_mean_sse(&other, width, height)
                + ETC1S_FLOOR.slack_per_byte * other.len() as u64;
            assert!(
                sse(&other, &decoded) <= allowed,
                "control: the error floor alone accepts the wrong input"
            );
            let error = verify_ktx2(&encoded, &other, width, height, mode, COLOUR).unwrap_err();
            assert_eq!(error.kind(), JobErrorKind::InvalidState);
            assert!(error.to_string().contains("detail"), "{error}");
        }
    }
}

#[test]
fn detail_floor_skips_detail_a_codec_cannot_carry() {
    // Chroma-only detail has no luma detail, so ETC1S, which has none to
    // give, is not held to it; low-amplitude noise is under the variance gate.
    for image in [chroma_noise(64, 64, 33), low_noise(64, 64, 34, 3)] {
        for quality in [1, 40] {
            encode_ktx2(&image, 64, 64, Mode::Etc1s { quality }, COLOUR).unwrap();
        }
    }
}

/// Decodes every level with the independent pure-Rust transcoder (`basisu`
/// crate, a port of the reference transcoder) and checks it against the
/// vendored transcoder's decode byte for byte. Returns the base level.
fn independent_decode(ktx2: &[u8], width: u32, height: u32) -> Vec<u8> {
    let transcoder = basisu::Transcoder::new(ktx2).expect("independent decoder rejects the file");
    assert_eq!(transcoder.base_dimensions(), (width, height));
    let mut base = None;
    for level in 0..transcoder.level_count() {
        let independent = transcoder
            .transcode(
                level,
                basisu::TargetFormat::Rgba32,
                basisu::DecodeFlags::NONE,
            )
            .expect("independent decoder fails a level");
        let (w, h) = ((width >> level).max(1), (height >> level).max(1));
        let vendored = decode_level_rgba(ktx2, level, w, h).unwrap();
        assert!(
            independent == vendored,
            "level {level}: the decoders disagree"
        );
        if level == 0 {
            base = Some(independent);
        }
    }
    base.unwrap()
}

/// Texture classes of the large-mesh design (slots in parentheses).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    /// baseColor, emissive: sRGB.
    Colour,
    /// occlusion: linear, one channel.
    DataR,
    /// metallicRoughness, ORM: linear.
    DataRgb,
    /// normal: linear, unit vectors.
    Normal,
}

/// The provisional profile table of the design (lossless is PNG, not here).
fn profile_mode(profile: &str, class: Class) -> (Mode, Params) {
    let etc1s = |quality| Mode::Etc1s { quality };
    let uastc = Mode::Uastc { zstd: true };
    let mode = match (profile, class) {
        ("fast", Class::Colour) => etc1s(40),
        ("balanced", Class::Colour) => etc1s(90),
        ("fast" | "balanced", Class::DataR) => etc1s(90),
        _ => uastc,
    };
    let params = if class == Class::Colour { COLOUR } else { DATA };
    (mode, params)
}

/// Integer triangle wave in 0..=255 with the given period.
fn tri(t: u32, period: u32) -> u32 {
    let t = t % period;
    let up = if t * 2 < period { t } else { period - t };
    up * 510 / period
}

fn occlusion(size: u32) -> Vec<u8> {
    let mut rng = Rng(42);
    (0..size)
        .flat_map(|y| (0..size).map(move |x| (x, y)))
        .flat_map(|(x, y)| {
            let crease = if (x + 2 * y) % 61 < 3 { 90 } else { 0 };
            let v = (160 + tri(x + y, 89) / 4 + tri(3 * x, 157) / 8)
                .saturating_sub(crease)
                .saturating_sub((rng.next() % 6) as u32)
                .min(255) as u8;
            [v, v, v, 255]
        })
        .collect()
}

fn orm(size: u32) -> Vec<u8> {
    let mut rng = Rng(43);
    let ao = occlusion(size);
    (0..size)
        .flat_map(|y| (0..size).map(move |x| (x, y)))
        .zip(ao.as_chunks::<4>().0)
        .flat_map(|((x, y), ao)| {
            let rough = (60 + tri(x * 2 + y, 211) / 2 + (rng.next() % 21) as u32).min(255);
            let metal = if (x / 40 + y / 56) % 3 == 0 { 255 } else { 0 };
            [ao[0], rough as u8, metal, 255]
        })
        .collect()
}

/// A tangent-space normal map of a smooth height field with a few ridges:
/// (encoded RGBA, unit normals).
fn normal_map(size: u32) -> (Vec<u8>, Vec<[f64; 3]>) {
    let height = |x: i64, y: i64| -> i64 {
        let (x, y) = (
            x.rem_euclid(size as i64) as u32,
            y.rem_euclid(size as i64) as u32,
        );
        // A ridge with a 4-texel ramp each side, not a step.
        let ridge = 6 * (4 - ((x + y) % 97).abs_diff(4).min(4));
        i64::from(tri(x * 2, 173) + tri(y * 3, 211) + tri(x + y, 131) / 2 + ridge)
    };
    let mut rgba = Vec::new();
    let mut normals = Vec::new();
    for y in 0..size as i64 {
        for x in 0..size as i64 {
            let dx = (height(x + 1, y) - height(x - 1, y)) as f64;
            let dy = (height(x, y + 1) - height(x, y - 1)) as f64;
            let n = [-dx, -dy, 24.0];
            let length = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            let n = n.map(|v| v / length);
            let bytes = n.map(|v| ((v + 1.0) * 127.5).round().clamp(0.0, 255.0) as u8);
            rgba.extend_from_slice(&[bytes[0], bytes[1], bytes[2], 255]);
            normals.push(unpack_normal(&bytes));
        }
    }
    (rgba, normals)
}

/// Decoded bytes to a renormalised unit vector.
fn unpack_normal(bytes: &[u8]) -> [f64; 3] {
    let n = [0, 1, 2].map(|i| f64::from(bytes[i]) / 127.5 - 1.0);
    let length = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-12);
    n.map(|v| v / length)
}

/// PSNR and maximum absolute error over the given channels.
fn channel_error(a: &[u8], b: &[u8], channels: &[usize]) -> (f64, u8) {
    let (mut sum, mut count, mut max) = (0u64, 0u64, 0u8);
    for (p, q) in a.as_chunks::<4>().0.iter().zip(b.as_chunks::<4>().0) {
        for &c in channels {
            let e = p[c].abs_diff(q[c]);
            sum += u64::from(e) * u64::from(e);
            count += 1;
            max = max.max(e);
        }
    }
    let psnr = if sum == 0 {
        f64::INFINITY
    } else {
        10.0 * (255.0f64 * 255.0 * count as f64 / sum as f64).log10()
    };
    (psnr, max)
}

/// Appearance floors per (profile, class) on the 256x256 fixtures below,
/// decoded by the independent transcoder. PSNR and maximum error are over RGB
/// (data-R: R only); for normals also the mean and 99th-percentile angle to
/// the source normal after renormalisation. The PSNR and angle floors are the
/// design's coarse class floors (colour 22 dB ETC1S / 34 dB UASTC, data
/// 30 dB, normal mean 3 and p99 12 degrees). The maximum-error floors are
/// about 1.3x the measured values (rounded up to a multiple of 8); the
/// measurements are in vendor/README.md and identical on all five targets
/// (byte-identical encodes). They catch a broken codec path, not a quality
/// regression; profile constants stay provisional until the final acceptance
/// milestone.
struct Floor {
    profile: &'static str,
    class: Class,
    min_psnr: f64,
    max_error: u8,
    max_mean_degrees: f64,
    max_p99_degrees: f64,
}

const NO_ANGLE: f64 = f64::INFINITY;

const FLOORS: [Floor; 12] = [
    Floor {
        profile: "fast",
        class: Class::Colour,
        min_psnr: 22.0,
        max_error: 56,
        max_mean_degrees: NO_ANGLE,
        max_p99_degrees: NO_ANGLE,
    },
    Floor {
        profile: "fast",
        class: Class::DataR,
        min_psnr: 30.0,
        max_error: 80,
        max_mean_degrees: NO_ANGLE,
        max_p99_degrees: NO_ANGLE,
    },
    Floor {
        profile: "fast",
        class: Class::DataRgb,
        min_psnr: 30.0,
        max_error: 32,
        max_mean_degrees: NO_ANGLE,
        max_p99_degrees: NO_ANGLE,
    },
    Floor {
        profile: "fast",
        class: Class::Normal,
        min_psnr: 30.0,
        max_error: 112,
        max_mean_degrees: 3.0,
        max_p99_degrees: 12.0,
    },
    Floor {
        profile: "balanced",
        class: Class::Colour,
        min_psnr: 22.0,
        max_error: 32,
        max_mean_degrees: NO_ANGLE,
        max_p99_degrees: NO_ANGLE,
    },
    Floor {
        profile: "balanced",
        class: Class::DataR,
        min_psnr: 30.0,
        max_error: 80,
        max_mean_degrees: NO_ANGLE,
        max_p99_degrees: NO_ANGLE,
    },
    Floor {
        profile: "balanced",
        class: Class::DataRgb,
        min_psnr: 30.0,
        max_error: 32,
        max_mean_degrees: NO_ANGLE,
        max_p99_degrees: NO_ANGLE,
    },
    Floor {
        profile: "balanced",
        class: Class::Normal,
        min_psnr: 30.0,
        max_error: 112,
        max_mean_degrees: 3.0,
        max_p99_degrees: 12.0,
    },
    Floor {
        profile: "quality",
        class: Class::Colour,
        min_psnr: 34.0,
        max_error: 24,
        max_mean_degrees: NO_ANGLE,
        max_p99_degrees: NO_ANGLE,
    },
    Floor {
        profile: "quality",
        class: Class::DataR,
        min_psnr: 30.0,
        max_error: 8,
        max_mean_degrees: NO_ANGLE,
        max_p99_degrees: NO_ANGLE,
    },
    Floor {
        profile: "quality",
        class: Class::DataRgb,
        min_psnr: 30.0,
        max_error: 32,
        max_mean_degrees: NO_ANGLE,
        max_p99_degrees: NO_ANGLE,
    },
    Floor {
        profile: "quality",
        class: Class::Normal,
        min_psnr: 30.0,
        max_error: 112,
        max_mean_degrees: 3.0,
        max_p99_degrees: 12.0,
    },
];

#[test]
fn profile_class_floors_hold_with_an_independent_decoder() {
    let size = 256;
    let colour = photo(size, size, 41, false);
    let data_r = occlusion(size);
    let data_rgb = orm(size);
    let (normal, normals) = normal_map(size);
    let mut failures = Vec::new();
    for floor in &FLOORS {
        let (image, channels): (&[u8], &[usize]) = match floor.class {
            Class::Colour => (&colour, &[0, 1, 2]),
            Class::DataR => (&data_r, &[0]),
            Class::DataRgb => (&data_rgb, &[0, 1, 2]),
            Class::Normal => (&normal, &[0, 1, 2]),
        };
        let (mode, params) = profile_mode(floor.profile, floor.class);
        let encoded = encode_ktx2(image, size, size, mode, params).unwrap();
        let decoded = independent_decode(&encoded, size, size);
        let (psnr, max_error) = channel_error(image, &decoded, channels);
        let (mut mean, mut p99) = (0.0, 0.0);
        if floor.class == Class::Normal {
            let mut angles: Vec<f64> = decoded
                .as_chunks::<4>()
                .0
                .iter()
                .zip(&normals)
                .map(|(bytes, source)| {
                    let n = unpack_normal(bytes);
                    let dot = n[0] * source[0] + n[1] * source[1] + n[2] * source[2];
                    dot.clamp(-1.0, 1.0).acos().to_degrees()
                })
                .collect();
            angles.sort_by(f64::total_cmp);
            mean = angles.iter().sum::<f64>() / angles.len() as f64;
            p99 = angles[angles.len() * 99 / 100];
        }
        println!(
            "basisu-ktx2-floor {} {:?} {mode:?}: psnr {psnr:.2} dB, max error {max_error}, \
             normal mean {mean:.2} p99 {p99:.2} deg, {} bytes",
            floor.profile,
            floor.class,
            encoded.len()
        );
        if psnr < floor.min_psnr
            || max_error > floor.max_error
            || mean > floor.max_mean_degrees
            || p99 > floor.max_p99_degrees
        {
            failures.push(format!("{} {:?}", floor.profile, floor.class));
        }
    }
    assert!(failures.is_empty(), "floors missed: {failures:?}");

    // Sensitivity control: the same metric on a decode of the wrong image
    // misses even the loosest floor.
    let (mode, params) = profile_mode("quality", Class::Colour);
    let encoded = encode_ktx2(&colour, size, size, mode, params).unwrap();
    let decoded = independent_decode(&encoded, size, size);
    let (psnr, max_error) = channel_error(&other_image(&colour, size), &decoded, &[0, 1, 2]);
    assert!(
        psnr < 22.0 && max_error > 112,
        "control: {psnr:.2} dB, max error {max_error}"
    );
}
