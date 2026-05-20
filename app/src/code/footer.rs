use std::collections::HashMap;
use std::path::{Path, PathBuf};

use lsp::supported_servers::LSPServerType;
use lsp::{
    LanguageId, LanguageServerId, LspManagerModel, LspManagerModelEvent, LspServerModel,
    LspState as LspModelState,
};
use warp_core::send_telemetry_from_ctx;

use crate::code::lsp_telemetry::{LspControlActionType, LspEnablementSource, LspTelemetryEvent};
use pathfinder_color::ColorU;
use pathfinder_geometry::vector::vec2f;
use warp_core::ui::theme::color::internal_colors;
use warp_core::ui::theme::{Fill as ThemeFill, WarpTheme};
use warp_core::ui::{appearance::Appearance, Icon};
use warpui::elements::{
    ChildAnchor, ChildView, Dismiss, Empty, Hoverable, MainAxisSize, MouseStateHandle,
    ParentAnchor, ParentOffsetBounds, Rect, Shrinkable,
};
use warpui::platform::Cursor;
use warpui::ui_components::components::{UiComponent, UiComponentStyles};
use warpui::{
    elements::{
        Border, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Fill, Flex,
        MainAxisAlignment, OffsetPositioning, Padding, ParentElement, Radius, Stack,
    },
    AppContext, Element, Entity, ModelHandle, SingletonEntity, View, WeakModelHandle,
};
use warpui::{TypedActionView, ViewContext, ViewHandle};

use warp_core::ui::theme::AnsiColorIdentifier;

#[cfg(feature = "local_fs")]
use crate::ai::persisted_workspace::PersistedWorkspaceEvent;
use crate::ai::persisted_workspace::{
    LSPEnablementResultForFile, LspRepoStatus, PersistedWorkspace,
};
use crate::settings::AISettings;
use crate::ui_components::blended_colors;
#[cfg(feature = "local_fs")]
use crate::user_config::is_tab_config_toml;
use crate::view_components::action_button::{
    ActionButton, ButtonSize, NakedTheme, PaneHeaderTheme,
};
#[cfg(feature = "local_fs")]
use repo_metadata::repositories::DetectedRepositories;
#[cfg(feature = "local_fs")]
use warp_util::local_or_remote_path::LocalOrRemotePath;

const FOOTER_HEIGHT: f32 = 24.;
/// Margin around the LSP icon container
const ICON_MARGIN: f32 = 4.;
const INDICATOR_SIZE: f32 = 8.;

#[derive(Default)]
struct SingleFileMouseStates {
    open_logs: MouseStateHandle,
    restart_server: MouseStateHandle,
    stop_server: MouseStateHandle,
    start_server: MouseStateHandle,
    remove_server: MouseStateHandle,
}

#[derive(Default)]
struct WorkspaceMouseStates {
    restart_all: MouseStateHandle,
    stop_all: MouseStateHandle,
    start_all: MouseStateHandle,
    manage_servers: MouseStateHandle,
}

/// Determines the operating mode of the footer.
enum FooterMode {
    /// Tab config editor — shows a skill CTA instead of LSP details.
    TabConfig { path: PathBuf },
    /// Single file editor — tracks one server for one file path.
    SingleFile {
        path: PathBuf,
        mouse_states: SingleFileMouseStates,
        /// Status of LSP server relevance and installation for the file's repo.
        lsp_repo_status: LspRepoStatus,
    },
    /// Workspace-level — tracks all servers for a repo root.
    Workspace {
        root_path: PathBuf,
        mouse_states: WorkspaceMouseStates,
        /// Per-server-type status of LSP relevance and installation.
        lsp_repo_statuses: LspRepoStatuses,
    },
}

impl FooterMode {
    fn path(&self) -> &Path {
        match self {
            FooterMode::TabConfig { path } => path,
            FooterMode::SingleFile { path, .. } => path,
            FooterMode::Workspace { root_path, .. } => root_path,
        }
    }

    /// Returns all CTA-worthy `LspRepoStatus` entries (i.e. those that need user action).
    /// For SingleFile, returns at most one. For Workspace, returns all
    /// `DisabledAndInstalled` or `DisabledAndNotInstalled` entries.
    fn cta_lsp_repo_statuses(&self) -> Vec<&LspRepoStatus> {
        match self {
            FooterMode::TabConfig { .. } => vec![],
            FooterMode::SingleFile {
                lsp_repo_status, ..
            } => {
                if matches!(
                    lsp_repo_status,
                    LspRepoStatus::DisabledAndInstalled { .. }
                        | LspRepoStatus::DisabledAndNotInstalled { .. }
                ) {
                    vec![lsp_repo_status]
                } else {
                    vec![]
                }
            }
            FooterMode::Workspace {
                lsp_repo_statuses, ..
            } => lsp_repo_statuses
                .values()
                .filter(|s| {
                    matches!(
                        s,
                        LspRepoStatus::DisabledAndInstalled { .. }
                            | LspRepoStatus::DisabledAndNotInstalled { .. }
                    )
                })
                .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum CodeFooterViewAction {
    CloseMenu,
    #[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
    ToggleMenu,
    EnableLSP,
    RunTabConfigSkill,
    #[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
    OpenLogs,
    #[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
    RestartServer,
    #[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
    StopServer,
    #[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
    StartServer,
    #[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
    RemoveServer,
    #[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
    RestartAllServers,
    #[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
    StopAllServers,
    #[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
    StartAllServers,
    #[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
    ManageServers,
}

enum LSPServerRenderStatus {
    Available,
    Stopped,
    Busy,
    Failed,
}

impl LSPServerRenderStatus {
    fn to_icon_color(&self, theme: &WarpTheme) -> ColorU {
        match self {
            LSPServerRenderStatus::Available => AnsiColorIdentifier::Green
                .to_ansi_color(&theme.terminal_colors().normal)
                .into(),
            LSPServerRenderStatus::Busy => AnsiColorIdentifier::Yellow
                .to_ansi_color(&theme.terminal_colors().normal)
                .into(),
            LSPServerRenderStatus::Failed => AnsiColorIdentifier::Red
                .to_ansi_color(&theme.terminal_colors().normal)
                .into(),
            LSPServerRenderStatus::Stopped => internal_colors::neutral_5(theme),
        }
    }

    fn render_status(server: Option<&LspServerModel>) -> Self {
        match server {
            Some(server) => match server.state() {
                LspModelState::Available { .. } if !server.has_pending_tasks() => {
                    LSPServerRenderStatus::Available
                }
                LspModelState::Starting | LspModelState::Available { .. } => {
                    LSPServerRenderStatus::Busy
                }
                LspModelState::Failed { .. } => LSPServerRenderStatus::Failed,
                LspModelState::Stopped { .. } | LspModelState::Stopping { .. } => {
                    LSPServerRenderStatus::Stopped
                }
            },
            None => LSPServerRenderStatus::Stopped,
        }
    }
}

pub struct CodeFooterView {
    mode: FooterMode,
    lsp_servers: Vec<WeakModelHandle<LspServerModel>>,
    /// Tracks the server IDs we've already subscribed to, so we don't duplicate subscriptions.
    subscribed_server_ids: Vec<LanguageServerId>,
    lsp_status_button: ViewHandle<ActionButton>,
    enable_lsp_button: Option<ViewHandle<ActionButton>>,
    tab_config_skill_button: Option<ViewHandle<ActionButton>>,
    is_lsp_menu_open: bool,
    /// Whether to render the top border. Disabled for code review footer.
    show_border: bool,
}

/// Wraps the per-server-type status map and enforces a single invariant on
/// every mutation: **a running server always wins**. All status updates must
/// go through [`update_status`](Self::update_status) so that stale async
/// results (e.g. an `InstallStatusUpdate` arriving after the server has
/// already started) can never clobber a live server's `Ready` status.
#[derive(Debug, Default)]
struct LspRepoStatuses {
    inner: HashMap<LSPServerType, LspRepoStatus>,
}

impl LspRepoStatuses {
    /// The single mutation point for workspace repo statuses.
    ///
    /// If the server type has a live (upgradeable) server in `lsp_servers`,
    /// the effective status is forced to `Ready` regardless of `proposed`.
    /// Otherwise `proposed` is used as-is.
    fn update_status(
        &mut self,
        server_type: LSPServerType,
        proposed: LspRepoStatus,
        lsp_servers: &[WeakModelHandle<LspServerModel>],
        app: &AppContext,
    ) {
        let is_live = lsp_servers.iter().any(|w| {
            w.upgrade(app)
                .is_some_and(|s| s.as_ref(app).server_type() == server_type)
        });
        let effective = if is_live {
            LspRepoStatus::Ready
        } else {
            proposed
        };
        self.inner.insert(server_type, effective);
    }

    fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    fn contains_key(&self, server_type: &LSPServerType) -> bool {
        self.inner.contains_key(server_type)
    }

    fn values(&self) -> impl Iterator<Item = &LspRepoStatus> {
        self.inner.values()
    }
}

impl CodeFooterView {
    #[cfg(feature = "local_fs")]
    fn is_tab_config_path(path: &Path) -> bool {
        is_tab_config_toml(path)
    }

    #[cfg(not(feature = "local_fs"))]
    fn is_tab_config_path(_path: &Path) -> bool {
        false
    }
    fn create_tab_config_skill_button(ctx: &mut ViewContext<Self>) -> ViewHandle<ActionButton> {
        ctx.add_typed_action_view(|_ctx| {
            ActionButton::new("/update-tab-config", NakedTheme)
                .with_icon(Icon::Oz)
                .with_size(ButtonSize::Small)
                .with_disabled_theme(PaneHeaderTheme)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(CodeFooterViewAction::RunTabConfigSkill);
                })
        })
    }

    fn render_tab_config_info_icon(theme: &WarpTheme) -> Box<dyn Element> {
        Container::new(
            ConstrainedBox::new(
                Icon::Info
                    .to_warpui_icon(theme.active_ui_text_color())
                    .finish(),
            )
            .with_width(12.)
            .with_height(12.)
            .finish(),
        )
        .with_margin_left(ICON_MARGIN)
        .finish()
    }

    fn is_tab_config_footer(&self) -> bool {
        matches!(self.mode, FooterMode::TabConfig { .. })
    }

    fn sync_tab_config_skill_button(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(button) = &self.tab_config_skill_button else {
            return;
        };

        let is_ai_enabled = AISettings::as_ref(ctx).is_any_ai_enabled(ctx);
        button.update(ctx, |button, ctx| {
            button.set_disabled(!is_ai_enabled, ctx);
            button.set_tooltip(
                Some(if is_ai_enabled {
                    "Open agent input with the /update-tab-config skill"
                } else {
                    "Enable AI to use the /update-tab-config skill"
                }),
                ctx,
            );
        });
    }
    fn create_lsp_status_button(
        disabled: bool,
        ctx: &mut ViewContext<Self>,
    ) -> ViewHandle<ActionButton> {
        ctx.add_typed_action_view(|ctx| {
            // Use Small size to get proper padding around the icon for a larger click target
            let mut button = ActionButton::new("", NakedTheme)
                .with_icon(Icon::Lightning)
                .with_size(ButtonSize::Small)
                .with_disabled_theme(NakedTheme)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(CodeFooterViewAction::ToggleMenu);
                });

            button.set_disabled(disabled, ctx);
            button
        })
    }

    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub fn new(path: PathBuf, ctx: &mut ViewContext<Self>) -> Self {
        let lsp_status_button = Self::create_lsp_status_button(true, ctx);
        if Self::is_tab_config_path(&path) {
            let tab_config_skill_button = Self::create_tab_config_skill_button(ctx);
            let mut footer = Self {
                mode: FooterMode::TabConfig { path },
                lsp_servers: Vec::new(),
                subscribed_server_ids: Vec::new(),
                lsp_status_button,
                enable_lsp_button: None,
                tab_config_skill_button: Some(tab_config_skill_button),
                is_lsp_menu_open: false,
                show_border: true,
            };
            footer.sync_tab_config_skill_button(ctx);
            ctx.subscribe_to_model(&AISettings::handle(ctx), |me, _, _, ctx| {
                me.sync_tab_config_skill_button(ctx);
            });
            return footer;
        }

        let server_type = LanguageId::from_path(&path).map(|id| id.server_type());

        // Create a button that dispatches EnableLSP action
        // The action handler will check lsp_repo_status to decide whether to install first
        let enable_lsp_button = server_type.map(|st| {
            let label = format!("Enable {}", st.binary_name());
            ctx.add_typed_action_view(|_ctx| {
                ActionButton::new(label, NakedTheme)
                    .with_size(ButtonSize::Small)
                    .on_click(|ctx| {
                        ctx.dispatch_typed_action(CodeFooterViewAction::EnableLSP);
                    })
            })
        });

        // Kick off detection via PersistedWorkspace and subscribe for updates
        #[cfg(feature = "local_fs")]
        let initial_status = {
            let status = Self::detect_installation_status(&path, ctx);

            // Update button label based on initial status (handles cached results)
            if let Some(enable_button) = &enable_lsp_button {
                if let Some(label) = Self::button_label_for_status(&status) {
                    enable_button.update(ctx, |button, ctx| {
                        button.set_label(label, ctx);
                    });
                }
            }

            // Subscribe to InstallStatusUpdate events from PersistedWorkspace
            let persisted = PersistedWorkspace::handle(ctx);
            ctx.subscribe_to_model(&persisted, move |me, _model_handle, event, ctx| {
                // Only handle InstallStatusUpdate events for our server type
                let PersistedWorkspaceEvent::InstallStatusUpdate {
                    server_type: event_server_type,
                    status,
                } = event
                else {
                    return;
                };

                let FooterMode::SingleFile { path, .. } = &me.mode else {
                    return;
                };

                let Some(current_server_type) =
                    LanguageId::from_path(path).map(|id| id.server_type())
                else {
                    return;
                };

                // Only update if the event is for the server type we're tracking
                if *event_server_type != current_server_type {
                    return;
                }

                // Convert LSPInstallationStatus to LspRepoStatus
                let new_status =
                    LspRepoStatus::from_installation_status(status, *event_server_type);

                if let FooterMode::SingleFile {
                    lsp_repo_status, ..
                } = &mut me.mode
                {
                    *lsp_repo_status = new_status;
                }
                me.update_enable_button_label(ctx);
                ctx.notify();
            });

            status
        };
        #[cfg(not(feature = "local_fs"))]
        let initial_status = LspRepoStatus::CheckingForInstallation;

        Self {
            mode: FooterMode::SingleFile {
                path,
                mouse_states: SingleFileMouseStates::default(),
                lsp_repo_status: initial_status,
            },
            lsp_servers: Vec::new(),
            subscribed_server_ids: Vec::new(),
            is_lsp_menu_open: false,
            lsp_status_button,
            enable_lsp_button,
            tab_config_skill_button: None,
            show_border: true,
        }
    }

    /// Creates a footer in workspace mode that tracks all LSP servers for a repo root.
    /// Used by code review to show a single aggregated footer.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub fn new_for_workspace(root_path: PathBuf, ctx: &mut ViewContext<Self>) -> Self {
        let lsp_status_button = Self::create_lsp_status_button(true, ctx);

        // Subscribe to LspManagerModel to auto-track server lifecycle
        let workspace_root = root_path.clone();
        ctx.subscribe_to_model(
            &LspManagerModel::handle(ctx),
            move |me, _, event, ctx| match event {
                LspManagerModelEvent::ServerStarted(path) if path == &workspace_root => {
                    me.refresh_workspace_servers(ctx);
                }
                LspManagerModelEvent::ServerStopped(path) if path == &workspace_root => {
                    ctx.notify();
                }
                LspManagerModelEvent::ServerRemoved {
                    workspace_root: removed_root,
                    ..
                } if removed_root == &workspace_root => {
                    me.refresh_workspace_servers(ctx);
                }
                _ => {}
            },
        );

        // Kick off async detection of available servers for this workspace
        #[cfg(feature = "local_fs")]
        {
            let persisted = PersistedWorkspace::handle(ctx);
            persisted.update(ctx, |model, ctx| {
                model.detect_available_servers_for_workspaces(vec![root_path.clone()], false, ctx);
            });

            // Subscribe to AvailableServersDetected to populate per-server statuses
            let workspace_root_for_detect = root_path.clone();
            ctx.subscribe_to_model(&persisted, move |me, _model_handle, event, ctx| {
                match event {
                    PersistedWorkspaceEvent::AvailableServersDetected {
                        workspace_path,
                        servers,
                    } if *workspace_path == workspace_root_for_detect => {
                        let FooterMode::Workspace {
                            root_path,
                            lsp_repo_statuses,
                            ..
                        } = &mut me.mode
                        else {
                            return;
                        };

                        // For each suggested server, detect its installation status
                        let root = root_path.clone();
                        for &server_type in servers {
                            let proposed =
                                PersistedWorkspace::handle(ctx).update(ctx, |model, ctx| {
                                    model.detect_lsp_workspace_status(
                                        root.clone(),
                                        server_type,
                                        ctx,
                                    )
                                });
                            lsp_repo_statuses.update_status(
                                server_type,
                                proposed,
                                &me.lsp_servers,
                                ctx,
                            );
                        }

                        // Create enable button for all CTA-worthy servers
                        let cta_statuses = me.mode.cta_lsp_repo_statuses();
                        if let Some(label) = Self::button_label_for_cta_statuses(&cta_statuses) {
                            me.enable_lsp_button = Some(ctx.add_typed_action_view(|_ctx| {
                                ActionButton::new(label, NakedTheme)
                                    .with_size(ButtonSize::Small)
                                    .on_click(|ctx| {
                                        ctx.dispatch_typed_action(CodeFooterViewAction::EnableLSP);
                                    })
                            }));
                        }

                        ctx.notify();
                    }
                    PersistedWorkspaceEvent::InstallStatusUpdate {
                        server_type,
                        status,
                    } => {
                        let FooterMode::Workspace {
                            lsp_repo_statuses, ..
                        } = &mut me.mode
                        else {
                            return;
                        };

                        // Only update if we're tracking this server type
                        if lsp_repo_statuses.contains_key(server_type) {
                            let proposed =
                                LspRepoStatus::from_installation_status(status, *server_type);
                            lsp_repo_statuses.update_status(
                                *server_type,
                                proposed,
                                &me.lsp_servers,
                                ctx,
                            );
                            me.update_enable_button_label(ctx);
                            ctx.notify();
                        }
                    }
                    PersistedWorkspaceEvent::AvailableServersDetected { .. }
                    | PersistedWorkspaceEvent::InstallationSucceeded
                    | PersistedWorkspaceEvent::InstallationFailed
                    | PersistedWorkspaceEvent::WorkspaceAdded { .. } => {}
                }
            });
        }

        let mut view = Self {
            mode: FooterMode::Workspace {
                root_path,
                mouse_states: WorkspaceMouseStates::default(),
                lsp_repo_statuses: LspRepoStatuses::default(),
            },
            lsp_servers: Vec::new(),
            subscribed_server_ids: Vec::new(),
            is_lsp_menu_open: false,
            lsp_status_button,
            enable_lsp_button: None,
            tab_config_skill_button: None,
            show_border: false,
        };

        // Populate initial servers from the manager
        view.refresh_workspace_servers(ctx);
        view
    }

    /// Refreshes the server list from the LspManagerModel for workspace mode.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    fn refresh_workspace_servers(&mut self, ctx: &mut ViewContext<Self>) {
        let FooterMode::Workspace { root_path, .. } = &self.mode else {
            return;
        };

        let lsp_manager = LspManagerModel::handle(ctx);
        let new_servers: Vec<ModelHandle<LspServerModel>> = lsp_manager
            .as_ref(ctx)
            .servers_for_workspace(root_path)
            .cloned()
            .unwrap_or_default();

        // Subscribe to any newly added servers (check by LanguageServerId)
        for server in &new_servers {
            let server_id = server.as_ref(ctx).id();
            if !self.subscribed_server_ids.contains(&server_id) {
                ctx.subscribe_to_model(server, |_, _, _, ctx| {
                    ctx.notify();
                });
            }
        }

        // Mark any server types that now have live servers as Ready.
        // Uses reconciled_repo_status (with Ready as the proposed status)
        // which is a no-op here since the server IS live, but keeps all
        // mutations going through the single reconciliation point.
        if let FooterMode::Workspace {
            lsp_repo_statuses, ..
        } = &mut self.mode
        {
            for server in &new_servers {
                let server_type = server.as_ref(ctx).server_type();
                lsp_repo_statuses.update_status(
                    server_type,
                    LspRepoStatus::Ready,
                    &self.lsp_servers,
                    ctx,
                );
            }
        }

        // Rebuild subscribed IDs to match current server set
        self.subscribed_server_ids = new_servers.iter().map(|s| s.as_ref(ctx).id()).collect();

        self.lsp_servers = new_servers.iter().map(|s| s.downgrade()).collect();
        let has_servers = !self.lsp_servers.is_empty();
        self.lsp_status_button.update(ctx, |button, ctx| {
            button.set_disabled(!has_servers, ctx);
        });

        // Update (or hide) the enable button based on remaining CTA statuses.
        self.update_enable_button_label(ctx);

        ctx.notify();
    }

    /// Returns the appropriate button label for the given LSP repo status.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    fn button_label_for_status(status: &LspRepoStatus) -> Option<String> {
        match status {
            LspRepoStatus::DisabledAndNotInstalled { server_type } => {
                Some(format!("Install {}", server_type.binary_name()))
            }
            LspRepoStatus::DisabledAndInstalled { server_type } => {
                Some(format!("Enable {}", server_type.binary_name()))
            }
            _ => None,
        }
    }

    /// Returns the appropriate button label for a set of CTA-worthy statuses.
    /// When multiple servers need action, uses plural labels
    /// ("Enable servers" / "Install servers").
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    fn button_label_for_cta_statuses(statuses: &[&LspRepoStatus]) -> Option<String> {
        match statuses.len() {
            0 => None,
            1 => Self::button_label_for_status(statuses[0]),
            _ => {
                let any_needs_install = statuses
                    .iter()
                    .any(|s| matches!(s, LspRepoStatus::DisabledAndNotInstalled { .. }));
                if any_needs_install {
                    Some("Install servers".to_string())
                } else {
                    Some("Enable servers".to_string())
                }
            }
        }
    }

    /// Detects LSP installation status for the given file path and returns the initial status.
    /// This is shared between `new` and `clear_server_subscription`. Only used in SingleFile mode.
    #[cfg(feature = "local_fs")]
    fn detect_installation_status(
        file_path: &std::path::Path,
        ctx: &mut ViewContext<Self>,
    ) -> LspRepoStatus {
        let server_type = LanguageId::from_path(file_path).map(|id| id.server_type());
        let Some(server_type) = server_type else {
            return LspRepoStatus::CheckingForInstallation;
        };

        let repo_root = DetectedRepositories::handle(ctx)
            .as_ref(ctx)
            .get_root_for_path(&LocalOrRemotePath::Local(file_path.to_path_buf()))
            .and_then(|r| r.to_local_path().map(std::path::Path::to_path_buf))
            .or_else(|| file_path.parent().map(|p| p.to_path_buf()));

        let Some(repo_root) = repo_root else {
            return LspRepoStatus::CheckingForInstallation;
        };

        PersistedWorkspace::handle(ctx).update(ctx, |model, ctx| {
            model.detect_lsp_workspace_status(repo_root, server_type, ctx)
        })
    }

    /// Updates the enable button label based on the current CTA-worthy repo statuses.
    /// Hides the button when no CTAs remain.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    fn update_enable_button_label(&mut self, ctx: &mut ViewContext<Self>) {
        let cta_statuses = self.mode.cta_lsp_repo_statuses();
        let Some(label) = Self::button_label_for_cta_statuses(&cta_statuses) else {
            // No CTA-worthy statuses remain — hide the button.
            self.enable_lsp_button = None;
            return;
        };

        if let Some(enable_button) = &self.enable_lsp_button {
            enable_button.update(ctx, |button, ctx| {
                button.set_label(label, ctx);
            });
        } else {
            self.enable_lsp_button = Some(ctx.add_typed_action_view(|_ctx| {
                ActionButton::new(label, NakedTheme)
                    .with_size(ButtonSize::Small)
                    .on_click(|ctx| {
                        ctx.dispatch_typed_action(CodeFooterViewAction::EnableLSP);
                    })
            }));
        }
    }

    /// Subscribes to a single server's events and adds it. Used in SingleFile mode.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub fn subscribe_to_server_events(
        &mut self,
        lsp_server: &ModelHandle<LspServerModel>,
        ctx: &mut ViewContext<Self>,
    ) {
        ctx.subscribe_to_model(lsp_server, |_, _, _, ctx| {
            ctx.notify();
        });

        self.lsp_servers = vec![lsp_server.downgrade()];
        self.subscribed_server_ids = vec![lsp_server.as_ref(ctx).id()];
        if let FooterMode::SingleFile {
            lsp_repo_status, ..
        } = &mut self.mode
        {
            *lsp_repo_status = LspRepoStatus::Ready;
        }

        self.lsp_status_button.update(ctx, |button, ctx| {
            button.set_disabled(false, ctx);
        });
        ctx.notify();
    }

    /// Clears the server subscription when a server is removed from the manager.
    /// This resets the footer to the "no server" state and kicks off installation detection.
    /// Used in SingleFile mode.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub fn clear_server_subscription(&mut self, ctx: &mut ViewContext<Self>) {
        self.lsp_servers.clear();
        self.subscribed_server_ids.clear();
        self.is_lsp_menu_open = false;

        self.lsp_status_button.update(ctx, |button, ctx| {
            button.set_disabled(true, ctx);
        });

        // Set initial status and kick off installation status detection
        #[cfg(feature = "local_fs")]
        if let FooterMode::SingleFile {
            path,
            lsp_repo_status,
            ..
        } = &mut self.mode
        {
            *lsp_repo_status = Self::detect_installation_status(path, ctx);
            self.update_enable_button_label(ctx);
        }
        #[cfg(not(feature = "local_fs"))]
        if let FooterMode::SingleFile {
            lsp_repo_status, ..
        } = &mut self.mode
        {
            *lsp_repo_status = LspRepoStatus::CheckingForInstallation;
        }

        ctx.notify();
    }

    fn render_indicator(
        lsp_icon: Box<dyn Element>,
        color: ColorU,
        background_color: ColorU,
    ) -> Box<dyn Element> {
        let circle = Container::new(Empty::new().finish())
            .with_background(Fill::Solid(color))
            .with_corner_radius(CornerRadius::with_all(Radius::Percentage(50.)))
            .with_border(Border::all(1.5).with_border_fill(background_color))
            .finish();

        let inner_size = INDICATOR_SIZE;
        let inner_circle_constrained = ConstrainedBox::new(circle)
            .with_width(inner_size)
            .with_height(inner_size)
            .finish();

        let mut stack = Stack::new();
        stack.add_child(lsp_icon);

        let badge_offset = vec2f(-2., 2.);
        stack.add_positioned_child(
            inner_circle_constrained,
            OffsetPositioning::offset_from_parent(
                badge_offset,
                ParentOffsetBounds::Unbounded,
                ParentAnchor::TopRight,
                ChildAnchor::Center,
            ),
        );

        stack.finish()
    }

    fn render_menu_title(
        title: String,
        background: ThemeFill,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        appearance
            .ui_builder()
            .span(title)
            .with_style(UiComponentStyles {
                font_color: Some(appearance.theme().disabled_text_color(background).into()),
                font_size: Some(12.),
                ..Default::default()
            })
            .build()
            .with_vertical_padding(4.)
            .with_horizontal_padding(16.)
            .finish()
    }

    fn render_server_row(server: &LspServerModel, appearance: &Appearance) -> Box<dyn Element> {
        let render_status = LSPServerRenderStatus::render_status(Some(server));
        let failed_error = match server.state() {
            LspModelState::Failed { error } => Some(error.clone()),
            _ => None,
        };

        let background = appearance.theme().surface_2();
        let mut text_col = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_child(
                appearance
                    .ui_builder()
                    .span(server.server_name())
                    .build()
                    .finish(),
            );

        if let Some(error) = failed_error.filter(|e| !e.trim().is_empty()) {
            text_col.add_child(
                appearance
                    .ui_builder()
                    .span(error)
                    .with_style(UiComponentStyles {
                        font_color: Some(
                            appearance.theme().sub_text_color(background).into_solid(),
                        ),
                        ..Default::default()
                    })
                    .build()
                    .finish(),
            );
        }

        Flex::row()
            .with_child(
                Container::new(
                    ConstrainedBox::new(
                        Container::new(Empty::new().finish())
                            .with_background(Fill::Solid(
                                render_status.to_icon_color(appearance.theme()),
                            ))
                            .with_uniform_margin(2.)
                            .with_corner_radius(CornerRadius::with_all(Radius::Percentage(50.)))
                            .finish(),
                    )
                    .with_width(14.)
                    .with_height(14.)
                    .finish(),
                )
                .with_padding_right(8.)
                .finish(),
            )
            .with_child(Shrinkable::new(1., text_col.finish()).finish())
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_main_axis_size(MainAxisSize::Max)
            .finish()
    }

    fn render_menu_separator(appearance: &Appearance) -> Box<dyn Element> {
        Container::new(
            ConstrainedBox::new(
                Rect::new()
                    .with_background_color(blended_colors::neutral_4(appearance.theme()))
                    .finish(),
            )
            .with_height(1.)
            .finish(),
        )
        .with_vertical_padding(4.)
        .finish()
    }

    fn wrap_menu_in_dismiss(col: Flex, appearance: &Appearance) -> Box<dyn Element> {
        let background = appearance.theme().surface_2();
        Dismiss::new(
            ConstrainedBox::new(
                Container::new(col.finish())
                    .with_vertical_padding(8.)
                    .with_background(background)
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
                    .with_border(
                        Border::all(1.)
                            .with_border_color(internal_colors::neutral_4(appearance.theme())),
                    )
                    .finish(),
            )
            .with_width(320.)
            .finish(),
        )
        .on_dismiss(|ctx, _app| ctx.dispatch_typed_action(CodeFooterViewAction::CloseMenu))
        .prevent_interaction_with_other_elements()
        .finish()
    }

    /// Renders the menu for SingleFile mode (single server detail + per-server actions).
    fn render_single_server_menu(
        &self,
        model: &LspServerModel,
        mouse_states: &SingleFileMouseStates,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let title = PersistedWorkspace::as_ref(app)
            .root_for_workspace(self.mode.path())
            .and_then(|path| path.file_name())
            .and_then(|directory_name| directory_name.to_str().map(|s| s.to_string()))
            .unwrap_or("unknown workspace".to_string());

        let background = appearance.theme().surface_2();

        let mut col = Flex::column()
            .with_child(Self::render_menu_title(title, background, appearance))
            .with_child(
                Container::new(Self::render_server_row(model, appearance))
                    .with_vertical_padding(4.)
                    .with_horizontal_padding(16.)
                    .finish(),
            );

        // Add separator and action items based on server status
        let render_status = LSPServerRenderStatus::render_status(Some(model));
        let has_actions = matches!(
            render_status,
            LSPServerRenderStatus::Available
                | LSPServerRenderStatus::Stopped
                | LSPServerRenderStatus::Failed
        );
        if has_actions {
            col.add_child(Self::render_menu_separator(appearance));
        }

        if matches!(render_status, LSPServerRenderStatus::Available) {
            col.add_child(Self::render_open_logs_menu_item(mouse_states, appearance));
            col.add_child(Self::render_restart_server_menu_item(
                mouse_states,
                appearance,
            ));
            col.add_child(Self::render_stop_server_menu_item(mouse_states, appearance));
        }

        if matches!(render_status, LSPServerRenderStatus::Stopped) {
            col.add_child(Self::render_start_server_menu_item(
                mouse_states,
                appearance,
            ));
        }

        if matches!(render_status, LSPServerRenderStatus::Failed) {
            col.add_child(Self::render_open_logs_menu_item(mouse_states, appearance));
            col.add_child(Self::render_restart_server_menu_item(
                mouse_states,
                appearance,
            ));
            col.add_child(Self::render_remove_server_menu_item(
                mouse_states,
                appearance,
            ));
        }

        Self::wrap_menu_in_dismiss(col, appearance)
    }

    /// Renders the menu for Workspace mode (multi-server list + global actions).
    fn render_workspace_menu(
        &self,
        mouse_states: &WorkspaceMouseStates,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let title = self
            .mode
            .path()
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown workspace")
            .to_string();

        let background = appearance.theme().surface_2();

        let mut col =
            Flex::column().with_child(Self::render_menu_title(title, background, appearance));

        // Render a row per server
        let mut has_running = false;
        let mut has_stopped = false;
        let mut server_count: usize = 0;
        for weak in &self.lsp_servers {
            let Some(server) = weak.upgrade(app) else {
                continue;
            };
            server_count += 1;
            let server_ref = server.as_ref(app);
            match LSPServerRenderStatus::render_status(Some(server_ref)) {
                LSPServerRenderStatus::Available | LSPServerRenderStatus::Busy => {
                    has_running = true;
                }
                LSPServerRenderStatus::Stopped | LSPServerRenderStatus::Failed => {
                    has_stopped = true;
                }
            }
            col.add_child(
                Container::new(Self::render_server_row(server_ref, appearance))
                    .with_vertical_padding(4.)
                    .with_horizontal_padding(16.)
                    .finish(),
            );
        }

        // Separator + conditional global actions based on server states
        col.add_child(Self::render_menu_separator(appearance));
        let is_plural = server_count > 1;
        if has_running {
            col.add_child(Self::render_restart_all_servers_menu_item(
                mouse_states,
                is_plural,
                appearance,
            ));
            col.add_child(Self::render_stop_all_servers_menu_item(
                mouse_states,
                is_plural,
                appearance,
            ));
        }
        if has_stopped {
            col.add_child(Self::render_start_all_servers_menu_item(
                mouse_states,
                has_running,
                is_plural,
                appearance,
            ));
        }
        col.add_child(Self::render_manage_servers_menu_item(
            mouse_states,
            appearance,
        ));

        Self::wrap_menu_in_dismiss(col, appearance)
    }

    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    /// Generic rendering function for each action row in the menu. Note that we need to take in a closure here
    /// as Hoverable needs to dynamically construct its inner element.
    fn render_menu_item<F: 'static + Fn() -> Box<dyn Element>>(
        appearance: &Appearance,
        mouse_state: MouseStateHandle,
        icon_creator: F,
        label: &'static str,
        action: CodeFooterViewAction,
    ) -> Box<dyn Element> {
        let theme = appearance.theme().clone();
        let background = theme.surface_2();
        let ui_builder = appearance.ui_builder().clone();

        Hoverable::new(mouse_state, move |state| {
            let is_hovered = state.is_hovered();
            let text_color = theme.main_text_color(background).into();

            let icon_size = 14.;
            let icon = ConstrainedBox::new(icon_creator())
                .with_width(icon_size)
                .with_height(icon_size)
                .finish();

            let label_element = ui_builder
                .span(label)
                .with_style(UiComponentStyles {
                    font_color: Some(text_color),
                    ..Default::default()
                })
                .build()
                .finish();

            let row = Flex::row()
                .with_child(Container::new(icon).with_padding_right(8.).finish())
                .with_child(label_element)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_main_axis_size(MainAxisSize::Max)
                .finish();

            let hover_background = if is_hovered {
                Some(Fill::Solid(internal_colors::neutral_4(&theme)))
            } else {
                None
            };

            let container = Container::new(row)
                .with_horizontal_padding(16.)
                .with_vertical_padding(4.);

            let container = if let Some(bg) = hover_background {
                container.with_background(bg)
            } else {
                container
            };

            container.finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(action.clone());
        })
        .finish()
    }

    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    fn render_open_logs_menu_item(
        mouse_states: &SingleFileMouseStates,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let background = theme.surface_2();
        let text_color: ColorU = theme.main_text_color(background).into();

        Self::render_menu_item(
            appearance,
            mouse_states.open_logs.clone(),
            move || {
                Icon::Code1
                    .to_warpui_icon(ThemeFill::Solid(text_color))
                    .finish()
            },
            "Open logs",
            CodeFooterViewAction::OpenLogs,
        )
    }

    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    fn render_restart_server_menu_item(
        mouse_states: &SingleFileMouseStates,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let background = theme.surface_2();
        let text_color: ColorU = theme.main_text_color(background).into();

        Self::render_menu_item(
            appearance,
            mouse_states.restart_server.clone(),
            move || {
                Icon::RefreshCcw
                    .to_warpui_icon(ThemeFill::Solid(text_color))
                    .finish()
            },
            "Restart server",
            CodeFooterViewAction::RestartServer,
        )
    }

    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    fn render_stop_server_menu_item(
        mouse_states: &SingleFileMouseStates,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let stop_color: ColorU = AnsiColorIdentifier::Red
            .to_ansi_color(&theme.terminal_colors().bright)
            .into();

        Self::render_menu_item(
            appearance,
            mouse_states.stop_server.clone(),
            move || {
                Container::new(Rect::new().with_background_color(stop_color).finish())
                    .with_uniform_padding(2.)
                    .finish()
            },
            "Stop server",
            CodeFooterViewAction::StopServer,
        )
    }

    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    fn render_start_server_menu_item(
        mouse_states: &SingleFileMouseStates,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let background = theme.surface_2();
        let text_color: ColorU = theme.main_text_color(background).into();

        Self::render_menu_item(
            appearance,
            mouse_states.start_server.clone(),
            move || {
                Icon::Play
                    .to_warpui_icon(ThemeFill::Solid(text_color))
                    .finish()
            },
            "Start server",
            CodeFooterViewAction::StartServer,
        )
    }

    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    fn render_remove_server_menu_item(
        mouse_states: &SingleFileMouseStates,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let background = theme.surface_2();
        let text_color: ColorU = theme.main_text_color(background).into();

        Self::render_menu_item(
            appearance,
            mouse_states.remove_server.clone(),
            move || {
                Icon::Trash
                    .to_warpui_icon(ThemeFill::Solid(text_color))
                    .finish()
            },
            "Remove server",
            CodeFooterViewAction::RemoveServer,
        )
    }

    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    fn render_restart_all_servers_menu_item(
        mouse_states: &WorkspaceMouseStates,
        is_plural: bool,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let background = theme.surface_2();
        let text_color: ColorU = theme.main_text_color(background).into();

        Self::render_menu_item(
            appearance,
            mouse_states.restart_all.clone(),
            move || {
                Icon::RefreshCcw
                    .to_warpui_icon(ThemeFill::Solid(text_color))
                    .finish()
            },
            if is_plural {
                "Restart all servers"
            } else {
                "Restart server"
            },
            CodeFooterViewAction::RestartAllServers,
        )
    }

    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    fn render_stop_all_servers_menu_item(
        mouse_states: &WorkspaceMouseStates,
        is_plural: bool,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let stop_color: ColorU = AnsiColorIdentifier::Red
            .to_ansi_color(&theme.terminal_colors().bright)
            .into();

        Self::render_menu_item(
            appearance,
            mouse_states.stop_all.clone(),
            move || {
                Container::new(Rect::new().with_background_color(stop_color).finish())
                    .with_uniform_padding(2.)
                    .finish()
            },
            if is_plural {
                "Stop all servers"
            } else {
                "Stop server"
            },
            CodeFooterViewAction::StopAllServers,
        )
    }

    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    fn render_start_all_servers_menu_item(
        mouse_states: &WorkspaceMouseStates,
        has_running: bool,
        is_plural: bool,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let background = theme.surface_2();
        let text_color: ColorU = theme.main_text_color(background).into();

        Self::render_menu_item(
            appearance,
            mouse_states.start_all.clone(),
            move || {
                Icon::Play
                    .to_warpui_icon(ThemeFill::Solid(text_color))
                    .finish()
            },
            if !is_plural {
                "Start server"
            } else if has_running {
                "Start all stopped servers"
            } else {
                "Start all servers"
            },
            CodeFooterViewAction::StartAllServers,
        )
    }

    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    fn render_manage_servers_menu_item(
        mouse_states: &WorkspaceMouseStates,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let background = theme.surface_2();
        let text_color: ColorU = theme.main_text_color(background).into();

        Self::render_menu_item(
            appearance,
            mouse_states.manage_servers.clone(),
            move || {
                Icon::Gear
                    .to_warpui_icon(ThemeFill::Solid(text_color))
                    .finish()
            },
            "Manage servers",
            CodeFooterViewAction::ManageServers,
        )
    }

    /// Computes the aggregate indicator color across all tracked servers.
    /// Priority: Failed > Busy > Stopped > Available.
    fn aggregate_indicator_color(&self, theme: &WarpTheme, app: &AppContext) -> ColorU {
        if self.lsp_servers.is_empty() {
            return LSPServerRenderStatus::Stopped.to_icon_color(theme);
        }

        let mut worst = LSPServerRenderStatus::Available;
        for weak in &self.lsp_servers {
            let Some(server) = weak.upgrade(app) else {
                continue;
            };
            let status = LSPServerRenderStatus::render_status(Some(server.as_ref(app)));
            worst = match (&worst, &status) {
                (_, LSPServerRenderStatus::Failed) => LSPServerRenderStatus::Failed,
                (LSPServerRenderStatus::Failed, _) => LSPServerRenderStatus::Failed,
                (_, LSPServerRenderStatus::Busy) => LSPServerRenderStatus::Busy,
                (LSPServerRenderStatus::Busy, _) => LSPServerRenderStatus::Busy,
                (_, LSPServerRenderStatus::Stopped) => LSPServerRenderStatus::Stopped,
                (LSPServerRenderStatus::Stopped, _) => LSPServerRenderStatus::Stopped,
                _ => LSPServerRenderStatus::Available,
            };
        }
        worst.to_icon_color(theme)
    }

    /// Returns strong handles for all servers that can still be upgraded.
    fn live_servers(&self, app: &AppContext) -> Vec<ModelHandle<LspServerModel>> {
        self.lsp_servers
            .iter()
            .filter_map(|weak| weak.upgrade(app))
            .collect()
    }

    fn render_lsp_icon(&self, appearance: &Appearance, app: &AppContext) -> Box<dyn Element> {
        if self.is_tab_config_footer() {
            return Empty::new().finish();
        }
        let theme = appearance.theme();
        let lsp_icon = ChildView::new(&self.lsp_status_button).finish();
        let background_color = theme.background().into_solid();

        let indicator_color = self.aggregate_indicator_color(theme, app);
        let indicator = Self::render_indicator(lsp_icon, indicator_color, background_color);

        let live = self.live_servers(app);
        if !self.is_lsp_menu_open || live.is_empty() {
            return indicator;
        }

        let menu = match &self.mode {
            FooterMode::TabConfig { .. } => return indicator,
            FooterMode::SingleFile { mouse_states, .. } => {
                let server = live[0].as_ref(app);
                self.render_single_server_menu(server, mouse_states, appearance, app)
            }
            FooterMode::Workspace { mouse_states, .. } => {
                self.render_workspace_menu(mouse_states, appearance, app)
            }
        };

        let mut element = Stack::new().with_child(indicator);
        element.add_positioned_child(
            menu,
            OffsetPositioning::offset_from_parent(
                vec2f(0., -2.),
                ParentOffsetBounds::WindowByPosition,
                ParentAnchor::TopLeft,
                ChildAnchor::BottomLeft,
            ),
        );
        element.finish()
    }

    fn render_status_text(
        theme: &WarpTheme,
        appearance: &Appearance,
        message: String,
    ) -> Box<dyn Element> {
        let status_content = appearance
            .ui_builder()
            .span(message)
            .with_style(UiComponentStyles {
                font_family_id: Some(appearance.ui_font_family()),
                font_color: Some(internal_colors::text_sub(theme, theme.background())),
                font_size: Some(12.0),
                ..Default::default()
            })
            .build()
            .finish();

        // Left margin only to separate from the icon; right margin removed to tighten
        // spacing between status text and the enable button action
        Container::new(status_content)
            .with_margin_left(ICON_MARGIN)
            .finish()
    }

    /// Returns a status message for a single server, if any.
    fn server_status_message(server: &LspServerModel) -> Option<String> {
        match server.state() {
            LspModelState::Available { .. } | LspModelState::Starting => server
                .latest_progress_update()
                .map(|update| update.to_display_message())
                .filter(|msg| !msg.trim().is_empty())
                .map(|msg| format!("{}: {msg}", server.server_name())),
            LspModelState::Stopped { .. } | LspModelState::Stopping { .. } => {
                Some(format!("{}: stopped", server.server_name()))
            }
            LspModelState::Failed { .. } => Some(format!("{}: error", server.server_name())),
        }
    }

    /// Returns the CTA message and button flag for workspace mode if any servers
    /// are disabled and need user action. Returns `None` if not in workspace mode
    /// or no CTA is needed.
    fn workspace_cta_message(&self) -> Option<(Option<String>, bool)> {
        let FooterMode::Workspace {
            root_path,
            lsp_repo_statuses,
            ..
        } = &self.mode
        else {
            return None;
        };

        if lsp_repo_statuses.is_empty() {
            return None;
        }

        let has_cta = lsp_repo_statuses.values().any(|s| {
            matches!(
                s,
                LspRepoStatus::DisabledAndInstalled { .. }
                    | LspRepoStatus::DisabledAndNotInstalled { .. }
            )
        });

        if has_cta {
            let root_name = root_path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("this workspace");
            Some((
                Some(format!(
                    "Language support is not currently enabled for {root_name}"
                )),
                true,
            ))
        } else {
            None
        }
    }

    /// Computes the aggregated status message across all servers.
    /// Priority: failed error > starting/progress text > stopped text.
    fn compute_status_message(&self, app: &AppContext) -> (Option<String>, bool) {
        if self.is_tab_config_footer() {
            return (None, false);
        }
        let live = self.live_servers(app);
        if !live.is_empty() {
            // Check for any failed server first
            for server in &live {
                let server_ref = server.as_ref(app);
                if let LspModelState::Failed { error } = server_ref.state() {
                    return (
                        Some(format!("{}: {error}", server_ref.server_name())),
                        false,
                    );
                }
            }
            // Then check for any starting/busy server
            for server in &live {
                let server_ref = server.as_ref(app);
                if let Some(msg) = Self::server_status_message(server_ref) {
                    if matches!(
                        server_ref.state(),
                        LspModelState::Starting | LspModelState::Available { .. }
                    ) {
                        return (Some(msg), false);
                    }
                }
            }
            // Then check stopped
            for server in &live {
                let server_ref = server.as_ref(app);
                if matches!(
                    server_ref.state(),
                    LspModelState::Stopped { .. } | LspModelState::Stopping { .. }
                ) {
                    return (
                        Some(format!("{}: stopped", server_ref.server_name())),
                        false,
                    );
                }
            }
            // All servers are available with no progress — but in workspace mode,
            // there may still be disabled servers that need a CTA.
            if let Some(cta) = self.workspace_cta_message() {
                return cta;
            }
            return (None, false);
        }

        // No servers — show enablement CTA based on mode
        match &self.mode {
            FooterMode::TabConfig { .. } => (None, false),
            FooterMode::SingleFile {
                path,
                lsp_repo_status,
                ..
            } => match PersistedWorkspace::as_ref(app).has_enabled_lsp_server_for_file_path(path) {
                LSPEnablementResultForFile::UnsupportedLanguage => (
                    Some("Language support is unavailable for this file type".to_string()),
                    false,
                ),
                LSPEnablementResultForFile::LSPNotEnabled { root_name } => match lsp_repo_status {
                    LspRepoStatus::CheckingForInstallation => (
                        Some(format!(
                            "Language support is not currently enabled for {}",
                            root_name.unwrap_or("this codebase".to_string())
                        )),
                        false,
                    ),
                    LspRepoStatus::Ready | LspRepoStatus::Enabled => (
                        Some("Language server is unavailable for this codebase".to_string()),
                        false,
                    ),
                    LspRepoStatus::DisabledAndNotInstalled { .. }
                    | LspRepoStatus::DisabledAndInstalled { .. } => (
                        Some(format!(
                            "Language support is not currently enabled for {}",
                            root_name.unwrap_or("this codebase".to_string())
                        )),
                        true,
                    ),
                    LspRepoStatus::Installing { server_type } => (
                        Some(format!("Installing {}...", server_type.binary_name())),
                        false,
                    ),
                },
                LSPEnablementResultForFile::Enabled => (None, false),
            },
            FooterMode::Workspace {
                root_path,
                lsp_repo_statuses,
                ..
            } => {
                if lsp_repo_statuses.is_empty() {
                    // Still waiting for async detection
                    return (None, false);
                }

                let root_name = root_path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("this workspace");

                // Check if any server has a CTA-worthy status
                if let Some(cta) = self.workspace_cta_message() {
                    return cta;
                }

                // Check if any server is installing
                for status in lsp_repo_statuses.values() {
                    if let LspRepoStatus::Installing { server_type } = status {
                        return (
                            Some(format!("Installing {}...", server_type.binary_name())),
                            false,
                        );
                    }
                }

                // Check if all are still checking
                let all_checking = lsp_repo_statuses
                    .values()
                    .all(|s| matches!(s, LspRepoStatus::CheckingForInstallation));
                if all_checking {
                    return (None, false);
                }

                // All servers are enabled/ready but no live servers — unavailable
                (
                    Some(format!("Language support is unavailable for {root_name}")),
                    false,
                )
            }
        }
    }
}

#[derive(Clone)]
#[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
pub enum CodeFooterViewEvent {
    RunTabConfigSkill {
        path: PathBuf,
    },
    EnableLSP {
        path: PathBuf,
        server_type: Option<LSPServerType>,
    },
    InstallAndEnableLSP {
        path: PathBuf,
        server_type: Option<LSPServerType>,
    },
    OpenLogs {
        path: PathBuf,
    },
    RestartServer {
        server: ModelHandle<LspServerModel>,
    },
    StopServer {
        server: ModelHandle<LspServerModel>,
    },
    StartServer {
        server: ModelHandle<LspServerModel>,
    },
    RestartAllServers {
        servers: Vec<ModelHandle<LspServerModel>>,
    },
    StopAllServers {
        servers: Vec<ModelHandle<LspServerModel>>,
    },
    StartAllServers {
        servers: Vec<ModelHandle<LspServerModel>>,
    },
    ManageServers,
}

impl Entity for CodeFooterView {
    type Event = CodeFooterViewEvent;
}

impl View for CodeFooterView {
    fn ui_name() -> &'static str {
        "CodeFooterView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let mut footer_content = Flex::row()
            .with_main_axis_alignment(MainAxisAlignment::Start)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Max);

        if self.is_tab_config_footer() {
            footer_content.add_child(Self::render_tab_config_info_icon(theme));
            footer_content.add_child(
                Shrinkable::new(
                    1.,
                    Self::render_status_text(
                        theme,
                        appearance,
                        "Use Oz to update this config".to_string(),
                    ),
                )
                .finish(),
            );
            if let Some(tab_config_skill_button) = &self.tab_config_skill_button {
                footer_content.add_child(
                    Container::new(ChildView::new(tab_config_skill_button).finish())
                        .with_margin_left(ICON_MARGIN)
                        .finish(),
                );
            }
        } else {
            footer_content.add_child(self.render_lsp_icon(appearance, app));

            let (status_message, should_show_enable_button) = self.compute_status_message(app);

            if let Some(status_message) = status_message {
                footer_content.add_child(
                    Shrinkable::new(
                        1.,
                        Self::render_status_text(theme, appearance, status_message),
                    )
                    .finish(),
                );
            }

            if should_show_enable_button {
                if let Some(enable_lsp) = &self.enable_lsp_button {
                    // Left margin only to separate from status text; right margin removed
                    // to tighten padding between elements
                    footer_content.add_child(
                        Container::new(ChildView::new(enable_lsp).finish())
                            .with_margin_left(ICON_MARGIN)
                            .finish(),
                    );
                }
            }
        }

        let mut container = Container::new(
            ConstrainedBox::new(
                Container::new(footer_content.finish())
                    .with_padding(Padding::uniform(4.0))
                    .finish(),
            )
            .with_height(FOOTER_HEIGHT)
            .finish(),
        )
        .with_background_color(theme.background().into());

        if self.show_border {
            container = container
                .with_border(Border::top(2.0).with_border_fill(internal_colors::neutral_2(theme)))
                .with_corner_radius(CornerRadius::with_bottom(Radius::Pixels(8.)));
        }

        container.finish()
    }
}

impl TypedActionView for CodeFooterView {
    type Action = CodeFooterViewAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            CodeFooterViewAction::ToggleMenu => {
                self.is_lsp_menu_open = !self.is_lsp_menu_open;
                ctx.notify();
            }
            CodeFooterViewAction::CloseMenu => {
                self.is_lsp_menu_open = false;
                ctx.notify();
            }
            CodeFooterViewAction::RunTabConfigSkill => {
                let FooterMode::TabConfig { path } = &self.mode else {
                    return;
                };
                ctx.emit(CodeFooterViewEvent::RunTabConfigSkill { path: path.clone() });
            }
            CodeFooterViewAction::EnableLSP => {
                let path = self.mode.path().to_path_buf();
                let cta_statuses: Vec<LspRepoStatus> = self
                    .mode
                    .cta_lsp_repo_statuses()
                    .into_iter()
                    .cloned()
                    .collect();

                for status in &cta_statuses {
                    let needed_install =
                        matches!(status, LspRepoStatus::DisabledAndNotInstalled { .. });
                    let server_type = match status {
                        LspRepoStatus::DisabledAndInstalled { server_type }
                        | LspRepoStatus::DisabledAndNotInstalled { server_type } => {
                            Some(*server_type)
                        }
                        _ => None,
                    };

                    if let Some(st) = server_type {
                        send_telemetry_from_ctx!(
                            LspTelemetryEvent::ServerEnabled {
                                server_type: st.binary_name().to_string(),
                                source: LspEnablementSource::FooterButton,
                                needed_install,
                            },
                            ctx
                        );
                    }

                    if needed_install {
                        ctx.emit(CodeFooterViewEvent::InstallAndEnableLSP {
                            path: path.clone(),
                            server_type,
                        });
                    } else {
                        ctx.emit(CodeFooterViewEvent::EnableLSP {
                            path: path.clone(),
                            server_type,
                        });
                    }
                }
            }
            CodeFooterViewAction::OpenLogs => {
                self.is_lsp_menu_open = false;
                let server_name = self
                    .lsp_servers
                    .first()
                    .and_then(|w| w.upgrade(ctx))
                    .map(|s| s.as_ref(ctx).server_name());
                send_telemetry_from_ctx!(
                    LspTelemetryEvent::ControlAction {
                        action: LspControlActionType::OpenLogs,
                        server_type: server_name,
                    },
                    ctx
                );
                ctx.emit(CodeFooterViewEvent::OpenLogs {
                    path: self.mode.path().to_path_buf(),
                });
                ctx.notify();
            }
            CodeFooterViewAction::RestartServer => {
                self.is_lsp_menu_open = false;
                if let Some(server) = self.lsp_servers.first().and_then(|w| w.upgrade(ctx)) {
                    send_telemetry_from_ctx!(
                        LspTelemetryEvent::ControlAction {
                            action: LspControlActionType::Restart,
                            server_type: Some(server.as_ref(ctx).server_name()),
                        },
                        ctx
                    );
                    ctx.emit(CodeFooterViewEvent::RestartServer {
                        server: server.clone(),
                    });
                }
                ctx.notify();
            }
            CodeFooterViewAction::StopServer => {
                self.is_lsp_menu_open = false;
                if let Some(server) = self.lsp_servers.first().and_then(|w| w.upgrade(ctx)) {
                    send_telemetry_from_ctx!(
                        LspTelemetryEvent::ControlAction {
                            action: LspControlActionType::Stop,
                            server_type: Some(server.as_ref(ctx).server_name()),
                        },
                        ctx
                    );
                    ctx.emit(CodeFooterViewEvent::StopServer {
                        server: server.clone(),
                    });
                }
                ctx.notify();
            }
            CodeFooterViewAction::StartServer => {
                self.is_lsp_menu_open = false;
                if let Some(server) = self.lsp_servers.first().and_then(|w| w.upgrade(ctx)) {
                    send_telemetry_from_ctx!(
                        LspTelemetryEvent::ControlAction {
                            action: LspControlActionType::Start,
                            server_type: Some(server.as_ref(ctx).server_name()),
                        },
                        ctx
                    );
                    ctx.emit(CodeFooterViewEvent::StartServer {
                        server: server.clone(),
                    });
                }
                ctx.notify();
            }
            CodeFooterViewAction::RemoveServer => {
                self.is_lsp_menu_open = false;
                if let Some(server) = self.lsp_servers.first().and_then(|w| w.upgrade(ctx)) {
                    let workspace_root = server.as_ref(ctx).initial_workspace().to_path_buf();
                    let server_type = server.as_ref(ctx).server_type();

                    send_telemetry_from_ctx!(
                        LspTelemetryEvent::ServerRemoved {
                            server_type: server_type.binary_name().to_string(),
                            source: LspEnablementSource::FooterButton,
                        },
                        ctx
                    );

                    // Remove from manager (stops and removes)
                    LspManagerModel::handle(ctx).update(ctx, |manager, ctx| {
                        manager.remove_server(&workspace_root, server_type, ctx);
                    });

                    // Disable in PersistedWorkspace
                    PersistedWorkspace::handle(ctx).update(ctx, |workspace, _| {
                        workspace.disable_lsp_server_for_path(&workspace_root, server_type);
                    });
                }
                ctx.notify();
            }
            CodeFooterViewAction::RestartAllServers => {
                self.is_lsp_menu_open = false;
                send_telemetry_from_ctx!(
                    LspTelemetryEvent::ControlAction {
                        action: LspControlActionType::RestartAll,
                        server_type: None,
                    },
                    ctx
                );
                let live = self.live_servers(ctx);
                ctx.emit(CodeFooterViewEvent::RestartAllServers { servers: live });
                ctx.notify();
            }
            CodeFooterViewAction::StopAllServers => {
                self.is_lsp_menu_open = false;
                send_telemetry_from_ctx!(
                    LspTelemetryEvent::ControlAction {
                        action: LspControlActionType::StopAll,
                        server_type: None,
                    },
                    ctx
                );
                let live = self.live_servers(ctx);
                ctx.emit(CodeFooterViewEvent::StopAllServers { servers: live });
                ctx.notify();
            }
            CodeFooterViewAction::StartAllServers => {
                self.is_lsp_menu_open = false;
                let live = self.live_servers(ctx);
                ctx.emit(CodeFooterViewEvent::StartAllServers { servers: live });
                ctx.notify();
            }
            CodeFooterViewAction::ManageServers => {
                self.is_lsp_menu_open = false;
                ctx.emit(CodeFooterViewEvent::ManageServers);
                ctx.notify();
            }
        }
    }
}
