// Independently authored consumer feasibility. No production writer or decoder.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import { pathToFileURL } from 'node:url';
const engine='/home/bend/.cache/rusty-tiles-117-browser-cache/node_modules/@cesium/engine';
const get=async p=>(await import(pathToFileURL(path.join(engine,'Source',p)))).default;
const MetadataSchema=await get('Scene/MetadataSchema.js');
const parse=await get('Scene/parseStructuralMetadata.js');
const ModelFeature=await get('Scene/Model/ModelFeature.js');
const ModelUtility=await get('Scene/Model/ModelUtility.js');
const schema={id:'rusty_tiles_source_v1',classes:{
 source_primitive:{properties:{node_index:{type:'SCALAR',componentType:'UINT32',required:true},mesh_index:{type:'SCALAR',componentType:'UINT32',required:true},primitive_index:{type:'SCALAR',componentType:'UINT32',required:true},node_name:{type:'STRING',required:true},node_name_present:{type:'SCALAR',componentType:'UINT8',required:true}}},
 source_triangle:{properties:Object.fromEntries(['node_index','mesh_index','primitive_index','triangle_index'].map(k=>[k,{type:'SCALAR',componentType:'UINT32',required:true}]))}
}};
const expected=[{node_index:3,mesh_index:2,primitive_index:4,node_name:'Building 🦉\0',node_name_present:1},{node_index:7,mesh_index:2,primitive_index:4,node_name:'Building 🦉\0',node_name_present:1},{node_index:8,mesh_index:2,primitive_index:4,node_name:'',node_name_present:0},{node_index:9,mesh_index:2,primitive_index:4,node_name:'',node_name_present:1}];
const views={};let view=0;
const add=b=>{const i=view++;views[i]=new Uint8Array(b);return i;};
const u32=a=>{const b=Buffer.alloc(a.length*4);a.forEach((v,i)=>b.writeUInt32LE(v,i*4));return add(b);};
const names=expected.map(r=>Buffer.from(r.node_name,'utf8'));let at=0;const offsets=[0];for(const b of names){at+=b.length;offsets.push(at);}
const properties=Object.fromEntries(['node_index','mesh_index','primitive_index'].map(k=>[k,{values:u32(expected.map(r=>r[k]))}]));
properties.node_name={values:add(Buffer.concat(names)),stringOffsets:u32(offsets),stringOffsetType:'UINT32'};
properties.node_name_present={values:add(expected.map(r=>r.node_name_present))};
const triProperties=Object.fromEntries(['node_index','mesh_index','primitive_index','triangle_index'].map(k=>[k,{values:u32(expected.map((r,i)=>k==='triangle_index'?65536+i:r[k]))}]));
const extension={schema,propertyTables:[{class:'source_primitive',count:4,properties},{class:'source_triangle',count:4,properties:triProperties}]};
const metadata=parse({extension,schema:MetadataSchema.fromJson(schema),bufferViews:views});
const queries=[];for(let tableIndex=0;tableIndex<2;tableIndex++){
 const table=metadata.getPropertyTable(tableIndex);
 for(let i=0;i<4;i++){
  // Exercise real ModelFeature.getProperty against the consumer property table.
  const feature=new ModelFeature({model:{},featureId:i,featureTable:{getProperty:(row,k)=>table.getProperty(row,k)}});
  const keys=tableIndex===0?Object.keys(expected[i]):['node_index','mesh_index','primitive_index','triangle_index'];
  const row=Object.fromEntries(keys.map(k=>[k,feature.getProperty(k)]));
  assert.deepEqual(row,tableIndex===0?expected[i]:{node_index:expected[i].node_index,mesh_index:2,primitive_index:4,triangle_index:65536+i});
  queries.push({table:tableIndex,row:i,properties:row});
 }
}
const featureSets=[{label:'source_primitive',positionalLabel:'featureId_0',propertyTableId:0},{label:'source_triangle',positionalLabel:'featureId_1',propertyTableId:1}];
assert.equal(ModelUtility.getFeatureIdsByLabel(featureSets,'featureId_0').propertyTableId,0);
assert.equal(ModelUtility.getFeatureIdsByLabel(featureSets,'source_triangle').propertyTableId,1);
assert.equal(ModelUtility.getFeatureIdsByLabel(featureSets,'missing'),undefined);
for(const id of [0,65535,65536,99999])assert.equal(new Float32Array([id])[0],id);
// Explicit corruption controls: geometry is irrelevant to this association oracle.
let detected=0;
for(const control of [()=>assert.deepEqual(queries[1].properties,expected[0]),()=>assert.deepEqual(queries[2].properties,expected[3]),()=>assert.equal(ModelUtility.getFeatureIdsByLabel(featureSets,'source_triangle').propertyTableId,0),()=>assert.equal(queries[4].properties.triangle_index,0)]){
 try{control();}catch(e){if(e instanceof assert.AssertionError)detected++;else throw e;}
}
assert.equal(detected,4);
const pins=['package.json','Source/Scene/parseStructuralMetadata.js','Source/Scene/MetadataSchema.js','Source/Scene/Model/ModelFeature.js','Source/Scene/Model/ModelUtility.js'].map(p=>{const b=fs.readFileSync(path.join(engine,p));return{path:p,sha256:crypto.createHash('sha256').update(b).digest('hex')};});
const result={scope:'Independent metadata decoding and ModelFeature property-query feasibility, not browser/GPU picking or production acceptance',engine:JSON.parse(fs.readFileSync(path.join(engine,'package.json'))).version,script_sha256:crypto.createHash('sha256').update(fs.readFileSync(new URL(import.meta.url))).digest('hex'),pins,queries,feature_label_queries:['featureId_0','source_triangle'],exact_f32_ids:[0,65535,65536,99999],sensitive_controls_detected:detected,passed:true};
fs.writeFileSync(new URL('identity-feasibility.json',import.meta.url),JSON.stringify(result,null,2)+'\n');
console.log(JSON.stringify({passed:true,queries:queries.length,sensitive_controls_detected:detected,engine:result.engine}));
