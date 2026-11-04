// usage: node enc.mjs KIND N STRIDE OUT   (upstream meshoptimizer encoder; vertex codec v0)
import fs from 'node:fs';
import { pathToFileURL } from 'node:url';
// MESHOPT_JS: directory of the meshoptimizer 1.3.0 npm package (default ./meshoptimizer).
const dir = process.env.MESHOPT_JS ?? new URL('./meshoptimizer', import.meta.url).pathname;
const { MeshoptEncoder } = await import(pathToFileURL(`${dir}/meshopt_encoder.js`).href);
await MeshoptEncoder.ready;
const [kind, ns, ss, out] = process.argv.slice(2); const n = +ns, stride = +ss;
let r;
if (kind === 'raw') r = MeshoptEncoder.encodeVertexBufferLevel(new Uint8Array(fs.readFileSync(out + '.raw')), n, stride, 2, 0);
else if (kind === 'zeros') r = MeshoptEncoder.encodeVertexBufferLevel(new Uint8Array(n * stride), n, stride, 2, 0);
else {
  const idx = new Uint32Array(3 * n);
  for (let t = 0; t < n; t++) for (let k = 0; k < 3; k++)
    idx[3 * t + k] = kind === 'tri-seq' ? 3 * t + k : kind === 'tri-repeat' ? k : kind === 'tri-fan' ? (k === 0 ? 0 : t + k) : NaN;
  let src = idx;
  if (stride === 2) src = Uint16Array.from(idx);
  r = MeshoptEncoder.encodeIndexBuffer(new Uint8Array(src.buffer), 3 * n, stride);
}
fs.writeFileSync(out, r);
console.log(JSON.stringify({ kind, n, stride, bytes: r.length, perByte: +(n * (kind === 'zeros' ? 1 : 3) / r.length).toFixed(3) }));
