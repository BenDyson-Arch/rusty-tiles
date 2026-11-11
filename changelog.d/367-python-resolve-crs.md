### Added

- Python: `resolve_source_crs(definition)` resolves one source CRS exactly as the
  converters do, without converting, and returns `accepted`, `kind`, `definition`,
  `crsOperation` (the reports' shape), `summary` and `refusal`; an unsupported definition
  is returned, not raised. `supported_epsg_codes()` lists every EPSG code the package
  resolves, and `crs_table_info()` gives the EPSG table's EPSG, PROJ and GDAL versions and
  counts. The Rust library exports the same as `resolve_source_crs`,
  `supported_epsg_codes` and `crs_table_info`.
