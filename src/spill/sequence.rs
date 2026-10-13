//! Sequential spill: records written once, in order, and read back in that order any number of
//! times. Files live in the owned scratch directory and are accounted like sort runs.
use super::{scratch::SpillFile, Record, Scratch, SpillError, SpillIo};
use std::{
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
    encoded: Vec<u8>,
    _record: PhantomData<T>,
}

impl<T: Record> SequenceWriter<T> {
    pub(crate) fn new(
        scratch: &Scratch,
        stem: &str,
        buffer_bytes: usize,
    ) -> Result<Self, SpillError> {
        if buffer_bytes == 0 {
            return Err(SpillError::InvalidConfig("stream buffer must be nonzero"));
        }
        if T::SIZE == 0 {
            return Err(SpillError::InvalidConfig("record size must be nonzero"));
        }
        let file = scratch.new_file(stem);
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
            encoded: vec![0; T::SIZE],
            _record: PhantomData,
        })
    }

    pub(crate) fn push(&mut self, record: &T) -> Result<(), SpillError> {
        let records = self.records.checked_add(1).ok_or(SpillError::Overflow)?;
        let bytes = records
            .checked_mul(T::SIZE as u64)
            .ok_or(SpillError::Overflow)?;
        record.encode(&mut self.encoded);
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
            ..
        } = self;
        drop(writer);
        let bytes = records
            .checked_mul(T::SIZE as u64)
            .ok_or(SpillError::Overflow)?;
        file.completed(bytes);
        Ok(Sequence {
            file,
            io,
            buffer_bytes,
            records,
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
    _record: PhantomData<T>,
}

impl<T: Record> Sequence<T> {
    pub(crate) fn len(&self) -> u64 {
        self.records
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
            encoded: vec![0; T::SIZE],
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
            return Err(SpillError::InvalidConfig("sequence range outside its records"));
        }
        let offset = first
            .checked_mul(T::SIZE as u64)
            .ok_or(SpillError::Overflow)?;
        let inner = self
            .io
            .open_at(self.file.path(), offset)
            .map_err(|e| SpillError::io("open spill sequence", self.file.path(), e))?;
        Ok(SequenceReader {
            reader: Some(BufReader::with_capacity(buffer_bytes, inner)),
            sequence: self,
            remaining: end - first,
            encoded: vec![0; T::SIZE],
            ranged: true,
        })
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
        Ok(Some(T::decode(&self.encoded)))
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
