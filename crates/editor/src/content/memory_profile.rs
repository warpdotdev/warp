//! A narrow entry point into [`BufferSumTree::append_str`], used only by the standalone
//! `editor_memory_profile` binary (`crates/editor_memory_profile`) that reproduces the
//! retained-bytes measurements from APP-4844.
//!
//! That binary depends on `warp_editor` as an ordinary library dependency rather than compiling
//! it as a test/bench target, specifically so it does not pull in this crate's `sum_tree`
//! `test-util` dev-dependency, which shrinks the tree's branching factor and would skew the
//! retained-bytes result. The `cursor` module stays private; this function is the one thing
//! that binary needs from it.

use sum_tree::SumTree;

use super::cursor::BufferSumTree;
use super::text::BufferText;

/// Appends `s` to `tree` via the real [`BufferSumTree::append_str`] implementation under test.
pub fn append_str_for_memory_profile(tree: &mut SumTree<BufferText>, s: &str) {
    tree.append_str(s);
}
