# GIS validation benchmarks

This suite compares topology-checker, QGIS 4.2, GRASS GIS, PostGIS SQL workflows
and gvSIG on six checks: points inside polygons, polygons containing a point,
duplicate points, line dangles, polygon overlaps and polygon gaps. Every tool
uses identical inputs at one common feature count. Full-suite mode retains the
broader rule catalog and reports unsupported checks and semantic differences.

The official QGIS 4.2 Ubuntu image is pinned by digest. gvSIG 2.7.0 build 5016 is
pinned by archive SHA-256 and runs with OpenJDK. The report records exact versions
and resolved package versions.

## Run

```sh
docker build --platform linux/amd64 -f benchmarks/gis/Dockerfile -t topology-gis-benchmark .
mkdir -p builds/gis-run
docker run --rm --platform linux/amd64 --cpuset-cpus=0 \
  -e GIS_SOURCE_REVISION="$(git rev-parse HEAD)" \
  -v "$PWD/builds/gis-run:/results" topology-gis-benchmark \
  python -m benchmarks.gis.run \
    --rules point/must-be-inside polygon/must-contain-point \
      point/must-not-overlap line/must-not-have-dangles \
      polygon/must-not-overlap polygon/must-not-have-gaps \
    --sizes 1000 --warmups 2 --repetitions 10 \
    --output /results/results --work /tmp/gis-work
```

Use `--smoke` for the same six checks with small inputs and one repetition.
Select workloads with `--rules`, `--sizes` and `--engines`. Defaults retain all
38 Rust rules, supported external counterparts and 1,000/10,000/100,000 features.
Keep work files on the container's Linux filesystem when using macOS to avoid
case-sensitive gvSIG asset collisions.

The manual `GIS benchmarks` workflow offers `headline` and `full` modes.
Headline mode measures all six checks at 1,000 features per input layer, with
two warmups and ten repetitions. The same container runs sequential engines in
rotating order on one logical CPU. Local emulated runs test correctness and
reproduction; publication requires a native x86_64 Linux CI run.

## Semantics and native mappings

Duplicate points produce every unordered feature pair, including three pairs
for three identical points. Dangles report affected original feature IDs: an
endpoint connected to a line interior is connected, and closed lines have no
free endpoints. Overlaps produce every unordered polygon pair with positive
intersection area, including equal and contained polygons. Boundary-only touches
are permitted. Gaps are bounded uncovered faces of polygon coverage; the exterior
is excluded, and covered islands remain holes in the gap geometry.

| Check | QGIS | GRASS | PostGIS | gvSIG |
| :--- | :--- | :--- | :--- | :--- |
| Points inside polygons | Stock containment rule | `v.select within`, reversed | Indexed `ST_ContainsProperly`, reversed | Stock containment rule |
| Polygons contain a point | Stock containment rule | `v.select contains`, reversed | Indexed `ST_ContainsProperly`, reversed | Stock containment rule |
| Duplicate points | `QgsSpatialIndex`, geometry equality | `v.to.db` coordinates, indexed SQLite pair query, native error points | Indexed `ST_Equals` pairs | Native STRtree, JTS equality |
| Line dangles | Indexed endpoint incidence | Native noding and node-degree inspection | Indexed endpoint-incidence SQL | Native STRtree, JTS endpoint incidence |
| Polygon overlaps | Indexed positive-area intersection | Native spatial candidates and attributed `v.overlay` on original polygons | Indexed positive-area `ST_Intersection` | Native STRtree, positive-area JTS intersection |
| Polygon gaps | Native union and bounded void difference | Bounded native faces with original-ring coverage classification | Native union and bounded void difference | Native JTS union and bounded void difference |

Each record names the operation actually used. GRASS retains separate original
polygon maps for overlays so equal or contained source features keep their IDs.
Its duplicate query reads coordinates extracted by GRASS and writes native error
vectors. Gap faces without centroids are classified against indexed original
rings using the native GRASS point-in-ring operation, so covered overlap faces
are retained as coverage. Noding constructs topology without repairing or removing violations.
gvSIG uses its bundled JTS geometry plugin for the four additional workflows.
The stock topology plans remain in use for containment and other mapped rules.

See the [QGIS geometry API](https://api.qgis.org/api/classQgsGeometry.html),
[GRASS coordinate extraction](https://grass.osgeo.org/grass84/manuals/v.to.db.html)
and [GRASS attributed overlay](https://grass.osgeo.org/grass84/manuals/v.overlay.html).

## Timing and correctness

Fresh geometry copies and source loading happen before the timer. Fresh spatial
indexes, topology construction, validation and native error geometry creation
are timed. Result extraction, serialization and common geometry comparison occur
after timing. GRASS includes module launch and native output writes. PostGIS
measures server-side GiST creation, `ANALYZE` and materialized SQL violations.
QGIS containment runs the stock C++ plugin with painting disabled. gvSIG disables
unrelated profile autoruns and UI work.

Pair checks compare both original IDs. Dangles compare affected feature IDs.
Gaps compare canonical bounded geometry, including islands. Rust emits union
rings; the comparison reconstructs bounded faces from gap rings and nested
exterior island rings outside timing. Collinear vertices and ring orientation
are normalized. Common geometry comparison never supplies validation results
for another tool.

Clean, violating, boundary and hole containment probes remain separate from the
six headline checks. Additional probes cover duplicate triples, T junctions,
closed lines, equal and contained polygons, boundary touches, bounded holes and
islands, covered overlap faces and self-connected line endpoints. All measured repetitions must match Rust and all probes must pass.

## Results and publication

The output includes `results.json`, `samples.csv`, `README.md`, `comparison.png`
and `comparison.svg`. A figure is generated only when all thirty combinations
complete at one common size of at least 1,000 features. It has six groups of
median-time bars, a logarithmic millisecond axis and a five-tool legend.
Statistics, versions, failures and provenance remain in the detailed report.

`publishable.json` additionally requires native Linux CI, one CPU in the recorded
affinity, two warmups, ten repetitions, engine versions, package versions,
container identity, source revision, source hashes and binary hash. The workflow
must upload both measurement reports and diagnostics successfully. Copy all
artifacts from a single successful native run into `results`; do not combine runs
or reuse historical QGIS timings. Partial results retain failures in JSON, CSV
and Markdown and withhold the graph and publication marker.

## Adapter contract and harness checks

Adapters expose `metadata`, `sample(rule, timeout)` and `close()`. Embedded
workers exchange atomic JSON requests and responses with separate preparation,
validation and extraction deadlines. Responses contain `seconds`, `raw_count`
and materialized `errors`, with both source IDs for pairs and geometry for gaps.

```sh
uv run --project benchmarks/gis --group dev pytest benchmarks/gis/tests
uv run --project benchmarks/gis --group dev ruff check benchmarks/gis
uv run --project benchmarks/gis --group dev pyright --project benchmarks/gis
```

Set `GIS_INTEGRATION=1` inside the container to run all native fixture checks.
