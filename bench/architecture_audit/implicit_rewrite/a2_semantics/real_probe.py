#!/usr/bin/env python3
"""Bounded raw-document/frame audit on current authored producer archives.

Imports this lane's independent binary decoder; never imports production or
the other lane's oracle. Does not decode compressed/quantized geometry.
"""
import argparse
from fractions import Fraction as F
import hashlib
import json
from pathlib import Path
import time
import zipfile
from probe import decode_subtree, require, corners, shifted, slot, resolve, multiply, IDENTITY

LABELS = ['vector-flat-rounded', 'vector-quantized', 'vector-compressed',
          'vector-quantized-compressed', 'vector-deep-tight', 'vector-array-b3dm',
          'vector-fragmented-line', 'point-flat-rounded', 'point-deep-tight',
          'point-duplicate-midpoint', 'shared-relative-resources']
HERE = Path(__file__).resolve().parent


def capture(path):
    require(path.stat().st_size < 16*1024*1024, 'probe archive ceiling')
    data = path.read_bytes()
    with zipfile.ZipFile(path) as archive:
        infos = archive.infolist()
        require(len(infos) < 256 and sum(x.file_size for x in infos) < 16*1024*1024, 'probe decoded ceiling')
        members = {x.filename: archive.read(x) for x in infos}
    return members, hashlib.sha256(data).hexdigest()


def audit(source, output, scheme):
    explicit = json.loads(source['tileset.json'])
    axes = 3 if scheme == 'OCTREE' else 2
    nodes = []
    edges = []
    repeated_slots = []
    def walk(n, path, frame, inherited):
        require(len(nodes) < 64 and len(path) < 8, 'probe tree ceiling')
        effective = n.get('refine', inherited)
        require(effective == 'REPLACE', 'source effective refinement')
        frame = multiply(frame, n.get('transform', IDENTITY))
        nodes.append((path, n, frame))
        b = n['boundingVolume']['box']
        used = set()
        for i, c in enumerate(n.get('children', [])):
            delta = c.get('transform', IDENTITY)[12:15]
            cb = c['boundingVolume']['box']
            center_slot = sum((F(cb[j])+F(delta[j]) >= F(b[j])) << j for j in range(axes))
            if center_slot in used:
                repeated_slots.append({'parent_path': list(path), 'child_index': i, 'slot': center_slot})
            used.add(center_slot)
            excess = []
            for j in range(3):
                half = F(b[3+4*j])/(2 if j < axes else 1)
                center = F(b[j])+(half if center_slot & (1 << j) else -half) if j < axes else F(b[j])
                excess.append(abs(F(cb[j])+F(delta[j])-center)+F(cb[3+4*j])-half)
            edges.append({'parent_path': list(path), 'child_index': i, 'center_slot': center_slot,
                          'local_cell_excess_exact': [str(x) for x in excess],
                          'exact_cell_supported': all(x <= 0 for x in excess),
                          'recorded_rounding': c.get('extras', {}).get('positionRoundingMetres'),
                          'recorded_quantization': c.get('extras', {}).get('quantizationErrorMetres')})
            walk(c, path+(i,), frame, effective)
    walk(explicit['root'], (), IDENTITY, None)
    report = {'source_nodes': len(nodes), 'edges': edges,
              'exact_cell_supported': all(e['exact_cell_supported'] for e in edges),
              'unique_local_center_slots': not repeated_slots,
              'repeated_local_slots': repeated_slots,
              'geometry_decoded': False, 'output_present': output is not None}
    if output is None:
        return report
    visits = []
    def paired(n, doc, path, parent_frame):
        root = doc['root']
        tiling = root['implicitTiling']
        require(tiling['subdivisionScheme'] == scheme, 'declared scheme')
        frame = multiply(parent_frame, root.get('transform', IDENTITY))
        source_frame = multiply(parent_frame, n.get('transform', IDENTITY))
        require(frame == source_frame, 'root transform exactly once')
        require(root['boundingVolume']['box'] == n['boundingVolume']['box'], 'root header bounds')
        require(root['geometricError'] == n['geometricError'], 'root header error')
        subtree_uri = resolve(tiling['subtrees']['uri'], 0, 0, 0, 0)
        tiles, flags, metadata = decode_subtree(output[subtree_uri], root, doc['schema'])
        require(metadata['TILE_BOUNDING_BOX'][0] == [F(x) for x in n['boundingVolume']['box']], 'root metadata bounds')
        require(metadata['TILE_GEOMETRIC_ERROR'][0] == [F(n['geometricError'])], 'root metadata error')
        contents = root.get('contents', [root['content']] if 'content' in root else [])
        original = n.get('contents', [n['content']] if 'content' in n else [])
        children = n.get('children', [])
        require(len(contents) == len(original)+bool(children), 'content slot inventory')
        require(sum(tiles) == 1+len(children), 'available node inventory')
        for i, content in enumerate(original):
            require(flags[i][0] == 1 and sum(flags[i]) == 1, 'original root availability')
            uri = resolve(contents[i]['uri'], 0, 0, 0, 0)
            require(output[uri] == source[content['uri']], 'original payload alias bytes')
        if children:
            require(flags[-1][0] == 0 and flags[-1][1:] == tiles[1:], 'only terminal links')
        available_slots = [i-1 for i in range(1, len(tiles)) if tiles[i]]
        remaining = list(children)
        for emitted_slot in available_slots:
            index = 1+emitted_slot
            rank = sum(tiles[:index])
            b = metadata['TILE_BOUNDING_BOX'][rank]
            candidates = [c for c in remaining if b == shifted(c['boundingVolume']['box'], c.get('transform', IDENTITY)[12:15])]
            require(len(candidates) == 1, 'unique link box matching source child')
            child = candidates[0]
            remaining.remove(child)
            require(metadata['TILE_GEOMETRIC_ERROR'][rank] == [F(child['geometricError'])], 'terminal error preservation')
            delta = child.get('transform', IDENTITY)[12:15]
            local_slot = sum((F(child['boundingVolume']['box'][j])+F(delta[j]) >= F(n['boundingVolume']['box'][j])) << j for j in range(axes))
            require(corners(b, frame) == corners(child['boundingVolume']['box'], multiply(frame, child.get('transform', IDENTITY))), 'link parent frame')
            uri = resolve(contents[-1]['uri'], 1, emitted_slot & 1, (emitted_slot >> 1) & 1, (emitted_slot >> 2) & 1 if axes == 3 else 0)
            visits.append({'path': list(path), 'emitted_slot': emitted_slot, 'local_center_slot': local_slot,
                           'slot_agrees': emitted_slot == local_slot})
            paired(child, json.loads(output[uri]), path+(children.index(child),), frame)
        require(not remaining, 'all child ownership retained')
    paired(explicit['root'], json.loads(output['tileset.json']), (), IDENTITY)
    report['paired_output'] = {'all_payload_aliases_equal': True, 'all_link_frames_errors_equal': True,
                               'all_local_center_slots_agree': all(v['slot_agrees'] for v in visits), 'links': visits}
    return report


def main():
    p = argparse.ArgumentParser()
    p.add_argument('artifact_directory', type=Path)
    p.add_argument('--write', action='store_true')
    args = p.parse_args()
    started = time.monotonic()
    raw_receipt = (args.artifact_directory/'receipt.json').read_bytes()
    upstream_receipt = json.loads(raw_receipt)
    pins = upstream_receipt['pins']
    require(pins['probe_sha256'] == '51129b3f41f10722c01c76def351b337c043b77b310ac3c17d6a2b4f8d66869d', 'final upstream driver identity')
    own_source = json.loads((HERE/'results.json').read_text())['inspected_source_sha256']
    require(pins['production_hash_match'], 'upstream production identity mismatch')
    require(all(pins['production_sha256'][k] == v for k, v in own_source.items() if k.startswith('src/')), 'paired production bytes differ from semantic audit')
    results = {'kind': 'current-producer-independent-document-audit', 'cases': {},
               'upstream_execution': {
                   'receipt_sha256': hashlib.sha256(raw_receipt).hexdigest(),
                   'probe_sha256': pins['probe_sha256'],
                   'binary_sha256': pins['binary_sha256'],
                   'frozen_source_commit': pins['frozen_source_commit'],
                   'frozen_manifest_sha256': pins['frozen_manifest_sha256'],
                   'relevant_production_bytes_match_semantic_audit': True}}
    for label in LABELS:
        source, source_hash = capture(args.artifact_directory/(label+'.3tz'))
        out_path = args.artifact_directory/(label+'-implicit.3tz')
        output, output_hash = capture(out_path) if out_path.exists() else (None, None)
        require(upstream_receipt['generated_artifacts'][label+'.3tz'] == source_hash, 'upstream source archive identity')
        if output is not None:
            require(upstream_receipt['generated_artifacts'][label+'-implicit.3tz'] == output_hash, 'upstream output archive identity')
        # Scheme comes from authored fixture intent, not imported producer labels.
        scheme = 'OCTREE' if label.startswith('point-') else 'QUADTREE'
        results['cases'][label] = dict(audit(source, output, scheme), source_archive_sha256=source_hash, output_archive_sha256=output_hash, declared_probe_scheme=scheme)
    require(time.monotonic()-started < 10, 'probe time ceiling')
    results['driver_sha256'] = hashlib.sha256(Path(__file__).read_bytes()).hexdigest()
    results['semantic_decoder_sha256'] = hashlib.sha256((HERE/'probe.py').read_bytes()).hexdigest()
    results['limits'] = {'max_seconds': 10, 'max_archive_bytes': 16777216,
                         'max_decoded_bytes': 16777216, 'max_members': 255, 'max_nodes': 64, 'max_depth': 7}
    if args.write:
        (HERE/'real-results.json').write_text(json.dumps(results, indent=2, sort_keys=True)+'\n')
    else:
        require(results == json.loads((HERE/'real-results.json').read_text()), 'real result mismatch')
    print(json.dumps({'status': 'pass', 'cases': len(results['cases']),
                     'outputs_checked': sum(v['output_present'] for v in results['cases'].values()),
                     'exact_cell_sources': sum(v['exact_cell_supported'] for v in results['cases'].values())}))


if __name__ == '__main__':
    main()
