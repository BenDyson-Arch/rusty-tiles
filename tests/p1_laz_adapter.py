#!/usr/bin/env python3
"""LAZ codec adapter for the independent stdlib P1 oracle.

laspy/lazrs author compressed delivery only. The original independently authored
LAS is the required-field/record oracle; laspy never decodes expected values or
output GLB/implicit data. All cases are required; missing codecs fail.
"""
import argparse
import hashlib
import importlib.metadata
import json
import pathlib
import tempfile

import laspy
import p1_point_oracle as oracle


def run(binary,directory):
    cases=[]
    for fmt in oracle.FORMATS:
        source=directory/f'format-{fmt}.las';compressed=source.with_suffix('.laz')
        oracle.fixture(source,fmt=fmt)
        laspy.read(source).write(compressed)
        for explicit in (False,True):
            output=directory/f'format-{fmt}-{explicit}.3tz'
            options=['--sourceCrs','local','--maxPoints','16','--chunkPoints','3','--metadata-attributes']
            if explicit:options.append('--explicit')
            completed,response=oracle.invoke(binary,compressed,output,options)
            oracle.require(completed.returncode==0,'LAZ conversion: '+completed.stdout)
            result=oracle.audit(source,output,16)
            result.update(format=fmt,explicit=explicit,source_las_sha256=hashlib.sha256(source.read_bytes()).hexdigest(),delivery_laz_sha256=hashlib.sha256(compressed.read_bytes()).hexdigest())
            cases.append(result)
    return {'binary':str(binary),'binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'delivery_encoder':{'laspy':laspy.__version__,'lazrs':importlib.metadata.version('lazrs')},'expected_reader':'independent stdlib original LAS enumeration','cases':cases}


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--binary',required=True);parser.add_argument('--json-output',required=True);args=parser.parse_args()
    binary=pathlib.Path(args.binary).resolve()
    with tempfile.TemporaryDirectory(prefix='p1-laz-') as tmp:result=run(binary,pathlib.Path(tmp))
    pathlib.Path(args.json_output).write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result,indent=2))
if __name__=='__main__':main()
