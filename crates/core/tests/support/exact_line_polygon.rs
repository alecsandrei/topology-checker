//! Exact rational reference for line and polygon contacts at zero tolerance.
//!
//! A line part is cut at every exact contact with the polygon boundary, and
//! each window between cuts is classified at its exact midpoint: overlap runs
//! are maximal runs inside or on the polygon, isolated contacts are boundary
//! cuts off those runs, and crossings are changes between interior and
//! exterior passages, reported at the smallest rounded contact between them.
use num_rational::BigRational as Q;
use num_traits::{One, Signed, ToPrimitive, Zero};
use std::collections::BTreeSet;
use topology_checker_core::geo::Coord;

pub type Point = (Q, Q);

pub fn exact(c: Coord) -> Point {
    (Q::from_float(c.x).unwrap(), Q::from_float(c.y).unwrap())
}
pub fn rounded(p: &Point) -> Coord {
    Coord {
        x: p.0.to_f64().unwrap(),
        y: p.1.to_f64().unwrap(),
    }
}
/// Bit patterns of a coordinate, with negative zero folded into zero.
pub fn bits(c: Coord) -> [u64; 2] {
    [c.x + 0., c.y + 0.].map(f64::to_bits)
}
fn cross(o: &Point, a: &Point, b: &Point) -> Q {
    (&a.0 - &o.0) * (&b.1 - &o.1) - (&a.1 - &o.1) * (&b.0 - &o.0)
}
fn between(a: &Q, b: &Q, m: &Q) -> bool {
    a.min(b) <= m && m <= a.max(b)
}
fn on_segment(c: &Point, d: &Point, m: &Point) -> bool {
    cross(c, d, m).is_zero() && between(&c.0, &d.0, &m.0) && between(&c.1, &d.1, &m.1)
}
fn position(edges: &[(Point, Point)], m: &Point) -> i8 {
    let mut inside = false;
    for (c, d) in edges {
        if on_segment(c, d, m) {
            return 0;
        }
        if (c.1 > m.1) != (d.1 > m.1) {
            let x = &c.0 + (&m.1 - &c.1) * (&d.0 - &c.0) / (&d.1 - &c.1);
            if m.0 < x {
                inside = !inside;
            }
        }
    }
    if inside { 1 } else { -1 }
}
fn at(a: &Point, b: &Point, t: &Q) -> Point {
    (&a.0 + t * (&b.0 - &a.0), &a.1 + t * (&b.1 - &a.1))
}
fn apart(x: &Q, y: &Q) -> bool {
    x.is_positive() && y.is_positive() || x.is_negative() && y.is_negative()
}

/// An overlap run: its exact cut points and the line segments of its first and
/// last windows.
pub struct Run {
    pub points: Vec<Point>,
    pub segments: [usize; 2],
}
pub struct Reference {
    pub runs: Vec<Run>,
    pub isolated: BTreeSet<[u64; 2]>,
    pub crossings: BTreeSet<[u64; 2]>,
}

/// `line` has no repeated consecutive coordinates; `rings` are closed.
pub fn reference(line: &[Coord], rings: &[Vec<Coord>]) -> Reference {
    let edges = rings
        .iter()
        .flat_map(|r| r.windows(2).map(|w| (exact(w[0]), exact(w[1]))))
        .collect::<Vec<_>>();
    let points = line.iter().map(|&c| exact(c)).collect::<Vec<_>>();
    let closed = line.first() == line.last();
    let mut cuts: Vec<(Point, usize)> = Vec::new();
    let mut states = Vec::new();
    for (i, w) in points.windows(2).enumerate() {
        let (a, b) = (&w[0], &w[1]);
        let d = (&b.0 - &a.0, &b.1 - &a.1);
        let dd = &d.0 * &d.0 + &d.1 * &d.1;
        let mut ts = BTreeSet::from([Q::zero(), Q::one()]);
        for (c, e) in &edges {
            let (o1, o2) = (cross(a, b, c), cross(a, b, e));
            if o1.is_zero() && o2.is_zero() {
                let project = |p: &Point| ((&p.0 - &a.0) * &d.0 + (&p.1 - &a.1) * &d.1) / &dd;
                let (tc, te) = (project(c), project(e));
                let low = tc.clone().min(te.clone()).max(Q::zero());
                let high = tc.max(te).min(Q::one());
                if low <= high {
                    ts.insert(low);
                    ts.insert(high);
                }
                continue;
            }
            if apart(&o1, &o2) || apart(&cross(c, e, a), &cross(c, e, b)) {
                continue;
            }
            let denominator = &d.0 * (&e.1 - &c.1) - &d.1 * (&e.0 - &c.0);
            if denominator.is_zero() {
                continue;
            }
            ts.insert(
                ((&c.0 - &a.0) * (&e.1 - &c.1) - (&c.1 - &a.1) * (&e.0 - &c.0)) / denominator,
            );
        }
        let ts = ts.into_iter().collect::<Vec<_>>();
        for t in &ts[..ts.len() - 1] {
            cuts.push((at(a, b, t), i));
        }
        for w in ts.windows(2) {
            let middle = (&w[0] + &w[1]) / Q::from_integer(2.into());
            states.push(position(&edges, &at(a, b, &middle)));
        }
    }
    cuts.push((points.last().unwrap().clone(), points.len() - 2));
    let contact = cuts
        .iter()
        .map(|(p, _)| position(&edges, p) == 0)
        .collect::<Vec<_>>();
    let n = states.len();
    let mut spans: Vec<[usize; 2]> = Vec::new();
    for (k, &state) in states.iter().enumerate() {
        if state >= 0 {
            match spans.last_mut() {
                Some(span) if span[1] == k => span[1] = k + 1,
                _ => spans.push([k, k + 1]),
            }
        }
    }
    if closed && spans.len() > 1 && states[0] >= 0 && states[n - 1] >= 0 {
        let first = spans.remove(0);
        spans.last_mut().unwrap()[1] = first[1] + n;
    }
    // Past the seam, cut `n` is the closing vertex and window `n` is window 0.
    let cut = |k: usize| if k > n { &cuts[k - n] } else { &cuts[k] };
    let window = |w: usize| cuts[w % n].1;
    let runs = spans
        .iter()
        .map(|s| Run {
            points: (s[0]..=s[1]).map(|k| cut(k).0.clone()).collect(),
            segments: [window(s[0]), window(s[1] - 1)],
        })
        .collect::<Vec<_>>();
    let on_run = |p: &Point| {
        runs.iter()
            .any(|r| r.points.windows(2).any(|w| on_segment(&w[0], &w[1], p)))
    };
    let isolated = cuts
        .iter()
        .zip(&contact)
        .filter(|((p, _), touch)| **touch && !on_run(p))
        .map(|((p, _), _)| bits(rounded(p)))
        .collect();
    let passages = states
        .iter()
        .enumerate()
        .filter(|(_, s)| **s != 0)
        .map(|(k, &s)| (k, s))
        .collect::<Vec<_>>();
    let mut crossings = BTreeSet::new();
    for j in 0..passages.len() {
        let before = match j {
            0 if closed => passages[passages.len() - 1],
            0 => continue,
            _ => passages[j - 1],
        };
        let now = passages[j];
        if before.1 * now.1 != -1 {
            continue;
        }
        let exit = before.0 + 1;
        let entry = if now.0 < exit { now.0 + n } else { now.0 };
        let point = (exit..=entry)
            .map(|k| if k > n { k - n } else { k })
            .filter(|&k| contact[k])
            .map(|k| rounded(&cuts[k].0))
            .min_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)))
            .expect("a transition passes a boundary contact");
        if !closed && (point == line[0] || point == *line.last().unwrap()) {
            continue;
        }
        crossings.insert(bits(point));
    }
    Reference {
        runs,
        isolated,
        crossings,
    }
}

/// `out` is the binary value of `e`, or one of the two values adjacent to it.
pub fn brackets(out: f64, e: &Q) -> bool {
    let q = |v: f64| Q::from_float(v).unwrap();
    q(out) == *e || q(out.next_down()) < *e && *e < q(out.next_up())
}
