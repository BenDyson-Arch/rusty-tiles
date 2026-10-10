#!/usr/bin/env python3
"""Root-only source/record ledger sensitivity model; never a production budget test."""
import hashlib,json
from pathlib import Path
HERE=Path(__file__).resolve().parent
sha=lambda raw:hashlib.sha256(raw).hexdigest()
def main():
    pins=json.loads((HERE/'integrity.json').read_text())
    for name,pin in pins['files'].items():
        raw=(HERE/name).read_bytes();assert len(raw)==pin['bytes'] and sha(raw)==pin['sha256'],name
    for name,pin in pins['externalInputs'].items():
        raw=Path(name).read_bytes();assert len(raw)==pin['bytes'] and sha(raw)==pin['sha256'],name
    facts=json.loads((HERE/'source-ledger.json').read_text())
    actual=[json.loads(line)for line in Path(facts['actualRawStdout']).read_text().splitlines()]
    stages={r['stage']:r for r in actual if 'stage'in r}
    plan=stages['scaffold_owned_key_pairs_and_fixed_edit'];parser=stages['actual_pinned_json_admission']
    assert plan['peakRequestedOldPlusNew']-plan['baselineRequested']==196987
    assert plan['liveRequested']-plan['baselineRequested']==66676
    assert parser['peakRequestedOldPlusNew']-parser['baselineRequested']==197041
    fixture=json.loads(Path(facts['fixtureInput']).read_text())
    assert [len(k.encode())for k in fixture]==[5,7,11,14,18,65537,3]
    assert sum(len(k.encode())for obj in [fixture,*fixture['buffers'],*fixture['bufferViews']]for k in obj)==65660
    evaluated=[]
    for case in json.loads((HERE/'expected-controls.json').read_text())['cases']:
        # Sum the frozen disjoint ledger inputs, rather than use a candidate's reported estimate.
        branches={name:sum(values.values())for name,values in case['branches'].items()}
        required=max(branches.values())+case['outsidePhaseBytes']
        assert required==case['expectedRequiredBytes'],case['name']
        observed='admitted' if required<=case['configuredLimit'] else 'resource_limit'
        assert observed==case['expectedCategory'],case['name']
        evaluated.append(dict(name=case['name'],requiredBytes=required,configuredLimit=case['configuredLimit'],category=observed))
    faulty=[]
    for case in json.loads((HERE/'expected-controls.json').read_text())['faultyLedgers']:
        claimed=sum(case['terms'].values())
        assert claimed==case['predictedWrongBytes'] and claimed!=case['independentCorrectBytes'],case['name']
        faulty.append(dict(name=case['name'],claimedBytes=claimed,expectedBytes=case['independentCorrectBytes'],status='rejected_by_independent_source_ledger'))
    print(json.dumps(dict(status='MODEL_ONLY_SOURCE_LEDGER_CONTROLS_PASS_NOT_PRODUCTION_ACCEPTANCE',productionProof=False,wholeOperationRssProof=False,cases=evaluated,faultyLedgers=faulty),indent=2))
if __name__=='__main__':main()
