//! Outward-rounded bounds over original binary coordinates and exact nodes.
//! A rounded node is used only to enclose its rational value, never as a decision.
use super::arrangement::Exact;
use std::cmp::Ordering;

#[derive(Clone, Copy, Debug)]
pub(super) struct Interval(pub f64, pub f64);
impl Interval {
    pub fn add(self, b: Self) -> Self {
        Self((self.0 + b.0).next_down(), (self.1 + b.1).next_up())
    }
    pub fn sub(self, b: Self) -> Self {
        Self((self.0 - b.1).next_down(), (self.1 - b.0).next_up())
    }
    pub fn mul(self, b: Self) -> Self {
        let v = [self.0 * b.0, self.0 * b.1, self.1 * b.0, self.1 * b.1];
        if v.iter().any(|x| !x.is_finite()) {
            return Self(f64::NEG_INFINITY, f64::INFINITY);
        }
        Self(
            v.iter().copied().fold(f64::INFINITY, f64::min).next_down(),
            v.iter()
                .copied()
                .fold(f64::NEG_INFINITY, f64::max)
                .next_up(),
        )
    }
    pub fn div(self, b: Self) -> Self {
        let v = [self.0 / b.0, self.0 / b.1, self.1 / b.0, self.1 / b.1];
        if b.0 <= 0. && b.1 >= 0. || v.iter().any(|x| !x.is_finite()) {
            return Self(f64::NEG_INFINITY, f64::INFINITY);
        }
        Self(
            v.iter().copied().fold(f64::INFINITY, f64::min).next_down(),
            v.iter()
                .copied()
                .fold(f64::NEG_INFINITY, f64::max)
                .next_up(),
        )
    }
    pub fn abs(self) -> Self {
        if self.0 >= 0. {
            self
        } else if self.1 <= 0. {
            Self(-self.1, -self.0)
        } else {
            Self(0., (-self.0).max(self.1))
        }
    }
    pub fn square(self) -> Self {
        let a = self.abs();
        Self((a.0 * a.0).next_down().max(0.), (a.1 * a.1).next_up())
    }
    pub fn sqrt(self) -> Self {
        Self(
            self.0.max(0.).sqrt().next_down().max(0.),
            self.1.sqrt().next_up(),
        )
    }
    pub fn sign(self) -> Option<Ordering> {
        if self.0 > 0. {
            Some(Ordering::Greater)
        } else if self.1 < 0. {
            Some(Ordering::Less)
        } else {
            None
        }
    }
}
pub(super) fn coordinates(p: &Exact) -> [Interval; 2] {
    let q = p.output();
    if matches!(p, Exact::Input(_)) {
        [Interval(q.x, q.x), Interval(q.y, q.y)]
    } else {
        [
            Interval(q.x.next_down(), q.x.next_up()),
            Interval(q.y.next_down(), q.y.next_up()),
        ]
    }
}
pub(super) fn compare(a: &Exact, b: &Exact, axis: usize) -> Ordering {
    if let (Exact::Input(a), Exact::Input(b)) = (a, b) {
        return (if axis == 0 {
            a.x.partial_cmp(&b.x)
        } else {
            a.y.partial_cmp(&b.y)
        })
        .expect("finite input");
    }
    let (a_bounds, b_bounds) = (coordinates(a)[axis], coordinates(b)[axis]);
    if let Some(order) = a_bounds.sub(b_bounds).sign() {
        return order;
    }
    let a = a.xy();
    let b = b.xy();
    if axis == 0 {
        a.0.cmp(&b.0)
    } else {
        a.1.cmp(&b.1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn products_beyond_the_float_range_are_unbounded() {
        let huge = Interval(1e200, 1e200);
        let product = huge.mul(huge);
        assert_eq!((product.0, product.1), (f64::NEG_INFINITY, f64::INFINITY));
        assert_eq!(product.sign(), None);
        let small = Interval(2., 3.).mul(Interval(-5., -4.));
        assert!(small.0 <= -15. && small.1 >= -8.);
        assert_eq!(small.sign(), Some(Ordering::Less));
    }
}
