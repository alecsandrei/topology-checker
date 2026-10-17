use std::path::Path;
use topology_checker_core::geo::{Geometry, Point};
use topology_checker_io::{
    crs, read,
    write::{self, DatasetLayer, Record},
};

const CUSTOM: &str = r#"PROJCS["custom foot",GEOGCS["GCS",DATUM["D",SPHEROID["WGS 84",6378137,298.257223563]],PRIMEM["Greenwich",0],UNIT["degree",0.0174532925199433]],PROJECTION["Transverse_Mercator"],PARAMETER["latitude_of_origin",0],PARAMETER["central_meridian",9],PARAMETER["scale_factor",0.9996],PARAMETER["false_easting",500000],PARAMETER["false_northing",0],UNIT["foot",0.3048]]"#;

fn layer(name: &str, srs: Option<&str>) -> DatasetLayer {
    DatasetLayer::new(
        name,
        srs.map(|s| crs::parse(s).unwrap()),
        vec![Record {
            id: Some(1),
            geometry: Some(Geometry::from(Point::new(5., 52.))),
            properties: Default::default(),
        }],
    )
}
fn written(dir: &Path, layers: &[DatasetLayer]) -> std::path::PathBuf {
    let path = dir.join("data.gpkg");
    write::dataset(&path, None, false, layers).unwrap();
    path
}

#[test]
fn custom_and_missing_coordinate_systems_roundtrip_per_layer() {
    let dir = tempfile::tempdir().unwrap();
    let path = written(
        dir.path(),
        &[
            layer("a", Some(CUSTOM)),
            layer("b", Some(CUSTOM)),
            layer("c", Some("+proj=utm +zone=33 +units=us-ft")),
            layer("d", None),
        ],
    );
    let layers = read::read(&path, None, "x").unwrap();
    let srs = layers.iter().map(|l| l.srs.clone()).collect::<Vec<_>>();
    assert_eq!(srs[0], srs[1]);
    assert_ne!(srs[0], srs[2]);
    assert!(srs[0].is_some() && srs[2].is_some());
    assert_eq!(srs[3], None);
    assert_eq!(srs[0].as_ref().unwrap().epsg(), None);
}

#[cfg(feature = "gdal-system")]
mod tampered {
    use super::*;
    use gdal::{Dataset, DatasetOptions, GdalOpenFlags, vector::sql::Dialect};

    fn edit(path: &Path, statements: &[&str]) {
        let dataset = Dataset::open_ex(
            path,
            DatasetOptions {
                open_flags: GdalOpenFlags::GDAL_OF_UPDATE | GdalOpenFlags::GDAL_OF_VECTOR,
                ..Default::default()
            },
        )
        .unwrap();
        for statement in statements {
            dataset
                .execute_sql(statement, None, Dialect::SQLITE)
                .unwrap();
        }
    }

    #[test]
    fn epsg_authority_stands_in_for_an_undefined_definition() {
        let dir = tempfile::tempdir().unwrap();
        let path = written(dir.path(), &[layer("a", Some("EPSG:4326"))]);
        edit(
            &path,
            &["UPDATE gpkg_spatial_ref_sys SET definition = 'undefined' WHERE srs_id = 4326"],
        );
        let read = read::read(&path, None, "x").unwrap();
        assert_eq!(read[0].srs.as_ref().unwrap().epsg(), Some(4326));

        edit(
            &path,
            &["UPDATE gpkg_spatial_ref_sys SET organization_coordsys_id = 'x' WHERE srs_id = 4326"],
        );
        let error = read::read(&path, None, "x").unwrap_err();
        assert!(
            format!("{error:#}").contains("expected integer"),
            "{error:#}"
        );

        edit(
            &path,
            &["UPDATE gpkg_spatial_ref_sys SET organization = 'NONE' WHERE srs_id = 4326"],
        );
        let error = read::read(&path, None, "x").unwrap_err();
        assert!(
            format!("{error:#}").contains("unsupported GeoPackage CRS"),
            "{error:#}"
        );
    }

    #[test]
    fn geometry_columns_must_hold_blobs() {
        let dir = tempfile::tempdir().unwrap();
        let path = written(dir.path(), &[layer("a", None)]);
        edit(&path, &["UPDATE a SET geom = 'text'"]);
        let error = read::read(&path, None, "x").unwrap_err();
        assert!(format!("{error:#}").contains("blob or NULL"), "{error:#}");
    }

    #[test]
    fn maximum_page_size_databases_are_readable() {
        let dir = tempfile::tempdir().unwrap();
        let path = written(dir.path(), &[layer("a", None)]);
        edit(&path, &["PRAGMA page_size = 65536", "VACUUM"]);
        assert_eq!(read::read(&path, None, "x").unwrap()[0].layer.len(), 1);
    }
}

#[test]
fn layers_sharing_a_custom_crs_have_a_common_crs() {
    let dir = tempfile::tempdir().unwrap();
    let path = written(
        dir.path(),
        &[layer("a", Some(CUSTOM)), layer("b", Some(CUSTOM))],
    );
    let layers = read::read(&path, None, "x").unwrap();
    assert_eq!(layers.len(), 2);
    let common = read::common_crs(&layers).unwrap();
    assert_eq!(common, layers[0].srs);
}
