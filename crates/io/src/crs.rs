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

/// Repository-owned CRS with conventional longitude/easting first coordinates.
#[derive(Debug, Clone)]
pub struct Crs {
    definition: String,
    native: Option<proj_core::CrsDef>,
    #[cfg(feature = "gdal-system")]
    fallback: Option<gdal::spatial_ref::SpatialRef>,
}
impl PartialEq for Crs {
    fn eq(&self, other: &Self) -> bool {
        match (&self.native, &other.native) {
            (Some(a), Some(b)) => a.semantically_equivalent(b),
            _ => {
                #[cfg(feature = "gdal-system")]
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
        if let Some(s) = &self.native {
            return s.is_geographic();
        }
        #[cfg(feature = "gdal-system")]
        if let Some(s) = &self.fallback {
            return s.is_geographic();
        }
        false
    }
    pub fn is_projected(&self) -> bool {
        if let Some(s) = &self.native {
            return s.is_projected();
        }
        #[cfg(feature = "gdal-system")]
        if let Some(s) = &self.fallback {
            return s.is_projected();
        }
        false
    }
    pub fn linear_units(&self) -> f64 {
        if let Some(s) = &self.native {
            return s.as_projected().map_or(1., |p| p.linear_unit_to_meter());
        }
        #[cfg(feature = "gdal-system")]
        if let Some(s) = &self.fallback {
            return s.linear_units();
        }
        1.
    }
    pub fn epsg(&self) -> Option<u32> {
        if let Some(s) = &self.native {
            return (s.epsg() != 0).then_some(s.epsg());
        }
        #[cfg(feature = "gdal-system")]
        if let Some(s) = &self.fallback {
            return s.auth_code().ok().and_then(|c| c.try_into().ok());
        }
        None
    }
    /// Serialize the CRS as WKT, preserving custom definitions.
    /// # Errors
    /// Returns an error when the CRS cannot be represented as WKT.
    pub fn to_wkt(&self) -> Result<String> {
        if let Some(s) = &self.native {
            return Ok(proj_wkt::to_wkt(s)?);
        }
        #[cfg(feature = "gdal-system")]
        if let Some(s) = &self.fallback {
            return Ok(s.to_wkt()?);
        }
        bail!("unsupported CRS")
    }
    /// # Errors
    /// Returns an error when the CRS cannot be represented as PROJJSON.
    pub fn to_projjson(&self) -> Result<String> {
        if let Some(s) = &self.native {
            return Ok(proj_wkt::to_projjson(s)?);
        }
        #[cfg(feature = "gdal-system")]
        if let Some(s) = &self.fallback {
            return Ok(s.to_projjson()?);
        }
        bail!("unsupported CRS")
    }
    /// # Errors
    /// Returns an error for a custom CRS without an authority identifier.
    pub fn authority(&self) -> Result<String> {
        self.epsg()
            .map(|c| format!("EPSG:{c}"))
            .ok_or_else(|| anyhow::anyhow!("custom CRS has no authority"))
    }
    pub fn name(&self) -> Option<String> {
        if let Some(s) = &self.native {
            return Some(s.name().to_string());
        }
        #[cfg(feature = "gdal-system")]
        if let Some(s) = &self.fallback {
            return s.name();
        }
        None
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
/// EPSG:3857 as GDAL writes it into GeoPackages and `.prj` files: WKT1 `Mercator_1SP`, which
/// read literally is the ellipsoidal projection. Only the PROJ.4 extension makes it the
/// spherical Web Mercator.
const GDAL_WEB_MERCATOR: &str = r#"PROJCS["WGS 84 / Pseudo-Mercator",GEOGCS["WGS 84",DATUM["WGS_1984",SPHEROID["WGS 84",6378137,298.257223563,AUTHORITY["EPSG","7030"]],AUTHORITY["EPSG","6326"]],PRIMEM["Greenwich",0,AUTHORITY["EPSG","8901"]],UNIT["degree",0.0174532925199433,AUTHORITY["EPSG","9122"]],AUTHORITY["EPSG","4326"]],PROJECTION["Mercator_1SP"],PARAMETER["central_meridian",0],PARAMETER["scale_factor",1],PARAMETER["false_easting",0],PARAMETER["false_northing",0],UNIT["metre",1,AUTHORITY["EPSG","9001"]],AXIS["Easting",EAST],AXIS["Northing",NORTH],EXTENSION["PROJ4","+proj=merc +a=6378137 +b=6378137 +lat_ts=0 +lon_0=0 +x_0=0 +y_0=0 +k=1 +units=m +nadgrids=@null +wktext +no_defs"],AUTHORITY["EPSG","3857"]]"#;
/// Whether the definition is exactly GDAL's, ignoring whitespace. Matching the whole text
/// keeps the check of EPSG authority tags strict for every other definition.
fn is_gdal_web_mercator(definition: &str) -> bool {
    let squeezed = |text: &str| text.split_whitespace().collect::<String>();
    definition.starts_with("PROJCS[") && squeezed(definition) == squeezed(GDAL_WEB_MERCATOR)
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
    } else if is_gdal_web_mercator(definition) {
        "EPSG:3857"
    } else {
        definition
    };
    let native = proj_wkt::parse_crs(definition);
    match native {
        Ok(native) => Ok(Crs {
            definition: definition.into(),
            native: Some(native),
            #[cfg(feature = "gdal-system")]
            fallback: None,
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
                        native: None,
                        fallback: Some(s),
                    });
                }
            }
            Err(error.into())
        }
    }
}
/// A horizontal XY transformation. Native operations always take precedence.
#[derive(Debug)]
pub struct Transform {
    native: Option<proj_core::Transform>,
    #[cfg(feature = "gdal-system")]
    fallback: Option<gdal::spatial_ref::CoordTransform>,
}
impl Transform {
    /// # Errors
    /// Returns an error when no supported horizontal operation connects the CRSs.
    pub fn new(from: &Crs, to: &Crs) -> Result<Self> {
        match proj_wkt::transform_from_crs_strings_horizontal(&from.definition, &to.definition) {
            Ok(native) => Ok(Self {
                native: Some(native),
                #[cfg(feature = "gdal-system")]
                fallback: None,
            }),
            Err(_error) => {
                #[cfg(feature = "gdal-system")]
                {
                    Ok(Self {
                        native: None,
                        fallback: Some(gdal::spatial_ref::CoordTransform::new(
                            &from.to_gdal()?,
                            &to.to_gdal()?,
                        )?),
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
        if x.iter().chain(y.iter()).any(|v| !v.is_finite()) {
            bail!("non-finite input coordinates");
        }
        if let Some(t) = &self.native {
            let result = x
                .iter()
                .zip(y.iter())
                .map(|(&x, &y)| t.convert((x, y)))
                .collect::<std::result::Result<Vec<_>, _>>()?;
            for ((x, y), (tx, ty)) in x.iter_mut().zip(y.iter_mut()).zip(result) {
                *x = tx;
                *y = ty;
            }
        } else {
            #[cfg(feature = "gdal-system")]
            self.fallback
                .as_ref()
                .expect("transform backend")
                .transform_coords(x, y, &mut [])?;
        }
        if x.iter().chain(y.iter()).any(|v| !v.is_finite()) {
            bail!("coordinate transformation produced non-finite coordinates");
        }
        Ok(())
    }
    /// # Errors
    /// Returns an error for invalid bounds, excessive densification or transformation failures.
    pub fn transform_bounds(&self, bounds: &[f64; 4], densify: i32) -> Result<[f64; 4]> {
        if let Some(t) = &self.native {
            let b = t.transform_bounds(
                proj_core::Bounds::new(bounds[0], bounds[1], bounds[2], bounds[3]),
                densify.try_into()?,
            )?;
            return Ok([b.min_x, b.min_y, b.max_x, b.max_y]);
        }
        #[cfg(feature = "gdal-system")]
        return Ok(self
            .fallback
            .as_ref()
            .expect("transform backend")
            .transform_bounds(bounds, densify)?);
        #[cfg(not(feature = "gdal-system"))]
        bail!("unsupported bounds transformation")
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
