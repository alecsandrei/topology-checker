use topology_checker_core::{Layer, Rule, Topology, geo::Point};
use topology_checker_io::{
    crs, read,
    report_store::DiskReport,
    write::{self, ReportLayout},
};

#[test]
fn binary_disk_rows_random_access_and_every_export_match_eager() {
    let root = tempfile::tempdir().unwrap();
    let mut topology = Topology::new(0.).unwrap();
    let layer = topology.add_layer(Layer::from_features(
        "points",
        [i64::MIN, 42, i64::MAX].map(|fid| (fid, Point::new(5., 52.).into())),
    ));
    topology
        .add_rule(Rule::PointMustNotOverlap, layer, None)
        .unwrap();
    let eager = topology.validate().unwrap();
    let stored = DiskReport::validate(&topology, Some(root.path())).unwrap();
    assert_eq!(stored.violation_count(), 3);
    for i in [2, 0, 1] {
        assert_eq!(stored.row(i).unwrap().1, eager.rules[0].violations[i]);
    }
    assert!(stored.row(3).is_err());
    assert_eq!(
        stored.rows().map(|r| r.unwrap().1).collect::<Vec<_>>(),
        eager.rules[0].violations
    );
    let srs = crs::parse("EPSG:4326").unwrap();
    for extension in [
        "geojson", "gpkg", "fgb", "parquet", "shp", "kml", "kmz", "gpx",
    ] {
        for layout in [ReportLayout::Complete, ReportLayout::ByFamily] {
            let a = root.path().join(format!("eager-{layout:?}.{extension}"));
            let b = root.path().join(format!("stored-{layout:?}.{extension}"));
            let a = write::report(&a, None, false, Some(&srs), &eager, layout);
            let b = write::stored_report(&b, None, false, Some(&srs), &stored, layout);
            if extension == "gpx" && layout == ReportLayout::ByFamily {
                assert!(
                    a.is_err() && b.is_err(),
                    "GPX retains its unsupported multipoint policy"
                );
                continue;
            }
            let a = a.unwrap();
            let b = b.unwrap();
            assert_eq!(a.len(), b.len(), "{extension}");
            for (a, b) in a.iter().zip(&b) {
                let a = read::read_with_attributes(a, None, "report", true).unwrap();
                let b = read::read_with_attributes(b, None, "report", true).unwrap();
                assert_eq!(a.len(), b.len());
                for (a, b) in a.iter().zip(&b) {
                    assert_eq!(a.srs, b.srs, "{extension}");
                    assert_eq!(a.attributes, b.attributes, "{extension}");
                    assert_eq!(
                        a.layer.features().collect::<Vec<_>>(),
                        b.layer.features().collect::<Vec<_>>(),
                        "{extension}"
                    );
                }
            }
        }
    }
    drop(stored);
    assert!(!std::fs::read_dir(root.path()).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("topology-report-")
    }));
}

#[test]
fn precision_failure_cleans_a_disk_report_with_previously_written_rows() {
    use topology_checker_core::{
        Error,
        geo::{LineString, Polygon, Rect, coord},
    };
    let root = tempfile::tempdir().unwrap();
    let mut topology = Topology::new(0.).unwrap();
    let points = topology.add_layer(Layer::from_geometries(
        "duplicates",
        [Point::new(0., 0.).into(), Point::new(0., 0.).into()],
    ));
    topology
        .add_rule(Rule::PointMustNotOverlap, points, None)
        .unwrap();
    let x = 1f64.next_up();
    let polygons = topology.add_layer(Layer::from_geometries(
        "narrow",
        [
            Rect::new(coord! {x:1.,y:0.}, coord! {x:x,y:1.})
                .to_polygon()
                .into(),
            Polygon::new(
                LineString::from(vec![(1., -1.), (x, -1.), (1., 1.), (1., -1.)]),
                vec![],
            )
            .into(),
        ],
    ));
    topology
        .add_rule(Rule::PolygonMustNotHaveGaps, polygons, None)
        .unwrap();
    let error = DiskReport::validate(&topology, Some(root.path())).unwrap_err();
    assert!(matches!(
        error.downcast_ref::<Error>(),
        Some(Error::Precision { .. })
    ));
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}
