//! Files as GDAL-based applications write them.
use geo::Point;
use serde_json::json;
use topology_checker_io::{
    crs, read,
    write::{self, DatasetLayer, Record},
};

/// Definition of EPSG:3857 as GDAL writes it into GeoPackages and `.prj` files.
const GDAL_WEB_MERCATOR: &str = r#"PROJCS["WGS 84 / Pseudo-Mercator",GEOGCS["WGS 84",DATUM["WGS_1984",SPHEROID["WGS 84",6378137,298.257223563,AUTHORITY["EPSG","7030"]],AUTHORITY["EPSG","6326"]],PRIMEM["Greenwich",0,AUTHORITY["EPSG","8901"]],UNIT["degree",0.0174532925199433,AUTHORITY["EPSG","9122"]],AUTHORITY["EPSG","4326"]],PROJECTION["Mercator_1SP"],PARAMETER["central_meridian",0],PARAMETER["scale_factor",1],PARAMETER["false_easting",0],PARAMETER["false_northing",0],UNIT["metre",1,AUTHORITY["EPSG","9001"]],AXIS["Easting",EAST],AXIS["Northing",NORTH],EXTENSION["PROJ4","+proj=merc +a=6378137 +b=6378137 +lat_ts=0 +lon_0=0 +x_0=0 +y_0=0 +k=1 +units=m +nadgrids=@null +wktext +no_defs"],AUTHORITY["EPSG","3857"]]"#;
#[cfg(not(feature = "gdal-system"))]
const SPHERICAL_EXTENSION: &str = r#"EXTENSION["PROJ4","+proj=merc +a=6378137 +b=6378137 +lat_ts=0 +lon_0=0 +x_0=0 +y_0=0 +k=1 +units=m +nadgrids=@null +wktext +no_defs"],"#;

#[test]
fn gdal_web_mercator_definition_is_epsg_3857() {
    let parsed = crs::parse(GDAL_WEB_MERCATOR).unwrap();
    assert_eq!(parsed.epsg(), Some(3857));
    assert_eq!(parsed, crs::parse("EPSG:3857").unwrap());
}

// With the GDAL fallback, GDAL decides what an altered definition means (it honours the PROJ.4
// extension and the authority tag), so only the native parser is strict.
#[cfg(not(feature = "gdal-system"))]
#[test]
fn ellipsoidal_mercator_tagged_as_web_mercator_is_still_rejected() {
    assert!(GDAL_WEB_MERCATOR.contains(SPHERICAL_EXTENSION));
    let ellipsoidal = GDAL_WEB_MERCATOR.replace(SPHERICAL_EXTENSION, "");
    assert!(crs::parse(&ellipsoidal).is_err());
}

// With the GDAL fallback, GDAL decides what an altered definition means (it honours the PROJ.4
// extension and the authority tag), so only the native parser is strict.
#[cfg(not(feature = "gdal-system"))]
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
        assert!(crs::parse(&altered).is_err(), "{to}");
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

/// EPSG:3844 as GDAL writes it into GeoPackages (WKT1), FlatGeobuf (WKT2), GeoParquet (PROJJSON),
/// and Shapefile `.prj` sidecars (ESRI WKT1).
const GDAL_STEREO70_WKT1: &str = r#"PROJCS["Pulkovo 1942(58) / Stereo70",GEOGCS["Pulkovo 1942(58)",DATUM["Pulkovo_1942_58",SPHEROID["Krassowsky 1940",6378245,298.3,AUTHORITY["EPSG","7024"]],AUTHORITY["EPSG","6179"]],PRIMEM["Greenwich",0,AUTHORITY["EPSG","8901"]],UNIT["degree",0.0174532925199433,AUTHORITY["EPSG","9122"]],AUTHORITY["EPSG","4179"]],PROJECTION["Oblique_Stereographic"],PARAMETER["latitude_of_origin",46],PARAMETER["central_meridian",25],PARAMETER["scale_factor",0.99975],PARAMETER["false_easting",500000],PARAMETER["false_northing",500000],UNIT["metre",1,AUTHORITY["EPSG","9001"]],AXIS["Northing",NORTH],AXIS["Easting",EAST],AUTHORITY["EPSG","3844"]]"#;
const GDAL_STEREO70_WKT2: &str = r#"PROJCRS["Pulkovo 1942(58) / Stereo70",BASEGEOGCRS["Pulkovo 1942(58)",DATUM["Pulkovo 1942(58)",ELLIPSOID["Krassowsky 1940",6378245,298.3,LENGTHUNIT["metre",1]]],PRIMEM["Greenwich",0,ANGLEUNIT["degree",0.0174532925199433]],ID["EPSG",4179]],CONVERSION["Stereo 70",METHOD["Oblique Stereographic",ID["EPSG",9809]],PARAMETER["Latitude of natural origin",46,ANGLEUNIT["degree",0.0174532925199433],ID["EPSG",8801]],PARAMETER["Longitude of natural origin",25,ANGLEUNIT["degree",0.0174532925199433],ID["EPSG",8802]],PARAMETER["Scale factor at natural origin",0.99975,SCALEUNIT["unity",1],ID["EPSG",8805]],PARAMETER["False easting",500000,LENGTHUNIT["metre",1],ID["EPSG",8806]],PARAMETER["False northing",500000,LENGTHUNIT["metre",1],ID["EPSG",8807]]],CS[cartesian,2],AXIS["northing (X)",north,ORDER[1],LENGTHUNIT["metre",1]],AXIS["easting (Y)",east,ORDER[2],LENGTHUNIT["metre",1]],USAGE[SCOPE["Engineering survey, topographic mapping."],AREA["Romania - onshore and offshore."],BBOX[43.44,20.26,48.27,31.41]],ID["EPSG",3844]]"#;
const GDAL_STEREO70_PROJJSON: &str = r#"{"$schema":"https://proj.org/schemas/v0.7/projjson.schema.json","type":"ProjectedCRS","name":"Pulkovo 1942(58) / Stereo70","base_crs":{"name":"Pulkovo 1942(58)","datum":{"type":"GeodeticReferenceFrame","name":"Pulkovo 1942(58)","ellipsoid":{"name":"Krassowsky 1940","semi_major_axis":6378245,"inverse_flattening":298.3}},"coordinate_system":{"subtype":"ellipsoidal","axis":[{"name":"Geodetic latitude","abbreviation":"Lat","direction":"north","unit":"degree"},{"name":"Geodetic longitude","abbreviation":"Lon","direction":"east","unit":"degree"}]},"id":{"authority":"EPSG","code":4179}},"conversion":{"name":"Stereo 70","method":{"name":"Oblique Stereographic","id":{"authority":"EPSG","code":9809}},"parameters":[{"name":"Latitude of natural origin","value":46,"unit":"degree","id":{"authority":"EPSG","code":8801}},{"name":"Longitude of natural origin","value":25,"unit":"degree","id":{"authority":"EPSG","code":8802}},{"name":"Scale factor at natural origin","value":0.99975,"unit":"unity","id":{"authority":"EPSG","code":8805}},{"name":"False easting","value":500000,"unit":"metre","id":{"authority":"EPSG","code":8806}},{"name":"False northing","value":500000,"unit":"metre","id":{"authority":"EPSG","code":8807}}]},"coordinate_system":{"subtype":"Cartesian","axis":[{"name":"Northing","abbreviation":"X","direction":"north","unit":"metre"},{"name":"Easting","abbreviation":"Y","direction":"east","unit":"metre"}]},"id":{"authority":"EPSG","code":3844}}"#;
const GDAL_STEREO70_ESRI: &str = r#"PROJCS["Pulkovo_1942_Adj_58_Stereo_70",GEOGCS["GCS_Pulkovo_1942_Adj_1958",DATUM["D_Pulkovo_1942_Adj_1958",SPHEROID["Krasovsky_1940",6378245.0,298.3]],PRIMEM["Greenwich",0.0],UNIT["Degree",0.0174532925199433]],PROJECTION["Double_Stereographic"],PARAMETER["False_Easting",500000.0],PARAMETER["False_Northing",500000.0],PARAMETER["Central_Meridian",25.0],PARAMETER["Scale_Factor",0.99975],PARAMETER["Latitude_Of_Origin",46.0],UNIT["Meter",1.0]]"#;
const GDAL_STEREO70_NOCT: &str = r#"PROJCS["Pulkovo 1942(58) / Stereo70",GEOGCS["Pulkovo 1942(58)",DATUM["Pulkovo_1942_58",SPHEROID["Krassowsky 1940",6378245,298.3]],PRIMEM["Greenwich",0],UNIT["degree",0.0174532925199433]],PROJECTION["Oblique_Stereographic"],PARAMETER["latitude_of_origin",46],PARAMETER["central_meridian",25],PARAMETER["scale_factor",0.99975],PARAMETER["false_easting",500000],PARAMETER["false_northing",500000],UNIT["metre",1],AXIS["Northing",NORTH],AXIS["Easting",EAST]]"#;

#[test]
fn gdal_stereo70_definitions_parse_and_transform_as_epsg_3844() {
    let canonical = crs::parse("EPSG:3844").unwrap();
    let wgs84 = crs::parse("EPSG:4326").unwrap();
    let web_mercator = crs::parse("EPSG:3857").unwrap();
    for (label, definition) in [
        ("wkt1", GDAL_STEREO70_WKT1),
        ("wkt2", GDAL_STEREO70_WKT2),
        ("projjson", GDAL_STEREO70_PROJJSON),
        ("esri", GDAL_STEREO70_ESRI),
        ("noct", GDAL_STEREO70_NOCT),
    ] {
        let parsed = crs::parse(definition).unwrap_or_else(|e| panic!("{label}: {e:#}"));
        assert_eq!(parsed.epsg(), Some(3844), "{label}");
        assert_eq!(parsed, canonical, "{label}");
        assert_eq!(
            crs::parse(&parsed.to_wkt().unwrap()).unwrap(),
            canonical,
            "{label} to_wkt"
        );
        assert_eq!(
            crs::parse(&parsed.to_projjson().unwrap()).unwrap(),
            canonical,
            "{label} to_projjson"
        );
        let to_wgs84 = crs::Transform::new(&parsed, &wgs84).unwrap();
        let mut x = [500_000.];
        let mut y = [500_000.];
        to_wgs84.transform_coords(&mut x, &mut y, &mut []).unwrap();
        assert!((x[0] - 25.).abs() < 1e-2, "{label} lon {}", x[0]);
        assert!((y[0] - 46.).abs() < 1e-2, "{label} lat {}", y[0]);
        assert!(
            crs::Transform::new(&parsed, &web_mercator).is_ok(),
            "{label}"
        );
    }
}

#[cfg(not(feature = "gdal-system"))]
#[test]
fn altered_stereo70_definitions_are_rejected_or_stay_custom() {
    for (from, to) in [
        (
            r#"PARAMETER["False easting",500000"#,
            r#"PARAMETER["False easting",0"#,
        ),
        (
            r#"PARAMETER["Longitude of natural origin",25"#,
            r#"PARAMETER["Longitude of natural origin",20"#,
        ),
        (
            r#"PARAMETER["Scale factor at natural origin",0.99975"#,
            r#"PARAMETER["Scale factor at natural origin",1"#,
        ),
        (
            r#"ELLIPSOID["Krassowsky 1940",6378245,298.3"#,
            r#"ELLIPSOID["Krassowsky 1940",6378137,298.3"#,
        ),
    ] {
        assert!(GDAL_STEREO70_WKT2.contains(from), "{from}");
        let altered = GDAL_STEREO70_WKT2.replace(from, to);
        assert!(crs::parse(&altered).is_err(), "{to}");
    }
    for (from, to) in [
        (
            r#""name":"False easting","value":500000"#,
            r#""name":"False easting","value":0"#,
        ),
        (
            r#""semi_major_axis":6378245"#,
            r#""semi_major_axis":6378137"#,
        ),
    ] {
        assert!(GDAL_STEREO70_PROJJSON.contains(from), "{from}");
        let altered = GDAL_STEREO70_PROJJSON.replace(from, to);
        assert!(crs::parse(&altered).is_err(), "{to}");
    }
    let altered_esri = GDAL_STEREO70_ESRI.replace(
        r#"PARAMETER["False_Easting",500000.0]"#,
        r#"PARAMETER["False_Easting",0.0]"#,
    );
    let parsed_altered_esri = crs::parse(&altered_esri).unwrap();
    assert_eq!(parsed_altered_esri.epsg(), None);
    assert_ne!(parsed_altered_esri, crs::parse("EPSG:3844").unwrap());
}

#[test]
fn datasets_in_epsg_3844_read_across_all_formats() {
    let dir = tempfile::tempdir().unwrap();
    let canonical = crs::parse("EPSG:3844").unwrap();
    let layer = DatasetLayer::new(
        "places",
        Some(canonical.clone()),
        vec![Record {
            id: Some(1),
            geometry: Some(Point::new(500_000., 500_000.).into()),
            properties: json!({"name": "center"}).as_object().unwrap().clone(),
        }],
    );
    for ext in ["gpkg", "fgb", "parquet", "shp"] {
        let path = dir.path().join(format!("stereo70.{ext}"));
        write::dataset(&path, None, false, std::slice::from_ref(&layer)).unwrap();
        if ext == "shp" {
            std::fs::write(path.with_extension("prj"), GDAL_STEREO70_ESRI).unwrap();
        }
        let read = read::read(&path, None, "places").unwrap_or_else(|e| panic!("{ext}: {e:#}"));
        let srs = read[0].srs.as_ref().expect(ext);
        assert_eq!(srs.epsg(), Some(3844), "{ext}");
        assert_eq!(srs, &canonical, "{ext}");
        assert!(crs::Transform::new(srs, &crs::parse("EPSG:3857").unwrap()).is_ok());
    }
}
