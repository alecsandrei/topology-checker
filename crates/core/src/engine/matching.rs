//! Shared intervals are evidence from original segments, independent of clustering.
use super::{cluster::distance, line_graph::projection};
use geo::{Coord, Line, LineIntersection, line_intersection::line_intersection};

pub(crate) fn point(line: Line, t: f64) -> Coord {
    if t == 0. {
        line.start
    } else if t == 1. {
        line.end
    } else {
        Coord {
            x: line.start.x + t * (line.end.x - line.start.x),
            y: line.start.y + t * (line.end.y - line.start.y),
        }
    }
}
/// The common longitudinal span must stay within tolerance at both ends.
/// Linear separation then stays within tolerance everywhere on that span.
/// Its length must exceed the transverse drift, excluding junction connectors.
pub(crate) fn interval(a: Line, b: Line, tolerance: f64) -> Option<[[f64; 2]; 2]> {
    evidence(a, b, tolerance).map(|e| e.spans)
}

pub(crate) struct Evidence {
    pub spans: [[f64; 2]; 2],
    pub support: [[f64; 2]; 2],
    pub exact: bool,
}

pub(crate) fn evidence(a: Line, b: Line, tolerance: f64) -> Option<Evidence> {
    let intersection = line_intersection(a, b);
    if tolerance == 0. {
        return match intersection {
            Some(LineIntersection::Collinear { intersection })
                if intersection.start != intersection.end =>
            {
                let span = |s| {
                    let x = projection(s, intersection.start).0;
                    let y = projection(s, intersection.end).0;
                    [x.min(y), x.max(y)]
                };
                let spans = [span(a), span(b)];
                Some(Evidence {
                    spans,
                    support: spans,
                    exact: true,
                })
            }
            _ => None,
        };
    }
    let endpoint_contact = matches!(
        intersection,
        Some(LineIntersection::SinglePoint {
            is_proper: false,
            ..
        })
    );
    let vector = |s: Line| (s.end.x - s.start.x, s.end.y - s.start.y);
    let (ax, ay) = vector(a);
    let (bx, by) = vector(b);
    let al = ax.hypot(ay);
    let bl = bx.hypot(by);
    if al == 0. || bl == 0. {
        return None;
    }
    // A canonical longitudinal direction makes operand and path reversal invariant.
    let choose_a = al > bl || al == bl && (ax.abs(), ay.abs()) >= (bx.abs(), by.abs());
    let (mut ux, mut uy) = if choose_a {
        (ax / al, ay / al)
    } else {
        (bx / bl, by / bl)
    };
    if ux < 0. || ux == 0. && uy < 0. {
        ux = -ux;
        uy = -uy;
    }
    let origin = if (a.start.x, a.start.y) < (b.start.x, b.start.y) {
        a.start
    } else {
        b.start
    };
    let along = |p: Coord| (p.x - origin.x) * ux + (p.y - origin.y) * uy;
    let av = [along(a.start), along(a.end)];
    let bv = [along(b.start), along(b.end)];
    let lo = av[0].min(av[1]).max(bv[0].min(bv[1]));
    let hi = av[0].max(av[1]).min(bv[0].max(bv[1]));
    if hi <= lo || av[0] == av[1] || bv[0] == bv[1] {
        return None;
    }
    let span = |v: [f64; 2]| {
        let x = ((lo - v[0]) / (v[1] - v[0])).clamp(0., 1.);
        let y = ((hi - v[0]) / (v[1] - v[0])).clamp(0., 1.);
        [x.min(y), x.max(y)]
    };
    let (mut sa, mut sb) = (span(av), span(bv));
    let support = [sa, sb];
    let at = |s: Line, v: [f64; 2], t: f64| point(s, ((t - v[0]) / (v[1] - v[0])).clamp(0., 1.));
    let (a0, a1, b0, b1) = (at(a, av, lo), at(a, av, hi), at(b, bv, lo), at(b, bv, hi));
    if distance(a0, b0) > tolerance || distance(a1, b1) > tolerance {
        return None;
    }
    // Compute change from original vectors. Rounded projected coordinates can
    // coincide at a short junction span and falsely erase transverse drift.
    let drift = (hi - lo)
        * (ax / (av[1] - av[0]) - bx / (bv[1] - bv[0]))
            .hypot(ay / (av[1] - av[0]) - by / (bv[1] - bv[0]));
    if drift >= hi - lo {
        return None;
    }
    // A junction needs sustained support beyond its tolerance neighbourhood.
    // Subtract transverse drift before testing the common longitudinal extent,
    // and do so before extending endpoint caps. Long near-duplicate passages
    // sharing source vertices still qualify, including unequal segmentation.
    if endpoint_contact && hi - lo <= tolerance + drift {
        return None;
    }
    // Short endpoint caps are covered only by an actual nearby target segment.
    // This also retains original vertices instead of replacing them by projections.
    for (s, other, range) in [(a, b, &mut sa), (b, a, &mut sb)] {
        if distance(s.start, projection(other, s.start).1) <= tolerance {
            range[0] = 0.;
        }
        if distance(s.end, projection(other, s.end).1) <= tolerance {
            range[1] = 1.;
        }
    }
    Some(Evidence {
        spans: [sa, sb],
        support,
        exact: matches!(intersection, Some(LineIntersection::Collinear { intersection }) if intersection.start != intersection.end),
    })
}

pub(crate) fn union(mut spans: Vec<[f64; 2]>) -> Vec<[f64; 2]> {
    spans.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
    let mut result: Vec<[f64; 2]> = Vec::new();
    for span in spans {
        if let Some(last) = result.last_mut()
            && span[0] <= last[1]
        {
            last[1] = last[1].max(span[1]);
        } else {
            result.push(span);
        }
    }
    result
}
