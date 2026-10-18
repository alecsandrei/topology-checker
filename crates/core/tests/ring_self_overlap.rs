//! One feature made of nearly coincident closed rings overlaps itself within tolerance.
use topology_checker_core::{
    Layer, Rule, Topology,
    geo::{Geometry, LineString, MultiLineString},
};

const RINGS: [&[(f64, f64)]; 3] = [
    &[
        (2.81955, 0.026325),
        (6.206325, -0.142425),
        (6.224775, 6.210375),
        (-0.0072, 5.83305),
        (-0.081225, 3.21285),
        (-0.078975, 0.029025),
        (2.81955, 0.026325),
    ],
    &[
        (2.09675, 0.05085),
        (0.010575, 0.182025),
        (-0.075825, 2.068175),
        (0.154575, 4.007875),
        (0.13905, 5.813925),
        (1.9307, 6.072675),
        (4.1809, 5.945325),
        (5.861175, 6.11565),
        (6.088425, -0.0963),
        (4.08775, -0.1647),
        (2.09675, 0.05085),
    ],
    &[
        (5.8056, -0.0972),
        (3.933175, 0.1989),
        (2.158175, -0.03915),
        (-0.1881, -0.089775),
        (0.080775, 5.838675),
        (1.804925, 6.181575),
        (3.984475, 6.041175),
        (6.043875, 5.954325),
        (5.8056, -0.0972),
    ],
];

#[test]
fn closed_rings_of_one_feature_overlap_each_other() {
    let rings = RINGS
        .iter()
        .map(|ring| LineString::from(ring.to_vec()))
        .collect();
    let feature: Geometry = MultiLineString(rings).into();
    for rule in [Rule::LineMustNotSelfOverlap, Rule::LineMustNotSelfIntersect] {
        let mut topology = Topology::new(0.25).unwrap();
        let layer = topology.add_layer(Layer::from_geometries("rings", [feature.clone()]));
        topology.add_rule(rule, layer, None).unwrap();
        let report = topology.validate().unwrap();
        assert!(report.violation_count() > 0, "{rule:?}");
    }
}
