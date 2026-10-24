// Independent published-byte decoder. No Rust/producer decoder, facts or locks.
import fs from 'node:fs';
const I=[1,0,0,0,0,1,0,0,0,0,1,0,0,0,0,1];
const mul=(a,b)=>I.map((_,i)=>[0,1,2,3].reduce((s,k)=>s+a[i%4+k*4]*b[k+Math.floor(i/4)*4],0));
function matrix(n) {
 if(n.matrix)return n.matrix;
 const [x,y,z,w]=n.rotation??[0,0,0,1],s=n.scale??[1,1,1],t=n.translation??[0,0,0];
 return [(1-2*y*y-2*z*z)*s[0],(2*x*y+2*z*w)*s[0],(2*x*z-2*y*w)*s[0],0,
 (2*x*y-2*z*w)*s[1],(1-2*x*x-2*z*z)*s[1],(2*y*z+2*x*w)*s[1],0,
 (2*x*z+2*y*w)*s[2],(2*y*z-2*x*w)*s[2],(1-2*x*x-2*y*y)*s[2],0,...t,1];
}
const point=(m,p)=>[0,1,2].map(a=>m[a]*p[0]+m[a+4]*p[1]+m[a+8]*p[2]+m[a+12]);
let meshopt;
export async function decode(file) {
 const b=fs.readFileSync(file);
 if(b.toString('ascii',0,4)!=='glTF'||b.readUInt32LE(8)!==b.length)throw Error('codecStructure: GLB');
 const jl=b.readUInt32LE(12),j=JSON.parse(b.subarray(20,20+jl)),bin=b.subarray(28+jl),cache=new Map();
 for(const [owner,allowed] of [[j,['EXT_structural_metadata']],...(j.nodes??[]).map(n=>[n,[]]),...(j.meshes??[]).map(m=>[m,[]]),...(j.meshes??[]).flatMap(m=>m.primitives.map(p=>[p,['EXT_mesh_features']]))])if(Object.keys(owner.extensions??{}).some(k=>!allowed.includes(k)))throw Error('unsupported oracle scene extension');
 if(j.animations||j.skins||j.meshes?.some(m=>m.weights||m.primitives.some(p=>p.targets))||j.nodes?.some(n=>n.weights))throw Error('unsupported oracle animation, skin or morph scene');
 async function view(id){
  if(cache.has(id))return cache.get(id);
  const v=j.bufferViews[id],e=v.extensions?.EXT_meshopt_compression;let data;
  if(e){
   if(!meshopt){const root=process.env.MESHOPT_JS??'/home/bend/.cache/rusty-tiles-117-browser-cache/node_modules';meshopt=(await import(`${root}/meshoptimizer/index.js`)).MeshoptDecoder;await meshopt.ready;}
   if(e.buffer!==0)throw Error('external meshopt buffer');
   data=new Uint8Array(e.count*e.byteStride);
   meshopt.decodeGltfBuffer(data,e.count,e.byteStride,bin.subarray(e.byteOffset??0,(e.byteOffset??0)+e.byteLength),e.mode,e.filter??'NONE');
  }else {if(v.buffer!==0)throw Error('external oracle buffer');data=bin.subarray(v.byteOffset??0,(v.byteOffset??0)+v.byteLength);}
  cache.set(id,data);return data;
 }
 const types={5120:[1,'getInt8',127],5121:[1,'getUint8',255],5122:[2,'getInt16',32767],5123:[2,'getUint16',65535],5125:[4,'getUint32',4294967295],5126:[4,'getFloat32',1]};
 async function acc(id){const a=j.accessors[id];if(a.sparse)throw Error('unsupported oracle sparse accessor');
  const v=j.bufferViews[a.bufferView],raw=await view(a.bufferView),d=new DataView(raw.buffer,raw.byteOffset,raw.byteLength),[w,get,max]=types[a.componentType],n={SCALAR:1,VEC2:2,VEC3:3,VEC4:4}[a.type],stride=v.byteStride??w*n;
  return Array.from({length:a.count},(_,i)=>Array.from({length:n},(_,k)=>{let q=d[get]((a.byteOffset??0)+i*stride+k*w,true);if(a.normalized)q=Math.max(-1,q/max);if(!Number.isFinite(q))throw Error('accessorValues');return q;}));
 }
 const meta=j.extensions?.EXT_structural_metadata, tables=[];
 for(const t of meta?.propertyTables??[]){const props={};if(!meta.schema.classes[t.class])throw Error('metadataIntegrity');
  for(const [key,p] of Object.entries(t.properties)){const schema=meta.schema.classes[t.class].properties[key];if(!schema||schema.type!=='SCALAR'||('normalized' in schema&&schema.normalized!==false)||('array' in schema&&schema.array!==false)||['count','offset','scale','noData','default'].some(k=>k in schema||k in p))throw Error('metadataIntegrity');if(!j.bufferViews[p.values])throw Error('metadataIntegrity');const raw=await view(p.values),d=new DataView(raw.buffer,raw.byteOffset,raw.byteLength),wide=schema.componentType==='UINT64',width=wide?8:schema.componentType==='UINT8'?1:4;
   if(!['UINT8','UINT32','UINT64'].includes(schema.componentType)||raw.length<t.count*width)throw Error('metadataIntegrity');
   props[key]=Array.from({length:t.count},(_,i)=>Number(wide?d.getBigUint64(i*8,true):width===1?d.getUint8(i):d.getUint32(i*4,true)));
  }tables.push({class:t.class,count:t.count,props});
 }
 const faces=[];
 async function visit(id,parent,stack){if(stack.has(id))throw Error('scene cycle');const n=j.nodes[id],m=mul(parent,matrix(n));stack=new Set([...stack,id]);
  if(n.mesh!==undefined)for(const [pi,p] of j.meshes[n.mesh].primitives.entries()){
   if((p.mode??4)!==4)throw Error('unsupported oracle primitive mode');
   const pos=(await acc(p.attributes.POSITION)).map(v=>point(m,v)),idx=p.indices===undefined?pos.map((_,i)=>i):(await acc(p.indices)).flat(),sets=[];
   for(const f of p.extensions?.EXT_mesh_features?.featureIds??[]){if(f.attribute===undefined)continue;const values=(await acc(p.attributes[`_FEATURE_ID_${f.attribute}`])).flat();const table=tables[f.propertyTable];
    if(!table||table.class!==f.label||f.featureCount!==table.count||values.length!==pos.length||values.some(i=>!Number.isInteger(i)||i<0||i>=table.count||i===f.nullFeatureId))throw Error('featureIdentity');sets.push({label:f.label,values,table});}
   if(idx.length%3)throw Error('primitiveIndices');
   for(let k=0;k<idx.length;k+=3){const ids={};for(const s of sets){const a=s.values[idx[k]];if([1,2].some(c=>s.values[idx[k+c]]!==a))throw Error('featureIdentity');ids[s.label]=Object.fromEntries(Object.entries(s.table.props).map(([key,col])=>[key,col[a]]));}
    faces.push({xyz:idx.slice(k,k+3).map(i=>pos[i]),ids,source:[id,n.mesh,pi,k/3],material:p.material===undefined?null:j.materials[p.material]});
   }
  }
  for(const child of n.children??[])await visit(child,m,stack);
 }
 for(const id of j.scenes[j.scene??0].nodes)await visit(id,I,new Set());
 return {faces,tables};
}
if(process.argv[1]===new URL(import.meta.url).pathname){
 try{process.stdout.write(JSON.stringify(await decode(process.argv[2])));}catch(e){console.error(e.stack);process.exitCode=1;}
}
