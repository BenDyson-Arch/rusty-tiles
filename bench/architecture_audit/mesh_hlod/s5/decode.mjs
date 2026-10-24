// Independent published-byte decoder. No Rust/producer decoder, facts or locks.
import fs from 'node:fs';
import {spawnSync} from 'node:child_process';
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
function normalMatrix(m) {
 // Inverse transpose of the linear transform, including reflections. Node
 // Translation never enters the direction transform.
 const a=[m[0],m[1],m[2]],b=[m[4],m[5],m[6]],c=[m[8],m[9],m[10]];
 const cross=(u,v)=>[u[1]*v[2]-u[2]*v[1],u[2]*v[0]-u[0]*v[2],u[0]*v[1]-u[1]*v[0]];
 const ca=cross(b,c),cb=cross(c,a),cc=cross(a,b),det=a.reduce((s,v,i)=>s+v*ca[i],0);
 if(!Number.isFinite(det)||det===0)throw Error('accessorValues: singular normal transform');
 return [...ca,...cb,...cc].map(v=>v/det);
}
const direction=(m,n)=>[0,1,2].map(i=>m[i]*n[0]+m[i+3]*n[1]+m[i+6]*n[2]);
function normal(m,n) {
 const q=direction(m,n),length=Math.hypot(...q);
 if(!length||!Number.isFinite(length))throw Error('accessorValues: normal');
 return q.map(v=>v/length);
}
// Half a scalar encoding step, transported to world coordinates. No accessor
// min/max, archive extras, producer profile or producer tolerance is trusted.
function scalarTolerance(a,values,width) {
 const maximum={5120:127,5121:255,5122:32767,5123:65535,5125:4294967295};
 return Array.from({length:width},(_,k)=>a.componentType===5126?
  values.reduce((s,v)=>Math.max(s,Math.abs(v[k])===0?2**-150:2**(Math.floor(Math.log2(Math.abs(v[k])))-24)),2**-150):
  .5/(a.normalized?maximum[a.componentType]:1));
}
function positionTolerance(local,values,m) {
 return [0,1,2].map(k=>local.reduce((s,v,c)=>s+Math.abs(m[k+c*4])*v,0)+
  32*Number.EPSILON*(Math.abs(m[k+12])+values.reduce((s,v)=>Math.max(s,...v.map((x,c)=>Math.abs(x*m[k+c*4]))),0)+1));
}
export async function decode(input) {
 const b=Buffer.isBuffer(input)?input:input instanceof Uint8Array?Buffer.from(input):fs.readFileSync(input);
 if(b.length<28||b.toString('ascii',0,4)!=='glTF'||b.readUInt32LE(4)!==2||b.readUInt32LE(8)!==b.length||b.toString('ascii',16,20)!=='JSON')throw Error('codecStructure: GLB');
 const jl=b.readUInt32LE(12),j=JSON.parse(b.subarray(20,20+jl)),cache=new Map(),viewTolerances=new Map();
 if(!j.buffers?.length||j.buffers.some(v=>v.uri!==undefined)||j.buffers.slice(1).some(v=>v.extensions?.EXT_meshopt_compression?.fallback!==true||!j.extensionsRequired?.includes('EXT_meshopt_compression'))||!b.subarray(24+jl,28+jl).equals(Buffer.from('BIN\0'))||j.buffers[0].byteLength>b.readUInt32LE(20+jl)||28+jl+b.readUInt32LE(20+jl)>b.length)throw Error('codecStructure: URI-free embedded BIN buffer required');
 const bin=b.subarray(28+jl,28+jl+j.buffers[0].byteLength);
 for(const [owner,allowed] of [[j,['EXT_structural_metadata']],...(j.nodes??[]).map(n=>[n,[]]),...(j.meshes??[]).map(m=>[m,[]]),...(j.meshes??[]).flatMap(m=>m.primitives.map(p=>[p,['EXT_mesh_features']]))])if(Object.keys(owner.extensions??{}).some(k=>!allowed.includes(k)))throw Error('unsupported oracle scene extension');
 if(j.animations||j.skins||j.meshes?.some(m=>m.weights||m.primitives.some(p=>p.targets))||j.nodes?.some(n=>n.weights))throw Error('unsupported oracle animation, skin or morph scene');
 async function view(id){
  if(cache.has(id))return cache.get(id);
  const v=j.bufferViews[id],e=v.extensions?.EXT_meshopt_compression;let data;
  if(e){
   if(e.buffer!==0)throw Error('external meshopt buffer');
   if(!Number.isSafeInteger(e.count)||!Number.isSafeInteger(e.byteStride)||e.count<=0||e.byteStride<=0||e.count*e.byteStride!==v.byteLength||(v.byteStride!==undefined&&v.byteStride!==e.byteStride)||!Number.isSafeInteger(e.byteLength)||e.byteLength<=0||!Number.isSafeInteger(e.byteOffset??0)||(e.byteOffset??0)<0||(e.byteOffset??0)+e.byteLength>bin.length)throw Error('codecStructure: meshopt layout');
   const r=spawnSync('python3',[new URL('meshopt.py',import.meta.url).pathname,e.mode,String(e.count),String(e.byteStride),e.filter??'NONE','--tolerances'],{input:bin.subarray(e.byteOffset??0,(e.byteOffset??0)+e.byteLength),maxBuffer:64*1024*1024});
   if(r.status!==0)throw Error('codecStructure: '+r.stderr);data=r.stdout;
   viewTolerances.set(id,JSON.parse(r.stderr.toString()));
  }else {if(v.buffer!==0)throw Error('external oracle buffer');data=bin.subarray(v.byteOffset??0,(v.byteOffset??0)+v.byteLength);}
  cache.set(id,data);return data;
 }
 const types={5120:[1,'getInt8',127],5121:[1,'getUint8',255],5122:[2,'getInt16',32767],5123:[2,'getUint16',65535],5125:[4,'getUint32',4294967295],5126:[4,'getFloat32',1]};
 async function acc(id){const a=j.accessors?.[id];if(!a||!types[a.componentType]||!['SCALAR','VEC2','VEC3','VEC4'].includes(a.type)||!Number.isSafeInteger(a.count)||a.count<0)throw Error('accessorValues');if(a.sparse)throw Error('unsupported oracle sparse accessor');
  const v=j.bufferViews[a.bufferView],raw=await view(a.bufferView),d=new DataView(raw.buffer,raw.byteOffset,raw.byteLength),[w,get,max]=types[a.componentType],n={SCALAR:1,VEC2:2,VEC3:3,VEC4:4}[a.type],stride=v.byteStride??w*n;
  return Array.from({length:a.count},(_,i)=>Array.from({length:n},(_,k)=>{let q=d[get]((a.byteOffset??0)+i*stride+k*w,true);if(a.normalized)q=Math.max(-1,q/max);if(!Number.isFinite(q))throw Error('accessorValues');return q;}));
 }
 function accessorTolerance(id,values,width) {
  const a=j.accessors[id],v=j.bufferViews[a.bufferView],extra=viewTolerances.get(a.bufferView)?.componentError;
  return scalarTolerance(a,values,width).map((error,k)=>error+(extra?.[(((a.byteOffset??0)+k*types[a.componentType][0])%(v.extensions?.EXT_meshopt_compression?.byteStride??4))/4]??0));
 }
 const meta=j.extensions?.EXT_structural_metadata, tables=[];
 for(const t of meta?.propertyTables??[]){const props={};if(!meta.schema.classes[t.class])throw Error('metadataIntegrity');
  for(const [key,p] of Object.entries(t.properties)){const schema=meta.schema.classes[t.class].properties[key];if(!schema||schema.type!=='SCALAR'||('normalized' in schema&&schema.normalized!==false)||('array' in schema&&schema.array!==false)||['count','offset','scale','noData','default'].some(k=>k in schema||k in p))throw Error('metadataIntegrity');if(!j.bufferViews[p.values])throw Error('metadataIntegrity');const raw=await view(p.values),d=new DataView(raw.buffer,raw.byteOffset,raw.byteLength),wide=schema.componentType==='UINT64',width=wide?8:schema.componentType==='UINT8'?1:4;
   if(!['UINT8','UINT32','UINT64'].includes(schema.componentType)||raw.length<t.count*width)throw Error('metadataIntegrity');
   props[key]=Array.from({length:t.count},(_,i)=>wide?d.getBigUint64(i*8,true).toString():width===1?d.getUint8(i):d.getUint32(i*4,true));
  }tables.push({class:t.class,count:t.count,props});
 }
 const faces=[];
 async function visit(id,parent,stack){if(stack.has(id))throw Error('scene cycle');const n=j.nodes[id],m=mul(parent,matrix(n));stack=new Set([...stack,id]);
  if(n.mesh!==undefined)for(const [pi,p] of j.meshes[n.mesh].primitives.entries()){
   if((p.mode??4)!==4)throw Error('unsupported oracle primitive mode');
   const local=await acc(p.attributes.POSITION);
   if(j.accessors[p.attributes.POSITION].type!=='VEC3')throw Error('accessorValues: POSITION');
   const pos=local.map(v=>point(m,v)),tolerance=positionTolerance(accessorTolerance(p.attributes.POSITION,local,3),local,m),idx=p.indices===undefined?pos.map((_,i)=>i):(await acc(p.indices)).flat(),sets=[];
   const directions=p.attributes.NORMAL===undefined?null:await acc(p.attributes.NORMAL),uvs=p.attributes.TEXCOORD_0===undefined?null:await acc(p.attributes.TEXCOORD_0);
   if(directions&&(directions.length!==pos.length||j.accessors[p.attributes.NORMAL].type!=='VEC3')||uvs&&(uvs.length!==pos.length||j.accessors[p.attributes.TEXCOORD_0].type!=='VEC2'))throw Error('accessorValues: NORMAL/TEXCOORD_0 shape');
   const nm=directions?normalMatrix(m):null,normals=directions?.map(v=>normal(nm,v));
   let normalTolerance=null;
   if(directions){
    const a=j.accessors[p.attributes.NORMAL],filterError=viewTolerances.get(a.bufferView)?.normalError??0;
    const error=filterError+Math.hypot(...accessorTolerance(p.attributes.NORMAL,directions,3));
    // ||L||_F bounds ||L||_2. Renormalization changes the error by at most
    // 2*||L||/||L*n||, so nonuniform/reflected scales remain accounted for.
    normalTolerance=directions.reduce((bound,n)=>Math.max(bound,Math.min(2,2*Math.hypot(...nm)*error/Math.hypot(...direction(nm,n))+64*Number.EPSILON)),0);
   }
   const uvTolerance=uvs?accessorTolerance(p.attributes.TEXCOORD_0,uvs,2):null;
   for(const f of p.extensions?.EXT_mesh_features?.featureIds??[]){if(f.attribute===undefined){if(['source_primitive','source_triangle'].includes(f.label))throw Error('featureIdentity');continue;}if(p.attributes[`_FEATURE_ID_${f.attribute}`]===undefined)throw Error('featureIdentity');const values=(await acc(p.attributes[`_FEATURE_ID_${f.attribute}`])).flat();const table=tables[f.propertyTable];
    if(!table||table.class!==f.label||f.featureCount!==table.count||values.length!==pos.length||values.some(i=>!Number.isInteger(i)||i<0||i>=table.count||i===f.nullFeatureId))throw Error('featureIdentity');sets.push({label:f.label,values,table});}
   if(idx.length%3||idx.some(i=>!Number.isSafeInteger(i)||i<0||i>=pos.length))throw Error('primitiveIndices');
   for(let k=0;k<idx.length;k+=3){const ids={};for(const s of sets){const a=s.values[idx[k]];if([1,2].some(c=>s.values[idx[k+c]]!==a))throw Error('featureIdentity');ids[s.label]=Object.fromEntries(Object.entries(s.table.props).map(([key,col])=>[key,col[a]]));}
    const corners=idx.slice(k,k+3);
    faces.push({xyz:corners.map(i=>pos[i]),normals:normals?corners.map(i=>normals[i]):null,uvs:uvs?corners.map(i=>uvs[i]):null,positionTolerance:tolerance,normalTolerance,uvTolerance,ids,source:[id,n.mesh,pi,k/3],material:p.material===undefined?null:j.materials[p.material]});
   }
  }
  for(const child of n.children??[])await visit(child,m,stack);
 }
 for(const id of j.scenes[j.scene??0].nodes)await visit(id,I,new Set());
 const hasHlodIdentity=tables.some(t=>['source_primitive','source_triangle'].includes(t.class))||faces.some(f=>Object.keys(f.ids).some(k=>['source_primitive','source_triangle'].includes(k)));
 return {faces,tables,hasHlodIdentity};
}
if(process.argv[1]===new URL(import.meta.url).pathname){
 try{process.stdout.write(JSON.stringify(await decode(process.argv[2]==='-'?fs.readFileSync(0):process.argv[2])));}catch(e){console.error(e.stack);process.exitCode=1;}
}
