// This wrapper supplies bytes to the separately pinned upstream reference codec.
import fs from 'node:fs';
import {pathToFileURL} from 'node:url';
const {MeshoptDecoder} = await import(pathToFileURL(process.argv[2]).href);
const requests = JSON.parse(fs.readFileSync(0, 'utf8'));
const result = requests.map(r => {
  const source = Buffer.from(r.bytes, 'base64');
  const target = new Uint8Array(r.count * r.stride);
  MeshoptDecoder.decodeGltfBuffer(target, r.count, r.stride, source, r.mode, r.filter);
  return Buffer.from(target).toString('base64');
});
process.stdout.write(JSON.stringify(result));
