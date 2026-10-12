//! Polygon overlay without coordinate quantization.
//!
//! Nodes retain exact binary input coordinates or exact rational intersections.
//! Directed faces carry winding counts. Only emitted vertices are rounded.
use super::{index, location::Location};
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
};

// Original vertices stay compact. Rational values are allocated only for actual
// intersections and face witnesses, rather than for every input coordinate.
#[derive(Debug, Clone)]
pub(super) enum Exact {
    Input(Coord),
    Rational(Box<(Q, Q)>),
}
impl Exact {
    fn input(p: Coord) -> Self {
        Self::Input(p)
    }
    fn rational(x: Q, y: Q) -> Self {
        Self::Rational(Box::new((x, y)))
    }
    pub(super) fn xy(&self) -> (Q, Q) {
        match self {
            Self::Input(p) => (
                Q::from_float(p.x).expect("finite input"),
                Q::from_float(p.y).expect("finite input"),
            ),
            Self::Rational(p) => (**p).clone(),
        }
    }
    pub(super) fn output(&self) -> Coord {
        match self {
            Self::Input(p) => *p,
            Self::Rational(p) => Coord {
                x: p.0.to_f64().expect("finite intersection"),
                y: p.1.to_f64().expect("finite intersection"),
            },
        }
    }
    fn compare(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Self::Input(a), Self::Input(b)) => {
                a.x.partial_cmp(&b.x)
                    .expect("finite")
                    .then_with(|| a.y.partial_cmp(&b.y).expect("finite"))
            }
            _ => {
                let a = self.xy();
                let b = other.xy();
                a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1))
            }
        }
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
    let (ax, ay) = a.xy();
    let (bx, by) = b.xy();
    let (cx, cy) = c.xy();
    (bx - &ax) * (cy - &ay) - (by - ay) * (cx - ax)
}
pub(super) fn on(a: &Exact, b: &Exact, p: &Exact) -> bool {
    if !orient(a, b, p).is_zero() {
        return false;
    }
    let (ax, ay) = a.xy();
    let (bx, by) = b.xy();
    let (px, py) = p.xy();
    px >= ax.clone().min(bx.clone())
        && px <= ax.max(bx)
        && py >= ay.clone().min(by.clone())
        && py <= ay.max(by)
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
    let (x, y) = Exact::input(a.start).xy();
    let a0 = (x, y);
    let (x, y) = Exact::input(a.end).xy();
    let a1 = (x, y);
    let (x, y) = Exact::input(b.start).xy();
    let b0 = (x, y);
    let (x, y) = Exact::input(b.end).xy();
    let b1 = (x, y);
    let rx = &a1.0 - &a0.0;
    let ry = &a1.1 - &a0.1;
    let sx = &b1.0 - &b0.0;
    let sy = &b1.1 - &b0.1;
    let t = ((&b0.0 - &a0.0) * &sy - (&b0.1 - &a0.1) * &sx) / (&rx * &sy - &ry * &sx);
    Exact::rational(a0.0 + &t * rx, a0.1 + t * ry)
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
    rational: HashMap<(Q, Q), usize>,
}
fn node(point: Exact, nodes: &mut Vec<Exact>, ids: &mut NodeIds) -> usize {
    if let Exact::Rational(p) = &point {
        let rounded = point.output();
        if Exact::input(rounded).xy() == **p {
            // Only original vertices are fixed during output rounding. A binary
            // representable intersection still belongs to the local solver.
            if let Some(&id) = ids.inputs.get(&(
                super::cluster::bits(rounded.x),
                super::cluster::bits(rounded.y),
            )) {
                return id;
            }
        }
    }
    let id = nodes.len();
    let old = match &point {
        Exact::Input(p) => ids
            .inputs
            .insert((super::cluster::bits(p.x), super::cluster::bits(p.y)), id),
        Exact::Rational(p) => ids.rational.insert((**p).clone(), id),
    };
    if let Some(old) = old {
        // Restore the existing identity; a contact may occur on many segments.
        match &point {
            Exact::Input(p) => {
                ids.inputs
                    .insert((super::cluster::bits(p.x), super::cluster::bits(p.y)), old);
            }
            Exact::Rational(p) => {
                ids.rational.insert((**p).clone(), old);
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
fn angle(a: &Exact, b: &Exact, origin: &Exact) -> Ordering {
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
    let (ox, oy) = origin.xy();
    let (ax, ay) = a.xy();
    let (bx, by) = b.xy();
    let (ax, ay, bx, by) = (ax - &ox, ay - &oy, bx - ox, by - oy);
    let half = |x: &Q, y: &Q| y < &Q::zero() || y.is_zero() && x < &Q::zero();
    half(&ax, &ay)
        .cmp(&half(&bx, &by))
        .then_with(|| (&ax * &by - &ay * &bx).cmp(&Q::zero()).reverse())
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
    let p = point.output();
    let query = rstar::AABB::from_corners(
        [p.x.next_down(), p.y.next_down()],
        [f64::MAX, p.y.next_up()],
    );
    let (_, py) = point.xy();
    let mut inside = false;
    for item in location.tree.locate_in_envelope_intersecting(&query) {
        let s = location.segments[item.data];
        let a = Exact::input(s.start);
        let b = Exact::input(s.end);
        let (_, ay) = a.xy();
        let (_, by) = b.xy();
        if (ay > py) != (by > py) && (orient(&a, &b, point) > Q::zero()) == (by > ay) {
            inside = !inside;
        }
    }
    inside
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
        for (location, operand) in &locations {
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
    let (ax, ay) = Exact::input(a).xy();
    let (bx, by) = Exact::input(b).xy();
    let two = Q::from_integer(2.into());
    contains_exact(
        location,
        &Exact::rational((ax + bx) / &two, (ay + by) / two),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
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
