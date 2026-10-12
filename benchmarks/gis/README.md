# GIS validation benchmarks

This suite compares topology-checker, QGIS 4.2, GRASS GIS, PostGIS SQL workflows
and gvSIG in one Linux container. The root README figure uses two shared
containment checks with 10,000 primary and 10,000 secondary features. The complete
report includes every supported rule, failures and differences in semantics.

QGIS is the official 4.2.3 Ubuntu image pinned by digest. gvSIG is the official
2.7.0 build 5016 archive pinned by SHA-256, running with OpenJDK. The image records
the resolved Linux package versions. FME is optional and currently requires an
existing licensed worker; the suite never registers accounts or acquires licenses.

## Run

```sh
docker build -f benchmarks/gis/Dockerfile -t topology-gis-benchmark .
mkdir -p builds/gis-run
docker run --rm --cpuset-cpus=0 \
  -v "$PWD/builds/gis-run:/results" topology-gis-benchmark \
  python -m benchmarks.gis.run --output /results/results --work /results/work
```

Use `--smoke` for short integration checks. Select workloads with `--rules`,
`--sizes` and `--engines`. Defaults are all 38 Rust rules, the supported external
counterparts, 1,000/10,000/100,000 features, two warmups and ten measured samples.

The `GIS benchmarks` GitHub Actions workflow runs the same image on a native
x86_64 Linux runner. Local emulated runs are useful for testing but cannot produce
the publication marker. All engines run sequentially with rotating order on one
logical CPU. The new Linux measurements must not be mixed with the historical
macOS measurements in `../qgis/results`.

## What is timed

Source geometry loading and fresh copies are outside the timer. Required spatial
indexes, topology construction, rule evaluation and materialized violation
geometries are inside. Response extraction and serialization are outside.

PostGIS records a server-side duration for GiST indexes, statistics and SQL
result materialization. GRASS includes native module launch and native output
writing. QGIS invokes the stock C++ Topology Checker with canvas painting
disabled. gvSIG invokes a fresh stock TopologyPlan with native memory stores and
UI disabled. Its private profile disables unrelated addon autoruns to avoid
background startup work during validation. Native indexing follows each store
provider's policy. Unavailable gvSIG factories are reported explicitly.

GRASS duplicate removal and dangle cleaning are repair workflows, so their
timings are retained without validation speedup ratios. The shared figure uses
single-point and single-polygon features, including polygon holes. Each engine
must pass clean, invalid and boundary probes, preserve affected feature IDs, and
complete every repetition before its timing is eligible for a comparison.

## Results and publication

The output contains `results.json`, `samples.csv`, `README.md`, `comparison.png`
and `comparison.svg`. The workflow uploads partial measurements, fixture files
and engine logs even on failure. `publishable.json` is produced only when every
OSS engine passes both shared comparisons on native Linux CI.

Retrieve a successful CI artifact, copy its results into this directory's
`results` folder, and update the root README image only when that marker exists.
The figure and report are generated exclusively from saved measurements. Failed,
unsupported or semantically different checks remain visible in the report.

## Adapter contract and optional FME

Python adapters expose `metadata`, `sample(rule, timeout)` and `close()`.
Embedded workers use atomic `request.json`/`response.json` files with separate
`preparing`, `validating` and `extracting` phases. A successful response contains
`seconds`, `raw_count` and an `errors` array with primary feature IDs. Timed
operations must materialize the corresponding error geometries before returning.

An existing licensed FME integration can supply `GIS_FME_WORKER_COMMAND` as a
JSON argv array, with this same contract and SpatialFilter containment workflows.
It receives `GIS_WORKER_DIRECTORY`, `GIS_WORKER_FIXTURE` and `GIS_WORKER_RULE`.
Without that configuration FME is recorded as unavailable and does not block
the OSS publication gate.

## Harness checks

```sh
uv run --project benchmarks/gis --group dev pytest benchmarks/gis/tests
uv run --project benchmarks/gis --group dev ruff check benchmarks/gis
uv run --project benchmarks/gis --group dev pyright --project benchmarks/gis
```
