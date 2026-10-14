#!/usr/bin/env node
// Independent published-byte reader; no Rust producer/atlas helper is called.
const fs = require('node:fs');
const crypto = require('node:crypto');
const modulePath = process.env.H8_MESHOPT_DECODER || '/home/bend/.cache/rusty-tiles-117-browser-cache/node_modules/meshoptimizer/meshopt_decoder.cjs';
const decoderModule = require(modulePath);
const Decoder = decoderModule.MeshoptDecoder || decoderModule;
const assert = (x, message) => { if (!x) throw new Error(message); };
(async () => {
  await Decoder.ready;
  const bytes = fs.readFileSync(process.argv[2]);
  assert(bytes.length <= 96 * 1024 * 1024, 'leaf member exceeds 96 MiB');
  assert(bytes.subarray(0, 4).toString() === 'glTF' && bytes.readUInt32LE(4) === 2, 'GLB2 header');
  assert(bytes.readUInt32LE(8) === bytes.length, 'GLB length');
  const jsonLength = bytes.readUInt32LE(12);
  assert(jsonLength % 4 === 0 && bytes.subarray(16, 20).toString() === 'JSON', 'JSON chunk');
  const document = JSON.parse(bytes.subarray(20, 20 + jsonLength).toString());
  const binHeader = 20 + jsonLength;
  assert(bytes.subarray(binHeader + 4, binHeader + 8).equals(Buffer.from([66, 73, 78, 0])), 'BIN chunk');
  const binary = bytes.subarray(binHeader + 8);
  assert(bytes.readUInt32LE(binHeader) === binary.length && binary.length % 4 === 0, 'BIN length');
  const range = (start, length) => {
    assert(Number.isSafeInteger(start) && Number.isSafeInteger(length) && start >= 0 && length > 0 && start + length <= binary.length, 'wire byte range');
    return binary.subarray(start, start + length);
  };
  const views = document.bufferViews.map(view => {
    const extension = view.extensions && view.extensions.EXT_meshopt_compression;
    if (!extension) {
      assert(view.buffer === 0, 'uncompressed view uses GLB buffer');
      return range(view.byteOffset || 0, view.byteLength).toString('base64');
    }
    assert(document.extensionsRequired.includes('EXT_meshopt_compression'), 'meshopt required declaration');
    // NONE is lossless wire compression; OCTAHEDRAL carries the profile's lossy leaf NORMAL.
    assert(extension.buffer === 0 && extension.mode === 'ATTRIBUTES' && (extension.filter === 'NONE' || (extension.filter === 'OCTAHEDRAL' && extension.byteStride === 8)), 'meshopt mode/filter');
    const compressed = range(extension.byteOffset || 0, extension.byteLength);
    assert(compressed[0] === 0xa0, 'pinned ATTRIBUTES codec header');
    assert(extension.count * extension.byteStride === view.byteLength, 'decoded view size');
    const decoded = new Uint8Array(view.byteLength);
    Decoder.decodeGltfBuffer(decoded, extension.count, extension.byteStride, compressed, extension.mode, extension.filter);
    return Buffer.from(decoded).toString('base64');
  });
  const sha = data => crypto.createHash('sha256').update(data).digest('hex');
  process.stdout.write(JSON.stringify({schemaVersion: 1, document, views, wireSha256: sha(bytes), decoderSha256: sha(fs.readFileSync(modulePath)), decoderPath: modulePath}));
})().catch(error => { process.stderr.write(String(error.stack || error) + '\n'); process.exitCode = 1; });
