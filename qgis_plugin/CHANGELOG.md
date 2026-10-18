# Changelog

## 0.2.0

* Processing algorithms: one per rule, with errors as three sinks, and **Check topology definition** for a `topology.toml`.
* The panel queues rules on layers and runs them in one pass; the errors are a tree by rule and kind, with zoom, selection and a map filter.
* Rules can be switched off, exported to and imported from a `topology.toml`, and are saved with the project.
* Invalid geometries are reported and left out of the run.
* The tool is checked in the background, and can be chosen in the panel.
* Rules can check only the selected features or the visible extent, and errors can be ignored; ignored errors are saved with the project.
* Export saves layers that are not files of their own in a GeoPackage next to the `topology.toml`.
