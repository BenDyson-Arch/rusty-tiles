#!/usr/bin/env node
// Decode every bufferView of each GLB listed on stdin (one path per line) with the
// reference meshopt decoder; write one JSON line per GLB: {path, document, views(base64)}.
const fs = require('node:fs');
const readline = require('node:readline');
const modulePath = '/home/bend/.cache/rusty-tiles-117-browser-cache/node_modules/meshoptimizer/meshopt_decoder.cjs';
const Decoder = require(modulePath).MeshoptDecoder || require(modulePath);
(async () => {
  await Decoder.ready;
  const rl = readline.createInterface({input: process.stdin});
  for await (const path of rl) {
    if (!path) continue;
    const bytes = fs.readFileSync(path);
    const jsonLength = bytes.readUInt32LE(12);
    const document = JSON.parse(bytes.subarray(20, 20 + jsonLength).toString());
    const binary = bytes.subarray(20 + jsonLength + 8);
    const views = document.bufferViews.map(view => {
      const e = view.extensions && view.extensions.EXT_meshopt_compression;
      if (!e) return binary.subarray(view.byteOffset || 0, (view.byteOffset || 0) + view.byteLength).toString('base64');
      const out = new Uint8Array(e.count * e.byteStride);
      Decoder.decodeGltfBuffer(out, e.count, e.byteStride, binary.subarray(e.byteOffset || 0, (e.byteOffset || 0) + e.byteLength), e.mode, e.filter || 'NONE');
      return Buffer.from(out).toString('base64');
    });
    process.stdout.write(JSON.stringify({path, bytes: bytes.length, document, views}) + '\n');
  }
})().catch(error => { process.stderr.write(String(error.stack || error) + '\n'); process.exitCode = 1; });
