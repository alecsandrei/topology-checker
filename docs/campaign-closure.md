# Campaign closure, 2026-10-12

All 21 correctness findings have tests against a freshly built
release extension. The four performance findings pass the representative gates
below. The sibling `topology-checker-test` campaign was read only. Its minimized
geometry was copied into `tests/fixtures/campaign`; its bug-present assertions
were not used as acceptance tests.

## Finding matrix

Test names in this table belong to `tests/test_campaign.py` unless another file
is named. Every row passed on the rebuilt release artifact.

| Finding | Implemented behavior | Affirmative evidence | Status |
| :--- | :--- | :--- | :--- |
| BUG-001 | Explicit shared boundary intervals do not become crossings. | `test_bug_001_boundary_runs_do_not_cross`, at zero and positive tolerance | Closed |
| BUG-002 | Line and polygon overlap emits the original shared edge. | `test_bug_002_003_shared_edge_is_line_geometry`, geometry, validity and provenance | Closed |
| BUG-003 | Line and polygon intersection emits the shared edge as a line. | Same test, both rule variants and three tolerances | Closed |
| BUG-004 | Polygon coverage uses local original coordinates and exact interval classification. | `test_bug_004_boundary_coverage_survives_distant_features` | Closed |
| BUG-005 | Self-intersecting polygon rings are rejected after cleanup. | `test_bug_005_007_020_invalid_polygons_rejected_atomically`, layer/FID/part/ring diagnostics and rollback | Closed |
| BUG-006 | Closed boundaries have no endpoint at their chosen start vertex. | `test_bug_006_ring_rotation_has_no_endpoint`, all four rotations | Closed |
| BUG-007 | Invalid nested or overlapping MultiPolygon parts are rejected. | Atomic rejection test for both original fixtures | Closed |
| BUG-008 | Boundary intervals fail properly-inside checks. | `test_bug_008_boundary_is_never_properly_inside`, three tolerances | Closed |
| BUG-009 | Self-rules operate only on the original feature's parts. | `test_bug_009_self_rules_ignore_other_features`, both self-rules | Closed |
| BUG-010 | Original multipart operands aggregate before exact intersection geometry is rounded. | `test_bug_010_valid_overlap_from_valid_multipart_inputs`, valid geometry and Shapely area | Closed |
| BUG-011 | Each tolerance-collapsed line part emits one typed native warning. | `test_bug_011_collapsed_parts_warn_once`; original-part and streaming-warning checks in `test_metamorphic.py` | Closed |
| BUG-012 | Open endpoint crossings within tolerance are exempt; closed parts have no exemption. | `test_bug_012_open_endpoint_crossing_exemption`; `test_closed_line_boundary_crossings_have_no_endpoint_exemption`, all ring rotations | Closed |
| BUG-013 | Python CRS equality ignores axis order. | `test_bug_013_crs_axis_order_equivalence`, EPSG:4326/CRS84 accepted and EPSG:3857 rejected | Closed |
| BUG-014 | Overlay components preserve local gap topology in the presence of distant features. | `test_bug_014_gap_components_preserve_local_topology` | Closed |
| BUG-015 | Overlap requires compatible original source intervals, excluding junction connectors. | `test_bug_015_junction_connectors_are_not_overlap`, four contact distances and three rules | Closed |
| BUG-016 | Canonical complete-link clustering is independent of feature order and FIDs. | `test_bug_016_feature_order_and_fid_independence`; exact binary transforms in `test_metamorphic.py` | Closed |
| BUG-017 | Line reversal preserves contact and overlap classification. | `test_bug_017_line_reversal`, all four affected rule variants | Closed |
| BUG-018 | Exact component overlay has finite face and boundary traversal. | `test_bug_018_release_gaps_finishes_correctly`, 236 valid original parts, isolated process finished in 0.40 s, 10-second timeout, Shapely shell and filtered-hole references | Closed |
| BUG-019 | Zero tolerance retains positive-area overlap decisions. | `test_bug_019_zero_tolerance_positive_area_decisions`, original real-world pairs; the exact-area triangle and every ring rotation/reversal in `test_polygon_precision.py` | Closed |
| BUG-020 | Closing spikes in either winding direction are rejected. | Atomic rejection test for both original spikes; original-ring diagnostics after cleanup | Closed |
| BUG-021 | Collapsed parts cannot acquire graph spikes or overlap geometry. | `test_bug_021_collapsed_lines_cannot_grow_spikes`, self-rules, pair overlap and boundary coverage | Closed |
| PERF-1 | Shared coordinates, lazy indexes, sparse source cuts and compact graph incidences reduce retained and construction memory. | Same-machine line and large-polygon workloads, 50.8% and 83.2% lower peak RSS | Closed on measured workloads |
| PERF-2 | Rust sinks, bounded Python streams, binary transport and disk-backed exports preserve every pair. | 3,123,750 and 12,497,500 streamed pairs; eager equivalence for all 38 variants; cancellation, consumer failure, pagination and export tests | Closed |
| PERF-3 | Cached boundary and point-location indexes restrict repeated queries to local segments. | 2,000 queries against a 200,000-vertex polygon: 28.087 s to 0.097 s, 288.5 times faster | Closed on measured workload |
| PERF-4 | Gap extraction uses finite component overlay and releases scratch storage. | Original valid subset and independently repaired time-zone fixture both finish correctly under the time and memory caps | Closed at zero and campaign tolerance |

## Performance evidence

Raw measurements are in [campaign-evidence.json](campaign-evidence.json).
Zero-tolerance line construction releases its coordinate hash before edge
allocation and compacts edge groups in place. Both isolated runs pass the memory
gate; their validation times (2.048 s and 1.983 s) exceed the 1.680 s baseline.
Each workload ran in its own process on the same macOS 26.7.1 arm64 machine,
using Python 3.13.13, NumPy 2.5.4, Shapely 2.2.0 and GEOS 3.14.1. Fixed artifacts
were rebuilt offline with Rust 1.95.0 in release mode. RSS is whole-process peak,
including Python input conversion and output decoding. Time includes native
validation and eager decoding, or full stream consumption. Baselines use the
campaign's existing installed extension, not a freshly rebuilt old revision.

| Workload | Baseline validation | Release validation | Baseline peak RSS | Release peak RSS | Decision |
| :--- | ---: | ---: | ---: | ---: | :--- |
| 50,000 separated lines, 20 vertices each, 100,000 dangles | 1.680 s | 2.048 s | 1,057,406,976 B | 519,995,392 B | 50.8% memory reduction, passes 50% gate in two isolated runs |
| 2,000 interior point queries, one 200,000-vertex polygon, cold validation | 28.087 s | 0.097 s | 1,162,477,568 B | 194,822,144 B | 288.5 times faster; 83.2% memory reduction |
| 2,500 coincident points, batch size 128 | Eager baseline unsuitable | 7.802 s | Not run | 58,884,096 B | All 3,123,750 pairs emitted |
| 5,000 coincident points, batch size 128 | Eager baseline unsuitable | 32.552 s | Not run | 62,537,728 B | All 12,497,500 pairs emitted; four times the pairs adds 6.2% RSS |
| Time-zone gaps, 428 valid original features, tolerance 1 | Campaign exceeded 900 s | 19.175 s | Campaign remained under 40 GB without a result | 2,357,411,840 B | 383 shells and 27 retained holes; exact-reference count, validity and stream equivalence pass |
| Time-zone gaps, all 444 repaired features, tolerance 1 | Campaign aborted under a 40 GB cap | 12.234 s | No successful baseline | 2,360,885,248 B | 20 shells and 9 retained holes; exact-reference count, validity and stream equivalence pass |
| Valid time-zone fixture, tolerance 0 | Not rerun | 17.902 s | Not run | 2,250,604,544 B | 383 shells and 43 independently established positive-area holes |
| Repaired time-zone fixture, tolerance 0 | Not rerun | 12.309 s | Not run | 2,153,316,352 B | 20 shells and 139 independently established positive-area holes |

Gap runs enforce a 900-second alarm and a 32 GiB address-space limit where
supported. Darwin does not provide a usable address-space limit, so a monitor
checks whole-process peak RSS every half second and exits above 32 GiB. Both
successful runs stay far below that cap. Preparation, GEOS diagnostics, repeated eager validation and full stream
comparisons bring campaign-tolerance process times to 56.702 s and 33.056 s
respectively. Zero-tolerance total times are 52.669 s and 33.675 s.
All benchmark assertions must pass before a measurement is emitted.

Reproduce the release measurements from the repository root:

```sh
UV_CACHE_DIR=/tmp/topology-uv-cache uv run --no-sync python scripts/campaign_benchmark.py lines --size 50000
UV_CACHE_DIR=/tmp/topology-uv-cache uv run --no-sync python scripts/campaign_benchmark.py points --size 200000 --queries 2000
UV_CACHE_DIR=/tmp/topology-uv-cache uv run --no-sync python scripts/campaign_benchmark.py duplicates --size 2500
UV_CACHE_DIR=/tmp/topology-uv-cache uv run --no-sync python scripts/campaign_benchmark.py duplicates --size 5000
UV_CACHE_DIR=/tmp/topology-uv-cache uv run --no-sync python scripts/campaign_benchmark.py gaps --equivalence
UV_CACHE_DIR=/tmp/topology-uv-cache uv run --no-sync python scripts/campaign_benchmark.py gaps --repair --equivalence
```

For baseline line and point measurements, set `PYTHONDONTWRITEBYTECODE=1` and
set `PYTHONPATH` to the sibling campaign's installed package directory. The gap
script reads prepared WKB directly and never imports campaign cache writers.

## Verification

- Rust workspace tests: 189 passed. The normally ignored 200,000-feature web
  responsiveness and stale-job exercise was run explicitly and passed in 2.70 s.
- Clippy for all workspace targets with warnings denied, and Rust formatting:
  passed.
- Python suite: 427 tests, including all original campaign tests,
  deterministic streams, early close, consumer failure and snapshot ownership.
- The boundary-edge property test checks 2,000 triangles at tolerance zero and
  0.001, across eight rules, including geometry, validity and provenance.
- Metamorphic checks cover feature permutations, signed FID remapping, line
  reversal, ring rotation, translation, binary scaling, distant features, and
  unary versus binary line-rule equivalence.
  A signed-zero test also checks polygon union and intersection winding.
  A crossing between adjacent floating-point coordinates verifies exact interval
  midpoints at zero and positive tolerance against Shapely.
- Ruff lint and formatting, and strict Pyright: passed.
- Frontend type checking, 19 unit tests, formatting and production build:
  passed. Browser tests against the rebuilt release backend: 10 passed.
- Disk-report exports match eager exports for every default native format and
  both dataset layouts. Web tests cover stable IDs, pagination, stale jobs,
  project revisions and export/import round-trips.
- Cancellation of a worker waiting for another worker's cache initialization
  completes without waiting for, cancelling, or discarding that initializer.
- macOS arm64 ABI3 release wheel installed into a separate environment without
  GeoPandas: eager and streaming checks passed. Source distribution and wheel
  metadata, native binary and required source checks passed. A wheel built from
  the extracted source archive also passed the regular-install smoke checks.
- Original bundled example sources and derived assets passed the generator's
  hash check. Invalid original Natural Earth polygons now fail explicitly under
  the selected input policy; source geometry remains intact.

## Practical limits

Performance results cover these representative workloads on this machine;
they do not establish timing or memory guarantees for every national dataset.
Duplicate output remains quadratic in count and time, while streaming memory
depends on inputs and batch size.

The exact gap reference independently establishes 43 positive-area holes for
valid inputs and 139 for repaired inputs at tolerance zero. At campaign tolerance
1, it retains 27 and 9 holes respectively. Every campaign run checks counts,
valid ring geometry, provenance, repeated output and eager/streaming equivalence.

The full repaired audit is in [polygon-precision-audit.json](polygon-precision-audit.json).
It records exact rational areas, boundary nodes, original source intervals,
layer FIDs and original part paths for all 139 holes. The valid-input summary is
in [polygon-precision-valid-audit.json](polygon-precision-valid-audit.json).
The Python reference cancels exactly coincident opposite source intervals,
computes Fraction intersections, and classifies infinitesimal sides against
original input polygons before reconstructing its empty-side graph. GEOS supplies
parsing and bounding-box indexes; its overlay is used only after the reference
count is complete, to audit diagnostic rings.

GEOS emits 106 repaired-fixture interior ring objects. Of these, 105 have positive
floating-point area; all 106 have positive exact area when their emitted binary
coordinates are interpreted as a stand-alone ring. The zero-floating-area ring
has exact area `1434079929761945 / 36893488147419103232`. Original source polygon
1337 (layer FID 433, original part path `[1]`) covers both infinitesimal sides of
every edge, so that diagnostic ring does not bound an exact empty region. A
stand-alone polygon made from those same binary vertices still has positive
area, and its overlap regression must survive every rotation and reversal.
The audit records source-side evidence for every GEOS ring, including this one.

Output conversion fixes original vertices and shares one coordinate for every
exact intersection node, including exactly representable intersections that are
not original vertices. Nearest rounding is verified first. Independent affected
neighborhoods search within 1, 2, 4, 8, 16, 32 and 64 floating-point steps, with at most
100,000 candidate assignments each and cooperative cancellation. Distinct nodes,
source ordering, orientation, hole ownership, intended contacts and valid,
non-overlapping parts are constraints. An unsatisfied constraint or exhausted
budget raises Rust `Error::Precision` or Python `PrecisionError` (`RuntimeError`).
Adjacent polygon collections also retain their original geometry for decisions
and emit exact feature unions for point-count and containment violations. Those
emissions propagate precision failures with rule, layer and FID context.
The campaign fixtures complete successfully; the precision-error test
covers a separate unrepresentable fixture. Failed streams have no final summary,
and failed exports and web jobs publish no partial results.

Optional system-GDAL builds and wheels for other platforms were not rebuilt in
this environment. Default pure Rust I/O, the macOS arm64 wheel, and the frontend
were rebuilt and checked. No TeX compilation was run.


## Follow-up closure, 2026-10-12

Nearby original line intervals now cover interior vertices and unequal segmentation at positive tolerance. Zero tolerance requires exact shared intervals. The same interval evidence drives line overlap, intersection and both boundary-coverage directions. Chicago boundary proximity remains contact; genuine TfL self-crossings survive tolerance clustering. Tests are integrated into `test_metamorphic.py` and the native cross-geometry suite, including original source fixtures, transformations, provenance and eager/streaming equivalence.

Constrained polygon emission preserves original vertices, exact node identities, source order, ring orientation, contact incidence and hole ownership. Ordinary bends use non-overlap constraints; deterministic domain pruning and forward checking precede bounded search. The search radius reaches 64 adjacent floating-point steps with the unchanged 100,000-assignment limit and cancellation. Original DC, ONS and BRP pairs, including minimized distinct contact failures, emit valid geometry. DC overlap agrees with the independent reference at tolerances 0, 0.001 and 1. Genuine precision failures retain their typed eager, stream and export behavior.

Multipart ingestion prepares source segment and point-location indexes once. Structural checks verify local scans for islands and representative containment checks for long hole rings. The original 200,000-vertex disc/island workload completes in 0.456 seconds, within its two-second gate. Polygon validation uses certified interval decisions, exact fallback when needed, cached rational rounding and canonical rational hashes. Original contribution and containment evidence replaces repeated overlays; rounded output never determines subsequent source decisions.

Three paired isolated baseline/release processes per dataset measure ingestion separately from validation. All selected validation gates pass: BGT overlap has medians 4.156 and 6.039 seconds (1.453 times baseline), CBS gaps 2.474 and 3.983 seconds (1.610 times), and Vienna overlap 8.463 and 2.344 seconds (0.277 times). The limit is twice the paired baseline median. The initial CBS gate miss is retained in machine-readable evidence alongside the corrected measurements.

The full ONS overlap workload completes in 107.7 seconds with 2,203 valid outputs and repeated/eager/streaming equivalence. Its process peaks at 1.72 GB. Additional follow-up acceptance runs and artifact identifiers are recorded in `campaign-evidence.json`.

The follow-up Rust workspace suite passes 202 checks. The rebuilt release wheel passes all 492 Python checks, Clippy, Ruff and strict Pyright. Frontend checks pass 19 unit and 10 browser tests. Regular wheel installation and a wheel rebuilt from the source archive pass without GeoPandas. No TeX compilation is performed, and the sibling campaign remains read only.


The ONS gap workload completes in 13.904 seconds with 5,174 valid rings; BRP gaps in 1.571 seconds with 5,923 rings. Both pass repeated/eager/streaming equivalence. Valid time-zone inputs retain 383 shells and 43 holes at zero tolerance, and 27 holes at tolerance 1. Repaired inputs retain 20 shells and 139 or 9 holes respectively. All four time-zone jobs finish in under 36 seconds total and peak below 2.7 GB, within the 15-minute and 32-GiB caps.

The 50,000-line workload peaks at 479,576,064 bytes, a 54.6% reduction from the established baseline. The polygon-query workload completes in 0.089 seconds against 28.087 seconds previously. Duplicate streaming returns every one of 3,123,750 and 12,497,500 pairs, with process peaks of 59.8 and 63.5 MB respectively.
