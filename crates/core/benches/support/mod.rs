use geo::{Geometry, LineString, MultiLineString, MultiPolygon, Rect, coord};
use topology_checker_core::{Layer, Rule, Topology, merge_lines};

pub fn multipart() -> Vec<Geometry> {
    (0..100)
        .map(|i| {
            Geometry::MultiLineString(MultiLineString(
                (0..10)
                    .map(|j| {
                        LineString::from(
                            (0..100)
                                .map(|k| (k as f64, (i * 10 + j) as f64))
                                .collect::<Vec<_>>(),
                        )
                    })
                    .collect(),
            ))
        })
        .collect()
}

pub fn aggregation() -> Vec<Geometry> {
    (0..2)
        .map(|_| {
            Geometry::MultiPolygon(MultiPolygon(
                (0..100)
                    .map(|i| {
                        Rect::new(
                            coord! {x: i as f64 * 3., y: 0.},
                            coord! {x: i as f64 * 3. + 2., y: 2.},
                        )
                        .to_polygon()
                    })
                    .collect(),
            ))
        })
        .collect()
}

pub fn long_lines() -> Vec<LineString> {
    vec![
        LineString::from((0..10_001).map(|i| (i as f64, 0.)).collect::<Vec<_>>()),
        LineString::from(
            (10_000..20_001)
                .rev()
                .map(|i| (i as f64, 0.))
                .collect::<Vec<_>>(),
        ),
    ]
}

pub fn clean(input: Vec<Geometry>) -> Vec<String> {
    let mut t = Topology::new(0.).unwrap();
    t.add_layer(Layer::from_geometries("multipart", input));
    t.warnings().unwrap()
}

pub fn aggregate(input: Vec<Geometry>) -> topology_checker_core::Report {
    let mut t = Topology::new(0.).unwrap();
    let id = t.add_layer(Layer::from_geometries("aggregation", input));
    t.add_rule(Rule::PolygonMustNotOverlap, id, None).unwrap();
    t.validate().unwrap()
}

pub fn merge(input: Vec<LineString>) -> Vec<LineString> {
    merge_lines(input, 0.).unwrap()
}
