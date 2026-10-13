These immutable geometry fixtures were copied from the sibling topology-checker-test campaign of 2026-10-11. geometries.json contains only geometry constants extracted from the minimized reproducers, without their bug-present assertions. The original campaign remains unchanged. Natural Earth supplied BUG-018; BUG-019 contains Boston Census and Washington DC boundaries; BUG-021 contains IGN Argentina pipeline geometry. See the original campaign DATASETS.md for provenance and licenses.

`boundary_run_crossings.json` preserves Seattle TIGER roads 498 and 1401 and
Census block groups 216 and 329 in EPSG:32610 (US government public domain).
Each connected road has more than 50 m strictly inside and 200 m strictly outside
the polygon's 1 cm uncertainty band, independently checked by GEOS buffering.
Their intervening boundary runs therefore connect opposite sides and cross.


Frozen geometry inputs from the external fix-check campaign (12 October 2026).

`dc_precinct_smd.json` is the original UTM 18N DC voting precinct 2019 FID 1
and single-member district 2023 FID 252. The campaign supplied the WKT reproducer.
Both inputs are valid; their independent GEOS intersection area is
371430.874 square metres, with the reproducer's relative acceptance bound of 0.0001.

`chicago_boundary.json` preserves Chicago transportation FID 31294 and community
area FID 57. `tfl_self_crossing.json` preserves TfL cycle lane FID 23456.
These are copied from the campaign's prepared WKB caches, with their original
source, CRS and provider metadata. No campaign files are modified or imported.
The Chicago excursion is within 1 mm of a sustained boundary interval. TfL's
first and third segments intersect more than 1 mm from both open endpoints.

REG-001's synthetic bent parallel copies have three or five vertices. Their
shared length is exactly twice sqrt(26), and the polygon's other sides total 20.
REG-003 uses a 200000-vertex disc and 400 square islands in the unoccupied corner
of its bounding box. The isolated benchmark enforces the two-second ingestion
gate; structural native tests enforce local scans without CI timing assumptions.

`ons_degree_two.json`, `ons_adjacent_nodes.json`, `ons_source_contacts.json`, and `ons_gaps.json` retain subsequences of original ONS boundary vertices. They cover ordinary bends, source ordering between adjacent floats, narrow incidence sectors requiring up to 64 floating-point steps, and gap articulation. `ons_overlap.json` preserves the original pair. `brp_contacts.json` retains original BRP vertices for three polygons meeting at contact nodes. Each file records source layers and FIDs. Valid original polygons have substantial intersections; the independent Fraction boundary reference establishes gap counts before output conversion.
