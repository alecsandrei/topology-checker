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
/// Untagged CRS definitions that carry authoritative datums without inline datum shifts
/// (common in ESRI .prj sidecars written by GIS software) need canonicalization to their
/// registry EPSG codes so `proj-core` can select coordinate operations to WGS84.
fn canonicalize_untagged(native: proj_core::CrsDef) -> Option<(String, proj_core::CrsDef)> {
    if native.epsg() != 0 {
        return None;
    }
    let datum_epsg = native.datum().epsg();
    let candidates: &[u32] = match datum_epsg {
        6179 => &[3844, 4179],
        6314 => &[31466, 31467, 31468, 31469, 4314],
        6150 => &[2056, 4150],
        6149 => &[21781, 4149],
        6289 => &[28992, 4289],
        6156 => &[5514, 4156],
        6277 => &[27700, 4277],
        6171 => &[2154, 4171],
        _ => return None,
    };
    candidates.iter().find_map(|&code| {
        let registry = proj_core::lookup_epsg(code)?;
        native
            .semantically_equivalent(&registry)
            .then(|| (format!("EPSG:{code}"), registry))
    })
}
fn projjson_epsg_id(value: &serde_json::Value) -> Option<u32> {
    let check = |id: &serde_json::Value| -> Option<u32> {
        id.get("authority")?
            .as_str()?
            .eq_ignore_ascii_case("EPSG")
            .then_some(())?;
        match id.get("code")? {
            serde_json::Value::Number(n) => n.as_u64().and_then(|n| u32::try_from(n).ok()),
            serde_json::Value::String(s) => s.parse().ok(),
            _ => None,
        }
    };
    value
        .get("id")
        .and_then(check)
        .or_else(|| value.get("ids")?.as_array()?.iter().find_map(check))
}
fn swap_projjson_axes(crs: &mut serde_json::Value) -> bool {
    let mut swapped = false;
    if let Some(base) = crs.get_mut("base_crs") {
        swapped |= swap_projjson_axes(base);
    }
    if let Some(components) = crs
        .get_mut("components")
        .and_then(serde_json::Value::as_array_mut)
    {
        for component in components {
            swapped |= swap_projjson_axes(component);
        }
    }
    let Some(axes) = crs
        .get_mut("coordinate_system")
        .and_then(|cs| cs.get_mut("axis"))
        .and_then(serde_json::Value::as_array_mut)
    else {
        return swapped;
    };
    let is_dir = |axis: &serde_json::Value, expected: &str| {
        axis.get("direction")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|d| d.trim().eq_ignore_ascii_case(expected))
    };
    let should_swap = match axes.as_slice() {
        [first, second] => is_dir(first, "north") && is_dir(second, "east"),
        [first, second, third] => {
            is_dir(first, "north") && is_dir(second, "east") && is_dir(third, "up")
        }
        _ => false,
    };
    if should_swap {
        axes.swap(0, 1);
        swapped = true;
    }
    swapped
}
/// Normalize authority-native `north, east` axis order in EPSG-tagged PROJJSON (as written
/// in GeoParquet metadata) so `proj-wkt` can validate the remaining CRS semantics.
fn normalize_projjson_axes(definition: &str) -> Option<String> {
    if !definition.starts_with('{') {
        return None;
    }
    let mut value: serde_json::Value = serde_json::from_str(definition).ok()?;
    let code = projjson_epsg_id(&value)?;
    proj_core::lookup_epsg(code)?;
    swap_projjson_axes(&mut value).then(|| serde_json::to_string(&value).ok())?
}
fn wkt_element(s: &str) -> Option<(&str, &str)> {
    let s = s.trim();
    let open = s.find('[')?;
    if !s.ends_with(']')
        || !s[..open]
            .trim()
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
    {
        return None;
    }
    Some((s[..open].trim(), &s[open + 1..s.len() - 1]))
}
fn split_wkt_fields(inner: &str) -> Vec<&str> {
    let mut fields = Vec::new();
    let mut start = 0;
    let mut depth = 0usize;
    let mut in_quotes = false;
    let bytes = inner.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                if in_quotes && bytes.get(i + 1) == Some(&b'"') {
                    i += 2;
                    continue;
                }
                in_quotes = !in_quotes;
            }
            _ if in_quotes => {}
            b'[' => depth += 1,
            b']' => depth = depth.saturating_sub(1),
            b',' if depth == 0 => {
                fields.push(inner[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    fields.push(inner[start..].trim());
    fields
}
fn is_wkt2(definition: &str) -> bool {
    wkt_element(definition).is_some_and(|(name, _)| {
        name.eq_ignore_ascii_case("PROJCRS")
            || name.eq_ignore_ascii_case("GEOGCRS")
            || name.eq_ignore_ascii_case("GEODCRS")
            || name.eq_ignore_ascii_case("COMPOUNDCRS")
    })
}
fn morph_wkt(definition: &str) -> Option<String> {
    let mut modified = false;
    let mut result = definition.to_string();

    const REPLACEMENTS: &[(&str, &str)] = &[
        ("Gauss_Kruger", "Transverse_Mercator"),
        ("Gauss-Kruger", "Transverse_Mercator"),
        (
            "Hotine_Oblique_Mercator_Azimuth_Center",
            "Hotine_Oblique_Mercator_Variant_B",
        ),
        ("Plate_Carrée", "Plate_Carree"),
        ("BASEGEODCRS", "BASEGEOGCRS"),
        (
            "Lambert Conic Conformal (2SP)",
            "Lambert Conformal Conic (2SP)",
        ),
        (
            "Lambert Conic Conformal (1SP)",
            "Lambert Conformal Conic (1SP)",
        ),
        ("Lambert Conic Conformal", "Lambert Conformal Conic"),
        ("Easting at false origin", "False easting"),
        ("Northing at false origin", "False northing"),
        ("Longitude of false origin", "Longitude of natural origin"),
        ("Latitude of false origin", "Latitude of natural origin"),
    ];

    for &(from, to) in REPLACEMENTS {
        if result.contains(from) {
            result = result.replace(from, to);
            modified = true;
        }
    }

    modified.then_some(result)
}
fn normalize_wkt_axes(definition: &str) -> Option<String> {
    let (name, inner) = wkt_element(definition)?;
    let mut name = name.to_string();
    let mut changed = false;
    if name.eq_ignore_ascii_case("BASEGEODCRS") {
        name = "BASEGEOGCRS".to_string();
        changed = true;
    }
    let mut fields = split_wkt_fields(inner)
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    for field in &mut fields {
        if let Some((child_name, child_inner)) = wkt_element(field) {
            if child_name.eq_ignore_ascii_case("BASEGEODCRS")
                || child_name.eq_ignore_ascii_case("BASEGEOGCRS")
                || child_name.eq_ignore_ascii_case("GEOGCRS")
                || child_name.eq_ignore_ascii_case("GEODCRS")
                || child_name.eq_ignore_ascii_case("PROJCRS")
                || child_name.eq_ignore_ascii_case("GEOGCS")
                || child_name.eq_ignore_ascii_case("PROJCS")
                || child_name.eq_ignore_ascii_case("COMPD_CS")
                || child_name.eq_ignore_ascii_case("COMPOUNDCRS")
            {
                if let Some(normalized_child) = normalize_wkt_axes(field) {
                    *field = normalized_child;
                    changed = true;
                }
            } else if child_name.eq_ignore_ascii_case("PROJECTION") {
                let proj_name = child_inner.trim().trim_matches('"');
                if proj_name.eq_ignore_ascii_case("Gauss_Kruger")
                    || proj_name.eq_ignore_ascii_case("Gauss-Kruger")
                {
                    *field = r#"PROJECTION["Transverse_Mercator"]"#.to_string();
                    changed = true;
                } else if proj_name.eq_ignore_ascii_case("Hotine_Oblique_Mercator_Azimuth_Center") {
                    *field = r#"PROJECTION["Hotine_Oblique_Mercator_Variant_B"]"#.to_string();
                    changed = true;
                } else if proj_name.contains('é') || proj_name.contains('É') {
                    *field = format!(r#"PROJECTION["{}"]"#, proj_name.replace(['é', 'É'], "e"));
                    changed = true;
                }
            } else if child_name.eq_ignore_ascii_case("METHOD") {
                let method_name = child_inner.trim().trim_matches('"');
                if method_name.eq_ignore_ascii_case("Gauss_Kruger")
                    || method_name.eq_ignore_ascii_case("Gauss-Kruger")
                {
                    *field = r#"METHOD["Transverse Mercator"]"#.to_string();
                    changed = true;
                } else if method_name.eq_ignore_ascii_case("Hotine_Oblique_Mercator_Azimuth_Center")
                {
                    *field = r#"METHOD["Hotine Oblique Mercator (variant B)"]"#.to_string();
                    changed = true;
                } else if method_name.eq_ignore_ascii_case("Lambert Conic Conformal (2SP)") {
                    *field = r#"METHOD["Lambert Conformal Conic (2SP)"]"#.to_string();
                    changed = true;
                } else if method_name.eq_ignore_ascii_case("Lambert Conic Conformal (1SP)") {
                    *field = r#"METHOD["Lambert Conformal Conic (1SP)"]"#.to_string();
                    changed = true;
                }
            }
        }
    }
    let axes = fields
        .iter()
        .enumerate()
        .filter_map(|(idx, field)| {
            let (elem, axis_inner) = wkt_element(field)?;
            elem.eq_ignore_ascii_case("AXIS").then(|| {
                let dir = split_wkt_fields(axis_inner)
                    .get(1)
                    .map(|f| f.trim().trim_matches('"').to_owned())
                    .unwrap_or_default();
                (idx, dir)
            })
        })
        .collect::<Vec<_>>();
    let swap_pair = match axes.as_slice() {
        [(i, first), (j, second)] => Some((*i, *j, first.as_str(), second.as_str())),
        [(i, first), (j, second), (_, third)] if third.eq_ignore_ascii_case("up") => {
            Some((*i, *j, first.as_str(), second.as_str()))
        }
        _ => None,
    };
    if let Some((i, j, first, second)) = swap_pair
        && first.eq_ignore_ascii_case("north")
        && second.eq_ignore_ascii_case("east")
    {
        fields.swap(i, j);
        changed = true;
    }
    changed.then(|| format!("{name}[{}]", fields.join(",")))
}
fn parse_native(
    definition: &str,
) -> std::result::Result<(String, proj_core::CrsDef), proj_wkt::ParseError> {
    match proj_wkt::parse_crs(definition) {
        Ok(native) => Ok(
            canonicalize_untagged(native.clone()).unwrap_or_else(|| (definition.into(), native))
        ),
        Err(error) => {
            if let Some(normalized) = normalize_projjson_axes(definition)
                && let Ok(native) = proj_wkt::parse_crs(&normalized)
            {
                return Ok((normalized, native));
            }
            let morphed = morph_wkt(definition);
            let candidate = morphed.as_deref().unwrap_or(definition);
            if morphed.is_some() {
                match proj_wkt::parse_crs(candidate) {
                    Ok(native) => {
                        if let Some(canonical) = canonicalize_untagged(native.clone()) {
                            return Ok(canonical);
                        }
                        return Ok((candidate.to_string(), native));
                    }
                    Err(e) => {
                        eprintln!("morphed candidate parse failed: {e:#}");
                    }
                }
            }
            if let Some(normalized) = normalize_wkt_axes(candidate)
                && let Ok(native) = proj_wkt::parse_crs(&normalized)
            {
                if let Some(canonical) = canonicalize_untagged(native.clone()) {
                    return Ok(canonical);
                }
                if is_wkt2(candidate) && native.epsg() != 0 {
                    return Ok((normalized, native));
                }
                if native.epsg() != 0 || morphed.is_some() {
                    return Ok((normalized, native));
                }
            }
            Err(error)
        }
    }
}
fn normalize_identifier(definition: &str) -> Option<String> {
    let s = definition.trim();
    let s = s.strip_prefix('\u{feff}').unwrap_or(s).trim();
    if s.is_empty() {
        return None;
    }
    let upper = s.to_ascii_uppercase();
    if matches!(
        upper.as_str(),
        "CRS84"
            | "CRS:84"
            | "OGC:CRS84"
            | "URN:OGC:DEF:CRS:OGC:1.3:CRS84"
            | "URN:OGC:DEF:CRS:OGC::CRS84"
    ) {
        return Some("OGC:CRS84".into());
    }
    if matches!(
        upper.as_str(),
        "OGC:CRS27" | "URN:OGC:DEF:CRS:OGC:1.3:CRS27" | "URN:OGC:DEF:CRS:OGC::CRS27" | "NAD27"
    ) {
        return Some("EPSG:4267".into());
    }
    if matches!(
        upper.as_str(),
        "OGC:CRS83" | "URN:OGC:DEF:CRS:OGC:1.3:CRS83" | "URN:OGC:DEF:CRS:OGC::CRS83" | "NAD83"
    ) {
        return Some("EPSG:4269".into());
    }
    if matches!(upper.as_str(), "WGS84" | "WGS 84") {
        return Some("EPSG:4326".into());
    }
    if upper == "OSGB36" {
        return Some("EPSG:27700".into());
    }
    if matches!(
        upper.as_str(),
        "EPSG:900913" | "EPSG:3785" | "ESRI:102100" | "ESRI:102113"
    ) {
        return Some("EPSG:3857".into());
    }
    for prefix in ["HTTP://WWW.OPENGIS.NET/", "HTTPS://WWW.OPENGIS.NET/"] {
        if let Some(rest) = upper.strip_prefix(prefix) {
            if let Some(path) = rest.strip_prefix("DEF/CRS/") {
                let parts: Vec<&str> = path.split('/').collect();
                if parts.len() >= 2 {
                    let auth = parts[0];
                    let code = *parts.last()?;
                    if auth == "OGC" && code == "CRS84" {
                        return Some("OGC:CRS84".into());
                    }
                    if auth == "OGC" && code == "CRS27" {
                        return Some("EPSG:4267".into());
                    }
                    if auth == "OGC" && code == "CRS83" {
                        return Some("EPSG:4269".into());
                    }
                    if auth == "EPSG" && code.bytes().all(|b| b.is_ascii_digit()) {
                        return Some(format!("EPSG:{code}"));
                    }
                }
            } else if let Some(code) = rest.strip_prefix("GML/SRS/EPSG.XML#")
                && code.bytes().all(|b| b.is_ascii_digit())
            {
                return Some(format!("EPSG:{code}"));
            }
        }
    }
    for prefix in ["URN:OGC:DEF:CRS:", "URN:X-OGC:DEF:CRS:"] {
        if let Some(rest) = upper.strip_prefix(prefix) {
            let parts: Vec<&str> = rest.split(':').collect();
            if parts.len() >= 2 {
                let auth = parts[0];
                let code = *parts.last()?;
                if auth == "OGC" && code == "CRS84" {
                    return Some("OGC:CRS84".into());
                }
                if auth == "OGC" && code == "CRS27" {
                    return Some("EPSG:4267".into());
                }
                if auth == "OGC" && code == "CRS83" {
                    return Some("EPSG:4269".into());
                }
                if auth == "EPSG" && code.bytes().all(|b| b.is_ascii_digit()) {
                    return Some(format!("EPSG:{code}"));
                }
            }
        }
    }
    if upper.starts_with("EPSG:") && s.contains('+') && !s.contains('[') && !s.contains('{') {
        let first = upper.split('+').next().unwrap().trim();
        if let Some(code) = first.strip_prefix("EPSG:")
            && code.bytes().all(|b| b.is_ascii_digit())
        {
            return Some(format!("EPSG:{code}"));
        }
    }
    None
}
/// Parse an EPSG, WKT, PROJ or PROJJSON definition.
/// # Errors
/// Returns an error for invalid or unsupported CRS definitions.
pub fn parse(definition: &str) -> Result<Crs> {
    let definition = definition.trim();
    let definition = definition
        .strip_prefix('\u{feff}')
        .unwrap_or(definition)
        .trim();
    if definition.is_empty() {
        bail!("empty CRS definition");
    }
    let normalized_storage;
    let definition = if let Some(normalized) = normalize_identifier(definition) {
        normalized_storage = normalized;
        normalized_storage.as_str()
    } else if is_gdal_web_mercator(definition) {
        "EPSG:3857"
    } else {
        definition
    };
    match parse_native(definition) {
        Ok((definition, native)) => Ok(Crs {
            definition,
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
                if let (Backend::Native(from_def), Backend::Native(to_def)) =
                    (&from.backend, &to.backend)
                    && let Ok(native) =
                        proj_core::Transform::from_horizontal_components(from_def, to_def)
                {
                    return Ok(Self {
                        operation: Operation::Native(Box::new(native)),
                    });
                }
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
