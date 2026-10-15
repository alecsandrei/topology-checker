//! Certified intervals of direct Euclidean proximity on original segments.
//!
//! Capsule transitions are rational values plus or minus one square root.
//! Ordering uses exact arithmetic on the binary inputs. Rounding is reserved
//! for diagnostic coordinates, never for coverage or interval ordering.
use super::certified::Interval;
use geo::{Coord, Line};
use num_rational::BigRational as Q;
use num_traits::{One, Signed, ToPrimitive, Zero};
use std::{cell::OnceCell, cmp::Ordering};

fn q(x: f64) -> Q {
    Q::from_float(x).expect("finite input")
}

/// Inclusive Euclidean coincidence of the original binary coordinates.
pub(crate) fn coincident(a: Coord, b: Coord, tolerance: f64) -> bool {
    if tolerance == 0. {
        return a == b;
    }
    let difference = Interval(a.x, a.x)
        .sub(Interval(b.x, b.x))
        .square()
        .add(Interval(a.y, a.y).sub(Interval(b.y, b.y)).square())
        .sub(Interval(tolerance, tolerance).square());
    if let Some(order) = difference.sign() {
        return order.is_lt();
    }
    let (dx, dy) = (q(a.x) - q(b.x), q(a.y) - q(b.y));
    &dx * &dx + &dy * &dy <= q(tolerance) * q(tolerance)
}

/// Inclusive distance to an original finite segment, with rational fallback.
pub(crate) fn near_segment(point: Coord, line: Line, tolerance: f64) -> bool {
    if tolerance == 0. {
        if point.x < line.start.x.min(line.end.x)
            || point.x > line.start.x.max(line.end.x)
            || point.y < line.start.y.min(line.end.y)
            || point.y > line.start.y.max(line.end.y)
        {
            return false;
        }
        if line.start.x == line.end.x
            || line.start.y == line.end.y
            || point == line.start
            || point == line.end
        {
            return true;
        }
    }
    let v = vector(line.start, line.end);
    let offset = vector(line.start, point);
    let parameter = idot(offset, v).div(idot(v, v));
    let parameter = Interval(parameter.0.clamp(0., 1.), parameter.1.clamp(0., 1.));
    let difference = offset[0]
        .sub(parameter.mul(v[0]))
        .square()
        .add(offset[1].sub(parameter.mul(v[1])).square())
        .sub(Interval(tolerance, tolerance).square());
    if let Some(order) = difference.sign() {
        return order.is_lt();
    }
    let [v, _, p] = exact_vectors(line, Line::new(point, point));
    let length = dot(&v, &v);
    let along = -dot(&p, &v);
    let squared = q(tolerance) * q(tolerance);
    if along <= Q::zero() || length.is_zero() {
        dot(&p, &p) <= squared
    } else if along >= length {
        let offset = [&p[0] + &v[0], &p[1] + &v[1]];
        dot(&offset, &offset) <= squared
    } else {
        let perpendicular = cross(&p, &v);
        &perpendicular * &perpendicular <= squared * length
    }
}

#[derive(Clone, Debug)]
struct ExactCut {
    center: Q,
    coefficient: Q,
    radicand: Q,
}

/// Sign of `a + b sqrt(r)`, without computing the square root.
fn sign(a: &Q, b: &Q, r: &Q) -> Ordering {
    if b.is_zero() || r.is_zero() {
        return a.cmp(&Q::zero());
    }
    if a.is_zero() || a.is_positive() == b.is_positive() {
        return b.cmp(&Q::zero());
    }
    let squares = (a * a).cmp(&(b * b * r));
    if a.is_positive() {
        squares
    } else {
        squares.reverse()
    }
}

impl ExactCut {
    pub(crate) fn rational(value: Q) -> Self {
        Self {
            center: value,
            coefficient: Q::zero(),
            radicand: Q::zero(),
        }
    }
    fn root(center: Q, radicand: Q, positive: bool) -> Self {
        if radicand.is_zero() {
            return Self::rational(center);
        }
        Self {
            center,
            coefficient: if positive { Q::one() } else { -Q::one() },
            radicand,
        }
    }
    pub(crate) fn cmp(&self, other: &Self) -> Ordering {
        // Compare u + b sqrt(r) + c sqrt(s). If the first two terms
        // oppose the third, compare their squares; the remaining expression
        // has only one radical. This also certifies equal algebraic cuts.
        let u = &self.center - &other.center;
        let v = sign(&u, &self.coefficient, &self.radicand);
        let c = -&other.coefficient;
        if c.is_zero() || other.radicand.is_zero() {
            return v;
        }
        let cs = c.cmp(&Q::zero());
        if v == Ordering::Equal || v == cs {
            return cs;
        }
        let a = &u * &u + &self.coefficient * &self.coefficient * &self.radicand
            - &c * &c * &other.radicand;
        let b = Q::from_integer(2.into()) * u * &self.coefficient;
        let squares = sign(&a, &b, &self.radicand);
        if v == Ordering::Greater {
            squares
        } else {
            squares.reverse()
        }
    }
    /// Evaluate one diagnostic coordinate with certified binary bracketing.
    /// The approximation is only a starting guess: exact comparisons refine
    /// it to the two adjacent binary numbers enclosing the algebraic value.
    fn rounded(&self, upward: bool) -> f64 {
        let guess = self.center.to_f64().unwrap_or(0.)
            + self.coefficient.to_f64().unwrap_or(0.)
                * self.radicand.to_f64().unwrap_or(f64::INFINITY).sqrt();
        let compare = |x| self.cmp(&Self::rational(q(x)));
        if guess.is_finite() {
            let order = compare(guess);
            if order == Ordering::Equal {
                return guess;
            }
            let next = if order.is_gt() {
                guess.next_up()
            } else {
                guess.next_down()
            };
            if next.is_finite() && compare(next) != order {
                return if upward == order.is_gt() { next } else { guess };
            }
        }
        // Catastrophic cancellation in center +/- radius must not erase a
        // short residual interval. Binary search uses the total order of
        // finite IEEE coordinates and takes at most 64 exact comparisons.
        let key = |x: f64| {
            let bits = x.to_bits();
            if x.is_sign_negative() {
                !bits
            } else {
                bits | 1 << 63
            }
        };
        let value = |bits: u64| {
            f64::from_bits(if bits & (1 << 63) != 0 {
                bits & !(1 << 63)
            } else {
                !bits
            })
        };
        let (mut low, mut high) = (key(-f64::MAX), key(f64::MAX));
        while high - low > 1 {
            let middle = low + (high - low) / 2;
            match compare(value(middle)) {
                Ordering::Less => high = middle,
                Ordering::Equal => return value(middle),
                Ordering::Greater => low = middle,
            }
        }
        value(if upward { high } else { low })
    }
    fn outward(&self, line: Line, forward: bool) -> Coord {
        let coordinate = |from, to| {
            Self {
                center: q(from) + &self.center * (q(to) - q(from)),
                coefficient: &self.coefficient * (q(to) - q(from)),
                radicand: self.radicand.clone(),
            }
            .rounded((to > from) == forward)
        };
        Coord {
            x: coordinate(line.start.x, line.end.x),
            y: coordinate(line.start.y, line.end.y),
        }
    }
}

/// A lazy exact transition with outward-rounded floating bounds. Most nearby
/// intervals can be ordered without constructing any rational expression.
#[derive(Clone, Debug)]
pub(crate) struct Cut {
    bounds: Interval,
    value: OnceCell<ExactCut>,
    expression: Option<Expression>,
}
#[derive(Clone, Copy, Debug)]
struct Expression {
    source: Line,
    boundary: Line,
    tolerance: f64,
    kind: Kind,
    positive: bool,
}
#[derive(Clone, Copy, Debug)]
enum Kind {
    Projection,
    Strip,
    Disc,
}

fn exact_vectors(source: Line, boundary: Line) -> [[Q; 2]; 3] {
    [
        [
            q(source.end.x) - q(source.start.x),
            q(source.end.y) - q(source.start.y),
        ],
        [
            q(boundary.end.x) - q(boundary.start.x),
            q(boundary.end.y) - q(boundary.start.y),
        ],
        [
            q(source.start.x) - q(boundary.start.x),
            q(source.start.y) - q(boundary.start.y),
        ],
    ]
}
fn dot(a: &[Q; 2], b: &[Q; 2]) -> Q {
    &a[0] * &b[0] + &a[1] * &b[1]
}
fn cross(a: &[Q; 2], b: &[Q; 2]) -> Q {
    &a[0] * &b[1] - &a[1] * &b[0]
}
impl Expression {
    fn exact(self) -> ExactCut {
        let [v, w, p] = exact_vectors(self.source, self.boundary);
        let tolerance2 = q(self.tolerance) * q(self.tolerance);
        let (center, radius) = match self.kind {
            Kind::Projection => {
                let endpoint = if self.positive {
                    dot(&w, &w)
                } else {
                    Q::zero()
                };
                return ExactCut::rational((endpoint - dot(&p, &w)) / dot(&v, &w));
            }
            Kind::Strip => {
                let drift = cross(&v, &w);
                (
                    -cross(&p, &w) / &drift,
                    tolerance2 * dot(&w, &w) / (&drift * &drift),
                )
            }
            Kind::Disc => {
                let length2 = dot(&v, &v);
                let perpendicular = cross(&p, &v);
                (
                    -dot(&p, &v) / &length2,
                    (tolerance2 * &length2 - &perpendicular * &perpendicular)
                        / (&length2 * &length2),
                )
            }
        };
        debug_assert!(!radius.is_negative());
        ExactCut::root(center, radius, self.positive)
    }
}
impl Cut {
    pub(crate) fn rational(value: Q) -> Self {
        let approx = value.to_f64().unwrap_or_else(|| {
            if value.is_negative() {
                f64::NEG_INFINITY
            } else {
                f64::INFINITY
            }
        });
        Self {
            bounds: Interval(approx.next_down(), approx.next_up()),
            value: OnceCell::from(ExactCut::rational(value)),
            expression: None,
        }
    }
    fn binary(value: f64) -> Self {
        Self {
            bounds: Interval(value, value),
            value: OnceCell::from(ExactCut::rational(q(value))),
            expression: None,
        }
    }
    fn lazy(bounds: Interval, expression: Expression) -> Self {
        Self {
            bounds,
            value: OnceCell::new(),
            expression: Some(expression),
        }
    }
    fn exact(&self) -> &ExactCut {
        self.value
            .get_or_init(|| self.expression.expect("lazy expression").exact())
    }
    pub(crate) fn cmp(&self, other: &Self) -> Ordering {
        if self.bounds.1 < other.bounds.0 {
            Ordering::Less
        } else if self.bounds.0 > other.bounds.1 {
            Ordering::Greater
        } else if self.bounds.0 == self.bounds.1
            && self.bounds.0 == other.bounds.0
            && other.bounds.0 == other.bounds.1
        {
            Ordering::Equal
        } else {
            self.exact().cmp(other.exact())
        }
    }
    pub(crate) fn outward(&self, line: Line, forward: bool) -> Coord {
        self.exact().outward(line, forward)
    }
}

fn clipped(span: [Cut; 2], bounds: &[Cut; 2]) -> Option<[Cut; 2]> {
    let [mut a, mut b] = span;
    if a.cmp(&bounds[0]).is_lt() {
        a = bounds[0].clone();
    }
    if b.cmp(&bounds[1]).is_gt() {
        b = bounds[1].clone();
    }
    (!a.cmp(&b).is_gt()).then_some([a, b])
}
fn scalar(v: f64) -> Interval {
    Interval(v, v)
}
fn idot(a: [Interval; 2], b: [Interval; 2]) -> Interval {
    a[0].mul(b[0]).add(a[1].mul(b[1]))
}
fn icross(a: [Interval; 2], b: [Interval; 2]) -> Interval {
    a[0].mul(b[1]).sub(a[1].mul(b[0]))
}
fn vector(a: Coord, b: Coord) -> [Interval; 2] {
    [scalar(b.x).sub(scalar(a.x)), scalar(b.y).sub(scalar(a.y))]
}
fn sign_with(value: Interval, exact: impl FnOnce() -> Q) -> Ordering {
    value.sign().unwrap_or_else(|| exact().cmp(&Q::zero()))
}
fn root_pair(center: Interval, radius2: Interval, expression: Expression) -> [Cut; 2] {
    let radius = radius2.sqrt();
    [
        Cut::lazy(
            center.sub(radius),
            Expression {
                positive: false,
                ..expression
            },
        ),
        Cut::lazy(
            center.add(radius),
            Expression {
                positive: true,
                ..expression
            },
        ),
    ]
}

/// Parameters directly within the inclusive Euclidean tolerance capsule.
/// The middle rectangle and both endpoint discs are unioned directly; no
/// chain, longitudinal-support test, or output-size threshold is involved.
pub(crate) fn capsule(source: Line, boundary: Line, tolerance: f64) -> Vec<[Cut; 2]> {
    crate::cancel::check();
    let v = vector(source.start, source.end);
    let w = vector(boundary.start, boundary.end);
    let p = vector(boundary.start, source.start);
    let tolerance2 = scalar(tolerance).square();
    let bounds = [Cut::binary(0.), Cut::binary(1.)];
    let mut spans = Vec::with_capacity(3);
    let expression = Expression {
        source,
        boundary,
        tolerance,
        kind: Kind::Projection,
        positive: false,
    };
    let length2 = idot(w, w);
    let projected = idot(p, w);
    let change = idot(v, w);
    let change_sign = sign_with(change, || {
        let [v, w, _] = exact_vectors(source, boundary);
        dot(&v, &w)
    });
    let boundary_nonzero = boundary.start != boundary.end;
    let projection = if !boundary_nonzero {
        None
    } else if change_sign == Ordering::Equal {
        let low = sign_with(projected, || {
            let [_, w, p] = exact_vectors(source, boundary);
            dot(&p, &w)
        });
        let high = sign_with(projected.sub(length2), || {
            let [_, w, p] = exact_vectors(source, boundary);
            dot(&p, &w) - dot(&w, &w)
        });
        (low != Ordering::Less && high != Ordering::Greater).then(|| bounds.clone())
    } else {
        let mut cuts = [
            Cut::lazy(scalar(0.).sub(projected).div(change), expression),
            Cut::lazy(
                length2.sub(projected).div(change),
                Expression {
                    positive: true,
                    ..expression
                },
            ),
        ];
        if change_sign == Ordering::Less {
            cuts.swap(0, 1);
        }
        clipped(cuts, &bounds)
    };
    if let Some(projection) = projection {
        let drift = icross(v, w);
        let offset = icross(p, w);
        let drift_sign = sign_with(drift, || {
            let [v, w, _] = exact_vectors(source, boundary);
            cross(&v, &w)
        });
        let strip = if drift_sign == Ordering::Equal {
            let distance = offset.square().sub(tolerance2.mul(length2));
            let order = sign_with(distance, || {
                let [_, w, p] = exact_vectors(source, boundary);
                let x = cross(&p, &w);
                &x * &x - q(tolerance) * q(tolerance) * dot(&w, &w)
            });
            (order != Ordering::Greater).then(|| projection.clone())
        } else {
            clipped(
                root_pair(
                    scalar(0.).sub(offset).div(drift),
                    tolerance2.mul(length2).div(drift.square()),
                    Expression {
                        kind: Kind::Strip,
                        ..expression
                    },
                ),
                &projection,
            )
        };
        if let Some(span) = strip {
            spans.push(span);
        }
    }
    let source_length2 = idot(v, v);
    for point in [boundary.start, boundary.end] {
        let offset = vector(point, source.start);
        let perpendicular = icross(offset, v);
        let discriminant = tolerance2.mul(source_length2).sub(perpendicular.square());
        let order = sign_with(discriminant, || {
            let [v, _, p] = exact_vectors(source, Line::new(point, point));
            let x = cross(&p, &v);
            q(tolerance) * q(tolerance) * dot(&v, &v) - &x * &x
        });
        if order == Ordering::Less {
            continue;
        }
        let center = scalar(0.).sub(idot(offset, v)).div(source_length2);
        let radius2 = discriminant.div(source_length2.square());
        let disc = root_pair(
            center,
            radius2,
            Expression {
                boundary: Line::new(point, point),
                kind: Kind::Disc,
                ..expression
            },
        );
        if let Some(span) = clipped(disc, &bounds) {
            spans.push(span);
        }
    }
    union(spans)
}

pub(crate) fn union(mut spans: Vec<[Cut; 2]>) -> Vec<[Cut; 2]> {
    spans.sort_by(|a, b| a[0].cmp(&b[0]).then_with(|| a[1].cmp(&b[1])));
    let mut result: Vec<[Cut; 2]> = Vec::new();
    for span in spans {
        if let Some(last) = result.last_mut()
            && !span[0].cmp(&last[1]).is_gt()
        {
            if span[1].cmp(&last[1]).is_gt() {
                last[1] = span[1].clone();
            }
        } else {
            result.push(span);
        }
    }
    result
}

/// The uncovered complement inside one exact overlay window.
pub(crate) fn outside(a: Q, b: Q, covered: &[[Cut; 2]]) -> Vec<[Cut; 2]> {
    let bounds = [Cut::rational(a), Cut::rational(b)];
    let mut start = bounds[0].clone();
    let mut result = Vec::new();
    for span in covered {
        let Some([a, b]) = clipped(span.clone(), &bounds) else {
            continue;
        };
        if start.cmp(&a).is_lt() {
            result.push([start, a]);
        }
        start = b;
    }
    if start.cmp(&bounds[1]).is_lt() {
        result.push([start, bounds[1].clone()]);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    fn cut(center: f64, radicand: f64, positive: bool) -> ExactCut {
        ExactCut::root(q(center), q(radicand), positive)
    }
    #[test]
    fn algebraic_order_certifies_equal_and_different_symbolic_roots() {
        let zero = ExactCut::rational(Q::zero());
        assert_eq!(cut(-2., 4., true).cmp(&zero), Ordering::Equal);
        assert_eq!(cut(1., 9., true).cmp(&cut(2., 4., true)), Ordering::Equal);
        assert_eq!(cut(1., 9., false).cmp(&cut(0., 4., false)), Ordering::Equal);
        assert_eq!(cut(0., 2., true).cmp(&cut(0., 3., true)), Ordering::Less);
        assert_eq!(
            cut(0., 2., false).cmp(&cut(0., 3., false)),
            Ordering::Greater
        );
        for i in -8..8 {
            for j in 0..8 {
                for k in -8..8 {
                    for l in 0..8 {
                        let a = cut(i as f64, j as f64, true);
                        let b = cut(k as f64, l as f64, false);
                        let expected = (i as f64 + (j as f64).sqrt())
                            .partial_cmp(&(k as f64 - (l as f64).sqrt()))
                            .unwrap();
                        assert_eq!(a.cmp(&b), expected, "{i} + sqrt({j}), {k} - sqrt({l})");
                    }
                }
            }
        }
    }
    #[test]
    fn inclusive_capsules_use_the_actual_binary_distance() {
        let boundary = Line::new((0., 0.), (8., 0.));
        for y in [1_f64.next_down(), 1., 1_f64.next_up()] {
            let spans = capsule(Line::new((2., y), (6., y)), boundary, 1.);
            assert_eq!(spans.is_empty(), y > 1.);
            if y <= 1. {
                assert_eq!(spans[0][0].cmp(&Cut::binary(0.)), Ordering::Equal);
                assert_eq!(spans[0][1].cmp(&Cut::binary(1.)), Ordering::Equal);
            }
        }
        // Endpoint discs use the Euclidean radius, including its exact rim.
        for x in [3_f64.next_down(), 3., 3_f64.next_up()] {
            let source = Line::new((x, 4.), (x, 8.));
            let spans = capsule(source, Line::new((0., 0.), (0., 0.)), 5.);
            assert_eq!(spans.is_empty(), x > 3.);
            if x == 3. {
                assert_eq!(spans[0][0].cmp(&spans[0][1]), Ordering::Equal);
            }
        }
    }
    #[test]
    fn outward_emission_preserves_a_sub_ulp_residual_in_both_directions() {
        let line = Line::new((1., 2.), (2., 3.));
        let at = Q::from_integer(1.into()) / Q::from_integer((1u64 << 54).into());
        let a = Cut::rational(at.clone());
        let b = Cut::rational(at * Q::from_integer(2.into()));
        assert_eq!(a.outward(line, false), line.start);
        assert_eq!(
            b.outward(line, true),
            Coord {
                x: 1_f64.next_up(),
                y: 2_f64.next_up()
            }
        );
        let reverse = Line::new(line.end, line.start);
        let a = Cut::rational(Q::one() - a.exact().center.clone());
        let b = Cut::rational(Q::one() - b.exact().center.clone());
        assert_eq!(
            b.outward(reverse, false),
            Coord {
                x: 1_f64.next_up(),
                y: 2_f64.next_up()
            }
        );
        assert_eq!(a.outward(reverse, true), line.start);
    }
}
