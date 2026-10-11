# Web app

```sh
topology-checker-web                 # http://127.0.0.1:8080
topology-checker-web --port 9000     # or TOPOLOGY_WEB_PORT=9000; 0 picks a free port
```

[Install](install.md#web-app) builds `target/release/topology-checker-web`. It listens on 127.0.0.1 only and rejects requests with another Host or Origin.

1. Import a file of up to 512 MiB: GeoJSON, GeoPackage, FlatGeobuf, GeoParquet, KML, KMZ, GPX, or a zipped Shapefile. Or open one of the [bundled examples](https://github.com/alecsandrei/topology-checker/blob/master/crates/server/examples/README.md).
2. Choose layers, a working CRS, a tolerance and rules.
3. Run the checks, then inspect violations in the table and on the map.
4. Export violations as one `violations` layer with the [CLI columns](cli.md#output-files). GeoJSON, KML, KMZ and GPX use WGS84. GeoPackage, FlatGeobuf and GeoParquet use the working CRS. A JSON report is also available.
5. Save a project zip to reopen the setup later.

Each browser tab has its own workspace. Workspaces idle for 4 hours are deleted.
