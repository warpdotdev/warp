use std::rc::Rc;

use warp_util::file::FileLoadError;
use warpui::elements::MouseStateHandle;
use warpui::{AppContext, ViewHandle};

use crate::code::local_code_editor::LocalCodeEditorView;

pub struct CodeReviewEditorState {
    pub editor: ViewHandle<LocalCodeEditorView>,
    unsaved_changes_mouse_state: MouseStateHandle,
    pub(super) editor_mouse_state: MouseStateHandle,
    load_finished: bool,
    load_error: Option<Rc<FileLoadError>>,
}

impl CodeReviewEditorState {
    #[cfg(not(target_family = "wasm"))]
    pub fn new(editor: ViewHandle<LocalCodeEditorView>) -> Self {
        Self {
            editor,
            unsaved_changes_mouse_state: MouseStateHandle::default(),
            editor_mouse_state: MouseStateHandle::default(),
            load_finished: false,
            load_error: None,
        }
    }

    /// Creates a new editor state that is already marked as loaded.
    /// Used for non-global buffer mode where content is loaded synchronously.
    pub fn new_loaded(editor: ViewHandle<LocalCodeEditorView>) -> Self {
        Self {
            editor,
            unsaved_changes_mouse_state: MouseStateHandle::default(),
            editor_mouse_state: MouseStateHandle::default(),
            load_finished: true,
            load_error: None,
        }
    }

    pub fn load_finished(&self) -> bool {
        self.load_finished
    }

    pub fn set_load_result(&mut self, error: Option<Rc<FileLoadError>>) {
        self.load_finished = true;
        self.load_error = error;
    }

    pub fn load_error(&self) -> Option<&FileLoadError> {
        self.load_error.as_deref()
    }

    pub fn editor(&self) -> &ViewHandle<LocalCodeEditorView> {
        &self.editor
    }

    pub fn unsaved_changes_mouse_state(&self) -> MouseStateHandle {
        self.unsaved_changes_mouse_state.clone()
    }

    pub fn has_unsaved_changes(&self, ctx: &AppContext) -> bool {
        self.editor.as_ref(ctx).has_unsaved_changes(ctx)
    }
}
