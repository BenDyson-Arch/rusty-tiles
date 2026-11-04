#!/usr/bin/env python3
"""Reproducible four-triangle, five constant 2048 RGBA source witness.

Preserves geometry/material/UV/COLOR provenance from the small low-variance
source, replaces only image signals and resolves all samplers to LINEAR / LML.
Streams PNG rows; truth records constantRGBA rather than bulk pixel arrays.
This is a resource/constant appearance witness, not high-frequency acceptance.
"""
import argparse
from copy import deepcopy
import hashlib
import json
from pathlib import Path
import shutil
import struct
import zlib

COLORS = ((100,150,200,255),(128,190,70,255),(128,128,255,255),
          (220,0,0,255),(20,40,80,255))


def digest(path):
    h=hashlib.sha256()
    with Path(path).open('rb') as source:
        while block:=source.read(1<<20):h.update(block)
    return h.hexdigest()


def chunk(stream,kind,body):
    stream.write(struct.pack('>I',len(body)))
    stream.write(kind);stream.write(body)
    stream.write(struct.pack('>I',zlib.crc32(body,zlib.crc32(kind))))


def constant_png(path,pixel,edge):
    # One row + bounded zlib output at a time, never edge² RGBA ownership.
    row=b'\0'+bytes(pixel)*edge
    compressor=zlib.compressobj(level=6)
    with path.open('wb') as stream:
        stream.write(b'\x89PNG\r\n\x1a\n')
        chunk(stream,b'IHDR',struct.pack('>IIBBBBB',edge,edge,8,6,0,0,0))
        for _ in range(edge):
            block=compressor.compress(row)
            if block:chunk(stream,b'IDAT',block)
        tail=compressor.flush()
        if tail:chunk(stream,b'IDAT',tail)
        chunk(stream,b'IEND',b'')


def generate(base,output,edge=2048):
    if edge!=2048:raise ValueError('Pinned witness edge is2048')
    if output.exists():raise ValueError('Output must be fresh')
    doc=json.loads((base/'source.gltf').read_text())
    truth=json.loads((base/'truth.json').read_text())
    if len(truth['triangles'])!=4 or len(doc['images'])!=5:
        raise ValueError('Base must be the four-triangle/five-image low-variance source')
    if truth.get('codecLowVarianceCandidate') is not True:
        raise ValueError('Base must explicitly identify low-variance candidate provenance')
    base_identity={p.name:digest(p) for p in sorted(base.iterdir()) if p.is_file()}
    output.mkdir(parents=True)
    completed=False
    try:
        for buffer in doc['buffers']:
            uri=buffer['uri']
            if Path(uri).name!=uri:raise ValueError('Witness expects local flat buffer URI')
            shutil.copyfile(base/uri,output/uri)
        images=[]
        for declaration,color in zip(doc['images'],COLORS):
            uri=declaration['uri']
            if Path(uri).name!=uri:raise ValueError('Witness expects flat image URI')
            constant_png(output/uri,color,edge)
            images.append({'uri':uri,'width':edge,'height':edge,'constantRGBA':list(color)})
        for sampler in doc['samplers']:
            sampler['minFilter']=9987;sampler['magFilter']=9729
        (output/'source.gltf').write_text(json.dumps(doc,sort_keys=True,indent=2)+'\n')
        result=deepcopy(truth)
        result.update(images=images,samplers=doc['samplers'],materials=doc['materials'],textures=doc['textures'],
                      signalScope='constant2048x2048 five-source-image resource fixture; no high-frequency appearance claim')
        result['assets']={p.name:digest(p) for p in sorted(output.iterdir()) if p.is_file()}
        result['inputSha256']=result['assets']['source.gltf']
        result['generator']={'path':str(Path(__file__).resolve()),'sha256':digest(__file__),
                             'baseIdentity':base_identity,'baseSourceSha256':base_identity['source.gltf'],
                             'contract':'Geometry/material/UV/COLOR and source provenance retained; only five image signals/min+mag filters changed; streamed constant PNG rows'}
        (output/'truth.json').write_text(json.dumps(result,sort_keys=True,indent=2)+'\n')
        if base_identity!={p.name:digest(p) for p in sorted(base.iterdir()) if p.is_file()}:
            raise ValueError('Immutable base source changed during witness generation')
        completed=True
        return {'output':str(output),'triangles':4,'source_sha256':result['inputSha256'],
                'truth_sha256':digest(output/'truth.json'),'generator_sha256':digest(__file__),
                'assets':result['assets'],'retained_geometry_sha256':result['assets'][doc['buffers'][0]['uri']]}
    finally:
        if not completed:shutil.rmtree(output)  # only the fresh generator-owned directory


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--base',type=Path,required=True)
    parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args()
    print(json.dumps(generate(args.base.resolve(strict=True),args.output.resolve()),indent=2))

if __name__=='__main__':main()
