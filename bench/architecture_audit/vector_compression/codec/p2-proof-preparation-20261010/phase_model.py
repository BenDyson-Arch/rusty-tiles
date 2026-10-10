#!/usr/bin/env python3
"""Independent model-only ownership/accounting sensitivity; never import or run targets."""
import json
from pathlib import Path
HERE=Path(__file__).resolve().parent
def peak(events,omit=(),drop_old_before_realloc=False,double_transfer=False):
    live={}; maximum=0
    def cost(): return sum(n for category,n in live.values() if category not in omit)
    for e in events:
        kind=e['op']; name=e['id']
        if kind=='alloc':
            assert name not in live; maximum=max(maximum,cost()+(0 if e['category'] in omit else e['bytes'])); live[name]=(e['category'],e['bytes'])
        elif kind=='free': assert name in live; del live[name]
        elif kind=='realloc':
            category,old=live[name];extra=0 if category in omit else e['bytes']
            maximum=max(maximum,cost()-(0 if category in omit or not drop_old_before_realloc else old)+extra);live[name]=(category,e['bytes'])
        elif kind=='transfer':
            assert name in live and 'bytes' not in e, 'transfer changes owner, never allocation'
            if double_transfer: live[name+'-fake-copy']=live[name]
        else: raise AssertionError(kind)
        maximum=max(maximum,cost())
    return maximum

def run():
    inputs=json.loads((HERE/'fixtures/ledger-cases.json').read_text()); records=[]
    for case in inputs['cases']:
        observed=peak(case['events']);assert observed==case['expectedPeak'],case['name']
        # Inclusive policy decision is independently frozen: exact admits, exact-1 refuses.
        assert observed<=case['exactCap'] and observed>case['oneBelowCap']
        controls=[]
        for category in case['mustDetectOmissions']:
            wrong=peak(case['events'],omit=[category]);assert wrong<observed
            assert wrong<=case['oneBelowCap'], 'faulty model wrongly admits the one-below witness'
            controls.append(dict(fault='omit_'+category,wrongPeak=wrong,detected='underestimate_and_false_admit'))
        if case.get('reallocControl'):
            wrong=peak(case['events'],drop_old_before_realloc=True);assert wrong<observed
            controls.append(dict(fault='drop_old_before_realloc',wrongPeak=wrong,detected='underestimate'))
        if case.get('transferControl'):
            wrong=peak(case['events'],double_transfer=True);assert wrong>observed
            controls.append(dict(fault='double_count_transferred_bridge',wrongPeak=wrong,detected='overestimate_and_false_refusal_at_exact_cap'))
        records.append(dict(name=case['name'],modelPeak=observed,controls=controls))
    limits=json.loads((HERE/'fixtures/default-boundaries.json').read_text())
    for row in limits['cases']:
        assert (row['value']<=row['testedCap'])==(row['expected']=='model_admitted')
    result=dict(evidenceClass='EXECUTED_INTEGER_MODEL_ONLY; no serde/FFI/allocator/codec/default acceptance',cases=records,defaultBoundaryDescriptors=len(limits['cases']),allFaultsSensitive=True)
    print(json.dumps(result,indent=2))
if __name__=='__main__':run()
