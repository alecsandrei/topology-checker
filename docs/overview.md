# topology-checker: overview, known problems and roadmap

Status as of 2026-10-10, commit `294ea80`, version `0.1.0-alpha`. All behavior described under "Known problems" was reproduced against a release build of that commit unless it says "by reading the code".

- [1. What the tool is](#1-what-the-tool-is)
- [2. How it works](#2-how-it-works)
- [3. Rules](#3-rules)
- [4. Using it](#4-using-it)
- [5. Known problems](#5-known-problems)
- [6. Directions](#6-directions)

## 1. What the tool is

topology-checker checks vector datasets against **topology rules**: spatial relationships that features in a layer, or between two layers, must satisfy. Examples are "lines must not have dangles" and "polygons must not overlap". It reports every place where the data breaks a rule and can write those places to a GIS file, so they can be inspected next to the data.

The project is modelled on two existing tools:

- [ArcGIS Pro geodatabase topology](https://doc.esri.com/en/arcgis-pro/latest/help/data/topologies/validating-a-topology.html)
- The [QGIS Topology Checker plugin](https://docs.qgis.org/latest/en/docs/user_manual/plugins/core_plugins/plugins_topology_checker.html)

Its rule names and error geometries follow ArcGIS's. The goal stated in the README is to be faster than those tools and usable without a GUI.

| | ArcGIS Pro topology | QGIS Topology Checker | topology-checker |
|---|---|---|---|
| Runs in | ArcGIS Pro. Creating a topology needs a Standard or Advanced license | QGIS desktop, as a dock panel | Command line, or as a Rust library. Builds on macOS, Linux and Windows with no GIS install |
| Input data | Feature classes in one feature dataset of a geodatabase | Any layer loaded in the project | Any file GDAL can read (bundled drivers: GPKG, Shapefile, GeoJSON, FlatGeobuf, CSV, OpenFileGDB, MapInfo) |
| Rules | 11 polygon, 16 line, 6 point rules | 19 rules over point/line/polygon layers | 13 rules (see [section 3](#3-rules)) |
| Errors carry feature IDs | Yes (origin and destination feature) | Yes (error table, zoom to feature) | **No** |
| Tolerance | Cluster tolerance | None documented | **None**. Coordinates are compared exactly |
| Exceptions | Errors can be marked as exceptions | No | No |
| Fixes | Per-rule fixes (merge, subtract, extend, trim, ...) | None (QGIS's separate Geometry Checker plugin has some) | None |
| Partial validation | Dirty areas: only edited areas are revalidated | "Validate extent" checks only the visible map extent | No. The whole dataset is checked every run |
| Output | Point/line/polygon error layers + Error Inspector | Table and map highlight | Error geometries in a file, with a `rule` field |

Where this tool can win is the space the other two don't cover well:

- Headless and scriptable, so it fits in CI and ETL pipelines.
- No license and no geodatabase required.
- Plain files in, plain files out.
- Multi-core.

## 2. How it works

```
input file ──► open: GDAL, or geozero for .json/.geojson ──► first layer only ──► Vec<geo::Geometry>
   ──► flatten to single parts (Vec<Point> / Vec<LineString> / Vec<Polygon>)
   ──► rule: rstar R-tree candidate pairs + geo predicates / boolean ops / sweep-line intersections, rayon for parallelism
   ──► TopologyResult::{Valid | Errors(Vec<TopologyError>)}
   ──► summary table on stdout ──► optional export of the error geometries through GDAL
```

| Path | Responsibility |
|---|---|
| [`src/main.rs`](../src/main.rs) | clap CLI with one match arm per rule (read → flatten → check → export), interactive mode, utilities |
| [`src/lib.rs`](../src/lib.rs) | `VectorDataset` (reading, SRS), `TopologyError` / `TopologyResult`, export (`ExportConfig`, `TopologyResults`) |
| [`src/rule/`](../src/rule) | One trait per rule, implemented on `Vec<Point>`, `Vec<LineString>`, `Vec<Polygon>` or `Vec<Geometry>` |
| [`src/util/geometry.rs`](../src/util/geometry.rs) | Flatten/explode helpers, `intersections()` wrapper around geo's sweep-line |
| [`src/util/io.rs`](../src/util/io.rs) | Open/create datasets, output driver inference from the file extension, SRS comparison |
| [`src/algorithm/`](../src/algorithm) | `merge_linestrings` (used by `utilities merge-linestrings`) and an unfinished optimized version |

Main dependencies:

- geo 0.28, rstar 0.12, rayon and clap 4.
- gdal 0.19, which compiles GDAL 3.12.1 and PROJ 9.6.2 from source and links them statically.
- geozero 0.13, used only to read GeoJSON.

Build instructions are in the [README](../README.md#building).

## 3. Rules

The "Error geometry" column is what gets written to the output file.

| CLI command | Reports | Error geometry | ArcGIS equivalent | QGIS equivalent |
|---|---|---|---|---|
| `point must-not-overlap` | Points at the same location as another point of the layer | Point | Must Be Disjoint | Must not have duplicates |
| `point must-not-overlap-with` | Points at the same location as a point of a second layer | Point | none (opposite of Must Be Coincident With) | none |
| `point must-be-inside` | Points not strictly inside any polygon. A point on a boundary counts as outside | Point | Must Be Properly Inside Polygons | Must be inside |
| `line must-not-overlap` | Collinear overlaps between lines of the layer | Line | Must Not Overlap | none |
| `line must-not-overlap-with` | Overlaps with lines of a second layer | Line | Must Not Overlap With | none |
| `line must-not-self-overlap` | Places where a line overlaps itself | Line | Must Not Self-Overlap | none |
| `line must-not-have-dangles` | Line endpoints that touch neither another line nor the line itself | Point | Must Not Have Dangles | Must not have dangles |
| `line must-not-intersect` | Crossing points, and collinear overlaps. An endpoint touching another line's interior is allowed | Point + Line | Must Not Intersect | none |
| `line must-be-inside` | Lines not contained in a polygon | Line | Must Be Inside | none |
| `polygon must-not-overlap` | Overlapping areas between polygons of the layer | Polygon | Must Not Overlap | Must not overlap |
| `polygon must-not-overlap-with` | Overlapping areas with polygons of a second layer | Polygon | Must Not Overlap With | Must not overlap with |
| `polygon must-not-have-gaps` | Boundary segments not shared with another polygon. Like ArcGIS, this includes holes and the outer boundary of the layer | Line | Must Not Have Gaps | Must not have gaps |
| `geometry must-not-be-multipart` | Multi-part features (`MultiPoint`, `MultiLineString`, `MultiPolygon`) | Same as input | Must Be Single Part (lines only) | Must not have multi-part geometries |

Utilities, which are not rules:

- `utilities merge-linestrings`: joins lines end to end wherever exactly two lines meet. This is the natural fix for pseudo nodes.
- `utilities explode-linestrings`: writes every segment as its own line.
- `gdal-drivers read | write | read-and-write`: lists the formats the build can handle.

Rules ArcGIS or QGIS have that this tool does not are listed in [section 5.3](#53-missing-compared-with-arcgis-and-qgis).

## 4. Using it

### Command line

```sh
topology-checker [--use-gdal] [--gdal-driver NAME] [--epsg CODE] [--elapsed] <point|line|polygon|geometry> <rule> ARGS...
```

Global flags:

| Flag | Meaning |
|---|---|
| `-u`, `--use-gdal` | Read `.json` / `.geojson` through GDAL instead of geozero. Other formats always go through GDAL |
| `--gdal-driver NAME` (env `GDAL_DRIVER`) | Output format. Currently only honored by `utilities`, see [5.4](#54-command-line-and-io) |
| `--epsg CODE` (env `EPSG`) | CRS of the GeoPackage written by interactive mode |
| `-e`, `--elapsed` | Print the run time |

Arguments are written `KEY=value`. **Only the position matters; the key is ignored** (see [5.4](#54-command-line-and-io)). Use them in this order:

```sh
topology-checker point   must-not-overlap        POINTS=in.shp  [OVERLAPS=out.shp]
topology-checker point   must-not-overlap-with   POINTS=in.shp  OTHER=other.shp  [OVERLAPS=out.shp]
topology-checker point   must-be-inside          POINTS=in.shp  POLYGONS=poly.shp  [out.shp]   # no KEY= here, see 5.4
topology-checker line    must-not-overlap        LINES=in.shp   [OVERLAPS=out.shp]
topology-checker line    must-not-overlap-with   LINES=in.shp   OTHER=other.shp  [OVERLAPS=out.shp]
topology-checker line    must-not-self-overlap   LINES=in.shp   [OVERLAPS=out.shp]
topology-checker line    must-not-have-dangles   LINES=in.shp   [DANGLES=out.shp]
topology-checker line    must-not-intersect      LINES=in.shp   [SINGLE_POINTS=pts.shp] [COLLINEAR_LINES=lines.shp]
topology-checker line    must-be-inside          LINES=in.shp   POLYGONS=poly.shp  [OUTSIDE_LINES=out.shp]
topology-checker polygon must-not-overlap        POLYGONS=in.shp [OVERLAPS=out.shp]
topology-checker polygon must-not-overlap-with   POLYGONS=in.shp OTHER=other.shp [OVERLAPS=out.shp]
topology-checker polygon must-not-have-gaps      POLYGONS=in.shp [GAPS=out.shp]
topology-checker geometry must-not-be-multipart  GEOMETRIES=in.shp [MULTIPARTS=out.shp]
topology-checker utilities merge-linestrings     LINESTRINGS=in.shp MERGED=out.shp
topology-checker utilities explode-linestrings   LINESTRINGS=in.shp LINES=out.shp
```

What a run does:

- **Summary:** it prints the number of errors per geometry type.
- **Output file:**
  - The format is inferred from the extension.
  - The layer is named after the geometry type (`POINT`, `LINESTRING`, `POLYGON`) and has a `rule` text field.
  - The CRS is copied from the first input.
  - An existing output file is replaced.
- **Inputs:** only the first layer of each input dataset is read.

### Interactive mode

```sh
topology-checker --epsg 3844 interactive OUTPUT=results.gpkg
line must-not-have-dangles lines=./roads.shp
polygon must-not-overlap polygons=./parcels.shp
summary
```

Interactive mode collects rules one per line. Typing `summary` runs all the collected rules in parallel and writes every error into one GeoPackage, with one layer per geometry type. **It currently crashes as soon as any rule finds an error** ([5.1](#51-crashes-on-ordinary-input)).

### As a library

Each rule is a trait implemented on vectors of `geo` geometries and returns a `TopologyResult`:

```rust
use geo::{line_string, LineString};
use topology_checker::{rule::MustNotHaveDangles, TopologyError};

let lines: Vec<LineString> = vec![
    line_string![(x: 0., y: 0.), (x: 1., y: 1.)],
    line_string![(x: 1., y: 1.), (x: 2., y: 2.)],
];
let result = lines.must_not_have_dangles();
if let TopologyError::Point(dangles) = result.unwrap_err_point() {
    assert_eq!(dangles.len(), 2); // (0, 0) and (2, 2)
}
```

The library example in the README is out of date and does not compile ([5.6](#56-code-health)). The example above does.

## 5. Known problems

Ordered from most to least urgent within each subsection.

### 5.1 Crashes on ordinary input

1. **Clean data plus an output path panics** for three rules with `Called unwrap_err on a Valid variant.`:
   - `polygon must-not-overlap-with`
   - `polygon must-not-have-gaps`
   - `geometry must-not-be-multipart`

   These rules export without first checking `!result.is_valid()` ([main.rs:687](../src/main.rs#L687), [:700](../src/main.rs#L700), [:717](../src/main.rs#L717)). Repro: two disjoint squares with `OVERLAPS=out.shp`.
2. **Interactive mode panics whenever a rule finds errors**, with `Can not clone ExportConfig when the field Dataset is Some()`. `TopologyResults::export` clones an `ExportConfig` that holds the open transaction ([lib.rs:472](../src/lib.rs#L472)), and the `Clone` impl panics in that case ([lib.rs:319](../src/lib.rs#L319)).
3. **The wrong geometry type panics** instead of returning an error. For example, a polygon file passed to a line rule gives `Unallowed geometries found.` ([util/geometry.rs:28](../src/util/geometry.rs#L28), and the same in `flatten_points` / `flatten_polygons`).
4. **An empty linestring panics** with `range start index 1 out of range` ([util/geometry.rs:111](../src/util/geometry.rs#L111)). `linestring_endpoints` unwraps on the same input ([:129](../src/util/geometry.rs#L129)).
5. **A feature without geometry aborts the GDAL read path** with a misleading `Unsupported GDAL geometry type` and three `ERROR 10: Pointer 'hGeom' is NULL` lines ([lib.rs:56-70](../src/lib.rs#L56-L70)). The geozero path silently drops the same feature instead, so the two read paths disagree.
6. Driver inference and file creation use `expect`/`unwrap` throughout ([util/io.rs](../src/util/io.rs)). A user mistake such as an unknown extension ends in a panic, not an error message.

### 5.2 Wrong results

Every row was reproduced with a two- or three-feature GeoJSON file.

| Rule | Input | Expected | Actual | Cause |
|---|---|---|---|---|
| `line must-not-overlap` | Two identical lines `(0 0, 2 0)` | 1 overlap | **No errors** | `intersections()` removes identical segments before the sweep ([util/geometry.rs:166](../src/util/geometry.rs#L166)). Exact duplicates, the most common overlap, are lost. A reversed duplicate is caught |
| `line must-not-overlap-with` | `(0 0, 2 0)` vs `(1 0, 3 0)` | Overlap `(1 0, 2 0)` | **No errors** | Only reports when one segment fully contains the other ([rule/must_not_overlap.rs:103](../src/rule/must_not_overlap.rs#L103)). When it does report, it writes the whole segment instead of the shared part |
| `line must-not-self-overlap` | `(0 0, 2 0, 2 1, 1 1, 1 0, 3 0)` | Overlap `(1 0, 2 0)` | **No errors** | Same containment-only test ([rule/must_not_overlap.rs:184](../src/rule/must_not_overlap.rs#L184)) |
| `polygon must-not-have-gaps` | Two adjacent squares; the shared edge has an extra vertex on one side only | 6 segments (outer boundary) | **8 segments** | A segment counts as shared only if it contains another segment ([rule/must_not_have_gaps.rs:29-36](../src/rule/must_not_have_gaps.rs#L29-L36)). The shorter pieces never contain the longer neighbour. Real neighbouring polygons rarely have matching vertices |
| `geometry must-not-be-multipart` | `MultiPolygon` features with a single part | No errors | **Every feature** | Checks the type, not the number of parts ([rule/must_not_be_multipart.rs:13-24](../src/rule/must_not_be_multipart.rs#L13-L24)). GeoPackages written by QGIS or with `-nlt PROMOTE_TO_MULTI` store every polygon this way |
| `line must-not-have-dangles` (and every rule) | Two lines whose endpoints are 1e-9 apart | 2 dangles | **4 dangles** | Coordinates are compared exactly. There is no tolerance |

Where the tool's rules differ from ArcGIS's definitions of the rules they are named after:

- **`line must-not-intersect` also reports a line crossing itself, and `line must-not-overlap` also reports a line doubling back on itself.** ArcGIS's Must Not Intersect says lines "can touch, intersect, and overlap themselves". It keeps those cases in separate rules: Must Not Self-Intersect and Must Not Self-Overlap.
- **`line must-be-inside` tests each polygon separately.** A line that runs from one polygon into an adjacent polygon of the same layer is flagged, even though it never leaves the layer. You need to decide which meaning is intended, document it, and test it.
- **Point and polygon overlaps are reported once per pair.** Three identical points produce three error points at one location. That only stops being confusing once errors carry feature IDs ([5.3](#53-missing-compared-with-arcgis-and-qgis)).

### 5.3 Missing compared with ArcGIS and QGIS

1. **No feature identity.** This is the biggest structural gap.
   - `VectorDataset::to_geo` returns bare `Vec<geo::Geometry>`, so FIDs and attributes are dropped ([lib.rs:40](../src/lib.rs#L40)).
   - `flatten_*` then splits multi-part features into parts and shuffles their order, because `par_bridge` does not preserve order.
   - As a result, an error cannot say which feature, or which pair of features, is at fault. ArcGIS errors record the origin and destination feature, and QGIS lists feature IDs and zooms to them.
   - Everything below that involves exceptions, fixes or filtering by attribute depends on this.
2. **No tolerance.** ArcGIS validates within a cluster tolerance. Here, slightly mis-snapped data produces false dangles, gaps and missed overlaps.
3. **Only the first layer of a dataset is read** ([lib.rs:47-50](../src/lib.rs#L47-L50)), and there is no `--layer` option. Multi-layer GeoPackages and FileGDBs are only partly usable.
4. **No exceptions.** `polygon must-not-have-gaps` always reports the layer's outer boundary. In ArcGIS users mark it as an exception; here there is no way to suppress it.
5. **Missing rules:**

   | Rule | ArcGIS | QGIS |
   |---|---|---|
   | Must not have invalid geometries | | ✓ |
   | Must not have duplicates (lines, polygons) | | ✓ |
   | Line must not have pseudo nodes | ✓ | ✓ |
   | Line must not self-intersect | ✓ | |
   | Line must not intersect with (two layers) | ✓ | |
   | Line must not intersect or touch interior (and the two-layer "with" variant) | ✓ | |
   | Line endpoint must be covered by points | ✓ | ✓ |
   | Line must be covered by (lines of another layer / polygon boundaries) | ✓ | |
   | Point must be covered by line / by line endpoint / by polygon boundary | ✓ | ✓ (first two) |
   | Point must be coincident with | ✓ | |
   | Polygon contains point / contains one point | ✓ | ✓ (contains) |
   | Polygon must be covered by / covered by feature class of / must cover each other | ✓ | |
   | Polygon boundary must be covered by lines / by boundary of | ✓ | |
   | Must be larger than cluster tolerance | ✓ | |
6. **No multi-rule topology definition**, apart from interactive mode, which crashes.
7. **Z and M values are silently dropped** when reading through GDAL.
8. **CRS handling is thin:**
   - Inputs are compared but never reprojected.
   - If either input has no CRS, the check is skipped ([util/io.rs:102](../src/util/io.rs#L102)).
   - `polygon must-not-overlap-with` does not compare CRSs at all ([main.rs:675-693](../src/main.rs#L675-L693)).
   - GeoJSON read through geozero has no CRS, so its outputs have none either.
   - All computations are planar, including on data in degrees.
9. **No fixes.**

### 5.4 Command line and I/O

1. **`KEY=value` keys are ignored.** Arguments are positional ([main.rs:7-18](../src/main.rs#L7-L18)).
   - `line must-not-have-dangles DANGLES=out.shp LINES=in.geojson` tries to read `out.shp` as the input.
   - A misspelled key such as `BANANA=in.geojson` is accepted.
   - The README example for `point must-not-overlap-with` puts `OVERLAPS=` before `OTHER=`, so it reads the output path as an input and fails.
2. **`point must-be-inside`'s output argument has no `KEY=` parser** ([main.rs:123-124](../src/main.rs#L123-L124)). `OUTSIDE=out.shp` creates a shapefile literally named `OUTSIDE=out.shp`.
3. **The output format for `.geojson` is picked at random.**
   - Driver inference does a substring match while iterating a `HashMap` ([util/io.rs:110-124](../src/util/io.rs#L110-L124)).
   - `geojson` is a substring of GeoJSONSeq's extensions (`geojsonl geojsons`).
   - In 12 identical runs, 7 wrote GeoJSON and 5 wrote newline-delimited GeoJSONSeq.
4. **`--gdal-driver` is ignored by every rule.** The export always infers the driver from the extension ([lib.rs:230](../src/lib.rs#L230)). `--gdal-driver GPKG ... DANGLES=out.shp` writes a shapefile.
5. **The exit code is 0 even when errors are found**, so the tool cannot gate a CI or QA pipeline.
6. **The summary is counts only**, such as `2 Point errors`. It gives no locations, no feature IDs and no machine-readable report.
7. `parse_rules` sets the output layer name to the rule name, but the export overwrites it with the geometry type ([lib.rs:255](../src/lib.rs#L255)).
8. `utilities explode-linestrings` names its output layer `merged_linestrings` ([main.rs:777](../src/main.rs#L777)).
9. `utilities merge-linestrings` prints `Iteration N completed...` debug lines to stdout ([algorithm/merge_lines.rs:195](../src/algorithm/merge_lines.rs#L195)).

### 5.5 Performance

Several rules scale quadratically. Timings were taken on an Apple M5 (10 cores) with synthetic GeoJSON inputs:

| Rule | 40k features | 80k | 160k | Cause |
|---|---|---|---|---|
| `line must-not-have-dangles` | 0.27 s | 0.99 s | 3.77 s | `dedup_lines` calls `Vec::contains` in a loop, O(n²) ([util/geometry.rs:138-148](../src/util/geometry.rs#L138-L148)). The same code runs inside `must-not-intersect`, `line must-not-overlap` and `merge_linestring_optimized` |
| `polygon must-not-overlap` | 1.79 s | 6.45 s | n/a | Visited pairs are kept in a `Vec` and searched for every candidate pair, O(k²) in the number of pairs, and the loop runs on one thread ([rule/must_not_overlap.rs:31-37](../src/rule/must_not_overlap.rs#L31-L37)). `point must-not-overlap` has the same code ([:128-134](../src/rule/must_not_overlap.rs#L128-L134)) |
| `point must-be-inside` | 0.08 s | 0.22 s | 0.65 s | `inside_points.contains(&point)` for every point, O(n·m) ([rule/must_be_inside.rs:32](../src/rule/must_be_inside.rs#L32), [:60](../src/rule/must_be_inside.rs#L60)) |

Doubling the input multiplies the time by about 3.5 to 4. If that trend holds, 1M line segments take about 2.5 minutes for dangles and 1M polygons take about 17 minutes for overlaps. Real road networks and parcel layers reach those sizes.

Other performance issues:

- `merge_linestrings` scans every line for every line on each pass, then repeats the pass until nothing changes.
- `par_bridge` is used on `Vec`s where `into_par_iter` would be faster and would keep the order.
- The whole dataset is held in memory as `geo` types. Nothing is streamed or tiled.
- There are no benchmarks, so the README's "faster than current alternatives" has not been measured.

### 5.6 Code health

- **`main.rs` repeats one pattern 13 times.** Each rule has a near-identical read → flatten → check → export block, so adding a rule touches four places.
- **`rule_name()` recovers the rule's name by serializing the clap command to JSON** ([main.rs:344-366](../src/main.rs#L344-L366)). Its own TODO calls this "really ugly".
- **Several APIs panic instead of returning errors:**
  - `ExportConfig` has a `Clone` impl that panics.
  - `TopologyResult` has `unwrap_err_*` methods that panic.
  - `MustNotOverlap<T, I, O>` has an unused type parameter.
  - The `prelude` omits `MustNotHaveGaps` and `MustNotSelfOverlap`.
- **geo is at 0.28; the latest is 0.33.** Newer versions bring:
  - Boolean operations built on `i_overlay`, which panic far less on awkward input.
  - A `Validation` trait (0.30) and unary union (0.30).
  - A Bentley-Ottmann `Intersections` that "no longer panics on pathological input" (0.31).
  - `Covers` and `ContainsProperly` (0.32).
  - `PreparedGeometry` for repeated relate operations.
- **There are two read paths (GDAL, and geozero for GeoJSON).** They behave differently for null geometries and CRS.
- **Test coverage is thin.** There are 25 unit tests:
  - Dangles, intersect, gaps and multipart have none.
  - The line tests of `must_be_inside` are empty stubs.
  - Nothing tests the CLI or I/O.
  - The 7 `merge_lines_optimized` tests (3 failing) are skipped in CI.
  - The README doctest is never compiled, because its wrapper sits inside `#[cfg(test)]` ([lib.rs:481-495](../src/lib.rs#L481-L495)). Its example is out of date: it expects `must_not_have_dangles()` to return `Vec<Point>`.
- **Small issues:** there are 2 compiler warnings, and CI runs no clippy or rustfmt. The README examples point at `./assets/*.shp`, which is gitignored.

## 6. Directions

**Positioning.** Don't try to be an editor. Be the topology validator that runs anywhere:

- Point it at files, or at a topology definition.
- Get ArcGIS-style error layers, with feature IDs, back in seconds.
- Use it from CI, cron or an ETL job, on any OS, with no license.

That is the gap ArcGIS (desktop, licensed, geodatabase-only) and the QGIS plugin (GUI-only) leave open.

The phases below are ordered so that each builds on the previous one.

### Phase 0: make what exists trustworthy (small)

- **Fix every crash in [5.1](#51-crashes-on-ordinary-input).**
  - `flatten_*` should return `Result`.
  - Null and empty geometries should be skipped and counted, not abort the run.
  - Replace `ExportConfig`'s panicking `Clone` by passing `&mut Dataset` explicitly.
- **Replace the positional `KEY=value` arguments with real named options**, for example `--lines in.shp --dangles out.shp`. If the `KEY=value` style is kept, validate the keys.
- **Make output driver inference deterministic:** match whole extensions, never substrings. Make every rule honor `--gdal-driver`.
- **Exit with a non-zero code when errors are found.** Either make it the default or put it behind `--fail-on-errors`.
- **Fix the README:**
  - Correct the argument order.
  - Replace the library example with the one in [section 4](#as-a-library).
  - Move the doctest wrapper out of `#[cfg(test)]` so the README is compiled in CI.
- **Add `cargo clippy` and `cargo fmt --check` to CI.**

### Phase 1: correct, identifiable results (the foundation)

- **Carry feature identity end to end.**
  - Read features as `{ layer, fid, part, geometry }`.
  - Have rules return records like `{ rule, geometry, origin: (layer, fid), destination: Option<(layer, fid)> }`.
  - Write `origin_layer`, `origin_fid`, `dest_layer` and `dest_fid` fields next to `rule`.

  This is what ArcGIS and QGIS errors contain, and exceptions, fixes and readable reports all depend on it.
- **Fix the rule bugs in [5.2](#52-wrong-results).** Turn each repro there into a test.
- **Write a one-paragraph spec for each rule:** what counts as an error, what the error geometry is, and how touching boundaries are treated. Align it with ArcGIS's definition of the same name, and split self-intersection and self-overlap into their own rules. The specs double as user documentation.
- **Test against a reference.** For each rule, keep a brute-force O(n²) reference implementation in the tests. Compare it with the fast implementation on generated inputs, using property tests such as `proptest`, plus small hand-made fixtures.
- **Use one read path, GDAL for every format, and drop geozero.** That gives consistent FIDs, CRS and null handling. Read every layer, or the one named by `--layer`.

### Phase 2: scale

- **Remove the quadratic code paths:**
  - Dedup with sort or `HashSet` instead of `Vec::contains`.
  - Emit each candidate pair once, using `i < j` indices from the R-tree.
  - Track inside/outside with a bitset.
  - Evaluate candidate pairs with `into_par_iter`.
- **Upgrade geo 0.28 → 0.33**, for robust boolean operations, the sweep-line rewrite, `Validation`, `Covers` and `PreparedGeometry`.
- **Add benchmarks.**
  - Use `criterion` for the rules.
  - Build a reproducible dataset set: for example, a country's OpenStreetMap roads and a cadastral parcel layer.
  - Time the same rules in the QGIS Topology Checker and ArcGIS Validate Topology.

  This turns the README's speed claim into a measured one.
- **For inputs that don't fit comfortably in memory:** split the extent into tiles with an overlap margin, check the tiles in parallel and merge the errors. Stream features from GDAL instead of collecting them.

### Phase 3: parity with ArcGIS and QGIS

- **Tolerance.** Add a `--tolerance`, ArcGIS's cluster tolerance, used only when comparing:
  - Snap-to-grid keys for endpoint matching.
  - Distance queries on the R-tree for "touches".

  Never modify the input. Warn when a tolerance is used on a geographic CRS.
- **Missing rules, cheapest and most useful first:**
  1. `must-not-have-invalid-geometries` (QGIS): geo's `Validation`.
  2. `must-not-have-duplicates` for all types (QGIS): hash normalized coordinates.
  3. `line must-not-have-pseudo-nodes` (ArcGIS and QGIS): endpoints where exactly two lines meet. `merge-linestrings` is already its fix.
  4. `line must-not-self-intersect`, `must-not-intersect-with` and `must-not-intersect-or-touch-interior[-with]` (ArcGIS). These reuse the existing sweep.
  5. The "covered by" family:
     - point covered by line / line endpoint / polygon boundary
     - line endpoint covered by point
     - line covered by lines / polygon boundary
     - polygon covered by polygons, must cover each other, boundary covered by
  6. `polygon contains-point` / `contains-one-point` (ArcGIS; QGIS "must contain").
  7. `must-be-larger-than-cluster-tolerance`, once tolerance exists.
- **A topology definition file to replace interactive mode:**

  ```toml
  tolerance = 0.001

  [layers]
  roads   = { path = "city.gpkg", layer = "roads" }
  parcels = { path = "city.gpkg", layer = "parcels" }

  [[rules]]
  rule = "line/must-not-have-dangles"
  layers = ["roads"]

  [[rules]]
  rule = "polygon/must-not-overlap"
  layers = ["parcels"]
  ```

  `topology-checker validate topology.toml --output errors.gpkg` would write one GeoPackage. It would contain `errors_point`, `errors_line` and `errors_polygon` layers plus a summary table, mirroring ArcGIS's error layers.
- **Exceptions.** Read accepted errors back from a previous run's error GeoPackage (an `is_exception` field) or from an exceptions file, and leave them out of the results. The outer boundary in the gaps rule is the first use case.
- **A rule registry in code.** Define a `Rule` trait with a name, input kinds, parameters and `check(&inputs) -> Vec<ErrorRecord>`. Have the CLI, the definition file and the library all use one list. This removes the 13 copied match arms and the `rule_name()` JSON trick.

### Phase 4: beyond detection

- **Fixes, written to a new dataset and never applied in place:**
  - Merge at pseudo nodes.
  - Extend or trim dangles within the tolerance.
  - Remove overlaps.
  - Create gap polygons. This needs unary union, which geo ≥ 0.30 has.
- **Partial validation**, like ArcGIS's dirty areas or QGIS's "Validate extent": add `--extent minx,miny,maxx,maxy` to check only part of a dataset.
- **QGIS integration.** Ship a QGIS style file for the error GeoPackage, or a small Processing provider that calls the CLI, so errors can be browsed on the map.

### Suggested next steps

1. Phase 0: the crash fixes and named arguments. These are small changes that make the tool safe to hand to someone else.
2. Phase 1: carry feature IDs through the pipeline. This is the one refactor everything else depends on, so do it before adding rules.
3. Phase 1: fix the rule bugs in [5.2](#52-wrong-results) together with the per-rule tests.

These questions need an owner's decision before Phase 1:

- Should `line must-be-inside` test against the polygon layer as a whole, or against each polygon?
- Should gaps be reported as boundary lines, as ArcGIS does, or as gap polygons?
- Should the library keep its trait-on-`Vec` API, or move to the rule registry?
