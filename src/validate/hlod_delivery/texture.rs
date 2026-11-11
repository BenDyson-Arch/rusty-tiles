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
use sha2::{Digest, Sha256};

/// Producer arithmetic slack (the producer's `texture_error::SLACK`): G is compared at
/// `(1 - SLACK)`, refinability needs `s(T) > r·(1 + SLACK)·s(C)`.
const SLACK: f64 = 1e-4;
/// Units per textured triangle at ingest: three UV reads and the two areas (calibrated by
/// `texture_work_cost_probe`).
pub(super) const TRIANGLE_UNITS: usize = 6;
/// Units per tile/reference row of the preorder pass and per ancestor contribution.
pub(super) const ROW_UNITS: usize = 48;

/// The declared rule (`version` 1). `refinableRatio` is pinned to 1.25 and `pxPerTexel` to a
/// profile value: 2 (balanced), 1.25 (quality, lossless) or null (fast: no texture term).
pub(super) const REFINABLE_RATIO: f64 = 1.25;
const PX_PER_TEXEL: [f64; 2] = [2., 1.25];

#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::validate) struct Rule {
    pub px_per_texel: f64,
    pub refinable_ratio: f64,
}

/// What the archive declares about the texture term.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(in crate::validate) enum Declaration {
    /// No declaration (and the tileset does not itself declare HLOD): any HLOD payload
    /// with a base-colour texture is refused at ingest.
    #[default]
    Absent,
    /// `pxPerTexel: null`: the fast form, no texture term.
    Fast,
    Rule(Rule),
}

impl Declaration {
    /// `asset.extras.rustyTiles.textureError`, strict: exactly `{version: 1, pxPerTexel:
    /// 2 | 1.25 | null, refinableRatio: 1.25}`. `hlod` is the resolved expectation (caller,
    /// tileset or conversion report); a declaration outside it is refused, and a tileset
    /// declaring `lod: "hlod"` must declare it. `profile` is the conversion report's
    /// `mesh_tiler` profile, if any: fast must declare null, balanced 2, quality and
    /// lossless 1.25.
    pub(in crate::validate) fn read(
        tileset: &serde_json::Value,
        hlod: bool,
        profile: Option<&str>,
    ) -> Result<Self, Error> {
        let rusty = &tileset["asset"]["extras"]["rustyTiles"];
        let refuse = |why: &str| Err(bad("textureError", why));
        let value = match (hlod, rusty.get("textureError")) {
            (_, None) if rusty["lod"].as_str() == Some("hlod") => {
                return refuse("an HLOD tileset must declare its texture rule")
            }
            (_, None) => return Ok(Self::Absent),
            (false, Some(_)) => return refuse("declared outside an HLOD tileset"),
            (true, Some(v)) => v,
        };
        let Some(object) = value.as_object() else {
            return refuse("declaration is not an object");
        };
        if object.len() != 3
            || !["version", "pxPerTexel", "refinableRatio"]
                .iter()
                .all(|k| object.contains_key(*k))
        {
            return refuse("declaration must hold exactly version, pxPerTexel and refinableRatio");
        }
        if !value["version"].is_u64() || value["version"].as_u64() != Some(1) {
            return refuse("version must be the integer 1");
        }
        if value["refinableRatio"].as_f64() != Some(REFINABLE_RATIO) {
            return refuse("refinableRatio must be 1.25");
        }
        let declared = match &value["pxPerTexel"] {
            serde_json::Value::Null => None,
            v => match v.as_f64().filter(|t| PX_PER_TEXEL.contains(t)) {
                Some(t) => Some(t),
                None => return refuse("pxPerTexel must be 2, 1.25 or null"),
            },
        };
        let expected = match profile {
            Some("fast") => Some(None),
            Some("balanced") => Some(Some(2.)),
            Some("quality" | "lossless") => Some(Some(1.25)),
            _ => None,
        };
        if expected.is_some_and(|e| e != declared) {
            return refuse("pxPerTexel disagrees with the conversion report's profile");
        }
        Ok(match declared {
            None => Self::Fast,
            Some(px_per_texel) => Self::Rule(Rule {
                px_per_texel,
                refinable_ratio: REFINABLE_RATIO,
            }),
        })
    }
}

/// Whether a payload binds a base-colour texture on any material (an undeclared HLOD
/// archive must not).
pub(super) fn textured(doc: &serde_json::Value, work: &mut identity::Work) -> Result<bool, Error> {
    for material in doc["materials"].as_array().into_iter().flatten() {
        work.charge(1)?;
        if material["pbrMetallicRoughness"]
            .get("baseColorTexture")
            .is_some()
        {
            return Ok(true);
        }
    }
    Ok(false)
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
        let view = &doc["bufferViews"]
            [identity::uint(&doc["images"][identity::uint(image)?]["bufferView"])?];
        let start = view["byteOffset"].as_u64().unwrap_or(0) as usize;
        let head = identity::slice(bin, start, 28.min(identity::uint(&view["byteLength"])?))?;
        let size = if head.starts_with(b"\xabKTX 20\xbb\r\n\x1a\n") && head.len() >= 28 {
            let w = |i: usize| u32::from_le_bytes(head[i..i + 4].try_into().unwrap());
            Some((w(20), w(24)))
        } else if head.starts_with(b"\x89PNG\r\n\x1a\n")
            && head.len() >= 24
            && &head[12..16] == b"IHDR"
        {
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
    /// Textured area over no UV area is infinitely coarse, never untextured.
    fn spacing(self) -> Option<f64> {
        (self.area > 0. || self.texels > 0.).then(|| (self.area / self.texels).sqrt())
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
            /// The tile's payload set, interned as SHA-256 over its sorted payload digests
            /// once its rows are read (`None`: no content), so chain steps compare in O(1).
            key: Option<[u8; 32]>,
            hasher: Option<Sha256>,
            own: Pool,
            below: Pool,
            /// Stack index of the nearest content ancestor.
            up: Option<usize>,
        }
        let mut counts = (0u64, 0u64);
        fn seal(frame: &mut Frame) {
            if let Some(h) = frame.hasher.take() {
                frame.key = Some(h.finalize().into());
            }
        }
        let mut close = |mut frame: Frame, stack: &mut [Frame]| -> Result<(), Error> {
            seal(&mut frame);
            if frame.key.is_none() {
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
            let mut at = frame.up.filter(|&a| stack[a].key != frame.key);
            let chain = at.map(|a| stack[a].key);
            while let Some(a) = at {
                effort.charge(ROW_UNITS)?;
                if Some(stack[a].key) != chain {
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
            effort.charge(ROW_UNITS)?;
            let id: i64 = row.get(0).map_err(sql)?;
            if stack.last().map(|f| f.id) != Some(id) {
                if let Some(top) = stack.last_mut() {
                    seal(top);
                }
                let parent: Option<i64> = row.get(1).map_err(sql)?;
                while stack.last().map(|f| f.id) != parent {
                    let frame = stack.pop().ok_or_else(|| {
                        Error::InvalidState("HLOD hierarchy is not in preorder".into())
                    })?;
                    close(frame, &mut stack)?;
                }
                let up = stack.len().checked_sub(1).and_then(|p| {
                    if stack[p].key.is_none() {
                        stack[p].up
                    } else {
                        Some(p)
                    }
                });
                let world: Vec<u8> = row.get(3).map_err(sql)?;
                stack.push(Frame {
                    id,
                    error: row.get::<_, Option<f64>>(2).map_err(sql)?.unwrap_or(0.),
                    scale: scale(decode::<16>(&world)?),
                    key: None,
                    hasher: None,
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
            if area > 0. || texels > 0. {
                let scale = frame.scale.ok_or_else(|| {
                    Error::unsupported(
                        "textureError: a textured tile placement is not a similarity",
                    )
                })?;
                frame.own.area += area * scale * scale;
                frame.own.texels += texels;
            }
            frame.hasher.get_or_insert_with(Sha256::new).update(&digest);
        }
        while let Some(frame) = stack.pop() {
            close(frame, &mut stack)?;
        }
        Ok(counts)
    }
}

#[cfg(all(test, unix))]
mod cost {
    //! Retained work-cost probe (CPU-pinned calibration, no acceptance claim).
    use super::*;
    use rusqlite::params;
    use std::sync::atomic::Ordering;

    fn cpu() -> f64 {
        let mut t = std::mem::MaybeUninit::<libc::timespec>::uninit();
        // SAFETY: successful clock_gettime initializes the supplied timespec.
        assert_eq!(
            unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, t.as_mut_ptr()) },
            0
        );
        // SAFETY: the successful call above initialized t.
        let t = unsafe { t.assume_init() };
        t.tv_sec as f64 + t.tv_nsec as f64 * 1e-9
    }
    /// A complete octree of `levels` below the root, every tile with its own textured
    /// payload, coarse pages coarser than their children (every tile refinable).
    fn store(levels: u32) -> (Store, u64) {
        let mut s = super::super::tests::store();
        s.materialize().unwrap();
        let identity = super::super::encode(&IDENTITY_WORLD);
        let mut next = 0i64;
        fn add(
            s: &Store,
            id: &mut i64,
            parent: Option<i64>,
            level: u32,
            levels: u32,
            world: &[u8],
        ) {
            let me = *id;
            *id += 1;
            let error = 1000. / f64::from(1u32 << level);
            s.connection
                .execute(
                    "INSERT INTO tiles(id,parent,world,error) VALUES(?,?,?,?)",
                    params![me, parent, world, error],
                )
                .unwrap();
            s.connection
                .execute(
                    "INSERT INTO payloads(rank,identity_present,triangle_present,digest,texture_area,texture_texels) VALUES(?,1,0,?,1e4,?)",
                    params![me, me.to_le_bytes().to_vec(), 4f64.powi(level as i32 + 4)],
                )
                .unwrap();
            s.connection
                .execute("INSERT INTO refs VALUES(?,?)", params![me, me])
                .unwrap();
            if level < levels {
                for _ in 0..8 {
                    add(s, id, Some(me), level + 1, levels, world);
                }
            }
        }
        add(&s, &mut next, None, 0, levels, &identity);
        s.connection.execute_batch("CREATE INDEX tile_parent ON tiles(parent); CREATE INDEX refs_tile ON refs(tile); CREATE INDEX refs_rank ON refs(rank);").unwrap();
        (s, next as u64)
    }
    const IDENTITY_WORLD: [f64; 16] = [
        1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
    ];

    #[test]
    #[ignore = "CPU-pinned calibration; runs no geometry or archive acceptance"]
    fn texture_work_cost_probe() {
        let rule = Rule {
            px_per_texel: 2.,
            refinable_ratio: 1.25,
        };
        for levels in [3, 4, 5] {
            let (s, tiles) = store(levels);
            for _ in 0..3 {
                let effort = super::super::tests::effort_for_tests();
                let start = cpu();
                let (refinable, _) = s.texture_error(rule, &effort).unwrap();
                let ns = (cpu() - start) * 1e9;
                let units = effort.used.load(Ordering::Relaxed);
                assert_eq!(refinable + 8u64.pow(levels), tiles);
                println!(
                    "TEXTURE_COST {}",
                    serde_json::json!({"case":"pass","n":tiles,"ns":ns,"units":units,"nsPerUnit":ns/units as f64})
                );
            }
        }
        // Review r1 P2-2: a chain of `n` copies, each holding `n` references, over `n`
        // distinct content tiles: every content tile walks the whole chain.
        for n in [100i64, 400] {
            let mut s = super::super::tests::store();
            s.materialize().unwrap();
            let identity = super::super::encode(&IDENTITY_WORLD);
            let c = &s.connection;
            for j in 0..n {
                c.execute("INSERT INTO payloads(rank,identity_present,triangle_present,digest,texture_area,texture_texels) VALUES(?,1,0,?,1e4,1e6)",
                    params![j, [j.to_le_bytes().to_vec(), vec![7u8; 24]].concat()]).unwrap();
            }
            for i in 0..n {
                c.execute(
                    "INSERT INTO tiles(id,parent,world,error) VALUES(?,?,?,?)",
                    params![i, (i > 0).then(|| i - 1), &identity, 1000.0],
                )
                .unwrap();
                for j in 0..n {
                    c.execute("INSERT INTO refs VALUES(?,?)", params![i, j])
                        .unwrap();
                }
            }
            for q in 0..n {
                c.execute(
                    "INSERT INTO tiles(id,parent,world,error) VALUES(?,?,?,?)",
                    params![n + q, n - 1, &identity, 1000.0],
                )
                .unwrap();
                c.execute("INSERT INTO payloads(rank,identity_present,triangle_present,digest,texture_area,texture_texels) VALUES(?,1,0,?,1e4,1e8)",
                    params![n + q, [(n + q).to_le_bytes().to_vec(), vec![9u8; 24]].concat()]).unwrap();
                c.execute("INSERT INTO refs VALUES(?,?)", params![n + q, n + q])
                    .unwrap();
            }
            c.execute_batch("CREATE INDEX tile_parent ON tiles(parent); CREATE INDEX refs_tile ON refs(tile); CREATE INDEX refs_rank ON refs(rank);").unwrap();
            for _ in 0..3 {
                let mut effort = super::super::tests::effort_for_tests();
                effort.budget.value = u64::MAX;
                let start = cpu();
                s.texture_error(rule, &effort).unwrap();
                let ns = (cpu() - start) * 1e9;
                let units = effort.used.load(Ordering::Relaxed);
                println!(
                    "TEXTURE_COST {}",
                    serde_json::json!({"case":"chain","n":n,"ns":ns,"units":units,"nsPerUnit":ns/units as f64})
                );
            }
        }
        // Ingest: the same published coarse payload with and without the declaration.
        let Some(path) = std::env::var_os("RT_TEXTURE_PROBE_GLB") else {
            return;
        };
        let bytes = std::fs::read(path).unwrap();
        let budget = super::super::tests::budget();
        for _ in 0..3 {
            let mut runs = [(0f64, 0u64); 2];
            for (k, declared) in [false, true].into_iter().enumerate() {
                for rank in 0..20u64 {
                    let mut s = super::super::tests::store();
                    if declared {
                        s.expect_texture(Declaration::Rule(rule));
                    }
                    let start = cpu();
                    let units = s.ingest(rank, &bytes, budget, 0, 1 << 30).unwrap();
                    runs[k].0 += (cpu() - start) * 1e9;
                    runs[k].1 += units;
                }
            }
            let (ns, units) = (runs[1].0 - runs[0].0, runs[1].1 - runs[0].1);
            println!(
                "TEXTURE_COST {}",
                serde_json::json!({"case":"ingest","ns":ns,"units":units,"nsPerUnit":ns/units as f64,"baseNsPerUnit":runs[0].0/runs[0].1 as f64})
            );
        }
    }
}
