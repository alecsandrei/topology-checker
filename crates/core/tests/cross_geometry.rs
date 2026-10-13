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
