use warp_core::features::FeatureFlag;
use warpui::elements::ScrollOffset;
use warpui::units::Pixels;
#[cfg(not(target_family = "wasm"))]
use warpui::{AppContext, WeakViewHandle};
use warpui::{ViewContext, ViewHandle};

#[cfg(not(target_family = "wasm"))]
use super::FILE_HEADER_HEIGHT;
use super::{CodeReviewView, CodeReviewViewState};
use crate::code::editor::model::StableEditorLine;
use crate::code::local_code_editor::LocalCodeEditorView;

/// Context for preserving scroll position across file diff content changes.
/// The scroll position can be in different regions of the file diff item.
#[derive(Clone, Debug)]
#[cfg_attr(target_family = "wasm", allow(dead_code))]
pub(super) enum RelocatableScrollContext {
    /// Scroll position is in the file header region.
    /// Stores the pixel offset from the top of the header.
    Header { offset: Pixels },
    /// Scroll is on a line in the editor content (current or removed).
    /// The [`StableEditorLine`] contains an internal anchor that
    /// automatically tracks through buffer edits.
    EditorLine {
        stable_line: StableEditorLine,
        intra_line_offset: Pixels,
    },
    /// Scroll position is in the footer region.
    /// Stores the pixel offset from the top of the footer.
    Footer { offset: Pixels },
}

impl CodeReviewView {
    /// Computes the adjusted item-relative scroll offset for a file diff item
    /// based on the captured scroll context. Called by the `ListState`
    /// adjustment closure when an item's height changes during layout.
    #[cfg(not(target_family = "wasm"))]
    pub(super) fn adjust_scroll_offset(
        view_handle: &WeakViewHandle<Self>,
        index: usize,
        captured_context: &RelocatableScrollContext,
        app: &AppContext,
    ) -> Option<Pixels> {
        if !FeatureFlag::CodeReviewScrollPreservation.is_enabled() {
            return None;
        }

        // The adjustment function returns item-relative offsets (offset_from_start),
        // NOT absolute positions. The ListState stores the result directly in
        // scroll_top.offset_from_start, which is relative to the current scroll item.

        match captured_context {
            RelocatableScrollContext::Header { offset } => Some(*offset),
            RelocatableScrollContext::EditorLine {
                stable_line,
                intra_line_offset,
            } => {
                let view_handle = view_handle.upgrade(app)?;
                let view = view_handle.as_ref(app);

                let CodeReviewViewState::Loaded(state) = view.state() else {
                    return None;
                };

                let editor_state = state
                    .file_states
                    .get_index(index)?
                    .1
                    .editor_state
                    .as_ref()?;
                let editor_view = editor_state.editor.as_ref(app).editor().as_ref(app);

                let line_offset = editor_view.line_top(stable_line, app)?;

                Some(Pixels::new(FILE_HEADER_HEIGHT) + line_offset + *intra_line_offset)
            }
            RelocatableScrollContext::Footer { offset } => {
                let view_handle = view_handle.upgrade(app)?;
                let view = view_handle.as_ref(app);

                let CodeReviewViewState::Loaded(state) = view.state() else {
                    return None;
                };

                let editor_state = state
                    .file_states
                    .get_index(index)?
                    .1
                    .editor_state
                    .as_ref()?;

                let content_height = editor_state
                    .editor
                    .as_ref(app)
                    .editor()
                    .as_ref(app)
                    .content_height(app);

                Some(Pixels::new(FILE_HEADER_HEIGHT) + content_height + *offset)
            }
        }
    }

    /// Computes the scroll preservation context for the given index and editor.
    /// Returns `Some(context)` only if the index is the currently scrolled item.
    /// Detects whether scroll is in header, editor content, or footer region.
    #[cfg(not(target_family = "wasm"))]
    pub(super) fn compute_scroll_context_for_index(
        &self,
        index: usize,
        editor: &ViewHandle<LocalCodeEditorView>,
        ctx: &mut ViewContext<Self>,
    ) -> Option<RelocatableScrollContext> {
        // Only compute context if this is the currently scrolled item
        let current_scroll_index = self.viewported_list_state.get_scroll_index();
        if current_scroll_index != index {
            return None;
        }

        // Get the scroll offset within this item
        let scroll_offset_in_item = self.viewported_list_state.get_scroll_offset();
        let file_header_height = Pixels::new(FILE_HEADER_HEIGHT);

        // Check if scroll is in the header region
        if scroll_offset_in_item < file_header_height {
            return Some(RelocatableScrollContext::Header {
                offset: scroll_offset_in_item,
            });
        }

        // Compute offset relative to editor content
        let scroll_offset_in_editor = scroll_offset_in_item - file_header_height;

        // Check footer region using the view-level accessor.
        let editor_view = editor.as_ref(ctx).editor();
        let content_height = editor_view.as_ref(ctx).content_height(ctx);

        if scroll_offset_in_editor >= content_height {
            return Some(RelocatableScrollContext::Footer {
                offset: scroll_offset_in_editor - content_height,
            });
        }

        // Clone the model handle so immutable borrows on ctx can be released
        // before the anchor creation which requires a mutable borrow.
        let editor_model_handle = editor_view.as_ref(ctx).model.clone();

        // Identify the line and create anchors (requires mutable context).
        let (stable_line, intra_line_offset) = editor_model_handle.update(ctx, |model, ctx| {
            model.line_at_vertical_offset(scroll_offset_in_editor, ctx)
        })?;

        Some(RelocatableScrollContext::EditorLine {
            stable_line,
            intra_line_offset,
        })
    }

    /// Wasm stub - scroll preservation not supported
    #[cfg(target_family = "wasm")]
    pub(super) fn compute_scroll_context_for_index(
        &self,
        _index: usize,
        _editor: &ViewHandle<LocalCodeEditorView>,
        _ctx: &mut ViewContext<Self>,
    ) -> Option<RelocatableScrollContext> {
        None
    }

    /// Called when scrolling settles (via debounced scroll events).
    /// Computes and stores the current scroll context on the ListState
    /// so the explicit invalidation path can adjust scroll position
    /// when a file diff item's height changes.
    fn on_scroll_settled(&mut self, _scroll_offset: ScrollOffset, ctx: &mut ViewContext<Self>) {
        let scroll_index = self.viewported_list_state.get_scroll_index();

        let Some(repo) = self.active_repo.as_ref() else {
            return;
        };

        let CodeReviewViewState::Loaded(state) = &repo.state else {
            return;
        };

        let Some((_, file_state)) = state.file_states.get_index(scroll_index) else {
            return;
        };

        let Some(editor_state) = &file_state.editor_state else {
            return;
        };

        let editor = editor_state.editor();

        if let Some(context) = self.compute_scroll_context_for_index(scroll_index, editor, ctx) {
            self.viewported_list_state.set_scroll_context(Some(context));
        }
    }

    /// Sets up scroll tracking from a scroll event receiver.
    /// Calls [`Self::on_scroll_settled`] on every scroll event so the
    /// scroll context is always up-to-date.
    /// No-op when [`FeatureFlag::CodeReviewScrollPreservation`] is disabled.
    pub(super) fn setup_scroll_tracking(
        scroll_rx: async_channel::Receiver<ScrollOffset>,
        ctx: &mut ViewContext<Self>,
    ) {
        if FeatureFlag::CodeReviewScrollPreservation.is_enabled() {
            ctx.spawn_stream_local(scroll_rx, Self::on_scroll_settled, |_, _| {});
        }
    }
}
