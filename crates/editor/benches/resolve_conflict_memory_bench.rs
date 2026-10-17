//! Measures peak/net allocation and wall-clock time for the two candidate
//! implementations of `GlobalBufferModel::resolve_conflict`'s content-apply
//! step: the old `Buffer::replace_all` (full-buffer replace) versus the new
//! `text_diff_sync` + `Buffer::insert_at_char_offset_ranges` (scoped diff).
//!
//! This is a manual, non-criterion-harness bench (`harness = false`) so it
//! can print a comparison table directly. Run with:
//!   cargo bench -p warp_editor --bench resolve_conflict_memory_bench

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use instant::Instant;
use warp_editor::content::buffer::Buffer;
use warp_editor::content::diff::text_diff_sync;
use warp_editor::content::selection_model::BufferSelectionModel;
use warp_editor::content::text::IndentBehavior;
use warp_util::content_version::ContentVersion;
use warpui_core::{App, ModelHandle};

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

#[derive(Clone, Copy, Default)]
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

fn mock_buffer(
    text: String,
    app: &mut App,
) -> (ModelHandle<Buffer>, ModelHandle<BufferSelectionModel>) {
    let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
    let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

    buffer.update(app, |buffer, ctx| {
        *buffer = Buffer::from_plain_text(
            &text,
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            selection.clone(),
            ctx,
        );
    });
    (buffer, selection)
}

/// Builds `lines` fixed-width lines of plain text, roughly `lines * (line_len
/// + 1)` bytes total.
fn build_content(lines: usize, line_len: usize) -> String {
    (0..lines)
        .map(|i| {
            let prefix = format!("{i:010}");
            let padding = "x".repeat(line_len.saturating_sub(prefix.len()));
            format!("{prefix}{padding}\n")
        })
        .collect()
}

/// One line changed in the middle of an otherwise identical buffer.
fn build_localized_variant(lines: usize, line_len: usize) -> String {
    let target = lines / 2;
    (0..lines)
        .map(|i| {
            if i == target {
                "CHANGED\n".to_string()
            } else {
                let prefix = format!("{i:010}");
                let padding = "x".repeat(line_len.saturating_sub(prefix.len()));
                format!("{prefix}{padding}\n")
            }
        })
        .collect()
}

/// Every `stride`-th line changed, scattered across the whole buffer.
fn build_scattered_variant(lines: usize, line_len: usize, stride: usize) -> String {
    (0..lines)
        .map(|i| {
            if i % stride == 0 {
                "CHANGED\n".to_string()
            } else {
                let prefix = format!("{i:010}");
                let padding = "x".repeat(line_len.saturating_sub(prefix.len()));
                format!("{prefix}{padding}\n")
            }
        })
        .collect()
}

fn measure_replace_all(old: String, new: String) -> (Duration, AllocationStats) {
    App::test((), |mut app| async move {
        let (buffer, _selection) = mock_buffer(old, &mut app);
        let start = Instant::now();
        let (_, stats) = measure_allocations(|| {
            buffer.update(&mut app, |buffer, ctx| {
                buffer.replace_all(&new, ctx);
            });
        });
        (start.elapsed(), stats)
    })
}

fn measure_diff_based(old: String, new: String) -> (Duration, AllocationStats) {
    App::test((), |mut app| async move {
        let (buffer, _selection) = mock_buffer(old, &mut app);
        let start = Instant::now();
        let (_, stats) = measure_allocations(|| {
            buffer.update(&mut app, |buffer, ctx| {
                let old_text = buffer.text().into_string();
                let diff = text_diff_sync(&old_text, &new);
                let char_edits = diff.to_char_offset_edits(buffer);
                buffer.insert_at_char_offset_ranges(char_edits, ContentVersion::new(), ctx);
            });
        });
        (start.elapsed(), stats)
    })
}

const REPETITIONS: usize = 3;

fn run_case(label: &str, old: &str, new: &str) {
    println!(
        "\n[{label}] old={} bytes new={} bytes",
        old.len(),
        new.len()
    );

    let mut best_old: Option<(Duration, AllocationStats)> = None;
    for _ in 0..REPETITIONS {
        let result = measure_replace_all(old.to_string(), new.to_string());
        if best_old.is_none_or(|(d, _)| result.0 < d) {
            best_old = Some(result);
        }
    }
    let (old_time, old_stats) = best_old.unwrap();
    println!(
        "  OLD (replace_all):        time={:>8.3}ms  allocs={:>7}  peak={:>10}B  net_retained={:>10}B",
        old_time.as_secs_f64() * 1000.0,
        old_stats.allocations,
        old_stats.peak_bytes,
        old_stats.live_bytes
    );

    let mut best_new: Option<(Duration, AllocationStats)> = None;
    for _ in 0..REPETITIONS {
        let result = measure_diff_based(old.to_string(), new.to_string());
        if best_new.is_none_or(|(d, _)| result.0 < d) {
            best_new = Some(result);
        }
    }
    let (new_time, new_stats) = best_new.unwrap();
    println!(
        "  NEW (diff + incremental): time={:>8.3}ms  allocs={:>7}  peak={:>10}B  net_retained={:>10}B",
        new_time.as_secs_f64() * 1000.0,
        new_stats.allocations,
        new_stats.peak_bytes,
        new_stats.live_bytes
    );
}

fn main() {
    const LINES: usize = 100_000;
    const LINE_LEN: usize = 25;
    const SCATTER_STRIDE: usize = 100;

    println!("=== resolve_conflict allocation/timing comparison (APP-5357) ===");
    println!("{LINES} lines x ~{LINE_LEN} bytes/line, best of {REPETITIONS} repetitions");

    let base = build_content(LINES, LINE_LEN);
    let localized = build_localized_variant(LINES, LINE_LEN);
    let scattered = build_scattered_variant(LINES, LINE_LEN, SCATTER_STRIDE);

    run_case("localized: 1 of 100000 lines changed", &base, &localized);
    run_case("scattered: 1000 of 100000 lines changed", &base, &scattered);
}
