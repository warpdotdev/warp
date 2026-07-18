use std::cmp::Ordering;
use std::collections::HashMap;
use std::path::PathBuf;

use markdown_parser::{FormattedText, FormattedTextFragment, FormattedTextLine};
use settings::ToggleableSetting as _;
use strum::IntoEnumIterator;
use uuid::Uuid;
use warp_core::features::FeatureFlag;
use warp_core::send_telemetry_from_ctx;
use warp_core::ui::theme::color::internal_colors;
use warp_core::ui::Icon;
use warp_errors::report_error;
use warpui::elements::{
    Align, Border, ChildView, ConstrainedBox, Container, CrossAxisAlignment, Expanded, Fill, Flex,
    FormattedTextElement, HighlightedHyperlink, MainAxisAlignment, MainAxisSize, ParentElement,
    Text,
};
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::ui_components::switch::SwitchStateHandle;
use warpui::{
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};

use crate::ai::mcp::gallery::MCPGalleryManagerEvent;
use crate::ai::mcp::templatable::{GalleryData, TemplatableMCPServer};
use crate::ai::mcp::templatable_manager::{
    TemplatableMCPServerManager, TemplatableMCPServerManagerEvent,
};
#[cfg(feature = "local_fs")]
use crate::ai::mcp::{
    // Import events for file-based manager and watcher conditionally
    // since their WASM variants don't export events.
    file_based_manager::FileBasedMCPManagerEvent,
    FileMCPWatcher,
    FileMCPWatcherEvent,
};
use crate::ai::mcp::{
    logs, FileBasedMCPManager, MCPGalleryManager, MCPProvider, MCPServerUpdate,
    TemplatableMCPServerInstallation,
};
use crate::appearance::Appearance;
use crate::cloud_object::model::persistence::{CloudModel, CloudModelEvent};
use crate::cloud_object::{GenericStringObjectFormat, JsonObjectType};
use crate::drive::CloudObjectTypeAndId;
use crate::editor::{
    EditorView, PropagateAndNoOpNavigationKeys, SingleLineEditorOptions, TextOptions,
};
use crate::modal::{Modal, ModalEvent, ModalViewState};
use crate::pane_group::Direction;
use crate::search_bar::SearchBar;
use crate::server::telemetry::{MCPTemplateInstallationSource, TelemetryEvent};
use crate::settings::{AISettings, AISettingsChangedEvent};
use crate::settings_view::mcp_servers::server_card::{
    ServerCardEvent, ServerCardOptions, ServerCardStatus, ServerCardView, TitleChip,
};
use crate::settings_view::mcp_servers::update_modal::{UpdateModalBody, UpdateModalBodyEvent};
use crate::settings_view::mcp_servers::{style, ServerCardItemId};
use crate::settings_view::mcp_servers_page::InstallOrigin;
use crate::settings_view::settings_page::{
    build_toggle_element, render_body_item_label, LocalOnlyIconState, ToggleState,
};
use crate::ui_components::blended_colors;
use crate::util::truncation::truncate_from_end;
use crate::view_components::action_button::{ActionButton, NakedTheme};
use crate::view_components::DismissibleToast;
use crate::workflows::local_workflows::tail_command_for_shell;
use crate::workspace::Workspace;
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::ToastStack;

const DESCRIPTION_TEXT: &str = "Add MCP servers to extend the Warp Agent's capabilities. MCP servers expose data sources or tools to agents through a standardized interface, essentially acting like plugins. Add a custom server, or use the presets to get started with popular servers. You can also find team servers that have been shared with you here. ";

#[derive(Debug, Clone)]
pub enum MCPServersListPageViewEvent {
    Add,
    Edit(ServerCardItemId),
    LogOut(ServerCardItemId, String),
    StartInstallation {
        templatable_mcp_server: TemplatableMCPServer,
        instructions_in_markdown: Option<String>,
        /// Where the install request was initiated from. List-page-originated
        /// events are always `InstallOrigin::InApp` because they are emitted in
        /// response to a direct user gesture on the gallery card. See
        /// `specs/GH686/product.md`.
        origin: InstallOrigin,
    },
    ShowModal,
    HideModal,
}

#[derive(Debug, Clone)]
pub enum MCPServersListPageViewAction {
    Add,
    ToggleFileBasedMcp,
}

const EMPTY_STATE_TEXT: &str = "Once you add a MCP server, it will be shown here.";
const NO_SEARCH_RESULTS_TEXT: &str = "No search results found";

pub struct MCPServersListPageView {
    server_cards: HashMap<ServerCardItemId, ViewHandle<ServerCardView>>,
    gallery_server_cards: HashMap<ServerCardItemId, ViewHandle<ServerCardView>>,
    // MCP server cards for uninstalled file-based servers, grouped by provider.
    file_based_template_cards: HashMap<MCPProvider, Vec<ViewHandle<ServerCardView>>>,
    update_modal_state: ModalViewState<Modal<UpdateModalBody>>,
    search_editor: ViewHandle<EditorView>,
    search_bar: ViewHandle<SearchBar>,
    add_button: ViewHandle<ActionButton>,
    file_based_mcp_toggle: SwitchStateHandle,
}

impl MCPServersListPageView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        Self::listen_to_cloud_model_events(ctx);

        // Subscribe to templatable MCP server manager state changes
        let templatable_manager = TemplatableMCPServerManager::handle(ctx);
        ctx.subscribe_to_model(&templatable_manager, |me, _, event, ctx| {
            me.handle_templatable_mcp_manager_event(event, ctx);
        });

        // Subscribe to MCP gallery server manager state changes
        let gallery_manager = MCPGalleryManager::handle(ctx);
        ctx.subscribe_to_model(&gallery_manager, |me, _, event, ctx| {
            me.handle_mcp_gallery_manager_event(event, ctx);
        });

        cfg_if::cfg_if!(
            if #[cfg(feature = "local_fs")] {
                // Refresh cards when active servers are spawned, removed, or logged out.
                let file_based_manager = FileBasedMCPManager::handle(ctx);
                ctx.subscribe_to_model(&file_based_manager, |me, _, event, ctx| match event {
                    FileBasedMCPManagerEvent::SpawnServers { .. }
                    | FileBasedMCPManagerEvent::DespawnServers { .. }
                    | FileBasedMCPManagerEvent::PurgeCredentials { .. } => {
                        // Refresh cards when servers are spawned or removed.
                        me.refresh_file_based_server_cards(ctx);
                    }
                    _ => {}
                });

                // Refresh cards when MCP config files are parsed or removed.
                let file_mcp_watcher = FileMCPWatcher::handle(ctx);
                ctx.subscribe_to_model(&file_mcp_watcher, |me, _, event, ctx| match event {
                    FileMCPWatcherEvent::ConfigParsed { .. }
                    | FileMCPWatcherEvent::ConfigRemoved { .. } => {
                        me.refresh_file_based_server_cards(ctx);
                    }
                    _ => {}
                });
            }
        );

        // Re-render when the file-based MCP enabled setting changes.
        ctx.subscribe_to_model(&AISettings::handle(ctx), |me, _, event, ctx| {
            if matches!(event, AISettingsChangedEvent::FileBasedMcpEnabled { .. }) {
                me.refresh_file_based_server_cards(ctx);
            }
        });

        let appearance = Appearance::handle(ctx);

        let update_modal_body = ctx.add_typed_action_view(UpdateModalBody::new);
        ctx.subscribe_to_view(&update_modal_body, |me, _, event, ctx| {
            me.handle_update_modal_body_event(event, ctx);
        });

        let update_modal = ctx.add_typed_action_view(|ctx| {
            Modal::new(None, update_modal_body, ctx).with_body_style(UiComponentStyles {
                padding: Some(Coords::uniform(0.)),
                ..Default::default()
            })
        });
        ctx.subscribe_to_view(&update_modal, |me, _, event, ctx| {
            me.handle_update_modal_event(event, ctx);
        });

        let update_modal_state = ModalViewState::new(update_modal);

        let gallery_server_cards = Self::create_gallery_server_cards(ctx);
        for server_card in gallery_server_cards.values() {
            ctx.subscribe_to_view(server_card, |me, _, event, ctx| {
                me.handle_server_card_event(event, ctx);
            });
        }

        // Match the standard settings search fields (e.g. the API keys search): use the UI font
        // family at the UI font size. Without an explicit size override the editor falls back to the
        // (larger) monospace size, which made this field render bigger than the others.
        let ui_font_size = appearance.as_ref(ctx).ui_font_size();
        let search_editor_text = TextOptions::ui_text(Some(ui_font_size), appearance.as_ref(ctx));
        let search_editor = {
            let options = SingleLineEditorOptions {
                text: search_editor_text,
                propagate_and_no_op_vertical_navigation_keys:
                    PropagateAndNoOpNavigationKeys::Always,
                ..Default::default()
            };
            ctx.add_typed_action_view(|ctx| EditorView::single_line(options, ctx))
        };
        ctx.subscribe_to_view(&search_editor, move |_, _, _, ctx| {
            ctx.notify();
        });

        search_editor.update(ctx, |editor, ctx| {
            editor.clear_buffer_and_reset_undo_stack(ctx);
            editor.set_placeholder_text("Search MCP Servers", ctx);
        });
        let search_bar = ctx.add_typed_action_view(|_| SearchBar::new(search_editor.clone()));

        let add_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Add", NakedTheme)
                .with_icon(Icon::Plus)
                .on_click(|ctx| ctx.dispatch_typed_action(MCPServersListPageViewAction::Add))
        });

        let mut me = Self {
            server_cards: Default::default(),
            gallery_server_cards,
            file_based_template_cards: Default::default(),
            update_modal_state,
            search_editor,
            search_bar,
            add_button,
            file_based_mcp_toggle: Default::default(),
        };

        me.create_server_cards(ctx);
        me.create_file_based_server_cards(ctx);
        me
    }

    fn listen_to_cloud_model_events(ctx: &mut ViewContext<Self>) {
        let cloud_model = CloudModel::handle(ctx);
        ctx.subscribe_to_model(&cloud_model, |me, _, event, ctx| match event {
            CloudModelEvent::ObjectUpdated {
                type_and_id:
                    CloudObjectTypeAndId::GenericStringObject {
                        object_type: GenericStringObjectFormat::Json(JsonObjectType::MCPServer),
                        id: _,
                    },
                source: _,
            }
            | CloudModelEvent::ObjectTrashed {
                type_and_id:
                    CloudObjectTypeAndId::GenericStringObject {
                        object_type: GenericStringObjectFormat::Json(JsonObjectType::MCPServer),
                        id: _,
                    },
                source: _,
            }
            | CloudModelEvent::ObjectUntrashed {
                type_and_id:
                    CloudObjectTypeAndId::GenericStringObject {
                        object_type: GenericStringObjectFormat::Json(JsonObjectType::MCPServer),
                        id: _,
                    },
                source: _,
            }
            | CloudModelEvent::ObjectCreated {
                type_and_id:
                    CloudObjectTypeAndId::GenericStringObject {
                        object_type: GenericStringObjectFormat::Json(JsonObjectType::MCPServer),
                        id: _,
                    },
            }
            | CloudModelEvent::ObjectDeleted {
                type_and_id:
                    CloudObjectTypeAndId::GenericStringObject {
                        object_type: GenericStringObjectFormat::Json(JsonObjectType::MCPServer),
                        id: _,
                    },
                folder_id: _,
            }
            | CloudModelEvent::ObjectSynced {
                type_and_id:
                    CloudObjectTypeAndId::GenericStringObject {
                        object_type: GenericStringObjectFormat::Json(JsonObjectType::MCPServer),
                        id: _,
                    },
                client_id: _,
                server_id: _,
            } => {
                me.refresh_server_cards(ctx);
            }
            _ => {}
        });
    }

    fn is_shared(item_id: ServerCardItemId, app: &AppContext) -> bool {
        match item_id {
            ServerCardItemId::TemplatableMCP(template_uuid) => {
                TemplatableMCPServerManager::as_ref(app)
                    .is_server_template_shared(template_uuid, app)
            }
            ServerCardItemId::TemplatableMCPInstallation(installation_uuid) => {
                TemplatableMCPServerManager::as_ref(app)
                    .is_server_installation_shared(installation_uuid, app)
            }
            ServerCardItemId::GalleryMCP(_) | ServerCardItemId::FileBasedMCP(_) => false,
        }
    }

    fn is_shareable(
        item_id: ServerCardItemId,
        server_card_status: ServerCardStatus,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        if !UserWorkspaces::as_ref(ctx).has_teams() {
            return false;
        }
        if TemplatableMCPServerManager::get_first_team_space_id(ctx).is_none() {
            return false;
        }
        match item_id {
            ServerCardItemId::TemplatableMCP(_)
            | ServerCardItemId::TemplatableMCPInstallation(_) => {
                let is_shared = Self::is_shared(item_id, ctx);
                let is_running = matches!(server_card_status, ServerCardStatus::Running);
                !is_shared && is_running
            }
            ServerCardItemId::GalleryMCP(_) | ServerCardItemId::FileBasedMCP(_) => false,
        }
    }

    fn register_server_card(&mut self, server_card: ServerCardView, ctx: &mut ViewContext<Self>) {
        let item_id = server_card.item_id;
        let handle = ctx.add_typed_action_view(move |_ctx| server_card);
        self.server_cards.insert(item_id, handle.clone());
        ctx.subscribe_to_view(&handle, |me, _, event, ctx| {
            me.handle_server_card_event(event, ctx);
        });
        ctx.notify();
    }

    fn create_template_server_card(
        &mut self,
        template: &TemplatableMCPServer,
        ctx: &mut ViewContext<Self>,
    ) {
        let template_uuid = template.uuid;
        let item_id = ServerCardItemId::TemplatableMCP(template_uuid);
        let title_chip_text = Self::get_title_chip_text(item_id, template_uuid, ctx);
        let server_card_status = ServerCardStatus::AvailableToSave;
        let is_shareable = Self::is_shareable(item_id, server_card_status, ctx);

        let server_card = ServerCardView::new(
            item_id,
            template.name.clone(),
            template
                .description
                .clone()
                .or_else(|| Some("Available to install".to_string())),
            None, // Templates can never have tools
            None, // Templates cannot have an error
            title_chip_text.into_iter().collect(),
            ServerCardOptions {
                show_share_icon_button: is_shareable,
                ..server_card_status.into()
            },
        );
        self.register_server_card(server_card, ctx);
    }

    fn create_installation_server_card(
        &mut self,
        installation: &TemplatableMCPServerInstallation,
        ctx: &mut ViewContext<Self>,
    ) {
        let installation_uuid = installation.uuid();
        let item_id = ServerCardItemId::TemplatableMCPInstallation(installation_uuid);
        let uses_oauth = Self::should_show_oauth_components(item_id, ctx);
        let server_card_status =
            match TemplatableMCPServerManager::as_ref(ctx).get_server_state(installation_uuid) {
                Some(state) => state.into(),
                None => ServerCardStatus::Installed,
            };
        let is_shareable = Self::is_shareable(item_id, server_card_status, ctx);
        let is_update_available = TemplatableMCPServerManager::as_ref(ctx)
            .is_update_available_for_installation(installation_uuid, ctx);
        let is_authorized_editor =
            TemplatableMCPServerManager::handle(ctx).read(ctx, |templatable_manager, ctx| {
                templatable_manager.is_authorized_editor(installation.template_uuid(), ctx)
            });
        let should_show_update_symbol = is_authorized_editor && is_update_available;

        let title_chip_text = Self::get_title_chip_text(item_id, installation.template_uuid(), ctx);
        let description = installation.templatable_mcp_server().description.clone();
        let tools = (server_card_status == ServerCardStatus::Running).then_some(
            TemplatableMCPServerManager::as_ref(ctx)
                .tools_for_server(installation_uuid)
                .iter()
                .map(|tool| tool.name.to_string())
                .collect(),
        );
        let error_text = if server_card_status == ServerCardStatus::Error {
            // Get specific error message if available
            TemplatableMCPServerManager::as_ref(ctx)
                .get_server_error_message(installation_uuid)
                .map(|s| s.to_string())
        } else {
            None
        };

        let server_card = ServerCardView::new(
            item_id,
            installation.templatable_mcp_server().name.clone(),
            description,
            tools,
            error_text,
            title_chip_text.into_iter().collect(),
            ServerCardOptions {
                show_log_out_icon_button: uses_oauth,
                show_share_icon_button: is_shareable,
                show_update_available_icon_button: should_show_update_symbol,
                ..server_card_status.into()
            },
        );
        self.register_server_card(server_card, ctx);
    }

    fn create_server_cards(&mut self, ctx: &mut ViewContext<Self>) {
        let template_servers: HashMap<Uuid, TemplatableMCPServer> =
            TemplatableMCPServerManager::as_ref(ctx)
                .get_all_templatable_mcp_servers()
                .iter()
                .map(|&template| (template.uuid, template.clone()))
                .collect();
        let installed_servers: HashMap<Uuid, TemplatableMCPServerInstallation> =
            TemplatableMCPServerManager::as_ref(ctx)
                .get_installed_templatable_servers()
                .clone();

        let mut uninstalled_templates = template_servers;
        for installed_server in installed_servers.values() {
            uninstalled_templates.remove(&installed_server.template_uuid());
        }

        // Create all the server cards
        self.server_cards = Default::default();
        for (_, installation) in installed_servers {
            self.create_installation_server_card(&installation, ctx);
        }
        for (_, template) in uninstalled_templates {
            self.create_template_server_card(&template, ctx);
        }
    }

    fn refresh_server_cards(&mut self, ctx: &mut ViewContext<Self>) {
        self.create_server_cards(ctx);
    }

    fn create_gallery_server_cards(
        ctx: &mut ViewContext<Self>,
    ) -> HashMap<ServerCardItemId, ViewHandle<ServerCardView>> {
        let gallery_manager = MCPGalleryManager::handle(ctx);
        let gallery_items = gallery_manager.as_ref(ctx).get_gallery();

        gallery_items
            .into_iter()
            .map(|gallery_item| {
                let item_id = ServerCardItemId::GalleryMCP(gallery_item.uuid());
                (
                    item_id,
                    ctx.add_typed_action_view(move |_ctx| {
                        ServerCardView::new(
                            item_id,
                            gallery_item.title(),
                            Some(gallery_item.description()),
                            None,
                            None,
                            vec![],
                            ServerCardStatus::AvailableToSave.into(),
                        )
                    }),
                )
            })
            .collect()
    }

    fn share_templatable_mcp_server(&mut self, template_uuid: Uuid, ctx: &mut ViewContext<Self>) {
        TemplatableMCPServerManager::handle(ctx).update(ctx, |templatable_manager, ctx| {
            templatable_manager.share_templatable_mcp_server(template_uuid, ctx);
        });
    }

    fn share_templatable_mcp_server_installation(
        &mut self,
        installation_uuid: Uuid,
        ctx: &mut ViewContext<Self>,
    ) {
        TemplatableMCPServerManager::handle(ctx).update(ctx, |templatable_manager, ctx| {
            templatable_manager.share_templatable_mcp_server_installation(installation_uuid, ctx);
        });
    }

    pub fn delete_server(&mut self, item_id: ServerCardItemId, ctx: &mut ViewContext<Self>) {
        match item_id {
            ServerCardItemId::TemplatableMCP(template_uuid) => {
                TemplatableMCPServerManager::handle(ctx).update(
                    ctx,
                    |mcp_server_manager: &mut TemplatableMCPServerManager, ctx| {
                        mcp_server_manager.delete_templatable_mcp_server(template_uuid, ctx);
                    },
                );
            }
            ServerCardItemId::TemplatableMCPInstallation(installation_uuid) => {
                let template_uuid =
                    TemplatableMCPServerManager::as_ref(ctx).get_template_uuid(installation_uuid);

                // Deleting the template will also delete the installation
                if let Some(template_uuid) = template_uuid {
                    TemplatableMCPServerManager::handle(ctx).update(
                        ctx,
                        |mcp_server_manager: &mut TemplatableMCPServerManager, ctx| {
                            mcp_server_manager.delete_templatable_mcp_server(template_uuid, ctx);
                        },
                    );

                    self.server_cards
                        .remove(&ServerCardItemId::TemplatableMCPInstallation(
                            installation_uuid,
                        ));
                }
            }
            ServerCardItemId::GalleryMCP(_) => {
                log::warn!("Delete is not implemented for gallery MCP items.")
            }
            ServerCardItemId::FileBasedMCP(_) => {
                log::warn!("Delete is not implemented for file-based MCP servers.")
            }
        }

        ctx.notify();
    }

    fn toggle_server_running_templatable(
        &self,
        installation_uuid: Uuid,
        switch_state: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        match switch_state {
            true => {
                TemplatableMCPServerManager::handle(ctx).update(ctx, |mcp_server_manager, ctx| {
                    mcp_server_manager.spawn_server(installation_uuid, ctx);
                });
            }
            false => {
                TemplatableMCPServerManager::handle(ctx).update(ctx, |mcp_server_manager, ctx| {
                    mcp_server_manager.shutdown_server(installation_uuid, ctx);
                })
            }
        }
    }

    fn get_server_card(
        &mut self,
        item_id: ServerCardItemId,
    ) -> Option<&mut ViewHandle<ServerCardView>> {
        self.server_cards.get_mut(&item_id)
    }

    fn should_show_oauth_components(item_id: ServerCardItemId, ctx: &AppContext) -> bool {
        match item_id {
            ServerCardItemId::TemplatableMCP(_) => false,
            ServerCardItemId::TemplatableMCPInstallation(uuid) => {
                let template_uuid =
                    TemplatableMCPServerManager::as_ref(ctx).get_template_uuid(uuid);
                if let Some(template_uuid) = template_uuid {
                    TemplatableMCPServerManager::as_ref(ctx)
                        .has_oauth_credentials_for_server(template_uuid)
                } else {
                    false
                }
            }
            ServerCardItemId::FileBasedMCP(uuid) => FileBasedMCPManager::as_ref(ctx)
                .get_installation_by_uuid(uuid)
                .is_some_and(|installation| {
                    installation.hash().is_some_and(|hash| {
                        TemplatableMCPServerManager::as_ref(ctx)
                            .has_oauth_credentials_for_file_based_server(hash)
                    })
                }),
            ServerCardItemId::GalleryMCP(_) => false,
        }
    }

    fn open_logs_for_server(&self, log_file_path: &PathBuf, ctx: &mut ViewContext<Self>) {
        let window_id = ctx.window_id();
        let Some(workspace_view_handle) = ctx
            .views_of_type::<Workspace>(window_id)
            .and_then(|views| views.first().cloned())
        else {
            report_error!("Could not find workspace when attempting to open MCP logs.");
            return;
        };

        workspace_view_handle.update(ctx, |workspace, ctx| {
            let active_pane_group = workspace.active_tab_pane_group();

            // If there's an active terminal session and it's not busy, return it.
            // If there is no terminal session open, add a terminal pane to the right and return the new terminal view handle.
            active_pane_group.update(ctx, |pane_group, ctx| {
                pane_group.add_terminal_pane(Direction::Right, None /*chosen_shell*/, ctx);
            });
            let Some(terminal_view_handle) = active_pane_group.as_ref(ctx).active_session_view(ctx)
            else {
                report_error!(
                    "Could not get terminal view handle when attempting to open MCP logs."
                );
                return;
            };

            terminal_view_handle.update(ctx, |terminal, ctx| {
                let shell_family = terminal.shell_family(ctx);
                let tail_command = tail_command_for_shell(shell_family, log_file_path);
                terminal.set_pending_command(&tail_command, ctx);
            });
        })
    }

    fn handle_server_card_event(&mut self, event: &ServerCardEvent, ctx: &mut ViewContext<Self>) {
        match event {
            ServerCardEvent::Edit(item_id) => {
                ctx.emit(MCPServersListPageViewEvent::Edit(*item_id));
            }
            ServerCardEvent::Share(item_id) => match item_id {
                ServerCardItemId::TemplatableMCP(template_uuid) => {
                    self.share_templatable_mcp_server(*template_uuid, ctx);
                }
                ServerCardItemId::TemplatableMCPInstallation(installation_uuid) => {
                    self.share_templatable_mcp_server_installation(*installation_uuid, ctx);
                }
                ServerCardItemId::GalleryMCP(_) => {
                    report_error!("Share is not implemented for gallery MCP items.")
                }
                ServerCardItemId::FileBasedMCP(_) => {
                    report_error!("Share is not implemented for file-based MCP servers.")
                }
            },
            ServerCardEvent::ViewLogs(item_id) => match item_id {
                ServerCardItemId::TemplatableMCP(_) => {
                    report_error!("Viewing logs is not implemented for templatable MCP.");
                }
                ServerCardItemId::TemplatableMCPInstallation(installation_uuid) => {
                    if let Some(template_uuid) = TemplatableMCPServerManager::as_ref(ctx)
                        .get_template_uuid(*installation_uuid)
                    {
                        let log_path = logs::log_file_path_from_uuid(&template_uuid);
                        self.open_logs_for_server(&log_path, ctx);
                    } else {
                        report_error!(
                            "Could not find template_uuid for installation",
                            extra: { "installation_uuid" => %installation_uuid }
                        );
                    }
                }
                ServerCardItemId::FileBasedMCP(uuid) => {
                    if let Some(installation) =
                        FileBasedMCPManager::as_ref(ctx).get_installation_by_uuid(*uuid)
                    {
                        let log_path = logs::log_file_path_from_uuid(&installation.template_uuid());
                        self.open_logs_for_server(&log_path, ctx);
                    } else {
                        report_error!(
                            "Could not find installation for file-based server",
                            extra: { "uuid" => %uuid }
                        );
                    }
                }
                ServerCardItemId::GalleryMCP(_) => {
                    report_error!("Viewing logs is not implemented for gallery MCP items.")
                }
            },
            ServerCardEvent::ToggleRunningSwitch(item_id, switch_state) => match item_id {
                ServerCardItemId::TemplatableMCP(_) => {
                    report_error!("Running a server is not implemented for templatable MCP.");
                }
                ServerCardItemId::TemplatableMCPInstallation(uuid) => {
                    self.toggle_server_running_templatable(*uuid, *switch_state, ctx);
                }
                ServerCardItemId::FileBasedMCP(uuid) => {
                    self.toggle_server_running_file_based(*uuid, *switch_state, ctx);
                }
                ServerCardItemId::GalleryMCP(_) => {
                    report_error!("Running a server is not implemented for gallery MCP items.")
                }
            },
            ServerCardEvent::Install(item_id) => match item_id {
                ServerCardItemId::FileBasedMCP(uuid) => {
                    // Clicking the template card for a file-based server starts it.
                    self.toggle_server_running_file_based(*uuid, true, ctx);
                }
                ServerCardItemId::TemplatableMCP(template_uuid) => {
                    let templatable_mcp_server = TemplatableMCPServerManager::as_ref(ctx)
                        .get_templatable_mcp_server(*template_uuid);
                    let is_shared = TemplatableMCPServerManager::as_ref(ctx)
                        .is_server_template_shared(*template_uuid, ctx);

                    if let Some(templatable_mcp_server) = templatable_mcp_server {
                        ctx.emit(MCPServersListPageViewEvent::StartInstallation {
                            templatable_mcp_server: templatable_mcp_server.clone(),
                            instructions_in_markdown: None,
                            origin: InstallOrigin::InApp,
                        });
                        let source: MCPTemplateInstallationSource = match is_shared {
                            true => MCPTemplateInstallationSource::Shared,
                            false => MCPTemplateInstallationSource::Local,
                        };
                        send_telemetry_from_ctx!(
                            TelemetryEvent::MCPTemplateInstalled { source },
                            ctx
                        );
                    }
                }
                ServerCardItemId::TemplatableMCPInstallation(_) => {
                    log::warn!("Installing is not supported for templatable MCP installations.");
                }
                ServerCardItemId::GalleryMCP(gallery_uuid) => {
                    self.install_from_gallery(*gallery_uuid, ctx);
                }
            },
            ServerCardEvent::InstallServerUpdate(item_id) => {
                let ServerCardItemId::TemplatableMCPInstallation(installation_uuid) = item_id
                else {
                    report_error!(
                        "Install server update is only supported for templatable MCP installations"
                    );
                    return;
                };
                self.start_server_update(*installation_uuid, ctx);
            }
            ServerCardEvent::LogOut(item_id) => {
                if let Some(server_card) = self.get_server_card(*item_id) {
                    let server_name = server_card.as_ref(ctx).title();
                    ctx.emit(MCPServersListPageViewEvent::LogOut(
                        *item_id,
                        server_name.to_string(),
                    ));
                    ctx.notify();
                }
            }
        }
    }

    fn start_server_update(&mut self, installation_uuid: Uuid, ctx: &mut ViewContext<Self>) {
        let Some(installation) =
            TemplatableMCPServerManager::as_ref(ctx).get_installed_server(&installation_uuid)
        else {
            log::warn!("Cannot update server {installation_uuid}: Could not find installation.");
            return;
        };
        let server_name = installation.templatable_mcp_server().name.clone();
        let available_updates = TemplatableMCPServerManager::as_ref(ctx)
            .get_updates_available_for_installation(installation_uuid, ctx);

        // Automatically install it if there's only one update available, otherwise open the modal so the user can select which one they want to proceed with
        if available_updates.len() == 1 {
            self.process_server_update(installation_uuid, available_updates[0].clone(), ctx);
        } else {
            self.update_modal_state.view.update(ctx, |modal, ctx| {
                modal.body().update(ctx, |body, ctx| {
                    body.set_installation(installation_uuid, server_name, available_updates, ctx);
                    ctx.notify();
                });
            });
            self.update_modal_state.open();
            ctx.focus(&self.update_modal_state.view);
            ctx.emit(MCPServersListPageViewEvent::ShowModal);
        }
        ctx.notify();
    }

    fn process_server_update(
        &mut self,
        installation_uuid: Uuid,
        update: MCPServerUpdate,
        ctx: &mut ViewContext<Self>,
    ) {
        let installation =
            TemplatableMCPServerManager::as_ref(ctx).get_installed_server(&installation_uuid);
        let Some(installation) = installation else {
            log::warn!(
                "Failed to update MCP server: Could not find installation {installation_uuid}"
            );
            return;
        };
        let local_templatable_mcp_server = installation.templatable_mcp_server();

        match update {
            MCPServerUpdate::CloudTemplate { .. } => {
                let latest_templatable_mcp_server = TemplatableMCPServerManager::as_ref(ctx)
                    .get_templatable_mcp_server(installation.template_uuid())
                    .cloned();
                let Some(latest_templatable_mcp_server) = latest_templatable_mcp_server else {
                    log::warn!(
                        "Failed to update MCP server: Could not find templatable MCP server for installation {installation_uuid}"
                    );
                    return;
                };

                if local_templatable_mcp_server.version == latest_templatable_mcp_server.version {
                    log::warn!("Failed to update MCP server: Installed server is up to date");
                    return;
                }
                if local_templatable_mcp_server.version > latest_templatable_mcp_server.version {
                    log::warn!(
                        "Failed to update MCP server: Installed server is ahead of the latest template"
                    );
                    return;
                }

                self.update_installation_via_template(
                    installation.clone(),
                    latest_templatable_mcp_server,
                    ctx,
                );
                // We do not have to update the cloud template, because this update came from a cloud template
                log::info!(
                    "Successfully updated server {installation_uuid} with the newest cloud template."
                );

                // Show the toast that the server updated, even though we don't update the cloud template in this case
                let window_id = ctx.window_id();
                ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                    let toast = DismissibleToast::success(String::from("MCP server updated"));
                    toast_stack.add_ephemeral_toast(toast, window_id, ctx);
                });
            }
            MCPServerUpdate::Gallery { .. } => {
                let Some(GalleryData {
                    gallery_item_id,
                    version: installed_gallery_version,
                }) = local_templatable_mcp_server.gallery_data
                else {
                    log::warn!(
                        "Failed to update MCP server to newest gallery version: Installed server is not from the MCP gallery."
                    );
                    return;
                };
                let Some(gallery_item) =
                    MCPGalleryManager::as_ref(ctx).get_gallery_item(gallery_item_id)
                else {
                    log::warn!(
                        "Failed to update MCP server to newest gallery version: Could not find gallery item with uuid {gallery_item_id}"
                    );
                    return;
                };

                if installed_gallery_version == gallery_item.version() {
                    log::warn!(
                        "Failed to update MCP server to newest gallery version: Installed server is up to date"
                    );
                    return;
                }
                if installed_gallery_version > gallery_item.version() {
                    log::warn!(
                        "Failed to update MCP server to newest gallery version: Installed server is ahead of the latest gallery item"
                    );
                    return;
                }

                let Some(gallery_templatable_mcp_server) =
                    MCPGalleryManager::as_ref(ctx).get_templatable_mcp_server(gallery_item_id)
                else {
                    log::warn!(
                        "Failed to update MCP server to newest gallery version: Could not find newest gallery item"
                    );
                    return;
                };

                // We need to update both the cloud template and the installation
                let new_template = TemplatableMCPServer {
                    uuid: installation.template_uuid(),
                    ..gallery_templatable_mcp_server.clone()
                };
                self.update_installation_via_template(
                    installation.clone(),
                    new_template.clone(),
                    ctx,
                );
                TemplatableMCPServerManager::handle(ctx).update(ctx, |templatable_manager, ctx| {
                    templatable_manager.update_templatable_mcp_server(new_template, ctx);
                });
                log::info!(
                    "Successfully updated server {installation_uuid} with the newest gallery template."
                );
                // We don't need to manually show a toast, because it will appear once the cloud template update goes through
            }
        };
    }

    fn update_installation_via_template(
        &mut self,
        installation: TemplatableMCPServerInstallation,
        new_templatable_mcp_server: TemplatableMCPServer,
        ctx: &mut ViewContext<Self>,
    ) {
        let installation_uuid = installation.uuid();
        if installation.template_uuid() != new_templatable_mcp_server.uuid {
            log::warn!(
                "Unable to update installation: installation template uuid differs from the new template uuid."
            );
            return;
        }

        let local_templatable_mcp_server = installation.templatable_mcp_server();

        if local_templatable_mcp_server.template.json == new_templatable_mcp_server.template.json
            || local_templatable_mcp_server.template.variables
                == new_templatable_mcp_server.template.variables
        {
            // Re-install the server with the same variable values
            // This will also bump the version of the installed server
            TemplatableMCPServerManager::handle(ctx).update(ctx, |manager, ctx| {
                manager.update_templatable_mcp_server_installation(
                    installation_uuid,
                    &new_templatable_mcp_server,
                    true,
                    ctx,
                );
            });
            return;
        }

        // Breaking changes are detected, uninstall the server and trigger the installation modal
        TemplatableMCPServerManager::handle(ctx).update(ctx, |manager, ctx| {
            manager.delete_templatable_mcp_server_installation(installation_uuid, ctx);
        });
        ctx.emit(MCPServersListPageViewEvent::StartInstallation {
            templatable_mcp_server: new_templatable_mcp_server,
            instructions_in_markdown: None,
            origin: InstallOrigin::InApp,
        });
        ctx.notify();
    }

    fn install_from_gallery(&mut self, gallery_uuid: Uuid, ctx: &mut ViewContext<Self>) {
        let gallery_server = MCPGalleryManager::as_ref(ctx).get_gallery_item(gallery_uuid);
        let Some(gallery_server) = gallery_server else {
            log::warn!(
                "Could not install gallery item {gallery_uuid}: Unable to find gallery item with matching id."
            );
            return;
        };

        let instructions = gallery_server.instructions_in_markdown().cloned();
        log::info!(
            "[ListPage] Installing from gallery with instructions: {:?}",
            instructions.as_ref().map(|s| truncate_from_end(s, 53))
        );
        let templatable_server: Result<TemplatableMCPServer, String> =
            gallery_server.clone().try_into();
        match templatable_server {
            Ok(templatable_server) => {
                ctx.emit(MCPServersListPageViewEvent::StartInstallation {
                    templatable_mcp_server: templatable_server,
                    instructions_in_markdown: instructions,
                    origin: InstallOrigin::InApp,
                });
                send_telemetry_from_ctx!(
                    TelemetryEvent::MCPTemplateInstalled {
                        source: MCPTemplateInstallationSource::Gallery
                    },
                    ctx
                );
            }
            Err(e) => {
                log::warn!("Could not install gallery item {gallery_uuid}: {e}");
            }
        };
    }

    fn handle_update_modal_body_event(
        &mut self,
        event: &UpdateModalBodyEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            UpdateModalBodyEvent::Cancel => {
                self.update_modal_state.view.update(ctx, |modal, ctx| {
                    modal.body().update(ctx, |body, ctx| {
                        body.clear(ctx);
                    });
                });
                self.update_modal_state.close();
                ctx.emit(MCPServersListPageViewEvent::HideModal);
                ctx.notify();
            }
            UpdateModalBodyEvent::Update {
                installation_uuid,
                update,
            } => {
                let Some(installation_uuid) = installation_uuid else {
                    log::warn!("Cannot update installation with uuid of None");
                    return;
                };
                self.process_server_update(*installation_uuid, update.clone(), ctx);
                self.update_modal_state.view.update(ctx, |modal, ctx| {
                    modal.body().update(ctx, |body, ctx| {
                        body.clear(ctx);
                    });
                });
                self.update_modal_state.close();
                ctx.emit(MCPServersListPageViewEvent::HideModal);
                ctx.notify();
            }
        }
    }

    fn handle_update_modal_event(&mut self, event: &ModalEvent, ctx: &mut ViewContext<Self>) {
        match event {
            ModalEvent::Close => {
                self.update_modal_state.close();
                ctx.emit(MCPServersListPageViewEvent::HideModal);
                ctx.notify();
            }
        }
    }

    fn handle_templatable_mcp_manager_event(
        &mut self,
        event: &TemplatableMCPServerManagerEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            TemplatableMCPServerManagerEvent::StateChanged { uuid: _, state: _ }
            | TemplatableMCPServerManagerEvent::AuthenticationRequired { uuid: _ }
            | TemplatableMCPServerManagerEvent::CredentialsChanged { uuid: _ }
            | TemplatableMCPServerManagerEvent::ServerInstallationAdded(_)
            | TemplatableMCPServerManagerEvent::ServerInstallationDeleted(_)
            | TemplatableMCPServerManagerEvent::TemplatableMCPServersUpdated
            | TemplatableMCPServerManagerEvent::LegacyServerConverted => {
                self.refresh_server_cards(ctx);
                self.refresh_file_based_server_cards(ctx);
            }
        }
    }

    fn handle_mcp_gallery_manager_event(
        &mut self,
        event: &MCPGalleryManagerEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            MCPGalleryManagerEvent::ItemsRefreshed => {
                self.refresh_gallery_cards(ctx);
                // We also need to refresh the server cards, because they use the gallery information to determine if an update is available
                self.refresh_server_cards(ctx);
            }
        }
    }

    fn refresh_gallery_cards(&mut self, ctx: &mut ViewContext<Self>) {
        self.gallery_server_cards = Self::create_gallery_server_cards(ctx);
        for server_card_handle in self.gallery_server_cards.values() {
            ctx.subscribe_to_view(server_card_handle, |me, _, event, ctx| {
                me.handle_server_card_event(event, ctx);
            });
        }
        ctx.notify();
    }

    pub fn get_modal_content(&self) -> Option<Box<dyn Element>> {
        if self.update_modal_state.is_open() {
            Some(self.update_modal_state.render())
        } else {
            None
        }
    }

    fn server_card_handle_matches_search(
        handle: &ViewHandle<ServerCardView>,
        search_term: &str,
        app: &AppContext,
    ) -> bool {
        let search_lower = search_term.to_lowercase();
        search_lower.is_empty()
            || handle
                .as_ref(app)
                .title()
                .to_lowercase()
                .contains(&search_lower)
    }

    fn render_file_based_mcp_section(
        &self,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let ai_settings = AISettings::as_ref(app);
        let is_enabled = *ai_settings.file_based_mcp_enabled;
        let is_any_ai_enabled = ai_settings.is_any_ai_enabled(app);

        let label = render_body_item_label::<MCPServersListPageViewAction>(
            "Auto-spawn servers from third-party agents".to_string(),
            None,
            None,
            LocalOnlyIconState::Hidden,
            ToggleState::Enabled,
            appearance,
        );

        let switch = appearance
            .ui_builder()
            .switch(self.file_based_mcp_toggle.clone())
            .check(is_enabled)
            .with_disabled(!is_any_ai_enabled)
            .with_disabled_styles(UiComponentStyles {
                background: Some(Fill::Solid(internal_colors::neutral_4(appearance.theme()))),
                foreground: Some(Fill::Solid(internal_colors::neutral_5(appearance.theme()))),
                ..Default::default()
            })
            .build()
            .on_click(move |ctx, _, _| {
                if !is_any_ai_enabled {
                    return;
                }
                ctx.dispatch_typed_action(MCPServersListPageViewAction::ToggleFileBasedMcp);
            })
            .finish();

        let toggle_row = build_toggle_element(label, switch, appearance, None);

        static FILE_BASED_MCP_DESCRIPTION_FRAGMENTS: std::sync::LazyLock<
            Vec<FormattedTextFragment>,
        > = std::sync::LazyLock::new(|| {
            vec![
                FormattedTextFragment::plain_text(
                    "Automatically detect and spawn MCP servers from globally-scoped third-party AI agent configuration files (e.g. in your home directory). Servers detected inside a repository are never spawned automatically and must be enabled individually in the \"Detected from\" sections below. ",
                ),
                FormattedTextFragment::hyperlink(
                    "See supported providers.",
                    "https://docs.warp.dev/agent-platform/capabilities/mcp#file-based-mcp-servers",
                ),
            ]
        });

        let description = FormattedTextElement::new(
            FormattedText::new([FormattedTextLine::Line(
                (*FILE_BASED_MCP_DESCRIPTION_FRAGMENTS).clone(),
            )]),
            style::CONTENT_FONT_SIZE,
            appearance.ui_font_family(),
            appearance.ui_font_family(),
            blended_colors::text_sub(appearance.theme(), appearance.theme().surface_1()),
            HighlightedHyperlink::default(),
        )
        .with_hyperlink_font_color(appearance.theme().accent().into_solid())
        .register_default_click_handlers(|url, _, ctx| {
            ctx.open_url(&url.url);
        })
        .finish();

        let description_container = Container::new(description)
            .with_margin_top(-12.)
            .with_margin_bottom(12.)
            .with_margin_right(48.)
            .finish();

        Flex::column()
            .with_child(toggle_row)
            .with_child(description_container)
            .finish()
    }

    fn render_page_body(&self, appearance: &Appearance, app: &AppContext) -> Box<dyn Element> {
        let description_fragments = vec![
            FormattedTextFragment::plain_text(DESCRIPTION_TEXT),
            FormattedTextFragment::hyperlink(
                "Learn more.",
                "https://docs.warp.dev/agent-platform/capabilities/mcp",
            ),
        ];

        let description = FormattedTextElement::new(
            FormattedText::new([FormattedTextLine::Line(description_fragments)]),
            style::CONTENT_FONT_SIZE,
            appearance.ui_font_family(),
            appearance.ui_font_family(),
            blended_colors::text_sub(appearance.theme(), appearance.theme().surface_1()),
            HighlightedHyperlink::default(),
        )
        .with_hyperlink_font_color(appearance.theme().accent().into_solid())
        .register_default_click_handlers(|url, _, ctx| {
            ctx.open_url(&url.url);
        })
        .finish();

        let mut page = Flex::column()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_spacing(style::PAGE_SPACING)
            .with_child(description);

        let search_term = self.search_editor.as_ref(app).buffer_text(app);

        // Collect filtered server cards by ID.
        let filtered_server_cards: HashMap<ServerCardItemId, ViewHandle<ServerCardView>> = self
            .server_cards
            .iter()
            .filter(|(_, v)| Self::server_card_handle_matches_search(v, &search_term, app))
            .map(|(k, v)| (*k, v.clone()))
            .collect();

        // Collect filtered gallery cards.
        let deduplicated_gallery_cards = self.deduplicate_gallery_cards(app);
        let filtered_gallery_cards: Vec<ViewHandle<ServerCardView>> = deduplicated_gallery_cards
            .values()
            .filter(|v| Self::server_card_handle_matches_search(v, &search_term, app))
            .cloned()
            .collect();

        // Collect filtered file-based server cards by provider.
        let mut filtered_file_based_cards: HashMap<MCPProvider, Vec<ViewHandle<ServerCardView>>> =
            HashMap::new();
        for provider in MCPProvider::iter() {
            if let Some(cards_for_provider) = self.file_based_template_cards.get(&provider) {
                let filtered: Vec<ViewHandle<ServerCardView>> = cards_for_provider
                    .iter()
                    .filter(|card| Self::server_card_handle_matches_search(card, &search_term, app))
                    .cloned()
                    .collect();
                if !filtered.is_empty() {
                    filtered_file_based_cards.insert(provider, filtered);
                }
            }
        }

        let has_any_content = !self.server_cards.is_empty()
            || !filtered_gallery_cards.is_empty()
            || !filtered_file_based_cards.is_empty();

        if !has_any_content {
            let empty_state = self.render_empty_state(appearance, app);
            page.add_child(empty_state);
        } else {
            page.add_child(self.render_controls());

            if FeatureFlag::FileBasedMcp.is_enabled() {
                page.add_child(self.render_file_based_mcp_section(appearance, app));
            }

            if filtered_server_cards.is_empty()
                && filtered_gallery_cards.is_empty()
                && filtered_file_based_cards.is_empty()
            {
                page.add_child(Self::render_no_search_results(appearance));
            } else {
                let (owned_server_cards, mut shared_server_cards) =
                    Self::separate_server_cards_by_installed(&filtered_server_cards, app);

                if !owned_server_cards.is_empty() {
                    page.add_child(self.render_server_cards_section(
                        "My MCPs",
                        &owned_server_cards,
                        appearance,
                        app,
                    ));
                }
                if !shared_server_cards.is_empty() {
                    shared_server_cards.extend(filtered_gallery_cards);
                    let team_name = UserWorkspaces::as_ref(app)
                        .current_team()
                        .map(|team| team.name.clone());
                    let shared_by_text = match team_name {
                        Some(name) => format!("Shared by Warp and {name}"),
                        None => "Shared by Warp and from other devices".to_string(),
                    };

                    page.add_child(self.render_server_cards_section(
                        &shared_by_text,
                        &shared_server_cards,
                        appearance,
                        app,
                    ));
                } else if !filtered_gallery_cards.is_empty() {
                    page.add_child(self.render_server_cards_section(
                        "Shared from Warp",
                        &filtered_gallery_cards,
                        appearance,
                        app,
                    ));
                }

                // Render one section per provider (e.g. "Detected from Claude").
                for (provider, cards) in &filtered_file_based_cards {
                    let section_title = format!("Detected from {}", provider.display_name());
                    page.add_child(self.render_server_cards_section(
                        &section_title,
                        cards,
                        appearance,
                        app,
                    ));
                }
            }
        }

        page.finish()
    }

    fn render_controls(&self) -> Box<dyn Element> {
        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(Expanded::new(1., ChildView::new(&self.search_bar).finish()).finish())
            .with_child(self.render_add_button())
            .finish()
    }

    fn render_add_button(&self) -> Box<dyn Element> {
        Container::new(ChildView::new(&self.add_button).finish())
            .with_margin_left(style::SECTION_MARGIN)
            .finish()
    }

    fn separate_server_cards_by_installed(
        server_cards: &HashMap<ServerCardItemId, ViewHandle<ServerCardView>>,
        app: &AppContext,
    ) -> (
        Vec<ViewHandle<ServerCardView>>,
        Vec<ViewHandle<ServerCardView>>,
    ) {
        let mut owned_server_cards = Vec::new();
        let mut shared_server_cards = Vec::new();
        for (item_id, server_card) in server_cards {
            match item_id {
                ServerCardItemId::TemplatableMCP(_) => {
                    if Self::is_shared(*item_id, app) {
                        shared_server_cards.push(server_card.clone());
                    } else {
                        owned_server_cards.push(server_card.clone());
                    }
                }
                ServerCardItemId::TemplatableMCPInstallation(_) => {
                    owned_server_cards.push(server_card.clone());
                }
                ServerCardItemId::FileBasedMCP(_) => {
                    owned_server_cards.push(server_card.clone());
                }
                ServerCardItemId::GalleryMCP(_) => {
                    log::warn!(
                        "Received an unexpected gallery server card when separating server cards by installed."
                    );
                }
            }
        }
        (owned_server_cards, shared_server_cards)
    }

    fn deduplicate_gallery_cards(
        &self,
        app: &AppContext,
    ) -> HashMap<ServerCardItemId, ViewHandle<ServerCardView>> {
        let gallery_ids = TemplatableMCPServerManager::as_ref(app).extract_server_info(
            |template| {
                template
                    .gallery_data
                    .map(|gallery_data| gallery_data.gallery_item_id)
            },
            |installation| installation.gallery_uuid(),
            app,
        );
        let names = TemplatableMCPServerManager::as_ref(app).extract_server_info(
            |template| Some(template.name.to_ascii_lowercase()),
            |installation| {
                Some(
                    installation
                        .templatable_mcp_server()
                        .name
                        .to_ascii_lowercase(),
                )
            },
            app,
        );

        let mut deduplicated_gallery_cards = self.gallery_server_cards.clone();
        deduplicated_gallery_cards.retain(|item_id, server_card| match item_id {
            ServerCardItemId::GalleryMCP(gallery_uuid) => {
                !names.contains(&server_card.as_ref(app).title().to_lowercase())
                    && !gallery_ids.contains(gallery_uuid)
            }
            _ => false,
        });
        deduplicated_gallery_cards
    }

    fn render_server_cards(
        &self,
        server_cards: &[ViewHandle<ServerCardView>],
        _appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let mut col = Flex::column().with_spacing(style::SERVER_CARD_LIST_SPACING);
        let mut ordered_server_cards: Vec<&ViewHandle<ServerCardView>> =
            server_cards.iter().collect();
        self.sort_server_cards(&mut ordered_server_cards, app);
        for server_card in ordered_server_cards {
            col.add_child(ChildView::new(server_card).finish());
        }
        col.finish()
    }

    fn sort_server_cards(
        &self,
        server_cards: &mut Vec<&ViewHandle<ServerCardView>>,
        app: &AppContext,
    ) {
        fn priority(item_id: ServerCardItemId) -> i8 {
            match item_id {
                ServerCardItemId::TemplatableMCPInstallation(_) => 1,
                ServerCardItemId::FileBasedMCP(_) => 1,
                ServerCardItemId::TemplatableMCP(_) => 2,
                ServerCardItemId::GalleryMCP(_) => 2,
            }
        }

        server_cards.sort_by(|a, b| {
            let a_ref = a.as_ref(app);
            let b_ref = b.as_ref(app);

            priority(a_ref.item_id)
                .cmp(&priority(b_ref.item_id))
                .then_with(|| {
                    // Only for uninstalled templates, we should put the ones in personal drive before the shared ones
                    if matches!(a_ref.item_id, ServerCardItemId::TemplatableMCP(_))
                        && matches!(b_ref.item_id, ServerCardItemId::TemplatableMCP(_))
                    {
                        Self::is_shared(a_ref.item_id, app)
                            .cmp(&Self::is_shared(b_ref.item_id, app))
                    } else {
                        Ordering::Equal
                    }
                })
                .then_with(|| {
                    a_ref
                        .title()
                        .to_lowercase()
                        .cmp(&b_ref.title().to_lowercase())
                })
                .then_with(|| a_ref.item_id.cmp(&b_ref.item_id))
        });
    }

    fn render_overline_header(&self, text: &str, appearance: &Appearance) -> Box<dyn Element> {
        Text::new(
            text.to_uppercase(),
            appearance.overline_font_family(),
            appearance.overline_font_size(),
        )
        .with_color(blended_colors::text_sub(
            appearance.theme(),
            appearance.theme().surface_2(),
        ))
        .finish()
    }

    fn render_server_cards_section(
        &self,
        header: &str,
        server_cards: &[ViewHandle<ServerCardView>],
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        Flex::column()
            .with_children([
                self.render_overline_header(header, appearance),
                self.render_server_cards(server_cards, appearance, app),
            ])
            .with_spacing(style::SERVER_CARD_LIST_SPACING)
            .finish()
    }

    fn render_empty_state(&self, appearance: &Appearance, _app: &AppContext) -> Box<dyn Element> {
        Container::new(
            ConstrainedBox::new(
                Align::new(
                    Flex::column()
                        .with_main_axis_size(MainAxisSize::Max)
                        .with_main_axis_alignment(MainAxisAlignment::Center)
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .with_child(
                            appearance
                                .ui_builder()
                                .wrappable_text(EMPTY_STATE_TEXT, true)
                                .with_style(style::description_text(appearance))
                                .build()
                                .finish(),
                        )
                        .with_child(self.render_add_button())
                        .finish(),
                )
                .finish(),
            )
            .with_height(style::EMPTY_STATE_HEIGHT)
            .finish(),
        )
        .with_border(
            Border::all(1.).with_border_color(internal_colors::neutral_2(appearance.theme())),
        )
        .with_margin_bottom(style::SECTION_MARGIN)
        .finish()
    }

    fn render_no_search_results(appearance: &Appearance) -> Box<dyn Element> {
        Container::new(
            ConstrainedBox::new(
                Align::new(
                    Flex::column()
                        .with_main_axis_size(MainAxisSize::Max)
                        .with_main_axis_alignment(MainAxisAlignment::Center)
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .with_child(
                            appearance
                                .ui_builder()
                                .wrappable_text(NO_SEARCH_RESULTS_TEXT, true)
                                .with_style(style::description_text(appearance))
                                .build()
                                .finish(),
                        )
                        .finish(),
                )
                .finish(),
            )
            .with_height(style::EMPTY_STATE_HEIGHT)
            .finish(),
        )
        .with_margin_bottom(style::SECTION_MARGIN)
        .finish()
    }

    fn file_based_root_chip_text(root_path: &PathBuf) -> Option<String> {
        // If the path is the user's home directory, set the text to "global".
        if let Some(home_dir) = dirs::home_dir() {
            if root_path == &home_dir {
                return Some("global".to_string());
            }
        }

        // If the path is the Warp data directory (e.g. ~/.warp or ~/.warp_dev), set the text to
        // "global". The Warp provider stores its data directory as the root path rather than the
        // home directory, unlike other providers that store the home directory directly.
        if root_path == &crate::warp_managed_paths_watcher::warp_data_dir() {
            return Some("global".to_string());
        }

        // Otherwise, set the text to the final path component.
        root_path
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.to_string())
    }

    fn get_file_based_title_chips(
        uuid: Uuid,
        provider_filter: Option<MCPProvider>,
        ctx: &AppContext,
    ) -> Vec<TitleChip> {
        // If a specific provider is given, only include that provider's directories for this installation.
        // Otherwise, include directories from all available providers for this installation.
        let providers = match provider_filter {
            Some(provider) => vec![provider],
            None => MCPProvider::iter().collect(),
        };

        let mut title_chips = Vec::new();
        for provider in providers {
            let paths = FileBasedMCPManager::as_ref(ctx)
                .directory_paths_for_installation_and_provider(uuid, provider);
            for path in paths {
                if let Some(text) = Self::file_based_root_chip_text(&path) {
                    title_chips.push(TitleChip::with_icon(text, provider.icon()));
                }
            }
        }

        // If global is present, only show global chips (global scope implies project-scope
        // chips are redundant).
        if title_chips.iter().any(|chip| chip.text == "global") {
            title_chips.retain(|chip| chip.text == "global");
        }

        title_chips
    }

    fn register_file_based_template_card(
        &mut self,
        provider: MCPProvider,
        server_card: ServerCardView,
        ctx: &mut ViewContext<Self>,
    ) {
        let handle = ctx.add_typed_action_view(move |_ctx| server_card);
        self.file_based_template_cards
            .entry(provider)
            .or_default()
            .push(handle.clone());
        ctx.subscribe_to_view(&handle, |me, _, event, ctx| {
            me.handle_server_card_event(event, ctx);
        });
        ctx.notify();
    }

    /// Creates template-style cards for a file-based server that is not yet running, one per
    /// provider where the installation was detected.
    fn create_file_based_template_cards(
        &mut self,
        installation: &TemplatableMCPServerInstallation,
        ctx: &mut ViewContext<Self>,
    ) {
        let uuid = installation.uuid();

        // Creates a template card for each (provider, uninstalled server) pair.
        for provider in MCPProvider::iter() {
            let title_chips = Self::get_file_based_title_chips(uuid, Some(provider), ctx);
            if title_chips.is_empty() {
                continue;
            }

            let server_card = ServerCardView::new(
                ServerCardItemId::FileBasedMCP(uuid),
                installation.templatable_mcp_server().name.clone(),
                installation
                    .templatable_mcp_server()
                    .description
                    .clone()
                    .or_else(|| Some("Detected from config file".to_string())),
                None, // tools only available when running
                None, // no error when not yet started
                title_chips,
                ServerCardOptions {
                    show_share_icon_button: false,
                    ..ServerCardStatus::AvailableToSave.into()
                },
            );
            self.register_file_based_template_card(provider, server_card, ctx);
        }
    }

    /// Creates an installation-style card for a file-based server that is running
    /// (or in a transitional state). Edit and share controls are omitted.
    fn create_file_based_spawned_card(
        &mut self,
        installation: &TemplatableMCPServerInstallation,
        ctx: &mut ViewContext<Self>,
    ) {
        let uuid = installation.uuid();
        let item_id = ServerCardItemId::FileBasedMCP(uuid);
        let uses_oauth = Self::should_show_oauth_components(item_id, ctx);
        let server_card_status =
            match TemplatableMCPServerManager::as_ref(ctx).get_server_state(uuid) {
                Some(state) => state.into(),
                None => ServerCardStatus::Installed,
            };
        let title_chips = Self::get_file_based_title_chips(uuid, None, ctx);
        let tools = (server_card_status == ServerCardStatus::Running).then_some(
            TemplatableMCPServerManager::as_ref(ctx)
                .tools_for_server(uuid)
                .iter()
                .map(|tool| tool.name.to_string())
                .collect(),
        );
        let error_text = if server_card_status == ServerCardStatus::Error {
            TemplatableMCPServerManager::as_ref(ctx)
                .get_server_error_message(uuid)
                .map(|s| s.to_string())
        } else {
            None
        };

        let server_card = ServerCardView::new(
            item_id,
            installation.templatable_mcp_server().name.clone(),
            installation.templatable_mcp_server().description.clone(),
            tools,
            error_text,
            title_chips,
            ServerCardOptions {
                // File-based servers cannot be edited or shared from settings.
                show_log_out_icon_button: uses_oauth,
                show_edit_config_icon_button: false,
                show_share_icon_button: false,
                ..server_card_status.into()
            },
        );
        self.register_server_card(server_card, ctx);
    }

    fn create_file_based_server_cards(&mut self, ctx: &mut ViewContext<Self>) {
        self.file_based_template_cards = Default::default();
        // Remove any previously promoted running file-based servers from the installed section.
        self.server_cards
            .retain(|id, _| !matches!(id, ServerCardItemId::FileBasedMCP(_)));

        let installations: Vec<TemplatableMCPServerInstallation> = FileBasedMCPManager::as_ref(ctx)
            .file_based_servers()
            .into_iter()
            .cloned()
            .collect();

        for installation in &installations {
            let uuid = installation.uuid();
            let has_state = TemplatableMCPServerManager::as_ref(ctx)
                .get_server_state(uuid)
                .is_some();
            if has_state {
                // Running servers are promoted to the main installed section.
                self.create_file_based_spawned_card(installation, ctx);
            } else {
                self.create_file_based_template_cards(installation, ctx);
            }
        }
    }

    #[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
    fn refresh_file_based_server_cards(&mut self, ctx: &mut ViewContext<Self>) {
        self.create_file_based_server_cards(ctx);
    }

    fn toggle_server_running_file_based(
        &self,
        uuid: Uuid,
        switch_state: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        match switch_state {
            true => {
                let installation = FileBasedMCPManager::as_ref(ctx)
                    .get_installation_by_uuid(uuid)
                    .cloned();
                if let Some(installation) = installation {
                    TemplatableMCPServerManager::handle(ctx).update(ctx, |mgr, ctx| {
                        mgr.spawn_ephemeral_server(installation, ctx);
                    });
                } else {
                    log::warn!("Cannot start file-based server {uuid}: installation not found");
                }
            }
            false => TemplatableMCPServerManager::handle(ctx).update(ctx, |mgr, ctx| {
                // Shuts down the file-based server without purging credentials.
                mgr.shutdown_server(uuid, ctx);
            }),
        }
    }

    fn get_title_chip_text(
        item_id: ServerCardItemId,
        template_uuid: Uuid,
        ctx: &mut ViewContext<Self>,
    ) -> Option<TitleChip> {
        match item_id {
            ServerCardItemId::TemplatableMCP(_)
            | ServerCardItemId::TemplatableMCPInstallation(_) => {
                let is_shared =
                    Self::is_shared(ServerCardItemId::TemplatableMCP(template_uuid), ctx);
                let creator =
                    TemplatableMCPServerManager::as_ref(ctx).get_creator(template_uuid, ctx);

                if is_shared {
                    match creator {
                        Some(creator) => Some(TitleChip::text(format!("Shared by: {creator}"))),
                        None => Some(TitleChip::text("Shared by a team member")),
                    }
                } else if matches!(item_id, ServerCardItemId::TemplatableMCP(_)) {
                    Some(TitleChip::text("From another device"))
                } else {
                    None
                }
            }
            _ => None,
        }
    }
}

impl Entity for MCPServersListPageView {
    type Event = MCPServersListPageViewEvent;
}

impl View for MCPServersListPageView {
    fn ui_name() -> &'static str {
        "MCPServersListPageView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        self.render_page_body(appearance, app)
    }
}

impl TypedActionView for MCPServersListPageView {
    type Action = MCPServersListPageViewAction;

    fn handle_action(
        &mut self,
        action: &MCPServersListPageViewAction,
        ctx: &mut ViewContext<Self>,
    ) {
        match action {
            MCPServersListPageViewAction::Add => {
                ctx.emit(MCPServersListPageViewEvent::Add);
            }
            MCPServersListPageViewAction::ToggleFileBasedMcp => {
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    if let Err(e) = settings.file_based_mcp_enabled.toggle_and_save_value(ctx) {
                        log::warn!("Failed to toggle file-based MCP setting: {e:?}");
                    }
                });
                ctx.notify();
            }
        }
    }
}
