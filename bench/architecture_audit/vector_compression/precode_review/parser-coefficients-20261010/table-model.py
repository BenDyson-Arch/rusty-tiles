import json
from fractions import Fraction
b=0
worst=(Fraction(0),0,0)
transitions=[]
for m in range(1,65537):
    cap=(b-1 if b<8 else (b//8)*7) if b else 0
    if m>cap:
        new=(4 if m<4 else 8 if m<8 else 16) if m<15 else 1 << (((m*8//7)-1).bit_length())
        peak=(25*b+16 if b else 0)+(25*new+16)
        transitions.append([m,b,new,peak])
        b=new
    else:
        peak=25*b+16
    ratio=Fraction(peak,m)
    if ratio>worst[0]:worst=(ratio,m,peak)
    assert peak<=116*m,(m,b,peak)
print(json.dumps({'evidenceClass':'independent source-formula integer model, not allocated-storage execution','mRange':[1,65536],'boundBytesPerKey':116,'worstRatio':str(worst[0]),'worstAtKeys':worst[1],'worstBytes':worst[2],'growthTransitions':transitions},indent=2))
