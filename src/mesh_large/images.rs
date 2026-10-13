//! Owned encoded-image snapshots. Pixel inspection and decoding have a separate owner.
//! Source reads finish before return; each image's sequence is scratch-accounted and
//! logical identities remain distinct even when confined resource paths alias.
use super::input::StreamSource;
use crate::{
    spill::{Record, Scratch, Sequence, SequenceWriter},
    JobError, JobErrorKind,
};
use serde::Serialize;
use sha2::{Digest, Sha256};

type Result<T> = std::result::Result<T, JobError>;
pub(crate) const MAX_ENCODED_IMAGE_BYTES: u64 = 32 * 1024 * 1024;
const PAYLOAD: usize = 4094;
fn invalid(message: &'static str) -> JobError {
    JobError::new(JobErrorKind::InvalidInput, message)
}
fn overflow() -> JobError {
    JobError::new(
        JobErrorKind::ResourceLimit,
        "image snapshot arithmetic overflow",
    )
}
fn admitted_length(length: u64) -> Result<usize> {
    if length == 0 {
        return Err(invalid("encoded image is empty"));
    }
    if length > MAX_ENCODED_IMAGE_BYTES {
        return Err(JobError::new(
            JobErrorKind::Unsupported,
            "encoded image exceeds 32 MiB admission ceiling",
        ));
    }
    usize::try_from(length).map_err(|_| overflow())
}
fn chunk_count(length: u64) -> Result<u64> {
    length
        .checked_add(PAYLOAD as u64 - 1)
        .ok_or_else(overflow)
        .map(|v| v / PAYLOAD as u64)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct ImageChunk {
    pub used: u16,
    pub payload: [u8; PAYLOAD],
}
impl Record for ImageChunk {
    const SIZE: usize = 4096;
    fn encode(&self, out: &mut [u8]) {
        out[..2].copy_from_slice(&self.used.to_le_bytes());
        out[2..].copy_from_slice(&self.payload);
    }
    fn decode(bytes: &[u8]) -> Self {
        Self {
            used: u16::from_le_bytes(bytes[..2].try_into().unwrap()),
            payload: bytes[2..].try_into().unwrap(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct ImageIdentity {
    pub index: u32,
    pub encoded_bytes: u64,
    pub sha256: [u8; 32],
    pub mime: Option<String>,
}
pub(crate) struct ImageSnapshot {
    pub identity: ImageIdentity,
    chunks: Sequence<ImageChunk>,
}
impl ImageSnapshot {
    /// Allocates one admitted encoded image, never all images. Verifies its complete
    /// chunk framing and content hash before passing bytes to the decoder owner.
    pub(crate) fn reconstruct(&self, check: &mut dyn FnMut() -> Result<()>) -> Result<Vec<u8>> {
        check()?;
        let length = admitted_length(self.identity.encoded_bytes)?;
        if self.chunks.len() != chunk_count(self.identity.encoded_bytes)? {
            return Err(invalid(
                "image snapshot chunk count disagrees with encoded length",
            ));
        }
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(length).map_err(|_| {
            JobError::new(
                JobErrorKind::ResourceLimit,
                "cannot allocate encoded image snapshot",
            )
        })?;
        let mut hash = Sha256::new();
        let mut reader = self.chunks.reader()?;
        let mut until_check = 0usize;
        while bytes.len() < length {
            if until_check < PAYLOAD {
                check()?;
                until_check = 64 * 1024;
            }
            let chunk = reader
                .next()
                .transpose()?
                .ok_or_else(|| invalid("truncated image snapshot"))?;
            let expected = (length - bytes.len()).min(PAYLOAD);
            if usize::from(chunk.used) != expected {
                return Err(invalid(
                    "image snapshot chunk length disagrees with encoded length",
                ));
            }
            if chunk.payload[expected..].iter().any(|v| *v != 0) {
                return Err(invalid("image snapshot chunk has nonzero padding"));
            }
            hash.update(&chunk.payload[..expected]);
            bytes.extend_from_slice(&chunk.payload[..expected]);
            until_check -= expected;
        }
        if reader.next().transpose()?.is_some() {
            return Err(invalid("extra image snapshot chunk"));
        }
        let digest: [u8; 32] = hash.finalize().into();
        if digest != self.identity.sha256 {
            return Err(invalid(
                "image snapshot SHA-256 disagrees with captured identity",
            ));
        }
        check()?;
        Ok(bytes)
    }
}

/// These explicit charges describe encoded snapshot work only; decoded rasters and
/// encoder storage must be added by their owners. Metadata is bounded by image count.
#[derive(Clone, Copy, Debug, Serialize)]
pub(crate) struct ImageCaptureBounds {
    pub images: u64,
    pub encoded_bytes: u64,
    pub scratch_bytes: u64,
    pub metadata_bytes: u64,
    pub capture_resident_bytes: u64,
    pub reconstruct_resident_bytes: u64,
}

/// Capture every declared image serially. A failure drops completed snapshots and the
/// unfinished writer, removing their scratch files. Source atomicity is not promised.
pub(crate) fn capture_images(
    source: &StreamSource,
    scratch: &Scratch,
    buffer_bytes: usize,
    check: &mut dyn FnMut() -> Result<()>,
) -> Result<(Vec<ImageSnapshot>, ImageCaptureBounds)> {
    check()?;
    let count = source.document.image_sources().len();
    let mut snapshots = Vec::new();
    snapshots.try_reserve_exact(count).map_err(|_| {
        JobError::new(
            JobErrorKind::ResourceLimit,
            "cannot allocate image snapshot metadata",
        )
    })?;
    let buffer = u64::try_from(buffer_bytes.max(ImageChunk::SIZE)).map_err(|_| overflow())?;
    let chunks_live = (ImageChunk::SIZE as u64)
        .checked_mul(2)
        .ok_or_else(overflow)?;
    let path_charge = u64::try_from(scratch.path().as_os_str().len())
        .map_err(|_| overflow())?
        .checked_add(64)
        .and_then(|v| v.checked_mul(2))
        .ok_or_else(overflow)?;
    let per_metadata = (std::mem::size_of::<ImageSnapshot>() as u64)
        .checked_add(10)
        .and_then(|v| v.checked_add(path_charge))
        .ok_or_else(overflow)?;
    let metadata_bytes = u64::try_from(snapshots.capacity())
        .map_err(|_| overflow())?
        .checked_mul(per_metadata)
        .ok_or_else(overflow)?;
    let mut bounds = ImageCaptureBounds {
        images: count as u64,
        encoded_bytes: 0,
        scratch_bytes: 0,
        metadata_bytes,
        capture_resident_bytes: metadata_bytes
            .checked_add(buffer)
            .and_then(|v| v.checked_add(chunks_live))
            .ok_or_else(overflow)?,
        reconstruct_resident_bytes: metadata_bytes
            .checked_add(buffer)
            .and_then(|v| v.checked_add(chunks_live))
            .and_then(|v| v.checked_add(MAX_ENCODED_IMAGE_BYTES))
            .ok_or_else(overflow)?,
    };
    for index in 0..count {
        check()?;
        let length = source.image_len(index)?;
        admitted_length(length)?;
        let chunks = chunk_count(length)?;
        bounds.encoded_bytes = bounds
            .encoded_bytes
            .checked_add(length)
            .ok_or_else(overflow)?;
        bounds.scratch_bytes = bounds
            .scratch_bytes
            .checked_add(
                chunks
                    .checked_mul(ImageChunk::SIZE as u64)
                    .ok_or_else(overflow)?,
            )
            .ok_or_else(overflow)?;
        let mut writer = SequenceWriter::<ImageChunk>::new(scratch, "image", buffer_bytes)?;
        let mut hash = Sha256::new();
        let mut offset = 0u64;
        let mut until_check = 0usize;
        while offset < length {
            if until_check < PAYLOAD {
                check()?;
                until_check = 64 * 1024;
            }
            let used =
                usize::try_from((length - offset).min(PAYLOAD as u64)).map_err(|_| overflow())?;
            let mut chunk = ImageChunk {
                used: u16::try_from(used).map_err(|_| overflow())?,
                payload: [0; PAYLOAD],
            };
            source.read_image_chunk(index, offset, &mut chunk.payload[..used])?;
            hash.update(&chunk.payload[..used]);
            writer.push(&chunk)?;
            offset = offset.checked_add(used as u64).ok_or_else(overflow)?;
            until_check -= used;
        }
        if writer.len() != chunks {
            return Err(invalid(
                "captured image chunk count disagrees with encoded length",
            ));
        }
        let mime = source.document.image_view(index)?.mime.map(str::to_owned);
        snapshots.push(ImageSnapshot {
            identity: ImageIdentity {
                index: u32::try_from(index).map_err(|_| overflow())?,
                encoded_bytes: length,
                sha256: hash.finalize().into(),
                mime,
            },
            chunks: writer.finish()?,
        });
    }
    source.verify_unchanged()?;
    check()?;
    Ok((snapshots, bounds))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;

    // Analytic 1x1 PNG fixture: no image encoding runs during snapshot tests.
    const PNG: &[u8] = &[
        137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 4,
        0, 0, 0, 181, 28, 12, 2, 0, 0, 0, 11, 73, 68, 65, 84, 120, 218, 99, 100, 248, 15, 0, 1, 5,
        1, 1, 39, 24, 227, 102, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
    ];
    fn fixture(encoded: &[u8]) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("scene.gltf");
        let mut bytes: Vec<_> = [0f32, 0., 0., 1., 0., 0., 0., 1., 0.]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        bytes.extend_from_slice(encoded);
        let doc = json!({"asset":{"version":"2.0"},"buffers":[{"uri":"data.bin","byteLength":bytes.len()}],"bufferViews":[{"buffer":0,"byteLength":36},{"buffer":0,"byteOffset":36,"byteLength":encoded.len()}],"accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]}],"images":[{"bufferView":1,"mimeType":"image/png"},{"uri":"image.png","mimeType":"image/png"},{"uri":"image.png"}],"meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}],"nodes":[{"mesh":0}],"scenes":[{"nodes":[0]}]});
        fs::write(&input, serde_json::to_vec(&doc).unwrap()).unwrap();
        fs::write(dir.path().join("data.bin"), bytes).unwrap();
        fs::write(dir.path().join("image.png"), encoded).unwrap();
        (dir, input)
    }
    #[test]
    fn buffer_and_shared_external_images_are_byte_exact_after_source_removal() {
        let (dir, input) = fixture(PNG);
        let source = StreamSource::open(&input, &mut || Ok(())).unwrap();
        let scratch = Scratch::create(Some(dir.path()), 0).unwrap();
        let (snapshots, bounds) = capture_images(&source, &scratch, 16, &mut || Ok(())).unwrap();
        assert_eq!(bounds.images, 3);
        assert_eq!(bounds.encoded_bytes, 3 * PNG.len() as u64);
        assert_eq!(bounds.scratch_bytes, 3 * ImageChunk::SIZE as u64);
        assert!(bounds.capture_resident_bytes >= 3 * ImageChunk::SIZE as u64);
        drop(source);
        fs::remove_file(&input).unwrap();
        fs::remove_file(dir.path().join("data.bin")).unwrap();
        fs::remove_file(dir.path().join("image.png")).unwrap();
        let expected: [u8; 32] = Sha256::digest(PNG).into();
        for (index, image) in snapshots.iter().enumerate() {
            assert_eq!(image.identity.index, index as u32);
            assert_eq!(image.identity.sha256, expected);
            let encoded = image.reconstruct(&mut || Ok(())).unwrap();
            assert_eq!(encoded, PNG);
            let (info, rgba) = crate::gltf_source::source::decode_image(
                &encoded,
                image.identity.mime.as_deref(),
                &mut || Ok(()),
            )
            .unwrap();
            assert_eq!(info.mime_type, "image/png");
            assert_eq!(info.extension, "png");
            assert_eq!((info.width, info.height), (1, 1));
            // The analytic PNG scanline is [filter Sub, luma 0, alpha 255].
            assert_eq!(rgba, [0, 0, 0, 255]);
            assert_eq!(
                image.identity.mime.as_deref(),
                if index == 2 { None } else { Some("image/png") }
            );
        }
        drop(snapshots);
        assert_eq!(fs::read_dir(scratch.path()).unwrap().count(), 0);
    }
    #[test]
    fn shared_decoder_refuses_wrong_mime_corruption_trailing_bytes_and_cancellation() {
        use crate::gltf_source::source::decode_image;
        let wrong_mime = decode_image(PNG, Some("image/jpeg"), &mut || Ok(())).unwrap_err();
        assert_eq!(wrong_mime.kind(), JobErrorKind::InvalidInput);
        let mut crc_flip = PNG.to_vec();
        crc_flip[29] ^= 1; // IHDR CRC, leaving the analytic pixel stream untouched.
        assert_eq!(
            decode_image(&crc_flip, Some("image/png"), &mut || Ok(()))
                .unwrap_err()
                .kind(),
            JobErrorKind::InvalidInput
        );
        let mut trailing = PNG.to_vec();
        trailing.push(0);
        assert_eq!(
            decode_image(&trailing, Some("image/png"), &mut || Ok(()))
                .unwrap_err()
                .kind(),
            JobErrorKind::InvalidInput
        );
        let cancelled = decode_image(PNG, Some("image/png"), &mut || {
            Err(JobError::new(
                JobErrorKind::Cancelled,
                "test decoder cancellation",
            ))
        })
        .unwrap_err();
        assert_eq!(cancelled.kind(), JobErrorKind::Cancelled);
        // Independently generated valid 1x1 grayscale+alpha 16-bit PNG, scanline
        // filter None, luma=0, alpha=65535. This source domain stays unsupported.
        const PNG16: &[u8] = &[
            137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1,
            16, 4, 0, 0, 0, 229, 140, 208, 65, 0, 0, 0, 13, 73, 68, 65, 84, 120, 156, 99, 96, 96,
            248, 255, 31, 0, 3, 2, 1, 255, 230, 119, 11, 174, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66,
            96, 130,
        ];
        assert_eq!(
            decode_image(PNG16, Some("image/png"), &mut || Ok(()))
                .unwrap_err()
                .kind(),
            JobErrorKind::Unsupported
        );
    }

    #[test]
    fn encoded_lengths_overflow_and_cancellation_are_typed_and_clean() {
        assert_eq!(
            admitted_length(0).unwrap_err().kind(),
            JobErrorKind::InvalidInput
        );
        assert_eq!(
            admitted_length(MAX_ENCODED_IMAGE_BYTES + 1)
                .unwrap_err()
                .kind(),
            JobErrorKind::Unsupported
        );
        assert_eq!(
            admitted_length(u64::from(u32::MAX) + 1).unwrap_err().kind(),
            JobErrorKind::Unsupported
        );
        assert_eq!(
            chunk_count(u64::MAX).unwrap_err().kind(),
            JobErrorKind::ResourceLimit
        );
        let (dir, input) = fixture(&vec![7; 70 * 1024]);
        let source = StreamSource::open(&input, &mut || Ok(())).unwrap();
        let scratch = Scratch::create(Some(dir.path()), 0).unwrap();
        let mut polls = 0;
        let error = match capture_images(&source, &scratch, 16, &mut || {
            polls += 1;
            if polls == 4 {
                Err(JobError::new(
                    JobErrorKind::Cancelled,
                    "test image cancellation",
                ))
            } else {
                Ok(())
            }
        }) {
            Ok(_) => panic!("cancellation control must fire"),
            Err(error) => error,
        };
        assert_eq!(error.kind(), JobErrorKind::Cancelled);
        assert_eq!(fs::read_dir(scratch.path()).unwrap().count(), 0);
        // The capture reached the next <=64KiB poll after writing real chunks.
        assert_eq!(polls, 4);
    }
    #[test]
    fn oversized_external_file_is_refused_before_encoded_allocation() {
        let (dir, input) = fixture(PNG);
        fs::OpenOptions::new()
            .write(true)
            .open(dir.path().join("image.png"))
            .unwrap()
            .set_len(u64::from(u32::MAX) + 1)
            .unwrap();
        let source = StreamSource::open(&input, &mut || Ok(())).unwrap();
        let scratch = Scratch::create(Some(dir.path()), 0).unwrap();
        let error = match capture_images(&source, &scratch, 16, &mut || Ok(())) {
            Ok(_) => panic!("oversized external image must be refused"),
            Err(error) => error,
        };
        assert_eq!(error.kind(), JobErrorKind::Unsupported);
        assert_eq!(fs::read_dir(scratch.path()).unwrap().count(), 0);
    }

    #[test]
    fn snapshot_reconstruction_rejects_bad_framing_and_hash() {
        let dir = tempfile::tempdir().unwrap();
        let scratch = Scratch::create(Some(dir.path()), 0).unwrap();
        for (used, length, extra, bad_hash) in [
            (0, 1, false, false),
            (1, 2, false, false),
            (1, 1, true, false),
            (1, 1, false, true),
        ] {
            let mut writer = SequenceWriter::<ImageChunk>::new(&scratch, "bad-image", 16).unwrap();
            let mut chunk = ImageChunk {
                used,
                payload: [0; PAYLOAD],
            };
            chunk.payload[0] = 9;
            writer.push(&chunk).unwrap();
            if extra {
                writer.push(&chunk).unwrap();
            }
            let image = ImageSnapshot {
                identity: ImageIdentity {
                    index: 0,
                    encoded_bytes: length,
                    sha256: if bad_hash {
                        [0; 32]
                    } else {
                        Sha256::digest([9]).into()
                    },
                    mime: None,
                },
                chunks: writer.finish().unwrap(),
            };
            assert_eq!(
                image.reconstruct(&mut || Ok(())).unwrap_err().kind(),
                JobErrorKind::InvalidInput
            );
        }
        assert_eq!(fs::read_dir(scratch.path()).unwrap().count(), 0);
    }
}
