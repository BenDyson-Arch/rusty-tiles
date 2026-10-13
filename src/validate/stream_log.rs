//! The phase-2 stream log (VS-B-4): the in-domain traversal's tile registrations and passing
//! references, in traversal order, streamed only after whole-archive admission.
//!
//! The precheck needs only the current path, so the traversal keeps no tile or reference
//! table. The stream, which must wait for whole-archive C1, exclusion and precheck admission,
//! reads this log instead: one record per tile and per passing reference, written only while
//! the archive is in domain and dropped as soon as it leaves it. Records stay in memory up to
//! [`CatalogConfig::stream_log_resident_bytes`], then go to an append-only scratch file read
//! back once, in order.
use super::containment::VolumeFacts;
use super::{spill_failure, Error, TileFact};
use crate::archive3tz::CatalogConfig;
use crate::spill::{ByteFile, ByteWriter, Scratch};
use std::io::Read;

/// One logged event.
#[derive(Clone, Copy, Debug, PartialEq)]
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

/// Reads entries back; any malformed byte is corrupt scratch.
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
    /// The next entry, or `None` at a clean end.
    fn next(&mut self) -> Result<Option<Entry>, Error> {
        let mut tag = [0u8; 1];
        match self.inner.read(&mut tag) {
            Ok(0) => return Ok(None),
            Ok(_) => {}
            Err(_) => return Err(Self::corrupt()),
        }
        Ok(Some(match tag[0] {
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
        }))
    }
}

/// The log being written.
pub(super) struct StreamLog {
    buffer: Vec<u8>,
    file: Option<ByteWriter>,
    resident: usize,
    buffer_bytes: usize,
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
        }
    }

    pub(super) fn push(&mut self, entry: &Entry, scratch: &Scratch) -> Result<(), Error> {
        self.buffer
            .try_reserve(1 + 8 + 8 + 2 + 129 + 2 * 97)
            .map_err(|_| super::refused())?;
        encode(entry, &mut self.buffer);
        if self.buffer.len() > self.resident {
            let file = match &mut self.file {
                Some(file) => file,
                None => self.file.insert(
                    ByteWriter::new(scratch, "stream-log", self.buffer_bytes)
                        .map_err(spill_failure)?,
                ),
            };
            file.append(&self.buffer).map_err(spill_failure)?;
            self.buffer.clear();
        }
        Ok(())
    }

    pub(super) fn finish(self) -> Result<Written, Error> {
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
        let mut drain = |inner: &mut dyn Read| -> Result<(), Error> {
            let mut reader = Reader { inner };
            while let Some(entry) = reader.next()? {
                each(entry)?;
            }
            Ok(())
        };
        match self {
            Written::Resident(bytes) => drain(&mut bytes.as_slice()),
            Written::Spilled(file, buffer) => {
                let mut reader = file.reader(*buffer).map_err(spill_failure)?;
                drain(&mut reader)
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
            let written = log.finish().unwrap();
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

    #[test]
    fn a_truncated_or_unknown_record_is_corrupt_scratch() {
        let mut bytes = Vec::new();
        entries().iter().for_each(|e| encode(e, &mut bytes));
        for cut in [1, 5, bytes.len() - 1] {
            let written = Written::Resident(bytes[..cut].to_vec());
            assert!(matches!(
                written.for_each(|_| Ok(())),
                Err(Error::InvalidState(_))
            ));
        }
        let written = Written::Resident(vec![9]);
        assert!(matches!(
            written.for_each(|_| Ok(())),
            Err(Error::InvalidState(_))
        ));
    }
}
