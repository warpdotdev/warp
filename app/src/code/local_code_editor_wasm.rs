use std::{path::PathBuf, rc::Rc};

use std::ops::Range;

use warp_editor::{content::buffer::InitialBufferState, render::model::LineCount};
use warp_util::file::{FileLoadError, FileSaveError};
use warpui::{
    elements::MouseStateHandle, AppContext, Element, Entity, TypedActionView, View, ViewContext,
    ViewHandle, WindowId,
};

use ai::diff_validation::DiffType;

use super::editor::view::CodeEditorView;
use super::ImmediateSaveError;
use crate::code::buffer_location::LocalOrRemotePath as BufferFileLocation;
use crate::terminal::TerminalView;
use crate::{code::editor::EditorReviewComment, code_review::comments::CommentId};
use warp_core::ui::appearance::Appearance;

pub use super::diff_viewer::DisplayMode;

#[derive(Debug)]
pub enum LocalCodeEditorEvent {
    #[allow(dead_code)]
    FileLoaded,
    #[allow(dead_code)]
    FailedToLoad { error: Rc<FileLoadError> },
    #[allow(dead_code)]
    FileSaved,
    #[allow(dead_code)]
    FailedToSave { error: Rc<FileSaveError> },
    #[allow(dead_code)]
    DiffAccepted,
    #[allow(dead_code)]
    DiffRejected,
    #[allow(dead_code)]
    VimMinimizeRequested,
    #[allow(dead_code)]
    UserEdited,
    #[allow(dead_code)]
    DiffStatusUpdated,
    #[allow(dead_code)]
    SelectionAddedAsContext {
        relative_file_path: String,
        line_range: Range<LineCount>,
        selected_text: String,
    },
    #[allow(dead_code)]
    DiscardUnsavedChanges { path: PathBuf },
    #[allow(dead_code)]
    CommentSaved { comment: EditorReviewComment },
    #[allow(dead_code)]
    DeleteComment { id: CommentId },
    #[allow(dead_code)]
    RequestOpenComment(CommentId),
    #[allow(dead_code)]
    ViewportUpdated,
    #[allow(dead_code)]
    DelayedRenderingFlushed,
    #[allow(dead_code)]
    LayoutInvalidated,
}

pub struct LocalCodeEditorView {
    editor: ViewHandle<CodeEditorView>,
}

impl LocalCodeEditorView {
    pub fn new(
        editor: ViewHandle<CodeEditorView>,
        _diff_type: Option<DiffType>,
        _enable_diff_nav_by_default: bool,
        _display_mode: Option<DisplayMode>,
        _ctx: &mut ViewContext<Self>,
    ) -> Self {
        Self { editor }
    }

    pub fn with_selection_as_context(self, _terminal_target_fn: Box<TerminalTargetFn>) -> Self {
        self
    }

    pub fn reset_with_state(&mut self, _state: InitialBufferState, _ctx: &mut ViewContext<Self>) {}

    pub fn editor(&self) -> &ViewHandle<CodeEditorView> {
        &self.editor
    }

    pub fn save_local(&self, _ctx: &mut ViewContext<Self>) -> Result<(), ImmediateSaveError> {
        Err(ImmediateSaveError::NoFileId)
    }

    pub fn has_unsaved_changes(&self, _ctx: &AppContext) -> bool {
        false
    }

    /// Returns the unified file location (local or remote).
    /// The WASM stub has no backing file, so this always returns `None`.
    pub fn file_location(&self) -> Option<&BufferFileLocation> {
        None
    }
}

impl Entity for LocalCodeEditorView {
    type Event = LocalCodeEditorEvent;
}

impl View for LocalCodeEditorView {
    fn ui_name() -> &'static str {
        "LocalCodeEditorView"
    }
    fn render(&self, _app: &AppContext) -> Box<dyn Element> {
        warpui::elements::Empty::new().finish()
    }
}

impl TypedActionView for LocalCodeEditorView {
    type Action = ();
}

type TerminalTargetFn = dyn Fn(WindowId, &AppContext) -> Option<ViewHandle<TerminalView>>;

pub fn render_unsaved_circle_with_tooltip(
    _mouse_state: MouseStateHandle,
    _tooltip_text: String,
    _size: f32,
    _right_margin: f32,
    _appearance: &Appearance,
) -> Box<dyn Element> {
    warpui::elements::Empty::new().finish()
}
