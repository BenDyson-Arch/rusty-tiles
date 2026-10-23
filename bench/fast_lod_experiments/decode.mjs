// Independent published glTF accessor/texture decoder; no browser.
import { createRequire } from 'node:module';
globalThis.require=createRequire(import.meta.url); globalThis.__dirname='/tmp';
import fs from 'node:fs'; import path from 'node:path'; import vm from 'node:vm';
import { MeshoptDecoder } from '/home/bend/.cache/rusty-tiles-117-browser-cache/node_modules/meshoptimizer/index.js';
await MeshoptDecoder.ready;
const basisPath='/home/bend/.cache/rusty-tiles-117-browser-cache/node_modules/three/examples/jsm/libs/basis/';
vm.runInThisContext(fs.readFileSync(basisPath+'basis_transcoder.js','utf8'));
const basis=await globalThis.BASIS({wasmBinary:fs.readFileSync(basisPath+'basis_transcoder.wasm')});basis.initializeBasis();
const [root,out]=process.argv.slice(2);fs.mkdirSync(out,{recursive:true});
let files=[];
if(root.endsWith('.glb')) files=[root]; else {
 const walk=d=>{for(const f of fs.readdirSync(d)){let p=path.join(d,f);if(fs.statSync(p).isDirectory())walk(p);else if(p.endsWith('.glb'))files.push(p);}};walk(root);files.sort();
}
const records=[];
for (let file of files) {
 const buf=fs.readFileSync(file);const len=buf.readUInt32LE(12);const j=JSON.parse(buf.subarray(20,20+len));const bin=buf.subarray(28+len);const cache=new Map();
 const view=i=>{if(cache.has(i))return cache.get(i);let v=j.bufferViews[i],e=v.extensions?.EXT_meshopt_compression,r;
 if(e){r=new Uint8Array(e.count*e.byteStride);MeshoptDecoder.decodeGltfBuffer(r,e.count,e.byteStride,bin.subarray(e.byteOffset||0,(e.byteOffset||0)+e.byteLength),e.mode,e.filter||'NONE');}else r=bin.subarray(v.byteOffset||0,(v.byteOffset||0)+v.byteLength);cache.set(i,r);return r;};
 const acc=ai=>{const a=j.accessors[ai],v=j.bufferViews[a.bufferView],raw=view(a.bufferView);const n={SCALAR:1,VEC2:2,VEC3:3,VEC4:4}[a.type];const widths={5120:1,5121:1,5122:2,5123:2,5125:4,5126:4};const get={5120:'getInt8',5121:'getUint8',5122:'getInt16',5123:'getUint16',5125:'getUint32',5126:'getFloat32'}[a.componentType];const w=widths[a.componentType],dv=new DataView(raw.buffer,raw.byteOffset,raw.byteLength);const out=new Float64Array(a.count*n);
 for(let i=0;i<a.count;i++)for(let c=0;c<n;c++){let x=dv[get]((a.byteOffset||0)+i*(v.byteStride||w*n)+c*w,true);if(a.normalized)x=a.componentType===5123?x/65535:a.componentType===5121?x/255:a.componentType===5122?Math.max(x/32767,-1):Math.max(x/127,-1);out[i*n+c]=x;}return {data:out,n};};
 const iid=new Map();
 const image=ii=>{if(iid.has(ii))return iid.get(ii);const im=j.images[ii],raw=im.bufferView!==undefined?view(im.bufferView):fs.readFileSync(path.join(path.dirname(file),im.uri));const id=records.length+'-'+ii;
 let info;if(im.mimeType==='image/ktx2'){
 const k=new basis.KTX2File(new Uint8Array(raw));if(!k.isValid()||!k.startTranscoding())throw Error('ktx '+file);const rgba=new Uint8Array(k.getImageTranscodedSizeInBytes(0,0,0,13));if(!k.transcodeImage(rgba,0,0,0,13,0,-1,-1))throw Error('decode');
 fs.writeFileSync(path.join(out,id+'.rgba'),rgba);info={file:id+'.rgba',width:k.getWidth(),height:k.getHeight(),raw:true};k.close();k.delete();
 }else {fs.writeFileSync(path.join(out,id+'.image'),raw);info={file:id+'.image',raw:false};}iid.set(ii,info);return info;};
 let all=[];let surfaces=[];
 const identity=[1,0,0,0,0,1,0,0,0,0,1,0,0,0,0,1];
 const multiply=(a,b)=>Array.from({length:16},(_,i)=>{let r=i%4,c=Math.floor(i/4);return [0,1,2,3].reduce((s,k)=>s+a[r+k*4]*b[k+c*4],0);});
 const visit=(ni,parent)=>{let node=j.nodes[ni];if(node.rotation)throw Error('unsupported rotation');let s=node.scale||[1,1,1],t=node.translation||[0,0,0];let m=multiply(parent,node.matrix||[s[0],0,0,0,0,s[1],0,0,0,0,s[2],0,...t,1]);
 if(node.mesh!==undefined)for(let pr of j.meshes[node.mesh].primitives){if(pr.mode!==undefined&&pr.mode!==4)throw Error('mode');let p=acc(pr.attributes.POSITION).data;let idx=pr.indices===undefined?Array.from({length:p.length/3},(_,i)=>i):acc(pr.indices).data;let mat=j.materials?.[pr.material]||{},bc=mat.pbrMetallicRoughness||{},bt=bc.baseColorTexture;let tex=bt?j.textures[bt.index]:null;let uv=pr.attributes['TEXCOORD_'+(bt?.texCoord||0)];uv=uv===undefined?null:acc(uv).data;let col=pr.attributes.COLOR_0===undefined?null:acc(pr.attributes.COLOR_0);let im=tex?image(tex.extensions?.KHR_texture_basisu?.source??tex.source):null;let sampler=tex?j.samplers?.[tex.sampler]||{}:{};let start=all.length/27;
 for(let q=0;q<idx.length;q+=3){for(let c=0;c<3;c++){let vi=idx[q+c],v=Array.from(p.subarray(vi*3,vi*3+3));all.push(...[0,1,2].map(k=>m[k]*v[0]+m[k+4]*v[1]+m[k+8]*v[2]+m[k+12]));}for(let c=0;c<3;c++){let vi=idx[q+c];all.push(...(uv?uv.subarray(vi*2,vi*2+2):[0,0]));}for(let c=0;c<3;c++){let vi=idx[q+c];for(let a=0;a<4;a++)all.push((bc.baseColorFactor?.[a]??1)*(col&&a<col.n?col.data[vi*col.n+a]:1));}}
 surfaces.push({first:start,count:idx.length/3,image:im,sampler,mesh:node.mesh,material:pr.material});}
 for(const c of node.children||[])visit(c,m);};
 for(const ni of j.scenes[j.scene||0].nodes)visit(ni,identity);
 let name=root.endsWith('.glb')?'source':path.relative(root,file);let id=records.length;fs.writeFileSync(path.join(out,id+'.tri.f64'),new Float64Array(all));records.push({id,file:name,triangles:all.length/27,surfaces});
}
fs.writeFileSync(path.join(out,'index.json'),JSON.stringify(records));
