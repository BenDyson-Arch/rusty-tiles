//! Geometry-supported categorical pages. Each ID208 page reduces the complete
//! packed tuple by mode, with exact PNG readback and nearest nonmip sampling.
use super::{
    charts::Layout,
    invalid, raster, resource,
    types::{FeatureIdentity, FrozenNode, IdPacket},
};
use crate::{
    mesh_large::leaves::id_texture::{self, IdSet, Request},
    JobError,
};
use std::collections::BTreeMap;
pub(super) fn bake(
    f: &FrozenNode,
    l: &Layout,
    memory: u64,
    work_limit: u64,
    check: &mut dyn FnMut() -> Result<(), JobError>,
) -> Result<Vec<IdPacket>, JobError> {
    let mut output = Vec::new();
    let mut work = 0u64;
    let mut output_charge = 0u64;
    for (page, plan) in l.pages.iter().enumerate() {
        let mut groups: BTreeMap<Vec<(u32, Option<u32>)>, Vec<usize>> = BTreeMap::new();
        for &chart in &plan.charts {
            for &fi in &l.charts[chart].faces {
                let mut schema: Vec<_> = f.faces[fi]
                    .features
                    .iter()
                    .map(|x| (x.set, x.table))
                    .collect();
                schema.sort_unstable();
                if schema.windows(2).any(|p| p[0].0 == p[1].0) {
                    return Err(invalid("immutable feature set duplicated"));
                }
                if !schema.is_empty() {
                    groups.entry(schema).or_default().push(fi);
                }
            }
        }
        for (schema, faces) in groups {
            if !faces.iter().any(|&fi| {
                super::math::face_normal(f.faces[fi].positions.map(|p| p.map(f64::from))).is_some()
            }) {
                continue;
            }
            check()?;
            let [width, height] = plan.size;
            let count = (width as usize)
                .checked_mul(height as usize)
                .ok_or_else(|| resource("categorical geometry page"))?;
            let charge = (count as u64)
                .checked_mul(32 + (schema.len() as u64) * 4)
                .ok_or_else(|| resource("categorical workspace"))?;
            if charge.checked_add(output_charge).is_none_or(|n| n > memory) {
                return Err(resource("categorical page lease"));
            }
            let mut sets = Vec::new();
            for &(set, table) in &schema {
                let maximum = faces
                    .iter()
                    .map(|&fi| {
                        f.faces[fi]
                            .features
                            .iter()
                            .find(|x| x.set == set && x.table == table)
                            .unwrap()
                            .value
                    })
                    .max()
                    .unwrap();
                let bytes=(1..=4).find(|b|maximum<((1u64<<(8*b))-1)).ok_or_else(||resource("categorical ID cannot reserve u32 null; retain integer attribute identity"))?;
                sets.push(IdSet { bytes });
            }
            let mut weights = vec![0.; count];
            let mut tuples: Vec<Vec<u64>> = Vec::new();
            let mut tuple_index = BTreeMap::new();
            let mut classes = BTreeMap::new();
            for &fi in &faces {
                let tuple: Vec<u64> = schema
                    .iter()
                    .map(|&(set, table)| {
                        f.faces[fi]
                            .features
                            .iter()
                            .find(|x| x.set == set && x.table == table)
                            .unwrap()
                            .value
                    })
                    .collect();
                let class = *tuple_index.entry(tuple.clone()).or_insert_with(|| {
                    let i = tuples.len();
                    tuples.push(tuple);
                    i
                });
                classes.insert(fi, class);
            }
            let mut tally: BTreeMap<(usize, usize), f64> = BTreeMap::new();
            for &fi in &faces {
                check()?;
                let t = l.uv[fi].map(|p| {
                    [
                        f64::from(p[0]) * f64::from(width),
                        f64::from(p[1]) * f64::from(height),
                    ]
                });
                let lo = std::array::from_fn::<_, 2, _>(|a| {
                    t.iter()
                        .map(|p| p[a])
                        .fold(f64::INFINITY, f64::min)
                        .floor()
                        .max(0.) as u32
                });
                let hi = std::array::from_fn::<_, 2, _>(|a| {
                    t.iter()
                        .map(|p| p[a])
                        .fold(f64::NEG_INFINITY, f64::max)
                        .ceil()
                        .max(0.) as u32
                });
                for y in lo[1]..hi[1].min(height) {
                    for x in lo[0]..hi[0].min(width) {
                        check()?;
                        work = work
                            .checked_add(1)
                            .ok_or_else(|| resource("categorical coverage work"))?;
                        if work > work_limit {
                            return Err(resource("categorical coverage work budget"));
                        }
                        let area = raster::coverage(t, x, y).0;
                        if area <= 0. {
                            continue;
                        }
                        let i = y as usize * width as usize + x as usize;
                        weights[i] += area;
                        // Equal complete tuples from different faces combine
                        // BEFORE selection. Stable tuple insertion order breaks ties.
                        let key = (i, classes[&fi]);
                        if !tally.contains_key(&key)
                            && charge
                                .checked_add(output_charge)
                                .and_then(|n| n.checked_add((tally.len() as u64 + 1) * 128))
                                .is_none_or(|n| n > memory)
                        {
                            return Err(resource("categorical tuple tally lease"));
                        }
                        *tally.entry(key).or_default() += area;
                    }
                }
            }
            let q = (u64::from(u32::MAX)
                .checked_sub(count as u64)
                .ok_or_else(|| resource("categorical support count"))?
                / (count as u64))
                .clamp(1, 65536) as f64;
            let mut support = vec![0; count];
            let mut winners = vec![0usize; count];
            let mut best = vec![0f64; count];
            for ((i, class), weight) in tally {
                if weight > best[i] {
                    best[i] = weight;
                    winners[i] = class;
                }
            }
            let mut ids = vec![
                0;
                count
                    .checked_mul(schema.len())
                    .ok_or_else(|| resource("categorical IDs"))?
            ];
            for i in 0..count {
                if weights[i] == 0. {
                    continue;
                }
                if weights[i] > 1. + 1e-8 {
                    return Err(invalid("categorical raster overlap"));
                }
                support[i] = (weights[i] * q).round().max(1.) as u32;
                for (j, &(set, table)) in schema.iter().enumerate() {
                    let _ = (set, table);
                    let v = tuples[winners[i]][j];
                    ids[i * schema.len() + j] =
                        u32::try_from(v).map_err(|_| resource("categorical ID width"))?;
                }
            }
            let texture = id_texture::bake(&Request {
                width,
                height,
                sets: &sets,
                ids: &ids,
                support: &support,
                memory_limit: memory
                    .checked_sub(output_charge)
                    .ok_or_else(|| resource("categorical complete output lease"))?,
            })?;
            for page in &texture.pages {
                for png in &page.levels {
                    output_charge = output_charge
                        .checked_add(png.capacity() as u64)
                        .ok_or_else(|| resource("categorical output sum"))?;
                    if output_charge > memory {
                        return Err(resource("categorical encoded output lease"));
                    }
                }
            }
            let descriptors = schema
                .into_iter()
                .map(|(set, table)| FeatureIdentity {
                    set,
                    table,
                    value: 0,
                })
                .collect();
            output.push(IdPacket {
                page,
                faces,
                sets: descriptors,
                texture,
            });
        }
    }
    Ok(output)
}
