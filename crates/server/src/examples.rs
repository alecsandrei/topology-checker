//! Offline, pinned Natural Earth examples imported through the regular workspace importer.
use crate::{
    dto::Example,
    model::*,
    workspace::{self, Workspace},
};
use anyhow::{Context, Result, bail};
use std::{fs, path::PathBuf, sync::OnceLock};
use uuid::Uuid;

/// Return the bundled example catalog, parsed once.
pub fn catalog() -> &'static [Example] {
    static CATALOG: OnceLock<Vec<Example>> = OnceLock::new();
    CATALOG.get_or_init(|| {
        serde_json::from_str(include_str!("../examples/catalog.json"))
            .expect("bundled example catalog")
    })
}
fn asset(name: &str) -> Result<&'static [u8]> {
    Ok(match name {
        "ne_110m_populated_places.geojson" => {
            include_bytes!("../examples/sources/ne_110m_populated_places.geojson")
        }
        "ne_110m_rivers_lake_centerlines.geojson" => {
            include_bytes!("../examples/sources/ne_110m_rivers_lake_centerlines.geojson")
        }
        "ne_50m_rivers_lake_centerlines.geojson" => {
            include_bytes!("../examples/sources/ne_50m_rivers_lake_centerlines.geojson")
        }
        "ne_110m_admin_0_countries.geojson" => {
            include_bytes!("../examples/sources/ne_110m_admin_0_countries.geojson")
        }
        "ne_110m_lakes.geojson" => include_bytes!("../examples/sources/ne_110m_lakes.geojson"),
        "cities_by_country.geojson" => {
            include_bytes!("../examples/generated/cities_by_country.geojson")
        }
        "country_city_atlas.geojson" => {
            include_bytes!("../examples/generated/country_city_atlas.geojson")
        }
        _ => bail!("unknown bundled dataset"),
    })
}
/// Create an example with fresh source and layer identities, without running checks.
///
/// # Errors
/// Returns an error for unknown examples, filesystem failures or importer failures.
pub fn load(id: &str) -> Result<Workspace> {
    let example = catalog()
        .iter()
        .find(|e| e.id == id)
        .context("unknown example")?;
    let mut ws = Workspace::new()?;
    let mut imported = Vec::new();
    for dataset in &example.datasets {
        let id = Uuid::new_v4().to_string();
        let source = DatasetSource {
            path: PathBuf::from("sources").join(&id).join(&dataset.filename),
            id,
            filename: dataset.filename.clone(),
            provenance: Some(dataset.provenance.clone()),
        };
        let path = ws.root().join(&source.path);
        fs::create_dir_all(path.parent().context("source directory")?)?;
        fs::write(&path, asset(&dataset.filename)?)?;
        let layers = workspace::import(&source, ws.root())?;
        imported.extend(layers.iter().map(|l| (l.id.clone(), dataset)));
        workspace::append(&mut ws, source, layers)?;
    }
    let mut config = ws.config().clone();
    for (id, dataset) in imported {
        let settings = config
            .layers
            .iter_mut()
            .find(|l| l.id == id)
            .context("imported layer settings")?;
        settings.selected = true;
        settings.visible = dataset.visible;
        config
            .checks
            .extend(dataset.checks.iter().map(|&rule| Check {
                rule,
                layer: id.clone(),
                other: None,
                enabled: true,
            }));
    }
    config.working_crs = Some("EPSG:4326".into());
    config.tolerance = 1e-8;
    normalize_config(
        &mut config,
        &Config {
            working_crs: Some("EPSG:4326".into()),
            ..Config::default()
        },
        ws.layers(),
    )?;
    ws.initialize(config)?;
    Ok(ws)
}

#[cfg(test)]
mod tests {
    #[test]
    fn bundled_catalog_parses_and_every_dataset_has_an_asset() {
        let catalog = super::catalog();
        assert!(!catalog.is_empty());
        for dataset in catalog.iter().flat_map(|e| &e.datasets) {
            assert!(
                super::asset(&dataset.filename).is_ok(),
                "{}",
                dataset.filename
            );
        }
    }
}
