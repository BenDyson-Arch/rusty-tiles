# Independent findings before candidate freeze

This separate nonauthor lane owns only this evidence directory. The candidate
source was live during these observations. These findings do not accept any
candidate; final source and artifact binding remain required. No Git, Cargo,
build, installation, producer, browser or contacts were used here. Actual CLI
controls below are tiny, serial and run at nice 10.

1. `ZipArchive::len()` in zip 2.4.2 is a deduplicated name-map count. The initial
   candidate admitted physical records but did not refuse duplicate raw names.
   A disjoint three-record archive with two `tileset.json` records and a
   24-byte index covering only the later record succeeds on frozen production.
   Its index should have 48 bytes under the physical cardinality requirement.
   [Actual control](duplicate-name-before.json); [independent driver](duplicate_name_probe.py).
   The root coordinator sent this finding to the implementation owner.

2. zip 2.4.2 interprets central Unicode Path `0x7075` as a replacement name. Raw
   logical names can therefore be unique while downstream names collapse. It
   also consumes three ZIP64 u64 values when the tag payload is at least 24
   bytes, even when the central sizes/offset have no sentinels. An independently
   authored non-sentinel offset override makes the downstream reader inspect
   a local child record nested inside another stored member despite disjoint
   raw central extents. Both gaps execute on frozen production with sensitive
   ordinary controls. [Actual controls](identity-override-before.json);
   [independent driver](identity_override_probe.py). The coordinator selected
   finite Unsupported outcomes for unproved name and ZIP64 overrides rather
   than allowing a second interpretation after admission.

3. zip 2.4.2 `get_metadata` discards errors from `read_central_header` and retries
   earlier EOCD candidates. These errors include actual source I/O failures and
   parsing failures from otherwise well-framed tags. An outer archive with one
   opaque nested-ZIP member is refused as a one-entry 3TZ control. Adding a
   fully framed empty NTFS tag to the outer central entry makes frozen
   production succeed using the earlier nested ZIP directory. This establishes
   selected-directory drift, not an executed unbounded allocation. The initial
   candidate's framing/profile rules also allowed that tag before asking the
   library to allocate its directory. [Actual controls](fallback-before.json);
   [independent driver](fallback_probe.py). Exact selected-directory allocation
   and actual directory-read I/O behavior remain stop gates until corrected and
   exercised on frozen candidate source.

Frozen controls use binary SHA256
`aa71fd57614844488d336dbc991df551fd9294c6d31d82027e1b4a43aedbf64b`;
receipts pin each driver, archive and actual result. The fallback driver composes
only this review lane's earlier builders and has no product/other-author fixture
imports. None of these controls establishes full ZIP conformance, every payload's
semantics, a whole-operation resource bound, full A2 acceptance or release scope.
