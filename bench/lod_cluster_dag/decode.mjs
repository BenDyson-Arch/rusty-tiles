// Decode the baseline's meshopt buffers and KTX2 images for independent measurement.
import fs from 'node:fs';
import path from 'node:path';
import {createRequire} from 'node:module';
const require=createRequire(import.meta.url);
const cache=process.env.MESHOPT_JS || '/home/bend/.cache/rusty-tiles-117-browser-cache/node_modules';
const {MeshoptDecoder}=await import(`${cache}/meshoptimizer/index.js`);
await MeshoptDecoder.ready;
const basisDir=process.env.BASIS_DIR || '/tmp/rt-audit-basisu/x/basis_universal-2_50/webgl/transcoder/build';
const factory=require(`${basisDir}/basis_transcoder.js`);
const basis=await factory({wasmBinary:fs.readFileSync(`${basisDir}/basis_transcoder.wasm`)});
basis.initializeBasis();
const [root,dest]=process.argv.slice(2);
function walk(dir){return fs.readdirSync(dir,{withFileTypes:true}).flatMap(e=>e.isDirectory()?walk(path.join(dir,e.name)):[path.join(dir,e.name)]);}
for(const file of walk(root).filter(f=>f.endsWith('.glb'))){
 const rel=path.relative(root,file), output=path.join(dest,rel);fs.mkdirSync(path.dirname(output),{recursive:true});
 const b=fs.readFileSync(file), jl=b.readUInt32LE(12), j=JSON.parse(b.subarray(20,20+jl)), bin=b.subarray(28+jl);
 let offset=0;const chunks=[];
 for(const v of j.bufferViews){
  const e=v.extensions?.EXT_meshopt_compression;let raw;
  if(e){raw=new Uint8Array(e.count*e.byteStride);MeshoptDecoder.decodeGltfBuffer(raw,e.count,e.byteStride,bin.subarray(e.byteOffset||0,(e.byteOffset||0)+e.byteLength),e.mode,e.filter||'NONE');delete v.extensions;}
  else raw=bin.subarray(v.byteOffset||0,(v.byteOffset||0)+v.byteLength);
  const pad=(-offset)&3;if(pad){chunks.push(Buffer.alloc(pad));offset+=pad;}
  v.buffer=0;v.byteOffset=offset;v.byteLength=raw.length;offset+=raw.length;chunks.push(Buffer.from(raw));
 }
 const rawbin=Buffer.concat(chunks);
 for(const [i,im] of (j.images||[]).entries()){
  if(im.mimeType!=='image/ktx2')continue;
  const v=j.bufferViews[im.bufferView];const k=new basis.KTX2File(new Uint8Array(rawbin.subarray(v.byteOffset,v.byteOffset+v.byteLength)));
  if(!k.isValid()||!k.startTranscoding())throw Error('KTX2 decode failed '+rel);
  const w=k.getWidth(),h=k.getHeight();const rgba=new Uint8Array(k.getImageTranscodedSizeInBytes(0,0,0,13));
  if(!k.transcodeImage(rgba,0,0,0,13,0,-1,-1))throw Error('RGBA transcode failed '+rel);
  const name=path.basename(output)+`.image${i}.rgba`;fs.writeFileSync(path.join(path.dirname(output),name),rgba);
  delete im.bufferView;im.uri=name;im.extras={width:w,height:h};k.close();k.delete();
 }
 j.buffers=[{byteLength:rawbin.length}];
 for(const name of ['extensionsRequired','extensionsUsed'])if(j[name])j[name]=j[name].filter(e=>e!=='EXT_meshopt_compression');
 let js=Buffer.from(JSON.stringify(j));js=Buffer.concat([js,Buffer.alloc((-js.length)&3,32)]);
 const bb=Buffer.concat([rawbin,Buffer.alloc((-rawbin.length)&3)]);const head=Buffer.alloc(20);head.write('glTF');head.writeUInt32LE(2,4);head.writeUInt32LE(28+js.length+bb.length,8);head.writeUInt32LE(js.length,12);head.write('JSON',16);
 const bh=Buffer.alloc(8);bh.writeUInt32LE(bb.length);bh.write('BIN\0',4);fs.writeFileSync(output,Buffer.concat([head,js,bh,bb]));
 console.log(rel);
}
