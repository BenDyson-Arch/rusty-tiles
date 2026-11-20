use super::*;

fn key(image: u32, origin: [i64; 2], size: [u32; 2]) -> CropRequest {
    CropRequest {
        image,
        class: 0,
        drop: 0,
        wrap: [crops::CLAMP, 10497],
        origin,
        width: size[0],
        height: size[1],
        gutter: 2,
        interior: [size[0].saturating_sub(4), size[1].saturating_sub(4)],
    }
}

fn noise(bytes: usize, seed: u64) -> Vec<u8> {
    let mut state = seed | 1;
    (0..bytes)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state as u8
        })
        .collect()
}

#[test]
fn requests_keep_their_crop_and_sort_image_first() {
    let mut keys = vec![
        CropRequest {
            image: u32::MAX,
            class: 3,
            drop: 12,
            wrap: [33648, 10497],
            origin: [i64::MIN, i64::MAX],
            width: u32::MAX,
            height: 1,
            gutter: 8,
            interior: [u32::MAX - 16, 0],
        },
        key(7, [-3, 4096], [64, 32]),
        key(2, [0, 0], [1, 1]),
    ];
    for k in &keys {
        assert_eq!(k.crop().origin, k.origin);
        assert_eq!(k.sampler().wrap_t, k.wrap[1]);
    }
    keys.sort_unstable();
    assert_eq!(
        keys.iter().map(|k| k.image).collect::<Vec<_>>(),
        [2, 7, u32::MAX]
    );
}

#[test]
fn the_predictor_round_trips_every_shape_exactly() {
    for (w, h) in [(1, 1), (3, 7), (64, 1), (1, 64), (128, 128), (61, 37)] {
        let rgba = noise(w * h * 4, (w * 31 + h) as u64);
        assert_eq!(unpredict(predict(&rgba, w, h), w, h), rgba, "{w}x{h}");
        // A smooth ramp leaves small residuals the codec can shrink.
        let ramp: Vec<u8> = (0..w * h * 4).map(|i| (i / (w * 4)) as u8).collect();
        assert_eq!(unpredict(predict(&ramp, w, h), w, h), ramp);
    }
}

#[test]
fn stored_blocks_decode_exactly_and_corruption_is_a_typed_error() {
    let k = key(0, [0, 0], [32, 16]);
    let rgba: Vec<u8> = (0..32 * 16 * 4).map(|i| (i % 7 + i / 128) as u8).collect();
    let mut scratch = Vec::new();
    let block = codec::encode(&predict(&rgba, 32, 16), &mut scratch);
    assert!(!block.stored, "a smooth crop compresses");
    assert_eq!(decode_block(&block.bytes, block.stored, &k).unwrap(), rgba);
    let truncated = &block.bytes[..block.bytes.len() / 2];
    let error = decode_block(truncated, false, &k).unwrap_err();
    assert_eq!(error.kind(), JobErrorKind::InvalidState);
    // A compressed block read as stored, or for a larger crop, is corrupt too.
    assert!(decode_block(&block.bytes, true, &k).is_err());
    assert!(decode_block(&block.bytes, false, &key(0, [0, 0], [32, 17])).is_err());
    // Noise does not shrink: it is stored raw and still round-trips.
    let rough = noise(32 * 16 * 4, 9);
    let block = codec::encode(&predict(&rough, 32, 16), &mut scratch);
    assert!(block.stored);
    assert_eq!(decode_block(&block.bytes, true, &k).unwrap(), rough);
}

#[test]
fn an_absent_crop_is_a_counted_miss() {
    let scratch = crate::spill::Scratch::create(None, 0).unwrap();
    let log = crate::spill::ByteWriter::new(&scratch, "leaf-crops", 4096)
        .unwrap()
        .finish()
        .unwrap();
    let store = CropStore {
        log,
        index: BTreeMap::new(),
        stats: CropStoreStats::default(),
        misses: AtomicU64::new(0),
    };
    let k = key(1, [8, 8], [16, 16]);
    assert!(store.crop(&k).unwrap().is_none());
    assert!(!store.covers(&[k]));
    assert!(store.covers(&[]));
    assert_eq!(store.stats().misses, 2);
}

#[test]
fn an_image_charge_covers_its_decode_chains_and_crops() {
    let info = crate::gltf_source::source::ImageInfo {
        mime_type: "image/jpeg",
        extension: "jpg",
        width: 8192,
        height: 8192,
        encoded_bytes: 15_000_000,
    };
    let texels = 8192u64 * 8192;
    let decode = texels * 16 + 15_000_000;
    assert_eq!(image_charge(&info, 0, 0, 0, None), decode);
    let leaf = texels * 16 * 2 + 8_000_000;
    assert_eq!(
        image_charge(&info, 2, 300_000_000, 4_000_000, None),
        decode + leaf + 300_000_000
    );
    // The coarse phase follows the leaf phase: the larger is held, beside every block.
    let chain = crate::mesh_large::hlod::bake::chain_bytes(8192, 8192);
    let coarse = texels * 4 + chain + 2 * 64 * 64 * 4;
    assert!(coarse < leaf);
    assert_eq!(
        image_charge(&info, 2, 300_000_000, 4_000_000, Some(200_000_000)),
        decode + leaf + 300_000_000 + 200_000_000
    );
    assert_eq!(
        image_charge(&info, 0, 0, 0, Some(200_000_000)),
        decode + coarse + 200_000_000
    );
}
