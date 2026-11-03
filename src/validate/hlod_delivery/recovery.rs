//! Declared root recovery, checked from delivery and decoded payload counts.
//! Actual refinement and the reconstructed ownership scaffold remain separate.
use super::{bad, identity, integer, sql, Effort, Error, Store};
use rusqlite::{params, OptionalExtension};
use serde::Deserialize;
use serde_json::{value::RawValue, Value};

// Calibrated against #309: tau_P=11.68 ns, tau_E=15.52 ns.
// See docs/architecture/validator-limits-amendment.md and the retained cost probe.
pub(crate) const READ_UNITS: usize = 32;
pub(super) const BORDER_UNITS: usize = 64;
pub(crate) const METADATA_UNITS: usize = 512;
pub(crate) const JSON_BYTE_UNITS: usize = 4;
pub(crate) const MAX_JSON: usize = 65536;
const SETUP_UNITS: usize = 16384;
const SCAN_UNITS: usize = 8;
const CENSUS_UNITS: usize = 128;
const LOOKUP_UNITS: usize = 1024;
const PAIR_UNITS: usize = 16;

#[cfg(all(test, unix))]
mod cost;

/// Stop while serializing: an untrusted declaration cannot allocate past its bound.
struct Json(Vec<u8>);
impl std::io::Write for Json {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > MAX_JSON - self.0.len() {
            return Err(std::io::Error::other("recovery declaration exceeds 64 KiB"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[derive(Clone)]
pub(crate) struct Metadata {
    pub(super) owner: Option<(u64, u32)>,
    pub(super) recovery: Option<String>,
}

impl Metadata {
    pub(crate) fn read(node: &Value) -> Result<Option<Self>, Error> {
        let extras = &node["extras"];
        let first = extras["first"]
            .as_str()
            .and_then(|v| v.parse().ok())
            .or_else(|| extras["first"].as_u64());
        let depth = extras["depth"].as_u64().and_then(|d| u32::try_from(d).ok());
        let recovery = extras
            .get("recovery")
            .map(|v| {
                let mut out = Json(Vec::with_capacity(MAX_JSON));
                serde_json::to_writer(&mut out, v)
                    .map_err(|_| bad("recoveryBounds", "recovery declaration exceeds 64 KiB"))?;
                Ok::<_, Error>(String::from_utf8(out.0).expect("JSON is UTF-8"))
            })
            .transpose()?;
        let owner = first.zip(depth);
        Ok((owner.is_some() || recovery.is_some()).then_some(Self { owner, recovery }))
    }
    pub(crate) fn declared(&self) -> bool {
        self.recovery.is_some()
    }
    pub(crate) fn cost(&self) -> usize {
        std::mem::size_of::<Self>() + self.recovery.as_ref().map_or(0, String::len)
    }
}

pub(super) fn payload_marker(raw: &RawValue) -> Result<bool, Error> {
    let value = identity::field(raw, "asset")
        .and_then(|r| identity::field(r, "extras"))
        .and_then(|r| identity::field(r, "rustyTiles"))
        .and_then(|r| identity::field(r, "descendantRecovery"));
    match value {
        None => Ok(false),
        Some(v) if serde_json::from_str::<u64>(v.get()).ok() == Some(1) => Ok(true),
        Some(_) => Err(bad("recoveryRelation", "invalid payload recovery version")),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Key {
    first: u64,
    end: u64,
    depth: u32,
    leaf_begin: u32,
    leaf_end: u32,
}
impl Key {
    fn valid(self) -> bool {
        self.first < self.end
            && self.end <= i64::MAX as u64
            && self.leaf_begin < self.leaf_end
            && self.depth <= 31
    }
    fn contains(self, child: Self) -> bool {
        self.depth < child.depth
            && self.first <= child.first
            && child.end <= self.end
            && self.leaf_begin <= child.leaf_begin
            && child.leaf_end <= self.leaf_end
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    key: Key,
    triangles: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Declaration {
    version: u32,
    root: Key,
    encoded_depth: u32,
    frontier: Vec<Input>,
    omitted: Vec<Key>,
}

impl Store {
    pub(in crate::validate) fn metadata(
        &mut self,
        id: u64,
        metadata: &Metadata,
    ) -> Result<(), Error> {
        if metadata.recovery.is_some() {
            self.checked = true;
            self.materialize()?;
        }
        if self.connection.0.is_none() {
            let i =
                *self.pending.index.get(&id).ok_or_else(|| {
                    Error::InvalidState("delivery metadata before its tile".into())
                })?;
            self.pending.tiles[i].metadata = Some(metadata.clone());
            return Ok(());
        }
        self.connection
            .execute(
                "UPDATE tiles SET rank_first=?,ownership_depth=?,recovery=? WHERE id=?",
                params![
                    metadata.owner.map(|k| k.0).map(integer).transpose()?,
                    metadata.owner.map(|k| k.1),
                    metadata.recovery,
                    integer(id)?
                ],
            )
            .map_err(sql)?;
        Ok(())
    }

    fn recovery_owner(&self, mut id: i64, effort: &Effort) -> Result<Option<(u64, u32)>, Error> {
        // An implicit root's generated cell inherits its header's ownership identity.
        loop {
            effort.charge(LOOKUP_UNITS)?;
            let (first, depth, parent): (Option<i64>, Option<u32>, Option<i64>) = self
                .connection
                .query_row(
                    "SELECT rank_first,ownership_depth,parent FROM tiles WHERE id=?",
                    [id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .map_err(sql)?;
            if let Some(owner) = first.zip(depth) {
                return Ok(Some((owner.0 as u64, owner.1)));
            }
            let Some(parent) = parent else {
                return Ok(None);
            };
            effort.charge(LOOKUP_UNITS)?;
            let placeholder: bool = self
                .connection
                .query_row("SELECT placeholder FROM tiles WHERE id=?", [parent], |r| {
                    r.get(0)
                })
                .map_err(sql)?;
            if !placeholder {
                return Ok(None);
            }
            id = parent;
        }
    }

    pub(super) fn recovery(&self, effort: &Effort) -> Result<(), Error> {
        #[cfg(all(test, unix))]
        let discovery = cost::Phase::new("discovery", effort);
        let rows = self.recovery_rows.0.saturating_add(self.recovery_rows.1);
        effort.charge(SETUP_UNITS.saturating_add(rows.saturating_mul(SCAN_UNITS)))?;
        let declarations: i64 = self
            .connection
            .query_row(
                "SELECT count(*) FROM tiles WHERE recovery IS NOT NULL",
                [],
                |r| r.get(0),
            )
            .map_err(sql)?;
        let marked: i64 = self
            .connection
            .query_row(
                "SELECT count(*) FROM refs r JOIN payloads p ON p.rank=r.rank WHERE p.recovery=1",
                [],
                |r| r.get(0),
            )
            .map_err(sql)?;
        if declarations == 0 && marked == 0 {
            return Ok(());
        }
        if declarations != 1 || marked != 1 {
            return Err(bad(
                "recoveryRelation",
                "payload and delivery recovery declarations differ",
            ));
        }
        let (root, last, cell, refine, placeholder, g, text):
            (i64, i64, Option<i64>, Option<bool>, bool, f64, String) = self.connection.query_row(
            "SELECT id,last,cell,refine,placeholder,error,recovery FROM tiles WHERE recovery IS NOT NULL",
            [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))).map_err(sql)?;
        let payload: Option<bool> = self.connection.query_row(
            "SELECT p.recovery FROM refs r JOIN payloads p ON p.rank=r.rank WHERE r.tile=? GROUP BY r.tile HAVING count(*)=1",
            [root], |r| r.get(0)).optional().map_err(sql)?;
        if cell.is_some() || refine != Some(true) || placeholder || payload != Some(true) {
            return Err(bad(
                "recoveryRelation",
                "recovery requires one explicit REPLACE ownership root",
            ));
        }
        #[cfg(all(test, unix))]
        drop(discovery);
        #[cfg(all(test, unix))]
        let admission = cost::Phase::new("admission", effort);
        effort.charge(text.len().saturating_mul(JSON_BYTE_UNITS))?;
        let declaration: Declaration = serde_json::from_str(&text)
            .map_err(|_| bad("recoveryRelation", "invalid recovery declaration"))?;
        let d = &declaration;
        if d.version != 1
            || !d.root.valid()
            || d.root.first != 0
            || d.root.depth != 0
            || d.root.leaf_begin != 0
            || d.encoded_depth > 30
            || self.recovery_owner(root, effort)? != Some((0, 0))
        {
            return Err(bad("recoveryRelation", "invalid recovery root"));
        }
        if d.frontier.is_empty()
            || d.frontier.len() > 8
            || d.omitted.is_empty()
            || d.omitted.len() > 256
            || d.frontier
                .iter()
                .any(|c| c.triangles == 0 || c.triangles > 8192)
            || d.frontier.iter().map(|c| c.triangles).sum::<u64>() > 8 * 8192
        {
            return Err(bad(
                "recoveryBounds",
                "recovery exceeds its input or ownership bounds",
            ));
        }

        #[cfg(all(test, unix))]
        drop(admission);
        #[cfg(all(test, unix))]
        let census_time = cost::Phase::new("census", effort);
        effort.charge(SETUP_UNITS.saturating_add(rows.saturating_mul(CENSUS_UNITS)))?;
        // One streamed terminal census. Source ranks and DFS leaf bounds are recomputed
        // from payload counts, not trusted from the relation or the producer report.
        self.connection.execute_batch("CREATE TEMP TABLE recovery_leaves(tile INTEGER PRIMARY KEY,first INTEGER,last INTEGER,lo INTEGER,hi INTEGER);").map_err(sql)?;
        let mut census = self.connection.prepare("SELECT t.id,coalesce(sum(p.triangles),0) FROM tiles t LEFT JOIN refs r ON r.tile=t.id LEFT JOIN payloads p ON p.rank=r.rank WHERE NOT EXISTS(SELECT 1 FROM tiles c WHERE c.parent=t.id) GROUP BY t.id ORDER BY t.id").map_err(sql)?;
        let mut rows = census.query([]).map_err(sql)?;
        let (mut rank, mut leaf) = (0u64, 0u64);
        let mut insert = self
            .connection
            .prepare("INSERT INTO recovery_leaves VALUES(?,?,?,?,?)")
            .map_err(sql)?;
        while let Some(r) = rows.next().map_err(sql)? {
            let id: i64 = r.get(0).map_err(sql)?;
            let count = u64::try_from(r.get::<_, i64>(1).map_err(sql)?)
                .map_err(|_| bad("recoveryCoverage", "negative terminal count"))?;
            let end = rank
                .checked_add(count)
                .ok_or_else(|| bad("recoveryCoverage", "terminal rank overflow"))?;
            if count == 0 {
                return Err(bad("recoveryCoverage", "terminal frontier has no content"));
            }
            insert
                .execute(params![
                    id,
                    integer(rank)?,
                    integer(end)?,
                    integer(leaf)?,
                    integer(leaf + 1)?
                ])
                .map_err(sql)?;
            (rank, leaf) = (end, leaf + 1);
        }
        #[cfg(all(test, unix))]
        drop(census_time);
        #[cfg(all(test, unix))]
        let coverage_time = cost::Phase::new("coverage", effort);
        // Prefixes are strictly increasing and complete, so two indexed endpoints
        // prove the same interval as min/max without rescanning each subtree.
        let cover = |id: i64, end: i64, k: Key| -> Result<bool, Error> {
            effort.charge(2 * LOOKUP_UNITS)?;
            let first: Option<(i64,u32)> = self.connection.query_row(
                "SELECT first,lo FROM recovery_leaves WHERE tile>=? AND tile<=? ORDER BY tile LIMIT 1",
                params![id,end], |r| Ok((r.get(0)?,r.get(1)?))).optional().map_err(sql)?;
            let last: Option<(i64,u32)> = self.connection.query_row(
                "SELECT last,hi FROM recovery_leaves WHERE tile>=? AND tile<=? ORDER BY tile DESC LIMIT 1",
                params![id,end], |r| Ok((r.get(0)?,r.get(1)?))).optional().map_err(sql)?;
            Ok(first == Some((k.first as i64, k.leaf_begin))
                && last == Some((k.end as i64, k.leaf_end)))
        };
        if !cover(root, last, d.root)? {
            return Err(bad(
                "recoveryCoverage",
                "root differs from its delivered terminal cover",
            ));
        }
        effort.charge(
            SETUP_UNITS.saturating_add(
                self.recovery_rows
                    .0
                    .saturating_add(self.recovery_rows.1)
                    .saturating_mul(SCAN_UNITS),
            ),
        )?;
        let mut query = self.connection.prepare(
            "SELECT t.id,t.last,t.error,count(r.rank),coalesce(sum(p.triangles),0) FROM tiles t LEFT JOIN refs r ON r.tile=t.id LEFT JOIN payloads p ON p.rank=r.rank WHERE t.cell=? AND t.placeholder=0 GROUP BY t.id ORDER BY t.id LIMIT 9").map_err(sql)?;
        let children: Vec<(i64, i64, f64, i64, i64)> = query
            .query_map([root], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })
            .map_err(sql)?
            .collect::<Result<_, _>>()
            .map_err(sql)?;
        if children.len() != d.frontier.len() {
            return Err(bad(
                "recoveryCoverage",
                "refinement children differ from the frontier",
            ));
        }
        for ((id, end, error, refs, triangles), input) in children.iter().zip(&d.frontier) {
            if !input.key.valid() || *refs != 1 || !cover(*id, *end, input.key)? {
                return Err(bad(
                    "recoveryCoverage",
                    "frontier differs from its delivered cover",
                ));
            }
            if self.recovery_owner(*id, effort)? != Some((input.key.first, input.key.depth))
                || (input.key.depth == 31 && id != end)
            {
                return Err(bad(
                    "recoveryOwnership",
                    "frontier ownership identity differs",
                ));
            }
            if *triangles != input.triangles as i64 {
                return Err(bad("recoveryBounds", "frontier triangle count differs"));
            }
            if g < *error {
                return Err(bad(
                    "recoveryError",
                    "root G is below its refinement frontier",
                ));
            }
        }

        #[cfg(all(test, unix))]
        drop(coverage_time);
        #[cfg(all(test, unix))]
        let _scaffold_time = cost::Phase::new("scaffold", effort);
        let size = 1 + d.omitted.len() + d.frontier.len();
        effort.charge(size.saturating_mul(size).saturating_mul(PAIR_UNITS))?;
        let mut keys = vec![Key {
            depth: d.encoded_depth,
            ..d.root
        }];
        keys.extend(&d.omitted);
        let split = keys.len();
        keys.extend(d.frontier.iter().map(|c| c.key));
        if d.omitted.iter().any(|k| k.depth > 30) {
            return Err(bad(
                "recoveryOwnership",
                "omitted cell exceeds internal depth",
            ));
        }
        if keys.iter().any(|k| !k.valid())
            || keys
                .iter()
                .copied()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != keys.len()
        {
            return Err(bad(
                "recoveryCoverage",
                "invalid or duplicate ownership cell",
            ));
        }
        let mut parents = vec![0usize; keys.len()];
        let mut owned = vec![Vec::new(); keys.len()];
        for i in 1..keys.len() {
            let p = keys
                .iter()
                .enumerate()
                .filter(|(_, k)| k.contains(keys[i]))
                .max_by_key(|(_, k)| k.depth)
                .map(|(p, _)| p)
                .ok_or_else(|| bad("recoveryOwnership", "ownership cell has no ancestor"))?;
            if p >= split || keys[i].depth != keys[p].depth + 1 {
                return Err(bad("recoveryOwnership", "ownership scaffold skips a level"));
            }
            parents[i] = p;
            owned[p].push(i);
        }
        for i in 0..split {
            let mut children = owned[i].clone();
            children.sort_unstable_by_key(|&c| keys[c]);
            if children.is_empty() || children.len() > 8 {
                return Err(bad(
                    "recoveryCoverage",
                    "ownership cell lacks its bounded cover",
                ));
            }
            let (mut first, mut leaf) = (keys[i].first, keys[i].leaf_begin);
            for c in children {
                if keys[c].first != first || keys[c].leaf_begin != leaf {
                    return Err(bad("recoveryCoverage", "gap or overlap in ownership cover"));
                }
                (first, leaf) = (keys[c].end, keys[c].leaf_end);
            }
            if first != keys[i].end || leaf != keys[i].leaf_end {
                return Err(bad(
                    "recoveryCoverage",
                    "ownership cover ends before its cell",
                ));
            }
        }
        // Negative IDs belong only to virtual ownership cells. Refinement parent/last
        // stay unchanged, so omission, G and drop checks still use actual delivery.
        for (i, k) in keys.iter().enumerate().take(split).skip(1) {
            let covered: Vec<_> = d
                .frontier
                .iter()
                .zip(&children)
                .filter(|(c, _)| k.contains(c.key))
                .collect();
            let first = covered.iter().map(|(_, c)| c.0).min().unwrap();
            let last = covered.iter().map(|(_, c)| c.1).max().unwrap();
            effort.charge(LOOKUP_UNITS)?;
            self.connection
                .execute(
                    "INSERT INTO recovery_cells VALUES(?,?,?)",
                    params![-(i as i64), first, last],
                )
                .map_err(sql)?;
        }
        for (i, child) in children.iter().enumerate() {
            let p = parents[split + i];
            effort.charge(LOOKUP_UNITS)?;
            self.connection
                .execute(
                    "UPDATE tiles SET cell=? WHERE id=?",
                    params![if p == 0 { root } else { -(p as i64) }, child.0],
                )
                .map_err(sql)?;
        }
        Ok(())
    }
}
