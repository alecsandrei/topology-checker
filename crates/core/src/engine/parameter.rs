//! Exact positions along line segments for zero-tolerance decisions.
//!
//! Each cut keeps certified bounds on its segment parameter. A proper crossing
//! computes its rational parameter once, because its output coordinate needs
//! it. An input coordinate on a segment, and a window midpoint, use rationals
//! only when the certified bounds cannot decide.
use super::{
    arrangement::{self, Exact},
    certified::Interval,
    line_graph::exact_crossing,
    location::Location,
};
use geo::{Coord, Line};
use num_rational::BigRational as Q;
use num_traits::{One, ToPrimitive, Zero};
use std::{cell::OnceCell, cmp::Ordering};

#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    Start,
    End,
    /// An input coordinate on the segment interior.
    On,
    /// A proper crossing with another segment.
    Crossing,
}

/// A cut at an exact parameter `t` in `[0, 1]` along `line`.
#[derive(Debug)]
pub(crate) struct Parameter {
    pub segment: usize,
    pub line: Line,
    /// Output coordinate: the input coordinate, or the nearest binary
    /// coordinate of the exact crossing.
    pub point: Coord,
    kind: Kind,
    bounds: Interval,
    exact: OnceCell<Q>,
    /// Exact coordinates of a crossing.
    crossing: Option<Box<[Q; 2]>>,
}

fn point(v: f64) -> Interval {
    Interval(v, v)
}
fn q(v: f64) -> Q {
    Q::from_float(v).expect("finite coordinate")
}
/// Order of a finite binary value against a rational, by integer comparison
/// of `m 2^e d` with `n`, without normalizing the binary value.
fn order(v: f64, exact: &Q) -> Ordering {
    let bits = v.to_bits();
    let biased = ((bits >> 52) & 0x7ff) as i64;
    let fraction = bits & ((1 << 52) - 1);
    let (mantissa, exponent) = if biased == 0 {
        (fraction, -1074)
    } else {
        (fraction | 1 << 52, biased - 1075)
    };
    let mut m = q(mantissa as f64).numer().clone();
    if v.is_sign_negative() {
        m = -m;
    }
    let (n, d) = (exact.numer(), exact.denom());
    if exponent >= 0 {
        ((m << exponent as usize) * d).cmp(n)
    } else {
        (m * d).cmp(&(n << (-exponent) as usize))
    }
}

impl Parameter {
    pub fn start(segment: usize, line: Line) -> Self {
        Self::vertex(segment, line, Kind::Start)
    }
    pub fn end(segment: usize, line: Line) -> Self {
        Self::vertex(segment, line, Kind::End)
    }
    fn vertex(segment: usize, line: Line, kind: Kind) -> Self {
        let (t, point) = if kind == Kind::Start {
            (0., line.start)
        } else {
            (1., line.end)
        };
        Self {
            segment,
            line,
            point,
            kind,
            bounds: Interval(t, t),
            exact: OnceCell::new(),
            crossing: None,
        }
    }
    /// An input coordinate lying exactly on `line`, as returned by a
    /// non-proper or collinear contact.
    pub fn on(segment: usize, line: Line, p: Coord) -> Self {
        if p == line.start {
            return Self::start(segment, line);
        }
        if p == line.end {
            return Self::end(segment, line);
        }
        let dx = point(line.end.x).sub(point(line.start.x));
        let dy = point(line.end.y).sub(point(line.start.y));
        let along = point(p.x)
            .sub(point(line.start.x))
            .mul(dx)
            .add(point(p.y).sub(point(line.start.y)).mul(dy));
        Self {
            segment,
            line,
            point: p,
            kind: Kind::On,
            bounds: along.div(dx.square().add(dy.square())),
            exact: OnceCell::new(),
            crossing: None,
        }
    }
    /// A proper crossing of `line` with `other`.
    pub fn crossing(segment: usize, line: Line, other: Line) -> Option<Self> {
        let (t, [x, y]) = exact_crossing(line, other)?;
        let point = Coord {
            x: x.to_f64()?,
            y: y.to_f64()?,
        };
        // Correct rounding encloses a normal value within one step.
        let r = t.to_f64()?;
        let bounds = if r.abs() >= f64::MIN_POSITIVE {
            Interval(r.next_down(), r.next_up())
        } else {
            Interval(-f64::MIN_POSITIVE, f64::MIN_POSITIVE)
        };
        Some(Self {
            segment,
            line,
            point,
            kind: Kind::Crossing,
            bounds,
            exact: OnceCell::from(t),
            crossing: Some(Box::new([x, y])),
        })
    }
    /// The same location as the next segment's start, so a shared vertex has
    /// one parameter.
    pub fn normalized(self, lines: &[Line]) -> Self {
        if self.kind == Kind::End && self.segment + 1 < lines.len() {
            Self::start(self.segment + 1, lines[self.segment + 1])
        } else {
            self
        }
    }
    /// Output coordinate for the start (`forward == false`) or end of an
    /// emitted run. Input coordinates are kept. A crossing is rounded away
    /// from the run on each axis along its segment direction, so the emitted
    /// piece covers the exact run and never passes the segment's vertices.
    pub fn outward(&self, forward: bool) -> Coord {
        let Some(exact) = &self.crossing else {
            return self.point;
        };
        // The nearest value is kept when it already lies on the outer side; a
        // constant coordinate is exact, so its direction is irrelevant.
        let directed = |v: f64, exact: &Q, from: f64, to: f64| match (to > from) == forward {
            true if order(v, exact).is_lt() => v.next_up(),
            false if order(v, exact).is_gt() => v.next_down(),
            _ => v,
        };
        let (a, b) = (self.line.start, self.line.end);
        Coord {
            x: directed(self.point.x, &exact[0], a.x, b.x),
            y: directed(self.point.y, &exact[1], a.y, b.y),
        }
    }
    pub(crate) fn exact(&self) -> &Q {
        self.exact.get_or_init(|| match self.kind {
            Kind::Start => Q::zero(),
            Kind::End => Q::one(),
            Kind::On => {
                let (dx, dy) = (
                    q(self.line.end.x) - q(self.line.start.x),
                    q(self.line.end.y) - q(self.line.start.y),
                );
                ((q(self.point.x) - q(self.line.start.x)) * &dx
                    + (q(self.point.y) - q(self.line.start.y)) * &dy)
                    / (&dx * &dx + &dy * &dy)
            }
            Kind::Crossing => unreachable!("crossings store their rational parameter"),
        })
    }
    /// Order along the line part: segment, then exact parameter.
    pub fn cmp(&self, other: &Self) -> Ordering {
        self.segment.cmp(&other.segment).then_with(|| {
            let (a, b) = (self.bounds, other.bounds);
            if a.1 < b.0 {
                Ordering::Less
            } else if a.0 > b.1 {
                Ordering::Greater
            } else if a.0 == a.1 && b.0 == b.1 && a.0 == b.0
                || self.kind != Kind::Crossing
                    && other.kind != Kind::Crossing
                    && self.point == other.point
            {
                // Vertices have degenerate bounds; an input coordinate has one
                // parameter on its segment.
                Ordering::Equal
            } else {
                self.exact().cmp(other.exact())
            }
        })
    }
}

/// Position of the open window between consecutive cuts `a < b`: 1 inside,
/// 0 on the boundary, -1 outside. `b` may be the next segment's start.
pub(crate) fn window_position(location: &Location, a: &Parameter, b: &Parameter) -> i8 {
    let line = a.line;
    let same = b.segment == a.segment;
    let end = if same { b.bounds } else { Interval(1., 1.) };
    let t = a.bounds.add(end).mul(Interval(0.5, 0.5));
    let p = [
        point(line.start.x).add(t.mul(point(line.end.x).sub(point(line.start.x)))),
        point(line.start.y).add(t.mul(point(line.end.y).sub(point(line.start.y)))),
    ];
    if let Some(position) = arrangement::certified_position(location, p) {
        return position;
    }
    let end = if same { b.exact().clone() } else { Q::one() };
    let t = (a.exact() + end) / Q::from_integer(2.into());
    let (sx, sy) = (q(line.start.x), q(line.start.y));
    let x = &sx + &t * (q(line.end.x) - &sx);
    let y = &sy + &t * (q(line.end.y) - &sy);
    arrangement::position_exact(location, &Exact::rational(x, y))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn vertical(x: f64) -> Line {
        Line::new(Coord { x, y: -1. }, Coord { x, y: 2. })
    }
    #[test]
    fn parameters_order_by_exact_value_when_bounds_overlap() {
        let line = Line::new(Coord { x: 0., y: 0. }, Coord { x: 3., y: 1. });
        let lines = [line];
        // Two crossings a few ulps apart along the segment, and an input point.
        let first = Parameter::crossing(0, line, vertical(1.)).unwrap();
        let second = Parameter::crossing(0, line, vertical(1f64.next_up())).unwrap();
        assert_eq!(first.cmp(&second), Ordering::Less);
        assert_eq!(second.cmp(&first), Ordering::Greater);
        let on = Parameter::on(0, line, Coord { x: 1.5, y: 0.5 });
        assert_eq!(
            on.cmp(&Parameter::on(0, line, Coord { x: 1.5, y: 0.5 })),
            Ordering::Equal
        );
        assert_eq!(first.cmp(&on), Ordering::Less);
        assert_eq!(
            Parameter::end(0, line)
                .normalized(&lines)
                .cmp(&Parameter::end(0, line)),
            Ordering::Equal
        );
        assert_eq!(
            Parameter::on(0, line, line.end).cmp(&Parameter::end(0, line)),
            Ordering::Equal
        );
    }
    #[test]
    fn run_ends_round_away_from_the_run_and_keep_input_coordinates() {
        // The crossing at x = 1 has y = 1/3, which is not representable.
        let line = Line::new(Coord { x: 0., y: 0. }, Coord { x: 3., y: 1. });
        let crossing = Parameter::crossing(0, line, vertical(1.)).unwrap();
        let third = 1. / 3.;
        assert_eq!(crossing.point, Coord { x: 1., y: third });
        let (before, after) = (crossing.outward(false), crossing.outward(true));
        assert_eq!(before.x, 1.);
        assert_eq!(after.x, 1.);
        // The exact value lies between them, one step apart.
        assert!(q(before.y) < Q::new(1.into(), 3.into()));
        assert!(q(after.y) > Q::new(1.into(), 3.into()));
        assert_eq!(after.y, before.y.next_up());
        // Reversing the segment swaps the two roundings.
        let reversed = Line::new(line.end, line.start);
        let crossing = Parameter::crossing(0, reversed, vertical(1.)).unwrap();
        assert_eq!(crossing.outward(false), after);
        assert_eq!(crossing.outward(true), before);
        let on = Parameter::on(0, line, Coord { x: 1.5, y: 0.5 });
        assert_eq!(on.outward(false), on.point);
        assert_eq!(on.outward(true), on.point);
    }
    #[test]
    fn binary_values_order_against_rationals_like_exact_comparison() {
        let third = Q::new(1.into(), 3.into());
        let values = [
            0.,
            -0.,
            f64::from_bits(1),
            -f64::MIN_POSITIVE,
            1. / 3.,
            -1. / 3.,
            623757.44,
            -4.8e6,
            1e300,
            f64::MAX.next_down(),
        ];
        for v in values {
            for exact in [
                q(v),
                third.clone(),
                -third.clone(),
                q(623757.44),
                q(v.next_up()),
            ] {
                assert_eq!(order(v, &exact), q(v).cmp(&exact), "{v} against {exact}");
            }
        }
    }
}
