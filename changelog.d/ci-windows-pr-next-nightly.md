### Changed

- CI: pull requests that run the fast tier also run the Windows `Portable CLI` job (the nextest `portable` profile, doctests and the packaged release CLI), so Windows failures show on the pull request that causes them instead of on the next push or nightly run. Docs-only pull requests still skip it.
- CI: the nightly run also tests `next/0.4.2`, the integration branch of the next patch release, with the full tier. Its failures go to their own "Full CI matrix failing on next/0.4.2" issue, and a pass there does not start the dev build, which only develop does.
