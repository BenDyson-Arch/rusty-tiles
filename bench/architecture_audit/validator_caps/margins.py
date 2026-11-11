"""Per-tile ratios of default vector output and margins at 100,000 tiles.

Usage: python3 -I margins.py RESULTS_DIR > margins.json

Reads analyze.jsonl (independent archive facts) and vector.json (wide-probe
counters from the measurement-only probe build) for the implicit vector
archives, takes the largest per-tile ratio of each consumed fact within each
content class, scales it to 100,000 tiles and divides each ceiling (old and
new) by that demand.
"""
import json
from pathlib import Path
import sys

TILES = 100_000
IMPLICIT = ['s1-mf2.3tz', 'points-78k-mf2.3tz', 'points-81k-mf2.3tz', 'points-90k-mf2-t130k.3tz',
            'points-165k-mf2-t300k.3tz', 'mixed-s7.8-mf2.3tz']
# Content classes: invented points (light payloads) and the perf-lane mixed composition.
CORPORA = {'points': [n for n in IMPLICIT if n.startswith('points')],
           'mixed': ['s1-mf2.3tz', 'mixed-s7.8-mf2.3tz']}
MiB = 1 << 20
# fact: (ceiling field, old, new, source)
FACTS = {
    'entries': ('archive_entries', 65536, 1 << 19, 'probe'),
    'expansionNodes': ('hierarchy_visits (implicit presentation nodes)', 65536, 1 << 18, 'probe'),
    'hierarchyVisits': ('hierarchy_visits (traversal tasks)', 65536, 1 << 18, 'probe'),
    'references': ('references', 262144, 1 << 20, 'probe'),
    'centralDirectoryBytes': ('central_directory_bytes', 16 * MiB, 32 * MiB, 'analyze'),
    'maxDocumentValueNodes': ('document_items (tileset.json)', 65536, 1 << 17, 'analyze'),
    'maxJsonDocumentBytes': ('json_bytes (tileset.json, unchanged)', 8 * MiB, 8 * MiB, 'analyze'),
    'maxMemberBytes': ('member_bytes (vector-build.json)', 64 * MiB, 128 * MiB, 'analyze'),
    'expansionBytes': ('document_decoded_bytes (implicit presentation bytes)', 64 * MiB, 128 * MiB, 'probe'),
    'sourceArchiveBytes': ('source_archive_bytes (unchanged)', 1 << 30, 1 << 30, 'analyze'),
    'storedMemberBytes': ('archive_stored_bytes (unchanged)', 1 << 30, 1 << 30, 'analyze'),
    'bytesReadAfterHierarchy': ('total_bytes_read (unchanged)', 1 << 31, 1 << 31, 'probe'),
    'payloadAccessorComponents': ('total_payload_elements (unchanged)', 16_000_000, 16_000_000, 'analyze'),
}


def main():
    results = Path(sys.argv[1])
    analyze = {Path(d['archive']).name: d for d in map(json.loads, (results / 'analyze.jsonl').read_text().splitlines())}
    probe = {r['archive']: r['runs'][0]['probe'][0] for r in json.loads((results / 'vector.json').read_text())
             if r['label'] == 'wide-probe' and r['runs'][0]['probe']}
    tiles = {name: probe[name]['tiles'] for name in IMPLICIT}
    out = {'tiles': TILES, 'archives': tiles, 'facts': {}}
    for fact, (field, old, new, source) in FACTS.items():
        values = {name: (probe if source == 'probe' else analyze)[name][fact] for name in IMPLICIT}
        ratios = {name: values[name] / tiles[name] for name in IMPLICIT}
        row = {'ceiling': field, 'old': old, 'new': new, 'perTile': {n: round(r, 4) for n, r in ratios.items()}}
        for corpus, names in CORPORA.items():
            worst = max(names, key=ratios.get)
            demand = ratios[worst] * TILES
            row[corpus] = {'worstArchive': worst, 'demandAt100k': round(demand),
                           'oldMargin': round(old / demand, 3), 'newMargin': round(new / demand, 3)}
        out['facts'][fact] = row
    print(json.dumps(out, indent=1))


if __name__ == '__main__':
    main()
