//! Canonical rule contracts: hand-specified scenarios per rule, execution modes and invariances.
#[path = "support/coverage_gaps.rs"]
mod coverage_gaps;
#[path = "support/rule_cases.rs"]
mod support;
use support::*;
use topology_checker_core::{
    Rule, ViolationKind as K,
    geo::{Geometry, LineString, Polygon},
};

fn point_duplicates(rule: Rule) -> Vec<Case> {
    let binary = rule.inputs().1.is_some();
    let mut out = Vec::new();
    let rows = vec![
        ("separated", p(0., 0.), p(2., 0.), 0., vec![]),
        ("diagonal-separated", p(0., 0.), p(1., 1.), 0., vec![]),
        (
            "coincident",
            p(1., 1.),
            p(1., 1.),
            0.,
            vec![pair(K::Overlap, p(1., 1.), binary)],
        ),
        (
            "nearby",
            p(0., 0.),
            p(0.125, 0.),
            0.25,
            vec![pair(K::Overlap, p(0., 0.), binary)],
        ),
        (
            "below-distance",
            p(0., 0.),
            p(0.25_f64.next_down(), 0.),
            0.25,
            vec![pair(K::Overlap, p(0., 0.), binary)],
        ),
        (
            "at-distance",
            p(0., 0.),
            p(0.25, 0.),
            0.25,
            vec![pair(K::Overlap, p(0., 0.), binary)],
        ),
        (
            "above-distance",
            p(0., 0.),
            p(0.25_f64.next_up(), 0.),
            0.25,
            vec![],
        ),
        (
            "zero-tolerance-neighbor",
            p(0., 0.),
            p(f64::MIN_POSITIVE, 0.),
            0.,
            vec![],
        ),
        ("empty-multipoint", mp(&[]), p(0., 0.), 0., vec![]),
        (
            "single-part-multi",
            mp(&[(0., 0.)]),
            p(0., 0.),
            0.,
            vec![pair(K::Overlap, p(0., 0.), binary)],
        ),
        (
            "partial-multi",
            mp(&[(0., 0.), (2., 0.)]),
            p(2., 0.),
            0.,
            vec![e(
                K::Overlap,
                p(2., 0.),
                Some((0, A, Some(1))),
                Some((usize::from(binary), if binary { A } else { B }, Some(0))),
            )],
        ),
        (
            "both-parts-contribute",
            mp(&[(0., 0.), (2., 0.)]),
            mp(&[(0., 0.), (2., 0.)]),
            0.,
            vec![e(
                K::Overlap,
                mp(&[(0., 0.), (2., 0.)]),
                Some((0, A, None)),
                Some((usize::from(binary), if binary { A } else { B }, None)),
            )],
        ),
        (
            "duplicate-source-coordinates",
            mp(&[(0., 0.), (0., 0.)]),
            p(0., 0.),
            0.,
            vec![e(
                K::Overlap,
                p(0., 0.),
                Some((0, A, None)),
                Some((usize::from(binary), if binary { A } else { B }, Some(0))),
            )],
        ),
        (
            "nested-point-parts",
            collection(vec![mp(&[]), collection(vec![p(0., 0.)])]),
            p(0., 0.),
            0.,
            vec![pair(K::Overlap, p(0., 0.), binary)],
        ),
        // Inside the tolerance envelope, but 0.265 apart.
        (
            "box-corner-beyond-radius",
            p(0., 0.),
            p(0.1875, 0.1875),
            0.25,
            vec![],
        ),
    ];
    for (name, a, b, t, expected) in rows {
        out.push(if binary {
            case(rule, name, vec![a], vec![b], t, expected)
        } else {
            case(rule, name, vec![a, b], vec![], t, expected)
        });
    }
    out.push(case(rule, "empty-input", vec![], vec![], 0., vec![]));
    if binary {
        out.push(case(
            rule,
            "empty-target",
            vec![p(0., 0.)],
            vec![],
            0.,
            vec![],
        ));
        out.push(case(
            rule,
            "one-source-two-targets",
            vec![p(0., 0.)],
            vec![p(0., 0.), p(0.125, 0.)],
            0.25,
            vec![
                pair(K::Overlap, p(0., 0.), true),
                e(
                    K::Overlap,
                    p(0., 0.),
                    Some((0, A, Some(0))),
                    Some((1, A + 1, Some(0))),
                ),
            ],
        ));
    } else {
        out.push(case(
            rule,
            "three-pair-clique",
            vec![p(0., 0.); 3],
            vec![],
            0.,
            vec![
                pair(K::Overlap, p(0., 0.), false),
                e(
                    K::Overlap,
                    p(0., 0.),
                    Some((0, A, Some(0))),
                    Some((0, B + 1, Some(0))),
                ),
                e(
                    K::Overlap,
                    p(0., 0.),
                    Some((0, B, Some(0))),
                    Some((0, B + 1, Some(0))),
                ),
            ],
        ));
    }
    out
}

fn point_relations(rule: Rule) -> Vec<Case> {
    let line_target = matches!(
        rule,
        Rule::PointMustBeCoveredByLines
            | Rule::PointMustBeCoveredByLineEndpoints
            | Rule::PointMustNotIntersectLines
    );
    let endpoints = rule == Rule::PointMustBeCoveredByLineEndpoints;
    let target = if line_target {
        l(&[(0., 0.), (8., 0.)])
    } else {
        square()
    };
    // (strict interior, area including boundary, boundary, line interior, line endpoint)
    let rows = vec![
        ("interior", p(4., 4.), true, true, false, false, false, 0.),
        ("outside", p(12., 4.), false, false, false, false, false, 0.),
        ("edge", p(4., 0.), false, true, true, true, false, 0.),
        ("vertex", p(0., 0.), false, true, true, false, true, 0.),
        (
            "far-endpoint",
            p(8., 0.),
            false,
            true,
            true,
            false,
            true,
            0.,
        ),
        (
            "hole-interior",
            p(4., 4.),
            false,
            false,
            false,
            false,
            false,
            0.,
        ),
        ("hole-edge", p(2., 4.), false, true, true, false, false, 0.),
        (
            "inside-below-threshold",
            p(0.25_f64.next_down(), 4.),
            false,
            true,
            true,
            false,
            false,
            0.25,
        ),
        (
            "inside-at-threshold",
            p(0.25, 4.),
            false,
            true,
            true,
            false,
            false,
            0.25,
        ),
        (
            "inside-above-threshold",
            p(0.25_f64.next_up(), 4.),
            true,
            true,
            false,
            false,
            false,
            0.25,
        ),
        (
            "outside-below-threshold",
            p(-0.25_f64.next_down(), 0.),
            false,
            true,
            true,
            false,
            true,
            0.25,
        ),
        (
            "outside-at-threshold",
            p(-0.25, 0.),
            false,
            true,
            true,
            false,
            true,
            0.25,
        ),
        (
            "outside-above-threshold",
            p(-0.25_f64.next_up(), 0.),
            false,
            false,
            false,
            false,
            false,
            0.25,
        ),
        (
            "line-below-threshold",
            p(4., 0.25_f64.next_down()),
            false,
            true,
            true,
            true,
            false,
            0.25,
        ),
        (
            "line-at-threshold",
            p(4., 0.25),
            false,
            true,
            true,
            true,
            false,
            0.25,
        ),
        (
            "line-above-threshold",
            p(4., 0.25_f64.next_up()),
            true,
            true,
            false,
            false,
            false,
            0.25,
        ),
    ];
    let mut out = vec![];
    for (mut name, mut g, strict, area, boundary, on_line, mut on_endpoint, t) in rows {
        // Interior, hole and clearance rows only place a point far from a line target.
        // Line-distance rows repeat the clearance rows on another polygon edge.
        if (line_target
            && (name == "interior" || name.starts_with("inside-") || name.starts_with("hole-")))
            || (!line_target && name.starts_with("line-"))
        {
            continue;
        }
        if endpoints && name.starts_with("line-") {
            g = p(
                0.,
                -match name {
                    "line-below-threshold" => 0.25_f64.next_down(),
                    "line-at-threshold" => 0.25,
                    _ => 0.25_f64.next_up(),
                },
            );
            on_endpoint = name != "line-above-threshold";
            name = match name {
                "line-below-threshold" => "endpoint-normal-below",
                "line-at-threshold" => "endpoint-normal-at",
                _ => "endpoint-normal-above",
            };
        }
        let hit = match rule {
            Rule::PointMustBeInside => !strict,
            Rule::PointMustBeCoveredByPolygons => !area,
            Rule::PointMustBeCoveredByPolygonBoundaries => !boundary,
            Rule::PointMustNotBeInsidePolygons => strict,
            Rule::PointMustNotIntersectPolygons => area,
            Rule::PointMustBeCoveredByLines => !(on_line || on_endpoint),
            Rule::PointMustBeCoveredByLineEndpoints => !on_endpoint,
            Rule::PointMustNotIntersectLines => on_line || on_endpoint,
            _ => unreachable!(),
        };
        let kind = match rule {
            Rule::PointMustBeInside => K::Outside,
            Rule::PointMustNotBeInsidePolygons => K::Contained,
            Rule::PointMustNotIntersectLines | Rule::PointMustNotIntersectPolygons => {
                K::Intersection
            }
            _ => K::NotCovered,
        };
        let prohibited = matches!(kind, K::Contained | K::Intersection);
        let expected = if hit {
            vec![if prohibited {
                pair(kind, g.clone(), true)
            } else {
                one(kind, g.clone())
            }]
        } else {
            vec![]
        };
        let b = if name.starts_with("hole-") {
            hole()
        } else {
            target.clone()
        };
        out.push(case(rule, name, vec![g], vec![b], t, expected));
    }
    let required = !matches!(
        rule,
        Rule::PointMustNotIntersectLines
            | Rule::PointMustNotIntersectPolygons
            | Rule::PointMustNotBeInsidePolygons
    );
    out.push(case(
        rule,
        "empty-input",
        vec![],
        vec![target.clone()],
        0.,
        vec![],
    ));
    out.push(case(
        rule,
        "empty-target",
        vec![p(0., 0.)],
        vec![],
        0.,
        if required {
            vec![one(
                if rule == Rule::PointMustBeInside {
                    K::Outside
                } else {
                    K::NotCovered
                },
                p(0., 0.),
            )]
        } else {
            vec![]
        },
    ));
    out.push(case(
        rule,
        "empty-multi",
        vec![mp(&[])],
        vec![target.clone()],
        0.,
        vec![],
    ));
    if line_target {
        let hit = if endpoints {
            true
        } else {
            rule == Rule::PointMustNotIntersectLines
        };
        out.push(case(
            rule,
            "closed-target-has-no-endpoints",
            vec![p(0., 0.)],
            vec![perimeter(&square())],
            0.,
            if hit {
                vec![if endpoints {
                    one(K::NotCovered, p(0., 0.))
                } else {
                    pair(K::Intersection, p(0., 0.), true)
                }]
            } else {
                vec![]
            },
        ));
        // Interior of the first line, endpoint of the second.
        out.push(case(
            rule,
            "two-line-contact",
            vec![p(4., 0.)],
            vec![l(&[(0., 0.), (8., 0.)]), l(&[(4., 0.), (4., 4.)])],
            0.,
            if rule == Rule::PointMustNotIntersectLines {
                vec![
                    pair(K::Intersection, p(4., 0.), true),
                    e(
                        K::Intersection,
                        p(4., 0.),
                        Some((0, A, Some(0))),
                        Some((1, A + 1, Some(0))),
                    ),
                ]
            } else {
                vec![]
            },
        ));
    } else {
        let strict = matches!(rule, Rule::PointMustBeInside);
        out.push(case(
            rule,
            "concave-notch",
            vec![p(4., 4.)],
            vec![concave()],
            0.,
            if strict
                || rule == Rule::PointMustBeCoveredByPolygons
                || rule == Rule::PointMustBeCoveredByPolygonBoundaries
            {
                vec![one(
                    if strict { K::Outside } else { K::NotCovered },
                    p(4., 4.),
                )]
            } else {
                vec![]
            },
        ));
    }
    out
}

fn line_pairs(rule: Rule) -> Vec<Case> {
    let binary = rule.inputs().1.is_some();
    let touches = matches!(
        rule,
        Rule::LineMustNotIntersectOrTouchInterior | Rule::LineMustNotIntersectOrTouchInteriorWith
    );
    let overlap_only = matches!(
        rule,
        Rule::LineMustNotOverlap | Rule::LineMustNotOverlapWith
    );
    let base = l(&[(0., 0.), (8., 0.)]);
    let rows = vec![
        (
            "disjoint",
            base.clone(),
            l(&[(0., 2.), (8., 2.)]),
            None,
            None,
            0.,
        ),
        (
            "endpoint-endpoint",
            base.clone(),
            l(&[(8., 0.), (8., 4.)]),
            None,
            None,
            0.,
        ),
        (
            "identical",
            base.clone(),
            base.clone(),
            Some(K::Overlap),
            Some(base.clone()),
            0.,
        ),
        (
            "partial-overlap",
            base.clone(),
            l(&[(4., 0.), (12., 0.)]),
            Some(K::Overlap),
            Some(l(&[(4., 0.), (8., 0.)])),
            0.,
        ),
        (
            "interior-crossing",
            base.clone(),
            l(&[(4., -4.), (4., 4.)]),
            Some(K::Crossing),
            Some(p(4., 0.)),
            0.,
        ),
        (
            "t-junction",
            base.clone(),
            l(&[(4., 0.), (4., 4.)]),
            Some(K::TouchInterior),
            Some(p(4., 0.)),
            0.,
        ),
        (
            "vertex-crossing",
            l(&[(0., 0.), (4., 0.), (8., 0.)]),
            l(&[(4., -4.), (4., 0.), (4., 4.)]),
            Some(K::Crossing),
            Some(p(4., 0.)),
            0.,
        ),
        (
            "reversed-shared-path",
            base.clone(),
            l(&[(8., 0.), (0., 0.)]),
            Some(K::Overlap),
            Some(base.clone()),
            0.,
        ),
        (
            "unequal-segmentation",
            base.clone(),
            l(&[(0., 0.), (2., 0.), (6., 0.), (8., 0.)]),
            Some(K::Overlap),
            Some(base.clone()),
            0.,
        ),
        (
            "bent-shared-path",
            l(&[(0., 0.), (4., 0.), (4., 4.)]),
            l(&[(2., 0.), (4., 0.), (4., 2.)]),
            Some(K::Overlap),
            Some(l(&[(2., 0.), (4., 0.), (4., 2.)])),
            0.,
        ),
        (
            "overlapping-envelopes-disjoint",
            l(&[(0., 0.), (0., 4.), (4., 4.)]),
            l(&[(1., 1.), (3., 1.), (3., 3.)]),
            None,
            None,
            0.,
        ),
        (
            "repeated-coordinate",
            l(&[(0., 0.), (0., 0.), (8., 0.)]),
            base.clone(),
            Some(K::Overlap),
            Some(base.clone()),
            0.,
        ),
        ("empty-line", l(&[]), base.clone(), None, None, 0.),
        (
            "collapsed-line",
            l(&[(0., 0.), (0., 0.)]),
            base.clone(),
            None,
            None,
            0.,
        ),
        (
            "isolated-interior-tangent",
            base.clone(),
            l(&[(2., 2.), (4., 0.), (6., 2.)]),
            Some(K::Crossing),
            Some(p(4., 0.)),
            0.,
        ),
    ];
    let mut out = vec![];
    for (name, a, b, kind, g, t) in rows {
        let expected = match kind {
            Some(k) if k == K::Overlap || (!overlap_only && (k == K::Crossing || touches)) => {
                vec![pair(k, g.unwrap(), binary)]
            }
            _ => vec![],
        };
        out.push(if binary {
            case(rule, name, vec![a], vec![b], t, expected)
        } else {
            case(rule, name, vec![a, b], vec![], t, expected)
        });
    }
    for (name, length) in [
        ("collapsed-input-below", 0.25_f64.next_down()),
        ("collapsed-input-at", 0.25),
        ("collapsed-input-above-control", 0.25_f64.next_up()),
    ] {
        let a = l(&[(0., 0.), (length, 0.)]);
        let expected = if length > 0.25 {
            vec![pair(K::Overlap, a.clone(), binary)]
        } else {
            vec![]
        };
        out.push(if binary {
            case(rule, name, vec![a.clone()], vec![a], 0.25, expected)
        } else {
            case(rule, name, vec![a.clone(), a], vec![], 0.25, expected)
        });
    }
    let multi = ml(vec![base.clone(), l(&[(0., 4.), (8., 4.)])]);
    let expected = vec![e(
        K::Overlap,
        multi.clone(),
        Some((0, A, None)),
        Some((usize::from(binary), if binary { A } else { B }, None)),
    )];
    out.push(if binary {
        case(
            rule,
            "multipart-shared",
            vec![multi.clone()],
            vec![multi],
            0.,
            expected,
        )
    } else {
        case(
            rule,
            "multipart-shared",
            vec![multi.clone(), multi],
            vec![],
            0.,
            expected,
        )
    });
    // The first feature is A; the others follow it in one layer or form the second layer.
    let other = |k: i64| {
        Some(if binary {
            (1, A + k, Some(0))
        } else {
            (0, B + k, Some(0))
        })
    };
    let mut push = |name, a: Geometry, others: Vec<Geometry>, t, expected| {
        out.push(if binary {
            case(rule, name, vec![a], others, t, expected)
        } else {
            let mut all = vec![a];
            all.extend(others);
            case(rule, name, all, vec![], t, expected)
        });
    };
    push(
        "later-part-shared",
        ml(vec![l(&[(0., 20.), (8., 20.)]), base.clone()]),
        vec![base.clone()],
        0.,
        vec![e(K::Overlap, base.clone(), Some((0, A, Some(1))), other(0))],
    );
    // Shares (0, 0) to (2, 0), then crosses back through (5, 0).
    let mut expected = vec![pair(K::Overlap, l(&[(0., 0.), (2., 0.)]), binary)];
    if !overlap_only {
        expected.push(pair(K::Crossing, p(5., 0.), binary));
    }
    push(
        "overlap-then-crossing",
        base.clone(),
        vec![l(&[(0., 0.), (2., 0.), (4., 2.), (6., -2.)])],
        0.,
        expected,
    );
    if !overlap_only {
        for (name, d) in [
            ("touch-within-below", 0.25_f64.next_down()),
            ("touch-within-at", 0.25),
            ("touch-within-above", 0.25_f64.next_up()),
        ] {
            push(
                name,
                base.clone(),
                vec![l(&[(4., d), (4., 4.)])],
                0.25,
                if touches && d <= 0.25 {
                    vec![pair(K::TouchInterior, p(4., d), binary)]
                } else {
                    vec![]
                },
            );
        }
        // Two endpoint touches do not make a crossing.
        push(
            "three-feature-node",
            base.clone(),
            vec![l(&[(4., 0.), (4., 4.)]), l(&[(4., 0.), (4., -4.)])],
            0.,
            if touches {
                vec![
                    pair(K::TouchInterior, p(4., 0.), binary),
                    e(K::TouchInterior, p(4., 0.), Some((0, A, Some(0))), other(1)),
                ]
            } else {
                vec![]
            },
        );
    }
    out.push(case(rule, "empty-input", vec![], vec![], 0., vec![]));
    if binary {
        out.push(case(
            rule,
            "empty-target",
            vec![base.clone()],
            vec![],
            0.,
            vec![],
        ));
        out.push(case(
            rule,
            "same-layer-pairs-ignored",
            vec![base.clone(), base, l(&[(4., -4.), (4., 4.)])],
            vec![l(&[(12., 0.), (16., 0.)])],
            0.,
            vec![],
        ));
    } else {
        for (name, g) in [
            ("self-retrace-excluded", l(&[(0., 0.), (4., 0.), (0., 0.)])),
            (
                "self-crossing-excluded",
                l(&[(0., 0.), (4., 4.), (0., 4.), (4., 0.)]),
            ),
        ] {
            out.push(case(rule, name, vec![g], vec![], 0., vec![]));
        }
    }
    if rule == Rule::LineMustNotOverlap {
        let shared = l(&[(0., 0.), (4., 0.)]);
        out.push(case(
            rule,
            "retrace-plus-other-feature",
            vec![l(&[(0., 0.), (4., 0.), (0., 0.)]), shared.clone()],
            vec![],
            0.,
            vec![pair(K::Overlap, shared, false)],
        ));
    }
    out
}
fn self_lines(rule: Rule) -> Vec<Case> {
    let crossing = rule == Rule::LineMustNotSelfIntersect;
    let mut out = vec![];
    let rows = vec![
        ("simple-segment", l(&[(0., 0.), (4., 0.)]), vec![]),
        ("simple-bend", l(&[(0., 0.), (4., 0.), (4., 4.)]), vec![]),
        (
            "retraced-segment",
            l(&[(0., 0.), (4., 0.), (0., 0.)]),
            vec![one(K::SelfOverlap, l(&[(0., 0.), (4., 0.)]))],
        ),
        (
            "partial-retrace",
            l(&[(0., 0.), (4., 0.), (2., 0.)]),
            vec![one(K::SelfOverlap, l(&[(2., 0.), (4., 0.)]))],
        ),
        ("closed-loop", perimeter(&square()), vec![]),
        (
            "self-crossing",
            l(&[(0., 0.), (4., 4.), (0., 4.), (4., 0.)]),
            if crossing {
                vec![one(K::SelfCrossing, p(2., 2.))]
            } else {
                vec![]
            },
        ),
        (
            "self-touch",
            l(&[(0., 0.), (2., 2.), (4., 0.), (2., 2.), (2., 4.)]),
            vec![one(K::SelfOverlap, l(&[(2., 2.), (4., 0.)]))],
        ),
        (
            "endpoint-on-interior",
            l(&[(2., 0.), (2., 2.), (0., 2.), (0., 0.), (4., 0.)]),
            vec![],
        ),
        (
            "repeated-vertex",
            l(&[(0., 0.), (0., 0.), (4., 0.)]),
            vec![],
        ),
        ("empty-line", l(&[]), vec![]),
        ("collapsed-line", l(&[(0., 0.), (0., 0.)]), vec![]),
        ("single-multi", ml(vec![l(&[(0., 0.), (4., 0.)])]), vec![]),
        (
            "multipart-retrace",
            ml(vec![l(&[(0., 0.), (4., 0.)]), l(&[(4., 0.), (0., 0.)])]),
            vec![e(
                K::SelfOverlap,
                l(&[(0., 0.), (4., 0.)]),
                Some((0, A, None)),
                None,
            )],
        ),
        (
            "multipart-crossing",
            ml(vec![l(&[(0., 0.), (4., 4.)]), l(&[(0., 4.), (4., 0.)])]),
            if crossing {
                vec![e(K::SelfCrossing, p(2., 2.), Some((0, A, None)), None)]
            } else {
                vec![]
            },
        ),
        (
            "skipped-first-part",
            ml(vec![l(&[]), l(&[(0., 0.), (4., 0.), (0., 0.)])]),
            vec![e(
                K::SelfOverlap,
                l(&[(0., 0.), (4., 0.)]),
                Some((0, A, Some(1))),
                None,
            )],
        ),
        // The closed ring starts and ends where another segment crosses it.
        (
            "closed-seam-crossing",
            l(&[(2., 2.), (4., 4.), (4., 0.), (0., 4.), (0., 0.), (2., 2.)]),
            if crossing {
                vec![one(K::SelfCrossing, p(2., 2.))]
            } else {
                vec![]
            },
        ),
    ];
    for (name, g, expected) in rows {
        out.push(case(rule, name, vec![g], vec![], 0., expected));
    }
    out.push(case(rule, "empty-input", vec![], vec![], 0., vec![]));
    out
}
fn graph_nodes(rule: Rule) -> Vec<Case> {
    let dangle = rule == Rule::LineMustNotHaveDangles;
    let seg = l(&[(0., 0.), (4., 0.)]);
    let node = |g| one(K::Dangle, g);
    let rows = vec![
        ("closed-loop", vec![perimeter(&square())], vec![]),
        (
            "duplicate-open-path",
            vec![seg.clone(), seg.clone()],
            if dangle {
                vec![]
            } else {
                vec![
                    pair(K::PseudoNode, p(0., 0.), false),
                    pair(K::PseudoNode, p(4., 0.), false),
                ]
            },
        ),
        (
            "isolated-segment",
            vec![seg.clone()],
            if dangle {
                vec![node(p(0., 0.)), node(p(4., 0.))]
            } else {
                vec![]
            },
        ),
        (
            "joined-path",
            vec![seg.clone(), l(&[(4., 0.), (8., 0.)])],
            if dangle {
                vec![
                    node(p(0., 0.)),
                    e(K::Dangle, p(8., 0.), Some((0, B, Some(0))), None),
                ]
            } else {
                vec![pair(K::PseudoNode, p(4., 0.), false)]
            },
        ),
        (
            "bent-join",
            vec![seg.clone(), l(&[(4., 0.), (4., 4.)])],
            if dangle {
                vec![
                    node(p(0., 0.)),
                    e(K::Dangle, p(4., 4.), Some((0, B, Some(0))), None),
                ]
            } else {
                vec![pair(K::PseudoNode, p(4., 0.), false)]
            },
        ),
        (
            "t-junction",
            vec![seg.clone(), l(&[(2., 0.), (2., 4.)])],
            if dangle {
                vec![
                    node(p(0., 0.)),
                    node(p(4., 0.)),
                    e(K::Dangle, p(2., 4.), Some((0, B, Some(0))), None),
                ]
            } else {
                vec![]
            },
        ),
        (
            "three-endpoint-star",
            vec![
                seg.clone(),
                l(&[(4., 0.), (8., 0.)]),
                l(&[(4., 0.), (4., 4.)]),
            ],
            if dangle {
                vec![
                    node(p(0., 0.)),
                    e(K::Dangle, p(8., 0.), Some((0, B, Some(0))), None),
                    e(K::Dangle, p(4., 4.), Some((0, B + 1, Some(0))), None),
                ]
            } else {
                vec![]
            },
        ),
        (
            "same-feature-join",
            vec![ml(vec![seg.clone(), l(&[(4., 0.), (8., 0.)])])],
            if dangle {
                vec![
                    node(p(0., 0.)),
                    e(K::Dangle, p(8., 0.), Some((0, A, Some(1))), None),
                ]
            } else {
                vec![]
            },
        ),
        (
            "repeated-coordinate",
            vec![l(&[(0., 0.), (0., 0.), (4., 0.)])],
            if dangle {
                vec![node(p(0., 0.)), node(p(4., 0.))]
            } else {
                vec![]
            },
        ),
        ("empty-line", vec![l(&[])], vec![]),
        ("degenerate-line", vec![l(&[(0., 0.), (0., 0.)])], vec![]),
        (
            "single-part-multi-loop",
            vec![ml(vec![perimeter(&square())])],
            vec![],
        ),
        (
            "skipped-part",
            vec![ml(vec![l(&[]), seg.clone()])],
            if dangle {
                vec![
                    e(K::Dangle, p(0., 0.), Some((0, A, Some(1))), None),
                    e(K::Dangle, p(4., 0.), Some((0, A, Some(1))), None),
                ]
            } else {
                vec![]
            },
        ),
        (
            "two-feature-ring",
            vec![
                l(&[(0., 0.), (8., 0.), (8., 8.)]),
                l(&[(8., 8.), (0., 8.), (0., 0.)]),
            ],
            if dangle {
                vec![]
            } else {
                vec![
                    pair(K::PseudoNode, p(0., 0.), false),
                    pair(K::PseudoNode, p(8., 8.), false),
                ]
            },
        ),
    ];
    let mut out = vec![];
    for (name, a, expected) in rows {
        out.push(case(rule, name, a, vec![], 0., expected));
    }
    out.push(case(rule, "empty-input", vec![], vec![], 0., vec![]));
    for (name, offset) in [
        ("join-below", 0.25_f64.next_down()),
        ("join-at", 0.25),
        ("join-above", 0.25_f64.next_up()),
    ] {
        // Place the near join at zero to retain adjacent floating-point values.
        let a = l(&[(-4., 0.), (0., 0.)]);
        let b = l(&[(offset, 0.), (4., 0.)]);
        let expected = if offset <= 0.25 {
            if dangle {
                vec![
                    node(p(-4., 0.)),
                    e(K::Dangle, p(4., 0.), Some((0, B, Some(0))), None),
                ]
            } else {
                vec![pair(K::PseudoNode, p(0., 0.), false)]
            }
        } else if dangle {
            vec![
                node(p(-4., 0.)),
                node(p(0., 0.)),
                e(K::Dangle, p(offset, 0.), Some((0, B, Some(0))), None),
                e(K::Dangle, p(4., 0.), Some((0, B, Some(0))), None),
            ]
        } else {
            vec![]
        };
        out.push(case(rule, name, vec![a, b], vec![], 0.25, expected));
    }
    // An endpoint near another line's interior joins it, leaving a three-way node.
    for (name, d) in [
        ("undershoot-below", 0.25_f64.next_down()),
        ("undershoot-at", 0.25),
        ("undershoot-above", 0.25_f64.next_up()),
    ] {
        let mut expected = vec![
            node(p(0., 0.)),
            node(p(8., 0.)),
            e(K::Dangle, p(4., 4.), Some((0, B, Some(0))), None),
        ];
        if d > 0.25 {
            expected.push(e(K::Dangle, p(4., d), Some((0, B, Some(0))), None));
        }
        out.push(case(
            rule,
            name,
            vec![l(&[(0., 0.), (8., 0.)]), l(&[(4., d), (4., 4.)])],
            vec![],
            0.25,
            if dangle { expected } else { vec![] },
        ));
    }
    // A final bend turns back past a right angle, so the end lies within
    // tolerance of the previous segment just before the bend. Up to the
    // tolerance the bend vertex joins the end; just beyond it the bend is a
    // real segment. Either way the line has not returned to itself.
    for (name, bend) in [("final-bend-at", 0.25), ("final-bend-above", 0.2578125)] {
        let hooked = l(&[(2., -8.), (-bend, 0.), (0., 0.)]);
        out.push(case(
            rule,
            name,
            vec![hooked.clone(), l(&[(0., 0.), (0., 8.)])],
            vec![],
            0.25,
            if dangle {
                vec![
                    node(p(2., -8.)),
                    e(K::Dangle, p(0., 8.), Some((0, B, Some(0))), None),
                ]
            } else {
                vec![pair(K::PseudoNode, p(0., 0.), false)]
            },
        ));
        out.push(case(
            rule,
            if bend == 0.25 {
                "final-bend-at-alone"
            } else {
                "final-bend-above-alone"
            },
            vec![hooked],
            vec![],
            0.25,
            if dangle {
                vec![node(p(2., -8.)), node(p(0., 0.))]
            } else {
                vec![]
            },
        ));
    }
    // Here the last segment runs back along the line beyond the tolerance from
    // the point it ends beside, so the end touches the line's own interior.
    out.push(case(
        rule,
        "final-bend-returning",
        vec![
            l(&[(0., -8.), (0., 0.), (0.125, -1.)]),
            l(&[(0.125, -1.), (4., -1.)]),
        ],
        vec![],
        0.25,
        if dangle {
            vec![
                node(p(0., -8.)),
                e(K::Dangle, p(4., -1.), Some((0, B, Some(0))), None),
            ]
        } else {
            vec![]
        },
    ));
    out
}

fn line_polygon(rule: Rule) -> Vec<Case> {
    // Expected covered pieces, shared edges, crossings, and contact are independent fixtures.
    let rows = vec![
        (
            "interior",
            l(&[(1., 1.), (7., 1.)]),
            square(),
            vec![],
            vec![],
            vec![],
            Some(l(&[(1., 1.), (7., 1.)])),
            true,
        ),
        (
            "outside",
            l(&[(-4., 4.), (-2., 4.)]),
            square(),
            vec![l(&[(-4., 4.), (-2., 4.)])],
            vec![],
            vec![],
            None,
            false,
        ),
        (
            "boundary-run",
            l(&[(0., 0.), (8., 0.)]),
            square(),
            vec![],
            vec![l(&[(0., 0.), (8., 0.)])],
            vec![],
            Some(l(&[(0., 0.), (8., 0.)])),
            false,
        ),
        (
            "entry-exit",
            l(&[(-2., 4.), (10., 4.)]),
            square(),
            vec![l(&[(-2., 4.), (0., 4.)]), l(&[(8., 4.), (10., 4.)])],
            vec![],
            vec![(0., 4.), (8., 4.)],
            Some(l(&[(0., 4.), (8., 4.)])),
            false,
        ),
        (
            "partial-entry",
            l(&[(-2., 4.), (4., 4.)]),
            square(),
            vec![l(&[(-2., 4.), (0., 4.)])],
            vec![],
            vec![(0., 4.)],
            Some(l(&[(0., 4.), (4., 4.)])),
            false,
        ),
        (
            "tangent-vertex",
            l(&[(-2., 2.), (0., 0.), (2., -2.)]),
            square(),
            vec![l(&[(-2., 2.), (0., 0.), (2., -2.)])],
            vec![],
            vec![],
            Some(p(0., 0.)),
            false,
        ),
        (
            "endpoint-contact",
            l(&[(-2., 4.), (0., 4.)]),
            square(),
            vec![l(&[(-2., 4.), (0., 4.)])],
            vec![],
            vec![],
            Some(p(0., 4.)),
            false,
        ),
        (
            "vertex-passage",
            l(&[(-2., -2.), (0., 0.), (2., 2.)]),
            square(),
            vec![l(&[(-2., -2.), (0., 0.)])],
            vec![],
            vec![(0., 0.)],
            Some(l(&[(0., 0.), (2., 2.)])),
            false,
        ),
        (
            "hole-crossing",
            l(&[(1., 4.), (7., 4.)]),
            hole(),
            vec![l(&[(2., 4.), (6., 4.)])],
            vec![],
            vec![(2., 4.), (6., 4.)],
            Some(ml(vec![l(&[(1., 4.), (2., 4.)]), l(&[(6., 4.), (7., 4.)])])),
            false,
        ),
        (
            "hole-boundary",
            l(&[(2., 2.), (6., 2.)]),
            hole(),
            vec![],
            vec![l(&[(2., 2.), (6., 2.)])],
            vec![],
            Some(l(&[(2., 2.), (6., 2.)])),
            false,
        ),
        (
            "hole-interior",
            l(&[(3., 4.), (5., 4.)]),
            hole(),
            vec![l(&[(3., 4.), (5., 4.)])],
            vec![],
            vec![],
            None,
            false,
        ),
        (
            "concave-notch",
            l(&[(4., 4.), (6., 4.)]),
            concave(),
            vec![l(&[(4., 4.), (6., 4.)])],
            vec![],
            vec![],
            None,
            false,
        ),
        (
            "repeated-coordinate",
            l(&[(1., 1.), (1., 1.), (7., 1.)]),
            square(),
            vec![],
            vec![],
            vec![],
            Some(l(&[(1., 1.), (7., 1.)])),
            true,
        ),
    ];
    let mut out = vec![];
    for (name, g, target, outside, edges, crossings, contact, proper) in rows {
        let expected = match rule {
            Rule::LineMustBeInside => {
                if outside.is_empty() {
                    vec![]
                } else {
                    vec![one(
                        K::Outside,
                        if outside.len() == 1 {
                            outside[0].clone()
                        } else {
                            ml(outside)
                        },
                    )]
                }
            }
            Rule::LineMustBeProperlyInsidePolygons => {
                if proper {
                    vec![]
                } else {
                    vec![one(K::Outside, g.clone())]
                }
            }
            Rule::LineMustBeCoveredByPolygonBoundaries => {
                if edges.first() == Some(&g) {
                    vec![]
                } else {
                    vec![one(K::NotCovered, g.clone())]
                }
            }
            Rule::LineMustNotOverlapPolygonBoundaries => edges
                .into_iter()
                .map(|g| pair(K::Overlap, g, true))
                .collect(),
            Rule::LineMustNotCrossPolygonBoundaries => {
                if crossings.is_empty() {
                    vec![]
                } else {
                    vec![pair(
                        K::Crossing,
                        if crossings.len() == 1 {
                            p(crossings[0].0, crossings[0].1)
                        } else {
                            mp(&crossings)
                        },
                        true,
                    )]
                }
            }
            Rule::LineMustNotOverlapWithPolygons => match contact {
                Some(g) if !matches!(g, Geometry::Point(_)) => vec![pair(K::Overlap, g, true)],
                _ => vec![],
            },
            Rule::LineMustNotIntersectPolygons => contact
                .into_iter()
                .map(|g| pair(K::Intersection, g, true))
                .collect(),
            _ => unreachable!(),
        };
        out.push(case(rule, name, vec![g], vec![target], 0., expected));
    }
    out.push(case(
        rule,
        "empty-input",
        vec![],
        vec![square()],
        0.,
        vec![],
    ));
    out.push(case(
        rule,
        "empty-multi-line",
        vec![ml(vec![])],
        vec![square()],
        0.,
        vec![],
    ));
    out.push(case(
        rule,
        "degenerate-line",
        vec![l(&[(0., 0.), (0., 0.)])],
        vec![square()],
        0.,
        vec![],
    ));
    let required = matches!(
        rule,
        Rule::LineMustBeInside
            | Rule::LineMustBeProperlyInsidePolygons
            | Rule::LineMustBeCoveredByPolygonBoundaries
    );
    let g = l(&[(1., 1.), (7., 1.)]);
    out.push(case(
        rule,
        "empty-target",
        vec![g.clone()],
        vec![],
        0.,
        if required {
            vec![one(
                if rule == Rule::LineMustBeCoveredByPolygonBoundaries {
                    K::NotCovered
                } else {
                    K::Outside
                },
                g,
            )]
        } else {
            vec![]
        },
    ));
    // The outside-piece fixture retains a long source. The other short
    // whole-line fixtures also exercise graph input collapse.
    if matches!(
        rule,
        Rule::LineMustBeInside
            | Rule::LineMustBeCoveredByPolygonBoundaries
            | Rule::LineMustNotOverlapWithPolygons
            | Rule::LineMustNotOverlapPolygonBoundaries
    ) {
        for (name, length) in [
            ("piece-below", 0.25_f64.next_down()),
            ("piece-at", 0.25),
            ("piece-above", 0.25_f64.next_up()),
        ] {
            let name = if rule == Rule::LineMustBeInside {
                name
            } else {
                match name {
                    "piece-below" => "collapsed-whole-line-below",
                    "piece-at" => "collapsed-whole-line-at",
                    _ => "collapsed-whole-line-above-control",
                }
            };
            let g = if rule == Rule::LineMustBeInside {
                l(&[(-length, 4.), (4., 4.)])
            } else if rule == Rule::LineMustBeCoveredByPolygonBoundaries {
                l(&[(0., 1.), (length, 1.)])
            } else {
                l(&[(0., 0.), (length, 0.)])
            };
            let expected = if length > 0.25 {
                vec![if rule == Rule::LineMustBeInside {
                    one(K::Outside, l(&[(-length, 4.), (-0.25, 4.)]))
                } else if rule == Rule::LineMustBeCoveredByPolygonBoundaries {
                    one(K::NotCovered, g.clone())
                } else {
                    pair(K::Overlap, g.clone(), true)
                }]
            } else {
                vec![]
            };
            out.push(case(rule, name, vec![g], vec![square()], 0.25, expected));
        }
    }
    if rule == Rule::LineMustNotCrossPolygonBoundaries {
        // Outside, along the bottom edge, then inside or back outside.
        for (name, end, expected) in [
            (
                "boundary-run-entry",
                (6., 2.),
                vec![pair(K::Crossing, p(4., 0.), true)],
            ),
            ("boundary-run-return", (6., -2.), vec![]),
        ] {
            out.push(case(
                rule,
                name,
                vec![l(&[(4., -2.), (4., 0.), (6., 0.), end])],
                vec![square()],
                0.,
                expected,
            ));
        }
    }
    if rule == Rule::LineMustNotIntersectPolygons {
        // A triangle apex approaches the line interior; no edge runs along it.
        for (name, d) in [
            ("vertex-near-interior-below", 0.25_f64.next_down()),
            ("vertex-near-interior-at", 0.25),
            ("vertex-near-interior-above", 0.25_f64.next_up()),
        ] {
            let triangle = Polygon::new(
                LineString::from(vec![(4., d), (8., 4.), (0., 4.), (4., d)]),
                vec![],
            );
            out.push(case(
                rule,
                name,
                vec![l(&[(0., 0.), (8., 0.)])],
                vec![triangle.into()],
                0.25,
                if d <= 0.25 {
                    vec![pair(K::Intersection, p(4., 0.), true)]
                } else {
                    vec![]
                },
            ));
        }
    }
    out
}
fn endpoints(rule: Rule) -> Vec<Case> {
    let polygon = rule == Rule::LineEndpointsMustBeCoveredByPolygonBoundaries;
    let targets = |pts: &[(f64, f64)]| {
        if polygon {
            vec![square()]
        } else {
            vec![mp(pts)]
        }
    };
    let rows = vec![
        (
            "both-covered",
            l(&[(0., 0.), (8., 0.)]),
            targets(&[(0., 0.), (8., 0.)]),
            vec![],
        ),
        (
            "both-uncovered",
            l(&[(1., 1.), (7., 1.)]),
            targets(&[]),
            vec![one(K::NotCovered, mp(&[(1., 1.), (7., 1.)]))],
        ),
        (
            "one-uncovered",
            l(&[(0., 0.), (4., 4.)]),
            targets(&[(0., 0.)]),
            vec![one(K::NotCovered, p(4., 4.))],
        ),
        ("closed-loop", perimeter(&square()), vec![], vec![]),
        ("empty-line", l(&[]), targets(&[]), vec![]),
        (
            "collapsed-line",
            l(&[(0., 0.), (0., 0.)]),
            targets(&[]),
            vec![],
        ),
        (
            "repeated-coordinate",
            l(&[(0., 0.), (0., 0.), (8., 0.)]),
            targets(&[(0., 0.), (8., 0.)]),
            vec![],
        ),
        (
            "single-multi-loop",
            ml(vec![perimeter(&square())]),
            vec![],
            vec![],
        ),
        (
            "interior-target-only",
            l(&[(2., 2.), (6., 2.)]),
            if polygon {
                vec![square()]
            } else {
                vec![p(4., 2.)]
            },
            vec![one(K::NotCovered, mp(&[(2., 2.), (6., 2.)]))],
        ),
        (
            "skipped-original-part",
            ml(vec![l(&[]), l(&[(0., 0.), (4., 4.)])]),
            targets(&[(0., 0.)]),
            vec![e(K::NotCovered, p(4., 4.), Some((0, A, Some(1))), None)],
        ),
        (
            "multipart-aggregation",
            ml(vec![l(&[(1., 1.), (2., 1.)]), l(&[(3., 1.), (4., 1.)])]),
            targets(&[]),
            vec![e(
                K::NotCovered,
                mp(&[(1., 1.), (2., 1.), (3., 1.), (4., 1.)]),
                Some((0, A, None)),
                None,
            )],
        ),
    ];
    let mut out = vec![];
    for (name, g, b, expected) in rows {
        out.push(case(rule, name, vec![g], b, 0., expected));
    }
    out.push(case(rule, "empty-input", vec![], targets(&[]), 0., vec![]));
    out.push(case(
        rule,
        "empty-target",
        vec![l(&[(0., 0.), (8., 0.)])],
        vec![],
        0.,
        vec![one(K::NotCovered, mp(&[(0., 0.), (8., 0.)]))],
    ));
    for (name, d) in [
        ("distance-below", 0.25_f64.next_down()),
        ("distance-at", 0.25),
        ("distance-above", 0.25_f64.next_up()),
    ] {
        let g = l(&[(-d, 4.), (0., 0.)]);
        let expected = if d > 0.25 {
            vec![one(K::NotCovered, p(-d, 4.))]
        } else {
            vec![]
        };
        out.push(case(
            rule,
            name,
            vec![g],
            if polygon {
                vec![square()]
            } else {
                vec![mp(&[(0., 4.), (0., 0.)])]
            },
            0.25,
            expected,
        ));
    }
    if polygon {
        let parcels = vec![square(), rect(8., 0., 8., 8.).into()];
        for (name, g, b, expected) in [
            (
                "hole-boundary-endpoint",
                l(&[(2., 4.), (1., 4.)]),
                vec![hole()],
                vec![one(K::NotCovered, p(1., 4.))],
            ),
            (
                "shared-edge-endpoint",
                l(&[(8., 4.), (4., 4.)]),
                parcels,
                vec![one(K::NotCovered, p(4., 4.))],
            ),
            (
                "endpoints-on-two-polygons",
                l(&[(0., 4.), (16., 4.)]),
                vec![square(), rect(12., 0., 4., 8.).into()],
                vec![],
            ),
        ] {
            out.push(case(rule, name, vec![g], b, 0., expected));
        }
    }
    out
}

fn polygon_overlap(rule: Rule) -> Vec<Case> {
    let binary = rule.inputs().1.is_some();
    let rows = vec![
        ("disjoint", square(), rect(10., 0., 8., 8.).into(), None, 0.),
        (
            "edge-adjacent",
            square(),
            rect(8., 0., 8., 8.).into(),
            None,
            0.,
        ),
        ("equal", square(), square(), Some(square()), 0.),
        (
            "partial-overlap",
            square(),
            rect(4., 0., 8., 8.).into(),
            Some(rect(4., 0., 4., 8.).into()),
            0.,
        ),
        (
            "nested",
            square(),
            rect(2., 2., 4., 4.).into(),
            Some(rect(2., 2., 4., 4.).into()),
            0.,
        ),
        (
            "point-adjacent",
            square(),
            rect(8., 8., 4., 4.).into(),
            None,
            0.,
        ),
        ("inside-hole", hole(), rect(3., 3., 2., 2.).into(), None, 0.),
        ("hole-equal", hole(), hole(), Some(hole()), 0.),
        (
            "concave-notch",
            concave(),
            rect(4., 4., 2., 2.).into(),
            None,
            0.,
        ),
        (
            "concave-covered",
            concave(),
            rect(0.5, 0.5, 1., 1.).into(),
            Some(rect(0.5, 0.5, 1., 1.).into()),
            0.,
        ),
        (
            "source-inside-target",
            rect(2., 2., 4., 4.).into(),
            square(),
            Some(rect(2., 2., 4., 4.).into()),
            0.,
        ),
        (
            "single-multi",
            polys(vec![square()]),
            square(),
            Some(square()),
            0.,
        ),
        ("empty-multi", polys(vec![]), square(), None, 0.),
        // For a 2 by 2 square, 2 * area / perimeter is exactly 1.
        (
            "sliver-tolerance-below",
            rect(0., 0., 2., 2.).into(),
            rect(0., 0., 2., 2.).into(),
            Some(rect(0., 0., 2., 2.).into()),
            1_f64.next_down(),
        ),
        (
            "sliver-tolerance-at",
            rect(0., 0., 2., 2.).into(),
            rect(0., 0., 2., 2.).into(),
            Some(rect(0., 0., 2., 2.).into()),
            1.,
        ),
        (
            "sliver-tolerance-above",
            rect(0., 0., 2., 2.).into(),
            rect(0., 0., 2., 2.).into(),
            None,
            1_f64.next_up(),
        ),
    ];
    let mut out = vec![];
    for (name, a, b, g, t) in rows {
        let expected = g.into_iter().map(|g| pair(K::Overlap, g, binary)).collect();
        out.push(if binary {
            case(rule, name, vec![a], vec![b], t, expected)
        } else {
            case(rule, name, vec![a, b], vec![], t, expected)
        });
    }
    let multi = polys(vec![square(), rect(12., 0., 8., 8.).into()]);
    let expected = vec![e(
        K::Overlap,
        multi.clone(),
        Some((0, A, None)),
        Some((usize::from(binary), if binary { A } else { B }, None)),
    )];
    out.push(if binary {
        case(
            rule,
            "multipart-overlap",
            vec![multi.clone()],
            vec![multi],
            0.,
            expected,
        )
    } else {
        case(
            rule,
            "multipart-overlap",
            vec![multi.clone(), multi],
            vec![],
            0.,
            expected,
        )
    });
    out.push(case(rule, "empty-input", vec![], vec![], 0., vec![]));
    if binary {
        out.push(case(
            rule,
            "empty-target",
            vec![square()],
            vec![],
            0.,
            vec![],
        ));
        out.push(case(
            rule,
            "same-layer-overlap-ignored",
            vec![square(), square()],
            vec![rect(12., 0., 8., 8.).into()],
            0.,
            vec![],
        ));
    }
    out
}
fn gaps() -> Vec<Case> {
    let rule = Rule::PolygonMustNotHaveGaps;
    let outer = |g: &Geometry| e(K::OuterBoundary, perimeter(g), None, None);
    let gap = |g: &Geometry| e(K::Gap, perimeter(g), None, None);
    let rows = vec![
        ("solid-square", vec![square()], vec![outer(&square())], 0.),
        (
            "solid-concave",
            vec![concave()],
            vec![outer(&concave())],
            0.,
        ),
        (
            "single-hole",
            vec![hole()],
            vec![outer(&square()), gap(&rect(2., 2., 4., 4.).into())],
            0.,
        ),
        (
            "two-holes",
            vec![
                Polygon::new(
                    rect(0., 0., 8., 8.).exterior().clone(),
                    vec![
                        rect(1., 1., 1., 1.).exterior().clone(),
                        rect(5., 5., 2., 2.).exterior().clone(),
                    ],
                )
                .into(),
            ],
            vec![
                outer(&square()),
                gap(&rect(1., 1., 1., 1.).into()),
                gap(&rect(5., 5., 2., 2.).into()),
            ],
            0.,
        ),
        (
            "hole-filled",
            vec![hole(), rect(2., 2., 4., 4.).into()],
            vec![outer(&square())],
            0.,
        ),
        (
            "adjacent-parcels",
            vec![rect(0., 0., 4., 8.).into(), rect(4., 0., 4., 8.).into()],
            vec![outer(&square())],
            0.,
        ),
        (
            "disconnected",
            vec![square(), rect(12., 0., 8., 8.).into()],
            vec![outer(&square()), outer(&rect(12., 0., 8., 8.).into())],
            0.,
        ),
        (
            "duplicate-coverage",
            vec![square(), square()],
            vec![outer(&square())],
            0.,
        ),
        (
            "nested-filled-area",
            vec![square(), rect(2., 2., 4., 4.).into()],
            vec![outer(&square())],
            0.,
        ),
        (
            "island-in-hole",
            vec![hole(), rect(3., 3., 2., 2.).into()],
            vec![
                outer(&square()),
                gap(&rect(2., 2., 4., 4.).into()),
                outer(&rect(3., 3., 2., 2.).into()),
            ],
            0.,
        ),
        ("empty-multi", vec![polys(vec![])], vec![], 0.),
        (
            "single-part-multi",
            vec![polys(vec![square()])],
            vec![outer(&square())],
            0.,
        ),
        (
            "threshold-below",
            vec![hole()],
            vec![outer(&square()), gap(&rect(2., 2., 4., 4.).into())],
            2_f64.next_down(),
        ),
        (
            "threshold-at",
            vec![hole()],
            vec![outer(&square()), gap(&rect(2., 2., 4., 4.).into())],
            2.,
        ),
        (
            "threshold-above",
            vec![hole()],
            vec![outer(&square())],
            2_f64.next_up(),
        ),
        ("empty-input", vec![], vec![], 0.),
    ];
    rows.into_iter()
        .map(|(name, a, expected, t)| case(rule, name, a, vec![], t, expected))
        .collect()
}
fn polygon_points(rule: Rule) -> Vec<Case> {
    let exactly = rule == Rule::PolygonMustContainOnePoint;
    let prohibit = rule == Rule::PolygonMustNotContainPoints;
    let kind = if exactly {
        K::PointCount
    } else {
        K::MissingPoint
    };
    let missing = || e(kind, square(), Some((0, A, None)), None);
    let unassigned = |g: Geometry| e(K::PointCount, g, None, Some((1, A, None)));
    let rows = vec![
        (
            "one-interior",
            square(),
            vec![p(1., 1.)],
            if prohibit {
                vec![pair(K::Contained, p(1., 1.), true)]
            } else {
                vec![]
            },
        ),
        (
            "boundary-point",
            square(),
            vec![p(0., 4.)],
            if prohibit {
                vec![]
            } else if exactly {
                vec![missing(), unassigned(p(0., 4.))]
            } else {
                vec![missing()]
            },
        ),
        (
            "outside-point",
            square(),
            vec![p(12., 4.)],
            if prohibit {
                vec![]
            } else if exactly {
                vec![missing(), unassigned(p(12., 4.))]
            } else {
                vec![missing()]
            },
        ),
        (
            "two-distinct-features",
            square(),
            vec![p(1., 1.), p(7., 7.)],
            if prohibit {
                vec![
                    pair(K::Contained, p(1., 1.), true),
                    e(
                        K::Contained,
                        p(7., 7.),
                        Some((0, A, Some(0))),
                        Some((1, A + 1, Some(0))),
                    ),
                ]
            } else if exactly {
                vec![missing()]
            } else {
                vec![]
            },
        ),
        (
            "multipart-one-feature",
            square(),
            vec![mp(&[(1., 1.), (7., 7.)])],
            if prohibit {
                vec![e(
                    K::Contained,
                    mp(&[(1., 1.), (7., 7.)]),
                    Some((0, A, Some(0))),
                    Some((1, A, None)),
                )]
            } else {
                vec![]
            },
        ),
        (
            "partial-multipart",
            square(),
            vec![mp(&[(1., 1.), (12., 4.)])],
            if prohibit {
                vec![pair(K::Contained, p(1., 1.), true)]
            } else if exactly {
                vec![
                    missing(),
                    e(
                        K::PointCount,
                        mp(&[(1., 1.), (12., 4.)]),
                        None,
                        Some((1, A, None)),
                    ),
                ]
            } else {
                vec![missing()]
            },
        ),
        (
            "hole-point",
            hole(),
            vec![p(4., 4.)],
            if prohibit {
                vec![]
            } else if exactly {
                vec![
                    e(kind, hole(), Some((0, A, None)), None),
                    unassigned(p(4., 4.)),
                ]
            } else {
                vec![e(kind, hole(), Some((0, A, None)), None)]
            },
        ),
        (
            "hole-boundary",
            hole(),
            vec![p(2., 4.)],
            if prohibit {
                vec![]
            } else if exactly {
                vec![
                    e(kind, hole(), Some((0, A, None)), None),
                    unassigned(p(2., 4.)),
                ]
            } else {
                vec![e(kind, hole(), Some((0, A, None)), None)]
            },
        ),
        (
            "empty-point-multi",
            square(),
            vec![mp(&[])],
            if prohibit { vec![] } else { vec![missing()] },
        ),
        (
            "empty-target",
            square(),
            vec![],
            if prohibit { vec![] } else { vec![missing()] },
        ),
        (
            "single-point-multi",
            square(),
            vec![mp(&[(1., 1.)])],
            if prohibit {
                vec![pair(K::Contained, p(1., 1.), true)]
            } else {
                vec![]
            },
        ),
    ];
    let mut out = vec![];
    for (name, a, b, expected) in rows {
        out.push(case(rule, name, vec![a], b, 0., expected));
    }
    out.push(case(
        rule,
        "empty-input",
        vec![],
        vec![p(1., 1.)],
        0.,
        if exactly {
            vec![unassigned(p(1., 1.))]
        } else {
            vec![]
        },
    ));
    out.push(case(
        rule,
        "empty-polygon-multi",
        vec![polys(vec![])],
        vec![],
        0.,
        vec![],
    ));
    for (name, d) in [
        ("strict-below", 0.25_f64.next_down()),
        ("strict-at", 0.25),
        ("strict-above", 0.25_f64.next_up()),
    ] {
        let g = p(d, 4.);
        let expected = if d > 0.25 {
            if prohibit {
                vec![pair(K::Contained, g.clone(), true)]
            } else {
                vec![]
            }
        } else if prohibit {
            vec![]
        } else if exactly {
            vec![missing(), unassigned(g.clone())]
        } else {
            vec![missing()]
        };
        out.push(case(rule, name, vec![square()], vec![g], 0.25, expected));
    }
    let expected = if exactly {
        vec![unassigned(p(4., 4.))]
    } else if prohibit {
        vec![
            pair(K::Contained, p(4., 4.), true),
            e(
                K::Contained,
                p(4., 4.),
                Some((0, B, Some(0))),
                Some((1, A, Some(0))),
            ),
        ]
    } else {
        vec![]
    };
    out.push(case(
        rule,
        "ambiguous-overlapping-polygons",
        vec![square(), rect(2., 2., 8., 8.).into()],
        vec![p(4., 4.)],
        0.,
        expected,
    ));
    let multi = polys(vec![square(), rect(12., 0., 8., 8.).into()]);
    let points = mp(&[(1., 1.), (13., 1.)]);
    out.push(case(
        rule,
        "disconnected-whole-feature",
        vec![multi],
        vec![points.clone()],
        0.,
        if prohibit {
            vec![e(
                K::Contained,
                points,
                Some((0, A, None)),
                Some((1, A, None)),
            )]
        } else {
            vec![]
        },
    ));
    out
}
fn polygon_boundary_coverage() -> Vec<Case> {
    let rule = Rule::PolygonBoundariesMustBeCoveredByLines;
    let edge = l(&[(0., 0.), (8., 0.)]);
    let rest = l(&[(8., 0.), (8., 8.), (0., 8.), (0., 0.)]);
    let rows = vec![
        (
            "complete-perimeter",
            square(),
            vec![perimeter(&square())],
            vec![],
        ),
        (
            "hole-perimeters",
            hole(),
            vec![ml(vec![
                perimeter(&square()),
                perimeter(&rect(2., 2., 4., 4.).into()),
            ])],
            vec![],
        ),
        (
            "uncovered-perimeter",
            square(),
            vec![],
            vec![one(K::NotCovered, perimeter(&square()))],
        ),
        (
            "one-covered-edge",
            square(),
            vec![edge.clone()],
            vec![one(K::NotCovered, rest)],
        ),
        (
            "hole-missing",
            hole(),
            vec![perimeter(&square())],
            vec![one(K::NotCovered, perimeter(&rect(2., 2., 4., 4.).into()))],
        ),
        (
            "only-hole-covered",
            hole(),
            vec![perimeter(&rect(2., 2., 4., 4.).into())],
            vec![one(K::NotCovered, perimeter(&square()))],
        ),
        (
            "concave-complete",
            concave(),
            vec![perimeter(&concave())],
            vec![],
        ),
        (
            "interior-line",
            square(),
            vec![l(&[(1., 1.), (7., 1.)])],
            vec![one(K::NotCovered, perimeter(&square()))],
        ),
        (
            "isolated-vertex-touch",
            square(),
            vec![l(&[(-2., 2.), (0., 0.), (2., -2.)])],
            vec![one(K::NotCovered, perimeter(&square()))],
        ),
        (
            "empty-line-target",
            square(),
            vec![ml(vec![])],
            vec![one(K::NotCovered, perimeter(&square()))],
        ),
        (
            "single-polygon-multi",
            polys(vec![square()]),
            vec![perimeter(&square())],
            vec![],
        ),
        ("empty-polygon-multi", polys(vec![]), vec![edge], vec![]),
        (
            "degenerate-target",
            square(),
            vec![l(&[(0., 0.), (0., 0.)])],
            vec![one(K::NotCovered, perimeter(&square()))],
        ),
    ];
    let mut out = vec![];
    for (name, a, b, expected) in rows {
        out.push(case(rule, name, vec![a], b, 0., expected));
    }
    out.push(case(rule, "empty-input", vec![], vec![], 0., vec![]));
    // The connected closed perimeter is tested against its total length, 8.
    for (name, t) in [
        ("length-tolerance-below", 8_f64.next_down()),
        ("length-tolerance-at", 8.),
        ("length-tolerance-above", 8_f64.next_up()),
    ] {
        let g: Geometry = rect(0., 0., 2., 2.).into();
        let expected = if t < 8. {
            vec![one(K::NotCovered, perimeter(&g))]
        } else {
            vec![]
        };
        out.push(case(rule, name, vec![g], vec![], t, expected));
    }
    let far: Geometry = rect(12., 0., 8., 8.).into();
    let multi = polys(vec![square(), far.clone()]);
    out.push(case(
        rule,
        "uncovered-later-part",
        vec![multi.clone()],
        vec![perimeter(&square())],
        0.,
        vec![e(
            K::NotCovered,
            perimeter(&far),
            Some((0, A, Some(1))),
            None,
        )],
    ));
    out.push(case(
        rule,
        "uncovered-whole-multi",
        vec![multi],
        vec![],
        0.,
        vec![e(
            K::NotCovered,
            ml(vec![perimeter(&square()), perimeter(&far)]),
            Some((0, A, None)),
            None,
        )],
    ));
    out.push(case(
        rule,
        "second-feature-uncovered",
        vec![square(), far.clone()],
        vec![perimeter(&far)],
        0.,
        vec![one(K::NotCovered, perimeter(&square()))],
    ));
    out
}

fn multipart() -> Vec<Case> {
    let rule = Rule::GeometryMustNotBeMultipart;
    let rows = vec![
        ("single-point", p(0., 0.), false),
        ("single-line", l(&[(0., 0.), (4., 0.)]), false),
        ("two-points", mp(&[(0., 0.), (4., 0.)]), true),
        (
            "two-lines",
            ml(vec![l(&[(0., 0.), (4., 0.)]), l(&[(0., 4.), (4., 4.)])]),
            true,
        ),
        ("single-polygon", square(), false),
        ("single-point-multi", mp(&[(0., 0.)]), false),
        (
            "single-line-multi",
            ml(vec![l(&[(0., 0.), (4., 0.)])]),
            false,
        ),
        ("single-polygon-multi", polys(vec![square()]), false),
        ("empty-point-multi", mp(&[]), false),
        ("empty-collection", collection(vec![]), false),
        (
            "nested-single",
            collection(vec![collection(vec![p(0., 0.)])]),
            false,
        ),
        (
            "nested-mixed",
            collection(vec![p(0., 0.), collection(vec![square()])]),
            true,
        ),
        ("holes-are-not-parts", hole(), false),
        (
            "two-polygons",
            polys(vec![square(), rect(12., 0., 8., 8.).into()]),
            true,
        ),
        ("coincident-parts", mp(&[(0., 0.), (0., 0.)]), true),
        (
            "degenerate-part-counts",
            ml(vec![l(&[]), l(&[(0., 0.), (4., 0.)])]),
            true,
        ),
    ];
    let mut out = vec![];
    for (name, g, invalid) in rows {
        let expected = if invalid {
            vec![e(K::Multipart, g.clone(), Some((0, A, None)), None)]
        } else {
            vec![]
        };
        out.push(case(rule, name, vec![g], vec![], 0., expected));
    }
    out.push(case(rule, "empty-input", vec![], vec![], 0., vec![]));
    out
}
fn containment() -> Vec<Case> {
    let rule = Rule::GeometryMustNotBeContainedBy;
    let rows = vec![
        ("outside-point", p(12., 4.), vec![square()], false),
        (
            "partial-line",
            l(&[(-2., 4.), (4., 4.)]),
            vec![square()],
            false,
        ),
        ("interior-point", p(4., 4.), vec![square()], true),
        (
            "interior-line",
            l(&[(1., 1.), (7., 1.)]),
            vec![square()],
            true,
        ),
        ("equal-polygon", square(), vec![square()], false),
        (
            "equal-line-resegmented",
            l(&[(0., 0.), (8., 0.)]),
            vec![l(&[(8., 0.), (4., 0.), (0., 0.)])],
            false,
        ),
        ("equal-point", p(0., 0.), vec![p(0., 0.)], false),
        (
            "boundary-line",
            l(&[(0., 0.), (8., 0.)]),
            vec![square()],
            false,
        ),
        (
            "line-interior-point",
            p(4., 0.),
            vec![l(&[(0., 0.), (8., 0.)])],
            true,
        ),
        (
            "line-endpoint-point",
            p(0., 0.),
            vec![l(&[(0., 0.), (8., 0.)])],
            false,
        ),
        ("polygon-boundary-point", p(0., 4.), vec![square()], false),
        (
            "nested-polygon",
            rect(2., 2., 4., 4.).into(),
            vec![square()],
            true,
        ),
        ("hole-point", p(4., 4.), vec![hole()], false),
        (
            "whole-multipoint",
            mp(&[(1., 1.), (7., 7.)]),
            vec![square()],
            true,
        ),
        (
            "partial-multipoint",
            mp(&[(1., 1.), (12., 1.)]),
            vec![square()],
            false,
        ),
        (
            "target-union-excluded",
            mp(&[(1., 1.), (9., 1.)]),
            vec![square(), rect(8., 0., 8., 8.).into()],
            false,
        ),
        (
            "mixed-nested-collection",
            collection(vec![
                p(1., 1.),
                collection(vec![l(&[(2., 2.), (3., 2.)]), rect(4., 4., 1., 1.).into()]),
            ]),
            vec![square()],
            true,
        ),
        ("empty-source-multi", mp(&[]), vec![square()], false),
        ("empty-target-multi", p(1., 1.), vec![polys(vec![])], false),
        ("empty-target", p(1., 1.), vec![], false),
        ("single-part-multi", mp(&[(1., 1.)]), vec![square()], true),
        ("concave-notch", p(4., 4.), vec![concave()], false),
        (
            "line-in-longer-line",
            l(&[(2., 0.), (6., 0.)]),
            vec![l(&[(0., 0.), (8., 0.)])],
            true,
        ),
        (
            "multipoint-boundary-and-interior",
            mp(&[(4., 4.), (0., 4.)]),
            vec![square()],
            true,
        ),
    ];
    let mut out = vec![];
    for (name, g, b, invalid) in rows {
        let part = None;
        let expected = if invalid {
            vec![e(
                K::Contained,
                if name == "mixed-nested-collection" {
                    collection(vec![
                        p(1., 1.),
                        l(&[(2., 2.), (3., 2.)]),
                        rect(4., 4., 1., 1.).into(),
                    ])
                } else if let Geometry::MultiPoint(m) = &g {
                    if m.0.len() == 1 {
                        Geometry::Point(m.0[0])
                    } else {
                        g.clone()
                    }
                } else {
                    g.clone()
                },
                Some((0, A, part)),
                Some((1, A, None)),
            )]
        } else {
            vec![]
        };
        out.push(case(rule, name, vec![g], b, 0., expected));
    }
    out.push(case(
        rule,
        "empty-input",
        vec![],
        vec![square()],
        0.,
        vec![],
    ));
    out.push(case(
        rule,
        "tolerance-does-not-buffer",
        vec![p(-0.125, 4.)],
        vec![square()],
        1.,
        vec![],
    ));
    out.push(case(
        rule,
        "two-containing-targets",
        vec![p(4., 4.)],
        vec![square(), rect(2., 2., 4., 4.).into()],
        0.,
        vec![
            e(
                K::Contained,
                p(4., 4.),
                Some((0, A, None)),
                Some((1, A, None)),
            ),
            e(
                K::Contained,
                p(4., 4.),
                Some((0, A, None)),
                Some((1, A + 1, None)),
            ),
        ],
    ));
    out
}
fn base_cases(rule: Rule) -> Vec<Case> {
    match rule {
        Rule::PointMustNotOverlap | Rule::PointMustNotOverlapWith => point_duplicates(rule),
        Rule::PointMustBeInside
        | Rule::PointMustBeCoveredByLines
        | Rule::PointMustBeCoveredByLineEndpoints
        | Rule::PointMustBeCoveredByPolygonBoundaries
        | Rule::PointMustBeCoveredByPolygons
        | Rule::PointMustNotBeInsidePolygons
        | Rule::PointMustNotIntersectLines
        | Rule::PointMustNotIntersectPolygons => point_relations(rule),
        Rule::LineMustNotHaveDangles | Rule::LineMustNotHavePseudoNodes => graph_nodes(rule),
        Rule::LineMustNotOverlap
        | Rule::LineMustNotOverlapWith
        | Rule::LineMustNotIntersect
        | Rule::LineMustNotIntersectWith
        | Rule::LineMustNotIntersectOrTouchInterior
        | Rule::LineMustNotIntersectOrTouchInteriorWith => line_pairs(rule),
        Rule::LineMustNotSelfOverlap | Rule::LineMustNotSelfIntersect => self_lines(rule),
        Rule::LineMustBeInside
        | Rule::LineMustBeProperlyInsidePolygons
        | Rule::LineMustBeCoveredByPolygonBoundaries
        | Rule::LineMustNotOverlapPolygonBoundaries
        | Rule::LineMustNotCrossPolygonBoundaries
        | Rule::LineMustNotOverlapWithPolygons
        | Rule::LineMustNotIntersectPolygons => line_polygon(rule),
        Rule::LineEndpointsMustBeCoveredByPoints
        | Rule::LineEndpointsMustBeCoveredByPolygonBoundaries => endpoints(rule),
        Rule::PolygonMustNotOverlap | Rule::PolygonMustNotOverlapWith => polygon_overlap(rule),
        Rule::PolygonMustNotHaveGaps => gaps(),
        Rule::PolygonMustContainPoint
        | Rule::PolygonMustContainOnePoint
        | Rule::PolygonMustNotContainPoints => polygon_points(rule),
        Rule::PolygonBoundariesMustBeCoveredByLines => polygon_boundary_coverage(),
        Rule::GeometryMustNotBeMultipart => multipart(),
        Rule::GeometryMustNotBeContainedBy => containment(),
        _ => panic!("missing canonical rule {rule}"),
    }
}

fn check_rule(rule: Rule) {
    let mut failures = vec![];
    for c in cases(rule) {
        let result = std::panic::catch_unwind(|| {
            let fresh_report = topology(&c).validate().unwrap().rules.remove(0);
            let fresh = fresh_report.violations;
            assert_expected(&c, &fresh);
        });
        if result.is_err() {
            failures.push(c.name);
        }
    }
    assert!(
        failures.is_empty(),
        "{rule}: failing semantic cases {failures:?}"
    );
}
macro_rules! contract_tests {($($name:ident:$rule:ident),*$(,)?)=>{$(#[test]fn $name(){check_rule(Rule::$rule);})*};}
contract_tests! {
    point_duplicate_contract:PointMustNotOverlap, point_cross_duplicates:PointMustNotOverlapWith,
    point_inside:PointMustBeInside, point_line_coverage:PointMustBeCoveredByLines,
    point_endpoint_coverage:PointMustBeCoveredByLineEndpoints, point_boundary_coverage:PointMustBeCoveredByPolygonBoundaries,
    point_area_coverage:PointMustBeCoveredByPolygons, point_not_inside:PointMustNotBeInsidePolygons,
    point_line_contacts:PointMustNotIntersectLines, point_polygon_contacts:PointMustNotIntersectPolygons,
    dangles:LineMustNotHaveDangles,pseudos:LineMustNotHavePseudoNodes,
    line_overlap:LineMustNotOverlap,line_cross_overlap:LineMustNotOverlapWith,
    self_overlap:LineMustNotSelfOverlap,self_intersection:LineMustNotSelfIntersect,
    line_intersection:LineMustNotIntersect,line_cross_intersection:LineMustNotIntersectWith,
    line_interior_touch:LineMustNotIntersectOrTouchInterior,line_cross_interior_touch:LineMustNotIntersectOrTouchInteriorWith,
    line_inside:LineMustBeInside,line_strict_inside:LineMustBeProperlyInsidePolygons,
    line_boundary_coverage:LineMustBeCoveredByPolygonBoundaries,line_boundary_overlap:LineMustNotOverlapPolygonBoundaries,
    line_boundary_crossing:LineMustNotCrossPolygonBoundaries,line_polygon_overlap:LineMustNotOverlapWithPolygons,
    line_polygon_contact:LineMustNotIntersectPolygons,endpoints_points:LineEndpointsMustBeCoveredByPoints,
    endpoints_boundaries:LineEndpointsMustBeCoveredByPolygonBoundaries,
    polygon_overlap_contract:PolygonMustNotOverlap,polygon_cross_overlap:PolygonMustNotOverlapWith,
    polygon_gaps:PolygonMustNotHaveGaps,polygon_has_point:PolygonMustContainPoint,
    polygon_has_one_point:PolygonMustContainOnePoint,polygon_no_points:PolygonMustNotContainPoints,
    polygon_boundary_coverage_contract:PolygonBoundariesMustBeCoveredByLines,
    geometry_multipart:GeometryMustNotBeMultipart,geometry_containment:GeometryMustNotBeContainedBy,
}

#[test]
fn every_scenario_repeated_and_streamed_matches_separate_fresh_topologies() {
    for &rule in Rule::ALL {
        for c in cases(rule) {
            let fresh_report = topology(&c).validate().unwrap().rules.remove(0);
            let fresh = fresh_report.violations;
            let repeated = topology(&c);
            for _ in 0..2 {
                assert_same(
                    &fresh,
                    &repeated.validate().unwrap().rules.remove(0).violations,
                    c.name,
                );
            }
            let stream = topology(&c);
            let mut actual = vec![];
            let summary = stream
                .validate_into(|spec, v| {
                    assert_eq!(spec.rule, rule);
                    actual.push(v);
                    Ok::<_, ()>(())
                })
                .unwrap();
            assert_eq!(summary.violation_count(), fresh.len());
            assert_eq!(
                summary.rules[0].counts, fresh_report.counts,
                "{} {}",
                rule, c.name
            );
            assert_same(&fresh, &actual, c.name);
        }
    }
}
fn reverse_and_rotate(g: &Geometry, lines: bool, winding: bool, rotation: bool) -> Geometry {
    let ring = |r: &LineString| {
        if r.0.len() < 2 {
            return r.clone();
        }
        let mut c = r.0[..r.0.len() - 1].to_vec();
        if winding {
            c.reverse();
        }
        if rotation {
            c.rotate_left(1);
        }
        c.push(c[0]);
        LineString(c)
    };
    match g {
        Geometry::LineString(l) => {
            let mut l = l.clone();
            if lines {
                l.0.reverse();
            }
            l.into()
        }
        Geometry::MultiLineString(m) => ml(m
            .0
            .iter()
            .map(|l| reverse_and_rotate(&l.clone().into(), lines, winding, rotation))
            .collect()),
        Geometry::Polygon(p) => {
            Polygon::new(ring(p.exterior()), p.interiors().iter().map(ring).collect()).into()
        }
        Geometry::MultiPolygon(m) => polys(
            m.0.iter()
                .map(|p| reverse_and_rotate(&p.clone().into(), lines, winding, rotation))
                .collect(),
        ),
        Geometry::GeometryCollection(c) => collection(
            c.0.iter()
                .map(|g| reverse_and_rotate(g, lines, winding, rotation))
                .collect(),
        ),
        _ => g.clone(),
    }
}
fn check_invariance(operation: impl Fn(&Case)) {
    let mut failures = vec![];
    for &rule in Rule::ALL {
        for c in cases(rule) {
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| operation(&c))).is_err() {
                failures.push(format!("{rule}: {}", c.name));
            }
        }
    }
    assert!(failures.is_empty(), "invariance failures: {failures:?}");
}
#[test]
fn all_rules_feature_order_invariance() {
    check_invariance(|c| {
        let fresh = topology(c).validate().unwrap().rules.remove(0).violations;
        let mut order = c.clone();
        order.a.reverse();
        order.b.reverse();
        let actual = topology(&order)
            .validate()
            .unwrap()
            .rules
            .remove(0)
            .violations;
        // Unary pair contracts permit swapped roles. Preserve part references while
        // normalizing orientation, then require the same reported spatial set.
        let normalize = |mut rows: Vec<topology_checker_core::Violation>| {
            for v in &mut rows {
                if let (Some(a), Some(b)) = (v.origin, v.destination)
                    && a.layer == b.layer
                    && a.fid > b.fid
                {
                    v.origin = Some(b);
                    v.destination = Some(a);
                }
            }
            rows
        };
        assert_same(&normalize(fresh), &normalize(actual), c.name);
    });
}
#[test]
fn all_rules_line_reversal_polygon_winding_and_ring_rotation() {
    check_invariance(|c| {
        let fresh = topology(c).validate().unwrap().rules.remove(0).violations;
        let mut failures = vec![];
        for (label, lines, winding, rotation) in [
            ("line-reversal", true, false, false),
            ("polygon-winding", false, true, false),
            ("ring-rotation", false, false, true),
            ("combined", true, true, true),
        ] {
            let result = std::panic::catch_unwind(|| {
                let mut direction = c.clone();
                for (_, g) in direction.a.iter_mut().chain(direction.b.iter_mut()) {
                    *g = reverse_and_rotate(g, lines, winding, rotation);
                }
                assert_same(
                    &fresh,
                    &topology(&direction)
                        .validate()
                        .unwrap()
                        .rules
                        .remove(0)
                        .violations,
                    &format!("{} {label}", c.name),
                );
            });
            if result.is_err() {
                failures.push(label);
            }
        }
        assert!(
            failures.is_empty(),
            "{} direction failures: {failures:?}",
            c.name
        );
    });
}
#[test]
fn all_rules_exact_translation_and_scaling() {
    use topology_checker_core::geo::CoordsIter;
    let mut exclusions = vec![];
    let mut applied = 0;
    // Exclusions record actual lost representability, including tolerance.
    for &rule in Rule::ALL {
        for c in cases(rule) {
            for (scale, dx, dy) in [(1., 16., -32.), (2., 0., 0.), (0.5, 16., -32.)] {
                if exact_affine(&c, scale, dx, dy) {
                    applied += 1;
                } else {
                    let lost = c
                        .a
                        .iter()
                        .chain(&c.b)
                        .flat_map(|(_, g)| g.coords_iter())
                        .filter(|p| {
                            !exact_coordinate(p.x, scale, dx) || !exact_coordinate(p.y, scale, dy)
                        })
                        .count();
                    exclusions.push(format!("{rule}: {} scale={scale} offset=({dx},{dy}) lost-coordinates={lost} tolerance={}",c.name,c.tolerance));
                }
            }
        }
    }
    println!(
        "affine applied: {applied}; exclusions: {}",
        exclusions.len()
    );
    for exclusion in exclusions {
        println!("affine exclusion: {exclusion}");
    }
    check_invariance(|c| {
        let fresh = topology(c).validate().unwrap().rules.remove(0).violations;
        let mut failures = vec![];
        for (scale, dx, dy) in [(1., 16., -32.), (2., 0., 0.), (0.5, 16., -32.)] {
            if !exact_affine(c, scale, dx, dy) {
                continue;
            }
            let result = std::panic::catch_unwind(|| {
                let affine = transform(c, scale, dx, dy);
                let mut expected = fresh.clone();
                use topology_checker_core::geo::{MapCoords, coord};
                for v in &mut expected {
                    v.geometry = v
                        .geometry
                        .map_coords(|p| coord! {x:p.x*scale+dx,y:p.y*scale+dy});
                }
                assert_same(
                    &expected,
                    &topology(&affine)
                        .validate()
                        .unwrap()
                        .rules
                        .remove(0)
                        .violations,
                    &format!("{} scale={scale} offset=({dx},{dy})", c.name),
                );
            });
            if result.is_err() {
                failures.push((scale, dx, dy));
            }
        }
        assert!(
            failures.is_empty(),
            "{} affine failures: {failures:?}",
            c.name
        );
    });
}

fn exact_coordinate(x: f64, scale: f64, offset: f64) -> bool {
    let transformed = x * scale + offset;
    transformed.is_finite() && ((transformed - offset) / scale).to_bits() == x.to_bits()
}
fn exact_affine(c: &Case, scale: f64, dx: f64, dy: f64) -> bool {
    use topology_checker_core::geo::CoordsIter;
    exact_coordinate(c.tolerance, scale, 0.)
        && c.a
            .iter()
            .chain(&c.b)
            .flat_map(|(_, g)| g.coords_iter())
            .all(|p| exact_coordinate(p.x, scale, dx) && exact_coordinate(p.y, scale, dy))
}

#[test]
fn malformed_inputs_are_rejected() {
    use topology_checker_core::{Error, Layer, Topology};
    for tolerance in [-1., f64::NAN, f64::INFINITY] {
        assert!(matches!(Topology::new(tolerance), Err(Error::Tolerance(_))));
    }
    for (features, duplicate) in [
        (vec![(A, p(0., 0.)), (A, p(1., 1.))], true),
        (vec![(A, p(f64::NAN, 0.))], false),
    ] {
        let mut t = Topology::new(0.).unwrap();
        let a = t.add_layer(Layer::from_features("invalid", features));
        let err = t.add_rule(Rule::PointMustNotOverlap, a, None).unwrap_err();
        assert!(if duplicate {
            matches!(err, Error::DuplicateFid { fid: A, .. })
        } else {
            matches!(err, Error::NonFinite { fid: A, .. })
        });
    }
}

fn cases(rule: Rule) -> Vec<Case> {
    let mut out = base_cases(rule);
    out.extend(coverage_gaps::cases(rule));
    if matches!(
        rule,
        Rule::PolygonMustNotOverlap | Rule::PolygonMustNotOverlapWith
    ) {
        let binary = rule.inputs().1.is_some();
        let source = polys(vec![
            Polygon::new(LineString::empty(), vec![]).into(),
            square(),
        ]);
        let expected = vec![e(
            K::Overlap,
            square(),
            Some((0, A, Some(1))),
            Some((usize::from(binary), if binary { A } else { B }, Some(0))),
        )];
        out.push(if binary {
            case(
                rule,
                "skipped-polygon-original-index",
                vec![source],
                vec![square()],
                0.,
                expected,
            )
        } else {
            case(
                rule,
                "skipped-polygon-original-index",
                vec![source, square()],
                vec![],
                0.,
                expected,
            )
        });
    }
    if matches!(
        rule,
        Rule::LineMustNotOverlap
            | Rule::LineMustNotOverlapWith
            | Rule::LineMustNotIntersect
            | Rule::LineMustNotIntersectWith
            | Rule::LineMustNotIntersectOrTouchInterior
            | Rule::LineMustNotIntersectOrTouchInteriorWith
    ) {
        let binary = rule.inputs().1.is_some();
        for (name, d) in [
            ("parallel-distance-below", 0.25_f64.next_down()),
            ("parallel-distance-at", 0.25),
            ("parallel-distance-above", 0.25_f64.next_up()),
        ] {
            let a = l(&[(0., 0.), (8., 0.)]);
            let b = l(&[(0., d), (8., d)]);
            let expected = if d <= 0.25 {
                vec![pair(K::Overlap, a.clone(), binary)]
            } else {
                vec![]
            };
            out.push(if binary {
                case(rule, name, vec![a], vec![b], 0.25, expected)
            } else {
                case(rule, name, vec![a, b], vec![], 0.25, expected)
            });
        }
    }
    if matches!(
        rule,
        Rule::LineMustNotSelfOverlap | Rule::LineMustNotSelfIntersect
    ) {
        out.push(case(
            rule,
            "empty-multipart",
            vec![ml(vec![])],
            vec![],
            0.,
            vec![],
        ));
        out.push(case(
            rule,
            "tangential-two-lobes",
            vec![l(&[
                (0., 0.),
                (-1., 1.),
                (-2., 0.),
                (-1., -1.),
                (0., 0.),
                (1., -1.),
                (2., 0.),
                (1., 1.),
                (0., 0.),
            ])],
            vec![],
            0.,
            vec![],
        ));
        for (name, length) in [
            ("collapsed-retrace-below", 0.25_f64.next_down()),
            ("collapsed-retrace-at", 0.25),
            ("collapsed-retrace-above-control", 0.25_f64.next_up()),
        ] {
            let expected = if length > 0.25 {
                vec![one(K::SelfOverlap, l(&[(0., 0.), (length, 0.)]))]
            } else {
                vec![]
            };
            out.push(case(
                rule,
                name,
                vec![l(&[(0., 0.), (length, 0.), (0., 0.)])],
                vec![],
                0.25,
                expected,
            ));
        }
    }
    if matches!(
        rule,
        Rule::LineMustBeInside
            | Rule::LineMustBeProperlyInsidePolygons
            | Rule::LineMustBeCoveredByPolygonBoundaries
            | Rule::LineMustNotOverlapPolygonBoundaries
            | Rule::LineMustNotCrossPolygonBoundaries
            | Rule::LineMustNotOverlapWithPolygons
            | Rule::LineMustNotIntersectPolygons
    ) {
        let parcels = vec![square(), rect(8., 0., 8., 8.).into()];
        let g = l(&[(8., 1.), (8., 7.)]);
        let expected = if matches!(
            rule,
            Rule::LineMustNotOverlapPolygonBoundaries
                | Rule::LineMustNotOverlapWithPolygons
                | Rule::LineMustNotIntersectPolygons
        ) {
            let kind = if rule == Rule::LineMustNotIntersectPolygons {
                K::Intersection
            } else {
                K::Overlap
            };
            vec![
                pair(kind, g.clone(), true),
                e(
                    kind,
                    g.clone(),
                    Some((0, A, Some(0))),
                    Some((1, A + 1, Some(0))),
                ),
            ]
        } else {
            vec![]
        };
        out.push(case(
            rule,
            "individual-shared-parcel-edge",
            vec![g],
            parcels.clone(),
            0.,
            expected,
        ));
        let g = l(&[(1., 4.), (15., 4.)]);
        let expected = match rule {
            Rule::LineMustBeCoveredByPolygonBoundaries => vec![one(K::NotCovered, g.clone())],
            Rule::LineMustNotCrossPolygonBoundaries => vec![
                pair(K::Crossing, p(8., 4.), true),
                e(
                    K::Crossing,
                    p(8., 4.),
                    Some((0, A, Some(0))),
                    Some((1, A + 1, Some(0))),
                ),
            ],
            Rule::LineMustNotOverlapWithPolygons | Rule::LineMustNotIntersectPolygons => {
                let kind = if rule == Rule::LineMustNotIntersectPolygons {
                    K::Intersection
                } else {
                    K::Overlap
                };
                vec![
                    pair(kind, l(&[(1., 4.), (8., 4.)]), true),
                    e(
                        kind,
                        l(&[(8., 4.), (15., 4.)]),
                        Some((0, A, Some(0))),
                        Some((1, A + 1, Some(0))),
                    ),
                ]
            }
            _ => vec![],
        };
        out.push(case(
            rule,
            "polygon-union-spanning-line",
            vec![g],
            parcels,
            0.,
            expected,
        ));
        if rule == Rule::LineMustBeProperlyInsidePolygons {
            for (name, d) in [
                ("clearance-below", 0.25_f64.next_down()),
                ("clearance-at", 0.25),
                ("clearance-above", 0.25_f64.next_up()),
            ] {
                let g = l(&[(d, 1.), (d, 7.)]);
                let expected = if d <= 0.25 {
                    vec![one(K::Outside, g.clone())]
                } else {
                    vec![]
                };
                out.push(case(rule, name, vec![g], vec![square()], 0.25, expected));
            }
        }
        if rule == Rule::LineMustNotIntersectPolygons {
            for (name, d) in [
                ("proximity-below", 0.25_f64.next_down()),
                ("proximity-at", 0.25),
                ("proximity-above", 0.25_f64.next_up()),
            ] {
                let g = l(&[(-d, 1.), (-d, 7.)]);
                let expected = if d <= 0.25 {
                    vec![pair(K::Intersection, g.clone(), true)]
                } else {
                    vec![]
                };
                out.push(case(rule, name, vec![g], vec![square()], 0.25, expected));
            }
        }
        if matches!(
            rule,
            Rule::LineMustBeCoveredByPolygonBoundaries | Rule::LineMustNotOverlapPolygonBoundaries
        ) {
            for (name, d) in [
                ("boundary-distance-below", 0.25_f64.next_down()),
                ("boundary-distance-at", 0.25),
                ("boundary-distance-above", 0.25_f64.next_up()),
            ] {
                let g = l(&[(1., d), (7., d)]);
                let expected = if rule == Rule::LineMustBeCoveredByPolygonBoundaries {
                    if d > 0.25 {
                        vec![one(K::NotCovered, g.clone())]
                    } else {
                        vec![]
                    }
                } else if d <= 0.25 {
                    vec![pair(K::Overlap, g.clone(), true)]
                } else {
                    vec![]
                };
                out.push(case(rule, name, vec![g], vec![square()], 0.25, expected));
            }
        }
    }
    if matches!(
        rule,
        Rule::PointMustBeInside
            | Rule::PointMustBeCoveredByPolygons
            | Rule::PointMustBeCoveredByPolygonBoundaries
            | Rule::PointMustNotBeInsidePolygons
            | Rule::PointMustNotIntersectPolygons
    ) {
        let g = p(8., 4.);
        let parcels = vec![square(), rect(8., 0., 8., 8.).into()];
        let expected = match rule {
            Rule::PointMustBeInside => vec![one(K::Outside, g.clone())],
            Rule::PointMustNotIntersectPolygons => vec![
                pair(K::Intersection, g.clone(), true),
                e(
                    K::Intersection,
                    g.clone(),
                    Some((0, A, Some(0))),
                    Some((1, A + 1, Some(0))),
                ),
            ],
            _ => vec![],
        };
        out.push(case(
            rule,
            "shared-edge-individual-polygons",
            vec![g],
            parcels,
            0.,
            expected,
        ));
        let g = p(4., 4.);
        let b = vec![square(), rect(2., 2., 8., 8.).into()];
        let expected = match rule {
            Rule::PointMustBeCoveredByPolygonBoundaries => vec![one(K::NotCovered, g.clone())],
            Rule::PointMustNotBeInsidePolygons | Rule::PointMustNotIntersectPolygons => {
                let k = if rule == Rule::PointMustNotBeInsidePolygons {
                    K::Contained
                } else {
                    K::Intersection
                };
                vec![
                    pair(k, g.clone(), true),
                    e(
                        k,
                        g.clone(),
                        Some((0, A, Some(0))),
                        Some((1, A + 1, Some(0))),
                    ),
                ]
            }
            _ => vec![],
        };
        out.push(case(
            rule,
            "overlapping-target-polygon-pairs",
            vec![g],
            b,
            0.,
            expected,
        ));
    }
    out
}
