//! Bounded ISO WKB and GeoPackageBinary codecs for the seven basic types.
use anyhow::{Result, bail, ensure};
use geo::{
    Coord, Geometry, GeometryCollection, LineString, MultiLineString, MultiPoint, MultiPolygon,
    Point, Polygon,
};

#[derive(Debug)]
pub(crate) struct Decoded {
    pub geometry: Geometry,
    pub zm: bool,
}
struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
    zm: bool,
}
impl Reader<'_> {
    fn take<const N: usize>(&mut self) -> Result<[u8; N]> {
        let end = self
            .pos
            .checked_add(N)
            .ok_or_else(|| anyhow::anyhow!("WKB offset overflow"))?;
        let bytes = self
            .data
            .get(self.pos..end)
            .ok_or_else(|| anyhow::anyhow!("truncated WKB"))?;
        self.pos = end;
        Ok(bytes.try_into()?)
    }
    fn integer(&mut self, le: bool) -> Result<u32> {
        let b = self.take()?;
        Ok(if le {
            u32::from_le_bytes(b)
        } else {
            u32::from_be_bytes(b)
        })
    }
    fn double(&mut self, le: bool) -> Result<f64> {
        let b = self.take()?;
        Ok(if le {
            f64::from_le_bytes(b)
        } else {
            f64::from_be_bytes(b)
        })
    }
    fn count(&mut self, le: bool, min: usize) -> Result<usize> {
        let n = self.integer(le)? as usize;
        ensure!(
            n <= self.data.len().saturating_sub(self.pos) / min,
            "WKB count exceeds remaining data"
        );
        Ok(n)
    }
    fn coordinate(&mut self, le: bool, dim: usize) -> Result<Coord> {
        let x = self.double(le)?;
        let y = self.double(le)?;
        for _ in 2..dim {
            self.double(le)?;
        }
        Ok(Coord { x, y })
    }
    fn line(&mut self, le: bool, dim: usize) -> Result<LineString> {
        let n = self.count(le, dim * 8)?;
        let mut coords = Vec::with_capacity(n);
        for _ in 0..n {
            let c = self.coordinate(le, dim)?;
            ensure!(
                c.x.is_finite() && c.y.is_finite(),
                "non-finite WKB coordinate"
            );
            coords.push(c);
        }
        Ok(LineString(coords))
    }
    fn geometry(&mut self, depth: usize) -> Result<Geometry> {
        ensure!(depth <= 64, "WKB nesting exceeds 64 levels");
        let le = match self.take::<1>()?[0] {
            0 => false,
            1 => true,
            _ => bail!("invalid WKB byte order"),
        };
        let code = self.integer(le)?;
        ensure!(code & 0x10000000 == 0, "unsupported WKB geometry flags");
        let ewkb_z = code & 0x80000000 != 0;
        let ewkb_m = code & 0x40000000 != 0;
        let base = code & 0x0fffffff;
        if code & 0x20000000 != 0 {
            self.integer(le)?;
        }
        let dim = 2
            + usize::from(ewkb_z || base / 1000 == 1 || base / 1000 == 3)
            + usize::from(ewkb_m || base / 1000 == 2 || base / 1000 == 3);
        self.zm |= dim > 2;
        ensure!(base / 1000 <= 3, "unsupported WKB type {code}");
        Ok(match base % 1000 {
            1 => {
                let c = self.coordinate(le, dim)?;
                ensure!(
                    (c.x.is_finite() && c.y.is_finite()) || (c.x.is_nan() && c.y.is_nan()),
                    "invalid WKB point"
                );
                Point(c).into()
            }
            2 => self.line(le, dim)?.into(),
            3 => {
                let n = self.count(le, 4)?;
                let mut rings = Vec::with_capacity(n);
                for _ in 0..n {
                    rings.push(self.line(le, dim)?);
                }
                if rings.is_empty() {
                    Polygon::empty().into()
                } else {
                    let exterior = rings.remove(0);
                    Polygon::new(exterior, rings).into()
                }
            }
            kind @ 4..=7 => {
                let n = self.count(le, 5)?;
                let mut children = Vec::with_capacity(n);
                for _ in 0..n {
                    children.push(self.geometry(depth + 1)?);
                }
                match kind {
                    4 => MultiPoint(
                        children
                            .into_iter()
                            .map(|g| {
                                if let Geometry::Point(p) = g {
                                    Ok(p)
                                } else {
                                    bail!("invalid WKB multipoint child")
                                }
                            })
                            .collect::<Result<_>>()?,
                    )
                    .into(),
                    5 => MultiLineString(
                        children
                            .into_iter()
                            .map(|g| {
                                if let Geometry::LineString(p) = g {
                                    Ok(p)
                                } else {
                                    bail!("invalid WKB multiline child")
                                }
                            })
                            .collect::<Result<_>>()?,
                    )
                    .into(),
                    6 => MultiPolygon(
                        children
                            .into_iter()
                            .map(|g| {
                                if let Geometry::Polygon(p) = g {
                                    Ok(p)
                                } else {
                                    bail!("invalid WKB multipolygon child")
                                }
                            })
                            .collect::<Result<_>>()?,
                    )
                    .into(),
                    _ => Geometry::GeometryCollection(GeometryCollection(children)),
                }
            }
            kind => bail!("unsupported WKB geometry type {kind}"),
        })
    }
}
pub(crate) fn decode(data: &[u8]) -> Result<Decoded> {
    let mut reader = Reader {
        data,
        pos: 0,
        zm: false,
    };
    let geometry = reader.geometry(0)?;
    ensure!(reader.pos == data.len(), "trailing WKB bytes");
    Ok(Decoded {
        geometry,
        zm: reader.zm,
    })
}
pub(crate) fn empty(g: &Geometry) -> bool {
    match g {
        Geometry::Point(p) => p.x().is_nan() && p.y().is_nan(),
        Geometry::LineString(l) => l.0.is_empty(),
        Geometry::Polygon(p) => p.exterior().0.is_empty(),
        Geometry::MultiPoint(p) => p.0.iter().all(|p| empty(&(*p).into())),
        Geometry::MultiLineString(p) => p.0.iter().all(|p| p.0.is_empty()),
        Geometry::MultiPolygon(p) => p.0.iter().all(|p| p.exterior().0.is_empty()),
        Geometry::GeometryCollection(c) => c.0.iter().all(empty),
        _ => false,
    }
}
pub(crate) fn normalize(g: &Geometry) -> Geometry {
    match g {
        Geometry::Line(l) => Geometry::LineString((*l).into()),
        Geometry::Rect(r) => r.to_polygon().into(),
        Geometry::Triangle(t) => t.to_polygon().into(),
        Geometry::GeometryCollection(c) => {
            Geometry::GeometryCollection(GeometryCollection(c.0.iter().map(normalize).collect()))
        }
        _ => g.clone(),
    }
}
/// Remove empty members before passing a geometry to the topology engine.
/// Empty points use NaN in WKB, which must never become phantom coordinates.
pub(crate) fn without_empty_members(g: Geometry) -> Geometry {
    match g {
        Geometry::MultiPoint(mut p) => {
            p.0.retain(|p| !empty(&(*p).into()));
            p.into()
        }
        Geometry::MultiLineString(mut p) => {
            p.0.retain(|l| !l.0.is_empty());
            p.into()
        }
        Geometry::MultiPolygon(mut p) => {
            p.0.retain(|p| !p.exterior().0.is_empty());
            p.into()
        }
        Geometry::GeometryCollection(c) => Geometry::GeometryCollection(GeometryCollection(
            c.0.into_iter()
                .map(without_empty_members)
                .filter(|g| !empty(g))
                .collect(),
        )),
        g => g,
    }
}
pub(crate) fn type_code(g: &Geometry) -> u32 {
    match g {
        Geometry::Point(_) => 1,
        Geometry::LineString(_) | Geometry::Line(_) => 2,
        Geometry::Polygon(_) | Geometry::Triangle(_) | Geometry::Rect(_) => 3,
        Geometry::MultiPoint(_) => 4,
        Geometry::MultiLineString(_) => 5,
        Geometry::MultiPolygon(_) => 6,
        Geometry::GeometryCollection(_) => 7,
    }
}
pub(crate) fn type_name(g: &Geometry) -> &'static str {
    [
        "GEOMETRY",
        "POINT",
        "LINESTRING",
        "POLYGON",
        "MULTIPOINT",
        "MULTILINESTRING",
        "MULTIPOLYGON",
        "GEOMETRYCOLLECTION",
    ][type_code(g) as usize]
}
fn uint(b: &mut Vec<u8>, n: usize) -> Result<()> {
    b.extend(u32::try_from(n)?.to_le_bytes());
    Ok(())
}
fn coord(b: &mut Vec<u8>, c: Coord) {
    b.extend(c.x.to_le_bytes());
    b.extend(c.y.to_le_bytes());
}
fn line(b: &mut Vec<u8>, l: &LineString) -> Result<()> {
    uint(b, l.0.len())?;
    for &c in &l.0 {
        coord(b, c);
    }
    Ok(())
}
fn encode_into(b: &mut Vec<u8>, g: &Geometry, depth: usize) -> Result<()> {
    ensure!(depth <= 64, "geometry nesting exceeds 64 levels");
    b.push(1);
    b.extend(type_code(g).to_le_bytes());
    match g {
        Geometry::Point(p) => coord(b, p.0),
        Geometry::LineString(l) => line(b, l)?,
        Geometry::Polygon(p) => {
            if p.exterior().0.is_empty() {
                uint(b, 0)?;
            } else {
                uint(b, 1 + p.interiors().len())?;
                line(b, p.exterior())?;
                for r in p.interiors() {
                    line(b, r)?;
                }
            }
        }
        Geometry::MultiPoint(p) => {
            uint(b, p.0.len())?;
            for p in &p.0 {
                encode_into(b, &(*p).into(), depth + 1)?;
            }
        }
        Geometry::MultiLineString(p) => {
            uint(b, p.0.len())?;
            for p in &p.0 {
                encode_into(b, &p.clone().into(), depth + 1)?;
            }
        }
        Geometry::MultiPolygon(p) => {
            uint(b, p.0.len())?;
            for p in &p.0 {
                encode_into(b, &p.clone().into(), depth + 1)?;
            }
        }
        Geometry::GeometryCollection(c) => {
            uint(b, c.0.len())?;
            for p in c {
                encode_into(b, p, depth + 1)?;
            }
        }
        _ => unreachable!("normalized geometry"),
    }
    Ok(())
}
pub(crate) fn encode(g: &Geometry) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    encode_into(&mut bytes, &normalize(g), 0)?;
    Ok(bytes)
}
pub(crate) fn decode_gpkg(data: &[u8], srs: i32) -> Result<Decoded> {
    ensure!(
        data.len() >= 8 && &data[..2] == b"GP",
        "invalid GeoPackageBinary magic"
    );
    ensure!(data[2] == 0, "unsupported GeoPackageBinary version");
    let flags = data[3];
    ensure!(
        flags & 0xe0 == 0,
        "extended or reserved GeoPackageBinary flags"
    );
    let le = flags & 1 != 0;
    let srid = if le {
        i32::from_le_bytes(data[4..8].try_into()?)
    } else {
        i32::from_be_bytes(data[4..8].try_into()?)
    };
    ensure!(srid == srs, "geometry CRS differs from layer CRS");
    let doubles = match (flags >> 1) & 7 {
        0 => 0,
        1 => 4,
        2 | 3 => 6,
        4 => 8,
        _ => bail!("invalid GeoPackage envelope code"),
    };
    let start = 8 + doubles * 8;
    ensure!(data.len() >= start, "truncated GeoPackage envelope");
    for bytes in data[8..start].as_chunks::<8>().0 {
        let v = if le {
            f64::from_le_bytes(*bytes)
        } else {
            f64::from_be_bytes(*bytes)
        };
        ensure!(
            v.is_finite() || flags & 16 != 0,
            "non-finite GeoPackage envelope"
        );
    }
    let decoded = decode(&data[start..])?;
    ensure!(
        empty(&decoded.geometry) == (flags & 16 != 0),
        "GeoPackage empty flag disagrees with geometry"
    );
    Ok(decoded)
}
pub(crate) fn encode_gpkg(g: &Geometry, srs: i32) -> Result<Vec<u8>> {
    let mut bytes = vec![b'G', b'P', 0, 1 | if empty(g) { 16 } else { 0 }];
    bytes.extend(srs.to_le_bytes());
    bytes.extend(encode(g)?);
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn big_endian_envelopes_zm_and_empty_headers() {
        for envelope in 0..=4 {
            let doubles = match envelope {
                0 => 0,
                1 => 4,
                2 | 3 => 6,
                _ => 8,
            };
            let mut b = vec![b'G', b'P', 0, envelope << 1];
            b.extend(4326i32.to_be_bytes());
            for _ in 0..doubles {
                b.extend(1f64.to_be_bytes());
            }
            b.push(0);
            b.extend(3001u32.to_be_bytes());
            for v in [5f64, 52., 100., 200.] {
                b.extend(v.to_be_bytes());
            }
            let decoded = decode_gpkg(&b, 4326).unwrap();
            assert!(decoded.zm);
            assert_eq!(decoded.geometry, Point::new(5., 52.).into());
            for end in 0..b.len() {
                assert!(decode_gpkg(&b[..end], 4326).is_err());
            }
        }
        let empty = Point::new(f64::NAN, f64::NAN).into();
        let b = encode_gpkg(&empty, 0).unwrap();
        assert!(self::empty(&decode_gpkg(&b, 0).unwrap().geometry));
        let mut bad = b;
        bad[3] &= !16;
        assert!(decode_gpkg(&bad, 0).is_err());
    }
    #[test]
    fn malicious_counts_child_types_and_excessive_nesting_are_errors() {
        let mut b = vec![1];
        b.extend(2u32.to_le_bytes());
        b.extend(u32::MAX.to_le_bytes());
        assert!(decode(&b).is_err());
        let mut b = vec![1];
        b.extend(4u32.to_le_bytes());
        b.extend(1u32.to_le_bytes());
        b.extend(encode(&LineString::new(vec![]).into()).unwrap());
        assert!(decode(&b).is_err());
        let mut b = Vec::new();
        for _ in 0..70 {
            b.push(1);
            b.extend(7u32.to_le_bytes());
            b.extend(1u32.to_le_bytes());
        }
        b.extend(encode(&Point::new(0., 0.).into()).unwrap());
        assert!(decode(&b).is_err());
        let b = encode(&Point::new(0., 0.).into()).unwrap();
        for end in 0..b.len() {
            assert!(decode(&b[..end]).is_err());
        }
    }
}
