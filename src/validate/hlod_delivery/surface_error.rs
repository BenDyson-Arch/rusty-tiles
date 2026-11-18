//! Archive-only necessary consequence of two source-relative certificates.
//! For any unit axis the two directed support extrema are 1-Lipschitz under
//! Hausdorff distance. A coarse tile and its decoded leaf union therefore
//! differ at every extremum by at most coarse G + maximum descendant leaf G.
use super::{bad, encode, sql, Effort, Error, Point, Store, IDENTITY, ROW_UNITS};
use rusqlite::params;

#[derive(Clone, Copy, Debug)]
pub(super) struct Extent([f64; 6]);

impl Extent {
    pub(super) fn point(p: Point) -> Self {
        Self([p[0], p[1], p[2], p[0], p[1], p[2]])
    }

    pub(super) fn include(current: &mut Option<Self>, other: Self) {
        match current {
            None => *current = Some(other),
            Some(Self(v)) => {
                for axis in 0..3 {
                    v[axis] = v[axis].min(other.0[axis]);
                    v[axis + 3] = v[axis + 3].max(other.0[axis + 3]);
                }
            }
        }
    }

    fn within(self, other: Self, radius: f64) -> bool {
        self.0
            .into_iter()
            .zip(other.0)
            .all(|(a, b)| (a - b).abs().next_down() <= radius)
    }
}

impl Store {
    pub(in crate::validate) fn expect_surface_error(
        &mut self,
        tileset: &serde_json::Value,
        hlod: bool,
    ) -> Result<(), Error> {
        if let Some(policy) = tileset["asset"]["extras"]["rustyTiles"].get("surfaceErrorPolicy") {
            if !hlod || policy.as_str() != Some("hlod-certified-decoded-surface-v1") {
                return Err(bad(
                    "surfaceErrorPolicy",
                    "unsupported certified surface declaration",
                ));
            }
            self.certified_surface = true;
        }
        Ok(())
    }

    pub(super) fn record_extent(
        &self,
        tile: i64,
        extent: Option<Extent>,
        effort: &Effort,
    ) -> Result<(), Error> {
        if let (true, Some(Extent([x, y, z, xx, yy, zz]))) = (self.certified_surface, extent) {
            effort.charge(ROW_UNITS)?;
            self.connection.prepare_cached(
                "INSERT INTO surface_extents VALUES (?,?,?,?,?,?,?) ON CONFLICT(tile) DO UPDATE SET
                 x=min(x,excluded.x), y=min(y,excluded.y), z=min(z,excluded.z),
                 xx=max(xx,excluded.xx), yy=max(yy,excluded.yy), zz=max(zz,excluded.zz)"
            ).map_err(sql)?.execute(params![tile,x,y,z,xx,yy,zz]).map_err(sql)?;
        }
        Ok(())
    }

    pub(super) fn surface_error_extents(&self, effort: &Effort) -> Result<u64, Error> {
        if !self.certified_surface {
            return Ok(0);
        }
        // Stored G is local. The producer expresses every payload below the
        // common root in that frame. A transformed external archive retains
        // the normal structural checks and reports zero checked extrema tiles.
        let transformed: bool = self
            .connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM tiles WHERE world<>?)",
                [encode(&IDENTITY)],
                |r| r.get(0),
            )
            .map_err(sql)?;
        if transformed {
            return Ok(0);
        }
        struct Frame {
            last: i64,
            id: i64,
            error: f64,
            own: Option<Extent>,
            leaves: Option<Extent>,
            leaf_error: f64,
        }
        fn close(stack: &mut Vec<Frame>, checked: &mut u64) -> Result<(), Error> {
            let frame = stack
                .pop()
                .ok_or_else(|| Error::InvalidState("empty surface-error traversal stack".into()))?;
            if let (Some(own), Some(leaves)) = (frame.own, frame.leaves) {
                if !own.within(leaves, (frame.error + frame.leaf_error).next_up()) {
                    return Err(bad("surfaceErrorExtents", &format!("tile {} and its decoded leaf union exceed their combined G along a coordinate axis", frame.id)));
                }
                *checked += 1;
            }
            if let (Some(parent), Some(leaves)) = (stack.last_mut(), frame.leaves) {
                Extent::include(&mut parent.leaves, leaves);
                parent.leaf_error = parent.leaf_error.max(frame.leaf_error);
            }
            Ok(())
        }

        let mut query = self
            .connection
            .prepare(
                "SELECT t.id,t.last,t.leaf,t.error,e.x,e.y,e.z,e.xx,e.yy,e.zz
             FROM tiles t LEFT JOIN surface_extents e ON t.id=e.tile ORDER BY t.id",
            )
            .map_err(sql)?;
        let mut rows = query.query([]).map_err(sql)?;
        let mut stack: Vec<Frame> = Vec::new();
        let mut checked = 0;
        while let Some(row) = rows.next().map_err(sql)? {
            effort.charge(ROW_UNITS)?;
            let id = row.get(0).map_err(sql)?;
            while stack.last().is_some_and(|f| f.last < id) {
                close(&mut stack, &mut checked)?;
            }
            let extent = match row.get::<_, Option<f64>>(4).map_err(sql)? {
                None => None,
                Some(x) => Some(Extent([
                    x,
                    row.get(5).map_err(sql)?,
                    row.get(6).map_err(sql)?,
                    row.get(7).map_err(sql)?,
                    row.get(8).map_err(sql)?,
                    row.get(9).map_err(sql)?,
                ])),
            };
            let leaf: bool = row.get(2).map_err(sql)?;
            let error = row.get(3).map_err(sql)?;
            stack.push(Frame {
                last: row.get(1).map_err(sql)?,
                id,
                error,
                own: if leaf { None } else { extent },
                leaves: if leaf { extent } else { None },
                leaf_error: if leaf && extent.is_some() { error } else { 0. },
            });
        }
        while !stack.is_empty() {
            close(&mut stack, &mut checked)?;
        }
        Ok(checked)
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{effort_for_tests, store};
    use super::*;
    use serde_json::json;

    #[test]
    fn decoded_support_extrema_include_both_directions_and_leaf_encoding_slack() {
        let mut graph = store();
        graph.materialize().unwrap();
        let policy = json!({"asset":{"extras":{"rustyTiles":{"surfaceErrorPolicy":"hlod-certified-decoded-surface-v1"}}}});
        assert!(graph.expect_surface_error(&policy, false).is_err());
        graph.expect_surface_error(&policy, true).unwrap();
        let effort = effort_for_tests();
        let world = encode(&IDENTITY);
        // A projected gap of 2 exactly fits G_parent + G_leaf = 1.5 + 0.5.
        for (id, last, leaf, g) in [(0, 2, false, 1.5), (1, 1, true, 0.5), (2, 2, true, 0.)] {
            graph
                .connection
                .execute(
                    "INSERT INTO tiles(id,last,leaf,error,world) VALUES(?,?,?,?,?)",
                    params![id, last, leaf, g, world],
                )
                .unwrap();
        }
        for (tile, x) in [(0, 2.), (1, 0.), (2, 2.)] {
            graph
                .record_extent(tile, Some(Extent::point([x, 0., 0.])), &effort)
                .unwrap();
        }
        assert_eq!(graph.surface_error_extents(&effort).unwrap(), 1);
        graph
            .connection
            .execute("UPDATE tiles SET error=1.4 WHERE id=0", [])
            .unwrap();
        assert!(graph
            .surface_error_extents(&effort)
            .unwrap_err()
            .to_string()
            .contains("surfaceErrorExtents"));
        // Reverse-only violation, from an additional coarse payload at x=5.
        graph
            .connection
            .execute("UPDATE tiles SET error=1.5 WHERE id=0", [])
            .unwrap();
        graph
            .record_extent(0, Some(Extent::point([5., 0., 0.])), &effort)
            .unwrap();
        assert!(graph.surface_error_extents(&effort).is_err());
    }
}
