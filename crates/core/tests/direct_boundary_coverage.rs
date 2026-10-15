//! Direct Euclidean coverage, tolerant closure and short residual diagnostics.
use topology_checker_core::{
    Layer, Rule, Topology, Violation, ViolationKind,
    geo::{Geometry, LineString, Rect, coord},
};

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
fn required_boundary_coverage_keeps_each_short_uncovered_tail() {
    let polygon: Geometry = Rect::new(coord! {x: 0., y: 0.}, coord! {x: 10., y: 10.})
        .to_polygon()
        .into();
    let residuals = run(
        Rule::LineMustBeCoveredByPolygonBoundaries,
        vec![line(&[(5., -1.25), (5., 1.25)])],
        vec![polygon],
        1.,
    );
    assert_eq!(residuals.len(), 1);
    assert_eq!(residuals[0].kind, ViolationKind::NotCovered);
    let Geometry::MultiLineString(pieces) = &residuals[0].geometry else {
        panic!("both original residual tails are retained: {residuals:?}");
    };
    assert_eq!(pieces.0.len(), 2);
    for expected in [
        LineString::from(vec![(5., -1.25), (5., -1.)]),
        LineString::from(vec![(5., 1.), (5., 1.25)]),
    ] {
        assert!(pieces.0.contains(&expected), "{pieces:?}");
    }
}

#[test]
fn coverage_is_directional_and_does_not_require_sustained_overlap() {
    let polygon = Rect::new(coord! {x: 0., y: 0.}, coord! {x: 10., y: 10.}).to_polygon();
    let nearby = line(&[(5., -0.5), (5., 0.5)]);
    assert!(
        run(
            Rule::LineMustBeCoveredByPolygonBoundaries,
            vec![nearby.clone()],
            vec![polygon.clone().into()],
            1.,
        )
        .is_empty()
    );
    let reciprocal = run(
        Rule::PolygonBoundariesMustBeCoveredByLines,
        vec![polygon.into()],
        vec![nearby],
        1.,
    );
    assert_eq!(reciprocal.len(), 1);
    assert_eq!(reciprocal[0].kind, ViolationKind::NotCovered);
}

#[test]
fn boundary_distance_is_inclusive_and_keeps_sub_ulp_residuals() {
    for distance in [1_f64.next_down(), 1., 1_f64.next_up()] {
        for reverse in [false, true] {
            let mut points = vec![(5., -distance), (5., distance)];
            if reverse {
                points.reverse();
            }
            let polygon = Rect::new(coord! {x: 0., y: 0.}, coord! {x: 10., y: 10.}).to_polygon();
            let errors = run(
                Rule::LineMustBeCoveredByPolygonBoundaries,
                vec![line(&points)],
                vec![polygon.into()],
                1.,
            );
            assert_eq!(errors.is_empty(), distance <= 1.);
            for error in errors {
                assert_eq!(error.kind, ViolationKind::NotCovered);
                assert!(error.destination.is_none());
            }
        }
    }
}

#[test]
fn reciprocal_coverage_keeps_a_short_gap_across_a_source_vertex() {
    use topology_checker_core::geo::{LineString, Polygon};
    let polygon = Polygon::new(
        LineString::from(vec![
            (0., 0.),
            (10., 0.),
            (10., 10.),
            (5., 10.),
            (0., 10.),
            (0., 0.),
        ]),
        vec![],
    );
    let target = vec![line(&[
        (4., 10.),
        (0., 10.),
        (0., 0.),
        (10., 0.),
        (10., 10.),
        (6., 10.),
    ])];
    let errors = run(
        Rule::PolygonBoundariesMustBeCoveredByLines,
        vec![polygon.into()],
        target,
        0.75,
    );
    assert_eq!(errors.len(), 1);
    assert_eq!(
        errors[0].geometry,
        line(&[(5.25, 10.), (5., 10.), (4.75, 10.)])
    );
    assert_eq!(errors[0].origin.unwrap().part, Some(0));
    assert!(errors[0].destination.is_none());
}

#[test]
fn hole_coverage_follows_original_edges_under_transforms_and_segmentation() {
    use topology_checker_core::geo::{MapCoords, Polygon};
    let outer = Rect::new(coord! {x: 0., y: 0.}, coord! {x: 20., y: 20.}).to_polygon();
    let hole = Rect::new(coord! {x: 5., y: 5.}, coord! {x: 15., y: 15.}).to_polygon();
    let polygon: Geometry =
        Polygon::new(outer.exterior().clone(), vec![hole.exterior().clone()]).into();
    for (offset, angle) in [((0., 0.), 0_f64), ((1e6, -2e6), 0.73)] {
        let (s, c) = angle.sin_cos();
        let transform = |g: Geometry| {
            g.map_coords(|p| topology_checker_core::geo::Coord {
                x: p.x * c - p.y * s + offset.0,
                y: p.x * s + p.y * c + offset.1,
            })
        };
        for distance in [0.99, 1.01] {
            for segmented in [false, true] {
                let points = if segmented {
                    vec![
                        (5. + distance, 6.),
                        (5. + distance, 9.),
                        (5. + distance, 14.),
                    ]
                } else {
                    vec![(5. + distance, 6.), (5. + distance, 14.)]
                };
                let source = transform(line(&points));
                let errors = run(
                    Rule::LineMustBeCoveredByPolygonBoundaries,
                    vec![source],
                    vec![transform(polygon.clone())],
                    1.,
                );
                assert_eq!(errors.is_empty(), distance < 1.);
            }
        }
    }
}
