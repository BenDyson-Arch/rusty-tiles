//! The block codec of compressed spill files: LZ4 block format (lz4_flex, pure Rust), with a
//! stored (uncompressed) fallback for a block that does not shrink, so a stored block is never
//! larger than its records. Lossless: a block decodes to exactly the bytes encoded.
use std::io;

/// One encoded block: its bytes on disk and whether they are the raw records.
pub(crate) struct Encoded {
    pub bytes: Vec<u8>,
    pub stored: bool,
}

/// Encodes `raw`, reusing `scratch` as the compression buffer.
pub(crate) fn encode(raw: &[u8], scratch: &mut Vec<u8>) -> Encoded {
    scratch.resize(lz4_flex::block::get_maximum_output_size(raw.len()), 0);
    match lz4_flex::block::compress_into(raw, scratch) {
        Ok(n) if n < raw.len() => Encoded {
            bytes: scratch[..n].to_vec(),
            stored: false,
        },
        _ => Encoded {
            bytes: raw.to_vec(),
            stored: true,
        },
    }
}

/// Decodes one block of `out.len()` raw bytes.
pub(crate) fn decode(block: &[u8], stored: bool, out: &mut [u8]) -> io::Result<()> {
    if stored {
        if block.len() != out.len() {
            return Err(corrupt("stored block length differs from its records"));
        }
        out.copy_from_slice(block);
        return Ok(());
    }
    match lz4_flex::block::decompress_into(block, out) {
        Ok(n) if n == out.len() => Ok(()),
        Ok(_) => Err(corrupt("compressed block decodes short")),
        Err(error) => Err(corrupt(&format!("compressed block: {error}"))),
    }
}

fn corrupt(detail: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, detail.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_round_trip_and_incompressible_blocks_are_stored() {
        let mut scratch = Vec::new();
        let repetitive: Vec<u8> = (0..65_536u32).map(|i| (i / 97) as u8).collect();
        let e = encode(&repetitive, &mut scratch);
        assert!(!e.stored && e.bytes.len() < repetitive.len());
        let mut out = vec![0; repetitive.len()];
        decode(&e.bytes, e.stored, &mut out).unwrap();
        assert_eq!(out, repetitive);
        // A xorshift stream does not compress: it is stored as is, never larger.
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        let noise: Vec<u8> = (0..4096)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                state as u8
            })
            .collect();
        let e = encode(&noise, &mut scratch);
        assert!(e.stored && e.bytes == noise);
        let mut out = vec![0; noise.len()];
        decode(&e.bytes, e.stored, &mut out).unwrap();
        assert_eq!(out, noise);
        assert!(decode(&e.bytes[1..], true, &mut out).is_err());
    }

    #[test]
    fn corrupt_compressed_blocks_are_typed_errors() {
        let mut scratch = Vec::new();
        let raw = vec![7u8; 10_000];
        let e = encode(&raw, &mut scratch);
        assert!(!e.stored);
        let mut out = vec![0; raw.len()];
        let mut bad = e.bytes.clone();
        bad.truncate(bad.len() / 2);
        let error = decode(&bad, false, &mut out).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        // Fewer raw bytes than the block holds is corrupt too.
        let mut short = vec![0; raw.len() + 1];
        assert!(decode(&e.bytes, false, &mut short).is_err());
    }
}
