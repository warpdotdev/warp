#[cfg(feature = "local_fs")]
use crate::ai::mcp::{
    // Import events for file-based manager and watcher conditionally
    // since these controls do not export events.
    file_based_manager::FileBasedMCPManagerEvent,
    FileMCPWatcher,
    FileMCPWatcherEvent,
};
use crate::settings::{AISettings, AISettingsChangedEvent};
use crate::settings_view::settings_page::{
    build_toggle_element, render_body_item_label, LocalOnlyIconState, ToggleState,
};
use crate::{
    ai::mcp::{
        logs,
        templatable::TemplatableMCPServer,
        templatable_manager::{TemplatableMCPServerManager, TemplatableMCPServerManagerEvent},
        FileBasedMCPManager, MCPProvider, TemplatableMCPServerInstallation,
    },
    appearance::Appearance,
    editor::{EditorView, PropagateAndNoOpNavigationKeys, SingleLineEditorOptions, TextOptions},
    pane_group::Direction,
    search_bar::SearchBar,
    settings_view::mcp_servers::{
        server_card::{
            ServerCardEvent, ServerCardOptions, ServerCardStatus, ServerCardView, TitleChip,
        },
        style, ServerCardItemId,
    },
    ui_components::blended_colors,
    view_components::action_button::{ActionButton, NakedTheme},
    workflows::local_workflows::tail_command_for_shell,
    workspace::Workspace,
};
use markdown_parser::{FormattedText, FormattedTextFragment, FormattedTextLine};
use settings::ToggleableSetting as _;
use std::{collections::HashMap, path::PathBuf};
use strum::IntoEnumIterator;
use uuid::Uuid;
use warp_core::features::FeatureFlag;
use warp_core::ui::{appearance::AppearanceEvent, theme::color::internal_colors, Icon};
use warpui::{
    elements::{
        Align, Border, ChildView, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment,
        Expanded, Fill, Flex, FormattedTextElement, HighlightedHyperlink, MainAxisAlignment,
        MainAxisSize, ParentElement, Radius, Text,
    },
    ui_components::{
        components::{Coords, UiComponent, UiComponentStyles},
        switch::SwitchStateHandle,
    },
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};

const DESCRIPTION_TEXT: &str = "Add MCP servers to extend the Warper Agent's capabilities. MCP servers expose data sources or tools to agents through a standardized interface, essentially acting like plugins. Add a custom server, or use the presets to get started with popular servers. ";

#[derive(Debug, Clone)]
pub enum MCPServersListPageViewEvent {
    Add,
    Edit(ServerCardItemId),
    LogOut(ServerCardItemId, String),
    StartInstallation {
        templatable_mcp_server: TemplatableMCPServer,
        instructions_in_markdown: Option<String>,
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
    // MCP server cards for uninstalled file-based servers, grouped by provider.
    file_based_template_cards: HashMap<MCPProvider, Vec<ViewHandle<ServerCardView>>>,
    search_editor: ViewHandle<EditorView>,
    search_bar: ViewHandle<SearchBar>,
    add_button: ViewHandle<ActionButton>,
    file_based_mcp_toggle: SwitchStateHandle,
}

impl MCPServersListPageView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        // Subscribe to templatable MCP server manager state changes
        let templatable_manager = TemplatableMCPServerManager::handle(ctx);
        ctx.subscribe_to_model(&templatable_manager, |me, _, event, ctx| {
            me.handle_templatable_mcp_manager_event(event, ctx);
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
                });

                // Refresh cards when MCP config files are parsed or removed.
                let file_mcp_watcher = FileMCPWatcher::handle(ctx);
                ctx.subscribe_to_model(&file_mcp_watcher, |me, _, event, ctx| match event {
                    FileMCPWatcherEvent::ConfigParsed { .. }
                    | FileMCPWatcherEvent::ConfigRemoved { .. } => {
                        me.refresh_file_based_server_cards(ctx);
                    }
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
        ctx.subscribe_to_model(&appearance, move |me, _, event, ctx| {
            if let AppearanceEvent::ThemeChanged = event {
                let appearance = Appearance::as_ref(ctx);
                let search_bar_styles = UiComponentStyles {
                    background: Some(internal_colors::neutral_2(appearance.theme()).into()),
                    border_color: Some(internal_colors::neutral_4(appearance.theme()).into()),
                    border_radius: Some(CornerRadius::with_all(Radius::Pixels(4.))),
                    padding: Some(Coords {
                        top: 8.,
                        bottom: 8.,
                        left: 12.,
                        right: 12.,
                    }),
                    ..Default::default()
                };
                me.search_bar.update(ctx, |search_bar, _| {
                    search_bar.with_style(search_bar_styles)
                });
            }
        });

        let search_editor_text = TextOptions::ui_text(None, appearance.as_ref(ctx));
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
            file_based_template_cards: Default::default(),
            search_editor,
            search_bar,
            add_button,
            file_based_mcp_toggle: Default::default(),
        };

        me.create_server_cards(ctx);
        me.create_file_based_server_cards(ctx);
        me
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
            server_card_status.into(),
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
                show_update_available_icon_button: false,
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
        }
    }

    fn open_logs_for_server(&self, log_file_path: &PathBuf, ctx: &mut ViewContext<Self>) {
        let window_id = ctx.window_id();
        let Some(workspace_view_handle) = ctx
            .views_of_type::<Workspace>(window_id)
            .and_then(|views| views.first().cloned())
        else {
            log::error!("Could not find workspace when attempting to open MCP logs.");
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
                log::error!("Could not get terminal view handle when attempting to open MCP logs.");
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
            ServerCardEvent::ViewLogs(item_id) => match item_id {
                ServerCardItemId::TemplatableMCP(_) => {
                    log::error!("Viewing logs is not implemented for templatable MCP.");
                }
                ServerCardItemId::TemplatableMCPInstallation(installation_uuid) => {
                    if let Some(template_uuid) = TemplatableMCPServerManager::as_ref(ctx)
                        .get_template_uuid(*installation_uuid)
                    {
                        let log_path = logs::log_file_path_from_uuid(&template_uuid);
                        self.open_logs_for_server(&log_path, ctx);
                    } else {
                        log::error!(
                            "Could not find template_uuid for installation {installation_uuid}"
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
                        log::error!("Could not find installation for file-based server {uuid}");
                    }
                }
            },
            ServerCardEvent::ToggleRunningSwitch(item_id, switch_state) => match item_id {
                ServerCardItemId::TemplatableMCP(_) => {
                    log::error!("Running a server is not implemented for templatable MCP.");
                }
                ServerCardItemId::TemplatableMCPInstallation(uuid) => {
                    self.toggle_server_running_templatable(*uuid, *switch_state, ctx);
                }
                ServerCardItemId::FileBasedMCP(uuid) => {
                    self.toggle_server_running_file_based(*uuid, *switch_state, ctx);
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
                    if let Some(templatable_mcp_server) = templatable_mcp_server {
                        ctx.emit(MCPServersListPageViewEvent::StartInstallation {
                            templatable_mcp_server: templatable_mcp_server.clone(),
                            instructions_in_markdown: None,
                        });
                    }
                }
                ServerCardItemId::TemplatableMCPInstallation(_) => {
                    log::warn!("Installing is not supported for templatable MCP installations.");
                }
            },
            ServerCardEvent::InstallServerUpdate(_) => {}
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

    fn handle_templatable_mcp_manager_event(
        &mut self,
        event: &TemplatableMCPServerManagerEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            TemplatableMCPServerManagerEvent::StateChanged
            | TemplatableMCPServerManagerEvent::ServerInstallationAdded(_)
            | TemplatableMCPServerManagerEvent::ServerInstallationDeleted(_)
            | TemplatableMCPServerManagerEvent::TemplatableMCPServersUpdated => {
                self.refresh_server_cards(ctx);
                self.refresh_file_based_server_cards(ctx);
            }
        }
    }

    pub fn get_modal_content(&self) -> Option<Box<dyn Element>> {
        None
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
                FormattedTextFragment::plain_text("Supported providers are listed above."),
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
        let description_fragments = vec![FormattedTextFragment::plain_text(DESCRIPTION_TEXT)];

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

        let has_any_content =
            !self.server_cards.is_empty() || !filtered_file_based_cards.is_empty();

        if !has_any_content {
            let empty_state = self.render_empty_state(appearance, app);
            page.add_child(empty_state);
        } else {
            page.add_child(self.render_controls());

            if FeatureFlag::FileBasedMcp.is_enabled() {
                page.add_child(self.render_file_based_mcp_section(appearance, app));
            }

            if filtered_server_cards.is_empty() && filtered_file_based_cards.is_empty() {
                page.add_child(Self::render_no_search_results(appearance));
            } else {
                let (owned_server_cards, _shared_server_cards) =
                    Self::separate_server_cards_by_installed(&filtered_server_cards, app);

                if !owned_server_cards.is_empty() {
                    page.add_child(self.render_server_cards_section(
                        "My MCPs",
                        &owned_server_cards,
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
        _app: &AppContext,
    ) -> (
        Vec<ViewHandle<ServerCardView>>,
        Vec<ViewHandle<ServerCardView>>,
    ) {
        let mut owned_server_cards = Vec::new();
        let shared_server_cards = Vec::new();
        for (item_id, server_card) in server_cards {
            match item_id {
                ServerCardItemId::TemplatableMCP(_) => owned_server_cards.push(server_card.clone()),
                ServerCardItemId::TemplatableMCPInstallation(_) => {
                    owned_server_cards.push(server_card.clone());
                }
                ServerCardItemId::FileBasedMCP(_) => {
                    owned_server_cards.push(server_card.clone());
                }
            }
        }
        (owned_server_cards, shared_server_cards)
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
            }
        }

        server_cards.sort_by(|a, b| {
            let a_ref = a.as_ref(app);
            let b_ref = b.as_ref(app);

            priority(a_ref.item_id)
                .cmp(&priority(b_ref.item_id))
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
                ServerCardStatus::AvailableToSave.into(),
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
                show_log_out_icon_button: uses_oauth,
                show_edit_config_icon_button: false,
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
        _item_id: ServerCardItemId,
        _template_uuid: Uuid,
        _ctx: &mut ViewContext<Self>,
    ) -> Option<TitleChip> {
        None
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
