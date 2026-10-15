//! Direct boundary coincidence keeps short residuals and execution-mode contracts.
use topology_checker_core::{
    CancellationToken, Error, Layer, Rule, SinkError, Topology, ViolationKind,
    geo::{Coord, Geometry, LineString, MapCoords, MultiLineString, Rect, coord},
};

#[test]
fn near_edge_residuals_preserve_parts_inputs_and_execution_modes() {
    for quarter_turn in [false, true] {
        for shift in [0., 256.] {
            for reverse in [false, true] {
                let place = |p: Coord| {
                    let p = if quarter_turn {
                        coord! {x: -p.y, y: p.x}
                    } else {
                        p
                    };
                    coord! {x: p.x + shift, y: p.y - shift}
                };
                let line = |coordinates: Vec<(f64, f64)>| {
                    let mut line = LineString::from(coordinates).map_coords(place);
                    if reverse {
                        line.0.reverse();
                    }
                    line
                };
                let original: Geometry = MultiLineString(vec![
                    line(vec![(0.5, -0.25), (1.5, -0.25)]),
                    line(vec![(-1.25, 0.), (-0.75, 0.)]),
                    line(vec![(-0.75, 1.), (0.25, 1.)]),
                ])
                .into();
                let polygon = Rect::new(coord! {x: 0., y: 0.}, coord! {x: 2., y: 2.})
                    .to_polygon()
                    .map_coords(place);
                let mut topology = Topology::new(0.5).unwrap();
                let source = topology.add_layer(Layer::from_features(
                    "source",
                    [(-9_000_000_000, original.clone())],
                ));
                let target = topology.add_layer(Layer::from_features(
                    "target",
                    [(9_000_000_000, polygon.into())],
                ));
                topology
                    .add_rule(Rule::LineMustBeInside, source, Some(target))
                    .unwrap();

                let cancelled = CancellationToken::default();
                cancelled.cancel();
                assert!(matches!(
                    topology.validate_into_cancellable(cancelled, |_, _| Ok::<_, ()>(())),
                    Err(SinkError::Core(Error::Cancelled))
                ));
                let report = topology.validate().unwrap();
                let violations = &report.rules[0].violations;
                assert_eq!(violations.len(), 1, "{violations:?}");
                let violation = &violations[0];
                assert_eq!(violation.kind, ViolationKind::Outside);
                assert_eq!(violation.origin.unwrap().layer, source);
                assert_eq!(violation.origin.unwrap().fid, -9_000_000_000);
                assert_eq!(violation.origin.unwrap().part, None);
                assert_eq!(violation.destination, None);
                let Geometry::MultiLineString(residuals) = &violation.geometry else {
                    panic!("multiple contributing parts retain the multiline schema");
                };
                assert_eq!(residuals.0.len(), 2);
                for expected in [
                    line(vec![(-1.25, 0.), (-0.75, 0.)]),
                    line(vec![(-0.75, 1.), (-0.5, 1.)]),
                ] {
                    assert!(residuals.0.contains(&expected), "{residuals:?}");
                }
                let mut streamed = Vec::new();
                let summary = topology
                    .validate_into(|_, violation| {
                        streamed.push(violation);
                        Ok::<_, ()>(())
                    })
                    .unwrap();
                assert_eq!(&streamed, violations);
                assert_eq!(summary.violation_count(), 1);
                assert_eq!(
                    topology.validate().unwrap().rules[0].violations,
                    *violations
                );
                let (_, retained) = topology.layer(source).unwrap().features().next().unwrap();
                assert_eq!(retained, &original);
            }
        }
    }
}
