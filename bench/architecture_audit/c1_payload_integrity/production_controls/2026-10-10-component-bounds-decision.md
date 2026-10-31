# Static selected-primary component-bounds decision, 2026-10-10

Retain finite component interpretation of declared accessor bounds when applying
the viewless nonsparse exemption. For FLOAT, finite f64 followed by finite f32
interpretation is supported. Ordinary decimal rounding remains allowed; there
is no exact binary32 lexical representability requirement. The exemption removes
decoded-extrema equality and supplies no cross-component `min <= max` condition.
This is static source adjudication, not a production execution or broader
conformance acceptance.

The selected published glTF 2.0.1 source is revision
`8e798b02d254cea97659a333cfcb20875b62bdd4`, 148,064 bytes, SHA256
`10aebb6a8362a155b196448912bf6250bf046f150431fa73c6cd657c44185aff`.
Its exact retained `pinned-Specification.adoc` member is in the immutable
[primary raw bundle](../primary_controls/raw-evidence.tar.gz). The byte-identical
cache was read at `/home/bend/.cache/rusty-tiles-f1b3-gltf-spec.adoc`.

Accessor Data Types lines 1022–1037 identify FLOAT as 32 bits, require IEEE-754
single precision, and forbid NaN and either infinity. Accessors Bounds lines
1207–1215 require the appropriate array shape, permit arbitrary values without
sparse/bufferView, and describe floating bounds as single precision floats that
SHOULD be rounded before usage. Consequently, `0.1` remains valid when rounded
to binary32; `1e39` is finite in f64 but would become positive infinity under the
FLOAT component interpretation. The arbitrary-values clause does not supply a
different component datatype.

The exact selected accessor schema is 6,517 bytes, SHA256
`86f9aacb0b616e1f6a1d5b8ff81e92294602e9e3c98c7f13d2cefdf987c87e35`;
its `pinned-accessor.schema.json` member is retained in the same bundle.
Both `properties.min.gltf_detailedDescription` and
`properties.max.gltf_detailedDescription` require elements to be treated as the
same datatype as `componentType`. The structural schema has number items and
shape limits, with no min/max cross-array constraint or explicit numeric range
keywords on the bound items.

For integer components, the descriptive same-datatype rule supports preserving
applicable component meaning. The schema's bare `items: number` is not an
independent authority for a newly invented exact unsigned-lexeme bound rule,
and this decision introduces no such change. The production correction must
retain the bounded contract's finite/shape/component interpretation without
inventing additional conditions from the exemption.

Independent controls prepared in [driver.py](driver.py) pair finite-f64/FLOAT-
infinity refusal with stored/viewless ordinary decimal-rounding positives.
The stored positive independently uses Python `struct.pack('<f', 0.1)` and
`struct.unpack('<f', ...)`, separating binary32 rounding from the authored JSON
lexeme. Their fixture preparation has no target execution claim.
