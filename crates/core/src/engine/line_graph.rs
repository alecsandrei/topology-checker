use crate::{
    Error, FeatureRef, LayerId, Topology,
    engine::{
        cluster::{Cluster, distance},
        index,
    },
};
use geo::{
    BoundingRect, Coord, Line,
    line_intersection::{LineIntersection, line_intersection},
};
use num_rational::BigRational;
use num_traits::ToPrimitive;
use rayon::prelude::*;
use std::collections::HashMap;

#[derive(Debug)]
pub(crate) struct GraphPart {
    pub reference: FeatureRef,
    pub nodes: Vec<usize>,
}
impl GraphPart {
    pub fn closed(&self) -> bool {
        self.nodes.len() > 1 && self.nodes.first() == self.nodes.last()
    }
    pub fn endpoint(&self, node: usize) -> bool {
        !self.closed() && (self.nodes.first() == Some(&node) || self.nodes.last() == Some(&node))
    }
}
#[derive(Debug, Clone, Copy)]
pub(crate) struct EdgeUse {
    pub part: usize,
    pub forward: bool,
}
#[derive(Debug)]
pub(crate) struct Edge {
    pub nodes: [usize; 2],
    pub uses: Vec<EdgeUse>,
}
#[derive(Debug, Clone, Copy)]
pub(crate) struct Incident {
    pub edge: usize,
    pub part: usize,
}
#[derive(Debug)]
pub(crate) struct LineGraph {
    pub coords: Vec<Coord>,
    pub parts: Vec<GraphPart>,
    pub edges: Vec<Edge>,
    pub incident: Vec<Vec<Incident>>,
}
#[derive(Debug, Clone, Copy)]
pub(crate) struct Segment {
    pub line: Line,
    pub part: usize,
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
        Coord {
            x: line.start.x + parameter * dx,
            y: line.start.y + parameter * dy,
        },
    )
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
    Some(Coord {
        x: (ax + &t * rx).to_f64()?,
        y: (ay + t * ry).to_f64()?,
    })
}
impl LineGraph {
    pub fn build(t: &Topology, layers: &[(LayerId, InputMode)]) -> Result<Self, Error> {
        let mut parts = Vec::new();
        let mut segments = Vec::new();
        let mut endpoints = Vec::new();
        for &(layer, mode) in layers {
            let input = t.parts(layer)?;
            let lines = match mode {
                InputMode::Linework => &input.lines,
                InputMode::PolygonBoundary => &crate::engine::cross::views(input).boundaries,
            };
            for p in lines {
                let id = parts.len();
                parts.push(GraphPart {
                    // Graphs belong to one topology, so parts carry final layer IDs.
                    reference: p.reference.at(layer),
                    nodes: Vec::new(),
                });
                endpoints.push([p.geometry.0[0], p.geometry.0[p.geometry.0.len() - 1]]);
                segments.extend(p.geometry.lines().map(|line| Segment { line, part: id }));
            }
        }
        let tree = index::build(
            segments
                .iter()
                .enumerate()
                .map(|(i, s)| (i, s.line.bounding_rect())),
        );
        let pairs = index::pairs(&tree, &tree, 0., true);
        let intersections = pairs
            .par_iter()
            .filter_map(|&(i, j)| {
                line_intersection(segments[i].line, segments[j].line).map(|mut g| {
                    if let LineIntersection::SinglePoint {
                        intersection,
                        is_proper: true,
                    } = &mut g
                        && let Some(point) = proper_intersection(segments[i].line, segments[j].line)
                    {
                        *intersection = point;
                    }
                    (i, j, g)
                })
            })
            .collect::<Vec<_>>();
        let mut split = segments
            .iter()
            .map(|s| vec![s.line.start, s.line.end])
            .collect::<Vec<_>>();
        for (i, j, g) in intersections {
            match g {
                LineIntersection::SinglePoint { intersection, .. } => {
                    split[i].push(intersection);
                    split[j].push(intersection);
                }
                LineIntersection::Collinear { intersection } => {
                    split[i].extend([intersection.start, intersection.end]);
                    split[j].extend([intersection.start, intersection.end]);
                }
            }
        }
        // Endpoint cracking has no effect on the immutable input geometries.
        if t.tolerance() > 0. {
            for ends in &endpoints {
                for &point in ends {
                    let mut candidates = tree
                        .locate_in_envelope_intersecting(&index::at(point, t.tolerance()))
                        .map(|i| i.data)
                        .collect::<Vec<_>>();
                    candidates.sort_unstable();
                    for i in candidates {
                        let (_, projected) = projection(segments[i].line, point);
                        if distance(point, projected) <= t.tolerance() {
                            split[i].push(projected);
                        }
                    }
                }
            }
        }
        let mut cluster = Cluster::new(t.tolerance());
        for ends in &endpoints {
            for &point in ends {
                cluster.insert(point);
            }
        }
        let mut edges: Vec<Edge> = Vec::new();
        let mut keys = HashMap::new();
        for (s, mut cuts) in segments.into_iter().zip(split) {
            cuts.sort_by(|a, b| parameter(s.line, *a).total_cmp(&parameter(s.line, *b)));
            let mut nodes = cuts
                .into_iter()
                .map(|p| cluster.insert(p))
                .collect::<Vec<_>>();
            nodes.dedup();
            let path = &mut parts[s.part].nodes;
            for &node in &nodes {
                if path.last() != Some(&node) {
                    path.push(node);
                }
            }
            for pair in nodes.windows(2) {
                let [a, b] = [pair[0], pair[1]];
                if a == b {
                    continue;
                }
                let key = (a.min(b), a.max(b));
                let id = *keys.entry(key).or_insert_with(|| {
                    let id = edges.len();
                    edges.push(Edge {
                        nodes: [key.0, key.1],
                        uses: Vec::new(),
                    });
                    id
                });
                edges[id].uses.push(EdgeUse {
                    part: s.part,
                    forward: a == key.0,
                });
            }
        }
        let mut incident = vec![Vec::new(); cluster.coords.len()];
        for (id, edge) in edges.iter().enumerate() {
            for usage in &edge.uses {
                for node in if usage.forward {
                    edge.nodes
                } else {
                    [edge.nodes[1], edge.nodes[0]]
                } {
                    incident[node].push(Incident {
                        edge: id,
                        part: usage.part,
                    });
                }
            }
        }
        Ok(Self {
            coords: cluster.coords,
            parts,
            edges,
            incident,
        })
    }
}
