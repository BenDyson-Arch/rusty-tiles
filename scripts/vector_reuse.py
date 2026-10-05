"""Versioned subtree reuse and immutable content; changesets stay outside the tiler."""
import bisect
import copy
import hashlib
import json
import pathlib
import re
import zipfile

import numpy as np
from osgeo import gdal


def canonical(value):
    return json.dumps(value,sort_keys=True,separators=(',',':'),allow_nan=False).encode()


def digest(value):
    return hashlib.sha256(canonical(value)).hexdigest()


def contents(node):
    return node.get('contents',[node['content']] if 'content' in node else [])


class Reuse:
    def __init__(self,args,output,encoder):
        self.output=output
        self.previous=getattr(args,'reuse_tileset',None)
        self.encoder=encoder
        self.requested_previous=bool(self.previous)
        self.incompatible_reason=None
        self.old={};self.nodes={};self.records={};self.cuts={}
        self.reused_contents=0;self.reused_tiles=0;self.reused_subtrees=0;self.reused_uris=set()
        if self.previous:
            with zipfile.ZipFile(self.previous) as archive:
                for name in ('tileset.json','vector-build.json'):
                    if archive.getinfo(name).file_size>max(16777216,getattr(args,'max_tiles',100000)*4096):
                        raise ValueError('previous build metadata exceeds configured hierarchy limit')
                manifest=json.loads(archive.read('tileset.json'))
                raw=archive.read('vector-build.json');state=json.loads(raw)
            self.old=state
            if state.get('version')!=1 or len(state.get('records',{}))>getattr(args,'max_tiles',100000):
                raise ValueError('unsupported previous vector build state; run a fresh conversion')
            expected=manifest.get('asset',{}).get('extras',{}).pop('vectorBuildStateSha256',None)
            if not manifest.get('asset',{}).get('extras'):manifest['asset'].pop('extras',None)
            if expected!=hashlib.sha256(raw).hexdigest() or state.get('manifestSha256')!=digest(manifest):
                raise ValueError('previous manifest/build state integrity check failed')
            if state.get('config',{}).get('where')!=getattr(args,'where',None):
                self.incompatible_reason='attribute filter changed'
                self.previous=None;self.old={}
                return
            anchor=np.asarray(state['anchor'],dtype=float);frame=np.asarray(state['frame'],dtype=float)
            if anchor.shape!=(3,) or frame.shape!=(3,3) or not np.isfinite(anchor).all() or not np.isfinite(frame).all() or not np.allclose(frame@frame.T,np.eye(3),atol=1e-12):
                raise ValueError('invalid previous local frame')
            args._reuse_anchor=anchor;args._reuse_frame=frame
            def index(node):
                path=node.get('extras',{}).get('buildPath')
                if path is not None:
                    if path in self.nodes:raise ValueError('duplicate cached subtree path')
                    self.nodes[path]=node
                for child in node.get('children',[]):index(child)
            index(manifest['root'])
            self.paths=sorted(state['records'])
            self.cuts={path:record['cut'] for path,record in state['records'].items() if record.get('cut')}

    def configure(self,args,reader):
        self.config=dict(encoder=self.encoder,gdal=gdal.VersionInfo(),numpy=np.__version__,driver=reader.driver,
            schemas=reader.schemas,layers=[{k:v for k,v in layer.items() if k not in ('features','invalidFeatures','featuresWithoutGeometry')} for layer in reader.layer_reports],
            maxFeatures=args.max_features,maxParentFeatures=getattr(args,'max_parent_features',4096),maxVertices=getattr(args,'max_vertices',65536),
            maxBytes=getattr(args,'max_bytes',4194304),lodTolerance=getattr(args,'lod_tolerance',.1),
            lodLevels=getattr(args,'lod_levels',3),skipInvalid=getattr(args,'skip_invalid',False),repair=getattr(args,'repair',False),
            ambiguousOutlines=getattr(args,'ambiguous_outlines',False),sourceCrs=getattr(args,'source_crs',None),
            where=getattr(args,'where',None),heightOffset=getattr(args,'height_offset',None),listFields=getattr(args,'list_fields','error'),
            fields=getattr(args,'fields',[]) or [],dropFields=getattr(args,'drop_fields',[]) or [])
        if self.previous and self.old.get('config')!=self.config:
            raise ValueError('previous encoder, schema, CRS or conversion settings differ; run a fresh conversion without reuseTileset')
        self.configuration_hash=digest(self.config)

    def signature(self,db,path):
        hasher=hashlib.sha256(self.configuration_hash.encode())
        for value, in db.execute('SELECT fingerprint FROM features WHERE path>=? AND path<? ORDER BY fingerprint',(path,path+'~')):
            hasher.update(bytes.fromhex(value))
        return hasher.hexdigest()

    def restore(self,path,signature,counters,current_fragments,max_tiles):
        record=self.old.get('records',{}).get(path)
        if not record or record['signature']!=signature:return None
        if path not in self.nodes:raise ValueError('cached subtree is absent from previous manifest')
        node=copy.deepcopy(self.nodes[path]);node.pop('transform',None)
        node['_center']=np.asarray(record['center']);node['_padding']=record['padding']
        with zipfile.ZipFile(self.previous) as archive:
            def visit(value):
                counters['tiles']+=1
                if counters['tiles']>max_tiles:raise ValueError('reused hierarchy exceeds maxTiles')
                self.reused_tiles+=1
                if not value.get('children') and contents(value):counters['leafTiles']+=1
                if value.get('extras',{}).get('routing'):counters['routingTiles']+=1
                for content in contents(value):
                    uri=content['uri']
                    if not re.fullmatch(r't/[0-9a-f]{64}\.(glb|b3dm)',uri):
                        raise ValueError('previous content URI is not an immutable vector content path')
                    target=self.output/uri;data=archive.read(uri)
                    if hashlib.sha256(data).hexdigest()!=pathlib.Path(uri).stem:
                        raise ValueError(f'previous content checksum failed: {uri}')
                    if not target.exists():target.write_bytes(data)
                    self.reused_contents+=1;self.reused_uris.add(uri)
                if contents(value):
                    counters['maximumTileVertices']=max(counters['maximumTileVertices'],value['extras']['vertices'])
                    counters['maximumTileBytes']=max(counters['maximumTileBytes'],value['extras']['encodedBytes'])
                for child in value.get('children',[]):visit(child)
            visit(node)
        start=bisect.bisect_left(self.paths,path);end=bisect.bisect_left(self.paths,path+'~')
        for key in self.paths[start:end]:self.records[key]=self.old['records'][key]
        counters['fragments']+=record['fragments']-current_fragments
        self.reused_subtrees+=1
        return node

    def remember(self,path,signature,node,fragments,cut=None):
        node.setdefault('extras',{})['buildPath']=path
        self.records[path]=dict(signature=signature,center=node['_center'].tolist(),padding=node['_padding'],
                                fragments=fragments,cut=cut)

    def publish(self,manifest,reader):
        # Remove candidate encodings omitted from the final hierarchy. This also
        # makes an archive's immutable content set exactly match its manifest.
        used=set()
        def visit(node):
            used.update(c['uri'] for c in contents(node))
            for child in node.get('children',[]):visit(child)
        visit(manifest['root'])
        for file in (self.output/'t').iterdir():
            if 't/'+file.name not in used:file.unlink()
        state=dict(version=1,config=self.config,anchor=reader.anchor.tolist(),frame=reader.frame.tolist(),
                   records=self.records,manifestSha256=digest(manifest))
        raw=canonical(state)
        (self.output/'vector-build.json').write_bytes(raw)
        manifest['asset']['extras']=dict(vectorBuildStateSha256=hashlib.sha256(raw).hexdigest())
        return dict(previousTileset=bool(self.previous),requestedPreviousTileset=self.requested_previous,incompatibleReason=self.incompatible_reason,reusedSubtrees=self.reused_subtrees,
                    reusedTiles=self.reused_tiles,reusedContents=len(self.reused_uris),reusedContentReferences=self.reused_contents,
                    publishedContents=len(used),rebuiltContents=len(used-self.reused_uris))
