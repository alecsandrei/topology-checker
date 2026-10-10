//! Allocation counting is isolated from the safe library and Criterion timing runs.
mod support;
use std::{
    alloc::{GlobalAlloc, Layout, System},
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

struct Counting;
static ACTIVE: AtomicBool = AtomicBool::new(false);
static CALLS: AtomicUsize = AtomicUsize::new(0);
static BYTES: AtomicUsize = AtomicUsize::new(0);

// SAFETY: Every operation forwards the unchanged layout and pointer to System.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count(layout.size());
        // SAFETY: The caller supplies a valid allocation layout.
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        count(layout.size());
        // SAFETY: The caller supplies a valid allocation layout.
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: The caller supplies the original pointer and layout.
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        count(size);
        // SAFETY: The caller supplies a live pointer, its layout and a valid new size.
        unsafe { System.realloc(ptr, layout, size) }
    }
}
fn count(size: usize) {
    if ACTIVE.load(Ordering::Relaxed) {
        CALLS.fetch_add(1, Ordering::Relaxed);
        BYTES.fetch_add(size, Ordering::Relaxed);
    }
}
#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn measure<T>(name: &str, f: impl FnOnce() -> T) {
    CALLS.store(0, Ordering::Relaxed);
    BYTES.store(0, Ordering::Relaxed);
    ACTIVE.store(true, Ordering::Relaxed);
    let value = std::hint::black_box(f());
    ACTIVE.store(false, Ordering::Relaxed);
    println!(
        "{name}: calls={} requested_bytes={}",
        CALLS.load(Ordering::Relaxed),
        BYTES.load(Ordering::Relaxed)
    );
    drop(value);
}
fn main() {
    // Initialize Rayon before counting; use RAYON_NUM_THREADS=1 for repeatable counts.
    rayon::ThreadPoolBuilder::new().build_global().unwrap();
    rayon::broadcast(|_| {});
    let input = support::multipart();
    measure("multipart", || support::clean(input));
    let input = support::aggregation();
    measure("aggregation", || support::aggregate(input));
    let input = support::long_lines();
    measure("long-lines", || support::merge(input));
}
