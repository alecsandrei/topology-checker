# Native Linux GIS validation benchmarks

[Raw measurements](results.json) · [Samples CSV](samples.csv) · [SVG figure](comparison.svg)

![Six checks](comparison.png)

Times include fresh index or topology construction and materialized violations. Application startup, input loading, fresh copies and response extraction are excluded. PostGIS is measured as SQL workflows; GRASS module launch and native output writing are included. Comparisons require passing probes and matching workload provenance.

| Rule | Features | Engine | Median ms | Q1 ms | Q3 ms | Samples | Tool / ours | Status |
| :--- | ---: | :--- | ---: | ---: | ---: | ---: | ---: | :--- |
| `point/must-be-inside` | 1,000 | topology-checker | 1.923 | 1.870 | 2.002 | 10 | N/A | baseline |
| `point/must-be-inside` | 1,000 | QGIS | 17.180 | 16.994 | 17.828 | 10 | 8.93× | paired |
| `point/must-be-inside` | 1,000 | GRASS GIS | 185.258 | 182.856 | 189.732 | 10 | 96.33× | paired |
| `point/must-be-inside` | 1,000 | PostGIS SQL | 18.498 | 18.437 | 19.036 | 10 | 9.62× | paired |
| `point/must-be-inside` | 1,000 | gvSIG | 6894.545 | 6772.612 | 7009.839 | 10 | 3585.09× | paired |
| `polygon/must-contain-point` | 1,000 | topology-checker | 3.054 | 2.913 | 3.288 | 10 | N/A | baseline |
| `polygon/must-contain-point` | 1,000 | QGIS | 11.873 | 11.716 | 12.092 | 10 | 3.89× | paired |
| `polygon/must-contain-point` | 1,000 | GRASS GIS | 190.158 | 185.613 | 197.694 | 10 | 62.26× | paired |
| `polygon/must-contain-point` | 1,000 | PostGIS SQL | 16.710 | 16.574 | 17.387 | 10 | 5.47× | paired |
| `polygon/must-contain-point` | 1,000 | gvSIG | 6820.104 | 6780.933 | 7005.937 | 10 | 2233.00× | paired |
| `point/must-not-overlap` | 1,000 | topology-checker | 0.901 | 0.883 | 0.912 | 10 | N/A | baseline |
| `point/must-not-overlap` | 1,000 | QGIS | 20.074 | 15.788 | 35.023 | 10 | 22.28× | paired |
| `point/must-not-overlap` | 1,000 | GRASS GIS | 353.172 | 347.180 | 947.434 | 10 | 392.03× | paired |
| `point/must-not-overlap` | 1,000 | PostGIS SQL | 11.513 | 10.965 | 22.249 | 10 | 12.78× | paired |
| `point/must-not-overlap` | 1,000 | gvSIG | 14.758 | 8.714 | 20.696 | 10 | 16.38× | paired |
| `line/must-not-have-dangles` | 1,000 | topology-checker | 6.851 | 5.511 | 7.571 | 10 | N/A | baseline |
| `line/must-not-have-dangles` | 1,000 | QGIS | 20.911 | 14.743 | 34.399 | 10 | 3.05× | paired |
| `line/must-not-have-dangles` | 1,000 | GRASS GIS | 187.080 | 178.168 | 340.705 | 10 | 27.31× | paired |
| `line/must-not-have-dangles` | 1,000 | PostGIS SQL | 176.233 | 173.841 | 319.113 | 10 | 25.72× | paired |
| `line/must-not-have-dangles` | 1,000 | gvSIG | 15.888 | 11.391 | 22.834 | 10 | 2.32× | paired |
| `polygon/must-not-overlap` | 1,000 | topology-checker | 1.936 | 1.858 | 4.124 | 10 | N/A | baseline |
| `polygon/must-not-overlap` | 1,000 | QGIS | 12.788 | 12.686 | 14.087 | 10 | 6.60× | paired |
| `polygon/must-not-overlap` | 1,000 | GRASS GIS | 991.479 | 989.838 | 999.759 | 10 | 512.00× | paired |
| `polygon/must-not-overlap` | 1,000 | PostGIS SQL | 13.205 | 12.976 | 13.308 | 10 | 6.82× | paired |
| `polygon/must-not-overlap` | 1,000 | gvSIG | 14.021 | 9.345 | 17.864 | 10 | 7.24× | paired |
| `polygon/must-not-have-gaps` | 1,000 | topology-checker | 24.215 | 16.899 | 34.813 | 10 | N/A | baseline |
| `polygon/must-not-have-gaps` | 1,000 | QGIS | 98.238 | 96.484 | 121.046 | 10 | 4.06× | paired |
| `polygon/must-not-have-gaps` | 1,000 | GRASS GIS | 315.438 | 311.980 | 321.532 | 10 | 13.03× | paired |
| `polygon/must-not-have-gaps` | 1,000 | PostGIS SQL | 197.689 | 196.859 | 198.241 | 10 | 8.16× | paired |
| `polygon/must-not-have-gaps` | 1,000 | gvSIG | 217.379 | 204.000 | 243.655 | 10 | 8.98× | paired |

## Correctness probes

| Rule | Probe | Engine | Result |
| :--- | :--- | :--- | :--- |
| `point/must-be-inside` | clean | topology-checker | ok |
| `point/must-be-inside` | clean | QGIS | ok |
| `point/must-be-inside` | clean | GRASS GIS | ok |
| `point/must-be-inside` | clean | PostGIS SQL | ok |
| `point/must-be-inside` | clean | gvSIG | ok |
| `point/must-be-inside` | violation | topology-checker | ok |
| `point/must-be-inside` | violation | QGIS | ok |
| `point/must-be-inside` | violation | GRASS GIS | ok |
| `point/must-be-inside` | violation | PostGIS SQL | ok |
| `point/must-be-inside` | violation | gvSIG | ok |
| `point/must-be-inside` | point on polygon boundary | topology-checker | ok |
| `point/must-be-inside` | point on polygon boundary | QGIS | ok |
| `point/must-be-inside` | point on polygon boundary | GRASS GIS | ok |
| `point/must-be-inside` | point on polygon boundary | PostGIS SQL | ok |
| `point/must-be-inside` | point on polygon boundary | gvSIG | ok |
| `point/must-be-inside` | point in hole | topology-checker | ok |
| `point/must-be-inside` | point in hole | QGIS | ok |
| `point/must-be-inside` | point in hole | GRASS GIS | ok |
| `point/must-be-inside` | point in hole | PostGIS SQL | ok |
| `point/must-be-inside` | point in hole | gvSIG | ok |
| `point/must-be-inside` | point on hole boundary | topology-checker | ok |
| `point/must-be-inside` | point on hole boundary | QGIS | ok |
| `point/must-be-inside` | point on hole boundary | GRASS GIS | ok |
| `point/must-be-inside` | point on hole boundary | PostGIS SQL | ok |
| `point/must-be-inside` | point on hole boundary | gvSIG | ok |
| `polygon/must-contain-point` | clean | topology-checker | ok |
| `polygon/must-contain-point` | clean | QGIS | ok |
| `polygon/must-contain-point` | clean | GRASS GIS | ok |
| `polygon/must-contain-point` | clean | PostGIS SQL | ok |
| `polygon/must-contain-point` | clean | gvSIG | ok |
| `polygon/must-contain-point` | violation | topology-checker | ok |
| `polygon/must-contain-point` | violation | QGIS | ok |
| `polygon/must-contain-point` | violation | GRASS GIS | ok |
| `polygon/must-contain-point` | violation | PostGIS SQL | ok |
| `polygon/must-contain-point` | violation | gvSIG | ok |
| `polygon/must-contain-point` | boundary point | topology-checker | ok |
| `polygon/must-contain-point` | boundary point | QGIS | ok |
| `polygon/must-contain-point` | boundary point | GRASS GIS | ok |
| `polygon/must-contain-point` | boundary point | PostGIS SQL | ok |
| `polygon/must-contain-point` | boundary point | gvSIG | ok |
| `polygon/must-contain-point` | point in hole | topology-checker | ok |
| `polygon/must-contain-point` | point in hole | QGIS | ok |
| `polygon/must-contain-point` | point in hole | GRASS GIS | ok |
| `polygon/must-contain-point` | point in hole | PostGIS SQL | ok |
| `polygon/must-contain-point` | point in hole | gvSIG | ok |
| `polygon/must-contain-point` | point on hole boundary | topology-checker | ok |
| `polygon/must-contain-point` | point on hole boundary | QGIS | ok |
| `polygon/must-contain-point` | point on hole boundary | GRASS GIS | ok |
| `polygon/must-contain-point` | point on hole boundary | PostGIS SQL | ok |
| `polygon/must-contain-point` | point on hole boundary | gvSIG | ok |
| `point/must-not-overlap` | clean | topology-checker | ok |
| `point/must-not-overlap` | clean | QGIS | ok |
| `point/must-not-overlap` | clean | GRASS GIS | ok |
| `point/must-not-overlap` | clean | PostGIS SQL | ok |
| `point/must-not-overlap` | clean | gvSIG | ok |
| `point/must-not-overlap` | violation | topology-checker | ok |
| `point/must-not-overlap` | violation | QGIS | ok |
| `point/must-not-overlap` | violation | GRASS GIS | ok |
| `point/must-not-overlap` | violation | PostGIS SQL | ok |
| `point/must-not-overlap` | violation | gvSIG | ok |
| `point/must-not-overlap` | three coincident points | topology-checker | ok |
| `point/must-not-overlap` | three coincident points | QGIS | ok |
| `point/must-not-overlap` | three coincident points | GRASS GIS | ok |
| `point/must-not-overlap` | three coincident points | PostGIS SQL | ok |
| `point/must-not-overlap` | three coincident points | gvSIG | ok |
| `point/must-not-overlap` | duplicate triple | topology-checker | ok |
| `point/must-not-overlap` | duplicate triple | QGIS | ok |
| `point/must-not-overlap` | duplicate triple | GRASS GIS | ok |
| `point/must-not-overlap` | duplicate triple | PostGIS SQL | ok |
| `point/must-not-overlap` | duplicate triple | gvSIG | ok |
| `line/must-not-have-dangles` | clean | topology-checker | ok |
| `line/must-not-have-dangles` | clean | QGIS | ok |
| `line/must-not-have-dangles` | clean | GRASS GIS | ok |
| `line/must-not-have-dangles` | clean | PostGIS SQL | ok |
| `line/must-not-have-dangles` | clean | gvSIG | ok |
| `line/must-not-have-dangles` | violation | topology-checker | ok |
| `line/must-not-have-dangles` | violation | QGIS | ok |
| `line/must-not-have-dangles` | violation | GRASS GIS | ok |
| `line/must-not-have-dangles` | violation | PostGIS SQL | ok |
| `line/must-not-have-dangles` | violation | gvSIG | ok |
| `line/must-not-have-dangles` | endpoint on another line interior | topology-checker | ok |
| `line/must-not-have-dangles` | endpoint on another line interior | QGIS | ok |
| `line/must-not-have-dangles` | endpoint on another line interior | GRASS GIS | ok |
| `line/must-not-have-dangles` | endpoint on another line interior | PostGIS SQL | ok |
| `line/must-not-have-dangles` | endpoint on another line interior | gvSIG | ok |
| `line/must-not-have-dangles` | interior-noded connection across closed ring | topology-checker | ok |
| `line/must-not-have-dangles` | interior-noded connection across closed ring | QGIS | ok |
| `line/must-not-have-dangles` | interior-noded connection across closed ring | GRASS GIS | ok |
| `line/must-not-have-dangles` | interior-noded connection across closed ring | PostGIS SQL | ok |
| `line/must-not-have-dangles` | interior-noded connection across closed ring | gvSIG | ok |
| `line/must-not-have-dangles` | closed line | topology-checker | ok |
| `line/must-not-have-dangles` | closed line | QGIS | ok |
| `line/must-not-have-dangles` | closed line | GRASS GIS | ok |
| `line/must-not-have-dangles` | closed line | PostGIS SQL | ok |
| `line/must-not-have-dangles` | closed line | gvSIG | ok |
| `line/must-not-have-dangles` | T junction with interior connection | topology-checker | ok |
| `line/must-not-have-dangles` | T junction with interior connection | QGIS | ok |
| `line/must-not-have-dangles` | T junction with interior connection | GRASS GIS | ok |
| `line/must-not-have-dangles` | T junction with interior connection | PostGIS SQL | ok |
| `line/must-not-have-dangles` | T junction with interior connection | gvSIG | ok |
| `line/must-not-have-dangles` | closed line | topology-checker | ok |
| `line/must-not-have-dangles` | closed line | QGIS | ok |
| `line/must-not-have-dangles` | closed line | GRASS GIS | ok |
| `line/must-not-have-dangles` | closed line | PostGIS SQL | ok |
| `line/must-not-have-dangles` | closed line | gvSIG | ok |
| `line/must-not-have-dangles` | closed line with attached endpoint | topology-checker | ok |
| `line/must-not-have-dangles` | closed line with attached endpoint | QGIS | ok |
| `line/must-not-have-dangles` | closed line with attached endpoint | GRASS GIS | ok |
| `line/must-not-have-dangles` | closed line with attached endpoint | PostGIS SQL | ok |
| `line/must-not-have-dangles` | closed line with attached endpoint | gvSIG | ok |
| `line/must-not-have-dangles` | both endpoints connect to own interior | topology-checker | ok |
| `line/must-not-have-dangles` | both endpoints connect to own interior | QGIS | ok |
| `line/must-not-have-dangles` | both endpoints connect to own interior | GRASS GIS | ok |
| `line/must-not-have-dangles` | both endpoints connect to own interior | PostGIS SQL | ok |
| `line/must-not-have-dangles` | both endpoints connect to own interior | gvSIG | ok |
| `polygon/must-not-overlap` | clean | topology-checker | ok |
| `polygon/must-not-overlap` | clean | QGIS | ok |
| `polygon/must-not-overlap` | clean | GRASS GIS | ok |
| `polygon/must-not-overlap` | clean | PostGIS SQL | ok |
| `polygon/must-not-overlap` | clean | gvSIG | ok |
| `polygon/must-not-overlap` | violation | topology-checker | ok |
| `polygon/must-not-overlap` | violation | QGIS | ok |
| `polygon/must-not-overlap` | violation | GRASS GIS | ok |
| `polygon/must-not-overlap` | violation | PostGIS SQL | ok |
| `polygon/must-not-overlap` | violation | gvSIG | ok |
| `polygon/must-not-overlap` | equal polygons | topology-checker | ok |
| `polygon/must-not-overlap` | equal polygons | QGIS | ok |
| `polygon/must-not-overlap` | equal polygons | GRASS GIS | ok |
| `polygon/must-not-overlap` | equal polygons | PostGIS SQL | ok |
| `polygon/must-not-overlap` | equal polygons | gvSIG | ok |
| `polygon/must-not-overlap` | contained polygon | topology-checker | ok |
| `polygon/must-not-overlap` | contained polygon | QGIS | ok |
| `polygon/must-not-overlap` | contained polygon | GRASS GIS | ok |
| `polygon/must-not-overlap` | contained polygon | PostGIS SQL | ok |
| `polygon/must-not-overlap` | contained polygon | gvSIG | ok |
| `polygon/must-not-overlap` | edge contact | topology-checker | ok |
| `polygon/must-not-overlap` | edge contact | QGIS | ok |
| `polygon/must-not-overlap` | edge contact | GRASS GIS | ok |
| `polygon/must-not-overlap` | edge contact | PostGIS SQL | ok |
| `polygon/must-not-overlap` | edge contact | gvSIG | ok |
| `polygon/must-not-overlap` | equal polygons | topology-checker | ok |
| `polygon/must-not-overlap` | equal polygons | QGIS | ok |
| `polygon/must-not-overlap` | equal polygons | GRASS GIS | ok |
| `polygon/must-not-overlap` | equal polygons | PostGIS SQL | ok |
| `polygon/must-not-overlap` | equal polygons | gvSIG | ok |
| `polygon/must-not-overlap` | contained polygon | topology-checker | ok |
| `polygon/must-not-overlap` | contained polygon | QGIS | ok |
| `polygon/must-not-overlap` | contained polygon | GRASS GIS | ok |
| `polygon/must-not-overlap` | contained polygon | PostGIS SQL | ok |
| `polygon/must-not-overlap` | contained polygon | gvSIG | ok |
| `polygon/must-not-overlap` | boundary touch | topology-checker | ok |
| `polygon/must-not-overlap` | boundary touch | QGIS | ok |
| `polygon/must-not-overlap` | boundary touch | GRASS GIS | ok |
| `polygon/must-not-overlap` | boundary touch | PostGIS SQL | ok |
| `polygon/must-not-overlap` | boundary touch | gvSIG | ok |
| `polygon/must-not-overlap` | overlapping triple | topology-checker | ok |
| `polygon/must-not-overlap` | overlapping triple | QGIS | ok |
| `polygon/must-not-overlap` | overlapping triple | GRASS GIS | ok |
| `polygon/must-not-overlap` | overlapping triple | PostGIS SQL | ok |
| `polygon/must-not-overlap` | overlapping triple | gvSIG | ok |
| `polygon/must-not-have-gaps` | clean | topology-checker | ok |
| `polygon/must-not-have-gaps` | clean | QGIS | ok |
| `polygon/must-not-have-gaps` | clean | GRASS GIS | ok |
| `polygon/must-not-have-gaps` | clean | PostGIS SQL | ok |
| `polygon/must-not-have-gaps` | clean | gvSIG | ok |
| `polygon/must-not-have-gaps` | violation | topology-checker | ok |
| `polygon/must-not-have-gaps` | violation | QGIS | ok |
| `polygon/must-not-have-gaps` | violation | GRASS GIS | ok |
| `polygon/must-not-have-gaps` | violation | PostGIS SQL | ok |
| `polygon/must-not-have-gaps` | violation | gvSIG | ok |
| `polygon/must-not-have-gaps` | bounded hole | topology-checker | ok |
| `polygon/must-not-have-gaps` | bounded hole | QGIS | ok |
| `polygon/must-not-have-gaps` | bounded hole | GRASS GIS | ok |
| `polygon/must-not-have-gaps` | bounded hole | PostGIS SQL | ok |
| `polygon/must-not-have-gaps` | bounded hole | gvSIG | ok |
| `polygon/must-not-have-gaps` | bounded hole with island | topology-checker | ok |
| `polygon/must-not-have-gaps` | bounded hole with island | QGIS | ok |
| `polygon/must-not-have-gaps` | bounded hole with island | GRASS GIS | ok |
| `polygon/must-not-have-gaps` | bounded hole with island | PostGIS SQL | ok |
| `polygon/must-not-have-gaps` | bounded hole with island | gvSIG | ok |
| `polygon/must-not-have-gaps` | separate exteriors | topology-checker | ok |
| `polygon/must-not-have-gaps` | separate exteriors | QGIS | ok |
| `polygon/must-not-have-gaps` | separate exteriors | GRASS GIS | ok |
| `polygon/must-not-have-gaps` | separate exteriors | PostGIS SQL | ok |
| `polygon/must-not-have-gaps` | separate exteriors | gvSIG | ok |
| `polygon/must-not-have-gaps` | covered overlap faces | topology-checker | ok |
| `polygon/must-not-have-gaps` | covered overlap faces | QGIS | ok |
| `polygon/must-not-have-gaps` | covered overlap faces | GRASS GIS | ok |
| `polygon/must-not-have-gaps` | covered overlap faces | PostGIS SQL | ok |
| `polygon/must-not-have-gaps` | covered overlap faces | gvSIG | ok |
| `polygon/must-not-have-gaps` | filled hole | topology-checker | ok |
| `polygon/must-not-have-gaps` | filled hole | QGIS | ok |
| `polygon/must-not-have-gaps` | filled hole | GRASS GIS | ok |
| `polygon/must-not-have-gaps` | filled hole | PostGIS SQL | ok |
| `polygon/must-not-have-gaps` | filled hole | gvSIG | ok |

## Native operations

- `point/must-be-inside`, topology-checker: Topology.validate with fresh caches
- `point/must-be-inside`, QGIS: must be inside
- `point/must-be-inside`, GRASS GIS: v.select within, reverse selection
- `point/must-be-inside`, PostGIS SQL: GiST and ST_ContainsProperly reverse selection
- `point/must-be-inside`, gvSIG: PointMustBeProperlyInsidePolygon
- `polygon/must-contain-point`, topology-checker: Topology.validate with fresh caches
- `polygon/must-contain-point`, QGIS: must contain
- `polygon/must-contain-point`, GRASS GIS: v.select contains, reverse selection
- `polygon/must-contain-point`, PostGIS SQL: GiST and ST_ContainsProperly reverse selection
- `polygon/must-contain-point`, gvSIG: PolygonContainsPoint
- `point/must-not-overlap`, topology-checker: Topology.validate with fresh caches
- `point/must-not-overlap`, QGIS: QgsSpatialIndex and native geometry equality pairs
- `point/must-not-overlap`, GRASS GIS: v.to.db coordinates, SQLite duplicate-pair query, native error vectors
- `point/must-not-overlap`, PostGIS SQL: GiST and ST_Equals pairs
- `point/must-not-overlap`, gvSIG: native STRtree and JTS equalsTopo pairs
- `line/must-not-have-dangles`, topology-checker: Topology.validate with fresh caches
- `line/must-not-have-dangles`, QGIS: QgsSpatialIndex and endpoint incidence
- `line/must-not-have-dangles`, GRASS GIS: v.clean break, native node degree and original categories
- `line/must-not-have-dangles`, PostGIS SQL: GiST and endpoint-incidence SQL
- `line/must-not-have-dangles`, gvSIG: native STRtree and JTS endpoint incidence
- `polygon/must-not-overlap`, topology-checker: Topology.validate with fresh caches
- `polygon/must-not-overlap`, QGIS: QgsSpatialIndex and positive-area intersection pairs
- `polygon/must-not-overlap`, GRASS GIS: native spatial index and attributed v.overlay intersection pairs
- `polygon/must-not-overlap`, PostGIS SQL: GiST and positive-area ST_Intersection pairs
- `polygon/must-not-overlap`, gvSIG: native STRtree and JTS positive-area intersection pairs
- `polygon/must-not-have-gaps`, topology-checker: Topology.validate with fresh caches
- `polygon/must-not-have-gaps`, QGIS: QgsGeometry.unaryUnion and bounded void difference
- `polygon/must-not-have-gaps`, GRASS GIS: v.clean break,rmdupl,bpol and original-ring coverage of bounded native faces
- `polygon/must-not-have-gaps`, PostGIS SQL: ST_UnaryUnion and bounded void ST_Difference
- `polygon/must-not-have-gaps`, gvSIG: native JTS unary union and bounded void difference

## Engine availability and versions

- topology-checker: ready. Version: 0.1.0.
- QGIS: ready. Version: 4.2.3-Belém do Pará.
- GRASS GIS: ready. Version: 8.4.1.
- PostGIS SQL: ready. Version: 3.5.
- gvSIG: ready. Version: 2.7.0-5016.

## Provenance

```json
{
  "started_at": "2026-10-13T05:48:18.499908+00:00",
  "source_revision": "42732292e5e64cdb6bc1e50024834aa2af43aa57",
  "source_tree_sha256": "9ec282c77a2b6c05ca7a9be25dbeb85ad337c4b51044e7028ccfee5df29b5450",
  "tooling_sha256": {
    "benchmarks/gis/Dockerfile": "cc3bea975ed12cea7d6e62ebea32141aa0ae1cf26aa50e225bfeb56c1a2d30ec",
    "benchmarks/gis/README.md": "3256123cc8cc856675c4429bd42c747038321e5dc55194efb860c81665127c5e",
    "benchmarks/gis/__init__.py": "368bc3a69563653e7fe214556683a87731b8c6d6bc715bd02333b7bddf5c6159",
    "benchmarks/gis/adapters.py": "7a7f3a20eb3d8bb2ec6d7ba2c6d105fb7a4bf16346a3c6eb7416e55e39fe31d0",
    "benchmarks/gis/catalog.py": "9709a7a8bd1fe899ca652c55f723251c8121cf8efda50f8fe3bfc6893de983d7",
    "benchmarks/gis/common.py": "3c04d6b9dae8007fcd7a273f8de5660bcd623baf1679028f8f6009ef3abd7cbe",
    "benchmarks/gis/container-entrypoint.sh": "5e026c2e27cb56d918cad579e9020ad41796d0db333ea72de8cb5af108399b8e",
    "benchmarks/gis/grass_native.py": "cc201205f2053fb421f1d3a2d4fadb2521f39c2d8c0a08f01ebe12b5c1e12c97",
    "benchmarks/gis/grass_worker.py": "a17db2f4ee2c8e22b2770b2893a150fb1ebe1a5ef63485b7e8cf52476dfa39d0",
    "benchmarks/gis/gvsig-download-chain.pem": "cbd8aec7c2cf94b1c3914b8ce11240ef2ce18f49228f3da34d9a23dd22f52775",
    "benchmarks/gis/gvsig_bridge.py": "cfa2e7976c64df9168176e8c8b29cc1206c46133503354c0f3712c52d08aa430",
    "benchmarks/gis/gvsig_native.py": "1ff2a935f3ba51ef11e7c71e9811924b67cd4deeb893aa78f663ecd41ec514bf",
    "benchmarks/gis/launch_gvsig.sh": "478445cf12f18db956c587aa87dcd5cd889c3249d00a8ef2b5d3ce500054d7e5",
    "benchmarks/gis/plot.py": "740e5e5830c1d64e33278179e5d4dc2086d049faafef46458bca8d098e3fc029",
    "benchmarks/gis/postgis.py": "ecc2a18fcf6fe055669e1bfff574a3fa1abe3864f42d32b07bb7628552a8a383",
    "benchmarks/gis/probes.py": "99248c3152f1fd877cde1a0395add14fbdf44b258e71dd26917f7cbc246a0423",
    "benchmarks/gis/pyproject.toml": "2d1ca9625c6b3c54efe4dded451440f008aa849718e057bf187a0a1d3853df90",
    "benchmarks/gis/qgis_worker.py": "b00fd099d30c1c52312ec78a8d095b25149a69c4bf21e2909aea35c43777c32c",
    "benchmarks/gis/run.py": "275d23b97c5634e662cef3e80d26e7d8269deeb109d26780a674e7e5ead51d0b",
    "benchmarks/gis/uv.lock": "3057fb0caf6dcff64bcdb683aab98214922ad5df358530f325f5e90f92936c83",
    "Cargo.lock": "288fd902645017fd7db22753f807463991ea5e467db3858fb5e6e47206c23567",
    "crates/io/examples/qgis_benchmark.rs": "16d264dab8592527189a19454229befc9dc7ff7fd1555d3e4d2ea872202155ac",
    "benchmarks/qgis/fixtures.py": "b1f7ae49ed5d15fc7fec22eb2b49dacf8b78702d713a9274584e737a50fc0912",
    "benchmarks/qgis/fixtures.json": "6055ecc5eb1764b38c6796d987c02d0272966256a88b8a9047cd8ea40c118041",
    "benchmarks/qgis/qgis_startup.py": "738f88a2a404868e92fa9a32ac5b6bf245c3afd91c7bf0e96c9e0d81edc43774"
  },
  "platform": "Linux-6.17.0-1022-azure-x86_64-with-glibc2.42",
  "machine": "x86_64",
  "execution": {
    "native_linux_ci": true,
    "runner": "GitHub Actions 1000000973",
    "run_url": "https://github.com/alecsandrei/topology-checker/actions/runs/38353001312",
    "cpu_affinity": [
      0
    ]
  },
  "cpu": "processor\t: 0\nvendor_id\t: AuthenticAMD\ncpu family\t: 25\nmodel\t\t: 1\nmodel name\t: AMD EPYC 7763 64-Core Processor\nstepping\t: 1\nmicrocode\t: 0xffffffff\ncpu MHz\t\t: 3243.998\ncache size\t: 512 KB\nphysical id\t: 0\nsiblings\t: 4\ncore id\t\t: 0\ncpu cores\t: 2\napicid\t\t: 0\ninitial apicid\t: 0\nfpu\t\t: yes\nfpu_exception\t: yes\ncpuid level\t: 13\nwp\t\t: yes\nflags\t\t: fpu vme de pse tsc msr pae mce cx8 apic sep mtrr pge mca cmov pat pse36 clflush mmx fxsr sse sse2 ht syscall nx mmxext fxsr_opt pdpe1gb rdtscp lm constant_tsc rep_good nopl tsc_reliable nonstop_tsc cpuid extd_apicid aperfmperf tsc_known_freq pni pclmulqdq ssse3 fma cx16 pcid sse4_1 sse4_2 movbe popcnt aes xsave avx f16c rdrand hypervisor lahf_lm cmp_legacy svm cr8_legacy abm sse4a misalignsse 3dnowprefetch osvw topoext vmmcall fsgsbase bmi1 avx2 smep bmi2 erms invpcid rdseed adx smap clflushopt clwb sha_ni xsaveopt xsavec xgetbv1 xsaves user_shstk clzero xsaveerptr rdpru arat npt nrip_save tsc_scale vmcb_clean flushbyasid decodeassists pausefilter pfthreshold v_vmsave_vmload umip vaes vpclmulqdq rdpid fsrm\nbugs\t\t: sysret_ss_attrs null_seg spectre_v1 spectre_v2 spec_store_bypass srso spectre_v2_user tsa\nbogomips\t: 4890.84\nTLB size\t: 2560 4K pages\nclflush size\t: 64\ncache_alignment\t: 64\naddress sizes\t: 48 bits physical, 48 bits virtual\npower management:\n\nprocessor\t: 1\nvendor_id\t: AuthenticAMD\ncpu family\t: 25\nmodel\t\t: 1\nmodel name\t: AMD EPYC 7763 64-Core Processor\nstepping\t: 1\nmicrocode\t: 0xffffffff\ncpu MHz\t\t: 2445.424\ncache size\t: 512 KB\nphysical id\t: 0\nsiblings\t: 4\ncore id\t\t: 0\ncpu cores\t: 2\napicid\t\t: 1\ninitial apicid\t: 1\nfpu\t\t: yes\nfpu_exception\t: yes\ncpuid level\t: 13\nwp\t\t: yes\nflags\t\t: fpu vme de pse tsc msr pae mce cx8 apic sep mtrr pge mca cmov pat pse36 clflush mmx fxsr sse sse2 ht syscall nx mmxext fxsr_opt pdpe1gb rdtscp lm constant_tsc rep_good nopl tsc_reliable nonstop_tsc cpuid extd_apicid aperfmperf tsc_known_freq pni pclmulqdq ssse3 fma cx16 pcid sse4_1 sse4_2 movbe popcnt aes xsave avx f16c rdrand hypervisor lahf_lm cmp_legacy svm cr8_legacy abm sse4a misalignsse 3dnowprefetch osvw topoext vmmcall fsgsbase bmi1 avx2 smep bmi2 erms invpcid rdseed adx smap clflushopt clwb sha_ni xsaveopt xsavec xgetbv1 xsaves user_shstk clzero xsaveerptr rdpru arat npt nrip_save tsc_scale vmcb_clean flushbyasid decodeassists pausefilter pfthreshold v_vmsave_vmload umip vaes vpclmulqdq rdpid fsrm\nbugs\t\t: sysret_ss_attrs null_seg spectre_v1 spectre_v2 spec_store_bypass srso spectre_v2_user tsa\nbogomips\t: 4890.84\nTLB size\t: 2560 4K pages\nclflush size\t: 64\ncache_alignment\t: 64\naddress sizes\t: 48 bits physical, 48 bits virtual\npower management:\n\nprocessor\t: 2\nvendor_id\t: AuthenticAMD\ncpu family\t: 25\nmodel\t\t: 1\nmodel name\t: AMD EPYC 7763 64-Core Processor\nstepping\t: 1\nmicrocode\t: 0xffffffff\ncpu MHz\t\t: 2445.424\ncache size\t: 512 KB\nphysical id\t: 0\nsiblings\t: 4\ncore id\t\t: 1\ncpu cores\t: 2\napicid\t\t: 2\ninitial apicid\t: 2\nfpu\t\t: yes\nfpu_exception\t: yes\ncpuid level\t: 13\nwp\t\t: yes\nflags\t\t: fpu vme de pse tsc msr pae mce cx8 apic sep mtrr pge mca cmov pat pse36 clflush mmx fxsr sse sse2 ht syscall nx mmxext fxsr_opt pdpe1gb rdtscp lm constant_tsc rep_good nopl tsc_reliable nonstop_tsc cpuid extd_apicid aperfmperf tsc_known_freq pni pclmulqdq ssse3 fma cx16 pcid sse4_1 sse4_2 movbe popcnt aes xsave avx f16c rdrand hypervisor lahf_lm cmp_legacy svm cr8_legacy abm sse4a misalignsse 3dnowprefetch osvw topoext vmmcall fsgsbase bmi1 avx2 smep bmi2 erms invpcid rdseed adx smap clflushopt clwb sha_ni xsaveopt xsavec xgetbv1 xsaves user_shstk clzero xsaveerptr rdpru arat npt nrip_save tsc_scale vmcb_clean flushbyasid decodeassists pausefilter pfthreshold v_vmsave_vmload umip vaes vpclmulqdq rdpid fsrm\nbugs\t\t: sysret_ss_attrs null_seg spectre_v1 spectre_v2 spec_store_bypass srso spectre_v2_user tsa\nbogomips\t: 4890.84\nTLB size\t: 2560 4K pages\nclflush size\t: 64\ncache_alignment\t: 64\naddress sizes\t: 48 bits physical, 48 bits virtual\npower management:\n\nprocessor\t: 3\nvendor_id\t: AuthenticAMD\ncpu family\t: 25\nmodel\t\t: 1\nmodel name\t: AMD EPYC 7763 64-Core Processor\nstepping\t: 1\nmicrocode\t: 0xffffffff\ncpu MHz\t\t: 2445.424\ncache size\t: 512 KB\nphysical id\t: 0\nsiblings\t: 4\ncore id\t\t: 1\ncpu cores\t: 2\napicid\t\t: 3\ninitial apicid\t: 3\nfpu\t\t: yes\nfpu_exception\t: yes\ncpuid level\t: 13\nwp\t\t: yes\nflags\t\t: fpu vme de pse tsc msr pae mce cx8 apic sep mtrr pge mca cmov pat pse36 clflush mmx fxsr sse sse2 ht syscall nx mmxext fxsr_opt pdpe1gb rdtscp lm constant_tsc rep_good nopl tsc_reliable nonstop_tsc cpuid extd_apicid aperfmperf tsc_known_freq pni pclmulqdq ssse3 fma cx16 pcid sse4_1 sse4_2 movbe popcnt aes xsave avx f16c rdrand hypervisor lahf_lm cmp_legacy svm cr8_legacy abm sse4a misalignsse 3dnowprefetch osvw topoext vmmcall fsgsbase bmi1 avx2 smep bmi2 erms invpcid rdseed adx smap clflushopt clwb sha_ni xsaveopt xsavec xgetbv1 xsaves user_shstk clzero xsaveerptr rdpru arat npt nrip_save tsc_scale vmcb_clean flushbyasid decodeassists pausefilter pfthreshold v_vmsave_vmload umip vaes vpclmulqdq rdpid fsrm\nbugs\t\t: sysret_ss_attrs null_seg spectre_v1 spectre_v2 spec_store_bypass srso spectre_v2_user tsa\nbogomips\t: 4890.84\nTLB size\t: 2560 4K pages\nclflush size\t: 64\ncache_alignment\t: 64\naddress sizes\t: 48 bits physical, 48 bits virtual\npower management:\n\n",
  "memory": "MemTotal:       16373452 kB\nMemFree:          972100 kB\nMemAvailable:   14684160 kB\nBuffers:          278432 kB\nCached:         13061852 kB\nSwapCached:           40 kB\nActive:           820740 kB\nInactive:       12997588 kB\nActive(anon):     446380 kB\nInactive(anon):   124356 kB\nActive(file):     374360 kB\nInactive(file): 12873232 kB\nUnevictable:       46760 kB\nMlocked:           43688 kB\nSwapTotal:       3145724 kB\nSwapFree:        3145684 kB\nZswap:                 0 kB\nZswapped:              0 kB\nDirty:             25012 kB\nWriteback:             0 kB\nAnonPages:        484536 kB\nMapped:           512572 kB\nShmem:             83664 kB\nKReclaimable:     871860 kB\nSlab:            1043236 kB\nSReclaimable:     871860 kB\nSUnreclaim:       171376 kB\nKernelStack:        5216 kB\nPageTables:         9244 kB\nSecPageTables:         0 kB\nNFS_Unstable:          0 kB\nBounce:                0 kB\nWritebackTmp:          0 kB\nCommitLimit:    11332448 kB\nCommitted_AS:    2967344 kB\nVmallocTotal:   34359738367 kB\nVmallocUsed:       42664 kB\nVmallocChunk:          0 kB\nPercpu:             2272 kB\nHardwareCorrupted:     0 kB\nAnonHugePages:    198656 kB\nShmemHugePages:        0 kB\nShmemPmdMapped:        0 kB\nFileHugePages:         0 kB\nFilePmdMapped:         0 kB\nCmaTotal:              0 kB\nCmaFree:               0 kB\nUnaccepted:            0 kB\nBalloon:               0 kB\nHugePages_Total:       0\nHugePages_Free:        0\nHugePages_Rsvd:        0\nHugePages_Surp:        0\nHugepagesize:       2048 kB\nHugetlb:               0 kB\nDirectMap4k:      158848 kB\nDirectMap2M:     6127616 kB\nDirectMap1G:    12582912 kB\n",
  "packages": "adduser=3.152ubuntu1\nadwaita-icon-theme=49.0-1\nalsa-topology-conf=1.2.5.1-3\nalsa-ucm-conf=1.2.12-1ubuntu2.4\napt=3.1.6ubuntu2\nassistant-qt6=6.9.2-1\nat-spi2-common=2.57.1-1\nat-spi2-core=2.57.1-1\nautoconf=2.72-3.1ubuntu1\nautomake=1:1.17-4ubuntu1\nautotools-dev=20240727.1\nbase-files=14ubuntu3\nbase-passwd=3.6.7\nbash=5.2.37-2ubuntu5\nbinutils-common=2.45-7ubuntu1.2\nbinutils-x86-64-linux-gnu=2.45-7ubuntu1.2\nbinutils=2.45-7ubuntu1.2\nblt=2.5.3+dfsg-8\nbsdutils=1:2.41-4ubuntu4.2\nbuild-essential=12.12ubuntu1\nbzip2=1.0.8-6build1\nca-certificates=20260601~25.10.1\ncfortran=20210827-1\ncomerr-dev=2.1-1.47.2-3ubuntu2\ncoreutils-from-uutils=0.0.0~ubuntu24\ncoreutils=9.5-1ubuntu2+0.0.0~ubuntu24\ncpdb-backend-cups=2.0b7-0ubuntu2\ncpp-15-x86-64-linux-gnu=15.2.0-4ubuntu4\ncpp-15=15.2.0-4ubuntu4\ncpp-x86-64-linux-gnu=4:15.2.0-4ubuntu1\ncpp=4:15.2.0-4ubuntu1\ncurl=8.14.1-2ubuntu1.5\ndash=0.5.12-12ubuntu2\ndbus-bin=1.16.2-2ubuntu2\ndbus-daemon=1.16.2-2ubuntu2\ndbus-session-bus-common=1.16.2-2ubuntu2\ndbus-system-bus-common=1.16.2-2ubuntu2\ndbus-user-session=1.16.2-2ubuntu2\ndbus=1.16.2-2ubuntu2\ndconf-gsettings-backend=0.40.0-5willsync1\ndconf-service=0.40.0-5willsync1\ndebconf=1.5.91\ndebianutils=5.23.2\ndefault-libmysqlclient-dev=1.1.1ubuntu1\ndesigner-qt6=6.9.2-1\ndiffutils=1:3.10-4\ndirmngr=2.4.8-2ubuntu2.1\ndmsetup=2:1.02.205-2ubuntu2\ndpkg-dev=1.22.21ubuntu3.2\ndpkg=1.22.21ubuntu3.2\ne2fsprogs=1.47.2-3ubuntu2\nfindutils=4.10.0-3build1\nfontconfig-config=2.15.0-2.3ubuntu1\nfontconfig=2.15.0-2.3ubuntu1\nfonts-dejavu-core=2.37-8\nfonts-dejavu-mono=2.37-8\nfonts-lyx=2.4.4-1\ng++-15-x86-64-linux-gnu=15.2.0-4ubuntu4\ng++-15=15.2.0-4ubuntu4\ng++-x86-64-linux-gnu=4:15.2.0-4ubuntu1\ng++=4:15.2.0-4ubuntu1\ngcc-15-base=15.2.0-4ubuntu4\ngcc-15-x86-64-linux-gnu=15.2.0-4ubuntu4\ngcc-15=15.2.0-4ubuntu4\ngcc-x86-64-linux-gnu=4:15.2.0-4ubuntu1\ngcc=4:15.2.0-4ubuntu1\ngcr4=4.4.0.1-7\ngcr=3.41.2-4\ngdal-bin=3.10.3+dfsg-1build2\ngdal-data=3.10.3+dfsg-1build2\ngdal-plugins=3.10.3+dfsg-1build2\ngir1.2-girepository-2.0=1.84.0-1\ngir1.2-glib-2.0=2.86.0-2ubuntu0.3\ngit-man=1:2.51.0-1ubuntu1\ngit=1:2.51.0-1ubuntu1\ngnome-keyring-pkcs11=48.0-4ubuntu0.2\ngnome-keyring=48.0-4ubuntu0.2\ngnome-themes-extra-data=3.28-4\ngnu-coreutils=9.5-1ubuntu4.1\ngnupg-l10n=2.4.8-2ubuntu2.1\ngnupg-utils=2.4.8-2ubuntu2.1\ngnupg=2.4.8-2ubuntu2.1\ngpg-agent=2.4.8-2ubuntu2.1\ngpg-wks-client=2.4.8-2ubuntu2.1\ngpg=2.4.8-2ubuntu2.1\ngpgconf=2.4.8-2ubuntu2.1\ngpgsm=2.4.8-2ubuntu2.1\ngpgv=2.4.8-2ubuntu2.1\ngrass-core=8.4.1-1\ngrass-doc=8.4.1-1\ngrass-gui=8.4.1-1\ngrass=8.4.1-1\ngrep=3.11-4build1\ngsettings-desktop-schemas=49.0-1ubuntu1\ngstreamer1.0-gl=1.26.6-1ubuntu0.1\ngstreamer1.0-plugins-base=1.26.6-1ubuntu0.1\ngtk-update-icon-cache=4.20.1+ds-2\ngzip=1.13-1ubuntu4\nhdf5-helpers=1.14.5+repack-3build1\nhicolor-icon-theme=0.18-2\nhostname=3.25\nicu-devtools=76.1-4ubuntu2\ninit-system-helpers=1.68\nisa-support=27ubuntu1\niso-codes=4.18.0-1\nisympy-common=1.13.3-5\nisympy3=1.13.3-5\njavascript-common=12+nmu1\nkrb5-locales=1.21.3-5ubuntu2\nkrb5-multidev=1.21.3-5ubuntu2\nlibaacs0=0.11.1-4\nlibabsl20240722=20240722.0-4ubuntu1\nlibacl1=2.3.2-2\nlibaec-dev=1.1.3-1\nlibaec0=1.1.3-1\nlibaom-dev=3.12.1-1\nlibaom3=3.12.1-1\nlibapparmor1=5.0.0~alpha1-0ubuntu8.3\nlibapt-pkg7.0=3.1.6ubuntu2\nlibarmadillo-dev=1:14.2.3+dfsg-1\nlibarmadillo14=1:14.2.3+dfsg-1\nlibarpack2-dev=3.9.1-6\nlibarpack2t64=3.9.1-6\nlibasan8=15.2.0-4ubuntu4\nlibasound2-data=1.2.14-1ubuntu1.1\nlibasound2t64=1.2.14-1ubuntu1.1\nlibassimp5=5.4.3+ds-2\nlibassuan9=3.0.2-2\nlibasyncns0=0.8-6build4\nlibatk-bridge2.0-0t64=2.57.1-1\nlibatk1.0-0t64=2.57.1-1\nlibatomic1=15.2.0-4ubuntu4\nlibatspi2.0-0t64=2.57.1-1\nlibattr1=1:2.5.2-3build1\nlibaudit-common=1:4.0.5-1build1\nlibaudit1=1:4.0.5-1build1\nlibavahi-client3=0.8-16ubuntu3.2\nlibavahi-common-data=0.8-16ubuntu3.2\nlibavahi-common3=0.8-16ubuntu3.2\nlibavcodec61=7:7.1.1-1ubuntu4.2\nlibavformat61=7:7.1.1-1ubuntu4.2\nlibavif-dev=1.3.0-1ubuntu1\nlibavif16=1.3.0-1ubuntu1\nlibavutil59=7:7.1.1-1ubuntu4.2\nlibb2-1=0.98.1-1.1build1\nlibbdplus0=0.2.0-4\nlibbinutils=2.45-7ubuntu1.2\nlibblas-dev=3.12.1-6build1\nlibblas3=3.12.1-6build1\nlibblkid1=2.41-4ubuntu4.2\nlibblosc-dev=1.21.5+ds-1build1\nlibblosc1=1.21.5+ds-1build1\nlibbluray2=1:1.3.4-1build2\nlibboost-dev=1.88.0.0ubuntu1\nlibboost-serialization1.88.0=1.88.0-1.4ubuntu1\nlibboost1.88-dev=1.88.0-1.4ubuntu1\nlibbrotli-dev=1.1.0-2build5\nlibbrotli1=1.1.0-2build5\nlibbsd0=0.12.2-2build1\nlibbz2-1.0=1.0.8-6build1\nlibc-bin=2.42-0ubuntu3.1\nlibc-dev-bin=2.42-0ubuntu3.1\nlibc6-dev=2.42-0ubuntu3.1\nlibc6=2.42-0ubuntu3.1\nlibcairo-gobject2=1.18.4-1build1\nlibcairo-script-interpreter2=1.18.4-1build1\nlibcairo2=1.18.4-1build1\nlibcap-ng0=0.8.5-4build2\nlibcap2-bin=1:2.75-7ubuntu2.2\nlibcap2=1:2.75-7ubuntu2.2\nlibcc1-0=15.2.0-4ubuntu4\nlibcdparanoia0=3.10.2+debian-14ubuntu1\nlibcfitsio-dev=4.6.2-2\nlibcfitsio10t64=4.6.2-2\nlibchromaprint1=1.5.1-7\nlibcjson1=1.7.18-3.1ubuntu0.1\nlibclang-cpp19=1:19.1.7-6ubuntu3\nlibclang1-19=1:19.1.7-6ubuntu3\nlibcodec2-1.2=1.2.0-3\nlibcolord2=1.4.7-3build1\nlibcom-err2=1.47.2-3ubuntu2\nlibcpdb-frontend2t64=2.0~b7-0ubuntu3\nlibcpdb2t64=2.0~b7-0ubuntu3\nlibcrypt-dev=1:4.4.38-1build1\nlibcrypt1=1:4.4.38-1build1\nlibcryptsetup12=2:2.8.0-1ubuntu2\nlibctf-nobfd0=2.45-7ubuntu1.2\nlibctf0=2.45-7ubuntu1.2\nlibcups2t64=2.4.12-0ubuntu3.10\nlibcupsfilters2-common=2.1.1-0ubuntu3.1\nlibcupsfilters2t64=2.1.1-0ubuntu3.1\nlibcurl3t64-gnutls=8.14.1-2ubuntu1.5\nlibcurl4-openssl-dev=8.14.1-2ubuntu1.5\nlibcurl4t64=8.14.1-2ubuntu1.5\nlibdatrie1=0.2.13-4\nlibdav1d-dev=1.5.1-1\nlibdav1d7=1.5.1-1\nlibdb5.3t64=5.3.28+dfsg2-9ubuntu1\nlibdbus-1-3=1.16.2-2ubuntu2\nlibdconf1=0.40.0-5willsync1\nlibdebconfclient0=0.279ubuntu1\nlibdeflate-dev=1.23-2\nlibdeflate0=1.23-2\nlibdevmapper1.02.1=2:1.02.205-2ubuntu2\nlibdouble-conversion3=3.3.1-1build1\nlibdpkg-perl=1.22.21ubuntu3.2\nlibdraco8=1.5.6+dfsg-3build1\nlibdrm-amdgpu1=2.4.125-1ubuntu0.1\nlibdrm-common=2.4.125-1ubuntu0.1\nlibdrm-intel1=2.4.125-1ubuntu0.1\nlibdrm2=2.4.125-1ubuntu0.1\nlibduktape207=2.7.0+tests-0ubuntu3\nlibdvdnav4=6.1.1-3build1\nlibdvdread8t64=6.1.3-2\nlibedit2=3.1-20250104-1build1\nlibegl-mesa0=25.2.8-0ubuntu0.25.10.2\nlibegl1=1.7.0-1build2\nlibelf1t64=0.193-1\nlibepoxy0=1.5.10-2\nliberror-perl=0.17030-1\nlibevdev2=1.13.4+dfsg-1\nlibevent-2.1-7t64=2.1.12-stable-10build1\nlibexif12=0.6.25-1\nlibexiv2-28=0.28.5+dfsg-1ubuntu0.3\nlibexiv2-data=0.28.5+dfsg-1ubuntu0.3\nlibexpat1-dev=2.7.1-2ubuntu0.2\nlibexpat1=2.7.1-2ubuntu0.2\nlibext2fs2t64=1.47.2-3ubuntu2\nlibfcgi-bin=2.4.5-0.1build1\nlibfcgi0t64=2.4.5-0.1build1\nlibffi8=3.5.2-1build1\nlibfftw3-double3=3.3.10-2fakesync1build2\nlibfftw3-single3=3.3.10-2fakesync1build2\nlibflac14=1.5.0+ds-2\nlibfontconfig1=2.15.0-2.3ubuntu1\nlibfontenc1=1:1.1.8-1build1\nlibfreetype6=2.13.3+dfsg-1ubuntu0.1\nlibfreexl-dev=2.0.0-1build2\nlibfreexl1=2.0.0-1build2\nlibfribidi0=1.0.16-1\nlibfyba-dev=4.1.1-11build1\nlibfyba0t64=4.1.1-11build1\nlibgav1-1=0.19.0-3build1\nlibgbm1=25.2.8-0ubuntu0.25.10.2\nlibgcc-15-dev=15.2.0-4ubuntu4\nlibgcc-s1=15.2.0-4ubuntu4\nlibgck-1-0=3.41.2-4\nlibgck-2-2=4.4.0.1-7\nlibgcr-4-4=4.4.0.1-7\nlibgcr-base-3-1=3.41.2-4\nlibgcr-ui-3-1=3.41.2-4\nlibgcrypt20=1.11.0-7ubuntu0.1\nlibgdal-dev=3.10.3+dfsg-1build2\nlibgdal36=3.10.3+dfsg-1build2\nlibgdbm-compat4t64=1.26-1\nlibgdbm6t64=1.26-1\nlibgdk-pixbuf-2.0-0=2.42.12+dfsg-5ubuntu0.1\nlibgdk-pixbuf2.0-bin=2.42.12+dfsg-5ubuntu0.1\nlibgdk-pixbuf2.0-common=2.42.12+dfsg-5ubuntu0.1\nlibgeographiclib26=2.5-1\nlibgeos-c1t64=3.13.1-1\nlibgeos-dev=3.13.1-1\nlibgeos3.13.1=3.13.1-1\nlibgeotiff-dev=1.7.4-1\nlibgeotiff5=1.7.4-1\nlibgfortran5=15.2.0-4ubuntu4\nlibgif-dev=5.2.2-1ubuntu2\nlibgif7=5.2.2-1ubuntu2\nlibgirepository-1.0-1=1.84.0-1\nlibgl1-mesa-dri=25.2.8-0ubuntu0.25.10.2\nlibgl1=1.7.0-1build2\nlibgles2=1.7.0-1build2\nlibglib2.0-0t64=2.86.0-2ubuntu0.3\nlibglib2.0-data=2.86.0-2ubuntu0.3\nlibglu1-mesa=9.0.2-1.1build1\nlibglvnd0=1.7.0-1build2\nlibglx-mesa0=25.2.8-0ubuntu0.25.10.2\nlibglx0=1.7.0-1build2\nlibgme0=0.6.3-7build1\nlibgmp-dev=2:6.3.0+dfsg-5ubuntu1\nlibgmp10=2:6.3.0+dfsg-5ubuntu1\nlibgmpxx4ldbl=2:6.3.0+dfsg-5ubuntu1\nlibgnutls-dane0t64=3.8.9-3ubuntu2.2\nlibgnutls-openssl27t64=3.8.9-3ubuntu2.2\nlibgnutls28-dev=3.8.9-3ubuntu2.2\nlibgnutls30t64=3.8.9-3ubuntu2.2\nlibgomp1=15.2.0-4ubuntu4\nlibgpg-error0=1.51-4\nlibgpgme11t64=1.24.2-3ubuntu1\nlibgpgmepp6t64=1.24.2-3ubuntu1\nlibgprofng0=2.45-7ubuntu1.2\nlibgraphene-1.0-0=1.10.8-5\nlibgraphite2-3=1.3.14-2ubuntu1.3\nlibgsl28=2.8+dfsg-5.1ubuntu1\nlibgslcblas0=2.8+dfsg-5.1ubuntu1\nlibgsm1=1.0.22-1build1\nlibgssapi-krb5-2=1.21.3-5ubuntu2\nlibgssrpc4t64=1.21.3-5ubuntu2\nlibgstreamer-gl1.0-0=1.26.6-1ubuntu0.1\nlibgstreamer-plugins-base1.0-0=1.26.6-1ubuntu0.1\nlibgstreamer1.0-0=1.26.6-1\nlibgtk-3-0t64=3.24.50-1ubuntu2\nlibgtk-3-bin=3.24.50-1ubuntu2\nlibgtk-3-common=3.24.50-1ubuntu2\nlibgtk-4-1=4.20.1+ds-2\nlibgtk-4-bin=4.20.1+ds-2\nlibgtk-4-common=4.20.1+ds-2\nlibgudev-1.0-0=1:238-7\nlibharfbuzz-subset0=10.2.0-1\nlibharfbuzz0b=10.2.0-1\nlibhdf4-0-alt=4.3.0-1\nlibhdf4-alt-dev=4.3.0-1\nlibhdf5-310=1.14.5+repack-3build1\nlibhdf5-cpp-310=1.14.5+repack-3build1\nlibhdf5-dev=1.14.5+repack-3build1\nlibhdf5-fortran-310=1.14.5+repack-3build1\nlibhdf5-hl-310=1.14.5+repack-3build1\nlibhdf5-hl-cpp-310=1.14.5+repack-3build1\nlibhdf5-hl-fortran-310=1.14.5+repack-3build1\nlibheif-dev=1.20.2-1ubuntu0.6\nlibheif-plugin-aomdec=1.20.2-1ubuntu0.6\nlibheif-plugin-aomenc=1.20.2-1ubuntu0.6\nlibheif1=1.20.2-1ubuntu0.6\nlibhogweed6t64=3.10.1-1\nlibhwasan0=15.2.0-4ubuntu4\nlibhwy1t64=1.2.0-3ubuntu4\nlibice6=2:1.1.1-1\nlibicu-dev=76.1-4ubuntu2\nlibicu76=76.1-4ubuntu2\nlibidn2-0=2.3.8-4\nlibidn2-dev=2.3.8-4\nlibimagequant0=2.18.0-1build1\nlibinih1=59-1ubuntu1\nlibinireader0=59-1ubuntu1\nlibinput-bin=1.28.1-1ubuntu0.4\nlibinput10=1.28.1-1ubuntu0.4\nlibio-pty-perl=1:1.20-1build3\nlibipc-run-perl=20231003.0-2\nlibisl23=0.27-1\nlibitm1=15.2.0-4ubuntu4\nlibjansson4=2.14-2build3\nlibjbig-dev=2.1-6.1ubuntu2\nlibjbig0=2.1-6.1ubuntu2\nlibjpeg-dev=8c-2ubuntu11\nlibjpeg-turbo8-dev=2.1.5-4ubuntu2\nlibjpeg-turbo8=2.1.5-4ubuntu2\nlibjpeg8-dev=8c-2ubuntu11\nlibjpeg8=8c-2ubuntu11\nlibjs-jquery-ui=1.13.2+dfsg-1\nlibjs-jquery=3.6.1+dfsg+~3.5.14-1\nlibjs-leaflet=1.7.1~dfsg-7\nlibjs-sphinxdoc=8.2.3-1ubuntu2\nlibjs-underscore=1.13.4~dfsg+~1.11.4-3\nlibjson-c-dev=0.18+ds-1\nlibjson-c5=0.18+ds-1\nlibjson-perl=4.10000-1\nlibjxl0.11=0.11.1-6ubuntu1.2\nlibk5crypto3=1.21.3-5ubuntu2\nlibkadm5clnt-mit12=1.21.3-5ubuntu2\nlibkadm5srv-mit12=1.21.3-5ubuntu2\nlibkdb5-10t64=1.21.3-5ubuntu2\nlibkeyutils1=1.6.3-6ubuntu2\nlibkml-dev=1.3.0-12build1\nlibkmlbase1t64=1.3.0-12build1\nlibkmlconvenience1t64=1.3.0-12build1\nlibkmldom1t64=1.3.0-12build1\nlibkmlengine1t64=1.3.0-12build1\nlibkmlregionator1t64=1.3.0-12build1\nlibkmlxsd1t64=1.3.0-12build1\nlibkmod2=34.2-2ubuntu1.1\nlibkrb5-3=1.21.3-5ubuntu2\nlibkrb5-dev=1.21.3-5ubuntu2\nlibkrb5support0=1.21.3-5ubuntu2\nlibksba8=1.6.7-2\nliblapack-dev=3.12.1-6build1\nliblapack3=3.12.1-6build1\nliblastlog2-2=2.41-4ubuntu4.2\nliblcms2-2=2.16-2ubuntu0.2\nlibldap-common=2.6.10+dfsg-1ubuntu2.1\nlibldap-dev=2.6.10+dfsg-1ubuntu2.1\nlibldap2=2.6.10+dfsg-1ubuntu2.1\nliblerc-dev=4.0.0+ds-5ubuntu1\nliblerc4=4.0.0+ds-5ubuntu1\nlibllvm19=1:19.1.7-6ubuntu3\nlibllvm20=1:20.1.8-0ubuntu4\nliblsan0=15.2.0-4ubuntu4\nlibltdl-dev=2.5.4-4build1\nlibltdl7=2.5.4-4build1\nliblz4-1=1.10.0-4build1\nliblz4-dev=1.10.0-4build1\nliblzma-dev=5.8.1-1ubuntu0.1\nliblzma5=5.8.1-1ubuntu0.1\nliblzo2-2=2.10-3build1\nlibmbedcrypto16=3.6.2-3ubuntu1\nlibmd0=1.1.0-2build3\nlibmd4c0=0.5.2-2\nlibmeshoptimizer2d=0.22+dfsg-1\nlibminizip-dev=1:1.3.dfsg+really1.3.1-1ubuntu2\nlibminizip1t64=1:1.3.dfsg+really1.3.1-1ubuntu2\nlibmount1=2.41-4ubuntu4.2\nlibmp3lame0=3.100-6build1\nlibmpc3=1.3.1-1build3\nlibmpfr6=4.2.2-1build1\nlibmpg123-0t64=1.32.10-1\nlibmtdev1t64=1.1.7-1\nlibmysqlclient-dev=8.4.10-0ubuntu0.25.10.1\nlibmysqlclient24=8.4.10-0ubuntu0.25.10.1\nlibncursesw6=6.5+20250216-2build1\nlibnetcdf-dev=1:4.9.3-1build1\nlibnetcdf22=1:4.9.3-1build1\nlibnettle8t64=3.10.1-1\nlibnghttp2-14=1.64.0-1.1ubuntu1.2\nlibnghttp2-dev=1.64.0-1.1ubuntu1.2\nlibnorm1t64=1.5.9+dfsg-3.1build2\nlibnotify4=0.8.6-2\nlibnpth0t64=1.8-3\nlibnspr4=2:4.36-1ubuntu2\nlibnss-systemd=257.9-0ubuntu2.5\nlibnss3=2:3.114-1ubuntu0.2\nlibnuma1=2.0.19-1\nlibodbc2=2.3.12-2ubuntu2\nlibodbccr2=2.3.12-2ubuntu2\nlibodbcinst2=2.3.12-2ubuntu2\nlibogdi-dev=4.1.1+ds-5\nlibogdi4.1=4.1.1+ds-5\nlibogg0=1.3.5-3build1\nlibopengl0=1.7.0-1build2\nlibopenjp2-7-dev=2.5.3-2.1ubuntu0.1\nlibopenjp2-7=2.5.3-2.1ubuntu0.1\nlibopenmpt0t64=0.7.13-1build1\nlibopus0=1.5.2-2\nliborc-0.4-0t64=1:0.4.41-1\nlibp11-kit-dev=0.25.5-3ubuntu1\nlibp11-kit0=0.25.5-3ubuntu1\nlibpam-cap=1:2.75-7ubuntu2.2\nlibpam-gnome-keyring=48.0-4ubuntu0.2\nlibpam-modules-bin=1.7.0-5ubuntu2\nlibpam-modules=1.7.0-5ubuntu2\nlibpam-runtime=1.7.0-5ubuntu2\nlibpam-systemd=257.9-0ubuntu2.5\nlibpam0g=1.7.0-5ubuntu2\nlibpango-1.0-0=1.56.3-1build1\nlibpangocairo-1.0-0=1.56.3-1build1\nlibpangoft2-1.0-0=1.56.3-1build1\nlibpciaccess0=0.18.1-1ubuntu2\nlibpcre2-16-0=10.46-1\nlibpcre2-32-0=10.46-1\nlibpcre2-8-0=10.46-1\nlibpcre2-dev=10.46-1\nlibpcre2-posix3=10.46-1\nlibperl5.40=5.40.1-6ubuntu0.1\nlibpgm-5.3-0t64=5.3.128~dfsg-2.1build1\nlibpipewire-0.3-0t64=1.4.7-3ubuntu2\nlibpipewire-0.3-common=1.4.7-3ubuntu2\nlibpixman-1-0=0.44.0-3\nlibpkgconf3=1.8.1-4build1\nlibpng-dev=1.6.50-1ubuntu0.5\nlibpng16-16t64=1.6.50-1ubuntu0.5\nlibpoppler-cpp2=25.03.0-10ubuntu0.2\nlibpoppler-dev=25.03.0-10ubuntu0.2\nlibpoppler-private-dev=25.03.0-10ubuntu0.2\nlibpoppler147=25.03.0-10ubuntu0.2\nlibpq-dev=17.10-0ubuntu0.25.10.1\nlibpq5=17.10-0ubuntu0.25.10.1\nlibproc2-0=2:4.0.4-8ubuntu3\nlibproj-dev=9.6.0-1\nlibproj25=9.6.0-1\nlibprotobuf-c1=1.5.1-1ubuntu1\nlibprotobuf-lite32t64=3.21.12-11ubuntu3.1\nlibproxy1v5=0.5.11-1\nlibpsl-dev=0.21.2-1.1build1\nlibpsl5t64=0.21.2-1.1build1\nlibpugixml1v5=1.14-2\nlibpulse0=1:17.0+dfsg1-2ubuntu3\nlibpython3-dev=3.13.7-1\nlibpython3-stdlib=3.13.7-1\nlibpython3.13-dev=3.13.7-1ubuntu0.4\nlibpython3.13-minimal=3.13.7-1ubuntu0.4\nlibpython3.13-stdlib=3.13.7-1ubuntu0.4\nlibpython3.13=3.13.7-1ubuntu0.4\nlibqca-qt6-2=2.3.10-1\nlibqca-qt6-plugins=2.3.10-1\nlibqgis-3d4.2.3=1:4.2.3+43questing\nlibqgis-analysis4.2.3=1:4.2.3+43questing\nlibqgis-app4.2.3=1:4.2.3+43questing\nlibqgis-core4.2.3=1:4.2.3+43questing\nlibqgis-gui4.2.3=1:4.2.3+43questing\nlibqgis-native4.2.3=1:4.2.3+43questing\nlibqgis-server4.2.3=1:4.2.3+43questing\nlibqgisgrass8-4.2.3=1:4.2.3+43questing\nlibqgispython4.2.3=1:4.2.3+43questing\nlibqhull-dev=2020.2-6build1\nlibqhull-r8.0=2020.2-6build1\nlibqhull8.0=2020.2-6build1\nlibqhullcpp8.0=2020.2-6build1\nlibqpdf30=12.2.0-1ubuntu0.1\nlibqscintilla2-qt6-15=2.14.1+dfsg-1build4\nlibqscintilla2-qt6-l10n=2.14.1+dfsg-1build4\nlibqt63danimation6=6.9.2+dfsg-1\nlibqt63dcore6=6.9.2+dfsg-1\nlibqt63dextras6=6.9.2+dfsg-1\nlibqt63dinput6=6.9.2+dfsg-1\nlibqt63dlogic6=6.9.2+dfsg-1\nlibqt63dquickscene2d6=6.9.2+dfsg-1\nlibqt63drender6=6.9.2+dfsg-1\nlibqt6charts6=6.9.2-1\nlibqt6concurrent6=6.9.2+dfsg-1ubuntu1\nlibqt6core5compat6=6.9.2-1\nlibqt6core6t64=6.9.2+dfsg-1ubuntu1\nlibqt6dbus6=6.9.2+dfsg-1ubuntu1\nlibqt6designer6=6.9.2-1\nlibqt6designercomponents6=6.9.2-1\nlibqt6gui6=6.9.2+dfsg-1ubuntu1\nlibqt6help6=6.9.2-1\nlibqt6keychain1=0.15.0-1\nlibqt6multimedia6=6.9.2-1ubuntu1\nlibqt6multimediawidgets6=6.9.2-1ubuntu1\nlibqt6network6=6.9.2+dfsg-1ubuntu1\nlibqt6opengl6=6.9.2+dfsg-1ubuntu1\nlibqt6openglwidgets6=6.9.2+dfsg-1ubuntu1\nlibqt6pdf6=6.9.2+dfsg-2\nlibqt6positioning6=6.9.2-1\nlibqt6printsupport6=6.9.2+dfsg-1ubuntu1\nlibqt6qml6=6.9.2+dfsg-1\nlibqt6qmlmeta6=6.9.2+dfsg-1\nlibqt6qmlmodels6=6.9.2+dfsg-1\nlibqt6qmlworkerscript6=6.9.2+dfsg-1\nlibqt6quick6=6.9.2+dfsg-1\nlibqt6quickcontrols2-6=6.9.2+dfsg-1\nlibqt6quickshapes6=6.9.2+dfsg-1\nlibqt6quicktemplates2-6=6.9.2+dfsg-1\nlibqt6quickwidgets6=6.9.2+dfsg-1\nlibqt6serialport6=6.9.2-1\nlibqt6shadertools6=6.9.2-1\nlibqt6spatialaudio6=6.9.2-1ubuntu1\nlibqt6sql6-sqlite=6.9.2+dfsg-1ubuntu1\nlibqt6sql6=6.9.2+dfsg-1ubuntu1\nlibqt6svg6=6.9.2-1\nlibqt6svgwidgets6=6.9.2-1\nlibqt6test6=6.9.2+dfsg-1ubuntu1\nlibqt6uitools6=6.9.2-1\nlibqt6waylandclient6=6.9.2-1\nlibqt6waylandcompositor6=6.9.2-1\nlibqt6webchannel6=6.9.2-1\nlibqt6webengine6-data=6.9.2+dfsg-2\nlibqt6webenginecore6-bin=6.9.2+dfsg-2\nlibqt6webenginecore6=6.9.2+dfsg-2\nlibqt6webenginewidgets6=6.9.2+dfsg-2\nlibqt6widgets6=6.9.2+dfsg-1ubuntu1\nlibqt6wlshellintegration6=6.9.2-1\nlibqt6xml6=6.9.2+dfsg-1ubuntu1\nlibquadmath0=15.2.0-4ubuntu4\nlibrabbitmq4=0.15.0-1ubuntu0.25.10.1\nlibraqm0=0.10.3-1\nlibrav1e-dev=0.7.1-9\nlibrav1e0.7=0.7.1-9\nlibreadline8t64=8.3-3\nlibrist4=0.2.11+dfsg-1\nlibrsvg2-2=2.60.0+dfsg-1build1\nlibrsvg2-common=2.60.0+dfsg-1build1\nlibrtmp-dev=2.4+20151223.gitfa8646d.1-2build8\nlibrtmp1=2.4+20151223.gitfa8646d.1-2build8\nlibrttopo-dev=1.1.0-4\nlibrttopo1=1.1.0-4\nlibsasl2-2=2.1.28+dfsg1-9ubuntu1\nlibsasl2-modules-db=2.1.28+dfsg1-9ubuntu1\nlibsasl2-modules=2.1.28+dfsg1-9ubuntu1\nlibseccomp2=2.6.0-2ubuntu2\nlibsecret-1-0=0.21.7-2\nlibsecret-common=0.21.7-2\nlibselinux1=3.8.1-1build1\nlibsemanage-common=3.8.1-1\nlibsemanage2=3.8.1-1\nlibsensors-config=1:3.6.2-2\nlibsensors5=1:3.6.2-2\nlibsepol2=3.8.1-1\nlibsfcgal2=2.2.0-1\nlibsframe2=2.45-7ubuntu1.2\nlibsharpyuv-dev=1.5.0-0.1\nlibsharpyuv0=1.5.0-0.1\nlibshine3=3.1.1-3\nlibsm6=2:1.2.6-1\nlibsmartcols1=2.41-4ubuntu4.2\nlibsnappy1v5=1.2.2-1\nlibsndfile1=1.2.2-3\nlibsodium23=1.0.18-1ubuntu0.25.10.1\nlibsoxr0=0.1.3-4build3\nlibspa-0.2-modules=1.4.7-3ubuntu2\nlibspatialite-dev=5.1.0-3ubuntu1\nlibspatialite8t64=5.1.0-3ubuntu1\nlibspeex1=1.2.1-3\nlibsqlite3-0=3.46.1-8ubuntu0.1\nlibsqlite3-dev=3.46.1-8ubuntu0.1\nlibsqlite3-mod-spatialite=5.1.0-3ubuntu1\nlibsrt1.5-gnutls=1.5.4-1\nlibss2=1.47.2-3ubuntu2\nlibssh-4=0.11.2-1ubuntu0.3\nlibssh2-1-dev=1.11.1-1ubuntu0.25.10.2\nlibssh2-1t64=1.11.1-1ubuntu0.25.10.2\nlibssl-dev=3.5.3-1ubuntu3.4\nlibssl3t64=3.5.3-1ubuntu3.4\nlibstdc++-15-dev=15.2.0-4ubuntu4\nlibstdc++6=15.2.0-4ubuntu4\nlibsuperlu-dev=7.0.1+dfsg1-2\nlibsuperlu7=7.0.1+dfsg1-2\nlibsvtav1enc-dev=2.3.0+dfsg-1\nlibsvtav1enc2=2.3.0+dfsg-1\nlibswresample5=7:7.1.1-1ubuntu4.2\nlibswscale8=7:7.1.1-1ubuntu4.2\nlibsystemd-shared=257.9-0ubuntu2.5\nlibsystemd0=257.9-0ubuntu2.5\nlibsz2=1.1.3-1\nlibtasn1-6-dev=4.20.0-2ubuntu0.25.10.1\nlibtasn1-6=4.20.0-2ubuntu0.25.10.1\nlibtcl8.6=8.6.17+dfsg-1\nlibtext-charwidth-perl=0.04-11build4\nlibtext-wrapi18n-perl=0.06-10\nlibthai-data=0.1.29-2build1\nlibthai0=0.1.29-2build1\nlibtheoradec1=1.2.0~alpha1+dfsg-6\nlibtheoraenc1=1.2.0~alpha1+dfsg-6\nlibtiff-dev=4.7.0-3ubuntu3.1\nlibtiff6=4.7.0-3ubuntu3.1\nlibtiffxx6=4.7.0-3ubuntu3.1\nlibtinfo6=6.5+20250216-2build1\nlibtirpc-common=1.3.6+ds-1\nlibtirpc-dev=1.3.6+ds-1\nlibtirpc3t64=1.3.6+ds-1\nlibtk8.6=8.6.17-1\nlibts0t64=1.22-1.1build1\nlibtsan2=15.2.0-4ubuntu4\nlibtwolame0=0.4.0-2build3\nlibubsan1=15.2.0-4ubuntu4\nlibudev1=257.9-0ubuntu2.5\nlibudfread0=1.1.2-1build1\nlibunbound8=1.22.0-2ubuntu2.3\nlibunistring5=1.3-2\nlibunwind8=1.8.1-0.1ubuntu1\nliburiparser-dev=0.9.8+dfsg-2\nliburiparser1=0.9.8+dfsg-2\nlibutempter0=1.2.1-4\nlibuuid1=2.41-4ubuntu4.2\nlibva-drm2=2.22.0-3ubuntu3\nlibva-x11-2=2.22.0-3ubuntu3\nlibva2=2.22.0-3ubuntu3\nlibvdpau1=1.5-3\nlibvisual-0.4-0=0.4.2-2build1\nlibvorbis0a=1.3.7-3build1\nlibvorbisenc2=1.3.7-3build1\nlibvorbisfile3=1.3.7-3build1\nlibvpl2=1:2.15.0-1\nlibvpx9=1.15.0-2.1ubuntu0.1\nlibvulkan1=1.4.321.0-1\nlibwacom-common=2.16.1-1\nlibwacom9=2.16.1-1\nlibwayland-client0=1.24.0-1build1\nlibwayland-cursor0=1.24.0-1build1\nlibwayland-egl1=1.24.0-1build1\nlibwayland-server0=1.24.0-1build1\nlibwebp-dev=1.5.0-0.1\nlibwebp7=1.5.0-0.1\nlibwebpdecoder3=1.5.0-0.1\nlibwebpdemux2=1.5.0-0.1\nlibwebpmux3=1.5.0-0.1\nlibwebrtc-audio-processing-1-3=1.3-3build1\nlibwxbase3.2-1t64=3.2.8+dfsg-2\nlibwxgtk-gl3.2-1t64=3.2.8+dfsg-2\nlibwxgtk3.2-1t64=3.2.8+dfsg-2\nlibx11-6=2:1.8.12-1build1\nlibx11-data=2:1.8.12-1build1\nlibx11-xcb1=2:1.8.12-1build1\nlibx264-164=2:0.164.3108+git31e19f9-2build2\nlibx265-215=4.1-2\nlibxau6=1:1.0.11-1build1\nlibxaw7=2:1.0.16-1\nlibxcb-cursor0=0.1.5-1\nlibxcb-dri3-0=1.17.0-2build1\nlibxcb-glx0=1.17.0-2build1\nlibxcb-icccm4=0.4.2-1\nlibxcb-image0=0.4.0-2build1\nlibxcb-keysyms1=0.4.1-1\nlibxcb-present0=1.17.0-2build1\nlibxcb-randr0=1.17.0-2build1\nlibxcb-render-util0=0.3.10-1\nlibxcb-render0=1.17.0-2build1\nlibxcb-shape0=1.17.0-2build1\nlibxcb-shm0=1.17.0-2build1\nlibxcb-sync1=1.17.0-2build1\nlibxcb-util1=0.4.1-1\nlibxcb-xfixes0=1.17.0-2build1\nlibxcb-xinput0=1.17.0-2build1\nlibxcb-xkb1=1.17.0-2build1\nlibxcb1=1.17.0-2build1\nlibxcomposite1=1:0.4.6-1\nlibxcursor1=1:1.2.3-1\nlibxdamage1=1:1.1.6-1build1\nlibxdmcp6=1:1.1.5-1build1\nlibxerces-c-dev=3.2.4+debian-1.3build1\nlibxerces-c3.2t64=3.2.4+debian-1.3build1\nlibxext6=2:1.3.4-1build2\nlibxfixes3=1:6.0.0-2build1\nlibxfont2=1:2.0.6-1build1\nlibxft2=2.3.6-1build1\nlibxi6=2:1.8.2-1\nlibxinerama1=2:1.1.4-3build1\nlibxkbcommon-x11-0=1.7.0-2.1\nlibxkbcommon0=1.7.0-2.1\nlibxkbfile1=1:1.1.0-1build4\nlibxml2-16=2.14.5+dfsg-0.2ubuntu0.2\nlibxml2-dev=2.14.5+dfsg-0.2ubuntu0.2\nlibxmu6=2:1.1.3-3build3\nlibxmuu1=2:1.1.3-3build3\nlibxpm4=1:3.5.17-1build2\nlibxrandr2=2:1.5.4-1\nlibxrender1=1:0.9.12-1\nlibxshmfence1=1.3.3-1\nlibxslt1.1=1.1.43-0.1\nlibxss1=1:1.2.3-1build3\nlibxt6t64=1:1.2.1-1.3\nlibxtst6=2:1.2.5-1\nlibxvidcore4=2:1.3.7-1build1\nlibxxf86vm1=1:1.1.4-1build4\nlibxxhash-dev=0.8.3-2\nlibxxhash0=0.8.3-2\nlibyaml-0-2=0.2.5-2build2\nlibyuv-dev=0.0.1916.20250814-1\nlibyuv0=0.0.1916.20250814-1\nlibzip5=1.11.3-2\nlibzmq5=4.3.5-1build2\nlibzstd-dev=1.5.7+dfsg-1build2\nlibzstd1=1.5.7+dfsg-1build2\nlibzvbi-common=0.2.44-1ubuntu1\nlibzvbi0t64=0.2.44-1ubuntu1\nlinguist-qt6=6.9.2-1\nlinux-libc-dev=6.17.0-41.41\nlinux-sysctl-defaults=4.14ubuntu2\nlocales=2.42-0ubuntu3.1\nlogin.defs=1:4.17.4-2ubuntu2\nlogin=1:4.16.0-2+really2.41-4ubuntu4.2\nlogsave=1.47.2-3ubuntu2\nlto-disabled-list=69\nm4=1.4.19-8\nmake=4.4.1-2\nmanpages-dev=6.9.1-1\nmanpages=6.9.1-1\nmawk=1.3.4.20250131-1\nmedia-types=13.0.0\nmesa-libgallium=25.2.8-0ubuntu0.25.10.2\nmesa-vdpau-drivers=25.2.8-0ubuntu0.25.10.2\nmesa-vulkan-drivers=25.2.8-0ubuntu0.25.10.2\nmount=2.41-4ubuntu4.2\nmysql-common=5.8+1.1.1ubuntu1\nncurses-base=6.5+20250216-2build1\nncurses-bin=6.5+20250216-2build1\nnetbase=6.5\nnettle-dev=3.10.1-1\nnetworkd-dispatcher=2.2.4-1.1\nocl-icd-libopencl1=2.3.3-1\nopenssl-provider-legacy=3.5.3-1ubuntu3.4\nopenssl=3.5.3-1ubuntu3.4\np11-kit-modules=0.25.5-3ubuntu1\np11-kit=0.25.5-3ubuntu1\npasswd=1:4.17.4-2ubuntu2\npatch=2.8-2\nperl-base=5.40.1-6ubuntu0.1\nperl-modules-5.40=5.40.1-6ubuntu0.1\nperl=5.40.1-6ubuntu0.1\npinentry-curses=1.3.1-2ubuntu3\npinentry-gnome3=1.3.1-2ubuntu3\npkg-config=1.8.1-4build1\npkgconf-bin=1.8.1-4build1\npkgconf=1.8.1-4build1\npoppler-data=0.4.12-1\npostgresql-17-postgis-3-scripts=3.5.3+dfsg-2build1\npostgresql-17-postgis-3=3.5.3+dfsg-2build1\npostgresql-17=17.10-0ubuntu0.25.10.1\npostgresql-client-17=17.10-0ubuntu0.25.10.1\npostgresql-client-common=282ubuntu1\npostgresql-common-dev=282ubuntu1\npostgresql-common=282ubuntu1\npostgresql-postgis-scripts=3.5.3+dfsg-2build1\npostgresql-postgis=3.5.3+dfsg-2build1\npostgresql=17+282ubuntu1\nprocps=2:4.0.4-8ubuntu3\nproj-bin=9.6.0-1\nproj-data=9.6.0-1\npublicsuffix=20250328.1952-0.1\npyqt6-dev-tools=6.9.1-1build1\npython-babel-localedata=2.17.0-1\npython-matplotlib-data=3.10.1+dfsg1-4\npython3-attr=25.3.0-1\npython3-babel=2.17.0-1\npython3-brotli=1.1.0-2build5\npython3-bs4=4.13.4-3\npython3-certifi=2025.1.31+ds-1\npython3-chardet=5.2.0+dfsg-2\npython3-contourpy=1.3.1-1build1\npython3-cssselect=1.3.0-1\npython3-cycler=0.12.1-1\npython3-dateutil=2.9.0-4\npython3-dbus=1.4.0-1\npython3-decorator=5.2.1-2\npython3-dev=3.13.7-1\npython3-fastjsonschema=2.21.1-1\npython3-fonttools=4.55.3-2ubuntu0.25.10.1\npython3-fs=2.4.16-7\npython3-gdal=3.10.3+dfsg-1build2\npython3-gi=3.50.0-7\npython3-html5lib=1.2-2\npython3-idna=3.10-1\npython3-iniconfig=1.1.1-2\npython3-jinja2=3.1.6-1\npython3-json-pointer=2.4-3\npython3-jsonschema-specifications=2023.12.1-2\npython3-jsonschema=4.19.2-6ubuntu1\npython3-jupyter-core=5.7.2-5\npython3-kiwisolver=1.4.7-3build1\npython3-lxml=6.0.1-1\npython3-lz4=4.4.0+dfsg-2\npython3-markupsafe=2.1.5-1build4\npython3-matplotlib=3.10.1+dfsg1-4\npython3-minimal=3.13.7-1\npython3-mock=5.2.0-1\npython3-mpmath=1.3.0-1\npython3-nbformat=5.9.1-1\npython3-numpy-dev=1:2.2.4+ds-1ubuntu1\npython3-numpy=1:2.2.4+ds-1ubuntu1\npython3-olefile=0.47-1\npython3-owslib=0.33.0-1\npython3-packaging=25.0-1\npython3-pexpect=4.9-3\npython3-pil.imagetk=11.3.0-1ubuntu1.3\npython3-pil=11.3.0-1ubuntu1.3\npython3-pip-whl=25.1.1+dfsg-1ubuntu2\npython3-pip=25.1.1+dfsg-1ubuntu2\npython3-platformdirs=4.3.7-1\npython3-plotly=5.20.0+dfsg-1\npython3-pluggy=1.5.0-1\npython3-pooch=1.8.2-1\npython3-psycopg2=2.9.10-1build1\npython3-ptyprocess=0.7.0-6\npython3-pygments=2.18.0+dfsg-2\npython3-pyparsing=3.1.2-1\npython3-pyproj=3.7.1-1build1\npython3-pyqt6.qsci=2.14.1+dfsg-1build4\npython3-pyqt6.qtmultimedia=6.9.1-1build1\npython3-pyqt6.qtpositioning=6.9.1-1build1\npython3-pyqt6.qtserialport=6.9.1-1build1\npython3-pyqt6.qtsvg=6.9.1-1build1\npython3-pyqt6.sip=13.10.2-1\npython3-pyqt6=6.9.1-1build1\npython3-pytest=8.3.5-2\npython3-pytz=2025.2-3\npython3-qgis-common=1:4.2.3+43questing\npython3-qgis=1:4.2.3+43questing\npython3-referencing=0.36.2-1ubuntu1\npython3-requests=2.32.3+dfsg-5ubuntu2\npython3-rpds-py=0.21.0-2ubuntu2\npython3-scipy=1.15.3-1\npython3-setuptools-whl=78.1.1-0.1\npython3-soupsieve=2.7-2\npython3-sympy=1.13.3-5\npython3-tenacity=9.0.0-0ubuntu1\npython3-tk=3.13.5-1\npython3-tqdm=4.67.1-5\npython3-traitlets=5.14.3+really5.14.3-1\npython3-typing-extensions=4.13.2-1\npython3-tz=2025.2-3\npython3-ufolib2=0.17.1+dfsg1-1\npython3-unicodedata2=15.1.0+ds-1build3\npython3-urllib3=2.3.0-3ubuntu0.6\npython3-venv=3.13.7-1\npython3-webencodings=0.5.1-5\npython3-wheel=0.46.1-2\npython3-wxgtk4.0=4.2.3+dfsg-2\npython3-yaml=6.0.2-1build2\npython3.13-dev=3.13.7-1ubuntu0.4\npython3.13-minimal=3.13.7-1ubuntu0.4\npython3.13-tk=3.13.7-1ubuntu0.4\npython3.13-venv=3.13.7-1ubuntu0.4\npython3.13=3.13.7-1ubuntu0.4\npython3=3.13.7-1\nqdbus-qt6=6.9.2-1\nqdoc-qt6=6.9.2-1\nqgis-common=1:4.2.3+43questing\nqgis-plugin-grass-common=1:4.2.3+43questing\nqgis-plugin-grass=1:4.2.3+43questing\nqgis-provider-grass=1:4.2.3+43questing\nqgis-providers-common=1:4.2.3+43questing\nqgis-providers=1:4.2.3+43questing\nqgis=1:4.2.3+43questing\nqml6-module-qt-labs-qmlmodels=6.9.2+dfsg-1\nqml6-module-qtcharts=6.9.2-1\nqml6-module-qtqml-models=6.9.2+dfsg-1\nqml6-module-qtqml-workerscript=6.9.2+dfsg-1\nqml6-module-qtqml=6.9.2+dfsg-1\nqml6-module-qtquick-controls=6.9.2+dfsg-1\nqml6-module-qtquick-effects=6.9.2+dfsg-1\nqml6-module-qtquick-layouts=6.9.2+dfsg-1\nqml6-module-qtquick-shapes=6.9.2+dfsg-1\nqml6-module-qtquick-templates=6.9.2+dfsg-1\nqml6-module-qtquick-window=6.9.2+dfsg-1\nqml6-module-qtquick=6.9.2+dfsg-1\nqt6-3d-assimpsceneimport-plugin=6.9.2+dfsg-1\nqt6-3d-defaultgeometryloader-plugin=6.9.2+dfsg-1\nqt6-3d-gltfsceneio-plugin=6.9.2+dfsg-1\nqt6-3d-scene2d-plugin=6.9.2+dfsg-1\nqt6-documentation-tools=6.9.2-1\nqt6-gtk-platformtheme=6.9.2+dfsg-1ubuntu1\nqt6-image-formats-plugin-pdf=6.9.2+dfsg-2\nqt6-l10n-tools=6.9.2-1\nqt6-qpa-plugins=6.9.2+dfsg-1ubuntu1\nqt6-svg-plugins=6.9.2-1\nqt6-tools-dev-tools=6.9.2-1\nqt6-translations-l10n=6.9.2-1\nqt6-wayland=6.9.2-1\nreadline-common=8.3-3\nrpcsvc-proto=1.4.3-1\nrust-coreutils=0.2.2-0ubuntu2.1\nsed=4.9-2ubuntu0.25.10.1\nsensible-utils=0.0.25\nshared-mime-info=2.4-5build2\nsse3-support=27ubuntu1\nssl-cert=1.1.3ubuntu1\nsystemd-cryptsetup=257.9-0ubuntu2.5\nsystemd-resolved=257.9-0ubuntu2.5\nsystemd-sysv=257.9-0ubuntu2.5\nsystemd-timesyncd=257.9-0ubuntu2.5\nsystemd=257.9-0ubuntu2.5\nsysvinit-utils=3.14-4ubuntu1\ntar=1.35+dfsg-3.1build1\ntk8.6-blt2.5=2.5.3+dfsg-8\ntzdata=2026b-0ubuntu0.25.10.1\nubuntu-keyring=2023.11.28.1\nucf=3.0052\nunicode-data=15.1.0-1\nunixodbc-common=2.3.12-2ubuntu2\nunixodbc-dev=2.3.12-2ubuntu2\nunzip=6.0-28ubuntu7\nutil-linux=2.41-4ubuntu4.2\nvdpau-driver-all=1.5-3\nwget=1.25.0-2ubuntu3\nx11-common=1:7.7+24ubuntu1\nx11-xkb-utils=7.7+9\nxauth=1:1.1.2-1.1\nxbitmaps=1.1.1-2.2\nxdg-user-dirs=0.18-3\nxfonts-base=1:1.0.5+nmu1\nxfonts-encodings=1:1.0.5-0ubuntu2\nxfonts-utils=1:7.7+7\nxkb-data=2.42-1ubuntu1\nxserver-common=2:21.1.18-1ubuntu1.1\nxterm=402-1ubuntu1\nxvfb=2:21.1.18-1ubuntu1.1\nxz-utils=5.8.1-1ubuntu0.1\nzlib1g-dev=1:1.3.dfsg+really1.3.1-1ubuntu2\nzlib1g=1:1.3.dfsg+really1.3.1-1ubuntu2\n",
  "container_image": "sha256:ae67a07a90cdbe5ac530afc0076633954c74b4108ba1f7beb4d18a7fc15759cf",
  "rust_binary_sha256": "a7f6956bac1b4e0a80aa5b733b35f105a48a3b8ecfc0c4431fe61c9175fc5bbd",
  "sizes": [
    1000
  ],
  "repetitions": 10,
  "warmups": 2,
  "timeout_s": 60,
  "startup_timeout_s": 180,
  "crs": "EPSG:3857",
  "tolerance": 0,
  "rust_threads": 1,
  "headline_geometry_scope": "Single points, lines and polygons, including bounded holes and islands",
  "scope": {
    "included": "fresh indexes or topology, rule evaluation, materialized violation geometries",
    "excluded": "application startup, source loading, fresh copies, response extraction, serialization",
    "grass": "native module launch and native vector output writes included",
    "postgis": "server-side SQL workflows including GiST indexes and ANALYZE",
    "qgis": "C++ containment plugin or native indexed geometry workflow with error geometries"
  },
  "finished_at": "2026-10-13T06:04:32.491316+00:00"
}
```
