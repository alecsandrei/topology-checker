//! The `plugin.json` manifest, generated from the rule registry so every rule has a tool.

use rustgis_plugin::{Value, json};
use topology_checker_core::{GeometryKind, Rule};

/// Tolerance used when the form is left alone: about 11 cm, since RustGIS hands tools lon/lat degrees.
pub(crate) const DEFAULT_TOLERANCE: f64 = 1e-6;

/// The tool id for a rule: its canonical name without the family slash.
#[must_use]
pub fn tool_id(rule: Rule) -> String {
    rule.name().replace('/', "-")
}

/// The rule a tool id names.
#[must_use]
pub fn rule_of(tool: &str) -> Option<Rule> {
    Rule::ALL.iter().copied().find(|r| tool_id(*r) == tool)
}

fn family(rule: Rule) -> &'static str {
    rule.name().split('/').next().unwrap_or_default()
}

fn title(words: &str) -> String {
    words
        .split('-')
        .map(|w| {
            let mut c = w.chars();
            c.next()
                .map(|f| f.to_uppercase().chain(c).collect::<String>())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn tool_name(rule: Rule) -> String {
    let (fam, rest) = rule.name().split_once('/').unwrap_or_default();
    format!("{}: {}", title(fam), title(rest))
}

fn layer_kind(kind: GeometryKind) -> &'static str {
    match kind {
        GeometryKind::Point => "pointLayer",
        GeometryKind::Polygon => "polygonLayer",
        _ => "vectorLayer",
    }
}

fn tool(rule: Rule) -> Value {
    let (first, second) = rule.inputs();
    let mut params = vec![
        json!({"name": "layer", "label": "Layer", "kind": layer_kind(first), "required": true}),
    ];
    if let Some(second) = second {
        params.push(json!({"name": "other", "label": "Second layer", "kind": layer_kind(second), "required": true}));
    }
    params.push(json!({
        "name": "tolerance",
        "label": "Tolerance (degrees)",
        "kind": "number",
        "required": true,
        "default": DEFAULT_TOLERANCE,
        "help": "Planar distance tolerance in the layers' coordinate units. RustGIS gives tools longitude/latitude, so this is in degrees (0.00001 is about 1 m at the equator).",
    }));
    json!({"id": tool_id(rule), "name": tool_name(rule), "summary": rule.summary(), "params": params})
}

fn tab() -> Value {
    let groups = [
        ("point", "Points"),
        ("line", "Lines"),
        ("polygon", "Polygons"),
        ("geometry", "Geometry"),
    ];
    let groups: Vec<Value> = groups
        .iter()
        .map(|(fam, label)| {
            let in_family = |two_layers: bool| -> Vec<String> {
                Rule::ALL
                    .iter()
                    .filter(|r| family(**r) == *fam && r.inputs().1.is_some() == two_layers)
                    .map(|r| format!("tool:{}", tool_id(*r)))
                    .collect()
            };
            let mut items = Vec::new();
            for (menu, help, two) in [("One layer", "Rules that check a single layer.", false), ("Two layers", "Rules that compare two layers.", true)] {
                let ids = in_family(two);
                if !ids.is_empty() {
                    items.push(json!({"menu": format!("{label}: {}", menu.to_lowercase()), "help": help, "items": ids}));
                }
            }
            json!({"label": label, "items": items})
        })
        .collect();
    json!({"label": "Topology", "groups": groups})
}

/// The manifest as JSON.
#[must_use]
pub fn manifest() -> Value {
    json!({
        "id": "topology-checker",
        "name": "Topology Checker",
        "version": env!("CARGO_PKG_VERSION"),
        "description": "Checks map data for duplicate points, gaps, overlaps, dangles and other topology errors. One tool per rule; each adds a layer of the violations.",
        "author": "Alex-Andrei Cuvuliuc",
        "homepage": env!("CARGO_PKG_HOMEPAGE"),
        "license": env!("CARGO_PKG_LICENSE"),
        "entry": "plugin.wasm",
        "abi": rustgis_plugin::ABI,
        "minRustgisVersion": "0.1.0",
        "permissions": [],
        "tools": Rule::ALL.iter().map(|r| tool(*r)).collect::<Vec<_>>(),
        "tab": tab(),
    })
}
