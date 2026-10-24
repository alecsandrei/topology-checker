//! Scoped worker threads for large construction steps.
//!
//! Shared structures are built inside cache initialisers, where waiting on
//! rayon jobs can deadlock: a rayon thread waiting for a job steals other
//! work, which may wait for this very initialiser. These helpers use scoped
//! threads instead, pass on the caller's cancellation token and run small
//! inputs inline. Results never depend on the number of threads.
use crate::cancel;
use std::{
    any::Any,
    cmp::Ordering,
    ops::Range,
    panic::{self, AssertUnwindSafe},
    sync::{
        Mutex, PoisonError,
        atomic::{self, AtomicBool, AtomicUsize},
    },
};

/// Steps over fewer units run on the calling thread.
const INLINE: usize = 1 << 14;
/// Smallest task, in units.
const GRAIN: usize = 4096;
/// Tasks per thread, so uneven tasks still share the work.
const TASKS: usize = 8;

#[cfg(test)]
thread_local! {
    static FORCED: std::cell::Cell<Option<Workers>> = const { std::cell::Cell::new(None) };
}
thread_local! {
    static CAP: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

/// Run `f` with each step on at most `threads` threads, the calling one included,
/// for callers that already run on threads of their own.
pub(crate) fn capped<T>(threads: usize, f: impl FnOnce() -> T) -> T {
    struct Restore(Option<usize>);
    impl Drop for Restore {
        fn drop(&mut self) {
            CAP.with(|c| c.set(self.0));
        }
    }
    let _restore = Restore(CAP.with(|c| c.replace(Some(threads.max(1)))));
    f()
}
/// The thread limit of the [`capped`] call running on this thread, if any.
pub(crate) fn cap() -> Option<usize> {
    CAP.with(std::cell::Cell::get)
}

/// Threads for one construction step.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Workers {
    threads: usize,
    grain: usize,
}
impl Workers {
    /// Workers for a step over `work` similar units, decided on the calling thread.
    pub(crate) fn new(work: usize) -> Self {
        #[cfg(test)]
        if let Some(forced) = FORCED.with(std::cell::Cell::get) {
            return forced;
        }
        if work < INLINE {
            Self::with_threads(1)
        } else {
            Self::all()
        }
    }
    /// Every thread a step on this thread may use.
    pub(crate) fn all() -> Self {
        #[cfg(test)]
        if let Some(forced) = FORCED.with(std::cell::Cell::get) {
            return forced;
        }
        Self::with_threads(rayon::current_num_threads().min(cap().unwrap_or(usize::MAX)))
    }
    /// Exactly `threads` threads, the calling one included.
    pub(crate) fn with_threads(threads: usize) -> Self {
        Self {
            threads: threads.max(1),
            grain: GRAIN,
        }
    }
    /// Run `f` with steps split into tasks of `grain` units on `threads`
    /// threads, whatever their size.
    #[cfg(test)]
    pub(crate) fn forced<T>(threads: usize, grain: usize, f: impl FnOnce() -> T) -> T {
        struct Restore(Option<Workers>);
        impl Drop for Restore {
            fn drop(&mut self) {
                FORCED.with(|c| c.set(self.0));
            }
        }
        let forced = Self {
            threads: threads.max(1),
            grain: grain.max(1),
        };
        let _restore = Restore(FORCED.with(|c| c.replace(Some(forced))));
        f()
    }
    pub(crate) fn threads(&self) -> usize {
        self.threads
    }
    /// Consecutive ranges covering `0..len`, one per task.
    pub(crate) fn ranges(&self, len: usize) -> Vec<Range<usize>> {
        let tasks = if self.threads == 1 {
            1
        } else {
            (len / self.grain).clamp(1, self.threads * TASKS)
        };
        let size = len.div_ceil(tasks).max(1);
        (0..len)
            .step_by(size)
            .map(|start| start..(start + size).min(len))
            .collect()
    }
    /// `task(i)` for each `i` in `0..count`, results in order.
    pub(crate) fn run<R: Send>(&self, count: usize, task: impl Fn(usize) -> R + Sync) -> Vec<R> {
        let threads = self.threads.min(count);
        if threads <= 1 {
            return (0..count).map(task).collect();
        }
        let next = AtomicUsize::new(0);
        let failed = AtomicBool::new(false);
        let results = (0..count).map(|_| Mutex::new(None)).collect::<Vec<_>>();
        let work = || {
            panic::catch_unwind(AssertUnwindSafe(|| {
                while !failed.load(atomic::Ordering::Relaxed) {
                    let i = next.fetch_add(1, atomic::Ordering::Relaxed);
                    if i >= count {
                        break;
                    }
                    let result = task(i);
                    *results[i].lock().unwrap_or_else(PoisonError::into_inner) = Some(result);
                }
            }))
            .inspect_err(|_| failed.store(true, atomic::Ordering::Relaxed))
        };
        let token = cancel::current();
        let payload = std::thread::scope(|scope| {
            let handles = (1..threads)
                .filter_map(|_| {
                    let token = token.clone();
                    let work = &work;
                    // Without threads the calling thread does all the work.
                    std::thread::Builder::new()
                        .spawn_scoped(scope, move || {
                            cancel::scope(token, || {
                                #[cfg(test)]
                                let _live = cancel::hooks::Live::start();
                                work()
                            })
                        })
                        .ok()
                })
                .collect::<Vec<_>>();
            let mut payload = work().err();
            for handle in handles {
                match handle.join() {
                    Ok(Ok(())) => {}
                    Ok(Err(p)) | Err(p) => keep(&mut payload, p),
                }
            }
            payload
        });
        if let Some(payload) = payload {
            panic::resume_unwind(payload);
        }
        results
            .into_iter()
            .map(|r| {
                r.into_inner()
                    .unwrap_or_else(PoisonError::into_inner)
                    .expect("finished task")
            })
            .collect()
    }
    /// `task(item)` for each item, results in order.
    pub(crate) fn each<I: Send, R: Send>(
        &self,
        items: Vec<I>,
        task: impl Fn(I) -> R + Sync,
    ) -> Vec<R> {
        if self.threads.min(items.len()) <= 1 {
            return items.into_iter().map(task).collect();
        }
        let items = items
            .into_iter()
            .map(|i| Mutex::new(Some(i)))
            .collect::<Vec<_>>();
        self.run(items.len(), |i| {
            let item = items[i]
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take();
            task(item.expect("task item"))
        })
    }
    /// `task(range)` for consecutive ranges covering `0..len`, results in order.
    pub(crate) fn chunks<R: Send>(
        &self,
        len: usize,
        task: impl Fn(Range<usize>) -> R + Sync,
    ) -> Vec<R> {
        let ranges = self.ranges(len);
        self.run(ranges.len(), |i| task(ranges[i].clone()))
    }
    /// Set each element from its index.
    pub(crate) fn fill<T: Send>(&self, out: &mut [T], value: impl Fn(usize) -> T + Sync) {
        let ranges = self.ranges(out.len());
        let mut slices = Vec::with_capacity(ranges.len());
        let mut rest = out;
        for range in &ranges {
            let (head, tail) = rest.split_at_mut(range.len());
            slices.push((range.start, head));
            rest = tail;
        }
        self.each(slices, |(start, slice)| {
            for (k, slot) in slice.iter_mut().enumerate() {
                if k % GRAIN == 0 {
                    cancel::check();
                }
                *slot = value(start + k);
            }
        });
    }
    /// Sort unstably. Elements that compare equal must be interchangeable,
    /// so the order is the same for any number of threads.
    pub(crate) fn sort<T: Copy + Send + Sync>(
        &self,
        v: &mut Vec<T>,
        step: &'static str,
        compare: impl Fn(&T, &T) -> Ordering + Sync,
    ) {
        if self.threads == 1 || v.len() < 2 * self.grain.max(1) {
            sort(v, step, &compare);
            return;
        }
        // Buckets bounded by evenly spaced samples, sorted on their own.
        let buckets = self.threads * TASKS;
        let mut samples = v
            .iter()
            .step_by((v.len() / (buckets * 16)).max(1))
            .copied()
            .collect::<Vec<_>>();
        sort(&mut samples, step, &compare);
        let bounds = (1..buckets)
            .map(|k| samples[k * samples.len() / buckets])
            .collect::<Vec<_>>();
        let ranges = self.ranges(v.len());
        let source = &v[..];
        let classes = self.run(ranges.len(), |task| {
            let mut counts = vec![0; buckets];
            let ids = source[ranges[task].clone()]
                .iter()
                .enumerate()
                .map(|(k, x)| {
                    if k % GRAIN == 0 {
                        cancel::checkpoint(step);
                    }
                    let bucket = bounds.partition_point(|b| compare(b, x) == Ordering::Less);
                    counts[bucket] += 1;
                    bucket
                })
                .collect::<Vec<_>>();
            (ids, counts)
        });
        let mut out = v.clone();
        // Regions by bucket, then task, so each task fills its own regions.
        let mut regions = (0..ranges.len())
            .map(|_| Vec::with_capacity(buckets))
            .collect::<Vec<_>>();
        let mut sizes = vec![0; buckets];
        let mut rest = &mut out[..];
        for (bucket, size) in sizes.iter_mut().enumerate() {
            for (task, (_, counts)) in classes.iter().enumerate() {
                let (head, tail) = rest.split_at_mut(counts[bucket]);
                regions[task].push(head);
                rest = tail;
                *size += counts[bucket];
            }
        }
        self.each(
            regions.into_iter().enumerate().collect(),
            |(task, mut regions)| {
                let mut cursor = vec![0; buckets];
                for (x, &bucket) in source[ranges[task].clone()].iter().zip(&classes[task].0) {
                    regions[bucket][cursor[bucket]] = *x;
                    cursor[bucket] += 1;
                }
            },
        );
        let mut slices = Vec::with_capacity(buckets);
        let mut rest = &mut out[..];
        for size in sizes {
            let (head, tail) = rest.split_at_mut(size);
            slices.push(head);
            rest = tail;
        }
        self.each(slices, |slice| sort(slice, step, &compare));
        *v = out;
    }
}
/// Keep the first failure, but prefer a bug over cancellation.
fn keep(slot: &mut Option<Box<dyn Any + Send>>, payload: Box<dyn Any + Send>) {
    let replace = match slot {
        None => true,
        Some(old) => old.is::<cancel::Cancelled>() && !payload.is::<cancel::Cancelled>(),
    };
    if replace {
        *slot = Some(payload);
    }
}
/// Sort unstably on this thread, checking for cancellation as it goes.
pub(crate) fn sort<T>(v: &mut [T], step: &'static str, compare: impl Fn(&T, &T) -> Ordering) {
    let mut comparisons = 0_usize;
    v.sort_unstable_by(|a, b| {
        comparisons += 1;
        if comparisons.is_multiple_of(4 * GRAIN) {
            cancel::checkpoint(step);
        }
        compare(a, b)
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn results_keep_their_order_for_any_split() {
        let mut v = (0..10_000_u64)
            .map(|i| i.wrapping_mul(0x9e37_79b9_7f4a_7c15) % 997)
            .collect::<Vec<_>>();
        let mut expected = v.clone();
        expected.sort_unstable();
        for (threads, grain) in [(1, 1), (2, 1), (3, 7), (4, 100), (8, 4096)] {
            let workers = Workers::forced(threads, grain, || Workers::new(0));
            let sums = workers.chunks(v.len(), |r| r.sum::<usize>());
            assert_eq!(sums.iter().sum::<usize>(), (0..10_000_usize).sum::<usize>());
            assert_eq!(
                workers.run(100, |i| i * 2),
                (0..100).map(|i| i * 2).collect::<Vec<_>>()
            );
            let mut sorted = v.clone();
            workers.sort(&mut sorted, "sort", u64::cmp);
            assert_eq!(sorted, expected);
            let mut filled = vec![0; 1000];
            workers.fill(&mut filled, |i| i + 1);
            assert_eq!(filled, (1..=1000).collect::<Vec<_>>());
        }
        v.clear();
        Workers::forced(4, 1, || Workers::new(0)).sort(&mut v, "sort", u64::cmp);
        assert!(v.is_empty());
    }

    #[test]
    fn failures_reach_the_caller_and_bugs_win_over_cancellation() {
        let workers = Workers::forced(4, 1, || Workers::new(0));
        let token = cancel::CancellationToken::default();
        let result = panic::catch_unwind(|| {
            cancel::scope(Some(token.clone()), || {
                workers.run(64, |i| {
                    if i == 3 {
                        token.cancel();
                    }
                    if i == 40 {
                        panic::resume_unwind(Box::new("bug"));
                    }
                    cancel::check();
                })
            })
        });
        let payload = result.expect_err("failure");
        assert!(payload.is::<&str>() || payload.is::<cancel::Cancelled>());
        let mut slot = None;
        keep(&mut slot, Box::new(cancel::Cancelled));
        keep(&mut slot, Box::new("bug"));
        keep(&mut slot, Box::new(cancel::Cancelled));
        assert!(slot.expect("payload").is::<&str>());
    }
}
