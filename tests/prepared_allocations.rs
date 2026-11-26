use pkbyte::{build, BuildMode, BuildOptions, Picker, PrepareOptions, SamplingMethod, Table};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::hint::black_box;

std::thread_local! {
    // Count the test's current thread only. A const initializer avoids heap
    // allocation when the allocator itself first accesses this counter.
    static ALLOCATIONS: Cell<Option<usize>> = const { Cell::new(None) };
}

fn count_allocation() {
    let _ = ALLOCATIONS.try_with(|counter| {
        if let Some(count) = counter.get() {
            counter.set(Some(count.saturating_add(1)));
        }
    });
}

struct CountingAllocator;

// SAFETY: All allocation and deallocation requests are forwarded to System
// unchanged. Counting uses thread-local state and never unwinds.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count_allocation();
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        count_allocation();
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        count_allocation();
        unsafe { System.realloc(ptr, layout, new_size) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

fn tracked_allocations(run: impl FnOnce()) -> usize {
    ALLOCATIONS.with(|counter| counter.set(Some(0)));
    run();
    ALLOCATIONS.with(|counter| counter.replace(None).unwrap())
}

#[test]
fn prepared_get_and_sampling_do_not_allocate_after_setup() {
    let probe = tracked_allocations(|| {
        let buffer = Vec::<u8>::with_capacity(128);
        black_box(&buffer);
    });
    assert!(probe > 0, "allocation counter did not see the probe");

    let mut values: Vec<_> = (0..128)
        .map(|index| format!("aaaaaaaaaaaaaaaaaaaaaaaa{index:08b}bbbbbbbbbbbbbbbbbbbbbbbb"))
        .collect();
    values.extend(["", "🦀\0\n"].map(str::to_owned));
    for mode in [
        BuildMode::Phrase,
        BuildMode::EliasFanoMixed,
        BuildMode::EliasFanoPowerOfTwo,
    ] {
        let packed = build(
            &values,
            &BuildOptions {
                mode,
                ..BuildOptions::default()
            },
        )
        .unwrap();
        assert!(packed.stats.raw_rows >= 2);
        match mode {
            BuildMode::Phrase => assert!(packed.stats.compressed_rows > 0),
            BuildMode::EliasFanoMixed => assert!(packed.stats.ef_mixed_rows > 0),
            BuildMode::EliasFanoPowerOfTwo => assert!(packed.stats.ef_power_of_two_rows > 0),
            _ => unreachable!(),
        }
        let table = Table::open(&packed.bytes).unwrap();

        for (sampling, max_hint_bytes) in [
            (SamplingMethod::MultiplyHigh, 4096),
            (SamplingMethod::Modulo, 0),
        ] {
            let prepared = table
                .prepare(&PrepareOptions {
                    max_hint_bytes,
                    sampling,
                })
                .unwrap();
            let mut output = prepared.new_output_buffer().unwrap();
            let mut state = 42_u64;
            let mut next_u64 = || {
                state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
                state
            };

            let allocations = tracked_allocations(|| {
                for rank in 0..prepared.len() {
                    black_box(prepared.get_into(rank, &mut output).unwrap());
                }
                for _ in 0..4096 {
                    black_box(prepared.sample_into(&mut next_u64, &mut output).unwrap());
                }
            });
            assert_eq!(allocations, 0);
        }

        let mut picker = Picker::from_bytes(packed.bytes).unwrap();
        picker.set_seed(42);
        let mut output = Vec::new();
        picker.pick_into(&mut output).unwrap();
        let allocations = tracked_allocations(|| {
            for pick in 0..4096 {
                if pick % 2 == 0 {
                    output.clear();
                }
                black_box(picker.pick_into(&mut output).unwrap());
            }
        });
        assert_eq!(allocations, 0);
    }
}
