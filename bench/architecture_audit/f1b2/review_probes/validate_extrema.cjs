// Official Khronos validator, independent manually authored resource callbacks.
const fs = require('node:fs');
const validator = require('/home/bend/.cache/rusty-tiles-135-validator/node_modules/gltf-validator');
const path = require('node:path');
const doc = {
  asset: {version:'2.0'},
  buffers:[{uri:'manual.bin',byteLength:36}],
  bufferViews:[{buffer:0,byteLength:36,target:34962}],
  accessors:[{bufferView:0,componentType:5126,type:'VEC3',count:3,min:[0,0,0],max:[100,100,100]}],
  meshes:[{primitives:[{attributes:{POSITION:0}}]}],nodes:[{mesh:0}],scenes:[{nodes:[0]}],scene:0
};
const bytes=Buffer.alloc(36);
[11,21,31,13,21,31,11,24,31].forEach((x,i)=>bytes.writeFloatLE(x,i*4));
validator.validateString(JSON.stringify(doc), {
  uri:'manual.gltf', externalResourceFunction: async uri => {
    if(uri !== 'manual.bin') throw new Error('Unexpected URI');
    return new Uint8Array(bytes);
  }
}).then(report=> {
  fs.writeFileSync(path.join(__dirname,'extrema-validator-receipt.json'), JSON.stringify({validator_version:validator.version(),document:doc,positions:[11,21,31,13,21,31,11,24,31],report},null,2)+'\n');
  console.log(JSON.stringify(report.issues,null,2));
});
