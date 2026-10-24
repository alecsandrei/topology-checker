//! Cooperative cancellation for bounded validation workers.
use std::{
    cell::RefCell,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

/// A thread-safe cancellation flag. Clones address the same validation request.
#[derive(Debug, Clone, Default)]
pub struct CancellationToken {
    flag: Arc<AtomicBool>,
    /// What tests record about the request.
    #[cfg(test)]
    record: Option<Arc<std::sync::Mutex<hooks::Record>>>,
}
impl CancellationToken {
    /// Stop this request at the next construction or emission checkpoint.
    pub fn cancel(&self) {
        self.flag.store(true, Ordering::Release);
    }
    /// Whether cancellation was requested.
    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::Acquire)
    }
}
thread_local! { static CURRENT: RefCell<Option<CancellationToken>> = const { RefCell::new(None) }; }
#[derive(Debug)]
pub(crate) struct Cancelled;
pub(crate) fn check() {
    if CURRENT.with(|c| {
        c.borrow()
            .as_ref()
            .is_some_and(CancellationToken::is_cancelled)
    }) {
        // Unwinding releases construction scratch and leaves OnceLock caches unset.
        std::panic::resume_unwind(Box::new(Cancelled));
    }
}
/// The calling thread's token, for worker threads that serve the same request.
pub(crate) fn current() -> Option<CancellationToken> {
    CURRENT.with(|c| c.borrow().clone())
}
/// Cheap units of work, such as comparisons or points, between checks in long
/// loops: a check every microsecond or so, at no measurable cost. Loops whose
/// items take longer check every item, and plain copies every [`COPY`].
pub(crate) const CHECK: usize = 256;
/// Elements copied between checks, a fraction of a millisecond's work.
pub(crate) const COPY: usize = CHECK * CHECK;
/// [`check`] at a named construction step, which tests can cancel at.
pub(crate) fn checkpoint(step: &'static str) {
    #[cfg(test)]
    hooks::reached(step);
    #[cfg(not(test))]
    let _ = step;
    check();
}
/// [`checkpoint`] at every [`CHECK`]th unit of work, counted from zero.
pub(crate) fn every(unit: usize, step: &'static str) {
    if unit.is_multiple_of(CHECK) {
        checkpoint(step);
    }
}
pub(crate) fn scope<T>(token: Option<CancellationToken>, run: impl FnOnce() -> T) -> T {
    struct Restore(Option<CancellationToken>);
    impl Drop for Restore {
        fn drop(&mut self) {
            CURRENT.with(|c| *c.borrow_mut() = self.0.take());
        }
    }
    let _restore = Restore(CURRENT.with(|c| c.replace(token)));
    check();
    run()
}

/// Checkpoint visits and worker threads of a request, and cancellation at a
/// chosen visit.
#[cfg(test)]
pub(crate) mod hooks {
    use super::{CURRENT, CancellationToken};
    use std::{
        collections::BTreeMap,
        sync::{Arc, Mutex, MutexGuard, PoisonError},
    };
    #[derive(Debug, Default, Clone)]
    pub(crate) struct Record {
        /// Visits of each checkpoint.
        pub visits: BTreeMap<&'static str, usize>,
        /// Cancel at this visit of a checkpoint, counted from zero.
        pub cancel_at: Option<(&'static str, usize)>,
        /// Worker threads running now.
        pub live: usize,
        /// Worker threads started.
        pub workers: usize,
    }
    fn lock(record: &Mutex<Record>) -> MutexGuard<'_, Record> {
        record.lock().unwrap_or_else(PoisonError::into_inner)
    }
    /// A token whose request records what it does, cancelled at `cancel_at`.
    pub(crate) fn watched(cancel_at: Option<(&'static str, usize)>) -> CancellationToken {
        let record = Record {
            cancel_at,
            ..Record::default()
        };
        CancellationToken {
            record: Some(Arc::new(Mutex::new(record))),
            ..CancellationToken::default()
        }
    }
    /// What has been recorded for a watched token so far.
    pub(crate) fn record(token: &CancellationToken) -> Record {
        lock(token.record.as_ref().expect("watched token")).clone()
    }
    pub(super) fn reached(step: &'static str) {
        CURRENT.with(|c| {
            let current = c.borrow();
            let Some(token) = current.as_ref() else {
                return;
            };
            let Some(record) = &token.record else {
                return;
            };
            let mut record = lock(record);
            let cancel_at = record.cancel_at;
            let visit = record.visits.entry(step).or_default();
            if cancel_at == Some((step, *visit)) {
                token.cancel();
            }
            *visit += 1;
        });
    }
    /// A worker thread of a watched request, counted while it lives.
    pub(crate) struct Live(Option<Arc<Mutex<Record>>>);
    impl Live {
        pub(crate) fn start() -> Self {
            let record = CURRENT.with(|c| c.borrow().as_ref().and_then(|t| t.record.clone()));
            if let Some(record) = &record {
                let mut record = lock(record);
                record.live += 1;
                record.workers += 1;
            }
            Self(record)
        }
    }
    impl Drop for Live {
        fn drop(&mut self) {
            if let Some(record) = &self.0 {
                lock(record).live -= 1;
            }
        }
    }
}
