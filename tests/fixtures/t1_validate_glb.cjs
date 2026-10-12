// Khronos validator supplements the independent source/geometry oracle.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const validator = require('gltf-validator');
(async () => {
  const root = process.argv[2];
  // Every tile payload of the implicit forest, coarse levels and leaves alike.
  const uris = fs.readdirSync(root, {recursive: true}).map(name => name.split(path.sep).join('/'))
    .filter(name => name.endsWith('.glb')).sort();
  const reports = [];
  for (const uri of uris) {
    const report = await validator.validateBytes(new Uint8Array(fs.readFileSync(path.join(root, uri))), {uri, maxIssues: 100});
    assert.equal(report.issues.numErrors, 0, JSON.stringify(report.issues));
    assert.equal(report.issues.numWarnings, 0, JSON.stringify(report.issues));
    reports.push({uri, validatorVersion: report.validatorVersion, issues: report.issues});
  }
  assert(reports.length > 0);
  console.log(JSON.stringify({ok: true, reports}, null, 2));
})().catch(error => {console.error(error); process.exitCode = 1;});
