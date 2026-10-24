//! Typed binary report transport and closeable bounded validation workers.
use crate::{detached, encode, runtime_error, value_error};
use pyo3::{prelude::*, types::PyBytes};
use std::{
    collections::BTreeMap,
    sync::{
        Condvar, Mutex, PoisonError,
        mpsc::{self, Receiver},
    },
    thread::JoinHandle,
    time::Duration,
};
use topology_checker_core::{
    CancellationToken, FeatureRef, ReportSummary, RuleSpec, RuleSummary, SinkError, Topology,
    ValidationEvent, Violation, Warning,
};

type Reference = (usize, i64, Option<u32>);
fn reference(r: Option<FeatureRef>) -> Option<Reference> {
    r.map(|r| (r.layer.index(), r.fid, r.part))
}
#[pyclass(frozen, module = "topology_checker._native")]
#[derive(Debug, Clone)]
pub(crate) struct NativeViolation {
    #[pyo3(get)]
    pub rule_index: usize,
    #[pyo3(get)]
    pub error_type: String,
    #[pyo3(get)]
    pub origin: Option<Reference>,
    #[pyo3(get)]
    pub destination: Option<Reference>,
    bytes: Vec<u8>,
}
impl NativeViolation {
    pub fn new(spec: RuleSpec, v: Violation) -> PyResult<Self> {
        Ok(Self {
            rule_index: spec.id.index(),
            error_type: v.kind.name().into(),
            origin: reference(v.origin),
            destination: reference(v.destination),
            bytes: encode(&v.geometry)?,
        })
    }
}
#[pymethods]
impl NativeViolation {
    #[getter]
    fn wkb<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.bytes)
    }
}
#[pyclass(frozen, get_all, module = "topology_checker._native")]
#[derive(Debug, Clone)]
pub(crate) struct NativeWarning {
    pub message: String,
    pub layer: Option<usize>,
    pub fid: Option<i64>,
    pub part: Option<u32>,
}
impl From<Warning> for NativeWarning {
    fn from(w: Warning) -> Self {
        let (layer, fid, part) = match &w {
            Warning::CollapsedLine {
                layer, fid, part, ..
            } => (Some(layer.index()), Some(*fid), Some(*part)),
            Warning::DegenerateParts { layer, .. } => (Some(layer.index()), None, None),
            _ => (None, None, None),
        };
        Self {
            message: w.to_string(),
            layer,
            fid,
            part,
        }
    }
}
#[pyclass(frozen, get_all, module = "topology_checker._native")]
#[derive(Debug, Clone)]
pub(crate) struct NativeRuleSummary {
    pub layer: usize,
    pub other: Option<usize>,
    pub counts: BTreeMap<String, usize>,
    pub total: usize,
}
impl From<&RuleSummary> for NativeRuleSummary {
    fn from(r: &RuleSummary) -> Self {
        Self {
            layer: r.spec.layer.index(),
            other: r.spec.other.map(|i| i.index()),
            counts: r
                .counts
                .iter()
                .map(|(k, n)| (k.name().into(), *n))
                .collect(),
            total: r.total,
        }
    }
}
#[pyclass(frozen, get_all, module = "topology_checker._native")]
#[derive(Debug, Clone)]
pub(crate) struct NativeSummary {
    pub tolerance: f64,
    pub warnings: Vec<NativeWarning>,
    pub rules: Vec<NativeRuleSummary>,
}
impl From<ReportSummary> for NativeSummary {
    fn from(r: ReportSummary) -> Self {
        Self {
            tolerance: r.tolerance,
            warnings: r.warnings.into_iter().map(NativeWarning::from).collect(),
            rules: r.rules.iter().map(NativeRuleSummary::from).collect(),
        }
    }
}
#[pyclass(frozen, get_all, module = "topology_checker._native")]
#[derive(Debug)]
pub(crate) struct NativeReport {
    pub summary: NativeSummary,
    pub errors: Vec<NativeViolation>,
}
impl NativeReport {
    pub fn validate(core: &Topology) -> PyResult<Self> {
        let mut errors = Vec::new();
        let summary = core
            .validate_into(|spec, v| {
                errors.push(NativeViolation::new(spec, v)?);
                Ok::<_, PyErr>(())
            })
            .map_err(|e| match e {
                SinkError::Core(e) => super::core_error(e),
                SinkError::Consumer(e) => e,
            })?;
        Ok(Self {
            summary: summary.into(),
            errors,
        })
    }
}
/// One finished rule: its index, counts and ordered violations.
type RuleItem = (usize, NativeRuleSummary, Vec<NativeViolation>);
#[derive(Debug)]
enum Event<T> {
    Item(T),
    End(NativeSummary),
    Input(String),
    Precision(String),
    Error(String),
}
#[derive(Debug)]
enum Stop {
    Closed,
    Failed(String),
}
/// Validation worker threads that have not finished yet.
static WORKERS: (Mutex<usize>, Condvar) = (Mutex::new(0), Condvar::new());
/// Counts one worker from before its spawn until its thread body returns.
struct Live;
impl Live {
    fn new() -> Self {
        *WORKERS.0.lock().unwrap_or_else(PoisonError::into_inner) += 1;
        Self
    }
}
impl Drop for Live {
    fn drop(&mut self) {
        *WORKERS.0.lock().unwrap_or_else(PoisonError::into_inner) -= 1;
        WORKERS.1.notify_all();
    }
}
/// Wait up to `timeout` seconds for all workers to finish; return how many remain.
#[pyfunction]
#[pyo3(signature = (timeout=0.0))]
pub(crate) fn live_workers(py: Python<'_>, timeout: f64) -> PyResult<usize> {
    let timeout = Duration::try_from_secs_f64(timeout).map_err(value_error)?;
    detached(py, || {
        let count = WORKERS.0.lock().unwrap_or_else(PoisonError::into_inner);
        let (count, _) = WORKERS
            .1
            .wait_timeout_while(count, timeout, |n| *n > 0)
            .unwrap_or_else(PoisonError::into_inner);
        Ok(*count)
    })
}
#[derive(Debug)]
struct State<T> {
    receiver: Option<Receiver<Event<T>>>,
    worker: Option<JoinHandle<()>>,
    summary: Option<NativeSummary>,
    cancellation: CancellationToken,
}
impl<T: Send + 'static> State<T> {
    /// Validate a snapshot on a worker thread. `emit` turns events into items;
    /// at most `capacity` items wait in the queue.
    fn spawn(
        core: Topology,
        capacity: usize,
        emit: impl FnMut(ValidationEvent<'_>) -> Result<Option<T>, String> + Send + 'static,
    ) -> PyResult<Self> {
        let cancellation = CancellationToken::default();
        let worker_cancellation = cancellation.clone();
        let (sender, receiver) = mpsc::sync_channel(capacity);
        let live = Live::new();
        let worker = std::thread::Builder::new()
            .name("topology-validation".into())
            .spawn(move || {
                // Locals drop in reverse order, so the count falls after the snapshot is freed.
                let (_live, core, mut emit, sender) = (live, core, emit, sender);
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    core.validate_events_cancellable(worker_cancellation, |event| {
                        match emit(event).map_err(Stop::Failed)? {
                            Some(item) => sender.send(Event::Item(item)).map_err(|_| Stop::Closed),
                            None => Ok(()),
                        }
                    })
                }));
                let event = match outcome {
                    Ok(Ok(summary)) => Event::End(summary.into()),
                    Ok(
                        Err(SinkError::Consumer(Stop::Closed))
                        | Err(SinkError::Core(topology_checker_core::Error::Cancelled)),
                    ) => return,
                    Ok(Err(SinkError::Core(
                        e @ topology_checker_core::Error::Precision { .. },
                    ))) => Event::Precision(e.to_string()),
                    Ok(Err(SinkError::Core(e))) => Event::Input(e.to_string()),
                    Ok(Err(SinkError::Consumer(Stop::Failed(e)))) => Event::Error(e),
                    Err(_) => Event::Error("unexpected native panic".into()),
                };
                let _ = sender.send(event);
            })
            .map_err(runtime_error)?;
        Ok(Self {
            receiver: Some(receiver),
            worker: Some(worker),
            summary: None,
            cancellation,
        })
    }
}
impl<T> State<T> {
    /// Wait for the next item; `None` after exhaustion or close.
    fn next(&mut self) -> PyResult<Option<T>> {
        let Some(receiver) = &self.receiver else {
            return Ok(None);
        };
        let event = receiver.recv();
        if self.cancellation.is_cancelled() {
            // Closed from another thread while this one waited.
            self.close();
            return Ok(None);
        }
        let error = match event {
            Ok(Event::Item(item)) => return Ok(Some(item)),
            Ok(Event::End(summary)) => {
                self.summary = Some(summary);
                self.close();
                return Ok(None);
            }
            Ok(Event::Precision(error)) => super::PrecisionError::new_err(error),
            Ok(Event::Input(error)) => value_error(error),
            Ok(Event::Error(error)) => runtime_error(error),
            Err(error) => runtime_error(error),
        };
        self.close();
        Err(error)
    }
    fn close(&mut self) {
        // Disconnecting the bounded queue wakes a blocked sender immediately.
        self.cancellation.cancel();
        self.receiver.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
impl<T> Drop for State<T> {
    fn drop(&mut self) {
        self.cancellation.cancel();
        self.receiver.take();
        // The worker owns its snapshot and exits at a checkpoint without needing
        // the GIL. Dropping a stream must never join while holding the GIL.
        self.worker.take();
    }
}
#[pyclass(frozen, module = "topology_checker._native")]
#[derive(Debug)]
pub(crate) struct NativeValidationStream {
    state: Mutex<State<NativeViolation>>,
    cancellation: CancellationToken,
    batch_size: usize,
}
impl NativeValidationStream {
    pub fn new(core: Topology, batch_size: usize) -> PyResult<Self> {
        if batch_size == 0 {
            return Err(value_error("batch_size must be positive"));
        }
        let state = State::spawn(core, batch_size, |event| match event {
            ValidationEvent::Violation(spec, v) => NativeViolation::new(spec, v)
                .map(Some)
                .map_err(|e| e.to_string()),
            _ => Ok(None),
        })?;
        Ok(Self {
            cancellation: state.cancellation.clone(),
            state: Mutex::new(state),
            batch_size,
        })
    }
}
#[pymethods]
impl NativeValidationStream {
    fn next_batch(&self, py: Python<'_>) -> PyResult<Vec<NativeViolation>> {
        detached(py, || {
            let mut state = self.state.lock().map_err(runtime_error)?;
            let mut output = Vec::with_capacity(self.batch_size);
            while output.len() < self.batch_size {
                match state.next()? {
                    Some(v) => output.push(v),
                    None => break,
                }
            }
            Ok(output)
        })
    }
    fn close(&self, py: Python<'_>) -> PyResult<()> {
        // Cancel first: a consumer blocked in another thread holds the state.
        self.cancellation.cancel();
        detached(py, || {
            self.state.lock().map_err(runtime_error)?.close();
            Ok(())
        })
    }
    #[getter]
    fn summary(&self, py: Python<'_>) -> PyResult<Option<NativeSummary>> {
        detached(py, || {
            Ok(self.state.lock().map_err(runtime_error)?.summary.clone())
        })
    }
}
/// Rule reports in registration order, each sent when its rule ends.
#[pyclass(frozen, module = "topology_checker._native")]
#[derive(Debug)]
pub(crate) struct NativeReportStream {
    state: Mutex<State<RuleItem>>,
    cancellation: CancellationToken,
}
impl NativeReportStream {
    pub fn new(core: Topology) -> PyResult<Self> {
        let mut errors = Vec::new();
        // A rendezvous queue lets the worker run one rule ahead of the consumer.
        let state = State::spawn(core, 0, move |event| match event {
            ValidationEvent::Violation(spec, v) => {
                errors.push(NativeViolation::new(spec, v).map_err(|e| e.to_string())?);
                Ok(None)
            }
            ValidationEvent::RuleEnd(rule) => Ok(Some((
                rule.spec.id.index(),
                NativeRuleSummary::from(rule),
                std::mem::take(&mut errors),
            ))),
            _ => Ok(None),
        })?;
        Ok(Self {
            cancellation: state.cancellation.clone(),
            state: Mutex::new(state),
        })
    }
}
#[pymethods]
impl NativeReportStream {
    fn next_report(&self, py: Python<'_>) -> PyResult<Option<RuleItem>> {
        detached(py, || self.state.lock().map_err(runtime_error)?.next())
    }
    fn close(&self, py: Python<'_>) -> PyResult<()> {
        // Cancel first: a consumer blocked in another thread holds the state.
        self.cancellation.cancel();
        detached(py, || {
            self.state.lock().map_err(runtime_error)?.close();
            Ok(())
        })
    }
    #[getter]
    fn summary(&self, py: Python<'_>) -> PyResult<Option<NativeSummary>> {
        detached(py, || {
            Ok(self.state.lock().map_err(runtime_error)?.summary.clone())
        })
    }
}
