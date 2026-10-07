#!/usr/bin/env python3
"""Opt-in full leaf metadata audit against a matching uncompressed LAS source.
Usage: python audit_point_cloud.py source.las output.3tz
No data is downloaded; source coordinates must match the conversion input.
"""
import argparse
import json
import pathlib
import struct
import zipfile

import laspy
import numpy as np


def audit(source, archive):
    types={'UINT8':'<u1','UINT16':'<u2','UINT32':'<u4','UINT64':'<u8',
           'INT8':'<i1','INT16':'<i2','INT32':'<i4','INT64':'<i8','FLOAT32':'<f4','FLOAT64':'<f8'}
    with laspy.mmap(str(source)) as las, zipfile.ZipFile(archive) as z:
        count=las.header.point_count
        seen=np.zeros(count,dtype=np.bool_)
        root=json.loads(z.read('tileset.json'))['root']
        leaves=0; records=0
        def walk(node):
            if node.get('children'):
                for child in node['children']:yield from walk(child)
            else:yield node['content']['uri']
        for uri in walk(root):
            data=z.read(uri);n=struct.unpack_from('<I',data,12)[0]
            doc=json.loads(data[20:20+n]);binary=data[28+n:]
            meta=doc['extensions']['EXT_structural_metadata'];table=meta['propertyTables'][0]
            schema=meta['schema']['classes']['point']['properties']
            def column(name):
                view=doc['bufferViews'][table['properties'][name]['values']]
                return np.frombuffer(binary,types[schema[name]['componentType']],table['count'],view.get('byteOffset',0))
            ids=column('source_index').astype(np.int64)
            if np.any(ids<0) or np.any(ids>=count) or len(np.unique(ids))!=len(ids) or seen[ids].any():
                raise ValueError(f'{uri}: invalid or repeated source indices')
            seen[ids]=True
            for name in schema:
                if name=='source_index':continue
                source_name={'source_x':'x','source_y':'y','source_z':'z'}.get(name,name)
                expected=np.asarray(las[source_name][ids])
                if not np.array_equal(column(name),expected,equal_nan=True):
                    raise ValueError(f'{uri}: {name} differs from source records')
            records+=len(ids);leaves+=1
        if not seen.all():raise ValueError('missing full-detail source records')
        return dict(sourcePoints=count,leafRecords=records,leaves=leaves,allNumericFieldsMatch=True,
                    everySourceIndexExactlyOnce=True)


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('source',type=pathlib.Path);parser.add_argument('archive',type=pathlib.Path)
    args=parser.parse_args();print(json.dumps(audit(args.source,args.archive),indent=2))
