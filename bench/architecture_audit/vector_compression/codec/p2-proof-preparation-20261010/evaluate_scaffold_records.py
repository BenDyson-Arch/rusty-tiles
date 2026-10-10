#!/usr/bin/env python3
"""Compare future raw-probe allocation records with DRAFT terms; findings retain HELD status."""
from pathlib import Path
import argparse,json
HERE=Path(__file__).resolve().parent
if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('--run-dir',type=Path,required=True);a=p.parse_args();manifest=json.loads((HERE/'fixtures/serializer/manifest.json').read_text());rows=[];gaps=[]
    for case in manifest['cases']:
        records=[json.loads(line)for line in (a.run_dir/('serializer-'+case['name']+'.stdout')).read_text().splitlines()]
        stages={r['stage']:r for r in records if 'stage'in r};facts=next(r for r in records if 'exactJsonBytes'in r)
        assert facts['exactJsonBytes']==case['expectedJsonBytes'] and facts['exactOracleMatch'] and facts['deltaUpper']==case['deltaBound']
        parser=stages['actual_pinned_json_admission'];plan=stages['scaffold_owned_key_pairs_and_fixed_edit']
        parser_extra=parser['peakRequestedOldPlusNew']-parser['baselineRequested'];plan_extra=plan['peakRequestedOldPlusNew']-plan['baselineRequested']
        ej=8*case['inputBytes']+256*case['nodes']+65536;ep=2*case['inputBytes']+128*case['nodes']+256*case['views']
        row=dict(name=case['name'],scaffoldParserIncrementalPeak=parser_extra,draftEjson=ej,scaffoldPairStageIncrementalPeak=plan_extra,draftEplanA0=ep,parserTermGap=parser_extra>ej,planTermGap=plan_extra>ep,scope='Scaffold stage comparison only; all live baselines/oracle owners/actual production types and whole branch envelope require separate reconciliation')
        if row['parserTermGap']or row['planTermGap']:gaps.append(row)
        rows.append(row)
    print(json.dumps(dict(status='HELD_CALIBRATION_FINDINGS' if gaps else 'HELD_SCAFFOLD_TERMS_NOT_FALSIFIED_BY_THESE_INPUTS',productionProof=False,wholeOperationMemoryProof=False,findings=gaps,cases=rows),indent=2))
    raise SystemExit(2 if gaps else 0)
