use topology_checker_core::{
    Layer, Rule, Topology,
    geo::{Geometry, LineString, Polygon, Rect, coord},
};

fn l_shape() -> Geometry {
    Polygon::new(
        LineString::from(vec![
            (0., 0.),
            (10., 0.),
            (10., 4.),
            (4., 4.),
            (4., 10.),
            (0., 10.),
            (0., 0.),
        ]),
        vec![],
    )
    .into()
}
fn overlaps(rule: Rule, a: Geometry, b: Geometry) -> usize {
    let mut topology = Topology::new(0.).unwrap();
    let first = topology.add_layer(Layer::from_geometries("a", [a]));
    let second = topology.add_layer(Layer::from_geometries("b", [b]));
    topology.add_rule(rule, first, Some(second)).unwrap();
    topology.validate().unwrap().violation_count()
}
fn polygon(coords: &[(f64, f64)]) -> Geometry {
    Polygon::new(LineString::from(coords.to_vec()), vec![]).into()
}
fn square(x0: f64, y0: f64, x1: f64, y1: f64) -> Geometry {
    Rect::new(coord! {x: x0, y: y0}, coord! {x: x1, y: y1})
        .to_polygon()
        .into()
}

#[test]
fn polygons_inside_the_bounds_of_a_concave_polygon_still_overlap_it() {
    // The square lies inside the L's bounding box, but its corner is in the notch.
    let straddling = square(2., 2., 8., 8.);
    assert_eq!(
        overlaps(
            Rule::PolygonMustNotOverlapWith,
            straddling.clone(),
            l_shape()
        ),
        1
    );
    assert_eq!(
        overlaps(Rule::PolygonMustNotOverlapWith, l_shape(), straddling),
        1
    );
    // A square in the notch shares the bounding box but not any area.
    let notch = square(6., 6., 9., 9.);
    assert_eq!(
        overlaps(Rule::PolygonMustNotOverlapWith, notch, l_shape()),
        0
    );
}

#[test]
fn containment_is_certified_for_polygons_touching_their_container() {
    let container = square(0., 0., 10., 10.);
    for (name, inner) in [
        ("strictly inside", square(2., 2., 3., 3.)),
        ("sharing two edges", square(0., 0., 5., 5.)),
        (
            "touching at a vertex",
            polygon(&[(5., 0.), (7., 2.), (5., 4.), (3., 2.), (5., 0.)]),
        ),
        (
            "touching along part of an edge",
            polygon(&[(2., 0.), (6., 0.), (4., 3.), (2., 0.)]),
        ),
    ] {
        for (a, b) in [
            (inner.clone(), container.clone()),
            (container.clone(), inner.clone()),
        ] {
            assert_eq!(overlaps(Rule::PolygonMustNotOverlapWith, a, b), 1, "{name}");
        }
    }
}

#[test]
fn polygons_leaving_a_concave_container_through_its_vertices_overlap_it() {
    let outside_start = polygon(&[(5., 5.), (4., 4.), (2., 3.), (3., 6.), (5., 5.)]);
    // The first edge runs from inside the container, through its reflex vertex, into the notch.
    let through_vertex = polygon(&[(2., 2.), (6., 6.), (2., 6.), (2., 2.)]);
    for inner in [outside_start, through_vertex] {
        for (a, b) in [(inner.clone(), l_shape()), (l_shape(), inner.clone())] {
            assert_eq!(overlaps(Rule::PolygonMustNotOverlapWith, a, b), 1);
        }
    }
}

#[test]
fn parallel_diagonal_lines_beyond_the_tolerance_do_not_touch_a_boundary() {
    let triangle = polygon(&[(0., 0.), (10., 0.), (10., 10.), (0., 0.)]);
    let line: Geometry = LineString::from(vec![(0., 1.2), (8., 9.2)]).into();
    // The line is 1.2 / sqrt(2) = 0.85 from the hypotenuse, inside its bounding box.
    for (tolerance, touching) in [(0.5, 0), (1., 1)] {
        let mut topology = Topology::new(tolerance).unwrap();
        let lines = topology.add_layer(Layer::from_geometries("lines", [line.clone()]));
        let polygons = topology.add_layer(Layer::from_geometries("polygons", [triangle.clone()]));
        topology
            .add_rule(Rule::LineMustNotIntersectPolygons, lines, Some(polygons))
            .unwrap();
        assert_eq!(
            topology.validate().unwrap().violation_count(),
            touching,
            "tolerance {tolerance}"
        );
    }
}
