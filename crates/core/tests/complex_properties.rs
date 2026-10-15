//! Constrained valid geometry generators, with independent distance and DE-9IM oracles.
use geo::{
    Contains, Distance, Euclidean, Geometry, LineString, MultiLineString, MultiPoint, MultiPolygon,
    Point, Polygon, Rect, Relate, coord, coordinate_position::CoordPos, dimensions::Dimensions,
};
use proptest::prelude::*;
use std::collections::BTreeSet;
use topology_checker_core::{Layer, Report, Rule, Topology, ViolationKind as K};

fn run(rule: Rule, a: Vec<Geometry>, b: Vec<Geometry>, tolerance: f64) -> Report {
    let mut t = Topology::new(tolerance).unwrap();
    let a = t.add_layer(Layer::from_features(
        "primary",
        a.into_iter().enumerate().map(|(i, g)| (-100 + i as i64, g)),
    ));
    let b = rule.inputs().1.map(|_| {
        t.add_layer(Layer::from_features(
            "target",
            b.into_iter().enumerate().map(|(i, g)| (-100 + i as i64, g)),
        ))
    });
    t.add_rule(rule, a, b).unwrap();
    t.validate().unwrap()
}
fn bent_line() -> impl Strategy<Value = LineString> {
    (-8i32..9, -8i32..9, 1i32..5, 1i32..5, any::<bool>()).prop_map(|(x, y, w, h, reverse)| {
        let (x, y, w, h) = (f64::from(x), f64::from(y), f64::from(w), f64::from(h));
        let mut c = vec![(x, y), (x + w, y), (x + w, y + h)];
        if reverse {
            c.reverse();
        }
        LineString::from(c)
    })
}
fn complex_polygon() -> impl Strategy<Value = Polygon> {
    (-8i32..9, -8i32..9, 2i32..5, 0u8..3).prop_map(|(x, y, s, shape)| {
        let (x, y, s) = (f64::from(x), f64::from(y), f64::from(s));
        let outer = Rect::new(coord! {x:x,y:y}, coord! {x:x+4.*s,y:y+4.*s}).to_polygon();
        match shape {
            0 => Polygon::new(
                LineString::from(vec![
                    (x, y),
                    (x + 4. * s, y),
                    (x + 4. * s, y + s),
                    (x + s, y + s),
                    (x + s, y + 4. * s),
                    (x, y + 4. * s),
                    (x, y),
                ]),
                vec![],
            ),
            1 => Polygon::new(
                outer.exterior().clone(),
                vec![
                    Rect::new(coord! {x:x+s,y:y+s}, coord! {x:x+3.*s,y:y+3.*s})
                        .to_polygon()
                        .exterior()
                        .clone(),
                ],
            ),
            _ => outer,
        }
    })
}
fn multipart_polygon() -> impl Strategy<Value = MultiPolygon> {
    complex_polygon().prop_map(|p| {
        use geo::Translate;
        let other = p.translate(40., 0.);
        MultiPolygon(vec![p, other])
    })
}
fn point_parts() -> impl Strategy<Value = MultiPoint> {
    prop::collection::vec((-10i32..60, -10i32..25), 1..4).prop_map(|c| {
        MultiPoint(
            c.into_iter()
                .map(|(x, y)| Point::new(f64::from(x), f64::from(y)))
                .collect(),
        )
    })
}
/// Four 10 by 10 tiles meeting at (10, 10), each side moved inward or outward
/// by an amount around the tolerances tested.
fn tiles() -> impl Strategy<Value = Vec<Polygon>> {
    let shift = prop_oneof![
        Just(-0.003),
        Just(0.),
        Just(0.002),
        Just(0.004),
        Just(0.006),
        Just(0.012)
    ];
    prop::collection::vec(prop::array::uniform4(shift), 4).prop_map(|shifts| {
        shifts
            .into_iter()
            .enumerate()
            .map(|(i, [l, b, r, t])| {
                let (x, y) = (10. * (i % 2) as f64, 10. * (i / 2) as f64);
                Rect::new(
                    coord! {x: x + l, y: y + b},
                    coord! {x: x + 10. - r, y: y + 10. - t},
                )
                .to_polygon()
            })
            .collect()
    })
}
/// Short paths with vertices on a half-unit grid, offset into nearby cracks.
fn crack_path() -> impl Strategy<Value = LineString> {
    let offset = prop_oneof![
        Just(0.),
        Just(0.001),
        Just(0.003),
        Just(0.0049),
        Just(-0.002)
    ];
    let coordinate = (14i32..27, offset).prop_map(|(k, o)| f64::from(k) / 2. + o);
    prop::collection::vec((coordinate.clone(), coordinate), 2..5).prop_filter_map(
        "vertices apart",
        |c| {
            // A segment shorter than the tolerance is never reported, wherever
            // it lies, so keep the vertices well apart.
            let mut kept: Vec<(f64, f64)> = vec![];
            for p in c {
                if kept
                    .last()
                    .is_none_or(|q| (p.0 - q.0).hypot(p.1 - q.1) >= 0.25)
                {
                    kept.push(p);
                }
            }
            (kept.len() >= 2).then(|| LineString::from(kept))
        },
    )
}
fn origins(r: &Report) -> BTreeSet<i64> {
    r.violations().map(|v| v.origin.unwrap().fid).collect()
}
fn pairs(r: &Report) -> BTreeSet<(i64, i64, K)> {
    r.violations()
        .map(|v| (v.origin.unwrap().fid, v.destination.unwrap().fid, v.kind))
        .collect()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]
    #[test]
    fn bent_and_multipart_line_point_contacts_match_distance(
        points in prop::collection::vec(point_parts(),0..8),lines in prop::collection::vec(bent_line(),0..8),tolerance in prop_oneof![Just(0.),Just(0.25),Just(1.)]){
        use geo::Translate;
        let lines = lines
            .into_iter()
            .map(|l| MultiLineString(vec![l.clone(), l.translate(24., 0.)]))
            .collect::<Vec<_>>();
        let mut expected = BTreeSet::new();
        for (i, points) in points.iter().enumerate() {
            for (j, lines) in lines.iter().enumerate() {
                if points
                    .0
                    .iter()
                    .any(|p| Euclidean.distance(p, lines) <= tolerance)
                {
                    expected.insert((-100 + i as i64, -100 + j as i64, K::Intersection));
                }
            }
        }
        let a = points
            .iter()
            .cloned()
            .map(Geometry::MultiPoint)
            .collect::<Vec<_>>();
        let b = lines
            .iter()
            .cloned()
            .map(Geometry::MultiLineString)
            .collect::<Vec<_>>();
        prop_assert_eq!(
            pairs(&run(
                Rule::PointMustNotIntersectLines,
                a.clone(),
                b.clone(),
                tolerance
            )),
            expected
        );
        for rule in [
            Rule::PointMustBeCoveredByLines,
            Rule::PointMustBeCoveredByLineEndpoints,
        ] {
            let expected = points
                .iter()
                .enumerate()
                .filter(|(_, ps)| {
                    ps.0.iter().any(|p| {
                        !lines.iter().any(|ls| {
                            ls.0.iter().any(|l| {
                                if rule == Rule::PointMustBeCoveredByLines {
                                    Euclidean.distance(p, l) <= tolerance
                                } else {
                                    [l.0[0], *l.0.last().unwrap()]
                                        .iter()
                                        .any(|c| Euclidean.distance(p, &Point(*c)) <= tolerance)
                                }
                            })
                        })
                    })
                })
                .map(|(i, _)| -100 + i as i64)
                .collect::<BTreeSet<_>>();
            let actual = run(rule, a.clone(), b.clone(), tolerance);
            prop_assert_eq!(
                actual
                    .violations()
                    .map(|v| v.origin.unwrap().fid)
                    .collect::<BTreeSet<_>>(),
                expected
            );
        }
    }
    #[test]
    fn bent_line_contacts_with_concavity_and_holes_match_de9im(
        lines in prop::collection::vec(bent_line(),0..8),polygons in prop::collection::vec(complex_polygon(),0..6)){
        let mut contact = BTreeSet::new();
        let mut overlap = BTreeSet::new();
        let mut shared = BTreeSet::new();
        for (i, l) in lines.iter().enumerate() {
            for (j, p) in polygons.iter().enumerate() {
                let m = l.relate(p);
                let key = (-100 + i as i64, -100 + j as i64);
                if m.is_intersects() {
                    contact.insert((key.0, key.1, K::Intersection));
                }
                if m.get(CoordPos::Inside, CoordPos::Inside) == Dimensions::OneDimensional
                    || m.get(CoordPos::Inside, CoordPos::OnBoundary) == Dimensions::OneDimensional
                {
                    overlap.insert((key.0, key.1, K::Overlap));
                }
                if m.get(CoordPos::Inside, CoordPos::OnBoundary) == Dimensions::OneDimensional {
                    shared.insert((key.0, key.1, K::Overlap));
                }
            }
        }
        let a = lines
            .into_iter()
            .map(Geometry::LineString)
            .collect::<Vec<_>>();
        let b = polygons
            .into_iter()
            .map(Geometry::Polygon)
            .collect::<Vec<_>>();
        prop_assert_eq!(
            pairs(&run(
                Rule::LineMustNotIntersectPolygons,
                a.clone(),
                b.clone(),
                0.
            )),
            contact
        );
        prop_assert_eq!(
            pairs(&run(
                Rule::LineMustNotOverlapWithPolygons,
                a.clone(),
                b.clone(),
                0.
            )),
            overlap
        );
        prop_assert_eq!(
            pairs(&run(Rule::LineMustNotOverlapPolygonBoundaries, a, b, 0.)),
            shared
        );
    }
    #[test]
    fn multipart_point_assignments_require_every_part_inside(
        polygons in prop::collection::vec(multipart_polygon(),0..6),points in prop::collection::vec(point_parts(),0..8)){
        let assigned = |p: &MultiPolygon, ps: &MultiPoint| ps.0.iter().all(|q| p.contains(q));
        for rule in [
            Rule::PolygonMustContainPoint,
            Rule::PolygonMustContainOnePoint,
        ] {
            let expected_polygons = polygons
                .iter()
                .enumerate()
                .filter(|(_, p)| {
                    let n = points.iter().filter(|ps| assigned(p, ps)).count();
                    if rule == Rule::PolygonMustContainPoint {
                        n == 0
                    } else {
                        n != 1
                    }
                })
                .map(|(i, _)| -100 + i as i64)
                .collect::<BTreeSet<_>>();
            let expected_points = if rule == Rule::PolygonMustContainOnePoint {
                points
                    .iter()
                    .enumerate()
                    .filter(|(_, ps)| polygons.iter().filter(|p| assigned(p, ps)).count() != 1)
                    .map(|(i, _)| -100 + i as i64)
                    .collect()
            } else {
                BTreeSet::new()
            };
            let r = run(
                rule,
                polygons
                    .iter()
                    .cloned()
                    .map(Geometry::MultiPolygon)
                    .collect(),
                points.iter().cloned().map(Geometry::MultiPoint).collect(),
                0.,
            );
            prop_assert_eq!(
                r.violations()
                    .filter_map(|v| v.origin.map(|r| r.fid))
                    .collect::<BTreeSet<_>>(),
                expected_polygons
            );
            prop_assert_eq!(
                r.violations()
                    .filter(|v| v.origin.is_none())
                    .map(|v| v.destination.unwrap().fid)
                    .collect::<BTreeSet<_>>(),
                expected_points
            );
        }
    }
    #[test]
    fn concave_holed_multipart_overlap_matches_area_relation(polygons in prop::collection::vec(multipart_polygon(),0..6)){
        let mut expected = BTreeSet::new();
        for i in 0..polygons.len() {
            for j in i + 1..polygons.len() {
                if polygons[i]
                    .relate(&polygons[j])
                    .get(CoordPos::Inside, CoordPos::Inside)
                    == Dimensions::TwoDimensional
                {
                    expected.insert((-100 + i as i64, -100 + j as i64, K::Overlap));
                }
            }
        }
        prop_assert_eq!(
            pairs(&run(
                Rule::PolygonMustNotOverlap,
                polygons.into_iter().map(Geometry::MultiPolygon).collect(),
                vec![],
                0.
            )),
            expected
        );
    }
    #[test]
    fn multipart_point_area_coverage_matches_contains_and_boundary_distance(
        polygons in prop::collection::vec(multipart_polygon(),0..6),points in prop::collection::vec(point_parts(),0..8),tolerance in prop_oneof![Just(0.),Just(0.25),Just(1.)]){
        let covered = |q: &Point| {
            polygons.iter().any(|ps| {
                ps.0.iter().any(|p| {
                    p.contains(q)
                        || std::iter::once(p.exterior())
                            .chain(p.interiors())
                            .any(|ring| Euclidean.distance(ring, q) <= tolerance)
                })
            })
        };
        let expected = points
            .iter()
            .enumerate()
            .filter(|(_, ps)| ps.0.iter().any(|q| !covered(q)))
            .map(|(i, _)| -100 + i as i64)
            .collect::<BTreeSet<_>>();
        let r = run(
            Rule::PointMustBeCoveredByPolygons,
            points.into_iter().map(Geometry::MultiPoint).collect(),
            polygons.into_iter().map(Geometry::MultiPolygon).collect(),
            tolerance,
        );
        prop_assert_eq!(
            r.violations()
                .map(|v| v.origin.unwrap().fid)
                .collect::<BTreeSet<_>>(),
            expected
        );
    }
    #[test]
    fn lines_properly_inside_closed_coverage_are_inside_it(
        tiles in tiles(),
        lines in prop::collection::vec(crack_path(), 1..8),
        degrees in prop_oneof![Just(0.), Just(17.)],
        tolerance in prop_oneof![Just(0.), Just(0.005), Just(0.01)]){
        use geo::Rotate;
        let centre = Point::new(10., 10.);
        let rotate = |g: Geometry| g.rotate_around_point(degrees, centre);
        let tiles = tiles.into_iter().map(|p| rotate(p.into())).collect::<Vec<_>>();
        let lines = lines.into_iter().map(|l| rotate(l.into())).collect::<Vec<_>>();
        let strict = origins(&run(
            Rule::LineMustBeProperlyInsidePolygons,
            lines.clone(),
            tiles.clone(),
            tolerance,
        ));
        let outside = origins(&run(Rule::LineMustBeInside, lines, tiles, tolerance));
        prop_assert!(outside.is_subset(&strict), "{:?} {:?}", outside, strict);
    }
    #[test]
    fn lines_not_reported_outside_stay_within_tolerance_of_the_polygons(
        tiles in tiles(),
        lines in prop::collection::vec(crack_path(), 1..8),
        degrees in prop_oneof![Just(0.), Just(17.), Just(45.)],
        offset in prop_oneof![Just((0., 0.)), Just((5e5, 1.8e5)), Just((3e6, -2e6))],
        tolerance in prop_oneof![Just(0.), Just(0.005), Just(0.01)]){
        use geo::{Rotate, Translate};
        let centre = Point::new(10., 10.);
        let tiles = tiles
            .into_iter()
            .map(|p| p.rotate_around_point(degrees, centre).translate(offset.0, offset.1))
            .collect::<Vec<_>>();
        let mut lines = lines
            .into_iter()
            .map(|l| l.rotate_around_point(degrees, centre).translate(offset.0, offset.1))
            .collect::<Vec<_>>();
        // Lines leaving each corner along the extension of an edge. After
        // rounding, adjacent edges are only nearly perpendicular.
        for tile in &tiles {
            let ring = &tile.exterior().0;
            for k in 0..4 {
                let (a, before) = (ring[k], ring[(k + 3) % 4]);
                let length = (a.x - before.x).hypot(a.y - before.y);
                lines.push(LineString::from(vec![
                    (a.x, a.y),
                    (a.x + 3. * (a.x - before.x) / length, a.y + 3. * (a.y - before.y) / length),
                ]));
            }
        }
        let reported = origins(&run(
            Rule::LineMustBeInside,
            lines.iter().cloned().map(Geometry::LineString).collect(),
            tiles.iter().cloned().map(Geometry::Polygon).collect(),
            tolerance,
        ));
        // Closed gaps never carry a line beyond the tolerance from the polygons.
        for (i, l) in lines.iter().enumerate() {
            if reported.contains(&(-100 + i as i64)) {
                continue;
            }
            for s in l.lines() {
                for k in 0..=64 {
                    let t = f64::from(k) / 64.;
                    let p = Point::new(s.start.x + t * (s.end.x - s.start.x), s.start.y + t * (s.end.y - s.start.y));
                    let nearest = tiles
                        .iter()
                        .map(|tile| Euclidean.distance(&p, tile))
                        .fold(f64::INFINITY, f64::min);
                    prop_assert!(nearest <= tolerance * (1. + 1e-6) + 1e-9, "{:?} {:?}", p, nearest);
                }
            }
        }
    }
    #[test]
    fn bent_line_pairs_match_de9im(lines in prop::collection::vec(bent_line(),0..8)){
        let mut expected = BTreeSet::new();
        for i in 0..lines.len() {
            for j in i + 1..lines.len() {
                let m = lines[i].relate(&lines[j]);
                let kind = match m.get(CoordPos::Inside, CoordPos::Inside) {
                    Dimensions::OneDimensional => Some(K::Overlap),
                    Dimensions::ZeroDimensional => Some(K::Crossing),
                    _ => None,
                };
                if let Some(k) = kind {
                    expected.insert((-100 + i as i64, -100 + j as i64, k));
                }
            }
        }
        prop_assert_eq!(
            pairs(&run(
                Rule::LineMustNotIntersect,
                lines.into_iter().map(Geometry::LineString).collect(),
                vec![],
                0.
            )),
            expected
        );
    }
}
