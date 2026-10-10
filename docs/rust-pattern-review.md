# Rust pattern review

Reviewed on 2026-10-10 against commit `d98900f`. This document records findings and recommendations only. No implementation, test, dependency or CI changes are included.

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

## Recommended improvements

### 1. Fallible stdout handling, medium priority

Locations: `crates/cli/src/main.rs:62`, `crates/cli/src/main.rs:73`, `crates/cli/src/report.rs:6`, and utility output in `crates/cli/src/main.rs:274`.

The CLI uses `println!` throughout. Those calls bypass its `anyhow::Result` error path and can panic if stdout is closed by a downstream command. This is a real behavior issue for shell pipelines, not an obsolete syntax issue: [the standard library documents output failures as a panic condition](https://doc.rust-lang.org/std/macro.println.html).

Suggested change: lock stdout, pass a writer into output routines, use fallible writes and choose an explicit broken-pipe policy. Ripgrep treats a broken pipe as graceful termination. Other output failures should still produce the CLI's operational failure status. Preserve exit status 1 for fully emitted reports containing topology violations.

JSON output currently allocates a complete serialized string before printing. Writing JSON directly to the output writer would remove that extra string allocation. If this is implemented, test both text and JSON closed-pipe behavior: a serializer may wrap I/O errors, so handling only a directly downcastable `std::io::Error` is insufficient without checking the serializer's error API.

### 2. Avoidable geometry copies, medium priority

The current ownership boundaries are sound, but several functions copy data they could borrow or move:

| Location | Current behavior | Suggested approach |
| --- | --- | --- |
| `crates/core/src/model.rs:227` and `:232` | Clone each multipart line or polygon into a temporary `Geometry`, then copy coordinates again while cleaning it | Dispatch borrowed parts to typed helpers and allocate only the cleaned cached geometry |
| `crates/core/src/topology.rs:176` | Clone the first error, borrow an owned group and clone line or polygon payloads into the aggregate | Consume the group and move its geometry payloads, retaining provenance aggregation and ordering |
| `crates/core/src/merge.rs:96` | Clone each line into a temporary vector, reverse it if necessary, then remove its first coordinate | Append coordinates through forward or reverse iterators and handle snapped endpoints directly |
| `crates/cli/src/read.rs:174` | Clone exterior and interior rings after already collecting owned rings | Consume the ring iterator into the polygon constructor |

These costs grow with geometry size. No speedup is claimed here: measure allocation and runtime changes with multipart and long-line inputs before accepting a refactor. Retain duplicate uses, merge direction, snapped endpoints and feature-pair aggregation behavior.

### 3. Weakly typed output attributes, medium priority

Locations: `crates/cli/src/write.rs:82`, `:126` and `:298`.

Output attributes are represented as `[Option<String>; 6]`. FIDs are converted from `i64` to strings, then parsed back into `i64` when creating GDAL fields. Positions 3 and 5 carry special meaning in multiple places. Current generated values are valid, but this design adds allocations and makes mistakes easier during maintenance.

Suggested change: an internal struct with named fields, static rule and error names, optional layer names and `Option<i64>` FIDs. Retain the existing external schema, nullable values, signed IDs and Shapefile field aliases. This follows the type-safety guidance above and requires no public API change.

### 4. Public API documentation, medium priority

Locations: `crates/core/src/model.rs`, `crates/core/src/topology.rs`, `crates/core/src/merge.rs`, `crates/core/src/rule.rs` and `crates/core/src/error.rs`.

The README provides a working example, but many exported methods and fields lack item-level documentation. The pedantic scan reports seven public fallible functions without an `Errors` section, including `Topology::new`, `layer`, `add_rule`, `warnings`, `validate`, `merge_lines` and `explode_lines`.

Suggested change: document deferred layer validation, topology-scoped IDs, coordinate units, skipped degeneracies, registration order and operational failures. State that a topology violation is a successful report, while invalid input may return an error. Describe feature and part provenance where downstream callers encounter it. Consider a core-only `missing_docs` lint once these contracts are documented, following Tokio's library practice.

### 5. String-only rule parsing errors, low priority

Location: `crates/core/src/rule.rs:46`.

`Rule::from_str` returns `String`. It carries a useful message, but `String` does not implement `std::error::Error`. The CLI consequently adapts it with `anyhow::Error::msg`, and library consumers must do something similar.

Suggested change: a small typed parsing error implementing `Display` and `Error`, retaining the unknown name. This is an API improvement, not a correctness fix. Changing the associated `FromStr::Err` type is a public API change and should be treated as such even during the alpha phase.

### 6. Selected lint policies and result annotations, low priority

Locations: root and crate `Cargo.toml` files, `crates/core/src/lib.rs` and public query methods.

CI already runs formatting and Clippy with warnings denied. The manifests do not share a project-specific lint policy. Consider narrowly selected lints for undocumented unsafe blocks, missing error documentation and debug implementations. A safe-core policy such as `forbid(unsafe_code)` could preserve the current separation between the geometry engine and GDAL FFI.

Some pure public queries, including `Report::is_valid`, `error_count` and name accessors, could use `#[must_use]`. Review annotations deliberately; blanket annotation of every constructor or accessor can add noise.

Clap demonstrates a curated workspace policy. [Clippy explicitly cautions that pedantic lints can produce intentional false positives and that restriction lints should be selected individually](https://doc.rust-lang.org/stable/clippy/usage.html). Enabling entire groups is not recommended by this review.

### 7. Small allocation and readability cleanups, low priority

`Topology::warnings` (`crates/core/src/topology.rs:112`) collects an intermediate vector before filtering it into warning strings. A direct fallible loop would express the work more simply. `common_crs` (`crates/cli/src/read.rs:225`) similarly collects borrowed CRS references merely to compare them; iteration can suffice.

The graph cache clones `Arc` values through `.clone()` (`crates/core/src/topology.rs:85`). Explicit `Arc::clone` can clarify that the operation shares ownership rather than copying a graph, but `.clone()` is valid and not deprecated. This is a readability preference, below the priority of actual geometry copies.

### 8. Dependency feature review, low priority

Location: root `Cargo.toml`, the `geo` dependency enables its defaults.

[Geo's manifest](https://github.com/georust/geo/blob/main/geo/Cargo.toml) includes optional default functionality such as triangulation. This engine does not call triangulation APIs. Inspect the resolved feature graph and evaluate disabling unused defaults to reduce build work. [Tokio's library guidance recommends enabling only required features](https://github.com/tokio-rs/tokio/blob/master/tokio/src/lib.rs).

Do not make this change automatically: the core publicly reexports `geo`, so removing dependency features can reduce APIs visible to downstream users. Benchmark build effects and review that public surface first. This review does not recommend upgrading dependency versions just because newer versions exist.

### 9. Named module files instead of `mod.rs`, low priority

Locations: `crates/core/src/engine/mod.rs` and `crates/core/src/rules/mod.rs`.

The [Rust Reference encourages the newer named-file convention](https://doc.rust-lang.org/reference/items/modules.html#module-source-filenames), which avoids many indistinguishable `mod.rs` filenames and makes module naming more consistent. A module can live in `engine.rs` while its child modules remain in the `engine/` directory. The [edition guide describes this layout](https://doc.rust-lang.org/edition-guide/rust-2018/path-changes.html).

Suggested file moves, if a source cleanup is authorized later:

| Current file | Suggested file |
| --- | --- |
| `crates/core/src/engine/mod.rs` | `crates/core/src/engine.rs` |
| `crates/core/src/rules/mod.rs` | `crates/core/src/rules.rs` |

Keep the child files in their existing directories. Module declarations and ordinary import paths would remain the same; the inspected files do not contain custom path attributes requiring adjustment. Both layouts are supported, and `mod.rs` is not deprecated. Established projects still use it, including [Tokio's sync module](https://github.com/tokio-rs/tokio/tree/master/tokio/src/sync). This is an encouraged naming convention and a navigation improvement, with no runtime or performance benefit. No files were moved for this review.

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

After reverting temporary implementation edits, the tracked source matched `d98900f`. The workspace release build, formatting check and standard Clippy check with warnings denied passed on that restored source. The final repository change is this document only.

Performance recommendations are source-level observations, not measured improvements. The broken-pipe finding follows the output calls and documented standard-library behavior. No new baseline runtime regression tests were added. Recommendations preserve the Rust 1.88 minimum version unless explicitly reviewed otherwise.
