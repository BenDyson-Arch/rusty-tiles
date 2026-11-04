### Added

- Point-cloud 0.4 acceptance checks pair CLI and Python decoded LAS/LAZ fields,
  placement, settings and failure preservation on the current implicit forest.

### Fixed

- Point-cloud CLI height-decision errors now report `invalid_request`, matching
  Python's `InvalidRequestError`, rather than generic data errors.
