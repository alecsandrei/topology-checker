use crate::{
    Error, ErrorType, FeatureRef, Rule, RuleSpec, Topology, TopologyError,
    engine::{
        cross::{self, Views},
        index,
        line_graph::{InputMode, projection, proper_intersection},
    },
    model::Part,
};
use geo::{
    BoundingRect, Contains, Coord, Distance, Euclidean, Geometry, Length, Line, LineString, Point,
    Polygon, Relate,
    line_intersection::{LineIntersection, line_intersection},
};
use rayon::prelude::*;
use std::collections::{BTreeMap, BTreeSet};

type Identity = Option<(crate::LayerId, i64)>;
fn aggregate(errors: Vec<TopologyError>) -> Vec<TopologyError> {
    let mut groups: BTreeMap<(ErrorType, Identity, Identity), Vec<TopologyError>> = BTreeMap::new();
    for e in errors {
        groups
            .entry((
                e.error_type,
                e.origin.map(FeatureRef::feature),
                e.destination.map(FeatureRef::feature),
            ))
            .or_default()
            .push(e);
    }
    groups
        .into_values()
        .map(|group| {
            let mut first = group[0].clone();
            for e in &group[1..] {
                if let (Some(a), Some(b)) = (&mut first.origin, e.origin) {
                    a.aggregate(b);
                }
                if let (Some(a), Some(b)) = (&mut first.destination, e.destination) {
                    a.aggregate(b);
                }
            }
            first.geometry = cross::combine(group.into_iter().map(|e| e.geometry));
            first
        })
        .collect()
}
fn error(
    spec: RuleSpec,
    kind: ErrorType,
    geometry: impl Into<Geometry>,
    origin: Option<FeatureRef>,
    destination: Option<FeatureRef>,
) -> TopologyError {
    TopologyError {
        rule: spec.rule,
        error_type: kind,
        geometry: geometry.into(),
        origin,
        destination,
    }
}
fn boundary_distance(p: &Polygon, point: &Point) -> f64 {
    std::iter::once(p.exterior())
        .chain(p.interiors())
        .map(|r| Euclidean.distance(r, point))
        .fold(f64::INFINITY, f64::min)
}
fn strict(p: &Polygon, point: &Point, tolerance: f64) -> bool {
    p.contains(point) && boundary_distance(p, point) > tolerance
}
fn on_segments(
    point: &Point,
    segments: &[Part<Line>],
    tree: &index::Index,
    tolerance: f64,
) -> bool {
    tree.locate_in_envelope_intersecting(&index::at(point.0, tolerance))
        .any(|i| Euclidean.distance(&segments[i.data].geometry, point) <= tolerance)
}
fn on_points(point: &Point, points: &[Part<Point>], tree: &index::Index, tolerance: f64) -> bool {
    tree.locate_in_envelope_intersecting(&index::at(point.0, tolerance))
        .any(|i| crate::engine::cluster::distance(points[i.data].geometry.0, point.0) <= tolerance)
}

/// Extract connected selected pieces before applying the length threshold.
/// A closed path's first and last runs form one piece across the traversal seam.
fn selected_pieces(coords: &[Coord], selected: &[bool]) -> Vec<LineString> {
    let mut pieces = Vec::new();
    let mut run = Vec::new();
    for (i, &keep) in selected.iter().enumerate() {
        if keep {
            if run.is_empty() {
                run.push(coords[i]);
            }
            run.push(coords[i + 1]);
        } else if !run.is_empty() {
            pieces.push(LineString(std::mem::take(&mut run)));
        }
    }
    if !run.is_empty() {
        pieces.push(LineString(run));
    }
    if pieces.len() > 1
        && coords.first() == coords.last()
        && selected.first() == Some(&true)
        && selected.last() == Some(&true)
    {
        let first = pieces.remove(0);
        let last = pieces.last_mut().expect("two pieces before joining");
        last.0.extend(first.0.into_iter().skip(1));
    }
    pieces
}

pub(crate) fn run(t: &Topology, spec: RuleSpec) -> Result<Vec<TopologyError>, Error> {
    let a = t.parts(spec.layer)?;
    let b = t.parts(spec.other.ok_or(Error::Inputs {
        rule: spec.rule,
        expected: 2,
    })?)?;
    let av = cross::views(a);
    let bv = cross::views(b);
    let errors = match spec.rule {
        Rule::PointMustBeCoveredByLines
        | Rule::PointMustBeCoveredByLineEndpoints
        | Rule::PointMustBeCoveredByPolygonBoundaries => a
            .points
            .par_iter()
            .filter_map(|p| {
                let covered = match spec.rule {
                    Rule::PointMustBeCoveredByLines => {
                        on_segments(&p.geometry, &bv.segments, &bv.segment_index, t.tolerance())
                    }
                    Rule::PointMustBeCoveredByLineEndpoints => on_points(
                        &p.geometry,
                        &bv.endpoints,
                        &bv.endpoint_index,
                        t.tolerance(),
                    ),
                    _ => on_segments(
                        &p.geometry,
                        &bv.boundary_segments,
                        &bv.boundary_index,
                        t.tolerance(),
                    ),
                };
                (!covered).then(|| {
                    error(
                        spec,
                        ErrorType::NotCovered,
                        p.geometry,
                        Some(p.reference),
                        None,
                    )
                })
            })
            .collect(),
        Rule::LineEndpointsMustBeCoveredByPoints
        | Rule::LineEndpointsMustBeCoveredByPolygonBoundaries => av
            .endpoints
            .par_iter()
            .filter_map(|p| {
                let covered = if spec.rule == Rule::LineEndpointsMustBeCoveredByPoints {
                    on_points(&p.geometry, &b.points, &b.point_index, t.tolerance())
                } else {
                    on_segments(
                        &p.geometry,
                        &bv.boundary_segments,
                        &bv.boundary_index,
                        t.tolerance(),
                    )
                };
                (!covered).then(|| {
                    error(
                        spec,
                        ErrorType::NotCovered,
                        p.geometry,
                        Some(p.reference),
                        None,
                    )
                })
            })
            .collect(),
        Rule::PointMustNotIntersectLines => {
            index::pairs(&a.point_index, &b.line_index, t.tolerance(), false)
                .par_iter()
                .filter_map(|&(i, j)| {
                    let (p, l) = (&a.points[i], &b.lines[j]);
                    (Euclidean.distance(&l.geometry, &p.geometry) <= t.tolerance()).then(|| {
                        error(
                            spec,
                            ErrorType::Intersection,
                            p.geometry,
                            Some(p.reference),
                            Some(l.reference),
                        )
                    })
                })
                .collect()
        }
        Rule::PointMustBeCoveredByPolygons
        | Rule::PointMustNotBeInsidePolygons
        | Rule::PointMustNotIntersectPolygons => {
            let required = spec.rule == Rule::PointMustBeCoveredByPolygons;
            a.points
                .par_iter()
                .flat_map_iter(|p| {
                    let mut candidates = b
                        .polygon_index
                        .locate_in_envelope_intersecting(&index::at(p.geometry.0, t.tolerance()))
                        .map(|i| i.data)
                        .collect::<Vec<_>>();
                    candidates.sort_unstable();
                    let hits = candidates
                        .into_iter()
                        .filter(|&i| {
                            let polygon = &b.polygons[i].geometry;
                            if spec.rule == Rule::PointMustNotBeInsidePolygons {
                                strict(polygon, &p.geometry, t.tolerance())
                            } else {
                                polygon.contains(&p.geometry)
                                    || boundary_distance(polygon, &p.geometry) <= t.tolerance()
                            }
                        })
                        .collect::<Vec<_>>();
                    if required {
                        if hits.is_empty() {
                            vec![error(
                                spec,
                                ErrorType::NotCovered,
                                p.geometry,
                                Some(p.reference),
                                None,
                            )]
                        } else {
                            Vec::new()
                        }
                    } else {
                        hits.into_iter()
                            .map(|i| {
                                error(
                                    spec,
                                    if spec.rule == Rule::PointMustNotBeInsidePolygons {
                                        ErrorType::Contained
                                    } else {
                                        ErrorType::Intersection
                                    },
                                    p.geometry,
                                    Some(p.reference),
                                    Some(b.polygons[i].reference),
                                )
                            })
                            .collect()
                    }
                })
                .collect()
        }
        Rule::PolygonMustNotContainPoints => {
            index::pairs(&a.polygon_index, &b.point_index, t.tolerance(), false)
                .par_iter()
                .filter_map(|&(i, j)| {
                    let (p, q) = (&a.polygons[i], &b.points[j]);
                    strict(&p.geometry, &q.geometry, t.tolerance()).then(|| {
                        error(
                            spec,
                            ErrorType::Contained,
                            q.geometry,
                            Some(p.reference),
                            Some(q.reference),
                        )
                    })
                })
                .collect()
        }
        Rule::PolygonMustContainPoint | Rule::PolygonMustContainOnePoint => {
            let mut assignments = vec![Vec::new(); bv.features.len()];
            let mut counts = vec![0; av.features.len()];
            for (i, j) in index::pairs(&av.feature_index, &bv.feature_index, 0., false) {
                let (polygon, point) = (&av.features[i], &bv.features[j]);
                if !point.points.is_empty()
                    && point.points.iter().all(|p| {
                        polygon
                            .area
                            .0
                            .iter()
                            .any(|area| strict(area, p, t.tolerance()))
                    })
                {
                    assignments[j].push(i);
                    counts[i] += 1;
                }
            }
            let mut errors = Vec::new();
            for (i, p) in av.features.iter().enumerate() {
                if counts[i] == 0
                    || (spec.rule == Rule::PolygonMustContainOnePoint && counts[i] != 1)
                {
                    errors.push(error(
                        spec,
                        if spec.rule == Rule::PolygonMustContainPoint {
                            ErrorType::MissingPoint
                        } else {
                            ErrorType::PointCount
                        },
                        p.geometry.clone(),
                        Some(p.reference),
                        None,
                    ));
                }
            }
            if spec.rule == Rule::PolygonMustContainOnePoint {
                for (j, p) in bv.features.iter().enumerate() {
                    if assignments[j].len() != 1 {
                        errors.push(error(
                            spec,
                            ErrorType::PointCount,
                            p.geometry.clone(),
                            None,
                            Some(p.reference),
                        ));
                    }
                }
            }
            errors
        }
        Rule::GeometryMustNotBeContainedBy => {
            index::pairs(&av.feature_index, &bv.feature_index, 0., false)
                .par_iter()
                .filter_map(|&(i, j)| {
                    let (a, b) = (&av.features[i], &bv.features[j]);
                    let relation = b.geometry.relate(&a.geometry);
                    (relation.is_contains() && !relation.is_within()).then(|| {
                        error(
                            spec,
                            ErrorType::Contained,
                            a.geometry.clone(),
                            Some(a.reference),
                            Some(b.reference),
                        )
                    })
                })
                .collect()
        }
        Rule::LineMustBeCoveredByPolygonBoundaries
        | Rule::PolygonBoundariesMustBeCoveredByLines
        | Rule::LineMustNotOverlapPolygonBoundaries => return boundary_linework(t, spec),
        Rule::LineMustBeProperlyInsidePolygons => {
            let union = &crate::engine::polygons::union(b).geometry;
            a.lines
                .par_iter()
                .filter_map(|l| {
                    let inside = union.relate(&l.geometry).is_contains()
                        && union
                            .0
                            .iter()
                            .flat_map(|p| std::iter::once(p.exterior()).chain(p.interiors()))
                            .all(|r| Euclidean.distance(&l.geometry, r) > t.tolerance());
                    (!inside).then(|| {
                        error(
                            spec,
                            ErrorType::Outside,
                            l.geometry.clone(),
                            Some(l.reference),
                            None,
                        )
                    })
                })
                .collect()
        }
        Rule::LineMustNotCrossPolygonBoundaries
        | Rule::LineMustNotOverlapWithPolygons
        | Rule::LineMustNotIntersectPolygons => {
            index::pairs(&a.line_index, &b.polygon_index, t.tolerance(), false)
                .par_iter()
                .flat_map_iter(|&(i, j)| line_polygon(t, spec, &a.lines[i], &b.polygons[j], bv))
                .collect()
        }
        _ => unreachable!("cross-geometry dispatch"),
    };
    Ok(aggregate(errors))
}

fn boundary_linework(t: &Topology, spec: RuleSpec) -> Result<Vec<TopologyError>, Error> {
    let other = spec.other.expect("binary rule");
    let reverse = spec.rule == Rule::PolygonBoundariesMustBeCoveredByLines;
    let graph = t.graph_inputs(vec![
        (
            spec.layer,
            if reverse {
                InputMode::PolygonBoundary
            } else {
                InputMode::Linework
            },
        ),
        (
            other,
            if reverse {
                InputMode::Linework
            } else {
                InputMode::PolygonBoundary
            },
        ),
    ])?;
    let overlap = spec.rule == Rule::LineMustNotOverlapPolygonBoundaries;
    let mut targets: BTreeMap<_, BTreeSet<FeatureRef>> = BTreeMap::new();
    for edge in &graph.edges {
        for usage in &edge.uses {
            let r = graph.parts[usage.part].reference;
            if r.layer == other {
                targets
                    .entry((edge.nodes[0], edge.nodes[1]))
                    .or_default()
                    .insert(r);
            }
        }
    }
    let mut errors = Vec::new();
    for part in &graph.parts {
        if part.reference.layer != spec.layer {
            continue;
        }
        let destinations: Vec<_> = if overlap {
            part.nodes
                .windows(2)
                .filter_map(|pair| targets.get(&(pair[0].min(pair[1]), pair[0].max(pair[1]))))
                .flatten()
                .copied()
                .collect::<BTreeSet<_>>()
                .into_iter()
                .map(Some)
                .collect()
        } else {
            vec![None]
        };
        for destination in destinations {
            let coords = part
                .nodes
                .iter()
                .map(|&node| graph.coords[node])
                .collect::<Vec<_>>();
            let selected = part
                .nodes
                .windows(2)
                .map(|pair| {
                    let key = (pair[0].min(pair[1]), pair[0].max(pair[1]));
                    let covered = targets.get(&key);
                    if let Some(r) = destination {
                        covered.is_some_and(|set| set.contains(&r))
                    } else {
                        covered.is_none()
                    }
                })
                .collect::<Vec<_>>();
            for piece in selected_pieces(&coords, &selected) {
                if Euclidean.length(&piece) > t.tolerance() {
                    errors.push(error(
                        spec,
                        if overlap {
                            ErrorType::Overlap
                        } else {
                            ErrorType::NotCovered
                        },
                        piece,
                        Some(part.reference),
                        destination,
                    ));
                }
            }
        }
    }
    Ok(aggregate(errors))
}

#[derive(Debug)]
struct Cut {
    position: f64,
    point: Coord,
}
fn line_polygon(
    t: &Topology,
    spec: RuleSpec,
    line: &Part<LineString>,
    polygon: &Part<Polygon>,
    views: &Views,
) -> Vec<TopologyError> {
    let mut cuts = Vec::new();
    let mut contacts = Vec::new();
    for (i, segment) in line.geometry.lines().enumerate() {
        cuts.push(Cut {
            position: i as f64,
            point: segment.start,
        });
        cuts.push(Cut {
            position: (i + 1) as f64,
            point: segment.end,
        });
        let mut candidates = views
            .boundary_index
            .locate_in_envelope_intersecting(&index::expanded(
                segment.bounding_rect(),
                t.tolerance(),
            ))
            .map(|i| i.data)
            .collect::<Vec<_>>();
        candidates.sort_unstable();
        for j in candidates {
            let boundary = &views.boundary_segments[j];
            if boundary.reference != polygon.reference {
                continue;
            }
            if let Some(intersection) = line_intersection(segment, boundary.geometry) {
                let points = match intersection {
                    LineIntersection::SinglePoint {
                        intersection,
                        is_proper,
                    } => vec![if is_proper {
                        proper_intersection(segment, boundary.geometry).unwrap_or(intersection)
                    } else {
                        intersection
                    }],
                    LineIntersection::Collinear { intersection } => {
                        vec![intersection.start, intersection.end]
                    }
                };
                for point in points {
                    cuts.push(Cut {
                        position: i as f64 + projection(segment, point).0,
                        point,
                    });
                    contacts.push(Point(point));
                }
            }
            if spec.rule == Rule::LineMustNotIntersectPolygons && t.tolerance() > 0. {
                // A minimum segment distance occurs at an endpoint projection unless
                // they cross, which has already supplied an exact contact.
                for point in [segment.start, segment.end] {
                    if Euclidean.distance(&Point(point), &boundary.geometry) <= t.tolerance() {
                        contacts.push(Point(point));
                    }
                }
                for point in [boundary.geometry.start, boundary.geometry.end] {
                    let (_, projected) = projection(segment, point);
                    if crate::engine::cluster::distance(point, projected) <= t.tolerance() {
                        contacts.push(Point(projected));
                    }
                }
            }
        }
    }
    cuts.sort_by(|a, b| a.position.total_cmp(&b.position));
    cuts.dedup_by(|a, b| a.position == b.position);
    let states = cuts
        .windows(2)
        .map(|pair| {
            let midpoint = Point(Coord {
                x: pair[0].point.x + (pair[1].point.x - pair[0].point.x) / 2.,
                y: pair[0].point.y + (pair[1].point.y - pair[0].point.y) / 2.,
            });
            if boundary_distance(&polygon.geometry, &midpoint) == 0. {
                0
            } else if polygon.geometry.contains(&midpoint) {
                1
            } else {
                -1
            }
        })
        .collect::<Vec<_>>();
    if spec.rule == Rule::LineMustNotCrossPolygonBoundaries {
        let mut errors = Vec::new();
        for i in 1..cuts.len().saturating_sub(1) {
            if states[i - 1] * states[i] == -1 {
                errors.push(error(
                    spec,
                    ErrorType::Crossing,
                    Point(cuts[i].point),
                    Some(line.reference),
                    Some(polygon.reference),
                ));
            }
        }
        if line.geometry.0.first() == line.geometry.0.last()
            && states.len() > 1
            && states[0] * states[states.len() - 1] == -1
        {
            errors.push(error(
                spec,
                ErrorType::Crossing,
                Point(cuts[0].point),
                Some(line.reference),
                Some(polygon.reference),
            ));
        }
        return errors;
    }
    let mut pieces = Vec::new();
    let coords = cuts.iter().map(|cut| cut.point).collect::<Vec<_>>();
    let selected = states.iter().map(|&state| state >= 0).collect::<Vec<_>>();
    for piece in selected_pieces(&coords, &selected) {
        if Euclidean.length(&piece) > t.tolerance() {
            pieces.push(piece);
        } else if spec.rule == Rule::LineMustNotIntersectPolygons && piece.0.len() >= 2 {
            contacts.push(Point(piece.0[0]));
        }
    }
    let any = spec.rule == Rule::LineMustNotIntersectPolygons;
    let mut geometries = pieces
        .iter()
        .cloned()
        .map(Geometry::LineString)
        .collect::<Vec<_>>();
    if any {
        geometries.extend(
            contacts
                .into_iter()
                .filter(|p| !pieces.iter().any(|l| Euclidean.distance(l, p) == 0.))
                .map(Geometry::Point),
        );
    }
    if geometries.is_empty() {
        Vec::new()
    } else {
        vec![error(
            spec,
            if any {
                ErrorType::Intersection
            } else {
                ErrorType::Overlap
            },
            cross::combine(geometries),
            Some(line.reference),
            Some(polygon.reference),
        )]
    }
}
