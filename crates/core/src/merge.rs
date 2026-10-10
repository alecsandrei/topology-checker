use crate::{
    Error, Layer, LayerId,
    engine::{
        cluster::{Cluster, distance},
        index,
        line_graph::projection,
    },
};
use geo::{BoundingRect, Coord, Geometry, LineString};
use std::collections::BTreeMap;

/// Merge original line parts through degree-two endpoint joins, preserving interior junctions.
///
/// This independent Rust implementation uses a chain pass followed by a cycle pass.
/// Each original part is visited once. Index construction and contact queries are separate
/// from the linear traversal. The input coordinates are never modified in place.
pub fn merge_lines(
    lines: impl IntoIterator<Item = LineString>,
    tolerance: f64,
) -> Result<Vec<LineString>, Error> {
    if !tolerance.is_finite() || tolerance < 0. {
        return Err(Error::Tolerance(tolerance));
    }
    let layer = Layer::from_geometries("merge-lines", lines.into_iter().map(Geometry::LineString));
    let parts = layer.parts(LayerId(0))?;
    let lines = parts.lines.iter().map(|p| &p.geometry).collect::<Vec<_>>();
    let mut cluster = Cluster::new(tolerance);
    let edges = lines
        .iter()
        .map(|l| [cluster.insert(l.0[0]), cluster.insert(l.0[l.0.len() - 1])])
        .collect::<Vec<_>>();
    let mut incident = vec![Vec::new(); cluster.coords.len()];
    for (i, edge) in edges.iter().enumerate() {
        incident[edge[0]].push((i, true));
        incident[edge[1]].push((i, false));
    }
    let segments = lines
        .iter()
        .enumerate()
        .flat_map(|(part, l)| {
            l.lines()
                .enumerate()
                .map(move |(position, line)| (part, position, line))
        })
        .collect::<Vec<_>>();
    let tree = index::build(
        segments
            .iter()
            .enumerate()
            .map(|(i, (_, _, line))| (i, line.bounding_rect())),
    );
    let mut blocked = vec![false; incident.len()];
    for (part, line) in lines.iter().enumerate() {
        for (end, &point) in [line.0[0], line.0[line.0.len() - 1]].iter().enumerate() {
            let node = edges[part][end];
            for item in tree.locate_in_envelope_intersecting(&index::at(point, tolerance)) {
                let (owner, position, segment) = segments[item.data];
                let (parameter, projected) = projection(segment, point);
                if distance(point, projected) > tolerance {
                    continue;
                }
                // A true endpoint contact belongs in the endpoint graph. Interior contacts
                // block merging, including a vertex between two segments of a part.
                let at_start =
                    position == 0 && cluster.insert(segment.start) == node && parameter == 0.;
                let at_end = position + 2 == lines[owner].0.len()
                    && cluster.insert(segment.end) == node
                    && parameter == 1.;
                if !at_start && !at_end {
                    blocked[node] = true;
                }
            }
        }
    }
    let mut visited = vec![false; edges.len()];
    let mut outputs = Vec::new();
    let walk = |start: usize, forward: bool, visited: &mut [bool]| {
        let mut coords = Vec::new();
        let mut current = (start, forward);
        let mut forwards = 0;
        let mut backwards = 0;
        let mut minimum = start;
        loop {
            let (id, direction) = current;
            if visited[id] {
                break;
            }
            visited[id] = true;
            minimum = minimum.min(id);
            if direction {
                forwards += 1;
            } else {
                backwards += 1;
            }
            let mut part = lines[id].0.clone();
            if !direction {
                part.reverse();
            }
            let first = edges[id][usize::from(!direction)];
            let last = edges[id][usize::from(direction)];
            part[0] = cluster.coords[first];
            let length = part.len();
            part[length - 1] = cluster.coords[last];
            if !coords.is_empty() {
                part.remove(0);
            }
            coords.extend(part);
            if blocked[last] || incident[last].len() != 2 {
                break;
            }
            let Some(next) = incident[last].iter().copied().find(|(id, _)| !visited[*id]) else {
                break;
            };
            current = next;
        }
        if backwards > forwards {
            coords.reverse();
        }
        (minimum, LineString(coords))
    };
    for (node, uses) in incident.iter().enumerate() {
        if uses.len() == 2 && !blocked[node] {
            continue;
        }
        for &(id, forward) in uses {
            if !visited[id] {
                outputs.push(walk(id, forward, &mut visited));
            }
        }
    }
    for id in 0..edges.len() {
        if !visited[id] {
            outputs.push(walk(id, true, &mut visited));
        }
    }
    outputs.sort_by_key(|(id, _)| *id);
    Ok(outputs.into_iter().map(|(_, line)| line).collect())
}

/// Explode nondegenerate parts into individual segments, preserving duplicate uses.
pub fn explode_lines(
    lines: impl IntoIterator<Item = LineString>,
) -> Result<Vec<LineString>, Error> {
    let layer =
        Layer::from_geometries("explode-lines", lines.into_iter().map(Geometry::LineString));
    Ok(layer
        .parts(LayerId(0))?
        .lines
        .iter()
        .flat_map(|p| p.geometry.lines())
        .map(LineString::from)
        .collect())
}

/// Walk a subset of noded edges. Overlap geometry is merged independently per feature pair.
pub(crate) fn merge_edges(coords: &[Coord], edges: &[[usize; 2]]) -> Vec<LineString> {
    let mut incident: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (id, edge) in edges.iter().enumerate() {
        for &node in edge {
            incident.entry(node).or_default().push(id);
        }
    }
    let mut visited = vec![false; edges.len()];
    let mut output = Vec::new();
    let walk = |start: usize, node: usize, visited: &mut [bool]| {
        let mut current = start;
        let mut node = node;
        let mut path = vec![coords[node]];
        loop {
            if visited[current] {
                break;
            }
            visited[current] = true;
            let edge = edges[current];
            node = if edge[0] == node { edge[1] } else { edge[0] };
            path.push(coords[node]);
            let uses = &incident[&node];
            if uses.len() != 2 {
                break;
            }
            let Some(next) = uses.iter().copied().find(|id| !visited[*id]) else {
                break;
            };
            current = next;
        }
        if let (Some(first), Some(last)) = (path.first(), path.last())
            && (last.x.total_cmp(&first.x).then(last.y.total_cmp(&first.y))).is_lt()
        {
            path.reverse();
        }
        LineString(path)
    };
    for (&node, uses) in &incident {
        if uses.len() != 2 {
            for &id in uses {
                if !visited[id] {
                    output.push(walk(id, node, &mut visited));
                }
            }
        }
    }
    for id in 0..edges.len() {
        if !visited[id] {
            output.push(walk(id, edges[id][0], &mut visited));
        }
    }
    output
}
