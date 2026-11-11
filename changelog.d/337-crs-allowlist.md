### Added

- The standard build (CLI and Python wheel) resolves about 3,100 more EPSG codes from a
  GDAL-verified allow-list, including GDA2020/GDA94 MGA, GDA94/GDA2020 geographic, GDA94
  Lambert/Albers, NZTM2000, ETRS89 UTM and TM35FIN, for mesh, point-cloud and vector
  conversion. A code is listed only when its generated Helmert or null-shift definition
  lands within 1 mm of GDAL/PROJ's best operation at all 18 sampled points of its area of
  use (EPSG v13.102, PROJ 9.9.0, GDAL 3.13.3). Grid-based codes such as British National
  Grid (OSTN15), NAD27 and the original NAD83, coordinate epochs and dynamic reference
  frames (ITRF, WGS 84 (Gxxxx), WGS 72) stay refused; native builds keep using strict PROJ.
- `conversion.json` and the CLI summary name the EPSG datum operation an allow-listed code
  uses and EPSG's stated accuracy; a null transformation such as GDA2020 to WGS 84 is
  reported with its accuracy (3 m), not as exact. `doctor` reports the allow-list's size
  and versions, and `doctor --sourceCrs <definition>` resolves one CRS without converting.

### Changed

- The CRS refusal message reads `pure-Rust CRS transform unavailable` for every converter,
  instead of naming point clouds.
