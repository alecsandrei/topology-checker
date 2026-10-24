//! WKB boundary to the GDAL-free topology core. Python owns adapters and typed views.
use geozero::{CoordDimensions, ToGeo, ToWkb, wkb::Wkb};
use pyo3::{
    exceptions::{PyRuntimeError, PyValueError},
    prelude::*,
    types::PyBytes,
};
use serde_json::json;
mod stream;
mod wkb;
use std::{
    collections::HashSet,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::Mutex,
};
use topology_checker_core::{
    Layer, LayerId, LayerInput, Rule, Topology,
    geo::{CoordsIter, Geometry, LineString},
};

type RegistrationInput = (Option<usize>, String, Vec<(i64, Vec<u8>)>);
type PreparedLayer = (
    Vec<(i64, Py<PyBytes>)>,
    Vec<Py<PyBytes>>,
    Vec<(i64, Vec<usize>)>,
    Vec<String>,
    Vec<&'static str>,
    Vec<&'static str>,
);

pyo3::create_exception!(topology_checker._native, PrecisionError, PyRuntimeError);
fn core_error(error: topology_checker_core::Error) -> PyErr {
    match error {
        topology_checker_core::Error::Precision { .. } => {
            PrecisionError::new_err(error.to_string())
        }
        other => value_error(other),
    }
}
fn value_error(error: impl std::fmt::Display) -> PyErr {
    PyValueError::new_err(error.to_string())
}
fn runtime_error(error: impl std::fmt::Display) -> PyErr {
    PyRuntimeError::new_err(error.to_string())
}
fn detached<T: Send>(
    py: Python<'_>,
    operation: impl FnOnce() -> PyResult<T> + Send,
) -> PyResult<T> {
    py.detach(|| {
        catch_unwind(AssertUnwindSafe(operation))
            .map_err(|_| runtime_error("unexpected native panic"))?
    })
}
fn decode(bytes: Vec<u8>) -> PyResult<Geometry> {
    Wkb(bytes).to_geo().map_err(value_error)
}
fn encode(geometry: &Geometry) -> PyResult<Vec<u8>> {
    geometry
        .to_wkb(CoordDimensions::xy())
        .map_err(runtime_error)
}

#[pyclass(frozen, module = "topology_checker._native")]
#[derive(Debug)]
struct NativeTopology {
    core: Mutex<Topology>,
}

impl NativeTopology {
    fn access<T: Send>(
        &self,
        py: Python<'_>,
        f: impl FnOnce(&mut Topology) -> PyResult<T> + Send,
    ) -> PyResult<T> {
        detached(py, || {
            let mut core = self.core.lock().map_err(runtime_error)?;
            f(&mut core)
        })
    }
}

fn layer_id(core: &Topology, index: usize) -> PyResult<LayerId> {
    core.layers()
        .nth(index)
        .map(|(id, _)| id)
        .ok_or_else(|| value_error(format!("unknown layer {index}")))
}

#[pymethods]
impl NativeTopology {
    #[new]
    fn new(tolerance: f64) -> PyResult<Self> {
        Ok(Self {
            core: Mutex::new(Topology::new(tolerance).map_err(value_error)?),
        })
    }

    fn add_layer(
        &self,
        py: Python<'_>,
        name: String,
        features: Vec<(i64, Vec<u8>)>,
    ) -> PyResult<usize> {
        self.access(py, |core| {
            // Decode and validate before mutating the registration list.
            let mut seen = HashSet::new();
            let features = features
                .into_iter()
                .map(|(fid, bytes)| {
                    if !seen.insert(fid) {
                        return Err(value_error(format!("duplicate FID {fid}")));
                    }
                    let geometry = decode(bytes)?;
                    if geometry
                        .coords_iter()
                        .any(|c| !c.x.is_finite() || !c.y.is_finite())
                    {
                        return Err(value_error(format!(
                            "FID {fid}: coordinates must be finite"
                        )));
                    }
                    Ok((fid, geometry))
                })
                .collect::<PyResult<Vec<_>>>()?;
            let layer = Layer::from_features(name, features);
            // A private snapshot validates without consuming the real registration ID.
            let mut pending = Topology::new(core.tolerance()).map_err(value_error)?;
            let id = pending.add_layer(layer);
            pending
                .layer(id)
                .map_err(value_error)?
                .check(id)
                .map_err(value_error)?;
            let layer = pending.shared_layer(id).map_err(value_error)?;
            Ok(core.add_layer(layer).index())
        })
    }

    #[pyo3(signature = (rule, layer, other=None))]
    fn add_rule(
        &self,
        py: Python<'_>,
        rule: &str,
        layer: usize,
        other: Option<usize>,
    ) -> PyResult<()> {
        self.access(py, |core| {
            let rule = rule.parse::<Rule>().map_err(value_error)?;
            let layer = layer_id(core, layer)?;
            let other = other.map(|i| layer_id(core, i)).transpose()?;
            core.add_rule(rule, layer, other)
                .map(|_| ())
                .map_err(value_error)
        })
    }

    fn register_rule(
        &self,
        py: Python<'_>,
        rule: &str,
        inputs: Vec<RegistrationInput>,
    ) -> PyResult<(usize, Option<usize>)> {
        self.access(py, |core| {
            let rule = rule.parse::<Rule>().map_err(value_error)?;
            let mut inputs = inputs
                .into_iter()
                .map(|(id, name, features)| {
                    if let Some(id) = id {
                        return Ok(LayerInput::Existing(layer_id(core, id)?));
                    }
                    let features = features
                        .into_iter()
                        .map(|(fid, bytes)| Ok((fid, decode(bytes)?)))
                        .collect::<PyResult<Vec<_>>>()?;
                    Ok(LayerInput::New(Box::new(Layer::from_features(
                        name, features,
                    ))))
                })
                .collect::<PyResult<Vec<_>>>()?
                .into_iter();
            let layer = inputs
                .next()
                .ok_or_else(|| value_error("a primary input is required"))?;
            let other = inputs.next();
            if inputs.next().is_some() {
                return Err(value_error("at most two inputs are supported"));
            }
            let spec = core
                .register_rule(rule, layer, other)
                .map_err(value_error)?;
            Ok((spec.layer.index(), spec.other.map(LayerId::index)))
        })
    }

    fn warnings(&self, py: Python<'_>) -> PyResult<Vec<stream::NativeWarning>> {
        self.access(py, |core| {
            core.warnings()
                .map(|warnings| {
                    warnings
                        .into_iter()
                        .map(stream::NativeWarning::from)
                        .collect()
                })
                .map_err(value_error)
        })
    }

    fn validate(&self, py: Python<'_>) -> PyResult<stream::NativeReport> {
        self.access(py, |core| stream::NativeReport::validate(core))
    }
    #[pyo3(signature = (batch_size=1024))]
    fn iter_validate(
        &self,
        py: Python<'_>,
        batch_size: usize,
    ) -> PyResult<stream::NativeValidationStream> {
        self.access(py, |core| {
            stream::NativeValidationStream::new(core.snapshot(), batch_size)
        })
    }
    fn iter_reports(&self, py: Python<'_>) -> PyResult<stream::NativeReportStream> {
        self.access(py, |core| stream::NativeReportStream::new(core.snapshot()))
    }
}

#[pyfunction]
fn registry() -> PyResult<String> {
    let rules = Rule::ALL
        .iter()
        .map(|rule| {
            let (a, b) = rule.inputs();
            json!({"value": rule.name(), "description": rule.description(),
            "inputs": [a, b], "arcgis": rule.arcgis(), "qgis": rule.qgis()})
        })
        .collect::<Vec<_>>();
    serde_json::to_string(&rules).map_err(runtime_error)
}

/// Normalise WKB items into retained planar features and their XY snapshots.
#[pyfunction]
fn prepare(
    py: Python<'_>,
    name: &str,
    fids: Vec<i64>,
    items: Vec<Option<Py<PyBytes>>>,
) -> PyResult<PreparedLayer> {
    let slices = items
        .iter()
        .map(|item| item.as_ref().map(|bytes| bytes.as_bytes(py)))
        .collect::<Vec<_>>();
    let prepared = detached(py, || {
        wkb::prepare(name, &fids, &slices).map_err(value_error)
    })?;
    let mut features = Vec::with_capacity(prepared.features.len());
    let mut snapshots = Vec::with_capacity(prepared.features.len());
    for feature in prepared.features {
        let snapshot = match (feature.snapshot, &items[feature.index]) {
            (Some(bytes), _) => PyBytes::new(py, &bytes).unbind(),
            (None, Some(input)) => input.clone_ref(py),
            (None, None) => return Err(runtime_error("missing input reused")),
        };
        let clean = match feature.clean {
            Some(bytes) => PyBytes::new(py, &bytes).unbind(),
            None => snapshot.clone_ref(py),
        };
        features.push((feature.fid, clean));
        snapshots.push(snapshot);
    }
    Ok((
        features,
        snapshots,
        prepared.parts,
        prepared.warnings,
        prepared.families,
        prepared.hints,
    ))
}

fn collect_lines(geometry: Geometry, lines: &mut Vec<LineString>) -> PyResult<()> {
    match geometry {
        Geometry::LineString(line) => lines.push(line),
        Geometry::MultiLineString(multi) => lines.extend(multi.0),
        Geometry::GeometryCollection(collection) => {
            for child in collection {
                collect_lines(child, lines)?;
            }
        }
        _ => return Err(value_error("line utilities require line geometries")),
    }
    Ok(())
}

#[pyfunction]
#[pyo3(signature = (geometries, tolerance=None))]
fn lines(
    py: Python<'_>,
    geometries: Vec<Vec<u8>>,
    tolerance: Option<f64>,
) -> PyResult<Vec<Vec<u8>>> {
    detached(py, || {
        let mut lines = Vec::new();
        for bytes in geometries {
            collect_lines(decode(bytes)?, &mut lines)?;
        }
        let output = match tolerance {
            Some(tolerance) => topology_checker_core::merge_lines(lines, tolerance),
            None => topology_checker_core::explode_lines(lines),
        }
        .map_err(value_error)?;
        output
            .into_iter()
            .map(|line| encode(&Geometry::LineString(line)))
            .collect()
    })
}

#[pymodule]
fn _native(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add("PrecisionError", module.py().get_type::<PrecisionError>())?;
    module.add_class::<NativeTopology>()?;
    module.add_class::<stream::NativeValidationStream>()?;
    module.add_class::<stream::NativeReportStream>()?;
    module.add_class::<stream::NativeReport>()?;
    module.add_class::<stream::NativeSummary>()?;
    module.add_class::<stream::NativeRuleSummary>()?;
    module.add_class::<stream::NativeViolation>()?;
    module.add_function(wrap_pyfunction!(registry, module)?)?;
    module.add_function(wrap_pyfunction!(lines, module)?)?;
    module.add_function(wrap_pyfunction!(prepare, module)?)?;
    Ok(())
}
