//! Offline, pinned Natural Earth examples imported through the regular workspace importer.
use crate::{
    model::*,
    workspace::{self, Workspace},
};
use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::{fs, path::PathBuf};
use topology_checker_core::Rule;
use uuid::Uuid;

pub fn catalog() -> Value {
    serde_json::from_str(include_str!("../examples/catalog.json")).expect("bundled example catalog")
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
    let catalog = catalog();
    let example = catalog
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == id)
        .context("unknown example")?;
    let mut ws = Workspace::new()?;
    for dataset in example["datasets"].as_array().unwrap() {
        let filename = dataset["filename"].as_str().unwrap();
        let id = Uuid::new_v4().to_string();
        let source = DatasetSource {
            path: PathBuf::from("sources").join(&id).join(filename),
            id,
            filename: filename.into(),
            provenance: Some(dataset["provenance"].clone()),
        };
        let path = ws.storage.path().join(&source.path);
        fs::create_dir_all(path.parent().unwrap())?;
        fs::write(&path, asset(filename)?)?;
        let layers = workspace::import(&source, ws.storage.path())?;
        let layer_ids = layers.iter().map(|l| l.id.clone()).collect::<Vec<_>>();
        workspace::append(&mut ws, source, layers)?;
        for id in layer_ids {
            let settings = ws.config.layers.iter_mut().find(|l| l.id == id).unwrap();
            settings.selected = true;
            settings.visible = dataset["visible"].as_bool().unwrap();
            for rule in dataset["checks"].as_array().unwrap() {
                ws.config.checks.push(Check {
                    rule: rule.as_str().unwrap().parse::<Rule>()?,
                    layer: id.clone(),
                    other: None,
                    enabled: true,
                });
            }
        }
    }
    ws.config.working_crs = Some("EPSG:4326".into());
    ws.config.tolerance = 1e-8;
    normalize_config(
        &mut ws.config,
        &Config {
            working_crs: Some("EPSG:4326".into()),
            ..Config::default()
        },
        &ws.layers,
    )?;
    ws.prepared = prepare(&ws.config, &ws.layers)?;
    Ok(ws)
}
