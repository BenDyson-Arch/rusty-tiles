//! Opt-in audit location hints, independently re-derived from the source DB by S5.
//! Every counted operation has a descriptor. Shared affected-region banks are deliberate:
//! cycle-rank deltas need not have unique source ancestry. Each bank keeps its full bbox
//! and counts, and at most eight actual face/edge anchors, with truncation made explicit.
use super::{topology::Change, Built};
use crate::mesh_large::hlod::{interfaces::key, store::TileKey};
use serde::{ser::SerializeSeq, Serialize, Serializer};
use std::collections::BTreeMap;

type Point = [f32; 3];
type Face = [Point; 3];
const ANCHORS: usize = 8;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Locations {
    /// Exact descendant interval to search; a coarse representative is not ancestry.
    source_hint: TileKey,
    operations: Operations,
    closure: Region,
    dropped: Vec<Region>,
    opening: Opening,
}
#[derive(Debug)]
struct Operations {
    closures: u64,
    drops: u64,
    openings: u64,
}
#[derive(Serialize)]
struct Operation {
    kind: &'static str,
    ordinal: u64,
    bank: u64,
    scope: &'static str,
}
impl Serialize for Operations {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(None)?;
        for (kind, count, scope) in [
            ("closure", self.closures, "affected_regions"),
            ("drop", self.drops, "component"),
            ("opening", self.openings, "affected_regions"),
        ] {
            for ordinal in 0..count {
                seq.serialize_element(&Operation {
                    kind,
                    ordinal,
                    bank: if kind == "drop" { ordinal } else { 0 },
                    scope,
                })?;
            }
        }
        seq.end()
    }
}
#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct Region {
    min: Option<Point>,
    max: Option<Point>,
    face_count: u64,
    face_anchors: Vec<Face>,
    edge_count: u64,
    edge_anchors: Vec<[Point; 2]>,
    truncated: bool,
}
impl Region {
    fn point(&mut self, p: Point) {
        let min = self.min.get_or_insert(p);
        let max = self.max.get_or_insert(p);
        for a in 0..3 {
            min[a] = min[a].min(p[a]);
            max[a] = max[a].max(p[a]);
        }
    }
    fn face(&mut self, face: Face) {
        for p in face {
            self.point(p);
        }
        self.face_count += 1;
        if self.face_anchors.len() < ANCHORS {
            self.face_anchors.push(face);
        } else {
            self.truncated = true;
        }
    }
    fn edge(&mut self, edge: [Point; 2]) {
        for p in edge {
            self.point(p);
        }
        self.edge_count += 1;
        if self.edge_anchors.len() < ANCHORS {
            self.edge_anchors.push(edge);
        } else {
            self.truncated = true;
        }
    }
}
#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct Opening {
    /// Actual new rim plus faces lost from its affected source component(s).
    rim_and_lost_region: Region,
    incident_faces: Region,
}
fn root(parents: &mut [usize], mut at: usize) -> usize {
    while parents[at] != at {
        parents[at] = parents[parents[at]];
        at = parents[at];
    }
    at
}
fn dropped_components(faces: &[Face]) -> Vec<Region> {
    let mut parents: Vec<_> = (0..faces.len()).collect();
    let mut owners = BTreeMap::new();
    for (i, face) in faces.iter().enumerate() {
        for p in face {
            if let Some(previous) = owners.insert(key(*p), i) {
                let a = root(&mut parents, i);
                let b = root(&mut parents, previous);
                parents[a.max(b)] = a.min(b);
            }
        }
    }
    let mut groups = BTreeMap::<usize, Region>::new();
    for (i, face) in faces.iter().enumerate() {
        groups.entry(root(&mut parents, i)).or_default().face(*face);
    }
    groups.into_values().collect()
}
impl Locations {
    pub(crate) fn construction(key: TileKey, faces: &[Face]) -> Self {
        let dropped = dropped_components(faces);
        Self {
            source_hint: key,
            operations: Operations {
                closures: 0,
                drops: dropped.len() as u64,
                openings: 0,
            },
            closure: Region::default(),
            dropped,
            opening: Opening::default(),
        }
    }
    pub(crate) fn dropped_count(&self) -> u64 {
        self.dropped.len() as u64
    }
    pub(crate) fn capture(built: &Built) -> Result<Self, crate::JobError> {
        let Change {
            receipt,
            patch_faces,
            boundary,
            dropped,
            opening_reference,
            opening_boundary,
            opening_faces,
            ..
        } = &built.change;
        let face =
            |i: usize| built.geometry.triangles[i].map(|v| built.geometry.positions[v as usize]);
        let mut closure = Region::default();
        for &i in patch_faces {
            closure.face(face(i));
        }
        for &e in boundary {
            closure.edge(e);
        }
        let mut opening = Opening::default();
        for &(t, lost) in opening_reference {
            if lost {
                opening.rim_and_lost_region.face(t);
            }
        }
        for &e in opening_boundary {
            opening.rim_and_lost_region.edge(e);
        }
        for (i, &yes) in opening_faces.iter().enumerate() {
            if yes {
                opening.incident_faces.face(face(i));
            }
        }
        let dropped = dropped_components(dropped);
        if dropped.len() as u64 != receipt.dropped_components {
            return Err(super::state(
                "topology location banks do not cover every dropped component",
            ));
        }
        if receipt.closed_holes > 0 && closure.edge_count == 0 {
            return Err(super::state("closed-hole location has no source boundary"));
        }
        if receipt.opened_holes > 0 && opening.rim_and_lost_region.edge_count == 0 {
            return Err(super::state("opened-hole location has no proposed rim"));
        }
        Ok(Self {
            source_hint: built.geometry.key,
            operations: Operations {
                closures: receipt.closed_holes,
                drops: receipt.dropped_components,
                openings: receipt.opened_holes,
            },
            closure,
            dropped,
            opening,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dropped_location_banks_keep_each_component_and_its_exact_bounds() {
        let a = [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]];
        let b = [[1., 0., 0.], [1., 1., 0.], [0., 1., 0.]];
        let c = [[4., 0., 0.], [5., 0., 0.], [4., 1., 0.]];
        let groups = dropped_components(&[a, b, c]);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].face_anchors, [a, b]);
        assert_eq!(groups[0].min, Some([0., 0., 0.]));
        assert_eq!(groups[0].max, Some([1., 1., 0.]));
        assert_eq!(groups[1].face_anchors, [c]);
        assert_eq!(groups[1].min, Some([4., 0., 0.]));
        assert_eq!(groups[1].max, Some([5., 1., 0.]));
    }

    #[test]
    fn anchors_are_bounded_but_bbox_and_counts_cover_the_entire_region() {
        let mut region = Region::default();
        for i in 0..100 {
            let p = [i as f32, 0., 0.];
            region.face([p; 3]);
            region.edge([p; 2]);
        }
        assert_eq!(region.face_count, 100);
        assert_eq!(region.edge_count, 100);
        assert_eq!(region.face_anchors.len(), ANCHORS);
        assert_eq!(region.edge_anchors.len(), ANCHORS);
        assert_eq!(region.min, Some([0.; 3]));
        assert_eq!(region.max, Some([99., 0., 0.]));
        assert!(region.truncated);
    }

    #[test]
    fn each_counted_operation_has_an_explicit_shared_or_component_locator() {
        let operations = Operations {
            closures: 2,
            drops: 3,
            openings: 2,
        };
        let json = serde_json::to_value(operations).unwrap();
        let rows = json.as_array().unwrap();
        assert_eq!(rows.len(), 7);
        for (i, row) in rows[2..5].iter().enumerate() {
            assert_eq!(row["kind"], "drop");
            assert_eq!(row["bank"], i);
            assert_eq!(row["scope"], "component");
        }
        for row in rows[..2].iter().chain(&rows[5..]) {
            assert_eq!(row["bank"], 0);
            assert_eq!(row["scope"], "affected_regions");
        }
    }
}
