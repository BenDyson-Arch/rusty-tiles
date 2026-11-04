//! H-1 controls: the pinned `EXT_meshopt_compression` text (KhronosGroup/glTF
//! 5ccac1cb, Appendix A) requires header `0xa0` for ATTRIBUTES, `0xe1` for TRIANGLES
//! and `0xd1` for INDICES. The linked meshoptimizer decoder also accepts `0xa1`,
//! `0xe0` and `0xd0`, so those streams are proved decodable here before C1 must
//! still refuse them.
//!
//! Stream literals are copied from the pinned upstream
//! zeux/meshoptimizer@3d8e9b8a2a2b9a5becfc6fbc512307b04207ab5d js/meshopt_decoder.test.js
//! (cases named on each constant); `SEQUENCE_V0` alone is a header substitution.
use super::*;
use crate::content_integrity::meshopt::{self, Mode};

/// `decodeVertexBuffer`: header 0xa0, count 4, stride 12.
const VERTEX_V0: &[u8] = &[
    0xa0, 0x01, 0x3f, 0x00, 0x00, 0x00, 0x58, 0x57, 0x58, 0x01, 0x26, 0x00, 0x00, 0x00, 0x01, 0x0c,
    0x00, 0x00, 0x00, 0x58, 0x01, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x3f, 0x00,
    0x00, 0x00, 0x17, 0x18, 0x17, 0x01, 0x26, 0x00, 0x00, 0x00, 0x01, 0x0c, 0x00, 0x00, 0x00, 0x17,
    0x01, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00,
];
/// `decodeVertexBufferV1`: header 0xa1, same count, stride and expected bytes.
const VERTEX_V1: &[u8] = &[
    0xa1, 0xee, 0xaa, 0xee, 0x00, 0x4b, 0x4b, 0x4b, 0x00, 0x00, 0x4b, 0x00, 0x00, 0x7d, 0x7d, 0x7d,
    0x00, 0x00, 0x7d, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x62, 0x00, 0x62,
];
const VERTEX_EXPECTED: &[u8] = &[
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 44, 1, 0, 0, 0, 0, 0, 0, 244, 1, 0, 0, 0, 0, 44, 1, 0, 0,
    0, 0, 0, 0, 244, 1, 44, 1, 44, 1, 0, 0, 0, 0, 244, 1, 244, 1,
];
/// `decodeIndexBufferV1_More`: header 0xe1, 12 indices, stride 4.
const TRIANGLES_V1: &[u8] = &[
    0xe1, 0xf0, 0x10, 0xfe, 0xff, 0xf0, 0x0c, 0xff, 0x02, 0x02, 0x02, 0x00, 0x76, 0x87, 0x56, 0x67,
    0x78, 0xa9, 0x86, 0x65, 0x89, 0x68, 0x98, 0x01, 0x69, 0x00, 0x00,
];
/// `decodeIndexBuffer32`: header 0xe0, same body, count, stride and expected indices.
const TRIANGLES_V0: &[u8] = &[
    0xe0, 0xf0, 0x10, 0xfe, 0xff, 0xf0, 0x0c, 0xff, 0x02, 0x02, 0x02, 0x00, 0x76, 0x87, 0x56, 0x67,
    0x78, 0xa9, 0x86, 0x65, 0x89, 0x68, 0x98, 0x01, 0x69, 0x00, 0x00,
];
const TRIANGLES_EXPECTED: [u32; 12] = [0, 1, 2, 2, 1, 3, 4, 6, 5, 7, 8, 9];
/// `decodeIndexSequence`: header 0xd1, 6 indices, stride 4.
const SEQUENCE_V1: &[u8] = &[
    0xd1, 0x00, 0x04, 0xcd, 0x01, 0x04, 0x07, 0x98, 0x1f, 0x00, 0x00, 0x00, 0x00,
];
/// `SEQUENCE_V1` with header 0xd0; the linked decoder reads the body identically.
const SEQUENCE_V0: &[u8] = &[
    0xd0, 0x00, 0x04, 0xcd, 0x01, 0x04, 0x07, 0x98, 0x1f, 0x00, 0x00, 0x00, 0x00,
];
const SEQUENCE_EXPECTED: [u32; 6] = [0, 1, 51, 2, 49, 1000];

fn words(values: &[u32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}
fn token(mode: Mode) -> &'static str {
    match mode {
        Mode::Attributes => "ATTRIBUTES",
        Mode::Triangles => "TRIANGLES",
        Mode::Indices => "INDICES",
    }
}
/// The C1 owner path: extension record, mode/stride/count checks, H-1, decode.
fn owner(stream: &[u8], count: usize, stride: usize, mode: Mode) -> FormatResult<Vec<u8>> {
    let source = format!(
        r#"{{"buffer":0,"byteLength":{},"count":{count},"byteStride":{stride},"mode":"{}"}}"#,
        stream.len(),
        token(mode)
    );
    let admitted = json::admit(
        source.as_bytes(),
        super::super::JsonLimits {
            bytes: 512,
            depth: 64,
            value_nodes: 128,
        },
    )
    .unwrap();
    let extension: records::Meshopt<'_> = record(Some(admitted.raw())).unwrap();
    let buffers = [Cow::Borrowed(stream)];
    decode_meshopt(&extension, 0, &buffers, 4096).map(|(out, _, _)| out)
}
fn is_header_error(result: &FormatResult<Vec<u8>>) -> bool {
    matches!(result, Err(FormatError::InvalidInput(m)) if m.starts_with("meshopt stream header"))
}
/// (mode, allowed stream, forbidden decodable twin, count, stride, expected bytes)
type Twin = (Mode, &'static [u8], &'static [u8], usize, usize, Vec<u8>);
fn twins() -> [Twin; 3] {
    [
        (
            Mode::Attributes,
            VERTEX_V0,
            VERTEX_V1,
            4,
            12,
            VERTEX_EXPECTED.to_vec(),
        ),
        (
            Mode::Triangles,
            TRIANGLES_V1,
            TRIANGLES_V0,
            12,
            4,
            words(&TRIANGLES_EXPECTED),
        ),
        (
            Mode::Indices,
            SEQUENCE_V1,
            SEQUENCE_V0,
            6,
            4,
            words(&SEQUENCE_EXPECTED),
        ),
    ]
}

#[test]
fn every_header_byte_is_checked_against_the_one_pinned_byte_per_mode() {
    for (mode, allowed) in [
        (Mode::Attributes, 0xa0u8),
        (Mode::Triangles, 0xe1),
        (Mode::Indices, 0xd1),
    ] {
        assert_eq!(mode.header(), allowed);
        for byte in 0..=u8::MAX {
            let result = meshopt::check_header(&[byte, 0, 0, 0], mode);
            if byte == allowed {
                assert!(result.is_ok(), "{} 0x{byte:02x}", token(mode));
            } else {
                assert!(
                    matches!(&result, Err(FormatError::InvalidInput(m)) if m.starts_with("meshopt stream header")),
                    "{} 0x{byte:02x}: {result:?}",
                    token(mode)
                );
            }
        }
        assert!(matches!(
            meshopt::check_header(&[], mode),
            Err(FormatError::InvalidInput(m)) if m.starts_with("meshopt stream header")
        ));
    }
}

#[test]
fn allowed_upstream_streams_still_decode_to_the_upstream_bytes() {
    for (mode, allowed, _, count, stride, expected) in twins() {
        assert_eq!(
            owner(allowed, count, stride, mode).unwrap(),
            expected,
            "{}",
            token(mode)
        );
    }
}

#[test]
fn forbidden_decodable_versions_are_refused_before_the_decoder() {
    for (mode, _, forbidden, count, stride, expected) in twins() {
        // The linked decoder accepts the forbidden version and yields the twin's bytes,
        // so only H-1 can refuse it.
        assert_eq!(
            meshopt::decode(forbidden, count, stride, mode).unwrap(),
            expected
        );
        let result = owner(forbidden, count, stride, mode);
        assert!(is_header_error(&result), "{}: {result:?}", token(mode));
    }
}

#[test]
fn unknown_versions_and_other_mode_headers_fail_as_header_errors() {
    for (mode, allowed, _, count, stride, _) in twins() {
        for byte in [
            allowed[0] + 1,
            0x0f,
            0xff,
            0xa0,
            0xa1,
            0xe0,
            0xe1,
            0xd0,
            0xd1,
        ] {
            if byte == allowed[0] {
                continue;
            }
            let mut stream = allowed.to_vec();
            stream[0] = byte;
            let result = owner(&stream, count, stride, mode);
            assert!(
                is_header_error(&result),
                "{} 0x{byte:02x}: {result:?}",
                token(mode)
            );
        }
    }
}
