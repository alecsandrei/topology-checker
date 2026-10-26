//! Lazy immutable caches with cancellable waits for another initializer.
use std::sync::{Mutex, OnceLock, TryLockError};

#[derive(Debug)]
pub(crate) struct Cache<T> {
    value: OnceLock<T>,
    initialize: Mutex<()>,
    /// Whether one parallel task made the cache for itself. Caches that tasks share
    /// build the same way whichever of them needs it first.
    local: bool,
}
impl<T> Default for Cache<T> {
    fn default() -> Self {
        Self::new()
    }
}
impl<T> Cache<T> {
    /// A cache of whatever runs on this thread now.
    pub fn new() -> Self {
        Self::owned(crate::engine::parallel::local())
    }
    /// A cache of one parallel task when `local`, else one that tasks share.
    pub fn owned(local: bool) -> Self {
        Self {
            value: OnceLock::new(),
            initialize: Mutex::new(()),
            local,
        }
    }
    pub fn get_or_init(&self, initialize: impl FnOnce() -> T) -> &T {
        if let Some(value) = self.value.get() {
            return value;
        }
        loop {
            crate::cancel::check();
            let guard = match self.initialize.try_lock() {
                Ok(guard) => guard,
                Err(TryLockError::Poisoned(error)) => error.into_inner(),
                Err(TryLockError::WouldBlock) => {
                    std::thread::park_timeout(std::time::Duration::from_millis(10));
                    if let Some(value) = self.value.get() {
                        crate::cancel::check();
                        return value;
                    }
                    continue;
                }
            };
            let value = self.value.get_or_init(|| {
                if self.local {
                    initialize()
                } else {
                    crate::engine::parallel::shared(initialize)
                }
            });
            drop(guard);
            return value;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, mpsc};

    #[test]
    fn cancelling_a_waiter_does_not_wait_for_or_cancel_the_initializer() {
        let cache = Arc::new(Cache::new());
        let (started, owner_started) = mpsc::channel();
        let (release, owner_release) = mpsc::channel();
        let owner_cache = Arc::clone(&cache);
        let owner = std::thread::spawn(move || {
            *owner_cache.get_or_init(|| {
                started.send(()).unwrap();
                owner_release.recv().unwrap();
                7
            })
        });
        owner_started.recv().unwrap();
        let token = crate::CancellationToken::default();
        let waiter_token = token.clone();
        let waiter_cache = Arc::clone(&cache);
        let (entered, waiter_entered) = mpsc::channel();
        let (finished, waiter_finished) = mpsc::channel();
        let waiter = std::thread::spawn(move || {
            let result = std::panic::catch_unwind(|| {
                crate::cancel::scope(Some(waiter_token), || {
                    entered.send(()).unwrap();
                    waiter_cache.get_or_init(|| 99)
                })
            });
            finished
                .send(result.is_err_and(|error| error.is::<crate::cancel::Cancelled>()))
                .unwrap();
        });
        waiter_entered.recv().unwrap();
        token.cancel();
        let result = waiter_finished.recv_timeout(std::time::Duration::from_millis(500));
        release.send(()).unwrap();
        assert_eq!(owner.join().unwrap(), 7);
        waiter.join().unwrap();
        assert!(result.unwrap());
        assert_eq!(*cache.get_or_init(|| 99), 7);
    }

    #[test]
    fn a_panicking_initializer_does_not_poison_later_callers() {
        let cache = Cache::new();
        let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            cache.get_or_init(|| -> u8 { panic!("initializer failed") });
        }));
        assert!(failed.is_err());
        assert_eq!(*cache.get_or_init(|| 7), 7);
    }

    #[test]
    fn waiters_retry_until_a_slow_initializer_finishes() {
        let cache = Arc::new(Cache::new());
        let (started, owner_started) = mpsc::channel();
        let owner_cache = Arc::clone(&cache);
        let owner = std::thread::spawn(move || {
            *owner_cache.get_or_init(|| {
                started.send(()).unwrap();
                std::thread::sleep(std::time::Duration::from_millis(60));
                3
            })
        });
        owner_started.recv().unwrap();
        assert_eq!(*cache.get_or_init(|| 4), 3);
        assert_eq!(owner.join().unwrap(), 3);
    }
}
