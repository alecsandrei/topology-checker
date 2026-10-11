use std::sync::Arc;
use topology_checker_core::{
    Error, Layer, Rule, Topology, ViolationKind, Warning, explode_lines,
    geo::{Geometry, LineString, Point, Rect, coord},
    merge_lines,
};

fn point(x: f64, y: f64) -> Geometry {
    Point::new(x, y).into()
}
fn square(x: f64) -> Geometry {
    Rect::new(coord! {x: x, y: 0.}, coord! {x: x + 2., y: 2.})
        .to_polygon()
        .into()
}
fn line(coords: &[(f64, f64)]) -> Geometry {
    LineString::from(coords.to_vec()).into()
}

#[test]
fn binary_rules_distinguish_layers_with_shared_fids() {
    for (rule, a, b, kind) in [
        (
            Rule::PointMustNotOverlapWith,
            point(1., 1.),
            point(1., 1.),
            ViolationKind::Overlap,
        ),
        (
            Rule::PolygonMustNotOverlapWith,
            square(0.),
            square(1.),
            ViolationKind::Overlap,
        ),
        (
            Rule::PolygonMustNotContainPoints,
            square(0.),
            point(1., 1.),
            ViolationKind::Contained,
        ),
        (
            Rule::PointMustNotIntersectLines,
            point(1., 1.),
            line(&[(0., 1.), (2., 1.)]),
            ViolationKind::Intersection,
        ),
    ] {
        let mut t = Topology::new(0.).unwrap();
        let first = t.add_layer(Layer::from_features("a", [(7, a)]));
        let second = t.add_layer(Layer::from_features("b", [(7, b)]));
        t.add_rule(rule, first, Some(second)).unwrap();
        let report = t.validate().unwrap();
        assert_eq!(report.violation_count(), 1, "{rule}");
        let v = report.violations().next().unwrap();
        assert_eq!(v.kind, kind, "{rule}");
        assert_eq!(
            v.origin.map(|r| (r.layer, r.fid)),
            Some((first, 7)),
            "{rule}"
        );
        assert_eq!(
            v.destination.map(|r| (r.layer, r.fid)),
            Some((second, 7)),
            "{rule}"
        );
    }
}

#[test]
fn shared_layers_report_each_topology_layer_id() {
    let points = Arc::new(Layer::from_features(
        "points",
        [(3, point(0., 0.)), (4, point(0., 0.))],
    ));
    let mut first = Topology::new(0.).unwrap();
    let a = first.add_layer(Arc::clone(&points));
    let mut second = Topology::new(0.).unwrap();
    second.add_layer(Layer::from_geometries("padding", []));
    let b = second.add_layer(Arc::clone(&points));
    assert_ne!(a, b);
    for (t, id) in [(&mut first, a), (&mut second, b)] {
        t.add_rule(Rule::PointMustNotOverlap, id, None).unwrap();
        let report = t.validate().unwrap();
        let v = report.violations().next().unwrap();
        assert_eq!(v.origin.unwrap().layer, id);
        assert_eq!(v.destination.unwrap().layer, id);
    }
    assert_eq!(Arc::strong_count(&points), 3);
}

#[test]
fn registration_reports_same_layer_and_layer_ids_in_errors() {
    let mut t = Topology::new(0.).unwrap();
    let roads = t.add_layer(Layer::from_geometries(
        "roads",
        [line(&[(0., 0.), (1., 0.)])],
    ));
    let error = t
        .add_rule(Rule::LineMustNotOverlapWith, roads, Some(roads))
        .unwrap_err();
    assert!(matches!(error, Error::SameLayer { layer, .. } if layer == roads));
    assert_eq!(
        error.to_string(),
        "line/must-not-overlap-with requires two distinct input layers"
    );
    assert!(matches!(
        t.add_rule(Rule::LineMustNotOverlapWith, roads, None),
        Err(Error::Inputs { expected: 2, .. })
    ));

    let points = t.add_layer(Layer::from_features("data", [(5, point(0., 0.))]));
    let error = t
        .add_rule(Rule::LineMustNotHaveDangles, points, None)
        .unwrap_err();
    assert!(matches!(
        &error,
        Error::GeometryKind { layer, name, fid: 5, .. } if *layer == points && name == "data"
    ));
    assert_eq!(
        error.to_string(),
        "layer data, FID 5: expected Line, found Point"
    );
    let duplicates = t.add_layer(Layer::from_features(
        "data",
        [(1, point(0., 0.)), (1, point(1., 0.))],
    ));
    assert!(matches!(
        t.add_rule(Rule::PointMustNotOverlap, duplicates, None),
        Err(Error::DuplicateFid { layer, fid: 1, .. }) if layer == duplicates
    ));
    let nonfinite = t.add_layer(Layer::from_features("bad", [(2, point(f64::NAN, 0.))]));
    assert!(matches!(
        t.add_rule(Rule::PointMustNotOverlap, nonfinite, None),
        Err(Error::NonFinite { layer, fid: 2, .. }) if layer == nonfinite
    ));
}

#[test]
fn line_utilities_report_input_positions() {
    let lines = || {
        [
            LineString::from(vec![(0., 0.), (1., 0.)]),
            LineString::from(vec![(0., 0.), (f64::INFINITY, 1.)]),
        ]
    };
    for error in [
        merge_lines(lines(), 0.).unwrap_err(),
        explode_lines(lines()).unwrap_err(),
    ] {
        assert!(matches!(error, Error::NonFiniteLine { index: 1 }));
        assert_eq!(error.to_string(), "line 1: coordinates must be finite");
    }
}

#[test]
fn warnings_are_typed_with_stable_text() {
    let mut t = Topology::new(0.).unwrap();
    t.add_layer(Layer::from_geometries(
        "clean",
        [line(&[(0., 0.), (1., 0.)])],
    ));
    let roads = t.add_layer(Layer::from_geometries(
        "roads",
        [line(&[(0., 0.), (0., 0.)]), line(&[(0., 0.), (1., 0.)])],
    ));
    let warnings = t.warnings().unwrap();
    assert!(matches!(
        warnings.as_slice(),
        [Warning::DegenerateParts { layer, name, skipped: 1 }] if *layer == roads && name == "roads"
    ));
    assert_eq!(
        warnings[0].to_string(),
        "layer roads: skipped 1 degenerate parts or rings"
    );
    assert_eq!(t.validate().unwrap().warnings, warnings);
}

#[test]
fn rule_ids_locate_reports_including_duplicate_registrations() {
    let mut t = Topology::new(0.).unwrap();
    let roads = t.add_layer(Layer::from_geometries(
        "roads",
        [line(&[(0., 0.), (1., 0.)])],
    ));
    let points = t.add_layer(Layer::from_geometries("points", [point(5., 5.)]));
    let first = t
        .add_rule(Rule::LineMustNotHaveDangles, roads, None)
        .unwrap();
    let inside = t
        .add_rule(Rule::PointMustBeCoveredByLines, points, Some(roads))
        .unwrap();
    let again = t
        .add_rule(Rule::LineMustNotHaveDangles, roads, None)
        .unwrap();
    assert_ne!(first.id, again.id);
    assert_eq!(
        t.rules().map(|spec| spec.id.index()).collect::<Vec<_>>(),
        [0, 1, 2]
    );
    let report = t.validate().unwrap();
    assert_eq!(report.rule(inside.id).unwrap().spec, inside);
    assert_eq!(
        report.rule(inside.id).unwrap().violations[0].kind,
        ViolationKind::NotCovered
    );
    assert_eq!(report.rule(again.id).unwrap().violations.len(), 2);
}

#[test]
fn serialized_reports_keep_existing_field_names() {
    let mut t = Topology::new(0.).unwrap();
    let roads = t.add_layer(Layer::from_features(
        "roads",
        [
            (9, line(&[(0., 0.), (1., 0.)])),
            (10, line(&[(5., 5.), (5., 5.)])),
        ],
    ));
    t.add_rule(Rule::LineMustNotHaveDangles, roads, None)
        .unwrap();
    let json = serde_json::to_value(t.validate().unwrap()).unwrap();
    let rule = &json["rules"][0];
    assert_eq!(
        rule["spec"],
        serde_json::json!({"rule": "line/must-not-have-dangles", "layer": 0, "other": null})
    );
    assert_eq!(rule["errors"][0]["error_type"], "dangle");
    assert_eq!(rule["errors"][0]["origin"]["fid"], 9);
    assert_eq!(rule["counts"]["dangle"], 2);
    assert_eq!(
        json["warnings"],
        serde_json::json!([{"kind": "degenerate-parts", "layer": 0, "name": "roads", "skipped": 1}])
    );
}

#[test]
fn cleared_rules_restart_ids_and_keep_layers() {
    let mut topology = Topology::new(0.).unwrap();
    let roads = topology.add_layer(Layer::from_geometries(
        "roads",
        [line(&[(0., 0.), (2., 0.)]), line(&[(1., -1.), (1., 1.)])],
    ));
    topology
        .add_rule(Rule::LineMustNotHaveDangles, roads, None)
        .unwrap();
    assert_eq!(topology.validate().unwrap().violation_count(), 4);
    topology.clear_rules();
    assert_eq!(topology.rules().len(), 0);
    let spec = topology
        .add_rule(Rule::LineMustNotIntersect, roads, None)
        .unwrap();
    assert_eq!(spec.id.index(), 0);
    let report = topology.validate().unwrap();
    assert_eq!(report.rules.len(), 1);
    assert_eq!(
        report.rule(spec.id).unwrap().spec.rule,
        Rule::LineMustNotIntersect
    );
    assert_eq!(
        report.violations().next().unwrap().kind,
        ViolationKind::Crossing
    );
}
