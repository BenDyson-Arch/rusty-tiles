import fs from 'node:fs';
import {pathToFileURL} from 'node:url';
const {MeshoptDecoder} = await import(pathToFileURL(process.argv[2]).href);
const jobs = JSON.parse(fs.readFileSync(0,'utf8'));
process.stdout.write(JSON.stringify(jobs.map(j => {
  const out = new Uint8Array(j.count*j.stride);
  MeshoptDecoder.decodeGltfBuffer(out,j.count,j.stride,Buffer.from(j.data,'base64'),j.mode,'NONE');
  return Buffer.from(out).toString('base64');
})));
