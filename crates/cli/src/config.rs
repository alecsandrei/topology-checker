use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
use topology_checker_core::Rule;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub tolerance: Option<f64>,
    pub layers: BTreeMap<String, Input>,
    pub rules: Vec<Check>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub path: PathBuf,
    pub layer: Option<String>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Check {
    pub rule: Rule,
    pub layer: String,
    pub other: Option<String>,
}
impl Config {
    pub fn read(path: &Path) -> Result<Self> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let mut config: Self = toml::from_str(&text).context("invalid topology TOML")?;
        if config.rules.is_empty() {
            bail!("topology definition contains no rules");
        }
        for check in &config.rules {
            if !config.layers.contains_key(&check.layer) {
                bail!("unknown layer alias {}", check.layer);
            }
            if let Some(other) = &check.other
                && !config.layers.contains_key(other)
            {
                bail!("unknown layer alias {other}");
            }
            if check.other.is_some() != check.rule.inputs().1.is_some() {
                bail!("{} has the wrong number of inputs", check.rule);
            }
        }
        let parent = path.parent().unwrap_or(Path::new("."));
        for input in config.layers.values_mut() {
            input.path = parent.join(&input.path);
        }
        Ok(config)
    }
}
