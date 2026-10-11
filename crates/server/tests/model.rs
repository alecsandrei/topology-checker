use geo::{Geometry, GeometryCollection, LineString, MultiLineString, MultiPoint, Point, polygon};
use serde_json::{Map, Value};
use std::{path::PathBuf, sync::Arc};
use topology_checker_core::{Layer, Rule, Topology};
use topology_checker_web::{export, model::*};
fn source(id: &str, crs: Option<&str>, features: Vec<(i64, Geometry)>) -> Arc<SourceLayer> {
    let attributes = Arc::new(features.iter().map(|_| Map::new()).collect());
    let kinds = kinds(&features);
    Arc::new(SourceLayer {
        id: id.into(),
        dataset: id.into(),
        selector: id.into(),
        member: PathBuf::from("fixture.geojson"),
        name: id.into(),
        crs: crs.map(String::from),
        warnings: vec!["source warning".into()],
        features,
        attributes,
        kinds,
    })
}
fn config(sources: &[Arc<SourceLayer>], checks: Vec<Check>, crs: Option<&str>) -> Config {
    Config {
        working_crs: crs.map(String::from),
        tolerance: 0.,
        layers: sources
            .iter()
            .map(|s| LayerSettings {
                id: s.id.clone(),
                selected: true,
                visible: true,
                color: "#28786f".into(),
                source_crs: None,
            })
            .collect(),
        checks,
        view: ViewSettings::default(),
    }
}
fn check(rule: Rule, layer: &str, other: Option<&str>) -> Check {
    Check {
        rule,
        layer: layer.into(),
        other: other.map(String::from),
        enabled: true,
    }
}
#[test]
fn binary_mixed_geometry_results_match_direct_core_and_preserve_counts_in_exports() {
    let line = MultiLineString(vec![
        LineString::from(vec![(1., 1.), (2., 1.)]),
        LineString::from(vec![(-1., 1.), (0., 0.), (1., -1.)]),
    ]);
    let square = polygon![(x:0.,y:0.),(x:4.,y:0.),(x:4.,y:4.),(x:0.,y:4.),(x:0.,y:0.)];
    let sources = vec![
        source("line", Some("EPSG:3857"), vec![(-12, line.into())]),
        source(
            "polygon",
            Some("EPSG:3857"),
            vec![(9007199254740993, square.into())],
        ),
    ];
    let cfg = config(
        &sources,
        vec![check(
            Rule::LineMustNotIntersectPolygons,
            "line",
            Some("polygon"),
        )],
        Some("EPSG:3857"),
    );
    let prepared = prepare(&cfg, &sources).unwrap();
    let mut direct = Topology::new(0.).unwrap();
    let a = direct.add_layer(Layer::from_features("line", sources[0].features.clone()));
    let b = direct.add_layer(Layer::from_features("polygon", sources[1].features.clone()));
    direct
        .add_rule(Rule::LineMustNotIntersectPolygons, a, Some(b))
        .unwrap();
    let result = ValidationResult::run(7, cfg, prepared).unwrap();
    let direct = direct.validate().unwrap();
    assert_eq!(
        result.report.errors().next().unwrap().geometry,
        direct.errors().next().unwrap().geometry
    );
    assert_eq!(result.report.error_count(), 1);
    let row = result.violation(0, false).unwrap();
    let mut streamed = Vec::new();
    export::write_json(&result, &mut streamed).unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&streamed).unwrap(),
        result.complete_json().unwrap()
    );
    streamed.clear();
    export::write_geojson(&result, &mut streamed).unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&streamed).unwrap(),
        export::geojson(&result).unwrap()
    );
    assert_eq!(row["geometry"]["type"], "GeometryCollection");
    assert_eq!(row["origin"]["fid"], "-12");
    assert_eq!(row["destination"]["fid"], "9007199254740993");
    assert!(row["origin"]["part"].is_null());
    assert_eq!(
        export::geojson(&result).unwrap()["features"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("report.gpkg");
    export::geopackage(&result, &path).unwrap();
    let read = topology_checker_io::read::read_with_attributes(&path, None, "report", true)
        .unwrap()
        .remove(0);
    assert_eq!(read.layer.len(), 1);
    assert!(matches!(
        read.layer.features().next().unwrap().1,
        Geometry::GeometryCollection(_)
    ));
}
#[test]
fn viewport_limits_do_not_limit_validation_and_paginate_aggregated_violations() {
    let features = (0..12_001)
        .map(|i| {
            (
                i,
                MultiPoint(vec![Point::new(i as f64, 0.), Point::new(i as f64, 1.)]).into(),
            )
        })
        .collect();
    let sources = vec![source("many", None, features)];
    let cfg = config(
        &sources,
        vec![check(Rule::GeometryMustNotBeMultipart, "many", None)],
        None,
    );
    let prepared = prepare(&cfg, &sources).unwrap();
    let preview = prepared[0]
        .viewport([-1., -1., 13000., 2.], None, false)
        .unwrap();
    assert_eq!(preview["features"].as_array().unwrap().len(), FEATURE_LIMIT);
    assert_eq!(preview["incomplete"], true);
    let result = ValidationResult::run(1, cfg, prepared).unwrap();
    assert_eq!(result.report.error_count(), 12_001);
    assert_eq!(
        result.page(0).unwrap()["violations"]
            .as_array()
            .unwrap()
            .len(),
        100
    );
    assert_eq!(
        result.page(120).unwrap()["violations"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(result.page(usize::MAX).is_err());
}
#[test]
fn oversized_geometry_is_omitted_from_preview_without_simplification() {
    let line = LineString::from(
        (0..COORDINATE_LIMIT + 1)
            .map(|i| (i as f64, 0.))
            .collect::<Vec<_>>(),
    );
    let sources = vec![source("long", None, vec![(-12, line.into())])];
    let prepared = prepare(&config(&sources, vec![], None), &sources).unwrap();
    let viewport = prepared[0]
        .viewport([-1., -1., 1e6, 1.], None, false)
        .unwrap();
    assert!(viewport["features"].as_array().unwrap().is_empty());
    assert_eq!(viewport["incomplete"], true);
    assert_eq!(sources[0].features[0].1, prepared[0].features[0].1);
}
#[test]
fn mixed_crs_reprojection_source_assignment_and_display_coordinates_are_separate() {
    let projected = gdal::spatial_ref::CoordTransform::new(
        &topology_checker_io::crs::parse("EPSG:4326").unwrap(),
        &topology_checker_io::crs::parse("EPSG:3857").unwrap(),
    )
    .unwrap();
    let g = topology_checker_io::crs::transform(&Point::new(5., 52.).into(), &projected).unwrap();
    let sources = vec![
        source(
            "geographic",
            Some("EPSG:4326"),
            vec![(-12, Point::new(5., 52.).into())],
        ),
        source("projected", Some("EPSG:3857"), vec![(42, g.clone())]),
    ];
    let cfg = config(
        &sources,
        vec![check(
            Rule::PointMustNotOverlapWith,
            "geographic",
            Some("projected"),
        )],
        Some("EPSG:3857"),
    );
    let prepared = prepare(&cfg, &sources).unwrap();
    assert_eq!(prepared[0].features[0].1, g);
    assert_eq!(sources[0].features[0].1, Point::new(5., 52.).into());
    assert_eq!(
        ValidationResult::run(1, cfg, prepared)
            .unwrap()
            .report
            .error_count(),
        1
    );
    let unknown = vec![source(
        "unknown",
        None,
        vec![(0, Point::new(5., 52.).into())],
    )];
    let mut cfg = config(&unknown, vec![], Some("EPSG:3857"));
    assert!(
        prepare(&cfg, &unknown)
            .unwrap_err()
            .to_string()
            .contains("source CRS")
    );
    cfg.layers[0].source_crs = Some("EPSG:4326".into());
    assert!(prepare(&cfg, &unknown).is_ok());
    let geographic = config(&sources[..1], vec![], Some("EPSG:4326"));
    let prepared = prepare(&geographic, &sources[..1]).unwrap();
    let display = prepared[0]
        .viewport(
            [500000., 6700000., 600000., 6900000.],
            Some("EPSG:4326"),
            true,
        )
        .unwrap();
    assert_eq!(display["features"].as_array().unwrap().len(), 1);
    assert!(
        display["features"][0]["geometry"]["coordinates"][0]
            .as_f64()
            .unwrap()
            > 500000.
    );
    assert_eq!(prepared[0].features[0].1, Point::new(5., 52.).into());
}
#[test]
fn tolerance_changes_match_core_and_incompatible_checks_are_rejected() {
    let sources = vec![source(
        "points",
        None,
        vec![
            (-12, Point::new(0., 0.).into()),
            (42, Point::new(0.05, 0.).into()),
        ],
    )];
    let mut cfg = config(
        &sources,
        vec![check(Rule::PointMustNotOverlap, "points", None)],
        None,
    );
    let run = |cfg: Config| {
        ValidationResult::run(1, cfg.clone(), prepare(&cfg, &sources).unwrap()).unwrap()
    };
    assert_eq!(run(cfg.clone()).report.error_count(), 0);
    cfg.tolerance = 0.1;
    assert_eq!(run(cfg.clone()).report.error_count(), 1);
    assert!(
        run(cfg.clone()).summary()["warnings"]
            .as_array()
            .unwrap()
            .contains(&Value::String("source warning".into()))
    );
    cfg.checks[0].rule = Rule::LineMustNotHaveDangles;
    assert!(normalize_config(&mut cfg.clone(), &cfg, &sources).is_err());
}
#[test]
fn clean_geopackage_and_raw_xy_exports_have_explicit_behavior() {
    let sources = vec![source("empty", Some("EPSG:3857"), vec![])];
    let cfg = config(
        &sources,
        vec![check(Rule::GeometryMustNotBeMultipart, "empty", None)],
        Some("EPSG:3857"),
    );
    let result = ValidationResult::run(1, cfg.clone(), prepare(&cfg, &sources).unwrap()).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("empty.gpkg");
    export::geopackage(&result, &path).unwrap();
    assert!(
        topology_checker_io::read::read(&path, None, "empty").unwrap()[0]
            .layer
            .is_empty()
    );
    let raw = config(
        &sources,
        vec![check(Rule::GeometryMustNotBeMultipart, "empty", None)],
        None,
    );
    let result = ValidationResult::run(1, raw.clone(), prepare(&raw, &sources).unwrap()).unwrap();
    assert!(export::geojson(&result).is_err());
}
#[test]
fn all_geometry_collection_components_stay_one_violation() {
    let geometry = GeometryCollection(vec![
        Point::new(0., 0.).into(),
        LineString::from(vec![(0., 0.), (1., 1.)]).into(),
    ]);
    let sources = vec![source(
        "collection",
        None,
        vec![(-12, Geometry::GeometryCollection(geometry.clone()))],
    )];
    let cfg = config(
        &sources,
        vec![check(Rule::GeometryMustNotBeMultipart, "collection", None)],
        None,
    );
    let result = ValidationResult::run(1, cfg.clone(), prepare(&cfg, &sources).unwrap()).unwrap();
    assert_eq!(result.report.error_count(), 1);
    assert_eq!(
        result.report.errors().next().unwrap().geometry,
        Geometry::GeometryCollection(geometry)
    );
}
