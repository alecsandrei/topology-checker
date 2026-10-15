use geo::{
    Contains, Geometry, LineString, MultiLineString, Point, Rect, Relate, coord,
    coordinate_position::CoordPos, dimensions::Dimensions,
};
use proptest::{prelude::*, test_runner::TestCaseError};
use std::collections::{BTreeMap, BTreeSet};
use topology_checker_core::{Layer, Report, Rule, Topology, ViolationKind};

type Pairs = BTreeSet<(i64, i64, ViolationKind)>;
fn pairs(report: &Report) -> Pairs {
    report
        .violations()
        .map(|e| (e.origin.unwrap().fid, e.destination.unwrap().fid, e.kind))
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
fn relation(a: &LineString, b: &LineString) -> Option<ViolationKind> {
    match a.relate(b).get(CoordPos::Inside, CoordPos::Inside) {
        Dimensions::OneDimensional => Some(ViolationKind::Overlap),
        Dimensions::ZeroDimensional => Some(ViolationKind::Crossing),
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
            if ((points[i].0 - points[j].0) as f64).hypot((points[i].1 - points[j].1) as f64) <= tolerance { expected.insert((i as i64, j as i64, ViolationKind::Overlap)); }
        }}
        let report = check(points.into_iter().map(|(x, y)| Geometry::Point(Point::new(x as f64, y as f64))).collect(), None, Rule::PointMustNotOverlap, tolerance);
        prop_assert_eq!(pairs(&report), expected);
    }
    #[test]
    fn polygon_pairs_match_de9im(rectangles in prop::collection::vec((-5i32..5, -5i32..5, 1i32..5, 1i32..5), 0..12)) {
        let polygons = rectangles.into_iter().map(|(x,y,w,h)| Rect::new(coord! { x:x as f64,y:y as f64 },coord! { x:(x+w) as f64,y:(y+h) as f64 }).to_polygon()).collect::<Vec<_>>();
        let mut expected = Pairs::new();
        for i in 0..polygons.len() { for j in i + 1..polygons.len() {
            if polygons[i].relate(&polygons[j]).get(CoordPos::Inside, CoordPos::Inside) == Dimensions::TwoDimensional { expected.insert((i as i64,j as i64,ViolationKind::Overlap)); }
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
        let actual = report.violations().map(|e| e.origin.unwrap().fid).collect::<BTreeSet<_>>();
        prop_assert_eq!(actual, expected);
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]
    #[test]
    fn cross_layer_points_match_distance(a in prop::collection::vec((-4i32..5, -4i32..5), 0..20), b in prop::collection::vec((-4i32..5, -4i32..5), 0..20), tolerance in prop_oneof![Just(0.), Just(1.), Just(1.5)]) {
        let mut expected = Pairs::new();
        for (i,x) in a.iter().enumerate() { for (j,y) in b.iter().enumerate() {
            if ((x.0-y.0) as f64).hypot((x.1-y.1) as f64) <= tolerance { expected.insert((i as i64,j as i64,ViolationKind::Overlap)); }
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
                Dimensions::OneDimensional => Some(ViolationKind::Overlap),
                Dimensions::ZeroDimensional => Some(ViolationKind::Crossing),
                _ if matrix.get(CoordPos::Inside,CoordPos::OnBoundary) == Dimensions::ZeroDimensional || matrix.get(CoordPos::OnBoundary,CoordPos::Inside) == Dimensions::ZeroDimensional => Some(ViolationKind::TouchInterior),
                _ => None,
            };
            if let Some(kind) = kind { expected.insert((i as i64,j as i64,kind)); }
        }}
        prop_assert_eq!(pairs(&check(lines.into_iter().map(Geometry::LineString).collect(),None,Rule::LineMustNotIntersectOrTouchInterior,0.)),expected);
    }
}

/// Multipart lines on a small grid, some parts closed. No segment has zero length,
/// and segments of one feature meet only at grid vertices without retracing each
/// other, so every node is exact and the mod-2 boundary is well defined.
fn multi_line() -> impl Strategy<Value = MultiLineString> {
    use geo::line_intersection::{LineIntersection, line_intersection};
    let part = (
        prop::collection::vec((-3i32..4, -3i32..4), 2..4),
        any::<bool>(),
    )
        .prop_map(|(points, closed)| {
            let mut part = points
                .into_iter()
                .map(|(x, y)| coord! { x: x as f64, y: y as f64 })
                .collect::<Vec<_>>();
            if closed && part.len() > 2 {
                part.push(part[0]);
            }
            LineString(part)
        });
    prop::collection::vec(part, 1..4)
        .prop_map(MultiLineString)
        .prop_filter("distinct vertices, segments meeting at vertices", |m| {
            let segments = m.0.iter().flat_map(LineString::lines).collect::<Vec<_>>();
            segments.iter().enumerate().all(|(i, &a)| {
                a.start != a.end
                    && segments[i + 1..].iter().all(|&b| {
                        matches!(
                            line_intersection(a, b),
                            None | Some(LineIntersection::SinglePoint {
                                is_proper: false,
                                ..
                            })
                        )
                    })
            })
        })
}
/// Line-intersect results per feature pair against the DE-9IM, whose multipart
/// boundary is the mod-2 rule. Shared linework or points must be reported as an
/// overlap or a crossing; whether the lines also meet elsewhere is not visible in
/// the matrix. Otherwise an interior meeting a boundary is the only touch-interior.
fn match_mod_2_de9im(
    a: &[MultiLineString],
    b: Option<&[MultiLineString]>,
) -> Result<(), TestCaseError> {
    let multi = |lines: &[MultiLineString]| {
        lines
            .iter()
            .cloned()
            .map(Geometry::MultiLineString)
            .collect::<Vec<_>>()
    };
    let rules = if b.is_some() {
        [
            Rule::LineMustNotIntersectWith,
            Rule::LineMustNotIntersectOrTouchInteriorWith,
        ]
    } else {
        [
            Rule::LineMustNotIntersect,
            Rule::LineMustNotIntersectOrTouchInterior,
        ]
    };
    for (rule, touch) in rules.into_iter().zip([false, true]) {
        let mut found = BTreeMap::<_, BTreeSet<_>>::new();
        for (i, j, kind) in pairs(&check(multi(a), b.map(multi), rule, 0.)) {
            found.entry((i, j)).or_default().insert(kind);
        }
        for (i, x) in a.iter().enumerate() {
            for (j, y) in b.unwrap_or(a).iter().enumerate() {
                if b.is_none() && j <= i {
                    continue;
                }
                let kinds = found.remove(&(i as i64, j as i64)).unwrap_or_default();
                let matrix = x.relate(y);
                let touches = matrix.get(CoordPos::Inside, CoordPos::OnBoundary)
                    != Dimensions::Empty
                    || matrix.get(CoordPos::OnBoundary, CoordPos::Inside) != Dimensions::Empty;
                match matrix.get(CoordPos::Inside, CoordPos::Inside) {
                    Dimensions::OneDimensional => prop_assert!(
                        kinds.contains(&ViolationKind::Overlap),
                        "{rule} {i} {j}: {kinds:?}"
                    ),
                    Dimensions::ZeroDimensional => prop_assert!(
                        kinds.contains(&ViolationKind::Crossing)
                            && !kinds.contains(&ViolationKind::Overlap),
                        "{rule} {i} {j}: {kinds:?}"
                    ),
                    _ => prop_assert_eq!(
                        &kinds,
                        &if touch && touches {
                            BTreeSet::from([ViolationKind::TouchInterior])
                        } else {
                            BTreeSet::new()
                        },
                        "{} {} {}",
                        rule,
                        i,
                        j
                    ),
                }
                prop_assert!(touch || !kinds.contains(&ViolationKind::TouchInterior));
            }
        }
        prop_assert!(found.is_empty(), "{rule}: {found:?}");
    }
    Ok(())
}
proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]
    #[test]
    fn multipart_line_contacts_match_mod_2_de9im(lines in prop::collection::vec(multi_line(), 0..5)) {
        match_mod_2_de9im(&lines, None)?;
    }
    #[test]
    fn cross_layer_multipart_line_contacts_match_mod_2_de9im(a in prop::collection::vec(multi_line(), 0..4), b in prop::collection::vec(multi_line(), 0..4)) {
        match_mod_2_de9im(&a, Some(&b))?;
    }
}

fn polygon_strategy() -> impl Strategy<Value = geo::Polygon> {
    (-5i32..5, -5i32..5, 1i32..6, 1i32..6).prop_map(|(x, y, w, h)| {
        Rect::new(
            coord! {x:x as f64,y:y as f64},
            coord! {x:(x+w) as f64,y:(y+h) as f64},
        )
        .to_polygon()
    })
}
proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]
    #[test]
    fn cross_geometry_point_coverage_matches_brute_force(
        points in prop::collection::vec((-6i32..6,-6i32..6),0..15),
        lines in prop::collection::vec(line(),0..10),
        tolerance in prop_oneof![Just(0.),Just(0.25),Just(1.)]) {
        use geo::{Distance,Euclidean};
        let points = points.into_iter().map(|(x,y)| Point::new(x as f64,y as f64)).collect::<Vec<_>>();
        for rule in [Rule::PointMustBeCoveredByLines,Rule::PointMustBeCoveredByLineEndpoints] {
            let expected = points.iter().enumerate().filter(|(_,p)| !lines.iter().any(|l| {
                if rule == Rule::PointMustBeCoveredByLines {
                    Euclidean.distance(l,*p) <= tolerance
                } else {
                    // Generated parts have two integer-coordinate ends. Their
                    // squared separation gives an independent exact closure oracle.
                    let dx = l.0[0].x - l.0[1].x;
                    let dy = l.0[0].y - l.0[1].y;
                    dx * dx + dy * dy > tolerance * tolerance
                        && l.0.iter().any(|c| Euclidean.distance(&Point(*c),*p) <= tolerance)
                }
            })).map(|(i,_)| i as i64).collect::<BTreeSet<_>>();
            let r = check(points.iter().copied().map(Geometry::Point).collect(),Some(lines.iter().cloned().map(Geometry::LineString).collect()),rule,tolerance);
            prop_assert_eq!(r.violations().map(|e| e.origin.unwrap().fid).collect::<BTreeSet<_>>(),expected);
        }
        let mut expected = Pairs::new();
        for (i,p) in points.iter().enumerate() { for (j,l) in lines.iter().enumerate() { if Euclidean.distance(l,p) <= tolerance { expected.insert((i as i64,j as i64,ViolationKind::Intersection)); } } }
        prop_assert_eq!(pairs(&check(points.into_iter().map(Geometry::Point).collect(),Some(lines.into_iter().map(Geometry::LineString).collect()),Rule::PointMustNotIntersectLines,tolerance)),expected);
    }
    #[test]
    fn cross_geometry_polygon_contacts_match_de9im(
        lines in prop::collection::vec(line(),0..10), polygons in prop::collection::vec(polygon_strategy(),0..8)) {
        use geo::{Distance,Euclidean};
        let mut expected = Pairs::new();
        let mut overlap = Pairs::new();
        let mut boundary_overlap = Pairs::new();
        let mut crossings = Pairs::new();
        for (i,l) in lines.iter().enumerate() { for (j,p) in polygons.iter().enumerate() {
            let matrix = l.relate(p);
            if matrix.is_intersects() { expected.insert((i as i64,j as i64,ViolationKind::Intersection)); }
            if matrix.is_crosses() { crossings.insert((i as i64,j as i64,ViolationKind::Crossing)); }
            if matrix.get(CoordPos::Inside,CoordPos::OnBoundary) == Dimensions::OneDimensional { boundary_overlap.insert((i as i64,j as i64,ViolationKind::Overlap)); }
            if matrix.get(CoordPos::Inside,CoordPos::Inside) == Dimensions::OneDimensional || matrix.get(CoordPos::Inside,CoordPos::OnBoundary) == Dimensions::OneDimensional { overlap.insert((i as i64,j as i64,ViolationKind::Overlap)); }
        } }
        let input = lines.iter().cloned().map(Geometry::LineString).collect::<Vec<_>>();
        let target = polygons.iter().cloned().map(Geometry::Polygon).collect::<Vec<_>>();
        prop_assert_eq!(pairs(&check(input.clone(),Some(target.clone()),Rule::LineMustNotIntersectPolygons,0.)),expected);
        prop_assert_eq!(pairs(&check(input.clone(),Some(target.clone()),Rule::LineMustNotOverlapWithPolygons,0.)),overlap);
        prop_assert_eq!(pairs(&check(input.clone(),Some(target.clone()),Rule::LineMustNotOverlapPolygonBoundaries,0.)),boundary_overlap);
        prop_assert_eq!(pairs(&check(input.clone(),Some(target.clone()),Rule::LineMustNotCrossPolygonBoundaries,0.)),crossings);
        // Test strict containment against a single polygon using the independent
        // topological relationship and the minimum whole-line boundary distance.
        if let Some(p) = polygons.first() {
            let expected = lines.iter().enumerate().filter(|(_,l)| !(p.contains(*l) && Euclidean.distance(p.exterior(),*l) > 0.)).map(|(i,_)| i as i64).collect::<BTreeSet<_>>();
            let r = check(input,Some(vec![p.clone().into()]),Rule::LineMustBeProperlyInsidePolygons,0.);
            prop_assert_eq!(r.violations().map(|e| e.origin.unwrap().fid).collect::<BTreeSet<_>>(),expected);
        }
    }
    #[test]
    fn cross_geometry_feature_containment_matches_brute_force(
        lines in prop::collection::vec(line(),0..10), polygons in prop::collection::vec(polygon_strategy(),0..8)) {
        let mut expected = Pairs::new();
        for (i,l) in lines.iter().enumerate() { for (j,p) in polygons.iter().enumerate() { if p.contains(l) && !l.relate(p).is_contains() { expected.insert((i as i64,j as i64,ViolationKind::Contained)); } } }
        prop_assert_eq!(pairs(&check(lines.into_iter().map(Geometry::LineString).collect(),Some(polygons.into_iter().map(Geometry::Polygon).collect()),Rule::GeometryMustNotBeContainedBy,1.)),expected);
    }
    #[test]
    fn cross_geometry_point_counts_match_brute_force(
        points in prop::collection::vec((-6i32..6,-6i32..6),0..15), polygons in prop::collection::vec(polygon_strategy(),0..8)) {
        let points = points.into_iter().map(|(x,y)| Point::new(x as f64,y as f64)).collect::<Vec<_>>();
        for rule in [Rule::PolygonMustContainPoint,Rule::PolygonMustContainOnePoint] {
            let expected_polygons = polygons.iter().enumerate().filter(|(_,p)| {
                let count = points.iter().filter(|q| p.contains(*q)).count();
                if rule == Rule::PolygonMustContainPoint { count == 0 } else { count != 1 }
            }).map(|(i,_)| i as i64).collect::<BTreeSet<_>>();
            let expected_points = if rule == Rule::PolygonMustContainPoint { BTreeSet::new() } else { points.iter().enumerate().filter(|(_,q)| polygons.iter().filter(|p| p.contains(*q)).count() != 1).map(|(i,_)| i as i64).collect() };
            let r = check(polygons.iter().cloned().map(Geometry::Polygon).collect(),Some(points.iter().copied().map(Geometry::Point).collect()),rule,0.);
            prop_assert_eq!(r.violations().filter_map(|e| e.origin.map(|r| r.fid)).collect::<BTreeSet<_>>(),expected_polygons);
            prop_assert_eq!(r.violations().filter(|e| e.origin.is_none()).map(|e| e.destination.unwrap().fid).collect::<BTreeSet<_>>(),expected_points);
        }
    }
}
