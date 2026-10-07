"""Generate invented styling/picking cases; no download or user data required."""
import json
import pathlib
import sys
import types
from vector_compat import generate, pos

sys.path.insert(0,str(pathlib.Path(__file__).resolve().parents[1]))
from vector_test_support import vector


def generate_metadata(root):
    generate(root)
    for case in json.loads((root/'cases.json').read_text()):
        name=case['name'];source=root/(name+'.geojson');value=json.loads(source.read_text())
        value['features'][0]['properties'].update(category='survey',status='active',
            optional_text='present',optional_number=4.5,optional_integer=2**60+3)
        value['features'].append(dict(type='Feature',id='missing',properties=dict(name=None,
            category='other',status='inactive',optional_text=None,optional_number=None,optional_integer=None),
            geometry=dict(type='Point',coordinates=pos(-25,0))))
        source.write_text(json.dumps(value))
        vector.run(types.SimpleNamespace(input=str(source),output=str(root/name),max_features=64,
            max_vertices=8 if name=='fragmented' else 65536,max_bytes=16384,lod_tolerance=.2,lod_levels=3,
            repair=name=='outline',ambiguous_outlines=name=='outline'))
    # Rewrite cases with the missing-value sample while retaining the preview bootstrap.
    cases=json.loads((root/'cases.json').read_text())
    for case in cases:case['missing']=pos(-25,0)
    (root/'cases.json').write_text(json.dumps(cases))
    manifest=json.loads((root/'line/tileset.json').read_text())
    def prefix(node):
        for content in node.get('contents',[node['content']] if 'content'in node else []):
            content['uri']='line/'+content['uri']
        for child in node.get('children',[]):prefix(child)
    prefix(manifest['root'])
    (root/'tileset.json').write_text(json.dumps(manifest))


if __name__=='__main__':generate_metadata(pathlib.Path(sys.argv[1]))
