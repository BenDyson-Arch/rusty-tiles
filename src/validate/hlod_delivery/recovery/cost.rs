//! Retained work-cost probe, separate from the literal archive semantic controls.
use super::*;
use std::{cell::RefCell, sync::atomic::Ordering};

type Measurements = Vec<(&'static str, f64, u64)>;
thread_local! {
    static TIMES: RefCell<Option<Measurements>> = const { RefCell::new(None) };
}
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
pub(super) struct Phase<'a>(Option<(&'static str, f64, u64, &'a Effort)>);
impl<'a> Phase<'a> {
    pub(super) fn new(name: &'static str, effort: &'a Effort) -> Self {
        Self(
            TIMES
                .with_borrow(|v| v.is_some())
                .then(|| (name, cpu(), effort.used.load(Ordering::Relaxed), effort)),
        )
    }
}
impl Drop for Phase<'_> {
    fn drop(&mut self) {
        if let Some((name, start, units, effort)) = self.0 {
            let elapsed = cpu() - start;
            TIMES.with_borrow_mut(|v| {
                v.as_mut().unwrap().push((
                    name,
                    elapsed * 1e9,
                    effort.used.load(Ordering::Relaxed) - units,
                ))
            });
        }
    }
}
fn key(first: i64, end: i64, depth: i64) -> Value {
    serde_json::json!({"first":first,"end":end,"depth":depth,"leafBegin":first,"leafEnd":end})
}
// Eight disjoint frontier subtrees, each reached through an ownership-only chain.
// The SQL fixture contains actual terminal counts, independent of its declaration.
fn store(n: usize, chain: usize, declared: bool) -> Store {
    assert!(n.is_multiple_of(8) && chain <= 29);
    let (n, chain) = (n as i64, chain as i64);
    let mut s = super::super::tests::store();
    s.materialize().unwrap();
    let mut frontier = Vec::new();
    let mut omitted = Vec::new();
    for i in 0..8 {
        let first = i * n / 8;
        let end = (i + 1) * n / 8;
        frontier.push(serde_json::json!({"key":key(first,end,chain+1),"triangles":1}));
        for depth in 1..=chain {
            omitted.push(key(first, end, depth));
        }
    }
    let text = serde_json::json!({"version":1,"root":key(0,n,0),"encodedDepth":0,"frontier":frontier,"omitted":omitted}).to_string();
    {
        let mut tile = s.connection.prepare("INSERT INTO tiles(id,parent,world,refine,placeholder,last,leaf,error,cell,rank_first,ownership_depth,recovery) VALUES(?,?,X'',1,0,?,?,1,?,?,?,?)").unwrap();
        tile.execute(params![
            0,
            None::<i64>,
            n + 8,
            0,
            None::<i64>,
            0,
            0,
            declared.then_some(&text)
        ])
        .unwrap();
        for i in 0..8 {
            let first = i * n / 8;
            let end = (i + 1) * n / 8;
            let id = first + i + 1;
            tile.execute(params![
                id,
                0,
                end + i + 1,
                0,
                0,
                first,
                chain + 1,
                None::<&str>
            ])
            .unwrap();
            for j in first..end {
                let leaf = j + i + 2;
                tile.execute(params![leaf, id, leaf, 1, id, j, chain + 2, None::<&str>])
                    .unwrap();
            }
        }
    }
    s.connection.execute("INSERT INTO payloads(rank,identity_present,triangle_present,triangles,digest,recovery) VALUES(0,1,0,1,X'',?)", [declared]).unwrap();
    s.connection.execute("INSERT INTO payloads(rank,identity_present,triangle_present,triangles,digest) VALUES(1,1,0,1,X'')", []).unwrap();
    s.connection
        .execute(
            "INSERT INTO refs SELECT id,CASE WHEN id=0 THEN 0 ELSE 1 END FROM tiles",
            [],
        )
        .unwrap();
    s.connection.execute_batch("CREATE INDEX tile_parent ON tiles(parent); CREATE INDEX refs_tile ON refs(tile); CREATE INDEX refs_rank ON refs(rank);").unwrap();
    s.recovery_rows = (n as usize + 9, n as usize + 9);
    s
}
fn report(case: &str, n: usize, phase: &str, ns: f64, units: u64) {
    println!(
        "RECOVERY_COST {}",
        serde_json::json!({"case":case,"n":n,"phase":phase,"ns":ns,"units":units,"nsPerUnit":ns/units as f64})
    );
}
#[test]
#[ignore = "CPU-pinned calibration; runs no geometry or archive acceptance"]
fn recovery_work_cost_probe() {
    for n in [8, 2048, 131072] {
        for (name, chain, declared) in [
            ("absent", 1, false),
            ("recovery", 1, true),
            ("deep", 29, true),
        ] {
            if name == "deep" && n != 8 {
                continue;
            }
            for _ in 0..3 {
                let s = store(n, chain, declared);
                let mut effort = super::super::tests::effort_for_tests();
                effort.budget.value = u64::MAX;
                TIMES.with_borrow_mut(|v| *v = Some(Vec::new()));
                s.recovery(&effort).unwrap();
                for (phase, ns, units) in TIMES.with_borrow_mut(Option::take).unwrap() {
                    report(name, n, phase, ns, units);
                }
            }
        }
    }
    let mut s = store(131072, 1, false);
    let wide: serde_json::Map<String, Value> =
        (0..10000).map(|i| (format!("k{i}"), Value::Null)).collect();
    for (name, node, per) in [
        (
            "wide-missing",
            serde_json::json!({"extras":wide}),
            READ_UNITS,
        ),
        (
            "tiny-fields",
            serde_json::json!({"extras":{"recovery":vec![0; 30000]}}),
            READ_UNITS + METADATA_UNITS + MAX_JSON * JSON_BYTE_UNITS,
        ),
        (
            "long-rank",
            serde_json::json!({"extras":{"first":"0".repeat(16*MAX_JSON),"depth":0}}),
            READ_UNITS,
        ),
        ("missing", serde_json::json!({}), READ_UNITS),
        (
            "ordinary",
            serde_json::json!({"extras":{"first":0,"depth":0}}),
            READ_UNITS + METADATA_UNITS,
        ),
        (
            "declaration",
            serde_json::json!({"extras":{"first":0,"depth":0,"recovery": {"text":"x".repeat(MAX_JSON-16)}}}),
            READ_UNITS + METADATA_UNITS + MAX_JSON * JSON_BYTE_UNITS,
        ),
    ] {
        let n = if matches!(name, "declaration" | "tiny-fields") {
            1000
        } else {
            131072
        };
        for _ in 0..3 {
            let start = cpu();
            for id in 0..n {
                if let Some(m) = Metadata::read(std::hint::black_box(&node)).unwrap() {
                    s.metadata(id as u64, &m).unwrap();
                }
            }
            report(name, n, "metadata", (cpu() - start) * 1e9, (n * per) as u64);
        }
    }
    for (name, value) in [
        (
            "oversize-value",
            serde_json::json!({"x":"x".repeat(16*MAX_JSON)}),
        ),
        (
            "oversize-key",
            Value::Object(
                [("x".repeat(16 * MAX_JSON), Value::Null)]
                    .into_iter()
                    .collect(),
            ),
        ),
        ("oversize-array", serde_json::json!(vec![0; 16 * MAX_JSON])),
    ] {
        let node = serde_json::json!({"extras":{"recovery":value}});
        for _ in 0..3 {
            let start = cpu();
            for _ in 0..1000 {
                assert!(Metadata::read(std::hint::black_box(&node)).is_err());
            }
            report(
                name,
                1000,
                "metadata-refusal",
                (cpu() - start) * 1e9,
                (1000 * (READ_UNITS + MAX_JSON * JSON_BYTE_UNITS)) as u64,
            );
        }
    }
    for n in [2048, 131072] {
        let s = store(n, 1, false);
        {
            let mut put = s
                .connection
                .prepare("INSERT INTO border_map VALUES(?,?,X'',0)")
                .unwrap();
            for id in 1..=n {
                put.execute(params![(id as u64).to_be_bytes().as_slice(), id as i64])
                    .unwrap();
            }
        }
        let old = super::super::borders::OWNERS
            .replace("coalesce(v.last,c.last)", "c.last")
            .replace(" LEFT JOIN recovery_cells v ON v.id=t.cell", "")
            .replace("coalesce(v.first,c.id+1)", "c.id+1");
        for virtual_cells in [false, true] {
            if virtual_cells {
                for id in 1..=232i64 {
                    s.connection
                        .execute(
                            "INSERT INTO recovery_cells VALUES(?,?,?)",
                            params![-id, 1, n as i64],
                        )
                        .unwrap();
                }
                s.connection
                    .execute("UPDATE tiles SET cell=-(id%232+1) WHERE id%2=0", [])
                    .unwrap();
            }
            for _ in 0..3 {
                for (name, sql) in [
                    ("old", old.as_str()),
                    ("current", super::super::borders::OWNERS),
                ] {
                    let start = cpu();
                    let mut query = s.connection.prepare(sql).unwrap();
                    let mut rows = query.query([]).unwrap();
                    let mut count = 0;
                    while let Some(row) = rows.next().unwrap() {
                        let a: Option<i64> = row.get(4).unwrap();
                        let b: Option<i64> = row.get(9).unwrap();
                        std::hint::black_box((a, b));
                        count += 1;
                    }
                    assert_eq!(count, n);
                    report(
                        if virtual_cells {
                            "virtual-border"
                        } else {
                            "ordinary-border"
                        },
                        n,
                        name,
                        (cpu() - start) * 1e9,
                        (n * BORDER_UNITS) as u64,
                    );
                }
            }
        }
    }
    for n in [128, 8192, 131072] {
        let mut v = serde_json::Map::new();
        for i in 0..n {
            v.insert(format!("a{i}"), serde_json::json!(0));
        }
        // Sorted keys put the desired key after most small fields; escaped keys
        // exercise identity::field's decoded-key path as well as its raw scan.
        v.insert(
            "asset".into(),
            serde_json::json!({"extras":{"rustyTiles":{"descendantRecovery":1}}}),
        );
        let text = serde_json::to_string(&v)
            .unwrap()
            .replace("asset", "\\u0061sset");
        let raw: &RawValue = serde_json::from_str(&text).unwrap();
        for _ in 0..3 {
            let start = cpu();
            for _ in 0..100 {
                assert!(payload_marker(std::hint::black_box(raw)).unwrap());
            }
            report(
                "marker",
                n,
                "json",
                (cpu() - start) * 1e9,
                (100 * text.len() * 4 * JSON_BYTE_UNITS) as u64,
            );
        }
    }
}
#[test]
fn recovery_cost_refuses_before_census_and_bounds_metadata_serialization() {
    let s = store(8, 1, true);
    let mut effort = super::super::tests::effort_for_tests();
    effort.budget.value = SETUP_UNITS as u64;
    assert!(matches!(s.recovery(&effort), Err(Error::ResourceLimit(_))));
    let exists: bool = s
        .connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_temp_master WHERE name='recovery_leaves')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!exists);
    let node = serde_json::json!({"extras":{"recovery":{"x":"x".repeat(MAX_JSON)}}});
    assert!(matches!(Metadata::read(&node), Err(Error::InvalidInput(_))));
}

#[test]
fn recovery_metadata_bounds_precede_string_scans_and_match_canonical_ranks() {
    for text in [
        "".to_owned(),
        "00".to_owned(),
        "+0".to_owned(),
        "0".repeat(16 * MAX_JSON),
    ] {
        let node = serde_json::json!({"extras":{"first":text,"depth":0}});
        assert!(Metadata::read(&node).unwrap().is_none());
    }
    for n in [0, u64::MAX] {
        for value in [serde_json::json!(n), serde_json::json!(n.to_string())] {
            let node = serde_json::json!({"extras":{"first":value,"depth":0}});
            assert_eq!(Metadata::read(&node).unwrap().unwrap().owner, Some((n, 0)));
        }
    }
    let mut deep = Value::Null;
    for _ in 0..9 {
        deep = Value::Array(vec![deep]);
    }
    for value in [
        deep,
        serde_json::json!(vec![0; MAX_JSON + 1]),
        serde_json::json!({"x":"x".repeat(MAX_JSON)}),
        serde_json::json!({"x":"\n".repeat(MAX_JSON/2)}),
    ] {
        let node = serde_json::json!({"extras":{"recovery":value}});
        assert!(matches!(Metadata::read(&node), Err(Error::InvalidInput(_))));
    }
}
