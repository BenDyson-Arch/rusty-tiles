//! HLOD S3 acceptance on small analytic fixtures, checked on DECODED published coarse
//! tiles (own GLB/KTX2 decoding): fresh charts reproduce an analytic source signal, all
//! PBR slots/MASK/factors survive, a normal map keeps its world-space direction through the
//! coarse tangent frame, geometry and IDs are untouched by charting, image-free colourless
//! input bypasses appearance, and vertex colours follow the source. Also the evidence
//! lane measuring cut PSNR against the source in both directions.
use super::{source_index, store::TileKey};
use crate::{
    content_integrity::meshopt::{self as codec, Filter, Mode},
    mesh_large::leaves::producer::{mesh_to_leaf_payloads, LeafOutputRequest, LeafProfile},
    RunControl,
};
use serde_json::{json, Value};
use std::{
    fs,
    num::NonZeroUsize,
    path::{Path, PathBuf},
};

// ---------------------------------------------------------------------------------------
// Fixtures.

#[derive(Clone, Copy, PartialEq)]
enum Surface {
    /// The S1 bump sheet over [0, 100]² (two primitives meeting at x = 50).
    Bump,
    /// The plane y = 0 over [0, 100]².
    Plane,
}

#[derive(Clone)]
struct Fixture {
    surface: Surface,
    /// TEXCOORD_0 = (x, z) / 100.
    uv: bool,
    /// COLOR_0 = ramp (x/100, z/100, 0.25, 1).
    color: bool,
    /// NORMAL (0,1,0) and TANGENT (1,0,0,1) (plane only).
    frame: bool,
    material: Value,
    /// PNG images (RGBA8, 64x64), by generator.
    images: Vec<fn(u32, u32) -> [u8; 4]>,
}

fn height(surface: Surface, x: f32, z: f32) -> f32 {
    match surface {
        Surface::Plane => 0.,
        Surface::Bump => {
            let (u, v) = ((x - 50.) / 30., (z - 50.) / 30.);
            4. / (1. + u * u + v * v) + 0.01 * x
        }
    }
}

/// Smooth sRGB ramp: R = 255 u, G = 255 v, B = 96.
fn ramp(x: u32, y: u32) -> [u8; 4] {
    [(x * 255 / 63) as u8, (y * 255 / 63) as u8, 96, 255]
}

fn write_fixture(dir: &Path, f: &Fixture) -> PathBuf {
    fs::create_dir_all(dir).unwrap();
    let mut bin = Vec::new();
    let (mut views, mut accessors, mut meshes, mut nodes) = (vec![], vec![], vec![], vec![]);
    fn push(
        bin: &mut Vec<u8>,
        views: &mut Vec<Value>,
        accessors: &mut Vec<Value>,
        data: Vec<f32>,
        kind: &str,
        n: usize,
        minmax: bool,
    ) -> usize {
        let start = bin.len();
        for v in &data {
            bin.extend(v.to_le_bytes());
        }
        views.push(json!({"buffer":0,"byteOffset":start,"byteLength":bin.len()-start}));
        let mut a = json!({"bufferView":views.len()-1,"componentType":5126,"count":n,"type":kind});
        if minmax {
            let mut lo = [f32::INFINITY; 3];
            let mut hi = [f32::NEG_INFINITY; 3];
            for p in data.chunks(3) {
                for i in 0..3 {
                    lo[i] = lo[i].min(p[i]);
                    hi[i] = hi[i].max(p[i]);
                }
            }
            a["min"] = json!(lo);
            a["max"] = json!(hi);
        }
        accessors.push(a);
        accessors.len() - 1
    }
    // Two primitives (left/right halves), 30 x 60 cells each, as the S1 sheet.
    for (m, (x0, x1)) in [(0f32, 50f32), (50., 100.)].into_iter().enumerate() {
        let (nx, nz) = (30usize, 60usize);
        let mut pos = vec![];
        let mut uv = vec![];
        let mut col = vec![];
        let mut nrm = vec![];
        let mut tan = vec![];
        for i in 0..=nx {
            for j in 0..=nz {
                let x = x0 + (x1 - x0) * i as f32 / nx as f32;
                let z = 100. * j as f32 / nz as f32;
                pos.extend([x, height(f.surface, x, z), z]);
                uv.extend([x / 100., z / 100.]);
                col.extend([x / 100., z / 100., 0.25, 1.]);
                nrm.extend([0., 1., 0.]);
                tan.extend([1., 0., 0., 1.]);
            }
        }
        let n = pos.len() / 3;
        let mut idx = vec![];
        let w = nz + 1;
        for i in 0..nx {
            for j in 0..nz {
                let v = |a: usize, c: usize| (a * w + c) as u32;
                idx.extend([v(i, j), v(i, j + 1), v(i + 1, j)]);
                idx.extend([v(i + 1, j), v(i, j + 1), v(i + 1, j + 1)]);
            }
        }
        let mut attributes =
            json!({"POSITION": push(&mut bin, &mut views, &mut accessors, pos, "VEC3", n, true)});
        if f.uv {
            attributes["TEXCOORD_0"] = json!(push(
                &mut bin,
                &mut views,
                &mut accessors,
                uv,
                "VEC2",
                n,
                false
            ));
        }
        if f.color {
            attributes["COLOR_0"] = json!(push(
                &mut bin,
                &mut views,
                &mut accessors,
                col,
                "VEC4",
                n,
                false
            ));
        }
        if f.frame {
            attributes["NORMAL"] = json!(push(
                &mut bin,
                &mut views,
                &mut accessors,
                nrm,
                "VEC3",
                n,
                false
            ));
            attributes["TANGENT"] = json!(push(
                &mut bin,
                &mut views,
                &mut accessors,
                tan,
                "VEC4",
                n,
                false
            ));
        }
        let start = bin.len();
        for v in &idx {
            bin.extend(v.to_le_bytes());
        }
        views.push(json!({"buffer":0,"byteOffset":start,"byteLength":bin.len()-start}));
        accessors.push(json!({"bufferView":views.len()-1,"componentType":5125,"count":idx.len(),"type":"SCALAR"}));
        meshes.push(json!({"primitives":[{"attributes":attributes,"indices":accessors.len()-1,"material":0}]}));
        nodes.push(json!({"mesh":m}));
    }
    let mut images = vec![];
    let mut textures = vec![];
    for (i, g) in f.images.iter().enumerate() {
        let img = image::RgbaImage::from_fn(64, 64, |x, y| image::Rgba(g(x, y)));
        let name = format!("img{i}.png");
        img.save(dir.join(&name)).unwrap();
        images.push(json!({"uri":name}));
        textures.push(json!({"source":i,"sampler":0}));
    }
    fs::write(dir.join("scene.bin"), &bin).unwrap();
    let mut doc = json!({"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0,1]}],
        "nodes":nodes,"meshes":meshes,"materials":[f.material],
        "buffers":[{"uri":"scene.bin","byteLength":bin.len()}],"bufferViews":views,"accessors":accessors});
    if !images.is_empty() {
        doc["images"] = json!(images);
        doc["textures"] = json!(textures);
        doc["samplers"] = json!([{"magFilter":9729,"minFilter":9729,"wrapS":33071,"wrapT":33071}]);
    }
    let path = dir.join("scene.gltf");
    fs::write(&path, serde_json::to_vec(&doc).unwrap()).unwrap();
    path
}

fn stage(input: &Path, out: &Path) -> PathBuf {
    stage_capped(input, out, 300)
}

fn stage_capped(input: &Path, out: &Path, cap: usize) -> PathBuf {
    let scratch = out.with_extension("scratch");
    fs::create_dir_all(&scratch).unwrap();
    let request = LeafOutputRequest::new(input, out, LeafProfile::Balanced)
        .with_jobs(NonZeroUsize::new(2).unwrap())
        .with_scratch_dir(&scratch)
        .with_leaf_triangles(256)
        .with_hlod_settings(super::tests::test_settings(cap));
    let dir = mesh_to_leaf_payloads(request, &RunControl::default())
        .unwrap()
        .output;
    assert_eq!(fs::read_dir(&scratch).unwrap().count(), 0);
    dir
}

fn rows(path: &Path) -> Vec<Value> {
    fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

// ---------------------------------------------------------------------------------------
// Decoding (independent of the producer's writer).

struct Glb {
    doc: Value,
    bin: Vec<u8>,
}

/// One decoded primitive, expanded to corners.
struct Prim {
    positions: Vec<[f64; 3]>,
    uv: Option<Vec<[f64; 2]>>,
    color: Option<Vec<[f64; 4]>>,
    normal: Option<Vec<[f64; 3]>>,
    tangent: Option<Vec<[f64; 4]>>,
    features: Vec<u32>,
    material: Value,
}

impl Glb {
    fn new(bytes: &[u8]) -> Self {
        let u = |i: usize| u32::from_le_bytes(bytes[i..i + 4].try_into().unwrap()) as usize;
        assert_eq!(&bytes[..4], b"glTF");
        let jl = u(12);
        let doc: Value = serde_json::from_slice(&bytes[20..20 + jl]).unwrap();
        let bin = bytes[28 + jl..28 + jl + u(20 + jl)].to_vec();
        Self { doc, bin }
    }
    fn view(&self, i: usize) -> Vec<u8> {
        let v = &self.doc["bufferViews"][i];
        match v["extensions"].get("EXT_meshopt_compression") {
            Some(e) => {
                let s = e["byteOffset"].as_u64().unwrap_or(0) as usize;
                let mode = match e["mode"].as_str().unwrap() {
                    "ATTRIBUTES" => Mode::Attributes,
                    "TRIANGLES" => Mode::Triangles,
                    _ => Mode::Indices,
                };
                let filter = e["filter"]
                    .as_str()
                    .and_then(Filter::parse)
                    .unwrap_or(Filter::None);
                codec::decode_filtered(
                    &self.bin[s..s + e["byteLength"].as_u64().unwrap() as usize],
                    e["count"].as_u64().unwrap() as usize,
                    e["byteStride"].as_u64().unwrap() as usize,
                    mode,
                    filter,
                )
                .unwrap()
            }
            None => {
                let s = v["byteOffset"].as_u64().unwrap_or(0) as usize;
                self.bin[s..s + v["byteLength"].as_u64().unwrap() as usize].to_vec()
            }
        }
    }
    fn accessor(&self, a: usize) -> Vec<Vec<f64>> {
        let acc = &self.doc["accessors"][a];
        let n = acc["count"].as_u64().unwrap() as usize;
        let ct = acc["componentType"].as_u64().unwrap();
        let norm = acc["normalized"].as_bool().unwrap_or(false);
        let size = match ct {
            5120 | 5121 => 1,
            5122 | 5123 => 2,
            _ => 4,
        };
        let width = match acc["type"].as_str().unwrap() {
            "SCALAR" => 1,
            "VEC2" => 2,
            "VEC3" => 3,
            _ => 4,
        };
        let vi = acc["bufferView"].as_u64().unwrap() as usize;
        let data = self.view(vi);
        let stride = self.doc["bufferViews"][vi]["byteStride"]
            .as_u64()
            .map_or(size * width, |s| s as usize);
        let offset = acc["byteOffset"].as_u64().unwrap_or(0) as usize;
        (0..n)
            .map(|i| {
                (0..width)
                    .map(|c| {
                        let b = &data[offset + i * stride + c * size..];
                        match ct {
                            5120 => {
                                let v = f64::from(b[0] as i8);
                                if norm {
                                    (v / 127.).max(-1.)
                                } else {
                                    v
                                }
                            }
                            5121 => f64::from(b[0]) / if norm { 255. } else { 1. },
                            5122 => {
                                let v = f64::from(i16::from_le_bytes([b[0], b[1]]));
                                if norm {
                                    (v / 32767.).max(-1.)
                                } else {
                                    v
                                }
                            }
                            5123 => {
                                f64::from(u16::from_le_bytes([b[0], b[1]]))
                                    / if norm { 65535. } else { 1. }
                            }
                            5125 => f64::from(u32::from_le_bytes(b[..4].try_into().unwrap())),
                            _ => f64::from(f32::from_le_bytes(b[..4].try_into().unwrap())),
                        }
                    })
                    .collect()
            })
            .collect()
    }
    fn prims(&self) -> Vec<Prim> {
        let mut out = vec![];
        for root in self.doc["scenes"][0]["nodes"].as_array().unwrap() {
            let node = &self.doc["nodes"][root.as_u64().unwrap() as usize];
            let read = |key: &str, default: f64| -> Vec<f64> {
                node.get(key).map_or(vec![default; 3], |v| {
                    v.as_array()
                        .unwrap()
                        .iter()
                        .map(|x| x.as_f64().unwrap())
                        .collect()
                })
            };
            let (t, s) = (read("translation", 0.), read("scale", 1.));
            for prim in self.doc["meshes"][node["mesh"].as_u64().unwrap() as usize]["primitives"]
                .as_array()
                .unwrap()
            {
                let at = |k: &str| {
                    prim["attributes"]
                        .get(k)
                        .map(|a| self.accessor(a.as_u64().unwrap() as usize))
                };
                let index: Option<Vec<usize>> = prim.get("indices").map(|i| {
                    self.accessor(i.as_u64().unwrap() as usize)
                        .iter()
                        .map(|v| v[0] as usize)
                        .collect()
                });
                let expand = |v: Vec<Vec<f64>>| -> Vec<Vec<f64>> {
                    match &index {
                        Some(ix) => ix.iter().map(|&i| v[i].clone()).collect(),
                        None => v,
                    }
                };
                let positions = expand(at("POSITION").unwrap())
                    .iter()
                    .map(|p| std::array::from_fn(|a| p[a] * s[a] + t[a]))
                    .collect();
                out.push(Prim {
                    positions,
                    uv: at("TEXCOORD_0").map(|v| expand(v).iter().map(|x| [x[0], x[1]]).collect()),
                    color: at("COLOR_0").map(|v| {
                        expand(v)
                            .iter()
                            .map(|x| [x[0], x[1], x[2], *x.get(3).unwrap_or(&1.)])
                            .collect()
                    }),
                    normal: at("NORMAL")
                        .map(|v| expand(v).iter().map(|x| [x[0], x[1], x[2]]).collect()),
                    tangent: at("TANGENT")
                        .map(|v| expand(v).iter().map(|x| [x[0], x[1], x[2], x[3]]).collect()),
                    features: at("_FEATURE_ID_0")
                        .map(|v| expand(v).iter().map(|x| x[0] as u32).collect())
                        .unwrap_or_default(),
                    material: self.doc["materials"]
                        [prim["material"].as_u64().unwrap_or(0) as usize]
                        .clone(),
                });
            }
        }
        out
    }
    /// Level-0 RGBA8 of texture `texture`.
    fn texture(&self, texture: usize) -> (u32, u32, Vec<u8>) {
        let t = &self.doc["textures"][texture];
        let source = t
            .pointer("/extensions/KHR_texture_basisu/source")
            .or_else(|| t.get("source"))
            .unwrap()
            .as_u64()
            .unwrap() as usize;
        let image = &self.doc["images"][source];
        let bytes = self.view(image["bufferView"].as_u64().unwrap() as usize);
        if image["mimeType"] == "image/ktx2" {
            let w = u32::from_le_bytes(bytes[20..24].try_into().unwrap());
            let h = u32::from_le_bytes(bytes[24..28].try_into().unwrap());
            (
                w,
                h,
                crate::basisu_ktx2::decode_level_rgba(&bytes, 0, w, h).unwrap(),
            )
        } else {
            let img = image::load_from_memory(&bytes).unwrap().into_rgba8();
            (img.width(), img.height(), img.into_raw())
        }
    }
}

/// Bilinear clamp sample of RGBA8 at `uv`, components in [0, 1] (as stored).
fn bilinear(img: &(u32, u32, Vec<u8>), uv: [f64; 2]) -> [f64; 4] {
    let (w, h, px) = img;
    let x = (uv[0] * f64::from(*w) - 0.5).clamp(0., f64::from(*w - 1));
    let y = (uv[1] * f64::from(*h) - 0.5).clamp(0., f64::from(*h - 1));
    let (x0, y0) = (x.floor() as u32, y.floor() as u32);
    let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
    let (fx, fy) = (x - f64::from(x0), y - f64::from(y0));
    let get = |x: u32, y: u32, c: usize| f64::from(px[((y * w + x) * 4) as usize + c]) / 255.;
    std::array::from_fn(|c| {
        let a = get(x0, y0, c) + (get(x1, y0, c) - get(x0, y0, c)) * fx;
        let b = get(x0, y1, c) + (get(x1, y1, c) - get(x0, y1, c)) * fx;
        a + (b - a) * fy
    })
}

fn texture_index(material: &Value, pointer: &str) -> Option<usize> {
    material
        .pointer(pointer)
        .and_then(Value::as_u64)
        .map(|v| v as usize)
}

/// Every coarse tile of a stage, decoded.
fn coarse(dir: &Path) -> Vec<(Value, Glb)> {
    rows(&dir.join("proxies.jsonl"))
        .into_iter()
        .map(|r| {
            let g = Glb::new(&fs::read(dir.join(r["uri"].as_str().unwrap())).unwrap());
            (r, g)
        })
        .collect()
}

fn base_fixture() -> Fixture {
    Fixture {
        surface: Surface::Bump,
        uv: true,
        color: false,
        frame: false,
        material: json!({"pbrMetallicRoughness":{"baseColorTexture":{"index":0},"metallicFactor":0.0,"roughnessFactor":0.75}}),
        images: vec![ramp],
    }
}

// ---------------------------------------------------------------------------------------
// Tests.

#[test]
fn coarse_pages_rebake_the_source_ramp_with_content_sized_pages() {
    let work = tempfile::tempdir().unwrap();
    let input = write_fixture(&work.path().join("in"), &base_fixture());
    let dir = stage(&input, &work.path().join("out"));
    let tiles = coarse(&dir);
    assert!(!tiles.is_empty());
    let (mut sum, mut n) = (0., 0usize);
    for (row, glb) in &tiles {
        let a = &row["appearance"];
        assert_eq!(a["texturedGroups"], 1, "{a}");
        // Achieved spacing never finer than requested (G/16, floored at the source's
        // own spacing: 100 m over 64 texels here); capped exactly when coarser.
        let src = a["sourceSpacing"].as_f64().unwrap();
        assert!((src - 100. / 64.).abs() < 0.05, "{a}");
        let req = a["requestedSpacing"].as_f64().unwrap().max(src);
        let ach = a["achievedSpacing"].as_f64().unwrap();
        assert_eq!(
            a["sourceLimited"].as_bool().unwrap(),
            src > a["requestedSpacing"].as_f64().unwrap()
        );
        assert!(ach >= req * (1. - 1e-12), "{a}");
        assert_eq!(
            a["capped"].as_bool().unwrap(),
            ach > req * (1. + 1e-9),
            "{a}"
        );
        let [w, h] = [0, 1].map(|i| a["pageSize"][i].as_u64().unwrap());
        assert!(
            w % 4 == 0 && h % 4 == 0 && (32..=1024).contains(&w) && (32..=1024).contains(&h),
            "{w}x{h}"
        );
        assert!(glb.doc["extensionsUsed"]
            .as_array()
            .unwrap()
            .contains(&json!("KHR_texture_basisu")));
        for p in glb.prims() {
            assert_eq!(p.material["pbrMetallicRoughness"]["roughnessFactor"], 0.75);
            let tex = glb.texture(
                texture_index(&p.material, "/pbrMetallicRoughness/baseColorTexture/index").unwrap(),
            );
            assert_eq!([u64::from(tex.0), u64::from(tex.1)], [w, h]);
            let uv = p.uv.as_ref().unwrap();
            for f in 0..p.positions.len() / 3 {
                let c: [f64; 3] = std::array::from_fn(|a| {
                    (0..3).map(|k| p.positions[3 * f + k][a]).sum::<f64>() / 3.
                });
                let t: [f64; 2] =
                    std::array::from_fn(|a| (0..3).map(|k| uv[3 * f + k][a]).sum::<f64>() / 3.);
                let got = bilinear(&tex, t);
                // The source ramp at the face centroid, in stored sRGB units.
                let want = [c[0] / 100., c[2] / 100.];
                sum += (got[0] - want[0]).abs()
                    + (got[1] - want[1]).abs()
                    + (got[2] - 96. / 255.).abs();
                n += 3;
            }
        }
    }
    let mean = sum / n as f64;
    assert!(mean < 0.02, "mean absolute sRGB error {mean}");
    let manifest: Value =
        serde_json::from_slice(&fs::read(dir.join("manifest.json")).unwrap()).unwrap();
    let depths = manifest["hlod"]["appearanceByDepth"].as_array().unwrap();
    let pages: u64 = depths.iter().map(|d| d["pages"].as_u64().unwrap()).sum();
    assert_eq!(pages as usize, tiles.len());
}

/// D1 omission with a textured source: an omitted cell holds no content, so S3 charts,
/// bakes and textures nothing for it. At caps small enough to omit cells, every published
/// row is textured, the per-depth appearance (tiles, pages, donor searches) sums exactly
/// the published rows, and the stage holds one tile file per row (none for omitted cells).
#[test]
fn omitted_cells_get_no_pages_textures_or_donor_work() {
    let work = tempfile::tempdir().unwrap();
    let input = write_fixture(&work.path().join("in"), &base_fixture());
    let mut omitted_any = false;
    let mut all_omitted = false;
    for cap in [12, 40, 120] {
        let dir = stage_capped(&input, &work.path().join(format!("cap{cap}")), cap);
        let manifest: Value =
            serde_json::from_slice(&fs::read(dir.join("manifest.json")).unwrap()).unwrap();
        let hlod = &manifest["hlod"];
        let resources: Value =
            serde_json::from_slice(&fs::read(dir.join("resources.json")).unwrap()).unwrap();
        let diagnostics = &resources["hlod"]["diagnostics"];
        for (public, internal) in [
            ("omitted_proxies", "omittedProxies"),
            ("omission_sources", "omissionSources"),
            ("omitted_by_depth", "omittedByDepth"),
        ] {
            assert_eq!(diagnostics[public], hlod[internal], "cap {cap}: {public}");
            assert!(diagnostics.get(internal).is_none());
        }
        let by_depth = diagnostics["omitted_by_depth"].as_array().unwrap();
        let borders = diagnostics["borders_by_depth"].as_array().unwrap();
        assert_eq!(by_depth.len(), borders.len());
        assert_eq!(diagnostics["borders_by_depth"], hlod["bordersByDepth"]);
        for (count, depth) in by_depth.iter().zip(borders) {
            assert_eq!(count, &depth["omittedCells"]);
        }
        omitted_any |= hlod["omittedProxies"].as_u64().unwrap() > 0;
        let proxies = rows(&dir.join("proxies.jsonl"));
        if proxies.is_empty() {
            all_omitted = true;
            let cache = &diagnostics["image_cache"];
            for key in ["hits", "misses", "peakResidentBytes"] {
                assert_eq!(cache[key].as_u64(), Some(0), "cap {cap}: {diagnostics}");
            }
            assert_eq!(cache["loadCpuSeconds"].as_f64(), Some(0.));
            for key in ["surface", "charts", "query", "sample", "encode"] {
                assert_eq!(
                    diagnostics["appearance_seconds"][key].as_f64(),
                    Some(0.),
                    "discarded appearance work at cap {cap}: {diagnostics}"
                );
            }
        }
        let files = fs::read_dir(dir.join("proxies")).unwrap().count();
        assert_eq!(files, proxies.len(), "cap {cap}: a tile file without a row");
        let depths = hlod["appearanceByDepth"].as_array().unwrap();
        let sum = |key: &str| -> u64 { depths.iter().map(|d| d[key].as_u64().unwrap()).sum() };
        let rows_sum = |key: &str| -> u64 {
            proxies
                .iter()
                .map(|r| r["appearance"][key].as_u64().unwrap())
                .sum()
        };
        assert_eq!(sum("tiles"), proxies.len() as u64, "cap {cap}: {hlod}");
        assert_eq!(sum("pages"), rows_sum("pages"), "cap {cap}");
        assert_eq!(sum("searches"), rows_sum("searches"), "cap {cap}");
        for row in &proxies {
            assert_eq!(row["appearance"]["texturedGroups"], 1, "cap {cap}: {row}");
        }
    }
    assert!(omitted_any, "some cap omits cells");
    assert!(all_omitted, "one textured case must omit every coarse cell");
}

/// Adversarial: 32 strips, each its own textured material class and source image, so
/// coarse parents carry many material groups and many source images. Groups are baked
/// one at a time and share the tile's texel budget instead of each taking a full page.
#[test]
fn many_material_classes_and_images_per_parent_share_one_tile_budget() {
    let work = tempfile::tempdir().unwrap();
    let dir = work.path().join("in");
    fs::create_dir_all(&dir).unwrap();
    const STRIPS: usize = 32;
    let (mut bin, mut views, mut accessors, mut meshes, mut nodes) =
        (vec![], vec![], vec![], vec![], vec![]);
    let (mut materials, mut images, mut textures) = (vec![], vec![], vec![]);
    for k in 0..STRIPS {
        let (x0, x1) = (
            100. * k as f32 / STRIPS as f32,
            100. * (k + 1) as f32 / STRIPS as f32,
        );
        let (nx, nz) = (3usize, 60usize);
        let (mut pos, mut uv) = (vec![], vec![]);
        for i in 0..=nx {
            for j in 0..=nz {
                let x = x0 + (x1 - x0) * i as f32 / nx as f32;
                let z = 100. * j as f32 / nz as f32;
                pos.extend([x, height(Surface::Bump, x, z), z]);
                uv.extend([i as f32 / nx as f32, z / 100.]);
            }
        }
        let n = pos.len() / 3;
        let mut idx = vec![];
        for i in 0..nx {
            for j in 0..nz {
                let v = |a: usize, c: usize| (a * (nz + 1) + c) as u32;
                idx.extend([
                    v(i, j),
                    v(i, j + 1),
                    v(i + 1, j),
                    v(i + 1, j),
                    v(i, j + 1),
                    v(i + 1, j + 1),
                ]);
            }
        }
        let mut push = |data: Vec<u8>, count: usize, kind: &str, component: u32| {
            let start = bin.len();
            bin.extend(data);
            views.push(json!({"buffer":0,"byteOffset":start,"byteLength":bin.len()-start}));
            accessors.push(json!({"bufferView":views.len()-1,"componentType":component,"count":count,"type":kind}));
            accessors.len() - 1
        };
        let mut lo = [f32::INFINITY; 3];
        let mut hi = [f32::NEG_INFINITY; 3];
        for p in pos.chunks(3) {
            for a in 0..3 {
                lo[a] = lo[a].min(p[a]);
                hi[a] = hi[a].max(p[a]);
            }
        }
        let p = push(
            pos.iter().flat_map(|v| v.to_le_bytes()).collect(),
            n,
            "VEC3",
            5126,
        );
        let t = push(
            uv.iter().flat_map(|v| v.to_le_bytes()).collect(),
            n,
            "VEC2",
            5126,
        );
        let ix = push(
            idx.iter().flat_map(|v| v.to_le_bytes()).collect(),
            idx.len(),
            "SCALAR",
            5125,
        );
        accessors[p]["min"] = json!(lo);
        accessors[p]["max"] = json!(hi);
        meshes.push(json!({"primitives":[{"attributes":{"POSITION":p,"TEXCOORD_0":t},"indices":ix,"material":k}]}));
        nodes.push(json!({"mesh":k}));
        let shade = (k * 7 % 32) as u8 * 8;
        image::RgbaImage::from_fn(16, 16, |x, y| {
            image::Rgba([shade, (x * 16) as u8, (y * 16) as u8, 255])
        })
        .save(dir.join(format!("strip{k}.png")))
        .unwrap();
        images.push(json!({"uri":format!("strip{k}.png")}));
        textures.push(json!({"source":k,"sampler":0}));
        materials.push(
            json!({"pbrMetallicRoughness":{"baseColorTexture":{"index":k},
            "baseColorFactor":[1.0, 1.0 - k as f64 / 64.0, 1.0, 1.0],"metallicFactor":0.0}}),
        );
    }
    fs::write(dir.join("scene.bin"), &bin).unwrap();
    let doc = json!({"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":(0..STRIPS).collect::<Vec<_>>()}],
        "nodes":nodes,"meshes":meshes,"materials":materials,"images":images,"textures":textures,
        "samplers":[{"magFilter":9729,"minFilter":9729,"wrapS":33071,"wrapT":33071}],
        "buffers":[{"uri":"scene.bin","byteLength":bin.len()}],"bufferViews":views,"accessors":accessors});
    let input = dir.join("scene.gltf");
    fs::write(&input, serde_json::to_vec(&doc).unwrap()).unwrap();
    let out = stage(&input, &work.path().join("out"));
    let mut most = 0;
    for (row, glb) in coarse(&out) {
        let a = &row["appearance"];
        let groups = a["texturedGroups"].as_u64().unwrap();
        most = most.max(groups);
        assert_eq!(
            a["pages"].as_u64().unwrap(),
            groups,
            "one base colour page per group"
        );
        let triangles = row["triangles"].as_u64().unwrap();
        let budget = super::appearance::TILE_TEXELS + 64 * triangles;
        assert!(a["layoutTexels"].as_u64().unwrap() <= budget, "{a}");
        assert_eq!(glb.prims().len() as u64, groups);
    }
    assert!(most >= 16, "parents carry many material groups ({most})");
}

type Faces = Vec<([[u64; 3]; 3], u32)>;

/// Decoded positions and feature IDs of every coarse tile, per tile, faces sorted.
fn geometry(dir: &Path) -> Vec<(String, Faces)> {
    let mut out: Vec<_> = coarse(dir)
        .into_iter()
        .map(|(r, g)| {
            let mut faces: Faces = vec![];
            for p in g.prims() {
                for f in 0..p.positions.len() / 3 {
                    // Smallest corner first: the index codec may rotate corners.
                    let c: [_; 3] =
                        std::array::from_fn(|k| p.positions[3 * f + k].map(f64::to_bits));
                    let r = (0..3).min_by_key(|&k| c[k]).unwrap();
                    faces.push((std::array::from_fn(|k| c[(k + r) % 3]), p.features[3 * f]));
                }
            }
            faces.sort();
            (r["uri"].as_str().unwrap().to_owned(), faces)
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

#[test]
fn charting_keeps_geometry_and_ids_and_image_free_input_bypasses_appearance() {
    let work = tempfile::tempdir().unwrap();
    let textured = stage(
        &write_fixture(&work.path().join("a"), &base_fixture()),
        &work.path().join("ao"),
    );
    let plain = Fixture {
        uv: false,
        images: vec![],
        material: json!({"pbrMetallicRoughness":{"baseColorFactor":[0.5,0.4,0.3,1.0],"metallicFactor":0.0}}),
        ..base_fixture()
    };
    let flat = stage(
        &write_fixture(&work.path().join("b"), &plain),
        &work.path().join("bo"),
    );
    assert_eq!(
        geometry(&textured),
        geometry(&flat),
        "charting changed geometry or IDs"
    );
    for (row, glb) in coarse(&flat) {
        assert_eq!(row["appearance"]["texturedGroups"], 0);
        assert_eq!(row["appearance"]["vertexColorGroups"], 0);
        assert!(glb.doc.get("images").is_none());
        for p in glb.prims() {
            assert!(p.uv.is_none() && p.color.is_none() && p.normal.is_none());
            assert_eq!(
                p.material["pbrMetallicRoughness"]["baseColorFactor"],
                json!([0.5, 0.4, 0.3, 1.0])
            );
        }
    }
    let resources: Value =
        serde_json::from_slice(&fs::read(flat.join("resources.json")).unwrap()).unwrap();
    let plan = &resources["hlod"]["plan"];
    assert_eq!(plan["texture_workspace_bytes"], 0, "{plan}");
    assert_eq!(plan["color_workspace_bytes"], 0, "{plan}");
    assert_eq!(plan["image_cache_bytes"], 0, "{plan}");
}

#[test]
fn vertex_colours_follow_the_source_without_images() {
    let work = tempfile::tempdir().unwrap();
    let f = Fixture {
        uv: false,
        color: true,
        images: vec![],
        material: json!({"pbrMetallicRoughness":{"metallicFactor":0.0}}),
        ..base_fixture()
    };
    let dir = stage(
        &write_fixture(&work.path().join("in"), &f),
        &work.path().join("out"),
    );
    let (mut worst, mut n) = (0f64, 0);
    for (row, glb) in coarse(&dir) {
        assert_eq!(row["appearance"]["vertexColorGroups"], 1);
        assert!(glb.doc.get("images").is_none());
        for p in glb.prims() {
            let c = p.color.as_ref().unwrap();
            for (q, col) in p.positions.iter().zip(c) {
                // Nearest source point of a coarse vertex lies near it in x/z.
                worst = worst
                    .max((col[0] - q[0] / 100.).abs())
                    .max((col[1] - q[2] / 100.).abs());
                assert!((col[2] - 0.25).abs() < 1e-3);
                n += 1;
            }
        }
    }
    assert!(n > 0 && worst < 0.03, "worst vertex colour error {worst}");
}

/// Constant tangent-space normal (0.3, 0.1, 0.95) on the plane with N = +y, T = +x.
fn bumpy(_: u32, _: u32) -> [u8; 4] {
    let n = [0.3f64, 0.1, 0.95];
    let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    let q = |v: f64| ((v / l + 1.) * 0.5 * 255.).round() as u8;
    [q(n[0]), q(n[1]), q(n[2]), 255]
}
fn data(x: u32, y: u32) -> [u8; 4] {
    [200, (x * 4) as u8, (y * 4) as u8, 255]
}
fn masked(x: u32, _: u32) -> [u8; 4] {
    [180, 120, 60, if x < 32 { 0 } else { 255 }]
}

#[test]
fn all_slots_mask_factors_and_the_normal_frame_survive() {
    let work = tempfile::tempdir().unwrap();
    let f = Fixture {
        surface: Surface::Plane,
        uv: true,
        color: false,
        frame: true,
        material: json!({
            "pbrMetallicRoughness":{"baseColorTexture":{"index":0},"metallicRoughnessTexture":{"index":1},
                "baseColorFactor":[1.0,0.9,0.8,1.0],"metallicFactor":0.5,"roughnessFactor":0.7},
            "normalTexture":{"index":2,"scale":1.0},"occlusionTexture":{"index":1,"strength":0.6},
            "emissiveTexture":{"index":3},"emissiveFactor":[0.2,0.1,0.0],
            "alphaMode":"MASK","alphaCutoff":0.4,"doubleSided":true}),
        images: vec![masked, data, bumpy, ramp],
    };
    let dir = stage(
        &write_fixture(&work.path().join("in"), &f),
        &work.path().join("out"),
    );
    let want_world = {
        // T x + B y + N z with B = cross(N, T) * w = (0, 0, -1).
        let n = [0.3f64, 0.1, 0.95];
        let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        [n[0] / l, n[2] / l, -n[1] / l]
    };
    let mut worst = 0f64;
    let mut checked = 0;
    for (row, glb) in coarse(&dir) {
        assert_eq!(row["appearance"]["pages"], 5, "{}", row["appearance"]);
        for p in glb.prims() {
            let m = &p.material;
            assert_eq!(m["alphaMode"], "MASK");
            assert_eq!(m["alphaCutoff"], 0.4);
            assert_eq!(m["doubleSided"], true);
            assert_eq!(m["emissiveFactor"], json!([0.2, 0.1, 0.0]));
            assert_eq!(
                m["pbrMetallicRoughness"]["baseColorFactor"],
                json!([1.0, 0.9, 0.8, 1.0])
            );
            assert_eq!(m["pbrMetallicRoughness"]["metallicFactor"], 0.5);
            assert_eq!(m["occlusionTexture"]["strength"], 0.6);
            assert!(
                m["normalTexture"].get("scale").is_none(),
                "source scale absorbed"
            );
            for ptr in [
                "/pbrMetallicRoughness/baseColorTexture/index",
                "/pbrMetallicRoughness/metallicRoughnessTexture/index",
                "/normalTexture/index",
                "/occlusionTexture/index",
                "/emissiveTexture/index",
            ] {
                assert!(texture_index(m, ptr).is_some(), "{ptr}");
            }
            let normal_page = glb.texture(texture_index(m, "/normalTexture/index").unwrap());
            let base = glb
                .texture(texture_index(m, "/pbrMetallicRoughness/baseColorTexture/index").unwrap());
            let (uv, nrm, tan) = (
                p.uv.as_ref().unwrap(),
                p.normal.as_ref().unwrap(),
                p.tangent.as_ref().unwrap(),
            );
            for f in 0..p.positions.len() / 3 {
                for weights in [[1. / 3.; 3], [0.5, 0.5, 0.], [0., 0.5, 0.5], [0.5, 0., 0.5]] {
                    let c: [f64; 3] = std::array::from_fn(|a| {
                        (0..3)
                            .map(|k| p.positions[3 * f + k][a] * weights[k])
                            .sum::<f64>()
                    });
                    let t: [f64; 2] = std::array::from_fn(|a| {
                        (0..3).map(|k| uv[3 * f + k][a] * weights[k]).sum::<f64>()
                    });
                    // MASK alpha follows the source's x < 32 texel split (away from the edge).
                    if (c[0] - 50.).abs() > 6. {
                        let a = bilinear(&base, t)[3];
                        assert!(
                            if c[0] < 50. { a < 0.2 } else { a > 0.8 },
                            "alpha {a} at {c:?}"
                        );
                    }
                    let v = bilinear(&normal_page, t).map(|x| 2. * x - 1.);
                    let n: [f64; 3] = std::array::from_fn(|a| {
                        (0..3).map(|k| nrm[3 * f + k][a] * weights[k]).sum::<f64>()
                    });
                    let tt: [f64; 4] = std::array::from_fn(|a| {
                        (0..3).map(|k| tan[3 * f + k][a] * weights[k]).sum::<f64>()
                    });
                    let b = [
                        n[1] * tt[2] - n[2] * tt[1],
                        n[2] * tt[0] - n[0] * tt[2],
                        n[0] * tt[1] - n[1] * tt[0],
                    ]
                    .map(|x| x * tt[3].signum());
                    let w: [f64; 3] =
                        std::array::from_fn(|a| tt[a] * v[0] + b[a] * v[1] + n[a] * v[2]);
                    let l = (w[0] * w[0] + w[1] * w[1] + w[2] * w[2]).sqrt();
                    let cos =
                        (w[0] * want_world[0] + w[1] * want_world[1] + w[2] * want_world[2]) / l;
                    worst = worst.max(cos.clamp(-1., 1.).acos().to_degrees());
                    checked += 1;
                }
            }
        }
    }
    assert!(
        checked > 0 && worst < 4.,
        "worst world-normal angle {worst} deg"
    );
}

// ---------------------------------------------------------------------------------------
// Evidence lane: cut PSNR against the source, both directions.

/// One evaluated surface: triangles plus a linear-light base colour per (triangle, bary).
struct Eval {
    triangles: Vec<[[f64; 3]; 3]>,
    index: source_index::Surface,
}

impl Eval {
    fn new(triangles: Vec<[[f64; 3]; 3]>) -> Self {
        let rows = triangles
            .iter()
            .enumerate()
            .map(|(i, t)| source_index::Triangle::new(t.map(|p| p.map(|v| v as f32)), i as u64))
            .collect();
        Self {
            triangles,
            index: source_index::Surface::new(rows).unwrap(),
        }
    }
    /// Area-weighted deterministic samples: (triangle, barycentrics).
    fn samples(&self, n: usize, seed: u64) -> Vec<(usize, [f64; 3])> {
        let mut state = seed;
        let mut next = || {
            state = state.wrapping_add(0x9e3779b97f4a7c15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
            ((z ^ (z >> 31)) >> 11) as f64 / (1u64 << 53) as f64
        };
        let mut cumulative = Vec::with_capacity(self.triangles.len());
        let mut total = 0.;
        for t in &self.triangles {
            let e1: [f64; 3] = std::array::from_fn(|a| t[1][a] - t[0][a]);
            let e2: [f64; 3] = std::array::from_fn(|a| t[2][a] - t[0][a]);
            let c = [
                e1[1] * e2[2] - e1[2] * e2[1],
                e1[2] * e2[0] - e1[0] * e2[2],
                e1[0] * e2[1] - e1[1] * e2[0],
            ];
            total += (c[0] * c[0] + c[1] * c[1] + c[2] * c[2]).sqrt();
            cumulative.push(total);
        }
        (0..n)
            .map(|_| {
                let r = next() * total;
                let i = cumulative
                    .partition_point(|c| *c <= r)
                    .min(self.triangles.len() - 1);
                let (a, b) = (next().sqrt(), next());
                (i, [1. - a, a * (1. - b), a * b])
            })
            .collect()
    }
    fn point(&self, i: usize, w: [f64; 3]) -> [f64; 3] {
        let t = &self.triangles[i];
        std::array::from_fn(|a| t[0][a] * w[0] + t[1][a] * w[1] + t[2][a] * w[2])
    }
    fn nearest(&self, p: [f64; 3], hint: &mut Option<usize>) -> (usize, [f64; 3], f64) {
        let all = TileKey {
            first: 0,
            end: u64::MAX,
            depth: 0,
            leaf_begin: 0,
            leaf_end: 0,
        };
        let h = self.index.nearest_hint(p, all, hint);
        (h.rank as usize, h.bary, h.distance2.sqrt())
    }
}

fn srgb(v: f64) -> f64 {
    super::bake::srgb(v)
}

/// The colour lane of a decoded published cut: per triangle, its UV/COLOR_0/factor and
/// level-0 base colour page (linear light).
struct CutColour {
    uv: Vec<Option<[[f64; 2]; 3]>>,
    color: Vec<[[f64; 4]; 3]>,
    factor: Vec<[f64; 4]>,
    page: Vec<Option<usize>>,
    pages: Vec<(u32, u32, Vec<u8>)>,
}

impl CutColour {
    fn at(&self, t: usize, w: [f64; 3]) -> [f64; 3] {
        let col: [f64; 3] =
            std::array::from_fn(|a| (0..3).map(|k| self.color[t][k][a] * w[k]).sum());
        let tex = match (self.uv[t], self.page[t]) {
            (Some(uv), Some(p)) => {
                let q: [f64; 2] = std::array::from_fn(|a| (0..3).map(|k| uv[k][a] * w[k]).sum());
                let s = bilinear(&self.pages[p], q);
                [0, 1, 2].map(|a| super::bake::linear(s[a]))
            }
            _ => [1.; 3],
        };
        std::array::from_fn(|a| self.factor[t][a] * tex[a] * col[a])
    }
}

fn load_cut(dir: &Path, uris: &[String]) -> (Vec<[[f64; 3]; 3]>, CutColour) {
    let mut tris = vec![];
    let mut c = CutColour {
        uv: vec![],
        color: vec![],
        factor: vec![],
        page: vec![],
        pages: vec![],
    };
    for uri in uris {
        let glb = Glb::new(&fs::read(dir.join(uri)).unwrap());
        let mut cache: std::collections::HashMap<usize, usize> = Default::default();
        for p in glb.prims() {
            let factor = p.material["pbrMetallicRoughness"]["baseColorFactor"]
                .as_array()
                .map_or([1.; 4], |a| std::array::from_fn(|i| a[i].as_f64().unwrap()));
            let page = texture_index(&p.material, "/pbrMetallicRoughness/baseColorTexture/index")
                .map(|t| {
                    *cache.entry(t).or_insert_with(|| {
                        c.pages.push(glb.texture(t));
                        c.pages.len() - 1
                    })
                });
            for f in 0..p.positions.len() / 3 {
                tris.push(std::array::from_fn(|k| p.positions[3 * f + k]));
                c.uv.push(
                    p.uv.as_ref()
                        .map(|uv| std::array::from_fn(|k| uv[3 * f + k])),
                );
                c.color.push(
                    p.color
                        .as_ref()
                        .map_or([[1.; 4]; 3], |col| std::array::from_fn(|k| col[3 * f + k])),
                );
                c.factor.push(factor);
                c.page.push(page);
            }
        }
    }
    (tris, c)
}

/// Evidence lane, not CI: after `evidence_census` staged `RT_HLOD_EVIDENCE_INPUT` under
/// `RT_HLOD_EVIDENCE_DIR/stage`, measure cut PSNR (sRGB base colour, 200,000 area samples
/// per direction, nearest-surface correspondence) at cut depths 0, 2, 4, 6 and every
/// coarse depth, against the captured source. Writes `appearance.json`.
#[test]
#[ignore = "evidence lane: needs RT_HLOD_EVIDENCE_INPUT and RT_HLOD_EVIDENCE_DIR"]
fn evidence_appearance() {
    use crate::mesh_large::{
        config::{Profile, TreeConfig},
        full,
    };
    let input = PathBuf::from(std::env::var("RT_HLOD_EVIDENCE_INPUT").unwrap());
    let out = PathBuf::from(std::env::var("RT_HLOD_EVIDENCE_DIR").unwrap());
    let dir = out.join("stage");
    // Source: the captured scene with its own images, sampled at level 0 (bilinear).
    let mut config = TreeConfig::new(Profile::Balanced);
    let scratch = out.join("eval-scratch");
    fs::create_dir_all(&scratch).unwrap();
    config.scratch_dir = Some(scratch.clone());
    let captured = full::capture(&input, &config, &mut || Ok(())).unwrap();
    let cache = super::bake::ImageCache::new(u64::MAX);
    let source: Vec<crate::mesh_large::attributes::FullTriangle> = captured
        .triangles
        .reader()
        .unwrap()
        .map(|t| t.unwrap())
        .collect();
    let src = Eval::new(
        source
            .iter()
            .map(|t| t.triangle.positions.map(|p| p.map(f64::from)))
            .collect(),
    );
    let check = || Ok(());
    let source_colour = |i: usize, w: [f64; 3]| -> [f64; 3] {
        let t = &source[i];
        let factor = captured
            .materials
            .get(t.material as usize)
            .and_then(|m| {
                m.literal()
                    .pointer("/pbrMetallicRoughness/baseColorFactor")
                    .cloned()
            })
            .and_then(|v| {
                v.as_array()
                    .map(|a| std::array::from_fn::<f64, 4, _>(|i| a[i].as_f64().unwrap()))
            })
            .unwrap_or([1.; 4]);
        let col: [f64; 3] = if t.flags & crate::mesh_large::attributes::COLOR != 0 {
            std::array::from_fn(|a| {
                (0..3)
                    .map(|k| f64::from(t.corners[k].color[a]) * w[k])
                    .sum()
            })
        } else {
            [1.; 3]
        };
        let tex = match super::bake::binding(
            &captured,
            t.material,
            crate::gltf_source::source::TextureSlot::BaseColor,
        )
        .unwrap()
        {
            Some((image, sampler, set)) => {
                let chain = cache
                    .get(
                        &captured,
                        image,
                        crate::mesh_large::leaves::mips::Class::Colour,
                        0,
                        &check,
                    )
                    .unwrap();
                let uv: [f32; 2] = std::array::from_fn(|a| {
                    (0..3)
                        .map(|k| f64::from(t.corners[k].texcoords[set][a]) * w[k])
                        .sum::<f64>() as f32
                });
                let s = super::bake::sample(
                    &chain,
                    crate::mesh_large::leaves::mips::Class::Colour,
                    sampler,
                    uv,
                    0.,
                );
                [s[0], s[1], s[2]]
            }
            None => [1.; 3],
        };
        std::array::from_fn(|a| factor[a] * tex[a] * col[a])
    };
    // The tile tree from the stage rows.
    let mut tiles: Vec<(u64, u64, u32, String, bool)> = vec![];
    for (file, leaf) in [("leaves.jsonl", true), ("proxies.jsonl", false)] {
        for r in rows(&dir.join(file)) {
            tiles.push((
                r["first"].as_u64().unwrap(),
                r["end"].as_u64().unwrap(),
                r["depth"].as_u64().unwrap() as u32,
                r["uri"].as_str().unwrap().to_owned(),
                leaf,
            ));
        }
    }
    tiles.sort_by_key(|t| (t.0, t.2));
    let root = tiles
        .iter()
        .position(|t| t.0 == 0 && t.1 == source.len() as u64)
        .unwrap();
    let children = |i: usize| -> Vec<usize> {
        let (f, e, d) = (tiles[i].0, tiles[i].1, tiles[i].2);
        let inside: Vec<usize> = (0..tiles.len())
            .filter(|&j| j != i && tiles[j].0 >= f && tiles[j].1 <= e && tiles[j].2 > d)
            .collect();
        inside
            .iter()
            .copied()
            .filter(|&j| {
                !inside.iter().any(|&k| {
                    k != j
                        && tiles[k].0 <= tiles[j].0
                        && tiles[k].1 >= tiles[j].1
                        && tiles[k].2 < tiles[j].2
                })
            })
            .collect()
    };
    let max_depth = tiles
        .iter()
        .filter(|t| !t.4)
        .map(|t| t.2)
        .max()
        .unwrap_or(0);
    let mut cuts: Vec<u32> = (0..=max_depth).collect();
    cuts.dedup();
    let mut results = vec![];
    for depth in cuts {
        let mut cut = vec![];
        let mut stack = vec![root];
        while let Some(i) = stack.pop() {
            let kids = children(i);
            if tiles[i].2 >= depth || tiles[i].4 || kids.is_empty() {
                cut.push(tiles[i].3.clone());
            } else {
                stack.extend(kids);
            }
        }
        let (tris, colour) = load_cut(&dir, &cut);
        let lod = Eval::new(tris);
        let mut directions = serde_json::Map::new();
        for (label, from_source) in [("source_to_lod", true), ("lod_to_source", false)] {
            let (a, b) = if from_source {
                (&src, &lod)
            } else {
                (&lod, &src)
            };
            let samples = a.samples(200_000, 20261024 + u64::from(from_source));
            let (mut mse, mut hint) = (0., None);
            let mut distances = vec![];
            for (i, w) in &samples {
                let p = a.point(*i, *w);
                let (j, v, d) = b.nearest(p, &mut hint);
                let (ca, cb) = if from_source {
                    (source_colour(*i, *w), colour.at(j, v))
                } else {
                    (colour.at(*i, *w), source_colour(j, v))
                };
                for k in 0..3 {
                    let e = srgb(ca[k].clamp(0., 1.)) - srgb(cb[k].clamp(0., 1.));
                    mse += e * e;
                }
                distances.push(d);
            }
            mse /= (samples.len() * 3) as f64;
            distances.sort_by(f64::total_cmp);
            let q = |f: f64| distances[((distances.len() - 1) as f64 * f) as usize];
            directions.insert(label.into(), json!({"samples":samples.len(),"psnrSrgbDb":-10. * mse.max(1e-30).log10(),
                "geometryM":{"mean":distances.iter().sum::<f64>() / distances.len() as f64,"p95":q(0.95),"max":q(1.)}}));
        }
        let row = json!({"cutDepth":depth,"cutTiles":cut.len(),"triangles":lod.triangles.len(),"directions":directions});
        eprintln!("{row}");
        results.push(row);
    }
    fs::write(out.join("appearance.json"), serde_json::to_vec_pretty(&json!({"input":input,"cuts":results,
        "method":"200,000 area-weighted samples per direction; nearest-surface correspondence; base colour = factor x level-0 bilinear texture x COLOR_0 in linear light, compared in sRGB; published pages decoded at level 0 (KTX2 transcoded)"})).unwrap()).unwrap();
    drop(captured);
    let _ = fs::remove_dir_all(&scratch);
}

/// Same source-side checker signal sampled on decoded borders at different delivered
/// depths. Compare coincident points along unequal subdivisions, away from checker steps.
#[test]
fn checkerboard_normal_and_mask_borders_agree_across_mixed_lod() {
    use std::{collections::BTreeMap, sync::Arc};
    fn checker(x: u32, y: u32) -> [u8; 4] {
        let v = if ((x / 7) + (y / 7)).is_multiple_of(2) {
            32
        } else {
            224
        };
        [v, v, v, if x < 32 { 0 } else { 255 }]
    }
    struct Edge {
        a: [f64; 3],
        b: [f64; 3],
        uv: [[f64; 2]; 2],
        image: Arc<(u32, u32, Vec<u8>)>,
        normal_image: Arc<(u32, u32, Vec<u8>)>,
        normal: [[f64; 3]; 2],
        tangent: [[f64; 4]; 2],
    }
    let unit = |v: [f64; 3]| {
        let length = v.iter().map(|x| x * x).sum::<f64>().sqrt();
        v.map(|x| x / length)
    };
    let world_normal = |edge: &Edge, t: f64| {
        let uv = std::array::from_fn(|k| edge.uv[0][k] * (1. - t) + edge.uv[1][k] * t);
        let v = bilinear(&edge.normal_image, uv).map(|x| 2. * x - 1.);
        let n: [f64; 3] =
            std::array::from_fn(|k| edge.normal[0][k] * (1. - t) + edge.normal[1][k] * t);
        let tangent: [f64; 4] =
            std::array::from_fn(|k| edge.tangent[0][k] * (1. - t) + edge.tangent[1][k] * t);
        let b = [
            n[1] * tangent[2] - n[2] * tangent[1],
            n[2] * tangent[0] - n[0] * tangent[2],
            n[0] * tangent[1] - n[1] * tangent[0],
        ]
        .map(|x| x * tangent[3].signum());
        unit(std::array::from_fn(|k| {
            tangent[k] * v[0] + b[k] * v[1] + n[k] * v[2]
        }))
    };
    let want_world = unit([0.3, 0.95, -0.1]);
    let dot = |a: [f64; 3], b: [f64; 3]| (0..3).map(|k| a[k] * b[k]).sum::<f64>();
    let work = tempfile::tempdir().unwrap();
    let mut f = base_fixture();
    f.surface = Surface::Plane;
    f.images = vec![checker, bumpy];
    f.frame = true;
    f.material["normalTexture"] = json!({"index":1});
    f.material["alphaMode"] = json!("MASK");
    f.material["alphaCutoff"] = json!(0.4);
    let dir = stage(
        &write_fixture(&work.path().join("src"), &f),
        &work.path().join("out"),
    );
    let mut tiles = Vec::new();
    for (row, glb) in coarse(&dir) {
        let mut edges: BTreeMap<([u64; 3], [u64; 3]), (usize, Edge)> = BTreeMap::new();
        for p in glb.prims() {
            let page = Arc::new(glb.texture(
                texture_index(&p.material, "/pbrMetallicRoughness/baseColorTexture/index").unwrap(),
            ));
            assert_eq!(p.material["alphaMode"], "MASK");
            assert_eq!(p.material["alphaCutoff"], 0.4);
            let normal_page =
                Arc::new(glb.texture(texture_index(&p.material, "/normalTexture/index").unwrap()));
            let uv = p.uv.unwrap();
            let normal = p.normal.unwrap();
            let tangent = p.tangent.unwrap();
            for face in 0..p.positions.len() / 3 {
                for k in 0..3 {
                    let (a, b) = (3 * face + k, 3 * face + (k + 1) % 3);
                    let bits = |p: [f64; 3]| p.map(|x| if x == 0. { 0 } else { x.to_bits() });
                    let (ka, kb) = (bits(p.positions[a]), bits(p.positions[b]));
                    let entry = edges.entry((ka.min(kb), ka.max(kb))).or_insert_with(|| {
                        (
                            0,
                            Edge {
                                a: p.positions[a],
                                b: p.positions[b],
                                uv: [uv[a], uv[b]],
                                image: page.clone(),
                                normal_image: normal_page.clone(),
                                normal: [normal[a], normal[b]],
                                tangent: [tangent[a], tangent[b]],
                            },
                        )
                    });
                    entry.0 += 1;
                }
            }
        }
        tiles.push((
            row,
            edges
                .into_values()
                .filter(|(n, _)| *n == 1)
                .map(|(_, e)| e)
                .collect::<Vec<_>>(),
        ));
    }
    let mut checked = 0;
    let mut alpha_sides = [0usize; 2];
    for (i, (a, left)) in tiles.iter().enumerate() {
        for (b, right) in &tiles[i + 1..] {
            if a["depth"] == b["depth"] {
                continue;
            }
            if a["first"].as_u64().unwrap() < b["end"].as_u64().unwrap()
                && b["first"].as_u64().unwrap() < a["end"].as_u64().unwrap()
            {
                continue;
            }
            for e in left {
                for u in [0.125, 0.25, 0.375, 0.5, 0.625, 0.75, 0.875] {
                    let p: [f64; 3] = std::array::from_fn(|k| e.a[k] * (1. - u) + e.b[k] * u);
                    if [p[0], p[2]].iter().any(|x| {
                        let pixel = x / 100. * 64. - 0.5;
                        let r = pixel.rem_euclid(7.);
                        r < 2. || r > 5.
                    }) {
                        continue;
                    }
                    for f in right {
                        let d: [f64; 3] = std::array::from_fn(|k| f.b[k] - f.a[k]);
                        let length2 = d.iter().map(|x| x * x).sum::<f64>();
                        if length2 == 0. {
                            continue;
                        }
                        let t = (0..3).map(|k| (p[k] - f.a[k]) * d[k]).sum::<f64>() / length2;
                        if !(0.0..=1.0).contains(&t) {
                            continue;
                        }
                        if (0..3)
                            .map(|k| (f.a[k] + t * d[k] - p[k]).powi(2))
                            .sum::<f64>()
                            > 1e-12
                        {
                            continue;
                        }
                        let x = bilinear(
                            &e.image,
                            std::array::from_fn(|k| e.uv[0][k] * (1. - u) + e.uv[1][k] * u),
                        );
                        let y = bilinear(
                            &f.image,
                            std::array::from_fn(|k| f.uv[0][k] * (1. - t) + f.uv[1][k] * t),
                        );
                        let difference = (0..3).map(|k| (x[k] - y[k]).abs()).fold(0f64, f64::max);
                        assert!(
                            difference < 0.2,
                            "mixed depths {}/{} at {p:?}: {x:?} vs {y:?}",
                            a["depth"],
                            b["depth"]
                        );
                        // Decode each side through its own page and tangent frame, and
                        // compare both to the independent source-space normal.
                        let nx = world_normal(e, u);
                        let ny = world_normal(f, t);
                        let cos4 = 4f64.to_radians().cos();
                        assert!(
                            dot(nx, want_world) > cos4 && dot(ny, want_world) > cos4,
                            "source normal at {p:?}: {nx:?} / {ny:?}"
                        );
                        assert!(dot(nx, ny) > 8f64.to_radians().cos());
                        if (p[0] - 50.).abs() > 6. {
                            let opaque = p[0] > 50.;
                            assert_eq!(x[3] >= 0.4, opaque, "left MASK at {p:?}");
                            assert_eq!(y[3] >= 0.4, opaque, "right MASK at {p:?}");
                            assert!((x[3] - y[3]).abs() < 0.2);
                            alpha_sides[usize::from(opaque)] += 1;
                        }
                        checked += 1;
                        break;
                    }
                }
            }
        }
    }
    assert!(
        alpha_sides.iter().all(|&n| n > 0),
        "MASK sides {alpha_sides:?}"
    );
    assert!(
        checked >= 10,
        "only {checked} decoded mixed-depth border samples"
    );
}
