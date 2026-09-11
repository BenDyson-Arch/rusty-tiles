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
