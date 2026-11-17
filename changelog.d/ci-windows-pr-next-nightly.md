### Changed

- CI: pull requests that run the fast tier also run the Windows `Portable CLI` job (the nextest `portable` profile, doctests and the packaged release CLI), so Windows failures show on the pull request that causes them instead of on the next push or nightly run. Docs-only pull requests still skip it.
- CI: the nightly run can also test a release integration branch with the full tier, listed in `nightly.yml`'s `branch` matrix. Its failures go to that branch's own "Full CI matrix failing on <branch>" issue, and a pass there does not start the dev build, which only develop does.
