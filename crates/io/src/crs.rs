use anyhow::{Result, bail};
use gdal::spatial_ref::{AxisMappingStrategy, CoordTransform, SpatialRef};
use geo::{CoordsIter, Geometry, MapCoordsInPlace};

/// Parse a GDAL CRS definition with longitude/easting first.
///
/// # Errors
/// Returns an error for unsupported definitions or GDAL parsing failures.
pub fn parse(definition: &str) -> Result<SpatialRef> {
    let trimmed = definition.trim();
    if !(trimmed.starts_with("EPSG:")
        || trimmed.starts_with("+proj=")
        || [
            "GEOGCS[",
            "PROJCS[",
            "LOCAL_CS[",
            "GEOGCRS[",
            "PROJCRS[",
            "GEODCRS[",
            "BOUNDCRS[",
            "COMPOUNDCRS[",
            "ENGCRS[",
        ]
        .iter()
        .any(|prefix| trimmed.starts_with(prefix)))
    {
        bail!("use an EPSG code, WKT or PROJ definition for the CRS");
    }
    let mut srs = SpatialRef::from_definition(definition)?;
    srs.set_axis_mapping_strategy(AxisMappingStrategy::TraditionalGisOrder);
    Ok(srs)
}

/// Transform an original geometry without cleaning, simplification or rounding.
///
/// # Errors
/// Returns an error if GDAL cannot transform the coordinates or they become non-finite.
pub fn transform(geometry: &Geometry, operation: &CoordTransform) -> Result<Geometry> {
    let mut x = Vec::new();
    let mut y = Vec::new();
    for c in geometry.coords_iter() {
        x.push(c.x);
        y.push(c.y);
    }
    operation.transform_coords(&mut x, &mut y, &mut [])?;
    if x.iter().chain(&y).any(|v| !v.is_finite()) {
        bail!("coordinate transformation produced non-finite coordinates");
    }
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
