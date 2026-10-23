// Decode KTX2 to RGBA32 without a browser or GPU.
const fs = require('fs');
const base = process.env.BASIS_DIR || '/home/bend/.cache/rusty-tiles-117-browser-cache/node_modules/three/examples/jsm/libs/basis';
const mod = {exports:{}};
new Function('module','exports','require','__filename','__dirname', fs.readFileSync(base+'/basis_transcoder.js','utf8'))(mod,mod.exports,require,base+'/basis_transcoder.js',base);
(async()=>{
 const B=await mod.exports({wasmBinary:fs.readFileSync(base+'/basis_transcoder.wasm')});B.initializeBasis();
 for(const p of JSON.parse(fs.readFileSync(process.argv[2],'utf8'))){
  const f=new B.KTX2File(new Uint8Array(fs.readFileSync(p)));if(!f.isValid()||!f.startTranscoding())throw Error(p);
  const w=f.getWidth(),h=f.getHeight();const a=new Uint8Array(f.getImageTranscodedSizeInBytes(0,0,0,13));
  if(!f.transcodeImage(a,0,0,0,13,0,-1,-1))throw Error('transcode '+p);
  fs.writeFileSync(p+'.rgba',a);fs.writeFileSync(p+'.json',JSON.stringify([w,h]));f.close();f.delete();
 }
})();
