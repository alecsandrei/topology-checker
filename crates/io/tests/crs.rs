use std::rc::Rc;
use topology_checker_io::crs;

const FOOT_UTM: &str = r#"PROJCS["custom foot",GEOGCS["GCS",DATUM["D",SPHEROID["WGS 84",6378137,298.257223563]],PRIMEM["Greenwich",0],UNIT["degree",0.0174532925199433]],PROJECTION["Transverse_Mercator"],PARAMETER["latitude_of_origin",0],PARAMETER["central_meridian",9],PARAMETER["scale_factor",0.9996],PARAMETER["false_easting",500000],PARAMETER["false_northing",0],UNIT["foot",0.3048]]"#;

#[test]
fn transforms_are_cached_per_thread_and_the_cache_is_bounded() {
    let first = crs::transform_between("EPSG:4326", "EPSG:3857").unwrap();
    let again = crs::transform_between("EPSG:4326", "EPSG:3857").unwrap();
    assert!(Rc::ptr_eq(&first, &again));
    assert!(crs::transform_between("EPSG:4326", "not a crs").is_err());
    for zone in 1..=40 {
        crs::transform_between("EPSG:4326", &format!("EPSG:326{zone:02}")).unwrap();
    }
    // Filling past the bound discarded the earlier entries.
    let rebuilt = crs::transform_between("EPSG:4326", "EPSG:3857").unwrap();
    assert!(!Rc::ptr_eq(&first, &rebuilt));
}

#[test]
fn geographic_and_projected_crs_describe_themselves() {
    let geographic = crs::parse("EPSG:4326").unwrap();
    assert!(geographic.is_geographic() && !geographic.is_projected());
    assert_eq!(geographic.epsg(), Some(4326));
    assert_eq!(geographic.authority().unwrap(), "EPSG:4326");
    assert_eq!(geographic.angular_units_name().as_deref(), Some("degree"));
    assert_eq!(geographic.linear_units_name().as_deref(), Some("metre"));
    assert_eq!(geographic.name().as_deref(), Some("WGS 84"));
    assert!(geographic.to_wkt().unwrap().contains("WGS 84"));
    assert!(geographic.to_projjson().unwrap().contains("\"type\""));

    let utm = crs::parse("EPSG:32633").unwrap();
    assert!(utm.is_projected() && !utm.is_geographic());
    assert_eq!(utm.angular_units_name(), None);
    assert_eq!(utm.linear_units_name().as_deref(), Some("metre"));
    assert!(utm != geographic);

    assert_eq!(
        crs::parse("urn:ogc:def:crs:OGC:1.3:CRS84").unwrap(),
        crs::parse("OGC:CRS84").unwrap()
    );
}

#[test]
fn custom_units_are_named_and_custom_crs_have_no_authority() {
    let foot = crs::parse(FOOT_UTM).unwrap();
    assert_eq!(foot.epsg(), None);
    assert_eq!(foot.linear_units_name().as_deref(), Some("foot"));
    assert!(
        foot.authority()
            .unwrap_err()
            .to_string()
            .contains("authority")
    );

    let survey = crs::parse("+proj=utm +zone=33 +units=us-ft").unwrap();
    let name = survey.linear_units_name().unwrap();
    assert!(name.starts_with("unit (0.3048006"), "{name}");
}

#[test]
fn coordinate_transformations_reject_invalid_arrays() {
    let operation = crs::Transform::new(
        &crs::parse("EPSG:4326").unwrap(),
        &crs::parse("EPSG:3857").unwrap(),
    )
    .unwrap();
    assert!(
        operation
            .transform_coords(&mut [1.], &mut [1., 2.], &mut [])
            .is_err()
    );
    assert!(
        operation
            .transform_coords(&mut [f64::NAN], &mut [1.], &mut [])
            .is_err()
    );
    // Web Mercator is undefined at the poles.
    assert!(
        operation
            .transform_coords(&mut [0.], &mut [90.], &mut [])
            .is_err()
    );
    let bounds = operation.transform_bounds(&[0., 0., 1., 1.], 21).unwrap();
    assert!(bounds[2] > bounds[0] && bounds[3] > bounds[1]);
}

#[test]
fn identical_custom_crs_without_a_datum_shift_are_equal() {
    for definition in [FOOT_UTM, "+proj=utm +zone=33 +units=us-ft"] {
        let a = crs::parse(definition).unwrap();
        let b = crs::parse(definition).unwrap();
        assert_eq!(a, b, "{definition}");
    }
    assert_ne!(
        crs::parse(FOOT_UTM).unwrap(),
        crs::parse("+proj=utm +zone=33 +units=us-ft").unwrap()
    );
}

#[test]
fn custom_crs_equality_still_distinguishes_every_parameter() {
    let parse = |d: &str| crs::parse(d).unwrap();
    let equal = |a: &str, b: &str| parse(a) == parse(b);
    // Identical definitions without a datum shift, projected and geographic.
    assert!(equal(
        "+proj=utm +zone=33 +ellps=GRS80",
        "+proj=utm +zone=33 +ellps=GRS80"
    ));
    assert!(equal(
        "+proj=longlat +ellps=intl",
        "+proj=longlat +ellps=intl"
    ));
    // Any differing parameter keeps them apart.
    assert!(!equal(
        "+proj=utm +zone=33 +ellps=GRS80",
        "+proj=utm +zone=34 +ellps=GRS80"
    ));
    assert!(!equal(
        "+proj=utm +zone=33 +ellps=GRS80",
        "+proj=utm +zone=33 +ellps=intl"
    ));
    assert!(!equal(
        "+proj=utm +zone=33 +ellps=GRS80 +units=m",
        "+proj=utm +zone=33 +ellps=GRS80 +units=ft"
    ));
    assert!(!equal(
        "+proj=longlat +ellps=intl",
        "+proj=longlat +ellps=bessel"
    ));
    // Datum shifts are real differences: only identical shifts are equal.
    let shifted = |t: &str| format!("+proj=utm +zone=33 +ellps=intl +towgs84={t}");
    assert!(equal(&shifted("1,2,3"), &shifted("1,2,3")));
    assert!(!equal(&shifted("1,2,3"), &shifted("1,2,4")));
    // An explicit zero shift is not the same statement as an unknown one.
    assert!(!equal("+proj=utm +zone=33 +ellps=intl", &shifted("0,0,0")));
    // A custom CRS never equals a coded one.
    assert!(!equal("+proj=utm +zone=33 +ellps=GRS80", "EPSG:32633"));
    assert!(!equal("EPSG:32633", "+proj=utm +zone=33 +ellps=GRS80"));
}
