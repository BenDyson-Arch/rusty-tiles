//! Independently authored bounded controls for byte-bound C1 replay.
//! Fixtures are handcrafted glTF records and raw component words, not writer output.
use super::*;
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};
use std::cell::Cell;

thread_local! {
    static SCALARS: Cell<u64> = const { Cell::new(0) };
    static CONTAINERS: Cell<u64> = const { Cell::new(0) };
}

pub(super) fn scanned_component() {
    SCALARS.with(|count| count.set(count.get() + 1));
}

pub(super) fn inspected_ktx2() {
    CONTAINERS.with(|count| count.set(count.get() + 1));
}

fn reset() {
    SCALARS.with(|count| count.set(0));
    CONTAINERS.with(|count| count.set(0));
}

fn scans() -> (u64, u64) {
    (SCALARS.with(Cell::get), CONTAINERS.with(Cell::get))
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Cause(&'static str, u32);

#[derive(Debug, PartialEq)]
struct DrawTrace {
    accessor: usize,
    format: scene::PositionFormat,
    count: usize,
    marks: Vec<usize>,
    // Float words retain their exact bits, including negative zero.
    words: Vec<[u32; 3]>,
}

#[derive(Debug, PartialEq)]
enum Event {
    Enter(scene::NodeTransform, u64),
    Leave(scene::NodeTransform),
    Instance(usize, usize, u64, u64, Vec<DrawTrace>),
}

#[derive(Default)]
struct Consumer {
    events: Vec<Event>,
    only_instances: bool,
    fail_enter: bool,
}

impl scene::SceneConsumer for Consumer {
    type Error = Cause;

    fn instances_only(&self) -> bool {
        self.only_instances
    }

    fn enter(&mut self, transform: &scene::NodeTransform, depth: u64) -> Result<(), Cause> {
        self.events.push(Event::Enter(*transform, depth));
        if self.fail_enter {
            Err(Cause("consumer", 71))
        } else {
            Ok(())
        }
    }

    fn leave(&mut self, transform: &scene::NodeTransform) {
        self.events.push(Event::Leave(*transform));
    }

    fn instance(&mut self, instance: &scene::Instance<'_, '_>) -> Result<(), Cause> {
        let draws = instance
            .positions()
            .iter()
            .map(|&accessor| {
                let mut marks = Vec::new();
                instance.mark(accessor, |vertex| marks.push(vertex));
                let words = marks
                    .iter()
                    .map(|&vertex| match instance.vertex(accessor, vertex) {
                        scene::PositionWord::F32(word) => word.map(f32::to_bits),
                        scene::PositionWord::Int(word) => word.map(|v| v as u32),
                    })
                    .collect();
                DrawTrace {
                    accessor,
                    format: instance.format(accessor),
                    count: instance.count(accessor),
                    marks,
                    words,
                }
            })
            .collect();
        self.events.push(Event::Instance(
            instance.node(),
            instance.mesh(),
            instance.vertex_total(),
            instance.index_total(),
            draws,
        ));
        Ok(())
    }
}

type Failure = SceneError<Cause, Cause>;
type Trace = Vec<(String, usize)>;

fn limits() -> PayloadLimits {
    PayloadLimits {
        member_bytes: 65_536,
        decoded_bytes: 65_536,
        accessor_components: 4096,
        json: super::super::JsonLimits {
            bytes: 65_536,
            depth: 64,
            value_nodes: 4096,
        },
    }
}

#[allow(clippy::too_many_arguments)] // Explicit independent case inputs; test helper only.
fn execute(
    kind: PayloadKind,
    bytes: &[u8],
    resources: &[Vec<u8>],
    caps: PayloadLimits,
    remaining: u64,
    consumer: &mut Consumer,
    admission: Option<&ReplayAdmission>,
    fail_at: Option<usize>,
) -> (Result<PayloadInspection, Failure>, Trace) {
    let mut trace = Vec::new();
    let mut resolve = |uri: &str, maximum: usize| {
        let entry = trace.len();
        trace.push((uri.to_owned(), maximum));
        if fail_at == Some(entry) {
            return Err(Cause("resolver", entry as u32));
        }
        Ok(resources
            .get(entry)
            .expect("unexpected resolver call")
            .clone())
    };
    let result = match admission {
        Some(admission) => replay_scene(
            kind,
            bytes,
            caps,
            remaining,
            &mut resolve,
            consumer,
            admission,
        ),
        None => inspect_scene(kind, bytes, caps, remaining, &mut resolve, consumer),
    };
    (result, trace)
}

fn facts_equal(first: &PayloadInspection, second: &PayloadInspection) {
    assert_eq!(first.replay, second.replay);
    assert_eq!(first.schema_uri, second.schema_uri);
    assert_eq!(first.accessors_checked, second.accessors_checked);
    assert_eq!(first.primitives_checked, second.primitives_checked);
    assert_eq!(first.vertices, second.vertices);
    assert_eq!(first.elements_checked, second.elements_checked);
    assert_eq!(first.materialized_bytes, second.materialized_bytes);
    assert_eq!(first.not_inspected, second.not_inspected);
    assert_eq!(first.exclusions, second.exclusions);
    assert_eq!(first.json_nodes, second.json_nodes);
    assert_eq!(first.external_resources, second.external_resources);
    assert_eq!(first.instance_primitives, second.instance_primitives);
}

fn compare(kind: PayloadKind, bytes: &[u8], resources: &[Vec<u8>], only_instances: bool) {
    reset();
    let mut first_events = Consumer {
        only_instances,
        ..Default::default()
    };
    let (first, first_trace) = execute(
        kind,
        bytes,
        resources,
        limits(),
        4096,
        &mut first_events,
        None,
        None,
    );
    let first = first.unwrap();
    assert!(
        scans().0 > 0,
        "fixture must execute first-admission value checks"
    );
    reset();
    let mut replay_events = Consumer {
        only_instances,
        ..Default::default()
    };
    let (second, second_trace) = execute(
        kind,
        bytes,
        resources,
        limits(),
        4096,
        &mut replay_events,
        Some(&first.replay),
        None,
    );
    facts_equal(&first, &second.unwrap());
    assert_eq!(first_trace, second_trace);
    assert_eq!(first_events.events, replay_events.events);
    assert_eq!(
        scans(),
        (0, 0),
        "replay must remove C1 value/container work"
    );
}

fn fixture() -> (Value, Vec<u8>) {
    let positions = [-1.0f32, -0.0, 0.0, 2.0, 1.0, 0.0, 0.0, 0.0, 3.0];
    let mut data = positions
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .collect::<Vec<_>>();
    data.extend([0, 1, 2]);
    let doc = json!({
        "asset":{"version":"2.0"},
        "buffers":[{"uri":"geometry.bin","byteLength":39}],
        "bufferViews":[
            {"buffer":0,"byteLength":36,"target":34962},
            {"buffer":0,"byteOffset":36,"byteLength":3,"target":34963}
        ],
        "accessors":[
            {"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[-1,0,0],"max":[2,1,3]},
            {"bufferView":1,"componentType":5121,"count":3,"type":"SCALAR"}
        ],
        "meshes":[{"primitives":[{"attributes":{"POSITION":0},"indices":1}]}],
        "nodes":[{"mesh":0}],"scenes":[{"nodes":[0]}],"scene":0
    });
    (doc, data)
}

fn frame(doc: &Value, data: &[u8]) -> Vec<u8> {
    let mut doc = doc.clone();
    doc["buffers"][0].as_object_mut().unwrap().remove("uri");
    let mut text = serde_json::to_vec(&doc).unwrap();
    text.resize(text.len().next_multiple_of(4), b' ');
    let mut bin = data.to_vec();
    bin.resize(bin.len().next_multiple_of(4), 0);
    let mut out = b"glTF".to_vec();
    for number in [
        2u32,
        (28 + text.len() + bin.len()) as u32,
        text.len() as u32,
    ] {
        out.extend(number.to_le_bytes());
    }
    out.extend(b"JSON");
    out.extend(text);
    out.extend((bin.len() as u32).to_le_bytes());
    out.extend(b"BIN\0");
    out.extend(bin);
    out
}

fn admission(bytes: &[u8], resources: &[Vec<u8>]) -> ReplayAdmission {
    let (first, _) = execute(
        PayloadKind::LocalGltf,
        bytes,
        resources,
        limits(),
        4096,
        &mut Consumer::default(),
        None,
        None,
    );
    first.unwrap().replay
}

fn changed(result: Result<PayloadInspection, Failure>) {
    assert!(
        matches!(result,
        Err(SceneError::Payload(PayloadError::Format(FormatError::InvalidInput(ref message))))
        if message == "source archive changed during inspection"),
        "{result:?}"
    );
}

fn error_key(error: Failure) -> (&'static str, String) {
    match error {
        SceneError::Payload(PayloadError::Format(FormatError::InvalidInput(message))) => {
            ("invalid", message)
        }
        SceneError::Payload(PayloadError::Format(FormatError::Unsupported(message))) => {
            ("unsupported", message)
        }
        SceneError::Payload(PayloadError::Format(FormatError::ResourceLimit(message))) => {
            ("limit", message)
        }
        SceneError::Payload(PayloadError::Resolver(cause)) => ("resolver", format!("{cause:?}")),
        SceneError::Consumer(cause) => ("consumer", format!("{cause:?}")),
    }
}

#[test]
fn full_and_replay_preserve_raw_words_graph_order_and_shared_position_groups() {
    let (mut doc, mut data) = fixture();
    // A valid restart strip with two nonconstant colour-bearing segments.
    data.truncate(36);
    data.extend([0, 1, 255, 1, 2]);
    data.resize(44, 0);
    data.extend(
        [0.0f32, 0.5, 1.0, 0.25, 1.0, 0.0, 1.0, 0.0, 0.75]
            .iter()
            .flat_map(|v| v.to_le_bytes()),
    );
    doc["buffers"][0]["byteLength"] = json!(data.len());
    doc["bufferViews"][1]["byteLength"] = json!(5);
    doc["bufferViews"]
        .as_array_mut()
        .unwrap()
        .push(json!({"buffer":0,"byteOffset":44,"byteLength":36,"target":34962}));
    doc["accessors"][1]["count"] = json!(5);
    doc["accessors"]
        .as_array_mut()
        .unwrap()
        .push(json!({"bufferView":2,"componentType":5126,"count":3,"type":"VEC3"}));
    doc["extensionsUsed"] = json!(["KHR_mesh_primitive_restart"]);
    doc["extensionsRequired"] = doc["extensionsUsed"].clone();
    doc["meshes"][0]["primitives"] = json!([
        {"attributes":{"POSITION":0,"COLOR_0":2},"indices":1,"mode":3},
        {"attributes":{"POSITION":0,"COLOR_0":2},"indices":1,"mode":3}
    ]);
    doc["nodes"] = json!([
        {"children":[1,2],"translation":[1,2,3]},
        {"mesh":0,"rotation":[0,0,0,1]},
        {"children":[3]},
        {"scale":[2,2,2]},
        {"mesh":0}
    ]);
    for mode in [false, true] {
        compare(PayloadKind::Glb, &frame(&doc, &data), &[], mode);
    }
    reset();
    let mut consumer = Consumer::default();
    let (full, _) = execute(
        PayloadKind::Glb,
        &frame(&doc, &data),
        &[],
        limits(),
        4096,
        &mut consumer,
        None,
        None,
    );
    assert_eq!(full.unwrap().elements_checked, 23);
    assert_eq!(scans().0, 23);
    let draws = consumer
        .events
        .iter()
        .find_map(|event| match event {
            Event::Instance(_, _, t, idx, draws) => Some((*t, *idx, draws)),
            _ => None,
        })
        .unwrap();
    assert_eq!((draws.0, draws.1), (3, 10));
    assert_eq!(draws.2.len(), 1);
    assert_eq!(draws.2[0].marks, [0, 1, 1, 2, 0, 1, 1, 2]);
    assert_eq!(draws.2[0].words[0][1], (-0.0f32).to_bits());
}

#[test]
fn inline_data_uri_and_external_resources_keep_facts_and_resolver_limits() {
    let (mut doc, data) = fixture();
    compare(
        PayloadKind::LocalGltf,
        &serde_json::to_vec(&doc).unwrap(),
        std::slice::from_ref(&data),
        true,
    );
    doc["buffers"][0]["uri"] = json!(format!(
        "data:application/octet-stream;base64,{}",
        STANDARD.encode(&data)
    ));
    compare(
        PayloadKind::LocalGltf,
        &serde_json::to_vec(&doc).unwrap(),
        &[],
        true,
    );
}

#[test]
fn component_index_and_viewless_classes_replay_the_exact_draw_words() {
    for (component, normalized, values) in [
        (5126, false, [-1, 0, 0, 2, 1, 0, 0, 0, 3]),
        (5120, true, [-128, 0, 0, 127, 1, 0, 0, 0, 3]),
        (5122, true, [-32768, 0, 0, 32767, 1, 0, 0, 0, 3]),
        (5121, false, [0, 0, 0, 2, 1, 0, 0, 0, 3]),
        (5123, false, [0, 0, 0, 2, 1, 0, 0, 0, 3]),
    ] {
        for index_component in [5121, 5123, 5125] {
            let (mut doc, _) = fixture();
            let mut data = Vec::new();
            for value in values {
                match component {
                    5126 => data.extend((value as f32).to_le_bytes()),
                    5120 | 5121 => data.push(value as u8),
                    _ => data.extend((value as i16).to_le_bytes()),
                }
            }
            let vertex_bytes = data.len();
            data.resize(data.len().next_multiple_of(4), 0);
            let index_offset = data.len();
            for index in [2u32, 0, 1] {
                match index_component {
                    5121 => data.push(index as u8),
                    5123 => data.extend((index as u16).to_le_bytes()),
                    _ => data.extend(index.to_le_bytes()),
                }
            }
            doc["buffers"][0]["byteLength"] = json!(data.len());
            doc["bufferViews"][0]["byteLength"] = json!(vertex_bytes);
            doc["bufferViews"][0]["byteStride"] = json!(match component {
                5126 => 12,
                5120 | 5121 => 4,
                _ => 8,
            });
            // Quantized vertices are individually four-byte aligned; insert stride padding.
            if component != 5126 {
                let size = if [5120, 5121].contains(&component) {
                    1
                } else {
                    2
                };
                let stride = if size == 1 { 4 } else { 8 };
                let packed = data[..vertex_bytes].to_vec();
                let indices = data[index_offset..].to_vec();
                data.clear();
                for vertex in packed.chunks_exact(3 * size) {
                    data.extend(vertex);
                    data.resize(data.len() + stride - 3 * size, 0);
                }
                let offset = data.len();
                data.extend(indices);
                doc["bufferViews"][0]["byteLength"] = json!(offset);
                doc["bufferViews"][1]["byteOffset"] = json!(offset);
                doc["buffers"][0]["byteLength"] = json!(data.len());
            } else {
                doc["bufferViews"][1]["byteOffset"] = json!(index_offset);
            }
            doc["bufferViews"][1]["byteLength"] = json!(match index_component {
                5121 => 3,
                5123 => 6,
                _ => 12,
            });
            doc["accessors"][0]["componentType"] = json!(component);
            doc["accessors"][0]["normalized"] = json!(normalized);
            doc["accessors"][0]["min"] = json!([values[0], 0, 0]);
            doc["accessors"][0]["max"] = json!([values[3], 1, 3]);
            doc["accessors"][1]["componentType"] = json!(index_component);
            if component != 5126 {
                doc["extensionsUsed"] = json!(["KHR_mesh_quantization"]);
                doc["extensionsRequired"] = doc["extensionsUsed"].clone();
            }
            compare(PayloadKind::Glb, &frame(&doc, &data), &[], true);
        }
    }
    let (mut doc, _) = fixture();
    doc.as_object_mut().unwrap().remove("buffers");
    doc.as_object_mut().unwrap().remove("bufferViews");
    doc["accessors"] = json!([
        {"componentType":5126,"count":3,"type":"VEC3","min":[10,20,30],"max":[40,50,60]},
        {"componentType":5121,"count":3,"type":"SCALAR"}
    ]);
    compare(
        PayloadKind::LocalGltf,
        &serde_json::to_vec(&doc).unwrap(),
        &[],
        true,
    );

    let (doc, compressed) = meshopt_fixture();
    let bytes = serde_json::to_vec(&doc).unwrap();
    compare(
        PayloadKind::LocalGltf,
        &bytes,
        std::slice::from_ref(&compressed),
        true,
    );
    let mut consumer = Consumer::default();
    let (facts, _) = execute(
        PayloadKind::LocalGltf,
        &bytes,
        std::slice::from_ref(&compressed),
        limits(),
        4096,
        &mut consumer,
        None,
        None,
    );
    assert_eq!(
        facts.unwrap().materialized_bytes,
        compressed.len() as u64 + 48
    );
    let words = consumer
        .events
        .iter()
        .find_map(|event| match event {
            Event::Instance(_, _, _, _, draws) => Some(&draws[0].words),
            _ => None,
        })
        .unwrap();
    assert_eq!(words, &[[0, 0, 0], [44, 1, 0], [0, 0, 44], [44, 1, 44]]);
}

fn meshopt_fixture() -> (Value, Vec<u8>) {
    // Pinned upstream zeux/meshoptimizer@3d8e9b8a2a2b9a5becfc6fbc512307b04207ab5d
    // js/meshopt_decoder.test.js::decodeVertexBuffer, ATTRIBUTES v0/count4/stride12.
    // Independent expected raw POSITION triples use the first three bytes per vertex.
    let stream = vec![
        0xa0, 0x01, 0x3f, 0x00, 0x00, 0x00, 0x58, 0x57, 0x58, 0x01, 0x26, 0x00, 0x00, 0x00, 0x01,
        0x0c, 0x00, 0x00, 0x00, 0x58, 0x01, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01,
        0x3f, 0x00, 0x00, 0x00, 0x17, 0x18, 0x17, 0x01, 0x26, 0x00, 0x00, 0x00, 0x01, 0x0c, 0x00,
        0x00, 0x00, 0x17, 0x01, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];
    let doc = json!({
        "asset":{"version":"2.0"},
        "extensionsUsed":["EXT_meshopt_compression","KHR_mesh_quantization"],
        "extensionsRequired":["EXT_meshopt_compression","KHR_mesh_quantization"],
        "buffers":[
            {"byteLength":48,"extensions":{"EXT_meshopt_compression":{"fallback":true}}},
            {"uri":"meshopt.bin","byteLength":stream.len()}
        ],
        "bufferViews":[{"buffer":0,"byteLength":48,"byteStride":12,"target":34962,
            "extensions":{"EXT_meshopt_compression":{
                "buffer":1,"byteLength":stream.len(),"count":4,"byteStride":12,"mode":"ATTRIBUTES"
            }}
        }],
        "accessors":[{"bufferView":0,"componentType":5121,"count":4,"type":"VEC3","min":[0,0,0],"max":[44,1,44]}],
        "meshes":[{"primitives":[{"attributes":{"POSITION":0},"mode":0}]}],
        "nodes":[{"mesh":0}],"scenes":[{"nodes":[0]}],"scene":0
    });
    (doc, stream)
}

#[test]
fn member_wrong_kind_and_wrong_receipt_refuse_before_resolution_or_events() {
    let (doc, data) = fixture();
    let bytes = serde_json::to_vec(&doc).unwrap();
    let receipt = admission(&bytes, std::slice::from_ref(&data));
    let mut other_doc = doc.clone();
    other_doc["asset"]["generator"] = json!("different bytes");
    let other = serde_json::to_vec(&other_doc).unwrap();
    let wrong = admission(&other, std::slice::from_ref(&data));
    for (kind, source, receipt) in [
        (PayloadKind::LocalGltf, other.as_slice(), &receipt),
        (PayloadKind::Glb, bytes.as_slice(), &receipt),
        (PayloadKind::LocalGltf, bytes.as_slice(), &wrong),
    ] {
        let mut consumer = Consumer::default();
        let (result, trace) = execute(
            kind,
            source,
            &[],
            limits(),
            4096,
            &mut consumer,
            Some(receipt),
            None,
        );
        changed(result);
        assert!(trace.is_empty());
        assert!(consumer.events.is_empty());
    }

    let stored = receipt.to_stored_bytes();
    let restored = ReplayAdmission::from_stored_bytes(stored);
    assert_eq!(restored, receipt);
    let mut consumer = Consumer::default();
    let (result, _) = execute(
        PayloadKind::LocalGltf,
        &bytes,
        std::slice::from_ref(&data),
        limits(),
        4096,
        &mut consumer,
        Some(&restored),
        None,
    );
    assert!(result.is_ok());
    for offset in [0, 32] {
        let mut corrupted = stored;
        corrupted[offset] ^= 1;
        let restored = ReplayAdmission::from_stored_bytes(corrupted);
        let mut consumer = Consumer::default();
        let (result, trace) = execute(
            PayloadKind::LocalGltf,
            &bytes,
            std::slice::from_ref(&data),
            limits(),
            4096,
            &mut consumer,
            Some(&restored),
            None,
        );
        changed(result);
        assert_eq!(trace.len(), if offset == 0 { 0 } else { 1 });
        assert!(consumer.events.is_empty());
    }
}

#[test]
fn changed_resource_values_and_ignored_tails_refuse_before_events() {
    let (doc, mut data) = fixture();
    data.extend([7, 8, 9]); // External buffer trailing bytes are deliberately ignored by views.
    let bytes = serde_json::to_vec(&doc).unwrap();
    let receipt = admission(&bytes, std::slice::from_ref(&data));
    for at in [0, data.len() - 1] {
        let mut altered = data.clone();
        altered[at] ^= 1;
        let mut consumer = Consumer::default();
        let (result, trace) = execute(
            PayloadKind::LocalGltf,
            &bytes,
            &[altered],
            limits(),
            4096,
            &mut consumer,
            Some(&receipt),
            None,
        );
        changed(result);
        assert_eq!(trace.len(), 1);
        assert!(consumer.events.is_empty());
    }
}

#[test]
fn repeated_uri_order_and_ambiguous_unframed_boundaries_cannot_reuse_a_receipt() {
    let (mut doc, data) = fixture();
    doc["buffers"][0]["uri"] = json!("same.bin");
    doc["buffers"]
        .as_array_mut()
        .unwrap()
        .push(json!({"uri":"same.bin","byteLength":1}));
    let bytes = serde_json::to_vec(&doc).unwrap();
    // U is exactly the URI framing an incorrect data-length-free transcript would use.
    let mut u = (8u64).to_le_bytes().to_vec();
    u.extend(b"same.bin");
    let mut second = u.clone();
    second.push(7);
    let original = [data.clone(), second];
    let receipt = admission(&bytes, &original);
    let mut shifted_first = data.clone();
    shifted_first.extend(&u);
    let shifted = [shifted_first, vec![7]];
    let mut consumer = Consumer::default();
    let (result, trace) = execute(
        PayloadKind::LocalGltf,
        &bytes,
        &shifted,
        limits(),
        4096,
        &mut consumer,
        Some(&receipt),
        None,
    );
    changed(result);
    assert_eq!(
        trace
            .iter()
            .map(|entry| entry.0.as_str())
            .collect::<Vec<_>>(),
        ["same.bin", "same.bin"]
    );
    assert!(consumer.events.is_empty());

    doc["buffers"][1]["byteLength"] = json!(39);
    let bytes = serde_json::to_vec(&doc).unwrap();
    let mut alternative = data.clone();
    alternative[0] ^= 1;
    let original = [data, alternative];
    let receipt = admission(&bytes, &original);
    let reversed = [original[1].clone(), original[0].clone()];
    let mut consumer = Consumer::default();
    let (result, _) = execute(
        PayloadKind::LocalGltf,
        &bytes,
        &reversed,
        limits(),
        4096,
        &mut consumer,
        Some(&receipt),
        None,
    );
    changed(result);
    assert!(consumer.events.is_empty());
}

#[test]
fn resolver_errors_keep_the_exact_prefix_and_cause_before_consumer_events() {
    let (mut doc, data) = fixture();
    doc["buffers"]
        .as_array_mut()
        .unwrap()
        .push(json!({"uri":"other.bin","byteLength":1}));
    doc["images"] = json!([{"uri":"one.png"},{"uri":"two.png"}]);
    let resources = [data, vec![7], vec![1, 2, 3], vec![4, 5]];
    let bytes = serde_json::to_vec(&doc).unwrap();
    let receipt = admission(&bytes, &resources);
    for fail_at in 0..resources.len() {
        let mut full = Consumer::default();
        let mut replay = Consumer::default();
        let (a, a_trace) = execute(
            PayloadKind::LocalGltf,
            &bytes,
            &resources,
            limits(),
            4096,
            &mut full,
            None,
            Some(fail_at),
        );
        let (b, b_trace) = execute(
            PayloadKind::LocalGltf,
            &bytes,
            &resources,
            limits(),
            4096,
            &mut replay,
            Some(&receipt),
            Some(fail_at),
        );
        assert!(
            matches!(a, Err(SceneError::Payload(PayloadError::Resolver(Cause("resolver", n)))) if n == fail_at as u32)
        );
        assert!(
            matches!(b, Err(SceneError::Payload(PayloadError::Resolver(Cause("resolver", n)))) if n == fail_at as u32)
        );
        assert_eq!(a_trace, b_trace);
        assert_eq!(a_trace.len(), fail_at + 1);
        assert!(full.events.is_empty() && replay.events.is_empty());
    }
}

#[test]
fn admission_receipts_do_not_bypass_reduced_byte_json_decode_or_element_limits() {
    let (doc, data) = fixture();
    let bytes = serde_json::to_vec(&doc).unwrap();
    let receipt = admission(&bytes, std::slice::from_ref(&data));
    let cases = [
        (
            PayloadLimits {
                member_bytes: bytes.len() - 1,
                ..limits()
            },
            4096,
        ),
        (
            PayloadLimits {
                decoded_bytes: 35,
                ..limits()
            },
            4096,
        ),
        (
            PayloadLimits {
                accessor_components: 11,
                ..limits()
            },
            4096,
        ),
        (
            PayloadLimits {
                json: super::super::JsonLimits {
                    bytes: bytes.len() - 1,
                    ..limits().json
                },
                ..limits()
            },
            4096,
        ),
        (limits(), 11),
    ];
    for (caps, remaining) in cases {
        let mut full = Consumer::default();
        let mut replay = Consumer::default();
        let (a, a_trace) = execute(
            PayloadKind::LocalGltf,
            &bytes,
            std::slice::from_ref(&data),
            caps,
            remaining,
            &mut full,
            None,
            None,
        );
        let (b, b_trace) = execute(
            PayloadKind::LocalGltf,
            &bytes,
            std::slice::from_ref(&data),
            caps,
            remaining,
            &mut replay,
            Some(&receipt),
            None,
        );
        let a = error_key(a.unwrap_err());
        assert_eq!(a.0, "limit");
        assert_eq!(a, error_key(b.unwrap_err()));
        assert_eq!(a_trace, b_trace);
        assert!(full.events.is_empty() && replay.events.is_empty());
    }
    let caps = PayloadLimits {
        member_bytes: bytes.len(),
        decoded_bytes: 39,
        accessor_components: 12,
        ..limits()
    };
    let mut replay = Consumer::default();
    let (at_cap, _) = execute(
        PayloadKind::LocalGltf,
        &bytes,
        std::slice::from_ref(&data),
        caps,
        12,
        &mut replay,
        Some(&receipt),
        None,
    );
    assert!(at_cap.is_ok());
    let oversized = vec![0; caps.member_bytes + 1];
    let mut replay = Consumer::default();
    let (result, _) = execute(
        PayloadKind::LocalGltf,
        &bytes,
        &[oversized],
        caps,
        12,
        &mut replay,
        Some(&receipt),
        None,
    );
    assert_eq!(error_key(result.unwrap_err()).0, "limit");
    assert!(replay.events.is_empty());
}

#[test]
fn consumer_failure_is_unchanged_after_verified_reconstruction() {
    let (doc, data) = fixture();
    let bytes = serde_json::to_vec(&doc).unwrap();
    let receipt = admission(&bytes, std::slice::from_ref(&data));
    let mut a = Consumer {
        fail_enter: true,
        ..Default::default()
    };
    let mut b = Consumer {
        fail_enter: true,
        ..Default::default()
    };
    let (first, first_trace) = execute(
        PayloadKind::LocalGltf,
        &bytes,
        std::slice::from_ref(&data),
        limits(),
        4096,
        &mut a,
        None,
        None,
    );
    let (second, second_trace) = execute(
        PayloadKind::LocalGltf,
        &bytes,
        std::slice::from_ref(&data),
        limits(),
        4096,
        &mut b,
        Some(&receipt),
        None,
    );
    assert!(matches!(
        first,
        Err(SceneError::Consumer(Cause("consumer", 71)))
    ));
    assert!(matches!(
        second,
        Err(SceneError::Consumer(Cause("consumer", 71)))
    ));
    assert_eq!(first_trace, second_trace);
    assert_eq!(a.events, b.events);
}

fn ktx2_one_level() -> Vec<u8> {
    // Normative 4x4 RGB UASTC container: 80-byte header, one 24-byte level,
    // 44-byte Khronos basic DFD, 16-byte-aligned 16-byte block at offset 160.
    let mut bytes = vec![0u8; 176];
    bytes[..12].copy_from_slice(&[
        0xab, 0x4b, 0x54, 0x58, 0x20, 0x32, 0x30, 0xbb, 0x0d, 0x0a, 0x1a, 0x0a,
    ]);
    for (at, word) in [
        (16, 1u32),
        (20, 4),
        (24, 4),
        (36, 1),
        (40, 1),
        (48, 104),
        (52, 44),
        (104, 44),
        (112, 2 | (40 << 16)),
        (132, 127 << 16),
        (144, u32::MAX),
    ] {
        bytes[at..at + 4].copy_from_slice(&word.to_le_bytes());
    }
    for (at, word) in [(80, 160u64), (88, 16), (96, 16)] {
        bytes[at..at + 8].copy_from_slice(&word.to_le_bytes());
    }
    bytes[116..132].copy_from_slice(&[166, 1, 2, 0, 3, 3, 0, 0, 16, 0, 0, 0, 0, 0, 0, 0]);
    bytes
}

#[test]
fn actual_ktx2_containers_are_checked_on_admission_only_and_keep_labels() {
    let (mut doc, data) = fixture();
    let container = ktx2_one_level();
    doc["extensionsUsed"] = json!(["KHR_texture_basisu"]);
    doc["extensionsRequired"] = doc["extensionsUsed"].clone();
    doc["textures"] = json!([{"extensions":{"KHR_texture_basisu":{"source":0}}}]);
    doc["materials"] = json!([{"pbrMetallicRoughness":{"baseColorTexture":{"index":0}}}]);
    doc["meshes"][0]["primitives"][0]["material"] = json!(0);
    for embedded in [false, true] {
        let mut doc = doc.clone();
        let mut bin = data.clone();
        let resources = if embedded {
            bin.resize(40, 0);
            bin.extend(&container);
            doc["buffers"][0]["byteLength"] = json!(bin.len());
            doc["bufferViews"]
                .as_array_mut()
                .unwrap()
                .push(json!({"buffer":0,"byteOffset":40,"byteLength":176}));
            doc["images"] = json!([{"bufferView":2,"mimeType":"image/ktx2"}]);
            vec![]
        } else {
            doc["images"] = json!([{"uri":"colour.ktx2","mimeType":"image/ktx2"}]);
            vec![data.clone(), container.clone()]
        };
        let kind = if embedded {
            PayloadKind::Glb
        } else {
            PayloadKind::LocalGltf
        };
        let bytes = if embedded {
            frame(&doc, &bin)
        } else {
            serde_json::to_vec(&doc).unwrap()
        };
        reset();
        let mut first_events = Consumer::default();
        let (first, _) = execute(
            kind,
            &bytes,
            &resources,
            limits(),
            4096,
            &mut first_events,
            None,
            None,
        );
        let first = first.unwrap();
        assert_eq!(scans(), (12, 1));
        assert!(first
            .not_inspected
            .contains("KTX2 level data and transcoding"));
        reset();
        let mut second_events = Consumer::default();
        let (second, _) = execute(
            kind,
            &bytes,
            &resources,
            limits(),
            4096,
            &mut second_events,
            Some(&first.replay),
            None,
        );
        facts_equal(&first, &second.unwrap());
        assert_eq!(first_events.events, second_events.events);
        assert_eq!(scans(), (0, 0));
    }
}

#[test]
fn first_admission_keeps_value_restart_bounds_colour_and_container_failures() {
    let (doc, data) = fixture();
    let mut cases = Vec::new();
    let mut nan = data.clone();
    nan[..4].copy_from_slice(&f32::NAN.to_le_bytes());
    cases.push((doc.clone(), vec![nan], "invalid"));
    let mut lying = doc.clone();
    lying["accessors"][0]["max"] = json!([100, 1, 3]);
    cases.push((lying, vec![data.clone()], "invalid"));
    let mut bad_index = data.clone();
    bad_index[38] = 3;
    cases.push((doc.clone(), vec![bad_index], "invalid"));
    let mut colour = doc.clone();
    colour["meshes"][0]["primitives"][0]["attributes"]["COLOR_0"] = json!(0);
    cases.push((colour, vec![data.clone()], "invalid"));
    let mut restart = doc.clone();
    restart["extensionsUsed"] = json!(["KHR_mesh_primitive_restart"]);
    restart["extensionsRequired"] = restart["extensionsUsed"].clone();
    restart["meshes"][0]["primitives"][0]["mode"] = json!(3);
    let mut segment = data.clone();
    segment[37] = 255;
    cases.push((restart, vec![segment], "unsupported"));
    let mut image = doc.clone();
    image["extensionsUsed"] = json!(["KHR_texture_basisu"]);
    image["extensionsRequired"] = image["extensionsUsed"].clone();
    image["textures"] = json!([{"extensions":{"KHR_texture_basisu":{"source":0}}}]);
    image["images"] = json!([{"uri":"bad.ktx2","mimeType":"image/ktx2"}]);
    cases.push((image, vec![data, vec![0; 80]], "invalid"));
    for (doc, resources, wanted) in cases {
        let mut consumer = Consumer {
            fail_enter: true,
            ..Default::default()
        };
        let (result, _) = execute(
            PayloadKind::LocalGltf,
            &serde_json::to_vec(&doc).unwrap(),
            &resources,
            limits(),
            4096,
            &mut consumer,
            None,
            None,
        );
        assert_eq!(error_key(result.unwrap_err()).0, wanted);
        assert!(
            consumer.events.is_empty(),
            "C1 must precede a failing consumer"
        );
    }
}

#[test]
fn changed_resource_structural_and_later_resolver_errors_keep_causal_precedence() {
    let (mut doc, data) = fixture();
    doc["images"] = json!([{"uri":"image.png"}]);
    let bytes = serde_json::to_vec(&doc).unwrap();
    let resources = [data.clone(), vec![7]];
    let receipt = admission(&bytes, &resources);
    let mut consumer = Consumer::default();
    let (short, trace) = execute(
        PayloadKind::LocalGltf,
        &bytes,
        &[vec![0; 38], vec![7]],
        limits(),
        4096,
        &mut consumer,
        Some(&receipt),
        Some(1),
    );
    let key = error_key(short.unwrap_err());
    assert_eq!(
        key,
        (
            "invalid",
            "buffer byteLength does not match actual resource".into()
        )
    );
    assert_eq!(trace.len(), 1);
    assert!(consumer.events.is_empty());
    let mut changed_data = data;
    changed_data[..4].copy_from_slice(&f32::NAN.to_le_bytes());
    let mut consumer = Consumer::default();
    let (later, trace) = execute(
        PayloadKind::LocalGltf,
        &bytes,
        &[changed_data, vec![7]],
        limits(),
        4096,
        &mut consumer,
        Some(&receipt),
        Some(1),
    );
    assert!(matches!(
        later,
        Err(SceneError::Payload(PayloadError::Resolver(Cause(
            "resolver", 1
        ))))
    ));
    assert_eq!(trace.len(), 2);
    assert!(consumer.events.is_empty());

    let (mut doc, mut compressed) = meshopt_fixture();
    doc["images"] = json!([{"uri":"later.png"}]);
    let bytes = serde_json::to_vec(&doc).unwrap();
    let receipt = admission(&bytes, &[compressed.clone(), vec![7]]);
    compressed[0] = 0xa1; // Decodable upstream version, excluded by the normative header gate.
    let mut consumer = Consumer::default();
    let (result, trace) = execute(
        PayloadKind::LocalGltf,
        &bytes,
        &[compressed, vec![7]],
        limits(),
        4096,
        &mut consumer,
        Some(&receipt),
        Some(1),
    );
    let key = error_key(result.unwrap_err());
    assert_eq!(key.0, "invalid");
    assert!(key.1.starts_with("meshopt stream header"));
    assert_eq!(trace.len(), 1);
    assert!(consumer.events.is_empty());
}
