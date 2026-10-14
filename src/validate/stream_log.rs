//! The phase-2 stream log (VS-B-4): the in-domain traversal's tile registrations and passing
//! references, in traversal order, streamed only after whole-archive admission.
//!
//! The precheck needs only the current path, so the traversal keeps no tile or reference
//! table. The stream, which must wait for whole-archive C1, exclusion and precheck admission,
//! reads this log instead: one record per tile and per passing reference, written only while
//! the archive is in domain and dropped as soon as it leaves it. Records stay in memory up to
//! [`CatalogConfig::stream_log_resident_bytes`], then go to an append-only scratch file read
//! back once, in order.
//!
//! Each record is framed as `u16 length, payload, CRC-32 of the payload`, and the log ends
//! with a trailer frame holding the entry count. A frame whose CRC does not match, an entry
//! count that differs, or a log that ends without its trailer is corrupt scratch.
use super::containment::VolumeFacts;
use super::{spill_failure, Error, TileFact};
use crate::archive3tz::CatalogConfig;
use crate::spill::{ByteFile, ByteWriter, Scratch};
use std::io::Read;

/// One logged event. Entries are encoded and decoded one at a time, never stored together,
/// so the variant size difference costs nothing.
#[derive(Clone, Copy, Debug, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub(super) enum Entry {
    Tile(TileFact),
    Reference {
        tile: usize,
        content: Option<VolumeFacts>,
        rank: u64,
    },
}

fn put_volume(out: &mut Vec<u8>, volume: &Option<VolumeFacts>) {
    match volume {
        None => out.push(0),
        Some(VolumeFacts::Box(b)) => {
            out.push(1);
            b.iter().for_each(|v| out.extend(v.to_le_bytes()));
        }
        Some(VolumeFacts::Sphere(s)) => {
            out.push(2);
            s.iter().for_each(|v| out.extend(v.to_le_bytes()));
        }
        Some(VolumeFacts::Region) => out.push(3),
    }
}

fn encode(entry: &Entry, out: &mut Vec<u8>) {
    match entry {
        Entry::Tile(fact) => {
            out.push(1);
            out.extend(fact.parent.map_or(u64::MAX, |p| p as u64).to_le_bytes());
            out.extend((fact.depth as u64).to_le_bytes());
            out.extend(fact.chain.to_le_bytes());
            match &fact.transform {
                None => out.push(0),
                Some(m) => {
                    out.push(1);
                    m.iter().for_each(|v| out.extend(v.to_le_bytes()));
                }
            }
            put_volume(out, &fact.volume);
            put_volume(out, &fact.external);
        }
        Entry::Reference {
            tile,
            content,
            rank,
        } => {
            out.push(2);
            out.extend((*tile as u64).to_le_bytes());
            out.extend(rank.to_le_bytes());
            put_volume(out, content);
        }
    }
}

/// Payload tag of the trailer frame.
const TRAILER: u8 = 3;
/// Largest framed tile and reference records (`2 + payload + 4`), for the scratch estimate.
pub(super) const TILE_FRAME_MAX: u64 = 2 + (1 + 8 + 8 + 2 + 129 + 2 * 97) + 4;
#[cfg(test)]
pub(super) const REFERENCE_FRAME_MAX: u64 = 2 + (1 + 8 + 8 + 97) + 4;

/// Appends `payload` framed with its length and CRC-32.
fn frame(payload: &[u8], out: &mut Vec<u8>) {
    out.extend((payload.len() as u16).to_le_bytes());
    out.extend_from_slice(payload);
    out.extend(crc32fast::hash(payload).to_le_bytes());
}

/// Reads one entry's fields from a frame's payload; any malformed byte is corrupt scratch.
struct Reader<R: Read> {
    inner: R,
}

impl<R: Read> Reader<R> {
    fn corrupt() -> Error {
        Error::InvalidState("validation scratch: phase-2 stream log is corrupt".into())
    }
    fn bytes<const N: usize>(&mut self) -> Result<[u8; N], Error> {
        let mut out = [0u8; N];
        self.inner
            .read_exact(&mut out)
            .map_err(|_| Self::corrupt())?;
        Ok(out)
    }
    fn u64(&mut self) -> Result<u64, Error> {
        Ok(u64::from_le_bytes(self.bytes::<8>()?))
    }
    fn f64s<const N: usize>(&mut self) -> Result<[f64; N], Error> {
        let mut out = [0f64; N];
        for v in &mut out {
            *v = f64::from_le_bytes(self.bytes::<8>()?);
        }
        Ok(out)
    }
    fn volume(&mut self) -> Result<Option<VolumeFacts>, Error> {
        Ok(match self.bytes::<1>()?[0] {
            0 => None,
            1 => Some(VolumeFacts::Box(self.f64s::<12>()?)),
            2 => Some(VolumeFacts::Sphere(self.f64s::<4>()?)),
            3 => Some(VolumeFacts::Region),
            _ => return Err(Self::corrupt()),
        })
    }
    fn index(&mut self) -> Result<usize, Error> {
        usize::try_from(self.u64()?).map_err(|_| Self::corrupt())
    }
    /// The entry of one frame payload, whose tag byte is already read.
    fn entry(&mut self, tag: u8) -> Result<Entry, Error> {
        Ok(match tag {
            1 => {
                let parent = self.u64()?;
                let parent = (parent != u64::MAX)
                    .then(|| usize::try_from(parent).map_err(|_| Self::corrupt()))
                    .transpose()?;
                let depth = self.index()?;
                let chain = u16::from_le_bytes(self.bytes::<2>()?);
                let transform = match self.bytes::<1>()?[0] {
                    0 => None,
                    1 => Some(self.f64s::<16>()?),
                    _ => return Err(Self::corrupt()),
                };
                Entry::Tile(TileFact {
                    transform,
                    volume: self.volume()?,
                    external: self.volume()?,
                    parent,
                    depth,
                    chain,
                    owner: None,
                })
            }
            2 => Entry::Reference {
                tile: self.index()?,
                rank: self.u64()?,
                content: self.volume()?,
            },
            _ => return Err(Self::corrupt()),
        })
    }
}

/// Reads frames until the trailer and checks the entry count.
fn read_frames(
    inner: &mut dyn Read,
    each: &mut dyn FnMut(Entry) -> Result<(), Error>,
) -> Result<(), Error> {
    let corrupt = Reader::<&[u8]>::corrupt;
    let mut entries = 0u64;
    let mut payload = Vec::new();
    loop {
        let mut length = [0u8; 2];
        // A log always ends with its trailer: an end here is a truncation.
        inner.read_exact(&mut length).map_err(|_| corrupt())?;
        payload.resize(usize::from(u16::from_le_bytes(length)), 0);
        let mut crc = [0u8; 4];
        inner.read_exact(&mut payload).map_err(|_| corrupt())?;
        inner.read_exact(&mut crc).map_err(|_| corrupt())?;
        if payload.is_empty() || crc32fast::hash(&payload) != u32::from_le_bytes(crc) {
            return Err(corrupt());
        }
        let mut reader = Reader {
            inner: &payload[1..],
        };
        if payload[0] == TRAILER {
            let count = reader.u64()?;
            let mut extra = [0u8; 1];
            let more = inner.read(&mut extra).map_err(|_| corrupt())?;
            if count != entries || !reader.inner.is_empty() || more != 0 {
                return Err(corrupt());
            }
            return Ok(());
        }
        let entry = reader.entry(payload[0])?;
        if !reader.inner.is_empty() {
            return Err(corrupt());
        }
        entries += 1;
        each(entry)?;
    }
}

/// The log being written.
pub(super) struct StreamLog {
    buffer: Vec<u8>,
    file: Option<ByteWriter>,
    resident: usize,
    buffer_bytes: usize,
    entries: u64,
    /// The framed record being appended.
    record: Vec<u8>,
}

/// A completed log, read once in order.
pub(super) enum Written {
    Resident(Vec<u8>),
    Spilled(ByteFile, usize),
}

impl StreamLog {
    pub(super) fn new(config: &CatalogConfig) -> Self {
        Self {
            buffer: Vec::new(),
            file: None,
            resident: config.stream_log_resident_bytes,
            buffer_bytes: config.buffer_bytes,
            entries: 0,
            record: Vec::new(),
        }
    }

    /// Appends the framed record. The resident buffer never holds or reserves more than
    /// `resident` bytes: a record that would not fit flushes the buffer to scratch first.
    fn append(&mut self, scratch: &Scratch) -> Result<(), Error> {
        let needed = self.buffer.len() + self.record.len();
        if needed > self.resident {
            let file = match &mut self.file {
                Some(file) => file,
                None => self.file.insert(
                    ByteWriter::new(scratch, "stream-log", self.buffer_bytes)
                        .map_err(spill_failure)?,
                ),
            };
            file.append(&self.buffer).map_err(spill_failure)?;
            file.append(&self.record).map_err(spill_failure)?;
            self.buffer.clear();
            return Ok(());
        }
        if needed > self.buffer.capacity() {
            let target = (self.buffer.capacity() * 2).clamp(needed, self.resident);
            self.buffer
                .try_reserve_exact(target - self.buffer.len())
                .map_err(|_| super::refused())?;
        }
        self.buffer.extend_from_slice(&self.record);
        Ok(())
    }

    pub(super) fn push(&mut self, entry: &Entry, scratch: &Scratch) -> Result<(), Error> {
        let mut payload = Vec::with_capacity(TILE_FRAME_MAX as usize);
        encode(entry, &mut payload);
        self.record.clear();
        frame(&payload, &mut self.record);
        self.entries += 1;
        self.append(scratch)
    }

    pub(super) fn finish(mut self, scratch: &Scratch) -> Result<Written, Error> {
        let mut trailer = vec![TRAILER];
        trailer.extend(self.entries.to_le_bytes());
        self.record.clear();
        frame(&trailer, &mut self.record);
        self.append(scratch)?;
        match self.file {
            None => Ok(Written::Resident(self.buffer)),
            Some(mut file) => {
                file.append(&self.buffer).map_err(spill_failure)?;
                Ok(Written::Spilled(
                    file.finish().map_err(spill_failure)?,
                    self.buffer_bytes,
                ))
            }
        }
    }
}

impl Written {
    /// Calls `each` for every entry in order.
    pub(super) fn for_each(
        &self,
        mut each: impl FnMut(Entry) -> Result<(), Error>,
    ) -> Result<(), Error> {
        match self {
            Written::Resident(bytes) => read_frames(&mut bytes.as_slice(), &mut each),
            Written::Spilled(file, buffer) => {
                let mut reader = file.reader(*buffer).map_err(spill_failure)?;
                read_frames(&mut reader, &mut each)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries() -> Vec<Entry> {
        let mut m = [0f64; 16];
        m.iter_mut()
            .enumerate()
            .for_each(|(i, v)| *v = i as f64 * 0.5 - 3.);
        vec![
            Entry::Tile(TileFact {
                transform: None,
                volume: Some(VolumeFacts::Box(std::array::from_fn(|i| i as f64))),
                external: None,
                parent: None,
                depth: 0,
                chain: 0,
                owner: None,
            }),
            Entry::Tile(TileFact {
                transform: Some(m),
                volume: Some(VolumeFacts::Sphere([1., 2., 3., 4.])),
                external: Some(VolumeFacts::Region),
                parent: Some(0),
                depth: 7,
                chain: 0x0102,
                owner: None,
            }),
            Entry::Reference {
                tile: 1,
                content: Some(VolumeFacts::Box([-1.5; 12])),
                rank: u64::MAX - 3,
            },
            Entry::Tile(TileFact {
                transform: None,
                volume: None,
                external: None,
                parent: Some(1),
                depth: 8,
                chain: 1,
                owner: None,
            }),
            Entry::Reference {
                tile: 2,
                content: None,
                rank: 0,
            },
        ]
    }

    #[test]
    fn resident_and_spilled_logs_read_back_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let scratch = Scratch::deferred(crate::spill::Placement::In(dir.path().into()), 0);
        for config in [CatalogConfig::DEFAULT, CatalogConfig::FORCED_SPILL] {
            let mut log = StreamLog::new(&config);
            let input = entries().repeat(40);
            for entry in &input {
                log.push(entry, &scratch).unwrap();
            }
            let written = log.finish(&scratch).unwrap();
            assert_eq!(
                matches!(written, Written::Spilled(..)),
                config == CatalogConfig::FORCED_SPILL
            );
            let mut output = Vec::new();
            written
                .for_each(|entry| {
                    output.push(entry);
                    Ok(())
                })
                .unwrap();
            assert_eq!(output, input);
        }
    }

    /// A complete resident log of `entries()`, with the offsets where each frame ends.
    fn log_bytes() -> (Vec<u8>, Vec<usize>) {
        let dir = tempfile::tempdir().unwrap();
        let scratch = Scratch::deferred(crate::spill::Placement::In(dir.path().into()), 0);
        let mut log = StreamLog::new(&CatalogConfig::DEFAULT);
        let mut ends = Vec::new();
        for entry in entries() {
            log.push(&entry, &scratch).unwrap();
            ends.push(log.buffer.len());
        }
        let Written::Resident(bytes) = log.finish(&scratch).unwrap() else {
            unreachable!()
        };
        (bytes, ends)
    }

    fn corrupt(bytes: Vec<u8>) -> bool {
        matches!(
            Written::Resident(bytes).for_each(|_| Ok(())),
            Err(Error::InvalidState(m)) if m.contains("stream log is corrupt")
        )
    }

    #[test]
    fn a_truncated_altered_or_unterminated_log_is_corrupt_scratch() {
        let (bytes, ends) = log_bytes();
        assert!(!corrupt(bytes.clone()));
        // Truncation at every record boundary and inside records.
        for cut in ends.iter().copied().chain([1, 5, bytes.len() - 1]) {
            assert!(corrupt(bytes[..cut].to_vec()), "cut at {cut}");
        }
        // A flipped bit anywhere.
        for at in 0..bytes.len() {
            let mut flipped = bytes.clone();
            flipped[at] ^= 0x04;
            assert!(corrupt(flipped), "flip at {at}");
        }
        // A whole record removed keeps every frame valid but not the count.
        let mut missing = bytes[..ends[1]].to_vec();
        missing.extend_from_slice(&bytes[ends[2]..]);
        assert!(corrupt(missing));
        // Bytes after the trailer.
        let mut extra = bytes.clone();
        extra.push(0);
        assert!(corrupt(extra));
        // The spilled form checks the same.
        let dir = tempfile::tempdir().unwrap();
        let scratch = Scratch::deferred(crate::spill::Placement::In(dir.path().into()), 0);
        let mut log = StreamLog::new(&CatalogConfig::FORCED_SPILL);
        for entry in entries() {
            log.push(&entry, &scratch).unwrap();
        }
        let written = log.finish(&scratch).unwrap();
        let Written::Spilled(file, _) = &written else {
            unreachable!()
        };
        let path = file.path().to_owned();
        let len = std::fs::metadata(&path).unwrap().len();
        std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(len - 15)
            .unwrap();
        assert!(matches!(
            written.for_each(|_| Ok(())),
            Err(Error::InvalidState(_))
        ));
    }

    #[test]
    fn the_resident_buffer_never_grows_past_its_limit() {
        let dir = tempfile::tempdir().unwrap();
        let scratch = Scratch::deferred(crate::spill::Placement::In(dir.path().into()), 0);
        let config = CatalogConfig {
            stream_log_resident_bytes: 1000,
            ..CatalogConfig::DEFAULT
        };
        let mut log = StreamLog::new(&config);
        for entry in entries().repeat(50) {
            log.push(&entry, &scratch).unwrap();
            assert!(log.buffer.len() <= 1000 && log.buffer.capacity() <= 1000);
        }
        assert!(log.file.is_some());
    }

    #[test]
    fn frames_fit_the_declared_maxima() {
        let mut m = [0f64; 16];
        m[0] = 1.;
        let tile = Entry::Tile(TileFact {
            transform: Some(m),
            volume: Some(VolumeFacts::Box([1.; 12])),
            external: Some(VolumeFacts::Box([2.; 12])),
            parent: Some(1),
            depth: 2,
            chain: 3,
            owner: None,
        });
        let reference = Entry::Reference {
            tile: 1,
            content: Some(VolumeFacts::Box([3.; 12])),
            rank: 9,
        };
        for (entry, max) in [(tile, TILE_FRAME_MAX), (reference, REFERENCE_FRAME_MAX)] {
            let (mut payload, mut framed) = (Vec::new(), Vec::new());
            encode(&entry, &mut payload);
            frame(&payload, &mut framed);
            assert_eq!(framed.len() as u64, max);
        }
        assert_eq!(
            crate::archive3tz::STREAM_LOG_BYTES_PER_MEMBER,
            TILE_FRAME_MAX + REFERENCE_FRAME_MAX
        );
    }
}
