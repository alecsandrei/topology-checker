//! Endpoint coverage uses direct part closure, independently of cached views.
use std::sync::Arc;
use topology_checker_core::{
    Layer, Rule, Topology, Violation,
    geo::{Geometry, LineString, MapCoords, MultiLineString, Point},
};

fn part(end: (f64, f64)) -> LineString {
    LineString::from(vec![(0., 0.), (0., 20.), (20., 20.), end])
}

fn run(rule: Rule, lines: Arc<Layer>, tolerance: f64) -> Vec<Violation> {
    let mut topology = Topology::new(tolerance).unwrap();
    let other = topology.add_layer(Layer::from_geometries(
        "points",
        [Point::new(0., 0.).into()],
    ));
    let lines = topology.add_layer(lines);
    let empty = topology.add_layer(Layer::from_geometries("empty", []));
    let (source, target) = if rule == Rule::PointMustBeCoveredByLineEndpoints {
        (other, lines)
    } else {
        (lines, empty)
    };
    topology.add_rule(rule, source, Some(target)).unwrap();
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

const RULES: [Rule; 3] = [
    Rule::PointMustBeCoveredByLineEndpoints,
    Rule::LineEndpointsMustBeCoveredByPoints,
    Rule::LineEndpointsMustBeCoveredByPolygonBoundaries,
];

#[test]
fn closure_uses_the_inclusive_binary_distance_without_rounding_the_hypotenuse() {
    for y in [4_f64.next_down(), 4., 4_f64.next_up()] {
        let lines = Arc::new(Layer::from_features("lines", [(-17, part((3., y)).into())]));
        for rule in RULES {
            let violations = run(rule, Arc::clone(&lines), 5.);
            let closed = y <= 4.;
            if rule == Rule::PointMustBeCoveredByLineEndpoints {
                assert_eq!(violations.len(), usize::from(closed), "{rule}: {y}");
            } else {
                assert_eq!(violations.is_empty(), closed, "{rule}: {y}");
                for violation in violations {
                    assert_eq!(violation.origin.unwrap().fid, -17);
                    assert_eq!(violation.origin.unwrap().part, Some(0));
                }
            }
        }
    }
}

#[test]
fn shared_layer_endpoint_caches_do_not_depend_on_the_first_tolerance() {
    let lines = Arc::new(Layer::from_geometries("lines", [part((3., 4.)).into()]));
    for tolerance in [0., 5., 4.9, 6., 0.] {
        for rule in RULES {
            let violations = run(rule, Arc::clone(&lines), tolerance);
            let closed = tolerance >= 5.;
            assert_eq!(
                violations.is_empty(),
                if rule == Rule::PointMustBeCoveredByLineEndpoints {
                    !closed
                } else {
                    closed
                }
            );
        }
    }
    let closed = Arc::new(Layer::from_geometries("closed", [part((0., 0.)).into()]));
    for rule in RULES {
        assert_eq!(
            run(rule, Arc::clone(&closed), 0.).is_empty(),
            rule != Rule::PointMustBeCoveredByLineEndpoints
        );
    }
}

#[test]
fn closure_preserves_subnormal_distance_thresholds() {
    let tolerance = f64::from_bits(1);
    for bits in [0, 1, 2] {
        let lines = Arc::new(Layer::from_geometries(
            "tiny gap",
            [part((f64::from_bits(bits), 0.)).into()],
        ));
        for rule in RULES {
            let violations = run(rule, Arc::clone(&lines), tolerance);
            assert_eq!(
                violations.is_empty(),
                if rule == Rule::PointMustBeCoveredByLineEndpoints {
                    bits > 1
                } else {
                    bits <= 1
                },
            );
        }
    }
}

#[test]
fn multipart_closure_keeps_other_parts_and_original_coordinates() {
    for reverse in [false, true] {
        for (offset, rotation) in [((0., 0.), 0_f64), ((4096., -2048.), 0.7)] {
            for gap in [4.9, 5.1] {
                let mut near_closed = part((gap, 0.));
                let mut open = LineString::from(vec![(40., 0.), (60., 0.)]);
                if reverse {
                    near_closed.0.reverse();
                    open.0.reverse();
                }
                let geometry: Geometry = MultiLineString(vec![near_closed, open]).into();
                let (s, c) = rotation.sin_cos();
                let geometry = geometry.map_coords(|p| {
                    geo_coord(p.x * c - p.y * s + offset.0, p.x * s + p.y * c + offset.1)
                });
                let layer = Arc::new(Layer::from_features("multipart", [(71, geometry.clone())]));
                for rule in [
                    Rule::LineEndpointsMustBeCoveredByPoints,
                    Rule::LineEndpointsMustBeCoveredByPolygonBoundaries,
                ] {
                    let errors = run(rule, Arc::clone(&layer), 5.);
                    assert!(!errors.is_empty());
                    if gap < 5. {
                        assert_eq!(errors.len(), 1);
                        assert_eq!(errors[0].origin.unwrap().part, Some(1));
                    }
                }
                assert_eq!(layer.features().next().unwrap().1, &geometry);
            }
        }
    }
}

fn geo_coord(x: f64, y: f64) -> topology_checker_core::geo::Coord {
    topology_checker_core::geo::Coord { x, y }
}
