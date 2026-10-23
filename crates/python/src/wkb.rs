//! Strict WKB reading and planar normalisation of layer input.
//!
//! Accepts ISO and extended WKB in either byte order and writes 2D little-endian
//! ISO WKB. Empty leaves are dropped while their depth-first positions are kept.
use std::collections::BTreeSet;

/// Deepest collection nesting accepted from untrusted input.
const MAX_DEPTH: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Family {
    Point,
    Line,
    Polygon,
}

impl Family {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Point => "point",
            Self::Line => "line",
            Self::Polygon => "polygon",
        }
    }

    fn of(kind: u32) -> Option<Self> {
        match kind {
            1 | 4 => Some(Self::Point),
            2 | 5 => Some(Self::Line),
            3 | 6 => Some(Self::Polygon),
            _ => None,
        }
    }
}

type Coord = [f64; 2];

#[derive(Debug)]
enum Node {
    Point(Coord),
    Line(Vec<Coord>),
    Polygon(Vec<Vec<Coord>>),
    /// Multi* (4..=6) or GeometryCollection (7) with its children.
    Collection(u32, Vec<Node>),
}

impl Node {
    fn kind(&self) -> u32 {
        match self {
            Self::Point(_) => 1,
            Self::Line(_) => 2,
            Self::Polygon(_) => 3,
            Self::Collection(kind, _) => *kind,
        }
    }

    fn leaf_is_empty(&self) -> bool {
        match self {
            Self::Point([x, y]) => x.is_nan() && y.is_nan(),
            Self::Line(points) => points.is_empty(),
            Self::Polygon(rings) => rings.first().is_none_or(Vec::is_empty),
            Self::Collection(..) => false,
        }
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
    extra_dimensions: bool,
}

impl Reader<'_> {
    fn take<const N: usize>(&mut self) -> Result<[u8; N], String> {
        let chunk = self
            .bytes
            .get(self.offset..)
            .and_then(|rest| rest.first_chunk::<N>())
            .ok_or("unexpected end of data")?;
        self.offset += N;
        Ok(*chunk)
    }

    fn u32(&mut self, little: bool) -> Result<u32, String> {
        let bytes = self.take::<4>()?;
        Ok(if little {
            u32::from_le_bytes(bytes)
        } else {
            u32::from_be_bytes(bytes)
        })
    }

    fn f64(&mut self, little: bool) -> Result<f64, String> {
        let bytes = self.take::<8>()?;
        Ok(if little {
            f64::from_le_bytes(bytes)
        } else {
            f64::from_be_bytes(bytes)
        })
    }

    /// Read an element count, rejecting counts the remaining bytes cannot hold.
    fn count(&mut self, little: bool, minimum: usize) -> Result<usize, String> {
        let count = usize::try_from(self.u32(little)?).map_err(|e| e.to_string())?;
        let remaining = self.bytes.len() - self.offset;
        if count
            .checked_mul(minimum)
            .is_none_or(|size| size > remaining)
        {
            return Err("unexpected end of data".into());
        }
        Ok(count)
    }

    fn coord(&mut self, little: bool, dimensions: usize) -> Result<Coord, String> {
        let coord = [self.f64(little)?, self.f64(little)?];
        for _ in 2..dimensions {
            self.f64(little)?;
        }
        Ok(coord)
    }

    fn points(&mut self, little: bool, dimensions: usize) -> Result<Vec<Coord>, String> {
        let count = self.count(little, 8 * dimensions)?;
        (0..count).map(|_| self.coord(little, dimensions)).collect()
    }

    fn geometry(&mut self, depth: usize, parent: Option<u32>) -> Result<Node, String> {
        if depth > MAX_DEPTH {
            return Err("geometry nesting is too deep".into());
        }
        let little = match self.take::<1>()? {
            [0] => false,
            [1] => true,
            [other] => return Err(format!("invalid byte order {other}")),
        };
        let code = self.u32(little)?;
        // Extended WKB flags, then ISO thousands for Z, M and ZM.
        let mut z = code & 0x8000_0000 != 0;
        let mut m = code & 0x4000_0000 != 0;
        let srid = code & 0x2000_0000 != 0;
        let base = code & 0x1FFF_FFFF;
        let kind = base % 1000;
        match base / 1000 {
            0 => {}
            1 => z = true,
            2 => m = true,
            3 => (z, m) = (true, true),
            _ => return Err(format!("unsupported WKB geometry type {code}")),
        }
        if !(1..=7).contains(&kind) {
            return Err(format!("unsupported WKB geometry type {code}"));
        }
        if let Some(parent @ 4..=6) = parent
            && kind != parent - 3
        {
            return Err(format!(
                "{} cannot contain {}",
                type_name(parent),
                type_name(kind)
            ));
        }
        if srid {
            self.u32(little)?;
        }
        let dimensions = 2 + usize::from(z) + usize::from(m);
        self.extra_dimensions |= dimensions > 2;
        Ok(match kind {
            1 => Node::Point(self.coord(little, dimensions)?),
            2 => Node::Line(self.points(little, dimensions)?),
            3 => {
                let count = self.count(little, 4)?;
                Node::Polygon(
                    (0..count)
                        .map(|_| self.points(little, dimensions))
                        .collect::<Result<_, _>>()?,
                )
            }
            _ => {
                let count = self.count(little, 9)?;
                Node::Collection(
                    kind,
                    (0..count)
                        .map(|_| self.geometry(depth + 1, Some(kind)))
                        .collect::<Result<_, _>>()?,
                )
            }
        })
    }
}

fn type_name(kind: u32) -> &'static str {
    match kind {
        1 => "Point",
        2 => "LineString",
        3 => "Polygon",
        4 => "MultiPoint",
        5 => "MultiLineString",
        6 => "MultiPolygon",
        _ => "GeometryCollection",
    }
}

/// Parse one complete WKB value, reporting whether it had Z or M ordinates.
fn read(bytes: &[u8]) -> Result<(Node, bool), String> {
    let mut reader = Reader {
        bytes,
        offset: 0,
        extra_dimensions: false,
    };
    let node = reader.geometry(0, None)?;
    if reader.offset != bytes.len() {
        return Err("unexpected trailing bytes".into());
    }
    Ok((node, reader.extra_dimensions))
}

fn header(kind: u32, out: &mut Vec<u8>) {
    out.push(1);
    out.extend_from_slice(&kind.to_le_bytes());
}

fn count(value: usize, out: &mut Vec<u8>) {
    // Counts were read from u32 fields, so they always fit.
    out.extend_from_slice(&u32::try_from(value).unwrap_or(u32::MAX).to_le_bytes());
}

fn coords(points: &[Coord], out: &mut Vec<u8>) {
    count(points.len(), out);
    for [x, y] in points {
        out.extend_from_slice(&x.to_le_bytes());
        out.extend_from_slice(&y.to_le_bytes());
    }
}

fn write_leaf(node: &Node, out: &mut Vec<u8>) {
    header(node.kind(), out);
    match node {
        Node::Point([x, y]) => {
            out.extend_from_slice(&x.to_le_bytes());
            out.extend_from_slice(&y.to_le_bytes());
        }
        Node::Line(points) => coords(points, out),
        Node::Polygon(rings) => {
            count(rings.len(), out);
            for ring in rings {
                coords(ring, out);
            }
        }
        Node::Collection(..) => {}
    }
}

/// Write the full XY snapshot, including empty parts.
fn write(node: &Node, out: &mut Vec<u8>) {
    if let Node::Collection(kind, children) = node {
        header(*kind, out);
        count(children.len(), out);
        for child in children {
            write(child, out);
        }
    } else {
        write_leaf(node, out);
    }
}

#[derive(Debug, Default)]
struct Cleaner {
    families: BTreeSet<Family>,
    hints: BTreeSet<Family>,
    positions: Vec<usize>,
    position: usize,
    skipped: usize,
}

impl Cleaner {
    /// Write retained parts, returning whether anything nonempty was written.
    fn clean(&mut self, node: &Node, out: &mut Vec<u8>) -> bool {
        let family = Family::of(node.kind());
        if let Node::Collection(kind, children) = node {
            let start = out.len();
            header(*kind, out);
            count(0, out);
            let mut retained = 0_u32;
            for child in children {
                retained += u32::from(self.clean(child, out));
            }
            if retained == 0 {
                out.truncate(start);
                self.hints.extend(family);
                return false;
            }
            out[start + 5..start + 9].copy_from_slice(&retained.to_le_bytes());
            return true;
        }
        let original = self.position;
        self.position += 1;
        if node.leaf_is_empty() {
            self.hints.extend(family);
            self.skipped += 1;
            return false;
        }
        self.families.extend(family);
        self.positions.push(original);
        write_leaf(node, out);
        true
    }
}

/// A retained feature. `None` bytes reuse the input or the snapshot.
#[derive(Debug)]
pub(crate) struct Feature {
    pub index: usize,
    pub fid: i64,
    pub snapshot: Option<Vec<u8>>,
    pub clean: Option<Vec<u8>>,
}

#[derive(Debug, Default)]
pub(crate) struct Prepared {
    pub features: Vec<Feature>,
    /// Original part positions, only for features that lost empty parts.
    pub parts: Vec<(i64, Vec<usize>)>,
    pub warnings: Vec<String>,
    pub families: Vec<&'static str>,
    pub hints: Vec<&'static str>,
}

/// Normalise one layer's WKB items, mirroring the shapely input rules.
pub(crate) fn prepare(
    name: &str,
    fids: &[i64],
    items: &[Option<&[u8]>],
) -> Result<Prepared, String> {
    if fids.len() != items.len() {
        return Err("FIDs must match input length".into());
    }
    let mut prepared = Prepared::default();
    let mut families = BTreeSet::new();
    let mut hints = BTreeSet::new();
    for (index, (&fid, item)) in fids.iter().zip(items).enumerate() {
        let Some(bytes) = item else {
            prepared.warnings.push(format!(
                "layer {name}, input {index}, FID {fid}: skipped missing feature"
            ));
            continue;
        };
        let (node, extra) =
            read(bytes).map_err(|e| format!("layer {name}, FID {fid}: invalid WKB: {e}"))?;
        let mut cleaner = Cleaner::default();
        let mut clean = Vec::with_capacity(bytes.len());
        let retained = cleaner.clean(&node, &mut clean);
        hints.append(&mut cleaner.hints);
        if !retained {
            prepared.warnings.push(format!(
                "layer {name}, input {index}, FID {fid}: skipped empty feature"
            ));
            continue;
        }
        families.append(&mut cleaner.families);
        if extra {
            prepared.warnings.push(format!(
                "layer {name}, FID {fid}: discarded extra dimensions"
            ));
        }
        let mut snapshot = Vec::with_capacity(clean.len());
        write(&node, &mut snapshot);
        if cleaner.skipped > 0 {
            prepared.warnings.push(format!(
                "layer {name}, FID {fid}: skipped {} empty parts",
                cleaner.skipped
            ));
            prepared.parts.push((fid, cleaner.positions));
        }
        let clean = (clean != snapshot).then_some(clean);
        let snapshot = (snapshot != **bytes).then_some(snapshot);
        prepared.features.push(Feature {
            index,
            fid,
            snapshot,
            clean,
        });
    }
    prepared.families = families.into_iter().map(Family::name).collect();
    prepared.hints = hints.into_iter().map(Family::name).collect();
    Ok(prepared)
}
