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
pub struct CancellationToken(Arc<AtomicBool>);
impl CancellationToken {
    /// Stop this request at the next construction or emission checkpoint.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
    /// Whether cancellation was requested.
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}
#[cfg(test)]
impl CancellationToken {
    /// Whether both address the same request.
    pub(crate) fn same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
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
/// [`check`] at a named construction step, which tests can cancel at.
pub(crate) fn checkpoint(step: &'static str) {
    #[cfg(test)]
    hooks::reached(step);
    #[cfg(not(test))]
    let _ = step;
    check();
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

/// Checkpoint visits and worker threads per watched token, and cancellation
/// at a chosen visit.
#[cfg(test)]
pub(crate) mod hooks {
    use super::CancellationToken;
    use std::{
        collections::BTreeMap,
        sync::{
            Mutex, PoisonError,
            atomic::{AtomicUsize, Ordering},
        },
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
    type Watched = Vec<(CancellationToken, Record)>;
    static WATCHED: Mutex<Watched> = Mutex::new(Vec::new());
    static COUNT: AtomicUsize = AtomicUsize::new(0);
    fn current(f: impl FnOnce(&CancellationToken, &mut Record)) {
        if COUNT.load(Ordering::Acquire) == 0 {
            return;
        }
        let Some(token) = super::current() else {
            return;
        };
        let mut watched = WATCHED.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((_, record)) = watched.iter_mut().find(|(t, _)| t.same(&token)) {
            f(&token, record);
        }
    }
    pub(crate) fn watch(token: &CancellationToken, cancel_at: Option<(&'static str, usize)>) {
        let mut watched = WATCHED.lock().unwrap_or_else(PoisonError::into_inner);
        watched.push((
            token.clone(),
            Record {
                cancel_at,
                ..Record::default()
            },
        ));
        COUNT.fetch_add(1, Ordering::Release);
    }
    /// What has been recorded for a watched token so far.
    pub(crate) fn record(token: &CancellationToken) -> Record {
        let watched = WATCHED.lock().unwrap_or_else(PoisonError::into_inner);
        watched
            .iter()
            .find(|(t, _)| t.same(token))
            .map(|(_, record)| record.clone())
            .expect("watched token")
    }
    /// Stop watching a token and return what was recorded.
    pub(crate) fn unwatch(token: &CancellationToken) -> Record {
        let mut watched = WATCHED.lock().unwrap_or_else(PoisonError::into_inner);
        let index = watched
            .iter()
            .position(|(t, _)| t.same(token))
            .expect("watched token");
        COUNT.fetch_sub(1, Ordering::Release);
        watched.swap_remove(index).1
    }
    pub(crate) fn reached(step: &'static str) {
        current(|token, record| {
            let visit = record.visits.entry(step).or_default();
            if record.cancel_at == Some((step, *visit)) {
                token.cancel();
            }
            *visit += 1;
        });
    }
    /// A worker thread of a watched request, counted while it lives.
    pub(crate) struct Live(Option<CancellationToken>);
    impl Live {
        pub(crate) fn start() -> Self {
            let mut token = None;
            current(|t, record| {
                record.live += 1;
                record.workers += 1;
                token = Some(t.clone());
            });
            Self(token)
        }
    }
    impl Drop for Live {
        fn drop(&mut self) {
            if let Some(token) = &self.0 {
                let mut watched = WATCHED.lock().unwrap_or_else(PoisonError::into_inner);
                if let Some((_, record)) = watched.iter_mut().find(|(t, _)| t.same(token)) {
                    record.live -= 1;
                }
            }
        }
    }
}
