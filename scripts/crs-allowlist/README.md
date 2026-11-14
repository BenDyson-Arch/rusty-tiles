# EPSG allow-list generator

`generate.py` builds `src/crs/epsg_allowlist.tsv.gz`, the table of EPSG codes the
standard (non-GDAL) build resolves, and `tests/fixtures/crs_allowlist_reference.json`,
the GDAL ECEF references the unit tests check. See issue #337.

For every active EPSG projected and geographic 2D CRS in the local PROJ database, it:

1. samples a 3×3 longitude/latitude lattice over the CRS's area of use at heights
   0 m and 1,000 m (18 points) and converts them to source coordinates with GDAL;
2. computes GDAL's ECEF (EPSG:4978) result for each sample, with only-best operations,
   no ballpark transformations, PROJ networking off and a grid-free `PROJ_DATA`;
3. turns each grid-free, ranked PROJ operation into a definition with an explicit
   `+towgs84`/`TOWGS84` (a Helmert shift, or zeros for a null transformation), as a PROJ
   string and as WKT1;
4. runs every candidate through the crate's own resolver and proj4rs (the ignored lib
   test `crs::allowlist::generator::engine_results`), and keeps the first one whose 18
   answers all land within 1 mm of GDAL's.

A code is listed only when all 18 GDAL references exist and every sample passes, so the
table never answers where GDAL refuses. The built-in WGS 84 geographic, UTM and Mercator
codes are verified the same way and counted, but not repeated in the table. Each row
records the code, EPSG's stated accuracy of the reproduced datum transformation, its kind
(`conversion`, `null` or `helmert`), the transformation's EPSG id and name, and the
definition. The header records the EPSG dataset, PROJ and GDAL versions.

## Run it

Use a conda-forge environment whose versions match the table header (see
`generate.py versions`):

```sh
micromamba create -n crs-allowlist -c conda-forge python=3.13 gdal=3.13.3 libgdal-core=3.13.3 proj=9.9.0
micromamba run -n crs-allowlist python -I scripts/crs-allowlist/generate.py write
micromamba run -n crs-allowlist python -I scripts/crs-allowlist/generate.py check
```

`write` regenerates the table and the fixture. `check` regenerates both in a scratch
directory and fails if the GDAL/PROJ/EPSG versions differ from the table header, if any
row or fixture value drifted, or if any entry exceeds 1 mm. `--cargo` (or `$CARGO`)
supplies the cargo command, for example a memory-capped wrapper; `--summary FILE` writes
counts and the reason each refused code failed. The GDAL sweep takes about ten minutes on
an eight-core machine, plus a release build of the lib tests.

Changing the PROJ/GDAL pin means regenerating the table, reviewing the diff
(`zcat src/crs/epsg_allowlist.tsv.gz`) and updating the CI job's pins, which it reads
from the table header.
