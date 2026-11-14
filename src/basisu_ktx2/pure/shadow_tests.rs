//! Shadow tests: the pure-Rust path against the vendored C++ on a generated corpus. Every
//! encode must give identical KTX2 bytes and every level must decode to identical texels.
//! Nothing here is a committed fixture: inputs come from the codec tests' generators.
//!
//! `RUSTY_TILES_BASIS_SHADOW_REPORT=<path>` appends one line per comparison (case, digest,
//! result) to `<path>`, for evidence outside CI.

use std::fmt::Write as _;

use super::super::tests::{
    checker, fixture_mips, flat, low_noise, noise, normal_map, occlusion, orm, photo, Rng,
};
use super::super::{
    encode_mips_unverified, ffi, native_decode_level_rgba, native_encode_mips,
    rust_decode_level_rgba, rust_encode_mips, Etc1Hints, MipLevel, Mode, Params, Transfer,
    UASTC_FASTEST_ETC1_HINTS,
};
use super::decode::zstd_decompress;
use super::{decode_level, zstd_compress};

const REPORT_ENV: &str = "RUSTY_TILES_BASIS_SHADOW_REPORT";

/// `cPackUASTCETC1FastestHints | cPackUASTCETC1DisableFlipAndIndividual`, the leaf pages'
/// UASTC ETC1 hint flags ([`Etc1Hints::Fastest`]).
const FAST_HINTS: u32 = UASTC_FASTEST_ETC1_HINTS;
/// `cPackUASTCETC1FasterHints`.
const FASTER_HINTS: u32 = 0x40;

fn report(lines: &str) {
    if let Ok(path) = std::env::var(REPORT_ENV) {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .expect("open shadow report");
        file.write_all(lines.as_bytes())
            .expect("write shadow report");
    }
}

fn digest(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)[..8]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// A generated input image.
struct Input {
    name: String,
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

fn input(name: &str, width: u32, height: u32, rgba: Vec<u8>) -> Input {
    assert_eq!(rgba.len(), width as usize * height as usize * 4, "{name}");
    Input {
        name: name.to_owned(),
        width,
        height,
        rgba,
    }
}

/// Colour, data and degenerate content at block-aligned, odd and tiny sizes.
fn corpus() -> Vec<Input> {
    let mut rng = Rng(77);
    let extremes: Vec<u8> = [
        [0, 0, 0, 0],
        [255, 255, 255, 255],
        [255, 0, 255, 0],
        [0, 255, 0, 255],
    ]
    .concat();
    let random_alpha: Vec<u8> = (0..48 * 40)
        .flat_map(|i| {
            let v = rng.next().to_le_bytes();
            [
                v[0] / 2 + 64,
                v[1] / 2 + 64,
                128,
                if i % 7 == 0 { 0 } else { 255 },
            ]
        })
        .collect();
    vec![
        input("photo-192x128", 192, 128, photo(192, 128, 1, false)),
        input("photo-alpha-128x128", 128, 128, photo(128, 128, 2, true)),
        input("photo-384x256", 384, 256, photo(384, 256, 3, false)),
        input("photo-wide-512x8", 512, 8, photo(512, 8, 4, false)),
        input("noise-alpha-64x64", 64, 64, noise(64, 64, 5, true)),
        input("noise-61x37", 61, 37, noise(61, 37, 6, false)),
        input("low-noise-64x48", 64, 48, low_noise(64, 48, 7, 6)),
        input("occlusion-128", 128, 128, occlusion(128)),
        input("orm-96", 96, 96, orm(96)),
        input("normal-64", 64, 64, normal_map(64).0),
        input("checker-33x17", 33, 17, checker(33, 17)),
        input("flat-5x3", 5, 3, flat(5, 3, 8)),
        input("cutout-48x40", 48, 40, random_alpha),
        input("extremes-2x2", 2, 2, extremes),
        input("one-1x1", 1, 1, vec![200, 100, 50, 255]),
        input("column-1x7", 1, 7, photo(1, 7, 9, true)),
    ]
}

/// The input as a base-only request or with its full caller-authored chain.
fn chain(input: &Input, full: bool) -> Vec<Vec<u8>> {
    let params = Params {
        effort: 0,
        transfer: Transfer::Linear,
        mipmaps: full,
        etc1_hints: Etc1Hints::Full,
    };
    fixture_mips(&input.rgba, input.width, input.height, params)
}

fn levels<'a>(images: &'a [Vec<u8>], input: &Input) -> Vec<MipLevel<'a>> {
    images
        .iter()
        .enumerate()
        .map(|(i, rgba)| MipLevel {
            rgba,
            width: (input.width >> i).max(1),
            height: (input.height >> i).max(1),
        })
        .collect()
}

fn params(effort: u8, transfer: Transfer) -> Params {
    Params {
        effort,
        transfer,
        mipmaps: false,
        etc1_hints: Etc1Hints::Full,
    }
}

/// Encodes with the C++ oracle entry point, which can add UASTC pack flags.
fn oracle_encode(mips: &[MipLevel<'_>], mode: Mode, params: Params, extra: u32) -> Vec<u8> {
    let (format, quality, mut flags) = match mode {
        Mode::Etc1s { quality } => (ffi::FORMAT_ETC1S, i32::from(quality), 0),
        Mode::Uastc { zstd } => (
            ffi::FORMAT_UASTC_LDR_4X4,
            100,
            if zstd { ffi::FLAG_ZSTD } else { 0 },
        ),
    };
    if params.transfer == Transfer::Srgb {
        flags |= ffi::FLAG_SRGB;
    }
    let native: Vec<ffi::MipLevel> = mips
        .iter()
        .map(|m| ffi::MipLevel {
            rgba: m.rgba.as_ptr(),
            length: m.rgba.len(),
            width: m.width,
            height: m.height,
        })
        .collect();
    let (mut data, mut length) = (std::ptr::null_mut(), 0usize);
    // SAFETY: every level holds width * height * 4 bytes; the out-pointers are valid.
    let status = unsafe {
        ffi::rt_basisu_encode_mips_oracle(
            native.as_ptr(),
            native.len(),
            format,
            quality,
            i32::from(params.effort),
            flags,
            extra,
            &mut data,
            &mut length,
        )
    };
    assert_eq!(
        status,
        ffi::STATUS_OK,
        "oracle encode: {}",
        ffi::diagnostics()
    );
    // SAFETY: on success the shim returns `length` bytes released by rt_basisu_free.
    let bytes = unsafe { std::slice::from_raw_parts(data, length) }.to_vec();
    unsafe { ffi::rt_basisu_free(data) };
    bytes
}

/// Decodes every level with both transcoders and requires identical texels.
fn assert_decodes_match(ktx2: &[u8], input: &Input, level_count: usize, case: &str) {
    for level in 0..level_count as u32 {
        let (w, h) = (
            (input.width >> level).max(1),
            (input.height >> level).max(1),
        );
        let native = native_decode_level_rgba(ktx2, level, w, h)
            .unwrap_or_else(|e| panic!("{case}: C++ decode of level {level}: {}", e.message()));
        let rust = decode_level(ktx2, level, w, h)
            .unwrap_or_else(|e| panic!("{case}: Rust decode of level {level}: {e}"));
        assert!(native == rust, "{case}: level {level} decodes differently");
    }
}

/// Encodes `input` with both paths and requires identical bytes and decodes. Returns the
/// report line.
fn compare(input: &Input, full: bool, mode: Mode, params: Params, extra: u32) -> String {
    let images = chain(input, full);
    let mips = levels(&images, input);
    let case = format!(
        "{} {} {:?} effort {} {:?} flags {extra:#x}",
        input.name,
        if full { "full" } else { "base" },
        mode,
        params.effort,
        params.transfer
    );
    let cpp = if extra == 0 {
        native_encode_mips(&mips, mode, params)
            .unwrap_or_else(|e| panic!("{case}: C++ encode: {}", e.message()))
    } else {
        oracle_encode(&mips, mode, params, extra)
    };
    let rust = rust_encode_mips(&mips, mode, params, extra)
        .unwrap_or_else(|e| panic!("{case}: Rust encode: {}", e.message()));
    if cpp != rust {
        let first = cpp.iter().zip(&rust).position(|(a, b)| a != b);
        panic!(
            "{case}: KTX2 differs (C++ {} bytes {}, Rust {} bytes {}, first difference at {first:?})",
            cpp.len(),
            digest(&cpp),
            rust.len(),
            digest(&rust)
        );
    }
    assert_decodes_match(&rust, input, mips.len(), &case);
    format!(
        "{case}\t{}\t{} bytes\tidentical\n",
        digest(&rust),
        rust.len()
    )
}

const SRGB: Transfer = Transfer::Srgb;
const LINEAR: Transfer = Transfer::Linear;

/// The ETC1S modes the delivery profiles use (q50 sRGB, q90 sRGB, q90 linear scalar data),
/// base-only and full chains, plus quality and effort extremes on a subset.
#[test]
fn etc1s_encodes_are_identical() {
    let mut lines = String::new();
    let mut count = 0;
    for input in corpus() {
        let scalar = input.name.starts_with("occlusion");
        for full in [false, true] {
            let mut modes = vec![
                (Mode::Etc1s { quality: 50 }, params(0, SRGB)),
                (Mode::Etc1s { quality: 90 }, params(0, SRGB)),
            ];
            if scalar || input.name.starts_with("orm") || input.width * input.height <= 64 {
                modes.push((Mode::Etc1s { quality: 90 }, params(0, LINEAR)));
            }
            if input.name == "photo-alpha-128x128" || input.name == "noise-61x37" {
                modes.push((Mode::Etc1s { quality: 1 }, params(0, SRGB)));
                modes.push((Mode::Etc1s { quality: 100 }, params(0, LINEAR)));
                modes.push((Mode::Etc1s { quality: 90 }, params(3, SRGB)));
            }
            for (mode, params) in modes {
                lines += &compare(&input, full, mode, params, 0);
                count += 1;
            }
        }
    }
    report(&lines);
    assert!(count >= 70, "{count} ETC1S comparisons");
}

/// UASTC at every effort the profiles and ladder use (0/1/2/5/8: pack levels 0/0/1/2/3),
/// both transfers, with and without Zstandard, base-only and full chains.
#[test]
fn uastc_encodes_are_identical() {
    let mut lines = String::new();
    let mut count = 0;
    for input in corpus() {
        let small = u64::from(input.width) * u64::from(input.height) <= 128 * 128;
        for full in [false, true] {
            for (effort, transfer, zstd) in [
                (0, SRGB, true),
                (0, LINEAR, true),
                (0, LINEAR, false),
                (1, LINEAR, true),
                (2, SRGB, true),
                (2, LINEAR, false),
            ] {
                lines += &compare(
                    &input,
                    full,
                    Mode::Uastc { zstd },
                    params(effort, transfer),
                    0,
                );
                count += 1;
            }
            if small {
                for (effort, transfer) in [(5, SRGB), (8, LINEAR)] {
                    lines += &compare(
                        &input,
                        full,
                        Mode::Uastc { zstd: true },
                        params(effort, transfer),
                        0,
                    );
                    count += 1;
                }
            }
        }
    }
    report(&lines);
    assert!(count >= 200, "{count} UASTC comparisons");
}

/// The C++ oracle entry point reproduces the production encoder when it adds no flags, so
/// the flag comparisons below are against the same encoder settings.
#[test]
fn oracle_entry_matches_the_production_encoder() {
    let corpus = corpus();
    for input in corpus.iter().filter(|i| {
        ["photo-alpha-128x128", "checker-33x17", "column-1x7"].contains(&i.name.as_str())
    }) {
        for full in [false, true] {
            let images = chain(input, full);
            let mips = levels(&images, input);
            for (mode, params) in [
                (Mode::Etc1s { quality: 50 }, params(0, SRGB)),
                (Mode::Etc1s { quality: 90 }, params(0, LINEAR)),
                (Mode::Uastc { zstd: true }, params(2, SRGB)),
                (Mode::Uastc { zstd: false }, params(0, LINEAR)),
            ] {
                let production = native_encode_mips(&mips, mode, params).unwrap();
                assert!(
                    oracle_encode(&mips, mode, params, 0) == production,
                    "{} {mode:?}: oracle entry differs from rt_basisu_encode_mips",
                    input.name
                );
            }
        }
    }
}

/// UASTC with the ETC1 hint flags (0x180, and 0x40) is identical to the C++ with the same
/// pack flags.
#[test]
fn uastc_hint_flag_encodes_are_identical() {
    let mut lines = String::new();
    for input in corpus()
        .iter()
        .filter(|i| u64::from(i.width) * u64::from(i.height) <= 192 * 128)
    {
        for full in [false, true] {
            for (effort, transfer, extra) in [
                (0, SRGB, FAST_HINTS),
                (0, LINEAR, FAST_HINTS),
                (1, LINEAR, FAST_HINTS),
                (2, SRGB, FAST_HINTS),
                (5, SRGB, FAST_HINTS),
                (2, LINEAR, FASTER_HINTS),
            ] {
                lines += &compare(
                    input,
                    full,
                    Mode::Uastc { zstd: true },
                    params(effort, transfer),
                    extra,
                );
            }
        }
    }
    report(&lines);
}

/// The production encode with [`Etc1Hints::Fastest`] (leaf UASTC pages) takes the pure-Rust
/// path under this feature and must equal the production C++ entry with the same setting,
/// and differ from the full-hint encode (the flags reach both encoders).
#[test]
fn production_fastest_hints_match_on_both_paths() {
    let corpus = corpus();
    for input in corpus
        .iter()
        .filter(|i| ["photo-alpha-128x128", "noise-61x37", "column-1x7"].contains(&i.name.as_str()))
    {
        for full in [false, true] {
            let images = chain(input, full);
            let mips = levels(&images, input);
            for (zstd, effort, transfer) in [(true, 0, SRGB), (false, 2, LINEAR), (true, 5, SRGB)] {
                let mode = Mode::Uastc { zstd };
                let fastest = Params {
                    etc1_hints: Etc1Hints::Fastest,
                    ..params(effort, transfer)
                };
                let case = format!("{} full={full} {mode:?} effort {effort}", input.name);
                let rust = encode_mips_unverified(&mips, mode, fastest).unwrap();
                let cpp = native_encode_mips(&mips, mode, fastest).unwrap();
                assert!(rust == cpp, "{case}: production paths differ");
                assert!(
                    rust == oracle_encode(&mips, mode, params(effort, transfer), FAST_HINTS),
                    "{case}: production differs from the oracle with the hint flags"
                );
                let full_hints = encode_mips_unverified(&mips, mode, params(effort, transfer));
                if input.width * input.height >= 64 {
                    assert!(
                        rust != full_hints.unwrap(),
                        "{case}: hint flags had no effect"
                    );
                }
            }
        }
    }
}

extern "C" {
    fn ZSTD_compress(
        dst: *mut u8,
        capacity: usize,
        src: *const u8,
        size: usize,
        level: i32,
    ) -> usize;
    fn ZSTD_compressBound(size: usize) -> usize;
    fn ZSTD_isError(code: usize) -> u32;
}

fn c_zstd(data: &[u8], level: i32) -> Vec<u8> {
    // SAFETY: the vendored zstd 1.5.7 one-shot API with a buffer of ZSTD_compressBound bytes.
    unsafe {
        let mut out = vec![0u8; ZSTD_compressBound(data.len())];
        let written = ZSTD_compress(
            out.as_mut_ptr(),
            out.len(),
            data.as_ptr(),
            data.len(),
            level,
        );
        assert_eq!(ZSTD_isError(written), 0, "C zstd level {level}");
        out.truncate(written);
        out
    }
}

/// UASTC level payloads (raw blocks) of the corpus, plus degenerate byte strings.
fn zstd_payloads() -> Vec<(String, Vec<u8>)> {
    let mut payloads = Vec::new();
    for input in corpus().iter().filter(|i| {
        [
            "photo-384x256",
            "photo-alpha-128x128",
            "noise-61x37",
            "occlusion-128",
            "flat-5x3",
            "one-1x1",
        ]
        .contains(&i.name.as_str())
    }) {
        let images = chain(input, true);
        let mips = levels(&images, input);
        for effort in [0, 2] {
            let ktx2 = native_encode_mips(&mips, Mode::Uastc { zstd: false }, params(effort, SRGB))
                .unwrap();
            let parsed = super::super::container::parse(&ktx2).unwrap();
            for (i, level) in parsed.levels.iter().enumerate().take(3) {
                let at = level.offset as usize;
                payloads.push((
                    format!("{} e{effort} level {i}", input.name),
                    ktx2[at..at + level.length as usize].to_vec(),
                ));
            }
        }
    }
    let mut rng = Rng(99);
    payloads.push(("zeros 1 MiB".into(), vec![0; 1 << 20]));
    payloads.push(("one byte".into(), vec![7]));
    payloads.push((
        "random 70000".into(),
        (0..70_000).map(|_| rng.next() as u8).collect(),
    ));
    payloads.push((
        "period-3 run".into(),
        (0..300_001).map(|i| (i % 3) as u8).collect(),
    ));
    payloads
}

/// zstd-pure-rs equals the vendored zstd 1.5.7 at every level 1..=19 (the encoder uses 6),
/// and the decoder's wrapper (window bound, content size and checksum checks) decodes every
/// frame back to its input.
#[test]
fn zstd_levels_are_identical_to_c_zstd() {
    let mut lines = String::new();
    let mut frames = 0;
    for (name, data) in zstd_payloads() {
        for level in 1..=19 {
            let c = c_zstd(&data, level);
            let rust = zstd_compress(&data, level).unwrap();
            assert!(
                c == rust,
                "{name}: zstd level {level} differs ({} vs {} bytes)",
                c.len(),
                rust.len()
            );
            let mut decoded = vec![0u8; data.len()];
            let written = zstd_decompress(&rust, &mut decoded)
                .unwrap_or_else(|e| panic!("{name}: level {level} frame did not decode: {e}"));
            assert!(
                written == data.len() && decoded == data,
                "{name}: level {level} round trip"
            );
            frames += 1;
        }
        let _ = writeln!(
            lines,
            "zstd {name}: {} bytes, levels 1..=19 identical",
            data.len()
        );
    }
    report(&lines);
    assert!(frames >= 19 * 30, "{frames} frames");
}

/// A level whose recorded uncompressed size is not its block size is refused before any
/// allocation of that size (the vendored transcoder allocates up to 2 GiB first).
#[test]
fn zstd_level_sizes_bound_the_allocation() {
    let input = &corpus()[0];
    let images = chain(input, false);
    let mips = levels(&images, input);
    let ktx2 = rust_encode_mips(&mips, Mode::Uastc { zstd: true }, params(0, SRGB), 0).unwrap();
    let mut bomb = ktx2.clone();
    // Level 0's uncompressedByteLength: just under the vendored transcoder's 2 GiB limit.
    bomb[80 + 16..80 + 24].copy_from_slice(&(2048u64 * 1024 * 1024 - 1).to_le_bytes());
    let (result, peak) = crate::test_alloc::peak(|| decode_level(&bomb, 0, 192, 128));
    let error = result.unwrap_err();
    assert!(error.contains("records"), "{error}");
    assert!(peak < 1 << 20, "peak {peak} bytes");
    let (result, peak) = crate::test_alloc::peak(|| decode_level(&ktx2, 0, 192, 128));
    result.unwrap();
    let image = 192 * 128 * 4;
    assert!(peak < 4 * image + (1 << 20), "decode peak {peak} bytes");
}

/// A zstd frame: no content size, no checksum, window descriptor `window` (exponent in the
/// top five bits, so windowLog = 10 + `window >> 3`), then `blocks` of (type, size, body).
fn zstd_frame(window: u8, blocks: &[(u32, u32, &[u8])]) -> Vec<u8> {
    let mut frame = vec![0x28, 0xB5, 0x2F, 0xFD, 0x00, window];
    for (i, &(kind, size, body)) in blocks.iter().enumerate() {
        let last = u32::from(i + 1 == blocks.len());
        frame.extend_from_slice(&((size << 3) | (kind << 1) | last).to_le_bytes()[..3]);
        frame.extend_from_slice(body);
    }
    frame
}

/// A 4x4 UASTC Zstandard page whose level 0 is replaced by `frame` (appended to the file).
fn page_with_level_frame(frame: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let input = input("one-block", 4, 4, photo(4, 4, 11, false));
    let images = chain(&input, false);
    let mips = levels(&images, &input);
    let raw = rust_encode_mips(&mips, Mode::Uastc { zstd: false }, params(0, SRGB), 0).unwrap();
    let level = &super::super::container::parse(&raw).unwrap().levels[0];
    let block = raw[level.offset as usize..][..16].to_vec();
    let mut page = rust_encode_mips(&mips, Mode::Uastc { zstd: true }, params(0, SRGB), 0).unwrap();
    let offset = page.len() as u64;
    page.extend_from_slice(frame);
    page[80..88].copy_from_slice(&offset.to_le_bytes());
    page[88..96].copy_from_slice(&(frame.len() as u64).to_le_bytes());
    assert_eq!(u64::from_le_bytes(page[96..104].try_into().unwrap()), 16);
    (page, block)
}

/// A few KiB of RLE blocks that expand to 128 MiB under a 2^27 window, in a level of 16
/// bytes: refused before the window is allocated, and RLE blocks inside an allowed window
/// are refused once they overrun the level, also with bounded memory. The vendored
/// one-shot decoder refuses both too (its output is the 16-byte level).
#[test]
fn zstd_window_bombs_are_refused_with_bounded_memory() {
    const RLE: u32 = 1;
    let byte: &[u8] = &[0x55];
    let big = zstd_frame(17 << 3, &vec![(RLE, 128 << 10, byte); 1024]);
    let small = zstd_frame(0, &vec![(RLE, 1 << 10, byte); 4096]);
    assert!(big.len() < 5000 && small.len() < 20_000);
    for (name, frame, refusal) in [
        ("windowLog 27", &big, "exceeds the level's bound"),
        ("windowLog 10", &small, "did not decompress"),
    ] {
        let (page, _) = page_with_level_frame(frame);
        let (result, peak) = crate::test_alloc::peak(|| decode_level(&page, 0, 4, 4));
        assert!(peak < 1 << 20, "{name}: peak {peak} bytes");
        let error = result.unwrap_err();
        assert!(error.contains(refusal), "{name}: {error}");
        assert!(
            native_decode_level_rgba(&page, 0, 4, 4).is_err(),
            "{name}: C++ decodes it"
        );
    }
}

/// The documented strictness: a frame whose declared window exceeds its level's bound is
/// refused even when its content is valid, which the vendored decoder accepts. Frames zstd
/// writes with a known content size never declare such a window.
#[test]
fn zstd_windows_beyond_the_level_are_refused_where_zstd_accepts_them() {
    const RAW: u32 = 0;
    let probe = zstd_frame(17 << 3, &[(RAW, 16, &[0; 16])]);
    let (_, block) = page_with_level_frame(&probe);
    for (window, accepted) in [(17u8 << 3, false), (0, true)] {
        let (page, _) = page_with_level_frame(&zstd_frame(window, &[(RAW, 16, &block)]));
        let native = native_decode_level_rgba(&page, 0, 4, 4).unwrap();
        match decode_level(&page, 0, 4, 4) {
            Ok(rust) => assert!(accepted && rust == native, "window byte {window}"),
            Err(error) => assert!(
                !accepted && error.contains("exceeds the level's bound"),
                "window byte {window}: {error}"
            ),
        }
    }
}

/// The front end's refusals where the vendored transcoder would decode: each is documented
/// in `decode.rs` and docs/architecture/basis-pure-rust.md.
const STRICTER: [&str; 9] = [
    "vkFormat",
    "faces and",
    "does not suit the format",
    "ETC1S video",
    "shorter than its header",
    "Zstandard level records",
    "exceeds the level's bound",
    // ruzstd's literal-stream checks catch corruption (in frames without a checksum) that
    // zstd 1.5.7 decodes.
    "DecodedLiteralCountMismatch",
    "BitstreamReadMismatch",
];

/// Deterministic mutations of valid files: both transcoders decode every level of each
/// mutant. The Rust front end never panics, never accepts a file the C++ rejects, decodes
/// identically when both accept, and refuses what the C++ accepts only for a documented
/// reason.
#[test]
fn mutated_containers_decode_identically_or_are_refused() {
    let corpus = corpus();
    let pick = |name: &str| corpus.iter().find(|i| i.name == name).unwrap();
    let mut bases = Vec::new();
    for (name, mode, params, full) in [
        (
            "photo-alpha-128x128",
            Mode::Etc1s { quality: 90 },
            params(0, SRGB),
            true,
        ),
        (
            "photo-192x128",
            Mode::Etc1s { quality: 50 },
            params(0, SRGB),
            false,
        ),
        (
            "cutout-48x40",
            Mode::Uastc { zstd: true },
            params(0, LINEAR),
            true,
        ),
        (
            "noise-61x37",
            Mode::Uastc { zstd: false },
            params(0, SRGB),
            true,
        ),
    ] {
        let input = pick(name);
        let images = chain(input, full);
        let mips = levels(&images, input);
        bases.push((input, rust_encode_mips(&mips, mode, params, 0).unwrap()));
    }
    // RUSTY_TILES_BASIS_SHADOW_MUTANTS raises the count per base file for longer runs.
    let rounds = std::env::var("RUSTY_TILES_BASIS_SHADOW_MUTANTS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(300u64);
    let interesting: [u64; 9] = [0, 1, 2, 3, 4, 0x7F, 0xFFFF, 0xFFFF_FFFF, u64::MAX];
    let mut rng = Rng(1234);
    let (mut agree_ok, mut agree_err, mut stricter) = (0, 0, std::collections::BTreeMap::new());
    for (input, base) in &bases {
        let levels = super::super::container::parse(base).unwrap().levels.len() as u32;
        for round in 0..rounds {
            let mut file = base.clone();
            match round % 5 {
                // A random byte anywhere.
                0 => {
                    let at = (rng.next() % file.len() as u64) as usize;
                    file[at] ^= 1 << (rng.next() % 8);
                }
                // A header or level-index field set to a boundary value.
                1 => {
                    let words = (80 + 24 * levels as usize) / 4;
                    let at = (rng.next() % words as u64) as usize * 4;
                    let value = interesting[(rng.next() % 9) as usize];
                    file[at..at + 4].copy_from_slice(&(value as u32).to_le_bytes());
                }
                // A level-index or global-data 64-bit field set to a boundary or file length.
                2 => {
                    let at = 64 + 8 * (rng.next() % (2 + 3 * u64::from(levels))) as usize;
                    let value = match rng.next() % 3 {
                        0 => interesting[(rng.next() % 9) as usize],
                        1 => file.len() as u64 - rng.next() % 64,
                        _ => {
                            u64::from_le_bytes(file[at..at + 8].try_into().unwrap())
                                + 1
                                + rng.next() % 16
                        }
                    };
                    file[at..at + 8].copy_from_slice(&value.to_le_bytes());
                }
                // A byte in the DFD, key/value data or global data.
                3 => {
                    let start = 80 + 24 * levels as usize;
                    let at =
                        start + (rng.next() % 200.min(file.len() as u64 - start as u64)) as usize;
                    file[at] = rng.next() as u8;
                }
                // Truncation.
                _ => {
                    let keep = 81 + (rng.next() % (file.len() as u64 - 81)) as usize;
                    file.truncate(keep);
                }
            }
            for level in 0..levels {
                let (w, h) = (
                    (input.width >> level).max(1),
                    (input.height >> level).max(1),
                );
                // As verification and decoded views call it: the level must also have the
                // expected dimensions.
                let native = native_decode_level_rgba(&file, level, w, h);
                let rust =
                    rust_decode_level_rgba(&file, level, w, h).map_err(|e| e.message().to_owned());
                match (native, rust) {
                    (Ok(native), Ok(rust)) => {
                        assert!(
                            native == rust,
                            "{}: round {round} level {level} decodes differently",
                            input.name
                        );
                        agree_ok += 1;
                    }
                    (Err(_), Err(error)) => {
                        assert!(
                            !error.contains("panicked"),
                            "{}: round {round}: {error}",
                            input.name
                        );
                        agree_err += 1;
                    }
                    (Err(native), Ok(_)) => panic!(
                        "{}: round {round} level {level}: Rust decodes a file the C++ refuses ({})",
                        input.name,
                        native.message()
                    ),
                    (Ok(_), Err(error)) => {
                        let reason = STRICTER.iter().find(|r| error.contains(*r)).unwrap_or_else(|| {
                            panic!("{}: round {round} level {level}: undocumented refusal: {error}", input.name)
                        });
                        // The refusal's kind, without its numbers.
                        let tail = &error[error.find(*reason).unwrap_or(0)..];
                        let kind: String = tail
                            .chars()
                            .filter(|c| !c.is_ascii_digit())
                            .take_while(|&c| c != '{')
                            .collect();
                        let entry = stricter.entry(kind).or_insert((0, error.clone()));
                        entry.0 += 1;
                    }
                }
            }
        }
    }
    report(&format!(
        "mutants: {} files; both decode identically {agree_ok}, both refuse {agree_err}, Rust stricter {stricter:?}\n",
        bases.len() as u64 * rounds
    ));
    assert!(agree_ok > 100 && agree_err > 100, "{agree_ok} {agree_err}");
}
