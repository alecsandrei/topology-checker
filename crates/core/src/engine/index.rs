use super::parallel::Workers;
use geo::{Coord, CoordFloat, Rect};
use rstar::{
    AABB, RTree,
    primitives::{GeomWithData, Rectangle},
};
use std::ops::Range;

pub(crate) type Index = RTree<GeomWithData<Rectangle<[f64; 2]>, usize>>;
pub(crate) fn envelope(rect: Rect) -> AABB<[f64; 2]> {
    AABB::from_corners([rect.min().x, rect.min().y], [rect.max().x, rect.max().y])
}
pub(crate) fn expanded(rect: Rect, tolerance: f64) -> AABB<[f64; 2]> {
    AABB::from_corners(
        [rect.min().x - tolerance, rect.min().y - tolerance],
        [rect.max().x + tolerance, rect.max().y + tolerance],
    )
}
pub(crate) fn at(point: Coord, tolerance: f64) -> AABB<[f64; 2]> {
    expanded(Rect::new(point, point), tolerance)
}
pub(crate) fn build(items: impl IntoIterator<Item = (usize, Rect)>) -> Index {
    RTree::bulk_load(
        items
            .into_iter()
            .map(|(i, r)| GeomWithData::new(Rectangle::from_aabb(envelope(r)), i))
            .collect(),
    )
}
pub(crate) fn pairs<'a>(
    a: &'a Index,
    b: &'a Index,
    tolerance: f64,
    same: bool,
) -> impl Iterator<Item = (usize, usize)> + 'a {
    use rstar::RTreeObject;
    a.iter().flat_map(move |item| {
        let bounds = item.envelope();
        let rect = Rect::new(
            Coord {
                x: bounds.lower()[0],
                y: bounds.lower()[1],
            },
            Coord {
                x: bounds.upper()[0],
                y: bounds.upper()[1],
            },
        );
        b.locate_in_envelope_intersecting(&expanded(rect, tolerance))
            .filter(move |other| !same || item.data < other.data)
            .map(move |other| (item.data, other.data))
    })
}

/// Items with boxes, searched in a fixed order.
pub(crate) trait Search: Sync {
    /// Items whose box intersects `bounds`, in search order, with their boxes.
    fn search(&self, bounds: AABB<[f64; 2]>) -> impl Iterator<Item = (usize, AABB<[f64; 2]>)>;
    /// `visit(a, b, out)` for each pair of [`pairs`] of the index with itself,
    /// with the outputs in that order. Pairs whose first item is not `wanted`
    /// may be skipped, so `visit` must do nothing for them.
    fn pairs<R: Send>(
        &self,
        workers: &Workers,
        tolerance: f64,
        step: &'static str,
        wanted: impl Fn(usize) -> bool + Sync,
        visit: impl Fn(usize, usize, &mut Vec<R>) + Sync,
    ) -> Vec<R>;
}
#[cfg(test)]
impl Search for Index {
    fn search(&self, bounds: AABB<[f64; 2]>) -> impl Iterator<Item = (usize, AABB<[f64; 2]>)> {
        use rstar::RTreeObject;
        self.locate_in_envelope_intersecting(&bounds)
            .map(|item| (item.data, item.envelope()))
    }
    fn pairs<R: Send>(
        &self,
        _: &Workers,
        tolerance: f64,
        _: &'static str,
        _: impl Fn(usize) -> bool + Sync,
        visit: impl Fn(usize, usize, &mut Vec<R>) + Sync,
    ) -> Vec<R> {
        let mut out = Vec::new();
        for (a, b) in pairs(self, self, tolerance, true) {
            crate::cancel::check();
            visit(a, b, &mut out);
        }
        out
    }
}

/// Largest leaf of the bulk-loaded R-tree whose search order [`Packed`] keeps.
const LEAF: usize = 6;
/// Children of each [`Packed`] node.
const FANOUT: usize = 16;
/// Search frames: enough levels for any `usize` length.
const DEPTH: usize = 18;
/// Comparisons between cancellation checks while ordering.
const CHECK: usize = 1 << 14;
/// Items and pairs between cancellation checks while pairing.
const PAIR_CHECK: usize = 256;

/// A box and its item. It has the size of an R-tree rectangle with data, so
/// ordering takes the same steps.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct Entry {
    pub min: [f64; 2],
    pub max: [f64; 2],
    pub id: usize,
}
impl Entry {
    pub(crate) fn new(id: usize, rect: Rect) -> Self {
        let bounds = envelope(rect);
        Self {
            min: bounds.lower(),
            max: bounds.upper(),
            id,
        }
    }
    pub(crate) fn envelope(&self) -> AABB<[f64; 2]> {
        AABB::from_corners(self.min, self.max)
    }
    pub(crate) fn rect(&self) -> Rect {
        Rect::new(
            Coord {
                x: self.min[0],
                y: self.min[1],
            },
            Coord {
                x: self.max[0],
                y: self.max[1],
            },
        )
    }
    fn bounds(&self) -> [f64; 4] {
        [self.min[0], self.min[1], self.max[0], self.max[1]]
    }
}
fn intersects(a: [f64; 4], b: [f64; 4]) -> bool {
    a[0] <= b[2] && a[1] <= b[3] && a[2] >= b[0] && a[3] >= b[1]
}
fn merge(a: [f64; 4], b: [f64; 4]) -> [f64; 4] {
    [
        a[0].min(b[0]),
        a[1].min(b[1]),
        a[2].max(b[2]),
        a[3].max(b[3]),
    ]
}

/// A static box index. It finds the same items in the same order as an
/// R-tree bulk loaded from the same entries, without a node allocation per
/// leaf. It builds on several threads and stops when cancelled.
#[derive(Debug)]
pub(crate) struct Packed {
    /// Entries in search order.
    entries: Vec<Entry>,
    /// Bounds of each run of `FANOUT` entries, then of each run of those,
    /// up to a single root.
    levels: Vec<Vec<[f64; 4]>>,
}
impl Packed {
    pub(crate) fn bulk(mut entries: Vec<Entry>, workers: &Workers) -> Self {
        arrange(&mut entries, workers);
        Self::ordered(entries, workers)
    }
    /// An index over entries already in search order.
    pub(crate) fn ordered(entries: Vec<Entry>, workers: &Workers) -> Self {
        let mut levels: Vec<Vec<[f64; 4]>> = Vec::new();
        if !entries.is_empty() {
            levels.push(group(entries.len(), workers, |i| entries[i].bounds()));
            while let Some(below) = levels.last().filter(|level| level.len() > 1) {
                let above = group(below.len(), workers, |i| below[i]);
                levels.push(above);
            }
        }
        Self { entries, levels }
    }
    /// Entries in search order.
    #[cfg(test)]
    pub(crate) fn entries(&self) -> &[Entry] {
        &self.entries
    }
    /// Item IDs in search order, if they all fit.
    pub(crate) fn order(&self) -> Option<Vec<u32>> {
        self.entries
            .iter()
            .map(|e| u32::try_from(e.id).ok())
            .collect()
    }
    /// Entries whose box intersects `bounds`, in search order.
    pub(crate) fn locate(&self, bounds: AABB<[f64; 2]>) -> Locate<'_> {
        let mut frames = [(0, 0, 0); DEPTH];
        frames[0] = (self.levels.len(), 0, usize::from(!self.entries.is_empty()));
        Locate {
            tree: self,
            query: [
                bounds.lower()[0],
                bounds.lower()[1],
                bounds.upper()[0],
                bounds.upper()[1],
            ],
            frames,
            depth: 1,
        }
    }
}
impl Search for Packed {
    fn search(&self, bounds: AABB<[f64; 2]>) -> impl Iterator<Item = (usize, AABB<[f64; 2]>)> {
        self.locate(bounds).map(|e| (e.id, e.envelope()))
    }
    fn pairs<R: Send>(
        &self,
        workers: &Workers,
        tolerance: f64,
        step: &'static str,
        wanted: impl Fn(usize) -> bool + Sync,
        visit: impl Fn(usize, usize, &mut Vec<R>) + Sync,
    ) -> Vec<R> {
        let found = workers.chunks(self.entries.len(), |range| {
            let mut out = Vec::new();
            let mut work = 0_usize;
            let mut tick = || {
                if work.is_multiple_of(PAIR_CHECK) {
                    crate::cancel::checkpoint(step);
                }
                work += 1;
            };
            for item in &self.entries[range] {
                tick();
                if !wanted(item.id) {
                    continue;
                }
                for other in self.locate(expanded(item.rect(), tolerance)) {
                    tick();
                    if item.id < other.id {
                        visit(item.id, other.id, &mut out);
                    }
                }
            }
            out
        });
        let mut out = Vec::with_capacity(found.iter().map(Vec::len).sum());
        for part in found {
            out.extend(part);
        }
        out
    }
}
/// Bounds of each run of `FANOUT` boxes.
fn group(
    len: usize,
    workers: &Workers,
    bounds: impl Fn(usize) -> [f64; 4] + Sync,
) -> Vec<[f64; 4]> {
    let mut out = vec![[0.; 4]; len.div_ceil(FANOUT)];
    workers.fill(&mut out, |g| {
        (g * FANOUT + 1..((g + 1) * FANOUT).min(len))
            .fold(bounds(g * FANOUT), |b, i| merge(b, bounds(i)))
    });
    out
}
/// Search of a [`Packed`] index, depth first over its levels.
pub(crate) struct Locate<'a> {
    tree: &'a Packed,
    query: [f64; 4],
    /// Level (0 for entries), next index and end of each node being searched.
    frames: [(usize, usize, usize); DEPTH],
    depth: usize,
}
impl<'a> Iterator for Locate<'a> {
    type Item = &'a Entry;
    fn next(&mut self) -> Option<&'a Entry> {
        while self.depth > 0 {
            let (level, next, end) = &mut self.frames[self.depth - 1];
            if *next == *end {
                self.depth -= 1;
                continue;
            }
            let (level, i) = (*level, *next);
            *next += 1;
            if level == 0 {
                let entry = &self.tree.entries[i];
                if intersects(entry.bounds(), self.query) {
                    return Some(entry);
                }
            } else if intersects(self.tree.levels[level - 1][i], self.query) {
                let below = match level {
                    1 => self.tree.entries.len(),
                    _ => self.tree.levels[level - 2].len(),
                };
                self.frames[self.depth] = (level - 1, i * FANOUT, ((i + 1) * FANOUT).min(below));
                self.depth += 1;
            }
        }
        None
    }
}

/// Node or slab ordering in the bulk load of an R-tree.
#[derive(Debug, Clone, Copy)]
enum Task {
    /// A node, split into slabs on y.
    Node,
    /// A slab of a node with this many clusters on each axis, split on x.
    Slab(usize),
}
/// Clusters on each axis of a node, computed by the R-tree's float functions,
/// and at least two, where the R-tree would recurse without end.
fn clusters_per_axis(len: usize) -> usize {
    fn log<T: CoordFloat>(x: T, base: T) -> T {
        x.log(base)
    }
    fn powi<T: CoordFloat>(x: T, n: i32) -> T {
        x.powi(n)
    }
    fn powf<T: CoordFloat>(x: T, n: T) -> T {
        x.powf(n)
    }
    fn ceil<T: CoordFloat>(x: T) -> T {
        x.ceil()
    }
    let max = LEAF as f32;
    let depth = ceil(log(len as f32, max)) as usize;
    let subtree = powi(max, depth as i32 - 1);
    let clusters = ceil(len as f32 / subtree);
    (ceil(powf(clusters, 1. / 2.)) as usize).max(2)
}
fn select(entries: &mut [Entry], index: usize, axis: usize, count: &mut usize) {
    entries.select_nth_unstable_by(index, |l, r| {
        *count += 1;
        if count.is_multiple_of(CHECK) {
            crate::cancel::checkpoint("index");
        }
        l.min[axis]
            .partial_cmp(&r.min[axis])
            .expect("finite coordinates")
    });
}
/// Split into slabs of equal size on an axis, the R-tree's way, lowest first.
fn slabs(
    entries: &mut [Entry],
    clusters: usize,
    axis: usize,
    count: &mut usize,
    mut f: impl FnMut(&mut [Entry], &mut usize),
) {
    let size = entries.len().div_ceil(clusters);
    let mut rest = entries;
    while rest.len() > size {
        select(rest, size, axis, count);
        let (slab, tail) = std::mem::take(&mut rest).split_at_mut(size);
        f(slab, count);
        rest = tail;
    }
    if !rest.is_empty() {
        f(rest, count);
    }
}
/// Order one node or slab, with `inner` for each of its parts. A leaf ends
/// in reverse, the order an R-tree searches it in.
fn split_task(
    entries: &mut [Entry],
    task: Task,
    count: &mut usize,
    inner: &mut dyn FnMut(&mut [Entry], Task, &mut usize),
) {
    match task {
        Task::Node if entries.len() <= LEAF => entries.reverse(),
        Task::Node => {
            let clusters = clusters_per_axis(entries.len());
            slabs(entries, clusters, 1, count, |slab, count| {
                inner(slab, Task::Slab(clusters), count);
            });
        }
        Task::Slab(clusters) => slabs(entries, clusters, 0, count, |slab, count| {
            inner(slab, Task::Node, count);
        }),
    }
}
fn finish(entries: &mut [Entry], task: Task, count: &mut usize) {
    split_task(entries, task, count, &mut finish);
}
/// Disjoint slices for tasks over ascending, non-overlapping ranges.
fn split<'a>(
    mut rest: &'a mut [Entry],
    tasks: &[(Range<usize>, Task)],
) -> Vec<(usize, &'a mut [Entry], Task)> {
    let mut at = 0;
    let mut pieces = Vec::with_capacity(tasks.len());
    for (range, task) in tasks {
        let (_, tail) = rest.split_at_mut(range.start - at);
        let (piece, tail) = tail.split_at_mut(range.len());
        pieces.push((range.start, piece, *task));
        rest = tail;
        at = range.end;
    }
    pieces
}
/// Put entries in the search order of an R-tree bulk loaded from them. Nodes
/// and slabs are split level by level until each thread has several, then
/// each is finished depth first.
fn arrange(entries: &mut [Entry], workers: &Workers) {
    let mut tasks = vec![(0..entries.len(), Task::Node)];
    while workers.threads() > 1 && !tasks.is_empty() && tasks.len() < 4 * workers.threads() {
        let next = workers.each(split(entries, &tasks), |(start, piece, task)| {
            let mut parts = Vec::new();
            let mut at = start;
            split_task(piece, task, &mut 0, &mut |slab, task, _| {
                parts.push((at..at + slab.len(), task));
                at += slab.len();
            });
            parts
        });
        tasks = next.into_iter().flatten().collect();
    }
    workers.each(split(entries, &tasks), |(_, piece, task)| {
        finish(piece, task, &mut 0);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Boxes of mixed size with repeated coordinates and signed zeros.
    fn boxes(n: usize, seed: u64) -> Vec<(usize, Rect)> {
        let mut state = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        (0..n)
            .map(|i| {
                let mut value = |scale: u64| {
                    let v = (next() % scale) as f64 - (scale / 2) as f64;
                    if v == 0. && next() % 2 == 0 {
                        -0.
                    } else {
                        v / 4.
                    }
                };
                let a = Coord {
                    x: value(64),
                    y: value(64),
                };
                let size = [2, 4, 64][i % 3];
                let b = Coord {
                    x: a.x + value(size).abs(),
                    y: a.y + value(size).abs(),
                };
                (i, Rect::new(a, b))
            })
            .collect()
    }

    fn collect_pairs(tree: &impl Search, workers: &Workers, tolerance: f64) -> Vec<(usize, usize)> {
        let wanted = |a: usize| a % 3 != 1;
        tree.pairs(workers, tolerance, "pairs", wanted, |a, b, out| {
            if wanted(a) {
                out.push((a, b));
            }
        })
    }

    #[test]
    fn packed_search_matches_the_bulk_loaded_r_tree() {
        let sizes = [
            0, 1, 2, 6, 7, 12, 36, 37, 215, 216, 217, 1000, 1296, 5000, 7777,
        ];
        for (k, &n) in sizes.iter().enumerate() {
            let items = boxes(n, k as u64 + 1);
            let tree = build(items.clone());
            let expected = {
                use rstar::RTreeObject;
                tree.iter()
                    .map(|i| (i.data, i.envelope()))
                    .collect::<Vec<_>>()
            };
            for (threads, grain) in [(1, 4096), (2, 1), (4, 3), (8, 64)] {
                let workers = Workers::forced(threads, grain, || Workers::new(0));
                let entries = items.iter().map(|&(i, r)| Entry::new(i, r)).collect();
                let packed = Packed::bulk(entries, &workers);
                let found = packed
                    .entries()
                    .iter()
                    .map(|e| (e.id, e.envelope()))
                    .collect::<Vec<_>>();
                assert_eq!(found, expected, "{n} boxes on {threads} threads");
                for &(_, rect) in items.iter().step_by(7).take(64) {
                    for tolerance in [0., 0.25, 3.] {
                        let query = expanded(rect, tolerance);
                        assert!(packed.search(query).eq(tree.search(query)));
                    }
                }
                for tolerance in [0., 0.5] {
                    assert_eq!(
                        collect_pairs(&packed, &workers, tolerance),
                        collect_pairs(&tree, &workers, tolerance)
                    );
                }
                let ordered = Packed::ordered(packed.entries().to_vec(), &workers);
                let query = AABB::from_corners([-4., -4.], [4., 4.]);
                assert!(ordered.locate(query).eq(packed.locate(query)));
            }
        }
    }

    #[test]
    fn identical_boxes_keep_the_r_tree_order() {
        let rect = Rect::new(Coord { x: 1., y: 2. }, Coord { x: 3., y: 4. });
        for n in [7, 100, 2000] {
            let items = (0..n).map(|i| (i, rect)).collect::<Vec<_>>();
            let tree = build(items.clone());
            let workers = Workers::forced(3, 1, || Workers::new(0));
            let entries = items.iter().map(|&(i, r)| Entry::new(i, r)).collect();
            let packed = Packed::bulk(entries, &workers);
            assert!(
                packed
                    .entries()
                    .iter()
                    .map(|e| e.id)
                    .eq(tree.iter().map(|i| i.data))
            );
        }
    }

    #[test]
    fn ordering_and_pairing_stop_when_cancelled() {
        use crate::cancel::{self, CancellationToken, hooks};
        let entries = boxes(1 << 15, 9)
            .into_iter()
            .map(|(i, r)| Entry::new(i, r))
            .collect::<Vec<_>>();
        for (step, visit) in [("index", 0), ("index", 5), ("pairs", 0), ("pairs", 40)] {
            let token = CancellationToken::default();
            hooks::watch(&token, Some((step, visit)));
            let result = std::panic::catch_unwind(|| {
                cancel::scope(Some(token.clone()), || {
                    let workers = Workers::forced(4, 64, || Workers::new(0));
                    let packed = Packed::bulk(entries.clone(), &workers);
                    collect_pairs(&packed, &workers, 0.5)
                })
            });
            let record = hooks::unwatch(&token);
            assert!(result.expect_err("cancelled").is::<cancel::Cancelled>());
            // Each other thread passes at most one more checkpoint.
            let visits = record.visits[step];
            assert!((visit + 1..=visit + 4).contains(&visits), "{step} {visit}");
            assert_eq!(record.live, 0);
            assert!(record.workers > 0 || step == "index", "{step} {visit}");
        }
    }
}
