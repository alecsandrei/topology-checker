Validate planar GIS data with 24 typed constraints powered by 38 native Rust rules.

```sh
uv add 'topology-checker[geopandas]'
```

```python
import geopandas as gpd
from topology_checker import Rule, Topology

roads_gdf = gpd.read_file("roads.gpkg")
topology = Topology(tolerance=0.001)
topology.add_rule(Rule.MUST_NOT_HAVE_DANGLES, roads_gdf)
buildings_gdf = gpd.read_file("buildings.gpkg")
topology.add_rule(Rule.MUST_NOT_OVERLAP, roads_gdf, other=buildings_gdf)
report = topology.validate()
errors = report.to_geodataframe()
```

```python
from shapely.geometry import Point, box
from topology_checker import GeometryKind, Rule, Topology

topology = Topology()
points = topology.add_layer([Point(0.5, 0.5)], fids=[42], crs="EPSG:28992")
areas = topology.add_layer([box(0, 0, 1, 1)], name="parcels", crs="EPSG:28992")
topology.add_rule(Rule.MUST_BE_INSIDE, points, other=areas)
report = topology.validate()
print(report.is_valid, report.error_count, report.warnings)
print(report.to_dict())
print(topology.layer(points).name)
print(report.rules[0].resolved_rule.description)
empty_lines = topology.add_layer([], kind=GeometryKind.LINE)
topology.add_rule(Rule.MUST_NOT_HAVE_DANGLES, empty_lines)
```

```python
from shapely.geometry import LineString
from topology_checker import explode_lines, merge_lines

lines = [LineString([(0, 0), (1, 0)]), LineString([(1, 0), (2, 0)])]
merged = merge_lines(lines, tolerance=0.001)
segments = explode_lines(merged)
```
