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
    cell::Cell,
    cmp::Ordering,
    ops::Range,
    panic::{self, AssertUnwindSafe},
    sync::{
        Mutex, PoisonError,
        atomic::{self, AtomicBool, AtomicUsize},
        mpsc,
    },
};

/// Steps over fewer units run on the calling thread.
const INLINE: usize = 1 << 14;
/// Smallest task, in units.
const GRAIN: usize = 4096;
/// Most buckets of a parallel sort, so bucket numbers fit in `u16`.
const BUCKETS: usize = 1 << 16;
/// Tasks per thread, so uneven tasks still share the work.
const TASKS: usize = 8;

#[cfg(test)]
thread_local! {
    static FORCED: std::cell::Cell<Option<Workers>> = const { std::cell::Cell::new(None) };
}
type Setting = std::thread::LocalKey<std::cell::Cell<Option<usize>>>;
thread_local! {
    static CAP: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
    /// Within a task's own steps, the threads of the caches it shares with other tasks.
    static SHARED: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

/// Run `f` as one of several tasks that already run on threads of their own: each
/// of its steps uses at most `threads` threads, the calling one included, and the
/// caches it shares with the other tasks build on `shared` threads.
pub(crate) fn task<T>(threads: usize, shared: usize, f: impl FnOnce() -> T) -> T {
    with(&SHARED, Some(shared.max(1)), || {
        with(&CAP, Some(threads.max(1)), f)
    })
}
/// Whether this thread runs a [`task`]'s own steps, so caches made now are its own.
pub(crate) fn local() -> bool {
    SHARED.with(std::cell::Cell::get).is_some()
}
/// Build a cache that [`task`]s share on their `shared` threads, whichever task needs
/// it first and whatever threads that task holds, so the build is the same each time.
pub(crate) fn shared<T>(f: impl FnOnce() -> T) -> T {
    match SHARED.with(std::cell::Cell::get) {
        Some(threads) => with(&SHARED, None, || with(&CAP, Some(threads), f)),
        None => f(),
    }
}
fn with<T>(setting: &'static Setting, value: Option<usize>, f: impl FnOnce() -> T) -> T {
    struct Restore(&'static Setting, Option<usize>);
    impl Drop for Restore {
        fn drop(&mut self) {
            self.0.with(|c| c.set(self.1));
        }
    }
    let _restore = Restore(setting, setting.with(|c| c.replace(value)));
    f()
}
/// The thread limit of the [`task`] step running on this thread, if any.
pub(crate) fn cap() -> Option<usize> {
    CAP.with(std::cell::Cell::get)
}

/// Call `f` with a function that runs `job` on a thread of its own and waits for its
/// result. The allocator gives each thread an arena of its own, so the scratch of the
/// jobs stays apart from what this thread keeps, such as the caches it built: scratch
/// that lands in their gaps takes much longer to reach. The thread starts with the
/// first job and serves the caller's cancellation token and settings. If it cannot
/// start, jobs run on the calling thread.
pub(crate) fn apart<J: Send, R: Send, T>(
    job: impl Fn(J) -> R + Sync,
    f: impl FnOnce(&mut dyn FnMut(J) -> R) -> T,
) -> T {
    let job = &job;
    std::thread::scope(|scope| {
        // Dropped as `f` returns or unwinds, which ends the thread before the scope waits.
        let mut thread = None;
        let mut started = false;
        let mut run = |input: J| {
            if !started {
                started = true;
                let (jobs, inputs) = mpsc::channel::<J>();
                let (outputs, results) = mpsc::channel::<R>();
                let token = cancel::current();
                let settings = (CAP.with(Cell::get), SHARED.with(Cell::get));
                #[cfg(test)]
                let forced = FORCED.with(Cell::get);
                thread = std::thread::Builder::new()
                    .spawn_scoped(scope, move || {
                        #[cfg(test)]
                        FORCED.with(|c| c.set(forced));
                        cancel::scope(token, || {
                            #[cfg(test)]
                            let _live = cancel::hooks::Live::start();
                            with(&CAP, settings.0, || {
                                with(&SHARED, settings.1, || {
                                    for input in inputs {
                                        if outputs.send(job(input)).is_err() {
                                            break;
                                        }
                                    }
                                })
                            });
                        });
                    })
                    .ok()
                    .map(|handle| (jobs, results, handle));
            }
            let Some((jobs, results, _)) = &thread else {
                return job(input);
            };
            // The thread only stops early when a job fails, so its payload is the error.
            if jobs.send(input).is_ok()
                && let Ok(result) = results.recv()
            {
                return result;
            }
            let (_, _, handle) = thread.take().expect("job thread");
            match handle.join() {
                Ok(()) => unreachable!("the job thread stops only when its jobs end"),
                Err(payload) => panic::resume_unwind(payload),
            }
        };
        f(&mut run)
    })
}

/// Threads for one construction step.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Workers {
    threads: usize,
    grain: usize,
}
impl Workers {
    /// Workers for a step over `work` similar units, decided on the calling thread.
    pub(crate) fn new(work: usize) -> Self {
        #[cfg(test)]
        if let Some(forced) = Self::forced_here() {
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
        if let Some(forced) = Self::forced_here() {
            return forced;
        }
        // A task's helper threads are outside its pool, so its limit is all they know.
        Self::with_threads(cap().unwrap_or_else(rayon::current_num_threads))
    }
    /// The workers of a [`forced`](Self::forced) call on this thread, within the
    /// thread limit of a [`task`].
    #[cfg(test)]
    pub(crate) fn forced_here() -> Option<Self> {
        FORCED.with(std::cell::Cell::get).map(|forced| Self {
            threads: forced.threads.min(cap().unwrap_or(usize::MAX)),
            ..forced
        })
    }
    /// Exactly `threads` threads, the calling one included.
    pub(crate) fn with_threads(threads: usize) -> Self {
        Self {
            threads: threads.max(1),
            grain: GRAIN,
        }
    }
    /// Run `f` with steps split into tasks of `grain` units on `threads`
    /// threads, whatever their size. The threads of its steps do the same.
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
        self.run_with(count, || (), |(), i| task(i))
    }
    /// `task(state, i)` for each `i` in `0..count`, results in order, with `state`
    /// made once on each thread that runs tasks, so small tasks need not each set up
    /// what they share.
    pub(crate) fn run_with<S, R: Send>(
        &self,
        count: usize,
        state: impl Fn() -> S + Sync,
        task: impl Fn(&mut S, usize) -> R + Sync,
    ) -> Vec<R> {
        let threads = self.threads.min(count);
        if threads <= 1 {
            let mut held = None;
            return (0..count)
                .map(|i| task(held.get_or_insert_with(&state), i))
                .collect();
        }
        let next = AtomicUsize::new(0);
        let failed = AtomicBool::new(false);
        let results = (0..count).map(|_| Mutex::new(None)).collect::<Vec<_>>();
        // Within a capped step, each task keeps its nested steps to its own thread.
        let nested = cap().map(|_| 1);
        let work = || {
            with(&CAP, nested, || {
                panic::catch_unwind(AssertUnwindSafe(|| {
                    let mut held = None;
                    while !failed.load(atomic::Ordering::Relaxed) {
                        let i = next.fetch_add(1, atomic::Ordering::Relaxed);
                        if i >= count {
                            break;
                        }
                        let result = task(held.get_or_insert_with(&state), i);
                        *results[i].lock().unwrap_or_else(PoisonError::into_inner) = Some(result);
                    }
                }))
                .inspect_err(|_| failed.store(true, atomic::Ordering::Relaxed))
            })
        };
        let token = cancel::current();
        let shared = SHARED.with(std::cell::Cell::get);
        #[cfg(test)]
        let forced = FORCED.with(std::cell::Cell::get);
        let payload = std::thread::scope(|scope| {
            let handles = (1..threads)
                .filter_map(|_| {
                    let token = token.clone();
                    let work = &work;
                    // Without threads the calling thread does all the work.
                    std::thread::Builder::new()
                        .spawn_scoped(scope, move || {
                            #[cfg(test)]
                            FORCED.with(|c| c.set(forced));
                            cancel::scope(token, || {
                                #[cfg(test)]
                                let _live = cancel::hooks::Live::start();
                                with(&SHARED, shared, work)
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
        self.update(out, |i, slot| *slot = value(i));
    }
    /// `change(index, element)` for each element.
    pub(crate) fn update<T: Send>(&self, out: &mut [T], change: impl Fn(usize, &mut T) + Sync) {
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
                cancel::every(k, "fill");
                change(start + k, slot);
            }
        });
    }
    /// Sort unstably. Elements that compare equal must be interchangeable,
    /// so the order is the same for any number of threads. `step` names the
    /// sort's first checkpoint, and each stage has its own.
    pub(crate) fn sort<T: Copy + Send + Sync>(
        &self,
        v: &mut Vec<T>,
        step: &'static str,
        compare: impl Fn(&T, &T) -> Ordering + Sync,
    ) {
        cancel::checkpoint(step);
        if self.threads == 1 || v.len() < 2 * self.grain.max(1) {
            sort(v, "sort.serial", &compare);
            return;
        }
        // Buckets bounded by evenly spaced samples, sorted on their own.
        let buckets = (self.threads * TASKS).min(BUCKETS);
        let mut samples = v
            .iter()
            .step_by((v.len() / (buckets * 16)).max(1))
            .copied()
            .collect::<Vec<_>>();
        sort(&mut samples, "sort.samples", &compare);
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
                    cancel::every(k, "sort.classes");
                    let bucket = bounds.partition_point(|b| compare(b, x) == Ordering::Less);
                    counts[bucket] += 1;
                    u16::try_from(bucket).expect("bucket number")
                })
                .collect::<Vec<_>>();
            (ids, counts)
        });
        // Safe code needs an initialised destination; a copy is cheaper than
        // anything else that fills it.
        let mut out = copied(v, "sort.copy");
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
                // Moving elements runs at copying speed: one check per task.
                cancel::checkpoint("sort.moves");
                let mut cursor = vec![0; buckets];
                for (x, &bucket) in source[ranges[task].clone()].iter().zip(&classes[task].0) {
                    let bucket = usize::from(bucket);
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
        self.each(slices, |slice| sort(slice, "sort.buckets", &compare));
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
/// `len` copies of `value`, checking for cancellation as it goes.
pub(crate) fn repeated<T: Clone>(value: T, len: usize, step: &'static str) -> Vec<T> {
    let mut out = Vec::with_capacity(len);
    while out.len() < len {
        cancel::checkpoint(step);
        out.resize((out.len() + cancel::COPY).min(len), value.clone());
    }
    out
}
/// A copy of `v`, checking for cancellation as it goes.
pub(crate) fn copied<T: Clone>(v: &[T], step: &'static str) -> Vec<T> {
    let mut out = Vec::with_capacity(v.len());
    for chunk in v.chunks(cancel::COPY) {
        cancel::checkpoint(step);
        out.extend_from_slice(chunk);
    }
    out
}
/// Sort unstably on this thread, checking for cancellation as it goes.
pub(crate) fn sort<T>(v: &mut [T], step: &'static str, compare: impl Fn(&T, &T) -> Ordering) {
    let mut comparisons = 0_usize;
    v.sort_unstable_by(|a, b| {
        cancel::every(comparisons, step);
        comparisons += 1;
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
        for len in [0, 1, cancel::COPY, cancel::COPY + 1] {
            let ascending = (0..len).collect::<Vec<_>>();
            assert_eq!(copied(&ascending, "copied"), ascending);
            assert_eq!(repeated(3, len, "repeated"), vec![3; len]);
        }
        v.clear();
        Workers::forced(4, 1, || Workers::new(0)).sort(&mut v, "sort", u64::cmp);
        assert!(v.is_empty());
    }

    #[test]
    fn tasks_share_a_state_per_thread() {
        for threads in [1, 4] {
            let made = Mutex::new(Vec::new());
            let state = || {
                let thread = std::thread::current().id();
                made.lock().unwrap().push(thread);
                (thread, 0)
            };
            // Each task counts itself in the state of the thread it runs on.
            let found = Workers::with_threads(threads).run_with(200, state, |state, i| {
                assert_eq!(state.0, std::thread::current().id());
                state.1 += 1;
                (i * 2, state.0, state.1)
            });
            assert_eq!(
                found.iter().map(|f| f.0).collect::<Vec<_>>(),
                (0..200).map(|i| i * 2).collect::<Vec<_>>()
            );
            let made = made.into_inner().unwrap();
            assert!((1..=threads).contains(&made.len()));
            let mut ran = 0;
            for thread in &made {
                let mut counts = found
                    .iter()
                    .filter(|f| f.1 == *thread)
                    .map(|f| f.2)
                    .collect::<Vec<_>>();
                counts.sort_unstable();
                assert_eq!(counts, (1..=counts.len()).collect::<Vec<_>>());
                ran += counts.len();
            }
            assert_eq!(ran, 200);
        }
        // Without tasks, no state is made.
        let made = AtomicUsize::new(0);
        let make = || {
            made.fetch_add(1, atomic::Ordering::Relaxed);
        };
        for threads in [1, 4] {
            Workers::with_threads(threads).run_with(0, make, |(), i| i);
        }
        assert_eq!(made.into_inner(), 0);
    }

    /// The payload `run` fails with, on a token for its tasks.
    fn failure(token: &cancel::CancellationToken, run: impl FnOnce()) -> Box<dyn Any + Send> {
        panic::catch_unwind(AssertUnwindSafe(|| {
            cancel::scope(Some(token.clone()), run);
        }))
        .expect_err("failure")
    }

    #[test]
    fn failures_reach_the_caller_and_bugs_win_over_cancellation() {
        let workers = Workers::forced(4, 1, || Workers::new(0));
        let token = cancel::CancellationToken::default();
        let payload = failure(&token, || {
            workers.run(64, |i| {
                if i == 3 {
                    token.cancel();
                }
                cancel::check();
            });
        });
        assert!(payload.is::<cancel::Cancelled>());
        // Both tasks run at once, so both fail, whichever thread takes which.
        let token = cancel::CancellationToken::default();
        let both = std::sync::Barrier::new(2);
        let payload = failure(&token, || {
            workers.run(2, |i| {
                both.wait();
                if i == 0 {
                    token.cancel();
                    cancel::check();
                }
                panic::resume_unwind(Box::new("bug"));
            });
        });
        assert_eq!(payload.downcast_ref::<&str>(), Some(&"bug"));
        let mut slot = None;
        keep(&mut slot, Box::new(cancel::Cancelled));
        keep(&mut slot, Box::new("bug"));
        keep(&mut slot, Box::new(cancel::Cancelled));
        assert_eq!(slot.expect("payload").downcast_ref::<&str>(), Some(&"bug"));
    }

    #[test]
    fn jobs_apart_run_in_turn_on_one_thread_of_the_request() {
        let caller = std::thread::current().id();
        let token = cancel::hooks::watched(None);
        let found = cancel::scope(Some(token.clone()), || {
            Workers::forced(3, 1, || {
                apart(
                    |i: usize| {
                        let token = cancel::current().map(|t| cancel::hooks::record(&t).live);
                        (
                            i * 2,
                            std::thread::current().id(),
                            token,
                            Workers::all().threads(),
                        )
                    },
                    |run| (0..5).map(run).collect::<Vec<_>>(),
                )
            })
        });
        assert_eq!(
            found.iter().map(|f| f.0).collect::<Vec<_>>(),
            [0, 2, 4, 6, 8]
        );
        assert!(found.iter().all(|f| f.1 == found[0].1 && f.1 != caller));
        // The thread serves the request's token, as the one worker it has, and settings.
        assert!(found.iter().all(|f| f.2 == Some(1) && f.3 == 3));
        let record = cancel::hooks::record(&token);
        assert_eq!((record.workers, record.live), (1, 0));
        // Without jobs no thread starts.
        let token = cancel::hooks::watched(None);
        cancel::scope(Some(token.clone()), || apart(|()| (), |_| ()));
        assert_eq!(cancel::hooks::record(&token).workers, 0);
    }

    #[test]
    fn failures_apart_reach_the_caller_and_end_the_thread() {
        // A job's panic, cancellation in a job and a panic of the caller between jobs.
        let token = cancel::hooks::watched(None);
        let payload = failure(&token, || {
            apart(
                |i: usize| {
                    if i == 2 {
                        panic::resume_unwind(Box::new("job failed"));
                    }
                },
                |run| (0..4).for_each(run),
            );
        });
        assert_eq!(payload.downcast_ref::<&str>(), Some(&"job failed"));
        let payload = failure(&token, || {
            apart(
                |()| cancel::check(),
                |run| {
                    run(());
                    token.cancel();
                    run(());
                },
            );
        });
        assert!(payload.is::<cancel::Cancelled>());
        let token = cancel::hooks::watched(None);
        let payload = failure(&token, || {
            apart(
                |()| (),
                |run| {
                    run(());
                    panic::resume_unwind(Box::new("caller failed"));
                },
            );
        });
        assert_eq!(payload.downcast_ref::<&str>(), Some(&"caller failed"));
        let record = cancel::hooks::record(&token);
        assert_eq!((record.workers, record.live), (1, 0));
    }

    #[test]
    fn small_steps_run_on_the_calling_thread() {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(4)
            .build()
            .expect("pool");
        for (work, threads) in [(0, 1), (1000, 1), (INLINE - 1, 1), (INLINE, 4)] {
            let token = cancel::hooks::watched(None);
            let workers = pool.install(|| Workers::new(work));
            assert_eq!(workers.threads(), threads, "{work}");
            cancel::scope(Some(token.clone()), || workers.chunks(work, |r| r.len()));
            let record = cancel::hooks::record(&token);
            assert_eq!(record.workers, threads - 1, "{work}");
        }
    }

    #[test]
    fn steps_use_the_threads_of_their_pool_or_task() {
        let global = rayon::current_num_threads();
        let threads = |work| (Workers::new(work).threads(), Workers::all().threads());
        for size in [2, global + 3] {
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(size)
                .build()
                .expect("pool");
            assert_eq!(pool.install(|| threads(INLINE)), (size, size));
            // A task's limit holds whatever its pool, and shared caches use their own.
            let found =
                pool.install(|| task(3, 5, || (threads(INLINE), shared(|| threads(INLINE)))));
            assert_eq!(found, ((3, 3), (5, 5)), "{size}");
        }
        let plain = std::thread::spawn(move || threads(INLINE)).join().unwrap();
        assert_eq!(plain, (global, global));
    }

    #[test]
    fn helpers_of_a_task_step_keep_its_shared_threads() {
        // Each helper takes one of the tasks, as they wait for each other.
        let all = std::sync::Barrier::new(3);
        let found = task(3, 5, || {
            Workers::with_threads(3).run(3, |_| {
                all.wait();
                (
                    std::thread::current().id(),
                    local(),
                    Workers::all().threads(),
                    shared(|| Workers::all().threads()),
                )
            })
        });
        let ids = found
            .iter()
            .map(|f| f.0)
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(ids.len(), 3);
        assert!(
            found.iter().all(|f| (f.1, f.2, f.3) == (true, 1, 5)),
            "{found:?}"
        );
    }

    #[test]
    fn long_steps_stop_at_their_checkpoints() {
        let v = (0..1_u64 << 16)
            .map(|i| i.wrapping_mul(0x9e37_79b9_7f4a_7c15))
            .collect::<Vec<_>>();
        let stages = [
            "sort",
            "sort.buckets",
            "sort.classes",
            "sort.copy",
            "sort.moves",
            "sort.samples",
        ];
        for threads in [1, 4] {
            let workers = Workers::forced(threads, 64, || Workers::new(0));
            let sorted: &[&str] = if threads == 1 {
                &["sort", "sort.serial"]
            } else {
                &stages
            };
            let steps: [(&[&str], &dyn Fn()); 5] = [
                (&["fill"], &|| workers.fill(&mut vec![0; v.len()], |i| v[i])),
                (sorted, &|| workers.sort(&mut v.clone(), "sort", u64::cmp)),
                (&["serial"], &|| sort(&mut v.clone(), "serial", u64::cmp)),
                (&["repeated"], &|| drop(repeated(1, 1 << 20, "repeated"))),
                (&["copied"], &|| drop(copied(&v, "copied"))),
            ];
            for (expected, run) in steps {
                // Every checkpoint is passed, so one taken out shows.
                let token = cancel::hooks::watched(None);
                cancel::scope(Some(token.clone()), run);
                let visits = cancel::hooks::record(&token).visits;
                assert!(visits.keys().eq(expected), "{expected:?} on {threads}");
                for (step, count) in visits {
                    for visit in [0, count / 2] {
                        let token = cancel::hooks::watched(Some((step, visit)));
                        assert!(failure(&token, run).is::<cancel::Cancelled>(), "{step}");
                        let record = cancel::hooks::record(&token);
                        assert_eq!(record.live, 0);
                        // Each other thread passes at most one more checkpoint.
                        let after = record.visits[step];
                        assert!(
                            (visit + 1..=visit + threads).contains(&after),
                            "{step} {visit} on {threads} threads: {after}"
                        );
                    }
                }
            }
        }
    }
}
