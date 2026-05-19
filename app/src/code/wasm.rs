use std::path::{Path, PathBuf};

use warp_util::path::LineAndColumnArg;
use warpui::{
    elements::{DraggableState, Empty, MouseStateHandle},
    AppContext, Element, Entity, ModelHandle, TypedActionView, View, ViewContext, ViewHandle,
};

use super::{
    buffer_location::LocalOrRemotePath, editor_management::CodeSource,
    local_code_editor::LocalCodeEditorView,
};
use crate::pane_group::{
    focus_state::PaneFocusHandle,
    pane::view::{HeaderContent, HeaderRenderContext},
    BackingView, CodePane, PaneConfiguration, PaneEvent,
};
use ai::diff_validation::DiffDelta;

// Keybinding constants - exported so AI document view can reuse
pub const SAVE_FILE_BINDING_NAME: &str = "code_view:save";
pub const SAVE_FILE_BINDING_DESCRIPTION: &str = "Save file";

#[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
pub fn is_supported_code_file(_path: impl AsRef<Path>) -> bool {
    false
}

#[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
pub fn is_binary_file(_path: impl AsRef<Path>) -> bool {
    false
}

pub fn init(_app: &mut AppContext) {}

#[derive(Debug, Clone)]
pub enum CodeViewAction {
    RemoveTabAtIndex { index: usize },
}

#[derive(Debug, Clone)]
#[allow(unused)]
pub enum CodeViewEvent {
    Pane(PaneEvent),
    TabChanged {
        location: Option<LocalOrRemotePath>,
        tab_index: usize,
    },
    FileOpened {
        location: LocalOrRemotePath,
        tab_index: usize,
    },
    RunTabConfigSkill {
        path: PathBuf,
    },
    OpenLspLogs {
        log_path: PathBuf,
    },
}

#[derive(Debug, Clone)]
pub enum PendingSaveIntent {
    Save,
    Discard,
    Cancel,
}

#[allow(unused)]
#[derive(Debug, Clone)]
enum TabBarDragPosition {
    BeforeTab,
    AfterTab,
}

#[allow(unused)]
#[derive(Default, Clone)]
struct TabDataMouseStateHandles {
    tab_handle: MouseStateHandle,
    close_handle: MouseStateHandle,
    accept_mouse_state: MouseStateHandle,
    reject_mouse_state: MouseStateHandle,
    tab_draggable_state: DraggableState,
}

#[allow(unused)]
#[derive(Clone)]
pub struct TabData {
    location: Option<LocalOrRemotePath>,
    editor_view: ViewHandle<LocalCodeEditorView>,
    mouse_state_handles: TabDataMouseStateHandles,
    drag_position: Option<TabBarDragPosition>,
}

impl TabData {
    /// Returns the local filesystem path, if this tab is backed by a local file.
    /// Returns `None` for remote files and untitled tabs.
    pub fn local_path(&self) -> Option<PathBuf> {
        self.location
            .as_ref()
            .and_then(|loc| PathBuf::try_from(loc.clone()).ok())
    }
}

pub struct CodeView {
    pane_configuration: ModelHandle<PaneConfiguration>,
    source: CodeSource,
}

impl CodeView {
    pub fn new(
        source: CodeSource,
        _line_col: Option<LineAndColumnArg>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let pane_configuration = ctx.add_model(|_ctx| PaneConfiguration::new(""));

        Self {
            pane_configuration,
            source,
        }
    }

    pub fn tab_at(&self, _index: usize) -> Option<&TabData> {
        None
    }

    pub fn tab_count(&self) -> usize {
        0
    }

    pub fn active_tab_index(&self) -> usize {
        0
    }

    pub fn source(&self) -> &CodeSource {
        &self.source
    }

    pub fn open_or_focus_existing(
        &mut self,
        location: Option<LocalOrRemotePath>,
        line_col: Option<LineAndColumnArg>,
        ctx: &mut ViewContext<Self>,
    ) {
        if let Some(path) = location.and_then(|loc| PathBuf::try_from(loc).ok()) {
            self.open_local(None, path, line_col, ctx);
        }
    }

    pub fn open_local(
        &mut self,
        _diffs: Option<Vec<DiffDelta>>,
        _path: impl Into<PathBuf>,
        _line_col: Option<LineAndColumnArg>,
        _ctx: &mut ViewContext<Self>,
    ) {
    }

    pub fn local_path(&self, _ctx: &AppContext) -> Option<PathBuf> {
        None
    }

    pub fn focus(&self, _ctx: &mut ViewContext<Self>) {}

    pub fn pane_configuration(&self) -> ModelHandle<PaneConfiguration> {
        self.pane_configuration.clone()
    }

    pub fn contains_unsaved_changes(&self, _ctx: &AppContext) -> bool {
        false
    }

    pub fn active_tab_has_unsaved_changes(&self, _ctx: &AppContext) -> bool {
        false
    }

    pub fn close_overlays(&mut self, _ctx: &mut ViewContext<Self>) {
        // Not yet implemented
    }

    pub fn remove_tab_for_move(
        &mut self,
        _index: usize,
        _ctx: &mut ViewContext<Self>,
    ) -> Option<CodePane> {
        None
    }
}

impl Entity for CodeView {
    type Event = CodeViewEvent;
}

impl View for CodeView {
    fn ui_name() -> &'static str {
        "CodeView"
    }

    fn render(&self, _app: &AppContext) -> Box<dyn Element> {
        Empty::new().finish()
    }
}

impl TypedActionView for CodeView {
    type Action = CodeViewAction;

    fn handle_action(&mut self, _action: &Self::Action, _ctx: &mut ViewContext<Self>) {}
}

impl BackingView for CodeView {
    type PaneHeaderOverflowMenuAction = CodeViewAction;
    type CustomAction = ();
    type AssociatedData = ();

    fn handle_pane_header_overflow_menu_action(
        &mut self,
        _action: &Self::PaneHeaderOverflowMenuAction,
        _ctx: &mut ViewContext<Self>,
    ) {
    }

    fn close(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.emit(CodeViewEvent::Pane(PaneEvent::Close));
    }

    fn focus_contents(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.focus_self();
    }

    fn handle_custom_action(
        &mut self,
        _custom_action: &Self::CustomAction,
        _ctx: &mut ViewContext<Self>,
    ) {
    }

    fn render_header_content(
        &self,
        _ctx: &HeaderRenderContext<'_>,
        app: &AppContext,
    ) -> HeaderContent {
        HeaderContent::simple(self.pane_configuration.as_ref(app).title())
    }

    fn set_focus_handle(&mut self, _handle: PaneFocusHandle, _ctx: &mut ViewContext<Self>) {}
}
