use std::ops::Add;

use super::*;

#[test]
fn test_extend_and_push_tree() {
    let mut tree1 = SumTree::new();
    tree1.extend(0..20);

    let mut tree2 = SumTree::new();
    tree2.extend(50..100);

    tree1.push_tree(tree2);
    assert_eq!(tree1.items(), (0..20).chain(50..100).collect::<Vec<u8>>());
}

/// Leaf and internal node counts, for tests that care about a tree's shape rather than its content.
#[derive(Debug, Default, PartialEq, Eq)]
struct NodeCounts {
    leaves: usize,
    internal: usize,
    items: usize,
}

impl NodeCounts {
    fn of<T: Item>(tree: &SumTree<T>) -> Self {
        let mut counts = Self::default();
        counts.walk(tree);
        counts
    }

    fn walk<T: Item>(&mut self, tree: &SumTree<T>) {
        match tree.0.as_ref() {
            Node::Leaf { items, .. } => {
                self.leaves += 1;
                self.items += items.len();
            }
            Node::Internal { child_trees, .. } => {
                self.internal += 1;
                for child in child_trees {
                    self.walk(child);
                }
            }
        }
    }

    /// The share of the tree's leaf item slots that hold an item.
    fn leaf_occupancy(&self) -> f64 {
        if self.leaves == 0 {
            return 0.0;
        }
        self.items as f64 / (self.leaves * 2 * TREE_BASE) as f64
    }
}

/// A pushed item merges into the rightmost leaf rather than arriving in a leaf of its own.
///
/// `push` used to wrap its leaf in an internal node, which sent `push_tree_recursive` down its
/// `height_delta == 0` branch and adopted that one-item leaf whole: 1000 items produced 989 leaves
/// at 8.4% occupancy under the production `TREE_BASE` of 6, and a leaf allocation costs the full
/// size of `Node` however few items it holds.
///
/// Appending into the rightmost leaf splits it at `2 * TREE_BASE + 1` items and keeps
/// `TREE_BASE + 1` on the left, so sealed leaves settle at `TREE_BASE + 1` of their `2 * TREE_BASE`
/// slots rather than filling. That is a little over half full, not full: 75% here at the test
/// configuration's 4 slots, and 58% at production's 12. Both are far above 8.4%, and the
/// assertions below are written against `TREE_BASE` so they hold at either capacity.
#[test]
fn push_merges_into_the_rightmost_leaf_rather_than_adding_a_leaf_per_item() {
    const ITEMS: usize = 1000;

    let mut pushed = SumTree::<u8>::new();
    for item in 0..ITEMS {
        pushed.push(item as u8);
    }

    let counts = NodeCounts::of(&pushed);
    assert_eq!(counts.items, ITEMS);
    assert!(
        counts.leaves <= ITEMS.div_ceil(TREE_BASE + 1),
        "a pushed item should join the rightmost leaf, leaving about one leaf per {} items, \
         got {counts:?}",
        TREE_BASE + 1
    );
    assert!(
        counts.leaf_occupancy() > 0.5,
        "pushed items should leave their leaves over half full, got {counts:?}"
    );
}

/// `extend` still packs tighter than `push`, so a caller holding a collectable sequence still has a
/// reason to hand it over in one call.
///
/// This is the honest limit of merging into the rightmost leaf: it removes the leaf-per-item
/// pathology but not the append-split slack, because a bulk insert seals leaves at their full
/// `2 * TREE_BASE` while repeated appends seal them at `TREE_BASE + 1`. The ratio between the two is
/// therefore about 4:3 here and 12:7 at production capacity — worth keeping in view rather than
/// assuming the two paths became equivalent.
#[test]
fn extend_packs_tighter_than_push_even_after_merging() {
    let items = (0..1000).map(|item| item as u8).collect::<Vec<_>>();

    let mut pushed = SumTree::new();
    for item in items.iter().copied() {
        pushed.push(item);
    }
    let mut extended = SumTree::new();
    extended.extend(items.iter().copied());

    let pushed_counts = NodeCounts::of(&pushed);
    let extended_counts = NodeCounts::of(&extended);

    assert_eq!(pushed.items(), items, "both paths hold the same items");
    assert_eq!(extended.items(), items);
    assert!(
        extended_counts.leaf_occupancy() > pushed_counts.leaf_occupancy(),
        "extend should still pack tighter: pushed {pushed_counts:?}, extended {extended_counts:?}"
    );
    assert!(
        pushed_counts.leaves < extended_counts.leaves * 2,
        "push should stay within a small factor of extend: pushed {pushed_counts:?}, \
         extended {extended_counts:?}"
    );
}

/// Merging into the rightmost leaf writes through boundaries `push` used to leave alone, so this
/// interleaves the three ways of appending and checks the resulting sequence end to end.
#[test]
fn pushing_between_bulk_inserts_keeps_items_in_order() {
    let mut tree = SumTree::<u8>::new();
    let mut expected = Vec::new();

    for round in 0..20u8 {
        tree.push(round);
        expected.push(round);

        tree.extend(round * 3..round * 3 + round % 5);
        expected.extend(round * 3..round * 3 + round % 5);

        let mut other = SumTree::new();
        other.extend(100 + round..100 + round + round % 3);
        tree.push_tree(other);
        expected.extend(100 + round..100 + round + round % 3);

        tree.push(round + 200);
        expected.push(round + 200);
    }

    assert_eq!(tree.items(), expected);
    assert_eq!(tree.summary().count.0, expected.len());
    assert_eq!(
        tree.summary().sum.0,
        expected.iter().map(|item| *item as usize).sum::<usize>()
    );
}

#[test]
fn test_random() {
    for seed in 0..100 {
        use rand::distributions;
        use rand::prelude::*;

        let rng = &mut StdRng::seed_from_u64(seed);

        let mut tree = SumTree::<u8>::new();
        let count = rng.gen_range(0..10);
        tree.extend(rng.sample_iter(distributions::Standard).take(count));

        for _ in 0..5 {
            let splice_end = rng.gen_range(0..tree.extent::<Count>().0 + 1);
            let splice_start = rng.gen_range(0..splice_end + 1);
            let count = rng.gen_range(0..3);
            let tree_end = tree.extent::<Count>();
            let new_items = rng
                .sample_iter(distributions::Standard)
                .take(count)
                .collect::<Vec<u8>>();

            let mut reference_items = tree.items();
            reference_items.splice(splice_start..splice_end, new_items.clone());

            tree = {
                let mut cursor = tree.cursor::<Count, ()>();
                let mut new_tree = cursor.slice(&Count(splice_start), SeekBias::Right);
                new_tree.extend(new_items);
                cursor.seek(&Count(splice_end), SeekBias::Right);
                new_tree.push_tree(cursor.slice(&tree_end, SeekBias::Right));
                new_tree
            };

            assert_eq!(tree.items(), reference_items);

            let mut filter_cursor = tree.filter::<_, Count>(|summary| summary.contains_even);
            let mut reference_filter = tree
                .items()
                .into_iter()
                .enumerate()
                .filter(|(_, item)| (item & 1) == 0);
            while let Some(actual_item) = filter_cursor.item() {
                let (reference_index, reference_item) = reference_filter.next().unwrap();
                assert_eq!(actual_item, &reference_item);
                assert_eq!(filter_cursor.start().0, reference_index);
                filter_cursor.next();
            }
            assert!(reference_filter.next().is_none());

            let mut pos = rng.gen_range(0..tree.extent::<Count>().0 + 1);
            let mut before_start = false;
            let mut cursor = tree.cursor::<Count, Count>();
            cursor.seek(&Count(pos), SeekBias::Right);

            for i in 0..10 {
                assert_eq!(cursor.start().0, pos);

                if pos > 0 {
                    assert_eq!(cursor.prev_item().unwrap(), &reference_items[pos - 1]);
                } else {
                    assert_eq!(cursor.prev_item(), None);
                }

                if pos < reference_items.len() && !before_start {
                    assert_eq!(cursor.item().unwrap(), &reference_items[pos]);
                } else {
                    assert_eq!(cursor.item(), None);
                }

                if i < 5 {
                    cursor.next();
                    if pos < reference_items.len() {
                        pos += 1;
                        before_start = false;
                    }
                } else {
                    cursor.prev();
                    if pos == 0 {
                        before_start = true;
                    }
                    pos = pos.saturating_sub(1);
                }
            }
        }

        for _ in 0..10 {
            let end = rng.gen_range(0..tree.extent::<Count>().0 + 1);
            let start = rng.gen_range(0..end + 1);
            let start_bias = if rng.r#gen() {
                SeekBias::Left
            } else {
                SeekBias::Right
            };
            let end_bias = if rng.r#gen() {
                SeekBias::Left
            } else {
                SeekBias::Right
            };

            let mut cursor = tree.cursor::<Count, ()>();
            cursor.seek(&Count(start), start_bias);
            let slice = cursor.slice(&Count(end), end_bias);

            cursor.seek(&Count(start), start_bias);
            let summary = cursor.summary::<Sum>(&Count(end), end_bias);

            assert_eq!(summary, slice.summary().sum);
        }
    }
}

#[test]
fn test_update_last() {
    let mut tree = SumTree::new();
    tree.extend(vec![1]);
    tree.update_last(|item| *item += 1);
    assert_eq!(tree.summary().sum, Sum(2));

    tree.extend(vec![2, 0, 0, 4]);
    assert_eq!(tree.summary().sum, Sum(8));
    tree.update_last(|item| *item = 0);
    assert_eq!(tree.summary().sum, Sum(4));
}

#[test]
fn test_seek_position() {
    let mut tree = SumTree::new();
    tree.extend(vec![1, 1, 0, 0, 4, 1]);
    // Cursor state: 1 | 1 0 0 4 1
    let mut cursor = tree.cursor::<Sum, Count>();
    assert_eq!(cursor.slice(&Sum(2), SeekBias::Left).items(), [1]);
    assert_eq!(cursor.start(), &Count(1));
    assert_eq!(cursor.seek_position(), &Sum(1));
    assert_eq!(cursor.end_seek_position(), Sum(2));

    let mut cursor = tree.cursor::<Sum, Count>();
    // Cursor state: 1 1 0 0 | 4 1
    assert_eq!(cursor.slice(&Sum(2), SeekBias::Right).items(), [1, 1, 0, 0]);
    assert_eq!(cursor.start(), &Count(4));
    assert_eq!(cursor.seek_position(), &Sum(2));
    assert_eq!(cursor.end_seek_position(), Sum(6));

    let mut cursor = tree.cursor::<Sum, Count>();
    // Cursor state: 1 1 0 0 | 4 1
    assert_eq!(cursor.slice(&Sum(3), SeekBias::Left).items(), [1, 1, 0, 0]);
    assert_eq!(cursor.start(), &Count(4));
    assert_eq!(cursor.seek_position(), &Sum(2));
    assert_eq!(cursor.end_seek_position(), Sum(6));

    let mut cursor = tree.cursor::<Sum, Count>();
    // Cursor state: 1 1 0 0 | 4 1
    assert_eq!(cursor.slice(&Sum(3), SeekBias::Right).items(), [1, 1, 0, 0]);
    assert_eq!(cursor.start(), &Count(4));
    assert_eq!(cursor.seek_position(), &Sum(2));
    assert_eq!(cursor.end_seek_position(), Sum(6));

    let mut cursor = tree.cursor::<Sum, Count>();
    // Cursor state: 1 1 0 0 | 4 1
    assert_eq!(cursor.slice(&Sum(6), SeekBias::Left).items(), [1, 1, 0, 0]);
    assert_eq!(cursor.start(), &Count(4));
    assert_eq!(cursor.seek_position(), &Sum(2));
    assert_eq!(cursor.end_seek_position(), Sum(6));

    let mut cursor = tree.cursor::<Sum, Count>();
    // Cursor state: 1 1 0 0 4 | 1
    assert_eq!(
        cursor.slice(&Sum(6), SeekBias::Right).items(),
        [1, 1, 0, 0, 4]
    );
    assert_eq!(cursor.start(), &Count(5));
    assert_eq!(cursor.seek_position(), &Sum(6));
    assert_eq!(cursor.end_seek_position(), Sum(7));
}

#[test]
fn test_cursor() {
    // Empty tree
    let tree = SumTree::<u8>::new();
    let mut cursor = tree.cursor::<Count, Sum>();
    assert_eq!(
        cursor.slice(&Count(0), SeekBias::Right).items(),
        Vec::<u8>::new()
    );
    assert_eq!(cursor.item(), None);
    assert_eq!(cursor.prev_item(), None);
    assert_eq!(cursor.start(), &Sum(0));

    // Single-element tree
    let mut tree = SumTree::<u8>::new();
    tree.extend(vec![1]);
    let mut cursor = tree.cursor::<Count, Sum>();
    assert_eq!(
        cursor.slice(&Count(0), SeekBias::Right).items(),
        Vec::<u8>::new()
    );
    assert_eq!(cursor.item(), Some(&1));
    assert_eq!(cursor.prev_item(), None);
    assert_eq!(cursor.start(), &Sum(0));

    cursor.next();
    assert_eq!(cursor.item(), None);
    assert_eq!(cursor.prev_item(), Some(&1));
    assert_eq!(cursor.start(), &Sum(1));

    cursor.prev();
    assert_eq!(cursor.item(), Some(&1));
    assert_eq!(cursor.prev_item(), None);
    assert_eq!(cursor.start(), &Sum(0));

    let mut cursor = tree.cursor::<Count, Sum>();
    assert_eq!(cursor.slice(&Count(1), SeekBias::Right).items(), [1]);
    assert_eq!(cursor.item(), None);
    assert_eq!(cursor.prev_item(), Some(&1));
    assert_eq!(cursor.start(), &Sum(1));

    cursor.seek(&Count(0), SeekBias::Right);
    assert_eq!(
        cursor
            .slice(&tree.extent::<Count>(), SeekBias::Right)
            .items(),
        [1]
    );
    assert_eq!(cursor.item(), None);
    assert_eq!(cursor.prev_item(), Some(&1));
    assert_eq!(cursor.start(), &Sum(1));

    assert!(!cursor.seek(&Count(2), SeekBias::Left));

    cursor.seek(&Count(0), SeekBias::Left);
    assert_eq!(cursor.item(), Some(&1));
    assert_eq!(cursor.prev_item(), None);
    assert_eq!(cursor.start(), &Sum(0));

    // Left bias should position it at item 1
    cursor.seek_clamped(&Count(2), SeekBias::Left);
    assert_eq!(cursor.item(), Some(&1));
    assert_eq!(cursor.prev_item(), None);
    assert_eq!(cursor.start(), &Sum(0));

    // Right bias should position it past item 1
    cursor.seek_clamped(&Count(2), SeekBias::Right);
    assert_eq!(cursor.item(), None);
    assert_eq!(cursor.prev_item(), Some(&1));
    assert_eq!(cursor.start(), &Sum(1));

    // Multiple-element tree
    let mut tree = SumTree::new();
    tree.extend(vec![1, 2, 3, 4, 5, 6]);
    let mut cursor = tree.cursor::<Count, Sum>();

    assert_eq!(cursor.slice(&Count(2), SeekBias::Right).items(), [1, 2]);
    assert_eq!(cursor.item(), Some(&3));
    assert_eq!(cursor.prev_item(), Some(&2));
    assert_eq!(cursor.start(), &Sum(3));

    cursor.next();
    assert_eq!(cursor.item(), Some(&4));
    assert_eq!(cursor.prev_item(), Some(&3));
    assert_eq!(cursor.start(), &Sum(6));

    cursor.next();
    assert_eq!(cursor.item(), Some(&5));
    assert_eq!(cursor.prev_item(), Some(&4));
    assert_eq!(cursor.start(), &Sum(10));

    cursor.next();
    assert_eq!(cursor.item(), Some(&6));
    assert_eq!(cursor.prev_item(), Some(&5));
    assert_eq!(cursor.start(), &Sum(15));

    cursor.next();
    cursor.next();
    assert_eq!(cursor.item(), None);
    assert_eq!(cursor.prev_item(), Some(&6));
    assert_eq!(cursor.start(), &Sum(21));

    cursor.prev();
    assert_eq!(cursor.item(), Some(&6));
    assert_eq!(cursor.prev_item(), Some(&5));
    assert_eq!(cursor.start(), &Sum(15));

    cursor.prev();
    assert_eq!(cursor.item(), Some(&5));
    assert_eq!(cursor.prev_item(), Some(&4));
    assert_eq!(cursor.start(), &Sum(10));

    cursor.prev();
    assert_eq!(cursor.item(), Some(&4));
    assert_eq!(cursor.prev_item(), Some(&3));
    assert_eq!(cursor.start(), &Sum(6));

    cursor.prev();
    assert_eq!(cursor.item(), Some(&3));
    assert_eq!(cursor.prev_item(), Some(&2));
    assert_eq!(cursor.start(), &Sum(3));

    cursor.prev();
    assert_eq!(cursor.item(), Some(&2));
    assert_eq!(cursor.prev_item(), Some(&1));
    assert_eq!(cursor.start(), &Sum(1));

    cursor.prev();
    assert_eq!(cursor.item(), Some(&1));
    assert_eq!(cursor.prev_item(), None);
    assert_eq!(cursor.start(), &Sum(0));

    cursor.prev();
    assert_eq!(cursor.item(), None);
    assert_eq!(cursor.prev_item(), None);
    assert_eq!(cursor.start(), &Sum(0));

    cursor.next();
    assert_eq!(cursor.item(), Some(&1));
    assert_eq!(cursor.prev_item(), None);
    assert_eq!(cursor.start(), &Sum(0));

    let mut cursor = tree.cursor::<Count, Sum>();
    assert_eq!(
        cursor
            .slice(&tree.extent::<Count>(), SeekBias::Right)
            .items(),
        tree.items()
    );
    assert_eq!(cursor.item(), None);
    assert_eq!(cursor.prev_item(), Some(&6));
    assert_eq!(cursor.start(), &Sum(21));

    cursor.seek(&Count(3), SeekBias::Right);
    assert_eq!(
        cursor
            .slice(&tree.extent::<Count>(), SeekBias::Right)
            .items(),
        [4, 5, 6]
    );
    assert_eq!(cursor.item(), None);
    assert_eq!(cursor.prev_item(), Some(&6));
    assert_eq!(cursor.start(), &Sum(21));

    // Seeking can bias left or right
    cursor.seek(&Count(1), SeekBias::Left);
    assert_eq!(cursor.item(), Some(&1));
    cursor.seek(&Count(1), SeekBias::Right);
    assert_eq!(cursor.item(), Some(&2));

    // Slicing without resetting starts from where the cursor is parked at.
    cursor.seek(&Count(1), SeekBias::Right);
    assert_eq!(cursor.slice(&Count(3), SeekBias::Right).items(), vec![2, 3]);
    assert_eq!(cursor.slice(&Count(6), SeekBias::Left).items(), vec![4, 5]);
    assert_eq!(cursor.slice(&Count(6), SeekBias::Right).items(), vec![6]);

    // Seek with a max
    let mut cursor = tree.cursor::<Count, Sum>();
    cursor.seek_clamped(&Count(1), SeekBias::Right);
    assert_eq!(cursor.slice(&Count(3), SeekBias::Right).items(), vec![2, 3]);

    // Seek max past the end
    cursor.seek_clamped(&Count(8), SeekBias::Right);
    assert_eq!(cursor.item(), None);
    assert_eq!(cursor.prev_item(), Some(&6));
    assert_eq!(cursor.start(), &Sum(21));
    assert_eq!(
        cursor.rev().collect::<Vec<&u8>>(),
        vec![&6, &5, &4, &3, &2, &1]
    );

    // Seek max past the end with a left bias
    let mut cursor = tree.cursor::<Count, Sum>();
    cursor.seek(&Count(1), SeekBias::Right);
    cursor.seek_clamped(&Count(8), SeekBias::Left);
    assert_eq!(cursor.item(), Some(&6));
    assert_eq!(cursor.start(), &Sum(15));
    assert_eq!(
        cursor.rev().collect::<Vec<&u8>>(),
        vec![&6, &5, &4, &3, &2, &1]
    );

    // Seek backwards - this works because seeking multiple times
    // resets the cursor state.
    let mut cursor = tree.cursor::<Count, Sum>();
    cursor.seek(&Count(3), SeekBias::Right);
    assert_eq!(cursor.item(), Some(&4));
    cursor.seek(&Count(1), SeekBias::Right);
    assert_eq!(cursor.item(), Some(&2));
}

#[derive(Clone, Default, Debug)]
pub struct IntegersSummary {
    count: Count,
    sum: Sum,
    contains_even: bool,
}

#[derive(Ord, PartialOrd, Default, Eq, PartialEq, Clone, Debug)]
struct Count(usize);

#[derive(Ord, PartialOrd, Default, Eq, PartialEq, Clone, Debug)]
struct Sum(usize);

impl Item for u8 {
    type Summary = IntegersSummary;

    fn summary(&self) -> Self::Summary {
        IntegersSummary {
            count: Count(1),
            sum: Sum(*self as usize),
            contains_even: (*self & 1) == 0,
        }
    }
}

impl AddAssign<&Self> for IntegersSummary {
    fn add_assign(&mut self, other: &Self) {
        self.count.0 += &other.count.0;
        self.sum.0 += &other.sum.0;
        self.contains_even |= other.contains_even;
    }
}

impl Dimension<'_, IntegersSummary> for Count {
    fn add_summary(&mut self, summary: &IntegersSummary) {
        self.0 += summary.count.0;
    }
}

impl Dimension<'_, IntegersSummary> for Sum {
    fn add_summary(&mut self, summary: &IntegersSummary) {
        self.0 += summary.sum.0;
    }
}

impl Add<&Self> for Sum {
    type Output = Self;

    fn add(mut self, other: &Self) -> Self {
        self.0 += other.0;
        self
    }
}
