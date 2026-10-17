use std::path::Path;
use topology_checker_core::{Layer, Rule, Topology, geo::Point};
use topology_checker_io::{registry, report_store::DiskReport};

#[test]
fn project_archives_and_native_readable_extensions_are_recognised() {
    for extension in ["zip", "geojson", "gpkg", "shp", "kml", "gpx", "parquet"] {
        assert!(registry::project_extension(extension), "{extension}");
    }
    for extension in ["txt", "png", ""] {
        assert!(!registry::project_extension(extension), "{extension}");
    }
}

#[test]
fn unknown_output_formats_are_rejected() {
    assert!(registry::writable(Path::new("out.unknown"), None).is_err());
    assert!(registry::writable(Path::new("out.geojson"), Some("nonsense")).is_err());
    assert!(registry::writable(Path::new("out"), Some("parquet")).is_ok());
    assert!(registry::writable(Path::new("out"), Some("libkml")).is_ok());
}

#[cfg(feature = "gdal-system")]
#[test]
fn read_only_gdal_drivers_cannot_create_datasets() {
    let error = registry::writable(Path::new("out.vrt"), None).unwrap_err();
    assert!(error.to_string().contains("cannot create"), "{error}");
}

#[test]
fn removed_report_rows_surface_as_errors() {
    let root = tempfile::tempdir().unwrap();
    let mut topology = Topology::new(0.).unwrap();
    let layer = topology.add_layer(Layer::from_features(
        "points",
        [1, 2].map(|fid| (fid, Point::new(5., 52.).into())),
    ));
    topology
        .add_rule(Rule::PointMustNotOverlap, layer, None)
        .unwrap();
    let report = DiskReport::validate(&topology, Some(root.path())).unwrap();
    let count = report.violation_count();
    assert!(count > 0);
    assert!(report.row(0).is_ok());
    for entry in std::fs::read_dir(root.path()).unwrap() {
        std::fs::remove_file(entry.unwrap().path().join("rows.bin")).unwrap();
    }
    assert!(report.row(0).is_err());
    let out = root.path().join("lost.geojson");
    assert!(
        topology_checker_io::write::stored_report(
            &out,
            None,
            false,
            None,
            &report,
            topology_checker_io::write::ReportLayout::Complete
        )
        .is_err()
    );
    assert!(!out.exists());
    let rows = report.rows().collect::<Vec<_>>();
    assert_eq!(rows.len(), count);
    assert!(rows.iter().all(|row| row.is_err()));
}
