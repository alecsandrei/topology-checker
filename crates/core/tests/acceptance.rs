use topology_checker_core::{
    Error, ErrorType, Layer, Rule, Topology,
    geo::{Geometry, MultiPoint, MultiPolygon, Point, polygon},
};

fn run(
    rule: Rule,
    input: Vec<Geometry>,
    other: Option<Vec<Geometry>>,
    tolerance: f64,
) -> topology_checker_core::Report {
    let mut t = Topology::new(tolerance).unwrap();
    let a = t.add_layer(Layer::from_geometries("input", input));
    let b = other.map(|g| t.add_layer(Layer::from_geometries("other", g)));
    t.add_rule(rule, a, b).unwrap();
    t.validate().unwrap()
}
#[test]
fn three_coincident_points_produce_three_feature_pairs() {
    let r = run(
        Rule::PointMustNotOverlap,
        vec![Point::new(0., 0.).into(); 3],
        None,
        0.,
    );
    assert_eq!(r.error_count(), 3);
    assert!(r.errors().all(|e| e.origin != e.destination));
}
#[test]
fn overlap_aggregates_multipart_features() {
    let g = Geometry::MultiPoint(MultiPoint(vec![Point::new(0., 0.), Point::new(1., 0.)]));
    let r = run(Rule::PointMustNotOverlap, vec![g.clone(), g], None, 0.);
    assert_eq!(r.error_count(), 1);
    assert_eq!(r.errors().next().unwrap().origin.unwrap().part, None);
}
#[test]
fn single_part_multi_polygon_is_valid() {
    let p = polygon![(x:0.,y:0.),(x:1.,y:0.),(x:1.,y:1.),(x:0.,y:1.)];
    let r = run(
        Rule::GeometryMustNotBeMultipart,
        vec![MultiPolygon(vec![p]).into()],
        None,
        0.,
    );
    assert!(r.is_valid());
}
#[test]
fn point_boundary_is_outside() {
    let p = polygon![(x:0.,y:0.),(x:1.,y:0.),(x:1.,y:1.),(x:0.,y:1.)];
    let r = run(
        Rule::PointMustBeInside,
        vec![Point::new(0., 0.5).into(), Point::new(0.5, 0.5).into()],
        Some(vec![p.into()]),
        0.,
    );
    assert_eq!(r.error_count(), 1);
    assert_eq!(r.errors().next().unwrap().error_type, ErrorType::Outside);
}
#[test]
fn invalid_inputs_are_typed_errors() {
    assert!(matches!(Topology::new(f64::NAN), Err(Error::Tolerance(_))));
    let mut t = Topology::new(0.).unwrap();
    let a = t.add_layer(Layer::from_features(
        "bad",
        [(42, Point::new(f64::INFINITY, 0.).into())],
    ));
    assert!(matches!(
        t.add_rule(Rule::PointMustNotOverlap, a, None),
        Err(Error::NonFinite { fid: 42, .. })
    ));
    let a = t.add_layer(Layer::from_geometries("wrong", [Point::new(0., 0.).into()]));
    assert!(matches!(
        t.add_rule(Rule::LineMustNotOverlap, a, None),
        Err(Error::GeometryKind { fid: 0, .. })
    ));
}

fn square(x: f64, y: f64, size: f64) -> Geometry {
    polygon![(x:x,y:y),(x:x+size,y:y),(x:x+size,y:y+size),(x:x,y:y+size)].into()
}
#[test]
fn polygon_overlaps_report_each_feature_pair_once() {
    let r = run(
        Rule::PolygonMustNotOverlap,
        vec![square(0., 0., 2.), square(1., 0., 2.), square(5., 0., 1.)],
        None,
        0.,
    );
    assert_eq!(r.error_count(), 1);
    use topology_checker_core::geo::Area;
    let Geometry::Polygon(p) = &r.errors().next().unwrap().geometry else {
        panic!("expected polygon");
    };
    assert_eq!(p.unsigned_area(), 2.);
}
#[test]
fn adjacent_polygons_with_unmatched_vertices_have_no_gap() {
    let a = polygon![(x:0.,y:0.),(x:1.,y:0.),(x:1.,y:1.),(x:0.,y:1.)];
    let b = polygon![(x:1.,y:0.),(x:2.,y:0.),(x:2.,y:1.),(x:1.,y:1.),(x:1.,y:0.5)];
    let r = run(
        Rule::PolygonMustNotHaveGaps,
        vec![a.into(), b.into()],
        None,
        0.,
    );
    assert_eq!(r.error_count(), 1);
    assert_eq!(
        r.errors().next().unwrap().error_type,
        ErrorType::OuterBoundary
    );
}
#[test]
fn gap_holes_and_outer_boundaries_are_distinct() {
    let p = polygon!(exterior: [(x:0.,y:0.),(x:5.,y:0.),(x:5.,y:5.),(x:0.,y:5.)],interiors:[[(x:1.,y:1.),(x:2.,y:1.),(x:2.,y:2.),(x:1.,y:2.)]]);
    let r = run(Rule::PolygonMustNotHaveGaps, vec![p.into()], None, 0.);
    assert_eq!(r.error_count(), 2);
    assert_eq!(
        r.errors()
            .filter(|e| e.error_type == ErrorType::Gap)
            .count(),
        1
    );
}
#[test]
fn polygon_slivers_are_filtered_but_outer_boundaries_remain() {
    let r = run(
        Rule::PolygonMustNotOverlap,
        vec![square(0., 0., 1.), square(0.9999, 0., 1.)],
        None,
        0.001,
    );
    assert!(r.is_valid());
    let p = polygon!(exterior:[(x:0.,y:0.),(x:2.,y:0.),(x:2.,y:2.),(x:0.,y:2.)],interiors:[[(x:1.,y:1.),(x:1.0001,y:1.),(x:1.0001,y:1.5),(x:1.,y:1.5)]]);
    let r = run(Rule::PolygonMustNotHaveGaps, vec![p.into()], None, 0.001);
    assert_eq!(r.error_count(), 1);
}
#[test]
fn line_inside_uses_union_and_accepts_boundaries() {
    use topology_checker_core::geo::line_string;
    let lines = vec![
        line_string![(x:0.25,y:0.5),(x:1.75,y:0.5)].into(),
        line_string![(x:0.,y:0.),(x:2.,y:0.)].into(),
    ];
    let r = run(
        Rule::LineMustBeInside,
        lines,
        Some(vec![square(0., 0., 1.), square(1., 0., 1.)]),
        0.,
    );
    assert!(r.is_valid(), "{:?}", r.errors().collect::<Vec<_>>());
}
#[test]
fn line_outside_error_is_only_the_uncovered_piece() {
    use topology_checker_core::geo::{Euclidean, Length, line_string};
    let r = run(
        Rule::LineMustBeInside,
        vec![line_string![(x:0.5,y:0.5),(x:2.,y:0.5)].into()],
        Some(vec![square(0., 0., 1.)]),
        0.,
    );
    assert_eq!(r.error_count(), 1);
    let Geometry::LineString(l) = &r.errors().next().unwrap().geometry else {
        panic!("line error");
    };
    assert!((Euclidean.length(l) - 1.).abs() < 1e-8);
}
#[test]
fn opposite_winding_polygons_union_correctly() {
    use topology_checker_core::geo::{
        line_string,
        orient::{Direction, Orient},
    };
    let Geometry::Polygon(a) = square(0., 0., 1.) else {
        unreachable!()
    };
    let Geometry::Polygon(b) = square(1., 0., 1.) else {
        unreachable!()
    };
    let r = run(
        Rule::LineMustBeInside,
        vec![line_string![(x:0.2,y:0.5),(x:1.8,y:0.5)].into()],
        Some(vec![
            a.orient(Direction::Default).into(),
            b.orient(Direction::Reversed).into(),
        ]),
        0.,
    );
    assert!(r.is_valid());
}

#[test]
fn identical_and_reversed_lines_overlap_once() {
    use geo::{Euclidean, Length, line_string};
    let line = line_string![(x:0.,y:0.),(x:2.,y:0.)];
    for other in [line.clone(), line_string![(x:2.,y:0.),(x:0.,y:0.)]] {
        let r = run(
            Rule::LineMustNotOverlap,
            vec![line.clone().into(), other.into()],
            None,
            0.,
        );
        assert_eq!(r.error_count(), 1);
        let Geometry::LineString(l) = &r.errors().next().unwrap().geometry else {
            panic!("line");
        };
        assert_eq!(Euclidean.length(l), 2.);
    }
}
#[test]
fn partial_line_overlap_reports_only_shared_piece() {
    use geo::line_string;
    let a = line_string![(x:0.,y:0.),(x:2.,y:0.)];
    let b = line_string![(x:1.,y:0.),(x:3.,y:0.)];
    let r = run(
        Rule::LineMustNotOverlapWith,
        vec![a.into()],
        Some(vec![b.into()]),
        0.,
    );
    assert_eq!(r.error_count(), 1);
    assert_eq!(
        r.errors().next().unwrap().geometry,
        Geometry::LineString(line_string![(x:1.,y:0.),(x:2.,y:0.)])
    );
}
#[test]
fn partial_self_overlap_is_separate_from_other_feature_rules() {
    use geo::line_string;
    let line =
        line_string![(x:0.,y:0.),(x:2.,y:0.),(x:2.,y:1.),(x:1.,y:1.),(x:1.,y:0.),(x:3.,y:0.)];
    let r = run(
        Rule::LineMustNotSelfOverlap,
        vec![line.clone().into()],
        None,
        0.,
    );
    assert_eq!(r.error_count(), 1);
    assert_eq!(
        r.errors().next().unwrap().geometry,
        Geometry::LineString(line_string![(x:1.,y:0.),(x:2.,y:0.)])
    );
    for rule in [Rule::LineMustNotOverlap, Rule::LineMustNotIntersect] {
        assert!(run(rule, vec![line.clone().into()], None, 0.).is_valid());
    }
}
#[test]
fn tolerance_connects_endpoints_and_cracks_t_junctions() {
    use geo::line_string;
    let lines = vec![
        line_string![(x:0.,y:0.),(x:1.,y:0.)].into(),
        line_string![(x:1.000000001,y:0.),(x:2.,y:0.)].into(),
    ];
    assert_eq!(
        run(Rule::LineMustNotHaveDangles, lines.clone(), None, 0.).error_count(),
        4
    );
    assert_eq!(
        run(Rule::LineMustNotHaveDangles, lines, None, 0.001).error_count(),
        2
    );
    let lines = vec![
        line_string![(x:0.,y:0.),(x:2.,y:0.)].into(),
        line_string![(x:1.,y:0.0001),(x:1.,y:1.)].into(),
    ];
    assert_eq!(
        run(Rule::LineMustNotHaveDangles, lines, None, 0.001).error_count(),
        3
    );
}
#[test]
fn self_crossings_closed_loops_and_self_touches_have_distinct_semantics() {
    use geo::line_string;
    let crossing = line_string![(x:0.,y:0.),(x:2.,y:2.),(x:0.,y:2.),(x:2.,y:0.)];
    let r = run(
        Rule::LineMustNotSelfIntersect,
        vec![crossing.into()],
        None,
        0.,
    );
    assert_eq!(r.error_count(), 1);
    assert_eq!(
        r.errors().next().unwrap().error_type,
        ErrorType::SelfCrossing
    );
    let closed = line_string![(x:0.,y:0.),(x:1.,y:0.),(x:1.,y:1.),(x:0.,y:1.),(x:0.,y:0.)];
    for rule in [
        Rule::LineMustNotSelfIntersect,
        Rule::LineMustNotHavePseudoNodes,
        Rule::LineMustNotHaveDangles,
    ] {
        assert!(run(rule, vec![closed.clone().into()], None, 0.).is_valid());
    }
    let vertex_crossing = line_string![(x:0.,y:0.),(x:-1.,y:1.),(x:-2.,y:0.),(x:-1.,y:-1.),(x:0.,y:0.),(x:1.,y:1.),(x:2.,y:0.),(x:1.,y:-1.),(x:0.,y:0.)];
    assert_eq!(
        run(
            Rule::LineMustNotSelfIntersect,
            vec![vertex_crossing.into()],
            None,
            0.
        )
        .error_count(),
        1
    );
    let touch = line_string![(x:0.,y:0.),(x:-1.,y:1.),(x:-2.,y:0.),(x:-1.,y:-1.),(x:0.,y:0.),(x:1.,y:-1.),(x:2.,y:0.),(x:1.,y:1.),(x:0.,y:0.)];
    assert!(run(Rule::LineMustNotSelfIntersect, vec![touch.into()], None, 0.).is_valid());
}
#[test]
fn crossing_and_endpoint_interior_touch_are_separate() {
    use geo::line_string;
    let a = line_string![(x:0.,y:0.),(x:2.,y:0.)];
    let touch = line_string![(x:1.,y:0.),(x:1.,y:1.)];
    assert!(
        run(
            Rule::LineMustNotIntersect,
            vec![a.clone().into(), touch.clone().into()],
            None,
            0.
        )
        .is_valid()
    );
    let r = run(
        Rule::LineMustNotIntersectOrTouchInterior,
        vec![a.clone().into(), touch.into()],
        None,
        0.,
    );
    assert_eq!(r.error_count(), 1);
    assert_eq!(
        r.errors().next().unwrap().error_type,
        ErrorType::TouchInterior
    );
    let cross = line_string![(x:1.,y:-1.),(x:1.,y:1.)];
    let r = run(
        Rule::LineMustNotIntersectWith,
        vec![a.into()],
        Some(vec![cross.into()]),
        0.,
    );
    assert_eq!(r.error_count(), 1);
    assert_eq!(r.errors().next().unwrap().error_type, ErrorType::Crossing);
}
#[test]
fn pseudo_nodes_require_distinct_features() {
    use geo::{MultiLineString, line_string};
    let a = line_string![(x:0.,y:0.),(x:1.,y:0.)];
    let b = line_string![(x:1.,y:0.),(x:2.,y:0.)];
    assert_eq!(
        run(
            Rule::LineMustNotHavePseudoNodes,
            vec![a.clone().into(), b.clone().into()],
            None,
            0.
        )
        .error_count(),
        1
    );
    assert!(
        run(
            Rule::LineMustNotHavePseudoNodes,
            vec![MultiLineString(vec![a, b]).into()],
            None,
            0.
        )
        .is_valid()
    );
}
#[test]
fn unrelated_layers_do_not_change_single_layer_graph_results() {
    use geo::line_string;
    let mut t = Topology::new(0.).unwrap();
    let a = t.add_layer(Layer::from_geometries(
        "a",
        [line_string![(x:0.,y:0.),(x:1.,y:0.)].into()],
    ));
    let b = t.add_layer(Layer::from_geometries(
        "b",
        [line_string![(x:0.,y:0.),(x:0.,y:1.)].into()],
    ));
    t.add_rule(Rule::LineMustNotHaveDangles, a, None).unwrap();
    t.add_rule(Rule::LineMustNotIntersectWith, a, Some(b))
        .unwrap();
    let r = t.validate().unwrap();
    assert_eq!(r.rules[0].errors.len(), 2);
    assert_eq!(
        serde_free_signature(&r),
        serde_free_signature(&t.validate().unwrap())
    );
}
fn serde_free_signature(r: &topology_checker_core::Report) -> String {
    format!("{r:?}")
}
#[test]
fn empty_and_collapsed_lines_are_skipped_without_panics() {
    use geo::{LineString, line_string};
    let r = run(
        Rule::LineMustNotHaveDangles,
        vec![
            LineString::empty().into(),
            line_string![(x:0.,y:0.),(x:0.,y:0.)].into(),
        ],
        None,
        0.,
    );
    assert!(r.is_valid());
    assert_eq!(r.warnings.len(), 1);
}
#[test]
fn multipart_line_overlap_aggregates_disconnected_pieces() {
    use geo::{MultiLineString, line_string};
    let multi = MultiLineString(vec![
        line_string![(x:0.,y:0.),(x:1.,y:0.)],
        line_string![(x:2.,y:0.),(x:3.,y:0.)],
    ]);
    let r = run(
        Rule::LineMustNotOverlap,
        vec![multi.clone().into(), multi.into()],
        None,
        0.,
    );
    assert_eq!(r.error_count(), 1);
    let e = r.errors().next().unwrap();
    assert_eq!(e.origin.unwrap().part, None);
    assert!(matches!(e.geometry, Geometry::MultiLineString(_)));
}
