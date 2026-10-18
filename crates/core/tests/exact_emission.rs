//! Emitted polygons keep holes, islands and rounded nodes consistent.
use topology_checker_core::{
    Layer, Rule, Topology, ViolationKind,
    geo::{Geometry, LineString, Polygon},
};

fn polygon(shell: &[(f64, f64)], holes: &[&[(f64, f64)]]) -> Geometry {
    Polygon::new(
        LineString::from(shell.to_vec()),
        holes.iter().map(|h| LineString::from(h.to_vec())).collect(),
    )
    .into()
}
fn square(x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<(f64, f64)> {
    vec![(x0, y0), (x1, y0), (x1, y1), (x0, y1), (x0, y0)]
}

#[test]
fn islands_inside_holes_own_their_own_holes() {
    // A ring, an island inside its hole, and a hole inside the island: the innermost
    // hole lies inside both shells and belongs to the smaller one.
    let ring = polygon(&square(0., 0., 20., 20.), &[&square(4., 4., 16., 16.)]);
    let island = polygon(&square(6., 6., 14., 14.), &[&square(8., 8., 12., 12.)]);
    let mut topology = Topology::new(0.).unwrap();
    let layer = topology.add_layer(Layer::from_geometries("parcels", [ring, island]));
    topology
        .add_rule(Rule::PolygonMustNotHaveGaps, layer, None)
        .unwrap();
    let report = topology.validate().unwrap();
    let violations = &report.rules[0].violations;
    let count = |kind| violations.iter().filter(|v| v.kind == kind).count();
    // The gap between ring and island, the island's hole, and the two outer boundaries.
    assert_eq!(count(ViolationKind::Gap), 2);
    assert_eq!(count(ViolationKind::OuterBoundary), 2);
}

#[test]
fn holes_close_to_rounded_nodes_stay_inside_the_intersection() {
    // The triangle crosses the square's bottom edge at x = 11/3 and its right edge at
    // y = 19/14, which no double represents. The hole starts at the rounded height of
    // the second crossing, so ownership must be checked on the emitted coordinates.
    let y = 19f64 / 14.;
    let holed = polygon(
        &square(0., 0., 10., 10.),
        &[&[(5., y), (7., y), (7., y + 1.), (5., y + 1.), (5., y)]],
    );
    let triangle = polygon(&[(-1., -1.), (13., 2.), (2., 14.), (-1., -1.)], &[]);
    for tolerance in [0., 0.1] {
        let mut topology = Topology::new(tolerance).unwrap();
        let a = topology.add_layer(Layer::from_geometries("holed", [holed.clone()]));
        let b = topology.add_layer(Layer::from_geometries("triangle", [triangle.clone()]));
        topology
            .add_rule(Rule::PolygonMustNotOverlapWith, a, Some(b))
            .unwrap();
        let report = topology.validate().unwrap();
        let overlap = Polygon::try_from(report.rules[0].violations[0].geometry.clone()).unwrap();
        assert_eq!(overlap.interiors().len(), 1);
        let hole = &overlap.interiors()[0];
        assert!(hole.0.iter().all(|c| c.y >= y.next_down()));
    }
}

#[test]
fn intersection_emission_names_the_polygons_when_rounding_cannot_hold() {
    use topology_checker_core::Error;
    // Both polygons are a few ulps wide, so the exact intersection nodes between
    // them have no representable position that keeps the embedding.
    let (x1, x2) = (1. + f64::EPSILON, 1. + 2. * f64::EPSILON);
    let first = polygon(&[(1., 1.), (x2, 1.), (1., -1.), (1., 1.)], &[]);
    let second = polygon(
        &[(x2, 2.), (x1, 0.), (x1, 1.), (1., 0.), (x2, -1.), (x2, 2.)],
        &[],
    );
    let mut topology = Topology::new(0.).unwrap();
    let layer = topology.add_layer(Layer::from_features("slivers", [(7, first), (9, second)]));
    topology
        .add_rule(Rule::PolygonMustNotOverlap, layer, None)
        .unwrap();
    let result = topology.validate();
    assert!(
        matches!(
            &result,
            Err(Error::Precision { operation, rule: Some(Rule::PolygonMustNotOverlap), context, .. })
                if operation == "polygon intersection emission"
                    && context == ", layer slivers, FID 7; layer slivers, FID 9"
        ),
        "{result:?}"
    );
}

/// Polygons a few ulps wide around x = 1, given as multiples of the ulp and heights.
fn slivers(rings: &[&[(i32, f64)]]) -> Vec<Geometry> {
    rings
        .iter()
        .map(|ring| {
            let coords = ring
                .iter()
                .map(|&(ulps, y)| (1. + f64::from(ulps) * f64::EPSILON, y))
                .collect::<Vec<_>>();
            polygon(&coords, &[])
        })
        .collect()
}

#[test]
fn unions_of_ulp_wide_slivers_report_precision_failures_without_partial_results() {
    use topology_checker_core::{CancellationToken, Error, SinkError};
    let cases: [(f64, Vec<Geometry>); 3] = [
        (
            0.,
            slivers(&[
                &[(2, -1.), (3, 0.5), (4, -1.), (2, -1.), (2, -1.)],
                &[(2, -1.), (0, 0.), (1, 0.), (2, -1.)],
                &[(0, -2.), (0, 1.), (1, -1.), (2, 0.), (2, -1.), (0, -2.)],
            ]),
        ),
        (
            1e-15,
            slivers(&[
                &[(1, -1.), (1, 0.), (0, 1.), (1, -1.)],
                &[(1, 0.5), (0, -1.), (1, 0.), (1, 0.5)],
                &[(1, -1.), (0, 2.), (1, -0.5), (1, -1.)],
                &[(1, 2.), (0, -2.), (1, -1.), (1, 2.)],
            ]),
        ),
        (
            1e-15,
            slivers(&[
                &[(1, 0.5), (1, 1.), (2, 2.), (1, 0.5)],
                &[(2, 0.), (0, 0.5), (1, -1.), (2, 0.)],
                &[(0, -2.), (1, 2.), (2, -1.), (0, -2.)],
                &[(2, 1.), (1, 0.5), (0, 0.5), (0, -0.5), (2, -0.5), (2, 1.)],
                &[(0, 0.), (0, 0.), (0, 0.5), (0, 1.), (1, -1.), (0, 0.)],
            ]),
        ),
    ];
    for (tolerance, geometries) in cases {
        let mut topology = Topology::new(tolerance).unwrap();
        let layer = topology.add_layer(Layer::from_geometries("slivers", geometries));
        topology
            .add_rule(Rule::PolygonMustNotHaveGaps, layer, None)
            .unwrap();
        assert!(matches!(
            topology.validate(),
            Err(Error::Precision {
                rule: Some(Rule::PolygonMustNotHaveGaps),
                ..
            })
        ));
        let result = topology
            .validate_into_cancellable(CancellationToken::default(), |_, _| Ok::<_, ()>(()));
        assert!(matches!(
            result,
            Err(SinkError::Core(Error::Precision { .. }))
        ));
    }
}
