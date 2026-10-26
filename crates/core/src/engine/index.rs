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
/// [`candidates`] for the items of `a`, in its order. With `same`, `a` and `b` are one
/// index and each pair comes once.
pub(crate) fn pairs<'a>(
    a: &'a impl Search,
    b: &'a impl Search,
    tolerance: f64,
    same: bool,
) -> impl Iterator<Item = (usize, usize)> + 'a {
    candidates(a.items(), b, tolerance).filter(move |&(i, j)| !same || i < j)
}
/// Marks an item of [`arranged`] without a box.
const NONE: usize = usize::MAX;
/// Items `0..len` with the boxes `bounds` gives, in the search order of an R-tree bulk
/// loaded from them, found on several threads, checking for cancellation. Items
/// without a box stay as placeholders that [`nearby`] skips.
pub(crate) fn arranged(len: usize, bounds: impl Fn(usize) -> Option<Rect> + Sync) -> Vec<Entry> {
    let workers = Workers::new(len);
    let mut entries = boxed(len, bounds, &workers, "arranged");
    arrange(&mut entries, &workers);
    entries
}
/// A [`Packed`] index of the items `0..len` that `bounds` gives a box, which finds
/// them in the order of the R-tree [`build`] makes. It builds on several threads
/// and stops when cancelled, first at `step`.
pub(crate) fn packed(
    len: usize,
    step: &'static str,
    bounds: impl Fn(usize) -> Option<Rect> + Sync,
) -> Packed {
    let workers = Workers::new(len);
    let mut entries = boxed(len, bounds, &workers, step);
    entries.retain(|e| e.id != NONE);
    Packed::bulk(entries, &workers)
}
/// Entries of the items `0..len`, with placeholders for items without a box.
fn boxed(
    len: usize,
    bounds: impl Fn(usize) -> Option<Rect> + Sync,
    workers: &Workers,
    step: &'static str,
) -> Vec<Entry> {
    let mut entries = super::parallel::repeated(Entry::default(), len, step);
    workers.fill(&mut entries, |i| {
        bounds(i).map_or(
            Entry {
                id: NONE,
                ..Entry::default()
            },
            |rect| Entry::new(i, rect),
        )
    });
    entries
}
/// [`candidates`] for `items` from [`arranged`], in their order, so that consecutive
/// searches meet nearby parts of `b` whatever the order the items had.
pub(crate) fn nearby<'a>(
    items: &'a [Entry],
    b: &'a impl Search,
    tolerance: f64,
) -> impl Iterator<Item = (usize, usize)> + 'a {
    let boxes = items.iter().filter(|e| e.id != NONE);
    candidates(boxes.map(|e| (e.id, e.rect())), b, tolerance)
}
/// Each item of `boxes` with each item of `b` whose box it meets, grown by `tolerance`.
/// The search checks for cancellation every [`CHECK`](crate::cancel::CHECK) items
/// and candidates, against the token of the thread that starts it, so it stops
/// whichever thread runs it and however few pairs it finds.
pub(crate) fn candidates<'a>(
    boxes: impl Iterator<Item = (usize, Rect)> + 'a,
    b: &'a impl Search,
    tolerance: f64,
) -> impl Iterator<Item = (usize, usize)> + 'a {
    Candidates {
        boxes,
        search: move |rect| b.search(expanded(rect, tolerance)).map(|(id, _)| id),
        found: None,
        steps: 0,
        token: crate::cancel::current(),
    }
}
struct Candidates<A, F, I> {
    boxes: A,
    search: F,
    found: Option<(usize, I)>,
    steps: usize,
    token: Option<crate::CancellationToken>,
}
impl<A, F, I> Iterator for Candidates<A, F, I>
where
    A: Iterator<Item = (usize, Rect)>,
    F: FnMut(Rect) -> I,
    I: Iterator<Item = usize>,
{
    type Item = (usize, usize);
    fn next(&mut self) -> Option<(usize, usize)> {
        loop {
            if self.steps.is_multiple_of(crate::cancel::CHECK) {
                crate::cancel::scope(self.token.clone(), || {
                    crate::cancel::checkpoint("candidates");
                });
            }
            self.steps += 1;
            if let Some((i, found)) = &mut self.found
                && let Some(j) = found.next()
            {
                return Some((*i, j));
            }
            let (i, rect) = self.boxes.next()?;
            self.found = Some((i, (self.search)(rect)));
        }
    }
}

/// Items with boxes, searched in a fixed order.
pub(crate) trait Search: Sync {
    /// Items whose box intersects `bounds`, in search order, with their boxes.
    fn search(&self, bounds: AABB<[f64; 2]>) -> impl Iterator<Item = (usize, AABB<[f64; 2]>)>;
    /// Every item in search order, with its box.
    fn items(&self) -> impl Iterator<Item = (usize, Rect)>;
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
impl Search for Index {
    fn search(&self, bounds: AABB<[f64; 2]>) -> impl Iterator<Item = (usize, AABB<[f64; 2]>)> {
        use rstar::RTreeObject;
        self.locate_in_envelope_intersecting(&bounds)
            .map(|item| (item.data, item.envelope()))
    }
    fn items(&self) -> impl Iterator<Item = (usize, Rect)> {
        use rstar::RTreeObject;
        self.iter().map(|item| {
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
            (item.data, rect)
        })
    }
    fn pairs<R: Send>(
        &self,
        _: &Workers,
        tolerance: f64,
        step: &'static str,
        _: impl Fn(usize) -> bool + Sync,
        visit: impl Fn(usize, usize, &mut Vec<R>) + Sync,
    ) -> Vec<R> {
        let mut out = Vec::new();
        for (k, (a, b)) in pairs(self, self, tolerance, true).enumerate() {
            crate::cancel::every(k, step);
            visit(a, b, &mut out);
        }
        out
    }
}

/// Largest leaf of the bulk-loaded R-tree whose nodes and search order [`Packed`] keeps.
const LEAF: usize = 6;

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

/// A node of a [`Packed`] index.
#[derive(Debug, Clone, Copy)]
struct Node {
    bounds: [f64; 4],
    /// The first node after the ones below it.
    skip: usize,
    /// Its entries, none if it has nodes below it.
    start: usize,
    end: usize,
}
/// A static box index with the nodes of an R-tree bulk loaded from the same
/// entries, so it finds the same items in the same order with the same work,
/// without an allocation per node. It builds on several threads and stops when
/// cancelled.
#[derive(Debug)]
pub(crate) struct Packed {
    /// Entries in search order.
    entries: Vec<Entry>,
    /// Nodes depth first, each before the ones below it, which are in search order.
    nodes: Vec<Node>,
}
impl Packed {
    pub(crate) fn bulk(mut entries: Vec<Entry>, workers: &Workers) -> Self {
        arrange(&mut entries, workers);
        Self::ordered(entries, workers)
    }
    /// An index over entries already in search order. The nodes depend only on
    /// how many there are.
    pub(crate) fn ordered(entries: Vec<Entry>, workers: &Workers) -> Self {
        let mut nodes = Vec::new();
        if !entries.is_empty() {
            shape(&mut nodes, 0, entries.len());
        }
        // Leaves from their entries, then each node from the ones below it.
        workers.update(&mut nodes, |_, node| {
            if let Some(bounds) = entries[node.start..node.end]
                .iter()
                .map(Entry::bounds)
                .reduce(merge)
            {
                node.bounds = bounds;
            }
        });
        for i in (0..nodes.len()).rev() {
            crate::cancel::every(i, "index.bounds");
            let node = nodes[i];
            if node.start == node.end {
                let mut below = i + 1;
                let mut bounds = nodes[below].bounds;
                while nodes[below].skip < node.skip {
                    below = nodes[below].skip;
                    bounds = merge(bounds, nodes[below].bounds);
                }
                nodes[i].bounds = bounds;
            }
        }
        Self { entries, nodes }
    }
    /// Entries in search order.
    #[cfg(test)]
    pub(crate) fn entries(&self) -> &[Entry] {
        &self.entries
    }
    /// Item IDs in search order, if they all fit.
    pub(crate) fn order(&self, step: &'static str) -> Option<Vec<u32>> {
        let mut order = Vec::with_capacity(self.entries.len());
        for (k, e) in self.entries.iter().enumerate() {
            crate::cancel::every(k, step);
            order.push(u32::try_from(e.id).ok()?);
        }
        Some(order)
    }
    /// Entries whose box intersects `bounds`, in search order.
    pub(crate) fn locate(&self, bounds: AABB<[f64; 2]>) -> Locate<'_> {
        Locate {
            tree: self,
            query: [
                bounds.lower()[0],
                bounds.lower()[1],
                bounds.upper()[0],
                bounds.upper()[1],
            ],
            node: 0,
            leaf: [].iter(),
        }
    }
}
impl Search for Packed {
    fn search(&self, bounds: AABB<[f64; 2]>) -> impl Iterator<Item = (usize, AABB<[f64; 2]>)> {
        self.locate(bounds).map(|e| (e.id, e.envelope()))
    }
    fn items(&self) -> impl Iterator<Item = (usize, Rect)> {
        self.entries.iter().map(|e| (e.id, e.rect()))
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
                crate::cancel::every(work, step);
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
            crate::cancel::checkpoint("pairs.join");
            out.extend(part);
        }
        out
    }
}
/// Push the nodes of an R-tree bulk loaded from `len` entries, for the entries
/// from `start` on that [`arrange`] puts in its search order.
fn shape(nodes: &mut Vec<Node>, start: usize, len: usize) {
    crate::cancel::every(nodes.len(), "index.nodes");
    let at = nodes.len();
    nodes.push(Node {
        bounds: [0.; 4],
        skip: at + 1,
        start,
        end: start,
    });
    if len <= LEAF {
        nodes[at].end = start + len;
        return;
    }
    let clusters = clusters_per_axis(len);
    let mut first = start;
    for slab in sizes(len, clusters) {
        for size in sizes(slab, clusters) {
            shape(nodes, first, size);
            first += size;
        }
    }
    nodes[at].skip = nodes.len();
}
/// Search of a [`Packed`] index, depth first.
pub(crate) struct Locate<'a> {
    tree: &'a Packed,
    query: [f64; 4],
    /// The next node to look at.
    node: usize,
    /// The rest of the leaf being searched.
    leaf: std::slice::Iter<'a, Entry>,
}
impl<'a> Iterator for Locate<'a> {
    type Item = &'a Entry;
    fn next(&mut self) -> Option<&'a Entry> {
        loop {
            let query = self.query;
            if let Some(entry) = self.leaf.find(|e| intersects(e.bounds(), query)) {
                return Some(entry);
            }
            let node = self.tree.nodes.get(self.node)?;
            if intersects(node.bounds, query) {
                self.node += 1;
                self.leaf = self.tree.entries[node.start..node.end].iter();
            } else {
                self.node = node.skip;
            }
        }
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
        crate::cancel::every(*count, "index.arrange");
        *count += 1;
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
    let mut rest = entries;
    for size in sizes(rest.len(), clusters) {
        if rest.len() > size {
            select(rest, size, axis, count);
        }
        let (slab, tail) = std::mem::take(&mut rest).split_at_mut(size);
        f(slab, count);
        rest = tail;
    }
}
/// Sizes of the slabs of `len` entries: as many full ones as fit, then the rest.
fn sizes(len: usize, clusters: usize) -> impl Iterator<Item = usize> {
    let size = len.div_ceil(clusters).max(1);
    (0..len)
        .step_by(size)
        .map(move |start| size.min(len - start))
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

    /// Bounds and leaf items of an R-tree's nodes, depth first in search order.
    fn tree_nodes(tree: &Index) -> Vec<([f64; 4], Vec<usize>)> {
        use rstar::{ParentNode, RTreeNode, RTreeObject};
        fn walk<T: RTreeObject<Envelope = AABB<[f64; 2]>>>(
            node: &ParentNode<T>,
            id: &impl Fn(&T) -> usize,
            out: &mut Vec<([f64; 4], Vec<usize>)>,
        ) {
            let bounds = node.envelope();
            let (lower, upper) = (bounds.lower(), bounds.upper());
            // A search takes the children last first.
            let leaves = node
                .children()
                .iter()
                .rev()
                .filter_map(|child| match child {
                    RTreeNode::Leaf(item) => Some(id(item)),
                    RTreeNode::Parent(_) => None,
                });
            out.push(([lower[0], lower[1], upper[0], upper[1]], leaves.collect()));
            for child in node.children().iter().rev() {
                if let RTreeNode::Parent(parent) = child {
                    walk(parent, id, out);
                }
            }
        }
        let mut out = Vec::new();
        if tree.size() > 0 {
            walk(tree.root(), &|item| item.data, &mut out);
        }
        out
    }

    #[test]
    fn packed_nodes_are_those_of_the_bulk_loaded_r_tree() {
        let sizes = [0, 1, 6, 7, 36, 37, 215, 216, 217, 1296, 1297, 5000, 20_000];
        for (k, &n) in sizes.iter().enumerate() {
            let items = boxes(n, k as u64 + 40);
            let expected = tree_nodes(&build(items.clone()));
            for threads in [1, 4] {
                let workers = Workers::forced(threads, 16, || Workers::new(0));
                let entries = items.iter().map(|&(i, r)| Entry::new(i, r)).collect();
                let packed = Packed::bulk(entries, &workers);
                // The nodes depend only on the number of entries.
                let again = Packed::ordered(packed.entries.clone(), &workers);
                for packed in [&packed, &again] {
                    let found = packed
                        .nodes
                        .iter()
                        .map(|node| {
                            let leaf = &packed.entries[node.start..node.end];
                            (node.bounds, leaf.iter().map(|e| e.id).collect::<Vec<_>>())
                        })
                        .collect::<Vec<_>>();
                    assert_eq!(found, expected, "{n} boxes on {threads} threads");
                }
            }
        }
    }

    #[test]
    fn ordering_and_pairing_stop_when_cancelled() {
        use crate::cancel::{self, hooks};
        let entries = boxes(1 << 15, 9)
            .into_iter()
            .map(|(i, r)| Entry::new(i, r))
            .collect::<Vec<_>>();
        let run = |token: &cancel::CancellationToken| {
            std::panic::catch_unwind(|| {
                cancel::scope(Some(token.clone()), || {
                    let workers = Workers::forced(4, 64, || Workers::new(0));
                    let packed = Packed::bulk(entries.clone(), &workers);
                    assert!(packed.order("order").is_some());
                    collect_pairs(&packed, &workers, 0.5)
                })
            })
        };
        let token = hooks::watched(None);
        run(&token).expect("pairs");
        let visits = hooks::record(&token).visits;
        // Every checkpoint is passed, so one taken out shows.
        let steps = visits.keys().copied().collect::<Vec<_>>();
        assert_eq!(
            steps,
            [
                "fill",
                "index.arrange",
                "index.bounds",
                "index.nodes",
                "order",
                "pairs",
                "pairs.join"
            ]
        );
        for (step, count) in visits {
            for visit in [0, count / 2] {
                let token = hooks::watched(Some((step, visit)));
                let result = run(&token);
                let record = hooks::record(&token);
                assert!(
                    result.expect_err("cancelled").is::<cancel::Cancelled>(),
                    "{step} {visit}"
                );
                // Each other thread passes at most one more checkpoint.
                let visits = record.visits[step];
                assert!(
                    (visit + 1..=visit + 4).contains(&visits),
                    "{step} {visit}: {visits}"
                );
                assert_eq!(record.live, 0);
                assert!(record.workers > 0 || step != "pairs", "{step} {visit}");
            }
        }
    }

    #[test]
    fn candidate_searches_stop_on_any_thread_when_cancelled() {
        use crate::cancel::{self, hooks};
        let at = |x: usize, y: f64| {
            let corner = Coord { x: x as f64, y };
            Rect::new(corner, corner)
        };
        let many = 4 * cancel::CHECK;
        let row = build((0..many).map(|i| (i, at(i, 0.))));
        // Many boxes without candidates, then one box with many.
        let cases = [
            ((0..many).map(|i| (i, at(i, 5.))).collect::<Vec<_>>(), 0),
            (
                vec![(0, Rect::new(at(0, 0.).min(), at(many, 0.).max()))],
                many,
            ),
        ];
        for (boxes, pairs) in cases {
            assert_eq!(
                candidates(boxes.clone().into_iter(), &row, 0.).count(),
                pairs
            );
            let token = hooks::watched(Some(("candidates", 2)));
            let search = cancel::scope(Some(token.clone()), || {
                candidates(boxes.into_iter(), &row, 0.)
            });
            // The thread that runs the search has no token of its own.
            let result = std::thread::scope(|scope| {
                scope
                    .spawn(|| {
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| search.count()))
                    })
                    .join()
                    .unwrap()
            });
            assert!(result.expect_err("cancelled").is::<cancel::Cancelled>());
            assert_eq!(hooks::record(&token).visits["candidates"], 3);
        }
    }

    #[test]
    fn nearby_searches_in_the_order_of_a_bulk_loaded_tree() {
        for (n, seed) in [(0, 1), (1, 2), (7, 3), (500, 4), (5000, 5)] {
            let a = boxes(n, seed);
            let b = build(boxes(2000, seed + 100));
            let expected = pairs(&build(a.clone()), &b, 0.5, false).collect::<Vec<_>>();
            for threads in [1, 4] {
                let found = Workers::forced(threads, 64, || {
                    nearby(&arranged(n, |i| Some(a[i].1)), &b, 0.5).collect::<Vec<_>>()
                });
                assert_eq!(found, expected, "{n} on {threads}");
            }
            // Items without a box are left out.
            let some = |i: usize| (!i.is_multiple_of(3)).then_some(a[i].1);
            let mut found = nearby(&arranged(n, some), &b, 0.5).collect::<Vec<_>>();
            let mut expected = expected;
            expected.retain(|&(i, _)| !i.is_multiple_of(3));
            found.sort_unstable();
            expected.sort_unstable();
            assert_eq!(found, expected, "{n} with some");
        }
    }

    #[test]
    fn nearby_stops_in_each_step_when_cancelled() {
        use crate::cancel::{self, hooks};
        let threads = 4;
        let a = boxes(20_000, 6);
        let b = build(boxes(2000, 7));
        let run = |token: &cancel::CancellationToken| {
            std::panic::catch_unwind(|| {
                cancel::scope(Some(token.clone()), || {
                    Workers::forced(threads, 64, || {
                        nearby(&arranged(a.len(), |i| Some(a[i].1)), &b, 0.).count()
                    })
                })
            })
        };
        let token = hooks::watched(None);
        run(&token).expect("pairs");
        let visits = hooks::record(&token).visits;
        // Every checkpoint is passed, so one taken out shows.
        let steps = visits.keys().copied().collect::<Vec<_>>();
        assert_eq!(steps, ["arranged", "candidates", "fill", "index.arrange"]);
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

    #[test]
    fn packed_indexes_find_what_a_built_r_tree_finds() {
        for (n, seed) in [(0, 11), (1, 12), (7, 13), (5000, 14)] {
            let items = boxes(n, seed);
            // Items without a box are left out of both.
            let some = |i: usize| (!i.is_multiple_of(5)).then_some(items[i].1);
            let tree = build((0..n).filter_map(|i| some(i).map(|rect| (i, rect))));
            let queries = boxes(300, seed + 50);
            let a = arranged(queries.len(), |i| Some(queries[i].1));
            for threads in [1, 4] {
                let index = Workers::forced(threads, 64, || packed(n, "packed", some));
                for &(_, rect) in &queries {
                    let query = expanded(rect, 0.5);
                    assert!(
                        index.search(query).eq(tree.search(query)),
                        "{n} on {threads}"
                    );
                }
                assert!(nearby(&a, &index, 0.5).eq(nearby(&a, &tree, 0.5)));
            }
        }
    }

    #[test]
    fn packed_indexes_stop_in_each_step_when_cancelled() {
        use crate::cancel::{self, hooks};
        let threads = 4;
        let items = boxes(20_000, 15);
        let run = |token: &cancel::CancellationToken| {
            std::panic::catch_unwind(|| {
                cancel::scope(Some(token.clone()), || {
                    Workers::forced(threads, 64, || {
                        packed(items.len(), "packed", |i| Some(items[i].1))
                            .entries()
                            .len()
                    })
                })
            })
        };
        let token = hooks::watched(None);
        assert_eq!(run(&token).expect("index"), items.len());
        let visits = hooks::record(&token).visits;
        // Every checkpoint is passed, so one taken out shows.
        let steps = visits.keys().copied().collect::<Vec<_>>();
        assert_eq!(
            steps,
            [
                "fill",
                "index.arrange",
                "index.bounds",
                "index.nodes",
                "packed"
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
