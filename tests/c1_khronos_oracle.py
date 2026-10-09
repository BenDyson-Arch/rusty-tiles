"""Optional independent Khronos glTF positive controls; no compiler/downloads.
Requires Node and a supplied node_modules containing gltf-validator.
"""
import argparse, hashlib, json, pathlib, shutil, subprocess, tempfile, zipfile

SCRIPT = r'''
const fs = require('node:fs');
const path = require('node:path');
const validator = require('gltf-validator');
(async () => {
  const jobs = JSON.parse(fs.readFileSync(process.argv[2], 'utf8'));
  const reports = [];
  for (const job of jobs) {
    const options = {uri: job.uri, externalResourceFunction: async uri =>
      new Uint8Array(fs.readFileSync(path.resolve(path.dirname(job.path), uri)))};
    const bytes = new Uint8Array(fs.readFileSync(job.path));
    const report = job.uri.endsWith('.gltf')
      ? await validator.validateString(new TextDecoder().decode(bytes), options)
      : await validator.validateBytes(bytes, options);
    reports.push({name: job.name, report});
  }
  console.log(JSON.stringify({validatorVersion: validator.version(), reports}));
})().catch(e => {console.error(e); process.exitCode=1;});
'''


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--node-modules', type=pathlib.Path, required=True)
    parser.add_argument('--corpus', type=pathlib.Path, default=pathlib.Path(__file__).parent/'fixtures/c1')
    parser.add_argument('--output', type=pathlib.Path, required=True)
    args = parser.parse_args(); node = shutil.which('node'); assert node
    modules=args.node_modules.resolve(strict=True); package=modules/'gltf-validator/package.json'
    package_json=json.loads(package.read_text());assert package_json['version']=='2.0.0-dev.3.10'
    chosen=['triangle','min_version_2_0','points','lines','terrain','external_bin','interleaved_normalized',
            'matrix_padding','matrix_final_padding_omitted','unknown_chunk','quantized_positions','buffer_base64_octet-stream','buffer_base64_gltf-buffer']
    manifest=json.loads((args.corpus/'manifest.json').read_text()); by_name={c['name']:c for c in manifest['cases']}
    hashes={}; jobs=[]
    with tempfile.TemporaryDirectory(prefix='c1-khronos-') as temporary:
        root=pathlib.Path(temporary)
        for name in chosen:
            source=args.corpus/by_name[name]['path'];digest=hashlib.sha256(source.read_bytes()).hexdigest()
            assert digest==by_name[name]['sha256'];hashes[name]=digest
            directory=root/name;directory.mkdir()
            # Controlled checked fixtures only; never extract user archives here.
            with zipfile.ZipFile(source) as archive:
                for member in archive.namelist():
                    if member not in ['@3dtilesIndex1@','tileset.json']:
                        destination=directory/member;destination.parent.mkdir(parents=True,exist_ok=True)
                        destination.write_bytes(archive.read(member))
            uri='tile.gltf' if name=='external_bin' or name.startswith('buffer_base64_') else 'tile.glb'
            jobs.append(dict(name=name,path=str(directory/uri),uri=uri))
        jobs_path=root/'jobs.json';jobs_path.write_text(json.dumps(jobs));script=root/'validate.cjs';script.write_text(SCRIPT)
        import os
        result=subprocess.run([node,str(script),str(jobs_path)],env=dict(os.environ,NODE_PATH=str(modules)),
                              capture_output=True,text=True,timeout=60)
        assert result.returncode==0,result.stderr
        evidence=json.loads(result.stdout)
    evidence.update(schemaVersion=1,fixtureSha256=hashes,
      tool={'package':package_json['name'],'version':package_json['version'],
            'packageJsonSha256':hashlib.sha256(package.read_bytes()).hexdigest(),
            'implementationSha256':hashlib.sha256((modules/'gltf-validator/gltf_validator.dart.js').read_bytes()).hexdigest()},
      limitations=['Official validator independently checks these core/quantization positive controls.',
                   'Meshopt compressed golden fixture is excluded: this validator does not provide a decoded meshopt geometry oracle.',
                   'Pinned draft KHR_mesh_primitive_restart is excluded: this official validator does not implement the draft.',
                   'b3dm outer framing and 3TZ index are checked by the independent C1 corpus, not this glTF-only tool.'])
    args.output.parent.mkdir(parents=True,exist_ok=True);args.output.write_text(json.dumps(evidence,indent=2)+'\n')
    failures=[r for r in evidence['reports'] if r['report']['issues']['numErrors']]
    assert not failures,json.dumps(failures,indent=2)
    print(json.dumps({'ok':True,'controls':len(chosen),'evidence':str(args.output)}))


if __name__=='__main__':main()
