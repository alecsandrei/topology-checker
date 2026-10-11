use crate::{
    codec, crs,
    native::stem,
    read::{RawFeature, RawLayer},
    write::{DatasetLayer, Record},
};
use anyhow::{Context, Result, bail, ensure};
use geo::{Geometry, LineString, MultiLineString};
use serde_json::{Map, Value, json};
use std::{
    collections::HashMap,
    fs::File,
    io::{BufReader, BufWriter, Read, Write},
    path::Path,
};
fn wgs84(layer: &DatasetLayer) -> Result<Vec<Record>> {
    let source = layer
        .srs
        .as_ref()
        .context("assign a CRS before KML or GPX export")?;
    let transform = crs::Transform::new(source, &crs::parse("EPSG:4326")?)?;
    layer
        .records
        .iter()
        .map(|r| {
            Ok(Record {
                id: r.id,
                properties: r.properties.clone(),
                geometry: r
                    .geometry
                    .as_ref()
                    .map(|g| crs::transform(g, &transform))
                    .transpose()?,
            })
        })
        .collect()
}
fn payload(r: &Record) -> String {
    json!({"id":r.id,"properties":r.properties,"type":r.geometry.as_ref().map(codec::type_name)})
        .to_string()
}
fn apply_payload(s: &str, f: &mut RawFeature) -> Result<bool> {
    let Ok(v) = serde_json::from_str::<Value>(s) else {
        return Ok(false);
    };
    let Some(p) = v["properties"].as_object() else {
        return Ok(false);
    };
    f.id = v["id"].as_i64();
    f.properties = p.clone();
    if let Some(Geometry::GeometryCollection(c)) = &f.geometry {
        f.geometry = Some(match v["type"].as_str() {
            Some("MULTIPOINT") => geo::MultiPoint(
                c.iter()
                    .map(|g| {
                        if let Geometry::Point(p) = g {
                            Ok(*p)
                        } else {
                            bail!("invalid KML multipoint")
                        }
                    })
                    .collect::<Result<_>>()?,
            )
            .into(),
            Some("MULTILINESTRING") => geo::MultiLineString(
                c.iter()
                    .map(|g| {
                        if let Geometry::LineString(p) = g {
                            Ok(p.clone())
                        } else {
                            bail!("invalid KML multiline")
                        }
                    })
                    .collect::<Result<_>>()?,
            )
            .into(),
            Some("MULTIPOLYGON") => geo::MultiPolygon(
                c.iter()
                    .map(|g| {
                        if let Geometry::Polygon(p) = g {
                            Ok(p.clone())
                        } else {
                            bail!("invalid KML multipolygon")
                        }
                    })
                    .collect::<Result<_>>()?,
            )
            .into(),
            _ => Geometry::GeometryCollection(c.clone()),
        });
    }
    Ok(true)
}
fn kml_zm(g: &kml::types::Geometry) -> bool {
    use kml::types::Geometry::*;
    match g {
        Point(p) => p.coord.z.is_some(),
        LineString(l) => l.coords.iter().any(|c| c.z.is_some()),
        LinearRing(l) => l.coords.iter().any(|c| c.z.is_some()),
        Polygon(p) => {
            p.outer.coords.iter().any(|c| c.z.is_some())
                || p.inner
                    .iter()
                    .any(|r| r.coords.iter().any(|c| c.z.is_some()))
        }
        MultiGeometry(c) => c.geometries.iter().any(kml_zm),
        _ => false,
    }
}
fn extended(elements: &[kml::types::Element], f: &mut RawFeature, depth: usize) -> Result<()> {
    ensure!(depth <= 64, "KML attribute nesting exceeds 64 levels");
    for e in elements {
        if e.name == "Data" || e.name == "SimpleData" {
            if let Some(name) = e.attrs.get("name") {
                let value = e
                    .content
                    .as_deref()
                    .or_else(|| {
                        e.children
                            .iter()
                            .find(|c| c.name == "value")
                            .and_then(|v| v.content.as_deref())
                    })
                    .unwrap_or_default();
                if name == "topology_checker" && apply_payload(value, f)? {
                    continue;
                }
                f.properties.insert(name.clone(), value.into());
            }
        } else {
            extended(&e.children, f, depth + 1)?;
        }
    }
    Ok(())
}
fn kml_features(k: &kml::Kml, features: &mut Vec<RawFeature>, depth: usize) -> Result<()> {
    ensure!(depth <= 64, "KML nesting exceeds 64 levels");
    use kml::Kml::*;
    match k {
        KmlDocument(d) => {
            for k in &d.elements {
                kml_features(k, features, depth + 1)?;
            }
        }
        Document { elements, .. } => {
            for k in elements {
                kml_features(k, features, depth + 1)?;
            }
        }
        Folder(d) => {
            for k in &d.elements {
                kml_features(k, features, depth + 1)?;
            }
        }
        Placemark(p) => {
            let mut properties = Map::new();
            if let Some(name) = &p.name {
                properties.insert("name".into(), name.clone().into());
            }
            if let Some(desc) = &p.description {
                properties.insert("description".into(), desc.clone().into());
            }
            let mut f = RawFeature {
                id: p.attrs.get("id").and_then(|s| s.parse().ok()),
                geometry: p.geometry.clone().map(Geometry::try_from).transpose()?,
                properties,
                zm: p.geometry.as_ref().is_some_and(kml_zm),
            };
            extended(&p.children, &mut f, 0)?;
            features.push(f);
        }
        _ => {}
    }
    Ok(())
}
pub(crate) fn read_kml(path: &Path) -> Result<Vec<RawLayer>> {
    let mut features = Vec::new();
    let document = if crate::registry::extension(path) == "kmz" {
        let mut zip = zip::ZipArchive::new(File::open(path)?)?;
        ensure!(zip.len() <= 10_000, "KMZ contains too many entries");
        let mut candidates = Vec::new();
        let mut total = 0u64;
        for i in 0..zip.len() {
            let e = zip.by_index(i)?;
            ensure!(e.enclosed_name().is_some(), "unsafe KMZ path");
            total = total.checked_add(e.size()).context("KMZ length overflow")?;
            if e.name().to_ascii_lowercase().ends_with(".kml") {
                candidates.push((i, e.name().to_string()));
            }
        }
        ensure!(
            total <= 512 * 1024 * 1024,
            "KMZ exceeds 512 MiB expansion allowance"
        );
        let i = candidates
            .iter()
            .find(|(_, n)| n.eq_ignore_ascii_case("doc.kml"))
            .or_else(|| {
                if candidates.len() == 1 {
                    candidates.first()
                } else {
                    None
                }
            })
            .context("KMZ must contain doc.kml or one KML document")?
            .0;
        let mut entry = zip.by_index(i)?;
        let size = entry.size();
        let mut data = Vec::new();
        Read::by_ref(&mut entry)
            .take(size + 1)
            .read_to_end(&mut data)?;
        ensure!(data.len() as u64 == size, "KMZ entry length mismatch");
        kml::KmlReader::<_, f64>::from_reader(std::io::Cursor::new(data)).read()?
    } else {
        kml::KmlReader::<_, f64>::from_path(path)?.read()?
    };
    ensure!(
        matches!(document, kml::Kml::KmlDocument(_)),
        "KML input must have a kml root element"
    );
    kml_features(&document, &mut features, 0)?;
    Ok(vec![RawLayer {
        name: stem(path),
        srs: Some(crs::parse("EPSG:4326")?),
        features,
    }])
}
pub(crate) fn write_kml(path: &Path, layer: &DatasetLayer) -> Result<()> {
    use kml::types::{Element, KmlDocument, KmlVersion, Placemark};
    let mut elements = Vec::new();
    for r in wgs84(layer)? {
        let data = Element {
            name: "ExtendedData".into(),
            children: vec![Element {
                name: "Data".into(),
                attrs: HashMap::from([("name".into(), "topology_checker".into())]),
                children: vec![Element {
                    name: "value".into(),
                    content: Some(payload(&r)),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        elements.push(kml::Kml::Placemark(Placemark {
            name: r
                .properties
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_string),
            attrs: r
                .id
                .map(|id| HashMap::from([("id".into(), id.to_string())]))
                .unwrap_or_default(),
            geometry: r
                .geometry
                .map(|g| kml::types::Geometry::from(codec::normalize(&g))),
            children: vec![data],
            ..Default::default()
        }));
    }
    let document = kml::Kml::KmlDocument(KmlDocument {
        version: KmlVersion::V22,
        elements,
        ..Default::default()
    });
    if crate::registry::extension(path) == "kmz" {
        let mut zip = zip::ZipWriter::new(File::create(path)?);
        zip.start_file(
            "doc.kml",
            zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated),
        )?;
        kml::KmlWriter::from_writer(&mut zip).write(&document)?;
        zip.finish()?;
    } else {
        kml::KmlWriter::from_writer(BufWriter::new(File::create(path)?)).write(&document)?;
    }
    Ok(())
}
pub(crate) fn read_gpx(path: &Path) -> Result<Vec<RawLayer>> {
    let data = gpx::read(BufReader::new(File::open(path)?))?;
    let mut features = Vec::new();
    for w in data.waypoints {
        let mut properties = Map::new();
        if let Some(n) = &w.name {
            properties.insert("name".into(), n.clone().into());
        }
        if let Some(d) = &w.description {
            properties.insert("description".into(), d.clone().into());
        }
        let mut f = RawFeature {
            id: None,
            geometry: Some(w.point().into()),
            properties,
            zm: w.elevation.is_some(),
        };
        if let Some(s) = w.description {
            apply_payload(&s, &mut f)?;
        }
        features.push(f);
    }
    for r in data.routes {
        let zm = r.points.iter().any(|w| w.elevation.is_some());
        let g = LineString::from(r.points.iter().map(|p| p.point().0).collect::<Vec<_>>());
        let mut properties = Map::new();
        if let Some(n) = r.name {
            properties.insert("name".into(), n.into());
        }
        let mut f = RawFeature {
            id: r.number.map(i64::from),
            geometry: Some(g.into()),
            properties,
            zm,
        };
        if let Some(s) = r.description
            && !apply_payload(&s, &mut f)?
        {
            f.properties.insert("description".into(), s.into());
        }
        features.push(f);
    }
    for t in data.tracks {
        let zm = t
            .segments
            .iter()
            .any(|s| s.points.iter().any(|w| w.elevation.is_some()));
        let lines = t
            .segments
            .iter()
            .map(|s| LineString::from(s.points.iter().map(|p| p.point().0).collect::<Vec<_>>()))
            .collect::<Vec<_>>();
        let mut properties = Map::new();
        if let Some(n) = t.name {
            properties.insert("name".into(), n.into());
        }
        let mut f = RawFeature {
            id: t.number.map(i64::from),
            geometry: Some(MultiLineString(lines).into()),
            properties,
            zm,
        };
        if let Some(s) = t.description
            && !apply_payload(&s, &mut f)?
        {
            f.properties.insert("description".into(), s.into());
        }
        features.push(f);
    }
    Ok(vec![RawLayer {
        name: stem(path),
        srs: Some(crs::parse("EPSG:4326")?),
        features,
    }])
}
pub(crate) fn write_gpx(path: &Path, layer: &DatasetLayer) -> Result<()> {
    let mut data = gpx::Gpx {
        version: gpx::GpxVersion::Gpx11,
        creator: Some("topology-checker".into()),
        ..Default::default()
    };
    for r in wgs84(layer)? {
        let description = Some(payload(&r));
        let name = r
            .properties
            .get("name")
            .and_then(Value::as_str)
            .map(str::to_string);
        match r
            .geometry
            .as_ref()
            .map(codec::normalize)
            .context("GPX cannot encode null geometry")?
        {
            Geometry::Point(p) => {
                let mut w = gpx::Waypoint::new(p);
                w.name = name;
                w.description = description;
                data.waypoints.push(w);
            }
            Geometry::LineString(l) => data.routes.push(gpx::Route {
                name,
                description,
                points: l.points().map(gpx::Waypoint::new).collect(),
                ..Default::default()
            }),
            Geometry::MultiLineString(m) => data.tracks.push(gpx::Track {
                name,
                description,
                segments: m
                    .0
                    .into_iter()
                    .map(|l| gpx::TrackSegment {
                        points: l.points().map(gpx::Waypoint::new).collect(),
                    })
                    .collect(),
                ..Default::default()
            }),
            _ => bail!(
                "GPX export supports points, lines and multilines; polygon and collection geometry is unsupported"
            ),
        }
    }
    let mut out = BufWriter::new(File::create(path)?);
    gpx::write(&data, &mut out)?;
    out.flush()?;
    Ok(())
}
