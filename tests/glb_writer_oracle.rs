//! Byte oracle for `tests/support::write_glb`, the test writer that replaced
//! the removed legacy `glb_write` in fixtures. The committed output digests pin
//! only the textured, 16-bit-index path. This pins the 32-bit-index and
//! no-normals paths by sha256. The digests were checked byte-identical against
//! the legacy writer before it was deleted.

mod support;

use support::{write_glb, Primitive};

/// An n×n quad grid with a mild, deterministic relief.
fn grid(n: usize, normals: bool, uvs: bool) -> Primitive {
    let mut primitive = Primitive::default();
    for j in 0..=n {
        for i in 0..=n {
            let (u, v) = (i as f32 / n as f32, j as f32 / n as f32);
            primitive
                .positions
                .push([u * 20.0, v * 20.0, (u * 7.0).sin() * (v * 5.0).cos() * 0.5]);
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

/// Writes one grid with the test writer and asserts the pinned digest.
fn check(name: &str, normals: bool, textured: bool, n: usize, expected: &str) {
    let mut primitive = grid(n, normals, textured);
    primitive.png = textured.then(png);
    let ours = write_glb(&primitive);
    assert_eq!(sha256(&ours), expected, "{name}: pinned digest");
}

#[test]
fn u32_index_glb_matches_pinned_digest() {
    // 257 x 257 = 66049 vertices: more than 65535, so indices are u32.
    check(
        "u32 normals",
        true,
        false,
        256,
        "73377ee6c264d9cdd0c5cfd3ddc1fb79989a76c0139f9166b852b0f07a00cc38",
    );
    check(
        "u32 normals textured",
        true,
        true,
        256,
        "1598a16fab5188797a582fa8cfc33ecaf01186a45e141451d5d69e87405ed9a3",
    );
}

#[test]
fn no_normals_glb_matches_pinned_digest() {
    check(
        "no normals",
        false,
        false,
        8,
        "2199a0f0fb1c8e9af46358b03134a79d1fc328ed211683a241e2d3d947479659",
    );
    check(
        "no normals textured",
        false,
        true,
        8,
        "bd3fa72c5973fda45d5b1393bda2ec80f401c92b5858bc2f8a1b3f733c7e2c0c",
    );
    check(
        "no normals u32",
        false,
        false,
        256,
        "d5efeab31331a7f2622b66cd2b1a94927a50b0902dabfaff1fff9868f07bfc04",
    );
}
