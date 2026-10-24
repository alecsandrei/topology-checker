use geo::Coord;
use std::{
    cmp::Ordering,
    collections::{BTreeMap, HashMap},
};

pub(crate) fn bits(value: f64) -> u64 {
    if value == 0. { 0 } else { value.to_bits() }
}
pub(crate) fn distance(a: Coord, b: Coord) -> f64 {
    (a.x - b.x).hypot(a.y - b.y)
}

/// Stable representative clustering, with a fixed input order and no transitive snapping.
pub(crate) struct Cluster {
    tolerance: f64,
    exact: HashMap<(u64, u64), usize>,
    cells: HashMap<(i64, i64), Vec<usize>>,
    pub(crate) coords: Vec<Coord>,
    members: Vec<Vec<Coord>>,
}
impl Cluster {
    pub(crate) fn new(tolerance: f64) -> Self {
        Self {
            tolerance,
            exact: HashMap::new(),
            cells: HashMap::new(),
            coords: Vec::new(),
            members: Vec::new(),
        }
    }
    #[cfg(test)]
    pub fn finish(self) -> Vec<Coord> {
        self.coords
    }
    /// Look up the node of an already inserted coordinate.
    #[cfg(test)]
    pub(crate) fn get(&self, p: Coord) -> Option<usize> {
        self.exact.get(&(bits(p.x), bits(p.y))).copied()
    }
    fn cell(&self, p: Coord) -> (i64, i64) {
        (
            (p.x / self.tolerance).floor() as i64,
            (p.y / self.tolerance).floor() as i64,
        )
    }
    pub(crate) fn insert(&mut self, p: Coord) -> usize {
        let key = (bits(p.x), bits(p.y));
        if let Some(&id) = self.exact.get(&key) {
            return id;
        }
        if self.tolerance > 0. {
            let (x, y) = self.cell(p);
            let mut best: Option<(f64, usize)> = None;
            for dx in -1_i64..=1 {
                for dy in -1_i64..=1 {
                    if let Some(ids) = self
                        .cells
                        .get(&(x.saturating_add(dx), y.saturating_add(dy)))
                    {
                        for &id in ids {
                            let d = distance(p, self.coords[id]);
                            if d <= self.tolerance
                                && self.members[id]
                                    .iter()
                                    .all(|&member| distance(p, member) <= self.tolerance)
                                && best.is_none_or(|old| (d, id) < old)
                            {
                                best = Some((d, id));
                            }
                        }
                    }
                }
            }
            if let Some((_, id)) = best {
                self.members[id].push(p);
                self.exact.insert(key, id);
                return id;
            }
        }
        let id = self.coords.len();
        self.coords.push(p);
        if self.tolerance > 0. {
            self.members.push(vec![p]);
        }
        self.exact.insert(key, id);
        if self.tolerance > 0. {
            self.cells.entry(self.cell(p)).or_default().push(id);
        }
        id
    }
}

/// Points of one cell column, searched by row.
#[derive(Debug, Default)]
struct Column {
    x: i64,
    /// Rows and representatives, until there are too many to scan.
    few: Vec<(i64, usize)>,
    rows: BTreeMap<i64, Vec<usize>>,
}
impl Column {
    const FEW: usize = 32;
    fn new(x: i64) -> Self {
        Self {
            x,
            ..Self::default()
        }
    }
    fn push(&mut self, y: i64, id: usize) {
        if self.rows.is_empty() && self.few.len() < Self::FEW {
            self.few.push((y, id));
            return;
        }
        for (y, id) in self.few.drain(..) {
            self.rows.entry(y).or_default().push(id);
        }
        self.rows.entry(y).or_default().push(id);
    }
    fn near(&self, y: i64, mut f: impl FnMut(usize)) {
        if self.rows.is_empty() {
            for &(row, id) in &self.few {
                if (i128::from(row) - i128::from(y)).abs() <= 1 {
                    f(id);
                }
            }
        } else {
            for ids in self
                .rows
                .range(y.saturating_sub(1)..=y.saturating_add(1))
                .map(|(_, ids)| ids)
            {
                ids.iter().copied().for_each(&mut f);
            }
        }
    }
}

/// [`Cluster`] for points inserted in batches, each in [`order`]: the same
/// nodes, in the same order. Points are found by walking sorted lists rather
/// than by hashing, and nodes allocate nothing of their own.
#[derive(Debug)]
pub(crate) struct Sweep {
    tolerance: f64,
    coords: Vec<Coord>,
    /// Points joined to a representative, each linked to the one joined before.
    joined: Vec<(Coord, usize)>,
    /// Last point joined to each representative that has any.
    last_joined: BTreeMap<usize, usize>,
    /// Nodes of points with a zero coordinate, whose signed zeros sort apart.
    zeros: HashMap<(u64, u64), usize>,
    /// Other points of earlier batches with their nodes, in order.
    earlier: Vec<(Coord, usize)>,
    /// Cells and IDs of earlier batches' representatives, in order.
    cells: Vec<(i64, i64, usize)>,
}
/// Sort order of [`Sweep`] batches.
pub(crate) fn order(a: &Coord, b: &Coord) -> Ordering {
    a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y))
}
fn identical(a: Coord, b: Coord) -> bool {
    a.x.to_bits() == b.x.to_bits() && a.y.to_bits() == b.y.to_bits()
}
/// Points between cancellation checks.
const SWEEP_CHECK: usize = 1 << 16;
impl Sweep {
    pub(crate) fn new(tolerance: f64) -> Self {
        Self {
            tolerance,
            coords: Vec::new(),
            joined: Vec::new(),
            last_joined: BTreeMap::new(),
            zeros: HashMap::new(),
            earlier: Vec::new(),
            cells: Vec::new(),
        }
    }
    pub(crate) fn finish(self) -> Vec<Coord> {
        self.coords
    }
    fn cell(&self, p: Coord) -> (i64, i64) {
        (
            (p.x / self.tolerance).floor() as i64,
            (p.y / self.tolerance).floor() as i64,
        )
    }
    /// Whether `p` lies within tolerance of every point of a representative.
    fn fits(&self, id: usize, p: Coord) -> bool {
        let mut next = self.last_joined.get(&id).copied();
        while let Some(i) = next {
            let (member, before) = self.joined[i];
            if distance(p, member) > self.tolerance {
                return false;
            }
            next = (before != usize::MAX).then_some(before);
        }
        true
    }
    /// Insert the points of `items`, sorted by [`order`], and `assign` each its
    /// node. Points of the last batch are not kept for later lookups.
    pub(crate) fn batch<T>(
        &mut self,
        items: &[T],
        point: impl Fn(&T) -> Coord,
        last: bool,
        step: &'static str,
        mut assign: impl FnMut(&T, usize),
    ) {
        let tolerance = self.tolerance;
        let clustered = tolerance > 0.;
        let mut earlier = 0;
        let mut previous: Option<(Coord, usize)> = None;
        let mut found = Vec::new();
        let mut made = Vec::new();
        // Representatives of this batch in the column before the current one
        // and in it; earlier columns are out of reach.
        let mut columns: [Option<Column>; 2] = [None, None];
        // Earlier representatives in the columns within reach.
        let mut window = 0..0;
        for (k, item) in items.iter().enumerate() {
            if k % SWEEP_CHECK == 0 {
                crate::cancel::checkpoint(step);
            }
            let p = point(item);
            debug_assert!(previous.is_none_or(|(q, _)| order(&q, &p).is_le()));
            if let Some((q, id)) = previous
                && identical(p, q)
            {
                assign(item, id);
                continue;
            }
            let zero = p.x == 0. || p.y == 0.;
            let key = (bits(p.x), bits(p.y));
            let known = if zero {
                self.zeros.get(&key).copied()
            } else {
                while self
                    .earlier
                    .get(earlier)
                    .is_some_and(|(q, _)| order(q, &p).is_lt())
                {
                    earlier += 1;
                }
                self.earlier
                    .get(earlier)
                    .filter(|(q, _)| identical(*q, p))
                    .map(|&(_, id)| id)
            };
            let id = known.unwrap_or_else(|| {
                if !clustered {
                    self.coords.push(p);
                    return self.coords.len() - 1;
                }
                let (x, y) = self.cell(p);
                let current = columns[1].as_ref().map(|c| c.x);
                if current != Some(x) {
                    let next = current.is_some_and(|c| i128::from(c) + 1 == i128::from(x));
                    columns = [columns[1].take().filter(|_| next), Some(Column::new(x))];
                }
                let mut best: Option<(f64, usize)> = None;
                let mut consider = |id: usize| {
                    let d = distance(p, self.coords[id]);
                    if d <= tolerance && self.fits(id, p) && best.is_none_or(|old| (d, id) < old) {
                        best = Some((d, id));
                    }
                };
                let reach = |c: i64| (i128::from(c) - i128::from(x)).abs() <= 1;
                while self
                    .cells
                    .get(window.start)
                    .is_some_and(|c| !reach(c.0) && c.0 < x)
                {
                    window.start += 1;
                }
                window.end = window.end.max(window.start);
                while self.cells.get(window.end).is_some_and(|c| reach(c.0)) {
                    window.end += 1;
                }
                let near = &self.cells[window.clone()];
                if near.len() <= Column::FEW {
                    for &(_, row, id) in near {
                        if (i128::from(row) - i128::from(y)).abs() <= 1 {
                            consider(id);
                        }
                    }
                } else {
                    for column in [x.checked_sub(1), Some(x), x.checked_add(1)]
                        .into_iter()
                        .flatten()
                    {
                        let low = (column, y.saturating_sub(1));
                        let high = (column, y.saturating_add(1));
                        let start = near.partition_point(|c| (c.0, c.1) < low);
                        let end = near.partition_point(|c| (c.0, c.1) <= high);
                        for &(_, _, id) in &near[start..end] {
                            consider(id);
                        }
                    }
                }
                for column in columns.iter().flatten() {
                    column.near(y, &mut consider);
                }
                if let Some((_, id)) = best {
                    let before = self.last_joined.insert(id, self.joined.len());
                    self.joined.push((p, before.unwrap_or(usize::MAX)));
                    return id;
                }
                let id = self.coords.len();
                self.coords.push(p);
                if let [_, Some(current)] = &mut columns {
                    current.push(y, id);
                }
                if !last {
                    made.push((x, y, id));
                }
                id
            });
            if zero {
                self.zeros.entry(key).or_insert(id);
            } else if known.is_none() && !last {
                found.push((p, id));
            }
            previous = Some((p, id));
            assign(item, id);
        }
        if last {
            return;
        }
        self.earlier = merge(std::mem::take(&mut self.earlier), found, |a, b| {
            order(&a.0, &b.0)
        });
        made.sort_unstable();
        self.cells = merge(std::mem::take(&mut self.cells), made, Ord::cmp);
    }
}
/// Merge two sorted lists.
fn merge<T>(a: Vec<T>, b: Vec<T>, compare: impl Fn(&T, &T) -> Ordering) -> Vec<T> {
    if a.is_empty() {
        return b;
    }
    let mut out = Vec::with_capacity(a.len() + b.len());
    let mut b = b.into_iter().peekable();
    for x in a {
        while let Some(y) = b.next_if(|y| compare(y, &x).is_lt()) {
            out.push(y);
        }
        out.push(x);
    }
    out.extend(b);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn signed_zero_and_adjacent_cells() {
        let mut exact = Cluster::new(0.);
        assert_eq!(
            exact.insert(Coord { x: 0., y: 0. }),
            exact.insert(Coord { x: -0., y: -0. })
        );
        let mut c = Cluster::new(1.);
        assert_eq!(
            c.insert(Coord { x: 0.99, y: 0. }),
            c.insert(Coord { x: 1.01, y: 0. })
        );
        assert_ne!(c.insert(Coord { x: 4., y: 0. }), 0);
    }

    #[test]
    fn sweep_batches_give_the_nodes_of_ordered_insertion() {
        let mut state = 0x2545_f491_4f6c_dd1d_u64;
        let mut next = move |n: u64| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state % n
        };
        for case in 0..600 {
            let tolerance = [0., 1e-9, 0.25, 1., 3., 1e6][case % 6];
            let (scale, offset): (u64, f64) =
                [(8, 0.), (64, 0.), (2, 1e15), (4096, -3.), (1, 1e300)][case / 6 % 5];
            // Few columns with many rows, or the reverse, past the scan limit.
            let shape = case / 30 % 3;
            let mut value = |wide: bool| {
                let range = if wide { 40 * scale } else { scale };
                let v = next(range) as f64 / 4. - (range / 8) as f64;
                match next(9) {
                    0 => -0.,
                    1 => 0.,
                    _ if offset > 1e100 => v * offset + offset,
                    _ => v + offset,
                }
            };
            let mut batches: Vec<Vec<Coord>> = (0..3)
                .map(|b| {
                    (0..[300, 40, 300][b] + 1)
                        .map(|_| Coord {
                            x: value(shape == 1),
                            y: value(shape == 2),
                        })
                        .collect()
                })
                .collect();
            // Repeats across batches.
            let repeat = batches[0][..batches[0].len().min(20)].to_vec();
            batches[2].extend(repeat);
            let mut cluster = Cluster::new(tolerance);
            let mut sweep = Sweep::new(tolerance);
            for (b, batch) in batches.iter_mut().enumerate() {
                batch.sort_by(order);
                if b == 1 {
                    batch.dedup();
                }
                let expected = batch.iter().map(|&p| cluster.insert(p)).collect::<Vec<_>>();
                let mut ids = Vec::new();
                sweep.batch(batch, |&p| p, b == 2, "cluster", |_, id| ids.push(id));
                assert_eq!(ids, expected, "case {case} batch {b}");
            }
            assert_eq!(
                format!("{:?}", sweep.finish()),
                format!("{:?}", cluster.finish()),
                "case {case}"
            );
        }
    }
}
