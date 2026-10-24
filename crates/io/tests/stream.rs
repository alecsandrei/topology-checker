use topology_checker_core::{Layer, Rule, Topology, geo::Point};
use topology_checker_io::{
    crs, read,
    report_store::DiskReport,
    write::{self, ReportLayout},
};

#[test]
fn each_finished_rule_hands_over_its_own_stored_rows() {
    let root = tempfile::tempdir().unwrap();
    let mut topology = Topology::new(0.).unwrap();
    let layer = |name: &str, fids: &[i64]| {
        Layer::from_features(
            name,
            fids.iter()
                .map(|&fid| (fid, Point::new(5., 52.).into()))
                .collect::<Vec<_>>(),
        )
    };
    let points = topology.add_layer(layer("points", &[1, 2, 3]));
    let single = topology.add_layer(layer("single", &[7]));
    let copies = topology.add_layer(layer("copies", &[4, 5]));
    for (a, b) in [(points, None), (single, None), (points, Some(copies))] {
        let rule = if b.is_some() {
            Rule::PointMustNotOverlapWith
        } else {
            Rule::PointMustNotOverlap
        };
        topology.add_rule(rule, a, b).unwrap();
    }
    topology
        .add_rule(Rule::PointMustNotOverlap, copies, None)
        .unwrap();
    let mut seen = Vec::new();
    let stored = DiskReport::validate_with_progress(&topology, Some(root.path()), |rule, rows| {
        assert_eq!(rows.size_hint(), (rule.total, Some(rule.total)));
        // Rows left unread, or read only in part, do not shift later rules.
        let rows = match rule.spec.id.index() {
            0 => rows.take(1).collect::<anyhow::Result<Vec<_>>>()?,
            3 => Vec::new(),
            _ => rows.collect::<anyhow::Result<Vec<_>>>()?,
        };
        seen.push((rule.spec.id.index(), rule.total, rows));
        Ok(())
    })
    .unwrap();
    let totals = seen.iter().map(|(i, n, _)| (*i, *n)).collect::<Vec<_>>();
    assert_eq!(totals, [(0, 3), (1, 0), (2, 6), (3, 1)]);
    let mut all = stored.rows().map(Result::unwrap).collect::<Vec<_>>();
    for (index, total, rows) in seen {
        let rest = all.split_off(total);
        let expected = std::mem::replace(&mut all, rest);
        assert!(expected.iter().all(|(rule, _)| *rule == index));
        let expected = expected.into_iter().map(|(_, v)| v);
        let expected = match index {
            0 => expected.take(1).collect::<Vec<_>>(),
            3 => Vec::new(),
            _ => expected.collect(),
        };
        assert_eq!(rows, expected, "rule {index}");
    }
    assert!(all.is_empty());
}

#[test]
fn a_failing_rule_end_stops_validation_and_removes_the_rows() {
    let root = tempfile::tempdir().unwrap();
    let mut topology = Topology::new(0.).unwrap();
    let points = topology.add_layer(Layer::from_features(
        "points",
        [1, 2].map(|fid| (fid, Point::new(5., 52.).into())),
    ));
    for _ in 0..2 {
        topology
            .add_rule(Rule::PointMustNotOverlap, points, None)
            .unwrap();
    }
    let mut ended = 0;
    let error = DiskReport::validate_with_progress(&topology, Some(root.path()), |_, mut rows| {
        ended += 1;
        rows.next().unwrap()?;
        anyhow::bail!("output closed")
    })
    .unwrap_err();
    assert_eq!(error.to_string(), "output closed");
    assert_eq!(ended, 1);
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}

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
