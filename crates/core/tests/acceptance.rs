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
