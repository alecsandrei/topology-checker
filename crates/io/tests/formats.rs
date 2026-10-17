use std::path::{Path, PathBuf};
use topology_checker_core::geo::{Geometry, LineString, Point, Polygon, polygon};
use topology_checker_io::{
    crs, read,
    write::{self, DatasetLayer, Record},
};

const CUSTOM: &str = r#"PROJCS["custom foot",GEOGCS["GCS",DATUM["D",SPHEROID["WGS 84",6378137,298.257223563]],PRIMEM["Greenwich",0],UNIT["degree",0.0174532925199433]],PROJECTION["Transverse_Mercator"],PARAMETER["latitude_of_origin",0],PARAMETER["central_meridian",9],PARAMETER["scale_factor",0.9996],PARAMETER["false_easting",500000],PARAMETER["false_northing",0],UNIT["foot",0.3048]]"#;

fn text(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, body).unwrap();
    path
}
fn layer(srs: Option<&str>, geometries: Vec<Geometry>) -> DatasetLayer {
    DatasetLayer::new(
        "layer",
        srs.map(|s| crs::parse(s).unwrap()),
        geometries
            .into_iter()
            .enumerate()
            .map(|(i, g)| Record {
                id: Some(i as i64 + 1),
                geometry: Some(g),
                properties: Default::default(),
            })
            .collect(),
    )
}
fn error(path: &Path) -> String {
    format!("{:#}", read::read(path, None, "x").unwrap_err())
}

#[test]
fn geojson_roots_crs_members_and_structure_errors() {
    let dir = tempfile::tempdir().unwrap();
    let point = r#"{"type":"Point","coordinates":[5,52]}"#;
    let path = text(dir.path(), "bare-geometry.geojson", point);
    assert_eq!(read::read(&path, None, "x").unwrap()[0].layer.len(), 1);
    let path = text(
        dir.path(),
        "bare-feature.geojson",
        &format!(r#"{{"type":"Feature","properties":null,"geometry":{point}}}"#),
    );
    assert_eq!(read::read(&path, None, "x").unwrap()[0].layer.len(), 1);

    let projected = text(
        dir.path(),
        "projected.geojson",
        r#"{"type":"FeatureCollection","crs":{"type":"name","properties":{"name":"EPSG:3857"}},"features":[]}"#,
    );
    let layers = read::read(&projected, None, "x").unwrap();
    assert_eq!(layers[0].srs.as_ref().unwrap().epsg(), Some(3857));

    for (name, body, message) in [
        (
            "crs.geojson",
            r#"{"type":"FeatureCollection","crs":{"type":"link"},"features":[]}"#,
            "unsupported GeoJSON CRS",
        ),
        (
            "features.geojson",
            r#"{"type":"FeatureCollection","features":{}}"#,
            "features must be an array",
        ),
        (
            "geometry.geojson",
            r#"{"type":"FeatureCollection","features":[{"type":"Feature","properties":{}}]}"#,
            "missing geometry",
        ),
        (
            "properties.geojson",
            &format!(
                r#"{{"type":"FeatureCollection","features":[{{"type":"Feature","properties":[1],"geometry":{point}}}]}}"#
            ),
            "properties must be an object",
        ),
    ] {
        let path = text(dir.path(), name, body);
        assert!(error(&path).contains(message), "{name}: {}", error(&path));
    }
}

#[test]
fn custom_crs_survives_geojson_and_flatgeobuf_and_shapefile() {
    let dir = tempfile::tempdir().unwrap();
    for ext in ["geojson", "fgb", "shp"] {
        let path = dir.path().join(format!("custom.{ext}"));
        write::dataset(
            &path,
            None,
            false,
            &[layer(Some(CUSTOM), vec![Point::new(5., 52.).into()])],
        )
        .unwrap();
        let read = read::read(&path, None, "x").unwrap();
        let srs = read[0].srs.as_ref().unwrap();
        assert_eq!(srs.epsg(), None, "{ext}");
        assert_eq!(srs.linear_units_name().as_deref(), Some("foot"), "{ext}");
    }
}

#[test]
fn holes_collections_and_special_shapes_roundtrip_through_flatgeobuf() {
    let dir = tempfile::tempdir().unwrap();
    let holed = Polygon::new(
        LineString::from(vec![(0., 0.), (10., 0.), (10., 10.), (0., 0.)]),
        vec![LineString::from(vec![
            (1., 1.),
            (2., 1.),
            (2., 2.),
            (1., 1.),
        ])],
    );
    let geometries: Vec<Geometry> = vec![
        holed.into(),
        Geometry::Line(topology_checker_core::geo::Line::new((0., 0.), (1., 1.))),
        Geometry::Rect(topology_checker_core::geo::Rect::new((0., 0.), (1., 1.))),
        Geometry::Triangle(topology_checker_core::geo::Triangle::new(
            (0., 0.).into(),
            (1., 0.).into(),
            (1., 1.).into(),
        )),
        polygon![(x: 0., y: 0.), (x: 1., y: 0.), (x: 1., y: 1.), (x: 0., y: 0.)].into(),
    ];
    let mut dataset = layer(None, geometries);
    for record in &mut dataset.records {
        // Collides with the identity column the format reserves for feature ids.
        record
            .properties
            .insert("__topology_fid".into(), "mine".into());
    }
    dataset.fields = DatasetLayer::new("x", None, dataset.records.clone()).fields;
    let path = dir.path().join("shapes.fgb");
    write::dataset(&path, None, false, &[dataset]).unwrap();
    let read = read::read_with_attributes(&path, None, "x", true).unwrap();
    assert_eq!(read[0].layer.len(), 5);
    assert_eq!(read[0].attributes[0]["__topology_fid"], "mine");
    assert_eq!(read[0].srs, None);
}

#[test]
fn shapefiles_skip_deleted_rows_and_write_without_a_crs() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("plain.shp");
    write::dataset(
        &path,
        None,
        false,
        &[layer(
            None,
            vec![Point::new(1., 2.).into(), Point::new(3., 4.).into()],
        )],
    )
    .unwrap();
    assert!(!path.with_extension("prj").exists());
    let before = read::read(&path, None, "x").unwrap();
    assert_eq!(before[0].srs, None);
    assert_eq!(before[0].layer.len(), 2);

    // Mark the first record deleted, as other tools do when editing in place.
    let dbf = path.with_extension("dbf");
    let mut bytes = std::fs::read(&dbf).unwrap();
    let header = u16::from_le_bytes([bytes[8], bytes[9]]) as usize;
    bytes[header] = b'*';
    std::fs::write(&dbf, bytes).unwrap();
    let after = read::read(&path, None, "x").unwrap();
    assert_eq!(after[0].layer.len(), 1);
}
