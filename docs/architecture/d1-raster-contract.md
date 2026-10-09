# D1 raster directory contract

`RasterDirectoryRequest::web_mercator_rgb(input, output, z, x, y)` and the root
`raster_to_directory(request, &RunControl)` produce a new directory. D1 supports
CreateNew only; its output parent must already exist. Publication uses the D1
runtime, never the legacy raster job. Existing destinations conflict.

The finite native profile is a regular local GTiff file of at most 32 MiB,
256 by 256 pixels, three RGB UInt8 bands, declared EPSG:3857, no nodata, masks,
alpha, scaling, offset, or external dataset dependencies. Zoom is at most 24;
x and y are less than 2^z. The north-up affine must match that XYZ tile:
H = pi * 6378137; span = 2H / 2^z; west = -H + x*span;
north = H - y*span; pixel size = span/256. Each affine coefficient is compared
with absolute tolerance 1e-7 metres plus abs(expected)*1e-12.

Output members are `tiles/{z}/{x}/{y}.png`, `tilejson.json`, and `report.json`.
The PNG copies RGB samples exactly, with source row zero at the north edge.
TileJSON declares XYZ, PNG tiles, zoom, and geographic bounds. The versioned
report names `d1-web-mercator-rgb`, source bytes, dimensions, and tile address.
There is no reprojection, resampling, styling, COG, pyramid, or terrain claim.

Source identity and absolute paths are bound before observer calls. Source
metadata must remain coherent across native reading. Before GDAL opens the input, TIFF signature and known sidecars are checked.
GDAL opens with a GTiff-only driver list, internal georeferencing, and an explicit
single-file sibling list. GDAL handles remain
private and close before staging; only owned RGB bytes cross that boundary.
Producer files finish before event closure and directory sealing. Failure and
cleanup diagnostics follow the runtime. A failed native close is an I/O error;
when admission or reading already failed, close failure remains secondary. Default builds return typed Unsupported
and adapters expose native capability explicitly.

An independent fixture writes GeoTIFF tags and analytic RGB values without the
producer; validation decodes the PNG and checks every sample plus XYZ bounds.
Future GDAL-free work may add a concrete owned-byte decoder. D1 does not add an
unused generic backend interface or implement the broader #82 raster scope.

Every native band must declare a block width and height in 1..=256 before sample
reading, rejecting oversized strips/tiles. GDAL-supported GTiff compression and
planar storage are accepted when they decode to the admitted RGB samples.
Only PixelIsArea (the default when AREA_OR_POINT is absent) is admitted.
PixelIsPoint is refused because its point sample semantics are not represented
by this imagery output. Any nonempty COLOR_PROFILE domain is refused, including
ICC profiles, primaries, whitepoints and transfer functions. PNG copies bytes;
it does not make a colorimetric rendering fidelity claim. Descriptive labels,
EXIF/XMP and non-geospatial metadata are ignored and not copied to output.
Georeferencing, RGB interpretations, scale, offset and coverage are admitted
explicitly rather than ignored. See [GDAL GTiff color metadata](https://gdal.org/en/stable/drivers/raster/gtiff.html#color-profile-metadata)
and [GDAL PixelIsPoint interpretation](https://gdal.org/en/stable/development/rfc/rfc33_gtiff_pixelispoint.html).
The 32 MiB file ceiling bounds input size, and owned output pixels are fixed at
196,608 bytes. These are not a hard bound on native GDAL/libtiff allocations:
metadata parsing and codecs allocate internally before or during admission.
The near-ceiling 30 MiB metadata fixture observed about 152 MiB native process
peak memory; resource claims are limited to the measured finite fixtures.

All builds validate nonempty request paths, XYZ range, and cancellation first.
Default builds then return Unsupported without filesystem inspection or observer
calls; capability therefore dominates source and destination errors. Native
builds perform source and destination admission before their first observer call.

Before native opening, a bounded first-IFD tag scan (classic TIFF or BigTIFF,
either byte order, at most 4096 entries) refuses TIFF tags 301, 318, 319, 342 and
34675: transfer functions, whitepoint, primaries, transfer range and ICC profile.
This is structural admission only; GDAL owns decoding. It covers standalone
transfer curves that GDAL exposes in COLOR_PROFILE only when accompanying
primaries and whitepoint are present, as shown by [LoadICCProfile](https://github.com/OSGeo/gdal/blob/master/frmts/gtiff/gtiffdataset_read.cpp).
The native COLOR_PROFILE refusal remains an additional guard.

The first IFD must terminate the image chain; SubIFDs (tag 330) are refused.
TIFF orientation is absent/default or a single SHORT with value 1 (top-left).
Other orientations, image pages and embedded overviews are outside this pilot.
