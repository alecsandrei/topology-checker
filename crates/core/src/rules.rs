//! Rule implementations. [`dispatch`] is the only dispatch on [`Rule`]: each arm calls
//! a rule family with a typed mode, so adding a rule cannot fall through a nested match.
mod boundary;
mod containment;
mod coverage;
mod geometry;
mod line;
mod line_polygon;
mod point;
mod point_polygon;
mod polygon;
use crate::{Error, FeatureRef, Rule, RuleSpec, Topology, Violation, ViolationKind};
use boundary::Boundary;
use coverage::Target;
use geo::{Coord, Geometry, LineString};
use line::LineCheck;
use line_polygon::LinePolygon;
use point_polygon::{PointCount, PointTest};
use rayon::prelude::*;

pub(crate) fn run(t: &Topology, spec: RuleSpec) -> Result<Vec<Violation>, Error> {
    // A worker thread can pick this rule up inside another validation's scope.
    t.cancellable(|| dispatch(t, spec))
}
fn dispatch(t: &Topology, spec: RuleSpec) -> Result<Vec<Violation>, Error> {
    // Registration guarantees a secondary layer for every binary rule.
    let other = || {
        spec.other.ok_or(Error::Inputs {
            rule: spec.rule,
            expected: 2,
        })
    };
    match spec.rule {
        Rule::GeometryMustNotBeMultipart => geometry::multipart(t, spec),
        Rule::PointMustNotOverlap | Rule::PointMustNotOverlapWith => point::overlap(t, spec),
        Rule::PointMustBeInside => point::inside(t, spec, other()?),
        Rule::PolygonMustNotOverlap | Rule::PolygonMustNotOverlapWith => polygon::overlap(t, spec),
        Rule::PolygonMustNotHaveGaps => polygon::gaps(t, spec),
        Rule::LineMustBeInside => polygon::line_inside(t, spec, other()?),
        Rule::LineMustNotHaveDangles => line::run(t, spec, LineCheck::Dangles),
        Rule::LineMustNotHavePseudoNodes => line::run(t, spec, LineCheck::PseudoNodes),
        Rule::LineMustNotOverlap | Rule::LineMustNotOverlapWith => {
            line::run(t, spec, LineCheck::Overlap)
        }
        Rule::LineMustNotSelfOverlap => line::run_self(t, spec, false),
        Rule::LineMustNotSelfIntersect => line::run_self(t, spec, true),
        Rule::LineMustNotIntersect | Rule::LineMustNotIntersectWith => {
            line::run(t, spec, LineCheck::Intersect { touch: false })
        }
        Rule::LineMustNotIntersectOrTouchInterior
        | Rule::LineMustNotIntersectOrTouchInteriorWith => {
            line::run(t, spec, LineCheck::Intersect { touch: true })
        }
        Rule::PointMustBeCoveredByLines => coverage::points(t, spec, other()?, Target::Linework),
        Rule::PointMustBeCoveredByLineEndpoints => {
            coverage::points(t, spec, other()?, Target::LineEndpoints)
        }
        Rule::PointMustBeCoveredByPolygonBoundaries => {
            coverage::points(t, spec, other()?, Target::PolygonBoundaries)
        }
        Rule::LineEndpointsMustBeCoveredByPoints => {
            coverage::endpoints(t, spec, other()?, Target::Points)
        }
        Rule::LineEndpointsMustBeCoveredByPolygonBoundaries => {
            coverage::endpoints(t, spec, other()?, Target::PolygonBoundaries)
        }
        Rule::PointMustNotIntersectLines => coverage::line_contacts(t, spec, other()?),
        Rule::PointMustBeCoveredByPolygons => {
            point_polygon::points(t, spec, other()?, PointTest::Covered)
        }
        Rule::PointMustNotBeInsidePolygons => {
            point_polygon::points(t, spec, other()?, PointTest::NotInside)
        }
        Rule::PointMustNotIntersectPolygons => {
            point_polygon::points(t, spec, other()?, PointTest::NotIntersecting)
        }
        Rule::PolygonMustNotContainPoints => point_polygon::contained_points(t, spec, other()?),
        Rule::PolygonMustContainPoint => {
            point_polygon::point_counts(t, spec, other()?, PointCount::AtLeastOne)
        }
        Rule::PolygonMustContainOnePoint => {
            point_polygon::point_counts(t, spec, other()?, PointCount::ExactlyOne)
        }
        Rule::GeometryMustNotBeContainedBy => containment::contained(t, spec, other()?),
        Rule::LineMustBeCoveredByPolygonBoundaries => {
            boundary::uncovered(t, spec, other()?, Boundary::LinesCovered)
        }
        Rule::PolygonBoundariesMustBeCoveredByLines => {
            boundary::uncovered(t, spec, other()?, Boundary::BoundariesCovered)
        }
        Rule::LineMustNotOverlapPolygonBoundaries => boundary::shared(t, spec, other()?),
        Rule::LineMustBeProperlyInsidePolygons => line_polygon::properly_inside(t, spec, other()?),
        Rule::LineMustNotCrossPolygonBoundaries => {
            line_polygon::run(t, spec, other()?, LinePolygon::Cross)
        }
        Rule::LineMustNotOverlapWithPolygons => {
            line_polygon::run(t, spec, other()?, LinePolygon::Overlap)
        }
        Rule::LineMustNotIntersectPolygons => {
            line_polygon::run(t, spec, other()?, LinePolygon::Intersect)
        }
    }
}

fn violation(
    spec: RuleSpec,
    kind: ViolationKind,
    geometry: impl Into<Geometry>,
    origin: Option<FeatureRef>,
    destination: Option<FeatureRef>,
) -> Violation {
    Violation {
        rule: spec.rule,
        kind,
        geometry: geometry.into(),
        origin,
        destination,
    }
}
/// Extract connected selected pieces before applying the length threshold.
/// A closed path's first and last runs form one piece across the traversal seam.
fn selected_pieces(coords: &[Coord], selected: &[bool]) -> Vec<LineString> {
    selected_runs(coords, selected)
        .into_iter()
        .map(|run| LineString(run.into_iter().map(|i| coords[i]).collect()))
        .collect()
}
/// Coordinate indices of each piece of `selected_pieces`.
fn selected_runs(coords: &[Coord], selected: &[bool]) -> Vec<Vec<usize>> {
    let mut runs = Vec::new();
    let mut run = Vec::new();
    for (i, &keep) in selected.iter().enumerate() {
        if keep {
            if run.is_empty() {
                run.push(i);
            }
            run.push(i + 1);
        } else if !run.is_empty() {
            runs.push(std::mem::take(&mut run));
        }
    }
    if !run.is_empty() {
        runs.push(run);
    }
    if runs.len() > 1
        && coords.first() == coords.last()
        && selected.first() == Some(&true)
        && selected.last() == Some(&true)
    {
        let first = runs.remove(0);
        let last = runs.last_mut().expect("two runs before joining");
        last.extend(first.into_iter().skip(1));
    }
    runs
}

/// Emit deterministic aggregated violations, with a bounded duplicate-pair path.
pub(crate) fn run_into<E>(
    t: &Topology,
    spec: RuleSpec,
    mut sink: impl FnMut(Violation) -> Result<(), E>,
) -> Result<(), crate::SinkError<E>> {
    if matches!(
        spec.rule,
        Rule::PointMustNotOverlap | Rule::PointMustNotOverlapWith
    ) {
        point::overlap_into(t, spec, sink)
    } else if matches!(
        spec.rule,
        Rule::PolygonMustNotHaveGaps
            | Rule::GeometryMustNotBeMultipart
            | Rule::PolygonMustContainPoint
            | Rule::PolygonMustContainOnePoint
            | Rule::LineMustNotHaveDangles
            | Rule::LineMustNotHavePseudoNodes
    ) {
        emit(run(t, spec)?, &mut sink)
    } else {
        let batch = rayon::current_num_threads() * FEATURES_PER_THREAD;
        per_feature_into(t, spec, batch, &mut sink)
    }
}
/// Features per thread in one parallel batch of [`per_feature_into`]. Output waits
/// for a whole batch, so batches stay small enough to stream early and to bound the
/// violations held at once, and large enough to balance uneven features.
const FEATURES_PER_THREAD: usize = 8;
/// Kinds of a per-feature rule in emission order. Empty when each feature emits all
/// of its violations together.
fn feature_kinds(rule: Rule) -> &'static [ViolationKind] {
    match rule {
        Rule::LineMustNotSelfIntersect => {
            &[ViolationKind::SelfOverlap, ViolationKind::SelfCrossing]
        }
        Rule::LineMustNotIntersect | Rule::LineMustNotIntersectWith => {
            &[ViolationKind::Overlap, ViolationKind::Crossing]
        }
        Rule::LineMustNotIntersectOrTouchInterior
        | Rule::LineMustNotIntersectOrTouchInteriorWith => &[
            ViolationKind::Overlap,
            ViolationKind::Crossing,
            ViolationKind::TouchInterior,
        ],
        _ => &[],
    }
}
/// Run a rule on one original feature at a time and emit in kind order, then feature
/// order. Geometry scratch belongs to one feature and its candidate targets, and no
/// complete feature-pair report is retained. Batches of `batch` features run in
/// parallel and each feature runs once: its first kind is emitted with its batch, and
/// later kinds wait until every feature has emitted the first. The first feature runs
/// alone, so it builds the caches every feature shares, such as the line graph, before
/// workers would wait for them.
fn per_feature_into<E>(
    t: &Topology,
    spec: RuleSpec,
    batch: usize,
    sink: &mut impl FnMut(Violation) -> Result<(), E>,
) -> Result<(), crate::SinkError<E>> {
    let mut fids = t
        .layer(spec.layer)?
        .features()
        .map(|(fid, _)| fid)
        .collect::<Vec<_>>();
    fids.sort_unstable();
    let kinds = feature_kinds(spec.rule);
    let mut later = kinds.iter().skip(1).map(|_| Vec::new()).collect::<Vec<_>>();
    let feature = |fid| {
        // Worker threads do not see the caller's cancellation scope.
        t.cancellable(|| {
            #[cfg(test)]
            tests::visit(t, spec, fid)?;
            let mut snapshot = t.snapshot();
            snapshot.select_feature(spec.layer, fid)?;
            let mut violations = run(&snapshot, spec)?;
            sort(&mut violations);
            Ok::<_, Error>(violations)
        })
    };
    let first = fids.len().min(1);
    let batches = std::iter::once(&fids[..first]).chain(fids[first..].chunks(batch.max(1)));
    for fids in batches {
        crate::cancel::check();
        let results = match fids {
            &[fid] => vec![feature(fid)],
            _ => fids.par_iter().map(|&fid| feature(fid)).collect::<Vec<_>>(),
        };
        for violations in results {
            for v in violations? {
                match kinds.iter().position(|&kind| kind == v.kind) {
                    Some(0) => send(v, sink)?,
                    Some(i) => later[i - 1].push(v),
                    None if kinds.is_empty() => send(v, sink)?,
                    None => {}
                }
            }
        }
    }
    later.into_iter().flatten().try_for_each(|v| send(v, sink))
}
fn emit<E>(
    mut violations: Vec<Violation>,
    sink: &mut impl FnMut(Violation) -> Result<(), E>,
) -> Result<(), crate::SinkError<E>> {
    sort(&mut violations);
    violations.into_iter().try_for_each(|v| send(v, sink))
}
fn sort(violations: &mut [Violation]) {
    violations.sort_by_key(|v| {
        (
            v.kind,
            v.origin.map(FeatureRef::feature),
            v.destination.map(FeatureRef::feature),
        )
    });
}
fn send<E>(
    v: Violation,
    sink: &mut impl FnMut(Violation) -> Result<(), E>,
) -> Result<(), crate::SinkError<E>> {
    crate::cancel::check();
    sink(v).map_err(crate::SinkError::Consumer)
}

fn feature_geometry(
    t: &Topology,
    spec: RuleSpec,
    layer: crate::LayerId,
    feature: &crate::engine::cross::Feature,
) -> Result<Geometry, Error> {
    t.cancellable(|| feature.output()).map_err(|e| {
        e.precision_context(
            "polygon feature emission",
            spec.rule,
            format!(
                ", layer {}, FID {}",
                t.layer(layer).expect("registered layer").name(),
                feature.reference.fid
            ),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CancellationToken, GeometryKind, Layer, SinkError, model::PartRef};
    use geo::{
        GeometryCollection, LineString, MultiLineString, MultiPoint, MultiPolygon, Point, Polygon,
        Rect, coord,
    };
    use std::{
        convert::Infallible,
        panic::AssertUnwindSafe,
        sync::{Arc, Mutex, PoisonError, mpsc},
        time::{Duration, Instant},
    };

    #[derive(Debug, Clone)]
    enum Action {
        Cancel(CancellationToken),
        Fail,
        Panic(&'static str),
        Hold(Duration),
        Note(Arc<Mutex<Vec<Instant>>>),
    }
    /// Actions keyed by primary layer name and FID, taken as a worker starts that feature.
    static ACTIONS: Mutex<Vec<(String, i64, Action)>> = Mutex::new(Vec::new());
    pub(super) fn visit(t: &Topology, spec: RuleSpec, fid: i64) -> Result<(), Error> {
        let Ok(layer) = t.layer(spec.layer) else {
            return Ok(());
        };
        let action = ACTIONS
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .find(|(name, at, _)| name == layer.name() && *at == fid)
            .map(|(_, _, action)| action.clone());
        match action {
            Some(Action::Cancel(token)) => token.cancel(),
            Some(Action::Fail) => {
                return Err(Error::Precision {
                    operation: "feature".into(),
                    rule: Some(spec.rule),
                    context: format!(", FID {fid}"),
                    reason: "injected".into(),
                });
            }
            Some(Action::Panic(message)) => std::panic::panic_any(message),
            Some(Action::Hold(duration)) => std::thread::sleep(duration),
            Some(Action::Note(starts)) => starts
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(Instant::now()),
            None => {}
        }
        Ok(())
    }
    fn on(t: &Topology, spec: RuleSpec, fid: i64, action: Action) -> String {
        let name = t.layer(spec.layer).unwrap().name().to_string();
        ACTIONS
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((name.clone(), fid, action));
        name
    }
    fn clear(name: &str) {
        ACTIONS
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|(at, ..)| at != name);
    }

    /// Rules that [`run_into`] runs one feature at a time.
    fn per_feature(rule: Rule) -> bool {
        !matches!(
            rule,
            Rule::PointMustNotOverlap
                | Rule::PointMustNotOverlapWith
                | Rule::PolygonMustNotHaveGaps
                | Rule::GeometryMustNotBeMultipart
                | Rule::PolygonMustContainPoint
                | Rule::PolygonMustContainOnePoint
                | Rule::LineMustNotHaveDangles
                | Rule::LineMustNotHavePseudoNodes
        )
    }
    /// Emitted violations, then the result.
    type Emission = (Vec<Violation>, Result<(), Error>);
    /// Emission by a sequential pass per kind, each rerunning every feature in FID
    /// order on one reused snapshot.
    fn reference_emission(t: &Topology, spec: RuleSpec) -> Emission {
        if !per_feature(spec.rule) {
            return emission(t, spec, None);
        }
        let mut out = Vec::new();
        let result = (|| {
            let mut snapshot = t.snapshot();
            let mut fids = t
                .layer(spec.layer)?
                .features()
                .map(|(fid, _)| fid)
                .collect::<Vec<_>>();
            fids.sort_unstable();
            let kinds: &[ViolationKind] = match spec.rule {
                Rule::LineMustNotSelfIntersect => {
                    &[ViolationKind::SelfOverlap, ViolationKind::SelfCrossing]
                }
                Rule::LineMustNotIntersect | Rule::LineMustNotIntersectWith => {
                    &[ViolationKind::Overlap, ViolationKind::Crossing]
                }
                Rule::LineMustNotIntersectOrTouchInterior
                | Rule::LineMustNotIntersectOrTouchInteriorWith => &[
                    ViolationKind::Overlap,
                    ViolationKind::Crossing,
                    ViolationKind::TouchInterior,
                ],
                _ => &[],
            };
            for kind in if kinds.is_empty() {
                vec![None]
            } else {
                kinds.iter().copied().map(Some).collect()
            } {
                for &fid in &fids {
                    visit(t, spec, fid)?;
                    snapshot.select_feature(spec.layer, fid)?;
                    let mut violations = run(&snapshot, spec)?;
                    if let Some(kind) = kind {
                        violations.retain(|v| v.kind == kind);
                    }
                    sort(&mut violations);
                    out.extend(violations);
                }
            }
            Ok(())
        })();
        (out, result)
    }
    /// Emission by [`run_into`], or by [`per_feature_into`] with a given batch size.
    fn emission(t: &Topology, spec: RuleSpec, batch: Option<usize>) -> Emission {
        let mut out = Vec::new();
        let mut sink = |v| {
            out.push(v);
            Ok::<_, Infallible>(())
        };
        let result = match batch {
            Some(batch) => per_feature_into(t, spec, batch, &mut sink),
            None => run_into(t, spec, &mut sink),
        };
        (out, result.map_err(Error::from))
    }
    fn reference(t: &Topology, spec: RuleSpec) -> Result<Vec<Violation>, Error> {
        let (out, result) = reference_emission(t, spec);
        result.map(|()| out)
    }
    fn streamed(
        t: &Topology,
        spec: RuleSpec,
        batch: Option<usize>,
    ) -> Result<Vec<Violation>, Error> {
        let (out, result) = emission(t, spec, batch);
        result.map(|()| out)
    }
    /// A whole-layer run, as eager validation does it.
    fn eager(t: &Topology, spec: RuleSpec) -> Result<Vec<Violation>, Error> {
        run(t, spec)
    }

    /// Seeded xorshift, so a failure reproduces from its seed.
    struct Rng(u64);
    impl Rng {
        fn new(seed: u64) -> Self {
            Self((seed + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15))
        }
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }
        fn chance(&mut self, percent: usize) -> bool {
            self.below(100) < percent
        }
        fn pick<T: Copy>(&mut self, items: &[T]) -> T {
            items[self.below(items.len())]
        }
    }
    /// Coordinates `offset + scale * (grid + jitter)` on a 7 by 7 grid, so features
    /// often share vertices, segments and near misses.
    #[derive(Clone, Copy)]
    struct Frame {
        offset: f64,
        scale: f64,
        jitter: f64,
    }
    impl Frame {
        fn random(rng: &mut Rng) -> Self {
            Self {
                offset: rng.pick(&[0., -250., 1e3, 1e6, 1e9]),
                scale: rng.pick(&[1., 0.01, 100.]),
                jitter: rng.pick(&[0., 0., 1e-4, 0.3]),
            }
        }
        fn tolerance(self, rng: &mut Rng) -> f64 {
            self.scale * rng.pick(&[0., 0., 1e-3, 0.05, 0.6, 1.5])
        }
        fn at(self, rng: &mut Rng, x: usize, y: usize) -> Coord {
            let mut shift = || {
                if self.jitter == 0. || rng.chance(50) {
                    0.
                } else {
                    self.jitter * (rng.below(3) as f64 - 1.)
                }
            };
            coord! {
                x: self.offset + self.scale * (x as f64 + shift()),
                y: self.offset + self.scale * (y as f64 + shift()),
            }
        }
        fn point(self, rng: &mut Rng) -> Coord {
            let (x, y) = (rng.below(7), rng.below(7));
            self.at(rng, x, y)
        }
    }
    /// A part that may continue from `from`. Some parts are a single coordinate, zero
    /// length or closed.
    fn line_part(rng: &mut Rng, frame: Frame, from: Option<Coord>) -> LineString {
        let start = match from {
            Some(from) if rng.chance(60) => from,
            _ => frame.point(rng),
        };
        let mut coords = vec![start];
        match rng.below(14) {
            0 => {}
            1 => coords.push(start),
            _ => {
                for _ in 0..1 + rng.below(4) {
                    let last = *coords.last().unwrap();
                    let next = frame.point(rng);
                    coords.push(match rng.below(3) {
                        0 => coord! { x: last.x, y: next.y },
                        1 => coord! { x: next.x, y: last.y },
                        _ => next,
                    });
                }
                if coords.len() > 2 && rng.chance(20) {
                    coords.push(start);
                }
            }
        }
        LineString(coords)
    }
    fn line(rng: &mut Rng, frame: Frame, from: Option<Coord>) -> Geometry {
        let mut parts = vec![line_part(rng, frame, from)];
        while parts.len() < 4 && rng.chance(30) {
            let end = parts.last().and_then(|part| part.0.last().copied());
            parts.push(line_part(rng, frame, end));
        }
        if parts.len() == 1 && rng.chance(75) {
            parts.remove(0).into()
        } else {
            MultiLineString(parts).into()
        }
    }
    fn polygon(rng: &mut Rng, frame: Frame) -> Polygon {
        let (x, y) = (rng.below(6), rng.below(6));
        let (w, h) = (1 + rng.below(6 - x), 1 + rng.below(6 - y));
        let mut ring = |corners: &[(usize, usize)]| {
            let mut ring = corners
                .iter()
                .map(|&(i, j)| frame.at(rng, i, j))
                .collect::<Vec<_>>();
            ring.push(ring[0]);
            LineString(ring)
        };
        let exterior = if w == h && (x + y) % 3 == 0 {
            ring(&[(x, y), (x + w, y), (x, y + h)])
        } else {
            ring(&[(x, y), (x + w, y), (x + w, y + h), (x, y + h)])
        };
        let holes = if w > 2 && h > 2 && (x + y) % 2 == 0 {
            let (u, v) = (x + w - 1, y + h - 1);
            vec![ring(&[(x + 1, y + 1), (u, y + 1), (u, v), (x + 1, v)])]
        } else {
            Vec::new()
        };
        Polygon::new(exterior, holes)
    }
    fn feature(rng: &mut Rng, frame: Frame, kind: GeometryKind, from: Option<Coord>) -> Geometry {
        match kind {
            GeometryKind::Line => line(rng, frame, from),
            GeometryKind::Point if rng.chance(70) => Point(frame.point(rng)).into(),
            GeometryKind::Point => MultiPoint(
                (0..1 + rng.below(3))
                    .map(|_| Point(frame.point(rng)))
                    .collect(),
            )
            .into(),
            GeometryKind::Polygon if rng.chance(70) => polygon(rng, frame).into(),
            GeometryKind::Polygon => {
                MultiPolygon((0..1 + rng.below(2)).map(|_| polygon(rng, frame)).collect()).into()
            }
            _ => match rng.below(4) {
                0 => feature(rng, frame, GeometryKind::Line, from),
                1 => feature(rng, frame, GeometryKind::Point, from),
                2 => feature(rng, frame, GeometryKind::Polygon, from),
                _ => Geometry::GeometryCollection(GeometryCollection(vec![
                    Point(frame.point(rng)).into(),
                    line(rng, frame, from),
                ])),
            },
        }
    }
    fn end(geometry: &Geometry) -> Option<Coord> {
        match geometry {
            Geometry::LineString(line) => line.0.last().copied(),
            Geometry::MultiLineString(lines) => lines.0.last().and_then(|l| l.0.last().copied()),
            _ => None,
        }
    }
    /// Up to `size` features in shuffled FID order. Some duplicate or reverse an earlier
    /// feature or one of `shared`, and lines often continue from the previous feature.
    fn layer(
        rng: &mut Rng,
        frame: Frame,
        name: String,
        kind: GeometryKind,
        size: usize,
        shared: &[Geometry],
    ) -> Layer {
        let n = rng.below(size + 1);
        let mut fids = (0..n as i64).map(|i| i * 3 - 7).collect::<Vec<_>>();
        for i in (1..fids.len()).rev() {
            fids.swap(i, rng.below(i + 1));
        }
        let mut geometries: Vec<Geometry> = Vec::new();
        for _ in 0..n {
            let copies = geometries.len() + shared.len();
            let geometry = if copies > 0 && rng.chance(15) {
                let i = rng.below(copies);
                let mut copy = geometries
                    .get(i)
                    .unwrap_or_else(|| &shared[i - geometries.len()])
                    .clone();
                if let Geometry::LineString(line) = &mut copy
                    && rng.chance(50)
                {
                    line.0.reverse();
                }
                copy
            } else {
                let from = geometries.last().and_then(end);
                feature(rng, frame, kind, from)
            };
            geometries.push(geometry);
        }
        Layer::from_features(name, fids.into_iter().zip(geometries))
    }
    struct Case {
        topology: Topology,
        specs: Vec<RuleSpec>,
    }
    /// A seeded primary and secondary layer of every family, and each rule that
    /// registers on them. Only valid layers that a rule uses are added.
    fn case(seed: u64, name: &str, size: usize, rules: &[Rule]) -> Case {
        let mut rng = Rng::new(seed);
        let frame = Frame::random(&mut rng);
        let mut topology = Topology::new(frame.tolerance(&mut rng)).unwrap();
        let kinds = [
            GeometryKind::Line,
            GeometryKind::Point,
            GeometryKind::Polygon,
            GeometryKind::Any,
        ];
        let mut slots = Vec::new();
        for kind in kinds {
            let a = layer(
                &mut rng,
                frame,
                format!("{name}-{kind:?}-a"),
                kind,
                size,
                &[],
            );
            let shared = a.features().map(|(_, g)| g.clone()).collect::<Vec<_>>();
            let b = layer(
                &mut rng,
                frame,
                format!("{name}-{kind:?}-b"),
                kind,
                size,
                &shared,
            );
            slots.push((Some(a), None));
            slots.push((Some(b), None));
        }
        let slot = |kind, side| 2 * kinds.iter().position(|&k| k == kind).unwrap() + side;
        let mut add = |topology: &mut Topology, i: usize| {
            let (layer, id) = &mut slots[i];
            if let Some(layer) = layer.take() {
                *id = layer.parts().is_ok().then(|| topology.add_layer(layer));
            }
            *id
        };
        let mut specs = Vec::new();
        for &rule in rules {
            let (a, b) = rule.inputs();
            let primary = add(&mut topology, slot(a, 0));
            let other = b.map(|b| add(&mut topology, slot(b, 1)));
            if let (Some(primary), None | Some(Some(_))) = (primary, other)
                && let Ok(spec) = topology.add_rule(rule, primary, other.flatten())
            {
                specs.push(spec);
            }
        }
        Case { topology, specs }
    }
    fn line_rules() -> Vec<Rule> {
        Rule::ALL
            .iter()
            .copied()
            .filter(|rule| {
                matches!(
                    rule.inputs(),
                    (GeometryKind::Line, None | Some(GeometryKind::Line))
                ) || *rule == Rule::GeometryMustNotBeMultipart
            })
            .collect()
    }
    #[derive(Debug, Default)]
    struct Stats {
        cases: usize,
        specs: usize,
        violations: usize,
        /// Multi-kind outputs whose second kind comes from an earlier FID than the
        /// first kind's last violation, so buffering changed the order.
        reordered: usize,
        errors: usize,
    }
    fn fid(spec: RuleSpec, v: &Violation) -> Option<i64> {
        v.origin
            .filter(|origin| origin.layer == spec.layer)
            .map(|origin| origin.fid)
    }
    /// Compare new emission with the reference for every spec of one case, on cold
    /// caches and on every batch size.
    fn differential(seed: u64, size: usize, rules: &[Rule], stats: &mut Stats) {
        let cold = case(seed, "diff", size, rules);
        let warm = case(seed, "diff", size, rules);
        stats.cases += 1;
        for (i, (&spec, &cold_spec)) in warm.specs.iter().zip(&cold.specs).enumerate() {
            let new = format!("{:?}", emission(&cold.topology, cold_spec, None));
            let (expected, result) = reference_emission(&warm.topology, spec);
            let debug = format!("{:?}", (&expected, &result));
            assert_eq!(new, debug, "seed {seed}, {}", spec.rule);
            if per_feature(spec.rule) {
                // Batches of one to three features, so most cases span several.
                let batch = 1 + (seed as usize + i) % 3;
                let batched = format!("{:?}", emission(&warm.topology, spec, Some(batch)));
                assert_eq!(batched, debug, "seed {seed}, {}, batch {batch}", spec.rule);
            }
            if matches!(
                spec.rule,
                Rule::LineMustNotSelfOverlap | Rule::LineMustNotSelfIntersect
            ) {
                // Whole-layer self checks keep layer order across parallel features.
                let mut snapshot = warm.topology.snapshot();
                let sequential = warm
                    .topology
                    .layer(spec.layer)
                    .unwrap()
                    .features()
                    .map(|(fid, _)| {
                        snapshot.select_feature(spec.layer, fid)?;
                        run(&snapshot, spec)
                    })
                    .collect::<Result<Vec<_>, Error>>()
                    .map(|features| features.concat());
                assert_eq!(
                    format!("{:?}", eager(&warm.topology, spec)),
                    format!("{sequential:?}"),
                    "seed {seed}, {}",
                    spec.rule
                );
            }
            stats.specs += 1;
            match result.map(|()| expected) {
                Ok(violations) => {
                    stats.violations += violations.len();
                    let first = violations.first().map(|v| v.kind);
                    let (first, later): (Vec<_>, Vec<_>) =
                        violations.iter().partition(|v| Some(v.kind) == first);
                    let last_first = first.iter().filter_map(|v| fid(spec, v)).max();
                    let first_later = later.iter().filter_map(|v| fid(spec, v)).min();
                    if per_feature(spec.rule)
                        && first_later.zip(last_first).is_some_and(|(a, b)| a < b)
                    {
                        stats.reordered += 1;
                    }
                }
                Err(_) => stats.errors += 1,
            }
        }
    }

    #[test]
    fn feature_batches_match_per_kind_passes_for_every_line_rule() {
        let rules = line_rules();
        assert_eq!(rules.len(), 11);
        let mut stats = Stats::default();
        for seed in 0..1000 {
            differential(seed, 8, &rules, &mut stats);
        }
        assert!(stats.specs > 10_000, "{stats:?}");
        assert!(stats.violations > 40_000, "{stats:?}");
        assert!(stats.reordered > 1_200, "{stats:?}");
    }

    #[test]
    fn feature_batches_match_per_kind_passes_for_every_other_rule() {
        let lines = line_rules();
        let rules = Rule::ALL
            .iter()
            .copied()
            .filter(|rule| !lines.contains(rule))
            .collect::<Vec<_>>();
        let mut stats = Stats::default();
        for seed in 0..250 {
            differential(seed, 6, &rules, &mut stats);
        }
        assert!(stats.specs > 5_000, "{stats:?}");
        assert!(stats.violations > 9_000, "{stats:?}");
    }

    #[test]
    fn output_is_the_same_on_any_number_of_threads() {
        let pools = [1, 2, 4].map(|n| {
            rayon::ThreadPoolBuilder::new()
                .num_threads(n)
                .build()
                .unwrap()
        });
        let output = |seed| {
            let case = case(seed, "threads", 8, Rule::ALL);
            let mut streamed = Vec::new();
            let summary = case.topology.validate_into(|spec, v| {
                streamed.push((spec, v));
                Ok::<_, Infallible>(())
            });
            let eager = case.topology.validate();
            (streamed, format!("{summary:?}"), format!("{eager:?}"))
        };
        for seed in 0..60 {
            let expected = output(seed);
            let old = case(seed, "threads", 8, Rule::ALL);
            // Validation streams each rule's passes and stops at the first error.
            let mut passes = Vec::new();
            for &spec in &old.specs {
                let (violations, result) = reference_emission(&old.topology, spec);
                passes.extend(violations.into_iter().map(|v| (spec, v)));
                if result.is_err() {
                    break;
                }
            }
            assert!(expected.0 == passes, "seed {seed}");
            // The 4-thread pool runs twice, to catch output that varies between runs.
            for pool in [&pools[0], &pools[1], &pools[2], &pools[2]] {
                assert!(
                    pool.install(|| output(seed)) == expected,
                    "seed {seed}, {} threads",
                    pool.current_num_threads()
                );
            }
            assert!(output(seed) == expected, "seed {seed}, repeated");
        }
    }

    /// Fail unless every worker of `pool` is free again within a minute.
    fn assert_idle(pool: &rayon::ThreadPool) {
        let (done, finished) = mpsc::channel();
        pool.spawn_broadcast(move |_| done.send(()).unwrap());
        for _ in 0..pool.current_num_threads() {
            finished
                .recv_timeout(Duration::from_secs(60))
                .expect("a worker is stuck");
        }
    }

    #[test]
    fn cancelling_as_a_worker_starts_a_feature_stops_after_a_reference_prefix() {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(4)
            .build()
            .unwrap();
        let rules = Rule::ALL
            .iter()
            .copied()
            .filter(|&rule| per_feature(rule))
            .collect::<Vec<_>>();
        let mut cancelled = 0;
        for seed in 0..400 {
            let mut rng = Rng::new(seed ^ 0xCA9CE1);
            let rule = rng.pick(&rules);
            let case = case(seed, &format!("cancel{seed}"), 12, &[rule]);
            let Some(&spec) = case.specs.first() else {
                continue;
            };
            let fids = case
                .topology
                .layer(spec.layer)
                .unwrap()
                .features()
                .map(|(fid, _)| fid)
                .collect::<Vec<_>>();
            let Ok(expected) = reference(&case.topology, spec) else {
                continue;
            };
            if fids.is_empty() {
                continue;
            }
            let token = CancellationToken::default();
            let name = on(
                &case.topology,
                spec,
                rng.pick(&fids),
                Action::Cancel(token.clone()),
            );
            let mut emitted = Vec::new();
            let result = pool.install(|| {
                case.topology.validate_into_cancellable(token, |_, v| {
                    emitted.push(v);
                    Ok::<_, Infallible>(())
                })
            });
            clear(&name);
            assert!(
                matches!(result, Err(SinkError::Core(Error::Cancelled))),
                "seed {seed}, {rule}: {result:?}"
            );
            assert!(emitted.len() <= expected.len(), "seed {seed}, {rule}");
            assert_eq!(emitted, expected[..emitted.len()], "seed {seed}, {rule}");
            // Unwinding leaves caches unset; the next run rebuilds them.
            assert_eq!(
                format!(
                    "{:?}",
                    pool.install(|| streamed(&case.topology, spec, None))
                ),
                format!("{:?}", Ok::<_, Error>(expected)),
                "seed {seed}, {rule}"
            );
            cancelled += 1;
        }
        assert!(cancelled > 250, "{cancelled}");
        assert_idle(&pool);
    }

    /// Enough line features that each parallel rule runs for a while.
    fn busy(rules: &[Rule]) -> Case {
        let mut rng = Rng::new(7);
        let frame = Frame {
            offset: 1e6,
            scale: 1.,
            jitter: 0.,
        };
        let mut topology = Topology::new(0.001).unwrap();
        let mut features = Vec::new();
        for i in 0..160 {
            let mut geometry = feature(&mut rng, frame, GeometryKind::Line, None);
            let shift = coord! { x: (i % 16) as f64 * 4., y: (i / 16) as f64 * 4. };
            geo::MapCoordsInPlace::map_coords_in_place(&mut geometry, |c| c + shift);
            features.push((i, geometry));
        }
        let layer = topology.add_layer(Layer::from_features("busy", features.clone()));
        let other = topology.add_layer(Layer::from_features("busy-other", features));
        let specs = rules
            .iter()
            .map(|&rule| {
                let other = rule.inputs().1.map(|_| other);
                topology.add_rule(rule, layer, other).unwrap()
            })
            .collect();
        Case { topology, specs }
    }

    #[test]
    fn cancelling_at_random_times_stops_promptly_and_frees_every_worker() {
        let rules = [
            Rule::LineMustNotIntersectOrTouchInterior,
            Rule::LineMustNotSelfIntersect,
            Rule::LineMustNotIntersectWith,
            Rule::LineMustNotOverlap,
        ];
        let pool = Arc::new(
            rayon::ThreadPoolBuilder::new()
                .num_threads(4)
                .build()
                .unwrap(),
        );
        let warm = busy(&rules);
        let expected = warm
            .specs
            .iter()
            .map(|&spec| reference(&warm.topology, spec))
            .collect::<Result<Vec<_>, Error>>()
            .unwrap()
            .concat();
        assert!(!expected.is_empty());
        // An uncancelled run on the same path and pool, with cold caches as below.
        let cold = busy(&rules);
        let started = Instant::now();
        let run = pool.install(|| {
            let token = CancellationToken::default();
            cold.topology
                .validate_into_cancellable(token, |_, _| Ok::<_, Infallible>(()))
        });
        assert!(run.is_ok());
        let full = started.elapsed();
        let mut rng = Rng::new(11);
        let (mut cancelled, mut slowest) = (0, Duration::ZERO);
        for seed in 0..40 {
            let case = busy(&rules);
            let token = CancellationToken::default();
            let (send, receive) = mpsc::channel();
            let (worker_pool, worker_token) = (Arc::clone(&pool), token.clone());
            let worker = std::thread::spawn(move || {
                let mut emitted = Vec::new();
                let result = worker_pool.install(|| {
                    case.topology
                        .validate_into_cancellable(worker_token, |_, v| {
                            emitted.push(v);
                            Ok::<_, Infallible>(())
                        })
                        .map(|_| ())
                });
                send.send((result, emitted)).unwrap();
            });
            std::thread::sleep(full.mul_f64(rng.below(1000) as f64 / 800.));
            let at = Instant::now();
            token.cancel();
            let (result, emitted) = receive
                .recv_timeout(Duration::from_secs(30))
                .expect("cancellation stops the run");
            slowest = slowest.max(at.elapsed());
            worker.join().unwrap();
            match result {
                Ok(()) => assert_eq!(emitted, expected, "seed {seed}"),
                Err(SinkError::Core(Error::Cancelled)) => {
                    cancelled += 1;
                    assert_eq!(emitted, expected[..emitted.len()], "seed {seed}");
                }
                Err(error) => panic!("seed {seed}: {error:?}"),
            }
        }
        // Most delays fall within the run; machine load moves the share either way.
        assert!(cancelled > 10, "{cancelled}");
        assert!(slowest < Duration::from_secs(5), "{slowest:?}");
        assert_idle(&pool);
        let rerun = busy(&rules);
        assert_eq!(
            pool.install(|| {
                rerun
                    .specs
                    .iter()
                    .map(|&spec| streamed(&rerun.topology, spec, None))
                    .collect::<Result<Vec<_>, Error>>()
            })
            .unwrap()
            .concat(),
            expected
        );
    }

    #[test]
    fn the_first_feature_runs_alone_before_the_parallel_batches() {
        // It builds the caches every feature shares, so workers need not wait for them.
        let mut t = Topology::new(0.).unwrap();
        let lines = (0..24).map(|fid| {
            let x = fid as f64;
            let line = if fid % 2 == 0 {
                LineString::from(vec![(x, -1.), (x, 13.)])
            } else {
                LineString::from(vec![(-1., x / 2.), (25., x / 2.)])
            };
            (fid, line.into())
        });
        let layer = t.add_layer(Layer::from_features("first-alone", lines));
        let spec = t
            .add_rule(Rule::LineMustNotIntersectOrTouchInterior, layer, None)
            .unwrap();
        let expected = reference(&t, spec).unwrap();
        assert!(!expected.is_empty());
        let hold = Duration::from_millis(50);
        let starts = Arc::new(Mutex::new(Vec::new()));
        let name = on(&t, spec, 0, Action::Hold(hold));
        for fid in 1..24 {
            on(&t, spec, fid, Action::Note(Arc::clone(&starts)));
        }
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(4)
            .build()
            .unwrap();
        let started = Instant::now();
        let streamed = pool.install(|| streamed(&t, spec, None));
        clear(&name);
        assert_eq!(streamed.unwrap(), expected);
        let starts = starts.lock().unwrap_or_else(PoisonError::into_inner);
        assert_eq!(starts.len(), 23);
        assert!(starts.iter().all(|&at| at >= started + hold));
    }

    #[test]
    fn worker_panics_reach_the_caller_with_their_payload() {
        let pools = [1, 4].map(|n| {
            rayon::ThreadPoolBuilder::new()
                .num_threads(n)
                .build()
                .unwrap()
        });
        let rules = [
            Rule::LineMustNotIntersect,
            Rule::LineMustNotSelfIntersect,
            Rule::LineMustNotIntersectOrTouchInteriorWith,
            Rule::PolygonMustNotOverlap,
            Rule::LineMustBeInside,
            Rule::PointMustBeCoveredByLines,
        ];
        let mut panicked = 0;
        for seed in 0..60 {
            let mut rng = Rng::new(seed ^ 0x9A71C);
            let rule = rng.pick(&rules);
            let case = case(seed, &format!("panic{seed}"), 10, &[rule]);
            let Some(&spec) = case.specs.first() else {
                continue;
            };
            let fids = case
                .topology
                .layer(spec.layer)
                .unwrap()
                .features()
                .map(|(fid, _)| fid)
                .collect::<Vec<_>>();
            if fids.is_empty() {
                continue;
            }
            let expected = format!("{:?}", reference(&case.topology, spec));
            let name = on(
                &case.topology,
                spec,
                rng.pick(&fids),
                Action::Panic("feature worker failed"),
            );
            for pool in &pools {
                for cancellable in [false, true] {
                    let payload = std::panic::catch_unwind(AssertUnwindSafe(|| {
                        pool.install(|| {
                            let sink = |_, _| Ok::<_, Infallible>(());
                            if cancellable {
                                case.topology
                                    .validate_into_cancellable(CancellationToken::default(), sink)
                            } else {
                                case.topology.validate_into(sink)
                            }
                        })
                    }))
                    .expect_err("the panic propagates");
                    assert_eq!(
                        payload.downcast_ref::<&str>(),
                        Some(&"feature worker failed"),
                        "seed {seed}, {rule}"
                    );
                }
            }
            clear(&name);
            for pool in &pools {
                assert_eq!(
                    format!(
                        "{:?}",
                        pool.install(|| streamed(&case.topology, spec, None))
                    ),
                    expected,
                    "seed {seed}, {rule}"
                );
                assert_idle(pool);
            }
            panicked += 1;
        }
        assert!(panicked > 40, "{panicked}");
    }

    #[test]
    fn a_failing_feature_stops_emission_where_the_per_kind_passes_did() {
        let rules = Rule::ALL
            .iter()
            .copied()
            .filter(|&rule| per_feature(rule))
            .collect::<Vec<_>>();
        let (mut failed, mut after_output) = (0, 0);
        for seed in 0..300 {
            let mut rng = Rng::new(seed ^ 0xFA11);
            let rule = if seed % 2 == 0 {
                rng.pick(&[
                    Rule::LineMustNotIntersectOrTouchInterior,
                    Rule::LineMustNotIntersectOrTouchInteriorWith,
                    Rule::LineMustNotSelfIntersect,
                    Rule::LineMustNotIntersect,
                ])
            } else {
                rng.pick(&rules)
            };
            let case = case(seed, &format!("fail{seed}"), 12, &[rule]);
            let Some(&spec) = case.specs.first() else {
                continue;
            };
            let fids = case
                .topology
                .layer(spec.layer)
                .unwrap()
                .features()
                .map(|(fid, _)| fid)
                .collect::<Vec<_>>();
            if fids.is_empty() {
                continue;
            }
            let name = on(&case.topology, spec, rng.pick(&fids), Action::Fail);
            let (expected, result) = reference_emission(&case.topology, spec);
            let debug = format!("{:?}", (&expected, &result));
            for batch in [Some(1), Some(2), Some(3), Some(5), None] {
                assert_eq!(
                    format!("{:?}", emission(&case.topology, spec, batch)),
                    debug,
                    "seed {seed}, {rule}, batch {batch:?}"
                );
            }
            let mut emitted = Vec::new();
            let streamed = case.topology.validate_into(|_, v| {
                emitted.push(v);
                Ok::<_, Infallible>(())
            });
            clear(&name);
            assert!(matches!(result, Err(Error::Precision { .. })), "{result:?}");
            assert!(
                matches!(streamed, Err(SinkError::Core(Error::Precision { .. }))),
                "{streamed:?}"
            );
            assert_eq!(emitted, expected, "seed {seed}, {rule}");
            failed += 1;
            after_output += usize::from(!expected.is_empty());
        }
        assert!(failed > 250, "{failed}");
        assert!(after_output > 50, "{after_output}");
    }

    #[test]
    fn feature_emission_failures_name_their_layer_and_feature() {
        // The union of these overlapping slivers cannot be rounded.
        let x = 1f64.next_up();
        let rect = Rect::new(coord! {x: 1., y: 0.}, coord! {x: x, y: 1.}).to_polygon();
        let triangle = Polygon::new(
            LineString::from(vec![(1., -1.), (x, -1.), (1., 1.), (1., -1.)]),
            vec![],
        );
        let area = crate::engine::arrangement::overlay([&rect, &triangle], [], false);
        let feature = crate::engine::cross::Feature {
            reference: PartRef { fid: 5, part: None },
            geometry: Geometry::GeometryCollection(GeometryCollection(vec![])),
            points: Vec::new(),
            locations: Vec::new(),
            area: Some(area),
            non_areal: Vec::new(),
        };
        let mut topology = Topology::new(0.).unwrap();
        let polygons = topology.add_layer(Layer::from_geometries("parcels", []));
        let points = topology.add_layer(Layer::from_geometries("wells", []));
        let spec = topology
            .add_rule(Rule::PolygonMustContainPoint, polygons, Some(points))
            .unwrap();
        let error = feature_geometry(&topology, spec, polygons, &feature).unwrap_err();
        assert!(
            matches!(
                &error,
                Error::Precision { operation, rule: Some(Rule::PolygonMustContainPoint), context, .. }
                    if operation == "polygon feature emission" && context == ", layer parcels, FID 5"
            ),
            "{error:?}"
        );
    }
}
