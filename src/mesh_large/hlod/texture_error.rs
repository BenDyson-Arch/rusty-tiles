//! Texture-aware published G (D5 amendment A3, owner 2026-11-04).
//!
//! CesiumJS refines on a tile's published G alone (maximum SSE 16), so a coarse tile whose
//! page is far coarser than its G implies (the D5 page cap) is drawn blurry beside sharp
//! leaves and never refined. A profile with a texture term (`t` screen pixels per base
//! texel at SSE 16) publishes, for each coarse tile T,
//!
//! ```text
//! G'(T) = max(G(T), 16·s(T)/t · (1 + 2·SLACK) if T is refinable, max G'(child))
//! s(X)  = sqrt(A / X)   over X's textured triangles (base-colour page)
//! ```
//!
//! where A is their decoded surface area and X their base-level texel area (UV area times
//! the page's width × height), both measured from the published payload bytes
//! ([`super::decode::texture_sums`]). T is refinable when s(T) > r · s(children), the
//! children's sums pooled: refining actually delivers finer texels. Where the source
//! limits the children as much as T, nothing is raised. G is an upper bound, so raising it
//! stays honest; geometric G still drives every construction decision (simplification,
//! root competition, D5 spacing, honest-G sampling, seams, caps). The rule and `t`, `r`
//! are declared in `asset.extras.rustyTiles.textureError`, so validators recompute it
//! from archive content alone.
use crate::{JobError, JobErrorKind};
use serde::{Deserialize, Serialize};

/// CesiumJS's default maximum screen-space error.
pub(crate) const SSE: f64 = 16.;
/// Arithmetic slack between the producer's f32 client-order decode and a validator's
/// own decode: validators compare G at `(1 - SLACK)` and call a tile refinable only above
/// `r·(1 + SLACK)`; the producer publishes `2·SLACK` above the term.
pub(crate) const SLACK: f64 = 1e-4;
/// Refinable: children's pooled spacing finer than the tile's by more than this factor.
pub(crate) const REFINABLE_RATIO: f64 = 1.25;

/// The declared rule (`textureError` version 1).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Rule {
    pub version: u32,
    pub px_per_texel: f64,
    pub refinable_ratio: f64,
}

impl Rule {
    pub(crate) fn new(px_per_texel: f64) -> Self {
        Self {
            version: 1,
            px_per_texel,
            refinable_ratio: REFINABLE_RATIO,
        }
    }
}

/// Textured surface area (m²) and base-level texel area of a payload, or pooled payloads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Sums {
    pub area: f64,
    pub texels: f64,
}

impl Sums {
    pub(crate) fn add_triangle(&mut self, p: [[f32; 3]; 3], uv: [[f64; 2]; 3], texels: f64) {
        let p = p.map(|v| v.map(f64::from));
        let e1: [f64; 3] = std::array::from_fn(|a| p[1][a] - p[0][a]);
        let e2: [f64; 3] = std::array::from_fn(|a| p[2][a] - p[0][a]);
        let n = [
            e1[1] * e2[2] - e1[2] * e2[1],
            e1[2] * e2[0] - e1[0] * e2[2],
            e1[0] * e2[1] - e1[1] * e2[0],
        ];
        self.area += 0.5 * (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        let d = (uv[1][0] - uv[0][0]) * (uv[2][1] - uv[0][1])
            - (uv[1][1] - uv[0][1]) * (uv[2][0] - uv[0][0]);
        self.texels += 0.5 * d.abs() * texels;
    }

    pub(crate) fn add(self, other: Self) -> Self {
        Self {
            area: self.area + other.area,
            texels: self.texels + other.texels,
        }
    }

    /// Mean texel spacing in metres; `None` without textured surface.
    pub(crate) fn spacing(self) -> Option<f64> {
        (self.area > 0. && self.texels > 0.).then(|| (self.area / self.texels).sqrt())
    }
}

/// Base-level (width, height) of a KTX2 or PNG page.
pub(crate) fn image_size(bytes: &[u8]) -> Result<[u32; 2], JobError> {
    let word = |at: usize, big: bool| {
        bytes.get(at..at + 4).map(|b| {
            let b: [u8; 4] = b.try_into().unwrap();
            if big {
                u32::from_be_bytes(b)
            } else {
                u32::from_le_bytes(b)
            }
        })
    };
    let size = if bytes.starts_with(b"\xabKTX 20\xbb\r\n\x1a\n") {
        word(20, false).zip(word(24, false))
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") && bytes.get(12..16) == Some(b"IHDR") {
        word(16, true).zip(word(20, true))
    } else {
        None
    };
    size.map(|(w, h)| [w, h])
        .filter(|s| s[0] > 0 && s[1] > 0)
        .ok_or_else(|| {
            JobError::new(
                JobErrorKind::InvalidState,
                "base-colour page is not a KTX2 or PNG image",
            )
        })
}

/// One coarse tile's published error and the facts behind it (the `proxies.jsonl` row).
#[derive(Clone, Copy, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Published {
    /// S2's geometric G, which every construction decision used.
    pub geometric_error: f64,
    /// Largest published G among the children.
    pub children_published: f64,
    pub sums: Sums,
    pub children_sums: Sums,
    /// `16·s/t` with the producer margin, when refinable (0 otherwise).
    pub texture_error: f64,
    pub refinable: bool,
    /// The published G: max of the three.
    pub error: f64,
}

impl Published {
    /// The texture term set the published G (above the geometric G and the children's).
    pub(crate) fn by_texture(&self) -> bool {
        self.texture_error > self.geometric_error && self.texture_error >= self.children_published
    }
    /// Raised only to stay at or above a child's published G.
    pub(crate) fn by_children(&self) -> bool {
        self.children_published > self.geometric_error.max(self.texture_error)
    }
}

/// G' of a coarse tile with geometric `g`, its own sums and its children's
/// (published G, sums). Without a rule, G' is `max(g, children's G')`.
pub(crate) fn publish(
    rule: Option<Rule>,
    g: f64,
    sums: Sums,
    children: impl IntoIterator<Item = (f64, Sums)>,
) -> Published {
    let (children_published, children_sums) = children
        .into_iter()
        .fold((0f64, Sums::default()), |(e, s), (ce, cs)| (e.max(ce), s.add(cs)));
    let (refinable, texture_error) = match (rule, sums.spacing(), children_sums.spacing()) {
        (Some(rule), Some(s), Some(sc)) if s > rule.refinable_ratio * sc => (
            true,
            SSE * s / rule.px_per_texel * (1. + 2. * SLACK),
        ),
        _ => (false, 0.),
    };
    Published {
        geometric_error: g,
        children_published,
        sums,
        children_sums,
        texture_error,
        refinable,
        error: g.max(texture_error).max(children_published),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sums(area: f64, texels: f64) -> Sums {
        Sums { area, texels }
    }

    /// A page-capped tile over finer children takes 16·s/t; a source-limited one (children
    /// within the refinable ratio) keeps geometric G; no rule (fast) publishes geometric G;
    /// a child's G' always carries upward.
    #[test]
    fn refinable_tiles_take_the_texture_term_and_children_carry_upward() {
        let rule = Some(Rule::new(2.));
        // s = 0.25 m against children at 0.0625 m: refinable.
        let p = publish(rule, 0.1, sums(64., 1024.), [(0.05, sums(64., 16384.))]);
        assert!(p.refinable && p.by_texture() && !p.by_children());
        assert!((p.error / (16. * 0.25 / 2.) - 1. - 2. * SLACK).abs() < 1e-12);
        // Children within 1.25x: source-limited, nothing raised.
        let p = publish(rule, 0.1, sums(64., 1024.), [(0.05, sums(64., 1500.))]);
        assert!(!p.refinable && p.error == 0.1);
        assert!(publish(None, 0.1, sums(64., 1024.), [(0.05, sums(64., 16384.))]).error == 0.1);
        // A child's G' above everything else sets the parent's.
        let p = publish(rule, 0.1, sums(64., 1024.), [(9., sums(64., 1500.))]);
        assert!(p.by_children() && p.error == 9.);
        // Untextured children or tile: nothing to compare.
        assert!(!publish(rule, 0.1, Sums::default(), [(0., sums(1., 1.))]).refinable);
        assert!(!publish(rule, 0.1, sums(1., 1.), [(0., Sums::default())]).refinable);
    }

    #[test]
    fn page_sizes_come_from_ktx2_and_png_headers_only() {
        let mut ktx = b"\xabKTX 20\xbb\r\n\x1a\n".to_vec();
        ktx.extend([0u8; 8]);
        ktx.extend(512u32.to_le_bytes());
        ktx.extend(256u32.to_le_bytes());
        assert_eq!(image_size(&ktx).unwrap(), [512, 256]);
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        png.extend(64u32.to_be_bytes());
        png.extend(32u32.to_be_bytes());
        assert_eq!(image_size(&png).unwrap(), [64, 32]);
        assert!(image_size(b"\xff\xd8\xff\xe0").is_err());
        assert!(image_size(&png[..20]).is_err());
    }

    #[test]
    fn sums_measure_area_and_texels() {
        let mut s = Sums::default();
        s.add_triangle(
            [[0., 0., 0.], [2., 0., 0.], [0., 2., 0.]],
            [[0., 0.], [0.5, 0.], [0., 0.5]],
            1024. * 1024.,
        );
        assert_eq!(s.area, 2.);
        assert_eq!(s.texels, 0.125 * 1024. * 1024.);
        assert_eq!(s.spacing(), Some((2. / s.texels).sqrt()));
    }
}
