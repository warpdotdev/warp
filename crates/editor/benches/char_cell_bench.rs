use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use string_offset::CharOffset;
use warp_editor::render::model::CharCellState;

struct CountingAllocator;

static COUNT_ALLOCATIONS: AtomicBool = AtomicBool::new(false);
static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);
static LIVE_BYTES: AtomicUsize = AtomicUsize::new(0);
static PEAK_BYTES: AtomicUsize = AtomicUsize::new(0);

fn record_allocation(bytes: usize) {
    ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
    let live = LIVE_BYTES.fetch_add(bytes, Ordering::Relaxed) + bytes;
    PEAK_BYTES.fetch_max(live, Ordering::Relaxed);
}

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() && COUNT_ALLOCATIONS.load(Ordering::Relaxed) {
            record_allocation(layout.size());
        }
        ptr
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc_zeroed(layout) };
        if !ptr.is_null() && COUNT_ALLOCATIONS.load(Ordering::Relaxed) {
            record_allocation(layout.size());
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if COUNT_ALLOCATIONS.load(Ordering::Relaxed) {
            LIVE_BYTES.fetch_sub(layout.size(), Ordering::Relaxed);
        }
        unsafe { System.dealloc(ptr, layout) };
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let new_ptr = unsafe { System.realloc(ptr, layout, new_size) };
        if !new_ptr.is_null() && COUNT_ALLOCATIONS.load(Ordering::Relaxed) {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
            if new_size >= layout.size() {
                let added = new_size - layout.size();
                let live = LIVE_BYTES.fetch_add(added, Ordering::Relaxed) + added;
                PEAK_BYTES.fetch_max(live, Ordering::Relaxed);
            } else {
                LIVE_BYTES.fetch_sub(layout.size() - new_size, Ordering::Relaxed);
            }
        }
        new_ptr
    }
}

#[global_allocator]
static GLOBAL_ALLOCATOR: CountingAllocator = CountingAllocator;

#[derive(Clone, Copy)]
struct AllocationStats {
    allocations: usize,
    live_bytes: usize,
    peak_bytes: usize,
}

fn measure_allocations<T>(f: impl FnOnce() -> T) -> (T, AllocationStats) {
    ALLOCATIONS.store(0, Ordering::Relaxed);
    LIVE_BYTES.store(0, Ordering::Relaxed);
    PEAK_BYTES.store(0, Ordering::Relaxed);
    COUNT_ALLOCATIONS.store(true, Ordering::Relaxed);
    let value = f();
    COUNT_ALLOCATIONS.store(false, Ordering::Relaxed);
    (
        value,
        AllocationStats {
            allocations: ALLOCATIONS.load(Ordering::Relaxed),
            live_bytes: LIVE_BYTES.load(Ordering::Relaxed),
            peak_bytes: PEAK_BYTES.load(Ordering::Relaxed),
        },
    )
}

fn short_lines(count: usize) -> String {
    let mut text = String::with_capacity(count * 32);
    for line in 0..count {
        text.push_str("fn short_line_");
        text.push_str(&(line % 1000).to_string());
        text.push_str("() {}\n");
    }
    text
}

fn long_line(chars: usize) -> String {
    "word-with-breaks ".repeat(chars.div_ceil(17))
}

fn unicode_lines(count: usize) -> String {
    "你好 café a\u{301} hello-world 🚀\n".repeat(count)
}

fn dense_ghosts(line_count: usize) -> Vec<(String, usize)> {
    (0..line_count)
        .step_by(10)
        .map(|line| (format!("removed content at line {line}\n"), line))
        .collect()
}

fn hidden_ranges(line_count: usize) -> Vec<std::ops::Range<usize>> {
    (20..line_count.saturating_sub(20))
        .step_by(100)
        .map(|start| start..(start + 20).min(line_count))
        .collect()
}

fn bench_dataset(
    group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    name: &str,
    text: String,
    logical_lines: usize,
) {
    let char_count = text.chars().count();
    group.throughput(Throughput::Elements(char_count as u64));

    let state = CharCellState::new_for_test(80);
    let (_, vector_allocations) = measure_allocations(|| state.update_text(&text));
    let retained_bytes = state.text_index_retained_bytes();
    let dataset = format!(
        "{name}/chars={char_count}/retained={retained_bytes}B/allocs={}/live={}B/peak={}B",
        vector_allocations.allocations,
        vector_allocations.live_bytes,
        vector_allocations.peak_bytes,
    );

    group.bench_with_input(
        BenchmarkId::new("rebuild", &dataset),
        &text,
        |bench, text| bench.iter(|| state.update_text(black_box(text))),
    );

    group.bench_function(BenchmarkId::new("max_line", &dataset), |bench| {
        bench.iter(|| black_box(state.max_line()))
    });

    let target_offset = CharOffset::from(char_count.saturating_sub(1));
    group.bench_function(BenchmarkId::new("offset_to_point", &dataset), |bench| {
        bench.iter(|| black_box(state.offset_to_softwrap_point(black_box(target_offset))))
    });

    let target_point = state.offset_to_softwrap_point(target_offset);
    group.bench_function(BenchmarkId::new("point_to_offset", &dataset), |bench| {
        bench.iter(|| black_box(state.softwrap_point_to_offset(black_box(target_point))))
    });

    group.bench_function(BenchmarkId::new("visual_row_range", &dataset), |bench| {
        bench.iter(|| black_box(state.visual_row_char_range(black_box(target_offset))))
    });

    state.set_test_temporary_blocks(dense_ghosts(logical_lines));
    let hidden = hidden_ranges(logical_lines);
    group.bench_function(BenchmarkId::new("display_lattice", &dataset), |bench| {
        bench.iter(|| {
            let lattice = state.display_lattice(black_box(&hidden));
            black_box(lattice.rows().len())
        })
    });

    group.bench_function(BenchmarkId::new("resize_and_max_line", &dataset), |bench| {
        bench.iter(|| {
            let width = if state.terminal_width() == 80 { 81 } else { 80 };
            state.set_terminal_width(width);
            black_box(state.max_line())
        })
    });
}

fn char_cell_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("char_cell");
    bench_dataset(&mut group, "short_10k", short_lines(10_000), 10_001);
    bench_dataset(&mut group, "short_100k", short_lines(100_000), 100_001);
    bench_dataset(&mut group, "long_100k", long_line(100_000), 1);
    bench_dataset(&mut group, "unicode_10k", unicode_lines(10_000), 10_001);
    group.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .sample_size(10)
        .warm_up_time(Duration::from_secs(1))
        .measurement_time(Duration::from_secs(1));
    targets = char_cell_benchmarks
}
criterion_main!(benches);
