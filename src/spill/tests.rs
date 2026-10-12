use super::*;
use std::{
    fs::File,
    io::{Read, Write},
    path::Path,
    sync::{
        atomic::{AtomicU64, AtomicUsize, Ordering},
        Arc,
    },
};

/// Orders by `key` only, so ties exercise stability; `seq` records input position.
#[derive(Clone, Copy, Debug)]
struct Keyed {
    key: u16,
    seq: u32,
}
impl PartialEq for Keyed {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}
impl Eq for Keyed {}
impl PartialOrd for Keyed {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Keyed {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.key.cmp(&other.key)
    }
}
impl Record for Keyed {
    const SIZE: usize = 6;
    fn encode(&self, out: &mut [u8]) {
        out[..2].copy_from_slice(&self.key.to_le_bytes());
        out[2..].copy_from_slice(&self.seq.to_le_bytes());
    }
    fn decode(bytes: &[u8]) -> Self {
        Self {
            key: u16::from_le_bytes([bytes[0], bytes[1]]),
            seq: u32::from_le_bytes([bytes[2], bytes[3], bytes[4], bytes[5]]),
        }
    }
}

/// A total-order record shaped like (Morton key, source tuple).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Wide {
    morton: u64,
    a: u32,
    b: u32,
}
impl Record for Wide {
    const SIZE: usize = 16;
    fn encode(&self, out: &mut [u8]) {
        out[..8].copy_from_slice(&self.morton.to_le_bytes());
        out[8..12].copy_from_slice(&self.a.to_le_bytes());
        out[12..].copy_from_slice(&self.b.to_le_bytes());
    }
    fn decode(bytes: &[u8]) -> Self {
        Self {
            morton: u64::from_le_bytes(bytes[..8].try_into().unwrap()),
            a: u32::from_le_bytes(bytes[8..12].try_into().unwrap()),
            b: u32::from_le_bytes(bytes[12..].try_into().unwrap()),
        }
    }
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
}

fn keyed_input(n: usize, keys: u64, seed: u64) -> Vec<Keyed> {
    let mut rng = Rng(seed);
    (0..n)
        .map(|seq| Keyed {
            key: (rng.next() % keys) as u16,
            seq: seq as u32,
        })
        .collect()
}

fn wide(rng: &mut Rng, i: u64, keys: u64) -> Wide {
    Wide {
        morton: rng.next() % keys,
        a: i as u32,
        b: rng.next() as u32,
    }
}

fn config<T>(run_records: usize, fan_in: usize, jobs: usize, buffer_bytes: usize) -> SortConfig {
    SortConfig {
        run_bytes: run_records * size_of::<T>(),
        fan_in,
        buffer_bytes,
        jobs,
    }
}

fn new_scratch(io: Arc<dyn SpillIo>) -> (tempfile::TempDir, Scratch) {
    let parent = tempfile::tempdir().unwrap();
    let scratch = Scratch::create_with(Some(parent.path()), 0, io, inspect_volume).unwrap();
    (parent, scratch)
}

fn sort<T: Record>(
    scratch: &Scratch,
    config: SortConfig,
    input: &[T],
) -> Result<(Vec<T>, SortStats), SpillError> {
    let mut sorter = ExternalSorter::new(scratch, config)?;
    sorter.extend(input.iter().copied())?;
    let stream = sorter.finish()?;
    let stats = stream.stats();
    assert_eq!(stream.remaining(), input.len() as u64);
    Ok((stream.collect::<Result<_, _>>()?, stats))
}

fn entries(path: &Path) -> usize {
    std::fs::read_dir(path).unwrap().count()
}

fn pairs(records: &[Keyed]) -> Vec<(u16, u32)> {
    records.iter().map(|r| (r.key, r.seq)).collect()
}

fn encoded<T: Record>(records: &[T]) -> Vec<u8> {
    let mut bytes = vec![0; records.len() * T::SIZE];
    for (record, out) in records.iter().zip(bytes.chunks_mut(T::SIZE)) {
        record.encode(out);
    }
    bytes
}

#[test]
fn sweep_replays_in_memory_stable_sort_over_r_f_jobs_and_counts() {
    let mut cases = 0;
    for n in [0usize, 1, 2, 3, 5, 8, 13, 31, 64, 100, 257] {
        let input = keyed_input(n, 5, n as u64 + 1);
        let mut expected = input.clone();
        expected.sort(); // stable
        for r in [1usize, 2, 3, 4, 7, 16, 64, 300] {
            for f in [2usize, 3, 4, 8] {
                for jobs in [1usize, 2, 3] {
                    let (_parent, scratch) = new_scratch(Arc::new(FileIo));
                    let buffer = [1, 7, 64][cases % 3];
                    let (output, stats) =
                        sort(&scratch, config::<Keyed>(r, f, jobs, buffer), &input).unwrap();
                    assert_eq!(
                        pairs(&output),
                        pairs(&expected),
                        "n={n} R={r} F={f} jobs={jobs}"
                    );
                    assert_eq!(stats.records, n as u64);
                    assert_eq!(stats.runs > 0, n > r, "spills only past one run");
                    let need = SortConfig::scratch_need::<Keyed>(n as u64).unwrap();
                    assert!(scratch.peak_bytes() <= need, "peak within preflight need");
                    assert_eq!(scratch.live_bytes(), 0);
                    assert_eq!(entries(scratch.path()), 0, "runs removed after the stream");
                    cases += 1;
                }
            }
        }
    }
    assert_eq!(cases, 11 * 8 * 4 * 3);
}

#[test]
fn output_bytes_are_identical_across_r_f_and_jobs() {
    let mut rng = Rng(7);
    let input: Vec<Wide> = (0..20_000).map(|i| wide(&mut rng, i, 3_000)).collect();
    let mut expected = input.clone();
    expected.sort();
    let expected = encoded(&expected);
    let keyed = keyed_input(20_000, 40, 11);
    let mut keyed_expected = keyed.clone();
    keyed_expected.sort();
    for (r, f, jobs) in [
        (7, 2, 1),
        (1_000, 3, 2),
        (4_096, 64, 4),
        (4_095, 2, 3),
        (20_000, 2, 4),
        (50_000, 8, 1),
    ] {
        let (_parent, scratch) = new_scratch(Arc::new(FileIo));
        let (output, stats) = sort(&scratch, config::<Wide>(r, f, jobs, 4096), &input).unwrap();
        assert_eq!(encoded(&output), expected, "R={r} F={f} jobs={jobs}");
        if r == 7 {
            assert!(stats.merge_levels >= 10, "deep merge cascade exercised");
        }
        let (output, _) = sort(&scratch, config::<Keyed>(r, f, jobs, 4096), &keyed).unwrap();
        assert_eq!(encoded(&output), encoded(&keyed_expected));
    }
}

#[test]
fn invalid_configuration_is_typed() {
    let (_parent, scratch) = new_scratch(Arc::new(FileIo));
    let base = config::<Wide>(4, 2, 1, 8);
    for bad in [
        SortConfig { fan_in: 1, ..base },
        SortConfig { jobs: 0, ..base },
        SortConfig {
            buffer_bytes: 0,
            ..base
        },
        SortConfig {
            run_bytes: size_of::<Wide>() - 1,
            ..base
        },
    ] {
        assert!(matches!(
            ExternalSorter::<Wide>::new(&scratch, bad),
            Err(SpillError::InvalidConfig(_))
        ));
    }
    assert!(matches!(
        SortConfig::scratch_need::<Wide>(u64::MAX / 8),
        Err(SpillError::Overflow)
    ));
    let default = SortConfig::default();
    let bound = default.memory_bound::<Wide>();
    let (r, f, b) = (default.run_bytes as u64, default.fan_in as u64, 1u64 << 20);
    assert!(bound >= r * 3 / 2 + f * b, "covers 1.5·R + F·b");
    assert!(
        bound <= (0.31 * (1u64 << 30) as f64) as u64,
        "design M_sort ≈ 0.31 GiB"
    );
    let huge = SortConfig {
        run_bytes: usize::MAX,
        fan_in: usize::MAX,
        buffer_bytes: usize::MAX,
        jobs: usize::MAX,
    };
    assert_eq!(
        huge.memory_bound::<Wide>(),
        u64::MAX,
        "saturates, never wraps"
    );
}

#[derive(Clone, Debug, Default)]
struct Faults {
    max_write: Option<usize>,
    interrupt: bool,
    write_error_at: Option<(u64, io::ErrorKind)>,
    write_zero_at: Option<u64>,
    flush_error: bool,
    create_error_at: Option<usize>,
    open_error: bool,
    truncate_reads: Option<u64>,
    extra_read_byte: bool,
    read_error_at: Option<u64>,
}

#[derive(Default)]
struct Counters {
    written: AtomicU64,
    read: AtomicU64,
    creates: AtomicUsize,
}

struct FaultIo {
    faults: Faults,
    counters: Arc<Counters>,
}

impl FaultIo {
    fn new(faults: Faults) -> Arc<Self> {
        Arc::new(Self {
            faults,
            counters: Arc::default(),
        })
    }
}

impl SpillIo for FaultIo {
    fn create(&self, path: &Path) -> io::Result<Box<dyn Write + Send>> {
        let nth = self.counters.creates.fetch_add(1, Ordering::Relaxed);
        let file = FileIo.create(path)?;
        if self.faults.create_error_at == Some(nth) {
            return Err(io::Error::other("injected create failure"));
        }
        Ok(Box::new(FaultWriter {
            file,
            faults: self.faults.clone(),
            counters: self.counters.clone(),
            tick: false,
        }))
    }
    fn open(&self, path: &Path) -> io::Result<Box<dyn Read + Send>> {
        if self.faults.open_error {
            return Err(io::Error::other("injected open failure"));
        }
        Ok(Box::new(FaultReader {
            file: File::open(path)?,
            faults: self.faults.clone(),
            counters: self.counters.clone(),
            in_file: 0,
            extra_given: false,
            tick: false,
        }))
    }
}

struct FaultWriter {
    file: Box<dyn Write + Send>,
    faults: Faults,
    counters: Arc<Counters>,
    tick: bool,
}

impl Write for FaultWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.faults.interrupt {
            self.tick = !self.tick;
            if self.tick {
                return Err(io::ErrorKind::Interrupted.into());
            }
        }
        let written = self.counters.written.load(Ordering::Relaxed);
        if self.faults.write_zero_at.is_some_and(|at| written >= at) {
            return Ok(0);
        }
        if let Some((at, kind)) = self.faults.write_error_at {
            if written >= at {
                return Err(io::Error::new(kind, "injected write failure"));
            }
        }
        let limit = self.faults.max_write.unwrap_or(buf.len()).min(buf.len());
        let n = self.file.write(&buf[..limit])?;
        self.counters.written.fetch_add(n as u64, Ordering::Relaxed);
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        if self.faults.flush_error {
            return Err(io::Error::other("injected flush failure"));
        }
        self.file.flush()
    }
}

struct FaultReader {
    file: File,
    faults: Faults,
    counters: Arc<Counters>,
    in_file: u64,
    extra_given: bool,
    tick: bool,
}

impl Read for FaultReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        if self.faults.interrupt {
            self.tick = !self.tick;
            if self.tick {
                return Err(io::ErrorKind::Interrupted.into());
            }
        }
        if self
            .faults
            .read_error_at
            .is_some_and(|at| self.counters.read.load(Ordering::Relaxed) >= at)
        {
            return Err(io::Error::other("injected read failure"));
        }
        let mut limit = buf.len();
        if let Some(at) = self.faults.truncate_reads {
            limit = limit.min(at.saturating_sub(self.in_file) as usize);
        }
        let n = if limit == 0 {
            0
        } else {
            self.file.read(&mut buf[..limit])?
        };
        if n == 0 && self.faults.extra_read_byte && !self.extra_given {
            self.extra_given = true;
            buf[0] = 0xab;
            return Ok(1);
        }
        self.in_file += n as u64;
        self.counters.read.fetch_add(n as u64, Ordering::Relaxed);
        Ok(n)
    }
}

fn spilling_input() -> Vec<Wide> {
    let mut rng = Rng(3);
    (0..2_000).map(|i| wide(&mut rng, i, 500)).collect()
}

/// R = 100 records, F = 3: 20 runs (32,000 bytes). A full pass rewrites them into 7 runs
/// (6 of 4,800 bytes, then 3,200); the next-to-last pass merges only the first 6 into 2,
/// writing 28,800 bytes; the final merge streams 3 runs.
fn spilling_config() -> SortConfig {
    config::<Wide>(100, 3, 2, 64)
}

#[test]
fn next_to_last_pass_rewrites_only_leading_groups() {
    assert_eq!(ExternalSorter::<Wide>::pass_groups(65, 64), vec![2]);
    assert_eq!(ExternalSorter::<Wide>::pass_groups(20, 3), vec![3; 7]);
    assert_eq!(ExternalSorter::<Wide>::pass_groups(7, 3), vec![3, 3]);
    assert_eq!(ExternalSorter::<Wide>::pass_groups(8, 3), vec![3, 3, 2]);
    assert_eq!(ExternalSorter::<Wide>::pass_groups(4096, 64), vec![64; 64]);
    assert_eq!(ExternalSorter::<Wide>::pass_groups(4097, 64), vec![64; 65]);

    // 65 runs of 10 records at F = 64: one pass merging runs 0 and 1 only.
    let mut rng = Rng(9);
    let input: Vec<Wide> = (0..650).map(|i| wide(&mut rng, i, 40)).collect();
    let mut expected = input.clone();
    expected.sort();
    let io = FaultIo::new(Faults::default());
    let (_parent, scratch) = new_scratch(io.clone());
    let (output, stats) = sort(&scratch, config::<Wide>(10, 64, 1, 64), &input).unwrap();
    assert_eq!(output, expected);
    assert_eq!((stats.runs, stats.merge_levels), (65, 1));
    let generated = 650 * 16;
    let written = io.counters.written.load(Ordering::Relaxed);
    assert_eq!(written, generated + 2 * 10 * 16, "2 of 65 runs rewritten");
    assert_eq!(stats.spilled_bytes, written);

    // Stability and identity around the boundaries, with heavy ties.
    let keyed = keyed_input(650, 3, 13);
    let mut keyed_expected = keyed.clone();
    keyed_expected.sort();
    for (r, f, jobs, runs) in [
        (10, 64, 1, 65),
        (10, 64, 3, 65),
        (11, 64, 2, 60),
        (10, 8, 2, 65),
        (9, 8, 1, 73),
        (5, 3, 4, 130),
        (650, 2, 3, 1),
    ] {
        let (_parent, scratch) = new_scratch(Arc::new(FileIo));
        let (output, stats) = sort(&scratch, config::<Keyed>(r, f, jobs, 32), &keyed).unwrap();
        assert_eq!(
            pairs(&output),
            pairs(&keyed_expected),
            "R={r} F={f} jobs={jobs}"
        );
        assert_eq!(stats.runs.max(1), runs, "R={r}");
    }
}

#[test]
fn short_writes_and_interrupts_still_replay_exactly() {
    let input = spilling_input();
    let mut expected = input.clone();
    expected.sort();
    for faults in [
        Faults {
            max_write: Some(3),
            ..Faults::default()
        },
        Faults {
            interrupt: true,
            max_write: Some(5),
            ..Faults::default()
        },
    ] {
        let (_parent, scratch) = new_scratch(FaultIo::new(faults));
        let (output, stats) = sort(&scratch, spilling_config(), &input).unwrap();
        assert_eq!(output, expected);
        assert_eq!(stats.runs, 20);
        assert_eq!(stats.merge_levels, 2);
        assert_eq!(entries(scratch.path()), 0);
    }
}

fn io_kind(error: &SpillError) -> io::ErrorKind {
    match error {
        SpillError::Io { source, .. } => source.kind(),
        other => panic!("expected typed I/O error, got {other:?}"),
    }
}

/// Feeds the input, finishes and drains; returns the first error.
fn run_to_error(scratch: &Scratch, input: &[Wide]) -> SpillError {
    let mut sorter = ExternalSorter::new(scratch, spilling_config()).unwrap();
    let result = (|| {
        sorter.extend(input.iter().copied())?;
        Ok::<_, SpillError>(())
    })();
    if let Err(error) = result {
        assert_eq!(entries(scratch.path()), 0, "failed push removes every run");
        assert!(matches!(sorter.push(input[0]), Err(SpillError::Poisoned)));
        assert!(matches!(sorter.finish(), Err(SpillError::Poisoned)));
        return error;
    }
    let mut stream = match sorter.finish() {
        Ok(stream) => stream,
        Err(error) => {
            assert_eq!(
                entries(scratch.path()),
                0,
                "failed finish removes every run"
            );
            return error;
        }
    };
    let error = stream
        .by_ref()
        .find_map(Result::err)
        .expect("injected fault surfaces");
    assert!(stream.next().is_none(), "stream is fused after an error");
    assert_eq!(entries(scratch.path()), 0, "failed stream removes its runs");
    error
}

#[test]
fn write_faults_are_typed_and_clean_up() {
    let input = spilling_input();
    let cases = [
        // During run generation.
        (
            Faults {
                write_error_at: Some((500, io::ErrorKind::StorageFull)),
                ..Faults::default()
            },
            io::ErrorKind::StorageFull,
        ),
        // During an intermediate merge (run generation writes 32,000 bytes).
        (
            Faults {
                write_error_at: Some((40_000, io::ErrorKind::Other)),
                ..Faults::default()
            },
            io::ErrorKind::Other,
        ),
        (
            Faults {
                write_zero_at: Some(1_000),
                ..Faults::default()
            },
            io::ErrorKind::WriteZero,
        ),
        (
            Faults {
                write_zero_at: Some(33_000),
                max_write: Some(7),
                ..Faults::default()
            },
            io::ErrorKind::WriteZero,
        ),
        (
            Faults {
                flush_error: true,
                ..Faults::default()
            },
            io::ErrorKind::Other,
        ),
        (
            Faults {
                create_error_at: Some(0),
                ..Faults::default()
            },
            io::ErrorKind::Other,
        ),
        (
            Faults {
                create_error_at: Some(21),
                ..Faults::default()
            },
            io::ErrorKind::Other,
        ),
        // Second merge pass: creates 0-19 are runs and 20-26 the first pass; writes past
        // 32,000 + 32,000 bytes belong to the second pass.
        (
            Faults {
                create_error_at: Some(27),
                ..Faults::default()
            },
            io::ErrorKind::Other,
        ),
        (
            Faults {
                write_error_at: Some((66_000, io::ErrorKind::StorageFull)),
                ..Faults::default()
            },
            io::ErrorKind::StorageFull,
        ),
    ];
    for (faults, kind) in cases {
        let (_parent, scratch) = new_scratch(FaultIo::new(faults.clone()));
        let error = run_to_error(&scratch, &input);
        assert_eq!(io_kind(&error), kind, "{faults:?}");
        assert_eq!(scratch.live_bytes(), 0);
        let job = JobError::from(error);
        assert_eq!(job.kind(), JobErrorKind::Io);
        assert!(job.path().is_some_and(|p| p.starts_with(scratch.path())));
    }
}

#[test]
fn read_faults_and_corrupt_runs_are_typed_and_clean_up() {
    let input = spilling_input();
    for faults in [
        Faults {
            truncate_reads: Some(16 * 50 + 3),
            ..Faults::default()
        },
        Faults {
            extra_read_byte: true,
            ..Faults::default()
        },
    ] {
        let (_parent, scratch) = new_scratch(FaultIo::new(faults));
        let error = run_to_error(&scratch, &input);
        assert!(matches!(error, SpillError::CorruptRun { .. }), "{error:?}");
        let job = JobError::from(error);
        assert_eq!(job.kind(), JobErrorKind::Io);
        assert!(job.path().is_some_and(|p| p.starts_with(scratch.path())));
    }
    for (faults, at) in [
        (
            Faults {
                read_error_at: Some(100),
                ..Faults::default()
            },
            "first pass",
        ),
        (
            Faults {
                // The second pass reads bytes 32,000 to 60,800.
                read_error_at: Some(40_000),
                ..Faults::default()
            },
            "second pass",
        ),
        (
            Faults {
                // Intermediate passes read 32,000 + 28,800 bytes before the final stream.
                read_error_at: Some(70_000),
                ..Faults::default()
            },
            "final stream",
        ),
        (
            Faults {
                open_error: true,
                ..Faults::default()
            },
            "open",
        ),
    ] {
        let (_parent, scratch) = new_scratch(FaultIo::new(faults));
        let error = run_to_error(&scratch, &input);
        assert_eq!(io_kind(&error), io::ErrorKind::Other, "{at}");
    }
}

#[test]
fn drops_remove_scratch_files_and_directory() {
    let input = spilling_input();
    let (parent, scratch) = new_scratch(Arc::new(FileIo));
    let mut sorter = ExternalSorter::new(&scratch, spilling_config()).unwrap();
    sorter.extend(input.iter().copied()).unwrap();
    assert!(entries(scratch.path()) > 0);
    drop(sorter);
    assert_eq!(entries(scratch.path()), 0, "unfinished sorter removes runs");

    let mut sorter = ExternalSorter::new(&scratch, spilling_config()).unwrap();
    sorter.extend(input.iter().copied()).unwrap();
    let mut stream = sorter.finish().unwrap();
    stream.next().unwrap().unwrap();
    assert!(entries(scratch.path()) > 0);
    drop(stream);
    assert_eq!(
        entries(scratch.path()),
        0,
        "partly read stream removes runs"
    );
    assert_eq!(scratch.live_bytes(), 0);
    assert!(scratch.peak_bytes() > 0);

    let dir = scratch.path().to_owned();
    std::fs::write(dir.join("stray"), b"x").unwrap();
    scratch.close().unwrap();
    assert!(!dir.exists());

    let (_parent, scratch) = new_scratch(Arc::new(FileIo));
    let dir = scratch.path().to_owned();
    drop(scratch);
    assert!(!dir.exists(), "drop removes the scratch directory");
    assert_eq!(entries(parent.path()), 0);
}

#[test]
fn failed_removal_stays_accounted() {
    let (_parent, scratch) = new_scratch(Arc::new(FileIo));
    let mut file = scratch.new_file("blocked");
    // remove_file refuses a directory on every platform.
    std::fs::create_dir(file.path()).unwrap();
    file.completed(100);
    let blocked = file.path().to_owned();
    drop(file);
    assert_eq!(scratch.live_bytes(), 100, "still on disk, still counted");
    std::fs::remove_dir(blocked).unwrap();

    let mut file = scratch.new_file("gone");
    file.completed(50);
    drop(file); // never created: NotFound counts as removed
    assert_eq!(scratch.live_bytes(), 100);
}

fn fixed(available: u64) -> impl FnOnce(&Path) -> Result<Volume, SpillError> {
    move |_| {
        Ok(Volume {
            available_bytes: available,
            memory_backed: false,
            allocation_unit_bytes: 4096,
        })
    }
}

#[test]
fn preflight_refuses_one_byte_short_before_creating_scratch() {
    let parent = tempfile::tempdir().unwrap();
    let need = SortConfig::scratch_need::<Wide>(1_000_000).unwrap();
    let error = Scratch::create_with(Some(parent.path()), need, Arc::new(FileIo), fixed(need - 1))
        .err()
        .unwrap();
    let cause = match &error {
        SpillError::InsufficientSpace(cause) => cause.clone(),
        other => panic!("expected InsufficientSpace, got {other:?}"),
    };
    assert_eq!(
        cause,
        InsufficientSpace {
            path: parent.path().to_owned(),
            needed: need,
            available: need - 1,
            filesystem: SpaceRole::Scratch,
            stage: PreflightStage::Geometry,
        }
    );
    assert_eq!(entries(parent.path()), 0, "nothing created on refusal");
    let job = JobError::from(error);
    assert_eq!(job.kind(), JobErrorKind::Io);
    assert_eq!(job.path(), Some(parent.path()));
    assert!(job.message().contains("(geometry preflight)"), "{job}");
    let source = std::error::Error::source(&job).unwrap();
    assert_eq!(source.downcast_ref::<InsufficientSpace>(), Some(&cause));

    let scratch =
        Scratch::create_with(Some(parent.path()), need, Arc::new(FileIo), fixed(need)).unwrap();
    assert_eq!(scratch.available_at_start(), need);
    assert!(scratch.path().starts_with(parent.path()));

    let output = preflight(
        parent.path(),
        u64::MAX,
        SpaceRole::Output,
        PreflightStage::Texture,
    )
    .unwrap_err();
    assert!(matches!(
        output,
        SpillError::InsufficientSpace(InsufficientSpace {
            filesystem: SpaceRole::Output,
            stage: PreflightStage::Texture,
            ..
        })
    ));
}

#[test]
fn real_volume_preflight_and_missing_directory() {
    let parent = tempfile::tempdir().unwrap();
    let volume = inspect_volume(parent.path()).unwrap();
    assert!(volume.available_bytes > 0);
    let scratch = Scratch::create(Some(parent.path()), 1).unwrap();
    assert_eq!(scratch.memory_backed(), volume.memory_backed);
    drop(scratch);
    assert!(matches!(
        Scratch::create(Some(parent.path()), u64::MAX),
        Err(SpillError::InsufficientSpace(_))
    ));
    assert_eq!(entries(parent.path()), 0);
    let missing = parent.path().join("missing");
    let error = Scratch::create(Some(&missing), 0).err().unwrap();
    assert_eq!(io_kind(&error), io::ErrorKind::NotFound);
    assert!(
        error
            .to_string()
            .starts_with("scratch directory does not exist"),
        "{error}"
    );
    assert!(
        !missing.exists(),
        "an explicit scratch directory is never created"
    );
    assert_eq!(Scratch::default_parent(), std::env::temp_dir());
}

#[cfg(target_os = "linux")]
#[test]
fn tmpfs_is_reported_as_memory_backed() {
    let shm = Path::new("/dev/shm");
    if shm.is_dir() {
        assert!(
            inspect_volume(shm).unwrap().memory_backed,
            "/dev/shm is tmpfs"
        );
        let scratch = Scratch::create(Some(shm), 0).unwrap();
        assert!(scratch.memory_backed());
    }
    assert!(!inspect_volume(Path::new("/proc")).unwrap().memory_backed);
}

#[cfg(target_os = "linux")]
const MEMORY_CHILD: &str = "RUSTY_TILES_SPILL_MEMORY_CHILD";

#[cfg(target_os = "linux")]
fn memory_config() -> SortConfig {
    SortConfig {
        run_bytes: 256 << 10,
        fan_in: 4,
        buffer_bytes: 16 << 10,
        jobs: 2,
    }
}

#[cfg(target_os = "linux")]
fn vm_hwm_kib() -> u64 {
    let status = std::fs::read_to_string("/proc/self/status").unwrap();
    status
        .lines()
        .find_map(|line| line.strip_prefix("VmHWM:"))
        .and_then(|value| value.trim().trim_end_matches("kB").trim().parse().ok())
        .expect("VmHWM in /proc/self/status")
}

/// Subprocess body for `spill_memory_is_flat_in_input_size`: a fresh process, so VmHWM
/// reflects only this sort. Records are generated and verified as streams.
#[cfg(target_os = "linux")]
#[test]
#[ignore = "subprocess body of spill_memory_is_flat_in_input_size"]
fn spill_memory_child() {
    let Ok(records) = std::env::var(MEMORY_CHILD) else {
        return;
    };
    let records: u64 = records.parse().unwrap();
    let parent = tempfile::tempdir().unwrap();
    let scratch = Scratch::create(Some(parent.path()), 0).unwrap();
    let base = vm_hwm_kib();
    let mut sorter = ExternalSorter::new(&scratch, memory_config()).unwrap();
    let mut rng = Rng(5);
    let mut checksum = 0u64;
    for i in 0..records {
        let record = wide(&mut rng, i, records / 4 + 1);
        checksum = checksum.wrapping_add(record.morton ^ (u64::from(record.b) << 7) ^ i);
        sorter.push(record).unwrap();
    }
    let stream = sorter.finish().unwrap();
    let stats = stream.stats();
    let (mut count, mut previous) = (0u64, None);
    for record in stream {
        let record = record.unwrap();
        assert!(previous <= Some(record));
        previous = Some(record);
        count += 1;
        checksum =
            checksum.wrapping_sub(record.morton ^ (u64::from(record.b) << 7) ^ u64::from(record.a));
    }
    assert_eq!((count, checksum), (records, 0));
    println!(
        "SPILL_MEMORY base_kib={base} peak_kib={} merge_levels={}",
        vm_hwm_kib(),
        stats.merge_levels
    );
}

#[cfg(target_os = "linux")]
#[test]
fn spill_memory_is_flat_in_input_size() {
    fn child(records: u64) -> (u64, u64, u64) {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "spill::tests::spill_memory_child",
                "--exact",
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(MEMORY_CHILD, records.to_string())
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(output.status.success(), "child failed: {stdout}");
        let line = stdout
            .lines()
            .find_map(|line| line.split_once("SPILL_MEMORY ").map(|(_, report)| report))
            .unwrap_or_else(|| panic!("child report missing: {stdout}"));
        let field = |name: &str| -> u64 {
            line.split_whitespace()
                .find_map(|kv| kv.strip_prefix(name)?.strip_prefix('='))
                .unwrap()
                .parse()
                .unwrap()
        };
        (field("base_kib"), field("peak_kib"), field("merge_levels"))
    }
    let run_records = (memory_config().run_bytes / size_of::<Wide>()) as u64;
    // Both sizes run intermediate merges, so both touch the same code pages and threads:
    // 17 runs (two levels at F = 4) against 136 runs (34 MiB of records, three levels).
    let small = child(17 * run_records);
    let large = child(136 * run_records);
    assert_eq!((small.2, large.2), (2, 3));
    assert!(
        large.1 <= small.1 + 1024,
        "VmHWM grew with input size: small {small:?}, large {large:?}"
    );
    // 1.5·R + (F + 1)·b is about 0.5 MiB here. The fixed allowance covers first-touch code
    // pages, sort threads and the test harness (about 8 MiB measured), and stays below a third
    // of the records' 34 MiB.
    assert!(
        large.1 - large.0 <= 11 * 1024,
        "large sort grew VmHWM by {} KiB",
        large.1 - large.0
    );
}

#[test]
fn online_compaction_bounds_inventory_and_keeps_equal_key_input_order() {
    let input = keyed_input(2048, 7, 91);
    let mut expected = input.clone();
    expected.sort();
    for fan_in in [2, 3] {
        let (_parent, scratch) = new_scratch(Arc::new(FileIo));
        let mut sorter = ExternalSorter::new(&scratch, config::<Keyed>(1, fan_in, 1, 31)).unwrap();
        let mut largest = 0;
        for record in &input {
            sorter.push(*record).unwrap();
            largest = largest.max(sorter.inventory_len());
            assert!(
                sorter.inventory_len() < 128 * fan_in,
                "descriptor bound independent of count"
            );
        }
        assert!(
            largest < input.len() / 2,
            "control exercises live compaction"
        );
        let output = sorter
            .finish()
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(pairs(&output), pairs(&expected));
        assert_eq!(scratch.live_bytes(), 0);
        assert!(
            scratch.peak_bytes() <= SortConfig::scratch_need::<Keyed>(input.len() as u64).unwrap()
        );
    }
}

#[test]
fn sequence_accounts_unfinished_bytes_and_replays_short_writes() {
    let (_parent, scratch) = new_scratch(FaultIo::new(Faults {
        max_write: Some(3),
        ..Default::default()
    }));
    let input = keyed_input(20, 4, 77);
    let mut writer = SequenceWriter::new(&scratch, "accounted-sequence", 7).unwrap();
    for (i, record) in input.iter().enumerate() {
        writer.push(record).unwrap();
        assert_eq!(
            scratch.live_bytes(),
            (i as u64 + 1) * Keyed::SIZE as u64,
            "unfinished output contributes to peak"
        );
    }
    let sequence = writer.finish().unwrap();
    assert_eq!(scratch.live_bytes(), 20 * Keyed::SIZE as u64);
    assert_eq!(
        pairs(
            &sequence
                .reader()
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        ),
        pairs(&input)
    );
    drop(sequence);
    assert_eq!(scratch.live_bytes(), 0);
}

#[test]
fn failed_partial_writes_contribute_to_peak_before_cleanup() {
    let faults = Faults {
        // The fault is checked before each write, so force a short first write to reach
        // the failure while write_all still has bytes of this record left to emit.
        max_write: Some(3),
        write_error_at: Some((3, std::io::ErrorKind::StorageFull)),
        ..Default::default()
    };
    let record = Wide {
        morton: 0,
        a: 1,
        b: 2,
    };
    let sequence_io = FaultIo::new(faults.clone());
    let (_parent, scratch) = new_scratch(sequence_io.clone());
    let mut writer = SequenceWriter::new(&scratch, "partial-sequence", 1).unwrap();
    assert!(writer.push(&record).is_err());
    assert_eq!(sequence_io.counters.written.load(Ordering::Relaxed), 3);
    assert_eq!(scratch.peak_bytes(), Wide::SIZE as u64);
    drop(writer);
    assert_eq!(scratch.live_bytes(), 0);

    let sorter_io = FaultIo::new(faults);
    let (_parent, scratch) = new_scratch(sorter_io.clone());
    let mut sorter = ExternalSorter::new(&scratch, config::<Wide>(1, 2, 1, 1)).unwrap();
    sorter.push(record).unwrap();
    assert!(sorter.push(record).is_err());
    assert_eq!(sorter_io.counters.written.load(Ordering::Relaxed), 3);
    assert_eq!(scratch.peak_bytes(), Wide::SIZE as u64);
    assert_eq!(scratch.live_bytes(), 0);
}

#[test]
fn allocation_unit_is_preserved_by_preflight_and_scratch() {
    let parent = tempfile::tempdir().unwrap();
    let scratch = Scratch::create_with(Some(parent.path()), 65536, Arc::new(FileIo), |_| {
        Ok(Volume {
            available_bytes: 65536,
            memory_backed: true,
            allocation_unit_bytes: 65536,
        })
    })
    .unwrap();
    assert_eq!(scratch.allocation_unit_bytes(), 65536);
    assert!(scratch.memory_backed());
    scratch.close().unwrap();
    assert_eq!(entries(parent.path()), 0);
}

#[test]
fn zero_allocation_unit_is_a_typed_fault_before_scratch_creation() {
    let parent = tempfile::tempdir().unwrap();
    let error = Scratch::create_with(Some(parent.path()), 0, Arc::new(FileIo), |_| {
        Ok(Volume {
            available_bytes: u64::MAX,
            memory_backed: false,
            allocation_unit_bytes: 0,
        })
    })
    .err()
    .expect("invalid filesystem geometry must be refused");
    assert_eq!(io_kind(&error), io::ErrorKind::InvalidData);
    assert_eq!(entries(parent.path()), 0);
    let job = JobError::from(error);
    assert_eq!(job.kind(), JobErrorKind::Io);
    assert_eq!(job.path(), Some(parent.path()));
}

#[test]
fn real_volume_reports_nonzero_allocation_unit() {
    let parent = tempfile::tempdir().unwrap();
    let volume = inspect_volume(parent.path()).unwrap();
    assert!(volume.allocation_unit_bytes > 0);
    let scratch = Scratch::create(Some(parent.path()), 0).unwrap();
    assert_eq!(
        scratch.allocation_unit_bytes(),
        volume.allocation_unit_bytes
    );
}

#[cfg(windows)]
#[test]
fn relative_windows_path_uses_the_actual_working_directory_volume() {
    let relative = inspect_volume(Path::new(".")).unwrap();
    let absolute = inspect_volume(&std::env::current_dir().unwrap()).unwrap();
    assert_eq!(
        relative.allocation_unit_bytes,
        absolute.allocation_unit_bytes
    );
}
