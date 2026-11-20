use warpui::elements::MouseStateHandle;
use warpui::{AppContext, EntityId, ViewHandle};

use super::code_review_view::SplitBaselinePane;
use crate::code::local_code_editor::LocalCodeEditorView;

pub struct CodeReviewEditorState {
    pub editor: ViewHandle<LocalCodeEditorView>,
    unsaved_changes_mouse_state: MouseStateHandle,
    pub(super) editor_mouse_state: MouseStateHandle,
    /// Whether the buffer content has been loaded from disk (for global buffer mode).
    /// This is set to true when LocalCodeEditorEvent::DelayedRenderingFlushed or FailedToLoad fires.
    is_loaded: bool,
    /// The read-only base pane shown to the left of `editor` in the split diff layout.
    pub(crate) baseline: Option<SplitBaselinePane>,
}

impl CodeReviewEditorState {
    #[cfg(not(target_family = "wasm"))]
    pub fn new(editor: ViewHandle<LocalCodeEditorView>) -> Self {
        Self {
            editor,
            unsaved_changes_mouse_state: MouseStateHandle::default(),
            editor_mouse_state: MouseStateHandle::default(),
            is_loaded: false,
            baseline: None,
        }
    }

    /// Creates a new editor state that is already marked as loaded.
    /// Used for non-global buffer mode where content is loaded synchronously.
    pub fn new_loaded(editor: ViewHandle<LocalCodeEditorView>) -> Self {
        Self {
            editor,
            unsaved_changes_mouse_state: MouseStateHandle::default(),
            editor_mouse_state: MouseStateHandle::default(),
            is_loaded: true,
            baseline: None,
        }
    }

    /// Returns whether the buffer content has been loaded.
    pub fn is_loaded(&self) -> bool {
        self.is_loaded
    }

    /// Marks the editor as loaded.
    pub fn set_loaded(&mut self) {
        self.is_loaded = true;
    }

    pub fn editor(&self) -> &ViewHandle<LocalCodeEditorView> {
        &self.editor
    }

    pub fn unsaved_changes_mouse_state(&self) -> MouseStateHandle {
        self.unsaved_changes_mouse_state.clone()
    }

    /// The editors shown for this file: the base pane first when the split layout is active,
    /// then the modified pane.
    pub fn panes(&self) -> impl Iterator<Item = &ViewHandle<LocalCodeEditorView>> {
        self.baseline
            .as_ref()
            .map(|baseline| &baseline.editor)
            .into_iter()
            .chain(std::iter::once(&self.editor))
    }

    /// The pane with the given editor id, if it belongs to this file.
    pub fn pane(&self, editor_id: EntityId) -> Option<&ViewHandle<LocalCodeEditorView>> {
        self.panes().find(|pane| pane.id() == editor_id)
    }

    /// The pane with the given editor id, or the modified pane when no id is given.
    pub fn pane_or_modified(
        &self,
        editor_id: Option<EntityId>,
    ) -> Option<&ViewHandle<LocalCodeEditorView>> {
        match editor_id {
            Some(editor_id) => self.pane(editor_id),
            None => Some(&self.editor),
        }
    }

    pub fn has_unsaved_changes(&self, ctx: &AppContext) -> bool {
        self.editor.as_ref(ctx).has_unsaved_changes(ctx)
    }
}
