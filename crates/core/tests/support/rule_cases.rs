//! Hand-specified scenario support. No expected output comes from validation.
use topology_checker_core::{
    FeatureRef, Layer, Rule, Topology, Violation, ViolationKind,
    geo::{
        Geometry, GeometryCollection, LineString, MapCoords, MultiLineString, MultiPoint,
        MultiPolygon, Point, Polygon, Rect, Relate, coord,
    },
};

pub const A: i64 = -9_000_000_000;
pub const B: i64 = 9_000_000_000;
pub type Reference = (usize, i64, Option<u32>);
#[derive(Clone, Debug)]
pub struct Expected {
    pub kind: ViolationKind,
    pub geometry: Geometry,
    pub origin: Option<Reference>,
    pub destination: Option<Reference>,
}
#[derive(Clone, Debug)]
pub struct Case {
    pub name: &'static str,
    pub rule: Rule,
    pub a: Vec<(i64, Geometry)>,
    pub b: Vec<(i64, Geometry)>,
    pub tolerance: f64,
    pub expected: Vec<Expected>,
}
pub fn p(x: f64, y: f64) -> Geometry {
    Point::new(x, y).into()
}
pub fn l(c: &[(f64, f64)]) -> Geometry {
    LineString::from(c.to_vec()).into()
}
pub fn rect(x: f64, y: f64, w: f64, h: f64) -> Polygon {
    Rect::new(coord! {x:x,y:y}, coord! {x:x+w,y:y+h}).to_polygon()
}
pub fn square() -> Geometry {
    rect(0., 0., 8., 8.).into()
}
pub fn hole() -> Geometry {
    Polygon::new(
        rect(0., 0., 8., 8.).exterior().clone(),
        vec![rect(2., 2., 4., 4.).exterior().clone()],
    )
    .into()
}
pub fn concave() -> Geometry {
    Polygon::new(
        LineString::from(vec![
            (0., 0.),
            (8., 0.),
            (8., 2.),
            (2., 2.),
            (2., 8.),
            (0., 8.),
            (0., 0.),
        ]),
        vec![],
    )
    .into()
}
pub fn mp(c: &[(f64, f64)]) -> Geometry {
    MultiPoint(c.iter().map(|&(x, y)| Point::new(x, y)).collect()).into()
}
pub fn ml(gs: Vec<Geometry>) -> Geometry {
    MultiLineString(
        gs.into_iter()
            .map(|g| {
                if let Geometry::LineString(l) = g {
                    l
                } else {
                    panic!("line")
                }
            })
            .collect(),
    )
    .into()
}
pub fn polys(gs: Vec<Geometry>) -> Geometry {
    MultiPolygon(
        gs.into_iter()
            .map(|g| {
                if let Geometry::Polygon(p) = g {
                    p
                } else {
                    panic!("polygon")
                }
            })
            .collect(),
    )
    .into()
}
pub fn collection(gs: Vec<Geometry>) -> Geometry {
    Geometry::GeometryCollection(GeometryCollection(gs))
}
pub fn perimeter(g: &Geometry) -> Geometry {
    if let Geometry::Polygon(p) = g {
        p.exterior().clone().into()
    } else {
        panic!("polygon")
    }
}
pub fn e(kind: ViolationKind, g: Geometry, o: Option<Reference>, d: Option<Reference>) -> Expected {
    Expected {
        kind,
        geometry: g,
        origin: o,
        destination: d,
    }
}
pub fn one(kind: ViolationKind, g: Geometry) -> Expected {
    e(kind, g, Some((0, A, Some(0))), None)
}
pub fn pair(kind: ViolationKind, g: Geometry, binary: bool) -> Expected {
    e(
        kind,
        g,
        Some((0, A, Some(0))),
        Some((usize::from(binary), if binary { A } else { B }, Some(0))),
    )
}
pub fn case(
    rule: Rule,
    name: &'static str,
    a: Vec<Geometry>,
    b: Vec<Geometry>,
    tolerance: f64,
    expected: Vec<Expected>,
) -> Case {
    Case {
        name,
        rule,
        a: a.into_iter()
            .enumerate()
            .map(|(i, g)| (if i == 0 { A } else { B + i as i64 - 1 }, g))
            .collect(),
        b: b.into_iter()
            .enumerate()
            .map(|(i, g)| (A + i as i64, g))
            .collect(),
        tolerance,
        expected,
    }
}
pub fn topology(c: &Case) -> Topology {
    let mut t = Topology::new(c.tolerance).unwrap();
    let a = t.add_layer(Layer::from_features("primary", c.a.clone()));
    let b = c
        .rule
        .inputs()
        .1
        .map(|_| t.add_layer(Layer::from_features("target", c.b.clone())));
    t.add_rule(c.rule, a, b).unwrap();
    t
}
fn reference(r: Option<FeatureRef>) -> Option<Reference> {
    r.map(|r| (r.layer.index(), r.fid, r.part))
}
fn shape(g: &Geometry) -> (&'static str, u8) {
    match g {
        Geometry::Point(_) => ("point", 0),
        Geometry::MultiPoint(_) => ("multipoint", 0),
        Geometry::LineString(_) => ("line", 1),
        Geometry::MultiLineString(_) => ("multiline", 1),
        Geometry::Polygon(_) => ("polygon", 2),
        Geometry::MultiPolygon(_) => ("multipolygon", 2),
        Geometry::GeometryCollection(c) => (
            "collection",
            c.0.iter().map(|g| shape(g).1).max().unwrap_or(0),
        ),
        _ => panic!("unexpected output type"),
    }
}
pub fn equal_geometry(a: &Geometry, b: &Geometry) -> bool {
    if shape(a) != shape(b) {
        return false;
    }
    if let (Geometry::GeometryCollection(a), Geometry::GeometryCollection(b)) = (a, b) {
        if a.0.len() != b.0.len() {
            return false;
        }
        let mut used = vec![false; b.0.len()];
        for member in &a.0 {
            let Some(i) =
                b.0.iter()
                    .enumerate()
                    .position(|(i, candidate)| !used[i] && equal_geometry(member, candidate))
            else {
                return false;
            };
            used[i] = true;
        }
        return true;
    }
    a == b || a.relate(b).is_equal_topo()
}
pub fn assert_expected(c: &Case, actual: &[Violation]) {
    assert_eq!(
        actual.len(),
        c.expected.len(),
        "{} {}: {actual:?}",
        c.rule,
        c.name
    );
    let mut used = vec![false; actual.len()];
    for expected in &c.expected {
        let i = actual.iter().enumerate().position(|(i, v)| {
            !used[i]
                && v.rule == c.rule
                && v.kind == expected.kind
                && reference(v.origin) == expected.origin
                && reference(v.destination) == expected.destination
                && equal_geometry(&v.geometry, &expected.geometry)
        });
        assert!(
            i.is_some(),
            "{} {}: missing {expected:?}, actual {actual:?}",
            c.rule,
            c.name
        );
        used[i.unwrap()] = true;
    }
}
pub fn assert_same(a: &[Violation], b: &[Violation], label: &str) {
    assert_eq!(a.len(), b.len(), "{label}");
    let mut used = vec![false; b.len()];
    for x in a {
        let i = b.iter().enumerate().position(|(i, y)| {
            !used[i]
                && x.rule == y.rule
                && x.kind == y.kind
                && x.origin == y.origin
                && x.destination == y.destination
                && equal_geometry(&x.geometry, &y.geometry)
        });
        assert!(i.is_some(), "{label}: {x:?}, versus {b:?}");
        used[i.unwrap()] = true;
    }
}
pub fn transform(c: &Case, scale: f64, dx: f64, dy: f64) -> Case {
    let mut c = c.clone();
    for (_, g) in c.a.iter_mut().chain(c.b.iter_mut()) {
        *g = g.map_coords(|p| coord! {x:p.x*scale+dx,y:p.y*scale+dy});
    }
    for e in &mut c.expected {
        e.geometry = e
            .geometry
            .map_coords(|p| coord! {x:p.x*scale+dx,y:p.y*scale+dy});
    }
    c.tolerance *= scale;
    c
}
