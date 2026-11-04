//! D5 amendment A3: texture-aware published G, re-derived from archive content alone.
//!
//! A tileset declaring `asset.extras.rustyTiles.textureError` {version 1, pxPerTexel t,
//! refinableRatio r} promises, for every content tile T,
//! `G(T) >= 16·s(T)/t` whenever `s(T) > r·s(C(T))`. `s(X) = sqrt(A/X)` over the textured
//! triangles of X's payloads: A their decoded surface area in the tileset frame, X their
//! base-colour texel area (UV area times the KTX2/PNG base level's width × height).
//! C(T) are T's nearest content descendants, looking through content-less tiles and
//! copies of T's own payload down a single-child chain, pooled. Sums are taken once per
//! payload at ingest; one preorder pass over tiles and references checks every tile.
//! No producer receipt is read.
use super::{bad, decode, identity, sql, Accessor, Effort, Error, Matrix, Point, Store};

/// Producer arithmetic slack (the producer's `texture_error::SLACK`): G is compared at
/// `(1 - SLACK)`, refinability needs `s(T) > r·(1 + SLACK)·s(C)`.
const SLACK: f64 = 1e-4;
/// Units per textured triangle at ingest: three UV reads and the two areas.
pub(super) const TRIANGLE_UNITS: usize = 6;

/// The declared rule.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::validate) struct Rule {
    pub px_per_texel: f64,
    pub refinable_ratio: f64,
}

impl Rule {
    /// `asset.extras.rustyTiles.textureError`, if declared.
    pub(in crate::validate) fn declared(tileset: &serde_json::Value) -> Result<Option<Self>, Error> {
        let Some(value) = tileset["asset"]["extras"]["rustyTiles"].get("textureError") else {
            return Ok(None);
        };
        let number = |k: &str| value[k].as_f64().filter(|v| v.is_finite() && *v >= 1.);
        match (value["version"].as_u64(), number("pxPerTexel"), number("refinableRatio")) {
            (Some(1), Some(px_per_texel), Some(refinable_ratio)) => Ok(Some(Self {
                px_per_texel,
                refinable_ratio,
            })),
            _ => Err(bad(
                "textureError",
                "declaration is not version 1 with finite pxPerTexel and refinableRatio >= 1",
            )),
        }
    }
}

/// Per material of one payload: its base-colour texels per unit UV area and TEXCOORD set.
pub(super) fn pages(
    doc: &serde_json::Value,
    bin: &[u8],
    work: &mut identity::Work,
) -> Result<Vec<Option<(f64, u64)>>, Error> {
    let mut out = Vec::new();
    for material in doc["materials"].as_array().into_iter().flatten() {
        work.charge(4)?;
        let Some(base) = material["pbrMetallicRoughness"].get("baseColorTexture") else {
            out.push(None);
            continue;
        };
        if base.get("extensions").is_some() {
            return Err(Error::unsupported(
                "textureError: base-colour texture transform",
            ));
        }
        let texture = &doc["textures"][identity::uint(&base["index"])?];
        let image = texture["extensions"]["KHR_texture_basisu"]
            .get("source")
            .or_else(|| texture.get("source"))
            .ok_or_else(|| bad("textureError", "base-colour texture without image"))?;
        let view = &doc["bufferViews"][identity::uint(&doc["images"][identity::uint(image)?]["bufferView"])?];
        let start = view["byteOffset"].as_u64().unwrap_or(0) as usize;
        let head = identity::slice(bin, start, 28.min(identity::uint(&view["byteLength"])?))?;
        let size = if head.starts_with(b"\xabKTX 20\xbb\r\n\x1a\n") && head.len() >= 28 {
            let w = |i: usize| u32::from_le_bytes(head[i..i + 4].try_into().unwrap());
            Some((w(20), w(24)))
        } else if head.starts_with(b"\x89PNG\r\n\x1a\n") && head.len() >= 24 && &head[12..16] == b"IHDR" {
            let w = |i: usize| u32::from_be_bytes(head[i..i + 4].try_into().unwrap());
            Some((w(16), w(20)))
        } else {
            None
        };
        let (w, h) = size.filter(|(w, h)| *w > 0 && *h > 0).ok_or_else(|| {
            Error::unsupported("textureError: base-colour page is not a KTX2 or PNG image")
        })?;
        out.push(Some((
            f64::from(w) * f64::from(h),
            base["texCoord"].as_u64().unwrap_or(0),
        )));
    }
    Ok(out)
}

/// Textured surface area and texel area of one primitive's triangles.
pub(super) struct Sums<'a> {
    pub uv: Accessor,
    pub texels: f64,
    pub area: &'a mut f64,
    pub texel_area: &'a mut f64,
}

impl Sums<'_> {
    pub(super) fn triangle(
        &mut self,
        ids: [usize; 3],
        xyz: [Point; 3],
        work: &mut identity::Work,
    ) -> Result<(), Error> {
        work.charge(TRIANGLE_UNITS)?;
        let e1: Point = std::array::from_fn(|a| xyz[1][a] - xyz[0][a]);
        let e2: Point = std::array::from_fn(|a| xyz[2][a] - xyz[0][a]);
        let n = [
            e1[1] * e2[2] - e1[2] * e2[1],
            e1[2] * e2[0] - e1[0] * e2[2],
            e1[0] * e2[1] - e1[1] * e2[0],
        ];
        *self.area += 0.5 * (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        let mut uv = [[0.; 2]; 3];
        for (j, &id) in ids.iter().enumerate() {
            uv[j] = [self.uv.at(id, 0)?, self.uv.at(id, 1)?];
        }
        let d = (uv[1][0] - uv[0][0]) * (uv[2][1] - uv[0][1])
            - (uv[1][1] - uv[0][1]) * (uv[2][0] - uv[0][0]);
        *self.texel_area += 0.5 * d.abs() * self.texels;
        Ok(())
    }
}

/// The similarity scale of a tile's placement; other placements are outside the rule.
fn scale(world: Matrix) -> Option<f64> {
    let c: [Point; 3] = std::array::from_fn(|k| std::array::from_fn(|i| world[k * 4 + i]));
    let dot = |a: Point, b: Point| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let n = c.map(|v| dot(v, v));
    let tol = 1e-9 * n[0];
    if (n[1] - n[0]).abs() > tol
        || (n[2] - n[0]).abs() > tol
        || [(0, 1), (0, 2), (1, 2)]
            .iter()
            .any(|&(i, j)| dot(c[i], c[j]).abs() > tol)
    {
        return None;
    }
    Some(n[0].sqrt())
}

#[derive(Clone, Copy, Default)]
struct Pool {
    area: f64,
    texels: f64,
}
impl Pool {
    fn spacing(self) -> Option<f64> {
        (self.area > 0. && self.texels > 0.).then(|| (self.area / self.texels).sqrt())
    }
}

/// (refinable tiles, tiles whose G the texture term bounds within 1%).
impl Store {
    pub(super) fn texture_error(&self, rule: Rule, effort: &Effort) -> Result<(u64, u64), Error> {
        struct Frame {
            id: i64,
            error: f64,
            /// The placement's similarity scale (`None`: not a similarity).
            scale: Option<f64>,
            /// Sorted payload digests (empty: no content).
            digest: Vec<Vec<u8>>,
            own: Pool,
            below: Pool,
            /// Stack index of the nearest content ancestor.
            up: Option<usize>,
        }
        let mut counts = (0u64, 0u64);
        let mut close = |frame: Frame, stack: &mut [Frame]| -> Result<(), Error> {
            if frame.digest.is_empty() {
                return Ok(());
            }
            if let (Some(s), Some(sc)) = (frame.own.spacing(), frame.below.spacing()) {
                if s > rule.refinable_ratio * (1. + SLACK) * sc {
                    counts.0 += 1;
                    let need = 16. * s / rule.px_per_texel;
                    if frame.error < need * (1. - SLACK) {
                        return Err(bad(
                            "textureError",
                            &format!(
                                "tile {} geometricError {} is below its texture-implied {need} \
                                 (spacing {s} m, children {sc} m, t {})",
                                frame.id, frame.error, rule.px_per_texel
                            ),
                        ));
                    }
                    counts.1 += u64::from(frame.error <= need * 1.01);
                }
            }
            // Contribute to the nearest content ancestor and to its copies above it,
            // unless this tile is itself a copy of that ancestor's payload.
            let mut at = frame.up;
            if let Some(a) = at {
                if stack[a].digest == frame.digest {
                    at = None;
                }
            }
            let chain = at.map(|a| stack[a].digest.clone());
            while let Some(a) = at {
                effort.charge(1)?;
                if Some(&stack[a].digest) != chain.as_ref() {
                    break;
                }
                stack[a].below.area += frame.own.area;
                stack[a].below.texels += frame.own.texels;
                at = stack[a].up;
            }
            Ok(())
        };
        let mut query = self
            .connection
            .prepare("SELECT t.id,t.parent,t.error,t.world,p.digest,p.texture_area,p.texture_texels FROM tiles t LEFT JOIN refs r ON r.tile=t.id LEFT JOIN payloads p ON p.rank=r.rank ORDER BY t.id,p.digest")
            .map_err(sql)?;
        let mut rows = query.query([]).map_err(sql)?;
        let mut stack = Vec::<Frame>::new();
        while let Some(row) = rows.next().map_err(sql)? {
            effort.charge(1)?;
            let id: i64 = row.get(0).map_err(sql)?;
            if stack.last().map(|f| f.id) != Some(id) {
                let parent: Option<i64> = row.get(1).map_err(sql)?;
                while stack.last().map(|f| f.id) != parent {
                    let frame = stack.pop().ok_or_else(|| {
                        Error::InvalidState("HLOD hierarchy is not in preorder".into())
                    })?;
                    close(frame, &mut stack)?;
                }
                let up = stack
                    .len()
                    .checked_sub(1)
                    .and_then(|p| if stack[p].digest.is_empty() { stack[p].up } else { Some(p) });
                let world: Vec<u8> = row.get(3).map_err(sql)?;
                stack.push(Frame {
                    id,
                    error: row.get::<_, Option<f64>>(2).map_err(sql)?.unwrap_or(0.),
                    scale: scale(decode::<16>(&world)?),
                    digest: Vec::new(),
                    own: Pool::default(),
                    below: Pool::default(),
                    up,
                });
            }
            let Some(digest) = row.get::<_, Option<Vec<u8>>>(4).map_err(sql)? else {
                continue;
            };
            let frame = stack.last_mut().expect("frame pushed above");
            let area: f64 = row.get(5).map_err(sql)?;
            let texels: f64 = row.get(6).map_err(sql)?;
            if texels > 0. {
                let scale = frame.scale.ok_or_else(|| {
                    Error::unsupported("textureError: a textured tile placement is not a similarity")
                })?;
                frame.own.area += area * scale * scale;
                frame.own.texels += texels;
            }
            frame.digest.push(digest);
        }
        while let Some(frame) = stack.pop() {
            close(frame, &mut stack)?;
        }
        Ok(counts)
    }
}
