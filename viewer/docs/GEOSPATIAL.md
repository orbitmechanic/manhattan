# Geospatial stack (iteration 30 foundation)

Installed user-level (Python 3.14.8, system site-packages not writable):

| package | version |
|---|---|
| shapely | 2.2.0 |
| pyproj | 3.8.0 |
| rasterio | 1.5.2 (GDAL 3.x bundled) |
| geopandas | 1.2.0 |
| pyogrio | 0.13.0 |
| pandas | 3.1.1 |
| affine | 3.0.1 |

Verified: EPSG:2926 (1270000, 220000) → WGS84
(-122.334638, 47.593221); Shapely box/polygon clip area 1.0; rasterio
GeoTIFF write/read shape (4,4). No system GDAL CLI; rasterio/pyogrio bundle
GDAL, which covers DEM decode/window and GeoJSON/FeatureServer I/O.
