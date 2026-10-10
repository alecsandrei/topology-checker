# Performance measurements

Measured on 2026-10-10 on an Apple M5 with 10 CPU cores, macOS, Rust 1.95, release optimization and the default Rayon thread pool. These are synthetic workloads with bounded local candidate density, rather than claims about every GIS dataset. The CLI was built with bundled GDAL. Measurements exclude exporting errors.

## Reproduce the core benchmarks

```sh
cargo bench -p topology-checker-core --bench rules --locked -- --noplot
```

The Criterion target uses 10 samples, one second of warmup and a requested three-second measurement interval per case. Criterion extends the interval when needed. Geometry generation and the input clone in batch setup are excluded. The timed operation includes a fresh topology, owned layers, part views, R-trees, rule registration, graph construction or predicates, report creation and destruction. Each iteration starts with empty caches. Merge timing includes validation, indices, endpoint contacts, traversal and output allocation. There is no GDAL, disk I/O or JSON decoding in this target.

Generators in [`rules.rs`](../crates/core/benches/rules.rs):

- Dangles: `n` consecutive segments `(i, 0)` to `(i + 1, 0)`, with two final dangles.
- Polygon overlaps: rectangles `[1.5*i, 1.5*i + 2]` by `[0, 2]`, with an overlap between each neighboring pair.
- Point containment: `(i, 0)` points inside one rectangle `[-1, n + 1]` by `[-1, 1]`, with no violations.
- Merge: the same chain of consecutive segments, producing one line.

Criterion's central time estimates, milliseconds:

| Workload | 40k | 80k | 160k | 40k to 80k | 80k to 160k |
|---|---:|---:|---:|---:|---:|
| Dangles | 34.68 | 78.60 | 166.53 | 2.27x | 2.12x |
| Polygon overlaps | 41.16 | 90.07 | 182.89 | 2.19x | 2.03x |
| Point containment | 6.31 | 12.30 | 27.07 | 1.95x | 2.20x |
| Merge lines | 34.04 | 83.51 | 164.24 | 2.45x | 1.97x |

Each measured doubling is below the plan's 2.5x scaling target. The saved historical timings were 3.77 seconds for 160k dangles, 6.45 seconds for 80k polygon overlaps and 0.65 seconds for 160k point containment. Dividing those numbers by the core times gives roughly 23x, 72x and 24x, respectively, but the historical measurements included CLI overhead and did not preserve exact generator definitions. Those ratios are context, not an equivalent-workload speedup claim.

## Compare complete CLIs

The [`compare_cli`](../crates/core/examples/compare_cli.rs) example exports these same generators as GeoJSON and runs both binaries. It performs one untimed warmup followed by three runs per binary/workload/size. Each measured run includes process startup, GDAL reading, the rule and its human summary, with stdout discarded. It does not request error output. Both binaries use GDAL reading; the old CLI receives its legacy argument syntax and the new CLI uses exact tolerance zero. Different old/new validation exit codes are retained in the CSV, since the old CLI always returned zero for violations.

```sh
cargo run -p topology-checker-core --release --example compare_cli --locked -- \
    /path/to/old/topology-checker \
    /path/to/new/topology-checker \
    /tmp/topology-bench > /tmp/topology-cli-times.csv
```

The old binary is the release build of `294ea80`. Timings and medians from a separate run, without concurrent compilation, are recorded below. This comparison checks equivalent input files, while retaining the redesign's richer identity and report behavior.

Median seconds (three measured runs after warmup):

| Workload | 40k old / new | 80k old / new | 160k old / new | Speedup at the plan's size |
|---|---:|---:|---:|---:|
| Dangles | 0.319 / 0.326 | 0.988 / 0.237 | 3.743 / 0.493 | 7.6x at 160k |
| Polygon overlaps | 0.768 / 0.216 | 2.010 / 0.347 | 7.134 / 0.677 | 5.8x at 80k |
| Point containment | 0.445 / 0.076 | 0.298 / 0.153 | 0.816 / 0.286 | 2.9x at 160k |

The raw runs are in [`performance-cli.csv`](performance-cli.csv). Short runs show system noise, including the old 40k point result being slower than 80k, and the new 40k dangle result being slower than 80k. The 80k to 160k new CLI ratios are 2.08x, 1.95x and 1.87x. More repetitions on an otherwise idle machine would improve confidence in the short-run estimates.

The new core meets the scaling target on all measured workloads. **The 10x target is not met for every complete CLI run at the requested sizes.** Polygon overlap reaches about 10.5x at 160k, but the requested comparison size was 80k. GDAL input decoding, process startup, richer feature bookkeeping and report allocation contribute to complete CLI latency. The core-to-historical ratios above must not be presented as CLI speedups.

These measurements do not benchmark QGIS, ArcGIS or GEOS, and do not establish a fastest-available merger. The independent Rust merger has a linear chain/cycle traversal after indexed contact discovery. Dense intersections, extensive overlapping envelopes, positive-tolerance clustering, expensive polygons and large outputs can change both runtime and scaling. Proper crossings incur exact rational arithmetic to keep concurrent nodes consistent; these bounded-density chain fixtures exercise endpoint joins instead.
