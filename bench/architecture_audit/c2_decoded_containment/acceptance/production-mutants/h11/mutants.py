"""H11 corner fast-path production mutants.

Usage (repository root, clean `src/`):
    C2M_LOCK=... C2M_TARGET=... C2M_CPUS=... C2M_JOBS=4 \\
    python3 -I bench/architecture_audit/c2_decoded_containment/acceptance/production-mutants/h11/mutants.py \\
        OUT_DIR [--evidence DIR] [--forest DIR] [name ...]

Same driver as `../slice3/mutants.py` (exact single-occurrence string edits of `src/`, the release
parity suites, `<name>.patch` and `report.json` written next to this file), with the H11 rows. Each
mutant breaks the corner fast path in one way and is paired with an oracle-decided H11 control
(`h11/generate.py`) that it must flip, or, for the work bound, with the unit test that must fail.
The H11 unit tests are added to the run (`C2M_EXTRA_TESTS`).
"""
import importlib.util
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
os.environ.setdefault("C2M_EXTRA_TESTS", " ".join([
    "validate::containment::tests::corner_walks_stay_within_the_credit_bound",
    "validate::containment::tests::walk_marked_equals_the_per_vertex_walk",
    "validate::containment::tests::diagonal_corners_only_for_axis_aligned_box_chains",
    "validate::containment::tests::signed_clamp_and_first_outside_survive_the_fast_path",
    "validate::containment::tests::non_finite_words_take_the_per_vertex_walk",
    "validate::containment_controls::declared_accessor_bounds_cannot_hide_an_escape",
]))
_spec = importlib.util.spec_from_file_location(
    "slice3_driver", os.path.join(os.path.dirname(HERE), "slice3", "mutants.py"))
driver = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(driver)

C = "src/validate/containment.rs"
V = "src/validate.rs"
P = "src/content_integrity/payload.rs"
SC = "src/content_integrity/payload/scene.rs"

WALK_MARKED = (
    "pub(crate) fn walk_marked(\n"
    "    arena: &mut Arena,\n"
    "    n: Normalization,\n"
    "    levels: &[LevelFacts],\n"
    "    bitmap: &[u64],\n"
    "    credit: &mut Credit,\n"
    "    vertex: impl Fn(usize) -> RawVertex,\n"
    ") -> Option<(usize, Verdict)> {\n"
    "    let diagonal"
)
CALL = (
    "                containment::walk_marked(self.arena, n, self.levels, bitmap, self.credit, |v| {\n"
    "                    raw_vertex(instance.vertex(accessor, v))\n"
    "                });\n"
)
BOX_LOOP = (
    "            let v = 64 * (first + w) + bits.trailing_zeros() as usize;\n"
    "            bits &= bits - 1;\n"
    "            extent = Some(match (extent, vertex(v)) {\n"
)

H11 = [
    {"name": "two-diagonal-corners", "paired": "h11-sphere-offdiagonal.mutated",
     "row": "Wrong corner math: always test only the all-lo and all-hi corners (no axis-aligned/box condition)",
     "edits": [(C, "    let diagonal = diagonal_suffices(arena, levels);\n", "    let diagonal = true;\n")]},
    {"name": "ignore-tile-step", "paired": "h11-rotated-tile-step.mutated",
     "row": "Wrong corner math: the diagonal rule ignores the tile steps (a rotated tile transform over boxes "
            "tests two corners)",
     "edits": [(C, "            let step = level.to_parent.filter(|_| l > 0).is_none_or(|t| {\n"
                   "                (0..3).all(|r| (0..3).filter(|&c| t[4 * c + r] != 0.0).count() <= 1)\n"
                   "            });\n"
                   "            step && level\n",
                "            level\n")]},
    {"name": "inward-corner", "paired": "h11-ulp-last-vertex.mutated",
     "row": "Inward rounding: each varying raw-box bound moves one f32 ulp / one integer step inward",
     "edits": [(C, "    extent\n}\n/// Whether every tested quantity",
                "    extent.map(|(lo, hi)| match (lo, hi) {\n"
                "        (RawVertex::F32(l), RawVertex::F32(h)) => (\n"
                "            RawVertex::F32(std::array::from_fn(|c| if l[c] < h[c] { l[c].next_up() } else { l[c] })),\n"
                "            RawVertex::F32(std::array::from_fn(|c| if l[c] < h[c] { h[c].next_down() } else { h[c] })),\n"
                "        ),\n"
                "        (RawVertex::Int(l), RawVertex::Int(h)) => (\n"
                "            RawVertex::Int(std::array::from_fn(|c| if l[c] < h[c] { l[c] + 1 } else { l[c] })),\n"
                "            RawVertex::Int(std::array::from_fn(|c| if l[c] < h[c] { h[c] - 1 } else { h[c] })),\n"
                "        ),\n"
                "        other => other,\n"
                "    })\n}\n/// Whether every tested quantity")]},
    {"name": "skip-fallback", "paired": "h11-sphere-offdiagonal.mutated",
     "row": "Skipped fallback: an inconclusive corner test skips its chunk",
     "edits": [(C, "                if contained {\n                    credit.skips += 1;\n",
                "                if contained || credit.tests > 0 {\n                    credit.skips += 1;\n")]},
    {"name": "trust-declared-bounds", "paired": "h11-declared-bounds.mutated",
     "row": "Trust unverified accessor min/max: the box comes from the declared bounds (viewless "
            "declarations kept as C1 extents) instead of the marked words",
     "edits": [
         (P, "        parsed.min = min;\n        parsed.max = max;\n",
          "        parsed.min = min;\n        parsed.max = max;\n"
          "        if view.is_none() {\n"
          "            for (bound, out) in [(a.min, &mut parsed.min), (a.max, &mut parsed.max)] {\n"
          "                if let Some(bound) = bound {\n"
          "                    let declared = array::<&RawValue>(Some(bound), slots)?;\n"
          "                    for (raw, o) in declared.iter().zip(out.iter_mut()) {\n"
          "                        *o = serde_json::from_str::<f64>(raw.get()).unwrap_or(*o);\n"
          "                    }\n"
          "                }\n"
          "            }\n"
          "        }\n"),
         (SC, "    pub(crate) fn count(&self, accessor: usize) -> usize {\n",
          "    pub(crate) fn extent(&self, accessor: usize) -> ([f64; 3], [f64; 3]) {\n"
          "        let a = &self.draws.accessors[accessor];\n"
          "        (std::array::from_fn(|c| a.min[c]), std::array::from_fn(|c| a.max[c]))\n"
          "    }\n"
          "    pub(crate) fn count(&self, accessor: usize) -> usize {\n"),
         (C, WALK_MARKED,
          WALK_MARKED.replace("    let diagonal",
                              "    walk_marked_declared(arena, n, levels, bitmap, credit, vertex, None)\n}\n")
          + WALK_MARKED.replace("pub(crate) fn walk_marked(", "pub(crate) fn walk_marked_declared(")
          .replace("    vertex: impl Fn(usize) -> RawVertex,\n",
                   "    vertex: impl Fn(usize) -> RawVertex,\n    declared: Option<(RawVertex, RawVertex)>,\n")),
         (C, "            .then(|| raw_box(words, first, &vertex))\n",
          "            .then(|| declared.or_else(|| raw_box(words, first, &vertex)))\n"),
         (V, CALL,
          "                containment::walk_marked_declared(self.arena, n, self.levels, bitmap, self.credit, |v| {\n"
          "                    raw_vertex(instance.vertex(accessor, v))\n"
          "                }, {\n"
          "                    let (lo, hi) = instance.extent(accessor);\n"
          "                    let word = |p: [f64; 3]| match instance.format(accessor) {\n"
          "                        PositionFormat::F32 => RawVertex::F32(p.map(|x| x as f32)),\n"
          "                        _ => RawVertex::Int(p.map(|x| x as i32)),\n"
          "                    };\n"
          "                    Some((word(lo), word(hi)))\n"
          "                });\n"),
     ]},
    {"name": "box-skips-last-vertex", "paired": "h11-ulp-last-vertex.mutated",
     "row": "Unverified box: the raw box misses the chunk's last marked vertex",
     "edits": [(C, BOX_LOOP, BOX_LOOP.replace(
         "            extent = Some(",
         "            if bits == 0 && words[w + 1..].iter().all(|&x| x == 0) {\n                break;\n            }\n"
         "            extent = Some("))]},
    {"name": "report-first-marked-vertex", "paired": "h11-inconclusive-sphere.base",
     "row": "No fallback fidelity: an inconclusive corner test reports the chunk's first marked vertex",
     "edits": [(C, "                    contained = walk(arena, n, corner, levels) == Verdict::Contained;\n",
                "                    let verdict = walk(arena, n, corner, levels);\n"
                "                    contained = verdict == Verdict::Contained;\n"
                "                    if !contained && !arena.fault() {\n"
                "                        let w = words.iter().position(|&b| b != 0).unwrap();\n"
                "                        return Some((64 * (first + w) + words[w].trailing_zeros() as usize, verdict));\n"
                "                    }\n")]},
    {"name": "credit-ignored", "paired": "corner_walks_stay_within_the_credit_bound",
     "row": "Work bound: corner tests run without the T/64 credit check",
     "edits": [(C, "        let extent = (marked > 1 && credit.covers(1))\n", "        let extent = (marked > 1)\n"),
               (C, "            if count < marked && credit.covers(count) {\n                credit.spend(count);\n",
                "            if count < marked {\n"
                "                credit.left = credit.left.saturating_sub(count * CREDIT_DIVISOR);\n")]},
    {"name": "chunk-offset-ignored", "paired": "h11-third-chunk.mutated",
     "row": "Chunk offset: the raw box of every chunk reads the first chunk's vertices",
     "edits": [(C, BOX_LOOP, BOX_LOOP.replace("64 * (first + w)", "64 * w"))]},
]

driver.HERE = HERE
driver.MUTANTS = [{"name": "clean", "edits": [], "paired": None, "row": "no edit: must flip nothing"}] + H11

if __name__ == "__main__":
    driver.main(sys.argv[1:])
