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

#[test]
fn line_meeting_sinks_match_eager_reports_on_nearby_linework() {
    use topology_checker_core::geo::{Geometry, LineString, MultiLineString};
    // Deterministic xorshift input: lines along, across and near one base line,
    // with vertices on it, closed rings and multipart features.
    let mut state = 0x2545_f491_4f6c_dd1d_u64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state >> 11) as f64 / (1_u64 << 53) as f64
    };
    for case in 0..150 {
        let tolerance = if case % 2 == 0 { 0.25 } else { 1. };
        let angle = next() * std::f64::consts::PI;
        let (ux, uy) = (angle.cos(), angle.sin());
        let origin = ((next() - 0.5) * 2000., (next() - 0.5) * 2000.);
        let at = |s: f64, d: f64| (origin.0 + s * ux - d * uy, origin.1 + s * uy + d * ux);
        let length = 6. + next() * 10.;
        let mut parts = vec![vec![at(0., 0.), at(length / 2., 0.), at(length, 0.)]];
        for _ in 0..2 + (next() * 3.) as usize {
            let mut s = next() * length - 2.;
            let mut part = Vec::new();
            for _ in 0..2 + (next() * 5.) as usize {
                let d = match (next() * 4.) as usize {
                    0 => 0.,
                    1 => (next() - 0.5) * 2. * tolerance,
                    2 => (next() - 0.5) * 4. * tolerance,
                    _ => (next() - 0.5) * 10.,
                };
                part.push(at(s, d));
                s += (next() - 0.25) * 4.;
            }
            if part.len() > 2 && next() < 0.25 {
                part.push(part[0]);
            }
            parts.push(part);
        }
        let mut features: Vec<Geometry> = Vec::new();
        let mut parts = parts.into_iter().map(LineString::from).peekable();
        while let Some(part) = parts.next() {
            features.push(match parts.peek() {
                Some(_) if next() < 0.3 => {
                    MultiLineString(vec![part, parts.next().expect("part")]).into()
                }
                _ => part.into(),
            });
        }
        for rule in [
            Rule::LineMustNotIntersect,
            Rule::LineMustNotIntersectWith,
            Rule::LineMustNotIntersectOrTouchInterior,
            Rule::LineMustNotIntersectOrTouchInteriorWith,
        ] {
            let mut t = Topology::new(tolerance).unwrap();
            let mut source = features
                .iter()
                .cloned()
                .enumerate()
                .map(|(i, g)| (7 - 3 * i as i64, g))
                .collect::<Vec<_>>();
            if rule.inputs().1.is_some() {
                let target = t.add_layer(Layer::from_features("target", source.split_off(1)));
                let source = t.add_layer(Layer::from_features("source", source));
                t.add_rule(rule, source, Some(target)).unwrap();
            } else {
                let layer = t.add_layer(Layer::from_features("lines", source));
                t.add_rule(rule, layer, None).unwrap();
            }
            let eager = t.validate().unwrap();
            let mut streamed = Vec::new();
            t.validate_into(|_, v| {
                streamed.push(v);
                Ok::<_, ()>(())
            })
            .unwrap();
            assert_eq!(streamed, eager.rules[0].violations, "case {case}, {rule}");
        }
    }
}

#[test]
fn panics_in_the_sink_propagate_unchanged() {
    use topology_checker_core::CancellationToken;
    let mut topology = Topology::new(0.).unwrap();
    let layer = topology.add_layer(Layer::from_geometries(
        "duplicates",
        [Point::new(0., 0.).into(), Point::new(0., 0.).into()],
    ));
    topology
        .add_rule(Rule::PointMustNotOverlap, layer, None)
        .unwrap();
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = topology
            .validate_into_cancellable(CancellationToken::default(), |_, _| -> Result<(), ()> {
                panic!("sink failed")
            });
    }))
    .unwrap_err();
    assert_eq!(panic.downcast_ref::<&str>(), Some(&"sink failed"));
}

fn mixed_topology() -> Topology {
    use topology_checker_core::geo::LineString;
    let mut t = Topology::new(0.).unwrap();
    let duplicates = t.add_layer(Layer::from_features(
        "duplicates",
        [5, -2, 9].map(|fid| (fid, Point::new(0., 0.).into())),
    ));
    let distinct = t.add_layer(Layer::from_geometries(
        "distinct",
        [Point::new(1., 0.).into(), Point::new(2., 0.).into()],
    ));
    let lines = t.add_layer(Layer::from_geometries(
        "lines",
        [LineString::from(vec![(0., 0.), (1., 1.)]).into()],
    ));
    t.add_rule(Rule::PointMustNotOverlap, duplicates, None)
        .unwrap();
    t.add_rule(Rule::PointMustNotOverlap, distinct, None)
        .unwrap();
    t.add_rule(Rule::LineMustNotHaveDangles, lines, None)
        .unwrap();
    let single = t.add_layer(Layer::from_geometries(
        "single",
        [Point::new(0., 0.).into()],
    ));
    t.add_rule(Rule::PointMustNotOverlapWith, duplicates, Some(single))
        .unwrap();
    t.add_rule(Rule::PointMustNotOverlapWith, distinct, Some(duplicates))
        .unwrap();
    t
}

#[test]
fn rule_end_follows_each_rules_violations_once_in_order() {
    use topology_checker_core::ValidationEvent;
    let t = mixed_topology();
    let eager = t.validate().unwrap();
    let mut ended = Vec::new();
    let mut pending = Vec::new();
    let summary = t
        .validate_events(|event| {
            match event {
                ValidationEvent::Violation(spec, v) => {
                    assert_eq!(spec.id.index(), ended.len());
                    pending.push(v);
                }
                ValidationEvent::RuleEnd(rule) => {
                    assert_eq!(rule.total, pending.len());
                    ended.push((rule.clone(), std::mem::take(&mut pending)));
                }
                _ => unreachable!(),
            }
            Ok::<_, ()>(())
        })
        .unwrap();
    assert!(pending.is_empty());
    assert_eq!(ended.len(), eager.rules.len());
    for (i, ((rule, violations), report)) in ended.iter().zip(&eager.rules).enumerate() {
        assert_eq!(rule.spec, report.spec);
        assert_eq!(rule.spec.id.index(), i);
        assert_eq!(rule.counts, report.counts);
        assert_eq!(rule.counts, summary.rules[i].counts);
        assert_eq!(rule.total, summary.rules[i].total);
        assert_eq!((&rule.layer, &rule.other), (&report.layer, &report.other));
        assert_eq!(violations, &report.violations);
    }
    let totals = ended.iter().map(|(r, _)| r.total).collect::<Vec<_>>();
    assert_eq!(totals, [3, 0, 2, 3, 0]);
}

#[test]
fn cancellation_inside_a_rule_stops_before_its_end_and_later_rules() {
    use topology_checker_core::{CancellationToken, Error, ValidationEvent};
    let t = mixed_topology();
    let token = CancellationToken::default();
    let mut log = Vec::new();
    let error = t
        .validate_events_cancellable(token.clone(), |event| {
            match event {
                ValidationEvent::Violation(spec, _) => {
                    log.push((spec.id.index(), false));
                    if spec.id.index() == 2 {
                        token.cancel();
                    }
                }
                ValidationEvent::RuleEnd(rule) => log.push((rule.spec.id.index(), true)),
                _ => unreachable!(),
            }
            Ok::<_, ()>(())
        })
        .unwrap_err();
    assert!(matches!(error, SinkError::Core(Error::Cancelled)));
    assert_eq!(
        log,
        [
            (0, false),
            (0, false),
            (0, false),
            (0, true),
            (1, true),
            (2, false)
        ]
    );
    let token = CancellationToken::default();
    let mut ends = 0;
    let error = t
        .validate_events_cancellable(token.clone(), |event| {
            match event {
                ValidationEvent::RuleEnd(_) => {
                    ends += 1;
                    token.cancel();
                }
                _ => assert_eq!(ends, 0),
            }
            Ok::<_, ()>(())
        })
        .unwrap_err();
    assert!(matches!(error, SinkError::Core(Error::Cancelled)));
    assert_eq!(ends, 1);
    let mut ends = 0;
    let error = t
        .validate_events(|event| {
            if matches!(event, ValidationEvent::RuleEnd(_)) {
                ends += 1;
                return Err("stop");
            }
            assert_eq!(ends, 0);
            Ok(())
        })
        .unwrap_err();
    assert!(matches!(error, SinkError::Consumer("stop")));
    assert_eq!(ends, 1);
    assert_eq!(t.validate().unwrap().violation_count(), 8);
}
