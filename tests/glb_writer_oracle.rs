//! Byte oracle for `tests/support::write_glb`, the test writer that replaced
//! the legacy `glb_write` in fixtures. The committed output digests pin only
//! the textured, 16-bit-index path. This pins the 32-bit-index and no-normals
//! paths: against the legacy writer while it exists, and by sha256 so the
//! pinned half survives its deletion (the deletion PR drops the comparison
//! and keeps the hashes).

mod support;

use rusty_tiles::glb_write::{write_glb as legacy_write_glb, TilePrimitive};
use support::{write_glb, Primitive};

/// An n×n quad grid with a mild relief built from +, - and * only: IEEE basic
/// operations round identically on every platform, while libm `sin`/`cos` do not,
/// and the pinned digests must hold on every CI target.
fn grid(n: usize, normals: bool, uvs: bool) -> Primitive {
    let mut primitive = Primitive::default();
    for j in 0..=n {
        for i in 0..=n {
            let (u, v) = (i as f32 / n as f32, j as f32 / n as f32);
            primitive
                .positions
                .push([u * 20.0, v * 20.0, u * (1.0 - u) * v * (1.0 - v) * 8.0]);
            if uvs {
                primitive.uvs.push([u, v]);
            }
        }
    }
    for j in 0..n {
        for i in 0..n {
            let a = (j * (n + 1) + i) as u32;
            let c = a + (n + 1) as u32;
            primitive.indices.extend([a, a + 1, c, a + 1, c + 1, c]);
        }
    }
    if normals {
        primitive.normals = vec![[0.0, 0.0, 1.0]; primitive.positions.len()];
    }
    primitive
}

fn png() -> Vec<u8> {
    let texture = image::RgbaImage::from_fn(8, 8, |x, y| {
        image::Rgba([(x * 30) as u8, (y * 30) as u8, 7, 255])
    });
    let mut bytes = Vec::new();
    image::DynamicImage::ImageRgba8(texture)
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .unwrap();
    bytes
}

fn sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}

/// Writes one grid with the test writer and the legacy writer, asserts
/// identical bytes, then asserts the pinned digest.
fn check(name: &str, normals: bool, textured: bool, n: usize, expected: &str) {
    let mut primitive = grid(n, normals, textured);
    primitive.png = textured.then(png);
    let ours = write_glb(&primitive);
    let legacy = legacy_write_glb(&[TilePrimitive {
        positions: primitive.positions,
        normals: primitive.normals,
        uvs: primitive.uvs,
        indices: primitive.indices,
        jpeg: primitive.png,
    }])
    .unwrap();
    assert_eq!(
        ours, legacy,
        "{name}: test writer differs from legacy writer"
    );
    assert_eq!(sha256(&ours), expected, "{name}: pinned digest");
}

#[test]
fn u32_index_glb_matches_legacy_writer_and_pinned_digest() {
    // 257 x 257 = 66049 vertices: more than 65535, so indices are u32.
    check(
        "u32 normals",
        true,
        false,
        256,
        "285f75e6a85e752d95ce8c41fa3bd439839f96c505fa34601fc66b56a3a50c6d",
    );
    check(
        "u32 normals textured",
        true,
        true,
        256,
        "3303e5626fe4eb1ea4a27211e3dddeab9f2f1d02a8f8a0592160204ab1af33a5",
    );
}

#[test]
fn no_normals_glb_matches_legacy_writer_and_pinned_digest() {
    check(
        "no normals",
        false,
        false,
        8,
        "2a6369c63488f795248ba126f75df279f7fb96285f9af2a38376b9f7e5322137",
    );
    check(
        "no normals textured",
        false,
        true,
        8,
        "19c5879cd1bc7a16965644c4825943adc2d7a60083abb7def942f17c0c4dc560",
    );
    check(
        "no normals u32",
        false,
        false,
        256,
        "242275b15259fe104169c55e00c6ddad63d6fd3175feffaa94eaff7df55a9b1c",
    );
}
