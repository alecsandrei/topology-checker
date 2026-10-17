use std::path::Path;
use topology_checker_core::{
    Layer, Rule, Topology,
    geo::{Geometry, LineString, Point, polygon},
};
use topology_checker_io::{
    crs,
    report_store::DiskReport,
    write::{self, DatasetLayer, Record, ReportLayout},
};

fn topology(valid: bool) -> Topology {
    let mut topology = Topology::new(0.).unwrap();
    let offset = if valid { 10. } else { 0. };
    let points = topology.add_layer(Layer::from_features(
        "points",
        [
            (1, Point::new(5., 52.).into()),
            (2, Point::new(5. + offset, 52.).into()),
        ],
    ));
    let lines = topology.add_layer(Layer::from_features(
        "lines",
        [
            (1, LineString::from(vec![(0., 0.), (4., 0.)]).into()),
            (
                2,
                LineString::from(vec![(2. + offset, 0.), (6. + offset, 0.)]).into(),
            ),
        ],
    ));
    let square = |x: f64| -> Geometry {
        polygon![(x: x, y: 0.), (x: x + 2., y: 0.), (x: x + 2., y: 2.), (x: x, y: 2.)].into()
    };
    let polygons = topology.add_layer(Layer::from_features(
        "polygons",
        [(1, square(0.)), (2, square(1. + offset))],
    ));
    for (rule, layer) in [
        (Rule::PointMustNotOverlap, points),
        (Rule::LineMustNotOverlap, lines),
        (Rule::PolygonMustNotOverlap, polygons),
    ] {
        topology.add_rule(rule, layer, None).unwrap();
    }
    topology
}
fn stored(topology: &Topology, dir: &Path) -> DiskReport {
    DiskReport::validate(topology, Some(dir)).unwrap()
}
fn point_layer() -> DatasetLayer {
    DatasetLayer::new(
        "points",
        Some(crs::parse("EPSG:4326").unwrap()),
        vec![Record {
            id: Some(1),
            geometry: Some(Point::new(5., 52.).into()),
            properties: Default::default(),
        }],
    )
}

#[test]
fn family_reports_write_one_file_per_geometry_family_and_refuse_existing_files() {
    let dir = tempfile::tempdir().unwrap();
    let srs = crs::parse("EPSG:4326").unwrap();
    let topology = topology(false);
    let report = topology.validate().unwrap();
    let disk = stored(&topology, dir.path());
    let path = dir.path().join("out.geojson");
    let written = write::report(
        &path,
        None,
        false,
        Some(&srs),
        &report,
        ReportLayout::ByFamily,
    )
    .unwrap();
    let names = written
        .iter()
        .map(|p| p.file_name().unwrap().to_str().unwrap().to_string())
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        [
            "out_line.geojson",
            "out_point.geojson",
            "out_polygon.geojson"
        ]
    );
    let error = write::report(
        &path,
        None,
        false,
        Some(&srs),
        &report,
        ReportLayout::ByFamily,
    )
    .unwrap_err();
    assert!(format!("{error:#}").contains("already exists"), "{error:#}");
    write::report(
        &path,
        None,
        true,
        Some(&srs),
        &report,
        ReportLayout::ByFamily,
    )
    .unwrap();
    let error = write::stored_report(
        &path,
        None,
        false,
        Some(&srs),
        &disk,
        ReportLayout::ByFamily,
    )
    .unwrap_err();
    assert!(format!("{error:#}").contains("already exists"), "{error:#}");

    for explicit_path in [
        dir.path().join("no-extension"),
        Path::new("..").to_path_buf(),
    ] {
        let error = write::report(
            &explicit_path,
            Some("GeoJSON"),
            false,
            Some(&srs),
            &report,
            ReportLayout::ByFamily,
        )
        .unwrap_err();
        let message = format!("{error:#}");
        assert!(
            message.contains("extension required") || message.contains("invalid output filename"),
            "{message}"
        );
    }
}

#[test]
fn valid_reports_write_nothing_by_family() {
    let dir = tempfile::tempdir().unwrap();
    let topology = topology(true);
    let report = topology.validate().unwrap();
    assert!(report.is_valid());
    let path = dir.path().join("clean.geojson");
    assert!(
        write::report(&path, None, false, None, &report, ReportLayout::ByFamily)
            .unwrap()
            .is_empty()
    );
    let disk = stored(&topology, dir.path());
    assert!(
        write::stored_report(&path, None, false, None, &disk, ReportLayout::ByFamily)
            .unwrap()
            .is_empty()
    );
    assert!(!path.exists());
}

#[test]
fn sibling_files_and_directories_are_never_overwritten_silently() {
    let dir = tempfile::tempdir().unwrap();
    let srs = crs::parse("EPSG:4326").unwrap();
    let topology = topology(false);
    let report = topology.validate().unwrap();
    let disk = stored(&topology, dir.path());

    // A shapefile also owns its .dbf: an unrelated one must stop the write.
    std::fs::write(dir.path().join("data.dbf"), "keep").unwrap();
    let target = dir.path().join("data.shp");
    let error = write::dataset(&target, None, false, &[point_layer()]).unwrap_err();
    assert!(format!("{error:#}").contains("already exists"), "{error:#}");
    // Family outputs of a shapefile report are checked file by file as well.
    std::fs::write(dir.path().join("data_point.dbf"), "keep").unwrap();
    let error = write::stored_report(
        &target,
        None,
        false,
        Some(&srs),
        &disk,
        ReportLayout::ByFamily,
    )
    .unwrap_err();
    assert!(format!("{error:#}").contains("already exists"), "{error:#}");
    let error = write::report(
        &target,
        None,
        false,
        Some(&srs),
        &report,
        ReportLayout::ByFamily,
    )
    .unwrap_err();
    assert!(format!("{error:#}").contains("already exists"), "{error:#}");
    for name in ["data.dbf", "data_point.dbf"] {
        assert_eq!(
            std::fs::read_to_string(dir.path().join(name)).unwrap(),
            "keep"
        );
    }
    assert!(!target.exists());

    let folder = dir.path().join("folder");
    std::fs::create_dir(&folder).unwrap();
    let error = write::dataset(&folder, Some("GeoJSON"), true, &[point_layer()]).unwrap_err();
    assert!(format!("{error:#}").contains("is a directory"), "{error:#}");
}
