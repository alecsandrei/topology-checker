# RustGIS plugin

topology-checker is available as a plugin for [RustGIS](https://github.com/opengeos/RustGIS), the open-source desktop GIS. It runs as a sandboxed WebAssembly [plugin](https://github.com/opengeos/RustGIS/blob/main/docs/plugins.md) built on the same engine as the CLI and Python, and adds one tool for each of the [38 rules](rules.md).

## Install

1. Download `topology-checker-<version>.rgplugin` from the [latest release](https://github.com/alecsandrei/topology-checker/releases/latest). Do not unzip it.
2. In RustGIS open the Plugin Manager and choose **Install from File**.
3. Pick the `.rgplugin` file. The plugin asks for no permissions.

The tools appear on a **Topology** ribbon tab, grouped as Points, Lines, Polygons and Geometry, each with a menu for rules on one layer and a menu for rules that compare two layers. They are also listed with RustGIS's other geoprocessing tools as `plugin:topology-checker/<tool>`.

## Use a tool

Each tool takes the layer to check, a second layer for the rules that compare two, and a tolerance. Tool ids are the rule names with the slash replaced by a hyphen: `polygon/must-not-have-gaps` is `polygon-must-not-have-gaps`.

- If the layer has a selection, only the selected features are checked.
- Features without geometry are skipped.
- The result is a new layer named `<layer>: <rule>`. Each feature is a violation with `rule`, `error_type`, `origin_fid`, `origin_part`, `destination_fid` and `destination_part`. `origin_fid` is the source feature's `row` when RustGIS provides one, else its position in the checked features. Clean data adds no layer.
- Messages list the violation counts and any warnings, such as line parts that collapsed under the tolerance.
- Choosing the same layer twice for a two-layer rule is an error.

## Tolerance

RustGIS gives tools longitude and latitude, and the engine is planar, so the tolerance is in degrees. The default, `0.000001`, is about 11 cm at the equator. A degree of longitude is shorter away from the equator, so tolerances are not equal in every direction. Use the [CLI](cli.md) or [Python](python-usage.md) on projected data when you need a tolerance in metres.

## Limits

The plugin runs in a RustGIS sandbox with 512 MB of memory and an instruction budget per tool run. In a smoke test through the plugin interface in Node, 10,000 polygons took about 0.4 s for the overlap rule and 170 MB peak. Layers of several hundred thousand features may exceed the sandbox limits. Use the [CLI](cli.md) for those.

Invalid polygons are rejected with the feature and ring, as in the other front ends.

## Build from source

```sh
rustup target add wasm32-unknown-unknown
python3 scripts/package_rustgis_plugin.py          # dist/topology-checker-<version>.rgplugin
node scripts/rustgis_plugin_smoke.mjs target/wasm32-unknown-unknown/rustgis-plugin/topology_checker_rustgis.wasm
```

`crates/rustgis-plugin/plugin.json` is generated from the rule registry, and a test fails when it is stale:

```sh
cargo run -p topology-checker-rustgis --example manifest > crates/rustgis-plugin/plugin.json
```

The plugin SDK needs Rust 1.90, newer than the workspace's 1.88, so the MSRV check skips this crate.
