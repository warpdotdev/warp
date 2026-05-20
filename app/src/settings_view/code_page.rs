#[cfg(feature = "local_fs")]
use super::features::external_editor::ExternalEditorView;
use super::{
    flags,
    settings_page::{
        build_sub_header, render_body_item, render_separator, Category, MatchData, PageType,
        SettingsPageMeta, SettingsPageViewHandle, SettingsWidget, HEADER_PADDING,
        TOGGLE_BUTTON_RIGHT_PADDING,
    },
    LocalOnlyIconState, SettingsAction, SettingsSection, ToggleSettingActionPair, ToggleState,
};
#[cfg(not(target_family = "wasm"))]
use crate::remote_server::codebase_index_model::{
    RemoteCodebaseIndexModel, RemoteCodebaseIndexModelEvent, RemoteCodebaseIndexSettingsEntry,
};
use crate::{
    ai::persisted_workspace::{
        EnablementState, LspRepoStatus, PersistedWorkspace, PersistedWorkspaceEvent,
    },
    appearance::Appearance,
    code::{
        buffer_location::LocalOrRemotePath,
        lsp_telemetry::{LspControlActionType, LspEnablementSource, LspTelemetryEvent},
    },
    send_telemetry_from_ctx,
    settings::{AISettings, CodeSettings},
    terminal::general_settings::GeneralSettings,
    ui_components::{
        avatar::{Avatar, AvatarContent, StatusElementTypes},
        buttons::icon_button,
        icons::Icon,
    },
    view_components::{
        action_button::{ActionButton, SecondaryTheme},
        DismissibleToast,
    },
    workspace::tab_settings::TabSettings,
    workspace::ToastStack,
    workspaces::{
        update_manager::TeamUpdateManager, user_workspaces::UserWorkspaces,
        workspace::AdminEnablementSetting,
    },
    TelemetryEvent,
};
use ai::index::full_source_code_embedding::manager::{
    CodebaseIndexFinishedStatus, CodebaseIndexManager, CodebaseIndexManagerEvent,
    CodebaseIndexStatus, CodebaseIndexingError,
};
use ai::index::full_source_code_embedding::SyncProgress;
use ai::project_context::model::{ProjectContextModel, ProjectContextModelEvent};
use ai::workspace::WorkspaceMetadata;
use lsp::supported_servers::LSPServerType;
use lsp::{LspManagerModel, LspManagerModelEvent, LspServerModel, LspState};
use pathfinder_color::ColorU;
#[cfg(not(target_family = "wasm"))]
use remote_server::codebase_index_proto::{RemoteCodebaseIndexState, RemoteCodebaseIndexStatus};
use std::borrow::Cow;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use warp_core::{
    features::FeatureFlag,
    report_if_error,
    settings::ToggleableSetting as _,
    ui::theme::{AnsiColorIdentifier, Fill as ThemeFill},
};
use warp_util::path::user_friendly_path;
#[cfg(not(target_family = "wasm"))]
use warp_util::remote_path::RemotePath;
use warpui::{
    elements::{
        ChildView, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Element, Empty,
        Expanded, Fill, Flex, MainAxisAlignment, MainAxisSize, MouseStateHandle, ParentElement,
        Radius, Shrinkable,
    },
    fonts::Weight,
    id,
    keymap::ContextPredicate,
    platform::{Cursor, FilePickerConfiguration},
    ui_components::{
        button::ButtonVariant,
        components::{Coords, UiComponent, UiComponentStyles},
        switch::{SwitchStateHandle, TooltipConfig},
    },
    Action, AppContext, Entity, ModelHandle, SingletonEntity, TypedActionView, View, ViewContext,
    ViewHandle,
};

const MAIN_SECTION_MARGIN: f32 = 12.;
const SUB_SECTION_MARGIN: f32 = 8.;

const STATUS_ICON_SIZE: f32 = 16.;
const LSP_STATUS_INDICATOR_SIZE: f32 = 8.;
const CODE_FEATURE_NAME: &str = "Code";
const INITIALIZATION_SETTINGS_HEADER: &str = "Initialization Settings";
const CODEBASE_INDEXING_LABEL: &str = "Codebase indexing";
const CODEBASE_INDEX_DESCRIPTION: &str = "Warp can automatically index code repositories as you navigate them, helping agents quickly understand context and provide solutions. Code is never stored on the server. If a codebase is unable to be indexed, Warp can still navigate your codebase and gain insights via grep and find tool calling.";
const WARP_INDEXING_IGNORE_DESCRIPTION: &str = "To exclude specific files or directories from indexing, add them to the .warpindexingignore file in your repository directory. These files will still be accessible to AI features, but they won't be included in codebase embeddings.";
const AUTO_INDEX_FEATURE_NAME: &str = "Index new folders by default";
const AUTO_INDEX_DESCRIPTION: &str = "When set to true, Warp will automatically index code repositories as you navigate them - helping agents quickly understand context and provide targeted solutions.";
const INDEXING_DISABLED_ADMIN_TEXT: &str = "Team admins have disabled codebase indexing.";
const INDEXING_WORKSPACE_ENABLED_ADMIN_TEXT: &str = "Team admins have enabled codebase indexing.";
const INDEXING_DISABLED_GLOBAL_AI_TEXT: &str =
    "AI Features must be enabled to use codebase indexing.";
const CODEBASE_INDEX_LIMIT_REACHED: &str = "You have reached the maximum number of codebase indices for your plan. Delete existing indices to auto-index new codebases.";

/// Identifies which subpage of the Code settings the user is viewing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CodeSubpage {
    /// Codebase indexing and initialization settings.
    Indexing,
    /// External editor, code review panel, and project explorer settings.
    EditorAndCodeReview,
}

impl CodeSubpage {
    pub fn from_section(section: SettingsSection) -> Option<Self> {
        match section {
            SettingsSection::CodeIndexing => Some(Self::Indexing),
            SettingsSection::EditorAndCodeReview => Some(Self::EditorAndCodeReview),
            _ => None,
        }
    }

    pub fn title(&self) -> &'static str {
        match self {
            Self::Indexing => "Codebase Indexing",
            Self::EditorAndCodeReview => "Editor and Code Review",
        }
    }
}

#[derive(Clone, Default)]
struct LspServerRowMouseStates {
    restart: MouseStateHandle,
    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    view_logs: MouseStateHandle,
    toggle: SwitchStateHandle,
    install: MouseStateHandle,
}

#[derive(Clone)]
struct InitializedFoldersMouseStates {
    codebase_manual_resync: Vec<MouseStateHandle>,
    codebase_delete: Vec<MouseStateHandle>,
    #[cfg(not(target_family = "wasm"))]
    remote_codebase_manual_resync: Vec<MouseStateHandle>,
    #[cfg(not(target_family = "wasm"))]
    remote_codebase_delete: Vec<MouseStateHandle>,
    lsp_rows: Vec<LspServerRowMouseStates>,
    open_project_rules: Vec<MouseStateHandle>,
}

#[derive(Clone)]
struct IndexingStatusPresentation {
    text: Cow<'static, str>,
    color: ColorU,
    icon: Option<Icon>,
    show_retry: bool,
    show_delete: bool,
}

pub struct CodeSettingsPageView {
    page: PageType<Self>,
    active_subpage: Option<CodeSubpage>,
    codebase_manual_resync_mouse_states: Vec<MouseStateHandle>,
    codebase_delete_mouse_states: Vec<MouseStateHandle>,
    #[cfg(not(target_family = "wasm"))]
    remote_codebase_manual_resync_mouse_states: Vec<MouseStateHandle>,
    #[cfg(not(target_family = "wasm"))]
    remote_codebase_delete_mouse_states: Vec<MouseStateHandle>,
    /// Mouse states for LSP server row buttons.
    /// This is kept separate from the codebase mouse states because each workspace/folder
    /// can have 0 to multiple LSP servers, so the count doesn't match 1:1 with workspaces.
    /// The states are flattened into a single Vec, indexed by iterating through workspaces
    /// and their enabled servers in order.
    lsp_row_mouse_states: Vec<LspServerRowMouseStates>,
    open_project_rules_mouse_states: Vec<MouseStateHandle>,
    /// Tracks installation status for suggested LSP servers so the UI can decide
    /// whether to show "Available for download" vs "Installed" and whether the
    /// "+" button should trigger install or just enable.
    suggested_server_statuses: HashMap<(PathBuf, LSPServerType), LspRepoStatus>,
    #[cfg(feature = "local_fs")]
    external_editor_view: Option<ViewHandle<ExternalEditorView>>,
}

impl CodeSettingsPageView {
    pub fn new(ctx: &mut ViewContext<CodeSettingsPageView>) -> Self {
        let index_manager = CodebaseIndexManager::handle(ctx);
        let codebase_count = index_manager
            .as_ref(ctx)
            .get_codebase_index_statuses(ctx)
            .count();

        ctx.subscribe_to_model(&index_manager, |me, index, event, ctx| {
            if let CodebaseIndexManagerEvent::SyncStateUpdated = event {
                let codebase_count = index.as_ref(ctx).get_codebase_index_statuses(ctx).count();

                // Only update mouse states if the number of codebases changed
                if me.codebase_manual_resync_mouse_states.len() != codebase_count {
                    // Resize the vector to match the new codebase count, but preserve the existing mouse states
                    me.codebase_manual_resync_mouse_states
                        .resize_with(codebase_count, Default::default);
                    me.codebase_delete_mouse_states
                        .resize_with(codebase_count, Default::default);
                }

                me.resize_workspace_mouse_states(ctx);

                ctx.notify();
            }
        });

        #[cfg(not(target_family = "wasm"))]
        let remote_codebase_count = {
            let remote_index_model = RemoteCodebaseIndexModel::handle(ctx);
            let remote_codebase_count = remote_index_model.as_ref(ctx).entries_for_settings().len();
            ctx.subscribe_to_model(&remote_index_model, |me, model, event, ctx| match event {
                RemoteCodebaseIndexModelEvent::SettingsEntriesChanged => {
                    let remote_codebase_count = model.as_ref(ctx).entries_for_settings().len();
                    if me.remote_codebase_manual_resync_mouse_states.len() != remote_codebase_count
                    {
                        me.remote_codebase_manual_resync_mouse_states
                            .resize_with(remote_codebase_count, Default::default);
                        me.remote_codebase_delete_mouse_states
                            .resize_with(remote_codebase_count, Default::default);
                    }
                    ctx.notify();
                }
            });
            remote_codebase_count
        };

        // Calculate total LSP server count across all workspaces (enabled + disabled + suggested)
        let lsp_server_count = PersistedWorkspace::as_ref(ctx).total_lsp_server_count(true);

        // Subscribe to LSP manager events for real-time status updates
        ctx.subscribe_to_model(
            &LspManagerModel::handle(ctx),
            |me, _, event, ctx| match event {
                LspManagerModelEvent::ServerStarted(_)
                | LspManagerModelEvent::ServerStopped(_)
                | LspManagerModelEvent::ServerRemoved { .. } => {
                    // Recalculate LSP server count and resize mouse states if needed
                    let new_count = PersistedWorkspace::as_ref(ctx).total_lsp_server_count(true);
                    if me.lsp_row_mouse_states.len() != new_count {
                        me.lsp_row_mouse_states
                            .resize_with(new_count, Default::default);
                    }

                    me.resize_workspace_mouse_states(ctx);

                    ctx.notify();
                }
            },
        );

        // Subscribe to PersistedWorkspaceEvent to handle suggested server detection
        // and installation status updates. We don't scan for suggested servers
        // here — PersistedWorkspace::new() already kicks off detection at startup
        // and emits AvailableServersDetected for each workspace, which the
        // subscription below handles.
        let persisted = PersistedWorkspace::handle(ctx);

        ctx.subscribe_to_model(&persisted, move |me, _model, event, ctx| match event {
            PersistedWorkspaceEvent::AvailableServersDetected {
                workspace_path,
                servers,
            } => {
                // New suggested servers detected — kick off install detection
                // and resize mouse states.
                for &server_type in servers {
                    #[cfg(feature = "local_fs")]
                    let status = PersistedWorkspace::handle(ctx).update(ctx, |model, ctx| {
                        model.detect_lsp_workspace_status(workspace_path.clone(), server_type, ctx)
                    });
                    #[cfg(not(feature = "local_fs"))]
                    let status = LspRepoStatus::CheckingForInstallation;
                    me.suggested_server_statuses
                        .insert((workspace_path.clone(), server_type), status);
                }
                let new_count = PersistedWorkspace::as_ref(ctx).total_lsp_server_count(true);
                if me.lsp_row_mouse_states.len() != new_count {
                    me.lsp_row_mouse_states
                        .resize_with(new_count, Default::default);
                }
                me.resize_workspace_mouse_states(ctx);
                ctx.notify();
            }
            PersistedWorkspaceEvent::InstallStatusUpdate {
                server_type,
                status,
            } => {
                let new_status = LspRepoStatus::from_installation_status(status, *server_type);
                for ((_, st), repo_status) in &mut me.suggested_server_statuses {
                    if *st == *server_type {
                        *repo_status = new_status.clone();
                    }
                }
                ctx.notify();
            }
            PersistedWorkspaceEvent::InstallationSucceeded
            | PersistedWorkspaceEvent::InstallationFailed
            | PersistedWorkspaceEvent::WorkspaceAdded { .. } => {
                ctx.notify();
            }
        });

        // Re-render when project rules are added or removed so the
        // "Open project rules" button visibility stays up to date.
        ctx.subscribe_to_model(&ProjectContextModel::handle(ctx), |_me, _, event, ctx| {
            if matches!(event, ProjectContextModelEvent::KnownRulesChanged(_)) {
                ctx.notify();
            }
        });

        let manual_add_directory_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Index new folder", SecondaryTheme)
                .with_icon(Icon::FindAll)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(CodeSettingsPageAction::ManualAddDirectory);
                })
        });

        let code_page_widget = CodePageWidget {
            switch_state: Default::default(),
            auto_index_switch_state: Default::default(),
            manual_add_directory_button,
        };

        let workspace_count = PersistedWorkspace::as_ref(ctx).workspaces().count();

        #[cfg(feature = "local_fs")]
        let external_editor_view;
        let page = if FeatureFlag::OpenWarpNewSettingsModes.is_enabled() {
            #[cfg(feature = "local_fs")]
            {
                external_editor_view = Some(ctx.add_typed_action_view(ExternalEditorView::new));
            }

            let codebase_indexing_widgets: Vec<Box<dyn SettingsWidget<View = Self>>> =
                vec![Box::new(CodebaseIndexingCategorizedWidget {
                    inner: code_page_widget,
                })];
            #[cfg(feature = "local_fs")]
            let mut code_editor_review_widgets: Vec<
                Box<dyn SettingsWidget<View = Self>>,
            > = vec![Box::new(ExternalEditorCodeWidget)];
            #[cfg(not(feature = "local_fs"))]
            let mut code_editor_review_widgets: Vec<
                Box<dyn SettingsWidget<View = Self>>,
            > = vec![];
            code_editor_review_widgets.extend([
                Box::new(AutoOpenCodeReviewPaneCodeWidget::default())
                    as Box<dyn SettingsWidget<View = Self>>,
                Box::new(CodeReviewPanelToggleWidget::default()),
                Box::new(CodeReviewDiffStatsToggleWidget::default()),
                Box::new(ProjectExplorerToggleWidget::default()),
                Box::new(GlobalSearchToggleWidget::default()),
            ]);
            let categories = vec![
                Category::new("Codebase Indexing", codebase_indexing_widgets),
                Category::new("Code Editor and Review", code_editor_review_widgets),
            ];
            PageType::new_categorized(categories, None)
        } else {
            #[cfg(feature = "local_fs")]
            {
                external_editor_view = None;
            }
            let widgets: Vec<Box<dyn SettingsWidget<View = Self>>> =
                vec![Box::new(code_page_widget)];
            PageType::new_uncategorized(widgets, None)
        };

        Self {
            page,
            active_subpage: None,
            codebase_manual_resync_mouse_states: (0..codebase_count)
                .map(|_| Default::default())
                .collect(),
            codebase_delete_mouse_states: (0..codebase_count).map(|_| Default::default()).collect(),
            #[cfg(not(target_family = "wasm"))]
            remote_codebase_manual_resync_mouse_states: (0..remote_codebase_count)
                .map(|_| Default::default())
                .collect(),
            #[cfg(not(target_family = "wasm"))]
            remote_codebase_delete_mouse_states: (0..remote_codebase_count)
                .map(|_| Default::default())
                .collect(),
            lsp_row_mouse_states: (0..lsp_server_count).map(|_| Default::default()).collect(),
            open_project_rules_mouse_states: (0..workspace_count)
                .map(|_| Default::default())
                .collect(),
            suggested_server_statuses: HashMap::new(),
            #[cfg(feature = "local_fs")]
            external_editor_view,
        }
    }

    /// Set the active subpage and rebuild the page to show only the relevant widgets.
    pub fn set_active_subpage(
        &mut self,
        subpage: Option<CodeSubpage>,
        ctx: &mut ViewContext<Self>,
    ) {
        if self.active_subpage != subpage {
            self.active_subpage = subpage;
            // Rebuild the page with the relevant widgets for the selected subpage,
            // or the full categorized page when subpage is None.
            if let Some(subpage) = subpage {
                let manual_add_directory_button = ctx.add_typed_action_view(|_| {
                    ActionButton::new("Index new folder", SecondaryTheme)
                        .with_icon(Icon::FindAll)
                        .on_click(|ctx| {
                            ctx.dispatch_typed_action(CodeSettingsPageAction::ManualAddDirectory);
                        })
                });
                let mut widgets: Vec<Box<dyn SettingsWidget<View = Self>>> =
                    vec![Box::new(CodeSubpageHeaderWidget {
                        title: subpage.title(),
                    })];
                match subpage {
                    CodeSubpage::Indexing => {
                        widgets.push(Box::new(CodebaseIndexingCategorizedWidget {
                            inner: CodePageWidget {
                                switch_state: Default::default(),
                                auto_index_switch_state: Default::default(),
                                manual_add_directory_button,
                            },
                        }));
                    }
                    CodeSubpage::EditorAndCodeReview => {
                        #[cfg(feature = "local_fs")]
                        widgets.push(Box::new(ExternalEditorCodeWidget));
                        widgets.extend([
                            Box::new(AutoOpenCodeReviewPaneCodeWidget::default())
                                as Box<dyn SettingsWidget<View = Self>>,
                            Box::new(CodeReviewPanelToggleWidget::default()),
                            Box::new(CodeReviewDiffStatsToggleWidget::default()),
                            Box::new(ProjectExplorerToggleWidget::default()),
                            Box::new(GlobalSearchToggleWidget::default()),
                        ]);
                    }
                }
                // Subpage widgets render their own subheader-sized titles,
                // so we don't pass a page-level title.
                self.page = PageType::new_uncategorized(widgets, None);
            } else {
                // None: rebuild the full categorized page (all widgets).
                self.page = Self::build_full_page(ctx);
            }
            ctx.notify();
        }
    }

    /// Builds the full categorized page with all Code widgets.
    /// Used for the default/legacy view and when resetting to all-widgets mode for search.
    fn build_full_page(ctx: &mut ViewContext<Self>) -> PageType<Self> {
        if FeatureFlag::OpenWarpNewSettingsModes.is_enabled() {
            let manual_add_directory_button = ctx.add_typed_action_view(|_| {
                ActionButton::new("Index new folder", SecondaryTheme)
                    .with_icon(Icon::FindAll)
                    .on_click(|ctx| {
                        ctx.dispatch_typed_action(CodeSettingsPageAction::ManualAddDirectory);
                    })
            });
            let code_page_widget = CodePageWidget {
                switch_state: Default::default(),
                auto_index_switch_state: Default::default(),
                manual_add_directory_button,
            };
            let codebase_indexing_widgets: Vec<Box<dyn SettingsWidget<View = Self>>> =
                vec![Box::new(CodebaseIndexingCategorizedWidget {
                    inner: code_page_widget,
                })];
            #[cfg(feature = "local_fs")]
            let mut code_editor_review_widgets: Vec<
                Box<dyn SettingsWidget<View = Self>>,
            > = vec![Box::new(ExternalEditorCodeWidget)];
            #[cfg(not(feature = "local_fs"))]
            let mut code_editor_review_widgets: Vec<
                Box<dyn SettingsWidget<View = Self>>,
            > = vec![];
            code_editor_review_widgets.extend([
                Box::new(AutoOpenCodeReviewPaneCodeWidget::default())
                    as Box<dyn SettingsWidget<View = Self>>,
                Box::new(CodeReviewPanelToggleWidget::default()),
                Box::new(CodeReviewDiffStatsToggleWidget::default()),
                Box::new(ProjectExplorerToggleWidget::default()),
                Box::new(GlobalSearchToggleWidget::default()),
            ]);
            let categories = vec![
                Category::new("Codebase Indexing", codebase_indexing_widgets),
                Category::new("Code Editor and Review", code_editor_review_widgets),
            ];
            PageType::new_categorized(categories, None)
        } else {
            let manual_add_directory_button = ctx.add_typed_action_view(|_| {
                ActionButton::new("Index new folder", SecondaryTheme)
                    .with_icon(Icon::FindAll)
                    .on_click(|ctx| {
                        ctx.dispatch_typed_action(CodeSettingsPageAction::ManualAddDirectory);
                    })
            });
            let widgets: Vec<Box<dyn SettingsWidget<View = Self>>> =
                vec![Box::new(CodePageWidget {
                    switch_state: Default::default(),
                    auto_index_switch_state: Default::default(),
                    manual_add_directory_button,
                })];
            PageType::new_uncategorized(widgets, None)
        }
    }

    /// Resize `open_project_rules_mouse_states` to match the current workspace count.
    fn resize_workspace_mouse_states(&mut self, ctx: &AppContext) {
        let workspace_count = PersistedWorkspace::as_ref(ctx).workspaces().count();
        if self.open_project_rules_mouse_states.len() != workspace_count {
            self.open_project_rules_mouse_states
                .resize_with(workspace_count, Default::default);
        }
    }

    fn open_directory_picker(&mut self, ctx: &mut ViewContext<Self>) {
        let file_picker_config = FilePickerConfiguration::new().folders_only();
        let window_id = ctx.window_id();

        ctx.open_file_picker(
            move |result, ctx| match result {
                Ok(paths) => {
                    if let Some(directory_path) = paths.first() {
                        let path = PathBuf::from(directory_path);

                        CodebaseIndexManager::handle(ctx).update(ctx, |manager, ctx| {
                            manager.index_directory(path, ctx);
                        });
                    }
                }
                Err(err) => {
                    ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                        toast_stack.add_ephemeral_toast(
                            DismissibleToast::error(format!("{err}")),
                            window_id,
                            ctx,
                        );
                    });
                }
            },
            file_picker_config,
        );
    }
}

impl Entity for CodeSettingsPageView {
    type Event = CodeSettingsPageEvent;
}

impl View for CodeSettingsPageView {
    fn ui_name() -> &'static str {
        "CodePage"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        self.page.render(self, app)
    }
}

#[derive(Debug, Clone)]
pub enum CodeSettingsPageEvent {
    SignupAnonymousUser,
    OpenLspLogs { log_path: PathBuf },
    OpenProjectRules { rule_paths: Vec<PathBuf> },
}

// Define the code page actions.
#[derive(Debug, Clone)]
pub enum CodeSettingsPageAction {
    ToggleCodebaseContext,
    ToggleAutoIndexing,
    ManualResync(PathBuf),
    DeleteIndex(PathBuf),
    #[cfg(not(target_family = "wasm"))]
    ManualResyncRemote(RemotePath),
    #[cfg(not(target_family = "wasm"))]
    DeleteRemoteIndex(RemotePath),
    ManualAddDirectory,
    SignupAnonymousUser,
    /// Toggle an LSP server on/off for a workspace.
    ToggleLspServer {
        workspace_path: PathBuf,
        server_type: LSPServerType,
        currently_enabled: bool,
    },
    RestartLspServer {
        server: ModelHandle<LspServerModel>,
    },
    OpenLspLogs {
        log_path: PathBuf,
    },
    OpenProjectRules {
        rule_paths: Vec<PathBuf>,
    },
    ToggleCodeReviewPanel,
    ToggleShowCodeReviewDiffStats,
    ToggleAutoOpenCodeReviewPane,
    ToggleProjectExplorer,
    ToggleGlobalSearch,
    /// Install (if needed) and enable a suggested LSP server.
    InstallAndEnableLspServer {
        workspace_path: PathBuf,
        server_type: LSPServerType,
    },
    /// Enable a suggested LSP server that is already installed.
    EnableSuggestedLspServer {
        workspace_path: PathBuf,
        server_type: LSPServerType,
    },
}

impl TypedActionView for CodeSettingsPageView {
    type Action = CodeSettingsPageAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            CodeSettingsPageAction::ToggleCodebaseContext => {
                // If the organization has an explicit setting (on or off), ignore user toggles.
                let setting = UserWorkspaces::as_ref(ctx).team_allows_codebase_context();
                match setting {
                    AdminEnablementSetting::Enable | AdminEnablementSetting::Disable => {
                        return;
                    }
                    AdminEnablementSetting::RespectUserSetting => {
                        // Allow user to toggle
                    }
                }

                CodeSettings::handle(ctx).update(ctx, |settings, ctx| {
                    match settings.codebase_context_enabled.toggle_and_save_value(ctx) {
                        Ok(new_value) => {
                            send_telemetry_from_ctx!(
                                TelemetryEvent::ToggleCodebaseContext {
                                    is_codebase_context_enabled: new_value
                                },
                                ctx
                            );
                        }
                        Err(e) => {
                            log::warn!("Failed to set value for Codebase Context: {e:?}");
                        }
                    }
                });

                ctx.notify();
            }
            CodeSettingsPageAction::ToggleAutoIndexing => {
                CodeSettings::handle(ctx).update(ctx, |settings, ctx| {
                    match settings.auto_indexing_enabled.toggle_and_save_value(ctx) {
                        Ok(new_value) => {
                            send_telemetry_from_ctx!(
                                TelemetryEvent::ToggleAutoIndexing {
                                    is_autoindexing_enabled: new_value
                                },
                                ctx
                            );
                        }
                        Err(e) => {
                            log::warn!("Failed to set value for auto indexing: {e:?}");
                        }
                    }
                });

                ctx.notify();
            }
            CodeSettingsPageAction::ManualResync(repo_path) => {
                CodebaseIndexManager::handle(ctx).update(ctx, |manager, ctx| {
                    manager.try_manual_resync_codebase(repo_path, ctx);
                });
            }
            CodeSettingsPageAction::DeleteIndex(repo_path) => {
                CodebaseIndexManager::handle(ctx).update(ctx, |manager, ctx| {
                    manager.drop_index(repo_path.clone(), ctx);
                });
            }
            #[cfg(not(target_family = "wasm"))]
            CodeSettingsPageAction::ManualResyncRemote(remote_path) => {
                RemoteCodebaseIndexModel::handle(ctx).update(ctx, |model, ctx| {
                    model.request_index(remote_path.clone(), ctx);
                });
            }
            #[cfg(not(target_family = "wasm"))]
            CodeSettingsPageAction::DeleteRemoteIndex(remote_path) => {
                RemoteCodebaseIndexModel::handle(ctx).update(ctx, |model, ctx| {
                    model.drop_index(remote_path.clone(), ctx);
                });
            }
            CodeSettingsPageAction::ManualAddDirectory => {
                self.open_directory_picker(ctx);
            }
            CodeSettingsPageAction::SignupAnonymousUser => {
                ctx.emit(CodeSettingsPageEvent::SignupAnonymousUser);
            }
            CodeSettingsPageAction::ToggleLspServer {
                workspace_path,
                server_type,
                currently_enabled,
            } => {
                if *currently_enabled {
                    // Toggling OFF: stop and disable
                    send_telemetry_from_ctx!(
                        LspTelemetryEvent::ServerRemoved {
                            server_type: server_type.binary_name().to_string(),
                            source: LspEnablementSource::Settings,
                        },
                        ctx
                    );
                    LspManagerModel::handle(ctx).update(ctx, |manager, ctx| {
                        manager.remove_server(workspace_path, *server_type, ctx);
                    });
                    PersistedWorkspace::handle(ctx).update(ctx, |workspace, _| {
                        workspace.disable_lsp_server_for_path(workspace_path, *server_type);
                    });
                } else {
                    // Toggling ON: enable and spawn
                    send_telemetry_from_ctx!(
                        LspTelemetryEvent::ServerEnabled {
                            server_type: server_type.binary_name().to_string(),
                            source: LspEnablementSource::Settings,
                            needed_install: false,
                        },
                        ctx
                    );
                    let workspace_path = workspace_path.clone();
                    PersistedWorkspace::handle(ctx).update(ctx, |workspace, _ctx| {
                        workspace.enable_lsp_server_for_path(&workspace_path, *server_type);
                        #[cfg(feature = "local_fs")]
                        workspace.execute_lsp_task(
                            crate::ai::persisted_workspace::LspTask::Spawn {
                                file_path: workspace_path,
                            },
                            _ctx,
                        );
                    });
                }
                ctx.notify();
            }
            CodeSettingsPageAction::RestartLspServer { server } => {
                let server_name = server.as_ref(ctx).server_name();
                send_telemetry_from_ctx!(
                    LspTelemetryEvent::ControlAction {
                        action: LspControlActionType::Restart,
                        server_type: Some(server_name),
                    },
                    ctx
                );
                server.update(ctx, |server, ctx| {
                    server.restart(ctx);
                });
            }
            CodeSettingsPageAction::OpenLspLogs { log_path } => {
                send_telemetry_from_ctx!(
                    LspTelemetryEvent::ControlAction {
                        action: LspControlActionType::OpenLogs,
                        server_type: None,
                    },
                    ctx
                );
                ctx.emit(CodeSettingsPageEvent::OpenLspLogs {
                    log_path: log_path.clone(),
                });
            }
            CodeSettingsPageAction::OpenProjectRules { rule_paths } => {
                ctx.emit(CodeSettingsPageEvent::OpenProjectRules {
                    rule_paths: rule_paths.clone(),
                });
            }
            CodeSettingsPageAction::ToggleCodeReviewPanel => {
                TabSettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(settings.show_code_review_button.toggle_and_save_value(ctx));
                });
                ctx.notify();
            }
            CodeSettingsPageAction::ToggleShowCodeReviewDiffStats => {
                TabSettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(settings
                        .show_code_review_diff_stats
                        .toggle_and_save_value(ctx));
                });
                ctx.notify();
            }
            CodeSettingsPageAction::ToggleProjectExplorer => {
                CodeSettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(settings.show_project_explorer.toggle_and_save_value(ctx));
                });
                ctx.notify();
            }
            CodeSettingsPageAction::ToggleGlobalSearch => {
                CodeSettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(settings.show_global_search.toggle_and_save_value(ctx));
                });
                ctx.notify();
            }
            CodeSettingsPageAction::ToggleAutoOpenCodeReviewPane => {
                GeneralSettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(settings
                        .auto_open_code_review_pane_on_first_agent_change
                        .toggle_and_save_value(ctx));
                });
                send_telemetry_from_ctx!(
                    TelemetryEvent::FeaturesPageAction {
                        action: "ToggleAutoOpenCodeReviewPane".to_string(),
                        value: format!(
                            "{}",
                            *GeneralSettings::as_ref(ctx)
                                .auto_open_code_review_pane_on_first_agent_change
                        )
                    },
                    ctx
                );
                ctx.notify();
            }
            CodeSettingsPageAction::InstallAndEnableLspServer {
                workspace_path,
                server_type,
            } => {
                send_telemetry_from_ctx!(
                    LspTelemetryEvent::ServerEnabled {
                        server_type: server_type.binary_name().to_string(),
                        source: LspEnablementSource::Settings,
                        needed_install: true,
                    },
                    ctx
                );
                #[cfg(feature = "local_fs")]
                {
                    let workspace_path = workspace_path.clone();
                    let server_type = *server_type;
                    PersistedWorkspace::handle(ctx).update(ctx, |workspace, _ctx| {
                        workspace.execute_lsp_task(
                            crate::ai::persisted_workspace::LspTask::Install {
                                file_path: workspace_path.clone(),
                                repo_root: workspace_path,
                                server_type,
                            },
                            _ctx,
                        );
                    });
                }
                #[cfg(not(feature = "local_fs"))]
                let _ = workspace_path;
                ctx.notify();
            }
            CodeSettingsPageAction::EnableSuggestedLspServer {
                workspace_path,
                server_type,
            } => {
                send_telemetry_from_ctx!(
                    LspTelemetryEvent::ServerEnabled {
                        server_type: server_type.binary_name().to_string(),
                        source: LspEnablementSource::Settings,
                        needed_install: false,
                    },
                    ctx
                );
                let workspace_path = workspace_path.clone();
                let server_type = *server_type;
                PersistedWorkspace::handle(ctx).update(ctx, |workspace, _ctx| {
                    workspace.enable_lsp_server_for_path(&workspace_path, server_type);
                    #[cfg(feature = "local_fs")]
                    workspace.execute_lsp_task(
                        crate::ai::persisted_workspace::LspTask::Spawn {
                            file_path: workspace_path,
                        },
                        _ctx,
                    );
                });
                ctx.notify();
            }
        }
    }
}

pub fn init_actions_from_parent_view<T: Action + Clone>(
    app: &mut AppContext,
    context: &ContextPredicate,
    builder: fn(SettingsAction) -> T,
) {
    if FeatureFlag::FullSourceCodeEmbedding.is_enabled() {
        ToggleSettingActionPair::add_toggle_setting_action_pairs_as_bindings(
            vec![ToggleSettingActionPair::new(
                "codebase index",
                builder(SettingsAction::Code(
                    CodeSettingsPageAction::ToggleCodebaseContext,
                )),
                &(context.clone() & id!(flags::IS_ANY_AI_ENABLED)),
                flags::IS_CODEBASE_INDEXING_ENABLED,
            )],
            app,
        );

        ToggleSettingActionPair::add_toggle_setting_action_pairs_as_bindings(
            vec![ToggleSettingActionPair::new(
                "auto-indexing",
                builder(SettingsAction::Code(
                    CodeSettingsPageAction::ToggleAutoIndexing,
                )),
                &(context.clone() & id!(flags::IS_CODEBASE_INDEXING_ENABLED)),
                flags::IS_AUTOINDEXING_ENABLED,
            )],
            app,
        );
    }
}

struct CodePageWidget {
    switch_state: SwitchStateHandle,
    auto_index_switch_state: SwitchStateHandle,
    manual_add_directory_button: ViewHandle<ActionButton>,
}

impl SettingsWidget for CodePageWidget {
    type View = CodeSettingsPageView;

    fn search_terms(&self) -> &str {
        "code coding codebase repository index indexing indices context path lsp language server"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let mut content = Flex::column();

        let global_ai_enabled = AISettings::as_ref(app).is_any_ai_enabled(app);

        // Main "Code" header
        content.add_child(self.render_code_header(appearance));

        // Initialization Settings section
        content.add_child(render_separator(appearance));
        content.add_child(self.render_initialization_settings_header(appearance));
        content.add_child(self.render_codebase_indexing_toggle_row(
            global_ai_enabled,
            appearance,
            app,
        ));
        content.add_child(self.render_settings_subtext(
            global_ai_enabled,
            CODEBASE_INDEX_DESCRIPTION,
            appearance,
        ));
        content.add_child(self.render_settings_subtext(
            global_ai_enabled,
            WARP_INDEXING_IGNORE_DESCRIPTION,
            appearance,
        ));

        let codebase_context_enabled = UserWorkspaces::as_ref(app).is_codebase_context_enabled(app);
        if global_ai_enabled && codebase_context_enabled {
            content.add_children(self.render_autoindexing_rows(appearance, app));
        }

        // Initialized / indexed folders section
        content.add_child(render_separator(appearance));
        let mouse_states = InitializedFoldersMouseStates {
            codebase_manual_resync: view.codebase_manual_resync_mouse_states.clone(),
            codebase_delete: view.codebase_delete_mouse_states.clone(),
            #[cfg(not(target_family = "wasm"))]
            remote_codebase_manual_resync: view.remote_codebase_manual_resync_mouse_states.clone(),
            #[cfg(not(target_family = "wasm"))]
            remote_codebase_delete: view.remote_codebase_delete_mouse_states.clone(),
            lsp_rows: view.lsp_row_mouse_states.clone(),
            open_project_rules: view.open_project_rules_mouse_states.clone(),
        };

        content.add_child(self.render_initialized_folders(
            mouse_states,
            &view.suggested_server_statuses,
            appearance,
            app,
        ));

        Container::new(content.finish())
            .with_uniform_padding(24.0)
            .finish()
    }
}

impl CodePageWidget {
    fn render_autoindexing_rows(
        &self,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Vec<Box<dyn Element>> {
        let auto_indexing_enabled = *CodeSettings::as_ref(app).auto_indexing_enabled;
        let codebase_indexing_enabled =
            UserWorkspaces::as_ref(app).is_codebase_context_enabled(app);

        let mut rows = vec![
            self.render_autoindex_row(auto_indexing_enabled, appearance),
            // Use subtext styling for description (gray color per Figma)
            self.render_settings_subtext(
                codebase_indexing_enabled,
                AUTO_INDEX_DESCRIPTION,
                appearance,
            ),
        ];

        if codebase_indexing_enabled && !CodebaseIndexManager::as_ref(app).can_create_new_indices()
        {
            rows.push(self.render_settings_subtext(
                false,
                CODEBASE_INDEX_LIMIT_REACHED,
                appearance,
            ));
        }

        rows.push(
            Container::new(Empty::new().finish())
                .with_margin_bottom(16.0)
                .finish(),
        );
        rows
    }

    fn render_autoindex_row(
        &self,
        auto_indexing_enabled: bool,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let ui_builder = appearance.ui_builder();
        let theme = appearance.theme();

        Container::new(
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                .with_child(
                    ui_builder
                        .span(AUTO_INDEX_FEATURE_NAME)
                        .with_style(UiComponentStyles {
                            font_size: Some(16.0),
                            font_weight: Some(Weight::Semibold),
                            font_color: Some(theme.active_ui_text_color().into()),
                            ..Default::default()
                        })
                        .build()
                        .finish(),
                )
                .with_child(
                    Container::new(
                        ui_builder
                            .switch(self.auto_index_switch_state.clone())
                            .check(auto_indexing_enabled)
                            .build()
                            .on_click(move |ctx, _, _| {
                                ctx.dispatch_typed_action(
                                    CodeSettingsPageAction::ToggleAutoIndexing,
                                );
                            })
                            .finish(),
                    )
                    .with_padding_right(TOGGLE_BUTTON_RIGHT_PADDING)
                    .finish(),
                )
                .finish(),
        )
        .with_padding_bottom(6.)
        .finish()
    }

    /// Renders a settings subtext description (gray color per Figma).
    fn render_settings_subtext(
        &self,
        _active: bool,
        description: &'static str,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let ui_builder = appearance.ui_builder();
        let theme = appearance.theme();

        // Per Figma: subtext uses disabled_ui_text_color (#9b9b9b)
        ui_builder
            .paragraph(description)
            .with_style(UiComponentStyles {
                font_color: Some(theme.disabled_ui_text_color().into()),
                ..Default::default()
            })
            .build()
            .with_margin_bottom(8.0)
            .finish()
    }

    /// Renders the main "Code" header.
    fn render_code_header(&self, appearance: &Appearance) -> Box<dyn Element> {
        let ui_builder = appearance.ui_builder();
        let theme = appearance.theme();

        Container::new(
            ui_builder
                .span(CODE_FEATURE_NAME)
                .with_style(UiComponentStyles {
                    font_size: Some(24.0),
                    font_weight: Some(Weight::Bold),
                    font_color: Some(theme.active_ui_text_color().into()),
                    ..Default::default()
                })
                .build()
                .finish(),
        )
        .with_padding_bottom(15.)
        .finish()
    }

    /// Renders the "Initialization Settings" section header.
    fn render_initialization_settings_header(&self, appearance: &Appearance) -> Box<dyn Element> {
        let ui_builder = appearance.ui_builder();
        let theme = appearance.theme();

        Container::new(
            ui_builder
                .span(INITIALIZATION_SETTINGS_HEADER)
                .with_style(UiComponentStyles {
                    font_size: Some(18.0),
                    font_weight: Some(Weight::Semibold),
                    font_color: Some(theme.active_ui_text_color().into()),
                    ..Default::default()
                })
                .build()
                .finish(),
        )
        .with_margin_top(8.)
        .with_margin_bottom(12.)
        .finish()
    }

    /// Renders the "Codebase indexing" toggle row (legacy layout).
    fn render_codebase_indexing_toggle_row(
        &self,
        global_ai_enabled: bool,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let ui_builder = appearance.ui_builder();
        let theme = appearance.theme();
        let admin_setting = UserWorkspaces::as_ref(app).team_allows_codebase_context();

        let label = ui_builder
            .span(CODEBASE_INDEXING_LABEL)
            .with_style(UiComponentStyles {
                font_size: Some(16.0),
                font_weight: Some(Weight::Semibold),
                font_color: Some(theme.active_ui_text_color().into()),
                ..Default::default()
            })
            .build()
            .finish();

        let switch = ui_builder
            .switch(self.switch_state.clone())
            .check(UserWorkspaces::as_ref(app).is_codebase_context_enabled(app));

        let disabled_tooltip_text = match admin_setting {
            AdminEnablementSetting::Enable => Some(INDEXING_WORKSPACE_ENABLED_ADMIN_TEXT),
            AdminEnablementSetting::Disable => Some(INDEXING_DISABLED_ADMIN_TEXT),
            AdminEnablementSetting::RespectUserSetting if !global_ai_enabled => {
                Some(INDEXING_DISABLED_GLOBAL_AI_TEXT)
            }
            AdminEnablementSetting::RespectUserSetting => None,
        };

        let toggle_element = if let Some(tooltip_text) = disabled_tooltip_text {
            switch
                .with_tooltip(TooltipConfig {
                    text: tooltip_text.to_string(),
                    styles: ui_builder.default_tool_tip_styles(),
                })
                .disable()
                .build()
                .finish()
        } else {
            switch
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(CodeSettingsPageAction::ToggleCodebaseContext);
                })
                .finish()
        };

        let toggle = Container::new(toggle_element)
            .with_padding_right(TOGGLE_BUTTON_RIGHT_PADDING)
            .finish();

        Container::new(
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(label)
                .with_child(toggle)
                .finish(),
        )
        .with_padding_bottom(6.)
        .finish()
    }

    /// Renders the "Initialized / indexed folders" section.
    fn render_initialized_folders(
        &self,
        mouse_states: InitializedFoldersMouseStates,
        suggested_server_statuses: &HashMap<(PathBuf, LSPServerType), LspRepoStatus>,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let ui_builder = appearance.ui_builder();
        let theme = appearance.theme();

        let InitializedFoldersMouseStates {
            codebase_manual_resync: codebase_manual_resync_mouse_states,
            codebase_delete: codebase_delete_mouse_states,
            #[cfg(not(target_family = "wasm"))]
                remote_codebase_manual_resync: remote_codebase_manual_resync_mouse_states,
            #[cfg(not(target_family = "wasm"))]
                remote_codebase_delete: remote_codebase_delete_mouse_states,
            lsp_rows: lsp_row_mouse_states,
            open_project_rules: open_project_rules_mouse_states,
        } = mouse_states;

        let mut content = Flex::column();

        // Section header with "Index folder" button
        content.add_child(
            Container::new(
                Flex::row()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_child(
                        ui_builder
                            .span("Initialized / indexed folders")
                            .with_style(UiComponentStyles {
                                font_size: Some(16.0),
                                font_weight: Some(Weight::Semibold),
                                font_color: Some(theme.active_ui_text_color().into()),
                                ..Default::default()
                            })
                            .build()
                            .finish(),
                    )
                    .with_child(ChildView::new(&self.manual_add_directory_button).finish())
                    .finish(),
            )
            .with_margin_top(8.)
            .with_margin_bottom(12.)
            .finish(),
        );

        // Get workspaces from PersistedWorkspace
        let workspaces: Vec<WorkspaceMetadata> =
            PersistedWorkspace::as_ref(app).workspaces().collect();
        #[cfg(not(target_family = "wasm"))]
        let remote_entries = if FeatureFlag::RemoteCodebaseIndexing.is_enabled() {
            RemoteCodebaseIndexModel::as_ref(app).entries_for_settings()
        } else {
            Vec::new()
        };

        let codebase_manager = CodebaseIndexManager::as_ref(app);
        let lsp_manager = LspManagerModel::as_ref(app);
        let persisted_workspace = PersistedWorkspace::as_ref(app);

        let mut lsp_mouse_index = 0;
        let mut rendered_folder = false;

        for (workspace_idx, workspace) in workspaces.iter().enumerate() {
            let workspace_path = &workspace.path;

            // Get codebase index status if it exists
            let index_status =
                codebase_manager.get_codebase_index_status_for_path(workspace_path, app);

            // Get all LSP servers (enabled + disabled + suggested) for this workspace
            let all_servers: Vec<(LSPServerType, EnablementState)> = persisted_workspace
                .all_lsp_servers(workspace_path, true)
                .map(|iter| iter.collect())
                .unwrap_or_default();

            // Get mouse states for this workspace
            let resync_mouse = codebase_manual_resync_mouse_states
                .get(workspace_idx)
                .cloned()
                .unwrap_or_default();
            let delete_mouse = codebase_delete_mouse_states
                .get(workspace_idx)
                .cloned()
                .unwrap_or_default();

            // Skip workspaces that have neither an index nor any LSP servers
            if index_status.is_none() && all_servers.is_empty() {
                continue;
            }

            rendered_folder = true;

            // Get LSP server mouse states
            let lsp_mouse_states: Vec<LspServerRowMouseStates> = all_servers
                .iter()
                .map(|_| {
                    let state = lsp_row_mouse_states
                        .get(lsp_mouse_index)
                        .cloned()
                        .unwrap_or_default();

                    lsp_mouse_index += 1;

                    state
                })
                .collect();

            let open_rules_mouse = open_project_rules_mouse_states
                .get(workspace_idx)
                .cloned()
                .unwrap_or_default();

            content.add_child(self.render_workspace_row(
                workspace_path,
                index_status.as_ref(),
                &all_servers,
                lsp_manager,
                resync_mouse,
                delete_mouse,
                lsp_mouse_states,
                open_rules_mouse,
                suggested_server_statuses,
                appearance,
                app,
            ));
        }
        #[cfg(not(target_family = "wasm"))]
        for (entry_idx, entry) in remote_entries.iter().enumerate() {
            rendered_folder = true;
            let resync_mouse = remote_codebase_manual_resync_mouse_states
                .get(entry_idx)
                .cloned()
                .unwrap_or_default();
            let delete_mouse = remote_codebase_delete_mouse_states
                .get(entry_idx)
                .cloned()
                .unwrap_or_default();

            content.add_child(self.render_remote_workspace_row(
                entry,
                resync_mouse,
                delete_mouse,
                appearance,
            ));
        }

        if !rendered_folder {
            content.add_child(
                Container::new(
                    appearance
                        .ui_builder()
                        .paragraph("No folders have been initialized yet.")
                        .build()
                        .finish(),
                )
                .with_margin_bottom(MAIN_SECTION_MARGIN)
                .finish(),
            );
        }

        content.finish()
    }

    /// Renders a single workspace row with its indexing status and LSP servers.
    #[allow(clippy::too_many_arguments)]
    fn render_workspace_row(
        &self,
        workspace_path: &Path,
        index_status: Option<&CodebaseIndexStatus>,
        all_servers: &[(LSPServerType, EnablementState)],
        lsp_manager: &LspManagerModel,
        resync_mouse: MouseStateHandle,
        delete_mouse: MouseStateHandle,
        lsp_mouse_states: Vec<LspServerRowMouseStates>,
        open_rules_mouse: MouseStateHandle,
        suggested_server_statuses: &HashMap<(PathBuf, LSPServerType), LspRepoStatus>,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let ui_builder = appearance.ui_builder();
        let theme = appearance.theme();

        let mut workspace_content = Flex::column().with_spacing(MAIN_SECTION_MARGIN);

        // Workspace path header with "Open project rules" button
        let home_dir =
            dirs::home_dir().and_then(|home_dir| home_dir.to_str().map(|s| s.to_owned()));
        let user_friendly = user_friendly_path(
            workspace_path.to_string_lossy().as_ref(),
            home_dir.as_deref(),
        )
        .to_string();

        // Query ProjectContextModel for rules under this workspace
        let workspace_rule_paths =
            ProjectContextModel::as_ref(app).rules_for_workspace(workspace_path);

        // Only show "Open project rules" button if rules exist for this workspace
        let open_rules_button: Option<Box<dyn Element>> = if !workspace_rule_paths.is_empty() {
            let open_rules_button = ui_builder
                .button(ButtonVariant::Secondary, open_rules_mouse)
                .with_style(UiComponentStyles {
                    font_size: Some(12.),
                    padding: Some(Coords {
                        top: 4.,
                        bottom: 4.,
                        left: 8.,
                        right: 8.,
                    }),
                    ..Default::default()
                })
                .with_hovered_styles(UiComponentStyles {
                    background: Some(theme.surface_3().into()),
                    ..Default::default()
                })
                .with_text_and_icon_label(
                    warpui::ui_components::button::TextAndIcon::new(
                        warpui::ui_components::button::TextAndIconAlignment::IconFirst,
                        "Open project rules",
                        warpui::elements::Icon::new(
                            "bundled/svg/file-code-02.svg",
                            theme.foreground(),
                        ),
                        warpui::elements::MainAxisSize::Min,
                        warpui::elements::MainAxisAlignment::Center,
                        pathfinder_geometry::vector::vec2f(14., 14.),
                    )
                    .with_inner_padding(4.),
                )
                .build()
                .with_cursor(Cursor::PointingHand)
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(CodeSettingsPageAction::OpenProjectRules {
                        rule_paths: workspace_rule_paths.clone(),
                    });
                })
                .finish();
            Some(open_rules_button)
        } else {
            None
        };

        workspace_content.add_child(self.render_workspace_header(
            user_friendly,
            open_rules_button,
            appearance,
        ));

        // Indexing section (always rendered per design)
        workspace_content.add_child(self.render_indexing_subsection(
            workspace_path,
            index_status,
            resync_mouse,
            delete_mouse,
            appearance,
        ));

        // LSP Servers section (if any servers known)
        if !all_servers.is_empty() {
            workspace_content.add_child(self.render_lsp_servers_subsection(
                workspace_path,
                all_servers,
                lsp_manager,
                lsp_mouse_states,
                suggested_server_statuses,
                appearance,
                app,
            ));
        }

        self.render_workspace_row_container(workspace_content.finish(), appearance)
    }

    #[cfg(not(target_family = "wasm"))]
    fn render_remote_workspace_row(
        &self,
        entry: &RemoteCodebaseIndexSettingsEntry,
        resync_mouse: MouseStateHandle,
        delete_mouse: MouseStateHandle,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let mut workspace_content = Flex::column().with_spacing(MAIN_SECTION_MARGIN);

        let remote_path_label = format!("{}:{}", entry.host_label, entry.remote_path.path.as_str());
        workspace_content.add_child(self.render_workspace_header(
            remote_path_label,
            None,
            appearance,
        ));

        workspace_content.add_child(self.render_indexing_subsection_for_target(
            self.remote_indexing_status_presentation(&entry.status, appearance),
            Some(LocalOrRemotePath::Remote(entry.remote_path.clone())),
            resync_mouse,
            delete_mouse,
            appearance,
        ));

        self.render_workspace_row_container(workspace_content.finish(), appearance)
    }

    fn render_workspace_header(
        &self,
        label: String,
        action_button: Option<Box<dyn Element>>,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let path_label = Shrinkable::new(
            1.,
            appearance
                .ui_builder()
                .span(label)
                .with_style(UiComponentStyles {
                    font_family_id: Some(appearance.monospace_font_family()),
                    font_size: Some(appearance.ui_font_size()),
                    font_weight: Some(Weight::Bold),
                    font_color: Some(theme.active_ui_text_color().into()),
                    ..Default::default()
                })
                .build()
                .finish(),
        )
        .finish();

        let mut header_row = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_cross_axis_alignment(CrossAxisAlignment::Center);
        header_row.add_child(Expanded::new(1., path_label).finish());
        if let Some(action_button) = action_button {
            header_row.add_child(action_button);
        }
        header_row.finish()
    }

    fn render_workspace_row_container(
        &self,
        workspace_content: Box<dyn Element>,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        Container::new(workspace_content)
            .with_uniform_padding(MAIN_SECTION_MARGIN)
            .with_background(theme.surface_1())
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
            .with_margin_bottom(MAIN_SECTION_MARGIN)
            .finish()
    }

    /// Renders the indexing subsection within a workspace row.
    fn render_indexing_subsection(
        &self,
        workspace_path: &Path,
        index_status: Option<&CodebaseIndexStatus>,
        resync_mouse: MouseStateHandle,
        delete_mouse: MouseStateHandle,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        self.render_indexing_subsection_for_target(
            self.local_indexing_status_presentation(index_status, appearance),
            Some(LocalOrRemotePath::Local(workspace_path.to_path_buf())),
            resync_mouse,
            delete_mouse,
            appearance,
        )
    }

    fn render_indexing_subsection_for_target(
        &self,
        presentation: IndexingStatusPresentation,
        action_target: Option<LocalOrRemotePath>,
        resync_mouse: MouseStateHandle,
        delete_mouse: MouseStateHandle,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let ui_builder = appearance.ui_builder();
        let theme = appearance.theme();

        let mut column = Flex::column().with_spacing(SUB_SECTION_MARGIN);
        column.add_child(
            ui_builder
                .span("INDEXING")
                .with_style(UiComponentStyles {
                    font_size: Some(11.0),
                    font_weight: Some(Weight::Semibold),
                    font_color: Some(theme.disabled_ui_text_color().into()),
                    ..Default::default()
                })
                .build()
                .finish(),
        );

        let (status_label, action_buttons) = self.render_index_status_parts(
            presentation,
            action_target,
            resync_mouse,
            delete_mouse,
            appearance,
        );
        column.add_child(
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(status_label)
                .with_child(action_buttons)
                .finish(),
        );

        column.finish()
    }

    fn local_indexing_status_presentation(
        &self,
        index_state: Option<&CodebaseIndexStatus>,
        appearance: &Appearance,
    ) -> IndexingStatusPresentation {
        let theme = appearance.theme();
        let Some(index_state) = index_state else {
            return IndexingStatusPresentation {
                text: Cow::from("No index created"),
                color: theme.disabled_ui_text_color().into_solid(),
                icon: Some(Icon::SlashCircle),
                show_retry: false,
                show_delete: false,
            };
        };

        if index_state.has_pending() {
            let text = match index_state.sync_progress() {
                Some(SyncProgress::Discovering { total_nodes }) => {
                    Cow::from(format!("Discovered {total_nodes} chunks"))
                }
                Some(SyncProgress::Syncing {
                    completed_nodes,
                    total_nodes,
                }) => Cow::from(format!("Syncing - {completed_nodes} / {total_nodes}")),
                None => Cow::from("Syncing..."),
            };

            return IndexingStatusPresentation {
                text,
                color: theme.disabled_ui_text_color().into_solid(),
                icon: None,
                show_retry: false,
                show_delete: true,
            };
        }

        if let Some(completed_successfully) = index_state.last_sync_successful() {
            let (text, color, icon) = if completed_successfully {
                ("Synced", theme.ansi_fg_green(), Icon::Check)
            } else if let Some(CodebaseIndexFinishedStatus::Failed(
                CodebaseIndexingError::ExceededMaxFileLimit
                | CodebaseIndexingError::MaxDepthExceeded,
            )) = index_state.last_sync_result()
            {
                (
                    "Codebase too large",
                    theme.ui_warning_color(),
                    Icon::AlertTriangle,
                )
            } else if index_state.has_synced_version() {
                (
                    "Stale",
                    theme.nonactive_ui_detail().into_solid(),
                    Icon::ClockRefresh,
                )
            } else {
                ("Failed", theme.ui_error_color(), Icon::AlertTriangle)
            };

            return IndexingStatusPresentation {
                text: Cow::from(text),
                color,
                icon: Some(icon),
                show_retry: true,
                show_delete: true,
            };
        }

        log::warn!("No index state for codebase");
        IndexingStatusPresentation {
            text: Cow::from("No index built"),
            color: theme.nonactive_ui_text_color().into_solid(),
            icon: None,
            show_retry: false,
            show_delete: true,
        }
    }

    #[cfg(not(target_family = "wasm"))]
    fn remote_indexing_status_presentation(
        &self,
        status: &RemoteCodebaseIndexStatus,
        appearance: &Appearance,
    ) -> IndexingStatusPresentation {
        let theme = appearance.theme();

        match status.state {
            RemoteCodebaseIndexState::NotEnabled => IndexingStatusPresentation {
                text: Cow::from("No index created"),
                color: theme.disabled_ui_text_color().into_solid(),
                icon: Some(Icon::SlashCircle),
                show_retry: true,
                show_delete: true,
            },
            RemoteCodebaseIndexState::Unavailable => IndexingStatusPresentation {
                text: Cow::from("Unavailable"),
                color: theme.disabled_ui_text_color().into_solid(),
                icon: Some(Icon::SlashCircle),
                show_retry: false,
                show_delete: true,
            },
            RemoteCodebaseIndexState::Disabled => IndexingStatusPresentation {
                text: Cow::from("Disabled"),
                color: theme.disabled_ui_text_color().into_solid(),
                icon: Some(Icon::SlashCircle),
                show_retry: true,
                show_delete: true,
            },
            RemoteCodebaseIndexState::Queued => IndexingStatusPresentation {
                text: Cow::from("Queued"),
                color: theme.disabled_ui_text_color().into_solid(),
                icon: None,
                show_retry: false,
                show_delete: true,
            },
            RemoteCodebaseIndexState::Indexing => {
                let text = match (status.progress_completed, status.progress_total) {
                    (Some(completed), Some(total)) => {
                        Cow::from(format!("Indexing - {completed} / {total}"))
                    }
                    (Some(completed), None) => Cow::from(format!("Indexing - {completed}")),
                    (None, Some(total)) => Cow::from(format!("Indexing - 0 / {total}")),
                    (None, None) => Cow::from("Indexing..."),
                };

                IndexingStatusPresentation {
                    text,
                    color: theme.disabled_ui_text_color().into_solid(),
                    icon: None,
                    show_retry: false,
                    show_delete: true,
                }
            }
            RemoteCodebaseIndexState::Ready => IndexingStatusPresentation {
                text: Cow::from("Synced"),
                color: theme.ansi_fg_green(),
                icon: Some(Icon::Check),
                show_retry: true,
                show_delete: true,
            },
            RemoteCodebaseIndexState::Stale => IndexingStatusPresentation {
                text: Cow::from("Stale"),
                color: theme.nonactive_ui_detail().into_solid(),
                icon: Some(Icon::ClockRefresh),
                show_retry: true,
                show_delete: true,
            },
            RemoteCodebaseIndexState::Failed => IndexingStatusPresentation {
                text: Cow::from("Failed"),
                color: theme.ui_error_color(),
                icon: Some(Icon::AlertTriangle),
                show_retry: true,
                show_delete: true,
            },
        }
    }

    /// Returns (status_label, action_buttons) as separate elements for the indexing row.
    fn render_index_status_parts(
        &self,
        presentation: IndexingStatusPresentation,
        action_target: Option<LocalOrRemotePath>,
        manual_resync_mouse_state: MouseStateHandle,
        delete_mouse_state: MouseStateHandle,
        appearance: &Appearance,
    ) -> (Box<dyn Element>, Box<dyn Element>) {
        let theme = appearance.theme();
        let ui_builder = appearance.ui_builder();

        let mut label_row = Flex::row()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Center);
        if let Some(status_icon) = presentation.icon {
            label_row.add_child(
                Container::new(
                    ConstrainedBox::new(
                        status_icon
                            .to_warpui_icon(ThemeFill::Solid(presentation.color))
                            .finish(),
                    )
                    .with_width(STATUS_ICON_SIZE)
                    .with_height(STATUS_ICON_SIZE)
                    .finish(),
                )
                .with_margin_right(4.)
                .finish(),
            );
        }

        label_row.add_child(
            ui_builder
                .label(presentation.text)
                .with_style(UiComponentStyles {
                    font_color: Some(presentation.color),
                    font_size: Some(12.),
                    ..Default::default()
                })
                .build()
                .finish(),
        );

        let mut buttons_row = Flex::row()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(4.);
        if presentation.show_retry {
            if let Some(action_target) = action_target.clone() {
                buttons_row.add_child(
                    icon_button(appearance, Icon::Refresh, false, manual_resync_mouse_state)
                        .with_active_styles(UiComponentStyles {
                            background: Some(theme.surface_1().into()),
                            ..Default::default()
                        })
                        .build()
                        .on_click(move |ctx, _, _| match &action_target {
                            LocalOrRemotePath::Local(codebase_path) => {
                                ctx.dispatch_typed_action(CodeSettingsPageAction::ManualResync(
                                    codebase_path.clone(),
                                ));
                            }
                            #[cfg(not(target_family = "wasm"))]
                            LocalOrRemotePath::Remote(remote_path) => {
                                ctx.dispatch_typed_action(
                                    CodeSettingsPageAction::ManualResyncRemote(remote_path.clone()),
                                );
                            }
                            #[cfg(target_family = "wasm")]
                            LocalOrRemotePath::Remote(_) => {}
                        })
                        .finish(),
                );
            }
        }

        if presentation.show_delete {
            if let Some(action_target) = action_target {
                buttons_row.add_child(
                    icon_button(appearance, Icon::Trash, false, delete_mouse_state)
                        .with_active_styles(UiComponentStyles {
                            background: Some(theme.surface_1().into()),
                            ..Default::default()
                        })
                        .build()
                        .on_click(move |ctx, _, _| match &action_target {
                            LocalOrRemotePath::Local(codebase_path) => {
                                ctx.dispatch_typed_action(CodeSettingsPageAction::DeleteIndex(
                                    codebase_path.clone(),
                                ));
                            }
                            #[cfg(not(target_family = "wasm"))]
                            LocalOrRemotePath::Remote(remote_path) => {
                                ctx.dispatch_typed_action(
                                    CodeSettingsPageAction::DeleteRemoteIndex(remote_path.clone()),
                                );
                            }
                            #[cfg(target_family = "wasm")]
                            LocalOrRemotePath::Remote(_) => {}
                        })
                        .finish(),
                );
            }
        }

        (label_row.finish(), buttons_row.finish())
    }

    /// Renders the LSP servers subsection within a workspace row.
    #[allow(clippy::too_many_arguments)]
    fn render_lsp_servers_subsection(
        &self,
        workspace_path: &Path,
        all_servers: &[(LSPServerType, EnablementState)],
        lsp_manager: &LspManagerModel,
        lsp_mouse_states: Vec<LspServerRowMouseStates>,
        suggested_server_statuses: &HashMap<(PathBuf, LSPServerType), LspRepoStatus>,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let ui_builder = appearance.ui_builder();
        let theme = appearance.theme();

        let mut content = Flex::column().with_spacing(SUB_SECTION_MARGIN);

        // "LSP SERVERS" label
        content.add_child(
            ui_builder
                .span("LSP SERVERS")
                .with_style(UiComponentStyles {
                    font_size: Some(11.0),
                    font_weight: Some(Weight::Semibold),
                    font_color: Some(theme.disabled_ui_text_color().into()),
                    ..Default::default()
                })
                .build()
                .finish(),
        );

        // Get the actual server models for this workspace
        let server_models = lsp_manager.servers_for_workspace(workspace_path);

        for (idx, (server_type, enablement_state)) in all_servers.iter().enumerate() {
            let mouse_states = lsp_mouse_states.get(idx).cloned().unwrap_or_default();

            if *enablement_state == EnablementState::Suggested {
                // Render the "available for download" suggested server row.
                let repo_status = suggested_server_statuses
                    .get(&(workspace_path.to_path_buf(), *server_type))
                    .cloned();
                content.add_child(self.render_suggested_lsp_server_row(
                    workspace_path,
                    *server_type,
                    repo_status,
                    mouse_states,
                    appearance,
                ));
            } else {
                let is_enabled = *enablement_state == EnablementState::Yes;

                // Find the corresponding server model (only exists if enabled and running)
                let server_model = server_models.and_then(|servers| {
                    servers
                        .iter()
                        .find(|s| s.as_ref(app).server_type() == *server_type)
                });

                content.add_child(self.render_lsp_server_row(
                    workspace_path,
                    *server_type,
                    server_model,
                    is_enabled,
                    mouse_states,
                    appearance,
                    app,
                ));
            }
        }

        content.finish()
    }

    /// Renders a suggested LSP server row with "+" install/enable button.
    fn render_suggested_lsp_server_row(
        &self,
        workspace_path: &Path,
        server_type: LSPServerType,
        repo_status: Option<LspRepoStatus>,
        mouse_states: LspServerRowMouseStates,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let ui_builder = appearance.ui_builder();

        let mut row = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_cross_axis_alignment(CrossAxisAlignment::Center);

        // Left side: language initial badge + name/description column
        let mut left_content = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);

        // Language initial badge (no status dot for suggested servers)
        let badge_size = 36.0;
        let avatar = Avatar::new(
            AvatarContent::DisplayName(server_type.binary_name().to_string()),
            UiComponentStyles {
                width: Some(badge_size),
                height: Some(badge_size),
                border_radius: Some(CornerRadius::with_all(Radius::Percentage(50.))),
                font_family_id: Some(appearance.ui_font_family()),
                font_weight: Some(Weight::Bold),
                background: Some(theme.surface_3().into()),
                font_size: Some(16.),
                font_color: Some(theme.active_ui_text_color().into()),
                ..Default::default()
            },
        );

        left_content.add_child(
            Container::new(avatar.build().finish())
                .with_margin_right(8.)
                .finish(),
        );

        // Name + description
        let mut name_desc_column = Flex::column().with_spacing(4.);

        name_desc_column.add_child(
            ui_builder
                .span(server_type.binary_name())
                .with_style(UiComponentStyles {
                    font_size: Some(12.0),
                    font_color: Some(theme.active_ui_text_color().into()),
                    ..Default::default()
                })
                .build()
                .finish(),
        );

        let (description, is_installing) = match &repo_status {
            Some(LspRepoStatus::DisabledAndInstalled { .. }) => ("Installed", false),
            Some(LspRepoStatus::Installing { .. }) => ("Installing...", true),
            Some(LspRepoStatus::CheckingForInstallation) => ("Checking...", true),
            _ => ("Available for download", false),
        };

        name_desc_column.add_child(
            ui_builder
                .label(description)
                .with_style(UiComponentStyles {
                    font_color: Some(theme.disabled_ui_text_color().into()),
                    font_size: Some(12.),
                    ..Default::default()
                })
                .build()
                .finish(),
        );

        left_content.add_child(name_desc_column.finish());
        row.add_child(left_content.finish());

        // Right side: "+" button to install/enable
        if !is_installing {
            let workspace_path_clone = workspace_path.to_path_buf();
            let needs_install = matches!(
                &repo_status,
                None | Some(LspRepoStatus::DisabledAndNotInstalled { .. })
            );
            let install_button = icon_button(appearance, Icon::Plus, false, mouse_states.install)
                .with_style(UiComponentStyles {
                    border_width: Some(1.),
                    border_color: Some(theme.surface_3().into()),
                    ..Default::default()
                })
                .build()
                .with_cursor(Cursor::PointingHand)
                .on_click(move |ctx, _, _| {
                    if needs_install {
                        ctx.dispatch_typed_action(
                            CodeSettingsPageAction::InstallAndEnableLspServer {
                                workspace_path: workspace_path_clone.clone(),
                                server_type,
                            },
                        );
                    } else {
                        ctx.dispatch_typed_action(
                            CodeSettingsPageAction::EnableSuggestedLspServer {
                                workspace_path: workspace_path_clone.clone(),
                                server_type,
                            },
                        );
                    }
                })
                .finish();

            row.add_child(install_button);
        }

        Container::new(row.finish())
            .with_uniform_padding(12.)
            .with_background(theme.surface_2())
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
            .finish()
    }

    /// Renders a single LSP server row with language initial icon, status, and toggle.
    #[allow(clippy::too_many_arguments)]
    fn render_lsp_server_row(
        &self,
        workspace_path: &Path,
        server_type: LSPServerType,
        server_model: Option<&warpui::ModelHandle<LspServerModel>>,
        is_enabled: bool,
        mouse_states: LspServerRowMouseStates,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let ui_builder = appearance.ui_builder();

        let mut row = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_cross_axis_alignment(CrossAxisAlignment::Center);

        // Left side: language initial badge + name/status column
        let mut left_content = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);

        // Language initial badge with status dot overlay (using Avatar component)
        let (status_color, status_text) = self.get_lsp_status_info(server_model, app, theme);
        let is_failed = server_model
            .is_some_and(|model| matches!(model.as_ref(app).state(), LspState::Failed { .. }));

        // Language initial badge with status dot overlay (using Avatar component)
        let badge_size = 36.0;
        let mut avatar = Avatar::new(
            AvatarContent::DisplayName(server_type.binary_name().to_string()),
            UiComponentStyles {
                width: Some(badge_size),
                height: Some(badge_size),
                border_radius: Some(CornerRadius::with_all(Radius::Percentage(50.))),
                font_family_id: Some(appearance.ui_font_family()),
                font_weight: Some(Weight::Bold),
                background: Some(theme.surface_3().into()),
                font_size: Some(16.),
                font_color: Some(theme.active_ui_text_color().into()),
                ..Default::default()
            },
        );

        avatar = avatar.with_status_element_with_offset(
            StatusElementTypes::Circle,
            UiComponentStyles {
                width: Some(LSP_STATUS_INDICATOR_SIZE),
                height: Some(LSP_STATUS_INDICATOR_SIZE),
                border_radius: Some(CornerRadius::with_all(Radius::Percentage(50.))),
                background: Some(Fill::Solid(status_color)),
                ..Default::default()
            },
            -5.,
            5.,
        );

        left_content.add_child(
            Container::new(avatar.build().finish())
                .with_margin_right(8.)
                .finish(),
        );

        // Name + status on separate lines
        let mut name_status_column = Flex::column().with_spacing(4.);

        // Server name
        name_status_column.add_child(
            ui_builder
                .span(server_type.binary_name())
                .with_style(UiComponentStyles {
                    font_size: Some(12.0),
                    font_color: Some(theme.active_ui_text_color().into()),
                    ..Default::default()
                })
                .build()
                .finish(),
        );

        // Status text
        let status_text_color = if is_failed {
            Some(status_color)
        } else {
            Some(theme.disabled_ui_text_color().into())
        };

        name_status_column.add_child(
            ui_builder
                .label(status_text)
                .with_style(UiComponentStyles {
                    font_color: status_text_color,
                    font_size: Some(12.),
                    ..Default::default()
                })
                .build()
                .finish(),
        );

        left_content.add_child(name_status_column.finish());
        row.add_child(left_content.finish());

        // Right side: restart/logs buttons (if failed) + toggle switch (always)
        let mut right_content = Flex::row()
            .with_spacing(8.)
            .with_cross_axis_alignment(CrossAxisAlignment::Center);

        if is_failed {
            if let Some(server_handle) = server_model.cloned() {
                let server_for_action = server_handle.clone();
                let restart_button = ui_builder
                    .button(ButtonVariant::Secondary, mouse_states.restart)
                    .with_style(UiComponentStyles {
                        font_size: Some(12.),
                        ..Default::default()
                    })
                    .with_hovered_styles(UiComponentStyles {
                        background: Some(theme.surface_3().into()),
                        ..Default::default()
                    })
                    .with_text_label("Restart server".to_owned())
                    .build()
                    .with_cursor(Cursor::PointingHand)
                    .on_click(move |ctx, _, _| {
                        ctx.dispatch_typed_action(CodeSettingsPageAction::RestartLspServer {
                            server: server_for_action.clone(),
                        });
                    })
                    .finish();

                right_content.add_child(restart_button);
            }
        }

        // Show "View logs" when the server has been started (Available, Starting/Busy, or Failed)
        #[cfg(not(target_family = "wasm"))]
        {
            let has_logs = server_model.is_some_and(|model| {
                matches!(
                    model.as_ref(app).state(),
                    LspState::Available { .. } | LspState::Starting | LspState::Failed { .. }
                )
            });
            if has_logs {
                let log_path = crate::code::lsp_logs::log_file_path(server_type, workspace_path);
                let view_logs_button = ui_builder
                    .button(ButtonVariant::Accent, mouse_states.view_logs)
                    .with_style(UiComponentStyles {
                        font_size: Some(12.),
                        ..Default::default()
                    })
                    .with_text_label("View logs".to_owned())
                    .build()
                    .with_cursor(Cursor::PointingHand)
                    .on_click(move |ctx, _, _| {
                        ctx.dispatch_typed_action(CodeSettingsPageAction::OpenLspLogs {
                            log_path: log_path.clone(),
                        });
                    })
                    .finish();

                right_content.add_child(view_logs_button);
            }
        }

        // Toggle switch (always shown)
        let workspace_path_clone = workspace_path.to_path_buf();
        let server_type_clone = server_type;
        right_content.add_child(
            ui_builder
                .switch(mouse_states.toggle)
                .check(is_enabled)
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(CodeSettingsPageAction::ToggleLspServer {
                        workspace_path: workspace_path_clone.clone(),
                        server_type: server_type_clone,
                        currently_enabled: is_enabled,
                    });
                })
                .finish(),
        );

        row.add_child(right_content.finish());

        Container::new(row.finish())
            .with_uniform_padding(12.)
            .with_background(theme.surface_2())
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
            .finish()
    }

    /// Gets the status color and text for an LSP server.
    fn get_lsp_status_info(
        &self,
        server_model: Option<&warpui::ModelHandle<LspServerModel>>,
        app: &AppContext,
        theme: &warp_core::ui::theme::WarpTheme,
    ) -> (ColorU, &'static str) {
        match server_model {
            Some(model) => {
                let server = model.as_ref(app);
                match server.state() {
                    LspState::Available { .. } if !server.has_pending_tasks() => (
                        AnsiColorIdentifier::Green
                            .to_ansi_color(&theme.terminal_colors().normal)
                            .into(),
                        "Available",
                    ),
                    LspState::Starting | LspState::Available { .. } => (
                        AnsiColorIdentifier::Yellow
                            .to_ansi_color(&theme.terminal_colors().normal)
                            .into(),
                        "Busy",
                    ),
                    LspState::Failed { .. } => (
                        AnsiColorIdentifier::Red
                            .to_ansi_color(&theme.terminal_colors().normal)
                            .into(),
                        "Failed",
                    ),
                    LspState::Stopped { .. } | LspState::Stopping { .. } => {
                        (theme.disabled_ui_text_color().into_solid(), "Stopped")
                    }
                }
            }
            None => (theme.disabled_ui_text_color().into_solid(), "Not running"),
        }
    }
}

/// A simple widget that renders a subheader title for a Code subpage.
struct CodeSubpageHeaderWidget {
    title: &'static str,
}

impl SettingsWidget for CodeSubpageHeaderWidget {
    type View = CodeSettingsPageView;

    fn search_terms(&self) -> &str {
        self.title
    }

    fn render(
        &self,
        _view: &Self::View,
        appearance: &Appearance,
        _app: &AppContext,
    ) -> Box<dyn Element> {
        build_sub_header(appearance, self.title, None)
            .with_padding_bottom(HEADER_PADDING)
            .finish()
    }
}

struct CodebaseIndexingCategorizedWidget {
    inner: CodePageWidget,
}

impl SettingsWidget for CodebaseIndexingCategorizedWidget {
    type View = CodeSettingsPageView;

    fn search_terms(&self) -> &str {
        "codebase index indexing repository code context embedding auto-index lsp language server"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let ui_builder = appearance.ui_builder();
        let global_ai_enabled = AISettings::as_ref(app).is_any_ai_enabled(app);
        let codebase_context_enabled = UserWorkspaces::as_ref(app).is_codebase_context_enabled(app);

        let mut content = Flex::column();

        // Codebase indexing toggle using render_body_item for consistent styling
        let admin_setting = UserWorkspaces::as_ref(app).team_allows_codebase_context();
        let switch = ui_builder
            .switch(self.inner.switch_state.clone())
            .check(codebase_context_enabled);

        let disabled_tooltip_text = match admin_setting {
            AdminEnablementSetting::Enable => Some(INDEXING_WORKSPACE_ENABLED_ADMIN_TEXT),
            AdminEnablementSetting::Disable => Some(INDEXING_DISABLED_ADMIN_TEXT),
            AdminEnablementSetting::RespectUserSetting if !global_ai_enabled => {
                Some(INDEXING_DISABLED_GLOBAL_AI_TEXT)
            }
            AdminEnablementSetting::RespectUserSetting => None,
        };

        let toggle_element = if let Some(tooltip_text) = disabled_tooltip_text {
            switch
                .with_tooltip(TooltipConfig {
                    text: tooltip_text.to_string(),
                    styles: ui_builder.default_tool_tip_styles(),
                })
                .disable()
                .build()
                .finish()
        } else {
            switch
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(CodeSettingsPageAction::ToggleCodebaseContext);
                })
                .finish()
        };

        content.add_child(render_body_item::<CodeSettingsPageAction>(
            CODEBASE_INDEXING_LABEL.into(),
            None,
            LocalOnlyIconState::Hidden,
            ToggleState::Enabled,
            appearance,
            toggle_element,
            Some(CODEBASE_INDEX_DESCRIPTION.into()),
        ));

        // Auto-indexing toggle (only shown when codebase indexing is enabled)
        if global_ai_enabled && codebase_context_enabled {
            let auto_indexing_enabled = *CodeSettings::as_ref(app).auto_indexing_enabled;

            content.add_child(render_body_item::<CodeSettingsPageAction>(
                AUTO_INDEX_FEATURE_NAME.into(),
                None,
                LocalOnlyIconState::Hidden,
                ToggleState::Enabled,
                appearance,
                ui_builder
                    .switch(self.inner.auto_index_switch_state.clone())
                    .check(auto_indexing_enabled)
                    .build()
                    .on_click(move |ctx, _, _| {
                        ctx.dispatch_typed_action(CodeSettingsPageAction::ToggleAutoIndexing);
                    })
                    .finish(),
                Some(AUTO_INDEX_DESCRIPTION.into()),
            ));

            if !CodebaseIndexManager::as_ref(app).can_create_new_indices() {
                content.add_child(
                    ui_builder
                        .paragraph(CODEBASE_INDEX_LIMIT_REACHED)
                        .with_style(UiComponentStyles {
                            font_color: Some(appearance.theme().disabled_ui_text_color().into()),
                            ..Default::default()
                        })
                        .build()
                        .with_margin_bottom(8.0)
                        .finish(),
                );
            }
        }

        // Initialized / indexed folders section
        let mouse_states = InitializedFoldersMouseStates {
            codebase_manual_resync: view.codebase_manual_resync_mouse_states.clone(),
            codebase_delete: view.codebase_delete_mouse_states.clone(),
            #[cfg(not(target_family = "wasm"))]
            remote_codebase_manual_resync: view.remote_codebase_manual_resync_mouse_states.clone(),
            #[cfg(not(target_family = "wasm"))]
            remote_codebase_delete: view.remote_codebase_delete_mouse_states.clone(),
            lsp_rows: view.lsp_row_mouse_states.clone(),
            open_project_rules: view.open_project_rules_mouse_states.clone(),
        };
        content.add_child(self.inner.render_initialized_folders(
            mouse_states,
            &view.suggested_server_statuses,
            appearance,
            app,
        ));

        content.finish()
    }
}

#[cfg(feature = "local_fs")]
struct ExternalEditorCodeWidget;

#[cfg(feature = "local_fs")]
impl SettingsWidget for ExternalEditorCodeWidget {
    type View = CodeSettingsPageView;

    fn search_terms(&self) -> &str {
        "code editor open files markdown AI conversations layout pane tab"
    }

    fn render(
        &self,
        view: &Self::View,
        _appearance: &Appearance,
        _app: &AppContext,
    ) -> Box<dyn Element> {
        if let Some(editor_view) = &view.external_editor_view {
            ChildView::new(editor_view).finish()
        } else {
            Empty::new().finish()
        }
    }
}

#[derive(Default)]
struct AutoOpenCodeReviewPaneCodeWidget {
    switch_state: SwitchStateHandle,
}

impl SettingsWidget for AutoOpenCodeReviewPaneCodeWidget {
    type View = CodeSettingsPageView;

    fn search_terms(&self) -> &str {
        "oz auto open code review pane panel agent mode change first time accepted diff view conversation"
    }

    fn render(
        &self,
        _view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let general_settings = GeneralSettings::as_ref(app);
        render_body_item::<CodeSettingsPageAction>(
            "Auto open code review panel".into(),
            None,
            LocalOnlyIconState::Hidden,
            ToggleState::Enabled,
            appearance,
            appearance
                .ui_builder()
                .switch(self.switch_state.clone())
                .check(*general_settings.auto_open_code_review_pane_on_first_agent_change)
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(CodeSettingsPageAction::ToggleAutoOpenCodeReviewPane);
                })
                .finish(),
            Some("When this setting is on, the code review panel will open on the first accepted diff of a conversation".into()),
        )
    }
}

impl SettingsPageMeta for CodeSettingsPageView {
    fn section() -> SettingsSection {
        SettingsSection::Code
    }

    fn update_filter(&mut self, query: &str, ctx: &mut ViewContext<Self>) -> MatchData {
        self.page.update_filter(query, ctx)
    }

    fn should_render(&self, _ctx: &AppContext) -> bool {
        FeatureFlag::FullSourceCodeEmbedding.is_enabled()
            || FeatureFlag::OpenWarpNewSettingsModes.is_enabled()
    }

    fn on_page_selected(&mut self, _: bool, ctx: &mut ViewContext<Self>) {
        // We want to immediately see if the user is part of a workspace rather than wait for the next poll.
        std::mem::drop(
            TeamUpdateManager::handle(ctx)
                .update(ctx, |manager, ctx| manager.refresh_workspace_metadata(ctx)),
        );
    }

    fn scroll_to_widget(&mut self, widget_id: &'static str) {
        self.page.scroll_to_widget(widget_id)
    }

    fn clear_highlighted_widget(&mut self) {
        self.page.clear_highlighted_widget();
    }
}

impl From<ViewHandle<CodeSettingsPageView>> for SettingsPageViewHandle {
    fn from(view_handle: ViewHandle<CodeSettingsPageView>) -> Self {
        SettingsPageViewHandle::Code(view_handle)
    }
}

#[derive(Default)]
struct CodeReviewPanelToggleWidget {
    switch_state: SwitchStateHandle,
}

impl SettingsWidget for CodeReviewPanelToggleWidget {
    type View = CodeSettingsPageView;

    fn search_terms(&self) -> &str {
        "code review panel right side diff git"
    }

    fn render(
        &self,
        _view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let tab_settings = TabSettings::as_ref(app);

        render_body_item::<CodeSettingsPageAction>(
            "Show code review button".into(),
            None,
            LocalOnlyIconState::Hidden,
            ToggleState::Enabled,
            appearance,
            appearance
                .ui_builder()
                .switch(self.switch_state.clone())
                .check(*tab_settings.show_code_review_button)
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(CodeSettingsPageAction::ToggleCodeReviewPanel);
                })
                .finish(),
            Some(
                "Show a button in the top right of the window to toggle the code review panel."
                    .into(),
            ),
        )
    }
}

#[derive(Default)]
struct CodeReviewDiffStatsToggleWidget {
    switch_state: SwitchStateHandle,
}

impl SettingsWidget for CodeReviewDiffStatsToggleWidget {
    type View = CodeSettingsPageView;

    fn search_terms(&self) -> &str {
        "code review diff stats lines added removed counts"
    }

    fn render(
        &self,
        _view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let tab_settings = TabSettings::as_ref(app);

        render_body_item::<CodeSettingsPageAction>(
            "Show diff stats on code review button".into(),
            None,
            LocalOnlyIconState::Hidden,
            ToggleState::Enabled,
            appearance,
            appearance
                .ui_builder()
                .switch(self.switch_state.clone())
                .check(*tab_settings.show_code_review_diff_stats)
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(
                        CodeSettingsPageAction::ToggleShowCodeReviewDiffStats,
                    );
                })
                .finish(),
            Some("Show lines added and removed counts on the code review button.".into()),
        )
    }
}

#[derive(Default)]
struct ProjectExplorerToggleWidget {
    switch_state: SwitchStateHandle,
}

impl SettingsWidget for ProjectExplorerToggleWidget {
    type View = CodeSettingsPageView;

    fn search_terms(&self) -> &str {
        "project explorer file tree left panel tools"
    }

    fn render(
        &self,
        _view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let code_settings = CodeSettings::as_ref(app);

        render_body_item::<CodeSettingsPageAction>(
            "Project explorer".into(),
            None,
            LocalOnlyIconState::Hidden,
            ToggleState::Enabled,
            appearance,
            appearance
                .ui_builder()
                .switch(self.switch_state.clone())
                .check(*code_settings.show_project_explorer)
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(CodeSettingsPageAction::ToggleProjectExplorer);
                })
                .finish(),
            Some(
                "Adds an IDE-style project explorer / file tree to the left side tools panel."
                    .into(),
            ),
        )
    }
}

#[derive(Default)]
struct GlobalSearchToggleWidget {
    switch_state: SwitchStateHandle,
}

impl SettingsWidget for GlobalSearchToggleWidget {
    type View = CodeSettingsPageView;

    fn search_terms(&self) -> &str {
        "global search file search left panel tools"
    }

    fn render(
        &self,
        _view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let code_settings = CodeSettings::as_ref(app);

        render_body_item::<CodeSettingsPageAction>(
            "Global file search".into(),
            None,
            LocalOnlyIconState::Hidden,
            ToggleState::Enabled,
            appearance,
            appearance
                .ui_builder()
                .switch(self.switch_state.clone())
                .check(*code_settings.show_global_search)
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(CodeSettingsPageAction::ToggleGlobalSearch);
                })
                .finish(),
            Some("Adds global file search to the left side tools panel.".into()),
        )
    }
}
