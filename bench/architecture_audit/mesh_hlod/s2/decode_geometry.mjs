// Independent JS meshoptimizer decode of published bytes; geometry only, no producer facts.
// Usage: node decode_geometry.mjs SOURCE_GLB STAGE OUT
import fs from 'node:fs';
import path from 'node:path';
const cache=process.env.MESHOPT_JS || '/home/bend/.cache/rusty-tiles-117-browser-cache/node_modules';
const {MeshoptDecoder}=await import(`${cache}/meshoptimizer/index.js`);
await MeshoptDecoder.ready;
const [source,stage,out]=process.argv.slice(2);fs.mkdirSync(out,{recursive:true});
const rows=name=>fs.readFileSync(path.join(stage,name),'utf8').trim().split('\n').filter(Boolean).map(JSON.parse);
const files=[{uri:'source',file:source},...rows('leaves.jsonl').map(r=>({...r,leaf:true,file:path.join(stage,r.uri)})),...rows('proxies.jsonl').map(r=>({...r,leaf:false,file:path.join(stage,r.uri)}))];
const types={5120:Int8Array,5121:Uint8Array,5122:Int16Array,5123:Uint16Array,5125:Uint32Array,5126:Float32Array};
const widths={SCALAR:1,VEC2:2,VEC3:3,VEC4:4};
const index=[];
for(const [ordinal,row] of files.entries()) {
 const b=fs.readFileSync(row.file),jl=b.readUInt32LE(12),j=JSON.parse(b.subarray(20,20+jl)),bin=b.subarray(28+jl);const views=new Map();
 function view(id){if(views.has(id))return views.get(id);const v=j.bufferViews[id],e=v.extensions?.EXT_meshopt_compression;let data;
  if(e){data=new Uint8Array(e.count*e.byteStride);MeshoptDecoder.decodeGltfBuffer(data,e.count,e.byteStride,bin.subarray(e.byteOffset||0,(e.byteOffset||0)+e.byteLength),e.mode,e.filter||'NONE');}
  else data=bin.subarray(v.byteOffset||0,(v.byteOffset||0)+(v.byteLength||0));views.set(id,data);return data;
 }
 function acc(id){const a=j.accessors[id],v=j.bufferViews[a.bufferView],data=view(a.bufferView),T=types[a.componentType],n=widths[a.type],stride=v.byteStride||n*T.BYTES_PER_ELEMENT;const result=[];
  for(let i=0;i<a.count;i++){const offset=(a.byteOffset||0)+i*stride;const copy=data.slice(offset,offset+n*T.BYTES_PER_ELEMENT);result.push(Array.from(new T(copy.buffer,copy.byteOffset,n)));}return result;
 }
 const xyz=[];
 for(const node of j.nodes||[]){if(node.mesh===undefined)continue;if(node.matrix||node.rotation||node.children)throw Error('oracle supports identity/translation/scale fixtures only');
  for(const pr of j.meshes[node.mesh].primitives){const pos=acc(pr.attributes.POSITION),indices=pr.indices===undefined?pos.map((_,i)=>i):acc(pr.indices).flat();
   for(const i of indices){const p=pos[i];for(let a=0;a<3;a++)xyz.push(p[a]*(node.scale?.[a]??1)+(node.translation?.[a]??0));}
  }
 }
 const name=`${ordinal}.f64`;fs.writeFileSync(path.join(out,name),Buffer.from(new Float64Array(xyz).buffer));index.push({...row,file:name,triangles:xyz.length/9});
}
fs.writeFileSync(path.join(out,'index.json'),JSON.stringify(index,null,2));
