use gdal::{
    DriverManager,
    spatial_ref::SpatialRef,
    vector::{Feature, LayerAccess, LayerOptions, OGRFieldType, OGRwkbGeometryType, ToGdal},
};
use geo::{Geometry, GeometryCollection, LineString, Point, polygon};
use topology_checker_io::{crs, read};

#[test]
fn attributes_ids_and_import_warnings_survive_geojson_reading() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("attributes.geojson");
    std::fs::write(&path,r#"{"type":"FeatureCollection","features":[{"type":"Feature","id":-12,"properties":{"name":"negative","nullable":null},"geometry":{"type":"Point","coordinates":[5,52,3]}},{"type":"Feature","id":9007199254740993,"properties":{"name":"large","large":9007199254740993,"tags":["a","b"]},"geometry":{"type":"Point","coordinates":[6,53]}},{"type":"Feature","properties":{},"geometry":null},{"type":"Feature","properties":{},"geometry":{"type":"LineString","coordinates":[]}}]}"#).unwrap();
    let data = read::read_with_attributes(&path, None, "test", true)
        .unwrap()
        .remove(0);
    assert_eq!(
        data.layer.features().map(|(id, _)| id).collect::<Vec<_>>(),
        vec![-12, 9007199254740993]
    );
    assert_eq!(data.attributes[0]["name"], "negative");
    assert!(data.attributes[0]["nullable"].is_null());
    assert_eq!(data.attributes[1]["large"], "9007199254740993");
    assert!(data.warnings.iter().any(|w| w.contains("null geometries")));
    assert!(data.warnings.iter().any(|w| w.contains("empty geometries")));
    assert!(data.warnings.iter().any(|w| w.contains("Z/M")));
}
#[test]
fn every_vector_format_and_multilayer_selection_is_readable() {
    let dir = tempfile::tempdir().unwrap();
    for (driver, extension) in [
        ("GeoJSON", "geojson"),
        ("GPKG", "gpkg"),
        ("ESRI Shapefile", "shp"),
        ("FlatGeobuf", "fgb"),
    ] {
        let path = dir.path().join(format!("{driver}.{extension}"));
        {
            let mut dataset = DriverManager::get_driver_by_name(driver)
                .unwrap()
                .create_vector_only(&path)
                .unwrap();
            let srs = SpatialRef::from_epsg(28992).unwrap();
            let count = if driver == "GPKG" { 2 } else { 1 };
            for i in 0..count {
                let name = format!("points{i}");
                let layer = dataset
                    .create_layer(LayerOptions {
                        name: &name,
                        srs: Some(&srs),
                        ty: OGRwkbGeometryType::wkbPoint,
                        ..Default::default()
                    })
                    .unwrap();
                layer
                    .create_defn_fields(&[("label", OGRFieldType::OFTString)])
                    .unwrap();
                let mut feature = Feature::new(layer.defn()).unwrap();
                feature
                    .set_geometry(Point::new(155000., 463000.).to_gdal().unwrap())
                    .unwrap();
                feature.set_field_string(0, "retained").unwrap();
                feature.create(&layer).unwrap();
            }
        }
        let layers = read::read_with_attributes(&path, None, "fixture", true).unwrap();
        assert_eq!(layers.len(), if driver == "GPKG" { 2 } else { 1 });
        for layer in &layers {
            assert_eq!(layer.layer.len(), 1);
            assert_eq!(layer.attributes[0]["label"], "retained");
            assert!(layer.srs.is_some());
        }
        if driver == "GPKG" {
            assert_eq!(read::read(&path, Some("points1"), "one").unwrap().len(), 1);
        }
    }
}
#[test]
fn transformations_use_xy_axis_order_and_keep_collection_parts() {
    let source = crs::parse("EPSG:4326").unwrap();
    let target = crs::parse("EPSG:3857").unwrap();
    let transform = gdal::spatial_ref::CoordTransform::new(&source, &target).unwrap();
    let g = Geometry::GeometryCollection(GeometryCollection(vec![
        Point::new(5., 52.).into(),
        LineString::from(vec![(5., 52.), (6., 53.)]).into(),
        polygon![(x:5.,y:52.),(x:6.,y:52.),(x:6.,y:53.),(x:5.,y:52.)].into(),
    ]));
    let output = crs::transform(&g, &transform).unwrap();
    let Geometry::GeometryCollection(c) = output else {
        panic!("collection changed")
    };
    let Geometry::Point(point) = &c[0] else {
        panic!("point changed")
    };
    assert!((point.x() - 556597.4539663679).abs() < 1e-6);
    assert!((point.y() - 6800125.454397307).abs() < 1e-6);
    assert_eq!(c.len(), 3);
    assert!(crs::transform(&Point::new(5., 100.).into(), &transform).is_err());
    assert_eq!(read::default_tolerance(Some(&source)), 1e-8);
    assert_eq!(read::default_tolerance(Some(&target)), 0.001);
}
#[test]
fn inspection_rejects_disguised_unsupported_datasets() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("data.geojson");
    std::fs::write(&path, "<OGRVRTDataSource></OGRVRTDataSource>").unwrap();
    assert!(read::read_with_attributes(&path, None, "bad", true).is_err());
}
