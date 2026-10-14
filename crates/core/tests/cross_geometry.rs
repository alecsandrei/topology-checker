use topology_checker_core::{
    Layer, Rule, Topology, Violation, ViolationKind,
    geo::{
        Geometry, GeometryCollection, LineString, MultiLineString, MultiPoint, MultiPolygon, Point,
        Polygon, Rect, coord,
    },
};
#[path = "support/exact_line_polygon.rs"]
mod exact_line_polygon;
fn point(x: f64, y: f64) -> Geometry {
    Point::new(x, y).into()
}
fn line(coords: &[(f64, f64)]) -> Geometry {
    LineString::from(coords.to_vec()).into()
}
fn polygon(x: f64, y: f64, size: f64) -> Polygon {
    Rect::new(coord! {x:x,y:y}, coord! {x:x+size,y:y+size}).to_polygon()
}
fn square() -> Geometry {
    polygon(0., 0., 10.).into()
}
fn hole() -> Geometry {
    Polygon::new(
        polygon(0., 0., 10.).exterior().clone(),
        vec![polygon(3., 3., 4.).exterior().clone()],
    )
    .into()
}
fn run(rule: Rule, a: Vec<Geometry>, b: Vec<Geometry>, tolerance: f64) -> Vec<Violation> {
    let mut t = Topology::new(tolerance).unwrap();
    let a = t.add_layer(Layer::from_features(
        "origin",
        a.into_iter().enumerate().map(|(i, g)| (i as i64 - 20, g)),
    ));
    let b = t.add_layer(Layer::from_features(
        "destination",
        b.into_iter().enumerate().map(|(i, g)| (i as i64 - 40, g)),
    ));
    t.add_rule(rule, a, Some(b)).unwrap();
    t.validate().unwrap().rules.remove(0).violations
}
#[test]
fn every_new_rule_has_valid_and_invalid_examples() {
    let edge = line(&[(0., 0.), (10., 0.)]);
    let interior = line(&[(1., 1.), (2., 1.)]);
    let outside = line(&[(-2., -1.), (-1., -1.)]);
    let perimeter: Geometry = polygon(0., 0., 10.).exterior().clone().into();
    let cases = vec![
        (
            Rule::PointMustBeCoveredByLines,
            point(5., 0.),
            point(5., 1.),
            edge.clone(),
            edge.clone(),
        ),
        (
            Rule::PointMustBeCoveredByLineEndpoints,
            point(0., 0.),
            point(5., 0.),
            edge.clone(),
            edge.clone(),
        ),
        (
            Rule::PointMustBeCoveredByPolygonBoundaries,
            point(0., 5.),
            point(5., 5.),
            square(),
            square(),
        ),
        (
            Rule::LineEndpointsMustBeCoveredByPoints,
            edge.clone(),
            edge.clone(),
            MultiPoint(vec![Point::new(0., 0.), Point::new(10., 0.)]).into(),
            point(0., 0.),
        ),
        (
            Rule::LineMustBeCoveredByPolygonBoundaries,
            edge.clone(),
            interior.clone(),
            square(),
            square(),
        ),
        (
            Rule::PolygonBoundariesMustBeCoveredByLines,
            square(),
            square(),
            perimeter,
            edge.clone(),
        ),
        (
            Rule::PolygonMustContainPoint,
            square(),
            square(),
            point(1., 1.),
            point(0., 0.),
        ),
        (
            Rule::PolygonMustContainOnePoint,
            square(),
            square(),
            point(1., 1.),
            MultiPoint(vec![Point::new(1., 1.), Point::new(20., 20.)]).into(),
        ),
        (
            Rule::LineMustNotCrossPolygonBoundaries,
            edge.clone(),
            line(&[(-1., 5.), (11., 5.)]),
            square(),
            square(),
        ),
        (
            Rule::GeometryMustNotBeContainedBy,
            point(20., 20.),
            point(1., 1.),
            square(),
            square(),
        ),
        (
            Rule::PointMustBeCoveredByPolygons,
            point(0., 5.),
            point(20., 20.),
            square(),
            square(),
        ),
        (
            Rule::PointMustNotBeInsidePolygons,
            point(0., 5.),
            point(5., 5.),
            square(),
            square(),
        ),
        (
            Rule::PointMustNotIntersectLines,
            point(1., 1.),
            point(0., 0.),
            edge.clone(),
            edge.clone(),
        ),
        (
            Rule::PointMustNotIntersectPolygons,
            point(20., 20.),
            point(0., 0.),
            square(),
            square(),
        ),
        (
            Rule::LineMustBeProperlyInsidePolygons,
            interior.clone(),
            edge.clone(),
            square(),
            square(),
        ),
        (
            Rule::LineMustNotOverlapWithPolygons,
            outside.clone(),
            interior.clone(),
            square(),
            square(),
        ),
        (
            Rule::LineMustNotOverlapPolygonBoundaries,
            interior.clone(),
            edge.clone(),
            square(),
            square(),
        ),
        (
            Rule::LineMustNotIntersectPolygons,
            outside.clone(),
            interior.clone(),
            square(),
            square(),
        ),
        (
            Rule::LineEndpointsMustBeCoveredByPolygonBoundaries,
            edge,
            interior,
            square(),
            square(),
        ),
        (
            Rule::PolygonMustNotContainPoints,
            square(),
            square(),
            point(0., 0.),
            point(5., 5.),
        ),
    ];
    assert_eq!(cases.len(), 20);
    for (rule, valid, invalid, target_valid, target_invalid) in cases {
        assert!(
            run(rule, vec![valid], vec![target_valid], 0.).is_empty(),
            "valid {rule}"
        );
        assert!(
            !run(rule, vec![invalid], vec![target_invalid], 0.).is_empty(),
            "invalid {rule}"
        );
    }
}
#[test]
fn point_and_endpoint_coverage_use_inclusive_distance_thresholds() {
    for rule in [
        Rule::PointMustBeCoveredByLines,
        Rule::PointMustBeCoveredByLineEndpoints,
    ] {
        for (offset, errors) in [(0., 0), (0.25, 0), (0.250000001, 1)] {
            assert_eq!(
                run(
                    rule,
                    vec![point(0., offset)],
                    vec![line(&[(0., 0.), (10., 0.)])],
                    0.25
                )
                .len(),
                errors,
                "{rule}"
            );
        }
    }
    for rule in [
        Rule::PointMustBeCoveredByPolygonBoundaries,
        Rule::PointMustBeCoveredByPolygons,
    ] {
        assert!(run(rule, vec![point(-0.25, 5.)], vec![square()], 0.25).is_empty());
        assert_eq!(
            run(rule, vec![point(-0.250000001, 5.)], vec![square()], 0.25).len(),
            1
        );
    }
    for rule in [
        Rule::PointMustNotBeInsidePolygons,
        Rule::PolygonMustNotContainPoints,
    ] {
        let (a, b) = if rule == Rule::PolygonMustNotContainPoints {
            (
                vec![square()],
                vec![point(0.25, 5.), point(0.250000001, 5.)],
            )
        } else {
            (
                vec![point(0.25, 5.), point(0.250000001, 5.)],
                vec![square()],
            )
        };
        assert_eq!(run(rule, a, b, 0.25).len(), 1);
    }
    assert_eq!(
        run(
            Rule::PointMustNotIntersectLines,
            vec![point(5., 0.25)],
            vec![line(&[(0., 0.), (10., 0.)])],
            0.25
        )
        .len(),
        1
    );
    assert_eq!(
        run(
            Rule::PointMustNotIntersectPolygons,
            vec![point(-0.25, 5.)],
            vec![square()],
            0.25
        )
        .len(),
        1
    );
}
#[test]
fn boundaries_keep_holes_and_shared_edges() {
    let shared = line(&[(10., 1.), (10., 9.)]);
    let parcels = vec![square(), polygon(10., 0., 10.).into()];
    assert!(
        run(
            Rule::LineMustBeCoveredByPolygonBoundaries,
            vec![shared.clone()],
            parcels.clone(),
            0.
        )
        .is_empty()
    );
    assert_eq!(
        run(
            Rule::LineMustNotOverlapPolygonBoundaries,
            vec![shared],
            parcels,
            0.
        )
        .len(),
        2
    );
    assert!(
        run(
            Rule::PointMustBeCoveredByPolygonBoundaries,
            vec![point(3., 5.)],
            vec![hole()],
            0.
        )
        .is_empty()
    );
    assert_eq!(
        run(
            Rule::PointMustBeCoveredByPolygons,
            vec![point(5., 5.)],
            vec![hole()],
            0.
        )
        .len(),
        1
    );
    assert_eq!(
        run(
            Rule::LineMustNotCrossPolygonBoundaries,
            vec![line(&[(1., 5.), (9., 5.)])],
            vec![hole()],
            0.
        )
        .len(),
        1
    );
    let edges = Geometry::MultiLineString(MultiLineString(vec![
        polygon(0., 0., 10.).exterior().clone(),
        polygon(3., 3., 4.).exterior().clone(),
    ]));
    assert!(
        run(
            Rule::PolygonBoundariesMustBeCoveredByLines,
            vec![hole()],
            vec![edges],
            0.
        )
        .is_empty()
    );
}
#[test]
fn crossings_distinguish_vertices_tangencies_endpoint_touches_and_shared_paths() {
    for (coords, expected) in [
        (vec![(-1., -1.), (1., 1.)], 1),
        (vec![(-1., -1.), (0., 0.), (1., 1.)], 1),
        (vec![(-1., 1.), (0., 0.), (1., -1.)], 0),
        (vec![(-1., 5.), (0., 5.)], 0),
        (vec![(0., 5.), (5., 5.)], 0),
        (vec![(-1., 0.), (5., 0.), (5., 5.)], 1),
        (vec![(1., 1.), (2., 2.)], 0),
    ] {
        let errors = run(
            Rule::LineMustNotCrossPolygonBoundaries,
            vec![line(&coords)],
            vec![square()],
            0.,
        );
        assert_eq!(errors.len(), expected, "{coords:?}");
        if expected > 0 {
            assert_eq!(errors[0].geometry, point(0., 0.));
        }
    }
    let errors = run(
        Rule::LineMustNotCrossPolygonBoundaries,
        vec![line(&[(-1., 5.), (11., 5.)])],
        vec![square()],
        0.,
    );
    assert_eq!(errors.len(), 1);
    assert_eq!(
        errors[0].geometry,
        Geometry::MultiPoint(MultiPoint(vec![Point::new(0., 5.), Point::new(10., 5.)]))
    );
}
#[test]
fn strict_line_containment_uses_union_and_reports_whole_parts() {
    let adjacent = vec![polygon(0., 0., 5.).into(), polygon(5., 0., 5.).into()];
    assert!(
        run(
            Rule::LineMustBeProperlyInsidePolygons,
            vec![line(&[(1., 2.), (9., 2.)])],
            adjacent,
            0.
        )
        .is_empty()
    );
    let touching = line(&[(1., 1.), (0., 5.), (1., 9.)]);
    let errors = run(
        Rule::LineMustBeProperlyInsidePolygons,
        vec![touching.clone()],
        vec![square()],
        0.,
    );
    assert_eq!(errors[0].geometry, touching);
    assert_eq!(
        run(
            Rule::LineMustBeProperlyInsidePolygons,
            vec![line(&[(0.25, 1.), (0.25, 9.)])],
            vec![square()],
            0.25
        )
        .len(),
        1
    );
    assert!(
        run(
            Rule::LineMustBeProperlyInsidePolygons,
            vec![line(&[(0.250000001, 1.), (0.250000001, 9.)])],
            vec![square()],
            0.25
        )
        .is_empty()
    );
    assert_eq!(
        run(
            Rule::LineMustBeProperlyInsidePolygons,
            vec![line(&[(1., 5.), (9., 5.)])],
            vec![hole()],
            0.
        )
        .len(),
        1
    );
}
#[test]
fn line_overlap_and_contact_have_distinct_geometries_and_length_thresholds() {
    let crossing = line(&[(-1., 5.), (11., 5.)]);
    let errors = run(
        Rule::LineMustNotOverlapWithPolygons,
        vec![crossing],
        vec![square()],
        0.,
    );
    assert_eq!(errors[0].geometry, line(&[(0., 5.), (10., 5.)]));
    let tangent = line(&[(-1., 1.), (0., 0.), (1., -1.)]);
    assert!(
        run(
            Rule::LineMustNotOverlapWithPolygons,
            vec![tangent.clone()],
            vec![square()],
            0.
        )
        .is_empty()
    );
    assert_eq!(
        run(
            Rule::LineMustNotIntersectPolygons,
            vec![tangent],
            vec![square()],
            0.
        )[0]
        .geometry,
        point(0., 0.)
    );
    for length in [0.1, 0.25] {
        let short = line(&[(1., 1.), (1. + length, 1.)]);
        assert!(
            run(
                Rule::LineMustNotOverlapWithPolygons,
                vec![short.clone()],
                vec![square()],
                0.25
            )
            .is_empty()
        );
        assert_eq!(
            run(
                Rule::LineMustNotIntersectPolygons,
                vec![short],
                vec![square()],
                0.25
            )
            .len(),
            1
        );
    }
    assert_eq!(
        run(
            Rule::LineMustNotOverlapWithPolygons,
            vec![line(&[(1., 1.), (1.250000001, 1.)])],
            vec![square()],
            0.25
        )
        .len(),
        1
    );
    assert_eq!(
        run(
            Rule::LineMustNotIntersectPolygons,
            vec![line(&[(-0.25, 1.), (-0.25, 9.)])],
            vec![square()],
            0.25
        )
        .len(),
        1
    );
    assert!(
        run(
            Rule::LineMustNotIntersectPolygons,
            vec![line(&[(-0.250000001, 1.), (-0.250000001, 9.)])],
            vec![square()],
            0.25
        )
        .is_empty()
    );
}
#[test]
fn mixed_contacts_are_preserved_during_feature_aggregation() {
    let input = Geometry::MultiLineString(MultiLineString(vec![
        LineString::from(vec![(1., 1.), (2., 1.)]),
        LineString::from(vec![(-1., 1.), (0., 0.), (1., -1.)]),
    ]));
    let errors = run(
        Rule::LineMustNotIntersectPolygons,
        vec![input],
        vec![square()],
        0.,
    );
    assert_eq!(errors.len(), 1);
    let Geometry::GeometryCollection(c) = &errors[0].geometry else {
        panic!("mixed contact lost: {:?}", errors[0]);
    };
    assert_eq!(c.0, vec![point(0., 0.), line(&[(1., 1.), (2., 1.)])]);
    assert_eq!(errors[0].origin.unwrap().part, None);
}
#[test]
fn cardinality_counts_features_and_preserves_input_roles() {
    let points = vec![point(1., 1.), point(2., 2.)];
    assert!(
        run(
            Rule::PolygonMustContainPoint,
            vec![square()],
            points.clone(),
            0.
        )
        .is_empty()
    );
    let errors = run(Rule::PolygonMustContainOnePoint, vec![square()], points, 0.);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].origin.unwrap().fid, -20);
    assert!(errors[0].destination.is_none());
    assert_eq!(errors[0].kind, ViolationKind::PointCount);
    let multi = Geometry::MultiPoint(MultiPoint(vec![Point::new(1., 1.), Point::new(2., 2.)]));
    assert!(
        run(
            Rule::PolygonMustContainOnePoint,
            vec![square()],
            vec![multi],
            0.
        )
        .is_empty()
    );
    let multi = Geometry::MultiPoint(MultiPoint(vec![Point::new(1., 1.), Point::new(20., 20.)]));
    let errors = run(
        Rule::PolygonMustContainOnePoint,
        vec![square()],
        vec![multi],
        0.,
    );
    assert_eq!(errors.len(), 2);
    let point_error = errors.iter().find(|e| e.origin.is_none()).unwrap();
    assert_eq!(point_error.destination.unwrap().fid, -40);
    let errors = run(
        Rule::PolygonMustContainOnePoint,
        vec![square(), polygon(1., 1., 10.).into()],
        vec![point(5., 5.)],
        0.,
    );
    assert_eq!(errors.len(), 1);
    assert!(errors[0].origin.is_none());
    let multi = Geometry::MultiPolygon(MultiPolygon(vec![
        polygon(0., 0., 10.),
        polygon(20., 0., 10.),
    ]));
    let points = Geometry::MultiPoint(MultiPoint(vec![Point::new(5., 5.), Point::new(25., 5.)]));
    assert!(
        run(
            Rule::PolygonMustContainOnePoint,
            vec![multi],
            vec![points],
            0.
        )
        .is_empty()
    );
    assert_eq!(
        run(Rule::PolygonMustContainPoint, vec![square()], vec![], 0.)[0].kind,
        ViolationKind::MissingPoint
    );
    assert_eq!(
        run(
            Rule::PolygonMustContainOnePoint,
            vec![],
            vec![point(1., 1.)],
            0.
        )
        .len(),
        1
    );
}
#[test]
fn closed_parts_have_no_endpoints_and_cleaning_keeps_original_part_indices() {
    let closed: Geometry = polygon(0., 0., 10.).exterior().clone().into();
    for rule in [
        Rule::LineEndpointsMustBeCoveredByPoints,
        Rule::LineEndpointsMustBeCoveredByPolygonBoundaries,
    ] {
        assert!(run(rule, vec![closed.clone()], vec![], 0.).is_empty());
    }
    assert_eq!(
        run(
            Rule::PointMustBeCoveredByLineEndpoints,
            vec![point(0., 0.)],
            vec![closed],
            0.
        )
        .len(),
        1
    );
    let parts = Geometry::MultiLineString(MultiLineString(vec![
        LineString::empty(),
        LineString::from(vec![(0., 0.), (0., 0.), (1., 0.)]),
    ]));
    let errors = run(
        Rule::LineEndpointsMustBeCoveredByPoints,
        vec![parts],
        vec![point(0., 0.)],
        0.,
    );
    assert_eq!(errors[0].origin.unwrap().part, Some(1));
    assert_eq!(errors[0].geometry, point(1., 0.));
}
#[test]
fn whole_feature_containment_excludes_equality_and_never_uses_target_union_or_buffer() {
    assert!(
        run(
            Rule::GeometryMustNotBeContainedBy,
            vec![square()],
            vec![square()],
            5.
        )
        .is_empty()
    );
    assert!(
        run(
            Rule::GeometryMustNotBeContainedBy,
            vec![line(&[(-1., 5.), (11., 5.)])],
            vec![square()],
            5.
        )
        .is_empty()
    );
    assert_eq!(
        run(
            Rule::GeometryMustNotBeContainedBy,
            vec![line(&[(0., 5.), (5., 5.)])],
            vec![square()],
            0.
        )
        .len(),
        1
    );
    let multipoint =
        Geometry::MultiPoint(MultiPoint(vec![Point::new(1., 1.), Point::new(11., 1.)]));
    assert!(
        run(
            Rule::GeometryMustNotBeContainedBy,
            vec![multipoint],
            vec![square(), polygon(10., 0., 10.).into()],
            0.
        )
        .is_empty()
    );
    let mixed = Geometry::GeometryCollection(GeometryCollection(vec![
        point(1., 1.),
        line(&[(2., 2.), (3., 3.)]),
        polygon(4., 4., 1.).into(),
    ]));
    let errors = run(
        Rule::GeometryMustNotBeContainedBy,
        vec![mixed.clone()],
        vec![square()],
        0.,
    );
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].geometry, mixed);
    assert!(errors[0].origin.unwrap().part.is_none());
    assert_eq!(
        run(
            Rule::GeometryMustNotBeContainedBy,
            vec![point(5., 0.)],
            vec![line(&[(0., 0.), (10., 0.)])],
            0.
        )
        .len(),
        1
    );
    assert!(
        run(
            Rule::GeometryMustNotBeContainedBy,
            vec![point(0., 0.)],
            vec![line(&[(0., 0.), (10., 0.)])],
            0.
        )
        .is_empty()
    );
}
#[test]
fn empty_targets_fail_requirements_and_pass_prohibitions() {
    for &rule in &Rule::ALL[18..] {
        let geometry = match rule.inputs().0 {
            topology_checker_core::GeometryKind::Point => point(1., 1.),
            topology_checker_core::GeometryKind::Line => line(&[(1., 1.), (2., 2.)]),
            _ => square(),
        };
        let required = rule.name().contains("must-be-") || rule.name().contains("must-contain-");
        let errors = run(rule, vec![geometry], vec![], 0.);
        assert_eq!(!errors.is_empty(), required, "{rule}");
        assert!(run(rule, vec![], vec![], 0.).is_empty());
    }
}
#[test]
fn graph_modes_and_unrelated_layers_do_not_change_results() {
    let mut topology = Topology::new(0.01).unwrap();
    let a = topology.add_layer(Layer::from_geometries(
        "lines",
        [line(&[(0., 0.), (10., 0.)])],
    ));
    let b = topology.add_layer(Layer::from_geometries("polygon", [square()]));
    topology
        .add_rule(Rule::LineMustBeCoveredByPolygonBoundaries, a, Some(b))
        .unwrap();
    topology
        .add_rule(Rule::LineMustNotOverlapPolygonBoundaries, a, Some(b))
        .unwrap();
    let first = topology.validate().unwrap();
    let c = topology.add_layer(Layer::from_geometries(
        "unrelated",
        [line(&[(5., -5.), (5., 5.)])],
    ));
    topology
        .add_rule(Rule::LineMustNotIntersectWith, a, Some(c))
        .unwrap();
    topology
        .add_rule(Rule::GeometryMustNotBeContainedBy, a, Some(b))
        .unwrap();
    for _ in 0..4 {
        let report = topology.validate().unwrap();
        assert_eq!(first.rules[0].violations, report.rules[0].violations);
        assert_eq!(first.rules[1].violations, report.rules[1].violations);
    }
}

#[test]
fn closed_overlap_pieces_join_across_the_traversal_seam_before_length_filtering() {
    let input = line(&[
        (0.5, 1.),
        (2., 1.),
        (2., -1.),
        (-1., -1.),
        (-1., 1.),
        (0.5, 1.),
    ]);
    let target: Geometry = polygon(0., 0., 1.).into();
    let errors = run(
        Rule::LineMustNotOverlapWithPolygons,
        vec![input],
        vec![target],
        0.75,
    );
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].geometry, line(&[(0., 1.), (0.5, 1.), (1., 1.)]));
}

#[test]
fn feature_point_counts_use_combined_parts_and_ignore_skipped_parts() {
    // Adjacent polygon members may share an edge in a collection. A MultiPolygon
    // with that relationship is now rejected as invalid input.
    let polygons = Geometry::GeometryCollection(GeometryCollection(vec![
        polygon(0., 0., 5.).into(),
        polygon(5., 0., 5.).into(),
    ]));
    assert!(
        run(
            Rule::PolygonMustContainOnePoint,
            vec![polygons],
            vec![point(5., 2.)],
            0.
        )
        .is_empty()
    );
    let skipped = Geometry::GeometryCollection(GeometryCollection(vec![
        LineString::empty().into(),
        Point::new(2., 2.).into(),
    ]));
    assert_eq!(
        run(
            Rule::GeometryMustNotBeContainedBy,
            vec![skipped],
            vec![square()],
            0.
        )[0]
        .geometry,
        point(2., 2.)
    );
    for rule in [
        Rule::GeometryMustNotBeContainedBy,
        Rule::LineMustNotIntersectPolygons,
        Rule::LineMustBeProperlyInsidePolygons,
    ] {
        assert!(run(rule, vec![LineString::empty().into()], vec![square()], 0.).is_empty());
    }
}

#[test]
fn coverage_returns_only_uncovered_runs_and_tolerance_noding_accepts_near_edges() {
    let errors = run(
        Rule::LineMustBeCoveredByPolygonBoundaries,
        vec![line(&[(-2., 0.), (5., 0.), (5., 2.)])],
        vec![square()],
        0.,
    );
    assert_eq!(errors.len(), 1);
    assert_eq!(
        errors[0].geometry,
        Geometry::MultiLineString(MultiLineString(vec![
            LineString::from(vec![(-2., 0.), (0., 0.)]),
            LineString::from(vec![(5., 0.), (5., 2.)])
        ]))
    );
    assert!(
        run(
            Rule::LineMustBeCoveredByPolygonBoundaries,
            vec![line(&[(1., 0.25), (9., 0.25)])],
            vec![square()],
            0.25
        )
        .is_empty()
    );
    assert_eq!(
        run(
            Rule::LineMustBeCoveredByPolygonBoundaries,
            vec![line(&[(1., 0.250000001), (9., 0.250000001)])],
            vec![square()],
            0.25
        )
        .len(),
        1
    );
    assert_eq!(
        run(
            Rule::LineMustNotOverlapPolygonBoundaries,
            vec![line(&[(1., 0.25), (9., 0.25)])],
            vec![square()],
            0.25
        )
        .len(),
        1
    );
    assert_eq!(
        run(
            Rule::LineEndpointsMustBeCoveredByPolygonBoundaries,
            vec![line(&[(0.25, 1.), (9.75, 1.)])],
            vec![square()],
            0.25
        )
        .len(),
        0
    );
}

#[test]
fn contained_features_support_all_target_families_and_topological_equality() {
    let long = line(&[(0., 0.), (10., 0.)]);
    let reversed = line(&[(10., 0.), (5., 0.), (0., 0.)]);
    assert!(
        run(
            Rule::GeometryMustNotBeContainedBy,
            vec![long.clone()],
            vec![reversed],
            0.
        )
        .is_empty()
    );
    let shorter = line(&[(1., 0.), (9., 0.)]);
    assert_eq!(
        run(
            Rule::GeometryMustNotBeContainedBy,
            vec![shorter],
            vec![long],
            0.
        )
        .len(),
        1
    );
    let target = Geometry::MultiPoint(MultiPoint(vec![Point::new(1., 1.), Point::new(2., 2.)]));
    assert_eq!(
        run(
            Rule::GeometryMustNotBeContainedBy,
            vec![point(1., 1.)],
            vec![target],
            0.
        )
        .len(),
        1
    );
    assert!(
        run(
            Rule::GeometryMustNotBeContainedBy,
            vec![point(1., 1.)],
            vec![point(1., 1.)],
            0.
        )
        .is_empty()
    );
    let small: Geometry = polygon(1., 1., 2.).into();
    assert_eq!(
        run(
            Rule::GeometryMustNotBeContainedBy,
            vec![small],
            vec![square()],
            100.
        )
        .len(),
        1
    );
    let target = Geometry::GeometryCollection(GeometryCollection(vec![
        square(),
        line(&[(20., 0.), (30., 0.)]),
        point(40., 0.),
    ]));
    let input =
        Geometry::GeometryCollection(GeometryCollection(vec![point(1., 1.), point(25., 0.)]));
    assert_eq!(
        run(
            Rule::GeometryMustNotBeContainedBy,
            vec![input],
            vec![target],
            0.
        )
        .len(),
        1
    );
    let points = Geometry::MultiPoint(MultiPoint(vec![Point::new(1., 1.), Point::new(20., 20.)]));
    let errors = run(
        Rule::PolygonMustNotContainPoints,
        vec![square()],
        vec![points],
        0.,
    );
    assert_eq!(errors[0].geometry, point(1., 1.));
    assert_eq!(errors[0].destination.unwrap().part, Some(0));
}

use serde_json::Value;
use topology_checker_core::geo::{Area, Coord};

fn geometry(value: &Value) -> Geometry {
    let line = |v: &Value| {
        LineString(
            v.as_array()
                .unwrap()
                .iter()
                .map(|p| Coord {
                    x: p[0].as_f64().unwrap(),
                    y: p[1].as_f64().unwrap(),
                })
                .collect(),
        )
    };
    let polygon = |v: &Value| {
        let rings = v.as_array().unwrap();
        Polygon::new(line(&rings[0]), rings[1..].iter().map(line).collect())
    };
    let coordinates = &value["coordinates"];
    match value["type"].as_str().unwrap() {
        "LineString" => line(coordinates).into(),
        "MultiLineString" => {
            MultiLineString(coordinates.as_array().unwrap().iter().map(line).collect()).into()
        }
        "Polygon" => polygon(coordinates).into(),
        "MultiPolygon" => MultiPolygon(
            coordinates
                .as_array()
                .unwrap()
                .iter()
                .map(polygon)
                .collect(),
        )
        .into(),
        other => panic!("unexpected fixture geometry {other}"),
    }
}
fn fixture(name: &str) -> Vec<Geometry> {
    let data: Value = serde_json::from_str(include_str!("fixtures/geometry_cases.json")).unwrap();
    data[name]["geometries"]
        .as_array()
        .unwrap()
        .iter()
        .map(geometry)
        .collect()
}
fn topology(rule: Rule, geometries: Vec<Geometry>, binary: bool, tolerance: f64) -> Topology {
    let mut t = Topology::new(tolerance).unwrap();
    if binary {
        let a = t.add_layer(Layer::from_features(
            "source",
            [(-91, geometries[0].clone())],
        ));
        let b = t.add_layer(Layer::from_features(
            "target",
            [(872, geometries[1].clone())],
        ));
        t.add_rule(rule, a, Some(b)).unwrap();
    } else {
        let a = t.add_layer(Layer::from_features(
            "source",
            geometries
                .into_iter()
                .enumerate()
                .map(|(i, g)| (i as i64 - 91, g)),
        ));
        t.add_rule(rule, a, None).unwrap();
    }
    t
}
#[test]
fn dc_and_ons_emission_preserve_substantial_area_and_original_provenance() {
    for name in [
        "dc_precinct_smd",
        "ons_degree_two",
        "ons_adjacent_nodes",
        "ons_source_contacts",
    ] {
        for tolerance in [0., 0.001, 1.] {
            let t = topology(
                Rule::PolygonMustNotOverlapWith,
                fixture(name),
                true,
                tolerance,
            );
            let report = t.validate().unwrap();
            assert_eq!(report.violation_count(), 1, "{name}, {tolerance}");
            let v = report.violations().next().unwrap();
            assert_eq!(v.kind, ViolationKind::Overlap);
            assert_eq!(v.origin.unwrap().fid, -91);
            assert_eq!(v.destination.unwrap().fid, 872);
            let area = match &v.geometry {
                Geometry::Polygon(p) => p.unsigned_area(),
                Geometry::MultiPolygon(p) => p.unsigned_area(),
                _ => panic!("expected polygon"),
            };
            assert!(area > 0.);
            if name == "dc_precinct_smd" {
                assert!((area - 371430.874).abs() <= 37.1430874);
            }
            assert_eq!(
                serde_json::to_value(&report).unwrap(),
                serde_json::to_value(t.validate().unwrap()).unwrap()
            );
        }
    }
}
#[test]
fn ons_and_brp_gap_emission_accepts_original_contact_topology() {
    for name in ["ons_gaps", "brp_contacts"] {
        for tolerance in [0., 0.001, 1.] {
            let t = topology(
                Rule::PolygonMustNotHaveGaps,
                fixture(name),
                false,
                tolerance,
            );
            let report = t.validate().unwrap();
            assert!(report.violation_count() > 0);
            assert!(
                report
                    .violations()
                    .all(|v| v.origin.is_none() && v.destination.is_none())
            );
        }
    }
}
#[test]
fn chicago_near_boundary_interval_is_contact() {
    let t = topology(
        Rule::LineMustNotCrossPolygonBoundaries,
        fixture("chicago_boundary"),
        true,
        0.001,
    );
    assert_eq!(t.validate().unwrap().violation_count(), 0);
}
#[test]
fn nearby_bent_original_intervals_support_both_boundary_coverage_directions() {
    let a = LineString::from(vec![(0., 0.), (5., 1.), (10., 0.)]);
    let b = LineString::from(vec![(0., 0.001), (2.5, 0.501), (5., 1.001), (10., 0.001)]);
    for rule in [
        Rule::LineMustNotOverlapWith,
        Rule::LineMustNotIntersectWith,
        Rule::LineMustNotIntersectOrTouchInteriorWith,
    ] {
        let t = topology(rule, vec![a.clone().into(), b.clone().into()], true, 1.);
        let r = t.validate().unwrap();
        assert_eq!(r.violation_count(), 1);
        assert_eq!(r.violations().next().unwrap().kind, ViolationKind::Overlap);
    }
    let p = Polygon::new(
        LineString::from(vec![
            (0., 0.),
            (5., 1.),
            (10., 0.),
            (10., -5.),
            (0., -5.),
            (0., 0.),
        ]),
        vec![],
    );
    let t = topology(
        Rule::LineMustBeCoveredByPolygonBoundaries,
        vec![b.clone().into(), p.clone().into()],
        true,
        1.,
    );
    assert_eq!(t.validate().unwrap().violation_count(), 0);
    let t = topology(
        Rule::PolygonBoundariesMustBeCoveredByLines,
        vec![p.into(), b.into()],
        true,
        1.,
    );
    assert_eq!(t.validate().unwrap().violation_count(), 1);
}
#[test]
fn tfl_original_crossing_survives_tolerance() {
    let t = topology(
        Rule::LineMustNotSelfIntersect,
        fixture("tfl_self_crossing"),
        false,
        0.001,
    );
    let report = t.validate().unwrap();
    assert_eq!(
        report
            .violations()
            .filter(|v| v.kind == ViolationKind::SelfCrossing)
            .count(),
        1
    );
}

#[test]
fn short_interval_evidence_does_not_suppress_shallow_crossings() {
    let shapes = fixture("shallow_crossings");
    for (start, positive) in [(0, 0.001), (2, 1.)] {
        for tolerance in [0., positive] {
            for rule in [
                Rule::LineMustNotIntersect,
                Rule::LineMustNotIntersectWith,
                Rule::LineMustNotIntersectOrTouchInterior,
                Rule::LineMustNotIntersectOrTouchInteriorWith,
            ] {
                let t = topology(
                    rule,
                    shapes[start..start + 2].to_vec(),
                    rule.inputs().1.is_some(),
                    tolerance,
                );
                let report = t.validate().unwrap();
                assert!(report.violation_count() > 0, "{rule}");
                assert!(
                    report
                        .violations()
                        .any(|v| v.kind == ViolationKind::Crossing)
                );
                let mut streamed = Vec::new();
                t.validate_into(|_, v| {
                    streamed.push(v);
                    Ok::<_, ()>(())
                })
                .unwrap();
                assert_eq!(streamed, report.rules[0].violations);
            }
        }
    }
    for rule in [
        Rule::LineMustNotIntersectWith,
        Rule::LineMustNotIntersectOrTouchInteriorWith,
    ] {
        let errors = run(
            rule,
            vec![line(&[(0., 0.), (10., 0.)])],
            vec![line(&[(5., 0.), (15., 0.)])],
            1.,
        );
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].kind, ViolationKind::Overlap);
    }
}

#[test]
fn crossings_connect_original_passages_across_boundary_runs() {
    let shapes = fixture("boundary_run_crossings");
    for tolerance in [0., 0.01, 1.] {
        for start in [0, 2] {
            let t = topology(
                Rule::LineMustNotCrossPolygonBoundaries,
                shapes[start..start + 2].to_vec(),
                true,
                tolerance,
            );
            let eager = t.validate().unwrap();
            assert!(eager.violation_count() > 0);
            if tolerance > 0. {
                assert_eq!(eager.violation_count(), 1);
            }
            let mut streamed = Vec::new();
            t.validate_into(|_, v| {
                streamed.push(v);
                Ok::<_, ()>(())
            })
            .unwrap();
            assert_eq!(streamed, eager.rules[0].violations);
        }
    }
    for before in [-2., 2.] {
        for after in [-2., 2.] {
            let violations = run(
                Rule::LineMustNotCrossPolygonBoundaries,
                vec![line(&[(before, 2.), (0., 2.), (0., 5.), (after, 5.)])],
                vec![square()],
                0.01,
            );
            assert_eq!(violations.len(), usize::from(before != after));
        }
    }
}

#[test]
fn original_junction_contacts_do_not_expand_into_overlaps() {
    let shapes = fixture("junction_contacts");
    for tolerance in [0., 0.001, 1.] {
        for rule in [
            Rule::LineMustNotOverlap,
            Rule::LineMustNotIntersect,
            Rule::LineMustNotOverlapWith,
            Rule::LineMustNotIntersectWith,
        ] {
            for start in [0, 2] {
                for reverse in [false, true] {
                    let mut pair = shapes[start..start + 2].to_vec();
                    if reverse {
                        pair.reverse();
                    }
                    let t = topology(rule, pair, rule.inputs().1.is_some(), tolerance);
                    assert_eq!(t.validate().unwrap().violation_count(), 0, "{rule}");
                    let summary = t.validate_into(|_, _| Ok::<_, ()>(())).unwrap();
                    assert_eq!(summary.violation_count(), 0);
                }
            }
        }
        for rule in [Rule::LineMustNotSelfOverlap, Rule::LineMustNotSelfIntersect] {
            let t = topology(rule, vec![shapes[4].clone()], false, tolerance);
            assert_eq!(t.validate().unwrap().violation_count(), 0, "{rule}");
        }
    }
}

#[test]
fn boundary_proximity_requires_passages_on_both_sides() {
    use topology_checker_core::geo::MapCoords;
    let shapes = fixture("boundary_proximity");
    for offset in [coord! {x: 0., y: 0.}, coord! {x: 1048576., y: -524288.}] {
        for pair in shapes.as_chunks::<2>().0 {
            let transformed = pair
                .iter()
                .map(|g| {
                    g.map_coords(|p| Coord {
                        x: p.x + offset.x,
                        y: p.y + offset.y,
                    })
                })
                .collect();
            let t = topology(
                Rule::LineMustNotCrossPolygonBoundaries,
                transformed,
                true,
                0.001,
            );
            assert_eq!(t.validate().unwrap().violation_count(), 0);
            assert_eq!(
                t.validate_into(|_, _| Ok::<_, ()>(()))
                    .unwrap()
                    .violation_count(),
                0
            );
        }
    }
}

#[test]
fn vertices_just_beyond_the_boundary_band_remain_passages() {
    use topology_checker_core::geo::CoordsIter;
    let d = 0.25_f64.next_up();
    let square =
        Geometry::from(Rect::new(coord! {x: 0., y: 0.}, coord! {x: 8., y: 8.}).to_polygon());
    // The outside vertex lies on a later segment, where rounded band parameters
    // reach it in either direction.
    for (coords, crossings) in [
        (vec![(4., 2.), (4., 4.), (-d, 4.)], 1),
        (vec![(4., 2.), (-d, 4.), (4., 6.)], 2),
    ] {
        for reverse in [false, true] {
            let mut line = LineString::from(coords.clone());
            if reverse {
                line.0.reverse();
            }
            let report = topology(
                Rule::LineMustNotCrossPolygonBoundaries,
                vec![line.into(), square.clone()],
                true,
                0.25,
            )
            .validate()
            .unwrap();
            assert!(
                report
                    .violations()
                    .all(|v| v.kind == ViolationKind::Crossing)
            );
            let points: usize = report.violations().map(|v| v.geometry.coords_count()).sum();
            assert_eq!(points, crossings, "{coords:?}, reversed: {reverse}");
        }
    }
}

#[test]
fn nearby_junction_support_is_not_extended_into_shared_linework() {
    let shapes = fixture("junction_support");
    for (case, pair) in shapes.as_chunks::<2>().0.iter().enumerate() {
        for tolerance in [0., 1.] {
            for rule in [Rule::LineMustNotOverlap, Rule::LineMustNotOverlapWith] {
                let t = topology(rule, pair.to_vec(), rule.inputs().1.is_some(), tolerance);
                assert_eq!(
                    t.validate().unwrap().violation_count(),
                    0,
                    "{rule}, case {case}, tolerance {tolerance}"
                );
                assert_eq!(
                    t.validate_into(|_, _| Ok::<_, ()>(()))
                        .unwrap()
                        .violation_count(),
                    0
                );
            }
        }
    }
}

#[test]
fn compact_bent_and_segmented_nearby_runs_keep_longitudinal_support() {
    let cases = [
        (
            line(&[(-0.6, 0.), (-0.6, 0.6), (0.6, 0.6), (0.6, 0.), (0., 0.)]),
            line(&[(0., 0.), (-0.5, 0.), (-0.5, 0.5), (0.5, 0.5), (0.5, 0.)]),
        ),
        (
            line(&[
                (0., 0.),
                (0.5, 0.),
                (1., 0.),
                (1.5, 0.),
                (2., 0.),
                (2.5, 0.),
                (3., 0.),
                (3.5, 0.),
                (4., 0.),
            ]),
            line(&[(0., 0.25), (4., 0.25)]),
        ),
        (
            line(&[(0., 0.), (1.5, 0.)]),
            line(&[(0., 0.25), (1.5, 0.25)]),
        ),
        (
            line(&[(-0.75, -0.05), (0.75, 0.05)]),
            line(&[(-0.75, 0.05), (0.75, -0.05)]),
        ),
        (
            line(&[(0., 0.), (0.5, 0.), (1., 0.), (1.5, 0.)]),
            line(&[(0., 0.), (0.5, 0.), (1., 0.), (1.5, 0.)]),
        ),
    ];
    for (a, b) in cases {
        for rule in [Rule::LineMustNotOverlap, Rule::LineMustNotOverlapWith] {
            let t = topology(
                rule,
                vec![a.clone(), b.clone()],
                rule.inputs().1.is_some(),
                1.,
            );
            let report = t.validate().unwrap();
            assert_eq!(report.violation_count(), 1, "{a:?}, {b:?}");
            assert_eq!(
                report.violations().next().unwrap().kind,
                ViolationKind::Overlap
            );
        }
    }
}

#[test]
fn short_boundary_contributions_are_unioned_before_the_final_length_threshold() {
    let polygons = (0..6)
        .map(|i| {
            geo::Rect::new(
                coord! {x:i as f64 /2.,y:0.},
                coord! {x:(i+1) as f64/2.,y:10.},
            )
            .to_polygon()
            .into()
        })
        .collect();
    assert!(
        run(
            Rule::LineMustBeCoveredByPolygonBoundaries,
            vec![line(&[(0., 0.), (3., 0.)])],
            polygons,
            1.
        )
        .is_empty()
    );
    let mut lines = (0..6)
        .map(|i| {
            line(&[
                (i as f64 / 2., 0.),
                ((i + 1) as f64 / 2., 0.),
                ((i + 1) as f64 / 2., -10.),
            ])
        })
        .collect::<Vec<_>>();
    lines.extend([
        line(&[(0., 0.), (0., 10.)]),
        line(&[(0., 10.), (3., 10.)]),
        line(&[(3., 0.), (3., 10.)]),
    ]);
    assert!(
        run(
            Rule::PolygonBoundariesMustBeCoveredByLines,
            vec![
                geo::Rect::new(coord! {x:0.,y:0.}, coord! {x:3.,y:10.})
                    .to_polygon()
                    .into()
            ],
            lines,
            1.
        )
        .is_empty()
    );
}

#[test]
fn closed_nearby_run_support_connects_across_rotated_source_seams() {
    let a = [(0., 0.), (0.75, 0.), (0.75, 5.), (-0.75, 5.), (-0.75, 0.)];
    let b = [
        (0., 0.25),
        (0.75, 0.25),
        (0.75, -5.),
        (-0.75, -5.),
        (-0.75, 0.25),
    ];
    for rotate_a in 0..a.len() {
        for rotate_b in 0..b.len() {
            let ring = |coords: &[(f64, f64)], rotate: usize| {
                let mut coords = coords[rotate..]
                    .iter()
                    .chain(&coords[..rotate])
                    .copied()
                    .collect::<Vec<_>>();
                coords.push(coords[0]);
                line(&coords)
            };
            let report = run(
                Rule::LineMustNotOverlapWith,
                vec![ring(&a, rotate_a)],
                vec![ring(&b, rotate_b)],
                1.,
            );
            assert_eq!(report.len(), 1, "rotations {rotate_a}, {rotate_b}");
            assert_eq!(report[0].kind, ViolationKind::Overlap);
        }
    }
}

#[test]
fn uncertain_path_prefixes_preserve_original_interval_evidence() {
    for (a, b, tolerance) in [
        (
            line(&[(0., 0.), (9e307, 0.), (0., 0.), (9e307, 0.)]),
            line(&[(0., 1e300), (9e307, 1e300), (0., 1e300), (9e307, 1e300)]),
            1e301,
        ),
        (
            line(&[(1e16, 1e16), (0., 1e16), (0., 0.), (2., 0.)]),
            line(&[(0., 0.25), (1.5, 0.25)]),
            1.,
        ),
    ] {
        let errors = run(Rule::LineMustNotOverlapWith, vec![a], vec![b], tolerance);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].kind, ViolationKind::Overlap);
    }
}

#[test]
fn connected_shared_support_combines_short_contributions_across_parts() {
    for offset in [0., 0.25] {
        let a = line(&[(0., 0.), (3., 0.)]);
        let b: Geometry = MultiLineString(
            (0..6)
                .map(|i| {
                    LineString::from(vec![
                        (i as f64 / 2., offset),
                        ((i + 1) as f64 / 2., offset),
                        ((i + 1) as f64 / 2., -10.),
                    ])
                })
                .collect(),
        )
        .into();
        let errors = run(
            Rule::LineMustNotOverlapWith,
            vec![a.clone()],
            vec![b.clone()],
            1.,
        );
        assert_eq!(errors.len(), 1);
        let Geometry::MultiLineString(mut combined) = b else {
            unreachable!()
        };
        let Geometry::LineString(a) = a else {
            unreachable!()
        };
        combined.0.push(a);
        let t = topology(
            Rule::LineMustNotSelfOverlap,
            vec![combined.into()],
            false,
            1.,
        );
        let report = t.validate().unwrap();
        assert_eq!(report.violation_count(), 1);
        // Accepted vertical passages must not hide the connected horizontal run.
        let geometry = &report.violations().next().unwrap().geometry;
        let lines = match geometry {
            Geometry::LineString(line) => vec![line],
            Geometry::MultiLineString(lines) => lines.0.iter().collect(),
            _ => unreachable!(),
        };
        assert!(
            lines
                .iter()
                .any(|line| line.0.iter().any(|p| p.x == 0. && p.y == 0.)
                    && line.0.iter().any(|p| p.x == 3. && p.y == 0.)),
            "{geometry:?}"
        );
    }
}

#[test]
fn duplicate_and_disconnected_parts_cannot_restore_short_junction_support() {
    let shapes = fixture("junction_support");
    let parts = |geometry: &Geometry| match geometry {
        Geometry::LineString(line) => vec![line.clone()],
        Geometry::MultiLineString(lines) => lines.0.clone(),
        _ => unreachable!(),
    };
    for pair in shapes.as_chunks::<2>().0 {
        for duplicated in 0..2 {
            let mut a = parts(&pair[0]);
            let mut b = parts(&pair[1]);
            if duplicated == 0 {
                a.extend(a.clone());
            } else {
                b.extend(b.clone());
            }
            let errors = run(
                Rule::LineMustNotOverlapWith,
                vec![MultiLineString(a.clone()).into()],
                vec![MultiLineString(b.clone()).into()],
                1.,
            );
            assert!(errors.is_empty(), "duplicate side {duplicated}: {errors:?}");
            a.extend(b);
            // The distant three-way duplicate has ambiguous passage roles.
            // Its legitimate overlap must not resurrect the rejected junction.
            let distant = LineString::from(vec![(1e7, 0.), (1e7 + 10., 0.)]);
            a.extend([distant.clone(), distant.clone(), distant]);
            for reverse in [false, true] {
                let mut a = a.clone();
                if reverse {
                    a.reverse();
                    for part in &mut a {
                        part.0.reverse();
                    }
                }
                let t = topology(
                    Rule::LineMustNotSelfOverlap,
                    vec![MultiLineString(a).into()],
                    false,
                    1.,
                );
                let report = t.validate().unwrap();
                assert_eq!(report.violation_count(), 1);
                let error = report.violations().next().unwrap();
                // Only original duplicate linework, never a capped junction approach.
                use topology_checker_core::geo::Length;
                let expected = geo_length(&pair[duplicated]) + 20.;
                let actual = match &error.geometry {
                    Geometry::LineString(line) => {
                        topology_checker_core::geo::Euclidean.length(line)
                    }
                    Geometry::MultiLineString(lines) => {
                        topology_checker_core::geo::Euclidean.length(lines)
                    }
                    _ => unreachable!(),
                };
                assert!(
                    (actual - expected).abs() < 1e-6,
                    "{actual} versus {expected}"
                );
            }
        }
    }
}
fn geo_length(geometry: &Geometry) -> f64 {
    use topology_checker_core::geo::{Euclidean, Length};
    match geometry {
        Geometry::LineString(line) => Euclidean.length(line),
        Geometry::MultiLineString(lines) => Euclidean.length(lines),
        _ => unreachable!(),
    }
}

#[test]
fn each_multipart_passage_requires_its_own_connected_source_support() {
    let a = LineString::from(vec![(0., 0.), (1.5, 0.)]);
    let b = LineString::from(vec![(0.75, 0.5), (10., 0.5)]);
    let c = LineString::from(vec![(0.75, 1.25), (10., 1.25)]);
    for reverse in [false, true] {
        let mut parts = vec![a.clone(), b.clone(), c.clone()];
        if reverse {
            parts.reverse();
            for part in &mut parts {
                part.0.reverse();
            }
        }
        let t = topology(
            Rule::LineMustNotSelfOverlap,
            vec![MultiLineString(parts).into()],
            false,
            1.,
        );
        let report = t.validate().unwrap();
        assert_eq!(report.violation_count(), 1);
        let Geometry::LineString(line) = &report.violations().next().unwrap().geometry else {
            panic!("single connected overlap");
        };
        assert!(line.0.iter().all(|p| p.y == 0.5), "{line:?}");
        let errors = run(
            Rule::LineMustNotOverlapWith,
            vec![MultiLineString(vec![a.clone(), c.clone()]).into()],
            vec![b.clone().into()],
            1.,
        );
        assert_eq!(errors.len(), 1);
        let Geometry::LineString(line) = &errors[0].geometry else {
            panic!("single connected overlap");
        };
        assert!(line.0.iter().all(|p| p.y == 0.5), "{line:?}");
    }
}

#[test]
fn disconnected_short_support_is_not_joined_through_excluded_bridge_correspondences() {
    let left = vec![vec![(0., 0.), (1.5, 0.)], vec![(10., 0.), (11.5, 0.)]];
    let right = vec![
        vec![(0.75, 0.5), (2.25, 0.5)],
        vec![(10.75, 0.5), (12.25, 0.5)],
    ];
    let bridges = vec![
        vec![(1.5, 0.), (2., 100.), (12., 100.), (11.5, 0.)],
        vec![(0.75, 0.5), (-100., -101.), (-100., -201.), (10.75, 0.5)],
    ];
    for reverse in [false, true] {
        let build = |mut pieces: Vec<Vec<(f64, f64)>>| {
            pieces.extend(bridges.clone());
            if reverse {
                pieces.reverse();
                for line in &mut pieces {
                    line.reverse();
                }
            }
            Geometry::from(MultiLineString(
                pieces.into_iter().map(LineString::from).collect(),
            ))
        };
        let errors = run(
            Rule::LineMustNotOverlapWith,
            vec![build(left.clone())],
            vec![build(right.clone())],
            1.,
        );
        assert_eq!(errors.len(), 1);
        let lines = match &errors[0].geometry {
            Geometry::LineString(line) => vec![line],
            Geometry::MultiLineString(lines) => lines.0.iter().collect(),
            _ => unreachable!(),
        };
        assert!(
            lines
                .iter()
                .all(|line| !line.0.windows(2).any(|p| p[0].y == 0. && p[1].y == 0.)),
            "{errors:?}"
        );
    }
}

#[test]
fn source_endpoint_junctions_remain_endpoint_contacts_after_clustering() {
    use topology_checker_core::geo::MapCoords;
    let shapes = fixture("endpoint_junction_meetings");
    for pair in shapes.as_chunks::<2>().0 {
        for tolerance in [0., 0.001, 1.] {
            for shift in [0., 1048576.] {
                for scale in [0.5, 1., 2.] {
                    let transformed = pair
                        .iter()
                        .map(|g| {
                            g.map_coords(|p| Coord {
                                x: p.x * scale + shift,
                                y: p.y * scale - shift / 2.,
                            })
                        })
                        .collect::<Vec<_>>();
                    for rule in [
                        Rule::LineMustNotIntersect,
                        Rule::LineMustNotIntersectWith,
                        Rule::LineMustNotIntersectOrTouchInterior,
                        Rule::LineMustNotIntersectOrTouchInteriorWith,
                    ] {
                        let t = topology(
                            rule,
                            transformed.clone(),
                            rule.inputs().1.is_some(),
                            tolerance * scale,
                        );
                        let report = t.validate().unwrap();
                        assert!(
                            report
                                .violations()
                                .all(|v| v.kind != ViolationKind::Crossing),
                            "{rule}: {report:?}"
                        );
                        let mut streamed = Vec::new();
                        t.validate_into(|_, v| {
                            streamed.push(v);
                            Ok::<_, ()>(())
                        })
                        .unwrap();
                        assert_eq!(streamed, report.rules[0].violations);
                    }
                }
            }
        }
    }
}
#[test]
fn original_interior_contacts_survive_unrelated_endpoint_clustering() {
    for tolerance in [0., 0.25] {
        let a = line(&[(-10., 0.), (10., 0.)]);
        for (b, expected) in [
            (line(&[(-10., -0.5), (10., 0.5)]), ViolationKind::Crossing),
            (
                line(&[(-2., 2.), (0., 0.), (2., 2.)]),
                ViolationKind::Crossing,
            ),
            (
                line(&[(0., 0.125), (0., 10.)]),
                ViolationKind::TouchInterior,
            ),
        ] {
            for rule in [
                Rule::LineMustNotIntersectWith,
                Rule::LineMustNotIntersectOrTouchInteriorWith,
            ] {
                let errors = run(
                    rule,
                    vec![a.clone(), line(&[(0., 0.2), (100., 100.)])],
                    vec![b.clone()],
                    tolerance,
                );
                let contacts = errors
                    .iter()
                    .filter(|v| v.origin.unwrap().fid == -20 && v.kind != ViolationKind::Overlap)
                    .collect::<Vec<_>>();
                let want = if expected == ViolationKind::TouchInterior {
                    tolerance > 0. && rule == Rule::LineMustNotIntersectOrTouchInteriorWith
                } else {
                    true
                };
                assert_eq!(contacts.len(), usize::from(want), "{errors:?}");
                if want {
                    assert_eq!(contacts[0].kind, expected);
                }
            }
        }
    }
}

#[test]
fn connected_original_multipart_endpoints_form_through_passages() {
    let a: Geometry = MultiLineString(vec![
        LineString::from(vec![(-10., 0.), (0., 0.)]),
        LineString::from(vec![(0., 0.), (10., 0.)]),
    ])
    .into();
    let b = line(&[(0., -10.), (0., 10.)]);
    for tolerance in [0., 0.1] {
        for rule in [
            Rule::LineMustNotIntersectWith,
            Rule::LineMustNotIntersectOrTouchInteriorWith,
        ] {
            let errors = run(rule, vec![a.clone()], vec![b.clone()], tolerance);
            assert_eq!(errors.len(), 1);
            assert_eq!(errors[0].kind, ViolationKind::Crossing);
            assert_eq!(errors[0].geometry, point(0., 0.));
        }
    }
    // Distinct endpoints that merely cluster together remain separate junctions.
    let a: Geometry = MultiLineString(vec![
        LineString::from(vec![(-10., 0.), (0., 0.)]),
        LineString::from(vec![(0.05, 0.), (10., 0.)]),
    ])
    .into();
    assert!(run(Rule::LineMustNotIntersectWith, vec![a], vec![b], 0.1).is_empty());
}

#[test]
fn disconnected_emitted_overlap_cannot_subsume_discarded_short_contact_support() {
    let a = LineString::from(vec![(-10., 2.), (-0.4, 0.), (0.4, 0.), (10., -2.)]);
    let b = LineString::from(vec![(-10., -0.5), (10., 0.5)]);
    let shared = LineString::from(vec![(-10., 0.8), (10., 0.8)]);
    for rule in [
        Rule::LineMustNotIntersectWith,
        Rule::LineMustNotIntersectOrTouchInteriorWith,
    ] {
        let errors = run(
            rule,
            vec![MultiLineString(vec![a.clone(), shared.clone()]).into()],
            vec![MultiLineString(vec![b.clone(), shared.clone()]).into()],
            1.,
        );
        assert!(
            errors.iter().any(|v| v.kind == ViolationKind::Crossing
                && v.origin.unwrap().part == Some(0)
                && v.destination.unwrap().part == Some(0)),
            "{errors:?}"
        );
    }
}

#[test]
fn detached_runs_in_the_same_parts_cannot_suppress_original_passage_contacts() {
    let a = line(&[
        (-10., 0.8),
        (10., 0.8),
        (20., 10.),
        (-20., 10.),
        (-10., 2.),
        (-0.4, 0.),
        (0.4, 0.),
        (10., -2.),
    ]);
    let b = line(&[
        (-10., 0.8),
        (10., 0.8),
        (20., -10.),
        (-20., -10.),
        (-10., -0.5),
        (10., 0.5),
    ]);
    for rule in [
        Rule::LineMustNotIntersectWith,
        Rule::LineMustNotIntersectOrTouchInteriorWith,
    ] {
        let errors = run(rule, vec![a.clone()], vec![b.clone()], 1.);
        assert!(
            errors.iter().any(|v| v.kind == ViolationKind::Crossing),
            "{errors:?}"
        );
    }
}
#[test]
fn line_endpoints_inside_retained_overlaps_are_not_interior_contacts() {
    use topology_checker_core::geo::{Coord, MapCoords};
    // One line of each pair ends inside its overlap with the other. The cracked
    // endpoint and the overlap span are separately rounded forms of one parameter.
    let shapes = fixture("endpoints_inside_overlaps");
    for (pair, tolerance) in shapes.as_chunks::<2>().0.iter().zip([0.001, 0.001, 1.]) {
        let mut frames = vec![
            pair.to_vec(),
            pair.iter()
                .map(|g| {
                    g.map_coords(|p| Coord {
                        x: p.x + 1048576.,
                        y: p.y - 524288.,
                    })
                })
                .collect(),
        ];
        for line in 0..2 {
            for end in [true, false] {
                for axis in 0..2 {
                    for step in [f64::next_up, f64::next_down] {
                        let mut moved = pair.to_vec();
                        let Geometry::LineString(l) = &mut moved[line] else {
                            unreachable!()
                        };
                        let vertex = if end { l.0.len() - 1 } else { 0 };
                        let c = &mut l.0[vertex];
                        if axis == 0 {
                            c.x = step(c.x);
                        } else {
                            c.y = step(c.y);
                        }
                        frames.push(moved);
                    }
                }
            }
        }
        for frame in frames {
            for rule in [
                Rule::LineMustNotIntersect,
                Rule::LineMustNotIntersectWith,
                Rule::LineMustNotIntersectOrTouchInterior,
                Rule::LineMustNotIntersectOrTouchInteriorWith,
            ] {
                let t = topology(rule, frame.clone(), rule.inputs().1.is_some(), tolerance);
                let report = t.validate().unwrap();
                let kinds = report.violations().map(|v| v.kind).collect::<Vec<_>>();
                assert_eq!(kinds, [ViolationKind::Overlap], "{rule}: {report:?}");
                let mut streamed = Vec::new();
                t.validate_into(|_, v| {
                    streamed.push(v);
                    Ok::<_, ()>(())
                })
                .unwrap();
                assert_eq!(streamed, report.rules[0].violations);
            }
        }
    }
}
#[test]
fn oblique_endpoints_inside_retained_overlaps_are_not_interior_contacts() {
    // The endpoint (5, 0.9) is inside the overlap, but its projection onto the
    // longer line differs from the span end by its offset times the slope.
    let pair = vec![
        line(&[(-6., 1.12), (5., 0.9)]),
        line(&[(0., 0.), (10., 0.)]),
    ];
    for rule in [
        Rule::LineMustNotIntersectOrTouchInterior,
        Rule::LineMustNotIntersectOrTouchInteriorWith,
    ] {
        let t = topology(rule, pair.clone(), rule.inputs().1.is_some(), 1.);
        let report = t.validate().unwrap();
        let kinds = report.violations().map(|v| v.kind).collect::<Vec<_>>();
        assert_eq!(kinds, [ViolationKind::Overlap], "{rule}: {report:?}");
    }
}
/// The input, four translations and every coordinate of each line moved by one
/// unit in the last place in each direction.
fn rounding_frames(lines: &[Geometry]) -> Vec<Vec<Geometry>> {
    use topology_checker_core::geo::MapCoords;
    let mut frames = vec![lines.to_vec()];
    for (dx, dy) in [
        (1048576., -524288.),
        (-325000., 162500.),
        (7000., -3500.),
        (0.1, 0.3),
    ] {
        frames.push(
            lines
                .iter()
                .map(|g| {
                    g.map_coords(|p| Coord {
                        x: p.x + dx,
                        y: p.y + dy,
                    })
                })
                .collect(),
        );
    }
    for index in 0..lines.len() {
        for step in [f64::next_up, f64::next_down] {
            for axis in 0..2 {
                let mut moved = lines.to_vec();
                moved[index] = moved[index].map_coords(|mut p| {
                    if axis == 0 {
                        p.x = step(p.x);
                    } else {
                        p.y = step(p.y);
                    }
                    p
                });
                frames.push(moved);
            }
        }
    }
    frames
}
fn assert_line_meetings(lines: &[Geometry], tolerance: f64, expected: &[ViolationKind]) {
    for frame in rounding_frames(lines) {
        for rule in [
            Rule::LineMustNotIntersect,
            Rule::LineMustNotIntersectWith,
            Rule::LineMustNotIntersectOrTouchInterior,
            Rule::LineMustNotIntersectOrTouchInteriorWith,
        ] {
            let t = topology(rule, frame.clone(), rule.inputs().1.is_some(), tolerance);
            let report = t.validate().unwrap();
            let kinds = report.violations().map(|v| v.kind).collect::<Vec<_>>();
            assert_eq!(kinds, expected, "{rule}: {report:?}");
            let mut streamed = Vec::new();
            t.validate_into(|_, v| {
                streamed.push(v);
                Ok::<_, ()>(())
            })
            .unwrap();
            assert_eq!(streamed, report.rules[0].violations, "{rule}");
        }
    }
}
#[test]
fn vertices_on_lines_at_overlap_ends_meet_the_same_way_in_every_frame() {
    // The vertex (10.1, 3.535) lies on the second line up to rounding and ends the
    // overlap. The first line then leaves on the same side, or crosses there.
    let other = line(&[(0., 0.), (20., 7.)]);
    for tolerance in [1., 0.001] {
        for departure in [2., -2.] {
            let first = line(&[
                (2., 2. * 0.35 + 0.5 * tolerance),
                (10.1, 10.1 * 0.35),
                (12., 12. * 0.35 + departure * tolerance),
            ]);
            assert_line_meetings(
                &[first, other.clone()],
                tolerance,
                &[ViolationKind::Overlap],
            );
        }
    }
    // An endpoint one unit in the last place past an interior vertex of the line it
    // overlaps, and an endpoint next to the seam of a ring it overlaps.
    let shared = line(&[(-3., 0.5), (5., 0.5), (10., 0.5)]);
    let ring = line(&[
        (5., 0.5),
        (10., 0.5),
        (10., 5.),
        (0., 5.),
        (0., 0.5),
        (5., 0.5),
    ]);
    for end in [5., 5_f64.next_up()] {
        for first in [&shared, &ring] {
            assert_line_meetings(
                &[first.clone(), line(&[(-5., 0.), (end, 0.)])],
                1.,
                &[ViolationKind::Overlap],
            );
        }
    }
}
#[test]
fn crossings_beside_overlap_ends_and_rounded_tangencies_are_kept_in_every_frame() {
    // The vertex (10, 0.5) ends the overlap away from the other line, and the next
    // segment crosses it 0.007 further on.
    let other = line(&[(0., 0.), (20., 0.)]);
    assert_line_meetings(
        &[line(&[(0., 0.5), (10., 0.5), (10.05, -3.)]), other],
        1.,
        &[ViolationKind::Overlap, ViolationKind::Crossing],
    );
    // A vertex on the other line up to rounding, without an overlap, touches it from
    // one side or crosses it.
    let other = line(&[(0., 0.), (20., 7.)]);
    for departure in [1., -1.] {
        let first = line(&[
            (8., 8. * 0.35 + 1.),
            (10.1, 10.1 * 0.35),
            (12., 12. * 0.35 + departure),
        ]);
        assert_line_meetings(&[first, other.clone()], 0.25, &[ViolationKind::Crossing]);
    }
}
#[test]
fn endpoint_touches_are_kept_when_their_projection_joins_another_node() {
    // Both endpoints project to (5, 0) on the first line. That point joins the node of
    // the nearer endpoint at (5, -0.8), not the node of (5, 0.9).
    let lines = [
        line(&[(0., 0.), (10., 0.)]),
        line(&[(5., -0.8), (5., -5.)]),
        line(&[(5., 0.9), (5., 5.)]),
    ];
    for rule in [
        Rule::LineMustNotIntersectOrTouchInterior,
        Rule::LineMustNotIntersectOrTouchInteriorWith,
    ] {
        let mut t = Topology::new(1.).unwrap();
        let features = lines
            .iter()
            .cloned()
            .enumerate()
            .map(|(i, g)| (i as i64, g));
        if rule.inputs().1.is_some() {
            let mut features = features.collect::<Vec<_>>();
            let target = t.add_layer(Layer::from_features("target", features.split_off(1)));
            let source = t.add_layer(Layer::from_features("source", features));
            t.add_rule(rule, source, Some(target)).unwrap();
        } else {
            let layer = t.add_layer(Layer::from_features("lines", features));
            t.add_rule(rule, layer, None).unwrap();
        }
        let report = t.validate().unwrap();
        let touches = report
            .violations()
            .map(|v| (v.kind, v.geometry.clone()))
            .collect::<Vec<_>>();
        assert_eq!(
            touches,
            [
                (ViolationKind::TouchInterior, point(5., -0.8)),
                (ViolationKind::TouchInterior, point(5., 0.9)),
            ],
            "{rule}"
        );
        let mut streamed = Vec::new();
        t.validate_into(|_, v| {
            streamed.push(v);
            Ok::<_, ()>(())
        })
        .unwrap();
        assert_eq!(streamed, report.rules[0].violations, "{rule}");
    }
}
#[test]
fn rounded_vertices_meet_lines_only_within_tolerance() {
    // Each middle vertex is ten and sixteen units in the last place off the first line:
    // 9.3e-9 at 5e6 and 2 at 1e15. That is within rounding of these coordinates, so it
    // meets the line within tolerance, but never beyond it.
    for (lines, beyond, within) in [
        (
            [
                line(&[(5e6, 5e6), (5e6 + 10., 5e6)]),
                line(&[
                    (5e6 + 4., 5e6 + 2.),
                    (5e6 + 5., 5e6 + 10. * 2_f64.powi(-30)),
                    (5e6 + 6., 5e6 + 2.),
                ]),
            ],
            1e-9,
            1e-8,
        ),
        (
            [
                line(&[(1e15, 1e15), (1e15 + 100., 1e15)]),
                line(&[
                    (1e15 + 40., 1e15 + 50.),
                    (1e15 + 50., 1e15 + 2.),
                    (1e15 + 60., 1e15 + 50.),
                ]),
            ],
            1.,
            2.5,
        ),
    ] {
        assert_line_meetings(&lines, beyond, &[]);
        for rule in [Rule::LineMustNotIntersect, Rule::LineMustNotIntersectWith] {
            let t = topology(rule, lines.to_vec(), rule.inputs().1.is_some(), within);
            let report = t.validate().unwrap();
            let kinds = report.violations().map(|v| v.kind).collect::<Vec<_>>();
            assert_eq!(kinds, [ViolationKind::Crossing], "{rule}: {report:?}");
        }
    }
}
#[test]
fn crossings_beside_rounded_vertices_are_counted_once_in_every_frame() {
    // A vertex of the ring at (-263.0470419040875, 665.975889234363) lies on the first
    // line up to rounding, beside an exact crossing a few units in the last place away.
    let lines = [
        line(&[
            (-260.9313581947057, 662.4879633963319),
            (-263.34307200782433, 666.4639258170903),
            (-265.7547858209429, 670.4398882378487),
        ]),
        MultiLineString(vec![
            LineString::from(vec![
                (-265.9871113056022, 662.2784314330856),
                (-263.3989446666477, 665.2692641805332),
            ]),
            LineString::from(vec![
                (-263.0470419040875, 665.975889234363),
                (-265.3919752665837, 666.6081248816553),
                (-263.7137051171506, 671.294364393417),
                (-263.0470419040875, 665.975889234363),
            ]),
        ])
        .into(),
    ];
    assert_line_meetings(
        &lines,
        1.,
        &[ViolationKind::Crossing, ViolationKind::Crossing],
    );
}

mod zero_tolerance_contacts {
    //! Decimal data that touches in decimal can, in binary, cross, touch or
    //! miss a polygon by less than one floating-point step. At zero tolerance
    //! the line/polygon rules must follow the exact binary geometry in every
    //! exact frame, and emit each overlap run as a non-degenerate piece within
    //! its line segments.
    use super::exact_line_polygon::{Reference, Run, bits, brackets, exact, reference};
    use topology_checker_core::{
        Layer, Rule, Topology, Violation, ViolationKind,
        geo::{Coord, Geometry, LineString, Polygon},
    };

    const RULES: [Rule; 3] = [
        Rule::LineMustNotOverlapWithPolygons,
        Rule::LineMustNotIntersectPolygons,
        Rule::LineMustNotCrossPolygonBoundaries,
    ];
    type Case = (Vec<Coord>, Vec<Vec<Coord>>);
    type Family = (&'static str, fn(&mut Generator) -> Case);

    /// Violations of the three pair rules, or `None` for a rejected polygon.
    fn validate(line: &[Coord], rings: &[Vec<Coord>]) -> Option<Vec<Vec<Violation>>> {
        let mut t = Topology::new(0.).unwrap();
        let a = t.add_layer(Layer::from_geometries(
            "lines",
            [Geometry::LineString(LineString(line.to_vec()))],
        ));
        let polygon = Polygon::new(
            LineString(rings[0].clone()),
            rings[1..].iter().cloned().map(LineString).collect(),
        );
        let b = t.add_layer(Layer::from_geometries("polygons", [polygon.into()]));
        let invalid = |e| match e {
            topology_checker_core::Error::InvalidPolygon { .. } => None,
            e => panic!("{e}"),
        };
        for rule in RULES {
            if let Err(e) = t.add_rule(rule, a, Some(b)) {
                return invalid(e);
            }
        }
        match t.validate() {
            Ok(report) => Some(report.rules.into_iter().map(|r| r.violations).collect()),
            Err(e) => invalid(e),
        }
    }
    fn parts(g: &Geometry, lines: &mut Vec<LineString>, points: &mut Vec<Coord>) {
        match g {
            Geometry::LineString(l) => lines.push(l.clone()),
            Geometry::MultiLineString(m) => lines.extend(m.0.iter().cloned()),
            Geometry::Point(p) => points.push(p.0),
            Geometry::MultiPoint(m) => points.extend(m.0.iter().map(|p| p.0)),
            Geometry::GeometryCollection(c) => c.iter().for_each(|g| parts(g, lines, points)),
            g => panic!("unexpected geometry {g:?}"),
        }
    }
    /// Interior coordinates are exact; each end is one of the two binary
    /// values adjacent to the exact run end and lies within its segment.
    fn matches(piece: &LineString, run: &Run, line: &[Coord]) -> bool {
        let n = piece.0.len();
        let within = |c: Coord, s: usize| {
            let (a, b) = (line[s], line[s + 1]);
            a.x.min(b.x) <= c.x && c.x <= a.x.max(b.x) && a.y.min(b.y) <= c.y && c.y <= a.y.max(b.y)
        };
        n == run.points.len()
            && piece.0.windows(2).all(|w| w[0] != w[1])
            && (1..n - 1).all(|k| exact(piece.0[k]) == run.points[k])
            && [(0, run.segments[0]), (n - 1, run.segments[1])]
                .into_iter()
                .all(|(k, s)| {
                    brackets(piece.0[k].x, &run.points[k].0)
                        && brackets(piece.0[k].y, &run.points[k].1)
                        && within(piece.0[k], s)
                })
    }
    fn problems(case: &Case, expected: &Reference) -> Option<Vec<&'static str>> {
        let (line, rings) = case;
        let found = validate(line, rings)?;
        let mut out = Vec::new();
        let pieces = |v: &[Violation], kind| {
            let (mut lines, mut points) = (Vec::new(), Vec::new());
            for v in v {
                assert_eq!(v.kind, kind);
                parts(&v.geometry, &mut lines, &mut points);
            }
            (lines, points)
        };
        let runs_match = |lines: &[LineString]| {
            let mut unused = lines.iter().collect::<Vec<_>>();
            lines.len() == expected.runs.len()
                && expected.runs.iter().all(|run| {
                    unused
                        .iter()
                        .position(|l| matches(l, run, line))
                        .map(|i| unused.remove(i))
                        .is_some()
                })
        };
        let (lines, points) = pieces(&found[0], ViolationKind::Overlap);
        if found[0].len() > 1 || !points.is_empty() || !runs_match(&lines) {
            out.push("overlap");
        }
        let (lines, points) = pieces(&found[1], ViolationKind::Intersection);
        let points = points.into_iter().map(bits).collect();
        if found[1].len() > 1 || !runs_match(&lines) || expected.isolated != points {
            out.push("intersect");
        }
        let (_, points) = pieces(&found[2], ViolationKind::Crossing);
        if expected.crossings != points.into_iter().map(bits).collect() {
            out.push("cross");
        }
        Some(out)
    }

    /// Deterministic decimal coordinates at projected-CRS magnitudes.
    struct Generator(u64);
    impl Generator {
        fn next(&mut self, n: i64) -> i64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((self.0 >> 33) % n as u64) as i64
        }
        fn sign(&mut self) -> i64 {
            if self.next(2) == 0 { 1 } else { -1 }
        }
        fn chance(&mut self, percent: i64) -> bool {
            self.next(100) < percent
        }
        /// A cents-based coordinate frame and a direction with its left normal.
        fn frame(&mut self) -> (impl Fn(i64, i64) -> Coord + use<>, [i64; 4]) {
            const MAGNITUDES: [i64; 5] = [310_000, 620_000, 1_250_000, 2_500_000, 4_900_000];
            let mut origin = || MAGNITUDES[self.next(5) as usize] * 100 + self.next(1_000_000);
            let (x0, y0) = (origin(), origin());
            let scale = [2, 4, 8][self.next(3) as usize];
            let dx = scale * (50 + self.next(450)) * self.sign();
            let dy = scale * (50 + self.next(450)) * self.sign();
            let decimal = |c: i64| {
                format!("{}.{:02}", c / 100, c % 100)
                    .parse::<f64>()
                    .unwrap()
            };
            let at = move |u: i64, w: i64| Coord {
                x: decimal(x0 + u),
                y: decimal(y0 + w),
            };
            (at, [dx, dy, -dy, dx])
        }
        /// A polygon edge from `at(0, 0)` to `at(dx, dy)`, with the square on
        /// one side of it.
        fn square(
            &mut self,
            at: &impl Fn(i64, i64) -> Coord,
            [dx, dy, nx, ny]: [i64; 4],
        ) -> Vec<Coord> {
            let side = self.sign();
            vec![
                at(0, 0),
                at(dx, dy),
                at(dx + side * nx / 2, dy + side * ny / 2),
                at(side * nx / 2, side * ny / 2),
                at(0, 0),
            ]
        }
        fn through_vertex(&mut self) -> Case {
            let (at, [dx, dy, nx, ny]) = self.frame();
            let k = [2, 4][self.next(2) as usize];
            let (vx, vy) = (dx / k, dy / k);
            let mut corner = || {
                let (s, f, g) = (self.sign(), 1 + self.next(6), self.next(9) - 4);
                at(
                    vx + s * f * nx / 10 + g * dx / 10,
                    vy + s * f * ny / 10 + g * dy / 10,
                )
            };
            let (p, q) = (corner(), corner());
            let v = at(vx, vy);
            let mut ring = vec![v, p, q, v];
            if self.chance(30) {
                // An edge along the line in decimal.
                ring.insert(1, at(vx + dx / 8, vy + dy / 8));
            }
            let mut line = vec![at(0, 0), at(dx, dy)];
            if self.chance(30) {
                line.push(at(dx + 3 * nx / 10, dy + 3 * ny / 10));
            }
            (line, vec![ring])
        }
        fn vertex_on_edge(&mut self) -> Case {
            let (at, d) = self.frame();
            let [dx, dy, nx, ny] = d;
            let ring = self.square(&at, d);
            let k = [2, 4][self.next(2) as usize];
            let (s1, s2) = (self.sign(), self.sign());
            let first = at(dx / k + s1 * nx / 4 - dx / 4, dy / k + s1 * ny / 4 - dy / 4);
            let last = at(dx / k + s2 * nx / 4 + dx / 4, dy / k + s2 * ny / 4 + dy / 4);
            let v = at(dx / k, dy / k);
            let line = if self.chance(30) {
                // A second touch at another decimal point of the edge.
                let w = at(dx / 2 + s2 * nx / 8, dy / 2 + s2 * ny / 8);
                vec![first, v, w, at(3 * dx / 4, 3 * dy / 4), last]
            } else {
                vec![first, v, last]
            };
            (line, vec![ring])
        }
        fn collinear_run(&mut self) -> Case {
            let (at, d) = self.frame();
            let [dx, dy, nx, ny] = d;
            let ring = self.square(&at, d);
            // Eighths along the edge, possibly beyond its ends.
            const STOPS: [i64; 12] = [-2, -1, 0, 1, 2, 3, 4, 5, 6, 8, 9, 10];
            let (a, b) = (self.next(12) as usize, self.next(11) as usize);
            let b = if b >= a { b + 1 } else { b };
            let (u0, u1) = (STOPS[a.min(b)], STOPS[a.max(b)]);
            let mut line = vec![at(dx * u0 / 8, dy * u0 / 8), at(dx * u1 / 8, dy * u1 / 8)];
            if self.chance(70) {
                let s = self.sign();
                line.insert(
                    0,
                    at(
                        dx * u0 / 8 + s * nx / 4 - dx / 8,
                        dy * u0 / 8 + s * ny / 4 - dy / 8,
                    ),
                );
            }
            if self.chance(70) {
                let s = self.sign();
                line.push(at(
                    dx * u1 / 8 + s * nx / 4 + dx / 8,
                    dy * u1 / 8 + s * ny / 4 + dy / 8,
                ));
            }
            if self.chance(30) {
                line.reverse();
            }
            (line, vec![ring])
        }
        fn hole(&mut self) -> Case {
            let (at, d) = self.frame();
            let [dx, dy, nx, ny] = d;
            let shell = vec![
                at(-dx - nx, -dy - ny),
                at(2 * dx - nx, 2 * dy - ny),
                at(2 * dx + nx, 2 * dy + ny),
                at(-dx + nx, -dy + ny),
                at(-dx - nx, -dy - ny),
            ];
            let hole = self.square(&at, d);
            let (s1, s2) = (self.sign(), self.sign());
            let line = match self.next(3) {
                // Along the hole edge, then turning to either side.
                0 => vec![
                    at(-dx / 4, -dy / 4),
                    at(dx / 2, dy / 2),
                    at(dx / 2 + s1 * nx / 4, dy / 2 + s1 * ny / 4),
                ],
                // Bending at a decimal point of the hole edge.
                1 => vec![
                    at(dx / 4 + s1 * nx / 4, dy / 4 + s1 * ny / 4),
                    at(dx / 2, dy / 2),
                    at(3 * dx / 4 + s2 * nx / 4, 3 * dy / 4 + s2 * ny / 4),
                ],
                // Through a hole vertex.
                _ => vec![
                    at(2 * dx / 3 - nx / 3, 2 * dy / 3 - ny / 3),
                    at(dx, dy),
                    at(4 * dx / 3 + nx / 3, 4 * dy / 3 + ny / 3),
                ],
            };
            (line, vec![shell, hole])
        }
        /// Teeth whose tips lie on the line in decimal: many sub-ulp contacts
        /// on one segment.
        fn comb(&mut self) -> Case {
            let (at, _) = self.frame();
            let teeth = 2 + self.next(7);
            let n = 4 * teeth + 4;
            let dx = n * (20 + self.next(180)) * self.sign();
            let dy = n * (20 + self.next(180)) * self.sign();
            let (nx, ny) = (self.sign() * -dy / 4, self.sign() * dx / 4);
            let along = |u: i64| (dx * u / n, dy * u / n);
            let base =
                |u: i64, depth: i64| at(dx * u / n + depth * nx / 2, dy * u / n + depth * ny / 2);
            let mut ring = vec![base(-1, 3), base(-1, 1)];
            for j in 0..teeth {
                let u = 4 * j + 2;
                ring.push(base(u, 1));
                let (x, y) = along(u + 1);
                ring.push(at(x, y));
                if self.chance(30) {
                    let (x, y) = along(u + 2);
                    ring.push(at(x, y));
                }
                ring.push(base(u + 3, 1));
            }
            ring.extend([base(n + 1, 1), base(n + 1, 3), ring[0]]);
            let mut line = vec![at(0, 0), at(dx, dy)];
            if self.chance(30) {
                // Bend the line at a tooth tip.
                let (x, y) = along(4 * self.next(teeth) + 3);
                line = vec![at(0, 0), at(x, y), at(dx - nx / 4, dy - ny / 4)];
            }
            (line, vec![ring])
        }
        fn endpoint_on_edge(&mut self) -> Case {
            let (at, d) = self.frame();
            let [dx, dy, nx, ny] = d;
            let ring = self.square(&at, d);
            let k = [2, 4][self.next(2) as usize];
            let (s, g) = (self.sign(), self.next(9) - 4);
            let v = at(dx / k, dy / k);
            let other = at(
                dx / k + s * nx / 4 + g * dx / 10,
                dy / k + s * ny / 4 + g * dy / 10,
            );
            let line = if self.chance(50) {
                vec![v, other]
            } else {
                vec![other, v]
            };
            (line, vec![ring])
        }
        /// Close a line of another family, starting at any of its vertices.
        fn closed(&mut self) -> Case {
            let (mut line, rings) = match self.next(3) {
                0 => self.through_vertex(),
                1 => self.vertex_on_edge(),
                _ => self.collinear_run(),
            };
            if line.len() == 2 {
                let (a, b) = (line[0], line[1]);
                let cents = |v: f64| (v * 100.).round() / 100.;
                line.push(Coord {
                    x: cents((a.x + b.x) / 2. + (b.y - a.y) * 0.7),
                    y: cents((a.y + b.y) / 2. - (b.x - a.x) * 0.7),
                });
            }
            let k = self.next(line.len() as i64) as usize;
            line.rotate_left(k);
            line.push(line[0]);
            (line, rings)
        }
    }

    /// Exact transforms of a case: Sterbenz translations, power-of-two
    /// scalings, axis swaps and reflections, line reversal and ring rotation.
    fn frames(case: &Case) -> Vec<(&'static str, Case)> {
        let (line, rings) = case;
        let map = |f: &dyn Fn(Coord) -> Option<Coord>| -> Option<Case> {
            Some((
                line.iter().map(|&c| f(c)).collect::<Option<_>>()?,
                rings
                    .iter()
                    .map(|r| r.iter().map(|&c| f(c)).collect::<Option<_>>())
                    .collect::<Option<_>>()?,
            ))
        };
        let affine = |o: Coord, s: f64| {
            move |c: Coord| {
                let p = Coord {
                    x: (c.x - o.x) * s,
                    y: (c.y - o.y) * s,
                };
                (p.x / s + o.x == c.x && p.y / s + o.y == c.y).then_some(p)
            }
        };
        let first = line[0];
        let floor = Coord {
            x: first.x.floor(),
            y: first.y.floor(),
        };
        let zero = Coord { x: 0., y: 0. };
        let mut out = vec![("original", case.clone())];
        for (name, f) in [
            (
                "sterbenz",
                &affine(first, 1.) as &dyn Fn(Coord) -> Option<Coord>,
            ),
            ("floor", &affine(floor, 1.)),
            ("scale-2", &affine(zero, 2.)),
            ("scale-1/1024", &affine(zero, 1. / 1024.)),
            ("swap-xy", &|c: Coord| Some(Coord { x: c.y, y: c.x })),
            ("negate-x", &|c: Coord| Some(Coord { x: -c.x, y: c.y })),
        ] {
            if let Some(mapped) = map(f) {
                out.push((name, mapped));
            }
        }
        out.push((
            "reversed",
            (line.iter().rev().copied().collect(), rings.clone()),
        ));
        let rotated = rings
            .iter()
            .map(|r| {
                let mut r = r[1..].to_vec();
                let half = r.len() / 2;
                r.rotate_left(half);
                r.reverse();
                r.push(r[0]);
                r
            })
            .collect();
        out.push(("ring-rotation", (line.clone(), rotated)));
        out
    }

    #[test]
    fn sub_ulp_contacts_match_the_exact_reference_in_every_frame() {
        let mut generator = Generator(20261014);
        let families: [Family; 7] = [
            ("through-vertex", Generator::through_vertex),
            ("vertex-on-edge", Generator::vertex_on_edge),
            ("collinear-run", Generator::collinear_run),
            ("hole", Generator::hole),
            ("comb", Generator::comb),
            ("endpoint-on-edge", Generator::endpoint_on_edge),
            ("closed", Generator::closed),
        ];
        let mut failures = std::collections::BTreeMap::<String, usize>::new();
        let (mut runs, mut ends_moved, mut checked, mut invalid) = (0, 0, 0, 0);
        // The exact reference is slow without optimizations.
        let cases = if cfg!(debug_assertions) { 35 } else { 140 };
        for k in 0..cases {
            let (family, make) = families[k % families.len()];
            let case = make(&mut generator);
            if validate(&case.0, &case.1).is_none() {
                invalid += 1;
                continue;
            }
            for (frame, case) in frames(&case) {
                let expected = reference(&case.0, &case.1);
                runs += expected.runs.len();
                ends_moved += expected
                    .runs
                    .iter()
                    .filter(|r| {
                        super::exact_line_polygon::rounded(&r.points[0])
                            == super::exact_line_polygon::rounded(&r.points[r.points.len() - 1])
                    })
                    .count();
                checked += 1;
                let found = problems(&case, &expected).expect("exact frames keep polygons valid");
                for rule in found {
                    *failures
                        .entry(format!("{family} {frame} {rule}"))
                        .or_default() += 1;
                }
            }
        }
        println!(
            "{checked} frames, {invalid} invalid cases skipped, {runs} overlap runs, \
             {ends_moved} with coincident nearest ends"
        );
        assert!(
            invalid * 10 < cases,
            "at most a tenth of the generated polygons are invalid"
        );
        assert!(
            ends_moved > 0,
            "the generator must exercise collapsing runs"
        );
        assert!(failures.is_empty(), "{failures:#?}");
    }

    #[test]
    fn run_ends_round_away_from_the_run_within_the_line() {
        let ring = |c: &[(f64, f64)]| c.iter().map(|&(x, y)| Coord { x, y }).collect::<Vec<_>>();
        // The line ends inside the polygon by less than one step, and its entry
        // crossing rounds onto that end vertex.
        let line = ring(&[(623757.34, 319630.01), (623757.44, 319621.11)]);
        let polygon = ring(&[
            (623766.22, 319622.19),
            (623731.1, 319617.87),
            (623733.26, 319600.31),
            (623768.38, 319604.63),
            (623766.22, 319622.19),
        ]);
        let entry = Coord {
            x: 623757.4399999998,
            y: 319621.11000000004,
        };
        for reversed in [false, true] {
            let mut line = line.clone();
            let mut piece = vec![entry, line[1]];
            if reversed {
                line.reverse();
                piece.reverse();
            }
            let found = validate(&line, std::slice::from_ref(&polygon)).unwrap();
            for v in &found[..2] {
                assert_eq!(v.len(), 1);
                assert_eq!(
                    v[0].geometry,
                    Geometry::LineString(LineString(piece.clone()))
                );
            }
            assert!(found[2].is_empty());
        }
        // A line vertex lies inside a polygon edge by less than one step, and
        // both of its crossings round onto it. The vertex stays, and each end
        // rounds away from it along its own segment.
        let line = ring(&[
            (4906031.44, 317778.98),
            (4906036.98, 317783.42),
            (4906041.42, 317777.88),
        ]);
        let polygon = ring(&[
            (4906031.99, 317783.97),
            (4906051.95, 317781.77),
            (4906053.05, 317791.75),
            (4906033.09, 317793.95),
            (4906031.99, 317783.97),
        ]);
        let rings = [polygon];
        let expected = reference(&line, &rings);
        assert_eq!(expected.runs.len(), 1);
        let found = validate(&line, &rings).unwrap();
        let Geometry::LineString(piece) = &found[0][0].geometry else {
            panic!("one overlap piece");
        };
        assert_eq!(piece.0.len(), 3);
        assert_eq!(piece.0[1], line[1]);
        assert!(matches(piece, &expected.runs[0], &line));
        assert!(piece.0[0].x < line[1].x && piece.0[0].y < line[1].y);
        assert!(piece.0[2].x > line[1].x && piece.0[2].y < line[1].y);
    }

    #[test]
    fn adjacent_pieces_share_their_rounded_crossing() {
        // The crossing at x = 1 is not representable. Both pieces and both
        // crossing points must use the same nearest coordinate.
        let line = Geometry::LineString(LineString::from(vec![(0.25, 0.5), (1.75, 0.6)]));
        let square = |x: f64| {
            Geometry::Polygon(Polygon::new(
                LineString::from(vec![(x, 0.), (x + 1., 0.), (x + 1., 1.), (x, 1.), (x, 0.)]),
                vec![],
            ))
        };
        let shared = Coord {
            x: 1.,
            y: 0.5 + 0.1 * (0.75 / 1.5),
        };
        for rule in [
            Rule::LineMustNotOverlapWithPolygons,
            Rule::LineMustNotCrossPolygonBoundaries,
        ] {
            let mut t = Topology::new(0.).unwrap();
            let a = t.add_layer(Layer::from_geometries("lines", [line.clone()]));
            let b = t.add_layer(Layer::from_geometries("polygons", [square(0.), square(1.)]));
            t.add_rule(rule, a, Some(b)).unwrap();
            let report = t.validate().unwrap();
            let mut ends = Vec::new();
            for v in report.violations() {
                match &v.geometry {
                    Geometry::LineString(l) => ends.extend([l.0[0], l.0[l.0.len() - 1]]),
                    Geometry::Point(p) => ends.push(p.0),
                    g => panic!("{g:?}"),
                }
            }
            let at_shared = ends.iter().filter(|c| c.x == 1.).collect::<Vec<_>>();
            assert_eq!(at_shared.len(), 2, "{rule}: {ends:?}");
            assert!(
                at_shared
                    .iter()
                    .all(|&&c| c == *at_shared[0] && (c.y - shared.y).abs() < 1e-15)
            );
        }
    }
}

/// Exact translations of all coordinates: (+2^20, -2^19) and its opposite where
/// exact, and the shift to the binade below the first coordinate.
fn exact_shifts(lines: &[&LineString]) -> Vec<Coord> {
    let exact = |v: f64, d: f64| {
        let s = v + d;
        let r = s - v;
        (v - (s - r)) + (d - r) == 0.
    };
    let binade = |v: f64| -v.signum() * 2_f64.powi(v.abs().log2().floor() as i32);
    let first = lines[0].0[0];
    let mut shifts = Vec::new();
    for d in [
        Coord {
            x: 1048576.,
            y: -524288.,
        },
        Coord {
            x: -1048576.,
            y: 524288.,
        },
        Coord {
            x: binade(first.x),
            y: binade(first.y),
        },
    ] {
        if lines
            .iter()
            .flat_map(|l| &l.0)
            .all(|p| exact(p.x, d.x) && exact(p.y, d.y))
        {
            shifts.push(d);
        }
    }
    shifts
}

#[test]
fn diverging_point_junctions_share_no_linework() {
    let shapes = fixture("diverging_junctions");
    let line = |g: &Geometry| match g {
        Geometry::MultiLineString(lines) => lines.0[0].clone(),
        Geometry::LineString(line) => line.clone(),
        _ => unreachable!(),
    };
    for pair in shapes.as_chunks::<2>().0 {
        let (a, b) = (line(&pair[0]), line(&pair[1]));
        // The junction is the first vertex of B and an original vertex of A.
        assert!(a.0.contains(&b.0[0]));
        let mut variants = vec![(a.clone(), b.clone()), (b.clone(), a.clone())];
        let mut reversed = a.clone();
        reversed.0.reverse();
        variants.push((reversed, b.clone()));
        let shifts = exact_shifts(&[&a, &b]);
        assert!(!shifts.is_empty());
        for d in shifts {
            let shift = |l: &LineString| {
                LineString(
                    l.0.iter()
                        .map(|p| Coord {
                            x: p.x + d.x,
                            y: p.y + d.y,
                        })
                        .collect(),
                )
            };
            variants.push((shift(&a), shift(&b)));
        }
        // A junction missed by one ulp is still the same point junction.
        for step in [f64::next_up, f64::next_down] {
            for axis in 0..2 {
                let mut nudged = b.clone();
                let p = &mut nudged.0[0];
                if axis == 0 {
                    p.x = step(p.x);
                } else {
                    p.y = step(p.y);
                }
                variants.push((a.clone(), nudged));
            }
        }
        for (a, b) in variants {
            for rule in [
                Rule::LineMustNotOverlap,
                Rule::LineMustNotOverlapWith,
                Rule::LineMustNotIntersect,
                Rule::LineMustNotIntersectWith,
            ] {
                let t = topology(
                    rule,
                    vec![a.clone().into(), b.clone().into()],
                    rule.inputs().1.is_some(),
                    1.,
                );
                let report = t.validate().unwrap();
                assert_eq!(report.violation_count(), 0, "{rule}: {:?}", report.rules);
                assert_eq!(
                    t.validate_into(|_, _| Ok::<_, ()>(()))
                        .unwrap()
                        .violation_count(),
                    0
                );
            }
        }
    }
}

#[test]
fn nearly_parallel_runs_from_point_junctions_remain_overlaps() {
    let slope = 2_f64.to_radians().tan();
    for (a, b, length) in [
        // Shared node, then 5 m within 0.2 m before splitting.
        (
            line(&[(-10., -10.), (0., 0.), (5., 0.2), (10., 10.)]),
            line(&[(-10., 10.), (0., 0.), (5., 0.), (10., -10.)]),
            5.,
        ),
        // Diverging at 2 degrees for 20 m.
        (
            line(&[(0., 0.), (20., 0.)]),
            line(&[(0., 0.), (20., 20. * slope)]),
            20.,
        ),
        // A short junction connector before a nearly parallel 5 m run.
        (
            line(&[(0., 0.), (5., 0.), (5., 10.)]),
            line(&[(0., 0.), (0.3, 0.3), (5., 0.3), (5., -10.)]),
            5.,
        ),
    ] {
        for rule in [Rule::LineMustNotOverlap, Rule::LineMustNotOverlapWith] {
            let t = topology(
                rule,
                vec![a.clone(), b.clone()],
                rule.inputs().1.is_some(),
                1.,
            );
            let report = t.validate().unwrap();
            assert_eq!(report.violation_count(), 1, "{rule}: {a:?}, {b:?}");
            let error = report.violations().next().unwrap();
            assert_eq!(error.kind, ViolationKind::Overlap);
            assert!(geo_length(&error.geometry) > length - 1., "{error:?}");
        }
    }
}

fn translated(geometry: &Geometry, dx: f64, dy: f64) -> Geometry {
    use topology_checker_core::geo::MapCoords;
    geometry.map_coords(|p| Coord {
        x: p.x + dx,
        y: p.y + dy,
    })
}

#[test]
fn angled_excursions_between_meeting_points_share_no_linework() {
    let a = line(&[(-5., 0.), (0., 0.), (2., 0.), (7., 0.)]);
    for (a, b) in [
        // A tent between two vertices where the lines meet.
        (a.clone(), line(&[(2., 0.), (1., 0.9), (0., 0.)])),
        (
            a,
            line(&[(2., -10.), (2., 0.), (1., 0.9), (0., 0.), (0., -10.)]),
        ),
        // A hook that crosses the other line and returns to meet it.
        (
            line(&[(-5., 0.), (0., 0.), (5., 0.)]),
            line(&[(1.5, -10.), (1.5, -0.2), (0.7, 0.4), (0., 0.)]),
        ),
    ] {
        for (dx, dy) in [(0., 0.), (-256., -512.), (1024., 2048.)] {
            let pair = vec![translated(&a, dx, dy), translated(&b, dx, dy)];
            for rule in [
                Rule::LineMustNotOverlap,
                Rule::LineMustNotOverlapWith,
                Rule::LineMustNotIntersect,
                Rule::LineMustNotIntersectWith,
            ] {
                let t = topology(rule, pair.clone(), rule.inputs().1.is_some(), 1.);
                let report = t.validate().unwrap();
                assert!(
                    report
                        .violations()
                        .all(|v| v.kind != ViolationKind::Overlap),
                    "{rule}: {:?}",
                    report.rules
                );
            }
        }
    }
}

#[test]
fn arms_and_stubs_ending_within_tolerance_keep_their_overlaps() {
    let parts: Geometry = MultiLineString(
        (0..4)
            .map(|i| {
                let x = 1.5 * i as f64;
                LineString::from(vec![(x, 0.1), (x + 1.5, 0.7)])
            })
            .collect(),
    )
    .into();
    for (a, b, length) in [
        // An approach to within half a tolerance that turns away again.
        (
            line(&[(0., 0.), (10., 0.)]),
            line(&[(0., 3.), (2., 0.99), (3.4, 0.5), (5., 3.)]),
            1.4,
        ),
        // A near duplicate turning a corner, where one line ends.
        (
            line(&[(2.103, -0.185), (3.519, -0.233), (3.976, 0.269)]),
            line(&[
                (2.302, -0.192),
                (3.471, -0.466),
                (3.82, 0.085),
                (6.82, -2.915),
            ]),
            1.4168,
        ),
        // Parts rising at about 22 degrees and ending within tolerance.
        (line(&[(0., 0.), (6., 0.)]), parts, 6.),
        // An arm of short segments ending within tolerance of the other line.
        (
            line(&[
                (-3.578, 0.215),
                (-3.545, 0.231),
                (-1.15, 0.196),
                (0.215, -0.086),
            ]),
            line(&[
                (-3.545, 0.231),
                (-2.76, 0.263),
                (-1.886, 0.294),
                (-1.634, 0.065),
            ]),
            2.3953,
        ),
    ] {
        for rule in [Rule::LineMustNotOverlap, Rule::LineMustNotOverlapWith] {
            let t = topology(
                rule,
                vec![a.clone(), b.clone()],
                rule.inputs().1.is_some(),
                1.,
            );
            let report = t.validate().unwrap();
            assert_eq!(report.violation_count(), 1, "{rule}: {a:?}, {b:?}");
            let error = report.violations().next().unwrap();
            assert_eq!(error.kind, ViolationKind::Overlap);
            assert!(
                (geo_length(&error.geometry) - length).abs() < 1e-3,
                "{error:?}"
            );
        }
    }
}

#[test]
fn crossings_and_branches_leaving_tolerance_share_no_linework() {
    let a = line(&[(-10., 0.), (0., 0.), (10., 0.)]);
    // Long enough on both sides to leave tolerance of a crossing at 5 degrees.
    let long = line(&[(-12., 0.), (0., 0.), (12., 0.)]);
    let crossing = |degrees: f64| {
        let (sin, cos) = degrees.to_radians().sin_cos();
        line(&[-12., -1.2, -0.3, 0., 0.3, 1.2, 12.].map(|d| (d * cos, d * sin)))
    };
    for (a, b, name) in [
        (long.clone(), crossing(5.), "crossing at 5 degrees"),
        (long.clone(), crossing(20.), "crossing at 20 degrees"),
        // A branch leaving a gap smaller than one sixteenth of the tolerance.
        (
            a.clone(),
            line(&[
                (0., 1e-6),
                (0.16, 0.277129),
                (1.188601, 0.895175),
                (2.925083, 10.743252),
            ]),
            "branch across a micro gap",
        ),
        (
            a.clone(),
            line(&[
                (0., 0.05),
                (0.16, 0.277129),
                (1.188601, 0.895175),
                (2.925083, 10.743252),
            ]),
            "branch across a 0.05 gap",
        ),
        // A branch turning back across the other line as it leaves tolerance.
        (
            a.clone(),
            line(&[
                (0., 0.),
                (0.16, 0.277128),
                (1.188601, 0.895174),
                (1.688601, 0.029148),
                (6.688601, -8.631106),
            ]),
            "branch hooking back",
        ),
    ] {
        for rule in [
            Rule::LineMustNotOverlap,
            Rule::LineMustNotOverlapWith,
            Rule::LineMustNotIntersect,
            Rule::LineMustNotIntersectWith,
        ] {
            let t = topology(
                rule,
                vec![a.clone(), b.clone()],
                rule.inputs().1.is_some(),
                1.,
            );
            let report = t.validate().unwrap();
            assert!(
                report
                    .violations()
                    .all(|v| v.kind != ViolationKind::Overlap),
                "{name}, {rule}: {:?}",
                report.rules
            );
        }
    }
}

#[test]
fn angled_meetings_share_no_linework_wherever_vertices_lie() {
    let o = (515067.5256, 438859.8034);
    let ray = |degrees: f64, distances: &[f64]| -> Vec<(f64, f64)> {
        let (sin, cos) = degrees.to_radians().sin_cos();
        distances
            .iter()
            .map(|d| (o.0 + d * cos, o.1 + d * sin))
            .collect()
    };
    for degrees in [3_f64, 3.5, 5., 10., 20., 30., 40., 55., 90.] {
        // A vertex inside the length over which the lines stay within tolerance.
        let edge = 1. / degrees.to_radians().sin();
        for (near_a, near_b) in [
            (vec![], vec![]),
            (vec![0.5 * edge], vec![]),
            (vec![0.95 * edge], vec![]),
            (vec![0.6 * edge], vec![0.3 * edge]),
        ] {
            let mut a = vec![0.];
            a.extend(&near_a);
            a.push(50.);
            let mut b = vec![0.];
            b.extend(&near_b);
            b.push(50.);
            let mirrored = |d: &[f64]| {
                d.iter()
                    .rev()
                    .map(|x| -x)
                    .chain(d.iter().copied().skip(1))
                    .collect::<Vec<_>>()
            };
            for (name, a, b) in [
                ("vee", ray(0., &a), ray(degrees, &b)),
                ("tee", ray(0., &mirrored(&a)), ray(degrees, &b)),
                ("cross", ray(0., &mirrored(&a)), ray(degrees, &mirrored(&b))),
            ] {
                for (dx, dy) in [(0., 0.), (-262144., -262144.)] {
                    let pair = vec![translated(&line(&a), dx, dy), translated(&line(&b), dx, dy)];
                    let t = topology(Rule::LineMustNotOverlap, pair, false, 1.);
                    assert_eq!(
                        t.validate().unwrap().violation_count(),
                        0,
                        "{name} at {degrees} degrees, vertices {near_a:?} {near_b:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn collinear_overshoots_are_overlaps_beyond_the_tolerance() {
    for offset in [0., 0.5] {
        for overshoot in [0.5, 0.9, 1., 1.1, 1.5, 2., 4.] {
            let t = topology(
                Rule::LineMustNotOverlap,
                vec![
                    line(&[(-50., 0.), (50., 0.)]),
                    line(&[(0., 50.), (0., offset), (overshoot, offset)]),
                ],
                false,
                1.,
            );
            let report = t.validate().unwrap();
            if overshoot > 1. {
                assert_eq!(report.violation_count(), 1, "{offset}, {overshoot}");
                let length = geo_length(&report.violations().next().unwrap().geometry);
                assert!((length - overshoot).abs() < 1e-9, "{length}");
            } else {
                assert_eq!(report.violation_count(), 0, "{offset}, {overshoot}");
            }
        }
    }
}

#[test]
fn short_arms_beside_other_parts_are_judged_alike_in_every_frame() {
    // The second line starts on the first part of the other feature and ends
    // within tolerance of it, at about 22 degrees. A second part passes nearby.
    let first: Geometry = MultiLineString(vec![
        LineString::from(vec![
            (278.2273285251933, 603.8939346969543),
            (278.0687644786701, 609.3166935500648),
            (277.91020043214695, 614.7394524031753),
        ]),
        LineString::from(vec![
            (278.1037316278949, 608.120846053837),
            (277.95168902830767, 608.2062260920956),
            (278.0824848649056, 608.8474677226592),
            (278.0509241665339, 609.926817425561),
        ]),
    ])
    .into();
    let second = line(&[
        (278.07031191758216, 609.263772422962),
        (277.6367427523491, 610.2530399322869),
    ]);
    for rule in [
        Rule::LineMustNotOverlap,
        Rule::LineMustNotOverlapWith,
        Rule::LineMustNotIntersect,
        Rule::LineMustNotIntersectWith,
    ] {
        let results = [(0., 0.), (-256., -512.), (-256., 0.), (-128., -256.)].map(|(dx, dy)| {
            let pair = vec![translated(&first, dx, dy), translated(&second, dx, dy)];
            let t = topology(rule, pair, rule.inputs().1.is_some(), 1.);
            t.validate()
                .unwrap()
                .violations()
                .map(|v| (v.kind, (geo_length(&v.geometry) * 1e6).round()))
                .collect::<Vec<_>>()
        });
        assert!(
            results.iter().all(|r| r == &results[0]),
            "{rule}: {results:?}"
        );
    }
}

#[test]
fn passages_of_one_part_meeting_at_a_vertex_are_judged_as_runs() {
    let path = [
        (-10., 0.),
        (0., 0.),
        (10., 0.),
        (10., 10.),
        (-3., 10.),
        (-3., 3.),
        (0., 0.),
        (0.16, 0.277),
        (1.189, 0.895),
        (1.189, 6.),
    ];
    let parts: Geometry = MultiLineString(vec![
        LineString::from(path[..7].to_vec()),
        LineString::from(path[6..].to_vec()),
    ])
    .into();
    for geometry in [line(&path), parts] {
        for (dx, dy) in [(0., 0.), (-256., -512.), (1024., 2048.)] {
            for rule in [Rule::LineMustNotSelfOverlap, Rule::LineMustNotSelfIntersect] {
                let t = topology(rule, vec![translated(&geometry, dx, dy)], false, 1.);
                assert_eq!(t.validate().unwrap().violation_count(), 0, "{rule}");
            }
        }
    }
    // A passage retracing the same part remains a self-overlap.
    let t = topology(
        Rule::LineMustNotSelfOverlap,
        vec![line(&[
            (-10., 0.),
            (0., 0.),
            (10., 0.),
            (10., 10.),
            (-3., 10.),
            (-3., 0.3),
            (8., 0.3),
        ])],
        false,
        1.,
    );
    let report = t.validate().unwrap();
    assert_eq!(report.violation_count(), 1);
    assert!(geo_length(&report.violations().next().unwrap().geometry) > 10.);
}

#[test]
fn lines_meeting_and_running_alongside_keep_their_overlaps() {
    // A line through vertices one metre apart along x, at the given heights.
    let heights = |ys: &[f64]| {
        line(
            &ys.iter()
                .enumerate()
                .map(|(i, &y)| (i as f64, y))
                .collect::<Vec<_>>(),
        )
    };
    for (a, b, length, name) in [
        (
            heights(&[
                -0.008, 0.003, -0.05, -0.046, -0.009, -0.039, 0.022, -0.026, -0.04, -0.032, -0.027,
                -0.028, 0.002, -0.004, -0.019, 0.014, -0.029, 0.041, 0.046, 0.023, -0.007,
            ]),
            heights(&[
                0.801, 0.808, 0.755, 0.792, 0.803, 0.768, 0.759, 0.83, 0.787, 0.802, 0.842, 0.811,
                0.779, 0.848, 0.787, 0.752, 0.819, 0.76, 0.781, 0.834, 0.817,
            ]),
            19.011,
            "noisy parallels",
        ),
        (
            line(&[(-50., 0.), (0., 0.), (5., 0.), (55., 0.)]),
            line(&[(-17.3205, 10.), (0., 0.4), (5., 0.05), (22.3205, 10.)]),
            5.,
            "touch, then a turn away just past the run",
        ),
        (
            line(&[(-50., 0.), (70., 0.)]),
            line(&[
                (-17.3205, -10.),
                (0., 0.05),
                (10., 0.75),
                (20., 0.05),
                (21., 0.32),
                (38.32, 10.32),
            ]),
            21.,
            "lens between a crossing and a touch",
        ),
        (
            line(&[(-50., 0.), (0., 0.), (20., 0.), (70., 0.)]),
            line(&[
                (-17.3205, -10.),
                (0., 0.2773),
                (5.894, 0.2773),
                (10.2195, 0.2773),
                (14.2551, 0.2773),
                (20., 0.2773),
                (37.3205, 10.),
            ]),
            20.,
            "crossing, then alongside",
        ),
        (
            line(&[(-50., 0.), (0., 0.), (5., 0.), (55., 0.)]),
            line(&[
                (-17.3205, 10.),
                (0., -0.5908),
                (5., -0.5958),
                (22.3205, 10.),
            ]),
            5.,
            "crossing into a step alongside",
        ),
    ] {
        for (dx, dy) in [(0., 0.), (-256., -512.)] {
            let pair = vec![translated(&a, dx, dy), translated(&b, dx, dy)];
            for rule in [Rule::LineMustNotOverlap, Rule::LineMustNotOverlapWith] {
                let t = topology(rule, pair.clone(), rule.inputs().1.is_some(), 1.);
                let report = t.validate().unwrap();
                let overlaps = report
                    .violations()
                    .filter(|v| v.kind == ViolationKind::Overlap)
                    .map(|v| geo_length(&v.geometry))
                    .collect::<Vec<_>>();
                assert!(
                    overlaps.len() == 1 && (overlaps[0] - length).abs() < 1e-3,
                    "{name}, {rule}: {overlaps:?}"
                );
            }
        }
    }
    // A hairpin of one line returning alongside itself.
    let out = [
        -0.037, 0.035, 0.026, -0.024, 0., -0.005, 0.015, 0.029, -0.041, -0.047, 0.034, -0.007,
        0.026,
    ];
    let back = [
        0.75, 0.795, 0.822, 0.773, 0.845, 0.84, 0.753, 0.753, 0.804, 0.844, 0.788, 0.772, 0.792,
    ];
    let hairpin = out
        .iter()
        .enumerate()
        .map(|(i, &y)| (i as f64, y))
        .chain(back.iter().enumerate().map(|(i, &y)| (12. - i as f64, y)))
        .collect::<Vec<_>>();
    let t = topology(
        Rule::LineMustNotSelfOverlap,
        vec![line(&hairpin)],
        false,
        1.,
    );
    let report = t.validate().unwrap();
    assert_eq!(report.violation_count(), 1);
    let length = geo_length(&report.violations().next().unwrap().geometry);
    assert!((length - 12.0116).abs() < 1e-3, "{length}");
}
#[test]
fn parallel_runs_beside_meeting_points_keep_their_overlaps() {
    // The second line meets the first, steps out to a fixed offset within
    // tolerance, runs parallel for 10, then turns away. Only the stretches near
    // the meeting and the turn are explained by them.
    let a = line(&[(-60., 0.), (0., 0.), (60., 0.)]);
    for offset in [0.6, 0.7, 0.9] {
        for b in [
            line(&[
                (0., 0.),
                (offset, offset),
                (10. + offset, offset),
                (28.0205, 10.7),
            ]),
            line(&[
                (-1., -1.732),
                (offset / 1.732, offset),
                (10. + offset, offset),
                (28.0205, 10.7),
            ]),
        ] {
            for rule in [Rule::LineMustNotOverlap, Rule::LineMustNotIntersect] {
                let t = topology(rule, vec![a.clone(), b.clone()], false, 1.);
                let report = t.validate().unwrap();
                let shared = report
                    .violations()
                    .filter(|v| v.kind == ViolationKind::Overlap)
                    .map(|v| geo_length(&v.geometry))
                    .sum::<f64>();
                assert!(shared >= 10., "{rule} offset {offset}: {report:?}");
            }
        }
    }
}

/// Each line, its two lines swapped, a reversed first line, and every exact shift.
fn meeting_variants(a: &LineString, b: &LineString) -> Vec<(LineString, LineString)> {
    let mut reversed = a.clone();
    reversed.0.reverse();
    let mut variants = vec![
        (a.clone(), b.clone()),
        (b.clone(), a.clone()),
        (reversed, b.clone()),
    ];
    for d in exact_shifts(&[a, b]) {
        let shift = |l: &LineString| {
            LineString(
                l.0.iter()
                    .map(|p| Coord {
                        x: p.x + d.x,
                        y: p.y + d.y,
                    })
                    .collect(),
            )
        };
        variants.push((shift(a), shift(b)));
    }
    variants
}

#[test]
fn exact_crossings_beside_a_line_end_share_no_linework() {
    // A road starting 0.38 m from a contour and crossing it at about 40 degrees
    // 0.62 m later, clipped 10 m and 5 m from the crossing.
    let clips = [
        (
            LineString::from(vec![
                (359121.7392, 405151.7098),
                (359121.6125, 405152.1267),
                (359117.7663737586, 405162.6364),
            ]),
            LineString::from(vec![
                (359122.63164696156, 405162.6364),
                (359121.9703, 405153.4305),
                (359118.7588, 405144.7629),
                (359118.66375890526, 405142.6364),
            ]),
        ),
        (
            LineString::from(vec![
                (359121.7392, 405151.7098),
                (359121.6125, 405152.1267),
                (359119.5961720599, 405157.6364),
            ]),
            LineString::from(vec![
                (359122.27244961984, 405157.6364),
                (359121.9703, 405153.4305),
                (359119.8234828707, 405147.6364),
            ]),
        ),
    ];
    // A branch starting within tolerance below a line and crossing it steeply
    // before leaving at 31 degrees.
    let polar = |p: (f64, f64), degrees: f64, length: f64| {
        let (sin, cos) = degrees.to_radians().sin_cos();
        (p.0 + length * cos, p.1 + length * sin)
    };
    let mut branches = Vec::new();
    for gap in [0.07, 0.3] {
        let start = (0., -gap);
        let corner = polar(start, 60., 0.32 + gap);
        let bend = polar(corner, 31., 1.2);
        branches.push((
            LineString::from(vec![(-10., 0.), (0., 0.), (10., 0.)]),
            LineString::from(vec![start, corner, bend, polar(bend, 80., 10.)]),
        ));
    }
    for (a, b) in clips.iter().chain(&branches) {
        use topology_checker_core::geo::Intersects;
        assert!(a.intersects(b));
        for (a, b) in meeting_variants(a, b) {
            for rule in [
                Rule::LineMustNotOverlap,
                Rule::LineMustNotOverlapWith,
                Rule::LineMustNotIntersect,
                Rule::LineMustNotIntersectWith,
            ] {
                let t = topology(
                    rule,
                    vec![a.clone().into(), b.clone().into()],
                    rule.inputs().1.is_some(),
                    1.,
                );
                let report = t.validate().unwrap();
                assert!(
                    report
                        .violations()
                        .all(|v| v.kind != ViolationKind::Overlap),
                    "{rule}: {a:?} {b:?} {:?}",
                    report.rules
                );
            }
        }
    }
}

#[test]
fn collinear_overshoots_between_line_ends_are_judged_by_length() {
    // A line starting 0.0145 tolerances off another, 1.0002 tolerances before the
    // other ends 0.00176 tolerances off the first.
    let a = LineString::from(vec![
        (327292.5475178135, 4307213.47902075),
        (327292.9235756431, 4307229.3518706),
        (327292.9642650116, 4307230.7513395455),
        (327293.4554896322, 4307260.908348614),
        (327293.59337847243, 4307269.4775358485),
    ]);
    let b = LineString::from(vec![
        (327290.28918682, 4307134.755223263),
        (327290.5346351322, 4307143.212023801),
        (327291.9996989209, 4307194.973257096),
        (327292.02184057154, 4307195.983032145),
        (327292.34160825214, 4307207.829077485),
        (327292.5475397348, 4307213.480020527),
    ]);
    for (a, b) in meeting_variants(&a, &b) {
        for rule in [Rule::LineMustNotOverlap, Rule::LineMustNotOverlapWith] {
            let t = topology(
                rule,
                vec![a.clone().into(), b.clone().into()],
                rule.inputs().1.is_some(),
                0.001,
            );
            let report = t.validate().unwrap();
            let overlaps = report
                .violations()
                .filter(|v| v.kind == ViolationKind::Overlap)
                .map(|v| geo_length(&v.geometry))
                .collect::<Vec<_>>();
            assert!(
                overlaps.len() == 1 && (overlaps[0] - 0.0010001).abs() < 1e-7,
                "{rule}: {overlaps:?}"
            );
        }
    }
    // Overshoots by s tolerances, with the overshooting end on the other line,
    // a rounding error off it, or clearly off it, at small angles between them.
    for (tolerance, origin) in [
        (1., (0., 0.)),
        (0.001, (327292.5475178135, 4307213.47902075)),
    ] {
        let (sin, cos) = 17_f64.to_radians().sin_cos();
        let place = |p: (f64, f64)| {
            let (x, y) = (p.0 * tolerance, p.1 * tolerance);
            (origin.0 + x * cos - y * sin, origin.1 + x * sin + y * cos)
        };
        for s in [0.99999, 1.00001, 1.0001, 1.0002, 1.001, 1.01, 1.05, 1.1] {
            for offset in [0., 1e-6, 0.07, 0.2] {
                for degrees in [0_f64, 0.5, 1., 2., 2.9] {
                    let (dy, dx) = degrees.to_radians().sin_cos();
                    let a = line(&[place((0., 0.)), place((50., 0.))]);
                    let b = line(&[place((s - 50. * dx, offset - 50. * dy)), place((s, offset))]);
                    let t = topology(Rule::LineMustNotOverlapWith, vec![a, b], true, tolerance);
                    let lengths = t
                        .validate()
                        .unwrap()
                        .violations()
                        .map(|v| geo_length(&v.geometry) / tolerance)
                        .collect::<Vec<_>>();
                    let name = format!("{tolerance} {s} {offset} {degrees}");
                    if degrees == 0. {
                        // Exactly collinear: reported exactly when longer than the
                        // tolerance, wherever the overshooting end lies.
                        if s > 1. {
                            assert!(
                                lengths.len() == 1 && (lengths[0] - s).abs() < 1e-4,
                                "{name}: {lengths:?}"
                            );
                        } else {
                            assert!(lengths.is_empty(), "{name}: {lengths:?}");
                        }
                    } else if s >= 1.01 {
                        assert!(
                            lengths.len() == 1 && (lengths[0] - s).abs() < 0.02,
                            "{name}: {lengths:?}"
                        );
                    } else if s < 1. && offset <= 1e-6 {
                        assert!(lengths.is_empty(), "{name}: {lengths:?}");
                    }
                }
            }
        }
    }
}
#[test]
fn arms_folding_back_alongside_after_a_crossing_keep_their_overlaps() {
    // The second line crosses at 20 degrees, turns back and runs parallel 0.6
    // from the first for 3 before ending. That parallel stretch is shared.
    let a = line(&[(-20., 0.), (20., 0.)]);
    let b = line(&[(-27.4748, -10.), (1.6485, 0.6), (-1.3515, 0.6)]);
    for rule in [Rule::LineMustNotOverlap, Rule::LineMustNotIntersect] {
        let t = topology(rule, vec![a.clone(), b.clone()], false, 1.);
        let report = t.validate().unwrap();
        let shared = report
            .violations()
            .filter(|v| v.kind == ViolationKind::Overlap)
            .map(|v| geo_length(&v.geometry))
            .sum::<f64>();
        assert!((shared - 3.).abs() < 1e-9, "{rule}: {report:?}");
    }
}

/// A line arriving at `degrees` to the x axis and ending `offset` above it at
/// x = 0, `length` long, with vertices at the given distances before its end.
fn arm(degrees: f64, offset: f64, length: f64, vertices: &[f64]) -> Geometry {
    let (sin, cos) = degrees.to_radians().sin_cos();
    let mut distances = vertices
        .iter()
        .copied()
        .filter(|&s| s > 0. && s < length)
        .collect::<Vec<_>>();
    distances.sort_by(|a, b| b.total_cmp(a));
    let coords = std::iter::once(length)
        .chain(distances)
        .chain([0.])
        .map(|s| (-s * cos, offset + s * sin))
        .collect::<Vec<_>>();
    line(&coords)
}

fn overlaps(rule: Rule, geometries: Vec<Geometry>, tolerance: f64) -> Vec<f64> {
    let t = topology(rule, geometries, rule.inputs().1.is_some(), tolerance);
    t.validate()
        .unwrap()
        .violations()
        .filter(|v| v.kind == ViolationKind::Overlap)
        .map(|v| geo_length(&v.geometry))
        .collect()
}

#[test]
fn branches_ending_within_tolerance_share_no_linework_however_segmented() {
    // Within tolerance, coordinates are coincident: a line ending within
    // tolerance of another meets it there, and a branch arriving at an angle
    // shares no linework with it, wherever its vertices lie.
    let other = line(&[(-30., 0.), (30., 0.)]);
    let mut seed = 0x2545_f491_4f6c_dd1d_u64;
    for degrees in [3_f64, 5., 10., 20., 29., 45.] {
        for offset in [0., 0.03, 0.2, 0.5, 0.9, 1.] {
            for length in [1., 1.5, 2., 3., 5., 8., 40.] {
                // Arms lying wholly within tolerance are stubs, not branches.
                if length * degrees.to_radians().sin() + offset <= 1. {
                    continue;
                }
                let mut random = || {
                    seed ^= seed << 13;
                    seed ^= seed >> 7;
                    seed ^= seed << 17;
                    (0..1 + seed % 4)
                        .map(|k| ((seed >> (8 * k)) % 1024) as f64 / 1024. * length)
                        .collect::<Vec<_>>()
                };
                let vertices = [
                    vec![],
                    (1..160).map(|k| k as f64 / 4.).collect(),
                    (1..40).map(f64::from).collect(),
                    vec![1.2],
                    vec![0.5],
                    vec![0.1],
                    vec![length / 2.],
                    vec![0.9 * length],
                    random(),
                    random(),
                ];
                for vertices in &vertices {
                    let branch = arm(degrees, offset, length, vertices);
                    for (dx, dy) in [(0., 0.), (-256., -512.)] {
                        let pair = vec![translated(&branch, dx, dy), translated(&other, dx, dy)];
                        for rule in [
                            Rule::LineMustNotOverlap,
                            Rule::LineMustNotOverlapWith,
                            Rule::LineMustNotIntersect,
                        ] {
                            let found = overlaps(rule, pair.clone(), 1.);
                            assert!(
                                found.is_empty(),
                                "{rule} at {degrees} degrees, {offset} short, {length} long, \
                                 vertices {vertices:?}, shifted {dx} {dy}: {found:?}"
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn a_branch_ending_short_of_a_line_is_judged_alike_with_a_vertex_near_its_end() {
    // Arriving at 29 degrees and ending 0.2 short of the other line, with and
    // without a vertex 1.2 before its end.
    let other = LineString::from(vec![(-30., 0.), (30., 0.)]);
    for branch in [
        LineString::from(vec![(-34.985, 19.592), (-1.0495, 0.7818), (0., 0.2)]),
        LineString::from(vec![(-34.985, 19.592), (0., 0.2)]),
    ] {
        for (a, b) in meeting_variants(&branch, &other) {
            for rule in [
                Rule::LineMustNotOverlap,
                Rule::LineMustNotOverlapWith,
                Rule::LineMustNotIntersect,
                Rule::LineMustNotIntersectWith,
            ] {
                let found = overlaps(rule, vec![a.clone().into(), b.clone().into()], 1.);
                assert!(found.is_empty(), "{rule}: {a:?} {b:?} {found:?}");
            }
        }
    }
}

#[test]
fn lines_converging_to_ends_within_tolerance_share_linework_only_when_running_together() {
    // IGN Argentina underground ducts (ln_ign_ar_ductos, EPSG:8857, CC BY 4.0),
    // features 140 and 141: converging at about 6 degrees, they come within 1 m
    // about 2 m before 140 ends 0.81 m from 141, which ends 0.95 m from that end.
    let a = LineString::from(vec![
        (-5834686.0170614645, -4800765.125013224),
        (-5834687.872245371, -4800765.376502841),
        (-5834689.501543387, -4800765.546445226),
    ]);
    let b = LineString::from(vec![
        (-5834688.922602736, -4800761.618514752),
        (-5834687.884960355, -4800764.384984638),
        (-5834690.149162575, -4800764.852044867),
    ]);
    // A vee whose arms start 0.5 apart and part at 10 degrees.
    let (sin, cos) = 10_f64.to_radians().sin_cos();
    let vee = (
        LineString::from(vec![(0., 0.), (20., 0.)]),
        LineString::from(vec![(0., 0.5), (20. * cos, 0.5 + 20. * sin)]),
    );
    for (a, b) in [(a, b), vee] {
        for (a, b) in meeting_variants(&a, &b) {
            for rule in [
                Rule::LineMustNotOverlap,
                Rule::LineMustNotOverlapWith,
                Rule::LineMustNotIntersect,
            ] {
                let found = overlaps(rule, vec![a.clone().into(), b.clone().into()], 1.);
                assert!(found.is_empty(), "{rule}: {a:?} {b:?} {found:?}");
            }
        }
    }
    // Arms approaching a line at under 3 degrees run together with it beyond the
    // near zone of where they would have met it. Wherever the arm touching the
    // line keeps its overlap, so does every arm ending short of it.
    let other = line(&[(-120., 0.), (30., 0.)]);
    for degrees in [0.5, 1., 1.5, 2., 2.5] {
        for step in [0.25, 1.] {
            let vertices = (1..400).map(|k| k as f64 * step).collect::<Vec<_>>();
            for rule in [Rule::LineMustNotOverlap, Rule::LineMustNotOverlapWith] {
                let touching = overlaps(
                    rule,
                    vec![arm(degrees, 0., 100., &vertices), other.clone()],
                    1.,
                );
                if touching.is_empty() {
                    continue;
                }
                for offset in [0.1, 0.2, 0.3, 0.5, 0.7, 0.8, 0.9] {
                    let branch = arm(degrees, offset, 100., &vertices);
                    let found = overlaps(rule, vec![branch, other.clone()], 1.);
                    assert!(
                        found.len() == 1,
                        "{rule} at {degrees} degrees, {offset} short, vertices every {step}: \
                         {found:?}, touching {touching:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn nearly_parallel_runs_from_line_ends_within_tolerance_remain_overlaps() {
    for (a, b, length) in [
        // Starting 0.77 off and leaving at 1.5 degrees, then turning away and
        // returning alongside 0.5 off.
        (
            vec![(-40., 0.), (30., 0.)],
            vec![
                (0., 0.77),
                (-8.5, 1.),
                (-11., 3.),
                (-14., 0.5),
                (-20., 0.5),
                (-20., 6.),
            ],
            14.5,
        ),
        // Starting 0.5 off and drifting to 0.8 off over 18, then turning away.
        (
            vec![(0., 0.), (30., 0.)],
            vec![(0., 0.5), (18., 0.8), (22.33, 3.3)],
            18.,
        ),
        // Starting beyond the other line's end 0.69 off and drifting to 0.91 off
        // over 13, then turning away.
        (
            vec![(0., 0.), (-20., 0.)],
            vec![(5., 0.69), (-8., 0.91), (-13., 5.65)],
            8.,
        ),
        // Approaching at 2 degrees and ending 0.3 off, with vertices closer near
        // the end.
        (
            vec![(-80., 0.), (30., 0.)],
            vec![
                (-59.96345, 2.39397),
                (-39.975633, 1.69598),
                (-29.981725, 1.346985),
                (-19.987817, 0.99799),
                (-9.993908, 0.648995),
                (-4.996954, 0.474497),
                (-1.998782, 0.369799),
                (-0.999391, 0.334899),
                (0., 0.3),
            ],
            19.988,
        ),
    ] {
        let (a, b) = (LineString::from(a), LineString::from(b));
        for (a, b) in meeting_variants(&a, &b) {
            for rule in [
                Rule::LineMustNotOverlap,
                Rule::LineMustNotOverlapWith,
                Rule::LineMustNotIntersect,
                Rule::LineMustNotIntersectWith,
            ] {
                let found = overlaps(rule, vec![a.clone().into(), b.clone().into()], 1.);
                assert!(
                    found.len() == 1 && (found[0] - length).abs() < 1e-3,
                    "{rule}: {a:?} {b:?} {found:?}"
                );
            }
        }
    }
}

#[test]
fn lines_meeting_at_a_vertex_just_off_the_other_share_no_linework() {
    // The first line meets the second at a vertex 0.03 off it, arriving at about
    // 31 degrees and leaving at 10 degrees for 1.2 before ending.
    let a = LineString::from(vec![
        (5.6271801590919495, 2.0371830463409424),
        (-0.0056688785552978516, -0.02945953607559204),
        (-0.3518734574317932, 0.10268974304199219),
        (-1.2145591974258423, 0.4319842457771301),
    ]);
    let b = LineString::from(vec![
        (-29.459525227546692, 5.668895363807678),
        (-4.5737815499305725, 0.8801326155662537),
        (-1.9813232421875, 0.38126593828201294),
        (0.6447558999061584, -0.12407034635543823),
        (29.459525227546692, -5.668895363807678),
    ]);
    for (a, b) in meeting_variants(&a, &b) {
        for rule in [
            Rule::LineMustNotOverlap,
            Rule::LineMustNotOverlapWith,
            Rule::LineMustNotIntersect,
            Rule::LineMustNotIntersectWith,
        ] {
            let found = overlaps(rule, vec![a.clone().into(), b.clone().into()], 1.);
            assert!(found.is_empty(), "{rule}: {a:?} {b:?} {found:?}");
        }
    }
}

#[test]
fn parts_meeting_within_tolerance_are_judged_as_if_they_touched() {
    let self_overlaps = |geometry: &Geometry, dx: f64, dy: f64| {
        let t = topology(
            Rule::LineMustNotSelfOverlap,
            vec![translated(geometry, dx, dy)],
            false,
            1.,
        );
        t.validate()
            .unwrap()
            .violations()
            .map(|v| geo_length(&v.geometry))
            .collect::<Vec<_>>()
    };
    let parts = |coords: &[&[(f64, f64)]]| -> Geometry {
        MultiLineString(
            coords
                .iter()
                .map(|c| LineString::from(c.to_vec()))
                .collect(),
        )
        .into()
    };
    for (dx, dy) in [(0., 0.), (-256., -512.)] {
        // A part leaving another's end at an angle, starting within tolerance of
        // it, shares nothing with it, as when it starts at that end, or as one
        // line bending there.
        for degrees in [10_f64, 20., 45.] {
            let (sin, cos) = degrees.to_radians().sin_cos();
            let (mid, far) = ((5. * cos, 5. * sin), (10. * cos, 10. * sin));
            let mut geometries = vec![line(&[(10., 0.), (0., 0.), far])];
            for start in [(0.3, 0.2), (-0.4, 0.3), (0., 0.)] {
                geometries.push(parts(&[&[(10., 0.), (0., 0.)], &[start, far]]));
                geometries.push(parts(&[
                    &[(10., 0.), (5., 0.), (0., 0.)],
                    &[start, mid, far],
                ]));
            }
            for geometry in &geometries {
                let found = self_overlaps(geometry, dx, dy);
                assert!(
                    found.is_empty(),
                    "{degrees} degrees, {geometry:?}: {found:?}"
                );
            }
        }
        // A part folding back along another from near its end, or running along
        // it from near its start, keeps that shared stretch.
        for start in [(10.2, 0.4), (9.7, 0.4), (10., 0.)] {
            let geometry = parts(&[
                &[(0., 0.), (10., 0.)],
                &[start, (6., 0.4), (2., 0.4), (2., 10.)],
            ]);
            let found = self_overlaps(&geometry, dx, dy);
            assert!(
                found.len() == 1 && (found[0] - 8.).abs() < 1e-3,
                "{geometry:?}: {found:?}"
            );
        }
        for start in [(0.2, 0.4), (-0.3, 0.4)] {
            let geometry = parts(&[&[(0., 0.), (10., 0.)], &[start, (8., 0.4), (8., 10.)]]);
            let found = self_overlaps(&geometry, dx, dy);
            assert!(
                found.len() == 1 && found[0] > 7.5,
                "{geometry:?}: {found:?}"
            );
        }
    }
}

#[test]
fn stretches_within_tolerance_between_line_ends_and_meetings_remain_overlaps() {
    let cases = [
        // Natural and Land Survey of Iceland watercourses (ln_iceland_watercourse,
        // EPSG:3057, CC BY 4.0), features 168500 and 168505: a 2.4 m line lying
        // within 1 m of another and ending on it.
        (
            vec![
                (439513.1157645894, 472039.82003628544),
                (439513.0863, 472040.3112),
                (439513.3115, 472041.9254),
                (439513.6118, 472043.0892),
                (439514.0623, 472044.0652),
            ],
            vec![(439512.4014, 472042.3137), (439514.0623, 472044.0652)],
            1.3956,
        ),
        // OpenStreetMap roads (HOT export, ln_hotosm_rwanda_roads, EPSG:32735,
        // ODbL), features 6919 and 39914: 39914 starts within 0.28 m of 6919 and
        // reaches it 1.38 m later at a shared vertex.
        (
            vec![
                (875280.3662555307, 9848796.192978948),
                (875277.2748966113, 9848799.700723652),
                (875275.0775320219, 9848802.192978948),
            ],
            vec![
                (875277.9641303706, 9848798.503940983),
                (875277.2748966113, 9848799.700723652),
                (875279.3615717051, 9848802.192978948),
            ],
            1.3536,
        ),
        // OpenStreetMap roads (HOT export, ln_hotosm_bhutan_roads, EPSG:32646,
        // ODbL), features 19241 and 24085: 19241 starts within 0.65 m of 24085 and
        // reaches it 1.91 m later at a shared vertex.
        (
            vec![
                (351214.97485596675, 3053337.0000013374),
                (351213.92813964153, 3053338.597304434),
                (351213.5164375729, 3053340.7987),
            ],
            vec![
                (351211.4515, 3053339.394075475),
                (351213.92813964153, 3053338.597304434),
                (351214.84501361643, 3053334.7987),
            ],
            1.7983,
        ),
        // OpenStreetMap waterways (HOT export, ln_hotosm_haiti_waterways,
        // EPSG:32618, ODbL), features 8002 and 8003: 8002 leaves the end of 8003
        // along it at 3.6 degrees for 1.09 m, then bends away.
        (
            vec![
                (841720.0015203024, 2016666.532589315),
                (841722.5461576518, 2016668.1967764425),
                (841723.4568829531, 2016668.8001649126),
                (841722.9996172354, 2016671.4984706775),
            ],
            vec![
                (841726.0015203024, 2016670.8141139287),
                (841722.5461576518, 2016668.1967764425),
            ],
            1.0925,
        ),
    ];
    for (a, b, length) in cases {
        let (a, b) = (LineString::from(a), LineString::from(b));
        for (a, b) in meeting_variants(&a, &b) {
            for rule in [
                Rule::LineMustNotOverlap,
                Rule::LineMustNotOverlapWith,
                Rule::LineMustNotIntersect,
                Rule::LineMustNotIntersectWith,
            ] {
                let found = overlaps(rule, vec![a.clone().into(), b.clone().into()], 1.);
                assert!(
                    found.len() == 1 && (found[0] - length).abs() < 1e-3,
                    "{rule}: {a:?} {b:?} {found:?}"
                );
            }
        }
    }
}

#[test]
fn parts_continuing_from_an_end_within_tolerance_keep_the_line_unbroken() {
    // An approach to within half a tolerance that turns away again shares that
    // stretch, whether the line is one part or two joined at the turn, end to end
    // or a fraction of the tolerance apart. Alone, the approach is a branch
    // ending within tolerance and shares nothing.
    let other = line(&[(0., 0.), (10., 0.)]);
    let approach = LineString::from(vec![(0., 3.), (2., 0.99), (3.4, 0.5)]);
    let lines = [
        line(&[(0., 3.), (2., 0.99), (3.4, 0.5), (5., 3.)]),
        MultiLineString(vec![
            approach.clone(),
            LineString::from(vec![(3.4, 0.5), (5., 3.)]),
        ])
        .into(),
        MultiLineString(vec![
            approach.clone(),
            LineString::from(vec![(3.5, 0.6), (5., 3.)]),
        ])
        .into(),
    ];
    for (dx, dy) in [(0., 0.), (-256., -512.)] {
        for rule in [Rule::LineMustNotOverlap, Rule::LineMustNotOverlapWith] {
            for line in &lines {
                let pair = vec![translated(line, dx, dy), translated(&other, dx, dy)];
                let found = overlaps(rule, pair, 1.);
                assert!(
                    found.len() == 1 && (found[0] - 1.4).abs() < 1e-3,
                    "{rule}: {line:?} {found:?}"
                );
            }
            let pair = vec![
                translated(&approach.clone().into(), dx, dy),
                translated(&other, dx, dy),
            ];
            assert_eq!(overlaps(rule, pair, 1.), Vec::<f64>::new(), "{rule}");
        }
    }
}
