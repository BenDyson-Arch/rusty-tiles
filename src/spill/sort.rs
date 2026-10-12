//! External merge sort: sorted runs of at most `R` bytes, merged `F` at a time.
//!
//! Every merge breaks ties by source position (earlier input first) and runs are grouped
//! consecutively, so the output is the stable sort of the input for any `R`, `F` and `jobs`.
use super::{scratch::SpillFile, Record, Scratch, SpillError};
use std::{
    cmp::Ordering,
    collections::BinaryHeap,
    io::{self, BufReader, BufWriter, Read, Write},
    marker::PhantomData,
    path::Path,
    sync::Arc,
};

/// Spill sort resources. Resident memory is bounded by [`SortConfig::memory_bound`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SortConfig {
    /// `R`: in-memory run buffer, in bytes of resident records.
    pub run_bytes: usize,
    /// `F`: runs merged at once; also the final stream's fan-in.
    pub fan_in: usize,
    /// `b`: buffer per open run reader and per run writer.
    pub buffer_bytes: usize,
    /// Threads sorting one run buffer. Does not affect output.
    pub jobs: usize,
}

impl Default for SortConfig {
    /// `1.5·R + F·b` = 240 MiB + 64 MiB, within design §2.6 `M_sort` ≈ 0.31 GiB together with
    /// the margin in [`SortConfig::memory_bound`].
    fn default() -> Self {
        Self {
            run_bytes: 160 << 20,
            fan_in: 64,
            buffer_bytes: 1 << 20,
            jobs: 1,
        }
    }
}

impl SortConfig {
    fn run_records<T: Record>(&self) -> Result<usize, SpillError> {
        if T::SIZE == 0 {
            return Err(SpillError::InvalidConfig("record size must be nonzero"));
        }
        if self.fan_in < 2 {
            return Err(SpillError::InvalidConfig("fan-in must be at least 2"));
        }
        if self.jobs == 0 {
            return Err(SpillError::InvalidConfig("jobs must be at least 1"));
        }
        if self.buffer_bytes == 0 {
            return Err(SpillError::InvalidConfig("stream buffer must be nonzero"));
        }
        match self.run_bytes / size_of::<T>().max(1) {
            0 => Err(SpillError::InvalidConfig(
                "run buffer must hold at least one record",
            )),
            records => Ok(records),
        }
    }

    /// Upper bound on resident bytes: `1.5·R + (F + 1)·b` plus a margin.
    ///
    /// - `R`: the run buffer. It grows to at most `R / 2` before jumping to `R`, so a
    ///   reallocation never holds more than `1.5·R` either.
    /// - `R / 2`: the standard library's stable sort scratch, `ceil(n / 2)` elements per sorted
    ///   chunk of `n`, live only while a run is sorted.
    /// - `(F + 1)·b`: merge readers plus the run or merge writer.
    /// - **Margin:** the stable sort allocates at least `min(n, 8 MB / size_of::<T>())`
    ///   elements per sorting thread, which exceeds `n / 2` for small chunks, so the margin is
    ///   `jobs × 8 MB`, plus the merge heap (`F` entries).
    ///
    /// The run and merge phases do not overlap, so the sum is conservative.
    pub(crate) fn memory_bound<T: Record>(&self) -> u64 {
        const SORT_MIN_SCRATCH: u64 = 8_000_000;
        let (run, buffer, fan_in, jobs) = (
            self.run_bytes as u64,
            self.buffer_bytes as u64,
            self.fan_in as u64,
            self.jobs as u64,
        );
        let heap = fan_in * (size_of::<T>() as u64 + 16);
        run + run.div_ceil(2) + (fan_in + 1) * buffer + jobs * SORT_MIN_SCRATCH + heap
    }

    /// Scratch bytes one sort of `records` can hold at once: two generations of runs, because
    /// a merge's inputs are removed only after its output is complete.
    pub(crate) fn scratch_need<T: Record>(records: u64) -> Result<u64, SpillError> {
        records
            .checked_mul(T::SIZE as u64)
            .and_then(|bytes| bytes.checked_mul(2))
            .ok_or(SpillError::Overflow)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct SortStats {
    pub records: u64,
    /// Sorted runs written from the run buffer (0 when the input fit in memory).
    pub runs: u64,
    /// Intermediate merge levels before the final streamed merge.
    pub merge_levels: u64,
    /// Total bytes written to scratch.
    pub spilled_bytes: u64,
}

struct Run {
    file: SpillFile,
    records: u64,
}

trait Cursor<T> {
    fn next_record(&mut self) -> Result<Option<T>, SpillError>;
}

struct SliceCursor<'a, T>(std::slice::Iter<'a, T>);

impl<T: Copy> Cursor<T> for SliceCursor<'_, T> {
    fn next_record(&mut self) -> Result<Option<T>, SpillError> {
        Ok(self.0.next().copied())
    }
}

struct MemCursor<T> {
    data: Arc<Vec<T>>,
    next: usize,
    end: usize,
}

impl<T: Copy> Cursor<T> for MemCursor<T> {
    fn next_record(&mut self) -> Result<Option<T>, SpillError> {
        if self.next == self.end {
            return Ok(None);
        }
        self.next += 1;
        Ok(Some(self.data[self.next - 1]))
    }
}

struct RunReader<T> {
    // Declared before `run` so the handle closes before the file is removed.
    reader: BufReader<Box<dyn Read + Send>>,
    run: Run,
    remaining: u64,
    end_checked: bool,
    encoded: Vec<u8>,
    _record: PhantomData<T>,
}

impl<T: Record> RunReader<T> {
    fn open(scratch: &Scratch, run: Run, buffer_bytes: usize) -> Result<Self, SpillError> {
        let inner = scratch
            .io()
            .open(run.file.path())
            .map_err(|e| SpillError::io("open spill run", run.file.path(), e))?;
        Ok(Self {
            reader: BufReader::with_capacity(buffer_bytes, inner),
            remaining: run.records,
            run,
            end_checked: false,
            encoded: vec![0; T::SIZE],
            _record: PhantomData,
        })
    }

    fn corrupt(&self, detail: String) -> SpillError {
        SpillError::CorruptRun {
            path: self.run.file.path().to_owned(),
            detail,
        }
    }
}

impl<T: Record> Cursor<T> for RunReader<T> {
    fn next_record(&mut self) -> Result<Option<T>, SpillError> {
        if self.remaining == 0 {
            if !self.end_checked {
                self.end_checked = true;
                let mut probe = [0u8; 1];
                loop {
                    match self.reader.read(&mut probe) {
                        Ok(0) => break,
                        Ok(_) => {
                            let records = self.run.records;
                            return Err(self
                                .corrupt(format!("bytes follow the expected {records} records")));
                        }
                        Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                        Err(e) => {
                            return Err(SpillError::io("read spill run", self.run.file.path(), e))
                        }
                    }
                }
            }
            return Ok(None);
        }
        if let Err(e) = self.reader.read_exact(&mut self.encoded) {
            if e.kind() == io::ErrorKind::UnexpectedEof {
                let read = self.run.records - self.remaining;
                let records = self.run.records;
                return Err(self.corrupt(format!("ended after {read} of {records} records")));
            }
            return Err(SpillError::io("read spill run", self.run.file.path(), e));
        }
        self.remaining -= 1;
        Ok(Some(T::decode(&self.encoded)))
    }
}

/// Min-heap entry: smallest record first, then the earliest source.
struct Head<T> {
    record: T,
    source: usize,
}

impl<T: Ord> Ord for Head<T> {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .record
            .cmp(&self.record)
            .then_with(|| other.source.cmp(&self.source))
    }
}
impl<T: Ord> PartialOrd for Head<T> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl<T: Ord> PartialEq for Head<T> {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}
impl<T: Ord> Eq for Head<T> {}

/// Stable k-way merge of sources given in input order.
struct Merger<T, C> {
    sources: Vec<C>,
    heap: BinaryHeap<Head<T>>,
}

impl<T: Ord, C: Cursor<T>> Merger<T, C> {
    fn new(sources: Vec<C>) -> Result<Self, SpillError> {
        let mut merger = Self {
            heap: BinaryHeap::with_capacity(sources.len()),
            sources,
        };
        for source in 0..merger.sources.len() {
            merger.refill(source)?;
        }
        Ok(merger)
    }

    fn refill(&mut self, source: usize) -> Result<(), SpillError> {
        if let Some(record) = self.sources[source].next_record()? {
            self.heap.push(Head { record, source });
        }
        Ok(())
    }
}

impl<T: Ord, C: Cursor<T>> Cursor<T> for Merger<T, C> {
    fn next_record(&mut self) -> Result<Option<T>, SpillError> {
        let Some(head) = self.heap.pop() else {
            return Ok(None);
        };
        self.refill(head.source)?;
        Ok(Some(head.record))
    }
}

fn write_run<T: Record>(
    scratch: &Scratch,
    buffer_bytes: usize,
    records: &mut impl Cursor<T>,
) -> Result<Run, SpillError> {
    let mut file = scratch.new_file("run");
    // Declared after `file` so the handle closes before an error removes the file.
    let mut writer = BufWriter::with_capacity(
        buffer_bytes,
        scratch
            .io()
            .create(file.path())
            .map_err(|e| SpillError::io("create spill run", file.path(), e))?,
    );
    let mut encoded = vec![0; T::SIZE];
    let mut count = 0u64;
    while let Some(record) = records.next_record()? {
        record.encode(&mut encoded);
        writer
            .write_all(&encoded)
            .map_err(|e| SpillError::io("write spill run", file.path(), e))?;
        count += 1;
    }
    writer
        .flush()
        .map_err(|e| SpillError::io("write spill run", file.path(), e))?;
    drop(writer);
    let bytes = count
        .checked_mul(T::SIZE as u64)
        .ok_or(SpillError::Overflow)?;
    file.completed(bytes);
    Ok(Run {
        file,
        records: count,
    })
}

/// Stable-sorts `buffer` as at most `jobs` consecutive chunks; returns the chunk length.
fn sort_chunks<T: Record>(buffer: &mut [T], jobs: usize, at: &Path) -> Result<usize, SpillError> {
    let jobs = jobs.clamp(1, buffer.len().max(1));
    let chunk = buffer.len().div_ceil(jobs).max(1);
    if jobs == 1 {
        buffer.sort();
        return Ok(chunk);
    }
    std::thread::scope(|scope| {
        let mut workers = Vec::with_capacity(jobs);
        for part in buffer.chunks_mut(chunk) {
            let worker = std::thread::Builder::new()
                .name("rusty-tiles-spill-sort".into())
                .spawn_scoped(scope, move || part.sort())
                .map_err(|e| SpillError::io("start spill sort worker", at, e))?;
            workers.push(worker);
        }
        for worker in workers {
            if let Err(panic) = worker.join() {
                std::panic::resume_unwind(panic);
            }
        }
        Ok(chunk)
    })
}

/// Accepts records in input order and yields them as their stable sort.
pub(crate) struct ExternalSorter<'s, T: Record> {
    scratch: &'s Scratch,
    config: SortConfig,
    run_records: usize,
    buffer: Vec<T>,
    runs: Vec<Run>,
    stats: SortStats,
    poisoned: bool,
}

impl<'s, T: Record> ExternalSorter<'s, T> {
    pub(crate) fn new(scratch: &'s Scratch, config: SortConfig) -> Result<Self, SpillError> {
        let run_records = config.run_records::<T>()?;
        Ok(Self {
            scratch,
            config,
            run_records,
            buffer: Vec::new(),
            runs: Vec::new(),
            stats: SortStats::default(),
            poisoned: false,
        })
    }

    pub(crate) fn push(&mut self, record: T) -> Result<(), SpillError> {
        if self.poisoned {
            return Err(SpillError::Poisoned);
        }
        if self.buffer.len() == self.run_records {
            if let Err(error) = self.flush_run() {
                // Drop every run file now; later calls report the poisoned state.
                self.poisoned = true;
                self.runs = Vec::new();
                self.buffer = Vec::new();
                return Err(error);
            }
        }
        let capacity = self.buffer.capacity();
        if self.buffer.len() == capacity {
            // Double while the result stays within R / 2, then jump straight to R, so a
            // reallocation holds at most 1.5·R.
            let target = if capacity.saturating_mul(4) <= self.run_records {
                capacity.saturating_mul(2).max(64).min(self.run_records)
            } else {
                self.run_records
            };
            self.buffer.reserve_exact(target - capacity);
        }
        self.buffer.push(record);
        self.stats.records += 1;
        Ok(())
    }

    pub(crate) fn extend(
        &mut self,
        records: impl IntoIterator<Item = T>,
    ) -> Result<(), SpillError> {
        records.into_iter().try_for_each(|record| self.push(record))
    }

    pub(crate) fn stats(&self) -> SortStats {
        self.stats
    }

    fn flush_run(&mut self) -> Result<(), SpillError> {
        let chunk = sort_chunks(&mut self.buffer, self.config.jobs, self.scratch.path())?;
        let mut parts: Vec<_> = self
            .buffer
            .chunks(chunk)
            .map(|part| SliceCursor(part.iter()))
            .collect();
        let run = if parts.len() == 1 {
            write_run(self.scratch, self.config.buffer_bytes, &mut parts[0])?
        } else {
            let mut merger = Merger::new(parts)?;
            write_run(self.scratch, self.config.buffer_bytes, &mut merger)?
        };
        self.stats.runs += 1;
        self.stats.spilled_bytes += run.file_bytes();
        self.buffer.clear();
        self.runs.push(run);
        Ok(())
    }

    fn merge_level(&mut self) -> Result<(), SpillError> {
        let fan_in = self.config.fan_in;
        let mut pending = std::mem::take(&mut self.runs).into_iter();
        let mut merged = Vec::with_capacity(pending.len().div_ceil(fan_in));
        loop {
            let mut group: Vec<Run> = pending.by_ref().take(fan_in).collect();
            match group.len() {
                0 => break,
                1 => merged.append(&mut group),
                _ => {
                    let readers = group
                        .into_iter()
                        .map(|run| {
                            RunReader::<T>::open(self.scratch, run, self.config.buffer_bytes)
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    let mut merger = Merger::new(readers)?;
                    let run = write_run(self.scratch, self.config.buffer_bytes, &mut merger)?;
                    self.stats.spilled_bytes += run.file_bytes();
                    // Inputs are removed only now, after the output is complete.
                    drop(merger);
                    merged.push(run);
                }
            }
        }
        self.runs = merged;
        self.stats.merge_levels += 1;
        Ok(())
    }

    /// Ends input. Inputs that never filled a run stay in memory; otherwise runs are merged
    /// down to at most `F` and streamed. On error every scratch file is removed.
    pub(crate) fn finish(mut self) -> Result<SortedStream<'s, T>, SpillError> {
        if self.poisoned {
            return Err(SpillError::Poisoned);
        }
        let remaining = self.stats.records;
        if self.runs.is_empty() {
            let mut buffer = std::mem::take(&mut self.buffer);
            let chunk = sort_chunks(&mut buffer, self.config.jobs, self.scratch.path())?;
            let len = buffer.len();
            let data = Arc::new(buffer);
            let parts = (0..len)
                .step_by(chunk)
                .map(|start| MemCursor {
                    data: data.clone(),
                    next: start,
                    end: (start + chunk).min(len),
                })
                .collect();
            return Ok(SortedStream {
                source: Some(Source::Memory(Merger::new(parts)?)),
                remaining,
                stats: self.stats,
                _scratch: PhantomData,
            });
        }
        if !self.buffer.is_empty() {
            self.flush_run()?;
        }
        self.buffer = Vec::new();
        while self.runs.len() > self.config.fan_in {
            self.merge_level()?;
        }
        let readers = std::mem::take(&mut self.runs)
            .into_iter()
            .map(|run| RunReader::open(self.scratch, run, self.config.buffer_bytes))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(SortedStream {
            source: Some(Source::Disk(Merger::new(readers)?)),
            remaining,
            stats: self.stats,
            _scratch: PhantomData,
        })
    }
}

impl Run {
    fn file_bytes(&self) -> u64 {
        self.file.bytes()
    }
}

enum Source<T> {
    Memory(Merger<T, MemCursor<T>>),
    Disk(Merger<T, RunReader<T>>),
}

/// The sorted output. Fused after the first error; remaining scratch files are removed when it
/// ends, fails or is dropped.
pub(crate) struct SortedStream<'s, T: Record> {
    source: Option<Source<T>>,
    remaining: u64,
    stats: SortStats,
    _scratch: PhantomData<&'s Scratch>,
}

impl<T: Record> SortedStream<'_, T> {
    pub(crate) fn remaining(&self) -> u64 {
        self.remaining
    }

    pub(crate) fn stats(&self) -> SortStats {
        self.stats
    }
}

impl<T: Record> Iterator for SortedStream<'_, T> {
    type Item = Result<T, SpillError>;

    fn next(&mut self) -> Option<Self::Item> {
        let result = match self.source.as_mut()? {
            Source::Memory(merger) => merger.next_record(),
            Source::Disk(merger) => merger.next_record(),
        };
        match result {
            Ok(Some(record)) => {
                self.remaining = self.remaining.saturating_sub(1);
                Some(Ok(record))
            }
            Ok(None) => {
                self.source = None;
                None
            }
            Err(error) => {
                self.source = None;
                Some(Err(error))
            }
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match usize::try_from(self.remaining) {
            Ok(n) if self.source.is_some() => (n, Some(n)),
            Ok(_) => (0, Some(0)),
            Err(_) => (usize::MAX, None),
        }
    }
}
