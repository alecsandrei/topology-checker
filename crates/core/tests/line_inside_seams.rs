//! Lines running along or across the seams between adjacent polygons.
use topology_checker_core::{
    Layer, Rule, Topology,
    geo::{Coord, Geometry, LineString, Rect},
};

fn rectangle(x0: f64, y0: f64, x1: f64, y1: f64) -> Geometry {
    Rect::new(Coord { x: x0, y: y0 }, Coord { x: x1, y: y1 })
        .to_polygon()
        .into()
}
fn outside_pieces(gap: f64, line: &[(f64, f64)], tolerance: f64) -> usize {
    let mut topology = Topology::new(tolerance).unwrap();
    let source = topology.add_layer(Layer::from_geometries(
        "line",
        [LineString::from(line.to_vec()).into()],
    ));
    let target = topology.add_layer(Layer::from_geometries(
        "parcels",
        [
            rectangle(0., 0., 2., 2.),
            rectangle(2. + gap, 0., 4. + gap, 2.),
        ],
    ));
    topology
        .add_rule(Rule::LineMustBeInside, source, Some(target))
        .unwrap();
    topology.validate().unwrap().violation_count()
}

#[test]
fn lines_along_a_shared_edge_stay_inside_the_union() {
    for tolerance in [0., 0.25] {
        assert_eq!(
            outside_pieces(0., &[(2., 0.5), (2., 1.5)], tolerance),
            0,
            "{tolerance}"
        );
        // Running past the end of the seam leaves the polygons.
        assert_eq!(
            outside_pieces(0., &[(2., 1.5), (2., 3.)], tolerance),
            1,
            "{tolerance}"
        );
    }
}

#[test]
fn narrow_channels_between_polygons_are_bridged_by_the_tolerance() {
    let across = [(1., 1.), (3.05, 1.)];
    // The line runs down the middle of the gap.
    for (gap, expected) in [(0.1, 0), (0.7, 1)] {
        let middle = 2. + gap / 2.;
        let along = [(middle, 0.5), (middle, 1.5)];
        assert_eq!(outside_pieces(gap, &along, 0.25), expected, "along {gap}");
    }
    assert_eq!(outside_pieces(0.1, &across, 0.25), 0);
    assert_eq!(outside_pieces(0.7, &[(1., 1.), (3.35, 1.)], 0.25), 1);
}
