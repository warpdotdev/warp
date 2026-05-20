use std::collections::HashSet;
use std::path::PathBuf;

use warp_core::ui::theme::color::internal_colors;
use warp_core::{send_telemetry_from_ctx, ui::Icon};
use warp_util::path::LineAndColumnArg;
use warpui::{
    elements::{
        resizable_state_handle, ChildView, ConstrainedBox, Container, CrossAxisAlignment,
        DragBarSide, Element, Empty, Flex, MainAxisAlignment, MainAxisSize, MouseStateHandle,
        ParentElement, Resizable, ResizableStateHandle, Shrinkable,
    },
    platform::Cursor,
    ui_components::components::{Coords, UiComponent, UiComponentStyles},
    AppContext, Entity, FocusContext, ModelHandle, SingletonEntity, TypedActionView, View,
    ViewContext, ViewHandle, WeakViewHandle,
};

use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent_conversations_model::AgentConversationsModel;
use crate::code::buffer_location::LocalOrRemotePath;
#[cfg(feature = "local_fs")]
use crate::code::file_tree::FileTreeEvent;
use crate::coding_panel_enablement_state::CodingPanelEnablementState;
use crate::drive::panel::{DrivePanel, DrivePanelEvent};
use crate::pane_group::working_directories::WorkingDirectory;
use crate::pane_group::{PaneGroup, WorkingDirectoriesEvent, WorkingDirectoriesModel};
#[cfg(feature = "local_fs")]
use crate::server::telemetry::CodePanelsFileOpenEntrypoint;
use crate::server::telemetry::{FileTreeSource, WarpDriveSource};
use crate::settings_view::keybindings::{KeybindingChangedEvent, KeybindingChangedNotifier};
#[cfg(feature = "local_fs")]
use crate::util::file::external_editor::EditorSettings;
#[cfg(feature = "local_fs")]
use crate::util::openable_file_type::resolve_file_target_with_editor_choice;
use crate::util::openable_file_type::FileTarget;
use crate::workspace::view::conversation_list::view::{
    ConversationListView, Event as ConversationListViewEvent,
};
use crate::workspace::view::global_search::view::{
    Event as GlobalSearchViewEvent, GlobalSearchEntryFocus, GlobalSearchView,
};
use crate::workspace::view::{
    LEFT_PANEL_AGENT_CONVERSATIONS_BINDING_NAME, LEFT_PANEL_GLOBAL_SEARCH_BINDING_NAME,
    LEFT_PANEL_PROJECT_EXPLORER_BINDING_NAME, LEFT_PANEL_WARP_DRIVE_BINDING_NAME,
    OPEN_GLOBAL_SEARCH_BINDING_NAME, TOGGLE_CONVERSATION_LIST_VIEW_BINDING_NAME,
    TOGGLE_PROJECT_EXPLORER_BINDING_NAME, TOGGLE_WARP_DRIVE_BINDING_NAME,
};
use crate::{
    appearance::Appearance,
    code::file_tree::FileTreeView,
    drive::panel::{MAX_SIDEBAR_WIDTH_RATIO, MIN_SIDEBAR_WIDTH},
    pane_group::pane::view::header::{components::HEADER_EDGE_PADDING, PANE_HEADER_HEIGHT},
    pane_group::{self},
    terminal::resizable_data::{ModalType, ResizableData},
    ui_components::{
        buttons::{icon_button, icon_button_with_color},
        icons,
    },
    util::bindings::keybinding_name_to_display_string,
    workspace::WorkspaceAction,
    TelemetryEvent,
};

#[derive(Default)]
struct MouseStateHandles {
    project_explorer_button: MouseStateHandle,
    global_search_button: MouseStateHandle,
    warp_drive_button: MouseStateHandle,
    conversation_list_view_button: MouseStateHandle,
}

#[derive(Clone, Debug)]
pub enum LeftPanelAction {
    ProjectExplorer,
    GlobalSearch { entry_focus: GlobalSearchEntryFocus },
    WarpDrive,
    ConversationListView,
}

pub enum LeftPanelEvent {
    #[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
    FileTree(pane_group::Event),
    WarpDrive(DrivePanelEvent),
    #[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
    OpenFileWithTarget {
        location: LocalOrRemotePath,
        target: FileTarget,
        line_col: Option<LineAndColumnArg>,
    },
    NewConversationInNewTab,
    ShowDeleteConfirmationDialog {
        conversation_id: AIConversationId,
        conversation_title: String,
        terminal_view_id: Option<warpui::EntityId>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolPanelView {
    ProjectExplorer,
    GlobalSearch { entry_focus: GlobalSearchEntryFocus },
    WarpDrive,
    ConversationListView,
}

/// Encapsulates the active view state to enforce that all mutations go through
/// `active_view_state::set`, which handles necessary side effects.
mod active_view_state {
    use super::ToolPanelView;
    use warpui::ViewContext;

    pub struct ActiveViewState(ToolPanelView);

    impl ActiveViewState {
        pub fn get(&self) -> ToolPanelView {
            self.0
        }
    }

    pub fn new(view: ToolPanelView) -> ActiveViewState {
        ActiveViewState(view)
    }

    pub fn set(
        left_panel: &mut super::LeftPanelView,
        new_view: ToolPanelView,
        ctx: &mut ViewContext<super::LeftPanelView>,
    ) {
        let previous = left_panel.active_view.0;
        left_panel.active_view.0 = new_view;
        left_panel.update_button_active_states();
        ctx.notify();

        let was_conversation_list_open = previous == ToolPanelView::ConversationListView;
        let is_conversation_list_open = new_view == ToolPanelView::ConversationListView;
        if was_conversation_list_open && !is_conversation_list_open {
            left_panel.on_conversation_list_view_visibility_changed(false, ctx);
        } else if !was_conversation_list_open && is_conversation_list_open {
            left_panel.on_conversation_list_view_visibility_changed(true, ctx);
        }

        left_panel.update_active_file_tree_subscription_state(ctx);
    }
}

pub struct ToolbeltButtonConfig {
    pub icon: warp_core::ui::Icon,
    /// Optional icon to use when the given toolbelt option is in an active state.
    pub active_icon: Option<warp_core::ui::Icon>,
    pub tooltip_text: String,
    pub action: LeftPanelAction,
    /// Whether the button should be rendered with an "active" state.
    pub render_with_active_state: bool,
    /// Ordered list of binding names used to populate the tooltip keybinding display.
    ///
    /// Earlier bindings in the list are preferred in the tooltip.
    pub tooltip_keybinding_names: Vec<&'static str>,
    /// Cached keybinding display string for the tooltip.
    ///
    /// This is updated in response to [`KeybindingChangedEvent`]s.
    pub tooltip_keybinding: Option<String>,
}

pub struct LeftPanelView {
    resizable_state_handle: ResizableStateHandle,
    mouse_state_handles: MouseStateHandles,
    close_button_mouse_state: MouseStateHandle,
    warp_drive_view: ViewHandle<DrivePanel>,
    conversation_list_view: ViewHandle<ConversationListView>,
    active_view: active_view_state::ActiveViewState,
    toolbelt_buttons: Vec<ToolbeltButtonConfig>,
    active_pane_group: Option<WeakViewHandle<PaneGroup>>,
    #[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
    working_directories_model: ModelHandle<WorkingDirectoriesModel>,
    is_agent_management_view_open: bool,
    panel_position: super::PanelPosition,
}

fn toolbelt_tooltip_keybinding(binding_names: &[&'static str], app: &AppContext) -> Option<String> {
    let mut parts = Vec::new();
    let mut seen = HashSet::new();

    // Preserve caller-provided ordering so we can prioritize specific bindings.
    for binding_name in binding_names {
        if let Some(displayed) = keybinding_name_to_display_string(binding_name, app) {
            if seen.insert(displayed.clone()) {
                parts.push(displayed);
            }
        }
    }

    (!parts.is_empty()).then(|| parts.join(", "))
}

impl LeftPanelView {
    pub fn new(
        working_directories_model: ModelHandle<WorkingDirectoriesModel>,
        views: Vec<ToolPanelView>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let resizable_data_handle = ResizableData::handle(ctx);
        let resizable_state_handle = match resizable_data_handle
            .as_ref(ctx)
            .get_handle(ctx.window_id(), ModalType::LeftPanelWidth)
        {
            Some(handle) => handle,
            None => {
                log::error!("Couldn't retrieve left panel resizable state handle.");
                resizable_state_handle(600.0)
            }
        };
        let warp_drive_view = ctx.add_typed_action_view(DrivePanel::new);
        let conversation_list_view = ctx.add_typed_action_view(ConversationListView::new);

        ctx.subscribe_to_view(&warp_drive_view, |_me, _, event, ctx| {
            ctx.emit(LeftPanelEvent::WarpDrive(event.clone()));
        });

        ctx.subscribe_to_view(&conversation_list_view, |_me, _, event, ctx| match event {
            ConversationListViewEvent::NewConversationInNewTab => {
                ctx.emit(LeftPanelEvent::NewConversationInNewTab);
            }
            ConversationListViewEvent::ShowDeleteConfirmationDialog {
                conversation_id,
                conversation_title,
                terminal_view_id,
            } => {
                ctx.emit(LeftPanelEvent::ShowDeleteConfirmationDialog {
                    conversation_id: *conversation_id,
                    conversation_title: conversation_title.clone(),
                    terminal_view_id: *terminal_view_id,
                });
            }
        });

        let active_view = views.first().copied().unwrap_or(ToolPanelView::WarpDrive);
        let toolbelt_buttons = views
            .iter()
            .map(|view| Self::create_toolbelt_button_config(view, ctx))
            .collect();

        ctx.subscribe_to_model(
            &KeybindingChangedNotifier::handle(ctx),
            |me, _, event, ctx| match event {
                KeybindingChangedEvent::BindingChanged { .. } => {
                    for button in &mut me.toolbelt_buttons {
                        button.tooltip_keybinding =
                            toolbelt_tooltip_keybinding(&button.tooltip_keybinding_names, ctx);
                    }

                    ctx.notify();
                }
            },
        );

        ctx.subscribe_to_model(&working_directories_model, |me, _, event, ctx| {
            if let WorkingDirectoriesEvent::DirectoriesChanged {
                pane_group_id,
                directories,
            } = event
            {
                let Some(active_pane_group) = &me.active_pane_group else {
                    return;
                };
                let Some(active_pane_group) = active_pane_group.upgrade(ctx) else {
                    return;
                };
                if active_pane_group.id() != *pane_group_id {
                    return;
                }
                let has_terminal_session = directories.iter().any(|dir| dir.terminal_id.is_some());

                // Update GlobalSearchView root directories based on all working directories
                let roots: Vec<PathBuf> = directories.iter().map(|d| d.path.clone()).collect();

                let global_search_view =
                    me.get_or_create_global_search_view_for_pane_group(active_pane_group.id(), ctx);
                global_search_view.update(ctx, |view, view_ctx| {
                    view.set_root_directories(roots, view_ctx);
                });

                let directories: Vec<PathBuf> =
                    directories.iter().map(|dir| dir.path.clone()).collect();

                // Directories are already in display order (most recent first) from the model
                let directories = deduplicate_by_directory_name(directories);
                let file_tree_view =
                    me.get_or_create_file_tree_view_for_pane_group(active_pane_group.id(), ctx);

                let is_visible =
                    active_pane_group.as_ref(ctx).left_panel_open && me.is_file_tree_active();
                file_tree_view.update(ctx, |view, ctx| {
                    view.set_root_directories(directories, ctx);
                    view.set_has_terminal_session(has_terminal_session, ctx);
                    view.set_is_active(is_visible, ctx);

                    if is_visible {
                        view.auto_expand_to_most_recent_directory(ctx);
                    }
                });
                ctx.notify();
            }
        });

        let mut view = Self {
            resizable_state_handle,
            mouse_state_handles: Default::default(),
            close_button_mouse_state: Default::default(),
            warp_drive_view,
            conversation_list_view,
            active_view: active_view_state::new(active_view),
            toolbelt_buttons,
            active_pane_group: None,
            working_directories_model,
            is_agent_management_view_open: false,
            panel_position: super::PanelPosition::Left,
        };
        view.update_button_active_states();

        view
    }

    pub fn set_agent_management_view_open(&mut self, is_open: bool, ctx: &mut ViewContext<Self>) {
        self.is_agent_management_view_open = is_open;
        ctx.notify();
    }

    pub fn set_panel_position(
        &mut self,
        position: super::PanelPosition,
        ctx: &mut ViewContext<Self>,
    ) {
        self.panel_position = position;
        ctx.notify();
    }

    /// Updates the available tool panel views.
    /// If the currently active view is no longer available, switches to the first available view.
    pub fn update_available_views(
        &mut self,
        views: Vec<ToolPanelView>,
        ctx: &mut ViewContext<Self>,
    ) {
        // Check if the current active view is still available
        let current_view = self.active_view.get();
        let is_current_view_available = views.iter().any(|v| {
            // Use discriminant comparison for GlobalSearch since it has inner data
            match (v, &current_view) {
                (ToolPanelView::GlobalSearch { .. }, ToolPanelView::GlobalSearch { .. }) => true,
                _ => std::mem::discriminant(v) == std::mem::discriminant(&current_view),
            }
        });

        // Rebuild toolbelt buttons
        self.toolbelt_buttons = views
            .iter()
            .map(|view| Self::create_toolbelt_button_config(view, ctx))
            .collect();

        // If current view is no longer available, switch to the first available view
        if !is_current_view_available {
            if let Some(first_view) = views.first().copied() {
                active_view_state::set(self, first_view, ctx);
            }
        } else {
            self.update_button_active_states();
        }

        ctx.notify();
    }

    fn create_toolbelt_button_config(
        view: &ToolPanelView,
        ctx: &ViewContext<Self>,
    ) -> ToolbeltButtonConfig {
        match view {
            ToolPanelView::ProjectExplorer => {
                let tooltip_keybinding_names = vec![
                    LEFT_PANEL_PROJECT_EXPLORER_BINDING_NAME,
                    TOGGLE_PROJECT_EXPLORER_BINDING_NAME,
                ];

                ToolbeltButtonConfig {
                    icon: Icon::FileCopy,
                    active_icon: None,
                    tooltip_text: "Project explorer".to_string(),
                    action: LeftPanelAction::ProjectExplorer,
                    render_with_active_state: false,
                    tooltip_keybinding: toolbelt_tooltip_keybinding(&tooltip_keybinding_names, ctx),
                    tooltip_keybinding_names,
                }
            }
            ToolPanelView::GlobalSearch { .. } => {
                let tooltip_keybinding_names = vec![
                    LEFT_PANEL_GLOBAL_SEARCH_BINDING_NAME,
                    OPEN_GLOBAL_SEARCH_BINDING_NAME,
                ];

                ToolbeltButtonConfig {
                    icon: Icon::Search,
                    active_icon: None,
                    tooltip_text: "Global search".to_string(),
                    action: LeftPanelAction::GlobalSearch {
                        entry_focus: GlobalSearchEntryFocus::QueryEditor,
                    },
                    render_with_active_state: false,
                    tooltip_keybinding: toolbelt_tooltip_keybinding(&tooltip_keybinding_names, ctx),
                    tooltip_keybinding_names,
                }
            }
            ToolPanelView::WarpDrive => {
                let tooltip_keybinding_names = vec![
                    LEFT_PANEL_WARP_DRIVE_BINDING_NAME,
                    TOGGLE_WARP_DRIVE_BINDING_NAME,
                ];

                ToolbeltButtonConfig {
                    icon: Icon::WarpDrive,
                    active_icon: None,
                    tooltip_text: "Warp Drive".to_string(),
                    action: LeftPanelAction::WarpDrive,
                    render_with_active_state: false,
                    tooltip_keybinding: toolbelt_tooltip_keybinding(&tooltip_keybinding_names, ctx),
                    tooltip_keybinding_names,
                }
            }
            ToolPanelView::ConversationListView => {
                let tooltip_keybinding_names = vec![
                    LEFT_PANEL_AGENT_CONVERSATIONS_BINDING_NAME,
                    TOGGLE_CONVERSATION_LIST_VIEW_BINDING_NAME,
                ];

                ToolbeltButtonConfig {
                    icon: Icon::Conversation,
                    active_icon: Some(Icon::Conversation),
                    tooltip_text: "Agent conversations".to_string(),
                    action: LeftPanelAction::ConversationListView,
                    render_with_active_state: false,
                    tooltip_keybinding: toolbelt_tooltip_keybinding(&tooltip_keybinding_names, ctx),
                    tooltip_keybinding_names,
                }
            }
        }
    }

    fn get_or_create_global_search_view_for_pane_group(
        &mut self,
        pane_group_id: warpui::EntityId,
        ctx: &mut ViewContext<Self>,
    ) -> ViewHandle<GlobalSearchView> {
        if let Some(view) = self
            .working_directories_model
            .as_ref(ctx)
            .get_global_search_view(pane_group_id)
        {
            return view;
        }

        let global_search_view = ctx.add_typed_action_view(GlobalSearchView::new);

        ctx.subscribe_to_view(&global_search_view, |me, _, event, ctx| {
            me.handle_global_search_event(event, ctx);
        });

        self.working_directories_model.update(ctx, |model, _ctx| {
            model.store_global_search_view(pane_group_id, global_search_view.clone());
        });

        global_search_view
    }

    fn get_or_create_file_tree_view_for_pane_group(
        &mut self,
        pane_group_id: warpui::EntityId,
        ctx: &mut ViewContext<Self>,
    ) -> ViewHandle<FileTreeView> {
        if let Some(view) = self
            .working_directories_model
            .as_ref(ctx)
            .get_file_tree_view(pane_group_id)
        {
            return view;
        }

        let file_tree_view = ctx.add_typed_action_view(FileTreeView::new);

        #[cfg(feature = "local_fs")]
        ctx.subscribe_to_view(&file_tree_view, |me, _, event, ctx| {
            me.handle_file_tree_event(event, ctx);
        });

        self.working_directories_model.update(ctx, |model, _ctx| {
            model.store_file_tree_view(pane_group_id, file_tree_view.clone());
        });

        file_tree_view
    }

    pub fn active_global_search_view(
        &self,
        app: &AppContext,
    ) -> Option<ViewHandle<GlobalSearchView>> {
        let pane_group_id = self
            .active_pane_group
            .as_ref()
            .and_then(|pane_group| pane_group.upgrade(app))
            .map(|pane_group| pane_group.id())?;
        self.working_directories_model
            .as_ref(app)
            .get_global_search_view(pane_group_id)
    }

    fn active_file_tree_view(&self, app: &AppContext) -> Option<ViewHandle<FileTreeView>> {
        let pane_group_id = self
            .active_pane_group
            .as_ref()
            .and_then(|pane_group| pane_group.upgrade(app))
            .map(|pane_group| pane_group.id())?;
        self.working_directories_model
            .as_ref(app)
            .get_file_tree_view(pane_group_id)
    }

    pub fn active_view(&self) -> ToolPanelView {
        self.active_view.get()
    }

    pub fn is_warp_drive_active(&self) -> bool {
        self.active_view.get() == ToolPanelView::WarpDrive
    }

    pub fn is_file_tree_active(&self) -> bool {
        self.active_view.get() == ToolPanelView::ProjectExplorer
    }

    pub fn warp_drive_view(&self) -> &ViewHandle<DrivePanel> {
        &self.warp_drive_view
    }

    pub(crate) fn auto_expand_active_file_tree_to_most_recent_directory(
        &mut self,
        ctx: &mut ViewContext<Self>,
    ) {
        if let Some(file_tree_view) = self.active_file_tree_view(ctx) {
            file_tree_view.update(ctx, |view, ctx| {
                view.auto_expand_to_most_recent_directory(ctx);
            });
        }
    }

    pub fn restore_active_view_from_snapshot(
        &mut self,
        view: ToolPanelView,
        ctx: &mut ViewContext<Self>,
    ) {
        active_view_state::set(self, view, ctx);
    }

    /// Updates the active pane group ID so we filter events correctly.
    pub fn set_active_pane_group(
        &mut self,
        pane_group: ViewHandle<PaneGroup>,
        working_directories_model: &ModelHandle<WorkingDirectoriesModel>,
        ctx: &mut ViewContext<Self>,
    ) {
        let pane_group_id = pane_group.id();

        let previous_pane_group_id = self
            .active_pane_group
            .as_ref()
            .and_then(|pane_group| pane_group.upgrade(ctx))
            .map(|pane_group| pane_group.id());

        self.active_pane_group = Some(pane_group.downgrade());

        if let Some(previous_pane_group_id) = previous_pane_group_id {
            if previous_pane_group_id != pane_group_id {
                self.deactivate_file_tree_view_for_pane_group(previous_pane_group_id, ctx);
            }
        }

        // Query the current state from the model
        let active_directories: Vec<WorkingDirectory> =
            working_directories_model.read(ctx, |model, _| {
                model
                    .most_recent_directories_for_pane_group(pane_group_id)
                    .map(|dirs| dirs.collect())
                    .unwrap_or_default()
            });
        let has_terminal_session = active_directories
            .iter()
            .any(|dir| dir.terminal_id.is_some());

        // Update GlobalSearchView root directories based on all working directories
        let roots: Vec<PathBuf> = active_directories.iter().map(|d| d.path.clone()).collect();
        let global_search_view =
            self.get_or_create_global_search_view_for_pane_group(pane_group_id, ctx);
        global_search_view.update(ctx, |view, view_ctx| {
            view.set_root_directories(roots, view_ctx);
        });

        let directories: Vec<PathBuf> = active_directories
            .iter()
            .map(|dir| dir.path.clone())
            .collect();
        let directories = deduplicate_by_directory_name(directories);
        let active_file_model = pane_group.as_ref(ctx).active_file_model().clone();

        let file_tree_view = self.get_or_create_file_tree_view_for_pane_group(pane_group_id, ctx);
        let left_panel_open = pane_group.as_ref(ctx).left_panel_open;
        let is_visible = left_panel_open && self.is_file_tree_active();
        file_tree_view.update(ctx, |view, ctx| {
            view.set_root_directories(directories, ctx);
            view.set_has_terminal_session(has_terminal_session, ctx);
            view.set_active_file_model(active_file_model, ctx);
            view.set_is_active(is_visible, ctx);

            if is_visible {
                view.auto_expand_to_most_recent_directory(ctx);
            }
        });

        self.on_left_panel_visibility_changed(left_panel_open, ctx);

        ctx.notify();
    }

    pub fn update_coding_panel_enablement(
        &mut self,
        enablement: CodingPanelEnablementState,
        ctx: &mut ViewContext<Self>,
    ) {
        #[cfg(feature = "local_fs")]
        {
            if let Some(file_tree_view) = self.active_file_tree_view(ctx) {
                file_tree_view.update(ctx, |view, ctx| {
                    view.set_enablement_state(enablement, ctx);
                });
            }
        }

        if let Some(global_search_view) = self.active_global_search_view(ctx) {
            global_search_view.update(ctx, |view, view_ctx| {
                view.set_enablement_state(enablement, view_ctx);
            });
        }
    }

    pub fn focus_active_view_on_entry(&mut self, ctx: &mut ViewContext<Self>) {
        match self.active_view.get() {
            ToolPanelView::ProjectExplorer => {
                if let Some(file_tree_view) = self.active_file_tree_view(ctx) {
                    file_tree_view.update(ctx, |view, ctx| {
                        view.on_left_panel_focused(ctx);
                    });
                    ctx.focus(&file_tree_view);
                }
            }
            ToolPanelView::GlobalSearch { entry_focus } => {
                if let Some(global_search_view) = self.active_global_search_view(ctx) {
                    global_search_view.update(ctx, |view, ctx| {
                        view.on_left_panel_focused(entry_focus, ctx);
                    });
                }

                active_view_state::set(
                    self,
                    ToolPanelView::GlobalSearch {
                        entry_focus: GlobalSearchEntryFocus::Results,
                    },
                    ctx,
                );
            }
            ToolPanelView::WarpDrive => {
                ctx.focus(&self.warp_drive_view);
                self.warp_drive_view.update(ctx, |view, ctx| {
                    view.reset_focused_index_in_warp_drive(true, ctx);
                });
            }
            ToolPanelView::ConversationListView => {
                self.conversation_list_view.update(ctx, |view, ctx| {
                    view.on_left_panel_focused(ctx);
                });
            }
        }
    }

    #[cfg(not(feature = "local_fs"))]
    fn handle_global_search_event(
        &mut self,
        _event: &GlobalSearchViewEvent,
        _ctx: &mut ViewContext<Self>,
    ) {
    }

    #[cfg(feature = "local_fs")]
    fn handle_global_search_event(
        &mut self,
        event: &GlobalSearchViewEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            GlobalSearchViewEvent::OpenMatch {
                path,
                line_number,
                column_num,
            } => {
                let line_col = LineAndColumnArg {
                    line_num: *line_number as usize,
                    column_num: *column_num,
                };

                let settings = EditorSettings::as_ref(ctx);
                let target = resolve_file_target_with_editor_choice(
                    path,
                    *settings.open_code_panels_file_editor,
                    *settings.prefer_markdown_viewer,
                    *settings.open_file_layout,
                    None,
                );

                send_telemetry_from_ctx!(
                    TelemetryEvent::CodePanelsFileOpened {
                        entrypoint: CodePanelsFileOpenEntrypoint::GlobalSearch,
                        target: target.clone(),
                    },
                    ctx
                );

                ctx.emit(LeftPanelEvent::OpenFileWithTarget {
                    location: LocalOrRemotePath::Local(path.clone()),
                    target,
                    line_col: Some(line_col),
                });
            }
        }
    }

    #[cfg(feature = "local_fs")]
    fn handle_file_tree_event(&mut self, event: &FileTreeEvent, ctx: &mut ViewContext<Self>) {
        match event {
            FileTreeEvent::FileRenamed { old_path, new_path } => {
                ctx.emit(LeftPanelEvent::FileTree(pane_group::Event::FileRenamed {
                    old_path: old_path.clone(),
                    new_path: new_path.clone(),
                }));
            }
            FileTreeEvent::FileDeleted { path } => {
                ctx.emit(LeftPanelEvent::FileTree(pane_group::Event::FileDeleted {
                    path: path.clone(),
                }));
            }
            FileTreeEvent::AttachAsContext { path } => {
                ctx.emit(LeftPanelEvent::FileTree(
                    pane_group::Event::AttachPathAsContext { path: path.clone() },
                ));
            }
            FileTreeEvent::OpenFile {
                path,
                target,
                line_col,
            } => {
                ctx.emit(LeftPanelEvent::OpenFileWithTarget {
                    location: path.clone(),
                    target: target.clone(),
                    line_col: *line_col,
                });
            }
            FileTreeEvent::CDToDirectory { path } => {
                ctx.emit(LeftPanelEvent::FileTree(pane_group::Event::CDToDirectory {
                    path: path.clone(),
                }));
            }
            FileTreeEvent::OpenDirectoryInNewTab { path } => {
                ctx.emit(LeftPanelEvent::FileTree(
                    pane_group::Event::OpenDirectoryInNewTab { path: path.clone() },
                ));
            }
        }
    }
}

impl Entity for LeftPanelView {
    type Event = LeftPanelEvent;
}

impl LeftPanelView {
    fn close_button(&self, appearance: &Appearance, app: &AppContext) -> Box<dyn Element> {
        let ui_builder = appearance.ui_builder().clone();
        let tooltip_keybinding =
            keybinding_name_to_display_string("workspace:toggle_left_panel", app);

        let tooltip = if let Some(keybinding) = tooltip_keybinding {
            ui_builder
                .tool_tip_with_sublabel("Close panel".to_string(), keybinding)
                .build()
                .finish()
        } else {
            ui_builder
                .tool_tip("Close panel".to_string())
                .build()
                .finish()
        };

        let icon_color = appearance
            .theme()
            .sub_text_color(appearance.theme().background());
        icon_button_with_color(
            appearance,
            icons::Icon::X,
            false,
            self.close_button_mouse_state.clone(),
            icon_color,
        )
        .with_tooltip(move || tooltip)
        .build()
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(WorkspaceAction::ToggleLeftPanel);
        })
        .with_cursor(Cursor::PointingHand)
        .finish()
    }

    fn update_button_active_states(&mut self) {
        for button in &mut self.toolbelt_buttons {
            button.render_with_active_state = match &button.action {
                LeftPanelAction::ProjectExplorer => {
                    self.active_view.get() == ToolPanelView::ProjectExplorer
                }
                LeftPanelAction::GlobalSearch { .. } => {
                    matches!(self.active_view.get(), ToolPanelView::GlobalSearch { .. })
                }
                LeftPanelAction::WarpDrive => self.active_view.get() == ToolPanelView::WarpDrive,
                LeftPanelAction::ConversationListView => {
                    self.active_view.get() == ToolPanelView::ConversationListView
                }
            };
        }
    }

    fn render_button(
        button_config: &ToolbeltButtonConfig,
        mouse_state: MouseStateHandle,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let action = button_config.action.clone();
        let ui_builder = appearance.ui_builder().clone();
        let tooltip_keybinding = button_config.tooltip_keybinding.clone();

        let icon_color = if button_config.render_with_active_state {
            appearance.theme().foreground().into_solid()
        } else {
            appearance
                .theme()
                .sub_text_color(appearance.theme().background())
                .into_solid()
        };

        let tooltip = if let Some(keybinding) = tooltip_keybinding {
            ui_builder
                .tool_tip_with_sublabel(button_config.tooltip_text.clone(), keybinding)
                .build()
                .finish()
        } else {
            ui_builder
                .tool_tip(button_config.tooltip_text.clone())
                .build()
                .finish()
        };

        let icon = if button_config.render_with_active_state {
            button_config.active_icon.unwrap_or(button_config.icon)
        } else {
            button_config.icon
        };

        icon_button(
            appearance,
            icon,
            button_config.render_with_active_state,
            mouse_state.clone(),
        )
        .with_tooltip(move || tooltip)
        .with_style(UiComponentStyles {
            font_color: Some(icon_color),
            height: Some(24.),
            width: Some(24.),
            padding: Some(Coords::uniform(4.)),
            ..Default::default()
        })
        .with_active_styles(UiComponentStyles {
            font_color: Some(icon_color),
            height: Some(24.),
            width: Some(24.),
            padding: Some(Coords::uniform(4.)),
            background: Some(internal_colors::fg_overlay_3(appearance.theme()).into()),
            ..Default::default()
        })
        .build()
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(action.clone());
        })
        .with_cursor(Cursor::PointingHand)
        .finish()
    }
}

impl LeftPanelView {
    pub fn handle_action_with_force_open(
        &mut self,
        action: &LeftPanelAction,
        force_open: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        match action {
            LeftPanelAction::ProjectExplorer => {
                active_view_state::set(self, ToolPanelView::ProjectExplorer, ctx);
                if force_open {
                    send_telemetry_from_ctx!(
                        TelemetryEvent::FileTreeToggled {
                            source: FileTreeSource::ForceOpened,
                            is_code_mode_v2: true,
                            cli_agent: None,
                        },
                        ctx
                    );
                } else {
                    send_telemetry_from_ctx!(
                        TelemetryEvent::FileTreeToggled {
                            source: FileTreeSource::LeftPanelToolbelt,
                            is_code_mode_v2: true,
                            cli_agent: None,
                        },
                        ctx
                    );
                }
            }
            LeftPanelAction::GlobalSearch { entry_focus } => {
                let was_active = self.active_view.get()
                    == ToolPanelView::GlobalSearch {
                        entry_focus: *entry_focus,
                    };
                active_view_state::set(
                    self,
                    ToolPanelView::GlobalSearch {
                        entry_focus: *entry_focus,
                    },
                    ctx,
                );
                if !was_active {
                    send_telemetry_from_ctx!(TelemetryEvent::GlobalSearchOpened, ctx);
                }
            }
            LeftPanelAction::WarpDrive => {
                active_view_state::set(self, ToolPanelView::WarpDrive, ctx);
                if force_open {
                    send_telemetry_from_ctx!(
                        TelemetryEvent::WarpDriveOpened {
                            source: WarpDriveSource::ForceOpened,
                            is_code_mode_v2: true
                        },
                        ctx
                    );
                } else {
                    send_telemetry_from_ctx!(
                        TelemetryEvent::WarpDriveOpened {
                            source: WarpDriveSource::LeftPanelToolbelt,
                            is_code_mode_v2: true
                        },
                        ctx
                    );
                }
            }
            LeftPanelAction::ConversationListView => {
                active_view_state::set(self, ToolPanelView::ConversationListView, ctx);
                send_telemetry_from_ctx!(TelemetryEvent::ConversationListViewOpened, ctx);
            }
        }
    }

    pub fn on_left_panel_visibility_changed(&self, is_now_open: bool, ctx: &mut ViewContext<Self>) {
        if ToolPanelView::ConversationListView == self.active_view.get() {
            self.on_conversation_list_view_visibility_changed(is_now_open, ctx);
        }

        self.update_active_file_tree_subscription_state(ctx);
    }

    fn deactivate_file_tree_view_for_pane_group(
        &self,
        pane_group_id: warpui::EntityId,
        ctx: &mut ViewContext<Self>,
    ) {
        if let Some(view) = self
            .working_directories_model
            .as_ref(ctx)
            .get_file_tree_view(pane_group_id)
        {
            view.update(ctx, |view, ctx| {
                view.set_is_active(false, ctx);
            });
        }
    }

    fn update_active_file_tree_subscription_state(&self, ctx: &mut ViewContext<Self>) {
        let Some(active_pane_group) = self
            .active_pane_group
            .as_ref()
            .and_then(|pane_group| pane_group.upgrade(ctx))
        else {
            return;
        };

        let is_visible = active_pane_group.as_ref(ctx).left_panel_open
            && self.active_view.get() == ToolPanelView::ProjectExplorer;

        if let Some(file_tree_view) = self
            .working_directories_model
            .as_ref(ctx)
            .get_file_tree_view(active_pane_group.id())
        {
            file_tree_view.update(ctx, |view, ctx| {
                view.set_is_active(is_visible, ctx);
            });
        }
    }

    /// When the conversation list view's visibility changes,
    /// we need to update the conversation and tasks model to reflect the new state
    /// (this information is used to decide whether or not we should poll for new tasks).
    fn on_conversation_list_view_visibility_changed(
        &self,
        is_now_open: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        let window_id = ctx.window_id();
        let view_id = self.conversation_list_view.id();
        AgentConversationsModel::handle(ctx).update(ctx, |model, ctx| {
            if is_now_open {
                model.register_view_open(window_id, view_id, ctx);
            } else {
                model.register_view_closed(window_id, view_id, ctx);
            }
        });
    }
}

impl TypedActionView for LeftPanelView {
    type Action = LeftPanelAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        self.handle_action_with_force_open(action, false, ctx);
    }
}

impl View for LeftPanelView {
    fn ui_name() -> &'static str {
        "LeftPanelView"
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        // Focus the active tool panel view on-left-panel-focus.
        if focus_ctx.is_self_focused() {
            match self.active_view.get() {
                ToolPanelView::ProjectExplorer => {
                    if let Some(view) = self.active_file_tree_view(ctx) {
                        ctx.focus(&view);
                    }
                }
                ToolPanelView::GlobalSearch { .. } => {
                    if let Some(view) = self.active_global_search_view(ctx) {
                        ctx.focus(&view);
                    }
                }
                ToolPanelView::WarpDrive => ctx.focus(&self.warp_drive_view),
                ToolPanelView::ConversationListView => ctx.focus(&self.conversation_list_view),
            }
        }
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);

        let mouse_state_handles = vec![
            self.mouse_state_handles.project_explorer_button.clone(),
            self.mouse_state_handles.global_search_button.clone(),
            self.mouse_state_handles.warp_drive_button.clone(),
            self.mouse_state_handles
                .conversation_list_view_button
                .clone(),
        ];

        // If there is only one button in the toolbelt row,
        // there is no need to show it as it's a bit redundant.
        let toolbelt_button_row = if self.toolbelt_buttons.len() > 1 {
            Some(
                Flex::row()
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_spacing(4.0)
                    .with_children(self.toolbelt_buttons.iter().zip(&mouse_state_handles).map(
                        |(button_config, mouse_state)| {
                            Self::render_button(button_config, mouse_state.clone(), appearance)
                        },
                    ))
                    .with_main_axis_size(MainAxisSize::Min)
                    .finish(),
            )
        } else {
            None
        };

        let content_area: Box<dyn Element> = match self.active_view.get() {
            ToolPanelView::ProjectExplorer => {
                if let Some(file_tree_view) = self.active_file_tree_view(app) {
                    Shrinkable::new(
                        1.0,
                        Container::new(ChildView::new(&file_tree_view).finish())
                            .with_padding_left(2.)
                            .with_padding_right(2.)
                            .finish(),
                    )
                    .finish()
                } else {
                    Shrinkable::new(1.0, Container::new(Empty::new().finish()).finish()).finish()
                }
            }
            ToolPanelView::GlobalSearch { .. } => {
                if let Some(global_search_view) = self.active_global_search_view(app) {
                    Shrinkable::new(
                        1.0,
                        Container::new(ChildView::new(&global_search_view).finish()).finish(),
                    )
                    .finish()
                } else {
                    Shrinkable::new(1.0, Container::new(Empty::new().finish()).finish()).finish()
                }
            }
            ToolPanelView::WarpDrive => Shrinkable::new(
                1.0,
                Container::new(ChildView::new(&self.warp_drive_view).finish())
                    .with_padding_left(2.)
                    .with_padding_right(2.)
                    .finish(),
            )
            .finish(),
            ToolPanelView::ConversationListView => {
                Shrinkable::new(1.0, ChildView::new(&self.conversation_list_view).finish()).finish()
            }
        };

        let panel_content = Container::new({
            let column = Flex::column();

            let header_left = if let Some(row) = toolbelt_button_row {
                row
            } else {
                Flex::row().finish()
            };

            let header_row = Container::new(
                ConstrainedBox::new(
                    Flex::row()
                        .with_main_axis_size(MainAxisSize::Max)
                        .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .with_child(Shrinkable::new(1.0, header_left).finish())
                        .with_child(self.close_button(appearance, app))
                        .finish(),
                )
                .with_height(PANE_HEADER_HEIGHT)
                .finish(),
            )
            .with_padding_left(10.)
            .with_padding_right(HEADER_EDGE_PADDING)
            .finish();

            column
                .with_child(header_row)
                .with_child(Shrinkable::new(1.0, content_area).finish())
                .with_main_axis_size(MainAxisSize::Max)
                .finish()
        })
        .finish();

        if warpui::platform::is_mobile_device() {
            return panel_content;
        }

        let drag_side = match self.panel_position {
            super::PanelPosition::Left => DragBarSide::Right,
            super::PanelPosition::Right => DragBarSide::Left,
        };
        Resizable::new(self.resizable_state_handle.clone(), panel_content)
            .with_dragbar_side(drag_side)
            .on_resize(move |ctx, _| {
                ctx.notify();
            })
            .with_bounds_callback(Box::new(|window_size| {
                let min_width = MIN_SIDEBAR_WIDTH;
                let max_width = window_size.x() * MAX_SIDEBAR_WIDTH_RATIO;
                (min_width, max_width.max(min_width))
            }))
            .finish()
    }
}

fn deduplicate_by_directory_name(directories: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen_paths: HashSet<PathBuf> = HashSet::new();
    directories
        .into_iter()
        .filter(|path| seen_paths.insert(path.clone()))
        .collect()
}
