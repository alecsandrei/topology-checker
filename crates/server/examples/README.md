# Bundled examples

Five Natural Earth examples cover all seven GeoJSON geometry types. All assets
and catalog metadata are embedded in the executable, with no runtime downloads.
`catalog.json` records feature counts, rule presets, source URLs, the pinned
revision, SHA-256 hashes, download dates, license information and grouping methods.
See [DATA-LICENSE.md](DATA-LICENSE.md) for the public-domain data notice.

| Example | Features and geometry | Preset |
| :--- | :--- | :--- |
| World cities | 243 Points and country-grouped MultiPoints | Duplicate points on originals; multipart groups |
| Major rivers | 13 LineStrings at 110m and 462 MultiLineString records at 50m (461 imported) | Dangles and self-intersections, separately at each scale |
| Countries and islands | 148 Polygons and 29 MultiPolygons | Overlaps and multipart features |
| Lakes and reservoirs | 24 Polygons | Overlaps |
| Country and city atlas | 177 GeometryCollections | Multipart features |

Regenerate using Node (network access required):

```sh
node scripts/prepare-examples.mjs
```

Verify committed snapshots, generated data, counts, metadata, membership,
attributes and coordinate equality without network access:

```sh
node scripts/prepare-examples.mjs --check
```

The script rejects source checksum mismatches before writing each snapshot.
Grouping is deterministic: city groups use sorted `ADM0_A3` codes and retain
source member order. Atlas collections retain country source order, original
country geometry and matching city geometry. Country-code matching does not
imply spatial containment; cities with unmatched country codes are still
retained in the World cities example. No grouping clips, simplifies, merges or
moves coordinates. Each derived feature has an integer ID starting at 1.
Each member includes its source dataset, source ordinal, original attributes
and source identifier (`id`, otherwise `NE_ID`, otherwise zero-based ordinal).
The original five downloads retain every attribute and their exact file bytes.
Download dates are retained on regeneration for unchanged pinned snapshots.

Examples default to EPSG:4326, tolerance `1e-8` in angular units, and the offline
coordinate grid. Grouped cities and 110m rivers start hidden to avoid duplicate
rendering, but their checks remain enabled. Loading does not run validation. The 50m river source contains one empty Loire
MultiLineString, retained in the snapshot but skipped with a warning by the
existing importer. Catalog metadata distinguishes source and imported counts.
River endpoints, multipart countries and atlas collections are inspection
findings whose suitability depends on the rule and intended use.

`GET /api/examples` returns the catalog. `POST /api/examples/{id}` creates a
fresh workspace, returning 404 for unknown IDs. `POST /api/example` retains the
separate synthetic topology tutorial. Dataset provenance is saved in version 1
project source metadata and exposed in workspace dataset summaries. Projects
created before this addition remain readable without provenance.
