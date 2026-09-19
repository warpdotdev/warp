use std::mem::size_of;

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use sum_tree::{Node, SumTree};
use sum_tree_bench::{BenchItem, assert_production_capacity};

const FRESH_PUSHES: usize = 1_000;
const DOCUMENT_ITEMS: usize = 10_000;
const INCREMENTAL_PUSHES: usize = 100;
const MALLOC_TRIM_THRESHOLD: &str = "MALLOC_TRIM_THRESHOLD_";

fn push_benches(c: &mut Criterion) {
    assert_production_capacity();
    assert_benchmark_environment();
    println!(
        "sum_tree geometry: {} item slots per leaf, {} B per node, {} B per item",
        sum_tree::ITEMS_PER_LEAF,
        size_of::<Node<BenchItem>>(),
        size_of::<BenchItem>(),
    );
    let minor_faults_before = minor_faults();

    c.bench_function("push_1000_onto_empty_tree", |b| {
        b.iter(|| {
            let mut tree = SumTree::new();
            for _ in 0..FRESH_PUSHES {
                tree.push(BenchItem::default());
            }
            tree
        })
    });
    report_minor_faults("push_1000_onto_empty_tree", minor_faults_before);

    let document = {
        let mut tree = SumTree::new();
        tree.extend((0..DOCUMENT_ITEMS).map(|_| BenchItem::default()));
        tree
    };
    let minor_faults_before = minor_faults();

    c.bench_function("push_100_onto_10k_item_tree", |b| {
        b.iter_batched(
            // The clone shares every node, so the pushes pay the copy-on-write the editor pays
            // when it rebuilds a tree from a slice of the previous generation. Timing this against
            // an unshared tree would measure the easy case.
            || document.clone(),
            |mut tree| {
                for _ in 0..INCREMENTAL_PUSHES {
                    tree.push(BenchItem::default());
                }
                tree
            },
            BatchSize::PerIteration,
        )
    });
    report_minor_faults("push_100_onto_10k_item_tree", minor_faults_before);
}

fn assert_benchmark_environment() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    {
        let trim_threshold = std::env::var(MALLOC_TRIM_THRESHOLD).ok();
        assert_eq!(
            trim_threshold.as_deref(),
            Some("-1"),
            "{MALLOC_TRIM_THRESHOLD} must be -1 for this benchmark on Linux/glibc; run `\
             {MALLOC_TRIM_THRESHOLD}=-1 cargo bench -p sum_tree_bench`"
        );
        println!("allocator: Linux/glibc with {MALLOC_TRIM_THRESHOLD}=-1");
    }

    #[cfg(not(all(target_os = "linux", target_env = "gnu")))]
    println!("allocator: non-glibc; {MALLOC_TRIM_THRESHOLD} is not required");
}

#[cfg(unix)]
fn minor_faults() -> Option<u64> {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
    // SAFETY: getrusage writes a complete rusage value to a valid pointer when it returns zero.
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) } != 0 {
        return None;
    }
    // SAFETY: the successful getrusage call initialized usage.
    u64::try_from(unsafe { usage.assume_init() }.ru_minflt).ok()
}

#[cfg(not(unix))]
fn minor_faults() -> Option<u64> {
    None
}

fn report_minor_faults(benchmark: &str, before: Option<u64>) {
    match (before, minor_faults()) {
        (Some(before), Some(after)) => {
            println!("{benchmark}: {} minor faults", after.saturating_sub(before));
        }
        _ => println!("{benchmark}: minor fault count unavailable"),
    }
}

criterion_group!(benches, push_benches);
criterion_main!(benches);
