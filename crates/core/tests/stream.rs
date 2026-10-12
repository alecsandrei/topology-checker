use topology_checker_core::{Layer, Rule, SinkError, Topology, geo::Point};

#[test]
fn every_duplicate_pair_is_streamed_and_consumer_failure_is_preserved() {
    let mut t = Topology::new(0.).unwrap();
    let layer = t.add_layer(Layer::from_features(
        "duplicates",
        (0..100).rev().map(|i| (i, Point::new(0., 0.).into())),
    ));
    t.add_rule(Rule::PointMustNotOverlap, layer, None).unwrap();
    let eager = t.validate().unwrap();
    let mut streamed = Vec::new();
    let summary = t
        .validate_into(|spec, v| {
            assert_eq!(spec.rule, v.rule);
            streamed.push(v);
            Ok::<_, &str>(())
        })
        .unwrap();
    assert_eq!(streamed, eager.rules[0].violations);
    assert_eq!(summary.violation_count(), 4950);
    assert_eq!(summary.rules[0].counts, eager.rules[0].counts);
    let mut count = 0;
    let error = t
        .validate_into(|_, _| {
            count += 1;
            if count == 2 { Err("stop") } else { Ok(()) }
        })
        .unwrap_err();
    assert!(matches!(error, SinkError::Consumer("stop")));
    assert_eq!(count, 2);
    let snapshot = t.snapshot();
    t.clear_rules();
    assert_eq!(snapshot.validate().unwrap().violation_count(), 4950);
    assert_eq!(t.validate().unwrap().violation_count(), 0);
}

#[test]
fn every_rule_sink_matches_independent_eager_execution() {
    use topology_checker_core::{
        GeometryKind,
        geo::{Geometry, GeometryCollection, LineString, MultiLineString, MultiPoint, Rect, coord},
    };
    fn features(kind: GeometryKind) -> Vec<(i64, Geometry)> {
        [9, -4, 77]
            .into_iter()
            .enumerate()
            .map(|(i, fid)| {
                let x = i as f64;
                let point = Point::new(x, 0.5);
                let geometry: Geometry = match kind {
                    GeometryKind::Point => MultiPoint(vec![point, Point::new(x + 0.5, 0.5)]).into(),
                    GeometryKind::Line => MultiLineString(vec![
                        LineString::from(vec![(0., x), (3., 3. - x)]),
                        LineString::from(vec![(x, 0.), (x, 3.)]),
                    ])
                    .into(),
                    GeometryKind::Polygon => Rect::new(coord! {x:x,y:0.}, coord! {x:x+2.,y:2.})
                        .to_polygon()
                        .into(),
                    _ => Geometry::GeometryCollection(GeometryCollection(vec![
                        point.into(),
                        Rect::new(coord! {x:x,y:0.}, coord! {x:x+2.,y:2.})
                            .to_polygon()
                            .into(),
                    ])),
                };
                (fid, geometry)
            })
            .collect()
    }
    for rule in Rule::ALL {
        let mut t = Topology::new(0.001).unwrap();
        let (a, b) = rule.inputs();
        let a = t.add_layer(Layer::from_features("primary", features(a)));
        let b = b.map(|kind| t.add_layer(Layer::from_features("target", features(kind))));
        t.add_rule(*rule, a, b).unwrap();
        let eager = t.validate().unwrap();
        let mut streamed = Vec::new();
        t.validate_into(|_, v| {
            streamed.push(v);
            Ok::<_, &str>(())
        })
        .unwrap();
        assert_eq!(streamed, eager.rules[0].violations, "{rule}");
    }
}

#[test]
fn cancellation_leaves_no_partial_cache() {
    use topology_checker_core::{CancellationToken, Error, geo::LineString};
    let mut t = Topology::new(0.).unwrap();
    let layer = t.add_layer(Layer::from_geometries(
        "lines",
        (0..1000)
            .map(|i| LineString::from(vec![(i as f64 * 3., 0.), (i as f64 * 3. + 1., 1.)]).into()),
    ));
    t.add_rule(Rule::LineMustNotHaveDangles, layer, None)
        .unwrap();
    let token = CancellationToken::default();
    token.cancel();
    let error = t
        .validate_into_cancellable(token, |_, _| Ok::<_, &str>(()))
        .unwrap_err();
    assert!(matches!(error, SinkError::Core(Error::Cancelled)));
    assert_eq!(t.validate().unwrap().violation_count(), 2000);
}

#[test]
fn precision_failure_propagates_after_prior_rows_without_a_summary() {
    use topology_checker_core::{
        CancellationToken, Error,
        geo::{LineString, Polygon, Rect, coord},
    };
    let mut topology = Topology::new(0.).unwrap();
    let points = topology.add_layer(Layer::from_geometries(
        "duplicates",
        [Point::new(0., 0.).into(), Point::new(0., 0.).into()],
    ));
    topology
        .add_rule(Rule::PointMustNotOverlap, points, None)
        .unwrap();
    let x = 1f64.next_up();
    let polygons = topology.add_layer(Layer::from_features(
        "narrow",
        [
            (
                -17,
                Rect::new(coord! {x:1.,y:0.}, coord! {x:x,y:1.})
                    .to_polygon()
                    .into(),
            ),
            (
                82,
                Polygon::new(
                    LineString::from(vec![(1., -1.), (x, -1.), (1., 1.), (1., -1.)]),
                    vec![],
                )
                .into(),
            ),
        ],
    ));
    topology
        .add_rule(Rule::PolygonMustNotHaveGaps, polygons, None)
        .unwrap();
    for _ in 0..2 {
        assert!(matches!(
            topology.validate(),
            Err(Error::Precision {
                rule: Some(Rule::PolygonMustNotHaveGaps),
                ..
            })
        ));
        let mut count = 0;
        let result = topology.validate_into_cancellable(CancellationToken::default(), |_, _| {
            count += 1;
            Ok::<_, ()>(())
        });
        assert!(matches!(
            result,
            Err(SinkError::Core(Error::Precision { .. }))
        ));
        assert_eq!(count, 1);
    }
    topology.clear_rules();
    assert!(topology.validate().unwrap().is_valid());
}
