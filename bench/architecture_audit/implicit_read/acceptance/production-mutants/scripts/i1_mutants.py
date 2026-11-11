"""I1 production implementation mutants (contract §D). Each mutant is a list of exact (file, old, new)
edits applied to a clean checkout; `make` writes <name>.patch (git diff) and reverts.
python3 -I i1_mutants.py check|make REPO OUTDIR"""
import json, os, subprocess, sys

A = "src/content_integrity/implicit/availability.rs"
W = "src/content_integrity/implicit/walk.rs"
S = "src/content_integrity/implicit/subtree.rs"
M = "src/content_integrity/implicit/metadata.rs"
I = "src/content_integrity/implicit.rs"
V = "src/validate.rs"

OBJECTS = ("let objects: Vec<Source> = std::iter::once(tile.0)\n"
           "            .chain(contents.iter().map(|c| c.0))\n"
           "            .chain(std::iter::once(child.0))\n"
           "            .collect();\n")

MUTANTS = [
 ("msb-first", "A-3", None, [(A, "bytes[(i / 8) as usize] >> (i % 8) & 1 == 1", "bytes[(i / 8) as usize] >> (7 - i % 8) & 1 == 1")]),
 ("inclusive-rank", "B-1, W1", None, [(A, "        rank(&self.rank, bytes, i)\n    }", "        rank(&self.rank, bytes, i + 1)\n    }")]),
 ("drop-content-implies-tile", "A-6", None, [(A, "            if !holds {", "            if false && !holds {")]),
 ("drop-child-implies-tile", "A-7", None, [(A, "        // 8. child ⇒ tile.\n        if let Some(tile_bytes) = view_bytes(tile) {",
                                              "        // 8. child ⇒ tile.\n        if let Some(tile_bytes) = view_bytes(tile).filter(|_| false) {")]),
 ("drop-available-count", "A-4", None, [(A, "if count.is_some_and(|count| u128::from(count) != popcount(source, bits)) {",
                                          "if false && count.is_some_and(|count| u128::from(count) != popcount(source, bits)) {")]),
 ("key-by-name", "B-3", None, [(M, "        } else if semantic == TILE_BOUNDING_BOX {", "        } else if semantic == TILE_BOUNDING_BOX && id == \"boundingBox\" {"),
                               (M, "        } else if semantic == TILE_GEOMETRIC_ERROR {", "        } else if semantic == TILE_GEOMETRIC_ERROR && id == \"geometricError\" {")]),
 ("ignore-override", "B-4, W6", None, [(W, "        let volume = match live.subtree.columns.bounding_box {", "        let volume = match None::<super::metadata::Column> {")]),
 ("swap-x-y", "C-1, W2", None, [(W, "std::array::from_fn(|k| 2 * coords[k] + ((c >> k) & 1) as u32)",
                                 "std::array::from_fn(|k| 2 * coords[k] + ((c >> [1, 0, 2][k]) & 1) as u32)")]),
 ("error-not-halved", "B-12", None, [(W, "    Ok(root * power_of_two(level))\n}", "    Ok(root)\n}")]),
 ("ge-vs-gt-on-A", "A-8, A-9", None, [(A, "&& u64::from(shape.global) + u64::from(shape.levels) >= u64::from(shape.available)",
                                       "&& u64::from(shape.global) + u64::from(shape.levels) > u64::from(shape.available)")]),
 ("float32-with-float64-stride", "B-6", None, [(M, "            Self::F32 => 4,", "            Self::F32 => 8,")]),
 ("ignore-trailing-bits", "A-5", None, [(A, "if tail != 0 && bytes[bytes.len() - 1] >> tail != 0 {", "if false && tail != 0 && bytes[bytes.len() - 1] >> tail != 0 {")]),
 ("admit-json-templates", "C-6, W5u", None, [(I, "            if normalized.last_segment_is_json() {", "            if false && normalized.last_segment_is_json() {")]),
 ("case-sensitive-suffix", "C-7", None, [(I, "tail.len() >= 5 && tail[tail.len() - 5..].eq_ignore_ascii_case(b\".json\")", "tail.len() >= 5 && &tail[tail.len() - 5..] == b\".json\"")]),
 ("read-scale", "B-7", None, [(M, "    if let Some(key) = [\"offset\", \"scale\", \"noData\", \"default\"]", "    if let Some(key) = [\"offset\", \"noData\", \"default\"]"),
                              (M, "        if let Some(key) = [\"offset\", \"scale\"]", "        if let Some(key) = [\"offset\"]")]),
 ("ignore-content-uri", "C-12", None, [(M, "const UNSUPPORTED: [&str; 15] = [", "const UNSUPPORTED: [&str; 14] = ["), (M, "    \"TILE_REFINE\",\n    \"CONTENT_URI\",\n", "    \"TILE_REFINE\",\n")]),
 ("ignore-content-group-id", "C-13", None, [(M, "const UNSUPPORTED: [&str; 15] = [", "const UNSUPPORTED: [&str; 14] = ["), (M, "    \"CONTENT_URI\",\n    \"CONTENT_GROUP_ID\",\n    \"CONTENT_BOUNDING_BOX\",\n", "    \"CONTENT_URI\",\n    \"CONTENT_BOUNDING_BOX\",\n")]),
 ("drop-repeated-subtree-check", "A-11", None, [(S, "    if limits.subtrees.contains(&name) {", "    if false && limits.subtrees.contains(&name) {")]),
 ("no-source-memoization", "R7 (time bound)", "R7", [(A, "        // 3. One pass per distinct view source: trailing zeros and popcount.\n        for source in &mut distinct {",
     "        // 3. MUTANT: one pass per availability object, not per distinct source.\n        " + OBJECTS +
     "        for source in objects.iter().filter_map(|s| match s {\n            Source::View(d) => Some(*d),\n            _ => None,\n        }) {\n            let source = &mut distinct[source];")]),
 ("scan-per-object", "R7 (ResourceLimit)", None, [(A, "        let scan = distinct\n            .iter()\n            .map(|d| storage.view(d.view).len() as u128)\n            .sum::<u128>();",
     "        " + OBJECTS + "        let scan = objects\n            .iter()\n            .filter_map(|s| match s {\n                Source::View(d) => Some(storage.view(distinct[*d].view).len() as u128),\n                _ => None,\n            })\n            .sum::<u128>();")]),
 ("reservation-after-rank-table", "R5b", None, [
     (A, "        // 9. Visit reservation.\n        if tile_popcount > u128::from(limits.visits_remaining) {\n            return Err(limit(\n                \"subtree tile availability exceeds remaining hierarchy visits\",\n            ));\n        }\n        limits.visits_remaining -= tile_popcount as u64;\n", ""),
     (A, "        // 10 (allocation). The sparse index", "        // 9 (MUTANT: after the rank table). Visit reservation.\n        if u128::from(tile_popcount) > u128::from(limits.visits_remaining) {\n            return Err(limit(\n                \"subtree tile availability exceeds remaining hierarchy visits\",\n            ));\n        }\n        limits.visits_remaining -= tile_popcount;\n        // 10 (allocation). The sparse index")]),
 ("per-visit-charging", "R1b", None, [
     (A, "        // 9. Visit reservation.\n        if tile_popcount > u128::from(limits.visits_remaining) {\n            return Err(limit(\n                \"subtree tile availability exceeds remaining hierarchy visits\",\n            ));\n        }\n        limits.visits_remaining -= tile_popcount as u64;\n",
         "        // 9. MUTANT: no reservation; visits are charged one by one in the walk.\n"),
     (W, "        if self.state == State::Fresh {\n            self.enter(0, [0; 3], limits, read)?;\n            return self.visit().map(Some);",
         "        if self.state == State::Fresh {\n            self.enter(0, [0; 3], limits, read)?;\n            charge(limits)?;\n            return self.visit().map(Some);"),
     (W, "                    )?;\n                    return self.visit().map(Some);", "                    )?;\n                    charge(limits)?;\n                    return self.visit().map(Some);"),
     (W, "                self.enter(level, coords, limits, read)?;\n                return self.visit().map(Some);", "                self.enter(level, coords, limits, read)?;\n                charge(limits)?;\n                return self.visit().map(Some);"),
     (W, "impl<'r> Walk<'r> {", "fn charge<E>(limits: &mut ImplicitLimits) -> Result<(), ImplicitError<E>> {\n    limits.visits_remaining = limits.visits_remaining.checked_sub(1).ok_or_else(|| {\n        ImplicitError::Format(super::limit(\"tile visits exceed remaining hierarchy visits\"))\n    })?;\n    Ok(())\n}\n\nimpl<'r> Walk<'r> {")]),
 ("live-path-current-subtree-only", "R2", None, [(S, "        limits\n            .live_subtree_bytes\n            .checked_sub(*live)", "        limits\n            .live_subtree_bytes\n            .checked_sub(0 * *live)")]),
 ("cumulative-instead-of-live", "R2b", None, [(W, "        self.live -= live.subtree.live_bytes;\n", "")]),
 ("refine-forced-replace", "W7", None, [(V, "            Some(\"ADD\") => Some(\"ADD\"),", "            Some(\"ADD\") => Some(\"REPLACE\"),")]),
 ("padding-0x20-only", "D-8", None, [(S, "    // 2. JSON chunk, admitted exactly as C1 admits it.\n",
     "    // 2. MUTANT: the JSON chunk may be padded with 0x20 only.\n    if member[24..bin_start]\n        .iter()\n        .rev()\n        .take_while(|b| **b != b'}')\n        .any(|b| *b != b' ')\n    {\n        return Err(invalid(format!(\"subtree JSON padding must be spaces: {name}\")).into());\n    }\n")]),
 ("skip-post-walk-content-rule", "C-14", None, [(W, "        if !self.root.contents.is_empty() && self.emitted == 0 {", "        if false && !self.root.contents.is_empty() && self.emitted == 0 {")]),
 ("absolute-1e-6", "T-1", None, [(V, "            let tolerance = 1e-6 * (1. + b[3..].iter().map(|v| v * v).sum::<f64>().sqrt());", "            let tolerance = 1e-6;")]),
 ("detect-on-raw-last-segment", "C-7 DC-2 forms", None, [(I, "            if normalized.last_segment_is_json() {",
     "            let last = raw.rsplit('/').next().unwrap_or(\"\").as_bytes();\n            if last.len() >= 5 && last[last.len() - 5..].eq_ignore_ascii_case(b\".json\") {")]),
 ("resolve-raw-template-per-emission", "R8a (timeout)", "R8a", [
     (I, "pub(crate) struct Template {\n    pieces: Vec<Piece>,\n}", "pub(crate) struct Template {\n    pieces: Vec<Piece>,\n    raw: String,\n    base: String,\n}"),
     (I, "        normalized.into_template()\n    }", "        let mut template = normalized.into_template()?;\n        template.raw = raw.to_owned();\n        template.base = base.to_owned();\n        Ok(template)\n    }"),
     (I, "    fn substitute(&self, level: u32, coords: [u32; 3]) -> Result<String, FormatError> {\n",
         "    fn substitute(&self, level: u32, coords: [u32; 3]) -> Result<String, FormatError> {\n        // MUTANT: resolve the raw template again for every emission.\n        Normalized::resolve(&self.raw, &self.base)?;\n"),
     (I, "        Ok(Template { pieces })", "        Ok(Template {\n            pieces,\n            raw: String::new(),\n            base: String::new(),\n        })")]),
 ("no-variable-survival-check", "C-7c, R8", None, [(I, "        if (0..required).any(|v| !normalized.vars.iter().any(|(_, w)| *w as usize == v)) {",
     "        if false && (0..required).any(|v| !normalized.vars.iter().any(|(_, w)| *w as usize == v)) {")]),
 ("escape-guard-last-segment-only", "C-7b", None, [(I, "        for segment in raw.split('/') {\n            let s = segment.as_bytes();",
     "        for segment in raw.rsplit('/').take(1) {\n            let s = segment.as_bytes();")]),
 ("check-only-content-index", "R9", None, [(A, "        limits.refs_remaining -= entries as u64;\n", "")]),
 ("per-table-class-scan", "R7c (timeout)", "R7c", [(M, "        let cached = self.records.borrow().get(class).cloned();", "        let cached: Option<Rc<ClassRecord>> = None;")]),
 ("header-metadata-not-classified", "C-12b", None, [(I, "            schema\n                .ok_or_else(|| invalid(format!(\"unknown metadata class {class:?}\")))?\n                .admit(class, metadata::Use::Content)?;\n",
     "            let _ = (class, schema); // MUTANT: header class not classified\n")]),
]


def main():
    mode, repo, out = sys.argv[1], sys.argv[2], sys.argv[3]
    os.makedirs(out, exist_ok=True)
    rows = []
    for name, paired, timing, edits in MUTANTS:
        texts = {}
        for f, old, new in edits:
            texts.setdefault(f, open(os.path.join(repo, f)).read())
            n = texts[f].count(old)
            if n != 1:
                raise SystemExit(f"{name}: anchor occurs {n} times in {f}: {old[:70]!r}")
            texts[f] = texts[f].replace(old, new)
        if mode == "make":
            for f, t in texts.items():
                open(os.path.join(repo, f), "w").write(t)
            patch = subprocess.run(["git", "-C", repo, "diff"], capture_output=True, text=True, check=True).stdout
            open(os.path.join(out, name + ".patch"), "w").write(patch)
            subprocess.run(["git", "-C", repo, "checkout", "--", "src"], check=True)
        rows.append({"name": name, "paired": paired, "timingCase": timing})
    json.dump(rows, open(os.path.join(out, "mutants.json"), "w"), indent=1)
    print(len(rows), "mutants", mode)


main()
