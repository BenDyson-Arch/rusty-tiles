### Security

- Native `vector`: a source file can no longer select GDAL drivers that open
  other datasets, query services, download schemas, run algorithms or start
  processes named inside it (`OGR_VRT`, `GDALG`, `GPSBabel`, `WFS`, `OAPIF`, `CSW`,
  `GMLAS`). A `.vrt` could otherwise pull any readable local file, or
  a URL where GDAL has network drivers, into the published tiles. Every other
  registered vector driver is still accepted.
