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
// Open Data DC alleys (ln_dc_alleys, EPSG:32618, CC BY 4.0), FIDs 7552 and
// 8983. 8983 ends exactly at the start of 7552. Its last segment is a
// 1.0000016 mm bend, just over a 1 mm tolerance, turning slightly more than a
// right angle. No other alley comes within 1 m of the shared node.
const DC_ALLEY_7552: &[(f64, f64)] = &[
    (331434.0956944006, 4307888.253122057),
    (331434.28908729664, 4307894.198311851),
    (331435.25813373365, 4307942.673724342),
];
const DC_ALLEY_8983: &[(f64, f64)] = &[
    (331434.37577908125, 4307852.221315392),
    (331434.2442669176, 4307870.352559431),
    (331434.0946946393, 4307888.25314398),
    (331434.0956944006, 4307888.253122057),
];
fn line_violations(
    rule: Rule,
    lines: &[(i64, &[(f64, f64)])],
    tolerance: f64,
) -> Vec<topology_checker_core::Violation> {
    let mut t = Topology::new(tolerance).unwrap();
    let layer = t.add_layer(Layer::from_features(
        "lines",
        lines
            .iter()
            .map(|&(fid, c)| (fid, geo::LineString::from(c.to_vec()).into())),
    ));
    t.add_rule(rule, layer, None).unwrap();
    t.validate().unwrap().rules.remove(0).violations
}
fn sorted_points(mut points: Vec<(f64, f64)>) -> Vec<(f64, f64)> {
    points.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    points
}
fn points(violations: &[topology_checker_core::Violation]) -> Vec<(f64, f64)> {
    sorted_points(
        violations
            .iter()
            .map(|v| {
                let Geometry::Point(p) = v.geometry else {
                    panic!("point violation {v:?}");
                };
                (p.x(), p.y())
            })
            .collect(),
    )
}
#[test]
fn final_bend_just_over_tolerance_keeps_end_to_end_pseudo_node() {
    let mut frames = Vec::new();
    // Exact translations, including one to the origin and one that keeps every
    // coordinate in its binade.
    for (dx, dy) in [
        (0., 0.),
        (331434., 4307888.),
        (-65536., -1048576.),
        (200000., 4000000.),
    ] {
        let shift = |c: &[(f64, f64)]| {
            c.iter()
                .map(|&(x, y)| {
                    let p = (x - dx, y - dy);
                    assert_eq!((p.0 + dx, p.1 + dy), (x, y), "exact translation");
                    p
                })
                .collect::<Vec<_>>()
        };
        frames.push((shift(DC_ALLEY_7552), shift(DC_ALLEY_8983)));
    }
    // One-ulp moves of the bend vertex.
    for axis in 0..2 {
        for step in [f64::next_up, f64::next_down] {
            let mut bent = DC_ALLEY_8983.to_vec();
            if axis == 0 {
                bent[2].0 = step(bent[2].0);
            } else {
                bent[2].1 = step(bent[2].1);
            }
            frames.push((DC_ALLEY_7552.to_vec(), bent));
        }
    }
    for (a, b) in frames {
        let node = a[0];
        assert_eq!(b[3], node);
        let bend = (b[3].0 - b[2].0).hypot(b[3].1 - b[2].1);
        assert!(bend > 0.001 && bend < 0.0010001, "{bend}");
        let far = sorted_points(vec![a[2], b[0]]);
        let ends = sorted_points(vec![b[0], node]);
        for reverse in [false, true] {
            let orient = |c: &[(f64, f64)]| {
                let mut c = c.to_vec();
                if reverse {
                    c.reverse();
                }
                c
            };
            let (a, b) = (orient(&a), orient(&b));
            for lines in [
                [(7552, &a[..]), (8983, &b[..])],
                [(8983, &b[..]), (7552, &a[..])],
            ] {
                let pseudo = line_violations(Rule::LineMustNotHavePseudoNodes, &lines, 0.001);
                assert_eq!(points(&pseudo), [node], "{pseudo:?}");
                assert_eq!(pseudo[0].kind, ViolationKind::PseudoNode);
                assert_eq!(pseudo[0].origin.unwrap().fid, 7552);
                assert_eq!(pseudo[0].destination.unwrap().fid, 8983);
                let dangles = line_violations(Rule::LineMustNotHaveDangles, &lines, 0.001);
                assert_eq!(points(&dangles), far, "{dangles:?}");
                for rule in [
                    Rule::LineMustNotSelfOverlap,
                    Rule::LineMustNotSelfIntersect,
                    Rule::LineMustNotOverlap,
                    Rule::LineMustNotIntersect,
                    Rule::LineMustNotIntersectOrTouchInterior,
                ] {
                    let errors = line_violations(rule, &lines, 0.001);
                    assert!(errors.is_empty(), "{rule}: {errors:?}");
                }
            }
            // Alone, the bent line ends freely at the node.
            let dangles = line_violations(Rule::LineMustNotHaveDangles, &[(8983, &b[..])], 0.001);
            assert_eq!(points(&dangles), ends, "{dangles:?}");
        }
    }
}
#[test]
fn final_bends_touch_their_own_line_only_after_leaving_the_tolerance() {
    let tolerance = 0.001;
    let a = [(0., 0.), (0.19, 5.95), (1.16, 54.4)];
    // As in the DC alleys: the last segment, of length s, turns about 90.5
    // degrees, so its end is within s * 0.99997 of the previous segment.
    let alley = |s: f64| vec![(0.28, -36.), (0.15, -18.), (-s, 0.), (0., 0.)];
    // The previous segment heads north and the last one turns `turn` degrees.
    let turned = |turn: f64, s: f64| {
        let (x, y) = (turn.to_radians().sin() * s, turn.to_radians().cos() * s);
        vec![(-x, -y - 36.), (-x, -y - 18.), (-x, -y), (0., 0.)]
    };
    let mut free = Vec::new();
    for f in [
        0.5, 0.9, 1., 1.00000001, 1.0000001, 1.0000016, 1.00001, 1.0001, 1.001, 1.01, 1.1, 1.5, 2.,
        3.,
    ] {
        free.push((90.5, alley(f * tolerance)));
        for turn in [30., 60., 90., 91., 95., 120., 135.] {
            free.push((turn, turned(turn, f * tolerance)));
        }
    }
    // Sharper bends still end freely while the bend vertex stays within the
    // tolerance of the point the end lies beside.
    for (turn, f) in [
        (150., 1.00000001),
        (150., 1.1),
        (170., 1.00000001),
        (170., 1.01),
        (179., 1.00000001),
        (179., 1.0001),
    ] {
        free.push((turn, turned(turn, f * tolerance)));
    }
    for (turn, b) in &free {
        let lines = [(1, &a[..]), (2, &b[..])];
        let pseudo = line_violations(Rule::LineMustNotHavePseudoNodes, &lines, tolerance);
        assert_eq!(
            points(&pseudo),
            [(0., 0.)],
            "turn {turn}, {b:?}: {pseudo:?}"
        );
        let dangles = line_violations(Rule::LineMustNotHaveDangles, &[(2, &b[..])], tolerance);
        assert_eq!(dangles.len(), 2, "turn {turn}, {b:?}: {dangles:?}");
    }
    // The other line may start just off the end. Its own projection onto the
    // previous segment then lies farther from the bend than the end does.
    for (turn, f, start) in [
        (179.25, 0.974, (-0.000134, -0.000067)),
        (160.4, 0.952, (-0.000263, -0.000131)),
    ] {
        let b = turned(turn, f * tolerance);
        let mut a = a.to_vec();
        a[0] = start;
        let lines = [(1, &a[..]), (2, &b[..])];
        let pseudo = line_violations(Rule::LineMustNotHavePseudoNodes, &lines, tolerance);
        assert_eq!(pseudo.len(), 1, "turn {turn}, {b:?}: {pseudo:?}");
        let dangles = line_violations(Rule::LineMustNotHaveDangles, &lines, tolerance);
        assert_eq!(points(&dangles), sorted_points(vec![a[2], b[0]]));
    }
    // A last segment retracing the line beyond the tolerance ends on the
    // line's own interior.
    for f in [1.5, 2., 3.] {
        let b = turned(179., f * tolerance);
        let lines = [(1, &a[..]), (2, &b[..])];
        let pseudo = line_violations(Rule::LineMustNotHavePseudoNodes, &lines, tolerance);
        assert!(pseudo.is_empty(), "{b:?}: {pseudo:?}");
        let dangles = line_violations(Rule::LineMustNotHaveDangles, &[(2, &b[..])], tolerance);
        assert_eq!(points(&dangles), [b[0]], "{b:?}: {dangles:?}");
        let retrace = line_violations(Rule::LineMustNotSelfOverlap, &[(2, &b[..])], tolerance);
        assert_eq!(retrace.len(), 1, "{b:?}: {retrace:?}");
    }
}
#[test]
fn final_bends_after_a_vertex_near_the_end_keep_end_to_end_pseudo_nodes() {
    let a = [(0., 0.), (0.19, 5.95), (1.16, 54.4)];
    // A vertex just before the bend lies within tolerance of the end and
    // joins its node, while the bend vertex does not.
    let mut lines = vec![
        (1., vec![(-0.98, -36.), (-0.98, 0.36), (0., 0.)]),
        (
            1.,
            vec![(-0.98, -36.), (-0.98, 0.01), (-0.98, 0.36), (0., 0.)],
        ),
        (
            0.001,
            vec![
                (-0.00098, -0.036),
                (-0.00098, 0.00001),
                (-0.00098, 0.00036),
                (0., 0.),
            ],
        ),
    ];
    // The previous segment heads north, the last one of length `s` turns
    // `turn` degrees, and a further vertex lies `d` before the bend.
    for (turn, s, d) in [(110., 1.05, 0.35), (135., 1.2, 0.5), (150., 1.1, 0.3)] {
        let r = f64::to_radians(turn);
        let (x, y) = (-s * r.sin(), -s * r.cos());
        lines.push((
            1.,
            vec![(x, y - 36.), (x, y - 18.), (x, y - d), (x, y), (0., 0.)],
        ));
    }
    for (tolerance, b) in lines {
        let a = a.map(|(x, y)| (x * tolerance, y * tolerance));
        let pseudo = line_violations(
            Rule::LineMustNotHavePseudoNodes,
            &[(1, &a[..]), (2, &b[..])],
            tolerance,
        );
        assert_eq!(points(&pseudo), [(0., 0.)], "{b:?}: {pseudo:?}");
        let dangles = line_violations(Rule::LineMustNotHaveDangles, &[(2, &b[..])], tolerance);
        assert_eq!(
            points(&dangles),
            sorted_points(vec![b[0], (0., 0.)]),
            "{b:?}: {dangles:?}"
        );
    }
}
#[test]
fn nearby_ends_without_final_bends_keep_their_nodes() {
    // The first line starts 0.30 from the end of the second, and the third
    // 0.80 from it, but 1.1 from the first. No line returns to itself, so the
    // noding is unaffected: the first start joins the second line's interior,
    // and the second end and third start form a pseudo-node.
    let lines: [(i64, &[(f64, f64)]); 3] = [
        (1, &[(0.14, -0.27), (-9.6, -17.25)]),
        (2, &[(14.86, 21.77), (0., 0.)]),
        (3, &[(-0.45, 0.66), (-30.3, -9.5)]),
    ];
    let dangles = line_violations(Rule::LineMustNotHaveDangles, &lines, 1.);
    assert_eq!(
        points(&dangles),
        sorted_points(vec![(-30.3, -9.5), (-9.6, -17.25), (14.86, 21.77)])
    );
    let pseudo = line_violations(Rule::LineMustNotHavePseudoNodes, &lines, 1.);
    assert_eq!(points(&pseudo), [(-0.45, 0.66)]);
    assert_eq!(pseudo[0].origin.unwrap().fid, 2);
    assert_eq!(pseudo[0].destination.unwrap().fid, 3);
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

#[test]
fn crossings_beside_trimmed_final_bends_are_streamed() {
    // Each first line ends with a bend that folds back within tolerance, and
    // the other line crosses it near that bend. Streaming validation must find
    // the crossing like a full report does.
    let line = |c: &[(f64, f64)]| Geometry::from(geo::LineString::from(c.to_vec()));
    let cases = [
        [
            line(&[(0., -20.), (0., 0.), (0.996, -0.836)]),
            line(&[(-3.06, -3.96), (3.37, 3.70)]),
        ],
        [
            line(&[
                (-34.721737743146846, -6.753819932292435),
                (-34.991816149611964, -7.789329059999547),
                (-19.508694995156848, -5.735933515375656),
            ]),
            line(&[
                (-36.86541861946715, -55.610091149695386),
                (-34.721737743146846, -6.753819932292435),
            ]),
        ],
    ];
    for pair in cases {
        for rule in [
            Rule::LineMustNotIntersect,
            Rule::LineMustNotIntersectOrTouchInterior,
        ] {
            let mut t = Topology::new(1.).unwrap();
            let a = t.add_layer(Layer::from_geometries("lines", pair.clone()));
            t.add_rule(rule, a, None).unwrap();
            let report = t.validate().unwrap();
            assert!(
                report
                    .violations()
                    .any(|v| v.kind == ViolationKind::Crossing),
                "{rule}: {report:?}"
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
}
