use topology_checker_core::{
    Layer, Rule, Topology, Violation, ViolationKind,
    geo::{
        Geometry, GeometryCollection, LineString, MultiLineString, MultiPoint, MultiPolygon, Point,
        Polygon, Rect, coord,
    },
};
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
