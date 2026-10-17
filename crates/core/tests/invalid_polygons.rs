use topology_checker_core::{
    Error, Layer, Rule, Topology,
    geo::{Geometry, LineString, MultiPolygon, Polygon},
};

fn ring(coords: &[(f64, f64)]) -> LineString {
    LineString::from(coords.to_vec())
}
fn square(x: f64, y: f64, size: f64) -> LineString {
    ring(&[
        (x, y),
        (x + size, y),
        (x + size, y + size),
        (x, y + size),
        (x, y),
    ])
}
fn polygon(shell: LineString, holes: Vec<LineString>) -> Geometry {
    Polygon::new(shell, holes).into()
}

/// Register the geometry as a polygon layer and return the rejection, if any.
fn rejection(geometry: Geometry) -> Option<Error> {
    let mut topology = Topology::new(0.).unwrap();
    let id = topology.add_layer(Layer::from_geometries("polygons", [geometry]));
    topology
        .add_rule(Rule::PolygonMustNotOverlap, id, None)
        .and_then(|_| topology.validate().map(|_| ()))
        .err()
}
fn message(geometry: Geometry) -> String {
    rejection(geometry).expect("a rejected polygon").to_string()
}

#[test]
fn rings_that_cross_themselves_are_rejected() {
    let bowtie = ring(&[(0., 0.), (2., 2.), (2., 0.), (0., 2.), (0., 0.)]);
    assert!(
        message(polygon(bowtie, vec![]))
            .ends_with("ring Some(0): invalid polygon: self-intersection or closing spike")
    );
    let spike = ring(&[(0., 0.), (4., 0.), (4., 4.), (4., 0.), (0., 0.)]);
    assert!(
        message(polygon(spike, vec![]))
            .ends_with("ring Some(0): invalid polygon: self-intersection or closing spike")
    );
}

#[test]
fn holes_must_sit_inside_the_shell_without_touching_each_other() {
    let shell = || square(0., 0., 10.);
    let cases = [
        (
            "outside",
            vec![square(20., 20., 2.)],
            "hole outside shell or crossing shell",
        ),
        (
            "crossing the shell",
            vec![ring(&[(8., 2.), (12., 2.), (12., 4.), (8., 4.), (8., 2.)])],
            "crossing or overlapping polygon rings",
        ),
        (
            "sharing a shell edge",
            vec![ring(&[(0., 2.), (2., 2.), (2., 4.), (0., 4.), (0., 2.)])],
            "crossing or overlapping polygon rings",
        ),
        (
            "crossing another hole",
            vec![square(2., 2., 4.), square(4., 4., 4.)],
            "crossing or overlapping polygon rings",
        ),
        (
            "entering another hole from a touching vertex",
            vec![
                square(2., 2., 4.),
                ring(&[(4., 2.), (5., 4.), (3., 4.), (4., 2.)]),
            ],
            "crossing or overlapping polygon rings",
        ),
        (
            "nested in another hole",
            vec![square(2., 2., 6.), square(4., 4., 2.)],
            "nested or overlapping holes",
        ),
    ];
    for (name, holes, expected) in cases {
        let message = message(polygon(shell(), holes));
        assert!(
            message.contains("ring Some(")
                && message.contains(&format!("invalid polygon: {expected}")),
            "{name}: {message}"
        );
    }
}

#[test]
fn holes_that_enclose_part_of_the_interior_are_rejected() {
    // Two holes touching at two points cut the interior in half.
    let holes = vec![
        ring(&[(2., 2.), (5., 5.), (2., 8.), (2., 2.)]),
        ring(&[(8., 2.), (5., 5.), (8., 8.), (8., 2.)]),
        ring(&[(2., 2.), (8., 2.), (5., 4.), (2., 2.)]),
    ];
    let message = message(polygon(square(0., 0., 10.), holes));
    assert!(
        message.ends_with("invalid polygon: disconnected interior"),
        "{message}"
    );
}

#[test]
fn multipolygon_parts_must_not_overlap() {
    let overlapping = MultiPolygon::new(vec![
        Polygon::new(square(0., 0., 4.), vec![]),
        Polygon::new(square(2., 2., 4.), vec![]),
    ]);
    let text = message(overlapping.into());
    assert!(
        text.contains("ring None: invalid polygon: invalid multipart"),
        "{text}"
    );
    let nested = MultiPolygon::new(vec![
        Polygon::new(square(0., 0., 10.), vec![]),
        Polygon::new(square(2., 2., 2.), vec![]),
    ]);
    assert!(message(nested.into()).contains("ring None: invalid polygon: invalid multipart"));
}
