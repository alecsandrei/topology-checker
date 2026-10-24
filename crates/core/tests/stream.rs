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

/// Three overlapping features of one family, so most rules report something.
fn rule_features(
    kind: topology_checker_core::GeometryKind,
) -> Vec<(i64, topology_checker_core::geo::Geometry)> {
    use topology_checker_core::{
        GeometryKind,
        geo::{Geometry, GeometryCollection, LineString, MultiLineString, MultiPoint, Rect, coord},
    };
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

#[test]
fn every_rule_sink_matches_independent_eager_execution() {
    let features = rule_features;
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

/// One event as seen by a sink, owned so whole runs can be compared.
#[derive(Debug, PartialEq)]
enum Logged {
    Violation(usize, topology_checker_core::Violation),
    End(
        usize,
        std::collections::BTreeMap<topology_checker_core::ViolationKind, usize>,
        usize,
    ),
}

/// Every rule once, in registry order, over shared layers of each family.
fn every_rule_topology() -> Topology {
    use topology_checker_core::GeometryKind;
    let mut t = Topology::new(0.001).unwrap();
    let kinds = [
        GeometryKind::Point,
        GeometryKind::Line,
        GeometryKind::Polygon,
        GeometryKind::Any,
    ];
    let layers = kinds.map(|kind| {
        let primary = t.add_layer(Layer::from_features("primary", rule_features(kind)));
        let target = t.add_layer(Layer::from_features("target", rule_features(kind)));
        (kind, primary, target)
    });
    let find = |kind| *layers.iter().find(|(k, ..)| *k == kind).unwrap();
    for &rule in Rule::ALL {
        let (a, b) = rule.inputs();
        t.add_rule(rule, find(a).1, b.map(|kind| find(kind).2))
            .unwrap();
    }
    t
}

fn log_events(
    t: &Topology,
    token: topology_checker_core::CancellationToken,
    mut stop: impl FnMut(&Logged) -> bool,
) -> (
    Vec<Logged>,
    Result<topology_checker_core::ReportSummary, SinkError<()>>,
) {
    use topology_checker_core::ValidationEvent;
    let mut log = Vec::new();
    let result = t.validate_events_cancellable(token.clone(), |event| {
        log.push(match event {
            ValidationEvent::Violation(spec, v) => Logged::Violation(spec.id.index(), v),
            ValidationEvent::RuleEnd(rule) => {
                Logged::End(rule.spec.id.index(), rule.counts.clone(), rule.total)
            }
            _ => unreachable!(),
        });
        if stop(log.last().unwrap()) {
            token.cancel();
        }
        Ok(())
    });
    (log, result)
}

/// The events eager validation implies: each rule's violations, then its end.
fn expected_events(t: &Topology) -> Vec<Logged> {
    let eager = t.validate().unwrap();
    let mut expected = Vec::new();
    for (i, report) in eager.rules.iter().enumerate() {
        expected.extend(
            report
                .violations
                .iter()
                .map(|v| Logged::Violation(i, v.clone())),
        );
        expected.push(Logged::End(
            i,
            report.counts.clone(),
            report.violations.len(),
        ));
    }
    expected
}

#[test]
fn rule_end_matches_eager_reports_for_every_rule_alone_and_together() {
    use topology_checker_core::CancellationToken;
    for &rule in Rule::ALL {
        let mut t = Topology::new(0.001).unwrap();
        let (a, b) = rule.inputs();
        let a = t.add_layer(Layer::from_features("primary", rule_features(a)));
        let b = b.map(|kind| t.add_layer(Layer::from_features("target", rule_features(kind))));
        t.add_rule(rule, a, b).unwrap();
        let (log, result) = log_events(&t, CancellationToken::default(), |_| false);
        assert_eq!(log, expected_events(&t), "{rule}");
        let summary = result.unwrap();
        assert!(matches!(log.last(), Some(Logged::End(0, _, n)) if *n == summary.rules[0].total));
    }
    let t = every_rule_topology();
    let expected = expected_events(&t);
    let (log, result) = log_events(&t, CancellationToken::default(), |_| false);
    assert_eq!(log, expected);
    let summary = result.unwrap();
    let ends = log
        .iter()
        .filter_map(|e| match e {
            Logged::End(i, counts, total) => Some((*i, counts, *total)),
            Logged::Violation(..) => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(ends.len(), Rule::ALL.len());
    for (i, (index, counts, total)) in ends.into_iter().enumerate() {
        assert_eq!(index, i);
        assert_eq!(counts, &summary.rules[i].counts);
        assert_eq!(total, summary.rules[i].total);
    }
    // The fixture must exercise both passing and failing rules.
    assert!(summary.rules.iter().any(|r| r.total == 0));
    assert!(summary.rules.iter().any(|r| r.total > 1));
}

#[test]
fn cancelling_at_each_rule_end_stops_before_the_next_rule() {
    use topology_checker_core::{CancellationToken, Error};
    let t = every_rule_topology();
    let expected = expected_events(&t);
    let last = Rule::ALL.len() - 1;
    for k in 0..=last {
        let (log, result) = log_events(
            &t,
            CancellationToken::default(),
            |e| matches!(e, Logged::End(i, ..) if *i == k),
        );
        assert_eq!(log[..], expected[..log.len()], "rule {k}");
        assert!(matches!(log.last(), Some(Logged::End(i, ..)) if *i == k));
        if k == last {
            // Nothing is left to cancel after the final rule.
            assert!(result.is_ok());
        } else {
            assert!(
                matches!(result, Err(SinkError::Core(Error::Cancelled))),
                "rule {k}"
            );
        }
    }
    let token = CancellationToken::default();
    token.cancel();
    let (log, result) = log_events(&t, token, |_| false);
    assert!(log.is_empty());
    assert!(matches!(result, Err(SinkError::Core(Error::Cancelled))));
}

#[test]
fn cancelling_inside_each_rule_drops_its_end_and_every_later_rule() {
    use topology_checker_core::{CancellationToken, Error};
    let t = every_rule_topology();
    let expected = expected_events(&t);
    let mut cancelled = 0;
    for (position, event) in expected.iter().enumerate() {
        let Logged::Violation(rule, _) = event else {
            continue;
        };
        let mut seen = 0;
        let (log, result) = log_events(&t, CancellationToken::default(), |_| {
            seen += 1;
            seen == position + 1
        });
        assert_eq!(log[..], expected[..=position], "rule {rule}");
        assert!(matches!(result, Err(SinkError::Core(Error::Cancelled))));
        cancelled += 1;
    }
    assert!(cancelled > Rule::ALL.len());
}

/// Line features that report every kind of the per-feature line rules, spread
/// over more features than one parallel batch holds.
fn multi_kind_topology() -> Topology {
    use topology_checker_core::geo::{LineString, MultiLineString};
    let mut t = Topology::new(0.).unwrap();
    let features = (0..40).map(|i: i64| {
        let row = (i / 2) as f64;
        let x = (i % 10) as f64 + 0.5;
        let geometry = MultiLineString(vec![
            // Pairs of features share their row.
            LineString::from(vec![(0., row), (10., row)]),
            // Crosses its own row and the next one.
            LineString::from(vec![(x, row - 0.5), (x, row + 1.5)]),
            // Ends inside the row after next.
            LineString::from(vec![(x - 0.25, row + 2.), (x - 0.1, row + 2.5)]),
            // Retraces part of its own row.
            LineString::from(vec![(x - 0.4, row), (x - 0.3, row)]),
        ]);
        // FID order differs from insertion order.
        (if i % 2 == 0 { i } else { -i }, geometry.into())
    });
    let layer = t.add_layer(Layer::from_features("multi", features));
    for rule in [
        Rule::LineMustNotSelfIntersect,
        Rule::LineMustNotIntersect,
        Rule::LineMustNotIntersectOrTouchInterior,
    ] {
        t.add_rule(rule, layer, None).unwrap();
    }
    t
}

#[test]
fn rule_end_follows_later_kinds_of_parallel_feature_batches() {
    use std::collections::BTreeSet;
    use topology_checker_core::{CancellationToken, Error, ViolationKind};
    let t = multi_kind_topology();
    let expected = expected_events(&t);
    let eager = t.validate().unwrap();
    let kinds: [&[ViolationKind]; 3] = [
        &[ViolationKind::SelfOverlap, ViolationKind::SelfCrossing],
        &[ViolationKind::Overlap, ViolationKind::Crossing],
        &[
            ViolationKind::Overlap,
            ViolationKind::Crossing,
            ViolationKind::TouchInterior,
        ],
    ];
    // Each kind comes from more features than a one-thread batch of eight holds.
    for (report, kinds) in eager.rules.iter().zip(kinds) {
        for &kind in kinds {
            let features = report
                .violations
                .iter()
                .filter(|v| v.kind == kind)
                .map(|v| v.origin.unwrap().fid)
                .collect::<BTreeSet<_>>();
            assert!(features.len() > 8, "{} {kind:?}", report.spec.rule);
        }
    }
    // Cancel at the first and last violation of each kind of each rule.
    let mut marks = BTreeSet::new();
    for (i, event) in expected.iter().enumerate() {
        let Logged::Violation(rule, v) = event else {
            continue;
        };
        let same =
            |e: &Logged| matches!(e, Logged::Violation(r, w) if r == rule && w.kind == v.kind);
        if i == 0 || !same(&expected[i - 1]) {
            marks.insert(i);
        }
        if !same(&expected[i + 1]) {
            marks.insert(i);
        }
    }
    assert_eq!(marks.len(), 14);
    for threads in [1, 2, 4] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap();
        pool.install(|| {
            let (log, result) = log_events(&t, CancellationToken::default(), |_| false);
            assert_eq!(log, expected, "{threads} threads");
            result.unwrap();
            for &position in &marks {
                let mut seen = 0;
                let (log, result) = log_events(&t, CancellationToken::default(), |_| {
                    seen += 1;
                    seen == position + 1
                });
                assert_eq!(log[..], expected[..=position], "{threads} threads");
                assert!(matches!(result, Err(SinkError::Core(Error::Cancelled))));
            }
        });
    }
}

mod thread_count {
    use proptest::prelude::*;
    use std::convert::Infallible;
    use topology_checker_core::{
        GeometryKind, Layer, Rule, Topology,
        geo::{Geometry, LineString, MultiLineString, coord},
    };

    /// Multipart lines on a small grid, so features share vertices, segments and
    /// whole parts, and some parts are closed or have zero-length segments.
    fn line() -> impl Strategy<Value = Vec<Vec<(i32, i32)>>> {
        prop::collection::vec(prop::collection::vec((-3i32..4, -3i32..4), 2..5), 1..4)
    }
    fn geometry(parts: &[Vec<(i32, i32)>], offset: f64) -> Geometry {
        let part = |part: &Vec<(i32, i32)>| {
            LineString(
                part.iter()
                    .map(|&(x, y)| coord! { x: offset + x as f64, y: offset + y as f64 })
                    .collect(),
            )
        };
        MultiLineString(parts.iter().map(part).collect()).into()
    }
    fn streamed(t: &Topology, threads: Option<usize>) -> String {
        let run = || {
            let mut violations = Vec::new();
            let summary = t.validate_into(|spec, v| {
                violations.push((spec, v));
                Ok::<_, Infallible>(())
            });
            format!("{violations:?} {summary:?}")
        };
        match threads {
            Some(n) => rayon::ThreadPoolBuilder::new()
                .num_threads(n)
                .build()
                .unwrap()
                .install(run),
            None => run(),
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]
        #[test]
        fn line_rule_streams_do_not_depend_on_the_thread_count(
            a in prop::collection::vec(line(), 0..14),
            b in prop::collection::vec(line(), 0..6),
            copies in 0usize..4,
            tolerance in prop_oneof![Just(0.), Just(0.01), Just(0.6), Just(1.5)],
            offset in prop_oneof![Just(0.), Just(-250.), Just(1e6), Just(1e9)],
        ) {
            let mut t = Topology::new(tolerance).unwrap();
            let features = |lines: &[Vec<Vec<(i32, i32)>>], step: i64| {
                lines
                    .iter()
                    .enumerate()
                    .map(|(i, parts)| (i as i64 * step, geometry(parts, offset)))
                    .collect::<Vec<_>>()
            };
            let la = t.add_layer(Layer::from_features("a", features(&a, -3)));
            // The other layer repeats a few features of the first one.
            let mut other = a.iter().take(copies).cloned().collect::<Vec<_>>();
            other.extend(b);
            let lb = t.add_layer(Layer::from_features("b", features(&other, 7)));
            for &rule in Rule::ALL {
                match rule.inputs() {
                    (GeometryKind::Line, None) => t.add_rule(rule, la, None).unwrap(),
                    (GeometryKind::Line, Some(GeometryKind::Line)) => {
                        t.add_rule(rule, la, Some(lb)).unwrap()
                    }
                    _ => continue,
                };
            }
            let expected = streamed(&t, Some(1));
            prop_assert!(expected.contains("Ok("), "{expected}");
            for threads in [Some(2), Some(4), Some(4), None] {
                prop_assert_eq!(&streamed(&t, threads), &expected);
            }
        }
    }
}
