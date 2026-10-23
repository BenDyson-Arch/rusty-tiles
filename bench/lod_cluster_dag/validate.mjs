import fs from 'node:fs';
import path from 'node:path';
const cache=process.env.MESHOPT_JS || '/home/bend/.cache/rusty-tiles-117-browser-cache/node_modules';
const {default:validator}=await import(`${cache}/gltf-validator/index.js`);
const [root,out]=process.argv.slice(2);const rows=[];
for(const file of fs.readdirSync(path.join(root,'content')).filter(x=>x.endsWith('.glb'))){
 const f=path.join(root,'content',file);
 const r=await validator.validateBytes(new Uint8Array(fs.readFileSync(f)),{
  maxIssues:0, externalResourceFunction:uri=>Promise.resolve(new Uint8Array(fs.readFileSync(path.resolve(path.dirname(f),uri))))});
 rows.push({file,errors:r.issues.numErrors,warnings:r.issues.numWarnings,
   problems:r.issues.messages.filter(m=>m.severity<2),unsupported:r.issues.messages.filter(m=>m.code==='UNSUPPORTED_EXTENSION').map(m=>m.message)});
}
fs.writeFileSync(out,JSON.stringify(rows,null,2));console.log({files:rows.length,errors:rows.reduce((a,r)=>a+r.errors,0),warnings:rows.reduce((a,r)=>a+r.warnings,0)});
