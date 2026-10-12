//! Exact overlay boundaries and constrained conversion to binary output coordinates.
use super::{
    arrangement::{Exact, intersects, on, orient},
    certified::{self, Interval},
    index,
};
use crate::Error;
use geo::{BoundingRect, Coord, Kernel, Line, LineString, MultiPolygon, Polygon, Rect};
use num_rational::BigRational as Q;
use num_traits::{Signed, ToPrimitive, Zero};
use std::{
    borrow::Cow,
    collections::{BTreeSet, HashMap},
    sync::OnceLock,
};

const ROUNDING_RADII: [i32; 7] = [1, 2, 4, 8, 16, 32, 64];
const MAX_ROUNDING_STEPS: i32 = 64;
const ASSIGNMENT_BUDGET: usize = 100_000;

pub(super) fn area(nodes: &[Exact], ring: &[usize]) -> Q {
    #[cfg(test)]
    EXACT_AREAS.with(|n| n.set(n.get() + 1));
    let (ox, oy) = nodes[ring[0]].xy();
    let mut result = Q::zero();
    for pair in ring.windows(2) {
        crate::cancel::check();
        let (ax, ay) = nodes[pair[0]].xy();
        let (bx, by) = nodes[pair[1]].xy();
        result += (ax - &ox) * (by - &oy) - (ay - &oy) * (bx - &ox);
    }
    result / Q::from_integer(2.into())
}
#[cfg(test)]
thread_local! { static EXACT_AREAS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }
fn area_bounds(nodes: &[Exact], ring: &[usize]) -> Interval {
    let [ox, oy] = certified::coordinates(&nodes[ring[0]]);
    let mut sum = Interval(0., 0.);
    for pair in ring.windows(2) {
        crate::cancel::check();
        let [ax, ay] = certified::coordinates(&nodes[pair[0]]);
        let [bx, by] = certified::coordinates(&nodes[pair[1]]);
        sum = sum.add(ax.sub(ox).mul(by.sub(oy)).sub(ay.sub(oy).mul(bx.sub(ox))));
    }
    sum.mul(Interval(0.5, 0.5))
}
pub(super) fn area_sign(nodes: &[Exact], ring: &[usize]) -> std::cmp::Ordering {
    area_bounds(nodes, ring)
        .sign()
        .unwrap_or_else(|| area(nodes, ring).cmp(&Q::zero()))
}
#[derive(Debug)]
struct ExactLocation {
    edges: Vec<[usize; 2]>,
    tree: index::Index,
}
impl ExactLocation {
    fn new(nodes: &[Exact], rings: impl IntoIterator<Item = impl AsRef<[usize]>>) -> Self {
        let edges = rings
            .into_iter()
            .flat_map(|r| {
                r.as_ref()
                    .windows(2)
                    .map(|p| [p[0], p[1]])
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let tree = index::build(edges.iter().enumerate().map(|(i, e)| {
            (
                i,
                expanded_bounds(nodes[e[0]].output(), nodes[e[1]].output()),
            )
        }));
        Self { edges, tree }
    }
    fn position(&self, nodes: &[Exact], p: &Exact) -> i8 {
        let rounded = p.output();
        let query = rstar::AABB::from_corners(
            [rounded.x.next_down(), rounded.y.next_down()],
            [f64::MAX, rounded.y.next_up()],
        );
        let mut inside = false;
        for item in self.tree.locate_in_envelope_intersecting(&query) {
            let [a, b] = self.edges[item.data];
            let (a, b) = (&nodes[a], &nodes[b]);
            if on(a, b, p) {
                return 0;
            }
            if certified::compare(a, p, 1).is_gt() != certified::compare(b, p, 1).is_gt()
                && (orient(a, b, p) > Q::zero()) == certified::compare(b, a, 1).is_gt()
            {
                inside = !inside;
            }
        }
        if inside { 1 } else { -1 }
    }
}
// Rational bounds for each square root. Refinement certifies the inclusive
// threshold, including equality when all edge lengths are rational.
fn substantial(nodes: &[Exact], rings: &[&[usize]], tolerance: f64) -> bool {
    let a_bounds = area_bounds(nodes, rings[0]).abs();
    let a_bounds = rings[1..]
        .iter()
        .fold(a_bounds, |sum, r| sum.sub(area_bounds(nodes, r).abs()));
    if a_bounds.1 <= 0. {
        return false;
    }
    if tolerance == 0. && a_bounds.0 > 0. {
        return true;
    }
    if tolerance > 0. && a_bounds.0 > 0. {
        let perimeter =
            rings
                .iter()
                .flat_map(|r| r.windows(2))
                .fold(Interval(0., 0.), |sum, edge| {
                    let [ax, ay] = certified::coordinates(&nodes[edge[0]]);
                    let [bx, by] = certified::coordinates(&nodes[edge[1]]);
                    sum.add(bx.sub(ax).square().add(by.sub(ay).square()).sqrt())
                });
        let margin = a_bounds
            .mul(Interval(2., 2.))
            .sub(perimeter.mul(Interval(tolerance, tolerance)));
        if let Some(sign) = margin.sign() {
            return sign.is_gt();
        }
    }
    let a =
        area(nodes, rings[0]).abs() - rings[1..].iter().map(|r| area(nodes, r).abs()).sum::<Q>();
    if a <= Q::zero() {
        return false;
    }
    if tolerance == 0. {
        return true;
    }
    let threshold =
        a * Q::from_integer(2.into()) / Q::from_float(tolerance).expect("finite tolerance");
    let lengths = rings
        .iter()
        .flat_map(|r| r.windows(2))
        .map(|p| {
            let (ax, ay) = nodes[p[0]].xy();
            let (bx, by) = nodes[p[1]].xy();
            let (x, y) = (bx - ax, by - ay);
            &x * &x + &y * &y
        })
        .collect::<Vec<_>>();
    let mut bits = 64usize;
    loop {
        crate::cancel::check();
        let scale = Q::from_integer((Q::from_integer(2.into()).numer() << bits).clone());
        let mut lower = Q::zero();
        let mut upper = Q::zero();
        for square in &lengths {
            let scaled = square * &scale * &scale;
            let root = scaled.to_integer().sqrt();
            let lo = Q::from_integer(root) / &scale;
            let hi = if &lo * &lo == *square {
                lo.clone()
            } else {
                &lo + Q::from_integer(1.into()) / &scale
            };
            lower += lo;
            upper += hi;
        }
        if upper <= threshold {
            return true;
        }
        if lower > threshold {
            return false;
        }
        bits *= 2;
    }
}
#[derive(Debug)]
pub(super) struct ExactPolygon {
    pub shell: Vec<usize>,
    pub holes: Vec<Vec<usize>>,
}
struct Selection<'a> {
    shell: &'a [usize],
    holes: Vec<&'a [usize]>,
}
impl<'a> Selection<'a> {
    fn polygon(p: &'a ExactPolygon) -> Self {
        Self {
            shell: &p.shell,
            holes: p.holes.iter().map(Vec::as_slice).collect(),
        }
    }
}
#[derive(Debug)]
pub(super) struct Component {
    nodes: Vec<Exact>,
    polygons: Vec<ExactPolygon>,
    source_order: Vec<(usize, usize, usize)>,
    locations: crate::cache::Cache<Vec<ExactLocation>>,
    coordinates: crate::cache::Cache<Result<Vec<Coord>, Error>>,
}
impl Component {
    pub fn new(
        nodes: Vec<Exact>,
        shells: Vec<Vec<usize>>,
        holes: Vec<Vec<usize>>,
        source_order: Vec<(usize, usize, usize)>,
    ) -> Self {
        let shell_locations = if holes.is_empty() {
            vec![]
        } else {
            shells
                .iter()
                .map(|r| ExactLocation::new(&nodes, [r]))
                .collect::<Vec<_>>()
        };
        let shell_areas = shells
            .iter()
            .map(|_| OnceLock::<Q>::new())
            .collect::<Vec<_>>();
        let tree = index::build(
            shells
                .iter()
                .enumerate()
                .filter(|_| !holes.is_empty())
                .map(|(i, r)| (i, ring_bounds(&nodes, r))),
        );
        let mut polygons = shells
            .into_iter()
            .map(|shell| ExactPolygon {
                shell,
                holes: vec![],
            })
            .collect::<Vec<_>>();
        for hole in holes {
            crate::cancel::check();
            let p = nodes[hole[0]].output();
            let owner = tree
                .locate_in_envelope_intersecting(&index::at(p, 0.))
                .filter(|item| {
                    // A point contact alone cannot establish ownership. At least one
                    // exact hole vertex must lie strictly inside its shell.
                    hole.iter()
                        .any(|&id| shell_locations[item.data].position(&nodes, &nodes[id]) == 1)
                })
                .min_by(|a, b| {
                    shell_areas[a.data]
                        .get_or_init(|| area(&nodes, &polygons[a.data].shell))
                        .cmp(
                            shell_areas[b.data]
                                .get_or_init(|| area(&nodes, &polygons[b.data].shell)),
                        )
                })
                .expect("exact hole shell")
                .data;
            polygons[owner].holes.push(hole);
        }
        Self {
            nodes,
            polygons,
            source_order,
            locations: crate::cache::Cache::new(),
            coordinates: crate::cache::Cache::new(),
        }
    }
    fn locations(&self) -> &[ExactLocation] {
        self.locations.get_or_init(|| {
            self.polygons
                .iter()
                .map(|p| ExactLocation::new(&self.nodes, std::iter::once(&p.shell).chain(&p.holes)))
                .collect()
        })
    }
    fn coordinates(&self) -> Result<&[Coord], Error> {
        self.coordinates
            .get_or_init(|| self.solve_coordinates())
            .as_ref()
            .map(Vec::as_slice)
            .map_err(Clone::clone)
    }
    fn selected_coordinates<'a>(
        &'a self,
        selected: &[Selection<'_>],
    ) -> Result<Cow<'a, [Coord]>, Error> {
        let all = selected.len() == self.polygons.len()
            && selected.iter().zip(&self.polygons).all(|(s, p)| {
                std::ptr::eq(s.shell, p.shell.as_slice()) && s.holes.len() == p.holes.len()
            });
        if all {
            self.coordinates().map(Cow::Borrowed)
        } else {
            self.solve_selected(selected).map(Cow::Owned)
        }
    }
    fn solve_coordinates(&self) -> Result<Vec<Coord>, Error> {
        self.solve_selected(
            &self
                .polygons
                .iter()
                .map(Selection::polygon)
                .collect::<Vec<_>>(),
        )
    }
    fn solve_selected(&self, selected: &[Selection<'_>]) -> Result<Vec<Coord>, Error> {
        let mut output = self.nodes.iter().map(Exact::output).collect::<Vec<_>>();
        let rings = selected
            .iter()
            .flat_map(|p| std::iter::once(p.shell).chain(p.holes.iter().copied()))
            .collect::<Vec<_>>();
        let active = rings
            .iter()
            .flat_map(|r| r.iter().copied())
            .collect::<BTreeSet<_>>();
        if active
            .iter()
            .all(|&i| matches!(self.nodes[i], Exact::Input(_)))
        {
            return Ok(output);
        }
        let edges = rings
            .iter()
            .flat_map(|r| r.windows(2).map(|p| [p[0], p[1]]))
            .collect::<Vec<_>>();
        // Every possible contact within the largest search radius is included.
        let tree = index::build(
            edges
                .iter()
                .enumerate()
                .map(|(i, e)| (i, expanded_bounds(output[e[0]], output[e[1]]))),
        );
        let mut constraints = Vec::new();
        let mut neighbors = vec![BTreeSet::new(); self.nodes.len()];
        for &[a, b] in &edges {
            neighbors[a].insert(b);
            neighbors[b].insert(a);
        }
        for (i, a) in edges.iter().enumerate() {
            crate::cancel::check();
            for item in tree.locate_in_envelope_intersecting(&index::envelope(expanded_bounds(
                output[a[0]],
                output[a[1]],
            ))) {
                if item.data <= i {
                    continue;
                }
                let b = edges[item.data];
                let shared = a.iter().find(|id| b.contains(id)).copied();
                if let Some(o) = shared {
                    let x = *a.iter().find(|&&id| id != o).expect("nonzero edge");
                    let y = *b.iter().find(|&&id| id != o).expect("nonzero edge");
                    constraints.push(Constraint::NonOverlap(o, x, y));
                } else {
                    constraints.push(Constraint::Disjoint(*a, b));
                }
            }
            constraints.push(Constraint::Distinct(a[0], a[1]));
        }
        for (o, rays) in neighbors.iter().enumerate() {
            if rays.len() > 2 {
                let mut rays = rays.iter().copied().collect::<Vec<_>>();
                rays.sort_by(|&a, &b| {
                    super::arrangement::angle(&self.nodes[a], &self.nodes[b], &self.nodes[o])
                });
                constraints.push(Constraint::Incidence(o, rays));
            }
        }
        for &(a, b, _) in &self.source_order {
            if active.contains(&a)
                && active.contains(&b)
                && !(matches!(self.nodes[a], Exact::Input(_))
                    && matches!(self.nodes[b], Exact::Input(_)))
            {
                constraints.push(Constraint::Order(
                    a,
                    b,
                    Box::new(Direction::new(&self.nodes[a], &self.nodes[b])),
                ));
            }
        }
        for ring in &rings {
            constraints.push(Constraint::Orientation(
                ring.to_vec(),
                area_sign(&self.nodes, ring).is_gt(),
            ));
        }
        for polygon in selected {
            let shell = ExactLocation::new(&self.nodes, [polygon.shell]);
            for hole in &polygon.holes {
                let anchor = *hole
                    .iter()
                    .find(|&&id| shell.position(&self.nodes, &self.nodes[id]) == 1)
                    .expect("exact interior hole anchor");
                let p = output[anchor];
                let query = rstar::AABB::from_corners(
                    [
                        step(p.x, -MAX_ROUNDING_STEPS),
                        step(p.y, -MAX_ROUNDING_STEPS),
                    ],
                    [f64::MAX, step(p.y, MAX_ROUNDING_STEPS)],
                );
                let ray_edges = shell
                    .tree
                    .locate_in_envelope_intersecting(&query)
                    .map(|item| shell.edges[item.data])
                    .collect::<Vec<_>>();
                constraints.push(Constraint::Ownership(ray_edges, anchor));
            }
        }
        // Constraints certified over the entire largest candidate box cannot
        // couple independent rounding neighborhoods. In particular, long source
        // intervals must not join every intersection along a large ring into one
        // backtracking problem merely because they share incidence.
        let domains = self
            .nodes
            .iter()
            .map(|p| {
                let q = p.output();
                if matches!(p, Exact::Input(_)) {
                    [Interval(q.x, q.x), Interval(q.y, q.y)]
                } else {
                    [
                        Interval(
                            step(q.x, -MAX_ROUNDING_STEPS),
                            step(q.x, MAX_ROUNDING_STEPS),
                        ),
                        Interval(
                            step(q.y, -MAX_ROUNDING_STEPS),
                            step(q.y, MAX_ROUNDING_STEPS),
                        ),
                    ]
                }
            })
            .collect::<Vec<_>>();
        constraints.retain(|c| {
            crate::cancel::check();
            !c.guaranteed(&domains)
        });
        // Constraint incidence defines independent affected neighborhoods.
        let failed = constraints
            .iter()
            .enumerate()
            .filter_map(|(i, c)| (!c.satisfied(&output)).then_some(i))
            .collect::<Vec<_>>();
        if failed.is_empty() {
            return Ok(output);
        }
        let movable = active
            .iter()
            .copied()
            .filter(|&i| matches!(self.nodes[i], Exact::Rational(_)))
            .collect::<BTreeSet<_>>();
        let mut pending = failed
            .into_iter()
            .flat_map(|i| constraints[i].ids())
            .filter(|i| movable.contains(i))
            .collect::<BTreeSet<_>>();
        if pending.is_empty() {
            return Err(precision_error(
                "fixed input vertices cannot preserve the exact embedding",
            ));
        }
        let mut incidence = vec![Vec::new(); self.nodes.len()];
        for (i, c) in constraints.iter().enumerate() {
            for id in c.ids() {
                incidence[id].push(i);
            }
        }
        while let Some(&seed) = pending.first() {
            let mut group = BTreeSet::from([seed]);
            let mut local = BTreeSet::new();
            let mut queue = vec![seed];
            while let Some(id) = queue.pop() {
                crate::cancel::check();
                for &i in &incidence[id] {
                    if !local.insert(i) {
                        continue;
                    }
                    if !matches!(
                        constraints[i],
                        Constraint::Orientation(..) | Constraint::Ownership(..)
                    ) {
                        for id in constraints[i].ids() {
                            if movable.contains(&id) && group.insert(id) {
                                queue.push(id);
                            }
                        }
                    }
                }
            }
            for id in &group {
                pending.remove(id);
            }
            let mut ids = group.into_iter().collect::<Vec<_>>();
            ids.sort_by(|&a, &b| {
                let a = self.nodes[a].xy();
                let b = self.nodes[b].xy();
                a.0.cmp(&b.0).then(a.1.cmp(&b.1))
            });
            let local = local
                .into_iter()
                .map(|i| &constraints[i])
                .collect::<Vec<_>>();
            let mut assignments = 0;
            let mut solved = false;
            let nearest = ids
                .iter()
                .map(|&id| self.nodes[id].output())
                .collect::<Vec<_>>();
            for radius in ROUNDING_RADII {
                let candidates = candidates(radius);
                if search(
                    &ids,
                    &nearest,
                    &candidates,
                    &local,
                    &mut output,
                    &mut assignments,
                ) {
                    solved = true;
                    break;
                }
                if assignments >= ASSIGNMENT_BUDGET {
                    break;
                }
            }
            if !solved {
                return Err(precision_error(
                    "rounding constraints unsatisfied within 64 floating-point steps and 100000 assignments per neighborhood",
                ));
            }
        }
        if constraints.iter().any(|c| !c.satisfied(&output)) {
            return Err(precision_error("rounding changed the exact embedding"));
        }
        Ok(output)
    }
}
fn precision_error(reason: &str) -> Error {
    Error::Precision {
        operation: "polygon emission".into(),
        rule: None,
        context: String::new(),
        reason: reason.into(),
    }
}
fn ring_bounds(nodes: &[Exact], ring: &[usize]) -> Rect {
    LineString(ring.iter().map(|&i| nodes[i].output()).collect())
        .bounding_rect()
        .expect("ring bounds")
}
fn step(x: f64, n: i32) -> f64 {
    if n == 0 {
        return x;
    }
    if x == 0. {
        return f64::from_bits(n.unsigned_abs() as u64).copysign(n as f64);
    }
    let magnitude = x.to_bits() & 0x7fff_ffff_ffff_ffff;
    let delta = if x.is_sign_negative() {
        -(n as i64)
    } else {
        n as i64
    };
    if delta >= 0 {
        let bits = magnitude
            .saturating_add(delta as u64)
            .min(f64::INFINITY.to_bits());
        f64::from_bits(bits).copysign(x)
    } else if magnitude >= delta.unsigned_abs() {
        f64::from_bits(magnitude - delta.unsigned_abs()).copysign(x)
    } else {
        f64::from_bits(delta.unsigned_abs() - magnitude).copysign(-x)
    }
}
fn expanded_bounds(a: Coord, b: Coord) -> Rect {
    Rect::new(
        Coord {
            x: step(a.x.min(b.x), -MAX_ROUNDING_STEPS),
            y: step(a.y.min(b.y), -MAX_ROUNDING_STEPS),
        },
        Coord {
            x: step(a.x.max(b.x), MAX_ROUNDING_STEPS),
            y: step(a.y.max(b.y), MAX_ROUNDING_STEPS),
        },
    )
}
fn candidates(radius: i32) -> Vec<(i32, i32)> {
    let mut offsets = (-radius..=radius)
        .flat_map(|x| (-radius..=radius).map(move |y| (x, y)))
        .collect::<Vec<_>>();
    offsets.sort_by_key(|&(x, y)| (x.abs().max(y.abs()), x.abs() + y.abs(), x, y));
    offsets
}
/// Strict longitudinal order is a positive projection onto the exact source
/// direction. A bend at degree two may use the other coordinate when a dominant
/// coordinate lies between adjacent floats. Horizontal sources still require a
/// strictly ordered x coordinate, including the unrepresentable regression.
#[derive(Debug)]
struct Direction {
    a: Exact,
    b: Exact,
    bounds: [Interval; 2],
    exact: OnceLock<(Q, Q)>,
}
impl Direction {
    fn new(a: &Exact, b: &Exact) -> Self {
        let [ax, ay] = certified::coordinates(a);
        let [bx, by] = certified::coordinates(b);
        Self {
            a: a.clone(),
            b: b.clone(),
            bounds: [bx.sub(ax), by.sub(ay)],
            exact: OnceLock::new(),
        }
    }
    fn forward(&self, a: Coord, b: Coord) -> bool {
        let [ax, ay] = certified::coordinates(&Exact::Input(a));
        let [bx, by] = certified::coordinates(&Exact::Input(b));
        let dot = bx
            .sub(ax)
            .mul(self.bounds[0])
            .add(by.sub(ay).mul(self.bounds[1]));
        if let Some(sign) = dot.sign() {
            return sign.is_gt();
        }
        let (x, y) = self.exact.get_or_init(|| {
            let (ax, ay) = self.a.xy();
            let (bx, by) = self.b.xy();
            (bx - ax, by - ay)
        });
        let (ax, ay) = Exact::Input(a).xy();
        let (bx, by) = Exact::Input(b).xy();
        (bx - ax) * x + (by - ay) * y > Q::zero()
    }
}
fn turn_bounds(o: [Interval; 2], a: [Interval; 2], b: [Interval; 2]) -> Interval {
    a[0].sub(o[0])
        .mul(b[1].sub(o[1]))
        .sub(a[1].sub(o[1]).mul(b[0].sub(o[0])))
}
#[derive(Debug)]
enum Constraint {
    Distinct(usize, usize),
    Order(usize, usize, Box<Direction>),
    Disjoint([usize; 2], [usize; 2]),
    NonOverlap(usize, usize, usize),
    Incidence(usize, Vec<usize>),
    Orientation(Vec<usize>, bool),
    Ownership(Vec<[usize; 2]>, usize),
}
impl Constraint {
    fn guaranteed(&self, boxes: &[[Interval; 2]]) -> bool {
        match self {
            Self::Distinct(a, b) => {
                (0..2).any(|axis| boxes[*a][axis].sub(boxes[*b][axis]).sign().is_some())
            }
            Self::Order(a, b, d) => {
                boxes[*b][0]
                    .sub(boxes[*a][0])
                    .mul(d.bounds[0])
                    .add(boxes[*b][1].sub(boxes[*a][1]).mul(d.bounds[1]))
                    .0
                    > 0.
            }
            Self::NonOverlap(o, a, b) => {
                let (o, a, b) = (boxes[*o], boxes[*a], boxes[*b]);
                turn_bounds(o, a, b).sign().is_some()
                    || a[0]
                        .sub(o[0])
                        .mul(b[0].sub(o[0]))
                        .add(a[1].sub(o[1]).mul(b[1].sub(o[1])))
                        .1
                        < 0.
            }
            Self::Disjoint(a, b) => {
                let (a, b, c, d) = (boxes[a[0]], boxes[a[1]], boxes[b[0]], boxes[b[1]]);
                let separated = |o, x, y, z| {
                    let (s, t) = (turn_bounds(o, x, y), turn_bounds(o, x, z));
                    s.0 > 0. && t.0 > 0. || s.1 < 0. && t.1 < 0.
                };
                separated(a, b, c, d) || separated(c, d, a, b)
            }
            Self::Orientation(ring, positive) => {
                let origin = boxes[ring[0]];
                let area = ring.windows(2).fold(Interval(0., 0.), |sum, edge| {
                    sum.add(turn_bounds(origin, boxes[edge[0]], boxes[edge[1]]))
                });
                area.sign().is_some_and(|s| s.is_gt() == *positive)
            }
            Self::Ownership(edges, anchor) => {
                let p = boxes[*anchor];
                let above = |q: [Interval; 2]| {
                    if q[1].0 > p[1].1 {
                        Some(true)
                    } else if q[1].1 < p[1].0 {
                        Some(false)
                    } else {
                        None
                    }
                };
                let mut inside = false;
                for &[a, b] in edges {
                    let (a, b) = (boxes[a], boxes[b]);
                    let (Some(above_a), Some(above_b)) = (above(a), above(b)) else {
                        return false;
                    };
                    if above_a != above_b {
                        let Some(side) = turn_bounds(a, b, p).sign() else {
                            return false;
                        };
                        if side.is_gt() == above_b {
                            inside = !inside;
                        }
                    }
                }
                inside
            }
            _ => false,
        }
    }
    fn ids(&self) -> Vec<usize> {
        match self {
            Self::Distinct(a, b) | Self::Order(a, b, ..) => vec![*a, *b],
            Self::Disjoint(a, b) => vec![a[0], a[1], b[0], b[1]],
            Self::NonOverlap(o, a, b) => vec![*o, *a, *b],
            Self::Incidence(o, rays) => std::iter::once(*o).chain(rays.iter().copied()).collect(),
            Self::Orientation(r, _) => r.clone(),
            Self::Ownership(edges, anchor) => std::iter::once(*anchor)
                .chain(edges.iter().flatten().copied())
                .collect(),
        }
    }
    fn satisfied(&self, coords: &[Coord]) -> bool {
        let p = |i| Exact::Input(coords[i]);
        match self {
            Self::Distinct(a, b) => coords[*a] != coords[*b],
            Self::Order(a, b, direction) => direction.forward(coords[*a], coords[*b]),
            Self::Disjoint(a, b) => geo::line_intersection::line_intersection(
                Line::new(coords[a[0]], coords[a[1]]),
                Line::new(coords[b[0]], coords[b[1]]),
            )
            .is_none(),
            Self::Incidence(o, rays) => {
                let mut actual = rays.clone();
                actual.sort_by(|&a, &b| super::arrangement::angle(&p(a), &p(b), &p(*o)));
                let start = actual
                    .iter()
                    .position(|id| *id == rays[0])
                    .expect("same incidences");
                actual.rotate_left(start);
                actual == *rays
            }
            Self::NonOverlap(o, a, b) => {
                !orient(&p(*o), &p(*a), &p(*b)).is_zero()
                    || !on(&p(*o), &p(*a), &p(*b)) && !on(&p(*o), &p(*b), &p(*a))
            }
            Self::Ownership(edges, anchor) => {
                let point = coords[*anchor];
                let mut inside = false;
                for &[a, b] in edges {
                    let (a, b) = (coords[a], coords[b]);
                    let side = geo::kernels::RobustKernel::orient2d(a, b, point);
                    if side == geo::kernels::Orientation::Collinear
                        && point.x >= a.x.min(b.x)
                        && point.x <= a.x.max(b.x)
                        && point.y >= a.y.min(b.y)
                        && point.y <= a.y.max(b.y)
                    {
                        return false;
                    }
                    if (a.y > point.y) != (b.y > point.y)
                        && (side == geo::kernels::Orientation::CounterClockwise) == (b.y > a.y)
                    {
                        inside = !inside;
                    }
                }
                inside
            }
            Self::Orientation(r, positive) => {
                let nodes = r.iter().map(|&id| p(id)).collect::<Vec<_>>();
                let ids = (0..r.len()).collect::<Vec<_>>();
                let sign = area_sign(&nodes, &ids);
                !sign.is_eq() && sign.is_gt() == *positive
            }
        }
    }
}
fn search(
    ids: &[usize],
    nearest: &[Coord],
    candidates: &[(i32, i32)],
    constraints: &[&Constraint],
    coords: &mut [Coord],
    assignments: &mut usize,
) -> bool {
    let previous = ids
        .iter()
        .map(|&id| (id, coords[id]))
        .collect::<HashMap<_, _>>();
    let variables = ids.iter().copied().collect::<BTreeSet<_>>();
    let mut domains = Vec::new();
    for (&id, &nearest) in ids.iter().zip(nearest) {
        let unary = constraints
            .iter()
            .copied()
            .filter(|c| {
                c.ids()
                    .into_iter()
                    .filter(|i| variables.contains(i))
                    .all(|i| i == id)
            })
            .collect::<Vec<_>>();
        let mut domain = Vec::new();
        for &(x, y) in candidates {
            crate::cancel::check();
            if *assignments >= ASSIGNMENT_BUDGET {
                for (&id, &p) in &previous {
                    coords[id] = p;
                }
                return false;
            }
            *assignments += 1;
            let p = Coord {
                x: step(nearest.x, x),
                y: step(nearest.y, y),
            };
            if !p.x.is_finite() || !p.y.is_finite() {
                continue;
            }
            coords[id] = p;
            if unary.iter().all(|c| c.satisfied(coords)) {
                domain.push(p);
            }
        }
        coords[id] = previous[&id];
        if domain.is_empty() {
            for (&id, &p) in &previous {
                coords[id] = p;
            }
            return false;
        }
        domains.push((id, domain));
    }
    // Arc consistency removes unsupported candidates before committing any
    // backtracking assignment. Coalesce all binary constraints for the same
    // pair so support satisfies them together. The optional pruning work is
    // bounded; reaching its cap leaves the remaining domains conservative.
    let initial_positions = domains
        .iter()
        .enumerate()
        .map(|(i, (id, _))| (*id, i))
        .collect::<HashMap<_, _>>();
    let mut pairs = std::collections::BTreeMap::<(usize, usize), Vec<&Constraint>>::new();
    let mut weights = vec![0; domains.len()];
    for &c in constraints {
        let mut vars = c
            .ids()
            .iter()
            .filter_map(|id| initial_positions.get(id))
            .copied()
            .collect::<Vec<_>>();
        vars.sort_unstable();
        vars.dedup();
        for &v in &vars {
            weights[v] += 1;
        }
        if vars.len() == 2 {
            pairs.entry((vars[0], vars[1])).or_default().push(c);
        }
    }
    let mut work = 0;
    'prune: loop {
        let mut changed = false;
        for (&(a, b), checks) in &pairs {
            for (a, b) in [(a, b), (b, a)] {
                let (aid, bid) = (domains[a].0, domains[b].0);
                let mut retained = Vec::new();
                for &p in &domains[a].1 {
                    crate::cancel::check();
                    coords[aid] = p;
                    let mut supported = false;
                    for &q in &domains[b].1 {
                        crate::cancel::check();
                        work += 1;
                        if work > ASSIGNMENT_BUDGET {
                            for (&id, &p) in &previous {
                                coords[id] = p;
                            }
                            break 'prune;
                        }
                        coords[bid] = q;
                        if checks.iter().all(|c| c.satisfied(coords)) {
                            supported = true;
                            break;
                        }
                    }
                    if supported {
                        retained.push(p);
                    }
                }
                coords[aid] = previous[&aid];
                coords[bid] = previous[&bid];
                if retained.is_empty() {
                    for (&id, &p) in &previous {
                        coords[id] = p;
                    }
                    return false;
                }
                changed |= retained.len() != domains[a].1.len();
                domains[a].1 = retained;
            }
        }
        if !changed {
            break;
        }
    }
    // Smallest domains first, then the most constrained variable. Exact
    // geometric order is the stable final tie break.
    domains.sort_by_key(|(id, domain)| {
        (
            domain.len(),
            std::cmp::Reverse(weights[initial_positions[id]]),
            ids.iter().position(|i| i == id),
        )
    });
    let positions = domains
        .iter()
        .enumerate()
        .map(|(i, (id, _))| (*id, i))
        .collect::<HashMap<_, _>>();
    let mut ready = vec![Vec::new(); ids.len()];
    let mut incidence = vec![Vec::new(); ids.len()];
    for &constraint in constraints {
        crate::cancel::check();
        let mut vars = constraint
            .ids()
            .iter()
            .filter_map(|id| positions.get(id))
            .copied()
            .collect::<Vec<_>>();
        vars.sort_unstable();
        vars.dedup();
        if let Some(&last) = vars.last() {
            ready[last].push(constraint);
        }
        for &id in &vars {
            incidence[id].push((constraint, vars.clone()));
        }
    }
    let mut choices = vec![0; ids.len()];
    let mut depth = 0;
    while *assignments < ASSIGNMENT_BUDGET {
        crate::cancel::check();
        if choices[depth] == domains[depth].1.len() {
            choices[depth] = 0;
            coords[domains[depth].0] = previous[&domains[depth].0];
            if depth == 0 {
                break;
            }
            depth -= 1;
            continue;
        }
        *assignments += 1;
        coords[domains[depth].0] = domains[depth].1[choices[depth]];
        choices[depth] += 1;
        if !ready[depth].iter().all(|c| c.satisfied(coords)) {
            continue;
        }
        // Forward checking: every constraint with one remaining variable must
        // retain support in that variable's pruned domain before descending.
        let mut supported = true;
        for target in depth + 1..domains.len() {
            let checks = incidence[target]
                .iter()
                .filter(|(_, vars)| vars.iter().all(|&v| v <= depth || v == target))
                .map(|(c, _)| *c)
                .collect::<Vec<_>>();
            if checks.is_empty() {
                continue;
            }
            let id = domains[target].0;
            let old = coords[id];
            let mut found = false;
            for &p in &domains[target].1 {
                crate::cancel::check();
                if *assignments >= ASSIGNMENT_BUDGET {
                    break;
                }
                coords[id] = p;
                if checks.iter().all(|c| c.satisfied(coords)) {
                    found = true;
                    break;
                }
            }
            coords[id] = old;
            if !found {
                supported = false;
                break;
            }
        }
        if supported {
            depth += 1;
            if depth == domains.len() {
                return true;
            }
        }
    }
    for (&id, &p) in &previous {
        coords[id] = p;
    }
    false
}

#[derive(Debug, Default)]
pub(crate) struct Geometry(pub(super) Vec<Component>);
/// Emit an already validated original operand after certified filtering.
/// No rounded overlay coordinates participate in the containment decision.
pub(crate) fn original(polygon: &Polygon, tolerance: f64) -> MultiPolygon {
    let mut nodes = Vec::new();
    let mut rings = Vec::new();
    for ring in std::iter::once(polygon.exterior()).chain(polygon.interiors()) {
        let start = nodes.len();
        nodes.extend(ring.0.iter().copied().map(Exact::Input));
        rings.push((start..nodes.len()).collect::<Vec<_>>());
    }
    if !substantial(
        &nodes,
        &rings.iter().map(Vec::as_slice).collect::<Vec<_>>(),
        tolerance,
    ) {
        return MultiPolygon(vec![]);
    }
    let mut polygon = polygon.clone();
    use geo::Winding;
    polygon.exterior_mut(|r| r.make_ccw_winding());
    polygon.interiors_mut(|rings| {
        for r in rings {
            r.make_cw_winding();
        }
    });
    let mut polygons = vec![polygon];
    canonicalize(&mut polygons);
    MultiPolygon(polygons)
}
impl Geometry {
    pub fn emit(&self, tolerance: f64) -> Result<MultiPolygon, Error> {
        let mut polygons = Vec::new();
        for component in &self.0 {
            let selected = component
                .polygons
                .iter()
                .filter(|p| {
                    substantial(
                        &component.nodes,
                        &std::iter::once(p.shell.as_slice())
                            .chain(p.holes.iter().map(Vec::as_slice))
                            .collect::<Vec<_>>(),
                        tolerance,
                    )
                })
                .collect::<Vec<_>>();
            if selected.is_empty() {
                continue;
            }
            let coords = component.selected_coordinates(
                &selected
                    .iter()
                    .map(|p| Selection::polygon(p))
                    .collect::<Vec<_>>(),
            )?;
            let ring = |r: &[usize]| LineString(r.iter().map(|&i| coords[i]).collect());
            polygons.extend(
                selected.into_iter().map(|p| {
                    Polygon::new(ring(&p.shell), p.holes.iter().map(|r| ring(r)).collect())
                }),
            );
        }
        validate_output(&polygons)?;
        canonicalize(&mut polygons);
        Ok(MultiPolygon(polygons))
    }
    pub fn gaps(&self, tolerance: f64) -> Result<Vec<(bool, LineString)>, Error> {
        let mut result = Vec::new();
        for c in &self.0 {
            let selected = c
                .polygons
                .iter()
                .map(|p| {
                    p.holes
                        .iter()
                        .map(|h| substantial(&c.nodes, &[h], tolerance))
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>();
            let selections = c
                .polygons
                .iter()
                .zip(&selected)
                .map(|(p, keep)| Selection {
                    shell: &p.shell,
                    holes: p
                        .holes
                        .iter()
                        .zip(keep)
                        .filter_map(|(h, &keep)| keep.then_some(h.as_slice()))
                        .collect(),
                })
                .collect::<Vec<_>>();
            let coords = c.selected_coordinates(&selections)?;
            let ring = |r: &[usize]| LineString(r.iter().map(|&i| coords[i]).collect());
            let polygons = c
                .polygons
                .iter()
                .zip(&selected)
                .map(|(p, keep)| {
                    Polygon::new(
                        ring(&p.shell),
                        p.holes
                            .iter()
                            .zip(keep)
                            .filter(|(_, keep)| **keep)
                            .map(|(r, _)| ring(r))
                            .collect(),
                    )
                })
                .collect::<Vec<_>>();
            validate_output(&polygons)?;
            for (p, selected) in c.polygons.iter().zip(selected) {
                result.push((false, ring(&p.shell)));
                for (hole, keep) in p.holes.iter().zip(selected) {
                    if keep {
                        result.push((true, ring(hole)));
                    }
                }
            }
        }
        for (_, r) in &mut result {
            canonical_ring(r);
        }
        result.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| compare_ring(&a.1, &b.1)));
        Ok(result)
    }
}
fn validate_output(polygons: &[Polygon]) -> Result<(), Error> {
    if polygons
        .iter()
        .any(|p| super::validity::polygon(p).is_some())
    {
        return Err(precision_error(
            "emitted polygon has invalid ring ownership or contacts",
        ));
    }
    let tree = index::build(
        polygons
            .iter()
            .enumerate()
            .filter_map(|(i, p)| p.bounding_rect().map(|r| (i, r))),
    );
    let locations = polygons
        .iter()
        .map(super::location::Location::new)
        .collect::<Vec<_>>();
    if index::pairs(&tree, &tree, 0., true)
        .any(|(a, b)| super::validity::multipart(&locations[a], &locations[b]))
    {
        return Err(precision_error("emitted polygon parts overlap"));
    }
    Ok(())
}
fn compare_ring(a: &LineString, b: &LineString) -> std::cmp::Ordering {
    a.0.iter()
        .zip(&b.0)
        .map(|(a, b)| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)))
        .find(|o| !o.is_eq())
        .unwrap_or_else(|| a.0.len().cmp(&b.0.len()))
}
fn canonical_ring(r: &mut LineString) {
    r.0.pop();
    let start = (0..r.0.len())
        .min_by(|&a, &b| {
            r.0[a]
                .x
                .total_cmp(&r.0[b].x)
                .then(r.0[a].y.total_cmp(&r.0[b].y))
        })
        .expect("nonempty ring");
    r.0.rotate_left(start);
    r.0.push(r.0[0]);
}
fn canonicalize(polygons: &mut [Polygon]) {
    for p in polygons.iter_mut() {
        p.exterior_mut(canonical_ring);
        p.interiors_mut(|rings| {
            for r in rings.iter_mut() {
                canonical_ring(r);
            }
            rings.sort_by(compare_ring);
        });
    }
    polygons.sort_by(|a, b| compare_ring(a.exterior(), b.exterior()));
}

fn squared_distance(p: &Exact, a: &Exact, b: &Exact) -> Q {
    let (px, py) = p.xy();
    let (ax, ay) = a.xy();
    let (bx, by) = b.xy();
    let (x, y) = (&bx - &ax, &by - &ay);
    let length = &x * &x + &y * &y;
    let dot = (&px - &ax) * &x + (&py - &ay) * &y;
    let (dx, dy) = if dot <= Q::zero() || length.is_zero() {
        (px - ax, py - ay)
    } else if dot >= length {
        (px - bx, py - by)
    } else {
        let cross = (&px - &ax) * y - (&py - &ay) * x;
        return &cross * &cross / length;
    };
    &dx * &dx + &dy * &dy
}
impl Geometry {
    pub fn position(&self, point: Coord) -> i8 {
        let p = Exact::Input(point);
        self.0
            .iter()
            .flat_map(|c| c.locations().iter().map(|l| l.position(&c.nodes, &p)))
            .max()
            .unwrap_or(-1)
    }
    pub fn near(&self, point: Coord, tolerance: f64) -> bool {
        let p = Exact::Input(point);
        let tolerance = Q::from_float(tolerance).expect("finite tolerance");
        let squared = &tolerance * &tolerance;
        self.0.iter().any(|c| {
            c.locations().iter().any(|l| {
                l.tree
                    .locate_in_envelope_intersecting(&index::at(
                        point,
                        tolerance.to_f64().expect("finite tolerance"),
                    ))
                    .any(|item| {
                        let [a, b] = l.edges[item.data];
                        squared_distance(&p, &c.nodes[a], &c.nodes[b]) <= squared
                    })
            })
        })
    }
    pub fn properly_contains(&self, line: &LineString, tolerance: f64) -> bool {
        if self.position(line.0[0]) != 1 {
            return false;
        }
        let tol = Q::from_float(tolerance).expect("finite tolerance");
        let squared = &tol * &tol;
        for c in &self.0 {
            for location in c.locations() {
                for segment in line.lines() {
                    crate::cancel::check();
                    let (x, y) = (Exact::Input(segment.start), Exact::Input(segment.end));
                    for item in location
                        .tree
                        .locate_in_envelope_intersecting(&index::expanded(
                            segment.bounding_rect(),
                            tolerance,
                        ))
                    {
                        let [a, b] = location.edges[item.data];
                        let (a, b) = (&c.nodes[a], &c.nodes[b]);
                        if intersects(a, b, &x, &y)
                            || squared_distance(&x, a, b) <= squared
                            || squared_distance(&y, a, b) <= squared
                            || squared_distance(a, &x, &y) <= squared
                            || squared_distance(b, &x, &y) <= squared
                        {
                            return false;
                        }
                    }
                }
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_float_steps_match_successive_adjacent_values() {
        for x in [
            -f64::MAX,
            -1.,
            -f64::MIN_POSITIVE,
            -f64::from_bits(1),
            -0.,
            0.,
            f64::from_bits(1),
            f64::MIN_POSITIVE,
            1.,
            f64::MAX,
        ] {
            for n in -MAX_ROUNDING_STEPS..=MAX_ROUNDING_STEPS {
                let mut expected = x;
                for _ in 0..n.unsigned_abs() {
                    expected = if n < 0 {
                        expected.next_down()
                    } else {
                        expected.next_up()
                    };
                }
                assert_eq!(step(x, n).to_bits(), expected.to_bits(), "{x} {n}");
            }
        }
    }
    fn input(x: f64, y: f64) -> Exact {
        Exact::Input(Coord { x, y })
    }
    fn rational(x: Q, y: Q) -> Exact {
        Exact::rational(x, y)
    }
    fn q(x: f64) -> Q {
        Q::from_float(x).unwrap()
    }
    #[test]
    fn certified_area_and_perimeter_avoid_exact_fallback_for_large_rings() {
        let n = 4096;
        let nodes = (0..n)
            .map(|i| {
                let t = i as f64 * std::f64::consts::TAU / n as f64;
                input(1000. * t.cos(), 1000. * t.sin())
            })
            .collect::<Vec<_>>();
        let mut ring = (0..n).collect::<Vec<_>>();
        ring.push(0);
        EXACT_AREAS.with(|n| n.set(0));
        for tolerance in [0., 0.001, 1., 1000.] {
            assert_eq!(substantial(&nodes, &[&ring], tolerance), tolerance < 1000.);
        }
        assert_eq!(EXACT_AREAS.with(|n| n.get()), 0);
    }
    #[test]
    fn projected_source_order_uses_both_coordinates_at_adjacent_floats() {
        let x = 1f64.next_up();
        let middle = (q(1.) + q(x)) / Q::from_integer(2.into());
        let nodes = vec![
            input(0., 0.),
            input(1., 1.),
            rational(middle, q(1.) - q(2f64.powi(-55))),
            input(x, 1.),
            input(0., 2.),
        ];
        let geometry = Geometry(vec![Component::new(
            nodes,
            vec![vec![0, 1, 2, 3, 4, 0]],
            vec![],
            vec![(1, 2, 0), (2, 3, 0)],
        )]);
        let emitted = geometry.emit(0.).unwrap();
        assert!(super::super::validity::polygon(&emitted.0[0]).is_none());
        assert_eq!(emitted.0[0].exterior().0.len(), 6);
    }
    #[test]
    fn collapsed_intersections_retain_distinct_identities() {
        let d = q(2f64.powi(-56));
        let nodes = vec![
            input(0., 0.),
            rational(q(1.) + &d, q(0.)),
            rational(q(1.) + &d * Q::from_integer(2.into()), d),
            input(0., 1.),
        ];
        let c = Component::new(nodes, vec![vec![0, 1, 2, 3, 0]], vec![], vec![(1, 2, 0)]);
        let coordinates = c.coordinates().unwrap();
        assert_ne!(coordinates[1], coordinates[2]);
        assert_eq!(coordinates[0], Coord { x: 0., y: 0. });
        assert_eq!(coordinates[3], Coord { x: 0., y: 1. });
        Geometry(vec![c]).emit(0.).unwrap();
    }
    #[test]
    fn nearest_rounding_reverses_orientation_and_local_search_preserves_it() {
        let unit = q(2f64.powi(-52)) / Q::from_integer(100.into());
        let nodes = vec![
            input(0., 0.),
            rational(q(1.) + &unit * Q::from_integer(49.into()), q(1.) + &unit),
            rational(
                q(1.) + &unit * Q::from_integer(51.into()),
                q(1.) + &unit * Q::from_integer(49.into()),
            ),
        ];
        let ring = vec![0, 1, 2, 0];
        assert!(area(&nodes, &ring).is_positive());
        let nearest = nodes
            .iter()
            .map(|p| Exact::Input(p.output()))
            .collect::<Vec<_>>();
        assert!(area(&nearest, &ring).is_negative());
        let output = Geometry(vec![Component::new(nodes, vec![ring], vec![], vec![])])
            .emit(0.)
            .unwrap();
        assert!(super::super::validity::polygon(&output.0[0]).is_none());
    }
    #[test]
    fn hole_ownership_is_fixed_before_rounding_near_a_shell() {
        let x = q(2.) - q(2f64.powi(-55));
        let nodes = vec![
            input(0., 0.),
            input(2., 0.),
            input(2., 2.),
            input(0., 2.),
            input(1., 1.),
            rational(x.clone(), q(1.5)),
            rational(x, q(0.5)),
        ];
        let geometry = Geometry(vec![Component::new(
            nodes,
            vec![vec![0, 1, 2, 3, 0]],
            vec![vec![4, 5, 6, 4]],
            vec![],
        )]);
        let output = geometry.emit(0.).unwrap();
        assert_eq!(output.0[0].interiors().len(), 1);
        assert!(super::super::validity::polygon(&output.0[0]).is_none());
    }
    #[test]
    fn inclusive_area_threshold_and_exact_dyadic_cancellation() {
        let n = Q::from_integer(1434079929761945i64.into());
        let nodes = vec![
            input(-1782079.763005541, -2764781.752256425),
            input(-2115977.5161031107, 2038587.3908392591),
            input(-2115977.5161031107, 2038587.390839259),
        ];
        assert_eq!(
            area(&nodes, &[0, 1, 2, 0]),
            n / Q::from_integer((Q::from_integer(2.into()).numer() << 64usize).clone())
        );
        for ring in [vec![0, 1, 2, 0], vec![1, 2, 0, 1], vec![2, 1, 0, 2]] {
            assert!(substantial(&nodes, &[&ring], 0.));
        }
        let square = vec![input(0., 0.), input(1., 0.), input(1., 1.), input(0., 1.)];
        assert!(substantial(&square, &[&[0, 1, 2, 3, 0]], 0.5));
        assert!(!substantial(&square, &[&[0, 1, 2, 3, 0]], 0.5f64.next_up()));
    }
    #[test]
    fn representable_intersection_can_move_to_preserve_neighboring_source_order() {
        let x = 1f64.next_up();
        let middle = (q(1.) + q(x)) / Q::from_integer(2.into());
        let nodes = vec![
            input(0., 0.),
            rational(q(1.), q(0.)),
            rational(middle, q(0.)),
            input(x, 0.),
            input(0., 1.),
        ];
        let c = Component::new(
            nodes,
            vec![vec![0, 1, 2, 3, 4, 0]],
            vec![],
            vec![(0, 1, 0), (1, 2, 0), (2, 3, 0)],
        );
        let coordinates = c.coordinates().unwrap();
        assert!(coordinates[1].x < coordinates[2].x && coordinates[2].x < x);
        assert_eq!(coordinates[0], Coord { x: 0., y: 0. });
        assert_eq!(coordinates[3], Coord { x, y: 0. });
        Geometry(vec![c]).emit(0.).unwrap();
    }
    #[test]
    fn rounding_search_budget_is_bounded_and_restores_coordinates() {
        let original = Coord { x: 1., y: 1. };
        let mut coords = vec![original, original];
        let constraint = Constraint::Distinct(0, 1);
        let mut assignments = 0;
        assert!(!search(
            &[0],
            &[original],
            &vec![(0, 0); 100_001],
            &[&constraint],
            &mut coords,
            &mut assignments
        ));
        assert_eq!(assignments, 100_000);
        assert_eq!(coords, vec![original, original]);
    }
    #[test]
    fn rounding_search_observes_cancellation_before_assignments() {
        let token = crate::CancellationToken::default();
        let original = Coord { x: 1., y: 1. };
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            crate::cancel::scope(Some(token.clone()), || {
                token.cancel();
                search(
                    &[0],
                    &[original],
                    &[(0, 0)],
                    &[&Constraint::Distinct(0, 1)],
                    &mut [original, original],
                    &mut 0,
                )
            })
        }));
        assert!(result.unwrap_err().is::<crate::cancel::Cancelled>());
    }
    #[test]
    fn unsatisfied_source_order_is_a_precision_error() {
        let x = 1f64.next_up();
        let middle = (q(1.) + q(x)) / Q::from_integer(2.into());
        let nodes = vec![
            input(1., 0.),
            rational(middle, q(0.)),
            input(x, 0.),
            input(1., 1.),
        ];
        let c = Component::new(
            nodes,
            vec![vec![0, 1, 2, 3, 0]],
            vec![],
            vec![(0, 1, 0), (1, 2, 0)],
        );
        assert!(matches!(c.coordinates(), Err(Error::Precision { .. })));
    }
}
