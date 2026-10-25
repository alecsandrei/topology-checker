use crate::cache::Cache;
use crate::{
    Error, FeatureRef, LayerId, Topology,
    cancel::{self, checkpoint, every},
    engine::{
        cluster::{Sweep, distance},
        index::{self, Entry, Packed, Search},
        parallel::{self, Workers},
        storage::Shared,
    },
};
use geo::{
    BoundingRect, Coord, Line, LineString,
    line_intersection::{LineIntersection, line_intersection},
};
use num_rational::BigRational;
use num_traits::ToPrimitive;
use std::{
    collections::{BTreeMap, HashMap},
    ops::Range,
};
type FeatureKey = (LayerId, i64);
/// A point where two passages meet, by its traversal position on each part.
#[derive(Debug, Clone, Copy)]
struct Meeting {
    positions: [f64; 2],
    /// The open line end lying within tolerance of the other passage here, where
    /// the passages meet without touching.
    end: Option<Coord>,
}
/// Lines meeting at a smaller angle are treated as one shared course beyond this
/// many tolerances, so long nearly parallel runs keep their support.
const NEAR_ZONE_TOLERANCES: f64 = 20.;
/// A segment beside a passage: its envelope, its line, and whether each of its
/// vertices ends an open part.
type Beside = (rstar::AABB<[f64; 2]>, Line, [bool; 2]);
#[cfg(test)]
type MatchPairs = BTreeMap<(FeatureKey, FeatureKey), Vec<usize>>;

#[cfg(test)]
thread_local! {
    pub(crate) static MATCHING_WORK: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static PASSAGE_ROLE_SLOTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static JUNCTION_SUPPORT_WORK: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[derive(Debug)]
pub(crate) struct GraphPart {
    pub reference: FeatureRef,
    pub nodes: Vec<usize>,
    pub is_closed: bool,
    original: Shared<LineString>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Layer, Rule};

    #[test]
    fn multipart_restoration_allocates_roles_only_for_self_participants() {
        // Horizontal and vertical support are measured on different axes.
        for (self_group, vertical) in [(false, false), (true, false), (false, true), (true, true)] {
            let place = |x: f64, y: f64| if vertical { (y, x) } else { (x, y) };
            let mut t = Topology::new(1.).unwrap();
            let a = LineString::from(vec![place(0., 0.), place(3., 0.)]);
            let b = geo::MultiLineString(
                (0..6)
                    .map(|i| {
                        LineString::from(vec![
                            place(i as f64 / 2., 0.25),
                            place((i + 1) as f64 / 2., 0.25),
                            place((i + 1) as f64 / 2., -10.),
                        ])
                    })
                    .collect(),
            );
            let mut features = (0..512)
                .map(|i| {
                    (
                        i,
                        LineString::from(vec![
                            place(100. + i as f64 * 20., 0.),
                            place(110. + i as f64 * 20., 0.),
                        ])
                        .into(),
                    )
                })
                .collect::<Vec<(i64, geo::Geometry)>>();
            if self_group {
                let mut parts = b.0;
                parts.push(a);
                features.push((-1, geo::MultiLineString(parts).into()));
            } else {
                features.extend([(-1, a.into()), (-2, b.into())]);
            }
            let layer = t.add_layer(Layer::from_features("support", features));
            let g = t.graph(vec![layer]).unwrap();
            PASSAGE_ROLE_SLOTS.with(|n| n.set(0));
            assert!(g.selected_matches(None).next().is_some());
            assert_eq!(
                PASSAGE_ROLE_SLOTS.with(|n| n.get()),
                if self_group { 7 } else { 0 }
            );
        }
        // Independent short pairs enter restoration but cannot gain multipart support.
        let mut t = Topology::new(1.).unwrap();
        let shapes = (0..512).flat_map(|i| {
            let x = i as f64 * 20.;
            [
                LineString::from(vec![(x, 0.), (x + 0.5, 0.), (x + 0.5, 10.)]),
                LineString::from(vec![(x, 0.25), (x + 0.5, 0.25), (x + 0.5, -10.)]),
            ]
            .map(geo::Geometry::from)
        });
        let layer = t.add_layer(Layer::from_geometries("sparse", shapes));
        let g = t.graph(vec![layer]).unwrap();
        PASSAGE_ROLE_SLOTS.with(|n| n.set(0));
        assert!(g.selected_matches(None).next().is_none());
        assert_eq!(PASSAGE_ROLE_SLOTS.with(|n| n.get()), 0);
    }

    #[test]
    fn parallel_non_finite_and_empty_inputs_have_no_exact_crossing_or_collapse() {
        let line = |a: (f64, f64), b: (f64, f64)| {
            Line::new(Coord { x: a.0, y: a.1 }, Coord { x: b.0, y: b.1 })
        };
        assert!(exact_crossing(line((0., 0.), (1., 0.)), line((0., 1.), (1., 1.))).is_none());
        assert!(exact_crossing(line((0., 0.), (f64::NAN, 0.)), line((0., 1.), (1., 0.))).is_none());
        assert!(exact_crossing(line((0., 0.), (1., 1.)), line((0., 1.), (1., 0.))).is_some());
        assert!(collapsed(&LineString(vec![]), 1.));
    }

    #[test]
    fn points_at_a_distance_fall_back_to_the_segment_start_when_it_has_no_width() {
        let mut t = Topology::new(0.).unwrap();
        let layer = t.add_layer(Layer::from_geometries(
            "line",
            [LineString::from(vec![(0., 0.), (1., 0.), (2., 0.)]).into()],
        ));
        let g = t.graph(vec![layer]).unwrap();
        // The last segment is shorter than the spacing of the traversal distances.
        let at = |position| g.point_at(0, &[0., 1., 1.], position);
        assert_eq!(at(1.), Some(Coord { x: 1., y: 0. }));
        assert_eq!(at(0.5), Some(Coord { x: 0.5, y: 0. }));
        assert_eq!(at(2.), None);
        assert_eq!(g.point_at(0, &[], 0.), None);
    }

    #[test]
    fn matched_pairs_list_the_overlapping_matches_of_two_features() {
        let mut t = Topology::new(0.).unwrap();
        let layers = [
            vec![(0., 0.), (4., 0.)],
            vec![(2., 0.), (6., 0.)],
            vec![(10., 0.), (12., 0.)],
        ]
        .map(|line| {
            t.add_layer(Layer::from_geometries(
                "line",
                [LineString::from(line).into()],
            ))
        });
        let g = t.graph(layers.to_vec()).unwrap();
        assert_eq!(g.matched_pair((layers[0], 0), (layers[1], 0)).len(), 1);
        assert_eq!(g.matched_pair((layers[1], 0), (layers[0], 0)).len(), 1);
        assert!(g.matched_pair((layers[0], 0), (layers[2], 0)).is_empty());

        // Within tolerance but too short to be shared linework.
        let mut t = Topology::new(1.).unwrap();
        let layers = [
            vec![(0., 0.), (0.5, 0.), (0.5, 10.)],
            vec![(0., 0.25), (0.5, 0.25), (0.5, -10.)],
        ]
        .map(|line| {
            t.add_layer(Layer::from_geometries(
                "line",
                [LineString::from(line).into()],
            ))
        });
        let g = t.graph(layers.to_vec()).unwrap();
        assert!(!g.matches().is_empty());
        assert!(g.matched_pair((layers[0], 0), (layers[1], 0)).is_empty());
    }

    #[test]
    fn sustained_support_reuses_source_traversals_and_local_candidates() {
        let vertices = 2048;
        let mut t = Topology::new(1.).unwrap();
        let layer = t.add_layer(Layer::from_geometries(
            "nearby",
            [0., 0.25].map(|y| {
                LineString(
                    (0..vertices)
                        .map(|i| Coord {
                            x: i as f64 / 2.,
                            y,
                        })
                        .collect(),
                )
                .into()
            }),
        ));
        let g = t.graph(vec![layer]).unwrap();
        JUNCTION_SUPPORT_WORK.with(|n| n.set(0));
        assert!(g.selected_matches(None).next().is_some());
        assert!(JUNCTION_SUPPORT_WORK.with(|n| n.get()) < 100 * vertices);
    }

    #[test]
    fn source_junction_incidence_is_stable_under_translation() {
        let data: serde_json::Value =
            serde_json::from_str(include_str!("../../tests/fixtures/geometry_cases.json")).unwrap();
        let shapes = data["dc_junction_contacts"]["geometries"]
            .as_array()
            .unwrap();
        for start in [0, 2] {
            for shift in [false, true] {
                let mut t = Topology::new(0.001).unwrap();
                let layers = [start, start + 1].map(|index| {
                    let coords = shapes[index]["coordinates"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|p| {
                            let x = p[0].as_f64().unwrap() + if shift { 1048576. } else { 0. };
                            let y = p[1].as_f64().unwrap() - if shift { 524288. } else { 0. };
                            (x, y)
                        })
                        .collect::<Vec<_>>();
                    t.add_layer(Layer::from_geometries(
                        "source",
                        [LineString::from(coords).into()],
                    ))
                });
                let g = t.graph(layers.to_vec()).unwrap();
                assert!(g.matched_pair((layers[0], 0), (layers[1], 0)).is_empty());
                t.add_rule(
                    Rule::LineMustNotIntersectOrTouchInteriorWith,
                    layers[0],
                    Some(layers[1]),
                )
                .unwrap();
                let report = t.validate().unwrap();
                assert_eq!(report.violation_count(), 1);
                assert_eq!(
                    report.rules[0].violations[0].kind,
                    crate::ViolationKind::TouchInterior
                );
                let mut streamed = Vec::new();
                t.validate_into(|_, v| {
                    streamed.push(v);
                    Ok::<_, ()>(())
                })
                .unwrap();
                assert_eq!(streamed, report.rules[0].violations);
            }
        }
    }

    #[test]
    fn feature_streaming_visits_sparse_matches_and_canonical_directions_once() {
        for tolerance in [0., 0.001] {
            for rule in [
                Rule::LineMustNotOverlap,
                Rule::LineMustNotIntersect,
                Rule::LineMustNotIntersectOrTouchInterior,
                Rule::LineMustNotOverlapWith,
                Rule::LineMustNotIntersectWith,
                Rule::LineMustNotIntersectOrTouchInteriorWith,
            ] {
                let mut t = Topology::new(tolerance).unwrap();
                let lines = (0..512)
                    .map(|i| {
                        let x = (i / 2) as f64 * 20.;
                        let shift = (i % 2) as f64;
                        (
                            i as i64 * 7 - 91,
                            LineString::from(vec![(x + shift, 0.), (x + shift + 10., 0.)]).into(),
                        )
                    })
                    .collect::<Vec<_>>();
                let a = t.add_layer(Layer::from_features("primary", lines.clone()));
                let other = rule
                    .inputs()
                    .1
                    .map(|_| t.add_layer(Layer::from_features("target", lines)));
                t.add_rule(rule, a, other).unwrap();
                // Features run on pool threads; one thread keeps the count in one place.
                let pool = rayon::ThreadPoolBuilder::new()
                    .num_threads(1)
                    .build()
                    .unwrap();
                let mut streamed = Vec::new();
                let work = pool.install(|| {
                    MATCHING_WORK.with(|n| n.set(0));
                    t.validate_into(|_, v| {
                        streamed.push(v);
                        Ok::<_, ()>(())
                    })
                    .unwrap();
                    MATCHING_WORK.with(std::cell::Cell::get)
                });
                assert!(
                    work < 16_384,
                    "{rule}: {work} visits must be linear in sparse intervals"
                );
                assert!(!streamed.is_empty());
                assert_eq!(
                    streamed,
                    t.validate().unwrap().rules[0].violations,
                    "{rule}"
                );
            }
        }
    }

    #[test]
    fn ends_near_searches_each_x_in_the_window_by_y() {
        let [f, g] = [(LayerId(0), 1), (LayerId(0), 2)];
        let at = |x: f64, y: f64| Coord { x, y };
        let mut ends = vec![(g, at(0., 0.)), (f, at(1., 0.))];
        // Many ends on one x, as along a line their parts end on.
        ends.extend((0..1000).map(|i| (f, at(0., f64::from(i)))));
        ends.extend([
            (f, at(1e-9, 500.)),
            (f, at(-1e-9, 499.5)),
            (f, at(1e-9, 600.)),
        ]);
        ends.sort_by(|a, b| a.0.cmp(&b.0).then(coordinate_order(&a.1, &b.1)));
        let near = |point, window| ends_near(&ends, f, point, window).collect::<Vec<_>>();
        assert_eq!(
            near(at(0., 500.), 0.6),
            [at(-1e-9, 499.5), at(0., 500.), at(1e-9, 500.)]
        );
        assert_eq!(near(at(0., 0.), 0.), [at(0., 0.)]);
        assert!(near(at(0.5, 0.), 0.4).is_empty());
        assert_eq!(near(at(2., 0.), 1.), [at(1., 0.)]);
    }
}
#[cfg(test)]
mod reference;
impl GraphPart {
    pub fn closed(&self) -> bool {
        self.is_closed
    }
    pub fn endpoint(&self, node: usize) -> bool {
        !self.closed() && (self.nodes.first() == Some(&node) || self.nodes.last() == Some(&node))
    }
}
#[derive(Debug, Clone, Copy)]
pub(crate) struct EdgeUse {
    pub part: usize,
    pub source: usize,
}
#[derive(Debug)]
pub(crate) struct Edge {
    pub nodes: [usize; 2],
    pub uses: Range<usize>,
}
#[derive(Debug, Clone, Copy)]
pub(crate) struct Incident {
    pub part: usize,
}
#[derive(Debug)]
pub(crate) struct LineGraph {
    pub coords: Vec<Coord>,
    pub parts: Vec<GraphPart>,
    pub incident: Incidences,
    pub contacts: Contacts,
    sources: Vec<Segment>,
    matches: Cache<Vec<Matched>>,
    #[cfg(test)]
    match_pairs: Cache<MatchPairs>,
    match_features: Cache<BTreeMap<FeatureKey, Vec<usize>>>,
    canonical_directions: Cache<Vec<bool>>,
    pub tolerance: f64,
    contact_nodes: Cache<BTreeMap<FeatureKey, Vec<usize>>>,
    /// Sources in the search order of their index, to index them again
    /// without ordering, if their IDs fit.
    order: Option<Vec<u32>>,
}
#[derive(Debug, Clone, Copy)]
pub(crate) struct Matched {
    pub sources: [usize; 2],
    pub spans: [[f64; 2]; 2],
    /// Raw intervals still contribute to coverage before its final length test.
    pub overlap: bool,
}
/// Original incidences at one actual source contact, before node clustering.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Contact {
    pub parts: [usize; 2],
    pub interior: [bool; 2],
    pub points: [Coord; 2],
    pub sources: [usize; 2],
}
/// Contacts by node in node order, each node's in the order found.
#[derive(Debug)]
pub(crate) struct Contacts {
    nodes: Vec<usize>,
    /// Where the contacts of each node start, and where the last end.
    offsets: Vec<usize>,
    values: Vec<Contact>,
}
impl Contacts {
    /// Nodes with contacts, in order.
    pub fn keys(&self) -> std::slice::Iter<'_, usize> {
        self.nodes.iter()
    }
    pub fn get(&self, node: &usize) -> Option<&[Contact]> {
        let i = self.nodes.binary_search(node).ok()?;
        Some(&self.values[self.offsets[i]..self.offsets[i + 1]])
    }
    pub fn iter(&self) -> impl Iterator<Item = (usize, &[Contact])> {
        let lists = self.offsets.windows(2).map(|o| &self.values[o[0]..o[1]]);
        self.nodes.iter().copied().zip(lists)
    }
}
/// Compact contiguous incidences; construction allocation is released on caching.
#[derive(Debug)]
pub(crate) struct Incidences {
    offsets: Vec<usize>,
    values: Vec<Incident>,
}
impl Incidences {
    pub fn iter(&self) -> impl Iterator<Item = &[Incident]> {
        self.offsets.windows(2).map(|o| &self.values[o[0]..o[1]])
    }
}
impl std::ops::Index<usize> for Incidences {
    type Output = [Incident];
    fn index(&self, i: usize) -> &Self::Output {
        &self.values[self.offsets[i]..self.offsets[i + 1]]
    }
}
#[derive(Debug, Clone, Copy)]
pub(crate) struct Segment {
    pub part: usize,
    pub start: usize,
}
impl Segment {
    fn line(self, parts: &[GraphPart]) -> Line {
        let coords = &parts[self.part].original.0;
        Line::new(coords[self.start], coords[self.start + 1])
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum InputMode {
    Linework,
    PolygonBoundary,
}

/// Project onto a segment without squaring its unscaled length.
pub(crate) fn projection(line: Line, point: Coord) -> (f64, Coord) {
    let dx = line.end.x - line.start.x;
    let dy = line.end.y - line.start.y;
    let scale = dx.abs().max(dy.abs());
    if scale == 0. {
        return (0., line.start);
    }
    let x = dx / scale;
    let y = dy / scale;
    let parameter = (((point.x - line.start.x) / scale * x + (point.y - line.start.y) / scale * y)
        / (x * x + y * y))
        .clamp(0., 1.);
    (
        parameter,
        if parameter == 0. {
            line.start
        } else if parameter == 1. {
            line.end
        } else {
            Coord {
                x: line.start.x + parameter * dx,
                y: line.start.y + parameter * dy,
            }
        },
    )
}
/// About sixteen units in the last place of the largest coordinate: the reach of
/// rounding in a computation from these input coordinates, however they were translated.
pub(crate) fn rounding(points: &[Coord]) -> f64 {
    points
        .iter()
        .fold(0_f64, |m, p| m.max(p.x.abs()).max(p.y.abs()))
        * f64::EPSILON
        * 16.
}
fn parameter(line: Line, point: Coord) -> f64 {
    let dx = line.end.x - line.start.x;
    let dy = line.end.y - line.start.y;
    if dx.abs() >= dy.abs() {
        (point.x - line.start.x) / dx
    } else {
        (point.y - line.start.y) / dy
    }
}

/// Round the exact intersection of the input binary floats once. Pairwise floating point
/// formulas can otherwise round a concurrent crossing differently for each segment pair,
/// creating spurious tiny edges even at zero user tolerance. Endpoint contacts retain their
/// original coordinates and do not pay for rational arithmetic.
pub(crate) fn proper_intersection(a: Line, b: Line) -> Option<Coord> {
    let (_, [x, y]) = exact_crossing(a, b)?;
    Some(Coord {
        x: x.to_f64()?,
        y: y.to_f64()?,
    })
}
/// The exact crossing of the lines through `a` and `b`, from their binary
/// floats: its parameter along `a` and its rational coordinates. `None` for
/// parallel or non-finite input.
pub(crate) fn exact_crossing(a: Line, b: Line) -> Option<(BigRational, [BigRational; 2])> {
    let [ax, ay, bx, by, cx, cy, dx, dy] = [
        a.start.x, a.start.y, a.end.x, a.end.y, b.start.x, b.start.y, b.end.x, b.end.y,
    ]
    .map(BigRational::from_float);
    let (ax, ay, bx, by, cx, cy, dx, dy) = (ax?, ay?, bx?, by?, cx?, cy?, dx?, dy?);
    let rx = bx - &ax;
    let ry = by - &ay;
    let sx = dx - &cx;
    let sy = dy - &cy;
    let determinant = &rx * &sy - &ry * &sx;
    if determinant == BigRational::from_integer(0.into()) {
        return None;
    }
    let t = ((cx - &ax) * sy - (cy - &ay) * sx) / determinant;
    let point = [ax + &t * rx, ay + &t * ry];
    Some((t, point))
}
pub(crate) fn collapsed(line: &LineString, tolerance: f64) -> bool {
    if tolerance == 0. {
        return false;
    }
    let Some(bounds) = line.bounding_rect() else {
        return true;
    };
    if bounds.width() > tolerance || bounds.height() > tolerance {
        return false;
    }
    line.0.iter().enumerate().all(|(i, &p)| {
        checkpoint("collapsed");
        line.0[i + 1..].iter().all(|&q| distance(p, q) <= tolerance)
    })
}
/// One visit of a part's node path, with the first and last original points
/// it covers, their segment indices, and the edge arriving at it.
struct Visit {
    node: usize,
    first: (Coord, usize),
    last: (Coord, usize),
    edge: Option<([usize; 2], EdgeUse)>,
}
/// A part whose node path can drop returns to its end nodes.
struct Trim {
    part: usize,
    /// First and last kept visits.
    bounds: [usize; 2],
    /// Raw edge arriving at each visit.
    edges: Vec<Option<usize>>,
}
/// Most vertices within twice the tolerance of a line end for which returns
/// to the end are tested, at a cost of their square. Ends with more are left
/// as noded.
const END_VERTICES: usize = 64;
/// Whether a line end has not returned to its own line. `vertex(0)` is the far
/// end and `vertex(n - 1)` this end; segment `k` joins `vertex(k)` and
/// `vertex(k + 1)`. From the nearest point of every own segment within
/// tolerance of the end, and from each given point on segment `k`, the line
/// must reach the end within tolerance of the chord between them. Distance to
/// a segment is convex along a segment, so testing the vertices suffices.
fn free_end(
    vertex: impl Fn(usize) -> Coord,
    n: usize,
    points: [(Coord, usize); 2],
    tolerance: f64,
) -> bool {
    let end = vertex(n - 1);
    let chord = |point: Coord, k: usize| {
        let chord = Line::new(point, end);
        (k + 1..n).all(|i| {
            every(i - k - 1, "returns.chord");
            let p = vertex(i);
            distance(p, projection(chord, p).1) <= tolerance
        })
    };
    // Each chord lies within the tolerance of the end, so a vertex farther than
    // twice the tolerance after a nearby segment is a genuine return.
    let mut near = n - 1;
    while near > 0 && distance(vertex(near - 1), end) <= 2. * tolerance {
        every(n - 1 - near, "returns.near");
        near -= 1;
    }
    n - near <= END_VERTICES
        && points.into_iter().all(|(point, k)| chord(point, k))
        && (0..n - 1).all(|k| {
            every(k, "returns.segments");
            let (_, p) = projection(Line::new(vertex(k), vertex(k + 1)), end);
            distance(p, end) > tolerance || (k + 1 >= near && chord(p, k))
        })
}
/// Kept visits of an open part's node path. A path that reaches an end node
/// earlier, but has not returned to itself, ends there: the rest belongs to
/// the end node, as a final segment within tolerance does. Otherwise a sharp
/// final bend just longer than the tolerance would fold back as a retrace
/// through a three-way node. The earliest revisit is tested from its first and
/// last points.
fn kept_visits(path: &[Visit], coords: &[Coord], tolerance: f64) -> [usize; 2] {
    let n = path.len();
    let mut bounds = [0, n - 1];
    if n < 3 || path[0].node == path[n - 1].node {
        return bounds;
    }
    let m = coords.len();
    if let Some(j) = path[1..n - 1].iter().enumerate().position(|(k, v)| {
        every(k, "returns.last");
        v.node == path[n - 1].node
    }) && free_end(
        |i| coords[i],
        m,
        [path[j + 1].first, path[j + 1].last],
        tolerance,
    ) {
        bounds[1] = j + 1;
    }
    // From the start, points on segment `k` lie on reversed segment `m - 2 - k`.
    let reversed = |(point, k): (Coord, usize)| (point, m - 2 - k);
    if let Some(i) = path[1..n - 1].iter().enumerate().rposition(|(k, v)| {
        every(k, "returns.first");
        v.node == path[0].node
    }) && i + 1 < bounds[1]
        && free_end(
            |i| coords[m - 1 - i],
            m,
            [reversed(path[i + 1].first), reversed(path[i + 1].last)],
            tolerance,
        )
    {
        bounds[0] = i + 1;
    }
    bounds
}
/// Remove trimmed visits and their edges, unless one of their nodes is an end
/// node of another part or of the part's other end. Dangles and pseudo-nodes
/// of other lines are then unchanged; their edges and contacts are untouched.
/// An end that also lies within tolerance of another part, at a crack point
/// outside its node, keeps its path: it still touches that part.
fn trim_returns(
    parts: &[GraphPart],
    list: &[usize],
    spans: &mut [Range<usize>],
    raw_edges: &mut Vec<([usize; 2], EdgeUse)>,
    trims: Vec<Trim>,
    ties: &HashMap<(usize, bool), Vec<usize>>,
) {
    // Removed visits before the kept start and after the kept end, without
    // the end visits themselves.
    let removed = |trim: &Trim, n: usize| {
        [
            1..trim.bounds[0].max(1),
            (trim.bounds[1] + 1).min(n - 1)..n - 1,
        ]
    };
    // Parts ending at a removed node.
    let mut ends = HashMap::<usize, Vec<usize>>::new();
    for trim in &trims {
        let nodes = &list[spans[trim.part].clone()];
        for range in removed(trim, nodes.len()) {
            for (k, &node) in nodes[range].iter().enumerate() {
                every(k, "trim.removed");
                ends.entry(node).or_default();
            }
        }
    }
    for (id, part) in parts.iter().enumerate() {
        every(id, "trim.ends");
        if part.is_closed {
            continue;
        }
        let nodes = &list[spans[id].clone()];
        for node in [nodes.first(), nodes.last()].into_iter().flatten() {
            if let Some(ends) = ends.get_mut(node)
                && !ends.contains(&id)
            {
                ends.push(id);
            }
        }
    }
    let mut dropped = Vec::<usize>::new();
    for trim in trims {
        checkpoint("trim.parts");
        let span = spans[trim.part].clone();
        let nodes = &list[span.clone()];
        let n = nodes.len();
        let [mut start, mut end] = removed(&trim, n);
        let [first, last] = [nodes[0], nodes[n - 1]];
        let own = |i: &usize| {
            checkpoint("trim.own");
            ends[&nodes[*i]].iter().all(|&p| p == trim.part)
        };
        let tied = |end: bool, at: usize| {
            ties.get(&(trim.part, end))
                .is_some_and(|nodes| nodes.iter().any(|&node| node != at))
        };
        let mut bounds = [0, n - 1];
        if !tied(true, last) && end.all(|i| nodes[i] == last || (nodes[i] != first && own(&i))) {
            bounds[1] = trim.bounds[1];
            dropped.extend(trim.edges[bounds[1] + 1..].iter().flatten());
        }
        if !tied(false, first) && start.all(|i| nodes[i] == first || (nodes[i] != last && own(&i)))
        {
            bounds[0] = trim.bounds[0];
            dropped.extend(trim.edges[1..=bounds[0]].iter().flatten());
        }
        if bounds != [0, n - 1] {
            spans[trim.part] = span.start + bounds[0]..span.start + bounds[1] + 1;
        }
    }
    parallel::sort(&mut dropped, "trim.dropped", Ord::cmp);
    let mut next = dropped.into_iter().peekable();
    let mut index = 0;
    raw_edges.retain(|_| {
        every(index, "trim.edges");
        let keep = next.next_if_eq(&index).is_none();
        index += 1;
        keep
    });
}
fn coordinate_order(a: &Coord, b: &Coord) -> std::cmp::Ordering {
    a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y))
}
/// An open part's end, with its segment and its vertex number.
struct End {
    point: Coord,
    source: usize,
    vertex: usize,
    last: bool,
}
/// A point found on a segment: one of its vertices, numbered across all
/// parts, or a cut of its interior found by the same task.
#[derive(Debug, Clone, Copy)]
enum At {
    Vertex(usize),
    Cut(usize),
}
/// Contacts, cuts and crack points found by one task, in discovery order.
#[derive(Default)]
struct Found {
    contacts: Vec<(At, Contact)>,
    /// Segment and point of each cut.
    cuts: Vec<(usize, Coord)>,
    cracked: Vec<Coord>,
    ties: Vec<((usize, bool), Vec<At>)>,
}
impl Found {
    /// Cut segment `source`, whose first vertex is `vertex`, at `point`.
    fn cut(&mut self, source: usize, vertex: usize, line: Line, point: Coord) -> At {
        if point == line.start {
            At::Vertex(vertex)
        } else if point == line.end {
            At::Vertex(vertex + 1)
        } else {
            self.cuts.push((source, point));
            At::Cut(self.cuts.len() - 1)
        }
    }
    fn contact(&mut self, contact: Option<Contact>, at: At) {
        if let Some(contact) = contact {
            self.contacts.push((at, contact));
        }
    }
}
/// Runs of whole parts with similar numbers of segments, from the first
/// segment of each part followed by the number of segments.
fn part_ranges(workers: &Workers, starts: &[usize]) -> Vec<Range<usize>> {
    let parts = starts.len() - 1;
    let mut out = Vec::new();
    let mut first = 0;
    for range in workers.ranges(starts[parts]) {
        let end = starts[..parts].partition_point(|&s| s < range.end);
        if end > first {
            out.push(first..end);
            first = end;
        }
    }
    if first < parts {
        out.push(first..parts);
    }
    out
}
/// Ends of `feature` within `window` of `point` on both axes, from ends sorted by
/// feature and [`coordinate_order`] without negative zeros: one search over y for
/// each distinct x in the window.
fn ends_near(
    ends: &[(FeatureKey, Coord)],
    feature: FeatureKey,
    point: Coord,
    window: f64,
) -> impl Iterator<Item = Coord> + '_ {
    let mut next = ends.partition_point(|&(f, end)| (f, end.x) < (feature, point.x - window));
    std::iter::from_fn(move || {
        let &(_, first) = ends
            .get(next)
            .filter(|&&(f, end)| f == feature && end.x <= point.x + window)?;
        let run = &ends[next..];
        let run = &run[..run.partition_point(|&(f, end)| f == feature && end.x == first.x)];
        next += run.len();
        let low = run.partition_point(|&(_, end)| end.y < point.y - window);
        let high = run.partition_point(|&(_, end)| end.y <= point.y + window);
        Some(run[low..high].iter().map(|&(_, end)| end))
    })
    .flatten()
}
impl LineGraph {
    pub fn canonical_reversed(&self, part: usize) -> bool {
        self.canonical_directions.get_or_init(|| {
            self.parts
                .iter()
                .map(|p| {
                    checkpoint("directions");
                    #[cfg(test)]
                    MATCHING_WORK.with(|n| n.set(n.get() + p.original.0.len()));
                    p.original
                        .0
                        .iter()
                        .zip(p.original.0.iter().rev())
                        .map(|(a, b)| coordinate_order(a, b))
                        .find(|o| !o.is_eq())
                        .is_some_and(|o| o.is_gt())
                })
                .collect()
        })[part]
    }
    /// Streaming keeps only the selected feature's intervals in its working set.
    pub fn selected_matches(
        &self,
        feature: Option<FeatureKey>,
    ) -> Box<dyn Iterator<Item = &Matched> + '_> {
        let Some(feature) = feature else {
            return Box::new(self.matches().iter().filter(|m| m.overlap));
        };
        let ids = self.match_features.get_or_init(|| {
            let mut features = BTreeMap::<_, Vec<_>>::new();
            for (id, m) in self.matches().iter().enumerate() {
                every(id, "matches.features");
                if !m.overlap {
                    continue;
                }
                let [a, b] = m
                    .sources
                    .map(|s| self.parts[self.segment(s).part].reference.feature());
                features.entry(a).or_default().push(id);
                if a != b {
                    features.entry(b).or_default().push(id);
                }
            }
            features
        });
        Box::new(
            ids.get(&feature)
                .into_iter()
                .flatten()
                .map(|&id| &self.matches()[id]),
        )
    }
    #[cfg(test)]
    pub fn matched_pair(&self, a: (LayerId, i64), b: (LayerId, i64)) -> &[usize] {
        let key = if a < b { (a, b) } else { (b, a) };
        self.match_pairs
            .get_or_init(|| {
                let mut pairs = BTreeMap::<_, Vec<_>>::new();
                for (i, m) in self.matches().iter().enumerate() {
                    if !m.overlap {
                        continue;
                    }
                    let [a, b] = m
                        .sources
                        .map(|s| self.parts[self.segment(s).part].reference.feature());
                    let key = if a < b { (a, b) } else { (b, a) };
                    pairs.entry(key).or_default().push(i);
                }
                pairs
            })
            .get(&key)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }
    pub fn original(&self, part: usize) -> &LineString {
        &self.parts[part].original
    }
    pub fn segment(&self, source: usize) -> Segment {
        self.sources[source]
    }
    pub fn matches(&self) -> &[Matched] {
        self.matches.get_or_init(|| {
            let workers = Workers::new(self.sources.len());
            let tree = self.source_index(&workers);
            let found = tree.pairs(
                &workers,
                self.tolerance,
                "matches.pairs",
                |_| true,
                |a, b, out| {
                    if let Some(support) =
                        super::matching::evidence(self.source(a), self.source(b), self.tolerance)
                    {
                        let matched = Matched {
                            sources: [a, b],
                            spans: support.spans,
                            overlap: true,
                        };
                        out.push((matched, support));
                    }
                },
            );
            let (mut result, evidence): (Vec<_>, Vec<_>) = found.into_iter().unzip();
            if self.tolerance > 0. && !result.is_empty() {
                self.retain_sustained_matches(&tree, &workers, &mut result, &evidence);
            }
            drop(evidence);
            // Each pair of sources is matched once.
            workers.sort(&mut result, "matches.sort", |a, b| {
                a.sources.cmp(&b.sources)
            });
            result
        })
    }
    /// The index of construction again, in its search order when known.
    fn source_index(&self, workers: &Workers) -> Packed {
        let entry = |i: usize| Entry::new(i, self.source(i).bounding_rect());
        let mut entries = parallel::repeated(Entry::default(), self.sources.len(), "matches.index");
        match &self.order {
            Some(order) => {
                workers.fill(&mut entries, |k| entry(order[k] as usize));
                Packed::ordered(entries, workers)
            }
            None => {
                workers.fill(&mut entries, entry);
                Packed::bulk(entries, workers)
            }
        }
    }

    /// Filter connected original support, before output endpoint caps are used.
    /// A point junction can cover a short passage, but cannot establish overlap.
    fn retain_sustained_matches(
        &self,
        tree: &impl Search,
        workers: &Workers,
        matches: &mut [Matched],
        evidence: &[super::matching::Evidence],
    ) {
        let mut groups = BTreeMap::<_, Vec<_>>::new();
        for (i, m) in matches.iter().enumerate() {
            checkpoint("retain.groups");
            #[cfg(test)]
            JUNCTION_SUPPORT_WORK.with(|n| n.set(n.get() + 1));
            let parts = m.sources.map(|s| self.segment(s).part);
            let short = [0, 1].iter().any(|&side| {
                let line = self.source(m.sources[side]);
                (evidence[i].support[side][1] - evidence[i].support[side][0])
                    * distance(line.start, line.end)
                    <= self.tolerance
            });
            // Passages of one part that meet through a loop are judged as runs too.
            if !self.consecutive(m.sources[0], m.sources[1]) && (!evidence[i].exact || short) {
                groups.entry(parts).or_default();
            }
        }
        if groups.is_empty() {
            return;
        }
        for (i, m) in matches.iter().enumerate() {
            checkpoint("retain.ids");
            let parts = m.sources.map(|s| self.segment(s).part);
            if !self.consecutive(m.sources[0], m.sources[1])
                && let Some(ids) = groups.get_mut(&parts)
            {
                ids.push(i);
            }
        }
        // Original traversal distances distinguish a compact bent run from a
        // short junction approach. This temporary index is built once per part.
        let mut positions = vec![Vec::new(); self.parts.len()];
        let mut uncertainty = vec![0.; self.parts.len()];
        for parts in groups.keys() {
            for &part in parts {
                if !positions[part].is_empty() {
                    continue;
                }
                let mut total = 0.;
                let mut bounds = super::certified::Interval(0., 0.);
                positions[part].push(total);
                for line in self.original(part).lines() {
                    checkpoint("retain.positions");
                    #[cfg(test)]
                    JUNCTION_SUPPORT_WORK.with(|n| n.set(n.get() + 1));
                    total += distance(line.start, line.end);
                    use super::certified::Interval;
                    let x =
                        Interval(line.end.x, line.end.x).sub(Interval(line.start.x, line.start.x));
                    let y =
                        Interval(line.end.y, line.end.y).sub(Interval(line.start.y, line.start.y));
                    bounds = bounds.add(x.square().add(y.square()).sqrt());
                    uncertainty[part] = bounds.1 - bounds.0;
                    if !total.is_finite()
                        || !uncertainty[part].is_finite()
                        || uncertainty[part] >= self.tolerance
                    {
                        uncertainty[part] = f64::INFINITY;
                        break;
                    }
                    positions[part].push(total);
                }
            }
        }
        // Uncertifiable cumulative distances cannot reject original evidence.
        // This includes overflow and short intervals after very large prefixes.
        groups.retain(|parts, _| parts.iter().all(|&part| uncertainty[part].is_finite()));
        if groups.is_empty() {
            return;
        }
        let along = |source: usize, parameter: f64| {
            let segment = self.segment(source);
            let p = &positions[segment.part];
            p[segment.start] + parameter * (p[segment.start + 1] - p[segment.start])
        };
        enum Found {
            Meeting([usize; 2], Meeting),
            Junction([usize; 2], [f64; 2]),
        }
        // Pairs whose first part starts no group add nothing.
        let mut first = vec![false; self.parts.len()];
        for parts in groups.keys() {
            first[parts[0]] = true;
        }
        let found = tree.pairs(
            workers,
            self.tolerance,
            "matches.retain",
            |a| first[self.segment(a).part],
            |a, b, out| {
                #[cfg(test)]
                JUNCTION_SUPPORT_WORK.with(|n| n.set(n.get() + 1));
                let parts = [self.segment(a).part, self.segment(b).part];
                if !groups.contains_key(&parts) || self.consecutive(a, b) {
                    return;
                }
                let lines = [self.source(a), self.source(b)];
                let intersection = line_intersection(lines[0], lines[1]);
                // Points where the lines meet: they intersect or touch, or come within
                // a small fraction of the tolerance at a vertex, as a junction vertex
                // stored with rounding differences or a small digitizing gap does.
                let mut points = match intersection {
                    Some(LineIntersection::SinglePoint { intersection, .. }) => vec![intersection],
                    Some(LineIntersection::Collinear { intersection }) => {
                        vec![intersection.start, intersection.end]
                    }
                    None => Vec::new(),
                };
                for side in 0..2 {
                    for p in [lines[side].start, lines[side].end] {
                        if distance(p, projection(lines[1 - side], p).1) <= self.tolerance / 16. {
                            points.push(p);
                        }
                    }
                }
                for p in points {
                    let meeting = Meeting {
                        positions: [
                            along(a, projection(lines[0], p).0),
                            along(b, projection(lines[1], p).0),
                        ],
                        end: None,
                    };
                    out.push(Found::Meeting(parts, meeting));
                }
                if let Some(LineIntersection::SinglePoint {
                    intersection,
                    is_proper: false,
                }) = intersection
                {
                    out.push(Found::Junction(
                        parts,
                        [
                            along(a, projection(lines[0], intersection).0),
                            along(b, projection(lines[1], intersection).0),
                        ],
                    ));
                }
                for side in 0..2 {
                    let part = &self.parts[parts[side]];
                    if part.is_closed {
                        continue;
                    }
                    let source = self.segment([a, b][side]);
                    for (position, p) in [
                        (source.start, lines[side].start),
                        (source.start + 1, lines[side].end),
                    ] {
                        if position != 0 && position != part.original.0.len() - 1 {
                            continue;
                        }
                        let (parameter, projected) = projection(lines[1 - side], p);
                        if distance(p, projected) <= self.tolerance {
                            let mut contact = [0.; 2];
                            contact[side] = positions[parts[side]][position];
                            contact[1 - side] = along([a, b][1 - side], parameter);
                            out.push(Found::Junction(parts, contact));
                        }
                    }
                }
            },
        );
        let mut junctions = BTreeMap::<_, Vec<[f64; 2]>>::new();
        let mut meetings = BTreeMap::<_, Vec<Meeting>>::new();
        for (k, found) in found.into_iter().enumerate() {
            every(k, "retain.found");
            match found {
                Found::Meeting(parts, meeting) => meetings.entry(parts).or_default().push(meeting),
                Found::Junction(parts, junction) => {
                    junctions.entry(parts).or_default().push(junction);
                }
            }
        }
        // Several segment pairs report the same meeting point.
        // Meetings found so far have no end, so those that compare equal are the same.
        for list in meetings.values_mut() {
            checkpoint("retain.meetings");
            parallel::sort(list, "retain.meetings.sort", |a, b| {
                (a.positions[0].total_cmp(&b.positions[0]))
                    .then(a.positions[1].total_cmp(&b.positions[1]))
            });
            list.dedup_by(|a, b| a.positions == b.positions);
        }
        let mut rejected = vec![false; matches.len()];
        let mut explained = vec![false; matches.len()];
        fn root(parent: &mut [usize], mut i: usize) -> usize {
            while parent[i] != i {
                parent[i] = parent[parent[i]];
                i = parent[i];
            }
            i
        }
        for (parts, ids) in groups {
            checkpoint("retain.runs");
            let mut parent = (0..ids.len()).collect::<Vec<_>>();
            for (side, &part) in parts.iter().enumerate() {
                let mut spans = ids
                    .iter()
                    .enumerate()
                    .map(|(local, &id)| {
                        checkpoint("retain.spans");
                        let source = matches[id].sources[side];
                        let start = self.segment(source).start as f64;
                        let span = evidence[id].support[side];
                        (start + span[0], start + span[1], local)
                    })
                    .collect::<Vec<_>>();
                parallel::sort(&mut spans, "retain.spans.sort", |a, b| a.0.total_cmp(&b.0));
                if self.parts[part].is_closed {
                    let first = spans.first().expect("support");
                    let last = spans
                        .iter()
                        .max_by(|a, b| a.1.total_cmp(&b.1))
                        .expect("support");
                    let total = (self.original(part).0.len() - 1) as f64;
                    if first.0 == 0. && last.1 == total {
                        let a = root(&mut parent, first.2);
                        let b = root(&mut parent, last.2);
                        parent[a] = b;
                    }
                }
                let mut previous: Option<(f64, usize)> = None;
                for (lo, hi, id) in spans {
                    checkpoint("retain.spans.join");
                    if let Some((end, before)) = previous
                        && lo <= end
                    {
                        let a = root(&mut parent, id);
                        let b = root(&mut parent, before);
                        parent[a] = b;
                        previous = Some((end.max(hi), before));
                    } else {
                        previous = Some((hi, id));
                    }
                }
            }
            let mut components = BTreeMap::<_, Vec<_>>::new();
            for (local, &id) in ids.iter().enumerate() {
                checkpoint("retain.components");
                components
                    .entry(root(&mut parent, local))
                    .or_default()
                    .push(id);
            }
            let contacts = junctions.entry(parts).or_default();
            parallel::sort(contacts, "retain.junctions.sort", |a, b| {
                a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1]))
            });
            contacts.dedup();
            let entries = contacts
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    every(i, "retain.junctions");
                    let c = Coord { x: p[0], y: p[1] };
                    Entry::new(i, geo::Rect::new(c, c))
                })
                .collect::<Vec<_>>();
            // Only whether a contact is found counts, not the order of the search.
            let contact_index = Packed::bulk(entries, &Workers::new(contacts.len()));
            let mut ends_met = false;
            for component in components.into_values() {
                checkpoint("retain.support");
                let spans = [0, 1].map(|side| {
                    super::matching::union(
                        component
                            .iter()
                            .map(|&id| {
                                checkpoint("retain.support.spans");
                                #[cfg(test)]
                                JUNCTION_SUPPORT_WORK.with(|n| n.set(n.get() + 1));
                                let source = matches[id].sources[side];
                                evidence[id].support[side].map(|p| along(source, p))
                            })
                            .collect(),
                    )
                });
                // A longer output on one representative cannot manufacture
                // longitudinal support missing from the other original passage.
                let short = [0, 1].iter().any(|&side| {
                    spans[side].iter().map(|s| s[1] - s[0]).sum::<f64>()
                        + 4. * uncertainty[parts[side]] * spans[side].len() as f64
                        <= self.tolerance
                });
                let exact = [0, 1].iter().all(|&side| {
                    let spans = super::matching::union(
                        component
                            .iter()
                            .filter(|&&id| evidence[id].exact)
                            .map(|&id| {
                                evidence[id].support[side]
                                    .map(|p| along(matches[id].sources[side], p))
                            })
                            .collect(),
                    );
                    spans.iter().map(|s| s[1] - s[0]).sum::<f64>()
                        - 4. * uncertainty[parts[side]] * spans.len() as f64
                        > self.tolerance
                });
                let bounds = [0, 1].map(|side| {
                    let s = &spans[side];
                    if !self.parts[parts[side]].is_closed {
                        return [s[0][0], s.last().expect("support")[1]];
                    }
                    let total = *positions[parts[side]].last().expect("source");
                    // Cut the cycle in its largest unsupported gap. A run
                    // across the source seam then has the same bounds after rotation.
                    let mut gap = s[0][0] + total - s.last().expect("support")[1];
                    let mut cut = s[0][0];
                    for pair in s.windows(2) {
                        let candidate = pair[1][0] - pair[0][1];
                        if candidate > gap {
                            gap = candidate;
                            cut = pair[1][0];
                        }
                    }
                    [cut, cut + total - gap]
                });
                let mut confined = false;
                if !short && !exact {
                    for side in 0..2 {
                        let margin = 4. * uncertainty[parts[side]];
                        let low = bounds[side][1] - self.tolerance + margin;
                        let high = bounds[side][0] + self.tolerance - margin;
                        if low > high {
                            continue;
                        }
                        let other = 1 - side;
                        let total = *positions[parts[other]].last().expect("source");
                        let side_total = *positions[parts[side]].last().expect("source");
                        let offsets: &[f64] = if self.parts[parts[side]].is_closed {
                            &[0., -side_total, side_total]
                        } else {
                            &[0.]
                        };
                        for &offset in offsets {
                            let a_low = (low + offset).max(0.);
                            let b_high = (high + offset).min(side_total);
                            if a_low > b_high {
                                continue;
                            }
                            let mut a = Coord { x: 0., y: 0. };
                            let mut b = Coord { x: 0., y: 0. };
                            if side == 0 {
                                a.x = a_low;
                                b.x = b_high;
                                b.y = total;
                            } else {
                                a.y = a_low;
                                b.y = b_high;
                                b.x = total;
                            }
                            confined = contact_index
                                .locate(index::envelope(geo::Rect::new(a, b)))
                                .any(|item| {
                                    checkpoint("retain.confined");
                                    let mut p = contacts[item.id][other];
                                    if self.parts[parts[other]].is_closed && p < bounds[other][0] {
                                        p += total;
                                    }
                                    // Shared support continuing on both sides of a
                                    // junction is a run, rather than a one-sided cap.
                                    p < bounds[other][0] - 4. * uncertainty[parts[other]]
                                        || p > bounds[other][1] + 4. * uncertainty[parts[other]]
                                });
                            if confined {
                                break;
                            }
                        }
                        if confined {
                            break;
                        }
                    }
                }
                // Lines that meet at an angle stay within tolerance near the
                // meeting point. Only support beyond those near zones can make a
                // run shared linework. Without a meeting point nothing changes.
                let near = !short && !exact && {
                    // Line ends within tolerance of the other part meet it too.
                    if !ends_met {
                        let ends = self.end_meetings(tree, parts, &positions);
                        meetings.entry(parts).or_default().extend(ends);
                        ends_met = true;
                    }
                    self.near_zones_explain(
                        tree,
                        parts,
                        &positions,
                        &uncertainty,
                        &spans,
                        &meetings[&parts],
                    )
                };
                // Within one part, only a meeting point can bound a run.
                let judged = parts[0] != parts[1] && (short || confined);
                if judged || near {
                    for &id in &component {
                        rejected[id] = true;
                        // Other parts cannot restore what a meeting point explains.
                        explained[id] = near;
                    }
                }
            }
        }
        self.retain_connected_part_support(matches, evidence, &mut rejected, &explained);
        for (m, rejected) in matches.iter_mut().zip(rejected) {
            m.overlap = !rejected;
        }
    }
    /// Several source parts can contribute one connected shared passage.
    /// Restore those contributions only when both features supply a long run
    /// without relying on the endpoint caps used for output geometry.
    fn retain_connected_part_support(
        &self,
        matches: &[Matched],
        evidence: &[super::matching::Evidence],
        rejected: &mut [bool],
        explained: &[bool],
    ) {
        if !rejected.iter().any(|&r| r) {
            return;
        }
        let key = |m: &Matched| {
            m.sources
                .map(|s| self.parts[self.segment(s).part].reference.feature())
        };
        let mut groups = BTreeMap::<_, Vec<_>>::new();
        for (i, m) in matches.iter().enumerate() {
            checkpoint("support.groups");
            if rejected[i] && !explained[i] {
                groups.entry(key(m)).or_default();
            }
        }
        for (i, m) in matches.iter().enumerate() {
            checkpoint("support.ids");
            if !explained[i]
                && let Some(ids) = groups.get_mut(&key(m))
            {
                ids.push(i);
            }
        }
        fn root(parent: &mut [usize], mut i: usize) -> usize {
            while parent[i] != i {
                parent[i] = parent[parent[i]];
                i = parent[i];
            }
            i
        }
        for ids in groups.into_values() {
            checkpoint("support.each");
            let participants = ids
                .iter()
                .flat_map(|&id| matches[id].sources)
                .map(|source| {
                    checkpoint("support.participants");
                    self.segment(source).part
                })
                .collect::<std::collections::BTreeSet<_>>();
            // Two parts cannot acquire additional multipart support. In particular,
            // sparse independent feature pairs need no passage-role allocations.
            if participants.len() <= 2 {
                continue;
            }
            let self_group = key(&matches[ids[0]])[0] == key(&matches[ids[0]])[1];
            let mut roles = BTreeMap::new();
            let mut ignored = std::collections::BTreeSet::new();
            if self_group {
                // Source-index order is not a passage role. Infer opposite
                // passages from correspondence, prioritizing nearby evidence.
                // A part can take different roles in disconnected source runs.
                // Index connected uncapped intervals, not the complete part path.
                let mut run_parent = (0..2 * ids.len()).collect::<Vec<_>>();
                let mut runs = BTreeMap::<_, Vec<_>>::new();
                for (local, &id) in ids.iter().enumerate() {
                    checkpoint("support.runs");
                    for side in 0..2 {
                        let source = self.segment(matches[id].sources[side]);
                        let span = evidence[id].support[side];
                        runs.entry(source.part).or_default().push((
                            source.start as f64 + span[0],
                            source.start as f64 + span[1],
                            2 * local + side,
                        ));
                    }
                }
                for (part, mut spans) in runs {
                    parallel::sort(&mut spans, "support.runs.sort", |a, b| a.0.total_cmp(&b.0));
                    if self.parts[part].is_closed {
                        let first = spans.first().expect("support");
                        let last = spans
                            .iter()
                            .max_by(|a, b| a.1.total_cmp(&b.1))
                            .expect("support");
                        if first.0 == 0. && last.1 == (self.original(part).0.len() - 1) as f64 {
                            let a = root(&mut run_parent, first.2);
                            let b = root(&mut run_parent, last.2);
                            run_parent[a] = b;
                        }
                    }
                    let mut previous: Option<(f64, usize)> = None;
                    for (lo, hi, local) in spans {
                        checkpoint("support.runs.join");
                        if let Some((end, before)) = previous
                            && lo <= end
                        {
                            let a = root(&mut run_parent, local);
                            let b = root(&mut run_parent, before);
                            run_parent[a] = b;
                            previous = Some((end.max(hi), before));
                        } else {
                            previous = Some((hi, local));
                        }
                    }
                }
                let mut run_ids = BTreeMap::new();
                let mut passages = BTreeMap::new();
                for (local, &id) in ids.iter().enumerate() {
                    checkpoint("support.passages");
                    for side in 0..2 {
                        let run = root(&mut run_parent, 2 * local + side);
                        let next = run_ids.len();
                        passages.insert((id, side), *run_ids.entry(run).or_insert(next));
                    }
                }
                #[cfg(test)]
                PASSAGE_ROLE_SLOTS.with(|n| n.set(n.get() + run_ids.len()));
                let mut passage_parent = (0..run_ids.len()).collect::<Vec<_>>();
                let mut parity = vec![false; run_ids.len()];
                let mut rank = vec![0usize; run_ids.len()];
                fn passage(parent: &[usize], parity: &[bool], mut i: usize) -> (usize, bool) {
                    let mut side = false;
                    while parent[i] != i {
                        side ^= parity[i];
                        i = parent[i];
                    }
                    (i, side)
                }
                let mut support = BTreeMap::<_, BTreeMap<_, Vec<_>>>::new();
                for &id in &ids {
                    checkpoint("support.evidence");
                    if !rejected[id] {
                        continue;
                    }
                    for side in 0..2 {
                        support
                            .entry(passages[&(id, side)])
                            .or_default()
                            .entry(matches[id].sources[side])
                            .or_default()
                            .push(evidence[id].support[side]);
                    }
                }
                let lengths = support
                    .into_iter()
                    .map(|(passage, sources)| {
                        let length = sources
                            .into_iter()
                            .map(|(source, spans)| {
                                checkpoint("support.lengths");
                                let line = self.source(source);
                                super::matching::union(spans)
                                    .iter()
                                    .map(|span| span[1] - span[0])
                                    .sum::<f64>()
                                    * distance(line.start, line.end)
                            })
                            .sum::<f64>();
                        (passage, length)
                    })
                    .collect::<BTreeMap<_, _>>();
                let score = |id| {
                    [0, 1]
                        .map(|side| lengths.get(&passages[&(id, side)]).copied().unwrap_or(0.))
                        .into_iter()
                        .fold(0., f64::max)
                };
                let geometry_key = |id: usize| {
                    let mut pair = [0, 1].map(|side| {
                        let source = matches[id].sources[side];
                        let mut points = evidence[id].support[side]
                            .map(|t| super::matching::point(self.source(source), t));
                        if coordinate_order(&points[0], &points[1]).is_gt() {
                            points.swap(0, 1);
                        }
                        points
                    });
                    let cmp = |a: &[Coord; 2], b: &[Coord; 2]| {
                        coordinate_order(&a[0], &b[0]).then(coordinate_order(&a[1], &b[1]))
                    };
                    if cmp(&pair[0], &pair[1]).is_gt() {
                        pair.swap(0, 1);
                    }
                    pair
                };
                let mut ordered = ids.clone();
                parallel::sort(&mut ordered, "support.order", |&a, &b| {
                    (!rejected[a])
                        .cmp(&(!rejected[b]))
                        .then(score(b).total_cmp(&score(a)))
                        .then_with(|| {
                            let a = geometry_key(a);
                            let b = geometry_key(b);
                            a.into_iter()
                                .flatten()
                                .zip(b.into_iter().flatten())
                                .map(|(a, b)| coordinate_order(&a, &b))
                                .find(|o| !o.is_eq())
                                .unwrap_or(std::cmp::Ordering::Equal)
                        })
                });
                for id in ordered {
                    checkpoint("support.roles");
                    let parts = [passages[&(id, 0)], passages[&(id, 1)]];
                    let (a, pa) = passage(&passage_parent, &parity, parts[0]);
                    let (b, pb) = passage(&passage_parent, &parity, parts[1]);
                    if a == b {
                        if pa == pb {
                            // Keep the certified decision on a contradictory
                            // correspondence. Compatible connected support can
                            // still establish a different sustained passage.
                            ignored.insert(id);
                        }
                    } else if rank[a] < rank[b] {
                        passage_parent[a] = b;
                        parity[a] = pa ^ pb ^ true;
                    } else {
                        passage_parent[b] = a;
                        parity[b] = pa ^ pb ^ true;
                        if rank[a] == rank[b] {
                            rank[a] += 1;
                        }
                    }
                }
                for &id in &ids {
                    checkpoint("support.roles.assign");
                    for side in 0..2 {
                        let (_, role) = passage(&passage_parent, &parity, passages[&(id, side)]);
                        roles.insert((id, side), usize::from(role));
                    }
                }
            }
            let connected = |ids: &[usize]| {
                let mut parent = [0, 1].map(|_| (0..ids.len()).collect::<Vec<_>>());
                let mut spans = BTreeMap::<_, Vec<_>>::new();
                let mut nodes = BTreeMap::new();
                for side in 0..2 {
                    for (local, &id) in ids.iter().enumerate() {
                        checkpoint("connected.spans");
                        if ignored.contains(&id) {
                            continue;
                        }
                        let source = self.segment(matches[id].sources[side]);
                        let role = if self_group { roles[&(id, side)] } else { side };
                        let span = evidence[id].support[side];
                        spans.entry((role, source.part)).or_default().push((
                            source.start as f64 + span[0],
                            source.start as f64 + span[1],
                            local,
                        ));
                        for t in span {
                            let p =
                                super::matching::point(self.source(matches[id].sources[side]), t);
                            let k = (role, super::cluster::bits(p.x), super::cluster::bits(p.y));
                            if let Some(&(part, before)) = nodes.get(&k) {
                                if part != source.part {
                                    let a = root(&mut parent[role], local);
                                    let b = root(&mut parent[role], before);
                                    parent[role][a] = b;
                                }
                            } else {
                                nodes.insert(k, (source.part, local));
                            }
                        }
                    }
                }
                for ((role, part), mut spans) in spans {
                    parallel::sort(&mut spans, "connected.sort", |a, b| a.0.total_cmp(&b.0));
                    if self.parts[part].is_closed {
                        let first = spans.first().expect("support");
                        let last = spans
                            .iter()
                            .max_by(|a, b| a.1.total_cmp(&b.1))
                            .expect("support");
                        if first.0 == 0. && last.1 == (self.original(part).0.len() - 1) as f64 {
                            let a = root(&mut parent[role], first.2);
                            let b = root(&mut parent[role], last.2);
                            parent[role][a] = b;
                        }
                    }
                    let mut previous: Option<(f64, usize)> = None;
                    for (lo, hi, id) in spans {
                        checkpoint("connected.join");
                        if let Some((end, before)) = previous
                            && lo <= end
                        {
                            let a = root(&mut parent[role], id);
                            let b = root(&mut parent[role], before);
                            parent[role][a] = b;
                            previous = Some((end.max(hi), before));
                        } else {
                            previous = Some((hi, id));
                        }
                    }
                }
                let mut components = BTreeMap::<_, Vec<_>>::new();
                for (local, &id) in ids.iter().enumerate() {
                    checkpoint("connected.components");
                    if ignored.contains(&id) {
                        continue;
                    }
                    components
                        .entry([root(&mut parent[0], local), root(&mut parent[1], local)])
                        .or_default()
                        .push(id);
                }
                components.into_values().collect::<Vec<_>>()
            };
            let mut pending = connected(&ids);
            while let Some(component) = pending.pop() {
                checkpoint("sustained.pending");
                if !component.iter().any(|&id| rejected[id]) {
                    continue;
                }
                // Connectivity must hold inside the accepted correspondence group.
                // A bridge excluded by the opposite role cannot connect its support.
                let subdivisions = connected(&component);
                if subdivisions.len() > 1 {
                    pending.extend(subdivisions);
                    continue;
                }

                checkpoint("sustained.component");
                let mut multipart = false;
                let sustained = [0, 1].iter().all(|&side| {
                    use super::certified::Interval;
                    let mut by_source = BTreeMap::<_, Vec<_>>::new();
                    let mut parts = std::collections::BTreeSet::new();
                    for &id in &component {
                        checkpoint("sustained.sources");
                        for original_side in 0..2 {
                            let source = matches[id].sources[original_side];
                            let part = self.segment(source).part;
                            let role = if self_group {
                                roles[&(id, original_side)]
                            } else {
                                original_side
                            };
                            if role == side {
                                parts.insert(part);
                                by_source
                                    .entry(source)
                                    .or_default()
                                    .push(evidence[id].support[original_side]);
                            }
                        }
                    }
                    let multiple_parts = parts.len() > 1;
                    let mut part_lengths = BTreeMap::<usize, Interval>::new();
                    // Original supporting-line identities let repeated and
                    // unequally segmented parts contribute their geometric union.
                    let mut physical = BTreeMap::<_, Vec<_>>::new();
                    for (source, spans) in by_source {
                        checkpoint("sustained.lengths");
                        let line = self.source(source);
                        let x = Interval(line.end.x, line.end.x)
                            .sub(Interval(line.start.x, line.start.x));
                        let y = Interval(line.end.y, line.end.y)
                            .sub(Interval(line.start.y, line.start.y));
                        let length = x.square().add(y.square()).sqrt();
                        let spans = super::matching::union(spans);
                        let part = self.segment(source).part;
                        let total = part_lengths.entry(part).or_insert(Interval(0., 0.));
                        for span in &spans {
                            *total =
                                total.add(length.mul(
                                    Interval(span[1], span[1]).sub(Interval(span[0], span[0])),
                                ));
                        }
                        let q = |v: f64| {
                            BigRational::from_float(v).expect("finite original coordinate")
                        };
                        let x0 = q(line.start.x);
                        let y0 = q(line.start.y);
                        let dx = q(line.end.x) - &x0;
                        let dy = q(line.end.y) - &y0;
                        let vertical = dx == BigRational::from_integer(0.into());
                        let slope = if vertical {
                            BigRational::from_integer(0.into())
                        } else {
                            &dy / &dx
                        };
                        let intercept = if vertical {
                            x0.clone()
                        } else {
                            &y0 - &slope * &x0
                        };
                        let (origin, delta) = if vertical { (y0, dy) } else { (x0, dx) };
                        for span in spans {
                            if !span.iter().all(|v| v.is_finite()) {
                                return true;
                            }
                            let a = &origin + q(span[0]) * &delta;
                            let b = &origin + q(span[1]) * &delta;
                            physical
                                .entry((vertical, slope.clone(), intercept.clone()))
                                .or_default()
                                .push(if a <= b { [a, b] } else { [b, a] });
                        }
                    }
                    let mut total = Interval(0., 0.);
                    for ((vertical, slope, _), mut spans) in physical {
                        checkpoint("sustained.lines");
                        parallel::sort(&mut spans, "sustained.sort", |a, b| a[0].cmp(&b[0]));
                        let mut merged: Vec<[BigRational; 2]> = Vec::new();
                        for span in spans {
                            checkpoint("sustained.merge");
                            if let Some(last) = merged.last_mut()
                                && span[0] <= last[1]
                            {
                                last[1] = last[1].clone().max(span[1].clone());
                            } else {
                                merged.push(span);
                            }
                        }
                        let ratio = if vertical {
                            Interval(1., 1.)
                        } else {
                            let Some(slope) = slope.to_f64().filter(|v| v.is_finite()) else {
                                return true;
                            };
                            Interval(slope.next_down(), slope.next_up())
                                .square()
                                .add(Interval(1., 1.))
                                .sqrt()
                        };
                        for span in merged {
                            checkpoint("sustained.widths");
                            let Some(width) =
                                (&span[1] - &span[0]).to_f64().filter(|v| v.is_finite())
                            else {
                                return true;
                            };
                            total =
                                total.add(Interval(width.next_down(), width.next_up()).mul(ratio));
                        }
                    }
                    // Duplicate or contained parts add no connected physical support.
                    // Restoration requires a certified contribution beyond every
                    // individual part, not merely a second part identity.
                    let longest = part_lengths
                        .values()
                        .map(|length| length.1)
                        .fold(0., f64::max);
                    multipart |= multiple_parts && total.0 > longest;
                    // Inconclusive arithmetic preserves the original evidence.
                    total.1 > self.tolerance
                });
                if multipart && sustained {
                    for id in component {
                        checkpoint("sustained.restore");
                        rejected[id] = false;
                    }
                }
            }
        }
    }
    pub fn source(&self, index: usize) -> Line {
        self.sources[index].line(&self.parts)
    }
    /// Open ends of either part lying within tolerance of the other, meeting it at
    /// their nearest points. Coordinates within tolerance are coincident, so such
    /// an end meets the other line. An end from which a further part of the same
    /// feature continues ends no linework, and one within a sixteenth of the
    /// tolerance already meets the other part as a vertex does.
    fn end_meetings(
        &self,
        tree: &impl Search,
        parts: [usize; 2],
        positions: &[Vec<f64>],
    ) -> Vec<Meeting> {
        let tolerance = self.tolerance;
        let mut meetings = Vec::new();
        if parts[0] == parts[1] {
            return meetings;
        }
        for side in 0..2 {
            let part = &self.parts[parts[side]];
            if part.is_closed {
                continue;
            }
            let coords = &part.original.0;
            let feature = part.reference.feature();
            let total = *positions[parts[side]].last().expect("source");
            for (position, point) in [(0., coords[0]), (total, *coords.last().expect("source"))] {
                let mut closest = None;
                let mut continues = false;
                for (id, _) in tree.search(index::at(point, tolerance)) {
                    let segment = self.segment(id);
                    let (parameter, projected) = projection(self.source(id), point);
                    let offset = distance(point, projected);
                    if offset > tolerance {
                        continue;
                    }
                    if segment.part == parts[1 - side] {
                        let p = &positions[segment.part];
                        let at = p[segment.start]
                            + parameter * (p[segment.start + 1] - p[segment.start]);
                        if closest.is_none_or(|(best, _)| offset < best) {
                            closest = Some((offset, at));
                        }
                    } else if segment.part != parts[side]
                        && self.parts[segment.part].reference.feature() == feature
                    {
                        continues = true;
                    }
                }
                if let Some((offset, at)) = closest
                    && !continues
                    && offset > tolerance / 16.
                {
                    let mut meeting = Meeting {
                        positions: [at; 2],
                        end: Some(point),
                    };
                    meeting.positions[side] = position;
                    meetings.push(meeting);
                }
            }
        }
        meetings
    }
    /// Whether a run's support lies within the near zones of points where its
    /// passages meet, except for at most one tolerance, on both passages. Walking
    /// along one passage can miss linework the other runs alongside after a turn.
    ///
    /// Lines meeting at angle `a` stay within tolerance for about `tolerance / sin a`
    /// beyond the meeting point, wherever their vertices lie. A line end within
    /// tolerance of the other passage meets it too, as if the lines met where they
    /// would at the rate they part from it. A line approaching at an angle and
    /// ending within tolerance then shares no linework, however it is segmented,
    /// while one running nearly parallel keeps its support. Walking away from a
    /// meeting point along a passage, its near zone extends to where the passages
    /// leave tolerance of each other, plus one tolerance, if their separation grows
    /// steadily on the way. Lines diverging too slowly to leave tolerance within the
    /// cap get a zone that shrinks with their angle, and a line hooking back from
    /// its end across the other gets the tolerance next to the meeting point. Lines
    /// that leave tolerance only because the other passage ends there, or that run
    /// alongside each other before turning away, have no near zone, so lines that
    /// merely touch or cross while running alongside each other keep their
    /// support. A run beginning and ending where lines end beside each other, such
    /// as a short arm or a collinear overshoot, is judged by its length alone.
    fn near_zones_explain(
        &self,
        tree: &impl Search,
        parts: [usize; 2],
        positions: &[Vec<f64>],
        uncertainty: &[f64],
        spans: &[Vec<[f64; 2]>; 2],
        meetings: &[Meeting],
    ) -> bool {
        let tolerance = self.tolerance;
        let cap = NEAR_ZONE_TOLERANCES * tolerance;
        let reach = cap + 2. * tolerance;
        let total = parts.map(|part| *positions[part].last().expect("source"));
        let closed = parts.map(|part| self.parts[part].is_closed);
        // Circular distance along a part between a position and a span.
        let gap = |side: usize, v: f64, span: [f64; 2]| {
            let wraps: &[f64] = if closed[side] { &[-1., 0., 1.] } else { &[0.] };
            wraps
                .iter()
                .map(|&k| {
                    (span[0] - (v + k * total[side]))
                        .max(v + k * total[side] - span[1])
                        .max(0.)
                })
                .fold(f64::INFINITY, f64::min)
        };
        let relevant = meetings
            .iter()
            .filter(|m| {
                checkpoint("zones.relevant");
                (0..2).all(|side| {
                    spans[side]
                        .iter()
                        .any(|&span| gap(side, m.positions[side], span) <= reach)
                })
            })
            .collect::<Vec<_>>();
        if relevant.is_empty() {
            return false;
        }
        // Segments of the other passage within `radius` of a point, keeping only
        // those closer, along a single part, to `there` than to `here`, with
        // whether each of their vertices ends an open part.
        let nearby = |side: usize, point: Coord, radius: f64, here: f64, there: f64| {
            tree.search(index::at(point, radius))
                .filter_map(|(id, envelope)| {
                    let segment = self.segment(id);
                    let keep = segment.part == parts[1 - side]
                        && (parts[0] != parts[1] || {
                            let p = &positions[segment.part];
                            let span = [p[segment.start], p[segment.start + 1]];
                            gap(side, there, span) < gap(side, here, span)
                        });
                    let open = !closed[1 - side];
                    let last = self.original(segment.part).0.len() - 2;
                    keep.then(|| {
                        (
                            envelope,
                            self.source(id),
                            [open && segment.start == 0, open && segment.start == last],
                        )
                    })
                })
                .collect::<Vec<_>>()
        };
        // Distance from a point to the nearest of those segments, exact up to just
        // beyond the tolerance a walk step can cross, and the end of the other
        // passage if that is the nearest point.
        let nearest = |segments: &[Beside], point: Coord| {
            use rstar::Envelope;
            let bounds = index::at(point, 2. * tolerance);
            segments
                .iter()
                .filter(|(envelope, ..)| envelope.intersects(&bounds))
                .map(|&(_, line, ends)| {
                    let (parameter, projected) = projection(line, point);
                    let end = (parameter == 0. && ends[0] || parameter == 1. && ends[1])
                        .then_some(projected);
                    (distance(point, projected), end)
                })
                .fold((f64::INFINITY, None), |best, next| {
                    if next.0 < best.0 { next } else { best }
                })
        };
        let ends = (0..2)
            .filter(|&side| !closed[side])
            .flat_map(|side| {
                let coords = &self.original(parts[side]).0;
                [coords[0], *coords.last().expect("source")]
            })
            .collect::<Vec<_>>();
        // Ends of either line that lie beside the other passage.
        let beside = (0..2)
            .filter(|&side| !closed[side])
            .flat_map(|side| {
                let coords = &self.original(parts[side]).0;
                let there = spans[1 - side][0][0];
                [
                    (0., coords[0]),
                    (total[side], *coords.last().expect("source")),
                ]
                .into_iter()
                .filter(move |&(position, point)| {
                    nearest(&nearby(side, point, tolerance, position, there), point).0 <= tolerance
                })
                .map(|(_, point)| point)
            })
            .collect::<Vec<_>>();
        // A run that begins and ends where lines end beside each other, as a short
        // arm or a collinear overshoot does, is judged by its length alone.
        let bounded = (0..2).filter(|&side| !closed[side]).any(|side| {
            let extent = [spans[side][0][0], spans[side].last().expect("support")[1]];
            let [low, high] = extent.map(|v| {
                self.point_at(parts[side], &positions[parts[side]], v)
                    .map(|q| {
                        (0..beside.len())
                            .filter(|&e| distance(q, beside[e]) <= tolerance)
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default()
            });
            low.iter().any(|a| high.iter().any(|b| a != b))
        });
        if bounded {
            return false;
        }
        // Whether zones along one passage leave at most one tolerance of its
        // support unexplained.
        let explained = |side: usize, zones: Vec<[f64; 2]>| {
            let zones = super::matching::union(zones);
            let wraps: &[f64] = if closed[side] { &[-1., 0., 1.] } else { &[0.] };
            let remainder = spans[side]
                .iter()
                .map(|&span| {
                    checkpoint("zones.explained");
                    let covered = zones
                        .iter()
                        .flat_map(|zone| {
                            wraps.iter().map(move |&k| {
                                (span[1].min(zone[1] + k * total[side])
                                    - span[0].max(zone[0] + k * total[side]))
                                .max(0.)
                            })
                        })
                        .sum::<f64>();
                    (span[1] - span[0] - covered).max(0.)
                })
                .sum::<f64>();
            remainder + 4. * uncertainty[parts[side]] * spans[side].len() as f64 <= tolerance
        };
        let step = tolerance / 8.;
        let limit = cap + tolerance;
        (0..2).all(|side| {
            let part = parts[side];
            let p = &positions[part];
            let mut zones = Vec::new();
            for (index, meeting) in relevant.iter().enumerate() {
                checkpoint("zones.meetings");
                // No zone reaches further than `reach` from its meeting point, so
                // stop once the zones still possible cannot explain the run.
                let possible = zones
                    .iter()
                    .copied()
                    .chain(
                        relevant[index..]
                            .iter()
                            .map(|m| [m.positions[side] - reach, m.positions[side] + reach]),
                    )
                    .collect();
                if !explained(side, possible) {
                    return false;
                }
                let start = meeting.positions[side];
                let there = meeting.positions[1 - side];
                // Every point a walk visits lies within this distance of its start.
                let segments = self
                    .point_at(part, p, start)
                    .map_or_else(Vec::new, |point| {
                        nearby(side, point, limit + 3. * tolerance, start, there)
                    });
                let origin = self.point_at(part, p, start);
                let separation = |point: Coord| nearest(&segments, point).0;
                // From a line end meeting the other passage, separation is measured
                // from the end's offset, as if the lines met where they would at the
                // rate they part. It may wobble by an eighth of what remains of the
                // tolerance beyond that offset.
                let gap0 = meeting.end.map_or(0., |_| origin.map_or(0., separation));
                let wobble = (tolerance - gap0) / 8.;
                for direction in [1., -1.] {
                    // From a line end, the walk also samples the end of an open
                    // passage `room` away, wherever it lies between two steps.
                    let finish = (meeting.end.is_some() && !closed[side]).then(|| {
                        let coords = &self.original(part).0;
                        if direction > 0. {
                            (total[side] - start, *coords.last().expect("source"))
                        } else {
                            (start, coords[0])
                        }
                    });
                    let room = finish.map_or(f64::INFINITY, |(room, _)| room);
                    let at = |offset: f64| match finish {
                        Some((room, point)) if offset == room => Some(point),
                        _ => self.point_at(part, p, start + direction * offset),
                    };
                    let mut walk = Vec::new();
                    let mut peak = 0.;
                    // Where the other passage ends away from this meeting point,
                    // other than at the line end meeting it here, lines leaving
                    // tolerance there have not parted at an angle.
                    let mut ended = false;
                    let offsets = (1..)
                        .map(|k| k as f64 * step)
                        .take_while(|&o| o < room)
                        .chain((room > 0. && room.is_finite()).then_some(room))
                        .take_while(|&o| o <= limit);
                    for offset in offsets {
                        let Some(point) = at(offset) else {
                            break;
                        };
                        let (gap, end) = nearest(&segments, point);
                        ended = end.is_some_and(|end| {
                            Some(end) != meeting.end
                                && origin
                                    .is_none_or(|origin| distance(end, origin) > tolerance / 16.)
                        });
                        walk.push((offset, gap));
                        peak = gap.max(peak);
                        // Stop where the passages leave tolerance, or where they meet
                        // again after diverging. A line end meets the other passage
                        // beyond a sixteenth of the tolerance, so coming back within
                        // that is meeting it again, however little the lines parted:
                        // the stretch up to it lies between two meetings.
                        if gap > tolerance
                            || (peak >= tolerance / 2. || meeting.end.is_some())
                                && gap <= tolerance / 16.
                        {
                            break;
                        }
                    }
                    // Separation follows one divergence rate from the meeting point,
                    // as for lines meeting at an angle, rather than running alongside
                    // the other passage at any offset and then turning away.
                    let straight = |until: f64, rate: f64| {
                        walk.iter().take_while(|&&(offset, _)| offset < until).all(
                            |&(offset, gap)| {
                                (gap - gap0 - offset * rate).abs() <= offset * rate / 4. + wobble
                            },
                        )
                    };
                    // The zone of lines diverging at `rate`, shrinking smoothly for
                    // lines too close to parallel to leave tolerance before the cap,
                    // and reaching from where lines leaving a line end would have met.
                    let vanishing = |rate: f64| {
                        (limit * (2. * rate * limit / tolerance).min(1.) - gap0 / rate).max(0.)
                    };
                    // Separation grows steadily up to a peak, rather than stalling.
                    let grows = |until: f64, peak: f64, slack: f64| {
                        walk.iter()
                            .take_while(|&&(offset, _)| offset < until)
                            .all(|&(offset, gap)| gap >= 0.75 * offset / until * peak - slack)
                    };
                    let reach_end = match walk.last() {
                        // The passages leave tolerance of each other. Lines leaving a
                        // line end too slowly to leave tolerance within the cap from
                        // where they would have met get the zone of lines meeting there
                        // at a small angle.
                        Some(&(exit, gap)) if gap > tolerance => {
                            let rate = (tolerance - gap0) / exit;
                            (!ended && straight(exit, rate)).then(|| {
                                if meeting.end.is_some() && tolerance / rate > limit {
                                    vanishing(rate).min(exit + tolerance)
                                } else {
                                    exit + tolerance
                                }
                            })
                        }
                        // They diverge steadily without leaving tolerance before the
                        // cap, as lines meeting at a small angle do. The zone shrinks
                        // smoothly with that angle, so lines meeting at a vanishing
                        // angle keep their support.
                        Some(&(last, gap))
                            if last + step > limit && gap - gap0 > tolerance / 16. =>
                        {
                            let rate = (gap - gap0) / last;
                            straight(f64::INFINITY, rate).then(|| vanishing(rate))
                        }
                        // A line leaves the other from its end and hooks back across
                        // it, meeting it again before leaving tolerance and parting
                        // beyond. Only the tolerance next to this meeting point is
                        // explained; elsewhere lines that meet again ran together.
                        Some(&(rejoin, gap)) if gap <= tolerance / 16. => {
                            let peak_at = walk.iter().find(|w| w.1 == peak).map_or(0., |w| w.0);
                            let hooked = [0., rejoin].into_iter().any(|offset| {
                                at(offset).is_some_and(|point| {
                                    ends.iter().any(|&end| distance(point, end) <= 2. * step)
                                })
                            });
                            let parts_beyond =
                                (1..=16).map(|k| rejoin + k as f64 * step).any(|offset| {
                                    at(offset).is_none_or(|point| separation(point) > tolerance)
                                });
                            (hooked
                                && peak - gap0 >= tolerance / 2.
                                && grows(peak_at, peak, 0.)
                                && parts_beyond)
                                .then_some(rejoin.min(tolerance))
                        }
                        _ => None,
                    };
                    if let Some(exit) = reach_end {
                        let end = start + direction * exit;
                        zones.push([start.min(end), start.max(end)]);
                    }
                }
            }
            explained(side, zones)
        })
    }
    /// The original point at a traversal distance along an open or closed part.
    fn point_at(&self, part: usize, positions: &[f64], position: f64) -> Option<Coord> {
        let total = *positions.last()?;
        let position = if self.parts[part].is_closed && total > 0. {
            position.rem_euclid(total)
        } else if (0. ..=total).contains(&position) {
            position
        } else {
            return None;
        };
        let i = positions
            .partition_point(|&v| v <= position)
            .clamp(1, positions.len() - 1)
            - 1;
        let coords = &self.original(part).0;
        let width = positions[i + 1] - positions[i];
        let t = if width > 0. {
            ((position - positions[i]) / width).clamp(0., 1.)
        } else {
            0.
        };
        Some(super::matching::point(
            Line::new(coords[i], coords[i + 1]),
            t,
        ))
    }
    /// Neighbouring segments of one part, which share a vertex by construction.
    fn consecutive(&self, a: usize, b: usize) -> bool {
        let (a, b) = (self.segment(a), self.segment(b));
        let last = self.original(a.part).0.len() - 2;
        a.part == b.part
            && (a.start.abs_diff(b.start) == 1
                || self.parts[a.part].is_closed
                    && a.start.min(b.start) == 0
                    && a.start.max(b.start) == last)
    }
    /// Nodes holding original contacts of one feature, in node order. A contact is
    /// keyed by its own coordinate, which need not be a node of either noded part:
    /// the cracked point can join a different cluster than the contacting endpoint.
    pub fn feature_contacts(&self, key: FeatureKey) -> &[usize] {
        self.contact_nodes
            .get_or_init(|| {
                let mut features = BTreeMap::<_, Vec<_>>::new();
                for (node, contacts) in self.contacts.iter() {
                    checkpoint("contacts.features");
                    for contact in contacts {
                        for part in contact.parts {
                            let nodes = features
                                .entry(self.parts[part].reference.feature())
                                .or_default();
                            if nodes.last() != Some(&node) {
                                nodes.push(node);
                            }
                        }
                    }
                }
                features
            })
            .get(&key)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }
    pub fn build(t: &Topology, layers: &[(LayerId, InputMode)]) -> Result<Self, Error> {
        let tolerance = t.tolerance();
        let mut parts = Vec::new();
        let mut segments = Vec::new();
        // First segment of each part, then the number of segments.
        let mut starts = Vec::new();
        let mut ends = Vec::new();
        let mut magnitude = 0_f64;
        for &(layer, mode) in layers {
            let input = t.parts(layer)?;
            let lines: Box<dyn Iterator<Item = (crate::model::PartRef, &Shared<LineString>)> + '_> =
                match mode {
                    InputMode::Linework => {
                        Box::new(input.lines.iter().map(|p| (p.reference, &p.geometry)))
                    }
                    InputMode::PolygonBoundary => Box::new(
                        crate::engine::cross::boundaries(input)
                            .iter()
                            .map(|p| (p.reference, &p.geometry)),
                    ),
                };
            for (reference, geometry) in lines {
                checkpoint("parts");
                if mode == InputMode::Linework && collapsed(geometry, tolerance) {
                    continue;
                }
                let coords = &geometry.0;
                magnitude = coords
                    .iter()
                    .fold(magnitude, |m, p| m.max(p.x.abs()).max(p.y.abs()));
                let closed = coords.first() == coords.last();
                let id = parts.len();
                parts.push(GraphPart {
                    // Graphs belong to one topology, so parts carry final layer IDs.
                    reference: reference.at(layer),
                    nodes: Vec::new(),
                    is_closed: closed,
                    original: geometry.clone(),
                });
                // Vertices are numbered across all parts, so a part's vertices
                // follow its segments' numbers by its own number.
                let first = segments.len();
                starts.push(first);
                let n = coords.len();
                if !closed {
                    ends.push(End {
                        point: coords[0],
                        source: first,
                        vertex: first + id,
                        last: false,
                    });
                    ends.push(End {
                        point: coords[n - 1],
                        source: first + n - 2,
                        vertex: first + id + n - 1,
                        last: true,
                    });
                }
                segments.extend((0..n - 1).map(|start| Segment { start, part: id }));
            }
        }
        starts.push(segments.len());
        let workers = Workers::new(segments.len());
        // A point is on a feature's boundary when an odd number of its open parts end
        // there (the OGC mod-2 rule), counted on original feature coordinates. Closed
        // parts and passages add nothing; nearby clustered endpoints stay separate.
        // Ends are kept by feature and coordinate, once each with their number, to
        // count them and to find those beside a contact. Adding zero makes negative
        // zeros positive, so equal ends are identical and the order agrees with the
        // comparisons of the search.
        let positive = |p: Coord| Coord {
            x: p.x + 0.,
            y: p.y + 0.,
        };
        let blank = ((LayerId(0), 0), Coord { x: 0., y: 0. });
        let mut feature_ends = parallel::repeated(blank, ends.len(), "ends.init");
        workers.fill(&mut feature_ends, |i| {
            let end = &ends[i];
            let feature = parts[segments[end.source].part].reference.feature();
            (feature, positive(end.point))
        });
        let by_feature = |a: &(FeatureKey, Coord), b: &(FeatureKey, Coord)| {
            a.0.cmp(&b.0).then(coordinate_order(&a.1, &b.1))
        };
        workers.sort(&mut feature_ends, "ends.sort", by_feature);
        let mut end_counts = Vec::new();
        for i in 0..feature_ends.len() {
            every(i, "ends");
            let kept = end_counts.len();
            if kept > 0 && feature_ends[kept - 1] == feature_ends[i] {
                end_counts[kept - 1] += 1;
            } else {
                feature_ends[kept] = feature_ends[i];
                end_counts.push(1_usize);
            }
        }
        feature_ends.truncate(end_counts.len());
        let mut entries = parallel::repeated(Entry::default(), segments.len(), "index");
        workers.fill(&mut entries, |i| {
            Entry::new(i, segments[i].line(&parts).bounding_rect())
        });
        let tree = Packed::bulk(entries, &workers);
        let contact = |a: usize, b: usize, points: [Coord; 2]| {
            let sources = [a, b];
            let [a, b] = sources.map(|source| segments[source].part);
            if parts[a].reference.feature() == parts[b].reference.feature() {
                return None;
            }
            // The nearest end of the whole feature decides, and an end at the contact
            // itself is nearest. Otherwise the touched part's own ends count within
            // tolerance; other parts' ends only where they coincide with the contact
            // up to rounding of the coordinates the contact came from, so within twice
            // their rounding reach.
            let [l0, l1] = sources.map(|source| segments[source].line(&parts));
            let interior = [0, 1].map(|side| {
                let p = &parts[[a, b][side]];
                let feature = p.reference.feature();
                let point = points[side];
                let count = |end: Coord| {
                    let key = (feature, positive(end));
                    let found = feature_ends.binary_search_by(|e| by_feature(e, &key));
                    found.ok().map(|i| end_counts[i])
                };
                let nearest = || {
                    let own = [p.original.0[0], *p.original.0.last().expect("source")]
                        .into_iter()
                        .filter(|&end| !p.closed() && distance(point, end) <= tolerance);
                    // Rounding is the largest coordinate's, so an end only adds its own.
                    let base = rounding(&[point, l0.start, l0.end, l1.start, l1.end]);
                    let window = 2. * base.min(tolerance);
                    let others = ends_near(&feature_ends, feature, point, window).filter(|&end| {
                        distance(point, end) <= base.max(rounding(&[end])).min(tolerance)
                    });
                    own.chain(others).min_by(|a, b| {
                        distance(point, *a)
                            .total_cmp(&distance(point, *b))
                            .then(coordinate_order(a, b))
                    })
                };
                count(point)
                    .or_else(|| nearest().and_then(count))
                    .is_none_or(|count| count.is_multiple_of(2))
            });
            // Endpoint junctions need no retained meeting evidence.
            (interior[0] || interior[1]).then_some(Contact {
                parts: [a, b],
                interior,
                points,
                sources,
            })
        };
        // Exact intersections meet a vertex lying on another segment only where rounding
        // leaves it on or across that segment. A vertex within rounding of a segment, and
        // within tolerance, meets it from either side, as in data stored without that
        // rounding. A proper intersection beside such a segment end is the same meeting,
        // so it is recorded there: its own rounded coordinate could join another node.
        let near = |vertex: Coord, segment: Line| {
            let (_, projected) = projection(segment, vertex);
            (tolerance > 0.
                && distance(vertex, projected)
                    <= rounding(&[vertex, segment.start, segment.end]).min(tolerance))
            .then_some(projected)
        };
        // Interior vertices, including ring seams. Part endpoints are cracked instead.
        let vertex = |s: &Segment| {
            let part = &parts[s.part];
            (s.start > 0 || part.closed()).then(|| part.original.0[s.start])
        };
        let reach = (magnitude * f64::EPSILON * 16.).min(tolerance);
        // Each task finds the contacts of a run of segments with the later ones.
        let paired = workers.chunks(segments.len(), |range| {
            let mut found = Found::default();
            let mut work = 0_usize;
            for i in range {
                let s = segments[i];
                let line = s.line(&parts);
                let at = i + s.part;
                for candidate in tree.locate(index::expanded(line.bounding_rect(), reach)) {
                    every(work, "pairs");
                    work += 1;
                    let j = candidate.id;
                    if j <= i {
                        continue;
                    }
                    let other = segments[j].line(&parts);
                    let other_at = j + segments[j].part;
                    match line_intersection(line, other) {
                        Some(LineIntersection::SinglePoint {
                            intersection,
                            is_proper,
                        }) => {
                            let point = if is_proper {
                                proper_intersection(line, other).unwrap_or(intersection)
                            } else {
                                intersection
                            };
                            // The nearest segment end within rounding of the other segment.
                            let beside = [line.start, line.end, other.start, other.end]
                                .into_iter()
                                .enumerate()
                                .filter(|&(_, p)| is_proper && distance(p, point) <= tolerance)
                                .filter_map(|(k, p)| {
                                    Some(if k < 2 {
                                        (k, p, [p, near(p, other)?])
                                    } else {
                                        (k, p, [near(p, line)?, p])
                                    })
                                })
                                .min_by(|a, b| {
                                    distance(a.1, point).total_cmp(&distance(b.1, point))
                                });
                            let cut = found.cut(i, at, line, point);
                            found.cut(j, other_at, other, point);
                            match beside {
                                Some((k, _, points)) => {
                                    let end = [at, at + 1, other_at, other_at + 1][k];
                                    found.contact(contact(i, j, points), At::Vertex(end));
                                }
                                None => found.contact(contact(i, j, [point, point]), cut),
                            }
                        }
                        Some(LineIntersection::Collinear { intersection }) => {
                            for p in [intersection.start, intersection.end] {
                                let cut = found.cut(i, at, line, p);
                                found.cut(j, other_at, other, p);
                                found.contact(contact(i, j, [p, p]), cut);
                            }
                        }
                        None => {}
                    }
                    if let Some(v) = vertex(&s)
                        && let Some(projected) = near(v, other)
                    {
                        found.contact(contact(i, j, [v, projected]), At::Vertex(at));
                    }
                    if let Some(v) = vertex(&segments[j])
                        && let Some(projected) = near(v, line)
                    {
                        found.contact(contact(i, j, [projected, v]), At::Vertex(other_at));
                    }
                }
            }
            found
        });
        // Crack points on other parts, for line ends also near their own line.
        // Endpoint cracking has no effect on the immutable input geometries.
        let cracking = if tolerance > 0. {
            workers.chunks(ends.len(), |range| {
                let mut found = Found::default();
                let mut work = 0_usize;
                for end in &ends[range] {
                    let mut candidates = tree
                        .locate(index::at(end.point, tolerance))
                        .map(|e| e.id)
                        .collect::<Vec<_>>();
                    candidates.sort_unstable();
                    let part = segments[end.source].part;
                    let (mut own, mut others) = (false, Vec::new());
                    for i in candidates {
                        every(work, "crack");
                        work += 1;
                        let line = segments[i].line(&parts);
                        let (_, projected) = projection(line, end.point);
                        if distance(end.point, projected) <= tolerance {
                            let points = [end.point, projected];
                            found.contact(contact(end.source, i, points), At::Vertex(end.vertex));
                            let cut = found.cut(i, i + segments[i].part, line, projected);
                            found.cracked.push(projected);
                            if segments[i].part == part {
                                own |= i != end.source;
                            } else {
                                others.push(cut);
                            }
                        }
                    }
                    if own && !others.is_empty() {
                        found.ties.push(((part, end.last), others));
                    }
                }
                found
            })
        } else {
            Vec::new()
        };
        // Points are numbered as all vertices, then all cuts in discovery order.
        let vertices = segments.len() + parts.len();
        let mut original_contacts = Vec::new();
        let mut cut_sources = Vec::new();
        let mut cut_points = Vec::new();
        let mut cracked = Vec::new();
        let mut ties = Vec::new();
        for found in paired.into_iter().chain(cracking) {
            checkpoint("gather.contacts");
            let base = vertices + cut_points.len();
            let origin = move |at: At| match at {
                At::Vertex(v) => v,
                At::Cut(c) => base + c,
            };
            original_contacts.extend(found.contacts.into_iter().map(|(at, c)| (origin(at), c)));
            ties.extend(
                found
                    .ties
                    .into_iter()
                    .map(|(end, at)| (end, at.into_iter().map(origin).collect::<Vec<_>>())),
            );
            cut_sources.extend(found.cuts.iter().map(|&(source, _)| source));
            cut_points.extend(found.cuts.into_iter().map(|(_, p)| p));
            cracked.extend(found.cracked);
        }
        // Endpoint parity is construction evidence, not retained graph storage.
        // Release it before allocating noded edges and incidences.
        drop(end_counts);
        drop(feature_ends);
        let order = tree.order("order");
        drop(tree);
        // All contact coordinates are discovered against source segments first.
        // Canonical insertion makes clustering independent of FIDs and path direction.
        let mut sweep = Sweep::new(tolerance);
        let endpoints = ends.into_iter().map(|end| end.point).collect::<Vec<_>>();
        for mut batch in [endpoints, cracked] {
            // Points that compare equal are identical.
            workers.sort(&mut batch, "cluster.ends", coordinate_order);
            batch.dedup();
            sweep.batch(&batch, |&p| p, false, "cluster.batch", |_, _| {});
        }
        let mut points = Vec::with_capacity(vertices + cut_points.len());
        let all = parts
            .iter()
            .flat_map(|part| &part.original.0)
            .chain(&cut_points);
        for (origin, &p) in all.enumerate() {
            every(origin, "cluster.points");
            points.push((p, origin));
        }
        debug_assert_eq!(points.len(), vertices + cut_points.len());
        workers.sort(&mut points, "cluster.sort", |a, b| {
            coordinate_order(&a.0, &b.0).then(a.1.cmp(&b.1))
        });
        let mut node_of = vec![0; points.len()];
        sweep.batch(&points, |p| p.0, true, "cluster", |p, id| node_of[p.1] = id);
        drop(points);
        let ties = ties
            .into_iter()
            .enumerate()
            .map(|(k, (end, at))| {
                every(k, "ties");
                (end, at.into_iter().map(|o| node_of[o]).collect::<Vec<_>>())
            })
            .collect::<HashMap<_, _>>();
        // Cuts of each segment, in discovery order.
        let mut by_source = cut_sources
            .into_iter()
            .enumerate()
            .map(|(c, source)| (source, c))
            .collect::<Vec<_>>();
        workers.sort(&mut by_source, "cuts", Ord::cmp);
        // Each task nodes a run of whole parts. Their nodes stay in one list per
        // task until the end: see the last step.
        let noded = workers.each(part_ranges(&workers, &starts), |run: Range<usize>| {
            let mut raw_edges: Vec<([usize; 2], EdgeUse)> = Vec::new();
            let mut trims = Vec::new();
            let mut nodes = Vec::new();
            let mut spans = Vec::with_capacity(run.len());
            // Segments of one part are consecutive, so one path is built at a time.
            let mut path: Vec<Visit> = Vec::new();
            let mut cuts = Vec::new();
            let mut next = by_source.partition_point(|&(source, _)| source < starts[run.start]);
            let mut work = 0_usize;
            for id in run {
                let part = &parts[id];
                let coords = &part.original.0;
                for source in starts[id]..starts[id + 1] {
                    every(work, "noding");
                    work += 1;
                    let start = source - starts[id];
                    let vertex = source + id;
                    let line = Line::new(coords[start], coords[start + 1]);
                    cuts.clear();
                    while let Some(&(_, c)) = by_source.get(next).filter(|(s, _)| *s == source) {
                        cuts.push((cut_points[c], vertices + c));
                        next += 1;
                    }
                    cuts.extend([(line.start, vertex), (line.end, vertex + 1)]);
                    cuts.sort_by(|a, b| parameter(line, a.0).total_cmp(&parameter(line, b.0)));
                    cuts.dedup_by(|a, b| a.0 == b.0);
                    let mut previous = None;
                    for &(point, origin) in &cuts {
                        let node = node_of[origin];
                        let at = (point, start);
                        if previous == Some(node) {
                            if let Some(visit) = path.last_mut() {
                                visit.last = at;
                            }
                            continue;
                        }
                        let edge = previous.map(|p: usize| {
                            ([p.min(node), p.max(node)], EdgeUse { part: id, source })
                        });
                        previous = Some(node);
                        match path.last_mut() {
                            Some(visit) if visit.node == node => visit.last = at,
                            _ => path.push(Visit {
                                node,
                                first: at,
                                last: at,
                                edge,
                            }),
                        }
                    }
                }
                let first = nodes.len();
                nodes.extend(path.iter().map(|v| v.node));
                spans.push(first..nodes.len());
                if path.is_empty() {
                    continue;
                }
                let bounds = if part.is_closed || tolerance == 0. {
                    [0, path.len() - 1]
                } else {
                    kept_visits(&path, coords, tolerance)
                };
                if bounds == [0, path.len() - 1] {
                    raw_edges.extend(path.iter().filter_map(|v| v.edge));
                } else {
                    let edges = path
                        .iter()
                        .map(|v| {
                            v.edge.map(|edge| {
                                raw_edges.push(edge);
                                raw_edges.len() - 1
                            })
                        })
                        .collect();
                    trims.push(Trim {
                        part: id,
                        bounds,
                        edges,
                    });
                }
                path.clear();
            }
            (raw_edges, trims, nodes, spans)
        });
        drop(by_source);
        drop(cut_points);
        let mut raw_edges = Vec::with_capacity(segments.len());
        let mut trims = Vec::new();
        // Nodes of all parts, and the span of each part's.
        let mut nodes = Vec::new();
        let mut spans = Vec::with_capacity(parts.len());
        for (edges, run, run_nodes, run_spans) in noded {
            checkpoint("gather.edges");
            let base = raw_edges.len();
            trims.extend(run.into_iter().map(|mut trim| {
                trim.edges.iter_mut().flatten().for_each(|e| *e += base);
                trim
            }));
            raw_edges.extend(edges);
            let base = nodes.len();
            spans.extend(run_spans.into_iter().map(|s| s.start + base..s.end + base));
            nodes.extend(run_nodes);
        }
        if !trims.is_empty() {
            trim_returns(&parts, &nodes, &mut spans, &mut raw_edges, trims, &ties);
        }
        // Contacts by node, each node's in the order found.
        let mut keys = parallel::repeated((0, 0), original_contacts.len(), "contacts");
        workers.fill(&mut keys, |k| (node_of[original_contacts[k].0], k));
        drop(node_of);
        workers.sort(&mut keys, "contacts.sort", Ord::cmp);
        let mut values = parallel::repeated(Contact::default(), keys.len(), "contacts.values");
        workers.fill(&mut values, |i| original_contacts[keys[i].1].1);
        drop(original_contacts);
        let (mut contact_nodes, mut offsets) = (Vec::new(), Vec::new());
        for (i, &(node, _)) in keys.iter().enumerate() {
            every(i, "contacts.nodes");
            if contact_nodes.last() != Some(&node) {
                contact_nodes.push(node);
                offsets.push(i);
            }
        }
        offsets.push(keys.len());
        drop(keys);
        let contacts = Contacts {
            nodes: contact_nodes,
            offsets,
            values,
        };
        let coords = sweep.finish();
        workers.sort(&mut raw_edges, "edges.sort", |a, b| {
            (a.0, a.1.part, a.1.source).cmp(&(b.0, b.1.part, b.1.source))
        });
        let mut uses = Vec::with_capacity(raw_edges.len());
        let mut groups = 0;
        for i in 0..raw_edges.len() {
            every(i, "edges.group");
            let (nodes, usage) = raw_edges[i];
            let start = uses.len();
            uses.push(usage);
            // Compact group ranges in the original entries. The final collect
            // can reuse this allocation because source and target have the same
            // size and alignment; no second full edge array is needed.
            if groups == 0 || raw_edges[groups - 1].0 != nodes {
                raw_edges[groups] = (
                    nodes,
                    EdgeUse {
                        part: start,
                        source: start + 1,
                    },
                );
                groups += 1;
            } else {
                raw_edges[groups - 1].1.source = start + 1;
            }
        }
        raw_edges.truncate(groups);
        let edges = raw_edges
            .into_iter()
            .enumerate()
            .map(|(k, (nodes, range))| {
                every(k, "edges");
                Edge {
                    nodes,
                    uses: range.part..range.source,
                }
            })
            .collect::<Vec<_>>();
        let mut offsets = vec![0; coords.len() + 1];
        for (k, edge) in edges.iter().enumerate() {
            every(k, "edges.count");
            for node in edge.nodes {
                offsets[node + 1] += edge.uses.len();
            }
        }
        for i in 1..offsets.len() {
            every(i - 1, "edges.offsets");
            offsets[i] += offsets[i - 1];
        }
        let mut cursor = parallel::copied(&offsets[..coords.len()], "edges.cursor");
        let mut values =
            parallel::repeated(Incident { part: 0 }, uses.len() * 2, "edges.incident.init");
        for (k, edge) in edges.iter().enumerate() {
            every(k, "edges.incident");
            for usage in &uses[edge.uses.clone()] {
                for node in edge.nodes {
                    values[cursor[node]] = Incident { part: usage.part };
                    cursor[node] += 1;
                }
            }
        }
        drop((cursor, edges, uses));
        let incident = Incidences { offsets, values };
        // The many small lists of each part come last, without checks: freeing
        // them on cancellation would take about as long as finishing.
        cancel::scope(None, || {
            let mut runs = Vec::new();
            let mut rest = &mut parts[..];
            for range in part_ranges(&workers, &starts) {
                let (head, tail) = rest.split_at_mut(range.len());
                runs.push((range.start, head));
                rest = tail;
            }
            workers.each(runs, |(first, run): (usize, &mut [GraphPart])| {
                for (part, span) in run.iter_mut().zip(&spans[first..]) {
                    part.nodes = nodes[span.clone()].to_vec();
                }
            });
        });
        Ok(Self {
            matches: Cache::new(),
            #[cfg(test)]
            match_pairs: Cache::new(),
            match_features: Cache::new(),
            canonical_directions: Cache::new(),
            tolerance,
            contact_nodes: Cache::new(),
            coords,
            sources: segments,
            parts,
            incident,
            contacts,
            order,
        })
    }
}
