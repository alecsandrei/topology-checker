//! Files as GDAL-based applications write them.
use geo::Point;
use serde_json::json;
use topology_checker_io::{
    crs, read,
    write::{self, DatasetLayer, Record},
};

/// Definition of EPSG:3857 as GDAL writes it into GeoPackages and `.prj` files.
const GDAL_WEB_MERCATOR: &str = r#"PROJCS["WGS 84 / Pseudo-Mercator",GEOGCS["WGS 84",DATUM["WGS_1984",SPHEROID["WGS 84",6378137,298.257223563,AUTHORITY["EPSG","7030"]],AUTHORITY["EPSG","6326"]],PRIMEM["Greenwich",0,AUTHORITY["EPSG","8901"]],UNIT["degree",0.0174532925199433,AUTHORITY["EPSG","9122"]],AUTHORITY["EPSG","4326"]],PROJECTION["Mercator_1SP"],PARAMETER["central_meridian",0],PARAMETER["scale_factor",1],PARAMETER["false_easting",0],PARAMETER["false_northing",0],UNIT["metre",1,AUTHORITY["EPSG","9001"]],AXIS["Easting",EAST],AXIS["Northing",NORTH],EXTENSION["PROJ4","+proj=merc +a=6378137 +b=6378137 +lat_ts=0 +lon_0=0 +x_0=0 +y_0=0 +k=1 +units=m +nadgrids=@null +wktext +no_defs"],AUTHORITY["EPSG","3857"]]"#;
const SPHERICAL_EXTENSION: &str = r#"EXTENSION["PROJ4","+proj=merc +a=6378137 +b=6378137 +lat_ts=0 +lon_0=0 +x_0=0 +y_0=0 +k=1 +units=m +nadgrids=@null +wktext +no_defs"],"#;

/// An altered definition may be rejected, or read as some other CRS by the GDAL fallback (when
/// built with it), but never as EPSG:3857.
fn assert_not_web_mercator(definition: &str, message: &str) {
    if let Ok(parsed) = crs::parse(definition) {
        assert_ne!(parsed, crs::parse("EPSG:3857").unwrap(), "{message}");
    }
}

#[test]
fn gdal_web_mercator_definition_is_epsg_3857() {
    let parsed = crs::parse(GDAL_WEB_MERCATOR).unwrap();
    assert_eq!(parsed.epsg(), Some(3857));
    assert_eq!(parsed, crs::parse("EPSG:3857").unwrap());
}

#[test]
fn ellipsoidal_mercator_tagged_as_web_mercator_is_not_web_mercator() {
    assert!(GDAL_WEB_MERCATOR.contains(SPHERICAL_EXTENSION));
    let ellipsoidal = GDAL_WEB_MERCATOR.replace(SPHERICAL_EXTENSION, "");
    assert_not_web_mercator(&ellipsoidal, "ellipsoidal Mercator");
}

#[test]
fn only_gdals_exact_definition_is_taken_for_epsg_3857() {
    for (from, to) in [
        (
            r#"PARAMETER["false_easting",0]"#,
            r#"PARAMETER["false_easting",500000]"#,
        ),
        (
            r#"PARAMETER["central_meridian",0]"#,
            r#"PARAMETER["central_meridian",10]"#,
        ),
        (
            r#"PARAMETER["scale_factor",1]"#,
            r#"PARAMETER["scale_factor",0.9996]"#,
        ),
        ("+lat_ts=0", "+lat_ts=45"),
        ("+x_0=0", "+x_0=500000"),
        (r#"UNIT["metre",1,"#, r#"UNIT["foot",0.3048,"#),
        (r#"DATUM["WGS_1984""#, r#"DATUM["NAD83""#),
        ("298.257223563", "300"),
        (
            r#"AXIS["Easting",EAST],AXIS["Northing",NORTH]"#,
            r#"AXIS["Northing",NORTH],AXIS["Easting",EAST]"#,
        ),
    ] {
        assert!(GDAL_WEB_MERCATOR.contains(from), "{from}");
        let altered = GDAL_WEB_MERCATOR.replace(from, to);
        assert_not_web_mercator(&altered, to);
    }
}

#[test]
fn gdal_web_mercator_definition_may_be_wrapped_in_whitespace() {
    let indented = GDAL_WEB_MERCATOR.replace(',', ",\n    ");
    let parsed = crs::parse(&format!("  {indented}\n")).unwrap();
    assert_eq!(parsed.epsg(), Some(3857));
}

#[test]
fn geopackage_written_by_gdal_in_web_mercator_reads() {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/web_mercator.gpkg");
    let layers = read::read(&path, None, "points").unwrap();
    assert_eq!(layers.len(), 1);
    assert_eq!(layers[0].layer.len(), 2);
    assert_eq!(layers[0].srs.as_ref().and_then(|s| s.epsg()), Some(3857));
}

/// Write a shapefile, then replace its `.cpg` with the given bytes or remove it.
fn shapefile_with_cpg(cpg: Option<&[u8]>) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("places.shp");
    let layer = DatasetLayer::new(
        "places",
        Some(crs::parse("EPSG:4326").unwrap()),
        vec![Record {
            id: None,
            geometry: Some(Point::new(5., 52.).into()),
            properties: json!({"label": "café"}).as_object().unwrap().clone(),
        }],
    );
    write::dataset(&path, None, false, &[layer]).unwrap();
    match cpg {
        Some(bytes) => std::fs::write(path.with_extension("cpg"), bytes).unwrap(),
        None => std::fs::remove_file(path.with_extension("cpg")).unwrap(),
    }
    (dir, path)
}

#[test]
fn shapefile_cpg_without_a_usable_encoding_is_ignored() {
    let bom_utf8 = b"\xEF\xBB\xBFUTF-8".as_slice();
    for (name, cpg) in [
        ("missing", None),
        ("empty", Some(b"".as_slice())),
        ("newline", Some(b"\n".as_slice())),
        ("spaces", Some(b"  \r\n".as_slice())),
        ("ldid", Some(b"LDID/87".as_slice())),
        ("crlf", Some(b"UTF-8\r\n".as_slice())),
        ("bom", Some(bom_utf8)),
        ("second line", Some(b"UTF-8\nignored".as_slice())),
    ] {
        let (_dir, path) = shapefile_with_cpg(cpg);
        let read = read::read_with_attributes(&path, None, "places", true)
            .unwrap_or_else(|e| panic!("{name}: {e:#}"));
        assert_eq!(read[0].layer.len(), 1, "{name}");
        assert_eq!(read[0].attributes[0]["label"], "café", "{name}");
    }
}

#[test]
fn shapefile_cpg_code_page_names_select_the_encoding() {
    // The file holds UTF-8, so a single-byte encoding shows the bytes it decodes them as.
    for (cpg, expected) in [
        ("65001", "café"),
        ("UTF8", "café"),
        ("1252", "cafÃ©"),
        ("CP1252", "cafÃ©"),
        ("ISO-8859-1", "cafÃ©"),
        ("8859-1", "cafÃ©"),
        ("1251", "cafГ©"),
    ] {
        let (_dir, path) = shapefile_with_cpg(Some(cpg.as_bytes()));
        let read = read::read_with_attributes(&path, None, "places", true)
            .unwrap_or_else(|e| panic!("{cpg}: {e:#}"));
        assert_eq!(read[0].attributes[0]["label"], expected, "{cpg}");
    }
}

#[test]
fn shapefile_cpg_naming_an_undecodable_encoding_is_an_error() {
    for cpg in ["CP437", "+1252", "no such encoding"] {
        let (_dir, path) = shapefile_with_cpg(Some(cpg.as_bytes()));
        let error = read::read(&path, None, "places").unwrap_err();
        assert!(format!("{error:#}").contains("unsupported Shapefile CPG encoding"));
    }
}
