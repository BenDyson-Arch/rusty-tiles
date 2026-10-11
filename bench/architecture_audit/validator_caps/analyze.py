"""Independent archive facts for the validator caps (stdlib only, no product code).

Usage: python3 -I analyze.py ARCHIVE.3tz [...]

Reports entry count, central-directory bytes (from the end records), source and
stored bytes, member-kind counts, and the largest JSON value-node count of any
JSON document the validator parses (tileset/conversion/subtree JSON, GLB JSON
chunks, b3dm feature/batch-table JSON; vector-build.json is only hashed). Value
nodes follow the C1 contract: the root plus every scalar/container value; object
keys are not counted. payloadAccessorComponents sums count * components over the
accessors of every GLB/b3dm member (the total_payload_elements charge).
"""
import json
import struct
import sys
import zipfile


def nodes(value):
    count, stack = 0, [value]
    while stack:
        item = stack.pop()
        count += 1
        if isinstance(item, dict):
            stack.extend(item.values())
        elif isinstance(item, list):
            stack.extend(item)
    return count


def glb_json(data):
    if data[:4] != b'glTF':
        return []
    length, kind = struct.unpack_from('<II', data, 12)
    return [data[20:20 + length]] if kind == 0x4E4F534A else []


HASH_ONLY = ('vector-build.json',)  # referenced and hash-compared, never parsed


def documents(name, data):
    if name in HASH_ONLY or name.endswith('.jsonl'):
        return []
    if name.endswith('.json'):
        return [data]
    if data[:4] == b'b3dm':
        ftj, ftb, btj, btb = struct.unpack_from('<4I', data, 12)
        at = 28
        docs = [data[at:at + ftj]]
        at += ftj + ftb
        if btj:
            docs.append(data[at:at + btj])
        at += btj + btb
        return docs + glb_json(data[at:])
    if data[:4] == b'subt':
        length = struct.unpack_from('<Q', data, 8)[0]
        return [data[24:24 + length]]
    return glb_json(data)


COMPONENTS = {'SCALAR': 1, 'VEC2': 2, 'VEC3': 3, 'VEC4': 4, 'MAT2': 4, 'MAT3': 9, 'MAT4': 16}


def accessor_components(name, data):
    """C1 total_payload_elements charge of one payload: sum of count * components."""
    if name.endswith('.json') or name.endswith('.jsonl') or data[:4] == b'subt':
        return 0
    if data[:4] == b'b3dm':
        ftj, ftb, btj, btb = struct.unpack_from('<4I', data, 12)
        data = data[28 + ftj + ftb + btj + btb:]
    docs = glb_json(data)
    if not docs:
        return 0
    doc = json.loads(docs[0])
    return sum(a.get('count', 0) * COMPONENTS.get(a.get('type'), 0) for a in doc.get('accessors', []))


def end_records(path):
    with open(path, 'rb') as f:
        f.seek(0, 2)
        size = f.tell()
        f.seek(max(0, size - 65558))
        tail = f.read()
    at = tail.rfind(b'PK\x05\x06')
    entries, cd_size = struct.unpack_from('<HI', tail, at + 10)[0], struct.unpack_from('<I', tail, at + 12)[0]
    loc = tail.rfind(b'PK\x06\x07', 0, at)
    if loc >= 0 and loc + 20 == at:
        z = tail.rfind(b'PK\x06\x06', 0, loc)
        entries, cd_size = struct.unpack_from('<QQ', tail, z + 32)
    return size, entries, cd_size


def analyze(path):
    size, entries, cd_size = end_records(path)
    kinds, stored, max_member, max_doc, max_doc_name, max_doc_bytes = {}, 0, 0, 0, None, 0
    largest = []
    name_bytes = 0
    components = 0
    with zipfile.ZipFile(path) as archive:
        infos = archive.infolist()
        for info in infos:
            name = info.filename
            name_bytes += len(name.encode())
            stored += info.file_size
            max_member = max(max_member, info.file_size)
            largest = sorted(largest + [(info.file_size, name)], reverse=True)[:3]
            kind = name.rsplit('.', 1)[-1] if '.' in name.rsplit('/', 1)[-1] else name
            kinds[kind] = kinds.get(kind, 0) + 1
            data = archive.read(info)
            components += accessor_components(name, data)
            for doc in documents(name, data):
                if not doc.strip():
                    continue
                count = nodes(json.loads(doc))
                max_doc_bytes = max(max_doc_bytes, len(doc))
                if count > max_doc:
                    max_doc, max_doc_name = count, name
    assert entries == len(infos)
    return {'archive': str(path), 'sourceArchiveBytes': size, 'entries': entries,
            'centralDirectoryBytes': cd_size, 'centralDirectoryBytesPerEntry': round(cd_size / entries, 2),
            'meanNameBytes': round(name_bytes / entries, 2), 'storedMemberBytes': stored,
            'maxMemberBytes': max_member, 'largestMembers': largest, 'memberKinds': kinds,
            'maxDocumentValueNodes': max_doc, 'maxDocumentValueNodesMember': max_doc_name,
            'maxJsonDocumentBytes': max_doc_bytes, 'payloadAccessorComponents': components}


if __name__ == '__main__':
    for arg in sys.argv[1:]:
        print(json.dumps(analyze(arg)), flush=True)
