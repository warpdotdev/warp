//! Reproduces the retained-bytes and construction-time measurements cited on APP-4844 for
//! `BufferSumTree::append_str` (the SumTree that backs a code editor buffer's content).
//!
//! Run with `cargo run -p editor_memory_profile --release`.
//!
//! This binary links `warp_editor` as an ordinary library dependency rather than as a test
//! target, so `sum_tree`'s `test-util` feature (a `warp_editor` dev-dependency that shrinks the
//! tree's branching factor for easier test assertions) is not part of this build. See
//! `warp_editor::content::memory_profile` for the narrow function this binary calls to reach
//! `append_str` without that module becoming part of the crate's public API.
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use instant::Instant;
use sum_tree::SumTree;
use warp_editor::content::memory_profile::append_str_for_memory_profile;
use warp_editor::content::text::BufferText;

struct CountingAllocator;

static ALLOCATED: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ret = unsafe { System.alloc(layout) };
        if !ret.is_null() {
            ALLOCATED.fetch_add(layout.size(), Ordering::SeqCst);
        }
        ret
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
        ALLOCATED.fetch_sub(layout.size(), Ordering::SeqCst);
    }
}

#[global_allocator]
static GLOBAL: CountingAllocator = CountingAllocator;

/// Counts every element in the tree, plus the `Text`/`Newline` breakdown.
fn count_elements(tree: &SumTree<BufferText>) -> (usize, usize, usize) {
    let mut cursor = tree.cursor::<(), ()>();
    cursor.descend_to_first_item(tree, |_| true);
    let mut total = 0;
    let mut text_fragments = 0;
    let mut newlines = 0;
    while let Some(item) = cursor.item() {
        total += 1;
        match item {
            BufferText::Text { .. } => text_fragments += 1,
            BufferText::Newline => newlines += 1,
            _ => {}
        }
        cursor.next();
    }
    (total, text_fragments, newlines)
}

fn measure_retained_bytes(label: &str, content: &str) {
    let bytes_on_disk = content.len();

    let before = ALLOCATED.load(Ordering::SeqCst);
    let mut tree: SumTree<BufferText> = SumTree::new();
    append_str_for_memory_profile(&mut tree, content);
    let after = ALLOCATED.load(Ordering::SeqCst);
    let retained = after - before;

    let (total, text_fragments, newlines) = count_elements(&tree);

    println!("== {label} ==");
    println!("bytes_on_disk = {bytes_on_disk}");
    println!("element_count = {total} (text_fragments={text_fragments}, newlines={newlines})");
    println!("retained_bytes (measured, allocator delta) = {retained}");
    println!("multiple = {:.2}x", retained as f64 / bytes_on_disk as f64);
    std::mem::drop(tree);
}

fn measure_construction_time(label: &str, content: &str, iterations: u32) {
    let mut durations_micros = Vec::with_capacity(iterations as usize);
    for _ in 0..iterations {
        let start = Instant::now();
        let mut tree: SumTree<BufferText> = SumTree::new();
        append_str_for_memory_profile(&mut tree, content);
        durations_micros.push(start.elapsed().as_secs_f64() * 1000.0);
        std::mem::drop(tree);
    }
    let min = durations_micros
        .iter()
        .cloned()
        .fold(f64::INFINITY, f64::min);
    let max = durations_micros
        .iter()
        .cloned()
        .fold(f64::NEG_INFINITY, f64::max);
    println!("== {label} ==");
    println!("construction_time_ms (min..max over {iterations} runs) = {min:.2}..{max:.2}");
}

fn read_repo_file(relative_to_this_crate: &str) -> String {
    let path = format!("{}/{relative_to_this_crate}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("failed to read {path}: {e}"))
}

fn main() {
    println!(
        "size_of::<BufferText>() = {}",
        std::mem::size_of::<BufferText>()
    );
    println!(
        "TEXT_FRAGMENT_SIZE = {}",
        warp_editor::content::text::TEXT_FRAGMENT_SIZE
    );
    println!();

    // Realistic source files: retained-bytes multiple over bytes on disk.
    measure_retained_bytes(
        "large source file (buffer.rs)",
        &read_repo_file("../editor/src/content/buffer.rs"),
    );
    measure_retained_bytes(
        "medium source file (core.rs)",
        &read_repo_file("../editor/src/content/core.rs"),
    );
    measure_retained_bytes(
        "small source file (cursor.rs)",
        &read_repo_file("../editor/src/content/cursor.rs"),
    );

    // The other side of the trade-off: a workload of few, very long lines fragments more (and
    // takes longer to build) at a smaller TEXT_FRAGMENT_SIZE, since each line now needs more
    // fragments to cover the same byte span.
    let long_lines = std::iter::repeat_with(|| "a".repeat(32 * 1024))
        .take(48)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    let (total, ..) = count_elements(&{
        let mut tree = SumTree::new();
        append_str_for_memory_profile(&mut tree, &long_lines);
        tree
    });
    println!("== long-line workload (48 lines x 32 KiB) ==");
    println!("element_count = {total}");
    measure_construction_time("long-line workload (48 lines x 32 KiB)", &long_lines, 20);

    let short_lines = std::iter::repeat_with(|| "a".repeat(32))
        .take(48 * 1024)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    measure_construction_time(
        "short-line workload (48Ki lines x 32 bytes, same total bytes)",
        &short_lines,
        20,
    );
}
