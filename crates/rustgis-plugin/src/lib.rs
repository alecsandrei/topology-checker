//! topology-checker as a RustGIS plugin: one geoprocessing tool per rule.
//!
//! RustGIS hands a tool each layer parameter as a GeoJSON FeatureCollection (longitude/latitude, only the
//! selected features when the layer has a selection). The tool runs the rule with the core engine and returns
//! the violations as a new layer.

mod manifest;

use geojson::Geometry as GeoJsonGeometry;
use rustgis_plugin::{Kind, Request, ToolOutput, Value, geojson as gj, json, serde_json};
use topology_checker_core::geo::Geometry;
use topology_checker_core::{Layer, LayerInput, Report, Rule, Topology};

pub use manifest::{manifest, rule_of, tool_id};

/// The manifest as pretty-printed JSON, the text of `plugin.json`.
#[must_use]
pub fn manifest_json() -> String {
    format!("{:#}", manifest())
}

fn handle(req: Request) -> Result<Value, String> {
    match (req.kind, rule_of(&req.id)) {
        (Kind::Tool, Some(rule)) => run(rule, &req).map(Value::from),
        _ => Err(format!("Topology Checker has no '{}'", req.id)),
    }
}

rustgis_plugin::export!(handle);

/// Build a core layer from a tool's GeoJSON FeatureCollection.
///
/// Features are keyed by their `row` property when every feature has a distinct one (so results map back to
/// the source layer), else by position. Features without geometry are skipped.
fn read_layer(name: &str, collection: &Value) -> Result<Layer, String> {
    let features = collection
        .get("features")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    let rows: Option<Vec<i64>> = features
        .iter()
        .map(|f| f["properties"]["row"].as_i64())
        .collect();
    let rows =
        rows.filter(|r| r.iter().collect::<std::collections::BTreeSet<_>>().len() == r.len());
    let mut kept = Vec::with_capacity(features.len());
    for (i, f) in features.iter().enumerate() {
        if f["geometry"].is_null() {
            continue;
        }
        let g: GeoJsonGeometry = serde_json::from_value(f["geometry"].clone())
            .map_err(|e| format!("'{name}': feature {i}: {e}"))?;
        let g = Geometry::<f64>::try_from(&g).map_err(|e| format!("'{name}': feature {i}: {e}"))?;
        kept.push((rows.as_ref().map_or(i as i64, |r| r[i]), g));
    }
    if kept.is_empty() {
        return Err(format!("'{name}' has no features with geometry"));
    }
    Ok(Layer::from_features(name, kept))
}

fn run(rule: Rule, req: &Request) -> Result<ToolOutput, String> {
    let tolerance = req.f64("tolerance").unwrap_or(manifest::DEFAULT_TOLERANCE);
    let name = req.str("layer").ok_or("'layer' is required")?.to_string();
    let first = read_layer(&name, req.layer("layer")?)?;
    let second = if rule.inputs().1.is_some() {
        let other = req.str("other").ok_or("'other' is required")?.to_string();
        if other == name {
            return Err(format!(
                "{} compares two layers: choose a second layer other than '{name}'",
                rule.name()
            ));
        }
        Some(LayerInput::New(Box::new(read_layer(
            &other,
            req.layer("other")?,
        )?)))
    } else {
        None
    };
    let mut topology = Topology::new(tolerance).map_err(|e| e.to_string())?;
    topology
        .register_rule(rule, LayerInput::New(Box::new(first)), second)
        .map_err(|e| e.to_string())?;
    let report = topology.validate().map_err(|e| e.to_string())?;
    Ok(output(rule, &name, &report))
}

fn output(rule: Rule, layer: &str, report: &Report) -> ToolOutput {
    let mut out = ToolOutput::new();
    for w in &report.warnings {
        out = out.message(format!("Warning: {w}"));
    }
    let total = report.violation_count();
    if total == 0 {
        return out.message(format!("{}: no violations found.", rule.name()));
    }
    let features: Vec<Value> = report
        .violations()
        .map(|v| {
            let fid = |r: Option<topology_checker_core::FeatureRef>| r.map(|r| r.fid);
            gj::feature(
                serde_json::to_value(GeoJsonGeometry::from(&v.geometry)).unwrap_or(Value::Null),
                json!({
                    "rule": v.rule.name(),
                    "error_type": v.kind.name(),
                    "origin_fid": fid(v.origin),
                    "origin_part": v.origin.and_then(|r| r.part),
                    "destination_fid": fid(v.destination),
                    "destination_part": v.destination.and_then(|r| r.part),
                }),
            )
        })
        .collect();
    for r in &report.rules {
        let counts: Vec<String> = r
            .counts
            .iter()
            .map(|(k, n)| format!("{n} {}", k.name()))
            .collect();
        out = out.message(format!(
            "{}: {total} violation(s): {}.",
            rule.name(),
            counts.join(", ")
        ));
    }
    out.layer(
        &format!("{layer}: {}", rule.name()),
        gj::collection(features),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustgis_plugin::Map;

    fn request(id: &str, params: Value, layers: &[(&str, Value)]) -> Request {
        Request {
            kind: Kind::Tool,
            id: id.into(),
            params,
            layers: layers
                .iter()
                .map(|(n, v)| ((*n).to_string(), v.clone()))
                .collect::<Map<_, _>>(),
            files: Map::new(),
            lang: "en".into(),
        }
    }

    fn polygons(rings: &[[[f64; 2]; 5]]) -> Value {
        let features = rings
            .iter()
            .enumerate()
            .map(|(i, r)| {
                gj::feature(
                    json!({"type": "Polygon", "coordinates": [r]}),
                    json!({"row": 10 + i}),
                )
            })
            .collect();
        gj::collection(features)
    }

    fn square(x: f64, y: f64) -> [[f64; 2]; 5] {
        rect(x, y, x + 1.0, y + 1.0)
    }

    fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> [[f64; 2]; 5] {
        [[x0, y0], [x1, y0], [x1, y1], [x0, y1], [x0, y0]]
    }

    fn call(req: Request) -> Result<Value, String> {
        handle(req)
    }

    #[test]
    fn every_rule_has_one_tool() {
        let m = manifest();
        let ids: Vec<&str> = m["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids.len(), Rule::ALL.len());
        let unique: std::collections::BTreeSet<_> = ids.iter().collect();
        assert_eq!(unique.len(), ids.len());
        for r in Rule::ALL {
            assert_eq!(rule_of(&tool_id(*r)), Some(*r));
        }
    }

    #[test]
    fn ribbon_names_only_own_tools_within_limits() {
        let m = manifest();
        let tools: std::collections::BTreeSet<String> = m["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| format!("tool:{}", t["id"].as_str().unwrap()))
            .collect();
        let groups = m["tab"]["groups"].as_array().unwrap();
        assert!((1..=8).contains(&groups.len()));
        let mut listed = 0;
        for g in groups {
            let items = g["items"].as_array().unwrap();
            assert!((1..=16).contains(&items.len()));
            for menu in items {
                let entries = menu["items"].as_array().unwrap();
                assert!((1..=32).contains(&entries.len()));
                for e in entries {
                    assert!(tools.contains(e.as_str().unwrap()), "{e}");
                    listed += 1;
                }
            }
        }
        assert_eq!(
            listed,
            tools.len(),
            "every tool is on the ribbon exactly once"
        );
    }

    #[test]
    fn committed_manifest_is_current() {
        let committed: Value = serde_json::from_str(include_str!("../plugin.json")).unwrap();
        assert_eq!(
            committed,
            manifest(),
            "regenerate: cargo run -p topology-checker-rustgis --example manifest > crates/rustgis-plugin/plugin.json"
        );
    }

    #[test]
    fn gaps_become_a_violation_layer_keyed_by_the_source_rows() {
        // Four rectangles around an empty square: a gap in the middle.
        let layer = polygons(&[
            rect(0.0, 0.0, 3.0, 1.0),
            rect(0.0, 2.0, 3.0, 3.0),
            rect(0.0, 1.0, 1.0, 2.0),
            rect(2.0, 1.0, 3.0, 2.0),
        ]);
        let out = call(request(
            "polygon-must-not-have-gaps",
            json!({"layer": "Parcels", "tolerance": 1e-9}),
            &[("Parcels", layer)],
        ))
        .unwrap();
        let layers = out["layers"].as_array().unwrap();
        assert_eq!(layers.len(), 1);
        assert_eq!(layers[0]["name"], "Parcels: polygon/must-not-have-gaps");
        let features = layers[0]["geojson"]["features"].as_array().unwrap();
        let gaps: Vec<_> = features
            .iter()
            .filter(|f| f["properties"]["error_type"] == "gap")
            .collect();
        assert_eq!(gaps.len(), 1);
        assert!(out["messages"][0].as_str().unwrap().contains("violation"));
    }

    #[test]
    fn two_layer_rules_report_the_first_layers_row() {
        let points = gj::collection(vec![
            gj::feature(gj::point(0.5, 0.5), json!({"row": 7})),
            gj::feature(gj::point(5.0, 5.0), json!({"row": 8})),
        ]);
        let zones = polygons(&[square(0.0, 0.0)]);
        let params = json!({"layer": "Wells", "other": "Zones", "tolerance": 1e-9});
        let out = call(request(
            "point-must-be-inside",
            params,
            &[("Wells", points), ("Zones", zones)],
        ))
        .unwrap();
        let features = out["layers"][0]["geojson"]["features"].as_array().unwrap();
        assert_eq!(features.len(), 1);
        assert_eq!(features[0]["properties"]["origin_fid"], 8);
    }

    #[test]
    fn clean_data_adds_no_layer() {
        let layer = polygons(&[square(0.0, 0.0), square(1.0, 0.0)]);
        let out = call(request(
            "polygon-must-not-have-gaps",
            json!({"layer": "A", "tolerance": 1e-9}),
            &[("A", layer)],
        ))
        .unwrap();
        // The outer boundary is reported separately, but no gap exists between touching squares.
        let features = out["layers"].as_array().map(|l| {
            l.iter()
                .flat_map(|l| l["geojson"]["features"].as_array().unwrap().clone())
                .collect::<Vec<_>>()
        });
        assert!(
            features
                .unwrap_or_default()
                .iter()
                .all(|f| f["properties"]["error_type"] != "gap")
        );
    }

    #[test]
    fn mistakes_are_reported_not_panicked() {
        let layer = polygons(&[square(0.0, 0.0)]);
        let err = |req| call(req).unwrap_err();
        assert!(err(request("nope", json!({}), &[])).contains("no 'nope'"));
        assert!(
            err(request(
                "polygon-must-not-have-gaps",
                json!({"layer": "Missing"}),
                &[("A", layer.clone())]
            ))
            .contains("no layer named")
        );
        assert!(
            err(request(
                "polygon-must-not-have-gaps",
                json!({"layer": "A", "tolerance": -1.0}),
                &[("A", layer.clone())]
            ))
            .to_lowercase()
            .contains("tolerance")
        );
        let same = json!({"layer": "A", "other": "A", "tolerance": 1e-9});
        assert!(
            err(request(
                "polygon-must-not-overlap-with",
                same,
                &[("A", layer.clone())]
            ))
            .contains("second layer")
        );
        let points = gj::collection(vec![gj::feature(gj::point(0.0, 0.0), json!({}))]);
        assert!(
            !err(request(
                "polygon-must-not-have-gaps",
                json!({"layer": "P", "tolerance": 1e-9}),
                &[("P", points)]
            ))
            .is_empty()
        );
        let empty = gj::collection(vec![]);
        assert!(
            err(request(
                "polygon-must-not-have-gaps",
                json!({"layer": "E"}),
                &[("E", empty)]
            ))
            .contains("no features")
        );
    }
}
