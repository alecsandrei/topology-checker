use std::sync::Arc;
use topology_checker_core::{
    Error, GeometryKind, Layer, Rule, RuleSummary, Topology, ViolationKind, Warning, explode_lines,
    geo::{
        Euclidean, Geometry, GeometryCollection, Length, Line, LineString, MultiPolygon, Point,
        Polygon, Rect, Triangle, coord,
    },
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

#[test]
fn violation_kinds_have_stable_kebab_case_names() {
    for (kind, name) in [
        (ViolationKind::Overlap, "overlap"),
        (ViolationKind::Outside, "outside"),
        (ViolationKind::Dangle, "dangle"),
        (ViolationKind::PseudoNode, "pseudo-node"),
        (ViolationKind::Crossing, "crossing"),
        (ViolationKind::TouchInterior, "touch-interior"),
        (ViolationKind::SelfOverlap, "self-overlap"),
        (ViolationKind::SelfCrossing, "self-crossing"),
        (ViolationKind::Gap, "gap"),
        (ViolationKind::OuterBoundary, "outer-boundary"),
        (ViolationKind::Multipart, "multipart"),
        (ViolationKind::NotCovered, "not-covered"),
        (ViolationKind::MissingPoint, "missing-point"),
        (ViolationKind::PointCount, "point-count"),
        (ViolationKind::Contained, "contained"),
        (ViolationKind::Intersection, "intersection"),
    ] {
        assert_eq!(kind.name(), name);
        assert_eq!(serde_json::to_value(kind).unwrap(), name);
    }
}

#[test]
fn geometry_kinds_classify_every_geometry_variant() {
    let line = Line::new((0., 0.), (1., 0.));
    let triangle = Triangle::new((0., 0.).into(), (1., 0.).into(), (0., 1.).into());
    for (geometry, kind) in [
        (Geometry::from(Point::new(0., 0.)), GeometryKind::Point),
        (line.into(), GeometryKind::Line),
        (triangle.into(), GeometryKind::Polygon),
        (
            Geometry::GeometryCollection(GeometryCollection(vec![Point::new(0., 0.).into()])),
            GeometryKind::Collection,
        ),
    ] {
        assert_eq!(GeometryKind::of(&geometry), kind);
    }
    assert!(!GeometryKind::Collection.satisfies(GeometryKind::Point));
    assert!(GeometryKind::Collection.satisfies(GeometryKind::Any));
}

#[test]
fn segments_rectangles_and_triangles_behave_like_lines_and_polygons() {
    let mut t = Topology::new(0.).unwrap();
    let lines = t.add_layer(Layer::from_features(
        "segments",
        [
            (1, Line::new((0., 0.), (4., 0.)).into()),
            (2, Line::new((2., 0.), (6., 0.)).into()),
        ],
    ));
    let triangle = Triangle::new((0., 0.).into(), (4., 0.).into(), (0., 4.).into());
    let polygons = t.add_layer(Layer::from_features(
        "areas",
        [
            (1, triangle.into()),
            (
                2,
                Rect::new(coord! {x: 1., y: 1.}, coord! {x: 3., y: 3.}).into(),
            ),
        ],
    ));
    t.add_rule(Rule::LineMustNotOverlap, lines, None).unwrap();
    t.add_rule(Rule::PolygonMustNotOverlap, polygons, None)
        .unwrap();
    let report = t.validate().unwrap();
    assert_eq!(report.rules[0].violations.len(), 1);
    assert_eq!(report.rules[1].violations.len(), 1);
}

#[test]
fn cleaned_polygons_keep_valid_holes() {
    let shell = LineString::from(vec![
        (0., 0.),
        (0., 0.),
        (10., 0.),
        (10., 10.),
        (0., 10.),
        (0., 0.),
    ]);
    let hole = LineString::from(vec![(2., 2.), (4., 2.), (4., 4.), (2., 4.), (2., 2.)]);
    let degenerate = LineString::from(vec![(6., 6.), (7., 7.), (6., 6.)]);
    let mut t = Topology::new(0.).unwrap();
    let id = t.add_layer(Layer::from_geometries(
        "holed",
        [Polygon::new(shell, vec![hole, degenerate]).into()],
    ));
    t.add_rule(Rule::PolygonMustNotHaveGaps, id, None).unwrap();
    assert_eq!(t.warnings().unwrap().len(), 1);
    let report = t.validate().unwrap();
    let kinds = report.rules[0]
        .violations
        .iter()
        .map(|v| v.kind)
        .collect::<Vec<_>>();
    assert_eq!(kinds, [ViolationKind::Gap, ViolationKind::OuterBoundary]);
}

#[test]
fn unknown_layers_and_report_layer_names_are_reported() {
    let mut first = Topology::new(0.).unwrap();
    let mut second = Topology::new(0.).unwrap();
    first.add_layer(Layer::from_geometries("a", [point(0., 0.)]));
    let a = second.add_layer(Layer::from_geometries("padding", []));
    let b = second.add_layer(Layer::from_geometries("b", [point(0., 0.)]));
    assert!(first.layer(b).is_err());
    assert!(matches!(first.shared_layer(b), Err(Error::UnknownLayer(id)) if id == b));
    assert_eq!(second.shared_layer(b).unwrap().name(), "b");
    assert!(second.layer(a).unwrap().check(a).is_ok());

    let c = second.add_layer(Layer::from_geometries("c", [point(0., 0.)]));
    second
        .add_rule(Rule::PointMustNotOverlapWith, b, Some(c))
        .unwrap();
    let report = second.validate().unwrap();
    let rule = &report.rules[0];
    assert_eq!(rule.layer_name(b), Some("b"));
    assert_eq!(rule.layer_name(c), Some("c"));
    assert_eq!(rule.layer_name(a), None);
    let summary = RuleSummary::from(rule);
    assert_eq!(summary.total, rule.violations.len());
    assert_eq!(summary.counts, rule.counts);
    assert_eq!(summary.other.as_deref(), Some("c"));
}

#[test]
fn collapsed_lines_warn_with_their_feature() {
    let mut t = Topology::new(1.).unwrap();
    t.add_layer(Layer::from_features(
        "short",
        [(7, line(&[(0., 0.), (0.25, 0.), (0.5, 0.)]))],
    ));
    let warnings = t.warnings().unwrap();
    assert!(
        warnings
            .iter()
            .any(|w| w.to_string() == "layer short, FID 7, part 0: line collapsed under tolerance"),
        "{warnings:?}"
    );
}

fn linear_length(geometry: &Geometry) -> f64 {
    match geometry {
        Geometry::LineString(l) => l.lines().map(|s| Euclidean.length(&s)).sum(),
        Geometry::MultiLineString(m) => m.0.iter().map(|l| linear_length(&l.clone().into())).sum(),
        Geometry::GeometryCollection(c) => c.0.iter().map(linear_length).sum(),
        _ => 0.,
    }
}

#[test]
fn boundary_rules_see_holes_of_source_and_cleaned_polygons() {
    let ring = |x: f64, size: f64, repeat: bool| {
        let mut coords = vec![(x, x), (x + size, x), (x + size, x + size), (x, x + size)];
        if repeat {
            coords.insert(1, coords[0]);
        }
        coords.push(coords[0]);
        LineString::from(coords)
    };
    let holed =
        |x: f64, repeat: bool| Polygon::new(ring(x, 10., repeat), vec![ring(x + 2., 4., false)]);
    // Each case has the polygons' shells (perimeter 40) and holes (perimeter 16).
    let cases = [
        (
            Polygon::new(ring(0., 10., false), vec![ring(2., 4., false)]).into(),
            1,
        ),
        (
            MultiPolygon(vec![holed(0., false), holed(20., true)]).into(),
            2,
        ),
        (holed(0., true).into(), 1),
    ];
    for (geometry, polygons) in cases {
        let geometry: Geometry = geometry;
        let holes = match &geometry {
            Geometry::Polygon(p) => vec![p.clone()],
            Geometry::MultiPolygon(m) => m.0.clone(),
            _ => unreachable!(),
        }
        .into_iter()
        .flat_map(|p| p.interiors().to_vec())
        .collect::<Vec<_>>();
        for (covering, boundary_length) in [(holes, 40.), (vec![], 56.)] {
            let mut t = Topology::new(0.).unwrap();
            let areas = t.add_layer(Layer::from_geometries("areas", [geometry.clone()]));
            let lines = t.add_layer(Layer::from_geometries(
                "lines",
                covering.into_iter().map(Geometry::from),
            ));
            t.add_rule(
                Rule::PolygonBoundariesMustBeCoveredByLines,
                areas,
                Some(lines),
            )
            .unwrap();
            let report = t.validate().unwrap();
            let length: f64 = report
                .violations()
                .map(|v| linear_length(&v.geometry))
                .sum();
            assert_eq!(length, boundary_length * polygons as f64);
        }
    }
}
