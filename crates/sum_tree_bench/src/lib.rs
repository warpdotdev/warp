//! Timing harness for [`sum_tree::SumTree::push`].
//!
//! This lives outside `sum_tree` because that crate takes a dev-dependency on itself with
//! `test-util` enabled, which lowers `TREE_BASE`. A benchmark in its own `benches/` would measure 4
//! item slots per leaf where the product ships 12, and leaf geometry is the thing `push` changes.
//!
//! Which capacity a build gets depends on the packages it selects: `cargo bench -p sum_tree_bench`
//! links against production geometry, whereas naming `sum_tree` alongside it, or `--workspace`,
//! unifies that dev-dependency back in and lowers it. [`assert_production_capacity`] stops the run
//! rather than let it report the wrong tree.

use std::ops::AddAssign;

use sum_tree::Item;

/// Item slots per leaf in a production build.
const PRODUCTION_ITEMS_PER_LEAF: usize = 12;

/// Panics unless the linked `sum_tree` has production leaf geometry.
pub fn assert_production_capacity() {
    assert_eq!(
        sum_tree::ITEMS_PER_LEAF,
        PRODUCTION_ITEMS_PER_LEAF,
        "linked against a sum_tree built with `test-util`, which lowers TREE_BASE; timings from \
         this build describe leaf geometry the product does not ship"
    );
}

/// Stand-in for `BlockItem`, the item type the heap profiles attribute this memory to.
///
/// Only its size matters here: a leaf stores its items inline, so the payload width decides node
/// size and therefore how much memory traffic a push costs. 144 bytes is the measured size of
/// `BlockItem`; a pointer-sized stand-in would understate the work by an order of magnitude.
#[derive(Clone, Debug)]
pub struct BenchItem {
    // Never read: the bytes exist to be allocated, copied and moved at the right width.
    #[allow(dead_code)]
    payload: [u8; 144],
}

impl Default for BenchItem {
    fn default() -> Self {
        Self { payload: [0; 144] }
    }
}

/// Sized to match `LayoutSummary`, which leaves also store inline, once per item slot.
#[derive(Clone, Debug, Default)]
pub struct BenchSummary {
    items: usize,
    // Never read: as with `BenchItem::payload`, only the width matters.
    #[allow(dead_code)]
    padding: [u8; 32],
}

impl AddAssign<&Self> for BenchSummary {
    fn add_assign(&mut self, other: &Self) {
        self.items += other.items;
    }
}

impl Item for BenchItem {
    type Summary = BenchSummary;

    fn summary(&self) -> Self::Summary {
        BenchSummary {
            items: 1,
            padding: [0; 32],
        }
    }
}
