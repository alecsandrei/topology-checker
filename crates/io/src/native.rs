use crate::{
    codec, crs,
    read::{RawFeature, RawLayer},
    write::{DatasetLayer, FieldType},
};
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Map, Value, json};
use std::{
    fs::File,
    io::{BufReader, BufWriter},
    path::Path,
};
pub(crate) fn stem(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("dataset")
        .into()
}
fn numeric_id(value: Option<&Value>) -> Option<i64> {
    value.and_then(|v| {
        v.as_i64()
            .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
    })
}
fn has_zm(value: &Value) -> bool {
    match value {
        Value::Array(a) => {
            if a.first().is_some_and(Value::is_number) {
                a.len() > 2
            } else {
                a.iter().any(has_zm)
            }
        }
        Value::Object(o) => {
            o.get("coordinates").is_some_and(has_zm) || o.get("geometries").is_some_and(has_zm)
        }
        _ => false,
    }
}
pub(crate) fn read_geojson(path: &Path) -> Result<Vec<RawLayer>> {
    let root: Value = serde_json::from_reader(BufReader::new(File::open(path)?))?;
    let srs = match root.get("crs") {
        None | Some(Value::Null) => Some(crs::parse("EPSG:4326")?),
        Some(c) => Some(crs::parse(
            c.pointer("/properties/name")
                .and_then(Value::as_str)
                .context("unsupported GeoJSON CRS")?,
        )?),
    };
    let source = match root.get("type").and_then(Value::as_str) {
        Some("FeatureCollection") => root["features"]
            .as_array()
            .context("GeoJSON features must be an array")?
            .clone(),
        Some("Feature") => vec![root.clone()],
        _ => vec![json!({"geometry":root,"properties":{}})],
    };
    let mut features = Vec::new();
    for f in source {
        let g = f
            .get("geometry")
            .context("GeoJSON feature missing geometry")?;
        let geometry = if g.is_null() {
            None
        } else {
            Some(geojson::Geometry::try_from(g.clone())?.try_into()?)
        };
        let properties = match f.get("properties") {
            None | Some(Value::Null) => Map::new(),
            Some(p) => p
                .as_object()
                .context("GeoJSON properties must be an object")?
                .clone(),
        };
        features.push(RawFeature {
            id: numeric_id(f.get("id")),
            geometry,
            properties,
            zm: has_zm(g),
        });
    }
    Ok(vec![RawLayer {
        name: root
            .get("name")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| stem(path)),
        srs,
        features,
    }])
}
pub(crate) fn write_geojson(path: &Path, layer: &DatasetLayer) -> Result<()> {
    write_geojson_source(path, layer)
}
pub(crate) fn write_geojson_source(
    path: &Path,
    source: &dyn crate::write::RecordSource,
) -> Result<()> {
    use std::io::Write;
    let layer = source.metadata();
    let mut out = BufWriter::new(File::create(path)?);
    out.write_all(b"{\"type\":\"FeatureCollection\",\"name\":")?;
    serde_json::to_writer(&mut out, &layer.name)?;
    if let Some(s) = &layer.srs
        && s.epsg() != Some(4326)
    {
        out.write_all(b",\"crs\":")?;
        let crs = json!({"type":"name","properties":{"name":s.authority().unwrap_or(s.to_wkt()?)}});
        serde_json::to_writer(&mut out, &crs)?;
    }
    out.write_all(b",\"features\":[")?;
    for (i, r) in source.records().enumerate() {
        let r = r?;
        if i > 0 {
            out.write_all(b",")?;
        }
        let mut f = json!({"type":"Feature","geometry":r.geometry.as_ref().map(|g|geojson::Geometry::from(&codec::normalize(g))),"properties":r.properties});
        if let Some(id) = r.id {
            f["id"] = id.into();
        }
        serde_json::to_writer(&mut out, &f)?;
    }
    out.write_all(b"]}")?;
    out.flush()?;
    Ok(())
}

fn dbf_json(v: shapefile::dbase::FieldValue) -> Value {
    use shapefile::dbase::FieldValue::*;
    match v {
        Character(v) => v.map(Value::from).unwrap_or(Value::Null),
        Numeric(v) => v.map(Value::from).unwrap_or(Value::Null),
        Float(v) => v.map(Value::from).unwrap_or(Value::Null),
        Logical(v) => v.map(Value::from).unwrap_or(Value::Null),
        Integer(v) => v.into(),
        Double(v) | Currency(v) => v.into(),
        Date(v) => v
            .map(|d| format!("{:04}-{:02}-{:02}", d.year(), d.month(), d.day()).into())
            .unwrap_or(Value::Null),
        DateTime(v) => format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
            v.date().year(),
            v.date().month(),
            v.date().day(),
            v.time().hours(),
            v.time().minutes(),
            v.time().seconds()
        )
        .into(),
        Memo(v) => v.into(),
    }
}
pub(crate) fn read_shape(path: &Path) -> Result<Vec<RawLayer>> {
    use shapefile::{Reader, Shape, ShapeReader, dbase};
    let mut builder = dbase::ReaderBuilder::new();
    let cpg = path.with_extension("cpg");
    if cpg.exists() {
        let label = std::fs::read_to_string(cpg)?;
        let label = match label.trim() {
            "65001" => "UTF-8",
            "1252" => "windows-1252",
            s => s,
        };
        let encoding = encoding_rs::Encoding::for_label(label.as_bytes())
            .context("unsupported Shapefile CPG encoding")?;
        builder = builder.with_encoding(dbase::encoding::EncodingRs::from(encoding));
    }
    let dbf = builder.open(path.with_extension("dbf"))?;
    let exact_integers = dbf_integer_values(&path.with_extension("dbf"), dbf.fields())?;
    let shapes = ShapeReader::from_path(path)?;
    let mut reader = Reader::new(shapes, dbf);
    let srs = if path.with_extension("prj").exists() {
        let wkt = std::fs::read_to_string(path.with_extension("prj"))?;
        Some(crs::parse(&wkt)?)
    } else {
        None
    };
    let mut features = Vec::new();
    for (ordinal, item) in reader.iter_shapes_and_records().enumerate() {
        let (shape, record) = item?;
        let zm = matches!(
            shape,
            Shape::PointZ(_)
                | Shape::PointM(_)
                | Shape::PolylineZ(_)
                | Shape::PolylineM(_)
                | Shape::PolygonZ(_)
                | Shape::PolygonM(_)
                | Shape::MultipointZ(_)
                | Shape::MultipointM(_)
        );
        let geometry = if matches!(shape, Shape::NullShape) {
            None
        } else {
            Some(geo::Geometry::try_from(shape).map_err(|e| anyhow::anyhow!("{e}"))?)
        };
        let mut properties = record
            .into_iter()
            .map(|(n, v)| (n, dbf_json(v)))
            .collect::<Map<_, _>>();
        if let Some(values) = exact_integers.get(ordinal) {
            properties.extend(values.clone());
        }
        features.push(RawFeature {
            id: Some(ordinal.try_into()?),
            geometry,
            properties,
            zm,
        });
    }
    Ok(vec![RawLayer {
        name: stem(path),
        srs,
        features,
    }])
}
// dbase represents every ASCII numeric column as f64. Recover integral values
// from their decimal bytes before that conversion can discard low bits.
fn dbf_integer_values(
    path: &Path,
    fields: &[shapefile::dbase::FieldInfo],
) -> Result<Vec<Map<String, Value>>> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = BufReader::new(File::open(path)?);
    let mut header = [0u8; 32];
    file.read_exact(&mut header)?;
    let rows = u32::from_le_bytes(header[4..8].try_into()?) as u64;
    let begin = u16::from_le_bytes(header[8..10].try_into()?) as u64;
    let width = u16::from_le_bytes(header[10..12].try_into()?) as usize;
    ensure!(
        width >= 1
            && fields
                .iter()
                .map(|f| usize::from(f.length()))
                .sum::<usize>()
                < width,
        "invalid DBF record width"
    );
    ensure!(
        begin + rows * width as u64 <= file.get_ref().metadata()?.len(),
        "truncated DBF records"
    );
    file.seek(SeekFrom::Start(begin))?;
    let mut output = Vec::new();
    let mut row = vec![0u8; width];
    for _ in 0..rows {
        file.read_exact(&mut row)?;
        if row[0] == b'*' {
            continue;
        }
        let mut offset = 1;
        let mut values = Map::new();
        for field in fields {
            let end = offset + usize::from(field.length());
            if field.field_type() == shapefile::dbase::FieldType::Numeric {
                let s = std::str::from_utf8(&row[offset..end])?.trim();
                if let Ok(n) = s.parse::<i64>() {
                    values.insert(field.name().into(), n.into());
                }
            }
            offset = end;
        }
        output.push(values);
    }
    Ok(output)
}
pub(crate) fn write_shape(path: &Path, layer: &DatasetLayer) -> Result<()> {
    write_shape_source(path, layer)
}
pub(crate) fn write_shape_source(
    path: &Path,
    source: &dyn crate::write::RecordSource,
) -> Result<()> {
    let layer = source.metadata();
    use shapefile::{
        Shape, Writer,
        dbase::{self, FieldValue, TableWriterBuilder},
    };
    let mut table =
        TableWriterBuilder::with_encoding(dbase::encoding::EncodingRs::from(encoding_rs::UTF_8));
    for f in &layer.fields {
        let name = f
            .name
            .as_str()
            .try_into()
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        table = match f.kind {
            FieldType::Integer => table.add_numeric_field(name, 20, 0),
            FieldType::Real => table.add_numeric_field(name, 24, 10),
            FieldType::Boolean => table.add_logical_field(name),
            FieldType::Text => table.add_character_field(name, 254),
        };
    }
    let mut writer = Writer::from_path(path, table)?;
    for r in source.records() {
        let r = r?;
        let g = r
            .geometry
            .as_ref()
            .context("Shapefile export cannot encode null geometries")?;
        ensure!(
            !codec::empty(g),
            "Shapefile export cannot encode empty geometries"
        );
        let shape = Shape::try_from(codec::normalize(g)).map_err(|e| anyhow::anyhow!("{e}"))?;
        let mut record = dbase::Record::default();
        for f in &layer.fields {
            let v = r.properties.get(&f.name).unwrap_or(&Value::Null);
            let v = match f.kind {
                FieldType::Integer | FieldType::Real => FieldValue::Numeric(v.as_f64()),
                FieldType::Boolean => FieldValue::Logical(v.as_bool()),
                FieldType::Text => {
                    let s = if v.is_null() {
                        None
                    } else {
                        Some(
                            v.as_str()
                                .map(str::to_string)
                                .unwrap_or_else(|| v.to_string()),
                        )
                    };
                    ensure!(
                        s.as_ref().is_none_or(|s| s.len() <= 254),
                        "Shapefile attribute exceeds 254 bytes"
                    );
                    FieldValue::Character(s)
                }
            };
            record.insert(f.name.clone(), v);
        }
        match shape {
            Shape::Point(s) => writer.write_shape_and_record(&s, &record)?,
            Shape::Polyline(s) => writer.write_shape_and_record(&s, &record)?,
            Shape::Polygon(s) => writer.write_shape_and_record(&s, &record)?,
            Shape::Multipoint(s) => writer.write_shape_and_record(&s, &record)?,
            _ => bail!("unsupported output Shapefile geometry"),
        }
    }
    drop(writer);
    // Keep DBF numeric columns interoperable while replacing the f64-based
    // writer's rounded decimal representation with each exact signed integer.
    use std::io::{Read, Seek, SeekFrom, Write};
    let mut dbf = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path.with_extension("dbf"))?;
    let mut header = [0u8; 32];
    dbf.read_exact(&mut header)?;
    let begin = u16::from_le_bytes(header[8..10].try_into()?) as u64;
    let width = u16::from_le_bytes(header[10..12].try_into()?) as u64;
    for (ordinal, record) in source.records().enumerate() {
        let record = record?;
        let mut offset = 1u64;
        for field in &layer.fields {
            if field.kind == FieldType::Integer
                && let Some(value) = record.properties.get(&field.name).filter(|v| !v.is_null())
            {
                let value = value.as_i64().context("integer attribute required")?;
                dbf.seek(SeekFrom::Start(begin + ordinal as u64 * width + offset))?;
                dbf.write_all(format!("{value:>20}").as_bytes())?;
            }
            offset += match field.kind {
                FieldType::Integer => 20,
                FieldType::Real => 24,
                FieldType::Boolean => 1,
                FieldType::Text => 254,
            };
        }
    }
    dbf.flush()?;
    std::fs::write(path.with_extension("cpg"), "UTF-8")?;
    if let Some(s) = &layer.srs {
        std::fs::write(path.with_extension("prj"), s.to_wkt()?)?;
    }
    Ok(())
}
#[derive(Default)]
struct Properties(Map<String, Value>);
impl geozero::PropertyProcessor for Properties {
    fn property(
        &mut self,
        _: usize,
        name: &str,
        v: &geozero::ColumnValue,
    ) -> geozero::error::Result<bool> {
        use geozero::ColumnValue::*;
        let v = match v {
            Byte(n) => Value::from(*n),
            UByte(n) => Value::from(*n),
            Short(n) => Value::from(*n),
            UShort(n) => Value::from(*n),
            Int(n) => Value::from(*n),
            UInt(n) => Value::from(*n),
            Long(n) => Value::from(*n),
            ULong(n) => Value::from(*n),
            Float(n) => Value::from(*n),
            Double(n) => Value::from(*n),
            Bool(n) => Value::from(*n),
            String(s) | DateTime(s) => Value::from(*s),
            Json(s) => serde_json::from_str(s)
                .map_err(|e| geozero::error::GeozeroError::Property(e.to_string()))?,
            Binary(b) => Value::Array(b.iter().map(|n| Value::from(*n)).collect()),
        };
        self.0.insert(name.into(), v);
        Ok(false)
    }
}
pub(crate) fn read_fgb(path: &Path) -> Result<Vec<RawLayer>> {
    use flatgeobuf::{FallibleStreamingIterator, FgbReader};
    use geozero::FeatureProperties;
    let reader = FgbReader::open(BufReader::new(File::open(path)?))?;
    let header = reader.header();
    let name = header
        .name()
        .map(str::to_string)
        .unwrap_or_else(|| stem(path));
    let srs = header
        .crs()
        .map(|s| {
            if let Some(wkt) = s.wkt() {
                crs::parse(wkt)
            } else {
                crs::parse(&format!("{}:{}", s.org().unwrap_or("EPSG"), s.code()))
            }
        })
        .transpose()?;
    let zm = header.has_z() || header.has_m();
    let geometry_type = header.geometry_type();
    let identity = header
        .metadata()
        .and_then(|s| serde_json::from_str::<Value>(s).ok())
        .and_then(|v| {
            v.get("topology_checker_fid")
                .and_then(Value::as_str)
                .map(str::to_string)
        });
    let columns = header
        .columns()
        .map(|c| c.iter().map(|c| c.name().to_string()).collect::<Vec<_>>())
        .unwrap_or_default();
    let mut reader = reader.select_all()?;
    let mut features = Vec::new();
    while let Some(feature) = reader.next()? {
        let mut properties = Properties::default();
        feature.process_properties(&mut properties)?;
        for name in &columns {
            properties.0.entry(name.clone()).or_insert(Value::Null);
        }
        let id = if let Some(name) = &identity {
            numeric_id(properties.0.remove(name).as_ref())
        } else {
            Some(i64::try_from(features.len())?)
        };
        let geometry = if feature.geometry().is_none_or(|g| {
            g.type_() == flatgeobuf::GeometryType::Unknown
                && g.xy().is_none_or(|xy| xy.is_empty())
                && g.parts().is_none_or(|p| p.is_empty())
        }) {
            None
        } else {
            Some(read_fgb_geometry(
                feature.geometry().unwrap(),
                geometry_type,
                0,
            )?)
        };
        features.push(RawFeature {
            id,
            geometry,
            properties: properties.0,
            zm,
        });
    }
    Ok(vec![RawLayer {
        name,
        srs,
        features,
    }])
}
// Build geometries through the official FlatGeobuf schema. The streaming
// FgbWriter facade does not forward collection events, and FeatureWriter's
// collection state cannot preserve nested collections.
fn fgb_geometry<'a>(
    builder: &mut flatbuffers::FlatBufferBuilder<'a>,
    g: &geo::Geometry,
    depth: usize,
) -> Result<flatbuffers::WIPOffset<flatgeobuf::Geometry<'a>>> {
    use flatgeobuf::{Geometry as FgbGeometry, GeometryArgs, GeometryType};
    use geo::Geometry::*;
    ensure!(depth <= 64, "FlatGeobuf geometry nesting exceeds 64 levels");
    let mut xy = Vec::new();
    let mut ends = Vec::new();
    let mut children = Vec::new();
    let mut add_line = |line: &geo::LineString| -> Result<()> {
        for c in &line.0 {
            xy.extend([c.x, c.y]);
        }
        ends.push(u32::try_from(xy.len() / 2)?);
        Ok(())
    };
    let kind = match g {
        Point(p) => {
            xy.extend([p.x(), p.y()]);
            GeometryType::Point
        }
        LineString(l) => {
            add_line(l)?;
            GeometryType::LineString
        }
        Polygon(p) => {
            if !p.exterior().0.is_empty() {
                add_line(p.exterior())?;
                for r in p.interiors() {
                    add_line(r)?;
                }
            }
            GeometryType::Polygon
        }
        MultiPoint(p) => {
            for p in &p.0 {
                xy.extend([p.x(), p.y()]);
            }
            GeometryType::MultiPoint
        }
        MultiLineString(p) => {
            for l in &p.0 {
                add_line(l)?;
            }
            GeometryType::MultiLineString
        }
        MultiPolygon(p) => {
            for p in &p.0 {
                children.push(fgb_geometry(builder, &p.clone().into(), depth + 1)?);
            }
            GeometryType::MultiPolygon
        }
        GeometryCollection(c) => {
            for g in c {
                children.push(fgb_geometry(builder, g, depth + 1)?);
            }
            GeometryType::GeometryCollection
        }
        _ => return fgb_geometry(builder, &codec::normalize(g), depth),
    };
    let xy = (!xy.is_empty()).then(|| builder.create_vector(&xy));
    let ends = (!ends.is_empty()
        && matches!(kind, GeometryType::Polygon | GeometryType::MultiLineString))
    .then(|| builder.create_vector(&ends));
    let parts = matches!(
        kind,
        GeometryType::GeometryCollection | GeometryType::MultiPolygon
    )
    .then(|| builder.create_vector(&children));
    Ok(FgbGeometry::create(
        builder,
        &GeometryArgs {
            type_: kind,
            xy,
            ends,
            parts,
            ..Default::default()
        },
    ))
}
pub(crate) fn write_fgb(path: &Path, layer: &DatasetLayer) -> Result<()> {
    write_fgb_source(path, layer)
}
pub(crate) fn write_fgb_source(path: &Path, source: &dyn crate::write::RecordSource) -> Result<()> {
    let layer = source.metadata();
    use flatgeobuf::{
        Column, ColumnArgs, ColumnType, Crs, CrsArgs, Feature, FeatureArgs, Header, HeaderArgs,
    };
    use std::io::Write;
    let mut identity = "__topology_fid".to_string();
    while layer.fields.iter().any(|f| f.name == identity) {
        identity.push('_');
    }
    let mut json_fields = std::collections::HashSet::new();
    let mut feature_count = 0_u64;
    for r in source.records() {
        let r = r?;
        feature_count += 1;
        for f in &layer.fields {
            if f.kind == FieldType::Text
                && r.properties
                    .get(&f.name)
                    .is_some_and(|v| !v.is_null() && !v.is_string())
            {
                json_fields.insert(f.name.as_str());
            }
        }
    }
    let mut builder = flatbuffers::FlatBufferBuilder::new();
    let mut columns = Vec::new();
    let name = builder.create_string(&identity);
    columns.push(Column::create(
        &mut builder,
        &ColumnArgs {
            name: Some(name),
            type_: ColumnType::Long,
            ..Default::default()
        },
    ));
    ensure!(
        layer.fields.len() < u16::MAX as usize,
        "too many FlatGeobuf columns"
    );
    for f in &layer.fields {
        let name = builder.create_string(&f.name);
        columns.push(Column::create(
            &mut builder,
            &ColumnArgs {
                name: Some(name),
                type_: match f.kind {
                    FieldType::Integer => ColumnType::Long,
                    FieldType::Real => ColumnType::Double,
                    FieldType::Boolean => ColumnType::Bool,
                    FieldType::Text if json_fields.contains(f.name.as_str()) => ColumnType::Json,
                    FieldType::Text => ColumnType::String,
                },
                ..Default::default()
            },
        ));
    }
    let columns = builder.create_vector(&columns);
    let name = builder.create_string(&layer.name);
    let metadata = builder.create_string(&json!({"topology_checker_fid":identity}).to_string());
    let crs = if let Some(s) = &layer.srs {
        let wkt = builder.create_string(&s.to_wkt()?);
        let org = builder.create_string("EPSG");
        Some(Crs::create(
            &mut builder,
            &CrsArgs {
                org: Some(org),
                code: s.epsg().unwrap_or(0).try_into()?,
                wkt: Some(wkt),
                ..Default::default()
            },
        ))
    } else {
        None
    };
    let header = Header::create(
        &mut builder,
        &HeaderArgs {
            name: Some(name),
            columns: Some(columns),
            features_count: feature_count,
            index_node_size: 0,
            crs,
            metadata: Some(metadata),
            ..Default::default()
        },
    );
    flatgeobuf::finish_size_prefixed_header_buffer(&mut builder, header);
    let mut output = BufWriter::new(File::create(path)?);
    output.write_all(&[b'f', b'g', b'b', 3, b'f', b'g', b'b', 0])?;
    output.write_all(builder.finished_data())?;
    for (ordinal, r) in source.records().enumerate() {
        let r = r?;
        let mut builder = flatbuffers::FlatBufferBuilder::new();
        let geometry = r
            .geometry
            .as_ref()
            .map(|g| fgb_geometry(&mut builder, g, 0))
            .transpose()?;
        let mut properties = Vec::new();
        properties.extend(0u16.to_le_bytes());
        properties.extend(r.id.unwrap_or(i64::try_from(ordinal)?).to_le_bytes());
        for (i, f) in layer.fields.iter().enumerate() {
            let Some(v) = r.properties.get(&f.name).filter(|v| !v.is_null()) else {
                continue;
            };
            properties.extend(u16::try_from(i + 1)?.to_le_bytes());
            match f.kind {
                FieldType::Integer => properties.extend(
                    v.as_i64()
                        .context("integer attribute required")?
                        .to_le_bytes(),
                ),
                FieldType::Real => properties.extend(
                    v.as_f64()
                        .context("numeric attribute required")?
                        .to_le_bytes(),
                ),
                FieldType::Boolean => {
                    properties.push(u8::from(v.as_bool().context("boolean attribute required")?))
                }
                FieldType::Text => {
                    let s = if json_fields.contains(f.name.as_str()) {
                        v.to_string()
                    } else {
                        v.as_str().context("text attribute required")?.to_string()
                    };
                    properties.extend(u32::try_from(s.len())?.to_le_bytes());
                    properties.extend(s.as_bytes());
                }
            }
        }
        let properties = builder.create_vector(&properties);
        let feature = Feature::create(
            &mut builder,
            &FeatureArgs {
                geometry,
                properties: Some(properties),
                ..Default::default()
            },
        );
        flatgeobuf::finish_size_prefixed_feature_buffer(&mut builder, feature);
        output.write_all(builder.finished_data())?;
    }
    output.flush()?;
    Ok(())
}

fn read_fgb_geometry(
    g: flatgeobuf::Geometry<'_>,
    declared: flatgeobuf::GeometryType,
    depth: usize,
) -> Result<geo::Geometry> {
    use flatgeobuf::GeometryType;
    use geo::{
        Coord, Geometry, GeometryCollection, LineString, MultiLineString, MultiPoint, MultiPolygon,
        Point, Polygon,
    };
    ensure!(depth <= 64, "FlatGeobuf geometry nesting exceeds 64 levels");
    let ty = if declared == GeometryType::Unknown {
        g.type_()
    } else {
        declared
    };
    let values = g.xy();
    let len = values.map_or(0, |v| v.len());
    ensure!(len.is_multiple_of(2), "odd FlatGeobuf coordinate count");
    let mut coords = Vec::with_capacity(len / 2);
    if let Some(values) = values {
        for i in (0..len).step_by(2) {
            let c = Coord {
                x: values.get(i),
                y: values.get(i + 1),
            };
            ensure!(
                c.x.is_finite() && c.y.is_finite()
                    || ty == GeometryType::Point && c.x.is_nan() && c.y.is_nan(),
                "non-finite FlatGeobuf coordinate"
            );
            coords.push(c);
        }
    }
    let rings = || -> Result<Vec<LineString>> {
        if let Some(ends) = g.ends() {
            let mut begin = 0;
            let mut rings = Vec::new();
            for end in ends {
                let end = end as usize;
                ensure!(
                    end >= begin && end <= coords.len(),
                    "invalid FlatGeobuf ring offsets"
                );
                rings.push(LineString(coords[begin..end].to_vec()));
                begin = end;
            }
            ensure!(begin == coords.len(), "unconsumed FlatGeobuf coordinates");
            Ok(rings)
        } else if coords.is_empty() {
            Ok(vec![])
        } else {
            Ok(vec![LineString(coords.clone())])
        }
    };
    Ok(match ty {
        GeometryType::Point => {
            ensure!(
                coords.len() <= 1,
                "FlatGeobuf point contains multiple coordinates"
            );
            Point(coords.first().copied().unwrap_or(Coord {
                x: f64::NAN,
                y: f64::NAN,
            }))
            .into()
        }
        GeometryType::LineString => LineString(coords).into(),
        GeometryType::MultiPoint => MultiPoint(coords.into_iter().map(Point).collect()).into(),
        GeometryType::Polygon => {
            let mut rings = rings()?;
            if rings.is_empty() {
                Polygon::empty().into()
            } else {
                let exterior = rings.remove(0);
                Polygon::new(exterior, rings).into()
            }
        }
        GeometryType::MultiLineString => MultiLineString(rings()?).into(),
        GeometryType::MultiPolygon | GeometryType::GeometryCollection => {
            let mut children = Vec::new();
            if let Some(parts) = g.parts() {
                for child in parts {
                    children.push(read_fgb_geometry(
                        child,
                        if ty == GeometryType::MultiPolygon {
                            GeometryType::Polygon
                        } else {
                            GeometryType::Unknown
                        },
                        depth + 1,
                    )?);
                }
            }
            if ty == GeometryType::MultiPolygon {
                MultiPolygon(
                    children
                        .into_iter()
                        .map(|g| {
                            if let Geometry::Polygon(p) = g {
                                Ok(p)
                            } else {
                                bail!("invalid FlatGeobuf multipolygon child")
                            }
                        })
                        .collect::<Result<_>>()?,
                )
                .into()
            } else {
                Geometry::GeometryCollection(GeometryCollection(children))
            }
        }
        _ => bail!("unsupported FlatGeobuf geometry type {ty:?}"),
    })
}
