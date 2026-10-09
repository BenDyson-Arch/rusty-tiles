// Khronos validator supplements the independent source/geometry oracle.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const validator = require('gltf-validator');
(async () => {
  const root = process.argv[2];
  const tileset = JSON.parse(fs.readFileSync(path.join(root, 'tileset.json')));
  const reports = [];
  for (const leaf of tileset.root.children) {
    const uri = leaf.content.uri;
    const report = await validator.validateBytes(new Uint8Array(fs.readFileSync(path.join(root, uri))), {uri, maxIssues: 100});
    assert.equal(report.issues.numErrors, 0, JSON.stringify(report.issues));
    assert.equal(report.issues.numWarnings, 0, JSON.stringify(report.issues));
    reports.push({uri, validatorVersion: report.validatorVersion, issues: report.issues});
  }
  assert(reports.length > 0);
  console.log(JSON.stringify({ok: true, reports}, null, 2));
})().catch(error => {console.error(error); process.exitCode = 1;});
