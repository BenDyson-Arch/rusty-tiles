// Actual pinned Cesium modules, no algorithm monkey patch, no WebGL/network.
// This tests materialization and external loading, not viewer selection/picking.
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import assert from 'node:assert/strict';
import { pathToFileURL } from 'node:url';

const [packageDirectory, artifactDirectory, output] = process.argv.slice(2);
assert(packageDirectory && artifactDirectory && output);
const base = path.resolve(packageDirectory);
const src = path.join(base, 'Source');
async function load(name) { return (await import(pathToFileURL(path.join(src, name)).href)).default; }
const Cesium3DTileset = await load('Scene/Cesium3DTileset.js');
const Resource = await load('Core/Resource.js');
const MetadataSchema = await load('Scene/MetadataSchema.js');
const Implicit3DTileContent = await load('Scene/Implicit3DTileContent.js');
const Tileset3DTileContent = await load('Scene/Tileset3DTileContent.js');
const Matrix4 = (await import(pathToFileURL(path.join(base, '../core/index.js')).href)).Matrix4;
const sha = bytes => crypto.createHash('sha256').update(bytes).digest('hex');
const manifest = JSON.parse(fs.readFileSync(path.join(artifactDirectory, 'manifest.json')));
const results = {kind: 'actual-client-materialization-not-browser-selection', node: process.version,
  clientPackage: JSON.parse(fs.readFileSync(path.join(base, 'package.json'))), cases: {},
  driver_sha256: sha(fs.readFileSync(new URL(import.meta.url)))};
results.clientPackage = {name: results.clientPackage.name, version: results.clientPackage.version};
results.runtime_package_sha256 = {};
for (const name of ['package.json','index.js','../core/package.json','../core/index.js','../../cesium/package.json','../../.package-lock.json']) {
  results.runtime_package_sha256[name] = sha(fs.readFileSync(path.resolve(base,name)));
}
results.runtime_tree_sha256 = {};
function sourceTree(directory) {
  const entries = [];
  function walk(folder) {
    for (const name of fs.readdirSync(folder).sort()) {
      const file = path.join(folder,name);
      if (fs.statSync(file).isDirectory()) walk(file);
      else if (name.endsWith('.js')) entries.push([path.relative(directory,file),sha(fs.readFileSync(file))]);
    }
  }
  walk(directory);
  entries.sort((a,b) => a[0]<b[0]?-1:a[0]>b[0]?1:0);
  return {js_files:entries.length, sha256:sha(JSON.stringify(entries))};
}
results.runtime_tree_sha256.engine = sourceTree(src);
results.runtime_tree_sha256.core = sourceTree(path.join(base,'../core/Source'));
results.runtime_source_sha256 = {};
for (const name of ['Scene/Cesium3DTileset.js', 'Scene/Cesium3DTile.js', 'Scene/Implicit3DTileContent.js',
  'Scene/ImplicitSubtree.js', 'Scene/ImplicitTileset.js', 'Scene/Multiple3DTileContent.js',
  'Scene/Tileset3DTileContent.js', 'Scene/MetadataSchema.js', 'Scene/MetadataTable.js', 'Scene/MetadataSemantic.js']) {
  results.runtime_source_sha256[name] = sha(fs.readFileSync(path.join(src, name)));
}
for (const [caseName, input] of Object.entries(manifest.cases)) {
  const directory = path.resolve(artifactDirectory, caseName);
  for (const [name, hash] of Object.entries(input.member_sha256)) assert.equal(sha(fs.readFileSync(path.join(directory, name))), hash);
  if (input.kind === 'metadata-transform') {
    const tileset = new Cesium3DTileset({cullWithChildrenBounds:false});
    const entry = JSON.parse(fs.readFileSync(path.join(directory, 'tileset.json')));
    tileset._metadataExtension = {schema:MetadataSchema.fromJson(entry.schema)};
    const resource = new Resource({url:'http://127.0.0.1:8999/tileset.json'});
    const placeholder = tileset.loadTileset(resource, entry);
    const uri = placeholder.implicitTileset.subtreeUriTemplate.getDerivedResource({templateValues:placeholder.implicitCoordinates.getTemplateValues()}).url;
    const raw = fs.readFileSync(path.join(directory, new URL(uri, resource.url).pathname.slice(1)));
    const bytes = raw.buffer.slice(raw.byteOffset, raw.byteOffset+raw.byteLength);
    await Implicit3DTileContent.fromSubtreeJson(tileset,placeholder,resource,undefined,bytes,0);
    const root = placeholder.children[0];
    const present = root.metadata.getPropertyBySemantic('TILE_TRANSFORM');
    const actual = Array.from(root.computedTransform);
    assert.deepEqual(Array.from(present),input.expected_local_transform, 'semantic must be present and readable');
    assert.deepEqual(actual,Array.from(Matrix4.IDENTITY), 'pinned client ignores implicit TILE_TRANSFORM');
    assert.notDeepEqual(actual,input.expected_local_transform);
    results.cases[caseName] = {semantic_present:Array.from(present), computed_transform:actual,
      semantic_applied:false, unsupported_for_byte_preserving_single_tree:true};
    continue;
  }
  const source = JSON.parse(fs.readFileSync(path.join(directory, 'source.json')));
  const roots = [];
  const emitted = [];
  const tileset = new Cesium3DTileset({cullWithChildrenBounds: false});
  const entry = JSON.parse(fs.readFileSync(path.join(directory, 'tileset.json')));
  // fromUrl performs this parsing asynchronously. Direct schema setup avoids a
  // network/DOM dependency without altering any client materialization code.
  tileset._metadataExtension = {schema: MetadataSchema.fromJson(entry.schema)};
  let count = 0;
  let initialRoot;
  async function visit(document, parentTile, nid, uri) {
    assert(++count <= 8, 'bounded document count');
    const resource = new Resource({url: 'http://127.0.0.1:8999/'+uri});
    const placeholder = parentTile ? (Tileset3DTileContent.fromJson(tileset, parentTile, resource, document),
      parentTile.children[parentTile.children.length-1]) : tileset.loadTileset(resource, document);
    const values = placeholder.implicitCoordinates.getTemplateValues();
    const subtreeUri = placeholder.implicitTileset.subtreeUriTemplate.getDerivedResource({templateValues: values}).url;
    const localName = new URL(subtreeUri, resource.url).pathname.slice(1);
    const raw = fs.readFileSync(path.join(directory, localName));
    const bytes = raw.buffer.slice(raw.byteOffset, raw.byteOffset+raw.byteLength);
    const content = await Implicit3DTileContent.fromSubtreeJson(tileset, placeholder, resource, undefined, bytes, 0);
    assert(content.ready);
    const root = placeholder.children[0];
    if (nid === 'n0') initialRoot = root;
    const directChildren = root.children;
    roots.push({source_id: nid, computed_transform: Array.from(root.computedTransform),
      header_box: root._header.boundingVolume.box, geometric_error: root.geometricError,
      root_available_uris: (root._header.contents || []).map(c => new URL(c.uri, resource.url).pathname.slice(1))});
    const original = source.explicit_nodes.find(n => n.id === nid);
    assert.deepEqual(root._header.boundingVolume.box, original.bounds);
    assert.equal(root.geometricError, original.error);
    for (let i=0; i<original.payloads.length; i++) {
      const uri=roots.at(-1).root_available_uris[i];
      assert.equal(sha(fs.readFileSync(path.join(directory,uri))),sha(fs.readFileSync(path.join(directory,original.payloads[i]+'.glb'))));
    }
    if (input.kind !== 'root-external-available') {
      assert.equal(roots.at(-1).root_available_uris.filter(n => n.endsWith('.json')).length, 0);
    }
    assert.equal(directChildren.length, original.children.length);
    for (let i = 0; i < directChildren.length; i++) {
      const child = directChildren[i];
      const coordinates = child.implicitCoordinates.getTemplateValues();
      const available = child._header.contents || [];
      const uris = available.map(c => new URL(c.uri, resource.url).pathname.slice(1));
      assert.equal(uris.length, 1);
      assert(uris[0].endsWith('.json'));
      assert.equal(child.children.length, 0, 'terminal link has no implicit children');
      emitted.push({source_parent: nid, source_child: original.children[i], coordinates,
        computed_transform: Array.from(child.computedTransform),
        semantic_box: child._header.boundingVolume.box, geometric_error: child.geometricError,
        uri: uris[0], implicit_children_before_external_load: child.children.length});
      await visit(JSON.parse(fs.readFileSync(path.join(directory, uris[0]))), child, original.children[i], uris[0]);
    }
  }
  await visit(entry, undefined, 'n0', 'tileset.json');
  const actual = Object.fromEntries(roots.map(r => [r.source_id, r.computed_transform]));
  const expected = {};
  const identity = Array.from(Matrix4.IDENTITY);
  function expectedVisit(nid, frame) {
    const n = source.explicit_nodes.find(x => x.id === nid);
    const matrix = Matrix4.multiply(Matrix4.fromArray(frame), Matrix4.fromArray(n.transform), new Matrix4());
    expected[nid] = Array.from(matrix);
    for (const cid of n.children) expectedVisit(cid, expected[nid]);
  }
  expectedVisit('n0', identity);
  const equal = Object.keys(expected).every(nid => JSON.stringify(expected[nid]) === JSON.stringify(actual[nid]));
  if (input.kind === 'candidate') assert(equal, 'valid candidate source composition');
  if (input.kind === 'double-child-transform') assert(!equal, 'frame control must be detected');
  let forbiddenRootExternalLoaded=false;
  if (input.kind === 'root-external-available') {
    const uri=roots[0].root_available_uris.find(n=>n.endsWith('.json'));
    const previousCount=initialRoot.children.length;
    const external=Tileset3DTileContent.fromJson(tileset,initialRoot,
      new Resource({url:'http://127.0.0.1:8999/'+uri}), JSON.parse(fs.readFileSync(path.join(directory,uri))));
    assert(external.ready);
    assert.equal(initialRoot.children.length, previousCount+1);
    forbiddenRootExternalLoaded=true;
  }
  results.cases[caseName] = {roots, terminal_links: emitted, source_frame_equal: equal,
    root_external_slot_materialized: roots[0].root_available_uris.some(uri => uri.endsWith('.json')),
    forbidden_root_external_loaded_without_client_rejection:forbiddenRootExternalLoaded,
    no_webgl_no_content_rendering: true};
}
// The primary format assigns TILE_TRANSFORM; the pinned intended consumer's
// implicit materializer is tested with a present/readable override property.
fs.writeFileSync(output, JSON.stringify(results, null, 2)+'\n');
console.log(JSON.stringify({cases:Object.keys(results.cases).length,
  candidate_source_frames_equal:Object.entries(results.cases).filter(([k])=>k.endsWith('-candidate')).every(([,v])=>v.source_frame_equal),
  scope:results.kind}));
