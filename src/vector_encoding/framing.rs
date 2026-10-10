use super::{
    add, align, exact, invalid, limit, mul, CodecError, CompressionReceipt, Encoded,
    GeneratedCounts, Result, ValidatedCompressionLimits, Working,
};
use serde::Serialize;
use std::io::{self, Write};
pub(super) struct Count {
    pub n: usize,
    pub limit: usize,
    pub exceeded: bool,
}
impl Count {
    pub fn new(limit: usize) -> Self {
        Self {
            n: 0,
            limit,
            exceeded: false,
        }
    }
}
impl Write for Count {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        let Some(end) = self.n.checked_add(b.len()).filter(|n| *n <= self.limit) else {
            self.exceeded = true;
            return Err(io::Error::other("JSON count limit"));
        };
        self.n = end;
        Ok(b.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
struct Fixed<'a> {
    bytes: &'a mut Vec<u8>,
    end: usize,
}
impl Write for Fixed<'_> {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        let Some(end) = self
            .bytes
            .len()
            .checked_add(b.len())
            .filter(|n| *n <= self.end && *n <= self.bytes.capacity())
        else {
            return Err(io::Error::other("JSON emission disagreement"));
        };
        self.bytes.extend_from_slice(b);
        debug_assert_eq!(self.bytes.len(), end);
        Ok(b.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
pub(super) fn size(j: usize, u: usize, metadata: bool) -> Result<(usize, usize, usize)> {
    let jpad = if metadata {
        align(add(20, j)?, 8)? - 20
    } else {
        align(j, 4)?
    };
    let upad = align(u, if metadata { 8 } else { 4 })?;
    let total = add(add(28, jpad)?, upad)?;
    if total > u32::MAX as usize {
        return Err(limit("GLB output outside u32 domain"));
    }
    Ok((jpad, upad, total))
}
pub(super) fn emit<T: Serialize, E>(
    doc: &T,
    bin: &[u8],
    j: usize,
    metadata: bool,
) -> std::result::Result<Vec<u8>, CodecError<E>> {
    let (jpad, upad, total) = size(j, bin.len(), metadata)?;
    let mut bytes = exact(total)?;
    bytes.resize(20, 0);
    serde_json::to_writer(
        Fixed {
            bytes: &mut bytes,
            end: 20 + j,
        },
        doc,
    )
    .map_err(|_| CodecError::EncodingFailure("counted JSON emission"))?;
    if bytes.len() != 20 + j {
        return Err(invalid("JSON count/emission disagreement").into());
    }
    bytes.resize(20 + jpad, b' ');
    bytes.extend_from_slice(&(upad as u32).to_le_bytes());
    bytes.extend_from_slice(b"BIN\0");
    bytes.extend_from_slice(bin);
    bytes.resize(total, 0);
    bytes[..4].copy_from_slice(b"glTF");
    bytes[4..8].copy_from_slice(&2u32.to_le_bytes());
    bytes[8..12].copy_from_slice(&(total as u32).to_le_bytes());
    bytes[12..16].copy_from_slice(&(jpad as u32).to_le_bytes());
    bytes[16..20].copy_from_slice(b"JSON");
    Ok(bytes)
}
pub(crate) fn write_generated_glb<T: Serialize, E>(
    document: &T,
    bin: &[u8],
    metadata: bool,
    counts: GeneratedCounts,
    retained_working_bytes: usize,
    limits: &ValidatedCompressionLimits,
    checkpoint: &mut impl FnMut() -> std::result::Result<(), E>,
) -> std::result::Result<Encoded, CodecError<E>> {
    checkpoint().map_err(CodecError::Checkpoint)?;
    let l = limits.values;
    if counts.views > l.buffer_views
        || counts.accessors > l.accessors
        || counts.logical_bytes > l.logical_bytes
    {
        return Err(limit("generated content cardinality/logical limit").into());
    }
    let mut work = Working::new(retained_working_bytes, limits)?;
    let mut count = Count::new(l.json_bytes);
    if serde_json::to_writer(&mut count, document).is_err() {
        return Err(if count.exceeded {
            limit("generated JSON byte limit").into()
        } else {
            CodecError::EncodingFailure("generated JSON count")
        });
    }
    let (jpad, _, total) = size(count.n, bin.len(), metadata)?;
    if jpad > l.json_bytes {
        return Err(limit("generated padded JSON byte limit").into());
    }
    if total > l.source_bytes || total > l.candidate_bytes {
        return Err(limit("generated selected GLB byte limit").into());
    }
    let parser = add(
        add(mul(jpad, 8)?, mul(jpad.min(l.json_value_nodes), 256)?)?,
        65_536,
    )?;
    work.admit(add(total, parser)?)?;
    let bytes = emit(document, bin, count.n, metadata)?;
    {
        let _admitted =
            crate::content_integrity::json::admit(&bytes[20..20 + jpad], limits.json())?;
        // The checked document only borrows bytes; admission's temporary owners
        // have already ended. No Value adapter or codec Plan is constructed.
    }
    checkpoint().map_err(CodecError::Checkpoint)?;
    Ok(Encoded {
        bytes,
        receipt: CompressionReceipt {
            before_bytes: total,
            after_bytes: total,
            views: counts.views,
            raw_views: counts.views,
            accessors: counts.accessors,
            logical_bytes: counts.logical_bytes,
            estimated_peak_bytes: work.peak,
            ..CompressionReceipt::default()
        },
    })
}
