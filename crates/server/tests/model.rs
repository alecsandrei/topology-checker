use geo::{Geometry, GeometryCollection, LineString, MultiLineString, MultiPoint, Point, polygon};
use serde_json::{Map, Value};
use std::{path::PathBuf, sync::Arc};
use topology_checker_core::{Layer, Rule, Topology};
use topology_checker_web::{export, model::*};
fn source(id: &str, crs: Option<&str>, features: Vec<(i64, Geometry)>) -> Arc<SourceLayer> {
    let attributes = Arc::new(features.iter().map(|_| Map::new()).collect());
    let layer = Layer::from_features(id, features);
    let kinds = kinds(&layer);
    Arc::new(SourceLayer {
        id: id.into(),
        dataset: id.into(),
        selector: id.into(),
        member: PathBuf::from("fixture.geojson"),
        name: id.into(),
        crs: crs.map(String::from),
        warnings: vec!["source warning".into()],
        layer: Arc::new(layer),
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
    let a = direct.add_layer(Arc::clone(&sources[0].layer));
    let b = direct.add_layer(Arc::clone(&sources[1].layer));
    direct
        .add_rule(Rule::LineMustNotIntersectPolygons, a, Some(b))
        .unwrap();
    let result = ValidationResult::run(7, cfg, &prepared).unwrap();
    let direct = direct.validate().unwrap();
    assert_eq!(
        result.report.violations().next().unwrap().geometry,
        direct.violations().next().unwrap().geometry
    );
    assert_eq!(result.report.violation_count(), 1);
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
    assert!(matches!(
        row.geometry.value,
        geojson::Value::GeometryCollection(_)
    ));
    let origin = row.origin.as_ref().unwrap();
    assert_eq!(origin.fid, "-12");
    assert_eq!(row.destination.as_ref().unwrap().fid, "9007199254740993");
    assert!(origin.part.is_none());
    assert_eq!(
        export::geojson(&result).unwrap()["features"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("report.gpkg");
    export::dataset(&result, &path).unwrap();
    let read = topology_checker_io::read::read_with_attributes(&path, None, "report", true)
        .unwrap()
        .remove(0);
    assert_eq!(read.layer.len(), 1);
    assert!(matches!(
        read.layer.features().next().unwrap().1,
        Geometry::GeometryCollection(_)
    ));
    // The web export shares the CLI report columns.
    let attributes = &read.attributes[0];
    let mut columns = attributes.keys().map(String::as_str).collect::<Vec<_>>();
    columns.sort_unstable();
    assert_eq!(
        columns,
        [
            "destination_fid",
            "destination_layer",
            "destination_part",
            "error_type",
            "origin_fid",
            "origin_layer",
            "origin_part",
            "rule",
            "violation_id",
        ]
    );
    assert_eq!(attributes["violation_id"], 0);
    assert_eq!(attributes["origin_fid"], -12);
    assert_eq!(attributes["origin_part"], Value::Null);
    for extension in ["fgb", "parquet", "kml", "kmz"] {
        let path = dir.path().join(format!("report.{extension}"));
        export::dataset(&result, &path).unwrap();
        let read = topology_checker_io::read::read(&path, None, "report").unwrap();
        assert_eq!(read[0].layer.len(), 1, "{extension}");
        assert!(
            matches!(
                read[0].layer.features().next().unwrap().1,
                Geometry::GeometryCollection(_)
            ),
            "{extension}"
        );
    }
    assert!(export::dataset(&result, &dir.path().join("report.gpx")).is_err());
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
    let preview = prepared.layers[0]
        .viewport([-1., -1., 13000., 2.], None, false)
        .unwrap();
    assert_eq!(preview.features.len(), FEATURE_LIMIT);
    assert!(preview.incomplete);
    let result = ValidationResult::run(1, cfg, &prepared).unwrap();
    assert_eq!(result.report.violation_count(), 12_001);
    assert_eq!(result.page(0).unwrap().violations.len(), 100);
    assert_eq!(result.page(120).unwrap().violations.len(), 1);
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
    let viewport = prepared.layers[0]
        .viewport([-1., -1., 1e6, 1.], None, false)
        .unwrap();
    assert!(viewport.features.is_empty());
    assert!(viewport.incomplete);
    // Geometry already in the working CRS is shared, not copied.
    assert!(Arc::ptr_eq(&sources[0].layer, &prepared.layers[0].layer));
}
#[test]
fn mixed_crs_reprojection_source_assignment_and_display_coordinates_are_separate() {
    let projected = topology_checker_io::Transform::new(
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
    assert_eq!(prepared.layers[0].layer.feature(0).unwrap().1, &g);
    assert_eq!(
        sources[0].layer.feature(0).unwrap().1,
        &Point::new(5., 52.).into()
    );
    assert!(!Arc::ptr_eq(&sources[0].layer, &prepared.layers[0].layer));
    assert_eq!(
        ValidationResult::run(1, cfg, &prepared)
            .unwrap()
            .report
            .violation_count(),
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
    let display = prepared.layers[0]
        .viewport(
            [500000., 6700000., 600000., 6900000.],
            Some("EPSG:4326"),
            true,
        )
        .unwrap();
    assert_eq!(display.features.len(), 1);
    let geojson::Value::Point(point) = &display.features[0].geometry.value else {
        panic!("expected a point");
    };
    assert!(point[0] > 500000.);
    assert_eq!(
        prepared.layers[0].layer.feature(0).unwrap().1,
        &Point::new(5., 52.).into()
    );
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
        ValidationResult::run(1, cfg.clone(), &prepare(&cfg, &sources).unwrap()).unwrap()
    };
    assert_eq!(run(cfg.clone()).report.violation_count(), 0);
    cfg.tolerance = 0.1;
    assert_eq!(run(cfg.clone()).report.violation_count(), 1);
    assert!(
        run(cfg.clone())
            .summary()
            .warnings
            .contains(&"source warning".to_string())
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
    let result = ValidationResult::run(1, cfg.clone(), &prepare(&cfg, &sources).unwrap()).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("empty.gpkg");
    export::dataset(&result, &path).unwrap();
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
    let result = ValidationResult::run(1, raw.clone(), &prepare(&raw, &sources).unwrap()).unwrap();
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
    let result = ValidationResult::run(1, cfg.clone(), &prepare(&cfg, &sources).unwrap()).unwrap();
    assert_eq!(result.report.violation_count(), 1);
    assert_eq!(
        result.report.violations().next().unwrap().geometry,
        Geometry::GeometryCollection(geometry)
    );
}
#[test]
fn reused_topology_matches_fresh_validation_across_rules_and_tolerances() {
    let sources = vec![source(
        "roads",
        None,
        vec![
            (1, LineString::from(vec![(0., 0.), (2., 0.)]).into()),
            (2, LineString::from(vec![(1., -1.), (1., 0.05)]).into()),
        ],
    )];
    let mut cfg = config(
        &sources,
        vec![check(Rule::LineMustNotHaveDangles, "roads", None)],
        None,
    );
    let prepared = prepare(&cfg, &sources).unwrap();
    let count = |cfg: &Config, prepared: &Prepared| {
        ValidationResult::run(1, cfg.clone(), prepared)
            .unwrap()
            .report
            .violation_count()
    };
    let mut counts = Vec::new();
    for (rule, tolerance) in [
        (Rule::LineMustNotHaveDangles, 0.),
        (Rule::LineMustNotIntersect, 0.),
        (Rule::LineMustNotHaveDangles, 0.1),
        (Rule::LineMustNotIntersectOrTouchInterior, 0.1),
        (Rule::LineMustNotHaveDangles, 0.),
    ] {
        cfg.checks = vec![check(rule, "roads", None)];
        cfg.tolerance = tolerance;
        let reused = count(&cfg, &prepared);
        assert_eq!(
            reused,
            count(&cfg, &prepare(&cfg, &sources).unwrap()),
            "{rule} at {tolerance}"
        );
        counts.push(reused);
    }
    // The near miss becomes a junction within tolerance.
    assert_eq!(counts[0], 4);
    assert_eq!(counts[2], 3);
    assert_eq!(counts[4], 4);
}
