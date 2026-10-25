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
use crate::{
    Error, FeatureRef, Rule, RuleSpec, Topology, Violation, ViolationKind,
    engine::parallel::{self, Workers},
};
use boundary::Boundary;
use coverage::Target;
use geo::{Coord, Geometry, LineString};
use line::LineCheck;
use line_polygon::LinePolygon;
use point_polygon::{PointCount, PointTest};
use rayon::prelude::*;
use std::{
    sync::{
        OnceLock,
        atomic::{self, AtomicUsize},
    },
    time::{Duration, Instant},
};

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
        per_feature_into(t, spec, FEATURES_PER_THREAD, &mut sink)
    }
}
/// Features per thread in one parallel batch of [`per_feature_into`]. Output waits
/// for a whole batch, so batches stay small enough to stream early and to bound the
/// violations held at once, and large enough to balance uneven features.
const FEATURES_PER_THREAD: usize = 8;
/// Most features per thread in a batch of [`per_feature_into`] that grew after
/// [quick](QUICK_PER_HELPER) ones.
const MOST_PER_THREAD: usize = 256;
/// A parallel batch of [`per_feature_into`] that takes less than this per helper
/// thread doubles the features per thread of the next one, so that starting its
/// helpers, some 20 µs each, stays a small part of it. One that takes four times as
/// long starts over.
const QUICK_PER_HELPER: Duration = Duration::from_micros(500);
/// A batch of [`per_feature_into`] that takes at least this long hands the next one to
/// a thread apart from the calling thread, which holds the caches the first feature
/// built. The scratch of slow features is faster to reach there, which outweighs the
/// 10 µs or so each handover costs.
const SLOW_BATCH: Duration = Duration::from_millis(1);
/// Helper threads in use by the feature batches of every validation in the process.
static HELPERS: AtomicUsize = AtomicUsize::new(0);
/// Helper threads of one feature batch, returned when it ends.
struct Helpers {
    threads: usize,
    shared: usize,
    budget: &'static AtomicUsize,
}
impl Helpers {
    /// Up to `want` of the helpers free now. All validations together use one helper
    /// per CPU but one, and a batch never waits for them: without any it runs on the
    /// calling thread alone.
    fn take(want: usize) -> Self {
        #[cfg(test)]
        if let Some(free) = tests::free() {
            return Self {
                threads: want.min(free),
                shared: 0,
                budget: &HELPERS,
            };
        }
        // Tests that fix the thread count get it whatever runs beside them.
        #[cfg(test)]
        if tests::threads().is_some() || Workers::forced_here().is_some() {
            return Self {
                threads: want,
                shared: 0,
                budget: &HELPERS,
            };
        }
        static LIMIT: OnceLock<usize> = OnceLock::new();
        let limit = *LIMIT.get_or_init(|| {
            std::thread::available_parallelism().map_or(1, std::num::NonZero::get) - 1
        });
        Self::take_from(&HELPERS, limit, want)
    }
    /// Up to `want` of `limit` helpers, `budget` counting those in use.
    fn take_from(budget: &'static AtomicUsize, limit: usize, want: usize) -> Self {
        let mut used = budget.load(atomic::Ordering::Relaxed);
        loop {
            let threads = want.min(limit.saturating_sub(used));
            if threads == 0 {
                return Self {
                    threads,
                    shared: 0,
                    budget,
                };
            }
            match budget.compare_exchange_weak(
                used,
                used + threads,
                atomic::Ordering::Relaxed,
                atomic::Ordering::Relaxed,
            ) {
                Ok(_) => {
                    return Self {
                        threads,
                        shared: threads,
                        budget,
                    };
                }
                Err(now) => used = now,
            }
        }
    }
}
impl Drop for Helpers {
    fn drop(&mut self) {
        self.budget
            .fetch_sub(self.shared, atomic::Ordering::Relaxed);
    }
}
/// Whether this thread runs one feature of [`per_feature_into`]. Its steps then run on
/// the threads the feature holds, not on rayon: a rayon job can wait behind other
/// validations' jobs, which do not stop when this validation is cancelled.
pub(super) fn in_feature_task() -> bool {
    parallel::cap().is_some()
}
/// `task` of each item, results in order. A feature task runs them on the threads it
/// holds, other callers on rayon.
pub(super) fn each<I: Send, R: Send>(items: Vec<I>, task: impl Fn(I) -> R + Sync + Send) -> Vec<R> {
    if in_feature_task() {
        Workers::all().each(items, task)
    } else {
        items.into_par_iter().map(task).collect()
    }
}
/// The items of `task(i)` for each `i` in `0..len`, in order. Runs like [`each`], in
/// ranges of indices.
pub(super) fn flat_map<I: IntoIterator<Item: Send>>(
    len: usize,
    task: impl Fn(usize) -> I + Sync + Send,
) -> Vec<I::Item> {
    if in_feature_task() {
        Workers::all()
            .chunks(len, |range| range.flat_map(&task).collect::<Vec<_>>())
            .into_iter()
            .flatten()
            .collect()
    } else {
        (0..len).into_par_iter().flat_map_iter(task).collect()
    }
}
/// Candidate pairs held at once.
const PAIRS: usize = 1 << 14;
/// Candidate pairs a thread of a feature task takes at a time.
const PAIR_TASK: usize = 64;
/// The results `task` returns for candidate pairs, in pair order, run like [`each`]
/// on [`PAIRS`] pairs at a time. The threads of a feature task take [`PAIR_TASK`]
/// pairs at a time. On rayon the next pairs are found meanwhile, as threads taking
/// pairs one at a time from the search wait for each other.
pub(super) fn each_pair<R: Send>(
    mut pairs: impl Iterator<Item = (usize, usize)> + Send,
    task: impl Fn((usize, usize)) -> Option<R> + Sync + Send,
) -> Vec<R> {
    let mut results = Vec::new();
    let mut next = || pairs.by_ref().take(PAIRS).collect::<Vec<_>>();
    if in_feature_task() {
        let workers = Workers::all();
        loop {
            let chunk = next();
            if chunk.is_empty() {
                return results;
            }
            if chunk.len() <= PAIR_TASK || workers.alone() {
                results.extend(chunk.into_iter().filter_map(&task));
            } else {
                results.extend(pair_tasks(workers, &chunk, &task).into_iter().flatten());
            }
        }
    }
    let mut chunk = next();
    while !chunk.is_empty() {
        let found;
        (found, chunk) = rayon::join(
            || {
                chunk
                    .par_iter()
                    .filter_map(|&pair| task(pair))
                    .collect::<Vec<_>>()
            },
            &mut next,
        );
        results.extend(found);
    }
    results
}
/// The results of `task` for `pairs`, [`PAIR_TASK`] pairs to a task, in pair order.
fn pair_tasks<R: Send>(
    workers: Workers,
    pairs: &[(usize, usize)],
    task: &(impl Fn((usize, usize)) -> Option<R> + Sync),
) -> Vec<Vec<R>> {
    workers.run(pairs.len().div_ceil(PAIR_TASK), |k| {
        let pairs = pairs.iter().skip(k * PAIR_TASK).take(PAIR_TASK);
        pairs.filter_map(|&pair| task(pair)).collect()
    })
}
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
/// complete feature-pair report is retained. Batches of `per_thread` features per
/// thread run in parallel and each feature runs once: its first kind is emitted with
/// its batch, and later kinds wait until every feature has emitted the first. Caches
/// every feature shares, such as the line graph, build on every thread of the
/// validation, whichever feature needs them first and whatever helpers are free. The
/// first feature runs alone, so the others need not wait for them. Features of a batch
/// smaller than its threads split their own steps over the spare ones, and threads out
/// of features join the steps of those still running. Batches run on the calling
/// thread, or after a [slow](SLOW_BATCH) one on a thread apart, and on helpers of their
/// own, never on a shared pool, so cancelling stops them as soon as their features
/// check, whatever else runs in the process.
fn per_feature_into<E>(
    t: &Topology,
    spec: RuleSpec,
    per_thread: usize,
    sink: &mut impl FnMut(Violation) -> Result<(), E>,
) -> Result<(), crate::SinkError<E>> {
    let mut fids = t
        .layer(spec.layer)?
        .features()
        .map(|(fid, _)| fid)
        .collect::<Vec<_>>();
    fids.sort_unstable();
    let kinds = feature_kinds(spec.rule);
    // Holding later kinds saves a rerun per kind; a million held added about 10% to peak.
    let mut later = kinds.iter().skip(1).map(|_| Vec::new()).collect::<Vec<_>>();
    let all = Workers::all().threads();
    #[cfg(test)]
    let all = tests::threads().unwrap_or(all).max(1);
    let feature = |snapshot: &mut Topology, fid: i64, threads: usize| {
        // The selection checks this topology's token on whichever thread runs it.
        t.cancellable(|| {
            parallel::task(threads, all, || {
                #[cfg(test)]
                tests::visit(t, spec, fid)?;
                snapshot.select_feature(spec.layer, fid)?;
                let mut violations = run(snapshot, spec)?;
                sort(&mut violations);
                Ok::<_, Error>(violations)
            })
        })
    };
    // A snapshot per thread, as cheap features slow down sharing its making.
    let batch = |(fids, threads, steps): (&[i64], usize, usize)| {
        Workers::with_threads(threads).run_sharing(
            fids.len(),
            steps,
            || t.snapshot(),
            |snapshot, i| feature(snapshot, fids[i], steps),
        )
    };
    #[cfg(not(test))]
    let (slow, quick) = (SLOW_BATCH, QUICK_PER_HELPER);
    #[cfg(test)]
    let (slow, quick) = (tests::slow_batch(), tests::quick_per_helper());
    parallel::apart(batch, |apart| {
        let mut rest = &fids[..];
        let (mut first, mut after_slow) = (true, false);
        let mut grown = per_thread.max(1);
        while !rest.is_empty() {
            crate::cancel::check();
            let helpers = Helpers::take(all - 1);
            let threads = 1 + helpers.threads;
            let size = if first { 1 } else { threads * grown };
            let (fids, tail) = rest.split_at(size.min(rest.len()));
            // Fewer features than threads split their own steps over the rest.
            let steps = (threads / fids.len()).max(1);
            let started = Instant::now();
            let results = if after_slow {
                apart((fids, threads, steps))
            } else {
                batch((fids, threads, steps))
            };
            let took = started.elapsed();
            after_slow = took >= slow;
            if !first && helpers.threads > 0 {
                let helpers = u32::try_from(helpers.threads).unwrap_or(u32::MAX);
                grown = grow(grown, per_thread, took, quick.saturating_mul(helpers));
            }
            (rest, first) = (tail, false);
            drop(helpers);
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
        Ok::<_, crate::SinkError<E>>(())
    })?;
    later.into_iter().flatten().try_for_each(|v| send(v, sink))
}
/// Features per thread for the batch after one of `per` that took `took`: twice as
/// many after one [quicker](QUICK_PER_HELPER) than `quick`, and `least` again after
/// one four times as slow.
fn grow(per: usize, least: usize, took: Duration, quick: Duration) -> usize {
    let least = least.max(1);
    if took < quick {
        per.saturating_mul(2).min(MOST_PER_THREAD.max(least))
    } else if took >= quick.saturating_mul(4) {
        least
    } else {
        per
    }
}
fn emit<E>(
    mut violations: Vec<Violation>,
    sink: &mut impl FnMut(Violation) -> Result<(), E>,
) -> Result<(), crate::SinkError<E>> {
    sort(&mut violations);
    violations.into_iter().try_for_each(|v| send(v, sink))
}
/// Stable sort into emission order. Cancelling unwinds with every violation still
/// in the slice.
fn sort(violations: &mut [Violation]) {
    let key = |v: &Violation| {
        (
            v.kind,
            v.origin.map(FeatureRef::feature),
            v.destination.map(FeatureRef::feature),
        )
    };
    let mut compared = 0;
    violations.sort_by(|a, b| {
        crate::cancel::every(compared, "violations.sort");
        compared += 1;
        key(a).cmp(&key(b))
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
        cell::Cell,
        collections::HashSet,
        convert::Infallible,
        panic::AssertUnwindSafe,
        sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError, mpsc},
        thread::ThreadId,
        time::{Duration, Instant},
    };

    thread_local! { static THREADS: Cell<Option<usize>> = const { Cell::new(None) }; }
    /// Threads for feature batches started on this thread, whatever else is running.
    pub(super) fn threads() -> Option<usize> {
        THREADS.with(Cell::get)
    }
    /// Run `f` with feature batches on `threads` threads where they have the features,
    /// outside the helper budget. Busy tests use the budget instead, so they leave CPUs
    /// to the tests running beside them.
    fn with_threads<T>(threads: usize, f: impl FnOnce() -> T) -> T {
        struct Restore(Option<usize>);
        impl Drop for Restore {
            fn drop(&mut self) {
                THREADS.with(|c| c.set(self.0));
            }
        }
        let _restore = Restore(THREADS.with(|c| c.replace(Some(threads))));
        f()
    }
    thread_local! { static FREE: Cell<Option<usize>> = const { Cell::new(None) }; }
    /// Helpers free for feature batches started on this thread, whatever the budget holds.
    pub(super) fn free() -> Option<usize> {
        FREE.with(Cell::get)
    }
    /// Run `f` as if `free` helpers were left in the budget.
    fn with_free<T>(free: usize, f: impl FnOnce() -> T) -> T {
        struct Restore(Option<usize>);
        impl Drop for Restore {
            fn drop(&mut self) {
                FREE.with(|c| c.set(self.0));
            }
        }
        let _restore = Restore(FREE.with(|c| c.replace(Some(free))));
        f()
    }
    thread_local! { static SLOW: Cell<Duration> = const { Cell::new(Duration::ZERO) }; }
    /// How long a batch started on this thread takes to count as slow. Every batch does
    /// in tests, so the batches after the first one run apart.
    pub(super) fn slow_batch() -> Duration {
        SLOW.with(Cell::get)
    }
    /// Run `f` with batches slow from `slow` on.
    fn with_slow_batch<T>(slow: Duration, f: impl FnOnce() -> T) -> T {
        struct Restore(Duration);
        impl Drop for Restore {
            fn drop(&mut self) {
                SLOW.with(|c| c.set(self.0));
            }
        }
        let _restore = Restore(SLOW.with(|c| c.replace(slow)));
        f()
    }
    thread_local! { static QUICK: Cell<Duration> = const { Cell::new(Duration::ZERO) }; }
    /// How long a parallel batch started on this thread may take per helper to grow
    /// the next. None does in tests, so batches keep their size.
    pub(super) fn quick_per_helper() -> Duration {
        QUICK.with(Cell::get)
    }
    /// Run `f` with parallel batches quick below `quick` per helper.
    fn with_quick_per_helper<T>(quick: Duration, f: impl FnOnce() -> T) -> T {
        struct Restore(Duration);
        impl Drop for Restore {
            fn drop(&mut self) {
                QUICK.with(|c| c.set(self.0));
            }
        }
        let _restore = Restore(QUICK.with(|c| c.replace(quick)));
        f()
    }

    /// Longest wait for another thread in a test.
    const PATIENCE: Duration = Duration::from_secs(60);
    /// Threads that reached a point, and whether they may pass it.
    #[derive(Debug, Default)]
    struct Gate {
        state: Mutex<Passage>,
        changed: Condvar,
    }
    #[derive(Debug, Default)]
    struct Passage {
        arrivals: usize,
        threads: HashSet<ThreadId>,
        open: bool,
    }
    impl Gate {
        fn state(&self) -> MutexGuard<'_, Passage> {
            self.state.lock().unwrap_or_else(PoisonError::into_inner)
        }
        /// Count this thread in and return how many arrived before it.
        fn arrive(&self) -> usize {
            let mut state = self.state();
            state.arrivals += 1;
            state.threads.insert(std::thread::current().id());
            self.changed.notify_all();
            state.arrivals - 1
        }
        fn open(&self) {
            self.state().open = true;
            self.changed.notify_all();
        }
        /// Block until `ready` holds, for at most [`PATIENCE`].
        fn wait(&self, ready: impl Fn(&Passage) -> bool) -> MutexGuard<'_, Passage> {
            self.wait_for(PATIENCE, ready)
        }
        /// Block until `ready` holds, for at most `timeout`.
        fn wait_for(
            &self,
            timeout: Duration,
            ready: impl Fn(&Passage) -> bool,
        ) -> MutexGuard<'_, Passage> {
            self.changed
                .wait_timeout_while(self.state(), timeout, |state| !ready(state))
                .unwrap_or_else(PoisonError::into_inner)
                .0
        }
    }

    #[derive(Debug, Clone)]
    enum Action {
        Cancel(CancellationToken),
        Fail,
        Panic(&'static str),
        Hold(Duration),
        Note(Arc<Mutex<Vec<Instant>>>),
        /// Note the feature and the thread it runs on.
        Thread(Arc<Mutex<Vec<(i64, ThreadId)>>>),
        /// Arrive at the gate and go on, but wait until it opens when this many
        /// arrived before.
        Arrive(Arc<Gate>, usize),
        /// Arrive at the gate and wait until it opens, then check for cancellation as
        /// rule code does and, if that does not unwind, sleep before returning.
        Block(Arc<Gate>, Duration),
        /// Wait until this many threads arrived at the gate.
        Await(Arc<Gate>, usize),
    }
    /// How long a feature blocked at a gate keeps running when cancelling misses it.
    const GRACE: Duration = Duration::from_secs(1);
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
            Some(Action::Thread(threads)) => threads
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push((fid, std::thread::current().id())),
            Some(Action::Arrive(gate, wait)) => {
                let earlier = gate.arrive();
                if earlier == wait {
                    drop(gate.wait(|state| state.open));
                }
            }
            Some(Action::Block(gate, then)) => {
                gate.arrive();
                drop(gate.wait(|state| state.open));
                crate::cancel::check();
                std::thread::sleep(then);
            }
            Some(Action::Await(gate, arrivals)) => {
                drop(gate.wait(|state| state.arrivals >= arrivals));
            }
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
                with_threads(2, || {
                    case.topology.validate_into_cancellable(token, |_, v| {
                        emitted.push(v);
                        Ok::<_, Infallible>(())
                    })
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
    fn cancelling_at_random_points_stops_promptly_and_frees_every_worker() {
        let rules = [
            Rule::LineMustNotIntersectOrTouchInterior,
            Rule::LineMustNotSelfIntersect,
            Rule::LineMustNotIntersectWith,
            Rule::LineMustNotOverlap,
        ];
        let pools = [4, 2, 1].map(|n| {
            Arc::new(
                rayon::ThreadPoolBuilder::new()
                    .num_threads(n)
                    .build()
                    .unwrap(),
            )
        });
        let warm = busy(&rules);
        let expected = warm
            .specs
            .iter()
            .map(|&spec| reference(&warm.topology, spec))
            .collect::<Result<Vec<_>, Error>>()
            .unwrap()
            .concat();
        assert!(!expected.is_empty());
        let features = warm.topology.layer(warm.specs[0].layer).unwrap().len();
        let mut rng = Rng::new(11);
        let (mut cancelled, mut slowest) = (0, Duration::ZERO);
        for seed in 0..40 {
            let pool = &pools[seed % pools.len()];
            let case = busy(&rules);
            // Cancel up to 5 ms after a seeded feature start. Every rule starts each
            // feature once, so starts count progress on any machine. Every fourth run
            // picks a rule's first feature, which builds the caches the rule shares.
            // That feature waits until this thread is awake to time the cancel, and a
            // timer that fires late still cancels within a rule's worth of starts.
            let at = if seed % 4 == 0 {
                rng.below(rules.len()) * features
            } else {
                rng.below(rules.len() * features)
            };
            let delay = Duration::from_micros(rng.below(5000) as u64);
            let starts = Arc::new(Gate::default());
            let note = || Action::Arrive(Arc::clone(&starts), at);
            let name = on(&case.topology, case.specs[0], 0, note());
            for fid in 1..features as i64 {
                on(&case.topology, case.specs[0], fid, note());
            }
            let token = CancellationToken::default();
            let (send, receive) = mpsc::channel();
            let (worker_pool, worker_token) = (Arc::clone(pool), token.clone());
            let finished = Arc::clone(&starts);
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
                finished.open();
                send.send((result, emitted)).unwrap();
            });
            drop(starts.wait(|state| state.arrivals > at || state.open));
            starts.open();
            drop(starts.wait_for(delay, |state| state.arrivals > at + features));
            let cancel = Instant::now();
            token.cancel();
            let (result, emitted) = receive
                .recv_timeout(Duration::from_secs(30))
                .expect("cancellation stops the run");
            slowest = slowest.max(cancel.elapsed());
            worker.join().unwrap();
            clear(&name);
            match result {
                Ok(()) => assert_eq!(emitted, expected, "seed {seed}"),
                Err(SinkError::Core(Error::Cancelled)) => {
                    cancelled += 1;
                    assert_eq!(emitted, expected[..emitted.len()], "seed {seed}");
                }
                Err(error) => panic!("seed {seed}: {error:?}"),
            }
        }
        // Only a point within the last few milliseconds of a run lets it finish.
        assert!(cancelled > 10, "{cancelled}");
        assert!(slowest < Duration::from_secs(5), "{slowest:?}");
        for pool in &pools {
            assert_idle(pool);
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
    }

    #[test]
    fn cancelling_stops_features_already_running_on_worker_threads() {
        // Every feature of the one parallel batch blocks until cancellation and then
        // checks once, so only checks on the workers themselves stop it promptly.
        let mut t = Topology::new(0.).unwrap();
        let lines = (0..33).map(|fid| {
            let x = fid as f64;
            (fid, LineString::from(vec![(x, 0.), (x, 1.)]).into())
        });
        let layer = t.add_layer(Layer::from_features("worker-cancel", lines));
        let spec = t.add_rule(Rule::LineMustNotIntersect, layer, None).unwrap();
        let token = CancellationToken::default();
        let gate = Arc::new(Gate::default());
        let block = || Action::Block(Arc::clone(&gate), GRACE);
        let name = on(&t, spec, 1, block());
        for fid in 2..33 {
            on(&t, spec, fid, block());
        }
        let (stopped, result) = std::thread::scope(|scope| {
            let run = scope.spawn(|| {
                let result = with_threads(4, || {
                    t.validate_into_cancellable(token.clone(), |_, _| Ok::<_, Infallible>(()))
                });
                (Instant::now(), result.map(|_| ()))
            });
            drop(gate.wait(|state| state.threads.len() >= 4));
            let cancelled = Instant::now();
            token.cancel();
            gate.open();
            let (ended, result) = run.join().unwrap();
            (ended - cancelled, result)
        });
        clear(&name);
        assert_eq!(gate.state().threads.len(), 4);
        assert_eq!(gate.state().arrivals, 4);
        assert!(
            matches!(result, Err(SinkError::Core(Error::Cancelled))),
            "{result:?}"
        );
        assert!(stopped < GRACE / 5, "{stopped:?}");
    }

    #[test]
    fn whole_layer_rules_stop_at_the_next_check_after_their_inputs_are_built() {
        // Each step does a little for every feature between the cached inputs and
        // the first emitted violation.
        let count = 10_000;
        let features = |geometry: fn(f64) -> Geometry| {
            (0..count)
                .map(|fid| (fid, geometry(fid as f64 * 2.)))
                .collect::<Vec<_>>()
        };
        let disjoint = features(|x| LineString::from(vec![(x, 0.), (x, 1.)]).into());
        let chain = features(|x| LineString::from(vec![(x, 0.), (x + 2., 0.)]).into());
        let multi = features(|x| {
            MultiLineString(vec![
                LineString::from(vec![(x, 0.), (x, 1.)]),
                LineString::from(vec![(x, 2.), (x, 3.)]),
            ])
            .into()
        });
        let points = features(|x| Point::new(x, 0.).into());
        let squares = features(|x| {
            Rect::new(coord! {x: x, y: 0.}, coord! {x: x + 1., y: 1.})
                .to_polygon()
                .into()
        });
        let well = vec![(0, Point::new(-10., -10.).into())];
        // Points in the box of a triangle but outside it.
        let side = count as f64 + 1.;
        let triangle = Polygon::new(
            LineString::from(vec![(0., 0.), (side, 0.), (0., side), (0., 0.)]),
            vec![],
        );
        let edge = (0..count)
            .map(|fid| (fid, Point::new(fid as f64 + 1., side).into()))
            .collect();
        let cases = [
            (Rule::LineMustNotHaveDangles, disjoint, None),
            (Rule::LineMustNotHavePseudoNodes, chain, None),
            (Rule::GeometryMustNotBeMultipart, multi, None),
            (Rule::PointMustNotOverlap, points, None),
            (Rule::PolygonMustContainPoint, squares, Some(well)),
            (
                Rule::PolygonMustContainOnePoint,
                vec![(0, triangle.into())],
                Some(edge),
            ),
        ];
        let reached_by_case = [
            "dangles violations.sort",
            "pseudos violations.sort",
            "multipart violations.sort",
            "point.features",
            "candidates aggregate aggregate.gather aggregate.merge violations.sort",
            "candidates aggregate aggregate.gather aggregate.merge violations.sort",
        ];
        // Steps after the inputs are built.
        let steps = [
            "dangles",
            "pseudos",
            "multipart",
            "point.features",
            "candidates",
            "aggregate",
            "aggregate.gather",
            "aggregate.merge",
            "violations.sort",
        ];
        for ((rule, features, other), expected) in cases.into_iter().zip(reached_by_case) {
            let mut t = Topology::new(0.).unwrap();
            let layer = t.add_layer(Layer::from_features("many", features));
            let other = other.map(|f| t.add_layer(Layer::from_features("other", f)));
            t.add_rule(rule, layer, other).unwrap();
            let token = crate::cancel::hooks::watched(None);
            assert!(validated(&t, token.clone()).0.is_ok(), "{rule:?}");
            let visits = crate::cancel::hooks::record(&token).visits;
            let reached = steps
                .into_iter()
                .filter(|step| visits.contains_key(step))
                .collect::<Vec<_>>();
            let expected = expected.split_whitespace().collect::<Vec<_>>();
            assert_eq!(reached, expected, "{rule:?}");
            for step in reached {
                // A check at least every CHECK features' worth of work.
                let least = count as usize / crate::cancel::CHECK;
                assert_stops_at_each_check(&t, step, least, &format!("{rule:?}"));
            }
        }
    }

    #[test]
    fn a_feature_whose_parts_find_no_pairs_stops_at_the_next_check() {
        // No part of the feature is in the box of the other layer's feature, so only
        // the steps before and of the search run.
        let count = 10_000;
        let far = |i: usize| (i as f64, 10.);
        let points = MultiPoint::from((0..count).map(|i| Point::from(far(i))).collect::<Vec<_>>());
        let lines = (0..count).map(|i| LineString::from(vec![far(i), (i as f64 + 0.5, 10.)]));
        let squares = (0..count).map(|i| Rect::new(far(i), (i as f64 + 0.5, 10.5)).to_polygon());
        let end = count as f64;
        let cases: [(Rule, Geometry, Geometry, &[&str]); 3] = [
            (
                Rule::PointMustNotIntersectLines,
                points.into(),
                LineString::from(vec![(0., 0.), (end, 0.)]).into(),
                &["candidates", "index.arrange"],
            ),
            (
                Rule::LineMustNotIntersectPolygons,
                MultiLineString::new(lines.collect()).into(),
                Rect::new((0., -1.), (end, 0.)).to_polygon().into(),
                &["candidates", "index.arrange"],
            ),
            (
                Rule::PolygonMustNotContainPoints,
                MultiPolygon::new(squares.collect()).into(),
                Point::new(0., 0.).into(),
                &["candidates", "index.arrange", "locations"],
            ),
        ];
        for (rule, feature, other, steps) in cases {
            let mut t = Topology::new(0.).unwrap();
            let layer = t.add_layer(Layer::from_features("far", [(1, feature)]));
            let other = t.add_layer(Layer::from_features("near", [(1, other)]));
            t.add_rule(rule, layer, Some(other)).unwrap();
            for &step in steps {
                let least = count / crate::cancel::CHECK;
                assert_stops_at_each_check(&t, step, least, &format!("{rule:?}"));
            }
        }
    }

    /// Validate `t` with `token`, counting the violations sent.
    fn validated(t: &Topology, token: CancellationToken) -> (Result<(), Error>, usize) {
        let mut sent = 0;
        let result = t.validate_into_cancellable(token, |_, _| {
            sent += 1;
            Ok::<_, Infallible>(())
        });
        (result.map(|_| ()).map_err(Error::from), sent)
    }
    /// Check that `t`'s validation passes `step` at least `least` times, and that
    /// cancelling at its first, middle and last visit stops at the next check, before
    /// any violation is sent.
    fn assert_stops_at_each_check(t: &Topology, step: &'static str, least: usize, case: &str) {
        let token = crate::cancel::hooks::watched(None);
        assert!(validated(t, token.clone()).0.is_ok(), "{case} {step}");
        let checks = crate::cancel::hooks::record(&token)
            .visits
            .get(step)
            .copied()
            .unwrap_or_default();
        assert!(checks >= least, "{case} {step}: {checks}");
        for visit in [0, checks / 2, checks - 1] {
            let token = crate::cancel::hooks::watched(Some((step, visit)));
            let (result, sent) = validated(t, token.clone());
            assert!(
                matches!(result, Err(Error::Cancelled)),
                "{case} {step} {visit}: {result:?}"
            );
            let after = crate::cancel::hooks::record(&token).visits[step];
            assert_eq!(after, visit + 1, "{case} {step} {visit}");
            assert_eq!(sent, 0, "{case} {step} {visit}");
        }
    }

    #[test]
    fn a_cancelled_validation_never_waits_for_another_one() {
        // Both validations share a pool. A cancels while two features of a batch run,
        // and B is blocked in its first feature until A has returned.
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(2)
            .build()
            .unwrap();
        let topology = |name: &str| {
            let mut t = Topology::new(0.).unwrap();
            let lines = (0..3).map(|fid| {
                let x = fid as f64;
                (fid, LineString::from(vec![(x, 0.), (x, 1.)]).into())
            });
            let layer = t.add_layer(Layer::from_features(name, lines));
            let spec = t.add_rule(Rule::LineMustNotIntersect, layer, None).unwrap();
            (t, spec)
        };
        let ((a, a_spec), (b, b_spec)) = (topology("concurrent-a"), topology("concurrent-b"));
        let expected = reference(&b, b_spec).unwrap();
        let (a_gate, b_gate) = (Arc::new(Gate::default()), Arc::new(Gate::default()));
        // Feature 1 returns once feature 2 is blocked on another thread.
        let a_name = on(&a, a_spec, 1, Action::Await(Arc::clone(&a_gate), 1));
        on(&a, a_spec, 2, Action::Block(Arc::clone(&a_gate), GRACE));
        let b_name = on(
            &b,
            b_spec,
            0,
            Action::Block(Arc::clone(&b_gate), Duration::ZERO),
        );
        let token = CancellationToken::default();
        let run = |t: &Topology, token: CancellationToken| {
            pool.install(|| {
                with_threads(2, || {
                    let mut out = Vec::new();
                    let result = t.validate_into_cancellable(token, |_, v| {
                        out.push(v);
                        Ok::<_, Infallible>(())
                    });
                    result.map(|_| out)
                })
            })
        };
        let (done, a_result) = mpsc::channel();
        std::thread::scope(|scope| {
            let a_run = scope.spawn(|| done.send(run(&a, token.clone())).unwrap());
            drop(a_gate.wait(|state| state.arrivals == 1));
            let b_run = scope.spawn(|| run(&b, CancellationToken::default()));
            drop(b_gate.wait(|state| state.arrivals == 1));
            token.cancel();
            a_gate.open();
            let a_result = a_result.recv_timeout(Duration::from_secs(10));
            b_gate.open();
            assert!(
                matches!(a_result, Ok(Err(SinkError::Core(Error::Cancelled)))),
                "{a_result:?}"
            );
            assert_eq!(b_run.join().unwrap().unwrap(), expected);
            a_run.join().unwrap();
        });
        clear(&a_name);
        clear(&b_name);
        assert_idle(&pool);
    }

    #[test]
    fn large_features_split_their_points_and_pairs_without_changing_the_output() {
        // One feature with more candidate pairs than a feature task holds at once, the
        // last of its threads' tasks short of PAIR_TASK pairs.
        let count = PAIRS + PAIRS / 2 + 37;
        let points = (0..count)
            .map(|i| Point::new(i as f64, 0.))
            .collect::<Vec<_>>();
        // Even lines cover their points, odd ones only cross their boxes.
        let lines = (0..count.div_ceil(64)).map(|k| {
            let x = (k * 64) as f64;
            let rise = if k % 2 == 0 { 0. } else { 1. };
            let line = LineString::from(vec![(x - 0.5, -rise), (x + 63.5, rise)]);
            (k as i64, line.into())
        });
        let mut t = Topology::new(0.).unwrap();
        let a = t.add_layer(Layer::from_features(
            "large-feature",
            [(1, MultiPoint::from(points).into())],
        ));
        let b = t.add_layer(Layer::from_features("large-feature-lines", lines));
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(4)
            .build()
            .unwrap();
        for rule in [
            Rule::PointMustNotIntersectLines,
            Rule::PointMustBeCoveredByLines,
        ] {
            let spec = t.add_rule(rule, a, Some(b)).unwrap();
            let expected = reference(&t, spec).unwrap();
            assert!(!expected.is_empty(), "{rule:?}");
            let streamed = pool.install(|| with_threads(4, || streamed(&t, spec, Some(8))));
            assert_eq!(streamed.unwrap(), expected, "{rule:?}");
        }
    }

    #[test]
    fn feature_batches_share_one_helper_budget() {
        static BUDGET: AtomicUsize = AtomicUsize::new(0);
        let used = || BUDGET.load(atomic::Ordering::Relaxed);
        let first = Helpers::take_from(&BUDGET, 3, 2);
        let second = Helpers::take_from(&BUDGET, 3, 2);
        let third = Helpers::take_from(&BUDGET, 3, 2);
        assert_eq!([first.threads, second.threads, third.threads], [2, 1, 0]);
        assert_eq!(used(), 3);
        drop(first);
        assert_eq!(used(), 1);
        let fourth = Helpers::take_from(&BUDGET, 3, 5);
        assert_eq!(fourth.threads, 2);
        drop((second, third, fourth));
        assert_eq!(used(), 0);
        // Batches taking and returning helpers at once never go over the limit.
        std::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| {
                    for want in 0..2000 {
                        let helpers = Helpers::take_from(&BUDGET, 3, want % 4);
                        assert!(helpers.threads <= want % 4);
                        assert!(used() <= 3);
                    }
                });
            }
        });
        assert_eq!(used(), 0);
        // Without a fixed thread count, batches count on the process budget.
        let helpers = Helpers::take(1);
        assert!(std::ptr::eq(helpers.budget, &HELPERS));
        assert_eq!(helpers.shared, helpers.threads);
        assert!(helpers.threads <= 1);
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
        let streamed = pool.install(|| with_threads(4, || streamed(&t, spec, None)));
        clear(&name);
        assert_eq!(streamed.unwrap(), expected);
        let starts = starts.lock().unwrap_or_else(PoisonError::into_inner);
        assert_eq!(starts.len(), 23);
        assert!(starts.iter().all(|&at| at >= started + hold));
    }

    #[test]
    fn the_batch_after_a_slow_one_runs_apart_from_the_calling_thread() {
        // Batches on one thread: [0], 1 to 8 with slow feature 3, 9 to 16, 17 to 24.
        let mut t = Topology::new(0.).unwrap();
        let lines = (0..25).map(|fid| {
            let x = fid as f64;
            let line = if fid % 2 == 0 {
                LineString::from(vec![(x, -1.), (x, 13.)])
            } else {
                LineString::from(vec![(-1., x / 2.), (25., x / 2.)])
            };
            (fid, line.into())
        });
        let layer = t.add_layer(Layer::from_features("apart", lines));
        let spec = t.add_rule(Rule::LineMustNotIntersect, layer, None).unwrap();
        let expected = reference(&t, spec).unwrap();
        assert!(!expected.is_empty());
        // Far apart, so that only the held feature makes its batch slow.
        let slow = SLOW_BATCH * 250;
        let threads = Arc::new(Mutex::new(Vec::new()));
        let name = on(&t, spec, 3, Action::Hold(slow + slow / 5));
        for fid in (0..25).filter(|&fid| fid != 3) {
            on(&t, spec, fid, Action::Thread(Arc::clone(&threads)));
        }
        let streamed = with_threads(1, || with_slow_batch(slow, || streamed(&t, spec, None)));
        clear(&name);
        assert_eq!(streamed.unwrap(), expected);
        let caller = std::thread::current().id();
        let threads = threads.lock().unwrap_or_else(PoisonError::into_inner);
        assert_eq!(threads.len(), 24);
        let apart = threads
            .iter()
            .filter(|&&(_, thread)| thread != caller)
            .collect::<Vec<_>>();
        assert_eq!(
            apart.iter().map(|&&(fid, _)| fid).collect::<Vec<_>>(),
            (9..17).collect::<Vec<_>>()
        );
        assert!(apart.iter().all(|&&(_, thread)| thread == apart[0].1));
    }

    #[test]
    fn parallel_batches_grow_while_they_are_quick() {
        let mut t = Topology::new(0.).unwrap();
        // Pairs of overlapping lines, the first of which reports their overlap.
        let lines = (0..40).map(|fid| {
            let x = (fid / 2 * 5 + fid % 2) as f64;
            (fid, LineString::from(vec![(x, 0.), (x + 2., 0.)]).into())
        });
        let layer = t.add_layer(Layer::from_features("grow", lines));
        let spec = t.add_rule(Rule::LineMustNotOverlap, layer, None).unwrap();
        let expected = reference(&t, spec).unwrap();
        // Features run before each violation is emitted, which ends a batch.
        let ends = |quick: Duration| {
            let visits = Arc::new(Mutex::new(Vec::new()));
            let name = on(&t, spec, 0, Action::Thread(Arc::clone(&visits)));
            for fid in 1..40 {
                on(&t, spec, fid, Action::Thread(Arc::clone(&visits)));
            }
            let (mut out, mut ends) = (Vec::new(), Vec::new());
            let result = with_threads(2, || {
                with_quick_per_helper(quick, || {
                    per_feature_into(&t, spec, 1, &mut |v| {
                        out.push(v);
                        ends.push(visits.lock().unwrap_or_else(PoisonError::into_inner).len());
                        Ok::<_, Infallible>(())
                    })
                })
            });
            clear(&name);
            result.unwrap();
            assert_eq!(out, expected);
            ends.dedup();
            ends
        };
        // Batches of 1, then 1 feature on each of 2 threads: the last, of 39, emits none.
        let pairs = (1..=39).step_by(2).collect::<Vec<_>>();
        assert_eq!(ends(Duration::ZERO), pairs);
        // Batches of 1, then 2, 4, 8, 16 and the 9 features left.
        assert_eq!(ends(Duration::from_secs(3600)), [1, 3, 7, 15, 31, 40]);
    }

    #[test]
    fn quick_batches_grow_to_a_bound_and_slow_ones_start_over() {
        // What a batch on 8 threads may take to grow.
        let quick = QUICK_PER_HELPER * 7;
        let grown =
            std::iter::successors(Some(8), |&per| Some(grow(per, 8, Duration::ZERO, quick)));
        assert_eq!(
            grown.take(8).collect::<Vec<_>>(),
            [8, 16, 32, 64, 128, 256, 256, 256]
        );
        assert_eq!(grow(64, 8, quick, quick), 64);
        assert_eq!(grow(64, 8, quick * 4 - Duration::from_nanos(1), quick), 64);
        assert_eq!(grow(64, 8, quick * 4, quick), 8);
        // Batches asked to be larger than the bound keep their size.
        assert_eq!(grow(512, 512, Duration::ZERO, quick), 512);
        assert_eq!(grow(0, 0, quick * 4, quick), 1);
    }

    #[test]
    fn shared_caches_build_the_same_whatever_helpers_are_free() {
        // The first feature builds the line graph and its matches for every feature.
        let features = (0..20_000_i64)
            .map(|fid| {
                let (x, y) = ((fid % 200) as f64, (fid / 200) as f64);
                let (dx, dy) = [(1., 0.), (0., 0.5), (0.5, 0.5), (2., 0.)][(fid % 4) as usize];
                let line = LineString::from(vec![(x, y), (x + dx, y + dy)]);
                (fid, Geometry::from(line))
            })
            .collect::<Vec<_>>();
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(4)
            .build()
            .unwrap();
        let visits = |free| {
            let mut t = Topology::new(0.001).unwrap();
            let layer = t.add_layer(Layer::from_features("shared", features.clone()));
            t.add_rule(Rule::LineMustNotOverlap, layer, None).unwrap();
            let token = crate::cancel::hooks::watched(None);
            pool.install(|| {
                with_free(free, || {
                    t.validate_into_cancellable(token.clone(), |_, _| Ok::<_, Infallible>(()))
                })
            })
            .unwrap();
            crate::cancel::hooks::record(&token).visits
        };
        let all = visits(3);
        assert!(all["pairs.join"] > 1, "{all:?}");
        assert_eq!(visits(0), all);
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
                            with_threads(pool.current_num_threads(), || {
                                let sink = |_, _| Ok::<_, Infallible>(());
                                if cancellable {
                                    case.topology.validate_into_cancellable(
                                        CancellationToken::default(),
                                        sink,
                                    )
                                } else {
                                    case.topology.validate_into(sink)
                                }
                            })
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
