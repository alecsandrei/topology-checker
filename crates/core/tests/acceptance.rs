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
