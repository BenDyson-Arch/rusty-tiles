#!/usr/bin/env python3
"""Independently authored tiny stored ZIP/ZIP64/3TZ records and framing oracle.

No product imports, zipfile reader, producer, build or coordinator fixture.
Fixture bytes are written only to a fresh external /tmp directory. Frozen CLI
comparison is optional, bounded, nice10 and serial; it is not the format oracle.
"""
import argparse
import base64
import binascii
import copy
import hashlib
import io
import json
import os
from pathlib import Path
import struct
import subprocess

HERE=Path(__file__).resolve().parent
INDEX='@3dtilesIndex1@'
LSIG=b'PK\x03\x04';CSIG=b'PK\x01\x02';DSIG=0x08074b50
FROZEN=Path('/home/bend/.cache/rusty-tiles-f1d2-evidence/rusty-tiles-portable-4553533')
FROZEN_SHA='aa71fd57614844488d336dbc991df551fd9294c6d31d82027e1b4a43aedbf64b'


class Failure(Exception):
    def __init__(self,kind,reason):self.kind=kind;self.reason=reason;super().__init__(reason)


def require(ok,reason,kind='invalid_input'):
    if not ok:raise Failure(kind,reason)


def sha(data):return hashlib.sha256(data).hexdigest()
def crc(data):return binascii.crc32(data)&0xffffffff
def tlv(tag,data):return struct.pack('<HH',tag,len(data))+data
def md5key(name):
    h=hashlib.md5(name.encode()).digest();return struct.unpack('<QQ',h)


def point_glb(resources=()):
    d={'asset':{'version':'2.0'},'scene':0,'scenes':[{'nodes':[0]}],
       'nodes':[{'mesh':0}],'meshes':[{'primitives':[{'attributes':{'POSITION':0},'mode':0}]}],
       'buffers':[{'byteLength':12}]+[{'uri':n,'byteLength':len(b)} for n,b in resources],
       'bufferViews':[{'buffer':0,'byteLength':12}],
       'accessors':[{'bufferView':0,'componentType':5126,'type':'VEC3','count':1,'min':[0,0,0],'max':[0,0,0]}]}
    j=json.dumps(d,separators=(',',':')).encode();j+=b' '*(-len(j)%4);binary=struct.pack('<3f',0,0,0)
    return struct.pack('<4sII',b'glTF',2,28+len(j)+len(binary))+struct.pack('<I4s',len(j),b'JSON')+j+struct.pack('<I4s',12,b'BIN\0')+binary


MANIFEST=json.dumps({'asset':{'version':'1.1'},'geometricError':0,'root':{'boundingVolume':{'box':[0,0,0,1,0,0,0,1,0,0,0,1]},'geometricError':0,'refine':'REPLACE','content':{'uri':'point.glb'}}},separators=(',',':')).encode()


class Record:
    def __init__(self,name,data,descriptor=None,zlocal=False,zcentral=False,lextra=b'',cextra=b'',zoffset=False):
        self.name=name;self.data=data;self.descriptor=descriptor;self.zlocal=zlocal;self.zcentral=zcentral
        self.lextra=lextra;self.cextra=cextra;self.offset=None;self.zoffset=zoffset
    def local(self):
        flags=8 if self.descriptor else 0;n=self.name.encode();size=len(self.data)
        extra=(tlv(1,struct.pack('<QQ',0 if flags else size,0 if flags else size)) if self.zlocal else b'')+self.lextra
        length=0xffffffff if self.zlocal else (0 if flags else size)
        h=struct.pack('<4s5H3I2H',LSIG,45 if self.zlocal else 20,flags,0,0,0,0 if flags else crc(self.data),length,length,len(n),len(extra))
        tail=b''
        if self.descriptor:
            signed=self.descriptor.startswith('signed');width=8 if self.descriptor.endswith('64') else 4
            tail=(struct.pack('<I',DSIG) if signed else b'')+struct.pack('<I',crc(self.data))+struct.pack('<QQ' if width==8 else '<II',size,size)
        return h+n+extra+self.data+tail
    def central(self):
        n=self.name.encode();size=len(self.data);extra=(tlv(1,struct.pack('<QQQ',size,size,self.offset)) if self.zcentral else (tlv(1,struct.pack('<Q',self.offset)) if self.zoffset else b''))+self.cextra
        length=0xffffffff if self.zcentral else size;offset=0xffffffff if self.zcentral or self.zoffset else self.offset
        return struct.pack('<4s6H3I5H2I',CSIG,45,45 if self.zcentral or self.zlocal else 20,8 if self.descriptor else 0,0,0,0,crc(self.data),length,length,len(n),len(extra),0,0,0,0,offset)+n+extra


def finish(body,records,zip64=False,order=None,index_change=None):
    ordered=records if order is None else [records[i] for i in order]
    payload=b''.join(hashlib.md5(r.name.encode()).digest()+struct.pack('<Q',r.offset) for r in sorted(ordered,key=lambda r:md5key(r.name)))
    if index_change:payload=index_change(payload)
    index=Record(INDEX,payload);index.offset=len(body);body+=index.local();ordered=ordered+[index]
    at=len(body);directory=b''.join(r.central() for r in ordered);body+=directory
    if zip64:
        zoff=len(body);body+=struct.pack('<4sQ2H2I4Q',b'PK\x06\x06',44,45,45,0,0,len(ordered),len(ordered),len(directory),at)
        body+=struct.pack('<4sIQI',b'PK\x06\x07',0,zoff,1)
        body+=struct.pack('<4s4H2IH',b'PK\x05\x06',0,0,0xffff,0xffff,0xffffffff,0xffffffff,0)
    else:body+=struct.pack('<4s4H2IH',b'PK\x05\x06',0,0,len(ordered),len(ordered),len(directory),at,0)
    return body,{'central_offset':at,'central_bytes':len(directory),'entries':len(ordered),'records':[(r.name,r.offset) for r in ordered]}


def simple(resources=(),descriptor=None,zlocal=False,zcentral=False,lextra=b'',cextra=b'',gap=0,order=None,zip64=False,index_change=None,zoffset=False):
    records=[Record('tileset.json',MANIFEST),Record('point.glb',point_glb(resources),descriptor,zlocal,zcentral,lextra,cextra,zoffset)]+[Record(n,b) for n,b in resources]
    body=b''
    for r in records:
        if body:body+=b'\0'*gap
        r.offset=len(body);body+=r.local()
    return finish(body,records,zip64,order,index_change)


def opaque_zip():
    r=Record('ordinary.txt',b'opaque nested ZIP content');r.offset=0;body=r.local();at=len(body);c=r.central();body+=c
    return body+struct.pack('<4s4H2IH',b'PK\x05\x06',0,0,1,1,len(c),at,0)


def nested(where):
    first=Record('tileset.json',MANIFEST);first.offset=0;body=first.local()
    if where=='header':
        point=Record('point.glb',point_glb([('carrier.bin',b'X')]))
        inner=point.local();carrier=Record('carrier.bin',b'X',lextra=tlv(0xcafe,b'pad!'+inner+b'end!'))
        carrier.offset=len(body);point.offset=carrier.offset+30+len(carrier.name)+4+4;body+=carrier.local()
    else:
        size=80
        for _ in range(8):
            point=Record('point.glb',point_glb([('carrier.bin',b'X'*size)]));inner=point.local()
            actual=b'prefix!!'+inner+b'suffix!!'
            newsize=len(actual) if where=='data' else 8+30+len(point.name)+32
            if newsize==size:break
            size=newsize
        require(newsize==size,'fixture fixed point','invalid_state')
        carrier=Record('carrier.bin',actual[:size]);carrier.offset=len(body)
        point.offset=carrier.offset+30+len(carrier.name)+8
        body+=carrier.local()+actual[size:]
    return finish(body,[first,point,carrier])


def crc_suffix(target):
    # Solve the affine CRC map for a four-byte word by 32-bit GF(2) elimination.
    zero=crc(b'\0'*4);basis={}
    for bit in range(32):
        effect=crc((1<<bit).to_bytes(4,'little'))^zero;solution=1<<bit
        while effect:
            pivot=effect.bit_length()-1
            if pivot in basis:effect^=basis[pivot][0];solution^=basis[pivot][1]
            else:basis[pivot]=(effect,solution);break
    value=target^zero;answer=0
    while value:
        pivot=value.bit_length()-1;effect,solution=basis[pivot];value^=effect;answer^=solution
    result=answer.to_bytes(4,'little');require(crc(result)==target,'CRC forge reference','invalid_state');return result


class Source:
    def __init__(self,stream,length,fault=None):self.stream=stream;self.length=length;self.fault=fault;self.reads=[];self.events=[]
    def read(self,at,n):
        require(0<=at<=self.length and n>=0 and at+n<=self.length,'truncated/out-of-source range')
        self.reads.append([at,n])
        try:
            if self.fault and self.fault(at,n):raise OSError(5,'injected EIO in independent source model')
            self.stream.seek(at);data=self.stream.read(n)
        except OSError as e:raise Failure('io',str(e)) from e
        require(len(data)==n,'truncated source read');return data


def extras(data,values):
    at=0;z=None;unicode_path=False
    while at<len(data):
        require(at+4<=len(data),'truncated extra TLV header');tag,length=struct.unpack_from('<HH',data,at);at+=4
        require(at+length<=len(data),'truncated extra TLV data');value=data[at:at+length];at+=length
        if tag==1:require(z is None,'duplicate ZIP64 extra');z=value
        if tag==0x7075:unicode_path=True
    cursor=0;out=[]
    for value in values:
        if value==0xffffffff:
            require(z is not None and cursor+8<=len(z),'missing/truncated sentinel ZIP64 value');value=struct.unpack_from('<Q',z,cursor)[0];cursor+=8
        out.append(value)
    return out,z is not None,unicode_path


def descriptor_candidates(data,crcvalue,size,width):
    ends=[]
    for skip in (0,4):
        if skip and (len(data)<4 or struct.unpack_from('<I',data)[0]!=DSIG):continue
        length=skip+4+2*width
        if len(data)<length:continue
        values=struct.unpack_from('<IQQ' if width==8 else '<III',data,skip)
        if values==(crcvalue,size,size):ends.append(length)
    return sorted(set(ends))


def choose_descriptor(ends,next_local,central_remaining):
    require(ends,'no matching descriptor candidate')
    require(any(end<=central_remaining for end in ends),'descriptor crosses central/source boundary')
    fitting=[end for end in ends if end<=central_remaining and end<=next_local]
    require(fitting,'descriptor crosses coherent local-record boundary','unsupported')
    require(len(fitting)==1,'ambiguous distinct descriptor ends','unsupported')
    return fitting[0]


DEFAULT_LIMITS={'archive':1<<20,'central':1<<18,'entries':128,'member':1<<16,'stored':1<<18}


def read_archive(source,limits=None,index=True):
    limits=DEFAULT_LIMITS if limits is None else limits
    require(source.length<=limits['archive'],'archive length ceiling','resource_limit');source.events.append('source-length-admitted')
    require(source.length>=22,'short end record')
    # Deliberately simple bounded fixture reader: exact final EOCD, no comments.
    e=source.read(source.length-22,22);require(e[:4]==b'PK\x05\x06','missing final EOCD')
    disk,cd_disk,count_disk,count,csize,coff,comment=struct.unpack_from('<4H2IH',e,4)
    require(comment==0,'fixture oracle only admits no EOCD comment','unsupported');require(disk==cd_disk==0,'multi-disk','unsupported')
    boundary=source.length-22
    if count==0xffff or csize==0xffffffff or coff==0xffffffff:
        loc=source.read(boundary-20,20);require(loc[:4]==b'PK\x06\x07','missing ZIP64 locator')
        ld,zo,nd=struct.unpack_from('<IQI',loc,4);require(ld==0 and nd==1,'ZIP64 multi-disk','unsupported')
        z=source.read(zo,56);require(z[:4]==b'PK\x06\x06','bad ZIP64 end signature');zs,v1,v2,d1,d2,n1,n2,csize,coff=struct.unpack_from('<Q2H2I4Q',z,4)
        require(zs==44 and zo+56==boundary-20,'ZIP64 end extent');require(d1==d2==0 and n1==n2,'ZIP64 disks/count','unsupported');count=n2;boundary=zo
    else:require(count==count_disk,'EOCD count mismatch')
    require(count<=limits['entries'],'entry count ceiling','resource_limit');source.events.append('entry-count-admitted')
    require(csize<=limits['central'],'central bytes ceiling','resource_limit');source.events.append('central-size-admitted')
    require(coff+csize<=boundary,'central outside source')
    entries=[];at=coff;total=0;raw_names=set()
    for _ in range(count):
        require(at+46<=coff+csize,'truncated central fixed header');h=source.read(at,46);require(h[:4]==CSIG,'central signature')
        made,needed,flags,method,time,date,ccrc,cs,us,nl,xl,cl,disk,internal,external,lo=struct.unpack_from('<6H3I5H2I',h,4)
        require(method==0,'non-stored member');require(not flags&1 and disk==0,'encrypted/multi-disk member','unsupported')
        require(at+46+nl+xl+cl<=coff+csize,'central variable extent');name=source.read(at+46,nl);extra=source.read(at+46+nl,xl)
        (us,cs,lo),cz,cu=extras(extra,[us,cs,lo]);require(not cu,'Unicode Path changes raw identity outside profile','unsupported')
        require(name not in raw_names,'duplicate raw logical name');raw_names.add(name)
        require(us==cs,'stored lengths differ');require(us<=limits['member'],'member bytes ceiling','resource_limit')
        total+=us;require(total<=limits['stored'],'stored-byte ceiling','resource_limit')
        require(lo+30<=coff,'local fixed header enters central');local=source.read(lo,30);require(local[:4]==LSIG,'local signature')
        lv,lf,lm,lt,ld,lc,lcs,lus,ln,lx=struct.unpack_from('<5H3I2H',local,4)
        require(lf==flags and lm==method,'local flags/method mismatch')
        require(lo+30+ln+lx<=coff,'local variable header enters central')
        require(source.read(lo+30,ln)==name,'local/central name mismatch')
        raw_lus,raw_lcs=lus,lcs
        (lus,lcs),lz,lu=extras(source.read(lo+30+ln,lx),[lus,lcs]);require(not lu,'Unicode Path changes raw identity outside profile','unsupported');start=lo+30+ln+lx;end=start+us
        require(end<=coff,'stored data enters central')
        desc=[]
        if flags&8:
            require(lc==0,'descriptor local CRC must be zero')
            # Classic local sizes are zero; ZIP64 local expansion is zero or actual.
            require((lus in (0,us) if raw_lus==0xffffffff else raw_lus==0) and
                    (lcs in (0,cs) if raw_lcs==0xffffffff else raw_lcs==0),'descriptor local size mismatch')
            require(lz or not cz,'central-only ZIP64 descriptor width outside finite profile','unsupported')
            width=8 if lz else 4
            desc=descriptor_candidates(source.read(end,min(24,coff-end)),ccrc,us,width)
            require(desc,'no matching descriptor')
        else:require(lc==ccrc and (lus,lcs)==(us,cs),'local CRC/size mismatch')
        entries.append({'name':name.decode('utf-8'),'offset':lo,'header_end':start,'data_end':end,'size':us,'crc':ccrc,'descriptor_candidates':desc})
        at+=46+nl+xl+cl
    require(at==coff+csize,'central length/count mismatch');source.events.append('directory-materialized')
    physical=sorted(entries,key=lambda x:x['offset'])
    for i,r in enumerate(physical):
        next_start=physical[i+1]['offset'] if i+1<len(physical) else coff
        if r['descriptor_candidates']:
            r['extent_end']=r['data_end']+choose_descriptor(r['descriptor_candidates'],next_start-r['data_end'],coff-r['data_end'])
        else:r['extent_end']=r['data_end']
        require(r['extent_end']<=next_start,'physical member extents overlap','unsupported')
    source.events.append('physical-extents-admitted')
    if index:
        require(len(entries)>=2 and entries[-1]['name']==INDEX,'3TZ final index member')
        require(any(r['name']=='tileset.json' for r in entries),'3TZ missing manifest')
        names=[r['name'] for r in entries];require(len(names)==len(set(names)),'duplicate member names')
        idx=entries[-1];require(idx['size']==24*(len(entries)-1),'3TZ index exact cardinality/length')
        source.events.append('index-length-admitted');raw=source.read(idx['header_end'],idx['size']);require(crc(raw)==idx['crc'],'index stored CRC mismatch')
        expected={r['offset']:hashlib.md5(r['name'].encode()).digest() for r in entries[:-1]};previous=None
        for at in range(0,len(raw),24):
            key=struct.unpack_from('<QQ',raw,at);offset=struct.unpack_from('<Q',raw,at+16)[0]
            require(previous is None or previous<=key,'index hash ordering');previous=key
            require(expected.pop(offset,None)==raw[at:at+16],'index offset/hash/name association')
        require(not expected,'index incomplete');source.events.append('index-admitted')
    # Oracle complete member CRCs are an independent fixture coherence control;
    # production envelope alone is not claimed to have read every payload.
    for r in entries:require(crc(source.read(r['header_end'],r['size']))==r['crc'],'member stored CRC mismatch')
    return {'entries':entries,'central_offset':coff,'central_bytes':csize,'stored_bytes':total,'events':source.events,'reads':source.reads}


def classify(raw,limits=None,index=True,fault=None,held_path=None):
    stream=io.BytesIO(raw) if held_path is None else open(held_path,'rb')
    try:
        source=Source(stream,len(raw),fault)
        try:return {'kind':'admitted','facts':read_archive(source,limits,index)}
        except Failure as e:return {'kind':e.kind,'reason':e.reason,'events':source.events,'reads':source.reads}
    finally:stream.close()


def fixtures():
    cases=[]
    def add(label,pair,expected='admitted',cli=True):cases.append({'label':label,'raw':pair[0],'layout':pair[1],'expected':expected,'cli':cli})
    add('adjacent-classic',simple());add('disjoint-with-padding',simple(gap=7));add('central-order-reversed',simple(order=[1,0]));add('zip64-end-and-sizes',simple(zlocal=True,zcentral=True,zip64=True))
    for mode in ('unsigned32','signed32','unsigned64','signed64'):
        wide=mode.endswith('64');add('descriptor-'+mode,simple(descriptor=mode,zlocal=wide,zcentral=wide))
    add('zip64-local-only',simple(zlocal=True))
    add('zip64-central-offset-only-no-descriptor',simple(zoffset=True))
    add('zip64-central-offset-only-descriptor',simple(zoffset=True,descriptor='signed32'),'unsupported')
    add('zip64-central-sizes-only-descriptor',simple(zcentral=True,descriptor='signed32'),'unsupported')
    add('descriptor-zip64-extra-no-sentinel',simple(descriptor='signed64',lextra=tlv(1,struct.pack('<QQ',0,0))))
    add('descriptor-local-ZIP64-with-32bit-width',simple(descriptor='signed32',zlocal=True),'invalid_input')
    add('unknown-wellframed-extras',simple(lextra=tlv(0xbeef,b'local'),cextra=tlv(0xcafe,b'central')))
    for where in ('data','header','partial-data'):add('physical-overlap-'+where,nested(where),'unsupported')
    add('ordinary-nested-ZIP-is-opaque',simple([('ordinary.zip',opaque_zip())]))
    payload=crc_suffix(DSIG);r=Record('crc.bin',payload,'unsigned32');r.offset=0
    first=Record('tileset.json',MANIFEST);first.offset=0;body=first.local();point=Record('point.glb',point_glb([('crc.bin',payload)]));point.offset=len(body);body+=point.local();r.offset=len(body);body+=r.local()
    add('unsigned-descriptor-CRC-equals-signature',finish(body,[first,point,r]))
    for label,le,ce in [('local-truncated-TLV-header',b'\x99',b''),('local-truncated-TLV-body',struct.pack('<HH',0xbeef,5)+b'x',b''),('central-truncated-TLV-header',b'',b'\x99'),('central-truncated-TLV-body',b'',struct.pack('<HH',0xbeef,5)+b'x'),('local-duplicate-ZIP64-no-sentinel',tlv(1,b'')+tlv(1,b''),b''),('central-duplicate-ZIP64-no-sentinel',b'',tlv(1,b'')+tlv(1,b''))]:
        add(label,simple(lextra=le,cextra=ce),'invalid_input')
    add('local-missing-sentinel-extra',simple(zlocal=True),'invalid_input',False)
    c=cases[-1];raw=bytearray(c['raw']);pointoff=c['layout']['records'][1][1];struct.pack_into('<H',raw,pointoff+30+len('point.glb'),0xbeef);c['raw']=bytes(raw);c['cli']=True
    add('central-missing-sentinel-extra',simple(zcentral=True),'invalid_input',False)
    c=cases[-1];raw=bytearray(c['raw']);coff=c['layout']['central_offset'];at=coff+46+len('tileset.json');struct.pack_into('<H',raw,at+46+len('point.glb'),0xbeef);c['raw']=bytes(raw);c['cli']=True
    # A central duplicate points at the same fully coherent local record. The
    # envelope profile refuses physical aliasing before the separate index gate.
    first=Record('tileset.json',MANIFEST);first.offset=0;body=first.local();point=Record('point.glb',point_glb());point.offset=len(body);body+=point.local()
    add('duplicate-coherent-local-offset',finish(body,[first,point,copy.copy(point)]),'invalid_input')
    pair=simple();raw=bytearray(pair[0]);idxoff=pair[1]['records'][-1][1];coff=pair[1]['central_offset'];index_header=coff+46+len('tileset.json')+46+len('point.glb')
    # Index payload has no descriptor and ends exactly at the central directory.
    for label,field in [('index-data-enters-central',18),('index-header-enters-central',28)]:
        changed=bytearray(raw)
        if field==18:
            size=struct.unpack_from('<I',changed,idxoff+18)[0]+1;struct.pack_into('<II',changed,idxoff+18,size,size);struct.pack_into('<II',changed,index_header+20,size,size)
        else:struct.pack_into('<H',changed,idxoff+28,struct.unpack_from('<I',changed,idxoff+18)[0]+1)
        add(label,(bytes(changed),pair[1]),'invalid_input')
    for label,mutate in [('index-too-short',lambda b:b[:-1]),('index-too-long',lambda b:b+b'x'),('index-order-reversed',lambda b:b[24:]+b[:24]),('index-bad-hash-association',lambda b:bytes([b[0]^1])+b[1:]),('index-bad-offset',lambda b:b[:16]+struct.pack('<Q',0xffffffffffffffff)+b[24:])]:
        add(label,simple(index_change=mutate),'invalid_input')
    # Raw index corruption with unchanged local/central CRC fields.
    changed=bytearray(pair[0]);changed[idxoff+30+len(INDEX)]^=1;add('index-CRC-mismatch',(bytes(changed),pair[1]),'invalid_input')
    changed=bytearray(pair[0]);struct.pack_into('<H',changed,pair[1]['records'][1][1]+6,8);point_central=coff+46+len('tileset.json');struct.pack_into('<H',changed,point_central+8,8);struct.pack_into('<III',changed,pair[1]['records'][1][1]+14,0,0,0)
    first=Record('tileset.json',MANIFEST);first.offset=0;body=first.local();point=Record('point.glb',point_glb(),descriptor='signed32');point.offset=len(body);body+=point.local()[:-4]
    add('descriptor-truncated-at-next-header',finish(body,[first,point]),'invalid_input')
    changed=bytearray(pair[0]);changed[pair[1]['records'][1][1]+30]^=1;add('local-central-name-mismatch',(bytes(changed),pair[1]),'invalid_input',False)
    changed=bytearray(pair[0]);struct.pack_into('<I',changed,pair[1]['records'][1][1]+22,1);add('local-central-size-mismatch',(bytes(changed),pair[1]),'invalid_input',False)
    def duplicates(name,short):
        first=Record('tileset.json',MANIFEST);first.offset=0;body=first.local();point=Record('point.glb',point_glb());point.offset=len(body);body+=point.local()
        original=first if name=='tileset.json' else point;duplicate=Record(name,original.data);duplicate.offset=len(body);body+=duplicate.local()
        def last_only(raw):return b''.join(raw[i:i+24] for i in range(0,len(raw),24) if struct.unpack_from('<Q',raw,i+16)[0]!=original.offset)
        return finish(body,[first,point,duplicate],index_change=last_only if short else None)
    add('duplicate-GLB-shortened-index',duplicates('point.glb',True),'invalid_input')
    add('duplicate-manifest-shortened-index',duplicates('tileset.json',True),'invalid_input')
    add('duplicate-GLB-full-cardinality-index',duplicates('point.glb',False),'invalid_input')
    unicode=tlv(0x7075,b'\x01'+struct.pack('<I',crc(b'point.glb'))+b'tileset.json')
    add('UnicodePath-local-identity-override',simple(lextra=unicode),'unsupported')
    add('UnicodePath-central-identity-override',simple(cextra=unicode),'unsupported')
    return cases


def descriptor_controls():
    # Distinct matching signed/unsigned ends need size=signature (~128MiB).
    # These are isolated bounded word/candidate controls, not full archives.
    words=struct.pack('<4I',DSIG,DSIG,DSIG,DSIG);ends=descriptor_candidates(words,DSIG,DSIG,4);require(ends==[12,16],'ambiguity candidate sensitivity','invalid_state')
    results=[]
    for label,next_local,central,expected in [('only-short-end-fits',12,16,'admitted'),('two-ends-fit',16,16,'unsupported'),('both-cross-local-boundary',11,16,'unsupported'),('both-cross-central-boundary',16,11,'invalid_input')]:
        try:end=choose_descriptor(ends,next_local,central);kind='admitted'
        except Failure as e:kind=e.kind;end=None
        require(kind==expected,'isolated descriptor control '+label,'invalid_state');results.append({'label':label,'raw_hex':words.hex(),'matching_ends':ends,'next_local_remaining':next_local,'central_remaining':central,'expected':expected,'actual':kind,'selected_end':end})
    return results


def limit_controls(raw,layout):
    baseline=classify(raw);require(baseline['kind']=='admitted','limit baseline','invalid_state');facts=baseline['facts'];results=[]
    values={'archive':len(raw),'central':layout['central_bytes'],'entries':layout['entries'],'member':max(r['size'] for r in facts['entries']),'stored':facts['stored_bytes']}
    for field,value in values.items():
        for delta,expected in [(0,'admitted'),(-1,'resource_limit')]:
            limits=dict(DEFAULT_LIMITS);limits[field]=value+delta;actual=classify(raw,limits)
            require(actual['kind']==expected,'sensitive limit '+field,'invalid_state');results.append({'field':field,'limit':value+delta,'actual_value':value,'expected':expected,**actual})
    return results


def main():
    p=argparse.ArgumentParser();p.add_argument('--work',type=Path,required=True);p.add_argument('--frozen',action='store_true');p.add_argument('--binary',type=Path);p.add_argument('--binary-sha256');p.add_argument('--cli-label',action='append');p.add_argument('--source-pin',type=Path);p.add_argument('--source-pin-sha256');args=p.parse_args()
    work=args.work.resolve();require(str(work).startswith('/tmp/rusty-tiles-archive-read-'),'external work namespace');work.mkdir(parents=True,exist_ok=False)
    binary=FROZEN if args.frozen else args.binary
    if binary:require(sha(binary.read_bytes())==(FROZEN_SHA if args.frozen else args.binary_sha256),'CLI binary identity')
    source_pin=None
    if binary and not args.frozen:
        require(args.source_pin is not None and args.source_pin_sha256 is not None,'candidate requires coordinator source pin','invalid_state')
        raw=args.source_pin.read_bytes();require(sha(raw)==args.source_pin_sha256,'candidate source pin identity');source_pin=json.loads(raw)
    cases=fixtures();require(len(cases)<=45,'bounded fixture workload','invalid_state');records=[]
    for c in cases:
        path=work/(c['label']+'.3tz');path.write_bytes(c['raw']);truth=classify(c['raw']);held=classify(c['raw'],held_path=path)
        require(truth==held,'Cursor/held-file independent oracle agreement','invalid_state');require(truth['kind']==c['expected'],'fixture oracle '+c['label']+' '+str(truth),'invalid_state')
        item={'label':c['label'],'archive_path':str(path),'archive_bytes':len(c['raw']),'archive_sha256':sha(c['raw']),'expected_format':c['expected'],'layout':c['layout'],'independent':truth,'oracle_cursor_held_file_agree':True}
        if binary and (c['cli'] or not args.frozen) and (not args.cli_label or c['label'] in args.cli_label):
            cmd=['nice','-n','10',str(binary),'validate','--json',str(path)]
            env=dict(os.environ,RAYON_NUM_THREADS='2');r=subprocess.run(cmd,env=env,capture_output=True,text=True,timeout=10)
            try:result=json.loads(r.stdout)
            except json.JSONDecodeError:result=None
            actual='admitted' if r.returncode==0 and isinstance(result,dict) and result.get('ok') is True else (result.get('error',{}).get('code','unclassified') if isinstance(result,dict) else 'unclassified')
            item['cli']={'command':cmd,'returncode':r.returncode,'stdout':r.stdout,'stderr':r.stderr,'json':result,'actual_category':actual,
                         'matches_format_category':actual==c['expected'],'role':'historical frozen behavior' if args.frozen else 'candidate behavior'}
        records.append(item)
    limits=limit_controls(cases[0]['raw'],cases[0]['layout']);isolated=descriptor_controls()
    io_cases=[]
    for label,at in [('end-read',len(cases[0]['raw'])-22),('central-read',cases[0]['layout']['central_offset'])]:
        actual=classify(cases[0]['raw'],fault=lambda offset,n,target=at:offset==target);require(actual['kind']=='io','I/O model sensitivity','invalid_state');io_cases.append({'label':label,**actual})
    root=HERE.parents[3];snapshot=Path('/tmp/rusty-tiles-a2-continuation')
    pins={name:sha((snapshot/name).read_bytes()) for name in ('src/validate/admission.rs','src/archive3tz.rs','src/validate.rs','src/validate/types.rs')}
    original_pin=json.loads((root/'bench/architecture_audit/implicit_rewrite/a2_real_sources/source-artifacts.json').read_bytes())
    require(all(original_pin['production_sha256'][name]==digest for name,digest in pins.items()),'historical source bytes match frozen manifest')
    pins['docs/architecture/archive-read-foundation-contract.md']=sha((root/'docs/architecture/archive-read-foundation-contract.md').read_bytes())
    receipt={'classification':'independent tiny fixture/oracle and optional CLI comparisons; no Rust allocation/I/O acceptance',
             'probe_sha256':sha(Path(__file__).read_bytes()),'binary_path':str(binary) if binary else None,'binary_sha256':sha(binary.read_bytes()) if binary else None,
             'historical_source_snapshot':str(snapshot),'historical_source_commit':original_pin['frozen_source_commit'],
             'historical_manifest_sha256':original_pin['frozen_manifest_sha256'],'inspected_files_sha256':pins,
             'candidate_source_pin':source_pin,'candidate_source_pin_sha256':args.source_pin_sha256,
             'records':records,'limit_controls':limits,'isolated_descriptor_controls':isolated,'injected_io_model':io_cases,
             'method':{'priority':10,'CLI_timeout_seconds':10,'concurrency':1,'RAYON_NUM_THREADS':2,'CLI_executions':sum('cli' in r for r in records),'Cargo_builds':0,'producers':0}}
    failures=[r['label'] for r in records if 'cli' in r and not r['cli']['matches_format_category']]
    receipt['candidate_category_failures']=failures if binary and not args.frozen else None
    (work/'receipt.json').write_text(json.dumps(receipt,indent=2,sort_keys=True)+'\n');print(json.dumps({'cases':len(records),'CLI_executions':receipt['method']['CLI_executions'],'limit_controls':len(limits),'isolated_descriptor_controls':len(isolated),'probe_sha256':receipt['probe_sha256'],'candidate_category_failures':receipt['candidate_category_failures']}))
    if binary and not args.frozen:require(not failures,'candidate category mismatches '+str(failures),'invalid_state')


if __name__=='__main__':main()
