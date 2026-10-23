use geo::polygon;
use topology_checker_io::{
    crs::{self, Transform},
    read,
    write::{self, DatasetLayer, Record},
};

// ============================================================================
// 1. AXIS PERMUTATIONS
// ============================================================================

#[test]
fn axis_permutations_projected_northing_first() {
    // Northing-first projected systems
    let cases = [
        ("EPSG:2180", "ETRF2000-PL / CS92", 500_000.0, 500_000.0),
        (
            "EPSG:2193",
            "NZGD2000 / New Zealand Transverse Mercator 2000",
            1_700_000.0,
            5_400_000.0,
        ),
        ("EPSG:3006", "SWEREF99 TM", 500_000.0, 6_500_000.0),
        (
            "EPSG:3035",
            "ETRS89-extended / LAEA Europe",
            4_321_000.0,
            3_210_000.0,
        ),
        (
            "EPSG:3844",
            "Pulkovo 1942(58) / Stereo70",
            500_000.0,
            500_000.0,
        ),
    ];

    let wgs84 = crs::parse("EPSG:4326").unwrap();

    for (code, expected_name_part, easting, northing) in cases {
        let crs = crs::parse(code).unwrap_or_else(|e| panic!("failed to parse {code}: {e:#}"));
        assert!(crs.is_projected(), "{code} should be projected");
        assert!(!crs.is_geographic(), "{code} should not be geographic");
        assert_eq!(
            crs.linear_units(),
            1.0,
            "{code} should have meter linear units"
        );
        assert!(
            crs.name()
                .as_deref()
                .unwrap_or("")
                .contains(expected_name_part),
            "{code} name {:?} should contain {expected_name_part}",
            crs.name()
        );

        // Forward to WGS84
        let to_wgs = Transform::new(&crs, &wgs84)
            .unwrap_or_else(|e| panic!("transform from {code} to WGS84: {e:#}"));
        let mut x = [easting];
        let mut y = [northing];
        to_wgs.transform_coords(&mut x, &mut y, &mut []).unwrap();
        assert!(x[0].is_finite() && y[0].is_finite());
        assert!(
            (-180.0..=180.0).contains(&x[0]),
            "lon out of bounds for {code}: {}",
            x[0]
        );
        assert!(
            (-90.0..=90.0).contains(&y[0]),
            "lat out of bounds for {code}: {}",
            y[0]
        );

        // Inverse back to projected
        let from_wgs = Transform::new(&wgs84, &crs)
            .unwrap_or_else(|e| panic!("transform from WGS84 to {code}: {e:#}"));
        let mut back_x = [x[0]];
        let mut back_y = [y[0]];
        from_wgs
            .transform_coords(&mut back_x, &mut back_y, &mut [])
            .unwrap();
        assert!(
            (back_x[0] - easting).abs() < 1e-2,
            "{code} roundtrip easting error: {} vs {}",
            back_x[0],
            easting
        );
        assert!(
            (back_y[0] - northing).abs() < 1e-2,
            "{code} roundtrip northing error: {} vs {}",
            back_y[0],
            northing
        );
    }
}

#[test]
fn axis_permutations_geographic_lat_first() {
    for code in ["EPSG:4326", "EPSG:4258"] {
        let crs = crs::parse(code).unwrap();
        assert!(crs.is_geographic(), "{code} should be geographic");
        assert!(!crs.is_projected(), "{code} should not be projected");
        assert_eq!(crs.angular_units_name().as_deref(), Some("degree"));

        let to_merc = Transform::new(&crs, &crs::parse("EPSG:3857").unwrap()).unwrap();
        let mut x = [15.0];
        let mut y = [50.0];
        to_merc.transform_coords(&mut x, &mut y, &mut []).unwrap();
        assert!(x[0] > 1_000_000.0 && y[0] > 1_000_000.0);
    }
}

#[test]
fn axis_permutations_polar_stereographic() {
    let cases = [
        ("EPSG:3413", -45.0, 75.0), // NSIDC Sea Ice North
        ("EPSG:3031", 0.0, -80.0),  // Antarctic Polar Stereo
    ];

    let wgs84 = crs::parse("EPSG:4326").unwrap();
    for (code, lon, lat) in cases {
        let polar = crs::parse(code).unwrap();
        assert!(polar.is_projected());

        let to_polar = Transform::new(&wgs84, &polar).unwrap();
        let mut x = [lon];
        let mut y = [lat];
        to_polar.transform_coords(&mut x, &mut y, &mut []).unwrap();
        assert!(x[0].is_finite() && y[0].is_finite());

        let to_wgs = Transform::new(&polar, &wgs84).unwrap();
        let mut rx = [x[0]];
        let mut ry = [y[0]];
        to_wgs.transform_coords(&mut rx, &mut ry, &mut []).unwrap();
        assert!(
            (rx[0] - lon).abs() < 1e-5,
            "{code} lon mismatch: {} vs {lon}",
            rx[0]
        );
        assert!(
            (ry[0] - lat).abs() < 1e-5,
            "{code} lat mismatch: {} vs {lat}",
            ry[0]
        );
    }
}

#[test]
fn axis_permutations_south_orientated_and_oblique() {
    // S-JTSK / Krovak East North (oblique)
    let sjtsk = crs::parse("EPSG:5514").unwrap();
    assert!(sjtsk.is_projected());
    assert_eq!(sjtsk.linear_units(), 1.0);

    let wgs84 = crs::parse("EPSG:4326").unwrap();
    let to_wgs = Transform::new(&sjtsk, &wgs84).unwrap();
    // Prague coordinates in S-JTSK (East, North coordinates: around -740000, -1040000)
    let mut x = [-740_000.0];
    let mut y = [-1_040_000.0];
    to_wgs.transform_coords(&mut x, &mut y, &mut []).unwrap();
    assert!((14.0..=15.0).contains(&x[0]), "Prague lon: {}", x[0]);
    assert!((50.0..=51.0).contains(&y[0]), "Prague lat: {}", y[0]);
}

// ============================================================================
// 2. 3D AND COMPOUND CRSS
// ============================================================================

#[test]
fn geographic_3d_and_geocentric_systems() {
    for code in ["EPSG:4979", "EPSG:4937"] {
        let crs = crs::parse(code).unwrap_or_else(|e| panic!("{code}: {e:#}"));
        assert!(crs.is_geographic(), "{code} should be geographic");
        assert_eq!(crs.angular_units_name().as_deref(), Some("degree"));
    }
}

#[test]
fn compound_horizontal_vertical_systems() {
    let cases = [
        ("EPSG:7415", 155_000.0, 463_000.0), // Amersfoort / RD New + NAP height
        ("EPSG:5498", 10.0, 52.0),           // ETRS89 + EVRF2007 height
    ];

    let wgs84 = crs::parse("EPSG:4326").unwrap();
    for (code, px, py) in cases {
        let crs = crs::parse(code).unwrap_or_else(|e| panic!("{code}: {e:#}"));
        assert!(crs.epsg() == Some(7415) || crs.epsg() == Some(5498));

        let t = Transform::new(&crs, &wgs84)
            .unwrap_or_else(|e| panic!("transform {code} to WGS84: {e:#}"));
        let mut x = [px];
        let mut y = [py];
        t.transform_coords(&mut x, &mut y, &mut []).unwrap();
        assert!(x[0].is_finite() && y[0].is_finite());
    }
}

#[test]
fn compound_syntax_string_normalization() {
    let cases = [
        ("EPSG:4326+3855", 4326, true, false),
        ("EPSG:28992+5709", 28992, false, true),
        ("EPSG:2180+5798", 2180, false, true),
    ];

    for (comp, expected_epsg, geo, proj) in cases {
        let crs = crs::parse(comp).unwrap_or_else(|e| panic!("parse {comp}: {e:#}"));
        assert_eq!(crs.epsg(), Some(expected_epsg), "{comp}");
        assert_eq!(crs.is_geographic(), geo, "{comp}");
        assert_eq!(crs.is_projected(), proj, "{comp}");
    }
}

// ============================================================================
// 3. NON-METER LINEAR UNITS
// ============================================================================

#[test]
fn non_meter_linear_units_accuracy() {
    let cases = [
        // US Survey Foot (1200 / 3937 = ~0.3048006096012192 m)
        (
            "EPSG:2264",
            0.3048006096012192,
            2_000_000.0,
            500_000.0,
            "EPSG:4326",
        ),
        (
            "EPSG:2229",
            0.3048006096012192,
            6_000_000.0,
            2_000_000.0,
            "EPSG:4326",
        ),
        // International Foot (0.3048 m)
        ("EPSG:2222", 0.3048, 700_000.0, 900_000.0, "EPSG:4326"),
        // Clarke's Link (0.2011661949036666 m) - base CRS EPSG:4157
        (
            "EPSG:2066",
            0.2011661949036666,
            196_190.0,
            176_053.0,
            "EPSG:4157",
        ),
        // Clarke's Foot (0.3047972651151 m) - base CRS EPSG:4302
        (
            "EPSG:2314",
            0.3047972651151,
            387_026.0,
            296_060.0,
            "EPSG:4302",
        ),
    ];

    for (code, expected_scale, test_x, test_y, target_geo) in cases {
        let crs = crs::parse(code).unwrap_or_else(|e| panic!("parse {code}: {e:#}"));
        let target =
            crs::parse(target_geo).unwrap_or_else(|e| panic!("parse target {target_geo}: {e:#}"));
        assert!(crs.is_projected(), "{code} should be projected");
        let factor = crs.linear_units();
        assert!(
            (factor - expected_scale).abs() < 1e-6,
            "{code} factor mismatch: {factor} vs {expected_scale}"
        );

        let t = Transform::new(&crs, &target)
            .unwrap_or_else(|e| panic!("transform {code} to {target_geo}: {e:#}"));
        let mut x = [test_x];
        let mut y = [test_y];
        t.transform_coords(&mut x, &mut y, &mut []).unwrap();
        assert!(
            x[0].is_finite() && y[0].is_finite(),
            "{code} returned non-finite"
        );
        assert!((-180.0..=180.0).contains(&x[0]), "{code} lon out of range");
        assert!((-90.0..=90.0).contains(&y[0]), "{code} lat out of range");

        let inv = Transform::new(&target, &crs)
            .unwrap_or_else(|e| panic!("inverse transform {target_geo} to {code}: {e:#}"));
        let mut rx = [x[0]];
        let mut ry = [y[0]];
        inv.transform_coords(&mut rx, &mut ry, &mut []).unwrap();
        assert!(
            (rx[0] - test_x).abs() < 1e-1,
            "{code} roundtrip x error: {} vs {test_x}",
            rx[0]
        );
        assert!(
            (ry[0] - test_y).abs() < 1e-1,
            "{code} roundtrip y error: {} vs {test_y}",
            ry[0]
        );
    }
}

// ============================================================================
// 4. PROJECTION METHOD FAMILIES
// ============================================================================

#[test]
fn projection_method_families_roundtrip() {
    let projections = [
        // 1. Web Mercator
        ("EPSG:3857", 0.0, 0.0, "EPSG:4326"),
        // 2. Transverse Mercator
        ("EPSG:32633", 15.0, 52.0, "EPSG:4326"),
        // 3. Mercator 1SP
        ("EPSG:3395", 0.0, 0.0, "EPSG:4326"),
        // 4. Mercator 2SP
        ("EPSG:3832", 150.0, -10.0, "EPSG:4326"),
        // 5. Lambert Conformal Conic 2SP
        ("EPSG:2154", 3.0, 46.5, "EPSG:4326"),
        // 6. Lambert Conformal Conic 1SP
        ("EPSG:3120", 30.0, -25.0, "EPSG:4326"),
        // 7. Lambert Conformal Conic Michigan
        ("EPSG:6201", -84.0, 43.0, "EPSG:4326"),
        // 8. Lambert Conformal Conic 1SP Variant B
        ("EPSG:31370", 4.3, 50.8, "EPSG:4326"),
        // 9. Albers Equal Area
        ("EPSG:3005", -125.0, 55.0, "EPSG:4326"),
        // 10. Polar Stereographic North
        ("EPSG:3413", -45.0, 75.0, "EPSG:4326"),
        // 11. Polar Stereographic South
        ("EPSG:3031", 0.0, -80.0, "EPSG:4326"),
        // 12. Polar Stereographic Variant C
        ("EPSG:3996", 0.0, 75.0, "EPSG:4326"),
        // 13. Equidistant Cylindrical / Plate Carrée
        ("EPSG:4087", 10.0, 20.0, "EPSG:4326"),
        // 14. Lambert Azimuthal Equal Area
        ("EPSG:3035", 10.0, 52.0, "EPSG:4326"),
        // 15. Oblique Stereographic / Double Stereographic
        ("EPSG:28992", 5.38, 52.15, "EPSG:4326"),
        ("EPSG:3844", 25.0, 46.0, "EPSG:4326"),
        // 16. Hotine Oblique Mercator
        ("EPSG:2056", 7.43, 46.95, "EPSG:4326"),
        ("EPSG:3078", -86.275, 45.005, "EPSG:4326"),
        // 17. Cassini-Soldner (DHDN / Soldner Berlin)
        ("EPSG:3068", 13.4, 52.5, "EPSG:4326"),
        // 18. Colombia Urban
        ("EPSG:6249", -74.0, 4.6, "EPSG:4326"),
        // 19. Krovak North Orientated
        ("EPSG:5514", 14.4, 50.1, "EPSG:4326"),
        // 20. Equal Earth
        ("EPSG:8857", 10.0, 20.0, "EPSG:4326"),
        // 21. American Polyconic (SIRGAS 2000 / Brazil Polyconic)
        ("EPSG:5880", -54.0, -10.0, "EPSG:4326"),
        // 22. Modified Azimuthal Equidistant (Guam 1963 / Yap Islands)
        ("EPSG:3295", 138.1, 9.5, "EPSG:4675"),
        // 23. Guam Projection (Guam 1963 / Guam SPCS)
        ("EPSG:3993", 144.75, 13.45, "EPSG:4675"),
        // 24. Laborde Oblique Mercator (Tananarive / Laborde Grid)
        ("EPSG:8441", 46.43, -18.9, "EPSG:4297"),
    ];

    for (code, lon, lat, geo_code) in projections {
        let crs = crs::parse(code).unwrap_or_else(|e| panic!("parse {code}: {e:#}"));
        let geo = crs::parse(geo_code).unwrap_or_else(|e| panic!("parse {geo_code}: {e:#}"));
        assert!(crs.is_projected(), "{code} should be projected");

        let to_proj = Transform::new(&geo, &crs)
            .unwrap_or_else(|e| panic!("forward transform {geo_code} to {code}: {e:#}"));
        let mut x = [lon];
        let mut y = [lat];
        to_proj.transform_coords(&mut x, &mut y, &mut []).unwrap();
        assert!(
            x[0].is_finite() && y[0].is_finite(),
            "{code} forward non-finite"
        );

        let to_geo = Transform::new(&crs, &geo)
            .unwrap_or_else(|e| panic!("inverse transform {code} to {geo_code}: {e:#}"));
        let mut rx = [x[0]];
        let mut ry = [y[0]];
        to_geo.transform_coords(&mut rx, &mut ry, &mut []).unwrap();
        assert!(
            (rx[0] - lon).abs() < 1e-4,
            "{code} lon roundtrip: {} vs {lon}",
            rx[0]
        );
        assert!(
            (ry[0] - lat).abs() < 1e-4,
            "{code} lat roundtrip: {} vs {lat}",
            ry[0]
        );
    }
}

// ============================================================================
// 5. LEGACY DATUMS AND REGIONAL FRAMES
// ============================================================================

#[test]
fn legacy_datums_and_regional_frames() {
    let datums = [
        ("EPSG:4267", "NAD27"),
        ("EPSG:4269", "NAD83"),
        ("EPSG:4258", "ETRS89"),
        ("EPSG:4277", "OSGB36"),
        ("EPSG:4230", "ED50"),
        ("EPSG:4301", "Tokyo"),
        ("EPSG:4284", "Pulkovo 1942"),
        ("EPSG:4179", "Pulkovo 1942(58)"),
        ("EPSG:4150", "CH1903+"),
        ("EPSG:4149", "CH1903"),
        ("EPSG:4156", "S-JTSK"),
        ("EPSG:4314", "DHDN"),
        ("EPSG:4171", "RGF93"),
        ("EPSG:7844", "GDA2020"),
        ("EPSG:4674", "SIRGAS 2000"),
        ("EPSG:6668", "JGD2011"),
        ("EPSG:4167", "NZGD2000"),
    ];

    for (code, name_part) in datums {
        let crs = crs::parse(code).unwrap_or_else(|e| panic!("parse {code}: {e:#}"));
        assert!(crs.is_geographic(), "{code} should be geographic");
        assert!(
            crs.name().as_deref().unwrap_or("").contains(name_part),
            "{code} name {:?} should contain {name_part}",
            crs.name()
        );
    }
}

// ============================================================================
// 6. WEB MERCATOR ALIASES
// ============================================================================

#[test]
fn web_mercator_aliases_are_identical_and_transform_consistently() {
    let base = crs::parse("EPSG:3857").unwrap();

    let aliases = [
        "EPSG:3857",
        "EPSG:3785",
        "EPSG:900913",
        "ESRI:102100",
        "ESRI:102113",
    ];

    for alias in aliases {
        let crs = crs::parse(alias).unwrap_or_else(|e| panic!("parse {alias}: {e:#}"));
        assert_eq!(crs, base, "{alias} should equal EPSG:3857");
        assert_eq!(crs.epsg(), Some(3857), "{alias} should resolve to 3857");
    }

    // ESRI WKT with Mercator_Auxiliary_Sphere
    let esri_wkt = r#"PROJCS["WGS_1984_Web_Mercator_Auxiliary_Sphere",GEOGCS["GCS_WGS_1984",DATUM["D_WGS_1984",SPHEROID["WGS 84",6378137.0,298.257223563]],PRIMEM["Greenwich",0.0],UNIT["Degree",0.0174532925199433]],PROJECTION["Mercator_Auxiliary_Sphere"],PARAMETER["False_Easting",0.0],PARAMETER["False_Northing",0.0],PARAMETER["Central_Meridian",0.0],PARAMETER["Standard_Parallel_1",0.0],PARAMETER["Auxiliary_Sphere_Type",0.0],UNIT["Meter",1.0]]"#;
    let esri_crs = crs::parse(esri_wkt).unwrap();
    assert_eq!(esri_crs, base);

    // Transforming between them is an identity
    let t = Transform::new(&esri_crs, &base).unwrap();
    let mut x = [1000.0];
    let mut y = [2000.0];
    t.transform_coords(&mut x, &mut y, &mut []).unwrap();
    assert!((x[0] - 1000.0).abs() < 1e-6);
    assert!((y[0] - 2000.0).abs() < 1e-6);
}

// ============================================================================
// 7. ESRI .PRJ MORPHING
// ============================================================================

#[test]
fn esri_prj_morphing_gauss_kruger_and_hotine_and_plate_carree() {
    // 1. DHDN 3-degree Gauss-Kruger zone 3
    let dhdn_gk = r#"PROJCS["DHDN_3_Degree_Gauss_Zone_3",GEOGCS["GCS_Deutsches_Hauptdreiecksnetz",DATUM["D_Deutsches_Hauptdreiecksnetz",SPHEROID["Bessel_1841",6377397.155,299.1528128]],PRIMEM["Greenwich",0.0],UNIT["Degree",0.0174532925199433]],PROJECTION["Gauss_Kruger"],PARAMETER["False_Easting",3500000.0],PARAMETER["False_Northing",0.0],PARAMETER["Central_Meridian",9.0],PARAMETER["Scale_Factor",1.0],PARAMETER["Latitude_Of_Origin",0.0],UNIT["Meter",1.0]]"#;
    let dhdn = crs::parse(dhdn_gk).unwrap();
    assert_eq!(dhdn.epsg(), Some(31467));
    assert_eq!(dhdn, crs::parse("EPSG:31467").unwrap());

    // 2. CH1903+ / LV95
    let ch1903_esri = r#"PROJCS["CH1903+_LV95",GEOGCS["GCS_CH1903+",DATUM["D_CH1903+",SPHEROID["Bessel_1841",6377397.155,299.1528128]],PRIMEM["Greenwich",0.0],UNIT["Degree",0.0174532925199433]],PROJECTION["Hotine_Oblique_Mercator_Azimuth_Center"],PARAMETER["False_Easting",2600000.0],PARAMETER["False_Northing",1200000.0],PARAMETER["Scale_Factor",1.0],PARAMETER["Azimuth",90.0],PARAMETER["Longitude_Of_Center",7.439583333333333],PARAMETER["Latitude_Of_Center",46.95240555555556],UNIT["Meter",1.0]]"#;
    let ch1903 = crs::parse(ch1903_esri).unwrap();
    assert_eq!(ch1903.epsg(), Some(2056));
    assert_eq!(ch1903, crs::parse("EPSG:2056").unwrap());

    // 3. Plate Carrée with accented e
    let plate_carree = r#"PROJCS["WGS_1984_Plate_Carree",GEOGCS["GCS_WGS_1984",DATUM["D_WGS_1984",SPHEROID["WGS 84",6378137.0,298.257223563]],PRIMEM["Greenwich",0.0],UNIT["Degree",0.0174532925199433]],PROJECTION["Plate_Carrée"],PARAMETER["False_Easting",0.0],PARAMETER["False_Northing",0.0],PARAMETER["Central_Meridian",0.0],UNIT["Meter",1.0]]"#;
    let pc = crs::parse(plate_carree).unwrap();
    assert!(pc.is_projected());

    // 4. WKT2 with BASEGEODCRS
    let wkt2_basegeod = r#"PROJCRS["WGS 84 / UTM zone 33N",BASEGEODCRS["WGS 84",DATUM["World Geodetic System 1984",ELLIPSOID["WGS 84",6378137,298.257223563]]],CONVERSION["UTM zone 33N",METHOD["Transverse Mercator"],PARAMETER["Latitude of natural origin",0],PARAMETER["Longitude of natural origin",15],PARAMETER["Scale factor at natural origin",0.9996],PARAMETER["False easting",500000],PARAMETER["False northing",0]],CS[Cartesian,2],AXIS["easting",east],AXIS["northing",north],LENGTHUNIT["metre",1],ID["EPSG",32633]]"#;
    let utm33 = crs::parse(wkt2_basegeod).unwrap();
    assert_eq!(utm33.epsg(), Some(32633));

    // 5. WKT2 with Lambert Conic Conformal (2SP)
    let wkt2_lcc = r#"PROJCRS["RGF93 v1 / Lambert-93",BASEGEOGCRS["RGF93 v1",DATUM["Reseau Geodesique Francais 1993 v1",ELLIPSOID["GRS 1980",6378137,298.257222101,LENGTHUNIT["metre",1]]],PRIMEM["Greenwich",0,ANGLEUNIT["degree",0.0174532925199433]],ID["EPSG",4171]],CONVERSION["Lambert-93",METHOD["Lambert Conic Conformal (2SP)",ID["EPSG",9802]],PARAMETER["Latitude of false origin",46.5,ANGLEUNIT["degree",0.0174532925199433],ID["EPSG",8821]],PARAMETER["Longitude of false origin",3,ANGLEUNIT["degree",0.0174532925199433],ID["EPSG",8822]],PARAMETER["Latitude of 1st standard parallel",49,ANGLEUNIT["degree",0.0174532925199433],ID["EPSG",8823]],PARAMETER["Latitude of 2nd standard parallel",44,ANGLEUNIT["degree",0.0174532925199433],ID["EPSG",8824]],PARAMETER["Easting at false origin",700000,LENGTHUNIT["metre",1],ID["EPSG",8826]],PARAMETER["Northing at false origin",6600000,LENGTHUNIT["metre",1],ID["EPSG",8827]]],CS[Cartesian,2],AXIS["easting (X)",east,ORDER[1],LENGTHUNIT["metre",1]],AXIS["northing (Y)",north,ORDER[2],LENGTHUNIT["metre",1]],ID["EPSG",2154]]"#;
    let rgf = crs::parse(wkt2_lcc).unwrap();
    assert_eq!(rgf.epsg(), Some(2154));
}

// ============================================================================
// 8. AUTHORITY SYNTAXES AND OGC URLS/URNS
// ============================================================================

#[test]
fn authority_syntaxes_and_ogc_urls_and_urns() {
    let cases = [
        ("http://www.opengis.net/def/crs/EPSG/0/4326", "EPSG:4326"),
        ("https://www.opengis.net/def/crs/EPSG/0/4326", "EPSG:4326"),
        ("http://www.opengis.net/def/crs/EPSG/7.9/3857", "EPSG:3857"),
        ("http://www.opengis.net/def/crs/OGC/1.3/CRS84", "OGC:CRS84"),
        ("https://www.opengis.net/def/crs/OGC/0/CRS84", "OGC:CRS84"),
        ("http://www.opengis.net/gml/srs/epsg.xml#4326", "EPSG:4326"),
        ("urn:ogc:def:crs:EPSG::4326", "EPSG:4326"),
        ("urn:ogc:def:crs:EPSG:7.9:4326", "EPSG:4326"),
        ("urn:x-ogc:def:crs:EPSG::4326", "EPSG:4326"),
        ("urn:ogc:def:crs:OGC:1.3:CRS84", "OGC:CRS84"),
        ("urn:ogc:def:crs:OGC::CRS84", "OGC:CRS84"),
        ("urn:ogc:def:crs:OGC:1.3:CRS27", "EPSG:4267"),
        ("urn:ogc:def:crs:OGC:1.3:CRS83", "EPSG:4269"),
        ("CRS84", "OGC:CRS84"),
        ("CRS:84", "OGC:CRS84"),
        ("OGC:CRS84", "OGC:CRS84"),
        ("OGC:CRS27", "EPSG:4267"),
        ("OGC:CRS83", "EPSG:4269"),
        ("WGS84", "EPSG:4326"),
        ("WGS 84", "EPSG:4326"),
        ("NAD83", "EPSG:4269"),
        ("NAD27", "EPSG:4267"),
        ("OSGB36", "EPSG:27700"),
    ];

    for (input, canonical) in cases {
        let parsed = crs::parse(input).unwrap_or_else(|e| panic!("parse {input}: {e:#}"));
        let expected =
            crs::parse(canonical).unwrap_or_else(|e| panic!("parse canonical {canonical}: {e:#}"));
        assert_eq!(parsed, expected, "Mismatch for input {input}");
    }
}

// ============================================================================
// 9. REAL WORLD QUIRKS: BOM AND WHITESPACE
// ============================================================================

#[test]
fn real_world_quirks_bom_and_whitespace() {
    let bom_inputs = [
        "\u{feff}EPSG:4326",
        "  \u{feff}  EPSG:3857  \r\n",
        "\u{feff}PROJCS[\"WGS 84 / UTM zone 33N\",GEOGCS[\"WGS 84\",DATUM[\"WGS_1984\",SPHEROID[\"WGS 84\",6378137,298.257223563]],PRIMEM[\"Greenwich\",0],UNIT[\"degree\",0.0174532925199433]],PROJECTION[\"Transverse_Mercator\"],PARAMETER[\"latitude_of_origin\",0],PARAMETER[\"central_meridian\",15],PARAMETER[\"scale_factor\",0.9996],PARAMETER[\"false_easting\",500000],PARAMETER[\"false_northing\",0],UNIT[\"metre\",1],AUTHORITY[\"EPSG\",\"32633\"]]",
    ];

    for input in bom_inputs {
        let crs = crs::parse(input).unwrap_or_else(|e| panic!("failed with BOM: {e:#}"));
        assert!(crs.is_geographic() || crs.is_projected());
    }
}

// ============================================================================
// 10. STRICT FAIL-CLOSED SEMANTICS
// ============================================================================

#[test]
fn strict_fail_closed_rejections() {
    let invalid_inputs = [
        // Empty / whitespace
        ("", "empty CRS definition"),
        ("   ", "empty CRS definition"),
        ("\u{feff}", "empty CRS definition"),
        // Syntactically invalid / unparsable by any engine
        ("NOT_A_VALID_CRS", "unrecognized"),
        ("EPSG:999999999", "unrecognized"),
        (
            r#"PROJCS["Unclosed_Bracket",GEOGCS["WGS 84""#,
            "unrecognized",
        ),
        (r#"{"type": "BrokenPROJJSON"}"#, "unrecognized"),
    ];

    for (input, reason) in invalid_inputs {
        let err = crs::parse(input).unwrap_err();
        let msg = format!("{err:#}");
        if reason == "empty CRS definition" {
            assert!(
                msg.contains(reason),
                "Expected '{reason}', got '{msg}' for input: {input}"
            );
        } else {
            assert!(!msg.is_empty(), "Expected error message for input: {input}");
        }
    }
}

// With the GDAL fallback, GDAL decides what an extended definition means,
// so only the native pure-Rust parser strictly fails closed on non-degree
// angular units or projections not in proj-core's built-in set.
#[cfg(not(feature = "gdal-system"))]
#[test]
fn native_parser_fail_closed_on_unsupported_extensions() {
    let invalid_inputs = [
        // Non-degree angular units (grads)
        (
            r#"GEOGCS["NTF (Paris)",DATUM["Nouvelle_Triangulation_Francaise_Paris",SPHEROID["Clarke 1880 (IGN)",6378249.2,293.4660212936269]],PRIMEM["Paris",2.33722917],UNIT["grad",0.01570796326794897]]"#,
            "angular units",
        ),
        (
            r#"GEODCRS["Custom grad",DATUM["World Geodetic System 1984",ELLIPSOID["WGS 84",6378137,298.257223563]],CS[ellipsoidal,2],AXIS["longitude",east,ORDER[1],ANGLEUNIT["grad",0.015707963267949]],AXIS["latitude",north,ORDER[2],ANGLEUNIT["grad",0.015707963267949]]]"#,
            "angular units",
        ),
        (
            r#"PROJCS["NTF_Paris_Lambert_Zone_II",GEOGCS["GCS_NTF_Paris",DATUM["D_NTF",SPHEROID["Clarke_1880_IGN",6378249.2,293.4660212936269]],PRIMEM["Paris",2.337229166666667],UNIT["Grad",0.01570796326794897]],PROJECTION["Lambert_Conformal_Conic"],PARAMETER["False_Easting",600000.0],PARAMETER["False_Northing",2200000.0],PARAMETER["Central_Meridian",0.0],PARAMETER["Standard_Parallel_1",52.0],PARAMETER["Scale_Factor",0.99987742],PARAMETER["Latitude_Of_Origin",52.0],UNIT["Meter",1.0]]"#,
            "angular units",
        ),
        // Unsupported projections
        (
            r#"PROJCS["NZMG",GEOGCS["NZGD49",DATUM["New_Zealand_Geodetic_Datum_1949",SPHEROID["International 1924",6378388,297]],PRIMEM["Greenwich",0],UNIT["degree",0.0174532925199433]],PROJECTION["New_Zealand_Map_Grid"],UNIT["metre",1]]"#,
            "unsupported",
        ),
        (
            r#"PROJCS["World_Robinson",GEOGCS["WGS 84",DATUM["WGS_1984",SPHEROID["WGS 84",6378137,298.257223563]],PRIMEM["Greenwich",0],UNIT["degree",0.0174532925199433]],PROJECTION["Robinson"],UNIT["metre",1]]"#,
            "unsupported",
        ),
        (
            r#"PROJCS["World_Mollweide",GEOGCS["WGS 84",DATUM["WGS_1984",SPHEROID["WGS 84",6378137,298.257223563]],PRIMEM["Greenwich",0],UNIT["degree",0.0174532925199433]],PROJECTION["Mollweide"],UNIT["metre",1]]"#,
            "unsupported",
        ),
        (
            r#"PROJCS["Bonne",GEOGCS["WGS 84",DATUM["WGS_1984",SPHEROID["WGS 84",6378137,298.257223563]],PRIMEM["Greenwich",0],UNIT["degree",0.0174532925199433]],PROJECTION["Bonne"],PARAMETER["central_meridian",0],PARAMETER["standard_parallel_1",60],UNIT["metre",1]]"#,
            "unsupported",
        ),
        // Non-geodetic systems
        (
            r#"LOCAL_CS["Cartesian Engineering Grid",LOCAL_DATUM["Unknown",0],UNIT["metre",1],AXIS["X",EAST],AXIS["Y",NORTH]]"#,
            "unrecognized",
        ),
        (
            r#"ENGCRS["Site Grid",EDATUM["Site Datum"],CS[Cartesian,2],AXIS["x",east],AXIS["y",north],LENGTHUNIT["metre",1]]"#,
            "unrecognized",
        ),
    ];

    for (input, reason) in invalid_inputs {
        let err = crs::parse(input).unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.to_ascii_lowercase()
                .contains(&reason.to_ascii_lowercase()),
            "Expected error containing '{reason}', got '{msg}' for input: {input}"
        );
    }
}

#[cfg(feature = "gdal-system")]
#[test]
fn gdal_fallback_supports_extended_projections_and_units() {
    let extended_inputs = [
        r#"GEOGCS["NTF (Paris)",DATUM["Nouvelle_Triangulation_Francaise_Paris",SPHEROID["Clarke 1880 (IGN)",6378249.2,293.4660212936269]],PRIMEM["Paris",2.33722917],UNIT["grad",0.01570796326794897]]"#,
        r#"PROJCS["World_Robinson",GEOGCS["WGS 84",DATUM["WGS_1984",SPHEROID["WGS 84",6378137,298.257223563]],PRIMEM["Greenwich",0],UNIT["degree",0.0174532925199433]],PROJECTION["Robinson"],UNIT["metre",1]]"#,
        r#"PROJCS["World_Mollweide",GEOGCS["WGS 84",DATUM["WGS_1984",SPHEROID["WGS 84",6378137,298.257223563]],PRIMEM["Greenwich",0],UNIT["degree",0.0174532925199433]],PROJECTION["Mollweide"],UNIT["metre",1]]"#,
    ];

    for input in extended_inputs {
        let crs = crs::parse(input).unwrap();
        assert!(crs.is_geographic() || crs.is_projected());
    }
}

// ============================================================================
// 11. MULTI-FORMAT DATASET ROUNDTRIPS
// ============================================================================

#[test]
fn multi_format_dataset_roundtrips_across_tricky_crss() {
    let dir = tempfile::tempdir().unwrap();

    let test_crss = [
        ("EPSG:2180", 500_000.0, 500_000.0), // Poland CS92 northing-first
        ("EPSG:2264", 2_000_000.0, 500_000.0), // US survey foot
        ("EPSG:7415", 155_000.0, 463_000.0), // Amersfoort compound
        ("OGC:CRS84", 10.0, 50.0),           // OGC CRS84 lon/lat
        ("EPSG:3035", 4_321_000.0, 3_210_000.0), // LAEA Europe
    ];

    for (crs_str, px, py) in test_crss {
        let crs = crs::parse(crs_str).unwrap();
        let layer = DatasetLayer::new(
            "tricky",
            Some(crs.clone()),
            vec![
                Record {
                    id: Some(1),
                    geometry: Some(
                        polygon![
                            (x: px, y: py),
                            (x: px + 10.0, y: py),
                            (x: px + 10.0, y: py + 10.0),
                            (x: px, y: py + 10.0),
                            (x: px, y: py)
                        ]
                        .into(),
                    ),
                    properties: serde_json::json!({"name": "poly1"})
                        .as_object()
                        .unwrap()
                        .clone(),
                },
                Record {
                    id: Some(2),
                    geometry: Some(
                        polygon![
                            (x: px + 50.0, y: py + 50.0),
                            (x: px + 100.0, y: py + 50.0),
                            (x: px + 100.0, y: py + 100.0),
                            (x: px + 50.0, y: py + 100.0),
                            (x: px + 50.0, y: py + 50.0)
                        ]
                        .into(),
                    ),
                    properties: serde_json::json!({"name": "poly2"})
                        .as_object()
                        .unwrap()
                        .clone(),
                },
            ],
        );

        for ext in ["gpkg", "fgb", "parquet", "geojson", "shp"] {
            let filename = format!("test_{}_{ext}.{ext}", crs_str.replace([':', '+'], "_"));
            let path = dir.path().join(filename);

            write::dataset(&path, None, false, std::slice::from_ref(&layer))
                .unwrap_or_else(|e| panic!("write to {ext} ({crs_str}): {e:#}"));

            let read_back = read::read(&path, None, "tricky")
                .unwrap_or_else(|e| panic!("read from {ext} ({crs_str}): {e:#}"));

            assert_eq!(read_back[0].layer.len(), 2, "{ext} count");

            // Verify CRS read back
            let read_crs = read_back[0].srs.as_ref().expect(ext);
            assert_eq!(read_crs, &crs, "{ext} CRS mismatch for {crs_str}");

            // Verify coordinates transformed to WGS 84
            let to_wgs = Transform::new(read_crs, &crs::parse("EPSG:4326").unwrap())
                .unwrap_or_else(|e| panic!("Transform read back {ext} to WGS84: {e:#}"));
            let mut x = [px];
            let mut y = [py];
            to_wgs.transform_coords(&mut x, &mut y, &mut []).unwrap();
            assert!(x[0].is_finite() && y[0].is_finite());
        }
    }
}
