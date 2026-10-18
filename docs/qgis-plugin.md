# QGIS plugin

**Topology Rules** adds the rules of topology-checker to QGIS. It runs the [CLI](cli.md) and does not include it. Each [rule](rules.md) is a Processing tool, and a panel runs several rules at once and lists their errors.

## Install

1. Install the [CLI](install.md#cli), 0.2.0 or newer.
2. In QGIS 3.34 or newer, open **Plugins > Manage and Install Plugins**, tick **Show also experimental plugins** in the Settings tab, and install **Topology Rules**.
3. If `topology-checker` is not on the `PATH`, choose it in the panel, or set its path under **Settings > Options > Processing > Providers > Topology Rules**.

## Panel

Open **Topology Rules** from the Vector menu or the toolbar.

1. Pick a layer and a rule, then **Add rule**. Rules that compare two layers also ask for the other layer. Repeat for more rules.
2. **Run** checks all the ticked rules in one pass.

The errors are a tree by rule and kind. Selecting one zooms to it, highlights it and filters the map to its group. **Select features** selects the features an error is about, and **Ignore error** hides an error until you stop ignoring it, also in later runs.

Under **Options**, **Selected features only** and **Only the visible extent** limit the layer each rule checks, not the layer it is checked against. **Export…** writes the ticked rules as a `topology.toml` for the [CLI](cli.md#check-a-topology-definition), saving layers that are not files of their own in a GeoPackage next to it. **Import…** reads one. The list, the options and the ignored errors are saved with the project.

## Processing

Each rule is a tool named after it, such as **Lines must not have dangles**, in the Topology Rules provider, grouped as Points, Lines, Polygons and All geometries. It adds the errors to the project as point, line and polygon layers. With **Selected features only**, only the selected features are checked. The second layer of a two-layer rule is reprojected to the CRS of the first.

**Check topology definition** runs a `topology.toml`. The tools also run without a window:

```sh
qgis_process run topologyrules:line_must_not_have_dangles --INPUT=roads.gpkg --ERRORS_POINT=errors.gpkg
```

## Tolerance and invalid geometries

The tolerance is in the units of the first layer's CRS. Left empty, it is the CLI default for that CRS: see [Tolerance and CRS](cli.md#tolerance-and-crs).

Features the tool cannot read, such as invalid polygons or coordinates that are not finite, are left out of the run and listed as errors of their own.

## Develop

See **Plugin development** in the [README](https://github.com/alecsandrei/topology-checker#plugin-development).
