//! External merge sort: sorted runs of at most `R` bytes, merged `F` at a time.
//!
//! Every merge breaks ties by source position (earlier input first) and runs are grouped
//! consecutively, so the output is the stable sort of the input for any `R`, `F` and `jobs`.
//!
//! A run is stored as consecutive chunk files of at most [`CHUNK_BYTES`]. Readers remove each
//! chunk as soon as it is consumed, so a streamed merge and its consumer's output share disk
//! space instead of adding up, and an intermediate merge frees its inputs while it writes.
//! Chunking changes no record order or content.
use super::{scratch::SpillFile, Record, Scratch, SpillError};
use std::{
    cmp::Ordering,
    collections::{BinaryHeap, VecDeque},
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
    /// Run payload/sort scratch and merge payload phases do not overlap, so their sum is
    /// conservative. The owner must additionally account for the bounded run inventory
    /// using `ExternalSorter::inventory_memory_bound` and its actual scratch path length.
    /// Saturates at `u64::MAX` rather than wrapping.
    pub(crate) fn memory_bound<T: Record>(&self) -> u64 {
        const SORT_MIN_SCRATCH: u64 = 8_000_000;
        let (run, buffer, fan_in, jobs) = (
            self.run_bytes as u64,
            self.buffer_bytes as u64,
            self.fan_in as u64,
            self.jobs as u64,
        );
        let heap = fan_in.saturating_mul(size_of::<T>() as u64 + 16);
        run.saturating_add(run.div_ceil(2))
            .saturating_add(fan_in.saturating_add(1).saturating_mul(buffer))
            .saturating_add(jobs.saturating_mul(SORT_MIN_SCRATCH))
            .saturating_add(heap)
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
    /// Highest online compaction generation, plus final intermediate merge passes.
    /// The final streamed merge is not counted.
    pub merge_levels: u64,
    /// Total bytes written to scratch.
    pub spilled_bytes: u64,
}

/// Largest run chunk file. Smaller chunks free disk sooner and raise the file count; with the
/// default `F = 64` an intermediate merge holds at most `F` partly read chunks (512 MiB).
pub(crate) const CHUNK_BYTES: usize = 8 << 20;

/// Records per chunk file for `T` at the production [`CHUNK_BYTES`] (the scratch model).
pub(crate) fn chunk_records<T: Record>() -> u64 {
    chunk_records_of::<T>(CHUNK_BYTES)
}

fn chunk_records_of<T: Record>(chunk_bytes: usize) -> u64 {
    (chunk_bytes / T::SIZE).max(1) as u64
}

struct Chunk {
    file: SpillFile,
    records: u64,
}

struct Run {
    /// Consecutive chunks in record order; a reader removes each once consumed.
    chunks: VecDeque<Chunk>,
    records: u64,
    /// Online compaction generation; runs stay in original input order.
    generation: u8,
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
    // Declared before `chunks` so the handle closes before its file is removed.
    reader: Option<BufReader<Box<dyn Read + Send>>>,
    /// The front chunk is the one being read.
    chunks: VecDeque<Chunk>,
    scratch_io: Arc<dyn super::SpillIo>,
    buffer_bytes: usize,
    /// Records left in the front chunk.
    in_chunk: u64,
    remaining: u64,
    records: u64,
    encoded: Vec<u8>,
    _record: PhantomData<T>,
}

impl<T: Record> RunReader<T> {
    fn open(scratch: &Scratch, run: Run, buffer_bytes: usize) -> Result<Self, SpillError> {
        let mut reader = Self {
            reader: None,
            chunks: run.chunks,
            scratch_io: scratch.io_handle(),
            buffer_bytes,
            in_chunk: 0,
            remaining: run.records,
            records: run.records,
            encoded: vec![0; T::SIZE],
            _record: PhantomData,
        };
        reader.open_front()?;
        Ok(reader)
    }

    fn open_front(&mut self) -> Result<(), SpillError> {
        let Some(chunk) = self.chunks.front() else {
            return Ok(());
        };
        let inner = self
            .scratch_io
            .open(chunk.file.path())
            .map_err(|e| SpillError::io("open spill run", chunk.file.path(), e))?;
        self.reader = Some(BufReader::with_capacity(self.buffer_bytes, inner));
        self.in_chunk = chunk.records;
        Ok(())
    }

    fn path(&self) -> &Path {
        self.chunks
            .front()
            .map_or(Path::new(""), |chunk| chunk.file.path())
    }

    fn corrupt(&self, detail: String) -> SpillError {
        SpillError::CorruptRun {
            path: self.path().to_owned(),
            detail,
        }
    }

    /// Verifies the front chunk ends exactly after its records, then closes and removes it.
    fn finish_front(&mut self) -> Result<(), SpillError> {
        if let Some(reader) = self.reader.as_mut() {
            let mut probe = [0u8; 1];
            loop {
                match reader.read(&mut probe) {
                    Ok(0) => break,
                    Ok(_) => {
                        let (chunk, records) = (
                            self.chunks.front().map_or(0, |chunk| chunk.records),
                            self.records,
                        );
                        return Err(self.corrupt(format!(
                            "bytes follow the expected {chunk} records of a chunk (run of {records} records)"
                        )));
                    }
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                    Err(e) => return Err(SpillError::io("read spill run", self.path(), e)),
                }
            }
        }
        self.reader = None;
        self.chunks.pop_front();
        Ok(())
    }
}

impl<T: Record> Cursor<T> for RunReader<T> {
    fn next_record(&mut self) -> Result<Option<T>, SpillError> {
        while self.in_chunk == 0 {
            if self.chunks.is_empty() {
                return Ok(None);
            }
            self.finish_front()?;
            self.open_front()?;
        }
        let reader = self.reader.as_mut().expect("open chunk has a reader");
        if let Err(e) = reader.read_exact(&mut self.encoded) {
            if e.kind() == io::ErrorKind::UnexpectedEof {
                let read = self.records - self.remaining;
                let records = self.records;
                return Err(self.corrupt(format!("ended after {read} of {records} records")));
            }
            return Err(SpillError::io("read spill run", self.path(), e));
        }
        self.in_chunk -= 1;
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

/// One chunk file being written.
struct OpenChunk {
    // Declared before `file` so the handle closes before an error removes the file.
    writer: BufWriter<Box<dyn Write + Send>>,
    file: SpillFile,
    records: u64,
}

impl OpenChunk {
    fn close<T: Record>(mut self) -> Result<Chunk, SpillError> {
        self.writer
            .flush()
            .map_err(|e| SpillError::io("write spill run", self.file.path(), e))?;
        let Self {
            writer,
            mut file,
            records,
        } = self;
        drop(writer);
        let bytes = records
            .checked_mul(T::SIZE as u64)
            .ok_or(SpillError::Overflow)?;
        file.completed(bytes);
        Ok(Chunk { file, records })
    }
}

fn write_run<T: Record>(
    scratch: &Scratch,
    buffer_bytes: usize,
    records: &mut impl Cursor<T>,
) -> Result<Run, SpillError> {
    let per_chunk = chunk_records_of::<T>(scratch.chunk_bytes());
    let mut chunks = VecDeque::new();
    let mut open: Option<OpenChunk> = None;
    let mut encoded = vec![0; T::SIZE];
    let mut count = 0u64;
    while let Some(record) = records.next_record()? {
        if open.as_ref().is_none_or(|chunk| chunk.records == per_chunk) {
            if let Some(full) = open.take() {
                chunks.push_back(full.close::<T>()?);
            }
            let file = scratch.new_file("run")?;
            let writer = BufWriter::with_capacity(
                buffer_bytes,
                scratch
                    .io()
                    .create(file.path())
                    .map_err(|e| SpillError::io("create spill run", file.path(), e))?,
            );
            open = Some(OpenChunk {
                writer,
                file,
                records: 0,
            });
        }
        let chunk = open.as_mut().expect("a chunk is open");
        record.encode(&mut encoded);
        let next = chunk.records.checked_add(1).ok_or(SpillError::Overflow)?;
        // A failing write_all may write a prefix before returning its error. Pre-account
        // the entire attempted record so failure peaks cannot omit that prefix.
        chunk.file.completed(
            next.checked_mul(T::SIZE as u64)
                .ok_or(SpillError::Overflow)?,
        );
        chunk
            .writer
            .write_all(&encoded)
            .map_err(|e| SpillError::io("write spill run", chunk.file.path(), e))?;
        chunk.records = next;
        count = count.checked_add(1).ok_or(SpillError::Overflow)?;
    }
    if let Some(last) = open.take() {
        chunks.push_back(last.close::<T>()?);
    }
    Ok(Run {
        chunks,
        records: count,
        generation: 0,
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
            self.buffer.reserve_exact(target.saturating_sub(capacity));
        }
        self.buffer.push(record);
        self.stats.records = self
            .stats
            .records
            .checked_add(1)
            .ok_or(SpillError::Overflow)?;
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

    #[cfg(test)]
    pub(super) fn inventory_len(&self) -> usize {
        self.runs.len()
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
        self.compact_inventory()?;
        Ok(())
    }

    /// Keep run descriptors bounded independently of the input count. Delay the first
    /// compaction until 64F runs so ordinary inputs retain the selective final-pass policy.
    /// Higher generations hold fewer than F descriptors. Since each such generation
    /// represents at least F times the records of its predecessor, u64 counts permit no
    /// more than 64 generations: fewer than 128F descriptors in all.
    fn compact_inventory(&mut self) -> Result<(), SpillError> {
        let mut generation = 0u8;
        loop {
            let threshold = if generation == 0 {
                self.config
                    .fan_in
                    .checked_mul(64)
                    .ok_or(SpillError::Overflow)?
            } else {
                self.config.fan_in
            };
            let Some(first) = self
                .runs
                .iter()
                .position(|run| run.generation == generation)
            else {
                break;
            };
            let count = self.runs[first..]
                .iter()
                .take_while(|run| run.generation == generation)
                .count();
            if count < threshold {
                break;
            }
            let readers = self
                .runs
                .drain(first..first + self.config.fan_in)
                .map(|run| RunReader::<T>::open(self.scratch, run, self.config.buffer_bytes))
                .collect::<Result<Vec<_>, _>>()?;
            let mut merger = Merger::new(readers)?;
            let mut run = write_run(self.scratch, self.config.buffer_bytes, &mut merger)?;
            run.generation = generation.checked_add(1).ok_or(SpillError::Overflow)?;
            self.stats.spilled_bytes += run.file_bytes();
            self.stats.merge_levels = self.stats.merge_levels.max(u64::from(run.generation));
            drop(merger);
            self.runs.insert(first, run);
            generation += 1;
        }
        Ok(())
    }

    /// Upper bound for descriptor inventory and its Vec reallocations, including owned
    /// paths: fewer than `128F` run descriptors (two generations while merging), and one
    /// chunk descriptor per live chunk file of `spill_bytes` (the sort's live spill, a merge's
    /// output included), at the production [`CHUNK_BYTES`]. The owning producer adds this to
    /// its resident-memory declaration.
    pub(crate) fn inventory_memory_bound(
        config: SortConfig,
        scratch_path_bytes: usize,
        spill_bytes: u64,
    ) -> u64 {
        let paths = (scratch_path_bytes as u64)
            .saturating_mul(2)
            .saturating_add(64);
        let runs = (config.fan_in as u64).saturating_mul(128).saturating_mul(2);
        let chunk_bytes = chunk_records::<T>().saturating_mul(T::SIZE as u64).max(1);
        let chunks = spill_bytes.div_ceil(chunk_bytes).saturating_add(runs);
        runs.saturating_mul((size_of::<Run>() as u64).saturating_add(paths))
            .saturating_add(
                chunks.saturating_mul((size_of::<Chunk>() as u64).saturating_add(paths)),
            )
    }

    /// Group sizes for one pass over `runs > F` runs. While a full pass would still leave more
    /// than `F` runs, every run is merged in groups of `F`. Otherwise this is the next-to-last
    /// pass: only enough leading groups are merged to leave exactly `F` runs, and the rest are
    /// not rewritten. Each group of `g` runs removes `g - 1`.
    pub(super) fn pass_groups(runs: usize, fan_in: usize) -> Vec<usize> {
        if runs.div_ceil(fan_in) > fan_in {
            return vec![fan_in; runs.div_ceil(fan_in)];
        }
        let excess = runs - fan_in;
        let mut groups = vec![fan_in; excess / (fan_in - 1)];
        if !excess.is_multiple_of(fan_in - 1) {
            groups.push(excess % (fan_in - 1) + 1);
        }
        groups
    }

    /// One merge pass. Groups are consecutive and keep their position, and the final merge
    /// breaks ties by lower source index, so order stays the stable sort of the input.
    fn merge_level(&mut self) -> Result<(), SpillError> {
        let groups = Self::pass_groups(self.runs.len(), self.config.fan_in);
        let mut pending = std::mem::take(&mut self.runs).into_iter();
        let mut merged = Vec::with_capacity(pending.len());
        for size in groups {
            let mut group: Vec<Run> = pending.by_ref().take(size).collect();
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
                    // Inputs were removed chunk by chunk as the merge read them; dropping
                    // the merger removes any remainder (e.g. after an error).
                    drop(merger);
                    merged.push(run);
                }
            }
        }
        merged.extend(pending);
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
        self.chunks.iter().map(|chunk| chunk.file.bytes()).sum()
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

    /// The whole sorted output as one vector, without a copy, when the input never left
    /// memory and was sorted as one chunk (`jobs == 1`) and nothing has been read yet.
    /// Otherwise the stream is returned unchanged.
    pub(crate) fn into_sorted_vec(mut self) -> Result<Vec<T>, Self> {
        let untouched = self.remaining == self.stats.records;
        match self.source.take() {
            None if self.stats.records == 0 => Ok(Vec::new()),
            Some(Source::Memory(merger)) if untouched && merger.sources.len() <= 1 => {
                let Merger { sources, heap } = merger;
                drop(heap);
                match sources.into_iter().next() {
                    None => Ok(Vec::new()),
                    Some(cursor) => {
                        let MemCursor { data, .. } = cursor;
                        Ok(Arc::try_unwrap(data).unwrap_or_else(|data| (*data).clone()))
                    }
                }
            }
            source => {
                self.source = source;
                Err(self)
            }
        }
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
