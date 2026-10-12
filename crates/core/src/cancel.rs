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
