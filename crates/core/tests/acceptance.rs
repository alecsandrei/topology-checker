use topology_checker_core::{
    Error, Layer, Rule, Topology, ViolationKind,
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
    assert_eq!(r.violation_count(), 3);
    assert!(r.violations().all(|e| e.origin != e.destination));
}
#[test]
fn overlap_aggregates_multipart_features() {
    let g = Geometry::MultiPoint(MultiPoint(vec![Point::new(0., 0.), Point::new(1., 0.)]));
    let r = run(Rule::PointMustNotOverlap, vec![g.clone(), g], None, 0.);
    assert_eq!(r.violation_count(), 1);
    assert_eq!(r.violations().next().unwrap().origin.unwrap().part, None);
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
    assert_eq!(r.violation_count(), 1);
    assert_eq!(r.violations().next().unwrap().kind, ViolationKind::Outside);
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
    assert_eq!(r.violation_count(), 1);
    use topology_checker_core::geo::Area;
    let Geometry::Polygon(p) = &r.violations().next().unwrap().geometry else {
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
    assert_eq!(r.violation_count(), 1);
    assert_eq!(
        r.violations().next().unwrap().kind,
        ViolationKind::OuterBoundary
    );
}
#[test]
fn gap_holes_and_outer_boundaries_are_distinct() {
    let p = polygon!(exterior: [(x:0.,y:0.),(x:5.,y:0.),(x:5.,y:5.),(x:0.,y:5.)],interiors:[[(x:1.,y:1.),(x:2.,y:1.),(x:2.,y:2.),(x:1.,y:2.)]]);
    let r = run(Rule::PolygonMustNotHaveGaps, vec![p.into()], None, 0.);
    assert_eq!(r.violation_count(), 2);
    assert_eq!(
        r.violations()
            .filter(|e| e.kind == ViolationKind::Gap)
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
    assert_eq!(r.violation_count(), 1);
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
    assert!(r.is_valid(), "{:?}", r.violations().collect::<Vec<_>>());
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
    assert_eq!(r.violation_count(), 1);
    let Geometry::LineString(l) = &r.violations().next().unwrap().geometry else {
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
        assert_eq!(r.violation_count(), 1);
        let Geometry::LineString(l) = &r.violations().next().unwrap().geometry else {
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
    assert_eq!(r.violation_count(), 1);
    assert_eq!(
        r.violations().next().unwrap().geometry,
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
    assert_eq!(r.violation_count(), 1);
    assert_eq!(
        r.violations().next().unwrap().geometry,
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
        run(Rule::LineMustNotHaveDangles, lines.clone(), None, 0.).violation_count(),
        4
    );
    assert_eq!(
        run(Rule::LineMustNotHaveDangles, lines, None, 0.001).violation_count(),
        2
    );
    let lines = vec![
        line_string![(x:0.,y:0.),(x:2.,y:0.)].into(),
        line_string![(x:1.,y:0.0001),(x:1.,y:1.)].into(),
    ];
    assert_eq!(
        run(Rule::LineMustNotHaveDangles, lines, None, 0.001).violation_count(),
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
    assert_eq!(r.violation_count(), 1);
    assert_eq!(
        r.violations().next().unwrap().kind,
        ViolationKind::SelfCrossing
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
        .violation_count(),
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
    assert_eq!(r.violation_count(), 1);
    assert_eq!(
        r.violations().next().unwrap().kind,
        ViolationKind::TouchInterior
    );
    let cross = line_string![(x:1.,y:-1.),(x:1.,y:1.)];
    let r = run(
        Rule::LineMustNotIntersectWith,
        vec![a.into()],
        Some(vec![cross.into()]),
        0.,
    );
    assert_eq!(r.violation_count(), 1);
    assert_eq!(r.violations().next().unwrap().kind, ViolationKind::Crossing);
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
        .violation_count(),
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
    assert_eq!(r.rules[0].violations.len(), 2);
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
    assert_eq!(r.violation_count(), 1);
    let e = r.violations().next().unwrap();
    assert_eq!(e.origin.unwrap().part, None);
    assert!(matches!(e.geometry, Geometry::MultiLineString(_)));
}

#[test]
fn concurrent_crossings_do_not_create_roundoff_overlaps() {
    use geo::LineString;
    let r = run(
        Rule::LineMustNotIntersectWith,
        vec![LineString::from(vec![(4., -4.), (-1., -1.)]).into()],
        Some(vec![
            LineString::from(vec![(0., -3.), (3., -3.)]).into(),
            LineString::from(vec![(3., -4.), (-1., 2.)]).into(),
        ]),
        0.,
    );
    assert_eq!(r.violation_count(), 2);
    assert!(r.violations().all(|e| e.kind == ViolationKind::Crossing));
    let coordinates = r.violations().map(|e| &e.geometry).collect::<Vec<_>>();
    assert_eq!(coordinates[0], coordinates[1]);
}

// Vienna zoning FID 25654 and land use FID 6316 (EPSG:31256) touch in
// decimal, but the binary land-use vertex (13116.27, 343792.55) lies 5.6e-12
// inside the zoning edge (DE-9IM II = 2). Tolerance 0 is exact on binary
// coordinates, so every exact frame reports the overlap; a positive tolerance
// treats it as a sliver.
const REG_014_ZONING: &[(f64, f64)] = &[
    (13175.4636, 343833.1577),
    (13184.61, 343871.1),
    (13181.57, 343874.65),
    (13181.3465, 343874.6307),
    (13180.2959, 343874.5397),
    (13089.03, 343866.64),
    (13079.7853, 343827.3631),
    (13073.38, 343800.15),
    (13076.5, 343799.66),
    (13079.61, 343799.16),
    (13082.73, 343798.65),
    (13085.85, 343798.13),
    (13088.96, 343797.6),
    (13092.08, 343797.06),
    (13095.19, 343796.51),
    (13098.3, 343795.96),
    (13101.4, 343795.39),
    (13104.51, 343794.81),
    (13107.61, 343794.23),
    (13110.72, 343793.64),
    (13113.82, 343793.04),
    (13116.92, 343792.42),
    (13118.1169, 343792.1798),
    (13120.01, 343791.8),
    (13123.11, 343791.17),
    (13126.2, 343790.53),
    (13129.3, 343789.89),
    (13132.39, 343789.23),
    (13135.47, 343788.56),
    (13138.56, 343787.89),
    (13141.64, 343787.2),
    (13144.72, 343786.51),
    (13147.8, 343785.81),
    (13150.88, 343785.1),
    (13153.96, 343784.37),
    (13157.03, 343783.64),
    (13158.3092, 343783.3317),
    (13159.2814, 343783.0974),
    (13160.1, 343782.9),
    (13163.17, 343782.16),
    (13175.4636, 343833.1577),
];
const REG_014_LANDUSE: &[(f64, f64)] = &[
    (13163.17, 343782.16),
    (13116.27, 343792.55),
    (13073.37, 343800.15),
    (13061.47, 343801.96),
    (13058.3294, 343791.643),
    (13052.0176, 343770.9089),
    (13051.54, 343769.34),
    (13047.6266, 343752.5616),
    (13029.97, 343676.86),
    (13049.2771, 343672.321),
    (13065.7861, 343754.5508),
    (13148.7596, 343735.5008),
    (13128.0369, 343653.8854),
    (13143.66, 343650.23),
    (13162.7608, 343731.1516),
    (13167.08, 343749.45),
    (13155.67, 343752.23),
    (13163.17, 343782.16),
];
/// Translate by an offset that keeps every coordinate exact (Sterbenz).
fn exactly_shifted(c: &[(f64, f64)], dx: f64, dy: f64) -> geo::Polygon {
    let ring = c
        .iter()
        .map(|&(x, y)| {
            let p = geo::Coord {
                x: x - dx,
                y: y - dy,
            };
            assert_eq!((p.x + dx, p.y + dy), (x, y), "exact translation");
            p
        })
        .collect();
    geo::Polygon::new(geo::LineString(ring), vec![])
}
#[test]
fn sub_ulp_binary_overlap_is_exact_at_zero_tolerance() {
    use geo::{Area, Relate, coordinate_position::CoordPos, dimensions::Dimensions};
    for (dx, dy) in [(0., 0.), (13116., 343792.)] {
        let zoning = exactly_shifted(REG_014_ZONING, dx, dy);
        let landuse = exactly_shifted(REG_014_LANDUSE, dx, dy);
        let ii = zoning
            .relate(&landuse)
            .get(CoordPos::Inside, CoordPos::Inside);
        assert_eq!(ii, Dimensions::TwoDimensional);
        for (rule, input, other) in [
            (
                Rule::PolygonMustNotOverlapWith,
                vec![zoning.clone().into()],
                Some(vec![landuse.clone().into()]),
            ),
            (
                Rule::PolygonMustNotOverlap,
                vec![zoning.clone().into(), landuse.clone().into()],
                None,
            ),
        ] {
            let r = run(rule, input.clone(), other.clone(), 0.);
            assert_eq!(r.violation_count(), 1, "{rule} offset ({dx}, {dy})");
            let e = r.violations().next().unwrap();
            assert_eq!(e.kind, ViolationKind::Overlap);
            let Geometry::Polygon(p) = &e.geometry else {
                panic!("polygon overlap");
            };
            assert!(p.unsigned_area() > 0. && p.unsigned_area() < 1e-20);
            assert!(run(rule, input, other, 1e-9).is_valid());
        }
    }
}
#[test]
fn sub_ulp_binary_gap_is_exact_at_zero_tolerance() {
    // P lies on a-b in decimal and 5.6e-12 north of it in binary. The tiles
    // around the sliver a-P-b share every other edge.
    let (a, b, p) = (
        (13113.82, 343793.04),
        (13116.92, 343792.42),
        (13116.27, 343792.55),
    );
    let tile = |c: &[(f64, f64)]| Geometry::from(exactly_shifted(c, 0., 0.));
    let tiles = vec![
        tile(&[a, (a.0, a.1 - 5.), (b.0, b.1 - 5.), b, a]),
        tile(&[a, p, (p.0, p.1 + 5.), (a.0, a.1 + 5.), a]),
        tile(&[p, b, (b.0, b.1 + 5.), (p.0, p.1 + 5.), p]),
        tile(&[
            (a.0 - 5., a.1 - 5.),
            (a.0, a.1 - 5.),
            a,
            (a.0, a.1 + 5.),
            (a.0 - 5., a.1 + 5.),
            (a.0 - 5., a.1 - 5.),
        ]),
    ];
    let gaps = |tolerance| {
        run(Rule::PolygonMustNotHaveGaps, tiles.clone(), None, tolerance)
            .violations()
            .filter(|e| e.kind == ViolationKind::Gap)
            .count()
    };
    assert_eq!((gaps(0.), gaps(1e-9)), (1, 0));
}
