# #146: remove compatibility packing mutations

Audit/implementation baseline: develop `29b96e501b1426e08cf7d3f01ba93964c6618a57`.
This is the packing removal portion of #146/#126. It uses the accepted bounded
[F0 package contract](foundation-contracts.md); it establishes no new semantic
tileset, GLB, implicit conversion, or release acceptance.

## Invariants, domain and failure

`package(PackageRequest, &RunControl)` owns every migrated packaging mutation.
Requests select a directory/exact `tileset.json` file or explicitly named regular
members, plus `OutputPolicy`. Inventory admission, source/output overlap, safe
unique names, reserved generated index names, source checks, byte copying,
receipt, cleanup and completed-file publication retain their F0 meanings. Opaque
payload bytes may contain deliberately invalid tileset or content semantics;
packaging does not certify them. No input is skipped to make a fixture pass.

Invalid inventories fail before publication. CreateNew/Replace, observer failure,
cancellation and cleanup diagnostics use the underlying package operation's
existing bounded contract. Migration must not restore an infallible Reporter,
optional `ConversionResult`, `force` option adapter, or second packaging facade.

The explicit-to-implicit converter remains legacy: it still owns its Job,
Reporter, hierarchy transformation and final publication. Its private candidate
packaging invokes package with an explicit admitted member inventory and a
silent default RunControl; this is not a claim that the outer conversion now
implements F0 cancellation, observer arbitration or publication.

## Ownership and dispositions

Remove `pack::{PackOptions,convert_to_3tz,convert_to_3tz_reported,pack_named_files}`,
the options-to-policy bridge, optional-report adaptation, and root mutation
aliases after migrating their repository callers. CLI and Python already use
package directly; their advertised packaging spellings do not call these Rust
compatibility functions.

The implicit operation calls private `archive3tz::validate_3tz` for its existing
container check, private `pack::tree_members` for its existing staged-tree selection,
and package for candidate packing. These are operation-owned calls, not a new
public implicit or archive facade. Move the legacy CodecError-to-Error adapter
to `error.rs`; this prevents error adaptation from depending on an obsolete
packaging mutation module. The format codec remains independent of Job/runtime.

Retain the current read-only `pack::{list_zip_names,validate_3tz,TZ_INDEX_NAME}`
exports provisionally for existing finite codec/test consumers. They do not
inherit broader acceptance from F0 or `validate::inspect`; removing their public
surface is a separate explicit disposition with caller migration. Production
format owners use the private archive constant rather than `pack` imports. The
demodata benchmark uses the intentionally retained root format constant.
The private staged-tree helper stays owned by the still-legacy converter; it is
not a supported public packaging operation and does not bypass F0 admission.

Move packaging entry-order reproducibility checks into the package owner.
Integration fixture construction uses PackageRequest directly, not a renamed
compatibility wrapper. Do not remove old mesh, wrapping, raster or implicit
operations, output::Job, global report types, vector report writing, or the
vector reuse fingerprint as a side effect of this packing slice.

## Independent acceptance and sensitive controls

The coordinator owns Cargo and records executed evidence against final source
and artifact identities. Before claiming completion:

1. Scan Rust source, fixture builders and exports: removed mutations/options have
   no callers or declarations; CLI/Python packaging still reaches package.
2. Replay admitted directory and named-member packaging fixtures. Independently
   decode ZIP member bytes/index and receipt counts, including the ZIP64 member
   count control and reversed caller entry order.
3. Retain the unsafe `../escape` member control with Replace against sentinel
   output; assert InvalidRequest, unchanged output and no new work artifacts.
   Retain missing-manifest rejection using the package domain failure.
4. Replay implicit conversion positive preservation and corrupted input checks.
   A fixture must reach the intended semantic rejection, rather than fail during
   package admission. Source/member naming errors that F0 rightfully rejects
   require test-owned raw ZIP construction if the control needs an archive
   outside the admitted package profile; never weaken production admission.
5. Replay native implicit corruption (subtree header, availability, metadata,
   missing content, boundary link and root bounds) and validator corruption
   fixtures. Payload-semantic corruption remains admissible opaque packaging.
6. Review the final source independently: accepted package behavior is retained;
   the still-legacy outer implicit Job/report lifecycle remains unaccepted.

Static observations and migrated source are not executed acceptance. The final
ledger must distinguish F0 evidence already established from checks rerun for
this API deletion and unresolved operation/publication gates.
