# Contributing to rusty-tiles

Bug reports, documentation improvements and focused code contributions are welcome. For a large change or new format, open an issue first so we can agree on scope and fidelity requirements.

## Report a problem

Include the command you ran, expected and actual results, rusty-tiles version, operating system, and relevant Rust/GDAL/Python versions. For rendering problems, include the viewer version and whether hardware acceleration is enabled. Share the smallest synthetic or openly licensed reproduction you can make.

Do not upload private survey data, sensitive locations, personal metadata, access tokens, or data you do not have permission to redistribute. Redact file paths and credentials from logs. Private datasets are not required for the default test suite.

## Branches and pull requests

| Branch | Purpose | How changes arrive |
| --- | --- | --- |
| `main` | Stable, releasable code; version tags are cut here | Release PR from `develop`, or a focused hotfix PR |
| `develop` | Integration branch for the next release | Feature, fix, test and documentation PRs |
| `feat/<description>`, `fix/<description>`, `docs/<description>`, `chore/<description>` | Short-lived work | Branch from `develop`; open a PR back to `develop` |
| `hotfix/<description>` | Urgent correction to a released version | Branch from `main`; PR to `main`, then sync `main` back into `develop` |

External contributors should fork the repository; repository write access is not required. For example, after cloning your fork and adding this repository as `upstream`:

```sh
git fetch upstream
git switch -c fix/describe-the-change upstream/develop
# Make and test your changes.
git push -u origin fix/describe-the-change
```

Open a PR targeting `develop`. Describe the problem, resulting behavior, validation, and any fidelity or performance tradeoff. Keep unrelated changes separate. Follow the existing code style; no special commit-message convention or contributor agreement is required.

Maintainers normally squash short-lived contribution PRs. Release promotions from `develop` to `main` use a merge commit so the shared branch history is preserved. After a release or hotfix, merge `main` back into `develop` through a PR before the next promotion. Do not rebase or force-push either shared branch. Delete your short-lived branch after merging; retain `main` and `develop`.

## Build and check

Use current stable Rust with rustfmt, a C++ compiler and pkg-config. libjpeg-turbo is optional for local builds. Python tests require Python 3, NumPy and GDAL/GEOS; CI pins its environment in `.github/workflows/ci.yml`.

```sh
cargo fmt --check
cargo test --locked
python3 -m unittest discover -s tests -p 'test_*.py'
RUSTY_TILES_DISABLE_NATIVE_JPEG=1 cargo test --locked --lib jpeg::tests
```

`cargo clippy --all-targets` is useful during review. Existing style warnings are not a required CI gate; avoid adding new warnings. Optional external-oracle and user-supplied-model tests are ignored by default and must be invoked explicitly. See the README for their dependencies and inputs.

For geometry, texture, coordinate or archive changes, add a regression that checks the meaningful output: triangle membership/winding, texels/materials, independent coordinate references, metadata values, archive indexing, or failure publication behavior. For preview changes, check rendering and picking using invented fixtures. For performance changes, compare the same source, settings, camera and hardware, and report memory and fidelity as well as timing. Do not claim hardware FPS from a software-rendered browser.

## Review and permissions

Both shared branches require a PR, the `Rust` and `Python` CI checks, an up-to-date branch, and resolved review discussions. Direct pushes, force-pushes and branch deletion are blocked, including for administrators under the core ruleset. Workflow tokens have read-only repository access and cannot approve PRs. CI does not receive deployment secrets.

The separate `main` review rule requires one code-owner approval and dismisses stale approvals when code changes. CODEOWNERS requests review from `@BenDyson-Arch`. While the project has one maintainer, repository administrators may bypass **only this approval rule, and only through a PR**, for their own changes. The PR and CI requirements remain enforced. Other contributors cannot merge without repository write permission. Review the exception when adding more maintainers.

Maintainers create versioned releases from verified commits on `main`. A passing contribution does not automatically publish a release or grant repository permissions. Contributions are licensed under the project's MIT license.


## Opt-in public point-cloud validation

The public Autzen source has 10,653,336 classified points. The download helper
records its CC BY 4.0 license and attribution. Its horizontal coordinates are
international feet and its NAVD88 heights are US survey feet; the helper converts
both to local metre XYZ and removes CRS declarations. This validates local point
conversion and metadata fidelity, without claiming an ellipsoidal datum transform.
Use the point-cloud command from its feature PR until it is merged.

```sh
python3 scripts/public_data.py autzen /path/to/cache
rusty-tiles point-cloud -i /path/to/cache/autzen-local-metres.las \
  -o /path/to/cache/autzen-local-metres.3tz --sourceCrs local \
  --maxPoints 50000 --chunkPoints 100000
python3 scripts/audit_point_cloud.py /path/to/cache/autzen-local-metres.las \
  /path/to/cache/autzen-local-metres.3tz
```

On 2026-10-05 the debug-build conversion took 31.9 seconds and 181.8 MiB peak
subprocess RSS, producing 613 tiles (307 leaves). A full leaf audit matched every
numeric LAS field and found every source record exactly once. The maximum
reported local float32 position rounding was 0.0000337 m. These measurements
apply to one machine/run, not a performance guarantee. Downloads and generated
archives stay outside the source tree and are not run in CI. NumPy,
`laspy[lazrs]` and pyproj are required for preparation; the audit uses an
uncompressed LAS file for memory-mapped source access.

Source and attribution: [PDAL Autzen data](https://github.com/PDAL/data/tree/main/autzen),
[CC BY 4.0 license](https://github.com/PDAL/data/blob/main/LICENSE).

## GeoPackage diff compatibility

Replacement belongs to the tiler; diff creation/application stays in external
libraries. The optional test uses upstream C++ geodiff and the local Go port:

```sh
# Run from the go-geodiff checkout to resolve its existing Go module.
go build -o /tmp/go-geodiff-driver /path/to/rusty-tiles/tests/geodiff_driver.go
# Run from rusty-tiles with the actual upstream binary (2.3.0 tested).
GEODIFF_CPP_BIN=/path/to/geodiff GO_GEODIFF_DRIVER=/tmp/go-geodiff-driver \
  python3 -m unittest discover -s tests -p 'test_geodiff_compat.py'
```

The suite generates invented GeoPackage fixtures, checks byte-identical
changesets, cross-applies them, and compares replacement output with fresh
world geometry and scalar properties. It separately exercises GDAL spatial-index
triggers. The Go `ST_IsEmpty` apply gap is reported explicitly as a known skip;
upstream indexed apply and failed Go transaction rollback are still checked.
No upstream source or database fixtures are bundled; CI's core replacement tests
run without external diff binaries.
