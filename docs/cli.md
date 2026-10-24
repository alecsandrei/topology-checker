# CLI

## Check one rule

```sh
topology-checker <group> <rule> <input> [--layer NAME] [--other INPUT] [--other-layer NAME] [-o OUTPUT]
```

```sh
topology-checker line must-not-have-dangles roads.gpkg --layer roads -o errors.gpkg
topology-checker point must-be-covered-by-lines stops.geojson --other routes.gpkg
```

Groups are `point`, `line`, `polygon` and `geometry`. See [Rules](rules.md). Without `--layer`, each layer in the input is checked separately. Two-layer rules check every input layer against every `--other` layer.

## Check a topology definition

```toml
tolerance = 0.001

[layers]
roads = { path = "city.gpkg", layer = "roads" }
stops = { path = "stops.geojson" }

[[rules]]
rule = "line/must-not-have-dangles"
layer = "roads"

[[rules]]
rule = "point/must-be-covered-by-lines"
layer = "stops"
other = "roads"
```

```sh
topology-checker validate topology.toml -o errors.gpkg
```

`tolerance` and `layer` are optional. Paths are relative to the TOML file. Unknown keys are errors.

## Tolerance and CRS

Tolerance is in coordinate units, and calculations are planar. `--tolerance` overrides the TOML value. If neither is set, the default depends on the CRS:

- projected CRS: 1 mm;
- geographic CRS: `1e-8` degrees;
- unknown CRS, or any input without one: 0.

Within the tolerance, coordinates are treated as coincident, so a near miss within tolerance is a touch, not a dangle or an overlap. Set the tolerance to the data's positional precision. Tolerance 0 is exact on the binary coordinates, so floating-point slivers become reportable.

Inputs must share a CRS; the CLI does not reproject. GeoJSON without a `crs` member is EPSG:4326. A geographic CRS with a nonzero tolerance prints a warning, because calculations are planar.

## Output files

`-o` writes `errors_point`, `errors_line` and `errors_polygon` layers. The format comes from the file extension or from `--driver`; `topology-checker drivers --write` lists the formats. Nothing is written when there are no violations. An existing file needs `--overwrite`.

Columns: `violation_id`, `rule`, `error_type`, `origin_layer`, `origin_fid`, `origin_part`, `destination_layer`, `destination_fid`, `destination_part`. A violation with several geometry types gets a row in each layer, with the same `violation_id`. Shapefiles shorten the names to `viol_id`, `orig_layer`, `orig_part`, `dest_layer`, `dest_fid` and `dest_part`.

## Output and exit codes

- stdout: a summary table, or the report as JSON with `--json`. With `--summary`, the JSON leaves out the list of violations and keeps the counts.
- `validate --stream` needs `--json`. It prints each entry of `rules` to stdout on its own line as the rule finishes, then a last line with `tolerance` and `warnings`.
- stderr: warnings, and any error. With `--json`, an error is printed as `{"error": "..."}`. `validate --progress` also prints `checked rule K/N (<rule>): C errors` as each rule finishes.

| Exit code | Meaning |
| --- | --- |
| 0 | No violations |
| 1 | Violations found |
| 2 | Error |

## Other commands

```sh
topology-checker rules                    # add --json for JSON
topology-checker drivers --read --write
topology-checker utilities merge-lines roads.gpkg --layer roads -o merged.gpkg
topology-checker utilities explode-lines roads.gpkg --layer roads -o segments.gpkg
```

`merge-lines` joins lines at endpoints shared by exactly two lines. `explode-lines` splits lines into single segments.
