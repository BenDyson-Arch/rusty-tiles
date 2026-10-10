# Separate source-package supplement, 2026-10-10

**Bounded source-package acceptance: yes** for the actual extracted root crate
on source `2ef8db1a329a675f578ced11fa1c19e02f8b7ba6`, tree
`c8633703eda9a1bd258df8c1bb748fa65a054e4d`. This closes the local source archive
compilation gate left open by the final-checkpoint review. Exact-head remote CI,
five wheel builds, ten installed-wheel lanes and four official Blender/platform
lanes remain merge gates. This accepts no publication, release, full A2, broader
geometry/metadata semantics or #113/#121 closure.

The reviewer remains `/root/payload_final_source_review`, separate from contract,
prerequisite, implementation and independent-control authors. This supplement
uses read-only Git/object inspection, tar/TOML/lock decoding, source/hash
comparison and retained execution verification. The reviewer runs no Cargo,
target, installation or heavy workload and changes only this new review folder.
The earlier immutable final-checkpoint review and executions retain source
`2ebfb749e9d71fdb72507fd1203472b226f23156` and their original artifact identities.

The 2ef commit changes 66 paths: added evidence/review files and the implementation
document. All **97 production and 179 selected acceptance input hashes** match
the original source pin, SHA256
`da9cfaeb8c576b492f66ec6c396c68a80c7a5a8851d88dee9db437be2ec7b12e`,
both in committed 2ef bytes and the current worktree. The earlier review receipt
and all its 15 bound files are unchanged. This establishes byte equivalence for
the selected implementation and controls; it does not rename earlier test runs
as 2ef executions.

The coordinator's actual `cargo package --locked -p rusty-tiles`, with default
verification enabled, finishes with exit zero in 78.38 seconds. The retained
log explicitly compiles `rusty-tiles v0.4.0` from
`/tmp/rusty-tiles-f1d2-final-target/package/rusty-tiles-0.4.0` and finishes its
dev profile. Its SHA256 is
`71da8c51baf412ee668b1d3bb844c7a52ff8c7fdfac6cde4502ba9dd91948a0a`.
This is actual extracted-source compilation, extending the earlier inventory
check. It is not another library/public control execution or an installed
artifact check; the package operation uses default features rather than a new
native-feature source-package build.

The final promoted, separately frozen archive has **5,527,107 bytes** and SHA256
`97c9d8c1635c3f5db5874fdae7624c2d6773318a9c9327e0bb3b4fa276539d69`.
Its `.cargo_vcs_info.json` names clean 2ef exactly. All **1,878 regular, uniquely
named members** match the retained inventory. The reviewer compares all **1,874
ordinary members** directly with immutable Git blobs from 2ef, rather than
trusting the current worktree. All **92 src files** and **271 selected root
source/control members** are byte-identical to the pinned inputs. The separate
Python package and CI workflow are correctly absent from the root-crate archive;
the removed validator-owned JSON/payload files are absent as well.

`Cargo.toml.orig` is byte-identical to committed Cargo.toml. The reviewer
reconstructs the normalized manifest and compares its complete parsed table:
workspace version resolves to 0.4.0, string dependency declarations become
version tables, automatic target discovery becomes explicit (including all 38
root Rust tests), and workspace membership is removed. Dependency versions,
features, targets, package metadata and build behavior are unchanged.

The packaged lock retains **288 package tables**, each exactly identical to its
original version/source/checksum/dependency table. No new or changed package
table appears. Exactly eight Python-only tables are pruned: portable-atomic,
pyo3, pyo3-build-config, pyo3-ffi, pyo3-macros, pyo3-macros-backend,
rusty-tiles-python and target-lexicon. The reviewer walks the original lock graph
from the root crate independently; its reachable set is exactly the packaged
288-table set. Lock normalization therefore introduces no dependency drift.

The separate lossless compilation index, SHA256
`2e65ea47fbe4fd031e9c4f899784d3c3d38c928185a8acf8c0d036237f2b8d8a`,
binds four records: actual log, coordinator receipt, execution runner and full
archive inventory. Compressed and expanded lengths/hashes match original bytes,
and every archive-inventory entry matches the actual frozen archive. The archive
itself remains external and is identified by path/hash in the receipts. The old
archive visible while Cargo was still verifying is excluded: only the promoted
post-success bytes support this decision.

No source or packaging defect was identified. A later docs/evidence commit and
remote/platform execution require their own exact-head provenance; this source
archive compilation remains a 2ef execution. The earlier bounded local decision
remains in force, with the local source-package compilation gate now closed and
the remote/wheel/official Blender merge gates still pending.
