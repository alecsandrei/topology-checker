//! Point decisions use inclusive distances on the actual binary coordinates.
use topology_checker_core::{
    Layer, Rule, Topology, Violation,
    geo::{Geometry, LineString, Point, Polygon, Rect, coord},
};

fn point(x: f64, y: f64) -> Geometry {
    Point::new(x, y).into()
}
fn line(points: &[(f64, f64)]) -> Geometry {
    LineString::from(points.to_vec()).into()
}
fn run(rule: Rule, source: Vec<Geometry>, target: Vec<Geometry>, tolerance: f64) -> Vec<Violation> {
    let mut topology = Topology::new(tolerance).unwrap();
    let source = topology.add_layer(Layer::from_geometries("source", source));
    let target = rule
        .inputs()
        .1
        .map(|_| topology.add_layer(Layer::from_geometries("target", target)));
    topology.add_rule(rule, source, target).unwrap();
    let expected = topology.validate().unwrap().rules.remove(0).violations;
    let mut streamed = Vec::new();
    topology
        .validate_into(|_, violation| {
            streamed.push(violation);
            Ok::<_, ()>(())
        })
        .unwrap();
    assert_eq!(streamed, expected);
    assert_eq!(topology.validate().unwrap().rules[0].violations, expected);
    expected
}

#[test]
fn rounded_hypotenuses_cannot_admit_points_above_the_threshold() {
    let polygon: Geometry = Rect::new(coord! {x:-10., y:-10.}, coord! {x:0.,y:0.})
        .to_polygon()
        .into();
    for y in [4_f64.next_down(), 4., 4_f64.next_up()] {
        let candidate = point(3., y);
        assert_eq!(
            run(
                Rule::PointMustNotOverlap,
                vec![point(0., 0.), candidate.clone()],
                vec![],
                5.
            )
            .is_empty(),
            y > 4.
        );
        for swap in [false, true] {
            let (a, b) = if swap {
                (point(0., 0.), candidate.clone())
            } else {
                (candidate.clone(), point(0., 0.))
            };
            assert_eq!(
                run(Rule::PointMustNotOverlapWith, vec![a], vec![b], 5.).is_empty(),
                y > 4.
            );
        }
        for rule in [
            Rule::PointMustBeCoveredByLines,
            Rule::PointMustBeCoveredByLineEndpoints,
            Rule::PointMustNotIntersectLines,
            Rule::PointMustBeCoveredByPolygonBoundaries,
            Rule::PointMustBeCoveredByPolygons,
            Rule::PointMustNotIntersectPolygons,
        ] {
            let target = if matches!(
                rule,
                Rule::PointMustBeCoveredByLines
                    | Rule::PointMustBeCoveredByLineEndpoints
                    | Rule::PointMustNotIntersectLines
            ) {
                line(&[(-10., 0.), (0., 0.)])
            } else {
                polygon.clone()
            };
            let errors = run(rule, vec![candidate.clone()], vec![target], 5.);
            let prohibited = matches!(
                rule,
                Rule::PointMustNotIntersectLines | Rule::PointMustNotIntersectPolygons
            );
            assert_eq!(
                errors.is_empty(),
                if prohibited { y > 4. } else { y <= 4. },
                "{rule}: {y}"
            );
        }
        let errors = run(
            Rule::LineEndpointsMustBeCoveredByPoints,
            vec![line(&[(100., 0.), (3., y)])],
            vec![point(100., 0.), point(0., 0.)],
            5.,
        );
        assert_eq!(errors.is_empty(), y <= 4.);
    }
}

#[test]
fn proper_interior_excludes_the_inclusive_boundary_band_in_all_point_rules() {
    let target: Geometry = Polygon::new(
        LineString::from(vec![
            (23., -11.),
            (-17., 19.),
            (-47., -21.),
            (-7., -51.),
            (23., -11.),
        ]),
        vec![],
    )
    .into();
    for tolerance in [5_f64.next_down(), 5., 5_f64.next_up()] {
        for rule in [Rule::PointMustBeInside, Rule::PointMustNotBeInsidePolygons] {
            let errors = run(rule, vec![point(0., 0.)], vec![target.clone()], tolerance);
            assert_eq!(
                errors.is_empty(),
                if rule == Rule::PointMustBeInside {
                    tolerance < 5.
                } else {
                    tolerance >= 5.
                }
            );
        }
        for rule in [
            Rule::PolygonMustContainPoint,
            Rule::PolygonMustContainOnePoint,
            Rule::PolygonMustNotContainPoints,
        ] {
            let errors = run(rule, vec![target.clone()], vec![point(0., 0.)], tolerance);
            assert_eq!(
                errors.is_empty(),
                if rule == Rule::PolygonMustNotContainPoints {
                    tolerance >= 5.
                } else {
                    tolerance < 5.
                }
            );
        }
    }
}

#[test]
fn point_coincidence_is_pairwise_under_feature_permutation() {
    for order in [[0, 1, 2], [2, 0, 1], [1, 2, 0]] {
        let features = [
            (-10, point(0., 0.)),
            (20, point(0.75, 0.)),
            (30, point(1.5, 0.)),
        ];
        let mut topology = Topology::new(1.).unwrap();
        let source = topology.add_layer(Layer::from_features(
            "chain",
            order.map(|i| features[i].clone()),
        ));
        topology
            .add_rule(Rule::PointMustNotOverlap, source, None)
            .unwrap();
        let result = topology.validate().unwrap();
        let mut pairs = result.rules[0]
            .violations
            .iter()
            .map(|v| {
                let (a, b) = (v.origin.unwrap().fid, v.destination.unwrap().fid);
                let lower = a.min(b);
                assert_eq!(
                    v.geometry,
                    if lower == -10 {
                        point(0., 0.)
                    } else {
                        point(0.75, 0.)
                    }
                );
                (lower, a.max(b))
            })
            .collect::<Vec<_>>();
        pairs.sort();
        assert_eq!(pairs, vec![(-10, 20), (20, 30)]);
    }
}

#[test]
fn tiny_nonzero_segment_distances_do_not_underflow_to_zero() {
    for rule in [
        Rule::PointMustBeCoveredByLines,
        Rule::PointMustNotIntersectLines,
    ] {
        for (p, target) in [
            (point(0., 1e-200), line(&[(-1., 0.), (1., 0.)])),
            (
                point(0.25e-200, 0.75e-200),
                line(&[(0., 0.), (1e-200, 1e-200)]),
            ),
        ] {
            let errors = run(rule, vec![p], vec![target], 0.);
            assert_eq!(errors.is_empty(), rule == Rule::PointMustNotIntersectLines);
        }
    }
}
