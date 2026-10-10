# Rust pattern review

Reviewed on 2026-10-10 against commit `d98900f`; implemented on 2026-10-10 from the unchanged source at `fba4141`. All nine medium and low priority recommendations are complete. The findings below retain the original review context; completion details, verification, measurements and alpha migration notes follow.

The code already uses current Rust patterns. Its edition 2024 workspace, library and CLI separation, typed operational errors, iterator-based APIs and standard-library caches are appropriate. The useful improvements concern avoidable allocation, CLI I/O behavior, public API contracts and a few weakly typed internal representations. A newer pattern is not automatically a better fit for this synchronous geometry engine.

## Comparison sources

These are primary project sources inspected for this review, rather than third-party style summaries. Branch links can change after the review date.

| Project or guidance | Relevant practice | Application here |
| --- | --- | --- |
| [ripgrep CLI entry point](https://github.com/BurntSushi/ripgrep/blob/master/crates/core/main.rs) | Fallible output operations and explicit handling of broken pipes at the application boundary | Improve output handling without changing rule execution |
| [clap workspace manifest](https://github.com/clap-rs/clap/blob/master/Cargo.toml) | Selected workspace lint policies rather than one universal style setting | Consider a small shared lint policy |
| [Tokio crate root](https://github.com/tokio-rs/tokio/blob/master/tokio/src/lib.rs) | Public documentation and debug implementation lints; guidance to keep library dependency features small | Improve public API documentation and assess unused features |
| [GeoRust geo crate root](https://github.com/georust/geo/blob/main/geo/src/lib.rs) | Metric-space `Distance` and `Length` traits, polygon unions and bulk-loaded spatial indexes | The geometry API and indexing choices here already align |
| [Rust API documentation guidelines](https://rust-lang.github.io/api-guidelines/documentation.html) | Document public contracts, including error conditions | Document constructors, registration, validation and utilities |
| [Rust API type safety guidelines](https://rust-lang.github.io/api-guidelines/type-safety.html) | Prefer meaningful types over interchangeable primitive representations | Keep FIDs numeric and name output fields |
| [Clippy usage guidance](https://doc.rust-lang.org/stable/clippy/usage.html) | Pedantic diagnostics are opinionated; restriction lints should be selected individually | Treat extra diagnostics as review inputs |

The recommendations below are my assessment of this code. Project examples establish useful practices, not a requirement to copy every convention.

## Review findings and completion status

### 1. Fallible stdout handling, medium priority

Implemented: one locked stdout writer flows through command execution and reporting, including help and version output. JSON streams directly with a trailing newline. Explicit flush failures enter the same error path as write failures. Both ordinary and JSON-wrapped broken pipes return status 0 without an error diagnostic; other output failures return 2, and complete violation reports retain 1. Stderr writes and flushes are best effort.

Help and argument diagnostics retain Clap's existing color adaptation through a direct dependency on the already resolved `anstream` 0.6.14. This adds no package or version to the resolved dependency graph.

Locations: `crates/cli/src/main.rs:62`, `crates/cli/src/main.rs:73`, `crates/cli/src/report.rs:6`, and utility output in `crates/cli/src/main.rs:274`.

The CLI uses `println!` throughout. Those calls bypass its `anyhow::Result` error path and can panic if stdout is closed by a downstream command. This is a real behavior issue for shell pipelines, not an obsolete syntax issue: [the standard library documents output failures as a panic condition](https://doc.rust-lang.org/std/macro.println.html).

Suggested change: lock stdout, pass a writer into output routines, use fallible writes and choose an explicit broken-pipe policy. Ripgrep treats a broken pipe as graceful termination. Other output failures should still produce the CLI's operational failure status. Preserve exit status 1 for fully emitted reports containing topology violations.

JSON output currently allocates a complete serialized string before printing. Writing JSON directly to the output writer would remove that extra string allocation. If this is implemented, test both text and JSON closed-pipe behavior: a serializer may wrap I/O errors, so handling only a directly downcastable `std::io::Error` is insufficient without checking the serializer's error API.

### 2. Avoidable geometry copies, medium priority

Implemented: multipart cleaning dispatches borrowed points, lines and polygons to typed helpers; aggregation consumes groups and moves payloads; merging appends forward or reverse interior coordinates with snapped endpoints and reserves each complete append; GDAL polygon conversion consumes its ring iterator. Part numbering, skipped counts, aggregation fallback and geometry priority remain covered by tests.

The current ownership boundaries are sound, but several functions copy data they could borrow or move:

| Location | Current behavior | Suggested approach |
| --- | --- | --- |
| `crates/core/src/model.rs:227` and `:232` | Clone each multipart line or polygon into a temporary `Geometry`, then copy coordinates again while cleaning it | Dispatch borrowed parts to typed helpers and allocate only the cleaned cached geometry |
| `crates/core/src/topology.rs:176` | Clone the first error, borrow an owned group and clone line or polygon payloads into the aggregate | Consume the group and move its geometry payloads, retaining provenance aggregation and ordering |
| `crates/core/src/merge.rs:96` | Clone each line into a temporary vector, reverse it if necessary, then remove its first coordinate | Append coordinates through forward or reverse iterators and handle snapped endpoints directly |
| `crates/cli/src/read.rs:174` | Clone exterior and interior rings after already collecting owned rings | Consume the ring iterator into the polygon constructor |

These costs grow with geometry size. No speedup is claimed here: measure allocation and runtime changes with multipart and long-line inputs before accepting a refactor. Retain duplicate uses, merge direction, snapped endpoints and feature-pair aggregation behavior.

### 3. Weakly typed output attributes, medium priority

Implemented: private named attributes retain static rule/error names, optional layer strings and nullable signed `i64` FIDs. GDAL field order, string/integer types, null values and Shapefile aliases are unchanged. Integration tests read signed and nullable IDs back from output datasets.

Locations: `crates/cli/src/write.rs:82`, `:126` and `:298`.

Output attributes are represented as `[Option<String>; 6]`. FIDs are converted from `i64` to strings, then parsed back into `i64` when creating GDAL fields. Positions 3 and 5 carry special meaning in multiple places. Current generated values are valid, but this design adds allocations and makes mistakes easier during maintenance.

Suggested change: an internal struct with named fields, static rule and error names, optional layer names and `Option<i64>` FIDs. Retain the existing external schema, nullable values, signed IDs and Shapefile field aliases. This follows the type-safety guidance above and requires no public API change.

### 4. Public API documentation, medium priority

Implemented: exported core types, fields, variants and methods have item documentation, including deferred validation, scoped IDs, planar units, degeneracies, provenance and operational errors. Public fallible functions have `Errors` sections. Core `missing_docs` and strict rustdoc checks enforce the documentation contract.

Locations: `crates/core/src/model.rs`, `crates/core/src/topology.rs`, `crates/core/src/merge.rs`, `crates/core/src/rule.rs` and `crates/core/src/error.rs`.

The README provides a working example, but many exported methods and fields lack item-level documentation. The pedantic scan reports seven public fallible functions without an `Errors` section, including `Topology::new`, `layer`, `add_rule`, `warnings`, `validate`, `merge_lines` and `explode_lines`.

Suggested change: document deferred layer validation, topology-scoped IDs, coordinate units, skipped degeneracies, registration order and operational failures. State that a topology violation is a successful report, while invalid input may return an error. Describe feature and part provenance where downstream callers encounter it. Consider a core-only `missing_docs` lint once these contracts are documented, following Tokio's library practice.

### 5. String-only rule parsing errors, low priority

Implemented: exported `ParseRuleError` implements `Debug`, `Display` and `std::error::Error`, exposes the original unknown name through `name()`, and replaces `String` as `Rule::FromStr::Err`. The display message is unchanged and the CLI no longer needs its string-error adapter.

Location: `crates/core/src/rule.rs:46`.

`Rule::from_str` returns `String`. It carries a useful message, but `String` does not implement `std::error::Error`. The CLI consequently adapts it with `anyhow::Error::msg`, and library consumers must do something similar.

Suggested change: a small typed parsing error implementing `Display` and `Error`, retaining the unknown name. This is an API improvement, not a correctness fix. Changing the associated `FromStr::Err` type is a public API change and should be treated as such even during the alpha phase.

### 6. Selected lint policies and result annotations, low priority

Implemented: workspace lints cover missing debug implementations, unsafe operations in unsafe functions, undocumented unsafe blocks, missing error documentation, reference-counted clones and debugging macros. Both crates inherit them. Core forbids unsafe code; the documented GDAL FID call remains in the CLI. Pure scalar and name queries have deliberate `must_use` annotations.

Locations: root and crate `Cargo.toml` files, `crates/core/src/lib.rs` and public query methods.

CI already runs formatting and Clippy with warnings denied. The manifests do not share a project-specific lint policy. Consider narrowly selected lints for undocumented unsafe blocks, missing error documentation and debug implementations. A safe-core policy such as `forbid(unsafe_code)` could preserve the current separation between the geometry engine and GDAL FFI.

Some pure public queries, including `Report::is_valid`, `error_count` and name accessors, could use `#[must_use]`. Review annotations deliberately; blanket annotation of every constructor or accessor can add noise.

Clap demonstrates a curated workspace policy. [Clippy explicitly cautions that pedantic lints can produce intentional false positives and that restriction lints should be selected individually](https://doc.rust-lang.org/stable/clippy/usage.html). Enabling entire groups is not recommended by this review.

### 7. Small allocation and readability cleanups, low priority

Implemented: warnings and CRS comparison use direct fallible loops. CRS comparisons still reject mismatches among known layers when any other layer lacks CRS. Shared graph ownership uses explicit `Arc::clone`; poison recovery is documented without changing the cache structure.

`Topology::warnings` (`crates/core/src/topology.rs:112`) collects an intermediate vector before filtering it into warning strings. A direct fallible loop would express the work more simply. `common_crs` (`crates/cli/src/read.rs:225`) similarly collects borrowed CRS references merely to compare them; iteration can suffice.

The graph cache clones `Arc` values through `.clone()` (`crates/core/src/topology.rs:85`). Explicit `Arc::clone` can clarify that the operation shares ownership rather than copying a graph, but `.clone()` is valid and not deprecated. This is a readability preference, below the priority of actual geometry copies.

### 8. Dependency feature review, low priority

Implemented with authorization: Geo defaults are disabled, with `serde` and `multithreading` explicitly retained. Five packages leave the lockfile: `earcut`, `spade`, `allocator-api2`, `foldhash` and `hashbrown` 0.15.5. All remaining package versions are unchanged. The reduced Geo reexport is an alpha API change, documented below.

Location: root `Cargo.toml`, the `geo` dependency enables its defaults.

[Geo's manifest](https://github.com/georust/geo/blob/main/geo/Cargo.toml) includes optional default functionality such as triangulation. This engine does not call triangulation APIs. Inspect the resolved feature graph and evaluate disabling unused defaults to reduce build work. [Tokio's library guidance recommends enabling only required features](https://github.com/tokio-rs/tokio/blob/master/tokio/src/lib.rs).

Do not make this change automatically: the core publicly reexports `geo`, so removing dependency features can reduce APIs visible to downstream users. Benchmark build effects and review that public surface first. This review does not recommend upgrading dependency versions just because newer versions exist.

### 9. Named module files instead of `mod.rs`, low priority

Implemented: `engine/mod.rs` moved to `engine.rs` and `rules/mod.rs` moved to `rules.rs`. Child files and module paths remain unchanged.

Locations: `crates/core/src/engine/mod.rs` and `crates/core/src/rules/mod.rs`.

The [Rust Reference encourages the newer named-file convention](https://doc.rust-lang.org/reference/items/modules.html#module-source-filenames), which avoids many indistinguishable `mod.rs` filenames and makes module naming more consistent. A module can live in `engine.rs` while its child modules remain in the `engine/` directory. The [edition guide describes this layout](https://doc.rust-lang.org/edition-guide/rust-2018/path-changes.html).

Completed file moves:

| Current file | Suggested file |
| --- | --- |
| `crates/core/src/engine/mod.rs` | `crates/core/src/engine.rs` |
| `crates/core/src/rules/mod.rs` | `crates/core/src/rules.rs` |

Child files remain in their existing directories. Module declarations and ordinary import paths remain the same; the inspected files do not contain custom path attributes requiring adjustment. Both layouts are supported, and `mod.rs` is not deprecated. Established projects still use it, including [Tokio's sync module](https://github.com/tokio-rs/tokio/tree/master/tokio/src/sync). This is an encouraged naming convention and a navigation improvement, with no runtime or performance benefit.

## Patterns worth retaining

- Standard-library `OnceLock` is appropriate for caches whose initialization needs a topology or layer input. The [standard library distinguishes this use from simpler `LazyLock` initialization](https://doc.rust-lang.org/std/sync/struct.OnceLock.html). Caching `Result` also retains deterministic validation failures.
- `Arc`, a briefly held map mutex and separate lazy initialization make sense for graph caches shared by parallel rules. An async mutex or Tokio runtime would not improve this synchronous CPU-bound design. The poison-recovery policy could use a rationale comment, but no correctness defect was established in this review.
- The library's typed `thiserror` enum and the CLI's contextual `anyhow` errors serve different callers appropriately. Operational errors remain distinct from violations.
- `Euclidean.distance`, `Euclidean.length`, `unary_union` and bulk-loaded R-trees match the API direction documented by GeoRust. There is no deprecated geometry API migration identified here.
- `BTreeMap` and explicit sorting provide deterministic reports. Replacing them with unordered maps would require preserving that guarantee and measuring a benefit.
- Exact float comparisons are intentional for zero tolerance and signed-zero normalization. Pedantic float-comparison warnings are not a reason to introduce an epsilon.
- The isolated GDAL FID FFI call already has a safety explanation and preserves signed IDs that the wrapper's convenience method omits. Removing it solely to avoid an unsafe block would lose behavior.
- Iterator inputs, borrowed views, private caches and root-level reexports are appropriate API boundaries. Validated clap invariants also justify the existing `expect` calls; they are different from unwrapping fallible user input.

## Verification and limits

A read-only Clippy scan of `d98900f` with the pedantic group enabled completed successfully and produced additional diagnostics. Its useful API documentation findings are included above. Other diagnostics concern function length, numeric casts, literal formatting and stylistic choices, including tests and benchmarks; they do not establish runtime defects.

Implementation verification includes the existing acceptance, merge, property and CLI suites, plus focused parsing, multipart cleaning/provenance, geometry aggregation, reversed long-line merging, nullable signed output IDs, CRS and writer-failure coverage. Injected writers test immediate and partial writes and final flush failures for text and JSON. Unix subprocess tests close the output channel before execution for help, rule/driver listing, validation and both utilities. Utility datasets are reopened and checked against exact feature counts and coordinates, without requiring GEOS.

Final verification passed on 2026-10-10:

- `cargo fmt --all -- --check` and `git diff --check`.
- `cargo test --workspace --release --locked --offline`: 83 tests including one doctest, with 5 CLI unit tests and 22 CLI integration tests. All seven property tests run their existing 256 cases.
- `cargo clippy --workspace --all-targets --release --locked --offline -- -D warnings`.
- `RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps --locked --offline`.
- `cargo +1.88.0 check --workspace --all-targets --locked --offline`, with the default bundled GDAL configuration.
- Before/after Criterion, isolated allocation counts and both feature-only clean release builds described below.

The repository README now contains only installation instructions and CLI/API examples. Detailed contracts remain in core item documentation; migration notes and measurements remain in this document. All changes remain local.

The package version remains `0.1.0-alpha`. There are two alpha API migrations:

- `Rule::FromStr::Err` is now `ParseRuleError`. Callers expecting a `String` can call `to_string()`; callers using error propagation can use the standard error implementation directly. `name()` preserves the original unknown input.
- The `geo` reexport no longer enables triangulation. Consumers needing those APIs should add a direct dependency on the same Geo version with `earcut` and/or `spade` enabled. No compatibility feature is added. Serde, multithreading, rule semantics, CLI schemas, deterministic ordering and Rust 1.88 support are retained.

### Measurement method

`crates/core/benches/patterns.rs` uses identical before/after workloads from `benches/support/mod.rs`: 100 multipart features with ten 100-coordinate line parts each; two identical 100-part polygon features producing one aggregated overlap; and two 10,001-coordinate lines, with the second reversed. Criterion uses 30 samples, a one-second warmup and three-second measurement. Input cloning occurs in batch setup, outside the timed operation. Runs use `RAYON_NUM_THREADS=1` and Rust 1.95.0 on aarch64 macOS. The baseline uses the original source and dependency features; the final implementation uses trimmed features. These results apply to these workloads and this machine.

`crates/core/benches/allocations.rs` separately forwards every allocation operation to `System`, with atomic counters enabled only during each operation. Input construction and Rayon initialization are excluded. Counts include allocations, zeroed allocations and reallocations, and bytes mean cumulative requested bytes, not peak live memory. The allocator is confined to the benchmark binary and is absent from the safe core and Criterion timing runs.

### Timing measurements and regression investigation

The original 30-sample baseline and final 30-sample comparison produced these 95% timing intervals and Criterion change estimates:

| Workload | Before interval | After interval | Criterion estimated change | Classification |
| --- | --- | --- | ---: | --- |
| Multipart cleaning | 453.38 to 478.93 µs | 411.35 to 473.89 µs | -4.91% | Within noise threshold |
| Polygon aggregation | 120.79 to 123.69 µs | 115.49 to 117.28 µs | -3.89% | Improvement |
| Reversed long-line merge | 1.9383 to 1.9962 ms | 2.1793 to 2.5279 ms | +11.81% | Regression requiring investigation |

The long-line regression was investigated with a fresh matched pair using 60 samples and five-second measurements. That pair narrowed the estimate to +4.73%. A further back-to-back comparison invoked the already built baseline and final benchmark binaries directly, excluding compilation between timing runs. Its before interval was 1.9786 to 2.0573 ms, its after interval was 1.9192 to 1.9456 ms, and Criterion classified the -2.02% estimate as within its noise threshold. The slowdown did not reproduce in this direct comparison.

The 60-sample aggregation pair detected no change (+0.41%, p = 0.78), and multipart baseline timings varied substantially across runs. These observations limit stable runtime conclusions: allocation reductions are measured, but a general speedup is not established, and no reproducible long-line regression was established. The initial positive results and subsequent regression classifications are retained here rather than selecting only favorable runs.

Reproduce measurements by copying the same benchmark sources into the original source checkout, then using a shared absolute `CRITERION_HOME` directory for both runs:

```sh
RAYON_NUM_THREADS=1 CRITERION_HOME=/tmp/topology-criterion cargo bench -p topology-checker-core --bench patterns -- --save-baseline before
RAYON_NUM_THREADS=1 CRITERION_HOME=/tmp/topology-criterion cargo bench -p topology-checker-core --bench patterns -- --baseline before
RAYON_NUM_THREADS=1 cargo bench -p topology-checker-core --bench allocations
```

### Allocation measurements

| Workload | Before calls | After calls | Before requested bytes | After requested bytes |
| --- | ---: | ---: | ---: | ---: |
| Multipart cleaning | 2,930 | 1,929 | 3,716,957 | 2,116,893 |
| Polygon aggregation | 3,291 | 2,989 | 452,178 | 428,034 |
| Reversed long-line merge | 14,399 | 14,397 | 11,249,408 | 10,929,376 |

The first iterator-based merge draft grew its output buffer again when pushing the final endpoint. Reserving each complete append removes that unnecessary growth. Counts above include this adjustment.

### Feature-only clean release builds

Two separate fresh target directories were built offline from the same original source, using the same Rust 1.95.0 compiler and cached registry sources. The after checkout differs only in Geo features and the corresponding lockfile removals. Both commands were `cargo build --workspace --release --locked --offline`, measured with `/usr/bin/time -l`.

| Measurement | Geo defaults | Trimmed Geo features |
| --- | ---: | ---: |
| Wall time | 221.49 s | 237.70 s |
| User CPU time | 911.56 s | 1,033.32 s |
| System CPU time | 146.24 s | 162.93 s |
| Compiling-package messages | 104 | 99 |

The resolved graph falls from 206 to 201 packages, with no remaining package version changes. Full builds still compile bundled GDAL and PROJ; this single pair of timings shows no build-speed improvement and cannot establish a statistically supported build-time regression. Brief overlapping verification compilations and host load also limit the timing comparison. The supported dependency result is five fewer packages, including both triangulation implementations.
