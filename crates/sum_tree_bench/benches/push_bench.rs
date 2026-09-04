use std::mem::size_of;

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use sum_tree::{Node, SumTree};
use sum_tree_bench::{BenchItem, assert_production_capacity};

const FRESH_PUSHES: usize = 1_000;
const DOCUMENT_ITEMS: usize = 10_000;
const INCREMENTAL_PUSHES: usize = 100;

fn push_benches(c: &mut Criterion) {
    assert_production_capacity();
    println!(
        "sum_tree geometry: {} item slots per leaf, {} B per node, {} B per item",
        sum_tree::ITEMS_PER_LEAF,
        size_of::<Node<BenchItem>>(),
        size_of::<BenchItem>(),
    );

    c.bench_function("push_1000_onto_empty_tree", |b| {
        b.iter(|| {
            let mut tree = SumTree::new();
            for _ in 0..FRESH_PUSHES {
                tree.push(BenchItem::default());
            }
            tree
        })
    });

    let document = {
        let mut tree = SumTree::new();
        tree.extend((0..DOCUMENT_ITEMS).map(|_| BenchItem::default()));
        tree
    };

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
            BatchSize::SmallInput,
        )
    });
}

criterion_group!(benches, push_benches);
criterion_main!(benches);
