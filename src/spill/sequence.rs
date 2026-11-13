//! Sequential spill: records written once, in order, and read back in that order any number of
//! times. Files live in the owned scratch directory and are accounted like sort runs.
use super::{
    record_size,
    scratch::{read_exact_at, SpillFile},
    Layout, Record, Scratch, SpillError, SpillIo,
};
use std::{
    fs::File,
    io::{self, BufReader, BufWriter, Read, Write},
    marker::PhantomData,
    sync::Arc,
};

/// Appends records to one scratch file. Dropping it unfinished removes the file.
pub(crate) struct SequenceWriter<T: Record> {
    // Declared before `file` so the handle closes before an error removes the file.
    writer: BufWriter<Box<dyn Write + Send>>,
    file: SpillFile,
    io: Arc<dyn SpillIo>,
    buffer_bytes: usize,
    records: u64,
    layout: Layout,
    encoded: Vec<u8>,
    /// Block compression state, for a writer from [`SequenceWriter::compressed`] (boxed: plain
    /// writers stay as small as before, and they are embedded in other spill types).
    blocks: Option<Box<BlockWriter>>,
    _record: PhantomData<T>,
}

/// Records gathered into fixed-count blocks, each written compressed or stored.
struct BlockWriter {
    raw: Vec<u8>,
    scratch: Vec<u8>,
    index: BlockIndex,
}

impl<T: Record> SequenceWriter<T> {
    pub(crate) fn new(
        scratch: &Scratch,
        stem: &str,
        buffer_bytes: usize,
    ) -> Result<Self, SpillError> {
        Self::with_layout(scratch, stem, buffer_bytes, Layout::FULL)
    }

    /// A writer encoding every record under `layout`; the sequence keeps it for its readers.
    pub(crate) fn with_layout(
        scratch: &Scratch,
        stem: &str,
        buffer_bytes: usize,
        layout: Layout,
    ) -> Result<Self, SpillError> {
        if buffer_bytes == 0 {
            return Err(SpillError::InvalidConfig("stream buffer must be nonzero"));
        }
        let size = record_size::<T>(layout)?;
        let file = scratch.new_file(stem)?;
        let io = scratch.io_handle();
        let inner = io
            .create(file.path())
            .map_err(|e| SpillError::io("create spill sequence", file.path(), e))?;
        Ok(Self {
            writer: BufWriter::with_capacity(buffer_bytes, inner),
            file,
            io,
            buffer_bytes,
            records: 0,
            layout,
            encoded: vec![0; size],
            blocks: None,
            _record: PhantomData,
        })
    }

    /// A writer whose sequence is stored as compressed blocks of a fixed record count (see
    /// [`block_records`]). Readers see the same records; the file holds at most their bytes
    /// (a block that does not shrink is stored as is), and its live bytes are what is on disk.
    pub(crate) fn compressed(
        scratch: &Scratch,
        stem: &str,
        buffer_bytes: usize,
        layout: Layout,
    ) -> Result<Self, SpillError> {
        let mut writer = Self::with_layout(scratch, stem, buffer_bytes, layout)?;
        let size = writer.encoded.len();
        let per_block = block_records(size, buffer_bytes);
        writer.blocks = Some(Box::new(BlockWriter {
            raw: Vec::with_capacity(per_block as usize * size),
            scratch: Vec::new(),
            index: BlockIndex {
                per_block,
                record_bytes: size,
                offsets: vec![0],
                stored: Vec::new(),
            },
        }));
        Ok(writer)
    }

    /// Writes the gathered records as one block (compressed, or stored when that is smaller).
    fn emit_block(&mut self) -> Result<(), SpillError> {
        let Some(blocks) = self.blocks.as_mut() else {
            return Ok(());
        };
        if blocks.raw.is_empty() {
            return Ok(());
        }
        let encoded = super::codec::encode(&blocks.raw, &mut blocks.scratch);
        let at = *blocks.index.offsets.last().expect("offsets start at zero");
        let end = at
            .checked_add(encoded.bytes.len() as u64)
            .ok_or(SpillError::Overflow)?;
        // Accounted before the write, as plain sequences are: a failed write may have
        // emitted a prefix.
        self.file.completed(end);
        self.writer
            .write_all(&encoded.bytes)
            .map_err(|e| SpillError::io("write spill sequence", self.file.path(), e))?;
        blocks.index.offsets.push(end);
        blocks.index.stored.push(encoded.stored);
        blocks.raw.clear();
        Ok(())
    }

    pub(crate) fn push(&mut self, record: &T) -> Result<(), SpillError> {
        if self.blocks.is_some() {
            let records = self.records.checked_add(1).ok_or(SpillError::Overflow)?;
            record.encode_in(self.layout, &mut self.encoded);
            let blocks = self.blocks.as_mut().expect("block writer");
            blocks.raw.extend_from_slice(&self.encoded);
            let full =
                blocks.raw.len() as u64 == blocks.index.per_block * self.encoded.len() as u64;
            self.records = records;
            if full {
                self.emit_block()?;
            }
            return Ok(());
        }
        let records = self.records.checked_add(1).ok_or(SpillError::Overflow)?;
        let bytes = records
            .checked_mul(self.encoded.len() as u64)
            .ok_or(SpillError::Overflow)?;
        record.encode_in(self.layout, &mut self.encoded);
        // Include the attempted record before writing: a failed write may have emitted a
        // strict prefix. Counting its full size remains conservative on that failure path.
        self.file.completed(bytes);
        self.writer
            .write_all(&self.encoded)
            .map_err(|e| SpillError::io("write spill sequence", self.file.path(), e))?;
        self.records = records;
        Ok(())
    }

    pub(crate) fn len(&self) -> u64 {
        self.records
    }

    pub(crate) fn finish(mut self) -> Result<Sequence<T>, SpillError> {
        self.emit_block()?;
        self.writer
            .flush()
            .map_err(|e| SpillError::io("write spill sequence", self.file.path(), e))?;
        let Self {
            writer,
            mut file,
            io,
            buffer_bytes,
            records,
            layout,
            encoded,
            blocks,
            ..
        } = self;
        drop(writer);
        let blocks = match blocks.map(|b| *b) {
            Some(BlockWriter { mut index, .. }) => {
                file.completed(*index.offsets.last().expect("offsets start at zero"));
                // The index stays resident while the sequence lives: no growth slack.
                index.offsets.shrink_to_fit();
                index.stored.shrink_to_fit();
                Some(Arc::new(index))
            }
            None => {
                let bytes = records
                    .checked_mul(encoded.len() as u64)
                    .ok_or(SpillError::Overflow)?;
                file.completed(bytes);
                None
            }
        };
        Ok(Sequence {
            file,
            io,
            buffer_bytes,
            records,
            layout,
            blocks,
            _record: PhantomData,
        })
    }
}

/// A completed sequence; its file is removed when it is dropped.
pub(crate) struct Sequence<T: Record> {
    file: SpillFile,
    io: Arc<dyn SpillIo>,
    buffer_bytes: usize,
    records: u64,
    /// Run-level encoding of every record (its width was checked by the writer).
    layout: Layout,
    /// Present when the file holds compressed blocks ([`SequenceWriter::compressed`]).
    blocks: Option<Arc<BlockIndex>>,
    _record: PhantomData<T>,
}

impl<T: Record> Sequence<T> {
    pub(crate) fn len(&self) -> u64 {
        self.records
    }

    /// The run-level encoding of the records.
    pub(crate) fn layout(&self) -> Layout {
        self.layout
    }

    /// Encoded bytes per record.
    fn size(&self) -> usize {
        T::size_in(self.layout)
    }

    /// A new reader from the first record. Readers are independent.
    pub(crate) fn reader(&self) -> Result<SequenceReader<'_, T>, SpillError> {
        let (inner, buffer) = self.open_bytes(0, self.buffer_bytes)?;
        Ok(SequenceReader {
            reader: Some(BufReader::with_capacity(buffer, inner)),
            sequence: self,
            remaining: self.records,
            encoded: vec![0; self.size()],
            ranged: false,
        })
    }

    /// A reader of records `[first, end)` only, positioned by seeking (fixed-size records).
    pub(crate) fn range(
        &self,
        first: u64,
        end: u64,
        buffer_bytes: usize,
    ) -> Result<SequenceReader<'_, T>, SpillError> {
        if first > end || end > self.records || buffer_bytes == 0 {
            return Err(SpillError::InvalidConfig(
                "sequence range outside its records",
            ));
        }
        let (inner, buffer) = self.open_bytes(first, buffer_bytes)?;
        Ok(SequenceReader {
            reader: Some(BufReader::with_capacity(buffer, inner)),
            sequence: self,
            remaining: end - first,
            encoded: vec![0; self.size()],
            ranged: true,
        })
    }
}

impl<T: Record> Sequence<T> {
    /// Positioned access to the completed records, independent of any sequential reader.
    pub(crate) fn random(&self) -> Result<RandomRecords<T>, SpillError> {
        let file = self
            .io
            .open_random(self.file.path())
            .map_err(|e| SpillError::io("open spill sequence", self.file.path(), e))?;
        Ok(RandomRecords {
            file,
            path: self.file.path().to_owned(),
            records: self.records,
            layout: self.layout,
            blocks: self.blocks.clone(),
            cache: std::sync::Mutex::new((usize::MAX, Vec::new(), Vec::new())),
            _record: PhantomData,
        })
    }

    /// The encoded byte stream from record `first` on, and the read buffer to put over it: the
    /// file itself, or its decoded blocks (which buffer a block already, so a small buffer).
    fn open_bytes(
        &self,
        first: u64,
        buffer_bytes: usize,
    ) -> Result<(Box<dyn Read + Send>, usize), SpillError> {
        let path = self.file.path();
        let Some(index) = &self.blocks else {
            let offset = first
                .checked_mul(self.size() as u64)
                .ok_or(SpillError::Overflow)?;
            let inner = self
                .io
                .open_at(path, offset)
                .map_err(|e| SpillError::io("open spill sequence", path, e))?;
            return Ok((inner, buffer_bytes));
        };
        let file = self
            .io
            .open_random(path)
            .map_err(|e| SpillError::io("open spill sequence", path, e))?;
        let block = first / index.per_block;
        let mut reader = BlockReader {
            file,
            index: index.clone(),
            records: self.records,
            next: block as usize,
            compressed: Vec::new(),
            raw: Vec::new(),
            pos: 0,
        };
        let skip = (first - block * index.per_block) * self.size() as u64;
        let skipped = io::copy(&mut (&mut reader).take(skip), &mut io::sink())
            .map_err(|e| SpillError::io("open spill sequence", path, e))?;
        if skipped != skip {
            return Err(SpillError::CorruptRun {
                path: path.to_owned(),
                detail: format!("compressed blocks end before record {first}"),
            });
        }
        Ok((Box::new(reader), self.size().max(1) * 16))
    }

    /// Bytes this sequence holds on disk (its compressed blocks, or all its records).
    #[cfg(test)]
    pub(crate) fn stored_bytes(&self) -> u64 {
        self.file.bytes()
    }
}

/// Records per compressed block for `record_bytes`-byte records under a stream buffer of
/// `buffer_bytes`: blocks of about a quarter of the buffer, at most 64 KiB, so a reader's block
/// and its compressed copy stay within the buffer charged for a plain reader.
pub(crate) fn block_records(record_bytes: usize, buffer_bytes: usize) -> u64 {
    let target = (buffer_bytes / 4).clamp(1, 64 << 10);
    (target / record_bytes.max(1)).max(1) as u64
}

/// Upper bound on the resident block index of compressed sequences holding up to `bytes`
/// record bytes in all, under `buffer_bytes`: a block holds at least half its target (or one
/// record), and the index keeps nine bytes per block, plus one partial block per sequence for
/// `sequences` sequences.
pub(crate) fn block_index_memory_bound(bytes: u64, buffer_bytes: usize, sequences: u64) -> u64 {
    let half = ((buffer_bytes / 4).clamp(1, 64 << 10) / 2).max(1) as u64;
    bytes
        .div_ceil(half)
        .saturating_add(sequences.saturating_mul(2))
        .saturating_mul(9)
}

/// Where each block of a compressed sequence lies: block `i` holds records
/// `[i * per_block, (i + 1) * per_block)` (the last may be short) at bytes
/// `offsets[i]..offsets[i + 1]`, compressed unless `stored[i]`.
struct BlockIndex {
    per_block: u64,
    record_bytes: usize,
    offsets: Vec<u64>,
    stored: Vec<bool>,
}

impl BlockIndex {
    fn blocks(&self) -> usize {
        self.stored.len()
    }

    /// Decodes block `block` of `file` into `raw` (of its records' bytes).
    fn read_block(
        &self,
        file: &File,
        block: usize,
        records: u64,
        compressed: &mut Vec<u8>,
        raw: &mut Vec<u8>,
    ) -> io::Result<()> {
        let (start, end) = (self.offsets[block], self.offsets[block + 1]);
        let first = block as u64 * self.per_block;
        let count = (records - first).min(self.per_block) as usize;
        raw.resize(count * self.record_bytes, 0);
        compressed.resize((end - start) as usize, 0);
        read_exact_at(file, compressed, start)?;
        super::codec::decode(compressed, self.stored[block], raw)
    }
}

/// The decoded bytes of a compressed sequence's blocks, in order, from one block on.
struct BlockReader {
    file: File,
    index: Arc<BlockIndex>,
    records: u64,
    next: usize,
    compressed: Vec<u8>,
    raw: Vec<u8>,
    pos: usize,
}

impl Read for BlockReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if self.pos == self.raw.len() {
            if self.next >= self.index.blocks() {
                return Ok(0);
            }
            self.index.read_block(
                &self.file,
                self.next,
                self.records,
                &mut self.compressed,
                &mut self.raw,
            )?;
            self.next += 1;
            self.pos = 0;
        }
        let n = out.len().min(self.raw.len() - self.pos);
        out[..n].copy_from_slice(&self.raw[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

/// Positioned reads of whole records of a completed [`Sequence`]. Holds its own handle.
pub(crate) struct RandomRecords<T: Record> {
    file: File,
    path: std::path::PathBuf,
    records: u64,
    layout: Layout,
    blocks: Option<Arc<BlockIndex>>,
    /// The last block decoded (compressed sequences): its index, its records' bytes and the
    /// buffer its compressed copy was read into. Neighbouring reads reuse it.
    cache: std::sync::Mutex<(usize, Vec<u8>, Vec<u8>)>,
    _record: PhantomData<T>,
}

/// The most a positioned reader of a compressed sequence of `record_bytes` records holds
/// besides the caller's buffers: one decoded block and its compressed copy, each at most the
/// block target (64 KiB) or one record when that is larger.
pub(crate) fn random_cache_bound(record_bytes: usize) -> u64 {
    2 * record_bytes.max(64 << 10) as u64
}

impl<T: Record> RandomRecords<T> {
    /// Replaces `out` with records `first..first + count`; the range must lie in the sequence.
    pub(crate) fn read(
        &self,
        first: u64,
        count: usize,
        encoded: &mut Vec<u8>,
        out: &mut Vec<T>,
    ) -> Result<(), SpillError> {
        let end = first
            .checked_add(count as u64)
            .filter(|&end| end <= self.records)
            .ok_or(SpillError::InvalidConfig(
                "record range outside the sequence",
            ))?;
        let size = T::size_in(self.layout);
        let bytes = count.checked_mul(size).ok_or(SpillError::Overflow)?;
        if let Some(index) = &self.blocks {
            // Every block the range touches is decoded, unless it is the last one decoded; its
            // records are copied out.
            encoded.clear();
            let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
            let (cached, raw, compressed) = &mut *cache;
            let mut at = first;
            while at < end {
                let block = (at / index.per_block) as usize;
                if *cached != block {
                    // A failed read leaves no block cached.
                    *cached = usize::MAX;
                    index
                        .read_block(&self.file, block, self.records, compressed, raw)
                        .map_err(|e| SpillError::io("read spill sequence", &self.path, e))?;
                    *cached = block;
                }
                let start = block as u64 * index.per_block;
                let lo = (at - start) as usize * size;
                let hi = ((end.min(start + index.per_block) - start) as usize) * size;
                encoded.extend_from_slice(&raw[lo..hi]);
                at = start + index.per_block;
            }
            out.clear();
            out.extend(
                encoded
                    .chunks_exact(size)
                    .map(|b| T::decode_in(self.layout, b)),
            );
            return Ok(());
        }
        encoded.resize(bytes, 0);
        let offset = first.checked_mul(size as u64).ok_or(SpillError::Overflow)?;
        read_exact_at(&self.file, encoded, offset).map_err(|e| {
            if e.kind() == io::ErrorKind::UnexpectedEof {
                SpillError::CorruptRun {
                    path: self.path.clone(),
                    detail: format!("records {first}..{end} end early"),
                }
            } else {
                SpillError::io("read spill sequence", &self.path, e)
            }
        })?;
        out.clear();
        out.extend(
            encoded
                .chunks_exact(size)
                .map(|bytes| T::decode_in(self.layout, bytes)),
        );
        Ok(())
    }
}

/// Appends raw bytes to one scratch file (a log of variable-length entries).
pub(crate) struct ByteWriter {
    // Declared before `file` so the handle closes before an error removes the file.
    writer: BufWriter<Box<dyn Write + Send>>,
    file: SpillFile,
    io: Arc<dyn SpillIo>,
    len: u64,
}

impl ByteWriter {
    pub(crate) fn new(
        scratch: &Scratch,
        stem: &str,
        buffer_bytes: usize,
    ) -> Result<Self, SpillError> {
        if buffer_bytes == 0 {
            return Err(SpillError::InvalidConfig("stream buffer must be nonzero"));
        }
        let file = scratch.new_file(stem)?;
        let io = scratch.io_handle();
        let inner = io
            .create(file.path())
            .map_err(|e| SpillError::io("create spill log", file.path(), e))?;
        Ok(Self {
            writer: BufWriter::with_capacity(buffer_bytes, inner),
            file,
            io,
            len: 0,
        })
    }

    pub(crate) fn append(&mut self, bytes: &[u8]) -> Result<(), SpillError> {
        let len = self
            .len
            .checked_add(bytes.len() as u64)
            .ok_or(SpillError::Overflow)?;
        self.file.completed(len);
        self.writer
            .write_all(bytes)
            .map_err(|e| SpillError::io("write spill log", self.file.path(), e))?;
        self.len = len;
        Ok(())
    }

    pub(crate) fn len(&self) -> u64 {
        self.len
    }

    pub(crate) fn finish(mut self) -> Result<ByteFile, SpillError> {
        self.writer
            .flush()
            .map_err(|e| SpillError::io("write spill log", self.file.path(), e))?;
        let Self {
            writer,
            file,
            io,
            len,
        } = self;
        drop(writer);
        let random = io
            .open_random(file.path())
            .map_err(|e| SpillError::io("open spill log", file.path(), e))?;
        Ok(ByteFile {
            random,
            file,
            io,
            len,
        })
    }
}

impl ByteWriter {
    /// Like [`ByteWriter::finish`], but the completed log holds no open handle: many
    /// such logs can live at once without consuming file descriptors.
    pub(crate) fn finish_closed(mut self) -> Result<ClosedByteFile, SpillError> {
        self.writer
            .flush()
            .map_err(|e| SpillError::io("write spill log", self.file.path(), e))?;
        let Self {
            writer,
            file,
            io,
            len,
        } = self;
        drop(writer);
        Ok(ClosedByteFile { file, io, len })
    }
}

/// A completed byte log without an open handle; reads go through a handle from
/// [`ClosedByteFile::open`]. Its file is removed when it is dropped.
pub(crate) struct ClosedByteFile {
    file: SpillFile,
    io: Arc<dyn SpillIo>,
    len: u64,
}

impl ClosedByteFile {
    /// A handle for positioned reads; it may outlive other handles but not the log.
    pub(crate) fn open(&self) -> Result<File, SpillError> {
        self.io
            .open_random(self.file.path())
            .map_err(|e| SpillError::io("open spill log", self.file.path(), e))
    }

    /// Reads exactly `bytes.len()` bytes at `offset` through `handle` (from [`Self::open`]).
    pub(crate) fn read_at(
        &self,
        handle: &File,
        offset: u64,
        bytes: &mut [u8],
    ) -> Result<(), SpillError> {
        if offset
            .checked_add(bytes.len() as u64)
            .is_none_or(|end| end > self.len)
        {
            return Err(SpillError::CorruptRun {
                path: self.file.path().to_owned(),
                detail: format!("range at {offset} outside the log"),
            });
        }
        read_exact_at(handle, bytes, offset)
            .map_err(|e| SpillError::io("read spill log", self.file.path(), e))
    }
}

/// A completed byte log; its file is removed when it is dropped.
pub(crate) struct ByteFile {
    // Declared before `file` so the handle closes before the file is removed.
    random: File,
    file: SpillFile,
    io: Arc<dyn SpillIo>,
    len: u64,
}

impl ByteFile {
    pub(crate) fn len(&self) -> u64 {
        self.len
    }

    /// Reads exactly `bytes.len()` bytes at `offset`; the range must lie in the log.
    pub(crate) fn read_at(&self, offset: u64, bytes: &mut [u8]) -> Result<(), SpillError> {
        if offset
            .checked_add(bytes.len() as u64)
            .is_none_or(|end| end > self.len)
        {
            return Err(self.corrupt(format!("range at {offset} outside the log")));
        }
        read_exact_at(&self.random, bytes, offset)
            .map_err(|e| SpillError::io("read spill log", self.file.path(), e))
    }

    /// A sequential reader of the whole log from its first byte.
    pub(crate) fn reader(
        &self,
        buffer_bytes: usize,
    ) -> Result<BufReader<Box<dyn Read + Send>>, SpillError> {
        let inner = self
            .io
            .open(self.file.path())
            .map_err(|e| SpillError::io("open spill log", self.file.path(), e))?;
        Ok(BufReader::with_capacity(buffer_bytes.max(1), inner))
    }

    pub(crate) fn path(&self) -> &std::path::Path {
        self.file.path()
    }

    fn corrupt(&self, detail: String) -> SpillError {
        SpillError::CorruptRun {
            path: self.file.path().to_owned(),
            detail,
        }
    }
}

/// Reads a sequence in write order. Fused after the first error; a missing or extra byte is a
/// typed corrupt-run error.
pub(crate) struct SequenceReader<'a, T: Record> {
    reader: Option<BufReader<Box<dyn Read + Send>>>,
    sequence: &'a Sequence<T>,
    remaining: u64,
    encoded: Vec<u8>,
    /// A range reader ends at its range; the rest of the file is not its business.
    ranged: bool,
}

impl<T: Record> SequenceReader<'_, T> {
    fn corrupt(&self, detail: String) -> SpillError {
        SpillError::CorruptRun {
            path: self.sequence.file.path().to_owned(),
            detail,
        }
    }

    fn read_next(&mut self) -> Result<Option<T>, SpillError> {
        let Some(reader) = self.reader.as_mut() else {
            return Ok(None);
        };
        if self.remaining == 0 && self.ranged {
            self.reader = None;
            return Ok(None);
        }
        if self.remaining == 0 {
            let mut probe = [0u8; 1];
            loop {
                match reader.read(&mut probe) {
                    Ok(0) => break,
                    Ok(_) => {
                        let records = self.sequence.records;
                        return Err(
                            self.corrupt(format!("bytes follow the expected {records} records"))
                        );
                    }
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                    Err(e) => {
                        return Err(SpillError::io(
                            "read spill sequence",
                            self.sequence.file.path(),
                            e,
                        ))
                    }
                }
            }
            self.reader = None;
            return Ok(None);
        }
        if let Err(e) = reader.read_exact(&mut self.encoded) {
            if e.kind() == io::ErrorKind::UnexpectedEof {
                let read = self.sequence.records - self.remaining;
                let records = self.sequence.records;
                return Err(self.corrupt(format!("ended after {read} of {records} records")));
            }
            return Err(SpillError::io(
                "read spill sequence",
                self.sequence.file.path(),
                e,
            ));
        }
        self.remaining -= 1;
        Ok(Some(T::decode_in(self.sequence.layout, &self.encoded)))
    }
}

impl<T: Record> Iterator for SequenceReader<'_, T> {
    type Item = Result<T, SpillError>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.read_next() {
            Ok(record) => record.map(Ok),
            Err(error) => {
                self.reader = None;
                Some(Err(error))
            }
        }
    }
}

/// Fixed-width slots in one scratch file, written and read at any index in any order. A slot
/// never written reads as zeros up to the file end; the file is sparse where the platform
/// allows. Holds its own read-write handle; the file is removed when it is dropped.
pub(crate) struct SlotFile {
    // Declared before `file` so the handle closes before the file is removed.
    handle: File,
    file: SpillFile,
    width: usize,
    /// One past the highest slot written.
    end: u64,
}

impl SlotFile {
    pub(crate) fn new(scratch: &Scratch, stem: &str, width: usize) -> Result<Self, SpillError> {
        if width == 0 {
            return Err(SpillError::InvalidConfig("slot width must be nonzero"));
        }
        let file = scratch.new_file(stem)?;
        let handle = scratch
            .io()
            .create_slots(file.path())
            .map_err(|e| SpillError::io("create spill slots", file.path(), e))?;
        Ok(Self {
            handle,
            file,
            width,
            end: 0,
        })
    }

    fn offset(&self, index: u64) -> Result<u64, SpillError> {
        index
            .checked_mul(self.width as u64)
            .ok_or(SpillError::Overflow)
    }

    /// Writes slot `index`; `bytes.len()` must be the slot width.
    pub(crate) fn write(&mut self, index: u64, bytes: &[u8]) -> Result<(), SpillError> {
        if bytes.len() != self.width {
            return Err(SpillError::InvalidConfig("slot write of the wrong width"));
        }
        let offset = self.offset(index)?;
        let end = index.checked_add(1).ok_or(SpillError::Overflow)?;
        if end > self.end {
            // Accounted at the logical extent before the write, as sequences are.
            self.file.completed(self.offset(end)?);
            self.end = end;
        }
        super::scratch::write_all_at(&self.handle, bytes, offset)
            .map_err(|e| SpillError::io("write spill slots", self.file.path(), e))
    }

    /// A contiguous run of complete slots, for bounded page caches. Accounts the logical
    /// extent before any positioned write, just like a single-slot write.
    pub(crate) fn write_many(&mut self, index: u64, bytes: &[u8]) -> Result<(), SpillError> {
        if bytes.is_empty() || !bytes.len().is_multiple_of(self.width) {
            return Err(SpillError::InvalidConfig(
                "slot batch write of the wrong width",
            ));
        }
        let end = index
            .checked_add((bytes.len() / self.width) as u64)
            .ok_or(SpillError::Overflow)?;
        if end > self.end {
            self.file.completed(self.offset(end)?);
            self.end = end;
        }
        super::scratch::write_all_at(&self.handle, bytes, self.offset(index)?)
            .map_err(|e| SpillError::io("write spill slots", self.file.path(), e))
    }
    pub(crate) fn read_many(&self, index: u64, bytes: &mut [u8]) -> Result<(), SpillError> {
        if bytes.is_empty() || !bytes.len().is_multiple_of(self.width) {
            return Err(SpillError::InvalidConfig(
                "slot batch read of the wrong width",
            ));
        }
        let end = index
            .checked_add((bytes.len() / self.width) as u64)
            .ok_or(SpillError::Overflow)?;
        if end > self.end {
            return Err(SpillError::CorruptRun {
                path: self.file.path().to_owned(),
                detail: "slot batch was never written".into(),
            });
        }
        read_exact_at(&self.handle, bytes, self.offset(index)?)
            .map_err(|e| SpillError::io("read spill slots", self.file.path(), e))
    }
    pub(crate) fn len(&self) -> u64 {
        self.end
    }

    /// Reads slot `index` into `bytes` (the slot width); a slot past the end is corrupt.
    pub(crate) fn read(&self, index: u64, bytes: &mut [u8]) -> Result<(), SpillError> {
        if bytes.len() != self.width {
            return Err(SpillError::InvalidConfig("slot read of the wrong width"));
        }
        if index >= self.end {
            return Err(SpillError::CorruptRun {
                path: self.file.path().to_owned(),
                detail: format!("slot {index} was never written"),
            });
        }
        read_exact_at(&self.handle, bytes, self.offset(index)?).map_err(|e| {
            if e.kind() == io::ErrorKind::UnexpectedEof {
                SpillError::CorruptRun {
                    path: self.file.path().to_owned(),
                    detail: format!("slot {index} ends early"),
                }
            } else {
                SpillError::io("read spill slots", self.file.path(), e)
            }
        })
    }
}
