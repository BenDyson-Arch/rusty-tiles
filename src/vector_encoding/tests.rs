use super::*;
use serde_json::{json, Value};
use std::convert::Infallible;
fn frame(doc: &Value, bin: &[u8], metadata: bool) -> Vec<u8> {
    let mut c = framing::Count::new(1_048_576);
    serde_json::to_writer(&mut c, doc).unwrap();
    framing::emit::<_, Infallible>(doc, bin, c.n, metadata).unwrap()
}
fn compress(
    source: &[u8],
    limits: CompressionLimits,
) -> std::result::Result<Encoded, CodecError<Infallible>> {
    let l = limits.validate().unwrap();
    let mut checkpoint = || Ok(());
    encode(
        prepare(source, source.len(), &l, &mut checkpoint)?,
        &mut checkpoint,
    )
}
fn ordinary() -> (Value, Vec<u8>) {
    (
        json!({"asset":{"version":"2.0"},"buffers":[{"byteLength":16}],"bufferViews":[{"buffer":0,"byteLength":16,"byteStride":8}],"accessors":[{"bufferView":0,"componentType":5123,"count":2,"type":"VEC3","normalized":true}]}),
        vec![1, 0, 2, 0, 3, 0, 0x91, 0x92, 4, 0, 5, 0, 6, 0, 0x93, 0x94],
    )
}
fn resource(r: std::result::Result<Encoded, CodecError<Infallible>>) {
    assert!(matches!(
        r,
        Err(CodecError::Format(FormatError::ResourceLimit(_)))
    ));
}
#[test]
fn physical_records_preserve_nonzero_padding_and_second_pass_identity() {
    let (doc, bin) = ordinary();
    let source = frame(&doc, &bin, false);
    let out = compress(&source, CompressionLimits::default()).unwrap();
    let j = u32::from_le_bytes(out.bytes[12..16].try_into().unwrap()) as usize;
    let parsed: Value = serde_json::from_slice(&out.bytes[20..20 + j]).unwrap();
    let e = &parsed["bufferViews"][0]["extensions"]["EXT_meshopt_compression"];
    assert_eq!(e["count"], 2);
    assert_eq!(e["byteStride"], 8);
    let start = 28 + j + e["byteOffset"].as_u64().unwrap() as usize;
    let n = e["byteLength"].as_u64().unwrap() as usize;
    let decoded =
        meshopt::encoding::decode_vertex_buffer::<[u8; 8]>(&out.bytes[start..start + n], 2)
            .unwrap();
    assert_eq!(decoded.into_iter().flatten().collect::<Vec<_>>(), bin);
    let second = compress(&out.bytes, CompressionLimits::default()).unwrap();
    assert_eq!(second.bytes, out.bytes);
    assert_eq!(second.receipt.existing_views, 1);
}
#[test]
fn compatible_short_windows_use_full_physical_count() {
    let (mut doc, mut bin) = ordinary();
    doc["accessors"][0]["count"] = json!(1);
    doc["accessors"].as_array_mut().unwrap().push(
        json!({"bufferView":0,"byteOffset":2,"componentType":5123,"count":1,"type":"SCALAR"}),
    );
    bin[15] = 0xfe;
    let output = compress(&frame(&doc, &bin, false), CompressionLimits::default()).unwrap();
    assert_eq!(output.receipt.compressed_views, 1);
}
#[test]
fn heterogeneous_natural_strides_and_partial_matrix_tail_are_raw_identity() {
    let doc = json!({"asset":{"version":"2.0"},"buffers":[{"byteLength":24}],"bufferViews":[{"buffer":0,"byteLength":24}],"accessors":[{"bufferView":0,"componentType":5126,"count":2,"type":"VEC3"},{"bufferView":0,"componentType":5125,"count":1,"type":"SCALAR"}]});
    let source = frame(&doc, &[0x5a; 24], false);
    assert_eq!(
        compress(&source, CompressionLimits::default())
            .unwrap()
            .bytes,
        source
    );
    let doc = json!({"asset":{"version":"2.0"},"buffers":[{"byteLength":22}],"bufferViews":[{"buffer":0,"byteLength":22}],"accessors":[{"bufferView":0,"componentType":5123,"count":1,"type":"MAT3"}]});
    let source = frame(&doc, &[0xa5; 22], false);
    assert_eq!(
        compress(&source, CompressionLimits::default())
            .unwrap()
            .bytes,
        source
    );
}
#[test]
fn shared_image_metadata_roles_override_dense_stream_selection() {
    let (mut doc, bin) = ordinary();
    doc["images"] = json!([{"bufferView":0,"mimeType":"image/png"}]);
    let source = frame(&doc, &bin, false);
    assert_eq!(
        compress(&source, CompressionLimits::default())
            .unwrap()
            .bytes,
        source
    );
}
#[test]
fn unknown_core_extension_is_unsupported_but_opaque_application_extensions_are_preserved() {
    let (mut doc, bin) = ordinary();
    doc["materials"] = json!([{"extras":{"extensions":{"UNKNOWN":{}}}}]);
    let good = frame(&doc, &bin, false);
    assert!(compress(&good, CompressionLimits::default()).is_ok());
    doc["materials"][0]["extensions"] = json!({"UNKNOWN":{}});
    assert!(matches!(
        compress(&frame(&doc, &bin, false), CompressionLimits::default()),
        Err(CodecError::Format(FormatError::Unsupported(_)))
    ));
}
#[test]
fn malformed_large_consumed_token_reports_finite_reason() {
    let (mut doc, bin) = ordinary();
    doc["accessors"][0]["componentType"] = json!("x".repeat(65536));
    match compress(&frame(&doc, &bin, false), CompressionLimits::default()) {
        Err(CodecError::Format(FormatError::InvalidInput(s))) => assert!(s.len() < 128),
        _ => panic!("wrong typed consumed-token failure"),
    }
}
#[test]
fn logical_cap_and_output_json_caps_precede_native() {
    let (doc, bin) = ordinary();
    let source = frame(&doc, &bin, false);
    let mut l = CompressionLimits {
        logical_bytes: 16,
        ..CompressionLimits::default()
    };
    assert!(compress(&source, l).is_ok());
    l.logical_bytes = 15;
    resource(compress(&source, l));
    let mut l = CompressionLimits {
        json_depth: 4,
        ..CompressionLimits::default()
    };
    resource(compress(&source, l));
    let count = crate::content_integrity::json::admit(
        &source[20..20 + u32::from_le_bytes(source[12..16].try_into().unwrap()) as usize],
        l.validate().unwrap().json(),
    )
    .unwrap()
    .value_nodes();
    assert_eq!(count, 18);
    l.json_depth = 64;
    l.json_value_nodes = count + 19;
    assert!(compress(&source, l).is_ok());
    l.json_value_nodes = count + 18;
    let (out, reached) = with_capacity_one(|| compress(&source, l));
    resource(out);
    assert_eq!(reached, 0);
}
#[test]
fn actual_native_zero_is_encoding_failure_and_checkpoint_keeps_cause() {
    let (doc, bin) = ordinary();
    let source = frame(&doc, &bin, false);
    let (out, reached) = with_capacity_one(|| compress(&source, CompressionLimits::default()));
    assert!(matches!(
        out,
        Err(CodecError::EncodingFailure("ATTRIBUTES encoder"))
    ));
    assert_eq!(reached, 1);
    let mut checkpoint = || Err("original-checkpoint");
    assert!(matches!(
        prepare(
            &source,
            source.len(),
            &CompressionLimits::default().validate().unwrap(),
            &mut checkpoint
        ),
        Err(CodecError::Checkpoint("original-checkpoint"))
    ));
}
#[test]
fn selected_metadata_framing_and_known_overlay_preserve_opaque_tokens() {
    let (mut doc, bin) = ordinary();
    doc["extensionsUsed"] = json!(["EXT_structural_metadata", "KHR_materials_unlit"]);
    doc["extensions"] = json!({"EXT_structural_metadata":{"schema":{"id":"test","classes":{"c":{"properties":{}}},"extras":{"extensions":{"UNKNOWN":{}}}}}});
    doc["materials"] =
        json!([{"extensions":{"KHR_materials_unlit":{}},"extras":{"kept":"raw body"}}]);
    let source = frame(&doc, &bin, true);
    let out = compress(&source, CompressionLimits::default()).unwrap();
    let j = u32::from_le_bytes(out.bytes[12..16].try_into().unwrap()) as usize;
    assert_eq!((20 + j) % 8, 0);
    assert_eq!((28 + j) % 8, 0);
    assert_eq!((out.bytes.len() - 28 - j) % 8, 0);
    let parsed: Value = serde_json::from_slice(&out.bytes[20..20 + j]).unwrap();
    assert_eq!(parsed["extensions"], doc["extensions"]);
    assert_eq!(parsed["materials"], doc["materials"]);
}
#[test]
fn untouched_raw_tokens_keep_large_integer_exponent_and_marker_lexemes() {
    let literal = r#"{"asset":{"version":"2.0"},"buffers":[{"byteLength":16}],"bufferViews":[{"buffer":0,"byteLength":16,"byteStride":8}],"accessors":[{"bufferView":0,"componentType":5123,"count":2,"type":"VEC3","normalized":true}],"extras":{ "big":9007199254740993,"u64":18446744073709551615,"huge":1e400,"fraction":123.4500e-2,"negativeZero":-0,"marker":{"$serde_json::private::Number":"1e400"} }}"#;
    let raw: &serde_json::value::RawValue = serde_json::from_str(literal).unwrap();
    let mut count = framing::Count::new(1_048_576);
    serde_json::to_writer(&mut count, raw).unwrap();
    let source = framing::emit::<_, Infallible>(&raw, &[0; 16], count.n, false).unwrap();
    let output = compress(&source, CompressionLimits::default()).unwrap();
    let needle=b"{ \"big\":9007199254740993,\"u64\":18446744073709551615,\"huge\":1e400,\"fraction\":123.4500e-2,\"negativeZero\":-0,\"marker\":{\"$serde_json::private::Number\":\"1e400\"} }";
    assert!(output.bytes.windows(needle.len()).any(|w| w == needle));
}
#[test]
fn mixed_existing_and_eligible_raw_views_select_whole_identity() {
    let (doc, original) = ordinary();
    let first = compress(&frame(&doc, &original, false), CompressionLimits::default()).unwrap();
    let j = u32::from_le_bytes(first.bytes[12..16].try_into().unwrap()) as usize;
    let mut doc: Value = serde_json::from_slice(&first.bytes[20..20 + j]).unwrap();
    let n = doc["buffers"][0]["byteLength"].as_u64().unwrap() as usize;
    let mut bin = first.bytes[28 + j..28 + j + n].to_vec();
    let offset = bin.len().div_ceil(8) * 8;
    bin.resize(offset, 0);
    bin.extend_from_slice(&original);
    doc["buffers"][0]["byteLength"] = json!(bin.len());
    doc["bufferViews"]
        .as_array_mut()
        .unwrap()
        .push(json!({"buffer":0,"byteOffset":offset,"byteLength":16,"byteStride":8}));
    doc["accessors"].as_array_mut().unwrap().push(
        json!({"bufferView":1,"componentType":5123,"count":2,"type":"VEC3","normalized":true}),
    );
    doc["bufferViews"][0]
        .as_object_mut()
        .unwrap()
        .remove("byteStride");
    doc["accessors"]
        .as_array_mut()
        .unwrap()
        .push(json!({"bufferView":0,"componentType":5123,"count":1,"type":"SCALAR"}));
    let source = frame(&doc, &bin, false);
    let (out, native_encodes) =
        with_capacity_one(|| compress(&source, CompressionLimits::default()));
    let out = out.unwrap();
    assert_eq!(native_encodes, 0);
    assert_eq!(out.bytes, source);
    assert_eq!(out.receipt.existing_views, 1);
    assert_eq!(out.receipt.raw_views, 1);
    assert_eq!(out.receipt.compressed_views, 0);
}

// Scoped facts report requested Vec capacities, not allocator usable bytes or RSS.
#[derive(Clone, Copy)]
struct Reservation {
    kind: &'static str,
    slots: usize,
    bytes: usize,
}
const EMPTY: Reservation = Reservation {
    kind: "",
    slots: 0,
    bytes: 0,
};
#[derive(Clone, Copy)]
struct Owners {
    active: bool,
    reservations: [Reservation; 32],
    len: usize,
    tasks: usize,
    accessors: usize,
    dropped: bool,
    copy_after_drop: bool,
}
const OWNERS: Owners = Owners {
    active: false,
    reservations: [EMPTY; 32],
    len: 0,
    tasks: 0,
    accessors: 0,
    dropped: false,
    copy_after_drop: false,
};
thread_local! {static OWNED:std::cell::RefCell<Owners>=const{std::cell::RefCell::new(OWNERS)};}
pub(super) fn reserved<T>(slots: usize) {
    OWNED.with(|c| {
        let mut o = c.borrow_mut();
        if o.active {
            let i = o.len;
            assert!(i < o.reservations.len());
            o.reservations[i] = Reservation {
                kind: std::any::type_name::<T>(),
                slots,
                bytes: slots * size_of::<T>(),
            };
            o.len += 1;
        }
    })
}
pub(super) fn scan_owners(tasks: usize, accessors: usize) {
    OWNED.with(|c| {
        let mut o = c.borrow_mut();
        if o.active {
            o.tasks = tasks;
            o.accessors = accessors;
        }
    })
}
pub(super) fn scan_dropped() {
    OWNED.with(|c| {
        let mut o = c.borrow_mut();
        if o.active {
            o.dropped = true;
        }
    })
}
pub(super) fn identity_copy() {
    OWNED.with(|c| {
        let mut o = c.borrow_mut();
        if o.active {
            o.copy_after_drop = o.dropped;
        }
    })
}
fn owned<T>(f: impl FnOnce() -> T) -> (T, Owners) {
    struct Restore(Owners);
    impl Drop for Restore {
        fn drop(&mut self) {
            OWNED.with(|c| *c.borrow_mut() = self.0);
        }
    }
    let old = OWNED.with(|c| {
        c.replace(Owners {
            active: true,
            ..OWNERS
        })
    });
    let _restore = Restore(old);
    let out = f();
    (out, OWNED.with(|c| *c.borrow()))
}
#[test]
fn actual_k_slots_and_drop_before_identity_copy() {
    let (mut doc, bin) = ordinary();
    doc["images"] = json!([{"bufferView":0,"mimeType":"image/png"}]);
    let source = frame(&doc, &bin, false);
    let j = u32::from_le_bytes(source[12..16].try_into().unwrap()) as usize;
    let limits = CompressionLimits::default();
    let k = crate::content_integrity::json::admit(
        &source[20..20 + j],
        limits.validate().unwrap().json(),
    )
    .unwrap()
    .value_nodes();
    let (out, o) = owned(|| compress(&source, limits));
    assert_eq!(out.unwrap().bytes, source);
    assert_eq!(o.tasks, k);
    assert_eq!(o.accessors, 1);
    assert!(o.copy_after_drop);
    let sizes = plan::owner_sizes();
    assert_eq!(&sizes[..5], &[40, 32, 144, 1, 32]);
    for (kind, slots, size) in [
        ("raw::Field", k, sizes[0]),
        ("plan::Task", k, sizes[1]),
        ("plan::View", 1, sizes[2]),
        ("plan::Accessor", 1, sizes[3]),
    ] {
        let record = o.reservations[..o.len]
            .iter()
            .find(|r| r.kind.contains(kind))
            .unwrap();
        assert_eq!(record.slots, slots);
        assert_eq!(record.bytes, slots * size);
        assert!(record.slots < limits.json_value_nodes);
    }
    println!("codec owner sizes={sizes:?} actual_k={k} task_slots={} accessor_slots={} dropped={} identity_copy_after_drop={}",o.tasks,o.accessors,o.dropped,o.copy_after_drop);
}
#[test]
fn actual_construction_gate_exact_and_onebelow_before_plan_reservation() {
    let j = 256;
    let k = 18;
    let extra = plan::construction_storage(j, k, 1, 1).unwrap();
    let base = 512;
    let mut limits = CompressionLimits {
        working_bytes: base + finite_error_storage() + extra,
        ..CompressionLimits::default()
    };
    let l = limits.validate().unwrap();
    let mut work = Working::new(base, &l).unwrap();
    let (out, accepted) = owned(|| plan::start_arena(j, k, 1, 1, &mut work));
    assert_eq!(out.unwrap().fields.capacity(), k);
    assert_eq!(accepted.len, 1);
    assert_eq!(accepted.reservations[0].slots, k);
    limits.working_bytes -= 1;
    let l = limits.validate().unwrap();
    let mut work = Working::new(base, &l).unwrap();
    let ((out, native), refused) =
        owned(|| with_capacity_one(|| plan::start_arena(j, k, 1, 1, &mut work)));
    assert!(matches!(out, Err(FormatError::ResourceLimit(_))));
    assert_eq!(refused.len, 0);
    assert_eq!(native, 0);
    println!("codec construction exact={} onebelow={} json_admission=separate plan_reservations={} refused_reservations={}",base+finite_error_storage()+extra,limits.working_bytes,accepted.len,refused.len);
}

#[test]
fn indexed_roles_consume_only_validated_unsigned_scalar_fact() {
    let baseline = json!({
        "asset":{"version":"2.0"},
        "buffers":[{"byteLength":16}],
        "bufferViews":[{"buffer":0,"byteLength":16}],
        "accessors":[
            {"bufferView":0,"componentType":5125,"count":4,"type":"SCALAR"},
            {"componentType":5126,"count":4,"type":"VEC3","min":[0,0,0],"max":[0,0,0]}
        ],
        "meshes":[{"primitives":[{"attributes":{"POSITION":1},"indices":0,"mode":0}]}]
    });
    // Every index is zero, within the four implicit-zero POSITION records.
    for component in [5121, 5123, 5125] {
        let mut doc = baseline.clone();
        doc["accessors"][0]["componentType"] = json!(component);
        assert!(compress(&frame(&doc, &[0; 16], false), CompressionLimits::default()).is_ok());
    }
    for (component, shape) in [(5122, "SCALAR"), (5123, "VEC2")] {
        let mut doc = baseline.clone();
        doc["accessors"][0]["componentType"] = json!(component);
        doc["accessors"][0]["type"] = json!(shape);
        let source = frame(&doc, &[0; 16], false);
        // Both accessor windows are valid; only the actual indices role fails.
        let (out, reached) = with_capacity_one(|| compress(&source, CompressionLimits::default()));
        match out {
            Err(CodecError::Format(FormatError::InvalidInput(reason))) => {
                assert_eq!(reason, "index accessor requires unsigned SCALAR")
            }
            _ => panic!("wrong indexed-role classification"),
        }
        assert_eq!(reached, 0);
    }
}

// Independent published BIN slack and selected framing are separate constraints.
fn metadata_document(doc: &mut Value) {
    doc["extensionsUsed"] = json!(["EXT_structural_metadata"]);
    doc["extensions"] = json!({"EXT_structural_metadata":{"schema":{"id":"tail","classes":{}}}});
}
fn c1(
    bytes: &[u8],
) -> std::result::Result<
    crate::content_integrity::payload::PayloadInspection,
    crate::content_integrity::PayloadError<Infallible>,
> {
    use crate::content_integrity::{
        payload::{inspect, PayloadKind},
        JsonLimits, PayloadLimits,
    };
    inspect(
        PayloadKind::Glb,
        bytes,
        PayloadLimits {
            member_bytes: 1_048_576,
            decoded_bytes: 1_048_576,
            accessor_components: 65_536,
            json: JsonLimits {
                bytes: 1_048_576,
                depth: 64,
                value_nodes: 65_536,
            },
        },
        65_536,
        |_, _| -> std::result::Result<Vec<u8>, Infallible> {
            panic!("embedded source must not resolve")
        },
    )
}
fn glb_parts(bytes: &[u8]) -> (Value, &[u8]) {
    let j = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
    let n = u32::from_le_bytes(bytes[20 + j..24 + j].try_into().unwrap()) as usize;
    assert_eq!(bytes.len(), 28 + j + n);
    (
        serde_json::from_slice(&bytes[20..20 + j]).unwrap(),
        &bytes[28 + j..],
    )
}
#[test]
fn metadata_core_slack_zero_to_three_admits_four_to_seven_refuses() {
    for slack in 0..8 {
        let n = 16 - slack;
        let mut doc = json!({"asset":{"version":"2.0"},"buffers":[{"byteLength":n}],"bufferViews":[{"buffer":0,"byteLength":n}],"accessors":[{"bufferView":0,"componentType":5121,"count":n,"type":"SCALAR"}]});
        metadata_document(&mut doc);
        let source = frame(&doc, &vec![7; n], true);
        let (result, reached) =
            with_capacity_one(|| compress(&source, CompressionLimits::default()));
        assert_eq!(reached, 0);
        if slack <= 3 {
            assert_eq!(result.unwrap().bytes, source);
            c1(&source).unwrap();
        } else {
            assert!(
                matches!(result, Err(CodecError::Format(FormatError::InvalidInput(ref reason))) if reason == "BIN padding outside selected profile")
            );
            assert!(matches!(
                c1(&source),
                Err(crate::content_integrity::PayloadError::Format(
                    FormatError::InvalidInput(_)
                ))
            ));
            // Identical view/window, with zero tail declared as unused buffer data.
            doc["buffers"][0]["byteLength"] = json!(16);
            let mut bin = vec![7; n];
            bin.resize(16, 0);
            let declared = frame(&doc, &bin, true);
            assert_eq!(
                compress(&declared, CompressionLimits::default())
                    .unwrap()
                    .bytes,
                declared
            );
            c1(&declared).unwrap();
        }
    }
}
fn metadata_with_raw_tail(n: usize) -> Vec<u8> {
    let (mut doc, mut bin) = ordinary();
    metadata_document(&mut doc);
    doc["bufferViews"]
        .as_array_mut()
        .unwrap()
        .push(json!({"buffer":0,"byteOffset":16,"byteLength":n}));
    doc["accessors"]
        .as_array_mut()
        .unwrap()
        .push(json!({"bufferView":1,"componentType":5121,"count":n,"type":"SCALAR"}));
    bin.extend_from_slice(&vec![7; n]);
    let declared = bin.len().div_ceil(8) * 8;
    bin.resize(declared, 0);
    doc["buffers"][0]["byteLength"] = json!(declared);
    frame(&doc, &bin, true)
}
fn assert_declared_metadata_tail(source: &[u8], n: usize) -> (Value, Vec<u8>) {
    c1(source).unwrap();
    let output = compress(source, CompressionLimits::default()).unwrap();
    assert_eq!(output.receipt.compressed_views, 1);
    assert_eq!(output.receipt.raw_views, 1);
    let (doc, bin) = glb_parts(&output.bytes);
    assert_eq!(
        doc["buffers"][0]["byteLength"].as_u64().unwrap() as usize,
        bin.len()
    );
    assert_eq!(bin.len() % 8, 0);
    let view = &doc["bufferViews"][1];
    let offset = view["byteOffset"].as_u64().unwrap() as usize;
    assert_eq!(offset % 8, 0);
    assert_eq!(view["byteLength"].as_u64().unwrap() as usize, n);
    assert_eq!(&bin[offset..offset + n], vec![7; n]);
    assert!(bin[offset + n..].iter().all(|b| *b == 0));
    c1(&output.bytes).unwrap();
    let second = compress(&output.bytes, CompressionLimits::default()).unwrap();
    assert_eq!(second.bytes, output.bytes);
    assert_eq!(second.receipt.existing_views, 1);
    assert_eq!(second.receipt.raw_views, 1);
    assert_eq!(second.receipt.compressed_views, 0);
    (doc, output.bytes)
}
#[test]
fn metadata_repack_residues_four_to_seven_declare_emitted_bin_extent() {
    for n in 12..16 {
        let source = metadata_with_raw_tail(n);
        let (doc, _) = assert_declared_metadata_tail(&source, n);
        let end = doc["bufferViews"][1]["byteOffset"].as_u64().unwrap() as usize + n;
        assert_eq!(end % 8, n % 8);
    }
}
#[test]
fn metadata_aligned_declaration_recounts_decimal_digit_transition() {
    let (mut doc, bin) = ordinary();
    metadata_document(&mut doc);
    let seed = compress(&frame(&doc, &bin, true), CompressionLimits::default()).unwrap();
    let (seed_doc, _) = glb_parts(&seed.bytes);
    let encoded = seed_doc["bufferViews"][0]["extensions"]["EXT_meshopt_compression"]["byteLength"]
        .as_u64()
        .unwrap() as usize;
    // Native length selects only the fixture tail; the final 99 -> 104 facts
    // below are independently asserted, never supplied as target results.
    let tail_start = encoded.div_ceil(8) * 8;
    assert!(
        tail_start < 97,
        "seed must reach the selected decimal boundary"
    );
    let n = 99 - tail_start;
    let source = metadata_with_raw_tail(n);
    let (doc, output) = assert_declared_metadata_tail(&source, n);
    assert_eq!(
        doc["bufferViews"][1]["byteOffset"].as_u64().unwrap() as usize,
        tail_start
    );
    assert_eq!(tail_start + n, 99);
    assert_eq!(doc["buffers"][0]["byteLength"], 104);
    assert_eq!(glb_parts(&output).1.len(), 104);
}
