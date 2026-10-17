use anyhow::{Result, bail};
use geo::{CoordsIter, Geometry, MapCoordsInPlace};
use std::{cell::RefCell, collections::HashMap, rc::Rc};

/// Transformations kept per thread, bounded so custom definitions cannot accumulate.
const CACHED_TRANSFORMS: usize = 32;
thread_local! {
    static TRANSFORMS: RefCell<HashMap<(String, String), Rc<Transform>>> =
        RefCell::new(HashMap::new());
}
/// Reuse the transformation between two CRS definitions on the current thread,
/// avoiding repeated parsing and operation setup for frequent display requests.
///
/// # Errors
/// Returns an error for invalid definitions or unsupported horizontal operations.
pub fn transform_between(from: &str, to: &str) -> Result<Rc<Transform>> {
    let key = (from.to_owned(), to.to_owned());
    if let Some(cached) = TRANSFORMS.with_borrow(|cache| cache.get(&key).cloned()) {
        return Ok(cached);
    }
    let operation = Rc::new(Transform::new(&parse(from)?, &parse(to)?)?);
    TRANSFORMS.with_borrow_mut(|cache| {
        if cache.len() >= CACHED_TRANSFORMS {
            cache.clear();
        }
        cache.insert(key, Rc::clone(&operation));
    });
    Ok(operation)
}

#[derive(Debug, Clone)]
enum Backend {
    Native(proj_core::CrsDef),
    #[cfg(feature = "gdal-system")]
    Fallback(gdal::spatial_ref::SpatialRef),
}
/// Repository-owned CRS with conventional longitude/easting first coordinates.
#[derive(Debug, Clone)]
pub struct Crs {
    definition: String,
    backend: Backend,
}
/// `semantically_equivalent` never equates two datums whose path to WGS84 is
/// unknown, so a custom CRS without a datum shift would differ from itself.
/// Compare such CRSs by ellipsoid, unit and projection parameters instead.
fn same_without_datum_shift(a: &proj_core::CrsDef, b: &proj_core::CrsDef) -> bool {
    use proj_core::{CrsDef, Datum, DatumToWgs84, GeographicCrsDef, ProjectedCrsDef};
    fn shiftless(def: &CrsDef) -> Option<CrsDef> {
        let datum = def.datum();
        if def.epsg() != 0
            || datum.epsg() != 0
            || !matches!(datum.to_wgs84(), DatumToWgs84::Unknown)
        {
            return None;
        }
        let datum = Datum::new(datum.ellipsoid(), DatumToWgs84::Identity).ok()?;
        match def {
            CrsDef::Geographic(_) => Some(CrsDef::Geographic(GeographicCrsDef::new(0, datum, ""))),
            CrsDef::Projected(p) => Some(CrsDef::Projected(ProjectedCrsDef::new(
                0,
                datum,
                p.method(),
                p.linear_unit(),
                "",
            ))),
            // A compound CRS keeps its vertical part, whose datum is always coded.
            CrsDef::Compound(_) => None,
        }
    }
    matches!((shiftless(a), shiftless(b)), (Some(a), Some(b)) if a.semantically_equivalent(&b))
}
impl PartialEq for Crs {
    fn eq(&self, other: &Self) -> bool {
        match (&self.backend, &other.backend) {
            (Backend::Native(a), Backend::Native(b)) => {
                a.semantically_equivalent(b) || same_without_datum_shift(a, b)
            }
            #[cfg(feature = "gdal-system")]
            _ => {
                if let (Ok(a), Ok(b)) = (self.to_gdal(), other.to_gdal()) {
                    return a == b;
                }
                self.definition == other.definition
            }
        }
    }
}
impl Crs {
    pub fn is_geographic(&self) -> bool {
        match &self.backend {
            Backend::Native(s) => s.is_geographic(),
            #[cfg(feature = "gdal-system")]
            Backend::Fallback(s) => s.is_geographic(),
        }
    }
    pub fn is_projected(&self) -> bool {
        match &self.backend {
            Backend::Native(s) => s.is_projected(),
            #[cfg(feature = "gdal-system")]
            Backend::Fallback(s) => s.is_projected(),
        }
    }
    pub fn linear_units(&self) -> f64 {
        match &self.backend {
            Backend::Native(s) => s.as_projected().map_or(1., |p| p.linear_unit_to_meter()),
            #[cfg(feature = "gdal-system")]
            Backend::Fallback(s) => s.linear_units(),
        }
    }
    pub fn epsg(&self) -> Option<u32> {
        match &self.backend {
            Backend::Native(s) => (s.epsg() != 0).then_some(s.epsg()),
            #[cfg(feature = "gdal-system")]
            Backend::Fallback(s) => s.auth_code().ok().and_then(|c| c.try_into().ok()),
        }
    }
    /// Serialize the CRS as WKT, preserving custom definitions.
    /// # Errors
    /// Returns an error when the CRS cannot be represented as WKT.
    pub fn to_wkt(&self) -> Result<String> {
        match &self.backend {
            Backend::Native(s) => Ok(proj_wkt::to_wkt(s)?),
            #[cfg(feature = "gdal-system")]
            Backend::Fallback(s) => Ok(s.to_wkt()?),
        }
    }
    /// # Errors
    /// Returns an error when the CRS cannot be represented as PROJJSON.
    pub fn to_projjson(&self) -> Result<String> {
        match &self.backend {
            Backend::Native(s) => Ok(proj_wkt::to_projjson(s)?),
            #[cfg(feature = "gdal-system")]
            Backend::Fallback(s) => Ok(s.to_projjson()?),
        }
    }
    /// # Errors
    /// Returns an error for a custom CRS without an authority identifier.
    pub fn authority(&self) -> Result<String> {
        self.epsg()
            .map(|c| format!("EPSG:{c}"))
            .ok_or_else(|| anyhow::anyhow!("custom CRS has no authority"))
    }
    pub fn name(&self) -> Option<String> {
        match &self.backend {
            Backend::Native(s) => Some(s.name().to_string()),
            #[cfg(feature = "gdal-system")]
            Backend::Fallback(s) => s.name(),
        }
    }
    pub fn angular_units_name(&self) -> Option<String> {
        self.is_geographic().then(|| "degree".into())
    }
    pub fn linear_units_name(&self) -> Option<String> {
        Some(match self.linear_units() {
            1. => "metre".into(),
            0.3048 => "foot".into(),
            n => format!("unit ({n} metres)"),
        })
    }
    #[cfg(feature = "gdal-system")]
    pub(crate) fn to_gdal(&self) -> Result<gdal::spatial_ref::SpatialRef> {
        crate::gdal_runtime::initialize()?;
        use gdal::spatial_ref::{AxisMappingStrategy, SpatialRef};
        let mut s = SpatialRef::from_definition(&self.definition)?;
        s.set_axis_mapping_strategy(AxisMappingStrategy::TraditionalGisOrder);
        Ok(s)
    }
}
/// Parse an EPSG, WKT, PROJ or PROJJSON definition.
/// # Errors
/// Returns an error for invalid or unsupported CRS definitions.
pub fn parse(definition: &str) -> Result<Crs> {
    let definition = definition.trim();
    let definition = if matches!(
        definition.to_ascii_uppercase().as_str(),
        "URN:OGC:DEF:CRS:OGC:1.3:CRS84" | "URN:OGC:DEF:CRS:OGC::CRS84"
    ) {
        "OGC:CRS84"
    } else {
        definition
    };
    match proj_wkt::parse_crs(definition) {
        Ok(native) => Ok(Crs {
            definition: definition.into(),
            backend: Backend::Native(native),
        }),
        Err(error) => {
            #[cfg(feature = "gdal-system")]
            {
                crate::gdal_runtime::initialize()?;
                use gdal::spatial_ref::{AxisMappingStrategy, SpatialRef};
                if let Ok(mut s) = SpatialRef::from_definition(definition) {
                    s.set_axis_mapping_strategy(AxisMappingStrategy::TraditionalGisOrder);
                    return Ok(Crs {
                        definition: definition.into(),
                        backend: Backend::Fallback(s),
                    });
                }
            }
            Err(error.into())
        }
    }
}
#[derive(Debug)]
enum Operation {
    Native(Box<proj_core::Transform>),
    #[cfg(feature = "gdal-system")]
    Fallback(gdal::spatial_ref::CoordTransform),
}
/// A horizontal XY transformation. Native operations always take precedence.
#[derive(Debug)]
pub struct Transform {
    operation: Operation,
}
impl Transform {
    /// # Errors
    /// Returns an error when no supported horizontal operation connects the CRSs.
    pub fn new(from: &Crs, to: &Crs) -> Result<Self> {
        match proj_wkt::transform_from_crs_strings_horizontal(&from.definition, &to.definition) {
            Ok(native) => Ok(Self {
                operation: Operation::Native(Box::new(native)),
            }),
            Err(_error) => {
                #[cfg(feature = "gdal-system")]
                {
                    let (from, to) = (from.to_gdal()?, to.to_gdal()?);
                    let operation = gdal::spatial_ref::CoordTransform::new(&from, &to)?;
                    Ok(Self {
                        operation: Operation::Fallback(operation),
                    })
                }
                #[cfg(not(feature = "gdal-system"))]
                Err(_error.into())
            }
        }
    }
    /// # Errors
    /// Returns an error for mismatched arrays, invalid coordinates or transformation failures.
    pub fn transform_coords(&self, x: &mut [f64], y: &mut [f64], _z: &mut [f64]) -> Result<()> {
        if x.len() != y.len() {
            bail!("coordinate array lengths differ");
        }
        ensure_finite(x, y, "non-finite input coordinates")?;
        match &self.operation {
            Operation::Native(t) => {
                let result = x
                    .iter()
                    .zip(y.iter())
                    .map(|(&x, &y)| t.convert((x, y)))
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                for ((x, y), (tx, ty)) in x.iter_mut().zip(y.iter_mut()).zip(result) {
                    *x = tx;
                    *y = ty;
                }
            }
            #[cfg(feature = "gdal-system")]
            Operation::Fallback(t) => t.transform_coords(x, y, &mut [])?,
        }
        ensure_finite(
            x,
            y,
            "coordinate transformation produced non-finite coordinates",
        )
    }
    /// # Errors
    /// Returns an error for invalid bounds, excessive densification or transformation failures.
    pub fn transform_bounds(&self, bounds: &[f64; 4], densify: i32) -> Result<[f64; 4]> {
        match &self.operation {
            Operation::Native(t) => {
                let b = t.transform_bounds(
                    proj_core::Bounds::new(bounds[0], bounds[1], bounds[2], bounds[3]),
                    densify.try_into()?,
                )?;
                Ok([b.min_x, b.min_y, b.max_x, b.max_y])
            }
            #[cfg(feature = "gdal-system")]
            Operation::Fallback(t) => Ok(t.transform_bounds(bounds, densify)?),
        }
    }
}
fn ensure_finite(x: &[f64], y: &[f64], message: &str) -> Result<()> {
    if x.iter().chain(y).all(|v| v.is_finite()) {
        Ok(())
    } else {
        bail!("{message}")
    }
}
/// Transform every original coordinate, preserving geometry structure.
/// # Errors
/// Returns an error for invalid coordinates or failed transformations.
pub fn transform(geometry: &Geometry, operation: &Transform) -> Result<Geometry> {
    let (mut x, mut y): (Vec<_>, Vec<_>) = geometry.coords_iter().map(|c| (c.x, c.y)).unzip();
    operation.transform_coords(&mut x, &mut y, &mut [])?;
    let mut transformed = geometry.clone();
    let coordinates = std::cell::RefCell::new(x.into_iter().zip(y));
    let coordinates = &coordinates;
    transformed.map_coords_in_place(|_| {
        let (x, y) = coordinates
            .borrow_mut()
            .next()
            .expect("same coordinate traversal");
        geo::Coord { x, y }
    });
    Ok(transformed)
}
