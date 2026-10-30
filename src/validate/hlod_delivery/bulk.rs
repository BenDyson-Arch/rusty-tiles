//! Bulk loads of the disk store. A row whose key arrives in payload order (leaf points,
//! segments and faces, ancestor requirements, border images, coincident-face twins and
//! multi-edge leaf faces) would be a random-key B-tree insert, whose cost grows with the
//! table once it outgrows SQLite's bounded page cache. Such rows go to one bounded external
//! sort at a time and are inserted in primary-key order, so every insert appends at the end of
//! its B-tree and costs the same at any table size. Each row is charged where it is produced,
//! as before; the load itself adds no charge. Rows with one key are merged the way the former
//! upserts merged them, so the tables hold exactly what they held.
use super::{spill_failure, sql, Error, Store};
use crate::spill::{ExternalSorter, Record, SortedStream};
use rusqlite::params;

/// A spill record whose bytes are its sort key: blobs as stored (SQLite compares blobs with
/// `memcmp`) and integers big-endian with the sign bit flipped, so sorted order is the store's
/// primary-key order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Packed<const N: usize>([u8; N]);
impl<const N: usize> Record for Packed<N> {
    const SIZE: usize = N;
    fn encode(&self, out: &mut [u8]) {
        out.copy_from_slice(&self.0);
    }
    fn decode(bytes: &[u8]) -> Self {
        Self(bytes.try_into().expect("record size"))
    }
}
impl<const N: usize> Packed<N> {
    fn new(parts: &[&[u8]]) -> Self {
        let mut bytes = [0; N];
        let mut at = 0;
        for part in parts {
            bytes[at..at + part.len()].copy_from_slice(part);
            at += part.len();
        }
        Self(bytes)
    }
    fn bytes(&self, at: usize, len: usize) -> &[u8] {
        &self.0[at..at + len]
    }
    fn integer(&self, at: usize) -> i64 {
        (u64::from_be_bytes(self.0[at..at + 8].try_into().unwrap()) ^ SIGN) as i64
    }
}
const SIGN: u64 = 1 << 63;
fn ordered(n: i64) -> [u8; 8] {
    ((n as u64) ^ SIGN).to_be_bytes()
}

pub(super) type Sorter<'s, const N: usize> = ExternalSorter<'s, Packed<N>>;

/// Leaf rows: kind (0 point, 1 segment, 2 face), key padded to 36 bytes, tile, and the
/// segment's forward and reverse incidences.
pub(super) type LeafRow = Packed<61>;
const POINT: u8 = 0;
const SEGMENT: u8 = 1;
const FACE: u8 = 2;
const KEY: usize = 36;
fn key_len(kind: u8) -> usize {
    12 * (usize::from(kind) + 1)
}
fn leaf_row(kind: u8, key: &[u8], tile: i64, counts: [i64; 2]) -> LeafRow {
    let mut padded = [0; KEY];
    padded[..key.len()].copy_from_slice(key);
    Packed::new(&[
        &[kind],
        &padded,
        &ordered(tile),
        &ordered(counts[0]),
        &ordered(counts[1]),
    ])
}
pub(super) fn point_row(key: &[u8], tile: i64) -> LeafRow {
    leaf_row(POINT, key, tile, [0, 0])
}
pub(super) fn segment_row(key: &[u8], tile: i64, forward: i64, reverse: i64) -> LeafRow {
    leaf_row(SEGMENT, key, tile, [forward, reverse])
}
pub(super) fn face_row(corners: &[u8], tile: i64) -> LeafRow {
    leaf_row(FACE, corners, tile, [0, 0])
}

/// Ancestor requirements: tile, kind (0 point, 1 segment), key padded to 24 bytes, directions.
pub(super) type Need = Packed<41>;
pub(super) fn need(tile: i64, kind: i64, key: &[u8], directions: i64) -> Need {
    let mut padded = [0; 24];
    padded[..key.len()].copy_from_slice(key);
    Packed::new(&[&ordered(tile), &[kind as u8], &padded, &ordered(directions)])
}

/// A coarse tile's border vertex image: key, tile, whether it lies on a chord, the chord
/// image (40 bytes, or none when retained) and the displacement's bits.
pub(super) type Image = Packed<69>;
pub(super) fn image(key: &[u8], tile: i64, image: &[u8], d: f64) -> Image {
    let mut padded = [0; 40];
    padded[..image.len()].copy_from_slice(image);
    Packed::new(&[
        key,
        &ordered(tile),
        &[u8::from(!image.is_empty())],
        &padded,
        &d.to_bits().to_be_bytes(),
    ])
}

/// A leaf face on a segment of a replayed leaf: key, tile, third corner and direction.
pub(super) type EdgeFace = Packed<45>;
pub(super) fn edge_face(key: &[u8], tile: i64, third: &[u8], reversed: bool) -> EdgeFace {
    Packed::new(&[key, &ordered(tile), third, &[u8::from(reversed)]])
}

/// An open segment's higher endpoint, whose lower endpoint is a shared point.
pub(super) type End = Packed<12>;
pub(super) fn end(key: &[u8]) -> End {
    Packed::new(&[key])
}

fn stream<const N: usize>(sorter: Sorter<'_, N>) -> Result<SortedStream<'_, Packed<N>>, Error> {
    sorter.finish().map_err(spill_failure)
}

/// A forward-only reader of one table's keys in primary-key order, for merge joins.
struct Keys<'a> {
    rows: rusqlite::Rows<'a>,
    current: Option<Vec<u8>>,
}
impl Keys<'_> {
    /// Whether `key` is in the table; keys must be asked in nondecreasing order.
    fn contains(&mut self, key: &[u8]) -> Result<bool, Error> {
        while self.current.as_deref().is_some_and(|c| c < key) {
            self.current = self
                .rows
                .next()
                .map_err(sql)?
                .map(|r| r.get(0))
                .transpose()
                .map_err(sql)?;
        }
        Ok(self.current.as_deref() == Some(key))
    }
}

impl Store {
    pub(super) fn sorter<const N: usize>(&self) -> Result<Sorter<'_, N>, Error> {
        ExternalSorter::new(&self._scratch, self.sort).map_err(spill_failure)
    }

    fn keys<'a>(statement: &'a mut rusqlite::Statement<'_>) -> Result<Keys<'a>, Error> {
        let mut rows = statement.query([]).map_err(sql)?;
        let current = rows
            .next()
            .map_err(sql)?
            .map(|r| r.get(0))
            .transpose()
            .map_err(sql)?;
        Ok(Keys { rows, current })
    }

    /// Loads the leaf points, segments and faces in key order. One (key, tile) is one row:
    /// points once, segment incidences summed, faces counted.
    pub(super) fn load_leaves(&self, rows: Sorter<'_, 61>) -> Result<(), Error> {
        let mut point = self
            .connection
            .prepare_cached("INSERT INTO leaf_points VALUES(?,?)")
            .map_err(sql)?;
        let mut segment = self
            .connection
            .prepare_cached("INSERT INTO leaf_segments VALUES(?,?,?,?)")
            .map_err(sql)?;
        let mut face = self
            .connection
            .prepare_cached("INSERT INTO leaf_faces VALUES(?,?,?)")
            .map_err(sql)?;
        let mut insert = |row: &LeafRow, counts: [i64; 2]| -> Result<(), Error> {
            let kind = row.0[0];
            let key = row.bytes(1, key_len(kind));
            let tile = row.integer(1 + KEY);
            match kind {
                POINT => point.execute(params![key, tile]),
                SEGMENT => segment.execute(params![key, tile, counts[0], counts[1]]),
                _ => face.execute(params![key, tile, counts[0]]),
            }
            .map_err(sql)?;
            Ok(())
        };
        // Kind, key and tile: the primary key of the row's table.
        const GROUP: usize = 1 + KEY + 8;
        let mut group: Option<(LeafRow, [i64; 2])> = None;
        for row in stream(rows)? {
            let row = row.map_err(spill_failure)?;
            let counts = match row.0[0] {
                SEGMENT => [row.integer(GROUP), row.integer(GROUP + 8)],
                FACE => [1, 0],
                _ => [0, 0],
            };
            match &mut group {
                Some((first, sum)) if first.0[..GROUP] == row.0[..GROUP] => {
                    for (s, n) in sum.iter_mut().zip(counts) {
                        *s = s.checked_add(n).ok_or_else(|| {
                            Error::limit(
                                "interfaceSegments: per-leaf face incidence exceeds disk counter range",
                            )
                        })?;
                    }
                }
                _ => {
                    if let Some((first, sum)) = group.replace((row, counts)) {
                        insert(&first, sum)?;
                    }
                }
            }
        }
        if let Some((first, sum)) = group {
            insert(&first, sum)?;
        }
        Ok(())
    }

    /// Every corner of each coincident leaf corner set (faces on one corner set, across all
    /// leaves, more than one), in key order.
    pub(super) fn load_twins(&self) -> Result<(), Error> {
        let mut twins = self.sorter::<48>()?;
        let mut query = self
            .connection
            .prepare("SELECT corners FROM leaf_faces GROUP BY corners HAVING sum(n)>1")
            .map_err(sql)?;
        let mut rows = query.query([]).map_err(sql)?;
        while let Some(row) = rows.next().map_err(sql)? {
            let corners: Vec<u8> = row.get(0).map_err(sql)?;
            for key in corners.chunks(12) {
                twins
                    .push(Packed::new(&[key, &corners]))
                    .map_err(spill_failure)?;
            }
        }
        drop(rows);
        let mut insert = self
            .connection
            .prepare_cached("INSERT INTO twin_points VALUES(?,?)")
            .map_err(sql)?;
        let mut previous = None;
        for twin in stream(twins)? {
            let twin = twin.map_err(spill_failure)?;
            if previous != Some(twin) {
                insert
                    .execute(params![twin.bytes(0, 12), twin.bytes(12, 36)])
                    .map_err(sql)?;
            }
            previous = Some(twin);
        }
        Ok(())
    }

    /// The faces on multi-incidence segments, by a merge join of the replayed faces with
    /// `multi_edges`, in key order.
    pub(super) fn load_edge_faces(&self, faces: Sorter<'_, 45>) -> Result<(), Error> {
        let mut statement = self
            .connection
            .prepare("SELECT key FROM multi_edges ORDER BY key")
            .map_err(sql)?;
        let mut multi = Self::keys(&mut statement)?;
        let mut insert = self
            .connection
            .prepare_cached("INSERT INTO edge_faces VALUES(?,?,?,?)")
            .map_err(sql)?;
        for face in stream(faces)? {
            let face = face.map_err(spill_failure)?;
            let key = face.bytes(0, 24);
            if multi.contains(key)? {
                insert
                    .execute(params![
                        key,
                        face.integer(24),
                        face.bytes(32, 12),
                        i64::from(face.0[44])
                    ])
                    .map_err(sql)?;
            }
        }
        Ok(())
    }

    /// The ancestor requirements, one row per (tile, kind, key) with its directions joined.
    pub(super) fn load_requirements(&self, needs: Sorter<'_, 41>) -> Result<(), Error> {
        let mut insert = self
            .connection
            .prepare_cached("INSERT INTO requirements VALUES(?,?,?,?)")
            .map_err(sql)?;
        let mut write = |n: &Need, directions: i64| {
            let kind = n.0[8];
            insert
                .execute(params![
                    n.integer(0),
                    i64::from(kind),
                    n.bytes(9, key_len(kind)),
                    directions
                ])
                .map(|_| ())
                .map_err(sql)
        };
        const GROUP: usize = 8 + 1 + 24;
        let mut group: Option<(Need, i64)> = None;
        for n in stream(needs)? {
            let n = n.map_err(spill_failure)?;
            let directions = n.integer(GROUP);
            match &mut group {
                Some((first, d)) if first.0[..GROUP] == n.0[..GROUP] => *d |= directions,
                _ => {
                    if let Some((first, d)) = group.replace((n, directions)) {
                        write(&first, d)?;
                    }
                }
            }
        }
        if let Some((first, d)) = group {
            write(&first, d)?;
        }
        Ok(())
    }

    /// The coarse border images, in key order.
    pub(super) fn load_images(&self, images: Sorter<'_, 69>) -> Result<(), Error> {
        let mut insert = self
            .connection
            .prepare_cached("INSERT INTO border_map VALUES(?,?,?,?)")
            .map_err(sql)?;
        for i in stream(images)? {
            let i = i.map_err(spill_failure)?;
            let image = if i.0[20] == 0 {
                &[][..]
            } else {
                i.bytes(21, 40)
            };
            let d = f64::from_bits(u64::from_be_bytes(i.bytes(61, 8).try_into().unwrap()));
            insert
                .execute(params![i.bytes(0, 12), i.integer(12), image, d])
                .map_err(sql)?;
        }
        Ok(())
    }

    /// Open segments whose endpoints are both shared points. Segments arrive in key order, so
    /// their lower endpoints are merged with `shared_point_keys` at once; each higher endpoint
    /// of a match is sorted and merged after the scan.
    pub(super) fn shared_ends(&self, ends: Sorter<'_, 12>) -> Result<u64, Error> {
        let mut statement = self
            .connection
            .prepare("SELECT key FROM shared_point_keys ORDER BY key")
            .map_err(sql)?;
        let mut shared = Self::keys(&mut statement)?;
        let mut count = 0u64;
        for e in stream(ends)? {
            let e = e.map_err(spill_failure)?;
            if shared.contains(&e.0)? {
                count += 1;
            }
        }
        Ok(count)
    }
}

/// The lower endpoints of open segments, read in segment key order, against the shared points.
pub(super) struct LowerEnds<'a> {
    keys: Keys<'a>,
}
impl<'a> LowerEnds<'a> {
    pub(super) fn new(statement: &'a mut rusqlite::Statement<'_>) -> Result<Self, Error> {
        Ok(Self {
            keys: Store::keys(statement)?,
        })
    }
    pub(super) fn shared(&mut self, key: &[u8]) -> Result<bool, Error> {
        self.keys.contains(key)
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{budget, replay_fixture, store_with};
    use super::super::{
        delivery_frame, edge_bytes, point_bytes, replay_surface, Budget, CanonicalPoint,
        DecodedInterfaceCensus, Effort, Point, Store, IDENTITY,
    };
    use super::*;
    use crate::archive3tz::CatalogConfig;
    use std::collections::{BTreeMap, BTreeSet};
    use std::sync::{atomic::AtomicU64, Arc};

    const CONFIGS: [CatalogConfig; 2] = [CatalogConfig::DEFAULT, CatalogConfig::FORCED_SPILL];

    fn rows(s: &Store, query: &str) -> Vec<Vec<rusqlite::types::Value>> {
        let mut q = s.connection.prepare(query).unwrap();
        let n = q.column_count();
        q.query_map([], |r| (0..n).map(|i| r.get(i)).collect())
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }
    fn blob(bytes: &[u8]) -> rusqlite::types::Value {
        rusqlite::types::Value::Blob(bytes.to_vec())
    }
    fn int(n: i64) -> rusqlite::types::Value {
        rusqlite::types::Value::Integer(n)
    }

    /// Every bulk-loaded table, from rows pushed out of key order with repeated keys, in
    /// memory and in runs of a record or two: each table holds exactly what its former
    /// per-row upsert built.
    #[test]
    fn bulk_loads_merge_repeated_keys_like_the_former_upserts() {
        let p = |n: u8| [n; 12];
        let e = |n: u8| [n; 24];
        let f = |n: u8| {
            let mut c = [0u8; 36];
            for (i, b) in c.iter_mut().enumerate() {
                *b = n + (i / 12) as u8;
            }
            c
        };
        for config in &CONFIGS {
            let mut s = store_with(config);
            s.materialize().unwrap();
            let mut leaves = s.sorter().unwrap();
            for row in [
                face_row(&f(1), 1),
                point_row(&p(9), 2),
                segment_row(&e(3), 1, 1, 0),
                point_row(&p(9), 1),
                face_row(&f(1), 2),
                segment_row(&e(3), 2, 1, 1),
                point_row(&p(9), 2),
                face_row(&f(1), 1),
                segment_row(&e(3), 1, 0, 2),
                point_row(&p(4), 1),
                face_row(&f(7), 1),
                segment_row(&e(2), 1, 1, 0),
                face_row(&f(1), 1),
            ] {
                leaves.push(row).unwrap();
            }
            s.load_leaves(leaves).unwrap();
            assert_eq!(
                rows(&s, "SELECT key,tile FROM leaf_points ORDER BY key,tile"),
                [(4, 1), (9, 1), (9, 2)].map(|(k, t)| vec![blob(&p(k)), int(t)])
            );
            assert_eq!(
                rows(
                    &s,
                    "SELECT key,tile,nf,nr FROM leaf_segments ORDER BY key,tile"
                ),
                [(2, 1, 1, 0), (3, 1, 1, 2), (3, 2, 1, 1)].map(|(k, t, a, b)| vec![
                    blob(&e(k)),
                    int(t),
                    int(a),
                    int(b)
                ])
            );
            assert_eq!(
                rows(
                    &s,
                    "SELECT corners,tile,n FROM leaf_faces ORDER BY corners,tile"
                ),
                [(1, 1, 3), (1, 2, 1), (7, 1, 1)].map(|(k, t, n)| vec![
                    blob(&f(k)),
                    int(t),
                    int(n)
                ])
            );
            // Twins: each corner of every corner set with more than one face in all.
            s.load_twins().unwrap();
            assert_eq!(
                rows(
                    &s,
                    "SELECT key,corners FROM twin_points ORDER BY key,corners"
                ),
                (0..3)
                    .map(|i| vec![blob(&f(1)[12 * i..12 * i + 12]), blob(&f(1))])
                    .collect::<Vec<_>>()
            );
            // Edge faces: only multi edges, every face side kept.
            s.connection
                .execute("INSERT INTO multi_edges VALUES(?)", [e(3)])
                .unwrap();
            let mut faces = s.sorter().unwrap();
            for (k, t, third, reversed) in [
                (3, 2, 5, true),
                (2, 1, 6, false),
                (3, 1, 5, false),
                (3, 1, 5, false),
            ] {
                faces
                    .push(edge_face(&e(k), t, &p(third), reversed))
                    .unwrap();
            }
            s.load_edge_faces(faces).unwrap();
            assert_eq!(
                rows(
                    &s,
                    "SELECT key,tile,third,dir FROM edge_faces ORDER BY key,tile,dir"
                ),
                [(1, 0), (1, 0), (2, 1)].map(|(t, d)| vec![
                    blob(&e(3)),
                    int(t),
                    blob(&p(5)),
                    int(d)
                ])
            );
            // Requirements: one row per (tile, kind, key), directions joined.
            let mut needs = s.sorter().unwrap();
            for n in [
                need(5, 1, &e(3), 1),
                need(5, 0, &p(9), 0),
                need(2, 1, &e(2), 1),
                need(5, 1, &e(3), 2),
            ] {
                needs.push(n).unwrap();
            }
            s.load_requirements(needs).unwrap();
            assert_eq!(
                rows(
                    &s,
                    "SELECT tile,kind,key,directions FROM requirements ORDER BY tile,kind,key"
                ),
                vec![
                    vec![int(2), int(1), blob(&e(2)), int(1)],
                    vec![int(5), int(0), blob(&p(9)), int(0)],
                    vec![int(5), int(1), blob(&e(3)), int(3)],
                ]
            );
            // Border images: retained (empty) and on a chord, with the displacement's bits.
            let mut images = s.sorter().unwrap();
            images.push(image(&p(9), 3, &[7; 40], 0.25)).unwrap();
            images.push(image(&p(4), 3, &[], 0.)).unwrap();
            s.load_images(images).unwrap();
            assert_eq!(
                rows(
                    &s,
                    "SELECT key,tile,image,d FROM border_map ORDER BY key,tile"
                ),
                vec![
                    vec![
                        blob(&p(4)),
                        int(3),
                        blob(&[]),
                        rusqlite::types::Value::Real(0.)
                    ],
                    vec![
                        blob(&p(9)),
                        int(3),
                        blob(&[7; 40]),
                        rusqlite::types::Value::Real(0.25)
                    ],
                ]
            );
            // Higher endpoints of open segments: each one shared counts once.
            for k in [4, 9] {
                s.connection
                    .execute("INSERT INTO shared_point_keys VALUES(?)", [p(k)])
                    .unwrap();
            }
            let mut ends = s.sorter().unwrap();
            for k in [9, 5, 4, 9] {
                ends.push(end(&p(k))).unwrap();
            }
            assert_eq!(s.shared_ends(ends).unwrap(), 3);
        }
    }

    #[test]
    fn summed_incidence_beyond_the_counter_range_is_refused() {
        let mut s = store_with(&CatalogConfig::FORCED_SPILL);
        s.materialize().unwrap();
        let mut leaves = s.sorter().unwrap();
        leaves.push(segment_row(&[1; 24], 1, i64::MAX, 0)).unwrap();
        leaves.push(segment_row(&[1; 24], 1, 1, 0)).unwrap();
        assert!(s
            .load_leaves(leaves)
            .unwrap_err()
            .to_string()
            .contains("per-leaf face incidence exceeds disk counter range"));
    }

    /// The hostile leaf class at CI size: leaves of pseudo-random triangles on a small
    /// lattice (shared points, shared and multi edges, coincident faces), loaded in bulk.
    /// The leaf tables equal the per-placement surfaces' rows, and the census and charged work
    /// are the same in memory and in runs of a record or two.
    #[test]
    fn hostile_random_leaves_load_every_row_once_at_any_run_size() {
        const LEAVES: u64 = 12;
        let mut x = 0x2545_F491_4F6C_DD1Du64;
        let mut next = || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            f64::from((x % 5) as u8) / 4.
        };
        let payloads: Vec<Vec<u8>> = (0..LEAVES)
            .map(|_| {
                let mut faces: Vec<[Point; 3]> = (0..96)
                    .map(|_| std::array::from_fn(|_| std::array::from_fn(|_| next())))
                    .collect();
                // A repeated face and a reversed one: per-leaf counts above one.
                let [a, b, c] = faces[1];
                faces.extend([faces[0], [b, a, c]]);
                replay_fixture(&faces)
            })
            .collect();
        let budget = Budget {
            value: 1 << 40,
            ..budget()
        };
        // Expected rows from each placement's surface, independently of the bulk path.
        let mut points = BTreeSet::<(Vec<u8>, i64)>::new();
        let mut segments = BTreeMap::<(Vec<u8>, i64), (i64, i64)>::new();
        let mut faces = BTreeMap::<(Vec<u8>, i64), i64>::new();
        let effort = Effort {
            budget,
            base: 0,
            used: Arc::new(AtomicU64::new(0)),
        };
        for (i, bytes) in payloads.iter().enumerate() {
            let tile = i as i64 + 1;
            let surface =
                replay_surface(bytes, delivery_frame(IDENTITY), 1 << 24, &effort).unwrap();
            for p in &surface.points {
                points.insert((point_bytes(*p), tile));
            }
            for (k, e) in &surface.edges {
                let c = segments.entry((edge_bytes(*k), tile)).or_default();
                *c = (c.0 + e.forward as i64, c.1 + e.reverse as i64);
            }
            for f in &surface.faces {
                let mut k: [CanonicalPoint; 3] = *f;
                k.sort_unstable();
                *faces
                    .entry((k.iter().flat_map(|p| point_bytes(*p)).collect(), tile))
                    .or_default() += 1;
            }
        }
        let owners = |keys: Vec<Vec<u8>>| {
            let mut by = BTreeMap::<Vec<u8>, usize>::new();
            for k in keys {
                *by.entry(k).or_default() += 1;
            }
            by
        };
        let shared_points = owners(points.iter().map(|(k, _)| k.clone()).collect());
        let shared_segments = owners(segments.keys().map(|(k, _)| k.clone()).collect());
        let mut totals = BTreeMap::<Vec<u8>, i64>::new();
        for ((k, _), (a, b)) in &segments {
            *totals.entry(k.clone()).or_default() += a + b;
        }
        let shared = |k: &[u8]| shared_points.get(k).is_some_and(|n| *n > 1);
        let open: Vec<&Vec<u8>> = totals
            .iter()
            .filter(|(_, n)| **n == 1)
            .map(|(k, _)| k)
            .collect();
        let expected = DecodedInterfaceCensus {
            shared_points: shared_points.values().filter(|n| **n > 1).count() as u64,
            shared_segments: shared_segments.values().filter(|n| **n > 1).count() as u64,
            source_open_edges: open.len() as u64,
            source_open_edges_with_shared_endpoints: open
                .iter()
                .filter(|k| shared(&k[..12]) && shared(&k[12..]))
                .count() as u64,
            max_payload_triangles: 0,
            ..Default::default()
        };
        assert!(expected.shared_points > 0 && expected.shared_segments > 0);
        assert!(expected.source_open_edges_with_shared_endpoints > 0);
        assert!(segments.values().any(|(a, b)| a + b > 1) && faces.values().any(|n| *n > 1));
        let mut work = Vec::new();
        for config in &CONFIGS {
            let mut s = store_with(config);
            s.decoded_limit = 1 << 24;
            s.tile(0, None, IDENTITY).unwrap();
            s.kind(0, None, false, 16., Some(1.)).unwrap();
            for leaf in 1..=LEAVES {
                s.tile(leaf, Some(0), IDENTITY).unwrap();
                s.reference(leaf, leaf - 1).unwrap();
                s.payload(leaf - 1, true, true).unwrap();
            }
            work.push(
                s.finish(budget, 0, &mut |rank| Ok(payloads[rank as usize].clone()))
                    .unwrap(),
            );
            let mut census = s.census().unwrap();
            census.omission = Default::default();
            assert_eq!(census, expected);
            let got = |q: &str| rows(&s, q);
            assert_eq!(
                got("SELECT key,tile FROM leaf_points ORDER BY key,tile"),
                points
                    .iter()
                    .map(|(k, t)| vec![blob(k), int(*t)])
                    .collect::<Vec<_>>()
            );
            assert_eq!(
                got("SELECT key,tile,nf,nr FROM leaf_segments ORDER BY key,tile"),
                segments
                    .iter()
                    .map(|((k, t), (a, b))| vec![blob(k), int(*t), int(*a), int(*b)])
                    .collect::<Vec<_>>()
            );
            assert_eq!(
                got("SELECT corners,tile,n FROM leaf_faces ORDER BY corners,tile"),
                faces
                    .iter()
                    .map(|((k, t), n)| vec![blob(k), int(*t), int(*n)])
                    .collect::<Vec<_>>()
            );
        }
        assert_eq!(work[0], work[1]);
    }
}
