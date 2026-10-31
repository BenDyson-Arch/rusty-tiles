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
    _record: PhantomData<T>,
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
            _record: PhantomData,
        })
    }

    pub(crate) fn push(&mut self, record: &T) -> Result<(), SpillError> {
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
            ..
        } = self;
        drop(writer);
        let bytes = records
            .checked_mul(encoded.len() as u64)
            .ok_or(SpillError::Overflow)?;
        file.completed(bytes);
        Ok(Sequence {
            file,
            io,
            buffer_bytes,
            records,
            layout,
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

    pub(crate) fn is_empty(&self) -> bool {
        self.records == 0
    }

    /// A new reader from the first record. Readers are independent.
    pub(crate) fn reader(&self) -> Result<SequenceReader<'_, T>, SpillError> {
        let inner = self
            .io
            .open(self.file.path())
            .map_err(|e| SpillError::io("open spill sequence", self.file.path(), e))?;
        Ok(SequenceReader {
            reader: Some(BufReader::with_capacity(self.buffer_bytes, inner)),
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
        let offset = first
            .checked_mul(self.size() as u64)
            .ok_or(SpillError::Overflow)?;
        let inner = self
            .io
            .open_at(self.file.path(), offset)
            .map_err(|e| SpillError::io("open spill sequence", self.file.path(), e))?;
        Ok(SequenceReader {
            reader: Some(BufReader::with_capacity(buffer_bytes, inner)),
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
            _record: PhantomData,
        })
    }
}

/// Positioned reads of whole records of a completed [`Sequence`]. Holds its own handle.
pub(crate) struct RandomRecords<T: Record> {
    file: File,
    path: std::path::PathBuf,
    records: u64,
    layout: Layout,
    _record: PhantomData<T>,
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
