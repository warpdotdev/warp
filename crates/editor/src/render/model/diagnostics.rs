//! Diagnostics for how many [`RenderState`] content trees are alive, and how large each one is.
//!
//! Heap profiles attribute most of the process's memory to the editor's content trees but cannot
//! tell a few enormous trees apart from very many small ones, so they keep coming back
//! inconclusive (APP-5445). Every live model registers a weak handle here, and the stats come from
//! tree summaries that are maintained anyway: collecting them costs one summary read per live
//! model and never walks a tree.
//!
//! This is diagnostic only. Nothing here affects layout, what is laid out, or when.

use std::cell::RefCell;

use warpui_core::{AppContext, WeakModelHandle};

use super::RenderState;

/// Inclusive upper bounds, in items, of the buckets a model's content tree is counted in. A model
/// falls in the first bucket whose bound it does not exceed; larger trees are counted separately in
/// [`RenderStateStats::models_above_largest_bucket`].
///
/// There is deliberately no zero bucket: a pixel-mode tree is seeded with a trailing newline and
/// `remove_final_trailing_newline_if_present` refuses to remove the last block, so no live
/// pixel-mode model can report zero items.
const ITEM_COUNT_BUCKETS: [usize; 5] = [10, 100, 1_000, 10_000, 100_000];

thread_local! {
    /// Every [`RenderState`] built on this thread through a non-test constructor, as a weak handle.
    ///
    /// Thread-local rather than global because reading a model requires an [`AppContext`], which
    /// confines model access to the thread that owns them — so a registry per thread sees exactly
    /// the models a caller on that thread could read, and needs no lock. `WeakModelHandle` is also
    /// not `Sync`, since `RenderState` holds `Cell`s.
    ///
    /// Entries are never removed. A handle that fails to upgrade is **not** treated as a dead
    /// model, because `upgrade` also fails for a live model that is momentarily out of the model
    /// map — which it is for the whole of its own update, event emission, observer callback, or
    /// spawned stream handler. Pruning on that signal would permanently drop a live model and
    /// undercount it for the rest of the process, which is the one failure mode that would mislead
    /// us. So the registry grows by one entry, an `EntityId`, for every `RenderState` ever created
    /// on this thread, and entries that cannot be resolved are reported rather than removed.
    static LIVE_RENDER_STATES: RefCell<Vec<WeakModelHandle<RenderState>>> =
        const { RefCell::new(Vec::new()) };
}

/// Record a newly built model so its content tree is visible to [`live_render_state_stats`].
pub(super) fn register(handle: WeakModelHandle<RenderState>) {
    LIVE_RENDER_STATES.with_borrow_mut(|registered| registered.push(handle));
}

/// The number of registry entries, resolvable or not.
#[cfg(test)]
pub(super) fn registry_len() -> usize {
    LIVE_RENDER_STATES.with_borrow(Vec::len)
}

/// How many content trees are alive, and how large they are.
///
/// Sizes are reported in items, lines and characters because those are not interchangeable. Items
/// are `BlockItem`s, which is what the tree's nodes hold and therefore what the heap profile's
/// bytes attach to; lines are what a per-laid-out-line cost has to be compared against. One item
/// can span many lines (a soft-wrapped paragraph, or a `TextBlock`'s `Vec1<Paragraph>`) or collapse
/// a whole region into one ([`super::BlockItem::Hidden`], which is the code-review configuration
/// this investigation suspects), so dividing bytes by items would give a per-unit cost wrong by
/// whatever that factor happens to be.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RenderStateStats {
    /// Live models on the pixel (GUI) layout path. These are the ones that hold a content tree.
    pub live_pixel_models: usize,
    /// Live models on the char-cell (TUI) layout path. These never populate the `SumTree`, so they
    /// are counted here rather than distorting the size figures with empty trees.
    pub live_char_cell_models: usize,
    /// Entries whose model could not be read: either dropped since it registered, or momentarily
    /// out of the model map while being updated. Reported rather than assumed dead, so that any
    /// shortfall in the live counts is visible here instead of silent.
    pub unresolved_entries: usize,
    /// Items across every live pixel-mode content tree.
    pub total_items: usize,
    /// Lines across every live pixel-mode content tree. This is the figure to compare against a
    /// per-laid-out-line cost.
    pub total_lines: usize,
    /// Characters of content across every live pixel-mode content tree.
    pub total_chars: usize,
    /// Items in the largest single pixel-mode content tree.
    pub largest_model_items: usize,
    /// Lines in the largest single pixel-mode content tree.
    pub largest_model_lines: usize,
    /// Pixel-mode models per bucket, parallel to [`RenderStateStats::bucket_upper_bounds`].
    pub models_by_item_count: [usize; ITEM_COUNT_BUCKETS.len()],
    /// Pixel-mode models holding more items than the largest bucket's bound.
    pub models_above_largest_bucket: usize,
}

impl RenderStateStats {
    /// The bucket bounds `models_by_item_count` is indexed by, so a caller can label the counts
    /// without restating them.
    pub fn bucket_upper_bounds() -> &'static [usize] {
        &ITEM_COUNT_BUCKETS
    }
}

/// Collect stats for every registered [`RenderState`].
///
/// Costs one weak-handle upgrade and one root-summary read per live model. Entries that cannot be
/// resolved are counted in [`RenderStateStats::unresolved_entries`] and left in place; see the
/// registry's own documentation for why they are not pruned.
pub fn live_render_state_stats(app: &AppContext) -> RenderStateStats {
    let mut stats = RenderStateStats::default();

    LIVE_RENDER_STATES.with_borrow(|registered| {
        for weak_handle in registered {
            let Some(handle) = weak_handle.upgrade(app) else {
                stats.unresolved_entries += 1;
                continue;
            };

            let render_state = handle.as_ref(app);
            if render_state.char_cell().is_some() {
                stats.live_char_cell_models += 1;
                continue;
            }

            let size = render_state.content_size();
            stats.live_pixel_models += 1;
            stats.total_items += size.items;
            stats.total_lines += size.lines;
            stats.total_chars += size.chars;
            stats.largest_model_items = stats.largest_model_items.max(size.items);
            stats.largest_model_lines = stats.largest_model_lines.max(size.lines);
            match ITEM_COUNT_BUCKETS
                .iter()
                .position(|bound| size.items <= *bound)
            {
                Some(bucket) => stats.models_by_item_count[bucket] += 1,
                None => stats.models_above_largest_bucket += 1,
            }
        }
    });

    stats
}

#[cfg(test)]
#[path = "diagnostics_tests.rs"]
mod tests;
