use geo::{
    Contains, Geometry, LineString, Point, Rect, Relate, coord, coordinate_position::CoordPos,
    dimensions::Dimensions,
};
use proptest::prelude::*;
use std::collections::BTreeSet;
use topology_checker_core::{ErrorType, Layer, Report, Rule, Topology};

type Pairs = BTreeSet<(i64, i64, ErrorType)>;
fn pairs(report: &Report) -> Pairs {
    report
        .errors()
        .map(|e| {
            (
                e.origin.unwrap().fid,
                e.destination.unwrap().fid,
                e.error_type,
            )
        })
        .collect()
}
fn check(input: Vec<Geometry>, other: Option<Vec<Geometry>>, rule: Rule, tolerance: f64) -> Report {
    let mut topology = Topology::new(tolerance).unwrap();
    let a = topology.add_layer(Layer::from_geometries("a", input));
    let b = other.map(|geometries| topology.add_layer(Layer::from_geometries("b", geometries)));
    topology.add_rule(rule, a, b).unwrap();
    topology.validate().unwrap()
}
fn line() -> impl Strategy<Value = LineString> {
    (-4i32..5, -4i32..5, -4i32..5, -4i32..5)
        .prop_filter("distinct endpoints", |&(x, y, u, v)| x != u || y != v)
        .prop_map(|(x, y, u, v)| LineString::from(vec![(x as f64, y as f64), (u as f64, v as f64)]))
}
fn relation(a: &LineString, b: &LineString) -> Option<ErrorType> {
    match a.relate(b).get(CoordPos::Inside, CoordPos::Inside) {
        Dimensions::OneDimensional => Some(ErrorType::Overlap),
        Dimensions::ZeroDimensional => Some(ErrorType::Crossing),
        _ => None,
    }
}
proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]
    #[test]
    fn line_pairs_match_de9im(lines in prop::collection::vec(line(), 0..15)) {
        let mut expected = Pairs::new();
        for i in 0..lines.len() { for j in i + 1..lines.len() {
            if let Some(kind) = relation(&lines[i], &lines[j]) { expected.insert((i as i64, j as i64, kind)); }
        }}
        let report = check(lines.into_iter().map(Geometry::LineString).collect(), None, Rule::LineMustNotIntersect, 0.);
        prop_assert_eq!(pairs(&report), expected);
    }
    #[test]
    fn cross_layer_line_pairs_match_de9im(a in prop::collection::vec(line(), 0..10), b in prop::collection::vec(line(), 0..10)) {
        let mut expected = Pairs::new();
        for (i, x) in a.iter().enumerate() { for (j, y) in b.iter().enumerate() {
            if let Some(kind) = relation(x, y) { expected.insert((i as i64, j as i64, kind)); }
        }}
        let report = check(a.into_iter().map(Geometry::LineString).collect(), Some(b.into_iter().map(Geometry::LineString).collect()), Rule::LineMustNotIntersectWith, 0.);
        prop_assert_eq!(pairs(&report), expected);
    }
    #[test]
    fn point_pairs_match_distance(points in prop::collection::vec((-4i32..5, -4i32..5), 0..25), tolerance in prop_oneof![Just(0.), Just(0.1), Just(1.), Just(1.5)]) {
        let mut expected = Pairs::new();
        for i in 0..points.len() { for j in i + 1..points.len() {
            if ((points[i].0 - points[j].0) as f64).hypot((points[i].1 - points[j].1) as f64) <= tolerance { expected.insert((i as i64, j as i64, ErrorType::Overlap)); }
        }}
        let report = check(points.into_iter().map(|(x, y)| Geometry::Point(Point::new(x as f64, y as f64))).collect(), None, Rule::PointMustNotOverlap, tolerance);
        prop_assert_eq!(pairs(&report), expected);
    }
    #[test]
    fn polygon_pairs_match_de9im(rectangles in prop::collection::vec((-5i32..5, -5i32..5, 1i32..5, 1i32..5), 0..12)) {
        let polygons = rectangles.into_iter().map(|(x,y,w,h)| Rect::new(coord! { x:x as f64,y:y as f64 },coord! { x:(x+w) as f64,y:(y+h) as f64 }).to_polygon()).collect::<Vec<_>>();
        let mut expected = Pairs::new();
        for i in 0..polygons.len() { for j in i + 1..polygons.len() {
            if polygons[i].relate(&polygons[j]).get(CoordPos::Inside, CoordPos::Inside) == Dimensions::TwoDimensional { expected.insert((i as i64,j as i64,ErrorType::Overlap)); }
        }}
        let report = check(polygons.into_iter().map(Geometry::Polygon).collect(), None, Rule::PolygonMustNotOverlap, 0.);
        prop_assert_eq!(pairs(&report), expected);
    }
    #[test]
    fn point_inside_matches_brute_force(points in prop::collection::vec((-6i32..6, -6i32..6), 0..20), rectangles in prop::collection::vec((-5i32..5, -5i32..5, 1i32..5, 1i32..5), 0..10)) {
        let points = points.into_iter().map(|(x,y)| Point::new(x as f64,y as f64)).collect::<Vec<_>>();
        let polygons = rectangles.into_iter().map(|(x,y,w,h)| Rect::new(coord! { x:x as f64,y:y as f64 },coord! { x:(x+w) as f64,y:(y+h) as f64 }).to_polygon()).collect::<Vec<_>>();
        let expected = points.iter().enumerate().filter(|(_,point)| !polygons.iter().any(|polygon| polygon.contains(*point))).map(|(i,_)| i as i64).collect::<BTreeSet<_>>();
        let report = check(points.into_iter().map(Geometry::Point).collect(), Some(polygons.into_iter().map(Geometry::Polygon).collect()), Rule::PointMustBeInside, 0.);
        let actual = report.errors().map(|e| e.origin.unwrap().fid).collect::<BTreeSet<_>>();
        prop_assert_eq!(actual, expected);
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]
    #[test]
    fn cross_layer_points_match_distance(a in prop::collection::vec((-4i32..5, -4i32..5), 0..20), b in prop::collection::vec((-4i32..5, -4i32..5), 0..20), tolerance in prop_oneof![Just(0.), Just(1.), Just(1.5)]) {
        let mut expected = Pairs::new();
        for (i,x) in a.iter().enumerate() { for (j,y) in b.iter().enumerate() {
            if ((x.0-y.0) as f64).hypot((x.1-y.1) as f64) <= tolerance { expected.insert((i as i64,j as i64,ErrorType::Overlap)); }
        }}
        let points = |input: Vec<(i32,i32)>| input.into_iter().map(|(x,y)| Geometry::Point(Point::new(x as f64,y as f64))).collect();
        prop_assert_eq!(pairs(&check(points(a),Some(points(b)),Rule::PointMustNotOverlapWith,tolerance)),expected);
    }
    #[test]
    fn interior_touches_match_de9im(lines in prop::collection::vec(line(), 0..15)) {
        let mut expected = Pairs::new();
        for i in 0..lines.len() { for j in i + 1..lines.len() {
            let matrix = lines[i].relate(&lines[j]);
            let kind = match matrix.get(CoordPos::Inside,CoordPos::Inside) {
                Dimensions::OneDimensional => Some(ErrorType::Overlap),
                Dimensions::ZeroDimensional => Some(ErrorType::Crossing),
                _ if matrix.get(CoordPos::Inside,CoordPos::OnBoundary) == Dimensions::ZeroDimensional || matrix.get(CoordPos::OnBoundary,CoordPos::Inside) == Dimensions::ZeroDimensional => Some(ErrorType::TouchInterior),
                _ => None,
            };
            if let Some(kind) = kind { expected.insert((i as i64,j as i64,kind)); }
        }}
        prop_assert_eq!(pairs(&check(lines.into_iter().map(Geometry::LineString).collect(),None,Rule::LineMustNotIntersectOrTouchInterior,0.)),expected);
    }
}
