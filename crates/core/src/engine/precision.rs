//! Exact overlay boundaries and constrained conversion to binary output coordinates.
use super::{
    arrangement::{Exact, intersects, on, orient},
    index,
};
use crate::Error;
use geo::{BoundingRect, Coord, Kernel, Line, LineString, MultiPolygon, Polygon, Rect};
use num_rational::BigRational as Q;
use num_traits::{Signed, ToPrimitive, Zero};
use std::{
    collections::{BTreeSet, HashMap},
    sync::OnceLock,
};

pub(super) fn area(nodes: &[Exact], ring: &[usize]) -> Q {
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
// Outward-rounded interval arithmetic certifies the common case. Exact
// rational arithmetic handles cancellation and sub-ULP intersections.
#[derive(Clone, Copy)]
struct Interval(f64, f64);
impl Interval {
    fn add(self, b: Self) -> Self {
        Self((self.0 + b.0).next_down(), (self.1 + b.1).next_up())
    }
    fn sub(self, b: Self) -> Self {
        Self((self.0 - b.1).next_down(), (self.1 - b.0).next_up())
    }
    fn mul(self, b: Self) -> Self {
        let v = [self.0 * b.0, self.0 * b.1, self.1 * b.0, self.1 * b.1];
        if v.iter().any(|x| !x.is_finite()) {
            return Self(f64::NEG_INFINITY, f64::INFINITY);
        }
        Self(
            v.iter().copied().fold(f64::INFINITY, f64::min).next_down(),
            v.iter()
                .copied()
                .fold(f64::NEG_INFINITY, f64::max)
                .next_up(),
        )
    }
}
pub(super) fn area_sign(nodes: &[Exact], ring: &[usize]) -> std::cmp::Ordering {
    fn intervals(p: &Exact) -> (Interval, Interval) {
        let q = p.output();
        if matches!(p, Exact::Input(_)) {
            (Interval(q.x, q.x), Interval(q.y, q.y))
        } else {
            (
                Interval(q.x.next_down(), q.x.next_up()),
                Interval(q.y.next_down(), q.y.next_up()),
            )
        }
    }
    let (ox, oy) = intervals(&nodes[ring[0]]);
    let mut sum = Interval(0., 0.);
    for pair in ring.windows(2) {
        crate::cancel::check();
        let (ax, ay) = intervals(&nodes[pair[0]]);
        let (bx, by) = intervals(&nodes[pair[1]]);
        sum = sum.add(ax.sub(ox).mul(by.sub(oy)).sub(ay.sub(oy).mul(bx.sub(ox))));
    }
    if sum.0 > 0. {
        std::cmp::Ordering::Greater
    } else if sum.1 < 0. {
        std::cmp::Ordering::Less
    } else {
        area(nodes, ring).cmp(&Q::zero())
    }
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
        let (_, py) = p.xy();
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
            let (_, ay) = a.xy();
            let (_, by) = b.xy();
            if (ay > py) != (by > py) && (orient(a, b, p) > Q::zero()) == (by > ay) {
                inside = !inside;
            }
        }
        if inside { 1 } else { -1 }
    }
}
// Rational bounds for each square root. Refinement certifies the inclusive
// threshold, including equality when all edge lengths are rational.
fn substantial(nodes: &[Exact], rings: &[&[usize]], tolerance: f64) -> bool {
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
#[derive(Debug)]
pub(super) struct Component {
    nodes: Vec<Exact>,
    polygons: Vec<ExactPolygon>,
    source_order: Vec<(usize, usize, usize)>,
    locations: Vec<ExactLocation>,
}
impl Component {
    pub fn new(
        nodes: Vec<Exact>,
        shells: Vec<Vec<usize>>,
        holes: Vec<Vec<usize>>,
        source_order: Vec<(usize, usize, usize)>,
    ) -> Self {
        let shell_locations = shells
            .iter()
            .map(|r| ExactLocation::new(&nodes, [r]))
            .collect::<Vec<_>>();
        let shell_areas = shells
            .iter()
            .map(|_| OnceLock::<Q>::new())
            .collect::<Vec<_>>();
        let tree = index::build(
            shells
                .iter()
                .enumerate()
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
        let locations = polygons
            .iter()
            .map(|p| ExactLocation::new(&nodes, std::iter::once(&p.shell).chain(&p.holes)))
            .collect();
        Self {
            nodes,
            polygons,
            source_order,
            locations,
        }
    }
    fn rings(&self) -> Vec<&[usize]> {
        self.polygons
            .iter()
            .flat_map(|p| {
                std::iter::once(p.shell.as_slice()).chain(p.holes.iter().map(Vec::as_slice))
            })
            .collect()
    }
    fn coordinates(&self) -> Result<Vec<Coord>, Error> {
        let mut output = self.nodes.iter().map(Exact::output).collect::<Vec<_>>();
        let rings = self.rings();
        let active = rings
            .iter()
            .flat_map(|r| r.iter().copied())
            .collect::<BTreeSet<_>>();
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
                    constraints.push(Constraint::Angle(
                        o,
                        x,
                        y,
                        orient(&self.nodes[o], &self.nodes[x], &self.nodes[y]).cmp(&Q::zero()),
                    ));
                } else {
                    constraints.push(Constraint::Disjoint(*a, b));
                }
            }
            constraints.push(Constraint::Distinct(a[0], a[1]));
        }
        for &(a, b, axis) in &self.source_order {
            if active.contains(&a) && active.contains(&b) {
                let (ax, ay) = self.nodes[a].xy();
                let (bx, by) = self.nodes[b].xy();
                let order = if axis == 0 { ax.cmp(&bx) } else { ay.cmp(&by) };
                constraints.push(Constraint::Order(a, b, axis, order));
            }
        }
        for ring in &rings {
            constraints.push(Constraint::Orientation(
                ring.to_vec(),
                area_sign(&self.nodes, ring).is_gt(),
            ));
        }
        for polygon in &self.polygons {
            let shell = ExactLocation::new(&self.nodes, [&polygon.shell]);
            for hole in &polygon.holes {
                let anchor = *hole
                    .iter()
                    .find(|&&id| shell.position(&self.nodes, &self.nodes[id]) == 1)
                    .expect("exact interior hole anchor");
                let p = output[anchor];
                let query = rstar::AABB::from_corners(
                    [step(p.x, -16), step(p.y, -16)],
                    [f64::MAX, step(p.y, 16)],
                );
                let ray_edges = shell
                    .tree
                    .locate_in_envelope_intersecting(&query)
                    .map(|item| shell.edges[item.data])
                    .collect::<Vec<_>>();
                constraints.push(Constraint::Ownership(ray_edges, anchor));
            }
        }
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
            for radius in [1, 2, 4, 8, 16] {
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
                if assignments >= 100_000 {
                    break;
                }
            }
            if !solved {
                return Err(precision_error(
                    "rounding constraints unsatisfied within 16 floating-point steps and 100000 assignments per neighborhood",
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
fn step(mut x: f64, n: i32) -> f64 {
    for _ in 0..n.unsigned_abs() {
        x = if n < 0 { x.next_down() } else { x.next_up() };
    }
    x
}
fn expanded_bounds(a: Coord, b: Coord) -> Rect {
    Rect::new(
        Coord {
            x: step(a.x.min(b.x), -16),
            y: step(a.y.min(b.y), -16),
        },
        Coord {
            x: step(a.x.max(b.x), 16),
            y: step(a.y.max(b.y), 16),
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
#[derive(Debug)]
enum Constraint {
    Distinct(usize, usize),
    Order(usize, usize, usize, std::cmp::Ordering),
    Disjoint([usize; 2], [usize; 2]),
    Angle(usize, usize, usize, std::cmp::Ordering),
    Orientation(Vec<usize>, bool),
    Ownership(Vec<[usize; 2]>, usize),
}
impl Constraint {
    fn ids(&self) -> Vec<usize> {
        match self {
            Self::Distinct(a, b) | Self::Order(a, b, ..) => vec![*a, *b],
            Self::Disjoint(a, b) => vec![a[0], a[1], b[0], b[1]],
            Self::Angle(o, a, b, _) => vec![*o, *a, *b],
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
            Self::Order(a, b, axis, order) => {
                let a = coords[*a];
                let b = coords[*b];
                (if *axis == 0 {
                    a.x.total_cmp(&b.x)
                } else {
                    a.y.total_cmp(&b.y)
                }) == *order
            }
            Self::Disjoint(a, b) => geo::line_intersection::line_intersection(
                Line::new(coords[a[0]], coords[a[1]]),
                Line::new(coords[b[0]], coords[b[1]]),
            )
            .is_none(),
            Self::Angle(o, a, b, order) => {
                let actual = orient(&p(*o), &p(*a), &p(*b)).cmp(&Q::zero());
                if order.is_eq() {
                    // Opposite collinear edges may bend, but cannot overlap.
                    actual != std::cmp::Ordering::Equal
                        || !on(&p(*o), &p(*a), &p(*b)) && !on(&p(*o), &p(*b), &p(*a))
                } else {
                    actual == *order
                }
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
    // Iterative backtracking keeps stack and candidate storage bounded even for
    // a crowded neighborhood. Constraints run as soon as their last variable
    // is assigned, so unrelated ring vertices are never copied per candidate.
    let positions = ids
        .iter()
        .enumerate()
        .map(|(i, &id)| (id, i))
        .collect::<HashMap<_, _>>();
    let mut ready = vec![Vec::new(); ids.len()];
    for &constraint in constraints {
        crate::cancel::check();
        let last = constraint
            .ids()
            .iter()
            .filter_map(|id| positions.get(id))
            .copied()
            .max()
            .expect("local constraint incidence");
        ready[last].push(constraint);
    }
    let previous = ids.iter().map(|&id| coords[id]).collect::<Vec<_>>();
    let mut choices = vec![0; ids.len()];
    let mut depth = 0;
    while *assignments < 100_000 {
        crate::cancel::check();
        if choices[depth] == candidates.len() {
            choices[depth] = 0;
            coords[ids[depth]] = previous[depth];
            if depth == 0 {
                break;
            }
            depth -= 1;
            continue;
        }
        let (x, y) = candidates[choices[depth]];
        choices[depth] += 1;
        *assignments += 1;
        let candidate = Coord {
            x: step(nearest[depth].x, x),
            y: step(nearest[depth].y, y),
        };
        if !candidate.x.is_finite() || !candidate.y.is_finite() {
            continue;
        }
        coords[ids[depth]] = candidate;
        if ready[depth].iter().all(|c| c.satisfied(coords)) {
            depth += 1;
            if depth == ids.len() {
                return true;
            }
        }
    }
    for (&id, &value) in ids.iter().zip(&previous) {
        coords[id] = value;
    }
    false
}
#[derive(Debug, Default)]
pub(crate) struct Geometry(pub(super) Vec<Component>);
impl Geometry {
    pub fn has_area(&self) -> bool {
        self.0.iter().any(|c| {
            c.polygons.iter().any(|p| {
                substantial(
                    &c.nodes,
                    &std::iter::once(p.shell.as_slice())
                        .chain(p.holes.iter().map(Vec::as_slice))
                        .collect::<Vec<_>>(),
                    0.,
                )
            })
        })
    }
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
            let coords = component.coordinates()?;
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
            let coords = c.coordinates()?;
            let ring = |r: &[usize]| LineString(r.iter().map(|&i| coords[i]).collect());
            let polygons = c
                .polygons
                .iter()
                .map(|p| Polygon::new(ring(&p.shell), p.holes.iter().map(|r| ring(r)).collect()))
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
    if index::pairs(&tree, &tree, 0., true).any(|(a, b)| {
        super::validity::multipart(
            &super::location::Location::new(&polygons[a]),
            &super::location::Location::new(&polygons[b]),
        )
    }) {
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
            .flat_map(|c| c.locations.iter().map(|l| l.position(&c.nodes, &p)))
            .max()
            .unwrap_or(-1)
    }
    pub fn near(&self, point: Coord, tolerance: f64) -> bool {
        let p = Exact::Input(point);
        let tolerance = Q::from_float(tolerance).expect("finite tolerance");
        let squared = &tolerance * &tolerance;
        self.0.iter().any(|c| {
            c.locations.iter().any(|l| {
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
            for location in &c.locations {
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
    fn input(x: f64, y: f64) -> Exact {
        Exact::Input(Coord { x, y })
    }
    fn rational(x: Q, y: Q) -> Exact {
        Exact::Rational(Box::new((x, y)))
    }
    fn q(x: f64) -> Q {
        Q::from_float(x).unwrap()
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
