//! Merge part-level violations into one violation per rule, kind and feature pair.
use crate::{FeatureRef, Violation, engine::parallel::Workers};
use geo::{
    Geometry, GeometryCollection, LineString, MultiLineString, MultiPoint, MultiPolygon, Point,
    Polygon,
};

/// How the geometries of merged violations are combined.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Merge {
    /// Keep only the highest dimension present, such as overlap areas over their edges.
    /// An unsupported first geometry, such as an empty multi geometry, is kept when
    /// nothing else contributes.
    #[cfg(test)]
    HighestDimension,
    /// Keep every dimension, such as isolated contacts alongside line pieces, in a
    /// collection when several are present.
    AllDimensions,
}
impl Merge {
    /// Combine geometries. Coincident points are deduplicated.
    pub(crate) fn geometry(self, geometries: impl IntoIterator<Item = Geometry>) -> Geometry {
        let mut collected = Collected::default();
        match self {
            #[cfg(test)]
            Self::HighestDimension => {
                let mut fallback = None;
                for (i, g) in geometries.into_iter().enumerate() {
                    if let Some(unsupported) = collected.simple(g)
                        && i == 0
                    {
                        fallback = Some(unsupported);
                    }
                }
                let [points, lines, polygons] = collected.dimensions();
                polygons
                    .or(lines)
                    .or(points)
                    .or(fallback)
                    .unwrap_or_else(|| MultiPoint(Vec::new()).into())
            }
            Self::AllDimensions => {
                for g in geometries {
                    collected.all(g);
                }
                let mut present = collected
                    .dimensions()
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>();
                if present.len() == 1 {
                    present.remove(0)
                } else {
                    Geometry::GeometryCollection(GeometryCollection(present))
                }
            }
        }
    }
}

/// Points, lines and polygons gathered from several geometries.
#[derive(Default)]
struct Collected {
    points: Vec<Point>,
    lines: Vec<LineString>,
    polygons: Vec<Polygon>,
}
impl Collected {
    /// Collect a nonempty simple or multi geometry, returning any other geometry unchanged.
    fn simple(&mut self, g: Geometry) -> Option<Geometry> {
        match g {
            Geometry::Point(g) => self.points.push(g),
            Geometry::MultiPoint(g) if !g.0.is_empty() => self.points.extend(g.0),
            Geometry::Line(g) => self.lines.push(g.into()),
            Geometry::LineString(g) => self.lines.push(g),
            Geometry::MultiLineString(g) if !g.0.is_empty() => self.lines.extend(g.0),
            Geometry::Polygon(g) => self.polygons.push(g),
            Geometry::MultiPolygon(g) if !g.0.is_empty() => self.polygons.extend(g.0),
            other => return Some(other),
        }
        None
    }
    /// Collect every part, converting rectangles and triangles and flattening collections.
    fn all(&mut self, g: Geometry) {
        match g {
            Geometry::Rect(g) => self.polygons.push(g.to_polygon()),
            Geometry::Triangle(g) => self.polygons.push(g.to_polygon()),
            Geometry::GeometryCollection(g) => {
                for child in g {
                    self.all(child);
                }
            }
            // Empty multi geometries contribute nothing.
            g => {
                let _ = self.simple(g);
            }
        }
    }
    /// Return deduplicated points, then lines, then polygons, each when present.
    fn dimensions(mut self) -> [Option<Geometry>; 3] {
        fn single_or_multi<T: Into<Geometry>, M: Into<Geometry>>(
            mut parts: Vec<T>,
            multi: fn(Vec<T>) -> M,
        ) -> Option<Geometry> {
            match parts.len() {
                0 => None,
                1 => Some(parts.remove(0).into()),
                _ => Some(multi(parts).into()),
            }
        }
        // Polygon rule aggregation occurs on original operands before emission.
        // Never feed emitted intersection coordinates into another overlay.
        // Parallel candidate discovery must not change multipart ordering.
        self.lines.sort_by(|a, b| {
            a.0.iter()
                .zip(&b.0)
                .map(|(a, b)| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)))
                .find(|order| !order.is_eq())
                .unwrap_or_else(|| a.0.len().cmp(&b.0.len()))
        });
        self.points
            .sort_by(|a, b| a.x().total_cmp(&b.x()).then(a.y().total_cmp(&b.y())));
        self.points.dedup();
        [
            single_or_multi(self.points, MultiPoint),
            single_or_multi(self.lines, MultiLineString),
            single_or_multi(self.polygons, MultiPolygon),
        ]
    }
}

/// Merge violations sharing a rule, kind and origin and destination features.
/// Part indices are cleared when several parts contributed; groups follow key order.
/// Groups are found by sorting, without an allocation each, so a cancelled
/// aggregation frees little, and merge on several threads. Each violation moves once
/// before its merge, into the task of its group.
pub(crate) fn aggregate(mut violations: Vec<Violation>, merge: Merge) -> Vec<Violation> {
    let len = violations.len();
    let workers = Workers::new(len);
    let mut order = Vec::with_capacity(len);
    for (i, v) in violations.iter().enumerate() {
        crate::cancel::every(i, "aggregate");
        order.push(place(v, i));
    }
    workers.sort(&mut order, "aggregate.sort", Ord::cmp);
    let same = |a: &Place, b: &Place| a[..GROUP] == b[..GROUP];
    // Whole groups for each task: their places, the number of groups before them
    // and their violations in order.
    let mut tasks = Vec::new();
    let (mut start, mut groups) = (0, 0);
    for range in workers.ranges(len) {
        let mut end = range.end.max(start);
        while end < len && same(&order[end], &order[end - 1]) {
            end += 1;
        }
        if end == start {
            continue;
        }
        let places = &order[start..end];
        let mut members = Vec::with_capacity(places.len());
        for (k, place) in places.iter().enumerate() {
            crate::cancel::every(start + k, "aggregate.gather");
            let at = usize::try_from(place[GROUP]).expect("a position");
            members.push(take(&mut violations[at]));
        }
        tasks.push((places, groups, members));
        groups += 1 + places.windows(2).filter(|w| !same(&w[0], &w[1])).count();
        start = end;
    }
    drop(violations);
    let merged = workers.each(tasks, |(places, before, members)| {
        let mut members = members.into_iter();
        let mut out = Vec::new();
        let mut k = 0;
        while k < places.len() {
            crate::cancel::every(before + out.len(), "aggregate.merge");
            let size = places[k..].partition_point(|other| same(other, &places[k]));
            out.push(merged(members.by_ref().take(size), merge));
            k += size;
        }
        out
    });
    let total = merged.iter().map(Vec::len).sum::<usize>();
    let mut out = Vec::new();
    for part in merged {
        crate::cancel::checkpoint("aggregate.join");
        // The others join the first part, which stays where it is.
        if out.is_empty() {
            out = part;
            out.reserve(total - out.len());
        } else {
            out.extend(part);
        }
    }
    out
}
/// Move a violation out, leaving it without a geometry.
fn take(v: &mut Violation) -> Violation {
    let empty = Geometry::GeometryCollection(GeometryCollection(Vec::new()));
    Violation {
        geometry: std::mem::replace(&mut v.geometry, empty),
        ..*v
    }
}

/// Words of a violation's group, then its position, which sort as the rule, kind,
/// origin and destination features, then the order of the violations.
type Place = [u64; GROUP + 1];
/// Words of the group in a [`Place`].
const GROUP: usize = 5;
fn place(v: &Violation, position: usize) -> Place {
    // Layers count from one, so a missing feature sorts first.
    let feature = |f: Option<FeatureRef>| {
        f.map_or([0, 0], |f| {
            [f.layer.0 as u64 + 1, f.fid.cast_unsigned() ^ (1 << 63)]
        })
    };
    let ([a, b], [c, d]) = (feature(v.origin), feature(v.destination));
    let kind = (v.rule as u64) << 32 | v.kind as u64;
    [kind, a, b, c, d, position as u64]
}

/// One violation for a nonempty group sharing a key, in group order.
fn merged(mut group: impl Iterator<Item = Violation>, merge: Merge) -> Violation {
    let mut first = group.next().expect("aggregation groups are nonempty");
    let mut geometries = vec![std::mem::replace(
        &mut first.geometry,
        Geometry::GeometryCollection(GeometryCollection(Vec::new())),
    )];
    for v in group {
        if let (Some(a), Some(b)) = (&mut first.origin, v.origin) {
            a.aggregate(b);
        }
        if let (Some(a), Some(b)) = (&mut first.destination, v.destination) {
            a.aggregate(b);
        }
        geometries.push(v.geometry);
    }
    first.geometry = merge.geometry(geometries);
    first
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{LayerId, Rule, ViolationKind};
    use geo::{Rect, coord};

    fn violation(geometry: Geometry, part: u32) -> Violation {
        Violation {
            rule: Rule::LineMustNotIntersect,
            kind: ViolationKind::Crossing,
            geometry,
            origin: Some(FeatureRef {
                layer: LayerId(0),
                fid: -4,
                part: Some(part),
            }),
            destination: Some(FeatureRef {
                layer: LayerId(1),
                fid: 8,
                part: Some(2),
            }),
        }
    }

    #[test]
    fn aggregation_retains_provenance_deduplication_and_geometry_priority() {
        let point = Point::new(1., 0.);
        let points = aggregate(
            vec![violation(point.into(), 0), violation(point.into(), 1)],
            Merge::HighestDimension,
        );
        assert_eq!(points.len(), 1);
        assert_eq!(points[0].geometry, Geometry::Point(point));
        assert_eq!(points[0].origin.unwrap().part, None);
        assert_eq!(points[0].destination.unwrap().part, Some(2));
        let line = LineString::from(vec![(0., 0.), (1., 0.)]);
        let polygon = Rect::new(coord! {x:0.,y:0.}, coord! {x:1.,y:1.}).to_polygon();
        let mixed = aggregate(
            vec![
                violation(point.into(), 0),
                violation(line.into(), 0),
                violation(polygon.clone().into(), 1),
            ],
            Merge::HighestDimension,
        );
        assert_eq!(mixed[0].geometry, Geometry::Polygon(polygon));
        let empty = Geometry::MultiLineString(MultiLineString(Vec::new()));
        assert_eq!(
            aggregate(vec![violation(empty.clone(), 0)], Merge::HighestDimension)[0].geometry,
            empty
        );
    }

    #[test]
    fn all_dimensions_keep_contacts_beside_pieces_and_flatten_collections() {
        let point = Point::new(1., 0.);
        let line = LineString::from(vec![(0., 0.), (1., 0.)]);
        let rect = Rect::new(coord! {x:0.,y:0.}, coord! {x:1.,y:1.});
        let merged = aggregate(
            vec![
                violation(point.into(), 0),
                violation(
                    Geometry::GeometryCollection(GeometryCollection(vec![
                        point.into(),
                        line.clone().into(),
                    ])),
                    1,
                ),
                violation(rect.into(), 1),
            ],
            Merge::AllDimensions,
        );
        assert_eq!(merged.len(), 1);
        assert_eq!(
            merged[0].geometry,
            Geometry::GeometryCollection(GeometryCollection(vec![
                point.into(),
                line.clone().into(),
                rect.to_polygon().into()
            ]))
        );
        assert_eq!(
            Merge::AllDimensions.geometry([line.clone().into()]),
            Geometry::LineString(line)
        );
        assert_eq!(
            Merge::AllDimensions.geometry([MultiPoint(Vec::new()).into()]),
            Geometry::GeometryCollection(GeometryCollection(Vec::new()))
        );
    }

    #[test]
    fn convenience_geometries_collect_as_lines_and_polygons() {
        let line = geo::Line::new(coord! {x: 0., y: 0.}, coord! {x: 1., y: 0.});
        let triangle = geo::Triangle::new(
            coord! {x: 0., y: 0.},
            coord! {x: 1., y: 0.},
            coord! {x: 0., y: 1.},
        );
        let merged = Merge::AllDimensions.geometry([line.into(), triangle.into()]);
        assert_eq!(
            merged,
            Geometry::GeometryCollection(GeometryCollection(vec![
                Geometry::LineString(line.into()),
                Geometry::Polygon(triangle.to_polygon()),
            ]))
        );
        let single = Merge::HighestDimension.geometry([line.into()]);
        assert_eq!(single, Geometry::LineString(line.into()));
    }

    /// Violations of a few rules and feature pairs, with points, lines and polygons,
    /// which merge in input order.
    fn many(n: usize, features: usize) -> Vec<Violation> {
        (0..n)
            .map(|i| {
                let x = (i * 7 % 13) as f64;
                let geometry = match i % 4 {
                    0 => Point::new(x, 0.).into(),
                    1 => LineString::from(vec![(x, 0.), (x + 1., 1.)]).into(),
                    2 => Rect::new(coord! {x: x, y: 0.}, coord! {x: x + 1., y: i as f64 + 1.})
                        .to_polygon()
                        .into(),
                    _ => Point::new(x, 1.).into(),
                };
                let feature = |layer, fid: usize| FeatureRef {
                    layer: LayerId(layer),
                    fid: (fid % features) as i64 - 3,
                    part: Some((i % 4) as u32),
                };
                Violation {
                    rule: [Rule::LineMustNotIntersect, Rule::PointMustNotOverlap][i % 2],
                    kind: ViolationKind::Crossing,
                    geometry,
                    origin: Some(feature(0, i * 31)),
                    destination: (i % 5 != 0).then(|| feature(1, i * 17 / 3)),
                }
            })
            .collect()
    }

    #[test]
    fn groups_merge_in_key_then_input_order_for_any_split() {
        for (n, features) in [
            (0, 1),
            (1, 1),
            (9, 2),
            (5000, 1),
            (20_000, 40),
            (20_000, 50_000),
        ] {
            let violations = many(n, features);
            let mut groups = std::collections::BTreeMap::<_, Vec<_>>::new();
            for v in violations.clone() {
                let origin = v.origin.map(FeatureRef::feature);
                let destination = v.destination.map(FeatureRef::feature);
                groups
                    .entry((v.rule, v.kind, origin, destination))
                    .or_default()
                    .push(v);
            }
            let expected = groups
                .into_values()
                .map(|group| merged(group.into_iter(), Merge::AllDimensions))
                .collect::<Vec<_>>();
            for (threads, grain) in [(1, 4096), (3, 1), (4, 64)] {
                let found = Workers::forced(threads, grain, || {
                    aggregate(violations.clone(), Merge::AllDimensions)
                });
                assert_eq!(found, expected, "{n} with {features} on {threads}");
            }
        }
    }

    #[test]
    fn aggregation_stops_in_each_step_when_cancelled() {
        use crate::cancel::{self, hooks};
        let threads = 4;
        let violations = many(20_000, 400);
        let run = |token: &cancel::CancellationToken| {
            std::panic::catch_unwind(|| {
                cancel::scope(Some(token.clone()), || {
                    Workers::forced(threads, 64, || {
                        aggregate(violations.clone(), Merge::AllDimensions).len()
                    })
                })
            })
        };
        let token = hooks::watched(None);
        run(&token).expect("merged");
        let visits = hooks::record(&token).visits;
        let steps = visits.keys().copied().collect::<Vec<_>>();
        assert_eq!(
            steps,
            [
                "aggregate",
                "aggregate.gather",
                "aggregate.join",
                "aggregate.merge",
                "aggregate.sort",
                "sort.buckets",
                "sort.classes",
                "sort.copy",
                "sort.moves",
                "sort.samples"
            ]
        );
        for (step, count) in visits {
            for visit in [0, count / 2, count - 1] {
                let token = hooks::watched(Some((step, visit)));
                let result = run(&token);
                let record = hooks::record(&token);
                assert!(
                    result.expect_err("cancelled").is::<cancel::Cancelled>(),
                    "{step} {visit}"
                );
                // Each other thread passes at most one more checkpoint.
                let after = record.visits[step];
                assert!(
                    (visit + 1..=visit + threads).contains(&after),
                    "{step} {visit}: {after}"
                );
                assert_eq!(record.live, 0);
            }
        }
    }
}
