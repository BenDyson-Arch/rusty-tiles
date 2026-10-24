// Bounded, one-GLB-at-a-time audit of the supplied H8/H17 outputs, not a glTF loader.
// Derived from texel_density.mjs; fails rather than guessing unsupported transforms.
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import {pathToFileURL} from 'node:url';
const [root, index, decoder] = process.argv.slice(2);
const {MeshoptDecoder} = await import(pathToFileURL(decoder));
await MeshoptDecoder.ready;
const rows = JSON.parse(fs.readFileSync(index));
for (const info of rows) {
  const b = fs.readFileSync(path.join(root, info.name));
  if (b.toString('ascii', 0, 4) !== 'glTF' || b.readUInt32LE(8) !== b.length) throw Error('GLB');
  const jl = b.readUInt32LE(12), j = JSON.parse(b.subarray(20, 20 + jl));
  const bin = b.subarray(28 + jl), cache = new Map();
  const view = i => {
    if (cache.has(i)) return cache.get(i);
    const v = j.bufferViews[i], e = v.extensions?.EXT_meshopt_compression;
    let out;
    if (e) {
      if (e.buffer !== 0) throw Error('external compressed buffer');
      out = new Uint8Array(e.count * e.byteStride);
      MeshoptDecoder.decodeGltfBuffer(out, e.count, e.byteStride,
        bin.subarray(e.byteOffset || 0, (e.byteOffset || 0) + e.byteLength), e.mode, e.filter || 'NONE');
    } else {
      if (v.buffer !== 0) throw Error('external buffer');
      out = bin.subarray(v.byteOffset || 0, (v.byteOffset || 0) + v.byteLength);
    }
    cache.set(i, out); return out;
  };
  const accessor = i => {
    const a = j.accessors[i], v = j.bufferViews[a.bufferView], r = view(a.bufferView);
    if (a.sparse) throw Error('sparse');
    const [size, read, mx] = {
      5120:[1,'getInt8',127],5121:[1,'getUint8',255],5122:[2,'getInt16',32767],
      5123:[2,'getUint16',65535],5125:[4,'getUint32',4294967295],5126:[4,'getFloat32',1]
    }[a.componentType];
    const n = {SCALAR:1,VEC3:3}[a.type];
    if (!n) throw Error('accessor type');
    const dv = new DataView(r.buffer, r.byteOffset, r.byteLength);
    return {count:a.count, at:(k,c=0) => {
      const x = dv[read]((a.byteOffset || 0) + k * (v.byteStride || size*n) + c*size, true);
      return a.normalized ? Math.max(-1,x/mx) : x;
    }};
  };
  let area = 0, triangles = 0, vertices = 0;
  const meshes = new Set();
  for (const ni of j.scenes[j.scene || 0].nodes) {
    const node = j.nodes[ni];
    if (node.children || node.rotation || node.matrix || meshes.has(node.mesh)) throw Error('scene domain');
    meshes.add(node.mesh);
    const s = node.scale || [1,1,1];
    for (const pr of j.meshes[node.mesh].primitives) {
      if ((pr.mode ?? 4) !== 4) throw Error('not triangles');
      const pos = accessor(pr.attributes.POSITION), idx = pr.indices === undefined ? null : accessor(pr.indices);
      const n = idx ? idx.count : pos.count;
      if (n % 3) throw Error('triangle count');
      vertices += pos.count; triangles += n/3;
      const at = k => { const v = idx ? idx.at(k) : k; return s.map((scale,c)=>pos.at(v,c)*scale); };
      for (let k=0;k<n;k+=3) {
        const a=at(k), b=at(k+1), c=at(k+2), u=b.map((v,i)=>v-a[i]), v=c.map((v,i)=>v-a[i]);
        area += .5*Math.hypot(u[1]*v[2]-u[2]*v[1],u[2]*v[0]-u[0]*v[2],u[0]*v[1]-u[1]*v[0]);
      }
    }
  }
  if (meshes.size !== j.meshes.length) throw Error('uninstantiated mesh');
  const images = new Set((j.images || []).map(im=>im.bufferView));
  let texture_bytes = 0;
  for (const i of images) {
    if (i === undefined || j.bufferViews[i].extensions?.EXT_meshopt_compression) throw Error('image domain');
    texture_bytes += j.bufferViews[i].byteLength;
  }
  // Keep JSON/chunk overhead fixed; count BIN padding with geometry (at most a few bytes/view).
  const fixed_bytes = 28+jl;
  console.log(JSON.stringify({...info,triangles,vertices,area,bytes:b.length,texture_bytes,
    fixed_bytes,geometry_bytes:b.length-texture_bytes-fixed_bytes,
    sha256:crypto.createHash('sha256').update(b).digest('hex')}));
}
