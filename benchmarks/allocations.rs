//! Measure allocation requests and live Rust heap for a single-threaded scope.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::hint::black_box;

#[derive(Clone, Copy, Default)]
pub(super) struct Requests {
    pub(super) calls: u64,
    pub(super) bytes: u64,
    pub(super) live_bytes: u64,
    pub(super) peak_live_bytes: u64,
    unbalanced: bool,
}

struct InstrumentedAllocator;

thread_local! {
    static REQUESTS: Cell<Option<Requests>> = const { Cell::new(None) };
}

fn record(requested: Option<usize>, released: usize, acquired: usize) {
    let _ = REQUESTS.try_with(|requests| {
        if let Some(mut count) = requests.get() {
            if let Some(bytes) = requested {
                count.calls = count.calls.saturating_add(1);
                count.bytes = count.bytes.saturating_add(bytes as u64);
            }
            if let Some(live) = count
                .live_bytes
                .checked_sub(released as u64)
                .and_then(|remaining| remaining.checked_add(acquired as u64))
            {
                count.live_bytes = live;
                count.peak_live_bytes = count.peak_live_bytes.max(live);
            } else {
                count.unbalanced = true;
            }
            requests.set(Some(count));
        }
    });
}

// SAFETY: All operations forward their arguments to System unchanged.
// The counter is thread-local and its updates neither allocate nor unwind.
unsafe impl GlobalAlloc for InstrumentedAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        let size = layout.size();
        record(Some(size), 0, if ptr.is_null() { 0 } else { size });
        ptr
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc_zeroed(layout) };
        let size = layout.size();
        record(Some(size), 0, if ptr.is_null() { 0 } else { size });
        ptr
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let resized = unsafe { System.realloc(ptr, layout, new_size) };
        if resized.is_null() {
            record(Some(new_size), 0, 0);
        } else {
            // Count the new live block in place of the old one, independent of
            // whether System moved it or kept it at the same address.
            record(Some(new_size), layout.size(), new_size);
        }
        resized
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
        record(None, layout.size(), 0);
    }
}

#[global_allocator]
static ALLOCATOR: InstrumentedAllocator = InstrumentedAllocator;

/// The measured operation may borrow existing allocations. It must only resize
/// or release blocks acquired inside this scope.
pub(super) fn track<T>(run: impl FnOnce() -> T) -> (T, Requests) {
    REQUESTS.with(|requests| assert!(requests.replace(Some(Requests::default())).is_none()));
    let result = run();
    let counts = REQUESTS.with(|requests| requests.replace(None).unwrap());
    assert!(!counts.unbalanced, "unbalanced heap measurement");
    (result, counts)
}

pub(super) fn verify_tracking() {
    let (retained, probe) = track(|| {
        let mut growing = black_box(Vec::<u8>::with_capacity(13));
        let temporary = black_box(vec![0u8; 7]);
        growing.reserve_exact(29);
        drop(temporary);
        growing.shrink_to(17);
        black_box(growing)
    });
    assert_eq!((probe.calls, probe.bytes), (4, 66));
    assert_eq!(probe.peak_live_bytes, 36);
    assert_eq!(probe.live_bytes, 17);
    assert_eq!(probe.live_bytes, retained.capacity() as u64);
}
