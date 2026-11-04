# VS-B budget amendment of the C1 payload-control expectations (2026-10-14)

[VS-B-2](../../../../../docs/architecture/validator-traversal-scale-amendment.md): default
validation reports each amplification budget as `max(old default, κ·S)`, with `S` the
archive's stored member bytes. Every frozen admitted expectation of the C1 lanes run by CI was
checked; only the KTX2 lane's `shared-view-heavy-kvd` (`S` = 8,768,585) exceeds a crossover
(visits and references at S/8 = 1,096,073; elements at 16·S = 140,297,360; containment units
at 128·S = 1,122,378,880). `amend.py` recomputed these from the regenerated archive's ZIP
directory, independent of the validator, asserted the old floors and wrote them into
`ktx2_basisu_amendment/2026-10-13/fixtures/manifest.json`. The lane's `integrity.json` and the
CI harness pin (`tests/payload_meshopt_stride_oracle.py`) carry the new manifest hash. Nothing
else in any report or case changed. `liveDocumentBytes` (VS-B-7) is not serialized until VS-C,
so no pinned `limits` object gains a key here.
amend.py sha256 `c83352df7823723227bcf059bc25ea5155cf1163b62ef69d0f52d049f77f7fd3`; the KTX2 runner.py applies it to regenerated cases (runner `8bf5cf00…`, manifest `4e5f995b…`).
