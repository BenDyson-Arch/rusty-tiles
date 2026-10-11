//! C2 scene/node admission, scene selection, glTF exclusion facts and the
//! selected-scene walk over live C1 accessor views. No math, policy or positions Vec.
use super::numbers::at;
use super::records::{self, array, push, record, Raw};
use super::{allocation, invalid, value, Accessor, FormatResult};
use serde_json::value::RawValue;
use std::borrow::Cow;

/// glTF extensions that C2 interprets or that cannot move drawn POSITION values.
pub(crate) const GLTF_ALLOWLIST: [&str; 9] = [
    "KHR_mesh_quantization",
    "EXT_meshopt_compression",
    "KHR_mesh_primitive_restart",
    "EXT_mesh_features",
    "EXT_structural_metadata",
    "EXT_mesh_polygon",
    "KHR_materials_unlit",
    "KHR_texture_basisu",
    "EXT_texture_webp",
];

/// Node-local transform as consumed: exact binary64 values of the JSON text.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum NodeTransform {
    /// None of matrix, translation, rotation or scale is present.
    Skip,
    /// Column-major matrix with an exact affine last row.
    Matrix([f64; 16]),
    /// Absent fields take their defaults.
    Trs {
        t: [f64; 3],
        q: [f64; 4],
        s: [f64; 3],
    },
}

/// Exclusion facts owned by the payload format (contract domain table).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ContentExclusions {
    pub rtc_center: bool,
    pub gltf_extension: bool,
    pub scene_selection: bool,
    pub skin: bool,
    pub morph_targets: bool,
    pub animation: bool,
}

/// Raw POSITION storage: component type and the normalized flag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PositionFormat {
    F32,
    I8 { normalized: bool },
    U8 { normalized: bool },
    I16 { normalized: bool },
    U16 { normalized: bool },
}

/// One raw POSITION word triple, without dequantization.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum PositionWord {
    F32([f32; 3]),
    Int([i32; 3]),
}

/// One core primitive as admitted by C1.
#[derive(Clone, Copy)]
pub(super) struct PrimitiveFact {
    pub position: usize,
    pub indices: Option<usize>,
    pub targets: bool,
}

/// Mesh facts gathered during C1's primitive pass.
#[derive(Clone, Copy, Default)]
pub(super) struct MeshFact {
    /// Range into the primitive table.
    pub primitives: (usize, usize),
    /// Range into the distinct-POSITION table, in first-appearance order.
    pub positions: (usize, usize),
    pub weights: bool,
}

pub(super) struct MeshTable {
    pub meshes: Vec<MeshFact>,
    pub primitives: Vec<PrimitiveFact>,
    pub positions: Vec<usize>,
}

struct NodeFact {
    children: (usize, usize),
    mesh: Option<usize>,
    skin: bool,
    weights: bool,
    transform: NodeTransform,
}

/// Admitted node graph and the optionally selected scene.
pub(super) struct Graph {
    nodes: Vec<NodeFact>,
    children: Vec<usize>,
    /// Selected scene roots, or None for `sceneSelection`.
    roots: Option<Vec<usize>>,
    meshes_below: Vec<bool>,
}

/// The phase-1 and phase-2 consumer of the selected-scene walk.
pub(crate) trait SceneConsumer {
    type Error;
    /// Phase 2 skips selected subtrees without any mesh instance.
    fn instances_only(&self) -> bool;
    /// Entering a selected node at path depth `depth` (scene roots have depth 1).
    fn enter(&mut self, transform: &NodeTransform, depth: u64) -> Result<(), Self::Error>;
    fn leave(&mut self, transform: &NodeTransform);
    /// A selected node with a mesh, visited after `enter` of that node.
    fn instance(&mut self, instance: &Instance<'_, '_>) -> Result<(), Self::Error>;
}

/// The C1-only consumer, used by callers without a C2 consumer.
#[cfg(test)]
pub(crate) struct NoScene;
#[cfg(test)]
impl SceneConsumer for NoScene {
    type Error = std::convert::Infallible;
    fn instances_only(&self) -> bool {
        true
    }
    fn enter(&mut self, _: &NodeTransform, _: u64) -> Result<(), Self::Error> {
        Ok(())
    }
    fn leave(&mut self, _: &NodeTransform) {}
    fn instance(&mut self, _: &Instance<'_, '_>) -> Result<(), Self::Error> {
        Ok(())
    }
}

fn number(raw: &RawValue, node: usize, field: &str) -> FormatResult<f64> {
    serde_json::from_str::<f64>(raw.get())
        .ok()
        .filter(|n| n.is_finite())
        .ok_or_else(|| {
            invalid(format!(
                "glTF node {node}: {field} must contain finite numbers"
            ))
        })
}
fn numbers<const N: usize>(
    raw: Raw<'_>,
    node: usize,
    field: &str,
    ceiling: usize,
) -> FormatResult<Option<[f64; N]>> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let values = array::<&RawValue>(Some(raw), ceiling)
        .map_err(|_| invalid(format!("glTF node {node}: {field} must be an array")))?;
    if values.len() != N {
        return Err(invalid(format!(
            "glTF node {node}: {field} must have {N} numbers"
        )));
    }
    let mut out = [0.; N];
    for (slot, raw) in out.iter_mut().zip(values) {
        *slot = number(raw, node, field)?;
    }
    Ok(Some(out))
}
fn transform(
    node: &records::Node<'_>,
    index: usize,
    ceiling: usize,
) -> FormatResult<NodeTransform> {
    let matrix = numbers::<16>(node.matrix, index, "matrix", ceiling)?;
    let t = numbers::<3>(node.translation, index, "translation", ceiling)?;
    let q = numbers::<4>(node.rotation, index, "rotation", ceiling)?;
    let s = numbers::<3>(node.scale, index, "scale", ceiling)?;
    if let Some(q) = q {
        if q.iter().any(|v| !(-1.0..=1.0).contains(v)) || q.iter().all(|v| *v == 0.) {
            return Err(invalid(format!(
                "glTF node {index}: rotation must be nonzero with components in [-1, 1]"
            )));
        }
    }
    if let Some(m) = matrix {
        if t.is_some() || q.is_some() || s.is_some() {
            return Err(invalid(format!(
                "glTF node {index}: matrix must not coexist with translation, rotation or scale"
            )));
        }
        if m[3] != 0. || m[7] != 0. || m[11] != 0. || m[15] != 1. {
            return Err(invalid(format!(
                "glTF node {index}: matrix last row must be 0,0,0,1"
            )));
        }
        return Ok(NodeTransform::Matrix(m));
    }
    if t.is_none() && q.is_none() && s.is_none() {
        return Ok(NodeTransform::Skip);
    }
    Ok(NodeTransform::Trs {
        t: t.unwrap_or([0.; 3]),
        q: q.unwrap_or([0., 0., 0., 1.]),
        s: s.unwrap_or([1.; 3]),
    })
}
fn index(values: &[impl Sized], raw: &RawValue, what: &str) -> FormatResult<usize> {
    at(values, Some(raw)).map_err(|_| invalid(format!("glTF {what} index is invalid")))
}

/// Admit `scene`, `scenes`, `nodes` (every node, selected or not) and select the scene.
pub(super) fn admit(
    root: &records::Root<'_>,
    meshes: &MeshTable,
    ceiling: usize,
) -> FormatResult<Graph> {
    let raw_nodes = array::<&RawValue>(root.nodes, ceiling)?;
    let mut nodes = Vec::new();
    let mut children = Vec::new();
    let mut parent = allocation(raw_nodes.len(), usize::MAX)?;
    for (i, raw) in raw_nodes.iter().enumerate() {
        let node: records::Node<'_> =
            record(Some(raw)).map_err(|_| invalid(format!("glTF node {i} must be an object")))?;
        let start = children.len();
        for child in array::<&RawValue>(node.children, ceiling)
            .map_err(|_| invalid(format!("glTF node {i}: children must be an array")))?
        {
            let child = index(&raw_nodes, child, "child node")?;
            if parent[child] != usize::MAX {
                return Err(invalid(format!(
                    "glTF node {child} has more than one parent"
                )));
            }
            parent[child] = i;
            push(&mut children, child, ceiling)?;
        }
        let mesh = node
            .mesh
            .map(|raw| index(&meshes.meshes, raw, "mesh"))
            .transpose()?;
        push(
            &mut nodes,
            NodeFact {
                children: (start, children.len()),
                mesh,
                skin: node.skin.is_some(),
                weights: node.weights.is_some(),
                transform: transform(&node, i, ceiling)?,
            },
            ceiling,
        )?;
    }
    let scenes = array::<&RawValue>(root.scenes, ceiling)?;
    let mut scene_roots = Vec::new();
    let mut stamp = allocation(nodes.len(), usize::MAX)?;
    for (s, raw) in scenes.iter().enumerate() {
        let scene: records::Scene<'_> =
            record(Some(raw)).map_err(|_| invalid(format!("glTF scene {s} must be an object")))?;
        let mut roots = Vec::new();
        for node in array::<&RawValue>(scene.nodes, ceiling)
            .map_err(|_| invalid(format!("glTF scene {s}: nodes must be an array")))?
        {
            let node = index(&nodes, node, "scene node")?;
            if parent[node] != usize::MAX {
                return Err(invalid(format!(
                    "glTF scene {s}: root node {node} has a parent"
                )));
            }
            if stamp[node] == s {
                return Err(invalid(format!(
                    "glTF scene {s}: root node {node} is listed twice"
                )));
            }
            stamp[node] = s;
            push(&mut roots, node, ceiling)?;
        }
        push(&mut scene_roots, roots, ceiling)?;
    }
    drop(stamp);
    let selected = match root.scene {
        Some(raw) => Some(index(&scenes, raw, "scene")?),
        None if scenes.len() == 1 => Some(0),
        None => None,
    };
    // Every node has at most one parent, so a node unreachable from a parentless
    // node lies on a cycle. Preorder from parentless nodes visits each once.
    let mut order = Vec::new();
    let mut pending = Vec::new();
    for start in (0..nodes.len()).filter(|n| parent[*n] == usize::MAX) {
        push(&mut pending, start, ceiling)?;
        while let Some(n) = pending.pop() {
            push(&mut order, n, ceiling)?;
            let (a, b) = nodes[n].children;
            for &child in children[a..b].iter().rev() {
                push(&mut pending, child, ceiling)?;
            }
        }
    }
    if order.len() != nodes.len() {
        return Err(invalid("glTF node graph contains a cycle"));
    }
    let mut meshes_below = allocation(nodes.len(), false)?;
    for &n in order.iter().rev() {
        let (a, b) = nodes[n].children;
        meshes_below[n] =
            nodes[n].mesh.is_some() || children[a..b].iter().any(|c| meshes_below[*c]);
    }
    let roots = selected.map(|s| std::mem::take(&mut scene_roots[s]));
    Ok(Graph {
        nodes,
        children,
        roots,
        meshes_below,
    })
}

impl Graph {
    /// Exclusion facts of the selected scene. `animations` is the root member.
    pub(super) fn exclusions(
        &self,
        meshes: &MeshTable,
        animations: Raw<'_>,
        ceiling: usize,
        out: &mut ContentExclusions,
    ) -> FormatResult<()> {
        out.animation = animations.is_some_and(|raw| {
            array::<&RawValue>(Some(raw), ceiling).map_or(true, |a| !a.is_empty())
        });
        let Some(roots) = &self.roots else {
            out.scene_selection = true;
            return Ok(());
        };
        let mut pending = Vec::new();
        for &root in roots {
            push(&mut pending, root, ceiling)?;
        }
        while let Some(n) = pending.pop() {
            let node = &self.nodes[n];
            out.skin |= node.skin;
            out.morph_targets |= node.weights;
            if let Some(mesh) = node.mesh {
                let mesh = meshes.meshes[mesh];
                out.morph_targets |= mesh.weights
                    || meshes.primitives[mesh.primitives.0..mesh.primitives.1]
                        .iter()
                        .any(|p| p.targets);
            }
            for &child in &self.children[node.children.0..node.children.1] {
                push(&mut pending, child, ceiling)?;
            }
        }
        Ok(())
    }

    /// Walk the selected scene root-first, children in order. Without a
    /// selected scene nothing is walked.
    pub(super) fn walk<C: SceneConsumer>(
        &self,
        meshes: &MeshTable,
        draws: Draws<'_, '_>,
        consumer: &mut C,
        ceiling: usize,
    ) -> FormatResult<Result<(), C::Error>> {
        let Some(roots) = &self.roots else {
            return Ok(Ok(()));
        };
        let prune = consumer.instances_only();
        let mut stack: Vec<(usize, usize)> = Vec::new();
        for &root in roots {
            if prune && !self.meshes_below[root] {
                continue;
            }
            if let Err(e) = self.visit(root, 1, meshes, draws, consumer) {
                return Ok(Err(e));
            }
            push(&mut stack, (root, self.nodes[root].children.0), ceiling)?;
            while let Some(top) = stack.last_mut() {
                let (n, cursor) = *top;
                if cursor < self.nodes[n].children.1 {
                    top.1 += 1;
                    let child = self.children[cursor];
                    if prune && !self.meshes_below[child] {
                        continue;
                    }
                    let depth = stack.len() as u64 + 1;
                    if let Err(e) = self.visit(child, depth, meshes, draws, consumer) {
                        return Ok(Err(e));
                    }
                    push(&mut stack, (child, self.nodes[child].children.0), ceiling)?;
                } else {
                    stack.pop();
                    consumer.leave(&self.nodes[n].transform);
                }
            }
        }
        Ok(Ok(()))
    }
    fn visit<C: SceneConsumer>(
        &self,
        n: usize,
        depth: u64,
        meshes: &MeshTable,
        draws: Draws<'_, '_>,
        consumer: &mut C,
    ) -> Result<(), C::Error> {
        let node = &self.nodes[n];
        consumer.enter(&node.transform, depth)?;
        if let Some(mesh) = node.mesh {
            let fact = meshes.meshes[mesh];
            consumer.instance(&Instance {
                node: n,
                mesh,
                primitives: &meshes.primitives[fact.primitives.0..fact.primitives.1],
                positions: &meshes.positions[fact.positions.0..fact.positions.1],
                draws,
            })?;
        }
        Ok(())
    }
}

/// Live C1 accessor views shared by every instance of one inspection.
#[derive(Clone, Copy)]
pub(super) struct Draws<'d, 'v> {
    pub accessors: &'d [Accessor],
    pub views: &'d [Cow<'v, [u8]>],
}

/// One core node instance: a selected node with a mesh.
pub(crate) struct Instance<'d, 'v> {
    node: usize,
    mesh: usize,
    primitives: &'d [PrimitiveFact],
    positions: &'d [usize],
    draws: Draws<'d, 'v>,
}

impl Instance<'_, '_> {
    pub(crate) fn node(&self) -> usize {
        self.node
    }
    pub(crate) fn mesh(&self) -> usize {
        self.mesh
    }
    /// Distinct POSITION accessors of this instance's primitives, in
    /// first-appearance order.
    pub(crate) fn positions(&self) -> &[usize] {
        self.positions
    }
    pub(crate) fn count(&self, accessor: usize) -> usize {
        self.draws.accessors[accessor].count
    }
    pub(crate) fn format(&self, accessor: usize) -> PositionFormat {
        let a = &self.draws.accessors[accessor];
        let normalized = a.normalized;
        match a.component {
            5120 => PositionFormat::I8 { normalized },
            5121 => PositionFormat::U8 { normalized },
            5122 => PositionFormat::I16 { normalized },
            5123 => PositionFormat::U16 { normalized },
            _ => PositionFormat::F32,
        }
    }
    /// `T` contribution: the counts of the distinct POSITION accessors.
    pub(crate) fn vertex_total(&self) -> u64 {
        self.positions
            .iter()
            .map(|a| self.draws.accessors[*a].count as u64)
            .sum()
    }
    /// `Idx` contribution: index count, or POSITION count when non-indexed.
    pub(crate) fn index_total(&self) -> u64 {
        self.primitives
            .iter()
            .map(|p| self.draws.accessors[p.indices.unwrap_or(p.position)].count as u64)
            .sum()
    }
    fn primitive_draws(&self, p: &PrimitiveFact, mut mark: impl FnMut(usize) -> bool) -> bool {
        let Some(indices) = p.indices else {
            return (0..self.draws.accessors[p.position].count).any(|i| !mark(i));
        };
        let a = &self.draws.accessors[indices];
        let restart = match a.component {
            5121 => 255,
            5123 => 65535,
            _ => u32::MAX as usize,
        };
        // C1 admitted every non-restart value below the POSITION count. Only
        // core draw indices are read; EXT_mesh_polygon loops are not draws.
        for k in 0..a.count {
            let v = value(a, self.draws.views, k, 0) as usize;
            if v != restart && !mark(v) {
                return true;
            }
        }
        false
    }
    /// Report every drawn vertex index of `accessor` (with repeats).
    pub(crate) fn mark(&self, accessor: usize, mut mark: impl FnMut(usize)) {
        for p in self.primitives.iter().filter(|p| p.position == accessor) {
            self.primitive_draws(p, |v| {
                mark(v);
                true
            });
        }
    }
    /// The first primitive of this mesh that draws `vertex` of `accessor`.
    pub(crate) fn primitive_drawing(&self, accessor: usize, vertex: usize) -> Option<usize> {
        self.primitives
            .iter()
            .position(|p| p.position == accessor && self.primitive_draws(p, |v| v != vertex))
    }
    /// The raw words of one POSITION vertex. Viewless storage is zero.
    pub(crate) fn vertex(&self, accessor: usize, i: usize) -> PositionWord {
        let a = &self.draws.accessors[accessor];
        let Some(view) = a.view else {
            return if a.component == 5126 {
                PositionWord::F32([0.; 3])
            } else {
                PositionWord::Int([0; 3])
            };
        };
        let bytes = &self.draws.views[view];
        let at = |c: usize| a.offset + i * a.stride + a.component_offsets[c];
        match a.component {
            5126 => PositionWord::F32(std::array::from_fn(|c| {
                f32::from_le_bytes(bytes[at(c)..at(c) + 4].try_into().unwrap())
            })),
            5120 => PositionWord::Int(std::array::from_fn(|c| i32::from(bytes[at(c)] as i8))),
            5121 => PositionWord::Int(std::array::from_fn(|c| i32::from(bytes[at(c)]))),
            5122 => PositionWord::Int(std::array::from_fn(|c| {
                i32::from(i16::from_le_bytes(
                    bytes[at(c)..at(c) + 2].try_into().unwrap(),
                ))
            })),
            _ => PositionWord::Int(std::array::from_fn(|c| {
                i32::from(u16::from_le_bytes(
                    bytes[at(c)..at(c) + 2].try_into().unwrap(),
                ))
            })),
        }
    }
}
