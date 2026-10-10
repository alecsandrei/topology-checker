# topology-checker

Validate topology in vector datasets from the command line, or use the GDAL-free Rust core. The workspace implements 18 rules and reports the layer and feature IDs responsible for each error. Coordinates and tolerance are planar `f64` values. This project is at `0.1.0-alpha`; the redesign changes the previous API and command syntax.

```sh
topology-checker line must-not-have-dangles roads.gpkg --layer roads -o errors.gpkg
topology-checker line must-not-overlap-with roads.gpkg --other rivers.shp -o errors.gpkg
topology-checker polygon must-not-have-gaps parcels.geojson --tolerance 0.001 -o gaps.gpkg
topology-checker --json validate topology.toml -o errors.gpkg
```

Inputs are positional paths. Two-input rules require `--other`; `--layer` and `--other-layer` select physical dataset layers. Without selectors, each physical layer is checked separately. Two-input rules expand all source and destination layer pairs. A layer containing an incompatible geometry kind fails with its name and FID.

Exit codes are **0** for valid data, **1** for topology violations and **2** for an unsuccessful run. Finding violations is a successful validation with a nonzero exit code, which lets scripts distinguish it from an I/O or configuration error.

## Build

The CLI bundles GDAL and PROJ by default. A separate GIS installation is unnecessary. The first build compiles both libraries and takes several minutes.

Prerequisites: Rust **1.88 or newer**, CMake, a C/C++ compiler, and the `sqlite3` executable. On macOS, install Xcode Command Line Tools. On Linux, install the compiler toolchain, CMake and sqlite3. On Windows, install Visual Studio Build Tools, CMake and SQLite, with sqlite3 on PATH.

```sh
cargo build --workspace --release --locked
./target/release/topology-checker rules
```

The bundled vector drivers include GPKG, ESRI Shapefile, GeoJSON, FlatGeobuf, CSV, OpenFileGDB, MapInfo File, VRT and Memory. To use an installed GDAL instead:

```sh
cargo build -p topology-checker --release --no-default-features --locked
```

The core builds independently without GDAL, PROJ, CMake or sqlite3:

```sh
cargo check -p topology-checker-core --locked
```

## Commands and options

```sh
topology-checker rules
topology-checker --json rules
topology-checker drivers --read
topology-checker drivers --write
topology-checker drivers --read --write
topology-checker utilities merge-lines roads.gpkg --layer roads -o merged.gpkg
topology-checker utilities explode-lines roads.gpkg --layer roads -o segments.gpkg
```

`rules` lists all definitions and ArcGIS/QGIS equivalents. `drivers` lists drivers available in the current build; combining its flags selects drivers that support both reading and writing. Run any command with `--help` for its arguments.

Global options:

| Option | Behavior |
|---|---|
| `--tolerance VALUE` | Finite, nonnegative distance in input coordinate units |
| `--json` | JSON report on stdout, including warnings and feature references |
| `--overwrite` | Allow replacing existing output datasets |
| `--driver NAME` | Explicit output driver, taking precedence over the extension |

Validation commands accept an optional `-o PATH` (or `--output PATH`). A clean validation creates no file. Existing outputs are preserved unless `--overwrite` is supplied. Utilities require an output path and write a readable empty line layer if their input has no usable lines.

## Topology definitions

```toml
tolerance = 0.001

[layers]
roads = { path = "city.gpkg", layer = "roads" }
rivers = { path = "rivers.shp" }
parcels = { path = "city.gpkg", layer = "parcels" }

[[rules]]
rule = "line/must-not-have-dangles"
layer = "roads"

[[rules]]
rule = "line/must-not-overlap-with"
layer = "roads"
other = "rivers"

[[rules]]
rule = "polygon/must-not-have-gaps"
layer = "parcels"
```

Run with `topology-checker validate topology.toml -o errors.gpkg`. Dataset paths are relative to the definition file. Layer aliases appear in the report. A CLI tolerance overrides the TOML value; otherwise the CRS default applies. Unknown aliases, unknown rules, unexpected fields and incorrect rule arity fail explicitly.

## Rules and tolerance

See [the rule table and architecture](docs/overview.md) for every rule. Important semantics:

- Overlap errors are aggregated per feature pair and error type, with multipart geometry where needed. Self-overlaps and self-crossings use separate rules.
- Closed line loops and self-touches are valid for the self-intersection rule; true crossings are errors. Pseudo nodes require two distinct features.
- Point containment is strict. A point on, or within tolerance of, any boundary of its containing polygon is outside.
- Line containment checks coverage by the polygon layer union, accepts boundaries, and reports only uncovered pieces longer than the tolerance.
- Gaps are union rings, exported as lines. `gap` identifies holes; `outer-boundary` identifies exterior rings and can be filtered in a GIS.
- A one-part `MultiPoint`, `MultiLineString` or `MultiPolygon` satisfies the multipart rule.
- Line merging follows chains through eligible degree-two endpoints, then handles isolated cycles. Interior junctions preserve the original lines. It retains duplicate uses, chooses the majority input direction, and resolves ties using the earliest input part. The independent Rust implementation uses the chain/cycle strategy documented by [GEOS LineMerger](https://libgeos.org/doxygen/classgeos_1_1operation_1_1linemerge_1_1LineMerger.html); it has no GEOS dependency.

The default tolerance is 0.001 metres converted to projected CRS units, `1e-8` for a geographic CRS, or zero if the CRS is unavailable. Geographic calculations remain planar, with a warning for a positive tolerance. Known input CRSs must agree; the tool does not reproject. Missing CRSs produce warnings and disable the shared output CRS.

Endpoint cracking and clustering use stable representative coordinates without modifying the input. Polygon overlap pieces and hole rings whose mean width (`2 * area / perimeter`) is below tolerance are ignored. Tolerance is not a general polygon snapping or geometry repair operation.

Null and empty geometries are skipped with counts. Degenerate parts and rings are skipped with counts. Z/M values are dropped with a warning. Nonfinite coordinates, duplicate FIDs and unsupported geometry kinds return errors. Valid polygon topology is assumed; invalid geometry detection is a future rule.

## Error output

A driver is inferred from an exact extension token. Unknown or ambiguous extensions require `--driver`. Multi-layer formats such as GPKG receive `errors_point`, `errors_line` and `errors_polygon` as applicable. Single-layer formats receive the requested path for one geometry family, or `<stem>_point.<ext>`, `<stem>_line.<ext>` and `<stem>_polygon.<ext>` for mixed errors. Every intended output is checked for existing files before writing starts.

Fields are `rule`, `error_type`, `origin_layer`, `origin_fid`, `destination_layer` and `destination_fid`. Feature IDs are signed 64-bit integers. Layer-wide gap errors have nullable feature references; single-feature errors have a null destination. The JSON report also carries layer context and an optional part index, which becomes null if several parts contributed to one pair error.

Shapefile's field length limit requires the aliases `orig_layer`, `dest_layer` and `dest_fid`; the CLI warns about these names. The input CRS is copied to outputs when available. Writers use a dataset transaction when supported, but overwriting a previous dataset is not an atomic replacement.

## Rust library and development

Use the [core API and compiled example](crates/core/README.md). `topology-checker-core` exports the geometry model, rule registry, reports, `merge_lines` and `explode_lines`; its normal dependencies contain no GDAL or GEOS.

```sh
cargo test --workspace --release --locked
cargo clippy --workspace --all-targets --release --locked -- -D warnings
cargo fmt --check
cargo bench -p topology-checker-core --bench rules --locked
```

Acceptance tests cover the historical failures. Property tests compare small random datasets against brute-force distance, containment and DE-9IM references. [Performance notes](docs/performance.md) separate core timings from complete CLI runs and explain the generated workloads. GitHub Actions configures Ubuntu, Windows, macOS and an MSRV check.

Exceptions, automatic fixes, partial validation, tiling, reprojection, invalid-geometry rules, duplicate rules and the covered-by family remain future work.
