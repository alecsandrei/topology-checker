//! Polygon overlay without coordinate quantization.
//!
//! Nodes retain exact binary input coordinates or exact rational intersections.
//! Directed faces carry winding counts. Only emitted vertices are rounded.
use super::{certified, index, location::Location};
use geo::{
    BoundingRect, Coord, Kernel, Line, Polygon, Rect, Winding,
    kernels::{Orientation, RobustKernel},
    line_intersection::{LineIntersection, line_intersection},
    winding_order::WindingOrder,
};
use num_rational::BigRational as Q;
use num_traits::{ToPrimitive, Zero};
use std::{
    cmp::Ordering,
    collections::{HashMap, VecDeque},
    hash::{Hash, Hasher},
};

// Original vertices stay compact. Rational values are allocated only for actual
// intersections and face witnesses, rather than for every input coordinate.
#[derive(Debug, Clone)]
pub(super) enum Exact {
    Input(Coord),
    Rational(Box<RationalNode>),
}
#[derive(Debug, Clone)]
pub(super) struct RationalNode {
    xy: (Q, Q),
    rounded: Coord,
}
impl Exact {
    fn input(p: Coord) -> Self {
        Self::Input(p)
    }
    pub(super) fn rational(x: Q, y: Q) -> Self {
        let rounded = Coord {
            x: x.to_f64().expect("finite intersection"),
            y: y.to_f64().expect("finite intersection"),
        };
        Self::Rational(Box::new(RationalNode {
            xy: (x, y),
            rounded,
        }))
    }
    pub(super) fn xy(&self) -> (Q, Q) {
        match self {
            Self::Input(p) => (
                Q::from_float(p.x).expect("finite input"),
                Q::from_float(p.y).expect("finite input"),
            ),
            Self::Rational(p) => p.xy.clone(),
        }
    }
    pub(super) fn output(&self) -> Coord {
        match self {
            Self::Input(p) => *p,
            Self::Rational(p) => p.rounded,
        }
    }
    fn compare(&self, other: &Self) -> Ordering {
        certified::compare(self, other, 0).then_with(|| certified::compare(self, other, 1))
    }
}
pub(super) fn orient(a: &Exact, b: &Exact, c: &Exact) -> Q {
    if let (Exact::Input(a), Exact::Input(b), Exact::Input(c)) = (a, b, c) {
        return Q::from_integer(match RobustKernel::orient2d(*a, *b, *c) {
            Orientation::Clockwise => (-1).into(),
            Orientation::Collinear => 0.into(),
            Orientation::CounterClockwise => 1.into(),
        });
    }
    let [ax, ay] = certified::coordinates(a);
    let [bx, by] = certified::coordinates(b);
    let [cx, cy] = certified::coordinates(c);
    if let Some(order) = bx
        .sub(ax)
        .mul(cy.sub(ay))
        .sub(by.sub(ay).mul(cx.sub(ax)))
        .sign()
    {
        return Q::from_integer(match order {
            Ordering::Less => (-1).into(),
            Ordering::Greater => 1.into(),
            Ordering::Equal => 0.into(),
        });
    }
    let (ax, ay) = a.xy();
    let (bx, by) = b.xy();
    let (cx, cy) = c.xy();
    (bx - &ax) * (cy - &ay) - (by - ay) * (cx - ax)
}
pub(super) fn on(a: &Exact, b: &Exact, p: &Exact) -> bool {
    if !orient(a, b, p).is_zero() {
        return false;
    }
    (0..2).all(|axis| {
        let x = certified::compare(p, a, axis);
        let y = certified::compare(p, b, axis);
        x.is_eq() || y.is_eq() || x != y
    })
}
pub(super) fn intersects(a: &Exact, b: &Exact, c: &Exact, d: &Exact) -> bool {
    let ab_c = orient(a, b, c);
    let ab_d = orient(a, b, d);
    let cd_a = orient(c, d, a);
    let cd_b = orient(c, d, b);
    (ab_c.is_zero() && on(a, b, c))
        || (ab_d.is_zero() && on(a, b, d))
        || (cd_a.is_zero() && on(c, d, a))
        || (cd_b.is_zero() && on(c, d, b))
        || ab_c.cmp(&Q::zero()) != ab_d.cmp(&Q::zero())
            && !ab_c.is_zero()
            && !ab_d.is_zero()
            && cd_a.cmp(&Q::zero()) != cd_b.cmp(&Q::zero())
            && !cd_a.is_zero()
            && !cd_b.is_zero()
}
fn crossing(a: Line, b: Line) -> Exact {
    let (_, [x, y]) = super::line_graph::exact_crossing(a, b).expect("proper crossing");
    Exact::rational(x, y)
}
#[derive(Debug)]
struct Source {
    line: Line,
    delta: [i32; 2],
}
#[derive(Debug)]
struct Edge {
    nodes: [usize; 2],
    delta: [i32; 2],
}
#[derive(Default)]
struct NodeIds {
    inputs: HashMap<(u64, u64), usize>,
    rational: HashMap<RationalKey, usize>,
}
// All rationals here are normalized by num-rational's constructors and arithmetic.
// Hashing canonical numerators and denominators avoids Ratio's Euclidean hash,
// which supports unnormalized external values and repeatedly divides big integers.
struct RationalKey((Q, Q));
impl PartialEq for RationalKey {
    fn eq(&self, b: &Self) -> bool {
        self.0.0.numer() == b.0.0.numer()
            && self.0.0.denom() == b.0.0.denom()
            && self.0.1.numer() == b.0.1.numer()
            && self.0.1.denom() == b.0.1.denom()
    }
}
impl Eq for RationalKey {}
impl Hash for RationalKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.0.numer().hash(state);
        self.0.0.denom().hash(state);
        self.0.1.numer().hash(state);
        self.0.1.denom().hash(state);
    }
}
fn node(point: Exact, nodes: &mut Vec<Exact>, ids: &mut NodeIds) -> usize {
    if let Exact::Rational(p) = &point {
        let rounded = point.output();
        // Only original vertices are fixed during output rounding. A binary
        // representable intersection still belongs to the local solver.
        if let Some(&id) = ids.inputs.get(&(
            super::cluster::bits(rounded.x),
            super::cluster::bits(rounded.y),
        )) && Exact::input(rounded).xy() == p.xy
        {
            return id;
        }
    }
    let id = nodes.len();
    let old = match &point {
        Exact::Input(p) => ids
            .inputs
            .insert((super::cluster::bits(p.x), super::cluster::bits(p.y)), id),
        Exact::Rational(p) => ids.rational.insert(RationalKey(p.xy.clone()), id),
    };
    if let Some(old) = old {
        // Restore the existing identity; a contact may occur on many segments.
        match &point {
            Exact::Input(p) => {
                ids.inputs
                    .insert((super::cluster::bits(p.x), super::cluster::bits(p.y)), old);
            }
            Exact::Rational(p) => {
                ids.rational.insert(RationalKey(p.xy.clone()), old);
            }
        }
        old
    } else {
        nodes.push(point);
        id
    }
}
fn ends(edge: usize, edges: &[Edge]) -> [usize; 2] {
    let nodes = edges[edge / 2].nodes;
    if edge.is_multiple_of(2) {
        nodes
    } else {
        [nodes[1], nodes[0]]
    }
}
pub(super) fn angle(a: &Exact, b: &Exact, origin: &Exact) -> Ordering {
    if let (Exact::Input(a), Exact::Input(b), Exact::Input(o)) = (a, b, origin) {
        let half = |p: &Coord| p.y < o.y || p.y == o.y && p.x < o.x;
        return half(a)
            .cmp(&half(b))
            .then_with(|| match RobustKernel::orient2d(*o, *a, *b) {
                Orientation::Clockwise => Ordering::Greater,
                Orientation::CounterClockwise => Ordering::Less,
                Orientation::Collinear => Ordering::Equal,
            });
    }
    let half = |p: &Exact| {
        let y = certified::compare(p, origin, 1);
        y.is_lt() || y.is_eq() && certified::compare(p, origin, 0).is_lt()
    };
    half(a)
        .cmp(&half(b))
        .then_with(|| orient(origin, a, b).cmp(&Q::zero()).reverse())
}
fn envelope(a: Coord, b: Coord) -> rstar::AABB<[f64; 2]> {
    rstar::AABB::from_corners(
        [a.x.min(b.x).next_down(), a.y.min(b.y).next_down()],
        [a.x.max(b.x).next_up(), a.y.max(b.y).next_up()],
    )
}
/// Choose a rational point in the left face, with an exact clearance check.
fn witness(h: usize, edges: &[Edge], nodes: &[Exact], tree: &index::Index) -> Exact {
    let [a, b] = ends(h, edges);
    let (a, b) = (&nodes[a], &nodes[b]);
    let two = Q::from_integer(2.into());
    let (ax, ay) = a.xy();
    let (bx, by) = b.xy();
    let mx = (&ax + &bx) / &two;
    let my = (&ay + &by) / &two;
    let middle = Exact::rational(mx.clone(), my.clone());
    let mut divisor = Q::from_integer(1_048_576.into());
    for _ in 0..4096 {
        let p = Exact::rational(&mx - (&by - &ay) / &divisor, &my + (&bx - &ax) / &divisor);
        let clear = tree
            .locate_in_envelope_intersecting(&envelope(middle.output(), p.output()))
            .all(|item| {
                if item.data == h / 2 {
                    return true;
                }
                let [c, d] = edges[item.data].nodes;
                !intersects(&middle, &p, &nodes[c], &nodes[d])
            });
        if clear {
            return p;
        }
        divisor *= &two;
    }
    panic!("rational face witness failed to make progress");
}
fn contains_exact(location: &Location, point: &Exact) -> bool {
    position_exact(location, point) == 1
}
pub(super) fn position_exact(location: &Location, point: &Exact) -> i8 {
    let p = point.output();
    let query = rstar::AABB::from_corners(
        [p.x.next_down(), p.y.next_down()],
        [f64::MAX, p.y.next_up()],
    );
    let mut inside = false;
    for item in location.tree.locate_in_envelope_intersecting(&query) {
        let s = location.segments[item.data];
        let a = Exact::input(s.start);
        let b = Exact::input(s.end);
        if on(&a, &b, point) {
            return 0;
        }
        if certified::compare(&a, point, 1).is_gt() != certified::compare(&b, point, 1).is_gt()
            && (orient(&a, &b, point) > Q::zero()) == (s.end.y > s.start.y)
        {
            inside = !inside;
        }
    }
    if inside { 1 } else { -1 }
}
fn selected(w: [i32; 2], intersection: bool) -> bool {
    if intersection {
        w[0] > 0 && w[1] > 0
    } else {
        w[0] + w[1] > 0
    }
}

fn component(polygons: &[(&Polygon, usize)], intersection: bool) -> super::precision::Component {
    let mut sources = Vec::new();
    for &(p, operand) in polygons {
        for (i, ring) in std::iter::once(p.exterior())
            .chain(p.interiors())
            .enumerate()
        {
            let ccw = ring.winding_order() == Some(WindingOrder::CounterClockwise);
            let mut delta = [0, 0];
            delta[operand] = if ccw == (i == 0) { 1 } else { -1 };
            sources.extend(ring.lines().map(|line| Source { line, delta }));
        }
    }
    let tree = index::build(
        sources
            .iter()
            .enumerate()
            .map(|(i, s)| (i, s.line.bounding_rect())),
    );
    let mut nodes = Vec::new();
    let mut ids = NodeIds::default();
    let mut cuts = sources
        .iter()
        .map(|s| {
            vec![
                node(Exact::input(s.line.start), &mut nodes, &mut ids),
                node(Exact::input(s.line.end), &mut nodes, &mut ids),
            ]
        })
        .collect::<Vec<_>>();
    for (i, s) in sources.iter().enumerate() {
        crate::cancel::check();
        for item in tree.locate_in_envelope_intersecting(&index::envelope(s.line.bounding_rect())) {
            let j = item.data;
            if j <= i {
                continue;
            }
            if let Some(contact) = line_intersection(s.line, sources[j].line) {
                match contact {
                    LineIntersection::SinglePoint {
                        intersection,
                        is_proper,
                    } => {
                        let p = if is_proper {
                            crossing(s.line, sources[j].line)
                        } else {
                            Exact::input(intersection)
                        };
                        let id = node(p, &mut nodes, &mut ids);
                        cuts[i].push(id);
                        cuts[j].push(id);
                    }
                    LineIntersection::Collinear { intersection } => {
                        for p in [intersection.start, intersection.end] {
                            let id = node(Exact::input(p), &mut nodes, &mut ids);
                            cuts[i].push(id);
                            cuts[j].push(id);
                        }
                    }
                }
            }
        }
    }
    drop(ids);
    drop(tree);
    let mut source_order = Vec::new();
    let mut edges: Vec<Edge> = Vec::new();
    let mut keys = HashMap::new();
    for (s, mut cuts) in sources.iter().zip(cuts) {
        crate::cancel::check();
        cuts.sort_unstable_by(|&a, &b| nodes[a].compare(&nodes[b]));
        cuts.dedup();
        // Source direction must use the same geometric order as its cut nodes.
        // IEEE signed zero denotes one coordinate, not two distinct x positions.
        let forward =
            Exact::input(s.line.start).compare(&Exact::input(s.line.end)) == Ordering::Less;
        for pair in cuts.windows(2) {
            let a = pair[0];
            let b = pair[1];
            let axis = usize::from(
                (s.line.end.y - s.line.start.y).abs() > (s.line.end.x - s.line.start.x).abs(),
            );
            source_order.push((a, b, axis));
            let key = (a.min(b), a.max(b));
            let id = *keys.entry(key).or_insert_with(|| {
                let id = edges.len();
                edges.push(Edge {
                    nodes: [key.0, key.1],
                    delta: [0, 0],
                });
                id
            });
            let sign = if (a == key.0) == forward { 1 } else { -1 };
            for k in 0..2 {
                edges[id].delta[k] += sign * s.delta[k];
            }
        }
    }
    drop(keys);
    drop(sources);
    let mut outgoing = vec![Vec::new(); nodes.len()];
    for h in 0..edges.len() * 2 {
        outgoing[ends(h, &edges)[0]].push(h);
    }
    for (id, hs) in outgoing.iter_mut().enumerate() {
        crate::cancel::check();
        hs.sort_unstable_by(|&a, &b| {
            angle(
                &nodes[ends(a, &edges)[1]],
                &nodes[ends(b, &edges)[1]],
                &nodes[id],
            )
        });
    }
    let mut next = vec![0; edges.len() * 2];
    for hs in &outgoing {
        for (i, &h) in hs.iter().enumerate() {
            next[h ^ 1] = hs[(i + hs.len() - 1) % hs.len()];
        }
    }
    let mut faces = vec![usize::MAX; next.len()];
    let mut representatives = Vec::new();
    for start in 0..next.len() {
        crate::cancel::check();
        if faces[start] != usize::MAX {
            continue;
        }
        let id = representatives.len();
        representatives.push(start);
        let mut h = start;
        loop {
            assert_eq!(faces[h], usize::MAX, "face traversal must progress");
            faces[h] = id;
            h = next[h];
            if h == start {
                break;
            }
        }
    }
    let tree = index::build(edges.iter().enumerate().map(|(i, e)| {
        (
            i,
            Rect::new(nodes[e.nodes[0]].output(), nodes[e.nodes[1]].output()),
        )
    }));
    let locations = polygons
        .iter()
        .map(|&(p, operand)| (Location::new(p), operand))
        .collect::<Vec<_>>();
    let location_bounds = index::build(
        polygons
            .iter()
            .enumerate()
            .filter_map(|(i, (p, _))| p.bounding_rect().map(|r| (i, r))),
    );
    let mut adjacent = vec![Vec::new(); representatives.len()];
    for (i, e) in edges.iter().enumerate() {
        let left = faces[i * 2];
        let right = faces[i * 2 + 1];
        adjacent[right].push((left, e.delta));
        adjacent[left].push((right, e.delta.map(|d| -d)));
    }
    let mut winding = vec![None; representatives.len()];
    for seed in 0..representatives.len() {
        if winding[seed].is_some() {
            continue;
        }
        let sample = witness(representatives[seed], &edges, &nodes, &tree);
        let mut value = [0, 0];
        let p = sample.output();
        let query = rstar::AABB::from_corners(
            [p.x.next_down(), p.y.next_down()],
            [p.x.next_up(), p.y.next_up()],
        );
        for item in location_bounds.locate_in_envelope_intersecting(&query) {
            let (location, operand) = &locations[item.data];
            if contains_exact(location, &sample) {
                value[*operand] += 1;
            }
        }
        winding[seed] = Some(value);
        let mut queue = VecDeque::from([seed]);
        while let Some(face) = queue.pop_front() {
            let value = winding[face].expect("queued face");
            for &(other, delta) in &adjacent[face] {
                let value = [value[0] + delta[0], value[1] + delta[1]];
                if let Some(old) = winding[other] {
                    assert_eq!(old, value, "consistent face winding");
                } else {
                    winding[other] = Some(value);
                    queue.push_back(other);
                }
            }
        }
    }
    drop(tree);
    drop(locations);
    drop(adjacent);
    let keep = (0..next.len())
        .map(|h| {
            selected(winding[faces[h]].expect("classified face"), intersection)
                && !selected(
                    winding[faces[h ^ 1]].expect("classified face"),
                    intersection,
                )
        })
        .collect::<Vec<_>>();
    let mut boundary_next = vec![usize::MAX; next.len()];
    for h in 0..next.len() {
        if !keep[h] {
            continue;
        }
        let hs = &outgoing[ends(h, &edges)[1]];
        let i = hs.iter().position(|&v| v == h ^ 1).expect("twin incidence");
        boundary_next[h] = (1..=hs.len())
            .map(|k| hs[(i + hs.len() - k) % hs.len()])
            .find(|&v| keep[v])
            .expect("boundary continuation");
    }
    let mut visited = vec![false; next.len()];
    let mut shells = Vec::new();
    let mut holes = Vec::new();
    for start in 0..next.len() {
        crate::cancel::check();
        if !keep[start] || visited[start] {
            continue;
        }
        let mut h = start;
        let mut ring = Vec::new();
        loop {
            assert!(!visited[h], "boundary traversal must progress");
            visited[h] = true;
            ring.push(ends(h, &edges)[0]);
            h = boundary_next[h];
            if h == start {
                break;
            }
        }
        ring.push(ring[0]);
        // Split by exact node identity, before any coordinate conversion.
        let mut path = Vec::new();
        let mut positions = HashMap::new();
        for id in ring {
            if let Some(&start) = positions.get(&id) {
                let mut cycle = vec![id];
                let tail = path.split_off(start + 1);
                for p in &tail {
                    positions.remove(p);
                }
                cycle.extend(tail);
                cycle.push(id);
                match super::precision::area_sign(&nodes, &cycle) {
                    Ordering::Greater => shells.push(cycle),
                    Ordering::Less => holes.push(cycle),
                    Ordering::Equal => {}
                }
            } else {
                positions.insert(id, path.len());
                path.push(id);
            }
        }
    }
    super::precision::Component::new(nodes, shells, holes, source_order)
}

/// Partition by spatial connectivity before constructing any arrangement scratch.
pub(crate) fn overlay<'a>(
    a: impl IntoIterator<Item = &'a Polygon>,
    b: impl IntoIterator<Item = &'a Polygon>,
    intersection: bool,
) -> super::precision::Geometry {
    let polygons = a
        .into_iter()
        .map(|p| (p, 0))
        .chain(b.into_iter().map(|p| (p, 1)))
        .collect::<Vec<_>>();
    if polygons.is_empty() {
        return super::precision::Geometry::default();
    }
    let tree = index::build(
        polygons
            .iter()
            .enumerate()
            .filter_map(|(i, (p, _))| p.bounding_rect().map(|r| (i, r))),
    );
    let mut visited = vec![false; polygons.len()];
    let mut output = Vec::new();
    for seed in 0..polygons.len() {
        crate::cancel::check();
        if visited[seed] {
            continue;
        }
        visited[seed] = true;
        let mut queue = VecDeque::from([seed]);
        let mut group = Vec::new();
        while let Some(i) = queue.pop_front() {
            group.push(polygons[i]);
            if let Some(rect) = polygons[i].0.bounding_rect() {
                for item in tree.locate_in_envelope_intersecting(&index::envelope(rect)) {
                    if !visited[item.data] {
                        visited[item.data] = true;
                        queue.push_back(item.data);
                    }
                }
            }
        }
        if !intersection || group.iter().any(|p| p.1 == 0) && group.iter().any(|p| p.1 == 1) {
            output.push(component(&group, intersection));
        }
    }
    super::precision::Geometry(output)
}

pub(crate) fn midpoint_inside(location: &Location, a: Coord, b: Coord) -> bool {
    midpoint_position(location, a, b) == 1
}
pub(crate) fn midpoint_position(location: &Location, a: Coord, b: Coord) -> i8 {
    if let Some(position) = certified_midpoint_position(location, a, b) {
        return position;
    }
    let (ax, ay) = Exact::input(a).xy();
    let (bx, by) = Exact::input(b).xy();
    let two = Q::from_integer(2.into());
    position_exact(
        location,
        &Exact::rational((ax + bx) / &two, (ay + by) / two),
    )
}

fn certified_midpoint_position(location: &Location, a: Coord, b: Coord) -> Option<i8> {
    use certified::Interval;
    let a = certified::coordinates(&Exact::input(a));
    let b = certified::coordinates(&Exact::input(b));
    let p = [
        a[0].add(b[0]).mul(Interval(0.5, 0.5)),
        a[1].add(b[1]).mul(Interval(0.5, 0.5)),
    ];
    certified_position(location, p)
}

/// Ray parity of every point in a coordinate box, or `None` when the box may
/// touch the boundary or straddle a decision.
pub(super) fn certified_position(location: &Location, p: [certified::Interval; 2]) -> Option<i8> {
    if p.iter().any(|i| !i.0.is_finite() || !i.1.is_finite()) {
        return None;
    }
    let query = rstar::AABB::from_corners([p[0].0, p[1].0], [f64::MAX, p[1].1]);
    let above = |y: f64| {
        if y > p[1].1 {
            Some(true)
        } else if y <= p[1].0 {
            Some(false)
        } else {
            None
        }
    };
    let mut inside = false;
    for item in location.tree.locate_in_envelope_intersecting(&query) {
        crate::cancel::check();
        let s = location.segments[item.data];
        let bounds = s.bounding_rect();
        let contact_possible = p[0].1 >= bounds.min().x
            && p[0].0 <= bounds.max().x
            && p[1].1 >= bounds.min().y
            && p[1].0 <= bounds.max().y;
        if s.start.y == s.end.y && !contact_possible {
            continue;
        }
        let (Some(a_above), Some(b_above)) = (above(s.start.y), above(s.end.y)) else {
            return None;
        };
        if a_above == b_above && !contact_possible {
            continue;
        }
        let [ax, ay] = certified::coordinates(&Exact::input(s.start));
        let [bx, by] = certified::coordinates(&Exact::input(s.end));
        let side = bx
            .sub(ax)
            .mul(p[1].sub(ay))
            .sub(by.sub(ay).mul(p[0].sub(ax)))
            .sign();
        let side = side?;
        if a_above != b_above && side.is_gt() == b_above {
            inside = !inside;
        }
    }
    Some(if inside { 1 } else { -1 })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn midpoint_certification_keeps_boundary_and_sub_ulp_fallbacks() {
        let rectangle = |left, right| {
            Location::new(
                &Rect::new(Coord { x: left, y: 0. }, Coord { x: right, y: 4. }).to_polygon(),
            )
        };
        let square = rectangle(0., 4.);
        let (a, b) = (Coord { x: 1., y: 2. }, Coord { x: 3., y: 2. });
        assert_eq!(certified_midpoint_position(&square, a, b), Some(1));
        let (a, b) = (Coord { x: 0., y: 1. }, Coord { x: 0., y: 3. });
        assert_eq!(certified_midpoint_position(&square, a, b), None);
        assert_eq!(midpoint_position(&square, a, b), 0);
        let (a, b) = (
            Coord { x: 1., y: 2. },
            Coord {
                x: 1f64.next_up(),
                y: 2.,
            },
        );
        let narrow = rectangle(a.x, b.x);
        assert_eq!(certified_midpoint_position(&narrow, a, b), None);
        assert_eq!(midpoint_position(&narrow, a, b), 1);
    }
    #[test]
    fn representable_intersections_are_movable_unless_they_are_original_vertices() {
        let mut nodes = Vec::new();
        let mut ids = NodeIds::default();
        let original = node(Exact::input(Coord { x: 0., y: 0. }), &mut nodes, &mut ids);
        let a = node(Exact::rational(Q::zero(), Q::zero()), &mut nodes, &mut ids);
        assert_eq!(a, original);
        assert!(matches!(nodes[a], Exact::Input(_)));
        let b = node(
            Exact::rational(Q::from_integer(1.into()), Q::zero()),
            &mut nodes,
            &mut ids,
        );
        assert!(matches!(nodes[b], Exact::Rational(_)));
        assert_eq!(
            b,
            node(
                Exact::rational(Q::from_integer(1.into()), Q::zero()),
                &mut nodes,
                &mut ids
            )
        );
    }
}
