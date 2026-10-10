# Independent polygon-control adjudication, 2026-10-10

The original positive oracle is invalid. The production rejection is justified
at this gate. The independent controls author attached polygon metadata to the
one-vertex POINTS base without supplying triangle indices; that was an oracle
authoring error, not a demonstrated implementation defect.

The supplied portable execution remains **204 pass / 1 fail out of205**. Its
artifact SHA256 is
`5480b1147d177425da87cfa115c0ef78e1d126b28ab4da0dc78259bd73304885`.
The exact failed archive SHA256 is
`82d920a5646c3414424bc65930f7638b8660cc6c119e62a934b1220212a7fbab`.
The target returned InvalidInput, exit3, with diagnostic
`polygon extension requires indexed TRIANGLES`.
[Original full receipt](original-failed-205.json.gz) and
[stdout](original-failure.stdout) retain the actual failure. This record does
not retroactively turn that execution green or change any original205
fixture, expectation, generator, private module, manifest or bundle.

The exact selected draft is CesiumGS/glTF
`c1a035499b70aeb5d8281470101423e5e285dfe3`.
[Freshly retained primary](references/EXT_mesh_polygon.md) is7,129 bytes,
SHA256 `777c13228de2de5838a8f0eed5c4f653674a5278c30306124bb8ccbe2fe1016d`.
[Retrieval receipt](references/retrieval-receipt.json) preserves its URL/revision.
Lines26–27 require TRIANGLES mode4 and an explicit indices property. The
original literal has mode0 and no indices. Its loop values0,0,0 also violate
the uniqueness requirement at line80. The primary therefore establishes that
the fixture could not justify an admitted expectation, independent of target
agreement. The original reference-OOB negative shared the malformed background,
so its successful InvalidInput category did not isolate reference checking.

The precode contract retains the existing bounded optional polygon-reference
profile and declines full topology certification. The retained vector draft
audit already identifies this exact polygon pin and TRIANGLES requirements.
Current production `content_integrity::payload` lines846–848 enforce the same
mode/indices gate before polygon accessor references. Static inspection and
the executed diagnostic agree here; no owning source correction is required.

The additive corrected205 lane preserves203 original archives and expectation
records byte-for-byte. It replaces only the invalid polygon positive and its
masked reference-OOB negative. The new positive independently packs a finite
CCW triangle at (0,0,0),(1,0,0),(0,1,0), ordinary U16 indices0,1,2, one polygon,
offset0, and unique exterior loop0,1,2. Its scalar-component count is17;
the public payload report must show5 accessors,1 primitive and3 vertices.
The paired reference negative changes only indicesOffsets to999.

Five additional controls preserve the original invalid source as a negative
and isolate modePOINTS, missing indices, wrong scalar reference, and valid
integral polygon notation. These are independently declared new inputs and
expectations, with no new execution claim. The unchanged original runner
consumes the new dated manifest/bundle; original referencePins and driverSha
remain identical, while correctionPrimaryPins/generator identities are
additive. No failure override is present. Root owns final artifact executions
and source/artifact binding. These controls establish no full polygon topology,
universal C1/glTF, A2 or release claim.
