//! Independently authored bounded acceptance inputs/expected outcomes.
//! Root includes this only as a crate test module and owns all compilation.
//! No producer fixtures or production self-report oracle supplies expectations.
use crate::content_integrity::{
    json,
    payload::{self, PayloadKind},
    FormatError, JsonLimits, PayloadError, PayloadLimits,
};
use std::cell::Cell;

fn json_limits(bytes: usize, depth: u64, value_nodes: usize) -> JsonLimits {
    JsonLimits { bytes, depth, value_nodes }
}

fn payload_limits(source_bytes: usize) -> PayloadLimits {
    PayloadLimits {
        member_bytes: source_bytes + 31,
        decoded_bytes: 64,
        accessor_components: 32,
        json: json_limits(8192, 16, 256),
    }
}

fn format_category(error: &FormatError) -> &'static str {
    match error {
        FormatError::InvalidInput(_) => "invalid_input",
        FormatError::Unsupported(_) => "unsupported",
        FormatError::ResourceLimit(_) => "resource_limit",
    }
}

fn payload_category<E>(error: &PayloadError<E>) -> &'static str {
    match error {
        PayloadError::Format(error) => format_category(error),
        PayloadError::Resolver(_) => "resolver",
    }
}

#[test]
fn independent_json_inclusive_boundaries_and_order() {
    let source = br#"{"a":[0]}"#;
    // Nine supplied bytes, root object + array + scalar, root depth zero.
    assert_eq!(source.len(), 9);
    let admitted = json::admit(source, json_limits(9, 2, 3)).unwrap();
    assert_eq!(admitted.value_nodes(), 3);
    assert_eq!(admitted.raw().get().as_bytes(), source);
    for limits in [json_limits(8, 2, 3), json_limits(9, 1, 3), json_limits(9, 2, 2)] {
        assert_eq!(format_category(&json::admit(source, limits).unwrap_err()), "resource_limit");
    }
    assert_eq!(json::admit(b"0", json_limits(1, 0, 1)).unwrap().value_nodes(), 1);
    assert_eq!(format_category(&json::admit(b"0", json_limits(1, 0, 0)).unwrap_err()), "resource_limit");
    // Immediate root duplicate wins over depth-limited first descendant.
    let duplicate = br#"{"a":[[0]],"\u0061":0}"#;
    assert_eq!(format_category(&json::admit(duplicate, json_limits(128, 1, 16)).unwrap_err()), "invalid_input");
    // Immediate sibling node admission wins over invalid descendant Unicode.
    let unicode = br#"{"a":"\ud800","b":0}"#;
    assert_eq!(format_category(&json::admit(unicode, json_limits(128, 8, 2)).unwrap_err()), "resource_limit");
    assert_eq!(format_category(&json::admit(unicode, json_limits(128, 8, 3)).unwrap_err()), "invalid_input");
    let malformed = br#"{"a":1e}"#;
    assert_eq!(format_category(&json::admit(malformed, json_limits(malformed.len() - 1, 8, 16)).unwrap_err()), "resource_limit");
    assert_eq!(format_category(&json::admit(malformed, json_limits(malformed.len(), 8, 16)).unwrap_err()), "invalid_input");
}

#[test]
fn independent_raw_numbers_owned_variants_and_marker_truth() {
    let limits = json_limits(512, 8, 32);
    assert_eq!(json::admit(b"1e400", limits).unwrap().raw().get(), "1e400");
    assert_eq!(format_category(&json::parse(b"1e400", limits).unwrap_err()), "unsupported");
    let integer = json::parse(b"1", limits).unwrap();
    let decimal = json::parse(b"1.0", limits).unwrap();
    assert!(integer.is_u64());
    assert!(decimal.is_f64());
    assert_ne!(integer, decimal, "ordinary numeric variants must not normalize");
    let marker = br#"{"$serde_json::private::Number":"1e400"}"#;
    let object = json::parse(marker, limits).unwrap();
    assert!(object.is_object());
    assert_eq!(object["$serde_json::private::Number"], "1e400");
    assert_eq!(serde_json::to_vec(&object).unwrap(), marker);
    for text in [br#""\ud800""#.as_slice(), br#""\udc00""#.as_slice(), br#""\ud800\u0041""#.as_slice(), br#"{"x":0,"\u0078":0}"#.as_slice()] {
        assert_eq!(format_category(&json::admit(text, limits).unwrap_err()), "invalid_input");
    }
    assert!(json::parse(br#""\ud83d\udc08""#, limits).is_ok());
}

#[test]
fn independent_outer_and_json_payload_bytes_equality() {
    let source = br#"{"asset":{"version":"2.0"}}"#;
    let mut limits = payload_limits(source.len());
    limits.member_bytes = source.len();
    limits.json.bytes = source.len();
    let facts = payload::inspect::<()>(PayloadKind::LocalGltf, source, limits, 0, |_, _| panic!("no resources")).unwrap();
    assert_eq!((facts.accessors_checked, facts.primitives_checked, facts.vertices, facts.elements_checked), (0, 0, 0, 0));
    limits.member_bytes -= 1;
    assert_eq!(payload_category(&payload::inspect::<()>(PayloadKind::LocalGltf, source, limits, 0, |_, _| panic!("outer bytes before callbacks")).unwrap_err()), "resource_limit");
    limits.member_bytes = source.len();
    limits.json.bytes -= 1;
    assert_eq!(payload_category(&payload::inspect::<()>(PayloadKind::LocalGltf, source, limits, 0, |_, _| panic!("JSON bytes before callbacks")).unwrap_err()), "resource_limit");
}

#[test]
fn independent_full_component_plan_before_callback_and_remaining_allowance() {
    // All accessors count, including the unused SCALAR: 2*3+5 = 11.
    let source = br#"{"asset":{"version":"2.0"},"buffers":[{"byteLength":24,"uri":"positions"}],"bufferViews":[{"buffer":0,"byteLength":24}],"accessors":[{"bufferView":0,"componentType":5126,"count":2,"type":"VEC3","min":[0,0,0],"max":[0,0,0]},{"componentType":5121,"count":5,"type":"SCALAR"}],"meshes":[{"primitives":[{"mode":1,"attributes":{"POSITION":0}}]}]}"#;
    let mut limits = payload_limits(source.len());
    limits.accessor_components = 11;
    let calls = Cell::new(0);
    let facts = payload::inspect::<()>(PayloadKind::LocalGltf, source, limits, 11, |uri, _| {
        assert_eq!(uri, "positions");
        calls.set(calls.get() + 1);
        Ok(vec![0; 24])
    }).unwrap();
    assert_eq!((facts.accessors_checked, facts.primitives_checked, facts.vertices, facts.elements_checked, calls.get()), (2, 1, 2, 11, 1));
    for (per_payload, remaining) in [(10, 11), (11, 10)] {
        limits.accessor_components = per_payload;
        assert_eq!(payload_category(&payload::inspect::<()>(PayloadKind::LocalGltf, source, limits, remaining, |_, _| panic!("full component plan before resolver")).unwrap_err()), "resource_limit");
    }
    // Independent simulated successful archive charge: 11+11 equality, then
    // one-over22-12=10 allowance must fail without resetting a nested default.
    limits.accessor_components = 11;
    for remaining in [22, 11] {
        assert_eq!(payload::inspect::<()>(PayloadKind::LocalGltf, source, limits, remaining, |_, _| Ok(vec![0; 24])).unwrap().elements_checked, 11);
    }
    assert_eq!(payload_category(&payload::inspect::<()>(PayloadKind::LocalGltf, source, limits, 10, |_, _| panic!("remaining aggregate allowance" )).unwrap_err()), "resource_limit");
}

#[test]
fn independent_declared_buffer_plan_beats_typed_resolver_failure() {
    for second in [17, 18] {
        let source = format!(r#"{{"asset":{{"version":"2.0"}},"buffers":[{{"byteLength":313,"uri":"first"}},{{"byteLength":{second},"uri":"last"}}]}}"#);
        let mut limits = payload_limits(source.len());
        limits.member_bytes = 330;
        let calls = Cell::new(0);
        let result = payload::inspect(PayloadKind::LocalGltf, source.as_bytes(), limits, 0, |_, _| {
            calls.set(calls.get() + 1);
            Err(17u16)
        });
        if second == 17 {
            assert!(matches!(result, Err(PayloadError::Resolver(17))));
            assert_eq!(calls.get(), 1);
        } else {
            assert_eq!(payload_category(&result.unwrap_err()), "resource_limit");
            assert_eq!(calls.get(), 0);
        }
    }
}

#[test]
fn independent_causal_identity_ignores_spoofing_text_and_io_kind() {
    let source = br#"{"asset":{"version":"2.0"},"buffers":[{"byteLength":1,"uri":"a"}]}"#;
    for kind in [std::io::ErrorKind::InvalidData, std::io::ErrorKind::UnexpectedEof, std::io::ErrorKind::PermissionDenied] {
        let text = "ResourceLimit duplicate key unsupported invalid_input";
        let result = payload::inspect(PayloadKind::LocalGltf, source, payload_limits(source.len()), 0, |_, _| {
            Err(std::io::Error::new(kind, text))
        });
        match result.unwrap_err() {
            PayloadError::Resolver(error) => {
                assert_eq!(error.kind(), kind);
                assert_eq!(error.to_string(), text);
            }
            other => panic!("actual causal Io arm lost: {other:?}"),
        }
    }
    let cause = Box::new(173u32);
    let identity = &*cause as *const u32;
    let mut pending = Some(cause);
    let result = payload::inspect(PayloadKind::LocalGltf, source, payload_limits(source.len()), 0, |_, _| Err(pending.take().unwrap()));
    match result.unwrap_err() {
        PayloadError::Resolver(cause) => assert_eq!(&*cause as *const u32, identity),
        other => panic!("boxed causal payload identity lost: {other:?}"),
    }
}

#[test]
fn independent_actual_remainders_and_repeated_declarations() {
    let source = br#"{"asset":{"version":"2.0"},"buffers":[{"byteLength":1,"uri":"same"},{"byteLength":1,"uri":"same"}],"images":[{"uri":"image"}]}"#;
    let limits = payload_limits(source.len());
    let mut calls = Vec::new();
    let facts = payload::inspect::<()>(PayloadKind::LocalGltf, source, limits, 0, |uri, allowance| {
        calls.push((uri.to_owned(), allowance));
        Ok(vec![0; if calls.len() == 1 { limits.member_bytes - 3 } else if calls.len() == 2 { 2 } else { 1 }])
    }).unwrap();
    assert_eq!(calls, [("same".to_owned(), limits.member_bytes), ("same".to_owned(), 3), ("image".to_owned(), 1)]);
    assert_eq!(facts.elements_checked, 0);
    assert!(facts.not_inspected.contains("image decoding"));
    let mut count = 0;
    let error = payload::inspect::<()>(PayloadKind::LocalGltf, source, limits, 0, |_, _| {
        count += 1;
        Ok(vec![0; if count == 1 { limits.member_bytes - 3 } else if count == 2 { 2 } else { 2 }])
    }).unwrap_err();
    assert_eq!(count, 3);
    assert_eq!(payload_category(&error), "resource_limit");
}

#[test]
fn independent_padded_inline_exact_remainder_and_separate_pools() {
    for (encoded, actual) in [("AA==", 1usize), ("AAA=", 2)] {
        let source = format!(r#"{{"asset":{{"version":"2.0"}},"buffers":[{{"byteLength":1,"uri":"prefix"}},{{"byteLength":{actual},"uri":"data:application/octet-stream;base64,{encoded}"}}],"bufferViews":[{{"buffer":1,"byteLength":{actual}}}]}}"#);
        let mut limits = payload_limits(source.len());
        limits.decoded_bytes = 3; // Conservative individual estimate, not a shared data/view pool.
        let prefix = limits.member_bytes - actual;
        let calls = Cell::new(0);
        let result = payload::inspect::<()>(PayloadKind::LocalGltf, source.as_bytes(), limits, 0, |uri, allowed| {
            assert_eq!(uri, "prefix");
            assert_eq!(allowed, limits.member_bytes);
            calls.set(calls.get() + 1);
            Ok(vec![0; prefix])
        });
        assert!(result.is_ok(), "padded actual remainder {actual}: {result:?}");
        assert_eq!(calls.get(), 1);
        assert_eq!(payload_category(&payload::inspect::<()>(PayloadKind::LocalGltf, source.as_bytes(), limits, 0, |_, _| Ok(vec![0; prefix + 1])).unwrap_err()), "resource_limit");
        limits.decoded_bytes = 2;
        assert_eq!(payload_category(&payload::inspect::<()>(PayloadKind::LocalGltf, source.as_bytes(), limits, 0, |_, _| Ok(vec![0; prefix])).unwrap_err()), "resource_limit");
    }
    let source = br#"{"asset":{"version":"2.0"},"buffers":[{"byteLength":3,"uri":"data:application/gltf-buffer;base64,AAAA"}],"bufferViews":[{"buffer":0,"byteLength":3}]}"#;
    let mut limits = payload_limits(source.len());
    limits.decoded_bytes = 3;
    assert!(payload::inspect::<()>(PayloadKind::LocalGltf, source, limits, 0, |_, _| panic!("inline-only")).is_ok());
}

#[test]
fn independent_overlapping_view_bytes_equality_and_one_over() {
    let source = br#"{"asset":{"version":"2.0"},"buffers":[{"byteLength":8,"uri":"b"}],"bufferViews":[{"buffer":0,"byteLength":7},{"buffer":0,"byteOffset":1,"byteLength":7}]}"#;
    let mut limits = payload_limits(source.len());
    limits.decoded_bytes = 14;
    assert!(payload::inspect::<()>(PayloadKind::LocalGltf, source, limits, 0, |_, _| Ok(vec![0; 8])).is_ok());
    limits.decoded_bytes = 13;
    assert_eq!(payload_category(&payload::inspect::<()>(PayloadKind::LocalGltf, source, limits, 0, |_, _| Ok(vec![0; 8])).unwrap_err()), "resource_limit");
}

#[test]
fn independent_meshopt_real_decoder_admission_and_primary_golden() {
    // The source fixture uses exact85-byte compressed and48-byte expected
    // literal vectors from independently pinned upstream meshoptimizer.
    let source = include_bytes!("fixtures/meshopt-none-placeholder-gltf/tile.gltf");
    let compressed = include_bytes!("../primary_controls/2026-10-10-pinned-reference-verification/golden-compressed.bin");
    let mut limits = payload_limits(source.len());
    limits.decoded_bytes = 48;
    limits.accessor_components = 12;
    let facts = payload::inspect::<()>(PayloadKind::LocalGltf, source, limits, 12, |uri, _| {
        assert_eq!(uri, "compressed.bin");
        Ok(compressed.to_vec())
    }).unwrap();
    assert_eq!((facts.accessors_checked, facts.primitives_checked, facts.vertices, facts.elements_checked), (1, 1, 4, 12));
    // Corrupt compressed bytes pair proves limit rejection before invoking an
    // otherwise failing actual codec. This is a category sensitivity oracle;
    // source review separately proves requested allocation/decode ordering.
    limits.decoded_bytes = 47;
    let error = payload::inspect::<()>(PayloadKind::LocalGltf, source, limits, 12, |_, _| Ok(vec![0; 85])).unwrap_err();
    assert_eq!(payload_category(&error), "resource_limit");
    limits.decoded_bytes = 48;
    let error = payload::inspect::<()>(PayloadKind::LocalGltf, source, limits, 12, |_, _| Ok(vec![0; 85])).unwrap_err();
    assert_eq!(payload_category(&error), "invalid_input");
}
