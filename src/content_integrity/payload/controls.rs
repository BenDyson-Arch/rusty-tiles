//! Sensitive controls for the private owner, including limits not exposed by C1.
use super::*;
use serde_json::json;
#[derive(Debug, PartialEq, Eq)]
struct ResolverCause {
    code: u32,
    message: &'static str,
}
fn limits() -> PayloadLimits {
    PayloadLimits {
        member_bytes: 512,
        decoded_bytes: 64,
        accessor_components: 9,
        json: super::super::JsonLimits {
            bytes: 512,
            depth: 64,
            value_nodes: 128,
        },
    }
}
fn direct(
    bytes: &[u8],
    limits: PayloadLimits,
    remaining: u64,
    resolve: impl FnMut(&str, usize) -> Result<Vec<u8>, ResolverCause>,
) -> Result<PayloadInspection, PayloadError<ResolverCause>> {
    inspect(PayloadKind::LocalGltf, bytes, limits, remaining, resolve)
}
#[test]
fn exact_numeric_work_and_domain_gates_have_distinct_causes() {
    for (token, wanted) in [
        ("3.0", Some(3)),
        ("300e-2", Some(3)),
        ("-0e99999", Some(0)),
        ("18446744073709551615.0", Some(u64::MAX)),
        ("1e400", None),
    ] {
        let admitted = json::admit(token.as_bytes(), limits().json).unwrap();
        assert_eq!(
            numbers::exact(Some(admitted.raw())).unwrap(),
            wanted.map_or(numbers::Unsigned::Overflow, numbers::Unsigned::Native)
        );
    }
    for token in [
        "1.0000000000000001",
        "10000000000000001e-16",
        "18446744073709551614.9",
        "1e-400",
        "-1",
        "null",
        "true",
        "\"3\"",
        "[3]",
        "{}",
    ] {
        let admitted = json::admit(token.as_bytes(), limits().json).unwrap();
        assert!(
            matches!(
                numbers::exact(Some(admitted.raw())),
                Err(FormatError::InvalidInput(_))
            ),
            "{token}"
        );
    }
    let admitted = json::admit(b"1e400", limits().json).unwrap();
    assert!(matches!(
        work(Some(admitted.raw()), 9),
        Err(FormatError::ResourceLimit(_))
    ));
    assert!(matches!(
        domain(Some(admitted.raw()), 9),
        Err(FormatError::InvalidInput(_))
    ));
    for (source, resource) in [
        (
            r#"{"asset":{"version":"2.0"},"accessors":[{"count":1e400,"type":"SCALAR","componentType":5121}]}"#,
            true,
        ),
        (
            r#"{"asset":{"version":"2.0"},"buffers":[{"byteLength":1e400,"uri":"x"}]}"#,
            true,
        ),
        (
            r#"{"asset":{"version":"2.0"},"accessors":[{"count":1,"type":"SCALAR","componentType":1e400}]}"#,
            false,
        ),
        (
            r#"{"asset":{"version":"2.0"},"accessors":[{"count":1,"type":"SCALAR","componentType":5121,"bufferView":1e400}]}"#,
            false,
        ),
    ] {
        let error = direct(source.as_bytes(), limits(), 9, |_, _| {
            panic!("numeric admission before read")
        })
        .unwrap_err();
        assert_eq!(
            matches!(error, PayloadError::Format(FormatError::ResourceLimit(_))),
            resource,
            "{source}: {error:?}"
        );
    }
}
#[test]
fn present_null_never_becomes_a_missing_unsigned_default() {
    let source=br#"{"asset":{"version":"2.0"},"buffers":[{"byteLength":4,"uri":"x"}],"bufferViews":[{"buffer":0,"byteLength":4,"byteOffset":null}]}"#;
    assert!(matches!(
        direct(source, limits(), 9, |_, _| panic!(
            "null admission before resource"
        )),
        Err(PayloadError::Format(FormatError::InvalidInput(_)))
    ));
    let admitted = json::admit(
        br#"{"count":1,"type":"SCALAR","componentType":5121,"byteOffset":null}"#,
        limits().json,
    )
    .unwrap();
    let accessor: records::Accessor<'_> = record(Some(admitted.raw())).unwrap();
    assert!(accessor.offset.is_some());
    assert!(matches!(
        off(accessor.offset),
        Err(FormatError::InvalidInput(_))
    ));
    let admitted = json::admit(
        br#"{"count":1,"type":"SCALAR","componentType":5121}"#,
        limits().json,
    )
    .unwrap();
    let accessor: records::Accessor<'_> = record(Some(admitted.raw())).unwrap();
    assert_eq!(off(accessor.offset).unwrap(), 0);
}
#[test]
fn complete_declared_buffer_plan_precedes_first_callback() {
    for (second, success) in [(256, true), (257, false)] {
        let bytes=serde_json::to_vec(&json!({"asset":{"version":"2.0"},"buffers":[{"byteLength":256,"uri":"a"},{"byteLength":second,"uri":"b"}]})).unwrap();
        let mut calls = 0;
        let result = direct(&bytes, limits(), 9, |_, maximum| {
            calls += 1;
            assert!(maximum >= 256);
            Ok(vec![0; 256])
        });
        assert_eq!(result.is_ok(), success);
        assert_eq!(calls, if success { 2 } else { 0 });
        if !success {
            assert!(matches!(
                result,
                Err(PayloadError::Format(FormatError::ResourceLimit(_)))
            ));
        }
    }
    let bytes=br#"{"asset":{"version":"2.0"},"buffers":[{"byteLength":256,"uri":"a"},{"byteLength":257,"uri":"b"}]}"#;
    assert!(matches!(
        direct(bytes, limits(), 9, |_, _| Err(ResolverCause {
            code: 7,
            message: "must not mask complete plan"
        })),
        Err(PayloadError::Format(FormatError::ResourceLimit(_)))
    ));
}
#[test]
fn complete_component_plan_precedes_callback_and_preserves_causal_error() {
    let bytes=serde_json::to_vec(&json!({"asset":{"version":"2.0"},"buffers":[{"byteLength":36,"uri":"external"}],"bufferViews":[{"buffer":0,"byteLength":36}],"accessors":[{"bufferView":0,"componentType":5126,"type":"VEC3","count":3,"min":[0,0,0],"max":[0,0,0]}],"meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}]})).unwrap();
    let mut calls = 0;
    let facts = direct(&bytes, limits(), 9, |_, _| {
        calls += 1;
        Ok(vec![0; 36])
    })
    .unwrap();
    assert_eq!((facts.elements_checked, calls), (9, 1));
    assert!(matches!(
        direct(&bytes, limits(), 8, |_, _| panic!(
            "one-over must precede resource"
        )),
        Err(PayloadError::Format(FormatError::ResourceLimit(_)))
    ));
    let error = direct(&bytes, limits(), 9, |_, _| {
        Err(ResolverCause {
            code: 41,
            message: "duplicate key ResourceLimit InvalidData",
        })
    })
    .unwrap_err();
    match error {
        PayloadError::Resolver(cause) => assert_eq!(
            cause,
            ResolverCause {
                code: 41,
                message: "duplicate key ResourceLimit InvalidData"
            }
        ),
        other => panic!("causal arm lost: {other:?}"),
    }
}
#[test]
fn resolver_receives_current_actual_remainder_before_each_allocation() {
    let bytes=br#"{"asset":{"version":"2.0"},"buffers":[{"byteLength":1,"uri":"a"},{"byteLength":1,"uri":"b"}]}"#;
    let mut maximums = Vec::new();
    direct(bytes, limits(), 9, |_, maximum| {
        maximums.push(maximum);
        Ok(vec![0; if maximums.len() == 1 { 511 } else { 1 }])
    })
    .unwrap();
    assert_eq!(maximums, [512, 1]);
    let bytes=br#"{"asset":{"version":"2.0"},"buffers":[{"byteLength":1,"uri":"a"}],"images":[{"uri":"image"}]}"#;
    let mut maximums = Vec::new();
    direct(bytes, limits(), 9, |_, maximum| {
        maximums.push(maximum);
        Ok(vec![0; if maximums.len() == 1 { 511 } else { 1 }])
    })
    .unwrap();
    assert_eq!(maximums, [512, 1]);
    assert!(matches!(
        direct(bytes, limits(), 9, |_, _| Ok(vec![0; 513])),
        Err(PayloadError::Format(FormatError::ResourceLimit(_)))
    ));
}
#[test]
fn padded_inline_actual_remainder_and_individual_order_are_sensitive() {
    for (encoded, allowance, expected) in [("AA==", 1, vec![0]), ("AAA=", 2, vec![0, 0])] {
        let uri = format!("data:application/octet-stream;base64,{encoded}");
        assert_eq!(decode_buffer_uri(&uri, 3, allowance).unwrap(), expected);
        assert!(matches!(
            decode_buffer_uri(&uri, 3, allowance - 1),
            Err(FormatError::ResourceLimit(_))
        ));
        assert!(matches!(
            decode_buffer_uri(&uri, 2, allowance),
            Err(FormatError::ResourceLimit(_))
        ));
    }
    for encoded in ["AB==", "AAB=", "AA=A", "A===", "A?==", "AAA"] {
        let uri = format!("data:application/gltf-buffer;base64,{encoded}");
        assert!(
            matches!(
                decode_buffer_uri(&uri, 6, 0),
                Err(FormatError::InvalidInput(_))
            ),
            "{encoded}"
        );
    }
    assert!(matches!(
        decode_buffer_uri("data:application/octet-stream;base64,????", 2, 0),
        Err(FormatError::ResourceLimit(_))
    ));
}
#[test]
fn aliases_charge_logical_views_once_and_keep_actual_resource_pool_separate() {
    let bytes=br#"{"asset":{"version":"2.0"},"buffers":[{"byteLength":4,"uri":"x"}],"bufferViews":[{"buffer":0,"byteLength":4},{"buffer":0,"byteLength":4}]}"#;
    let mut selected = limits();
    selected.decoded_bytes = 8;
    assert!(direct(bytes, selected, 9, |_, maximum| {
        assert_eq!(maximum, 512);
        Ok(vec![0; 4])
    })
    .is_ok());
    selected.decoded_bytes = 7;
    assert!(matches!(
        direct(bytes, selected, 9, |_, _| Ok(vec![0; 4])),
        Err(PayloadError::Format(FormatError::ResourceLimit(_)))
    ));
    let inline=br#"{"asset":{"version":"2.0"},"buffers":[{"byteLength":3,"uri":"data:application/octet-stream;base64,AAAA"}],"bufferViews":[{"buffer":0,"byteLength":3}]}"#;
    selected.decoded_bytes = 3;
    assert!(direct(inline, selected, 9, |_, _| panic!("inline")).is_ok());
}
#[test]
fn meshopt_partial_word_copy_never_grows_past_exact_request() {
    for length in [2usize, 6, 10] {
        let words = vec![u32::from_ne_bytes([1, 2, 3, 4]); length.div_ceil(4)];
        let out = crate::content_integrity::meshopt::copy_words(&words, length).unwrap();
        assert_eq!(out.len(), length);
        assert_eq!(
            out.capacity(),
            length,
            "observed exact request must not be grown by complete-word copy"
        );
        assert_eq!(
            out,
            vec![1, 2, 3, 4]
                .into_iter()
                .cycle()
                .take(length)
                .collect::<Vec<_>>()
        );
        assert!(words.len() * 4 + out.capacity() <= 2 * length + 3);
    }
}
#[test]
fn consumed_decimal_references_targets_strides_and_viewless_bounds() {
    let source=br#"{"asset":{"version":"2.0"},"buffers":[{"byteLength":36.0,"uri":"x"}],"bufferViews":[{"buffer":0e0,"byteLength":36e0,"byteStride":12.0,"target":34962.0}],"accessors":[{"bufferView":0.0,"componentType":5126.0,"type":"VEC3","count":3e0,"min":[0,0,0],"max":[0,0,0]}],"meshes":[{"primitives":[{"mode":4.0,"attributes":{"POSITION":0e0}}]}],"extras":{"opaque":1e400}}"#;
    assert_eq!(
        direct(source, limits(), 9, |_, _| Ok(vec![0; 36]))
            .unwrap()
            .vertices,
        3
    );
    let source=br#"{"asset":{"version":"2.0"},"accessors":[{"componentType":5126,"type":"VEC3","count":3,"min":[17,9,-4],"max":[-18,-2,8]}],"meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}],"extras":{"extensions":42,"opaque":1e400}}"#;
    assert_eq!(
        direct(source, limits(), 9, |_, _| panic!("viewless"))
            .unwrap()
            .vertices,
        3
    );
}
#[test]
fn real_meshopt_owner_admits_output_before_touching_invalid_compressed_stream() {
    let admitted = json::admit(
        br#"{"buffer":0,"byteLength":1,"count":2,"byteStride":4,"mode":"ATTRIBUTES"}"#,
        limits().json,
    )
    .unwrap();
    let extension: records::Meshopt<'_> = record(Some(admitted.raw())).unwrap();
    let buffers = [Cow::Borrowed(&[0u8][..])];
    assert!(matches!(
        decode_meshopt(&extension, 0, &buffers, 7),
        Err(FormatError::ResourceLimit(_))
    ));
    assert!(matches!(
        decode_meshopt(&extension, 0, &buffers, 8),
        Err(FormatError::InvalidInput(_))
    ));
}
#[test]
fn primitive_mode_preserves_defined_profile_refusals_and_rejects_undefined_enums() {
    for token in ["7", "18446744073709551615", "1e400"] {
        let source = format!(
            r#"{{"asset":{{"version":"2.0"}},"meshes":[{{"primitives":[{{"mode":{token}}}]}}]}}"#
        );
        assert!(
            matches!(
                direct(source.as_bytes(), limits(), 9, |_, _| panic!(
                    "no resources"
                )),
                Err(PayloadError::Format(FormatError::InvalidInput(_)))
            ),
            "{token}"
        );
    }
    for token in ["2", "5.0", "6e0"] {
        let source = format!(
            r#"{{"asset":{{"version":"2.0"}},"meshes":[{{"primitives":[{{"mode":{token}}}]}}]}}"#
        );
        assert!(
            matches!(
                direct(source.as_bytes(), limits(), 9, |_, _| panic!(
                    "no resources"
                )),
                Err(PayloadError::Format(FormatError::Unsupported(_)))
            ),
            "{token}"
        );
    }
}
