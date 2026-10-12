//! Vector GeoPackage 1.4 metadata and feature I/O over the Rust SQLite engine.
use crate::{
    codec, crs,
    read::{RawFeature, RawLayer},
    write::{DatasetLayer, FieldType},
};
use anyhow::{Context, Result, bail, ensure};
use geo::BoundingRect;
use serde_json::Value as Json;
use std::{collections::BTreeSet, path::Path, sync::Arc};
use turso_core::{
    Connection, Database, Numeric, OpenFlags, OpenOptions, PlatformIO, SqliteDialect, Value,
};

pub(crate) struct Sqlite {
    connection: Arc<Connection>,
    _database: Arc<Database>,
}
impl Sqlite {
    pub(crate) fn open(path: &Path, readonly: bool) -> Result<Self> {
        let io = Arc::new(PlatformIO::new()?);
        let options = OpenOptions::new(Arc::new(SqliteDialect)).flags(if readonly {
            OpenFlags::ReadOnly
        } else {
            OpenFlags::Create
        });
        let db = Database::open(io, path.to_str().context("invalid database path")?, options)?;
        let connection = db.connect()?;
        Ok(Self {
            connection,
            _database: db,
        })
    }
    pub(crate) fn query(&self, sql: &str, params: &[Value]) -> Result<Vec<Vec<Value>>> {
        let mut stmt = self.connection.prepare(sql)?;
        for (i, value) in params.iter().enumerate() {
            stmt.bind_at((i + 1).try_into()?, value.clone())?;
        }
        Ok(stmt.run_collect_rows()?)
    }
    fn exec(&self, sql: &str, params: &[Value]) -> Result<()> {
        self.query(sql, params)?;
        Ok(())
    }
    fn close(&self) -> Result<()> {
        Ok(self.connection.close()?)
    }
}
fn ident(name: &str) -> Result<String> {
    ensure!(
        !name.is_empty() && !name.contains('\0'),
        "invalid SQL identifier"
    );
    Ok(format!("\"{}\"", name.replace('"', "\"\"")))
}
fn text(s: impl Into<String>) -> Value {
    Value::build_text(s.into())
}
fn integer(n: i64) -> Value {
    Value::Numeric(Numeric::Integer(n))
}
fn get_integer(v: &Value) -> Result<i64> {
    if let Value::Numeric(Numeric::Integer(n)) = v {
        Ok(*n)
    } else {
        bail!("expected integer GeoPackage metadata")
    }
}
fn get_text(v: &Value) -> Result<&str> {
    v.to_text().context("expected text GeoPackage metadata")
}
fn json(v: &Value) -> Json {
    match v {
        Value::Null => Json::Null,
        Value::Numeric(Numeric::Integer(n)) => (*n).into(),
        Value::Numeric(Numeric::Float(n)) => Json::from(f64::from(*n)),
        Value::Text(t) => t.as_str().into(),
        Value::Blob(b) => Json::Array(b.iter().map(|n| Json::from(*n)).collect()),
    }
}
pub(crate) fn read(path: &Path) -> Result<Vec<RawLayer>> {
    use std::io::Read;
    let mut header = [0u8; 100];
    std::fs::File::open(path)?.read_exact(&mut header)?;
    ensure!(
        &header[..16] == b"SQLite format 3\0",
        "invalid GeoPackage SQLite header"
    );
    ensure!(
        u32::from_be_bytes(header[68..72].try_into()?) == 0x47504b47,
        "invalid GeoPackage application ID"
    );
    let page_size = match u16::from_be_bytes(header[16..18].try_into()?) {
        1 => 65_536u64,
        n => u64::from(n),
    };
    let file_size = std::fs::metadata(path)?.len();
    let pages = u32::from_be_bytes(header[28..32].try_into()?);
    ensure!(
        page_size >= 512 && page_size.is_power_of_two(),
        "invalid GeoPackage SQLite page size"
    );
    ensure!(
        file_size >= page_size
            && file_size.is_multiple_of(page_size)
            && u64::from(pages) <= file_size / page_size,
        "truncated GeoPackage SQLite pages"
    );
    let db = Sqlite::open(path, true)?;
    // Turso's catalog loader stops at virtual tables whose modules are absent.
    // Register ordinary B-trees through the engine's SQLite schema parser.
    // No spatial-index module is used, and the source remains read-only.
    let tables = db.query(
        "SELECT sql, rootpage FROM sqlite_schema WHERE type = 'table' AND rootpage > 0",
        &[],
    )?;
    for table in tables {
        use turso_core::dialect::Dialect;
        let parsed =
            SqliteDialect.parse_table_sql(get_text(&table[0])?, get_integer(&table[1])?)?;
        db.connection.with_schema_mut(|schema| {
            if schema.get_table(&parsed.name).is_none() {
                schema.add_btree_table(Arc::new(parsed))
            } else {
                Ok(())
            }
        })??;
    }
    let layers=db.query("SELECT c.table_name, g.column_name, g.srs_id, g.geometry_type_name FROM gpkg_contents c JOIN gpkg_geometry_columns g ON c.table_name = g.table_name WHERE c.data_type = 'features' ORDER BY c.table_name",&[])?;
    let mut output = Vec::new();
    for row in layers {
        let name = get_text(&row[0])?.to_string();
        let geom_column = get_text(&row[1])?;
        let srs_id = i32::try_from(get_integer(&row[2])?)?;
        ensure!(
            [
                "GEOMETRY",
                "POINT",
                "LINESTRING",
                "POLYGON",
                "MULTIPOINT",
                "MULTILINESTRING",
                "MULTIPOLYGON",
                "GEOMETRYCOLLECTION"
            ]
            .contains(&get_text(&row[3])?),
            "extended GeoPackage geometry types are unsupported"
        );
        let srs_rows=db.query("SELECT organization, organization_coordsys_id, definition FROM gpkg_spatial_ref_sys WHERE srs_id = ?",&[integer(srs_id.into())])?;
        let definition = srs_rows
            .first()
            .context("missing GeoPackage CRS metadata")?;
        let srs = if srs_id == 0 || srs_id == -1 {
            None
        } else {
            let wkt = get_text(&definition[2])?;
            if wkt != "undefined" {
                Some(crs::parse(wkt)?)
            } else if get_text(&definition[0])?.eq_ignore_ascii_case("EPSG") {
                Some(crs::parse(&format!(
                    "EPSG:{}",
                    get_integer(&definition[1])?
                ))?)
            } else {
                bail!("unsupported GeoPackage CRS definition")
            }
        };
        let columns = db.query(&format!("PRAGMA table_info({})", ident(&name)?), &[])?;
        let primary = columns
            .iter()
            .filter(|c| get_integer(&c[5]).is_ok_and(|n| n > 0))
            .collect::<Vec<_>>();
        ensure!(
            primary.len() == 1 && get_text(&primary[0][2])?.eq_ignore_ascii_case("INTEGER"),
            "GeoPackage feature table {name} must have one INTEGER primary key: {columns:?}"
        );
        let pk = get_text(&primary[0][1])?;
        let attributes = columns
            .iter()
            .map(|c| get_text(&c[1]))
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .filter(|c| *c != pk && *c != geom_column)
            .collect::<Vec<_>>();
        let mut select = vec![ident(pk)?, ident(geom_column)?];
        for c in &attributes {
            select.push(ident(c)?);
        }
        let rows = db.query(
            &format!(
                "SELECT {} FROM {} ORDER BY {}",
                select.join(","),
                ident(&name)?,
                ident(pk)?
            ),
            &[],
        )?;
        let mut features = Vec::new();
        for row in rows {
            let id = Some(get_integer(&row[0])?);
            let decoded = match &row[1] {
                Value::Null => None,
                Value::Blob(b) => Some(codec::decode_gpkg(b, srs_id)?),
                _ => bail!("GeoPackage geometry must be a blob or NULL"),
            };
            features.push(RawFeature {
                id,
                geometry: decoded.as_ref().map(|d| d.geometry.clone()),
                zm: decoded.is_some_and(|d| d.zm),
                properties: attributes
                    .iter()
                    .zip(&row[2..])
                    .map(|(n, v)| {
                        (
                            (*n).to_string(),
                            if columns.iter().any(|c| {
                                c[1].to_text() == Some(*n)
                                    && c[2]
                                        .to_text()
                                        .is_some_and(|t| t.eq_ignore_ascii_case("BOOLEAN"))
                            }) {
                                match v {
                                    Value::Numeric(Numeric::Integer(i)) => Json::Bool(*i != 0),
                                    _ => json(v),
                                }
                            } else {
                                json(v)
                            },
                        )
                    })
                    .collect(),
            });
        }
        output.push(RawLayer {
            name,
            srs,
            features,
        });
    }
    db.close()?;
    Ok(output)
}
const METADATA: &[&str] = &[
    "CREATE TABLE gpkg_spatial_ref_sys (srs_name TEXT NOT NULL, srs_id INTEGER NOT NULL PRIMARY KEY, organization TEXT NOT NULL, organization_coordsys_id INTEGER NOT NULL, definition TEXT NOT NULL, description TEXT)",
    "CREATE TABLE gpkg_contents (table_name TEXT NOT NULL PRIMARY KEY, data_type TEXT NOT NULL, identifier TEXT UNIQUE, description TEXT DEFAULT '', last_change DATETIME NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')), min_x DOUBLE, min_y DOUBLE, max_x DOUBLE, max_y DOUBLE, srs_id INTEGER, FOREIGN KEY (srs_id) REFERENCES gpkg_spatial_ref_sys(srs_id))",
    "CREATE TABLE gpkg_geometry_columns (table_name TEXT NOT NULL, column_name TEXT NOT NULL, geometry_type_name TEXT NOT NULL, srs_id INTEGER NOT NULL, z TINYINT NOT NULL, m TINYINT NOT NULL, PRIMARY KEY (table_name,column_name), UNIQUE (table_name), FOREIGN KEY (table_name) REFERENCES gpkg_contents(table_name), FOREIGN KEY (srs_id) REFERENCES gpkg_spatial_ref_sys(srs_id))",
];
fn sql_value(value: Option<&Json>) -> Result<Value> {
    Ok(match value {
        None | Some(Json::Null) => Value::Null,
        Some(Json::Bool(b)) => integer(i64::from(*b)),
        Some(Json::Number(n)) => {
            if let Some(i) = n.as_i64() {
                integer(i)
            } else if let Some(f) = n.as_f64() {
                Value::from_f64(f)
            } else {
                bail!("unsupported numeric value")
            }
        }
        Some(Json::String(s)) => text(s),
        Some(v) => text(v.to_string()),
    })
}
pub(crate) fn write(path: &Path, layers: &[DatasetLayer]) -> Result<()> {
    let sources = layers
        .iter()
        .map(|l| l as &dyn crate::write::RecordSource)
        .collect::<Vec<_>>();
    write_sources(path, &sources)
}
pub(crate) fn write_sources(path: &Path, layers: &[&dyn crate::write::RecordSource]) -> Result<()> {
    let db = Sqlite::open(path, false)?;
    db.exec("PRAGMA application_id = 1196444487", &[])?;
    db.exec("PRAGMA user_version = 10400", &[])?;
    db.exec("BEGIN IMMEDIATE", &[])?;
    for sql in METADATA {
        db.exec(sql, &[])?;
    }
    for (id, name, org, wkt) in [
        (-1, "Undefined Cartesian", "NONE", "undefined"),
        (0, "Undefined Geographic", "NONE", "undefined"),
        (4326, "WGS 84", "EPSG", ""),
    ] {
        let wkt = if id == 4326 {
            crs::parse("EPSG:4326")?.to_wkt()?
        } else {
            wkt.into()
        };
        db.exec(
            "INSERT INTO gpkg_spatial_ref_sys VALUES (?, ?, ?, ?, ?, NULL)",
            &[text(name), integer(id), text(org), integer(id), text(wkt)],
        )?;
    }
    let mut srids = BTreeSet::from([-1, 0, 4326]);
    let mut next_custom = 100_000_i32;
    let mut custom = Vec::<(crs::Crs, i32)>::new();
    for source in layers {
        let layer = source.metadata();
        let srs_id = match &layer.srs {
            None => 0,
            Some(s) => {
                if let Some(epsg) = s.epsg() {
                    i32::try_from(epsg)?
                } else if let Some((_, id)) = custom.iter().find(|(c, _)| c == s) {
                    *id
                } else {
                    next_custom += 1;
                    custom.push((s.clone(), next_custom));
                    next_custom
                }
            }
        };
        if srids.insert(srs_id) {
            let s = layer.srs.as_ref().context("missing custom CRS")?;
            db.exec(
                "INSERT INTO gpkg_spatial_ref_sys VALUES (?, ?, ?, ?, ?, NULL)",
                &[
                    text(s.name().unwrap_or("Custom CRS".into())),
                    integer(srs_id.into()),
                    text(if s.epsg().is_some() { "EPSG" } else { "NONE" }),
                    integer(s.epsg().unwrap_or(srs_id as u32).into()),
                    text(s.to_wkt()?),
                ],
            )?;
        }
        let mut columns = vec!["fid INTEGER PRIMARY KEY".into(), "geom BLOB".into()];
        for field in &layer.fields {
            ensure!(
                !["fid", "geom"]
                    .iter()
                    .any(|s| s.eq_ignore_ascii_case(&field.name)),
                "attribute name conflicts with GeoPackage identity or geometry"
            );
            columns.push(format!(
                "{} {}",
                ident(&field.name)?,
                match field.kind {
                    FieldType::Integer => "INTEGER",
                    FieldType::Real => "REAL",
                    FieldType::Boolean => "BOOLEAN",
                    FieldType::Text => "TEXT",
                }
            ));
        }
        db.exec(
            &format!(
                "CREATE TABLE {} ({})",
                ident(&layer.name)?,
                columns.join(",")
            ),
            &[],
        )?;
        let mut bounds = None::<[f64; 4]>;
        for record in source.records() {
            let record = record?;
            if let Some(g) = &record.geometry
                && !codec::empty(g)
                && let Some(b) = g.bounding_rect()
            {
                let e = [b.min().x, b.min().y, b.max().x, b.max().y];
                bounds = Some(bounds.map_or(e, |b| {
                    [
                        b[0].min(e[0]),
                        b[1].min(e[1]),
                        b[2].max(e[2]),
                        b[3].max(e[3]),
                    ]
                }));
            }
        }
        let mut metadata = vec![text(&layer.name), text(&layer.name), integer(srs_id.into())];
        for n in bounds.map(|b| b.map(Some)).unwrap_or([None; 4]) {
            metadata.push(n.map(Value::from_f64).unwrap_or(Value::Null));
        }
        db.exec("INSERT INTO gpkg_contents (table_name,data_type,identifier,srs_id,min_x,min_y,max_x,max_y) VALUES (?, 'features', ?, ?, ?, ?, ?, ?)",&metadata)?;
        let ty = layer.geometry_type.as_deref().unwrap_or("GEOMETRY");
        db.exec(
            "INSERT INTO gpkg_geometry_columns VALUES (?, 'geom', ?, ?, 0, 0)",
            &[text(&layer.name), text(ty), integer(srs_id.into())],
        )?;
        let insert = format!(
            "INSERT INTO {} VALUES ({})",
            ident(&layer.name)?,
            vec!["?"; 2 + layer.fields.len()].join(",")
        );
        for (ordinal, r) in source.records().enumerate() {
            let r = r?;
            let mut params = vec![
                integer(r.id.unwrap_or(i64::try_from(ordinal)?)),
                match &r.geometry {
                    None => Value::Null,
                    Some(g) => Value::from_slice(&codec::encode_gpkg(g, srs_id)?)?,
                },
            ];
            for field in &layer.fields {
                params.push(sql_value(r.properties.get(&field.name))?);
            }
            db.exec(&insert, &params)?;
        }
    }
    db.exec("COMMIT", &[])?;
    db.close()?;
    drop(db);
    // Closing checkpoints the WAL. SQLite's portable rollback mode header makes
    // the completed file standalone; no sidecars travel with the dataset.
    use std::io::{Seek, Write};
    let mut file = std::fs::OpenOptions::new().write(true).open(path)?;
    file.seek(std::io::SeekFrom::Start(18))?;
    file.write_all(&[1, 1])?;
    file.sync_all()?;
    Ok(())
}
