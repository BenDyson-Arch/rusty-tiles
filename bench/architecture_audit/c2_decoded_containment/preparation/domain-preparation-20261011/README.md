# C2 domain preparation

Frozen inputs for a separate whole precode assessment; no implementation or
target execution. The source snapshots are observations from the R2 checkout
while PR155 CI/wheels run, not a new accepted C2 baseline. R2's accepted develop
merge and a combined domain/math/resource review precede implementation.

Read contract-draft.md, decisions.json and source-inventory.json together. Primary
identities are in primary-pins.json; source identities in source-pins.json.
OGC web observation is selected text only, not a complete published-document
hash. Retained selected glTF source SHA10aebb... and Tiles source SHAb841fade...
match earlier preparation identities. No source/proof original was changed.

witnesses.json contains13 literal cases plus17 context/control recipes. Exact
truth is authored independently of repository helpers. W13 uses explicit exact
represented-binary64 rational constants; approximate quaternion admission still
requires combined review. Materialize only after root selects useful inputs:

    nice -n 10 taskset -c 0,1 python3 materialize.py --output NEW_EMPTY_DIRECTORY

The command is unexecuted. It creates glTF/BIN/GLB and one b3dm, preserving literal
expected/context records, and pins outputs. It performs no transform math and
does not create target reports or archives. Context controls need separately
frozen materialization: they are not claimed to reach production today.

Acceptance must independently compare actual decoded/instance facts to these
literal expectations or their certified mathematical enclosures. Public report
checks alone cannot observe intermediate point truth; a narrowly consumed test
seam or independent external decoder may be needed in the final proof. Mutating
expected output is never a substitute for a changed-target sensitive control.

Only AST/JSON syntax preparation checks may run in this author lane. Those checks
do not establish semantic witness truth, production behavior or allocation cost.
No Cargo/rustc/native/CLI, Git changes, repository edits or issue writes occurred.
