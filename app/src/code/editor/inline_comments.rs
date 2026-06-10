use std::collections::HashMap;
use std::sync::Arc;

use pathfinder_geometry::vector::vec2f;
use warp_editor::render::model::{CommentBlock, RenderLineLocation};
use warpui::units::Pixels;
use warpui::{EntityId, ModelHandle, ViewContext, ViewHandle, WindowId};

use crate::code::editor::comment_editor::DEFAULT_COMMENT_MAX_WIDTH;
use crate::code::editor::embedded_comment::LaidOutInlineSavedComment;
use crate::code::editor::inline_comment_view::InlineCommentView;
use crate::code::editor::line::EditorLineLocation;
use crate::code::editor::model::CodeEditorModel;
use crate::code::editor::view::CodeEditorView;
use crate::code::editor::EditorReviewComment;
use crate::code_review::comments::CommentId;
use crate::features::FeatureFlag;

/// The desired render-state entry for one inline comment view: its anchor, reserved height, and
/// the hosted view's entity id. Comparing the full entry set against the last synced one lets
/// [`InlineCommentsController::sync_blocks`] skip the render-tree rebuild when nothing changed.
type BlockEntry = (RenderLineLocation, Pixels, EntityId);

/// Embedded inline review-comment state for one `CodeEditorView`.
///
/// Owns the per-comment [`InlineCommentView`] handles (saved cards and unsaved drafts, keyed by
/// [`CommentId`]) and reconciles them into per-view [`CommentBlock`]s on the editor's render
/// state. The owning `CodeEditorView` keeps subscription wiring, comment persistence
/// (`save_comment`), and `CodeEditorEvent` emission; everything specific to the
/// `EmbeddedCodeReviewComments` flag's inline presentation lives here, so removing either the
/// embedded path or the legacy floating composer later stays localized.
///
/// Views are created and destroyed only in `&mut self` methods, never during `render`.
pub(crate) struct InlineCommentsController {
    /// Per-comment inline views, reconciled from the `ReviewCommentBatch` source of truth via
    /// [`Self::reconcile_saved`] (create new, update changed in place, drop removed) plus, when
    /// open, unsaved draft views. Only populated for line-targeted, non-outdated comments while
    /// the `EmbeddedCodeReviewComments` flag is enabled.
    views: HashMap<CommentId, ViewHandle<InlineCommentView>>,
    /// The block set most recently pushed to the render state, sorted deterministically.
    /// [`Self::sync_blocks`] early-outs when the desired set is unchanged, so frequent
    /// re-measure triggers (every inner body-editor layout pass) don't rebuild the host
    /// editor's content tree.
    ///
    /// The cache stays valid because the render state never drops `EmbeddedComment` blocks on
    /// its own: buffer edits and temporary-block resets both preserve them.
    last_synced_blocks: Vec<BlockEntry>,
}

impl InlineCommentsController {
    pub fn new() -> Self {
        Self {
            views: HashMap::new(),
            last_synced_blocks: Vec::new(),
        }
    }

    /// Reconcile the saved-comment views against the incoming batch, mirroring how
    /// `CommentListView::set_comments_internal` reconciles its cards: existing ids reuse (and
    /// update in place) their handle, new ids create one, and removed ids are dropped. Unsaved
    /// draft views are preserved. When the `EmbeddedCodeReviewComments` flag is off, no views are
    /// created so no inline blocks are rendered and there is no flag-off regression.
    pub fn reconcile_saved(
        &mut self,
        comments: Vec<EditorReviewComment>,
        ctx: &mut ViewContext<CodeEditorView>,
    ) {
        let mut new_views: HashMap<CommentId, ViewHandle<InlineCommentView>> = HashMap::new();

        if FeatureFlag::EmbeddedCodeReviewComments.is_enabled() {
            for comment in comments {
                let id = comment.id;
                let view = match self.views.remove(&id) {
                    Some(existing) => {
                        existing.update(ctx, |view, ctx| view.update_source(comment, ctx));
                        existing
                    }
                    None => {
                        let view =
                            ctx.add_typed_action_view(|ctx| InlineCommentView::new(comment, ctx));
                        Self::wire_view(&view, ctx);
                        view
                    }
                };
                new_views.insert(id, view);
            }
        }

        // Preserve unsaved draft views; any other handles still left correspond to removed saved
        // comments and are dropped here.
        for (id, view) in std::mem::take(&mut self.views) {
            if view.as_ref(ctx).is_new_draft() {
                new_views.insert(id, view);
            }
        }
        self.views = new_views;
    }

    /// Create a new unsaved draft view anchored below `line` and return its handle so the caller
    /// can focus its body.
    pub fn open_draft(
        &mut self,
        line: EditorLineLocation,
        ctx: &mut ViewContext<CodeEditorView>,
    ) -> ViewHandle<InlineCommentView> {
        let view = ctx.add_typed_action_view(|ctx| InlineCommentView::new_draft(line, ctx));
        Self::wire_view(&view, ctx);
        self.views.insert(view.as_ref(ctx).id(), view.clone());
        view
    }

    /// Switch the saved card for `id` into editing mode. Returns `false` when no inline view
    /// exists for that id.
    pub fn begin_editing(&mut self, id: &CommentId, ctx: &mut ViewContext<CodeEditorView>) -> bool {
        let Some(view) = self.views.get(id).cloned() else {
            return false;
        };
        view.update(ctx, |view, ctx| view.begin_editing(ctx));
        true
    }

    /// Apply a persisted comment back onto the same inline view (draft or edit -> saved).
    pub fn complete_save(
        &mut self,
        comment: EditorReviewComment,
        ctx: &mut ViewContext<CodeEditorView>,
    ) {
        if let Some(view) = self.views.get(&comment.id).cloned() {
            view.update(ctx, |view, ctx| view.complete_save(comment, ctx));
        }
    }

    /// Cancel the inline view for `id`: an unsaved draft is dropped entirely, while an edit of a
    /// saved comment is restored to its saved content.
    pub fn cancel(&mut self, id: &CommentId, ctx: &mut ViewContext<CodeEditorView>) {
        let Some(view) = self.views.get(id).cloned() else {
            return;
        };
        if view.as_ref(ctx).is_new_draft() {
            self.views.remove(id);
        } else {
            view.update(ctx, |view, ctx| view.cancel_editing(ctx));
        }
    }

    /// Drop all inline views. The next [`Self::sync_blocks`] clears the rendered blocks.
    pub fn clear(&mut self) {
        self.views.clear();
    }

    /// Reconcile the full set of inline comment blocks on the per-view render state with the
    /// current views: one block per inline view (saved card or unsaved draft), each reserving the
    /// view's current inline height at its anchor line. When the desired set matches what was
    /// last synced, this is a no-op, so callers can invoke it on every potential change (e.g.
    /// every body-editor layout pass) without rebuilding the host editor's content tree.
    ///
    /// When the `EmbeddedCodeReviewComments` flag is off the desired set is empty, which removes
    /// any previously synced blocks and otherwise does nothing.
    pub fn sync_blocks(
        &mut self,
        model: &ModelHandle<CodeEditorModel>,
        window_id: WindowId,
        ctx: &mut ViewContext<CodeEditorView>,
    ) {
        let mut entries: Vec<BlockEntry> = Vec::new();
        if FeatureFlag::EmbeddedCodeReviewComments.is_enabled() {
            for view in self.views.values() {
                let view_ref = view.as_ref(ctx);
                let location = view_ref
                    .line()
                    .clone()
                    .into_inline_comment_render_line_location();
                entries.push((location, view_ref.inline_height(ctx), view.id()));
            }
            // Deterministic ordering: keeps same-line stacking stable across syncs and makes the
            // equality check below meaningful despite hash-map iteration order.
            entries.sort_by_key(|(location, _, entity_id)| (*location, *entity_id));
        }

        if entries == self.last_synced_blocks {
            return;
        }

        let blocks = entries
            .iter()
            .map(|(location, height, entity_id)| {
                let size = vec2f(DEFAULT_COMMENT_MAX_WIDTH, height.as_f32());
                CommentBlock::new(
                    *location,
                    Arc::new(LaidOutInlineSavedComment::new(
                        *entity_id, window_id, size, *location,
                    )),
                )
            })
            .collect();
        model.update(ctx, |model, ctx| {
            model.set_inline_comment_blocks(blocks, ctx);
        });
        self.last_synced_blocks = entries;
    }

    /// Wire a newly created inline view: route its events to the owning `CodeEditorView` and
    /// re-sync the reserved blocks whenever its body editor re-lays out (so the block grows and
    /// shrinks with the content).
    fn wire_view(view: &ViewHandle<InlineCommentView>, ctx: &mut ViewContext<CodeEditorView>) {
        ctx.subscribe_to_view(view, |me, _, event, ctx| {
            me.handle_inline_comment_event(event, ctx);
        });
        let inner_render_state = view.as_ref(ctx).inner_render_state(ctx);
        ctx.observe(&inner_render_state, |me, _, ctx| {
            me.sync_inline_comment_blocks(ctx);
        });
    }
}
