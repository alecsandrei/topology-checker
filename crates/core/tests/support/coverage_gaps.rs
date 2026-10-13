//! Independent fixtures for thresholds, multipart provenance, mixed contacts and parcel gaps.
use super::support::*;
use topology_checker_core::{
    Rule, ViolationKind as K,
    geo::{Euclidean, Geometry, Length},
};

pub fn point_rule(rule: Rule) -> bool {
    matches!(
        rule,
        Rule::PointMustBeInside
            | Rule::PointMustBeCoveredByLines
            | Rule::PointMustBeCoveredByLineEndpoints
            | Rule::PointMustBeCoveredByPolygonBoundaries
            | Rule::PointMustBeCoveredByPolygons
            | Rule::PointMustNotBeInsidePolygons
            | Rule::PointMustNotIntersectLines
            | Rule::PointMustNotIntersectPolygons
    )
}
pub fn pair_rule(rule: Rule) -> bool {
    matches!(
        rule,
        Rule::LineMustNotOverlap
            | Rule::LineMustNotOverlapWith
            | Rule::LineMustNotIntersect
            | Rule::LineMustNotIntersectWith
            | Rule::LineMustNotIntersectOrTouchInterior
            | Rule::LineMustNotIntersectOrTouchInteriorWith
    )
}
pub fn line_polygon_rule(rule: Rule) -> bool {
    matches!(
        rule,
        Rule::LineMustBeInside
            | Rule::LineMustBeProperlyInsidePolygons
            | Rule::LineMustBeCoveredByPolygonBoundaries
            | Rule::LineMustNotOverlapPolygonBoundaries
            | Rule::LineMustNotCrossPolygonBoundaries
            | Rule::LineMustNotOverlapWithPolygons
            | Rule::LineMustNotIntersectPolygons
    )
}
pub fn cases(rule: Rule) -> Vec<Case> {
    let mut out = vec![];
    if point_rule(rule) {
        point_parts(rule, &mut out);
    }
    point_line_paths(rule, &mut out);
    emitted_length(rule, &mut out);
    if line_polygon_rule(rule) {
        line_parts(rule, &mut out);
    }
    distances(rule, &mut out);
    endpoint_crossings(rule, &mut out);
    if rule == Rule::LineMustNotIntersectPolygons {
        contacts(rule, &mut out);
    }
    if rule == Rule::PolygonMustNotHaveGaps {
        parcel_gaps(&mut out);
    }
    out
}

fn point_line_paths(rule: Rule, out: &mut Vec<Case>) {
    if !matches!(
        rule,
        Rule::PointMustBeCoveredByLines
            | Rule::PointMustBeCoveredByLineEndpoints
            | Rule::PointMustNotIntersectLines
    ) {
        return;
    }
    for (name, point, target, part) in [
        (
            "point-on-target-bend",
            p(4., 0.),
            l(&[(0., 0.), (4., 0.), (4., 8.)]),
            0,
        ),
        (
            "point-on-retained-target-part",
            p(12., 4.),
            ml(vec![
                l(&[]),
                l(&[(0., 0.), (8., 0.)]),
                l(&[(12., 0.), (12., 8.)]),
            ]),
            2,
        ),
    ] {
        let expected = match rule {
            Rule::PointMustBeCoveredByLines => vec![],
            Rule::PointMustBeCoveredByLineEndpoints => vec![one(K::NotCovered, point.clone())],
            _ => vec![e(
                K::Intersection,
                point.clone(),
                Some((0, A, Some(0))),
                Some((1, A, Some(part))),
            )],
        };
        out.push(case(rule, name, vec![point], vec![target], 0., expected));
    }
}

fn point_parts(rule: Rule, out: &mut Vec<Case>) {
    // Part positions: inside first target, shared endpoint, two uncovered
    // points, inside second target. Repetition must keep provenance even
    // when the reported coordinate is deduplicated.
    for (name, repeated, nested) in [
        ("mixed-point-parts", false, false),
        ("repeated-mixed-point-parts", true, false),
        ("nested-mixed-point-parts", false, true),
    ] {
        let mut coords = vec![(1., 1.), (0., 0.), (12., 4.), (14., 4.), (21., 1.)];
        if repeated {
            coords.extend([(1., 1.), (12., 4.)]);
        }
        let source = if nested {
            collection(vec![mp(&[]), p(1., 1.), collection(vec![mp(&coords[1..])])])
        } else {
            mp(&coords)
        };
        let line_target = matches!(
            rule,
            Rule::PointMustBeCoveredByLines
                | Rule::PointMustBeCoveredByLineEndpoints
                | Rule::PointMustNotIntersectLines
        );
        let target = if line_target {
            ml(vec![
                l(&[(0., 0.), (1., 0.), (1., 1.)]),
                l(&[(20., 0.), (21., 0.), (21., 1.)]),
            ])
        } else {
            polys(vec![square(), rect(20., 0., 8., 8.).into()])
        };
        let expected = match rule {
            Rule::PointMustBeInside => {
                let mut rows = vec![
                    e(K::Outside, p(0., 0.), Some((0, A, Some(1))), None),
                    e(K::Outside, p(12., 4.), Some((0, A, Some(2))), None),
                    e(K::Outside, p(14., 4.), Some((0, A, Some(3))), None),
                ];
                if repeated {
                    rows.push(e(K::Outside, p(12., 4.), Some((0, A, Some(6))), None));
                }
                rows
            }
            Rule::PointMustBeCoveredByLines
            | Rule::PointMustBeCoveredByLineEndpoints
            | Rule::PointMustBeCoveredByPolygons => vec![e(
                K::NotCovered,
                mp(&[(12., 4.), (14., 4.)]),
                Some((0, A, None)),
                None,
            )],
            Rule::PointMustBeCoveredByPolygonBoundaries => vec![e(
                K::NotCovered,
                mp(&[(1., 1.), (12., 4.), (14., 4.), (21., 1.)]),
                Some((0, A, None)),
                None,
            )],
            Rule::PointMustNotBeInsidePolygons => vec![e(
                K::Contained,
                mp(&[(1., 1.), (21., 1.)]),
                Some((0, A, None)),
                Some((1, A, None)),
            )],
            Rule::PointMustNotIntersectPolygons | Rule::PointMustNotIntersectLines => vec![e(
                K::Intersection,
                mp(&[(1., 1.), (0., 0.), (21., 1.)]),
                Some((0, A, None)),
                Some((1, A, None)),
            )],
            _ => unreachable!(),
        };
        out.push(case(rule, name, vec![source], vec![target], 0., expected));
    }
}

fn length(g: &Geometry) -> f64 {
    let Geometry::LineString(g) = g else {
        panic!("line fixture")
    };
    Euclidean.length(g)
}
fn emitted_length(rule: Rule, out: &mut Vec<Case>) {
    let self_rule = matches!(
        rule,
        Rule::LineMustNotSelfOverlap | Rule::LineMustNotSelfIntersect
    );
    let boundary_overlap = rule == Rule::LineMustNotOverlapPolygonBoundaries;
    let boundary_coverage = rule == Rule::LineMustBeCoveredByPolygonBoundaries;
    if !(pair_rule(rule) || self_rule || boundary_overlap || boundary_coverage) {
        return;
    }
    let binary = rule.inputs().1.is_some();
    let span = l(&[(-1., 0.), (0., 0.)]);
    assert_eq!(length(&span), 1.);
    for (name, t) in [
        ("emitted-unit-zero", 0.),
        ("emitted-unit-below", 1_f64.next_down()),
        ("emitted-unit-at", 1.),
        ("emitted-unit-above", 1_f64.next_up()),
    ] {
        let left = l(&[(-4., 0.), (0., 0.)]);
        let right = l(&[(-1., 0.), (4., 0.)]);
        assert!(length(&left) > t && length(&right) > t);
        let expected = if t < 1. {
            vec![if self_rule {
                e(K::SelfOverlap, span.clone(), Some((0, A, None)), None)
            } else if boundary_coverage {
                one(K::NotCovered, span.clone())
            } else {
                pair(K::Overlap, span.clone(), binary)
            }]
        } else {
            vec![]
        };
        let c = if self_rule {
            case(rule, name, vec![ml(vec![left, right])], vec![], t, expected)
        } else if boundary_overlap || boundary_coverage {
            let source = l(&[(-4., 0.), (4., 0.)]);
            assert!(length(&source) > t);
            let target = if boundary_overlap {
                vec![rect(-1., -4., 1., 4.).into()]
            } else {
                vec![rect(-4., -4., 3., 4.).into(), rect(0., -4., 4., 4.).into()]
            };
            case(rule, name, vec![source], target, t, expected)
        } else if binary {
            case(rule, name, vec![left], vec![right], t, expected)
        } else {
            case(rule, name, vec![left, right], vec![], t, expected)
        };
        out.push(c);
    }
}

fn line_parts(rule: Rule, out: &mut Vec<Case>) {
    // Independently named pieces keep the expected oracle separate from clipping.
    let entry = l(&[(-2., 4.), (2., 4.)]);
    let edge = l(&[(0., 0.), (8., 0.)]);
    let inside = l(&[(1., 1.), (7., 1.)]);
    let far_edge = l(&[(20., 0.), (28., 0.)]);
    let far_entry = l(&[(18., 4.), (22., 4.)]);
    let target = polys(vec![square(), rect(20., 0., 8., 8.).into()]);
    for (name, skip, far) in [
        ("contributing-multipart-lines", false, false),
        ("skipped-initial-multipart-lines", true, false),
        ("partial-multipart-lines", false, true),
        ("multipart-lines-multipart-target", true, true),
    ] {
        let offset = u32::from(skip);
        let mut source = vec![];
        if skip {
            source.push(l(&[]));
        }
        source.extend([entry.clone(), edge.clone(), inside.clone()]);
        if far {
            source.push(far_edge.clone());
            source.push(far_entry.clone());
        }
        let expected = match rule {
            Rule::LineMustBeInside => vec![e(
                K::Outside,
                if far {
                    ml(vec![l(&[(-2., 4.), (0., 4.)]), l(&[(18., 4.), (20., 4.)])])
                } else {
                    l(&[(-2., 4.), (0., 4.)])
                },
                Some((0, A, if far { None } else { Some(offset) })),
                None,
            )],
            Rule::LineMustBeProperlyInsidePolygons => {
                let mut pieces = vec![entry.clone(), edge.clone()];
                if far {
                    pieces.push(far_edge.clone());
                    pieces.push(far_entry.clone());
                }
                vec![e(K::Outside, ml(pieces), Some((0, A, None)), None)]
            }
            Rule::LineMustBeCoveredByPolygonBoundaries => {
                let mut pieces = vec![entry.clone(), inside.clone()];
                if far {
                    pieces.push(far_entry.clone());
                }
                vec![e(K::NotCovered, ml(pieces), Some((0, A, None)), None)]
            }
            Rule::LineMustNotCrossPolygonBoundaries => vec![e(
                K::Crossing,
                if far {
                    mp(&[(0., 4.), (20., 4.)])
                } else {
                    p(0., 4.)
                },
                Some((0, A, if far { None } else { Some(offset) })),
                Some((1, A, if far { None } else { Some(0) })),
            )],
            Rule::LineMustNotOverlapPolygonBoundaries => {
                if far {
                    vec![e(
                        K::Overlap,
                        ml(vec![edge.clone(), far_edge.clone()]),
                        Some((0, A, None)),
                        Some((1, A, None)),
                    )]
                } else {
                    vec![e(
                        K::Overlap,
                        edge.clone(),
                        Some((0, A, Some(offset + 1))),
                        Some((1, A, Some(0))),
                    )]
                }
            }
            Rule::LineMustNotOverlapWithPolygons | Rule::LineMustNotIntersectPolygons => {
                let mut pieces = vec![l(&[(0., 4.), (2., 4.)]), edge.clone(), inside.clone()];
                if far {
                    pieces.push(far_edge.clone());
                    pieces.push(l(&[(20., 4.), (22., 4.)]));
                }
                vec![e(
                    if rule == Rule::LineMustNotIntersectPolygons {
                        K::Intersection
                    } else {
                        K::Overlap
                    },
                    ml(pieces),
                    Some((0, A, None)),
                    Some((1, A, if far { None } else { Some(0) })),
                )]
            }
            _ => unreachable!(),
        };
        out.push(case(
            rule,
            name,
            vec![ml(source)],
            vec![target.clone()],
            0.,
            expected,
        ));
    }
}

fn distances(rule: Rule, out: &mut Vec<Case>) {
    if !matches!(
        rule,
        Rule::PolygonBoundariesMustBeCoveredByLines | Rule::LineMustNotOverlapWithPolygons
    ) {
        return;
    }
    for (name, d) in [
        ("isolated-edge-distance-below", 0.25_f64.next_down()),
        ("isolated-edge-distance-at", 0.25),
        ("isolated-edge-distance-above", 0.25_f64.next_up()),
    ] {
        let (a, b, expected) = if rule == Rule::PolygonBoundariesMustBeCoveredByLines {
            // All other edges are covered exactly, so only the bottom edge can fail.
            (
                vec![square()],
                vec![ml(vec![
                    l(&[(0., -d), (8., -d)]),
                    l(&[(8., 0.), (8., 8.), (0., 8.), (0., 0.)]),
                ])],
                if d > 0.25 {
                    vec![one(K::NotCovered, l(&[(0., 0.), (8., 0.)]))]
                } else {
                    vec![]
                },
            )
        } else {
            let g = l(&[(1., -d), (7., -d)]);
            (
                vec![g.clone()],
                vec![square()],
                if d <= 0.25 {
                    vec![pair(K::Overlap, g, true)]
                } else {
                    vec![]
                },
            )
        };
        out.push(case(rule, name, a, b, 0.25, expected));
    }
}

fn endpoint_crossings(rule: Rule, out: &mut Vec<Case>) {
    if !matches!(
        rule,
        Rule::LineMustNotCrossPolygonBoundaries | Rule::LineMustNotSelfIntersect
    ) {
        return;
    }
    for (d, open_name, closed_name) in [
        (
            0.25_f64.next_down(),
            "open-crossing-below",
            "closed-crossing-below",
        ),
        (0.25, "open-crossing-at", "closed-crossing-at"),
        (
            0.25_f64.next_up(),
            "open-crossing-above",
            "closed-crossing-above",
        ),
    ] {
        for closed in [false, true] {
            let polygon = rule == Rule::LineMustNotCrossPolygonBoundaries;
            let source = if polygon {
                if closed {
                    l(&[(-d, 4.), (4., 4.), (4., 6.), (-d, 6.), (-d, 4.)])
                } else {
                    l(&[(-d, 4.), (4., 4.)])
                }
            } else {
                let mut points = vec![(-d, 0.), (4., 0.), (4., -4.), (0., -4.), (0., 4.)];
                if closed {
                    points.extend([(-4., 4.), (-4., 0.), (-d, 0.)]);
                }
                l(&points)
            };
            let expected = if closed || d > 0.25 {
                vec![if polygon {
                    pair(
                        K::Crossing,
                        if closed {
                            mp(&[(0., 4.), (0., 6.)])
                        } else {
                            p(0., 4.)
                        },
                        true,
                    )
                } else {
                    one(K::SelfCrossing, p(0., 0.))
                }]
            } else {
                vec![]
            };
            out.push(case(
                rule,
                if closed { closed_name } else { open_name },
                vec![source],
                if polygon { vec![square()] } else { vec![] },
                0.25,
                expected,
            ));
        }
    }
}

fn contacts(rule: Rule, out: &mut Vec<Case>) {
    for (name, d) in [
        ("interior-contact-length-below", 0.25_f64.next_down()),
        ("interior-contact-length-at", 0.25),
        ("interior-contact-length-above", 0.25_f64.next_up()),
    ] {
        let g = l(&[(0., 4.), (d, 4.)]);
        out.push(case(
            rule,
            name,
            vec![g.clone()],
            vec![rect(-4., 0., 8., 8.).into()],
            0.25,
            vec![pair(
                K::Intersection,
                if d <= 0.25 { p(0., 4.) } else { g },
                true,
            )],
        ));
    }
    let inside = l(&[(1., 1.), (7., 1.)]);
    let far = l(&[(21., 1.), (27., 1.)]);
    let target = polys(vec![square(), rect(20., 0., 8., 8.).into()]);
    out.push(case(
        rule,
        "mixed-line-and-point-contact",
        vec![ml(vec![
            l(&[]),
            inside.clone(),
            l(&[(-2., 2.), (0., 0.), (2., -2.)]),
            far.clone(),
        ])],
        vec![target],
        0.,
        vec![e(
            K::Intersection,
            collection(vec![ml(vec![inside, far]), p(0., 0.)]),
            Some((0, A, None)),
            Some((1, A, None)),
        )],
    ));
    // One retained source part supplies a line and an isolated touch against
    // two target parts, so its original index survives destination aggregation.
    let source = l(&[
        (1., 1.),
        (7., 1.),
        (10., 1.),
        (10., -2.),
        (20., -2.),
        (20., 0.),
        (22., -2.),
    ]);
    out.push(case(
        rule,
        "mixed-contact-skipped-single-source-part",
        vec![ml(vec![l(&[]), source])],
        vec![polys(vec![square(), rect(20., 0., 8., 8.).into()])],
        0.,
        vec![e(
            K::Intersection,
            collection(vec![l(&[(1., 1.), (8., 1.)]), p(20., 0.)]),
            Some((0, A, Some(1))),
            Some((1, A, None)),
        )],
    ));
}

fn parcel_gaps(out: &mut Vec<Case>) {
    let rule = Rule::PolygonMustNotHaveGaps;
    let parcels = vec![
        rect(0., 0., 8., 2.).into(),
        rect(0., 6., 8., 2.).into(),
        rect(0., 2., 2., 4.).into(),
        rect(6., 2., 2., 4.).into(),
    ];
    for g in &parcels {
        let Geometry::Polygon(p) = g else {
            unreachable!()
        };
        assert!(p.interiors().is_empty());
    }
    let outer = e(K::OuterBoundary, perimeter(&square()), None, None);
    let gap = e(K::Gap, perimeter(&rect(2., 2., 4., 4.).into()), None, None);
    for (name, t) in [
        ("parcel-enclosed-gap", 0.),
        ("parcel-sliver-below", 2_f64.next_down()),
        ("parcel-sliver-at", 2.),
        ("parcel-sliver-above", 2_f64.next_up()),
    ] {
        let mut expected = vec![outer.clone()];
        if t <= 2. {
            expected.push(gap.clone());
        }
        out.push(case(rule, name, parcels.clone(), vec![], t, expected));
    }
    let mut filled = parcels.clone();
    filled.push(rect(2., 2., 4., 4.).into());
    out.push(case(
        rule,
        "parcel-gap-filled",
        filled,
        vec![],
        0.,
        vec![outer.clone()],
    ));
    let mut island = parcels.clone();
    island.push(rect(3., 3., 2., 2.).into());
    out.push(case(
        rule,
        "parcel-gap-island",
        island,
        vec![],
        0.,
        vec![
            outer,
            gap,
            e(
                K::OuterBoundary,
                perimeter(&rect(3., 3., 2., 2.).into()),
                None,
                None,
            ),
        ],
    ));
    let opening = l(&[
        (0., 0.),
        (8., 0.),
        (8., 6.),
        (6., 6.),
        (6., 2.),
        (2., 2.),
        (2., 6.),
        (0., 6.),
        (0., 0.),
    ]);
    let mut opened = parcels;
    opened.remove(1);
    out.push(case(
        rule,
        "parcel-gap-opened",
        opened,
        vec![],
        0.,
        vec![e(K::OuterBoundary, opening, None, None)],
    ));
}
