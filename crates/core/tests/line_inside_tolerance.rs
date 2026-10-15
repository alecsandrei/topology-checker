//! Direct Euclidean coincidence for permissive line containment.
use topology_checker_core::{
    Layer, Rule, Topology, Violation, ViolationKind,
    geo::{
        Coord, Euclidean, Geometry, Length, LineString, MultiLineString, Point, Polygon, Rect,
        Rotate, Translate,
    },
};
fn rectangle(x0: f64, y0: f64, x1: f64, y1: f64) -> Polygon {
    Rect::new(Coord { x: x0, y: y0 }, Coord { x: x1, y: y1 }).to_polygon()
}
fn line(points: &[(f64, f64)]) -> Geometry {
    LineString::from(points.to_vec()).into()
}
fn run(source: Geometry, target: Vec<Polygon>, tolerance: f64) -> Vec<Violation> {
    let mut topology = Topology::new(tolerance).unwrap();
    let a = topology.add_layer(Layer::from_features("source", [(71, source)]));
    let b = topology.add_layer(Layer::from_features(
        "target",
        target
            .into_iter()
            .enumerate()
            .map(|(i, p)| (100 + i as i64, p.into())),
    ));
    topology
        .add_rule(Rule::LineMustBeInside, a, Some(b))
        .unwrap();
    let report = topology.validate().unwrap();
    assert_eq!(
        topology.validate().unwrap().rules[0].violations,
        report.rules[0].violations
    );
    let mut streamed = Vec::new();
    topology
        .validate_into(|_, violation| {
            streamed.push(violation);
            Ok::<_, ()>(())
        })
        .unwrap();
    assert_eq!(streamed, report.rules[0].violations);
    for violation in &streamed {
        assert_eq!(violation.origin.unwrap().fid, 71);
        assert!(violation.destination.is_none());
        assert_eq!(violation.kind, ViolationKind::Outside);
    }
    streamed
}
fn length(violations: &[Violation]) -> f64 {
    violations
        .iter()
        .map(|v| match &v.geometry {
            Geometry::LineString(line) => Euclidean.length(line),
            Geometry::MultiLineString(lines) => Euclidean.length(lines),
            other => panic!("unexpected output {other:?}"),
        })
        .sum()
}
#[test]
fn exterior_and_hole_coincidence_is_inclusive_on_binary_inputs() {
    let target = rectangle(0., 0., 10., 10.);
    for offset in [1e-9, 1_f64.next_down(), 1., 1_f64.next_up()] {
        let pieces = run(
            line(&[(2., -offset), (8., -offset)]),
            vec![target.clone()],
            1.,
        );
        assert_eq!(pieces.is_empty(), offset <= 1., "{offset}: {pieces:?}");
    }
    let target = Polygon::new(
        rectangle(-10., -10., 10., 10.).exterior().clone(),
        vec![rectangle(0., 0., 6., 6.).exterior().clone()],
    );
    for offset in [1e-9, 1_f64.next_down(), 1., 1_f64.next_up()] {
        let pieces = run(
            line(&[(2., offset), (4., offset)]),
            vec![target.clone()],
            1.,
        );
        assert_eq!(pieces.is_empty(), offset <= 1., "hole {offset}: {pieces:?}");
    }
}
#[test]
fn boundary_capsules_preserve_short_uncovered_original_tails() {
    let source = line(&[(2., 0.), (10.125, 0.)]);
    assert_eq!(
        run(source, vec![rectangle(0., 0., 10., 10.)], 0.1)[0].geometry,
        line(&[(10.1, 0.), (10.125, 0.)])
    );
    let entirely_outside = line(&[(20., 0.), (20.025, 0.)]);
    assert_eq!(
        run(
            entirely_outside.clone(),
            vec![rectangle(0., 0., 10., 10.)],
            0.1
        )[0]
        .geometry,
        entirely_outside
    );
}
#[test]
fn corner_caps_are_euclidean_and_keep_both_residual_tails() {
    let pieces = run(
        line(&[(-2., -0.6), (12., -0.6)]),
        vec![rectangle(0., 0., 10., 10.)],
        1.,
    );
    assert_eq!(pieces.len(), 1);
    assert!((length(&pieces) - 2.4).abs() < 1e-12, "{pieces:?}");
    for x in [3_f64.next_down(), 3., 3_f64.next_up()] {
        let pieces = run(
            line(&[(-x, -4.), (-x, -3.5)]),
            vec![rectangle(0., 0., 10., 10.)],
            5.,
        );
        assert_eq!(pieces.is_empty(), x <= 3., "{x}: {pieces:?}");
    }
}
#[test]
fn coincidence_does_not_propagate_through_other_boundary_edges() {
    let target = vec![rectangle(0., 0., 10., 10.), rectangle(0., -0.75, 10., -0.5)];
    let source = line(&[(2., -1.8), (8., -1.8)]);
    assert!((length(&run(source, target, 1.)) - 6.).abs() < 1e-12);
}
#[test]
fn reversed_segmented_and_transformed_sources_keep_direct_coverage() {
    let original = vec![(2., -0.6), (10., -0.6), (12., -0.6)];
    for degrees in [0., 17., 90., 173.] {
        for (dx, dy) in [(0., 0.), (500_000., -200_000.)] {
            let target = rectangle(0., 0., 10., 10.)
                .rotate_around_point(degrees, Point::new(0., 0.))
                .translate(dx, dy);
            for mut points in [original.clone(), vec![original[0], original[2]]] {
                for reverse in [false, true] {
                    if reverse {
                        points.reverse();
                    }
                    let source = LineString::from(points.clone())
                        .rotate_around_point(degrees, Point::new(0., 0.))
                        .translate(dx, dy);
                    let pieces = run(source.into(), vec![target.clone()], 1.);
                    assert!(
                        (length(&pieces) - 1.2).abs() < 1e-8,
                        "{degrees} {dx} {dy}: {pieces:?}"
                    );
                }
            }
            let covered = LineString::from(vec![(2., -0.99), (8., -0.99)])
                .rotate_around_point(degrees, Point::new(0., 0.))
                .translate(dx, dy);
            assert!(run(covered.into(), vec![target.clone()], 1.).is_empty());
            let uncovered = LineString::from(vec![(2., -1.01), (8., -1.01)])
                .rotate_around_point(degrees, Point::new(0., 0.))
                .translate(dx, dy);
            assert!((length(&run(uncovered.into(), vec![target], 1.)) - 6.).abs() < 1e-8);
        }
    }
}
#[test]
fn multipart_source_roles_and_input_coordinates_survive_clipping() {
    let source = MultiLineString::new(vec![
        LineString::from(vec![(2., -0.5), (8., -0.5)]),
        LineString::from(vec![(10., 5.), (11.25, 5.)]),
    ]);
    let pieces = run(source.into(), vec![rectangle(0., 0., 10., 10.)], 1.);
    assert_eq!(pieces.len(), 1);
    assert_eq!(pieces[0].origin.unwrap().part, Some(1));
    assert_eq!(pieces[0].geometry, line(&[(11., 5.), (11.25, 5.)]));
}
