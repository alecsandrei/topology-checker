use geo::Coord;
use std::collections::HashMap;

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
    pub fn finish(self) -> Vec<Coord> {
        self.coords
    }
    /// Look up the node of an already inserted coordinate.
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
}
