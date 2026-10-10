// Root supplies exact pinned upstream MeshoptDecoder module, no installs/fetch.
// This independent consumer shares upstream codec lineage with Rust meshopt;
// byte comparison is independent of the producer path, not kernel diversity.
import fs from 'node:fs'; import path from 'node:path'; import crypto from 'node:crypto';
import {createRequire} from 'node:module';
const [root,decoderPath,decoderSha]=process.argv.slice(2);
if(!root||!decoderPath||!decoderSha)throw Error('usage: node consumer.mjs EXTRACTED_ROOT PINNED_COMMONJS_DECODER SHA256');
const hash=b=>crypto.createHash('sha256').update(b).digest('hex');
if(hash(fs.readFileSync(decoderPath))!==decoderSha)throw Error('decoder pin mismatch');
const mod=createRequire(import.meta.url)(path.resolve(decoderPath));const decoder=mod.MeshoptDecoder||mod;
await decoder.ready;let records=[];
for(const name of fs.readdirSync(root).filter(x=>x.startsWith('payload-'))){
 const dir=path.join(root,name);const doc=JSON.parse(fs.readFileSync(path.join(dir,'document.json')));
 for(let i=0;i<doc.bufferViews.length;i++){
  const v=doc.bufferViews[i];const e=v.extensions?.EXT_meshopt_compression;if(!e)continue;
  if(e.mode!=='ATTRIBUTES'||(e.filter||'NONE')!=='NONE')throw Error('outside prepared profile');
  const source=fs.readFileSync(path.join(dir,`encoded-${String(i).padStart(4,'0')}.bin`));
  const target=new Uint8Array(e.count*e.byteStride);if(target.length!==v.byteLength)throw Error('decoded length mismatch');
  decoder.decodeGltfBuffer(target,e.count,e.byteStride,source,e.mode,e.filter||'NONE');
  const out=path.join(dir,`view-${String(i).padStart(4,'0')}.bin`);if(fs.existsSync(out))throw Error('refuse decoded overwrite');
  fs.writeFileSync(out,target);records.push({payload:name,view:i,count:e.count,stride:e.byteStride,sha256:hash(target)});
 }
}
fs.writeFileSync(path.join(root,'consumer-receipt.json'),JSON.stringify({decoderPath,decoderSha,
 scriptSha256:hash(fs.readFileSync(new URL(import.meta.url))),scope:'upstream shared-kernel consumer; no semantic validity inference',records},null,2)+'\n');
