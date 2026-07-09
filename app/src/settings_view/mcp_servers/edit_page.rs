use std::collections::HashMap;
use std::path::Path;
#[cfg(feature = "local_fs")]
use std::sync::Arc;

#[cfg(feature = "local_fs")]
#[cfg(not(target_family = "wasm"))]
use diesel::SqliteConnection;
#[cfg(feature = "local_fs")]
use parking_lot::Mutex;
use pathfinder_geometry::vector::vec2f;
use settings::Setting as _;
use uuid::Uuid;
use warp_core::send_telemetry_from_ctx;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::color::internal_colors;
use warp_editor::content::buffer::InitialBufferState;
use warp_editor::render::element::VerticalExpansionBehavior;
use warpui::elements::{
    Border, ChildAnchor, ChildView, Container, CornerRadius, CrossAxisAlignment, Flex,
    MainAxisAlignment, MainAxisSize, MouseStateHandle, OffsetPositioning, ParentAnchor,
    ParentElement, ParentOffsetBounds, Radius, Shrinkable, Stack, Text,
};
use warpui::platform::Cursor;
use warpui::ui_components::components::UiComponent;
use warpui::{
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};

use crate::ai::blocklist::secret_redaction::find_secrets_in_text;
use crate::ai::mcp::parsing::{prettify_json, resolve_json, ParsedTemplatableMCPServerResult};
use crate::ai::mcp::templatable::CloudTemplatableMCPServer;
use crate::ai::mcp::{
    MCPServer, TemplatableMCPServer, TemplatableMCPServerInstallation, TemplatableMCPServerManager,
    TransportType,
};
use crate::banner::{Banner, BannerTextContent};
use crate::cloud_object::{CloudObject, Space};
use crate::code::editor::view::{CodeEditorRenderOptions, CodeEditorView};
use crate::persistence::ModelEvent;
#[cfg(feature = "local_fs")]
use crate::persistence::{database_file_path_for_scope, establish_ro_connection, PersistenceScope};
use crate::server::cloud_objects::update_manager::InitiatedBy;
use crate::server::telemetry::{MCPTemplateCreationSource, TelemetryEvent};
use crate::settings_view::mcp_servers::destructive_mcp_confirmation_dialog::{
    DestructiveMCPConfirmationDialog, DestructiveMCPConfirmationDialogEvent,
    DestructiveMCPConfirmationDialogVariant,
};
use crate::settings_view::mcp_servers::{style, ServerCardItemId};
use crate::terminal::safe_mode_settings::SafeModeSettings;
use crate::ui_components::buttons::icon_button;
use crate::ui_components::icons::Icon;
use crate::view_components::action_button::{
    ActionButton, DangerNakedTheme, DangerSecondaryTheme, PrimaryTheme,
};
use crate::view_components::DismissibleToast;
use crate::workspace::ToastStack;
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::{report_error, GlobalResourceHandlesProvider};

const DEFAULT_JSON_TEXT: &str = r#"{
    "": {
        "serverUrl": ""
    }
}
"#;

#[derive(Debug, Clone)]
pub enum MCPServersEditPageViewEvent {
    Back,
    Reinstall(Uuid),
    Delete(ServerCardItemId),
    LogOut(ServerCardItemId, Option<String>),
}

#[derive(Debug, Clone)]
pub enum MCPServersEditPageViewAction {
    Back,
    Reinstall,
    Save,
    Delete,
    Unshare,
    LogOut,
}

#[allow(clippy::large_enum_variant)]
pub enum ServerModel {
    CloudTemplatableMCPServer(CloudTemplatableMCPServer),
    LocalTemplatableMCPInstallation(TemplatableMCPServerInstallation),
    None,
}

impl ServerModel {
    pub fn name(&self) -> Option<String> {
        match self {
            ServerModel::CloudTemplatableMCPServer(cloud_templatable_server) => {
                Some(cloud_templatable_server.display_name())
            }
            ServerModel::LocalTemplatableMCPInstallation(templatable_mcp_server_installation) => {
                Some(
                    templatable_mcp_server_installation
                        .templatable_mcp_server()
                        .name
                        .clone(),
                )
            }
            ServerModel::None => None,
        }
    }
}

pub struct MCPServersEditPageView {
    server_card_item_id: Option<ServerCardItemId>,
    server_model: ServerModel,
    save_button: ViewHandle<ActionButton>,
    reinstall_button: ViewHandle<ActionButton>,
    delete_button: ViewHandle<ActionButton>,
    unshare_button: ViewHandle<ActionButton>,
    back_button: MouseStateHandle,
    json_editor: ViewHandle<CodeEditorView>,
    destructive_mcp_confirmation_dialog: ViewHandle<DestructiveMCPConfirmationDialog>,
    log_out_icon_button_mouse_handle: MouseStateHandle,
    editing_disabled_banner: ViewHandle<Banner<()>>,

    #[cfg(feature = "local_fs")]
    #[allow(dead_code)]
    database_connection: Option<Arc<Mutex<SqliteConnection>>>,
}

impl MCPServersEditPageView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let save_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Save", PrimaryTheme)
                .with_icon(Icon::Check)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(MCPServersEditPageViewAction::Save);
                })
        });

        let reinstall_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Edit Variables", PrimaryTheme).on_click(|ctx| {
                ctx.dispatch_typed_action(MCPServersEditPageViewAction::Reinstall);
            })
        });

        let delete_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Delete MCP", DangerSecondaryTheme)
                .with_icon(Icon::Trash)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(MCPServersEditPageViewAction::Delete);
                })
        });

        let unshare_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Remove from team", DangerNakedTheme)
                .with_icon(Icon::MinusCircle)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(MCPServersEditPageViewAction::Unshare);
                })
        });

        let json_editor = ctx.add_typed_action_view(|ctx| {
            #[cfg_attr(target_family = "wasm", allow(unused_mut))]
            let mut editor = CodeEditorView::new(
                None,
                None,
                CodeEditorRenderOptions::new(VerticalExpansionBehavior::FillMaxHeight),
                ctx,
            )
            .with_horizontal_scrollbar_appearance(
                warpui::elements::new_scrollable::ScrollableAppearance::new(
                    warpui::elements::ScrollbarWidth::Auto,
                    true,
                ),
            );
            editor.set_language_with_local_path(Path::new("/mcp.json"), ctx);
            editor
        });

        let destructive_mcp_confirmation_dialog =
            ctx.add_typed_action_view(DestructiveMCPConfirmationDialog::new);
        ctx.subscribe_to_view(&destructive_mcp_confirmation_dialog, |me, _, event, ctx| {
            me.handle_delete_confirmation_event(event, ctx);
        });

        let editing_disabled_banner = ctx.add_typed_action_view(|_| {
            Banner::new_without_close(BannerTextContent::plain_text(
                "Only team admins and the creator of the MCP server can edit the MCP server.",
            ))
            .with_icon(Icon::Warning)
        });

        #[cfg(feature = "local_fs")]
        let database_connection = database_file_path_for_scope(&PersistenceScope::App)
            .to_str()
            .and_then(|db_url| {
                establish_ro_connection(db_url)
                    .ok()
                    .map(|conn| Arc::new(Mutex::new(conn)))
            });

        Self {
            server_card_item_id: None,
            server_model: ServerModel::None,
            save_button,
            reinstall_button,
            delete_button,
            unshare_button,
            back_button: Default::default(),
            json_editor,
            destructive_mcp_confirmation_dialog,
            log_out_icon_button_mouse_handle: Default::default(),
            editing_disabled_banner,

            #[cfg(feature = "local_fs")]
            database_connection,
        }
    }

    pub fn set_mcp_server(
        &mut self,
        item_id: Option<ServerCardItemId>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.server_card_item_id = item_id;
        match item_id {
            Some(ServerCardItemId::TemplatableMCP(template_uuid)) => {
                let cloud_templatable_mcp_server = TemplatableMCPServerManager::as_ref(ctx)
                    .get_cloud_templatable_mcp_server(template_uuid);

                if let Some(cloud_templatable_mcp_server) = cloud_templatable_mcp_server {
                    self.server_model = ServerModel::CloudTemplatableMCPServer(
                        cloud_templatable_mcp_server.clone(),
                    );
                    let templatable_mcp_server = &cloud_templatable_mcp_server.model().string_model;
                    let json = templatable_mcp_server.to_user_json();

                    self.json_editor.update(ctx, |view, ctx| {
                        let state = InitialBufferState::plain_text(&json);
                        view.reset(state, ctx);
                    });
                }
            }
            Some(ServerCardItemId::TemplatableMCPInstallation(installation_uuid)) => {
                let installation = TemplatableMCPServerManager::as_ref(ctx)
                    .get_installed_server(&installation_uuid);

                if let Some(installation) = installation {
                    self.server_model =
                        ServerModel::LocalTemplatableMCPInstallation(installation.clone());
                    // This shouldn't be necessary for newly created mcps but some older ones may not have been saved with pretty json
                    let resolved_json = prettify_json(&resolve_json(installation));

                    self.json_editor.update(ctx, |view, ctx| {
                        let state = InitialBufferState::plain_text(&resolved_json);
                        view.reset(state, ctx);
                    });
                }
            }
            Some(ServerCardItemId::GalleryMCP(_uuid)) => {
                log::warn!("Editing of gallery MCP unimplemented");
            }
            Some(ServerCardItemId::FileBasedMCP(_)) => {
                log::warn!("Editing of file-based MCP unimplemented");
            }
            None => {
                self.server_model = ServerModel::None;
                self.json_editor.update(ctx, |view, ctx| {
                    let state = InitialBufferState::plain_text(DEFAULT_JSON_TEXT);
                    view.reset(state, ctx);
                });
            }
        }

        if Self::is_editable(item_id, ctx) {
            self.json_editor.update(ctx, |editor, ctx| {
                editor.set_interaction_state(crate::editor::InteractionState::Editable, ctx);
            });
        } else {
            self.json_editor.update(ctx, |editor, ctx| {
                editor.set_interaction_state(crate::editor::InteractionState::Selectable, ctx);
            });
        }

        ctx.notify();
    }

    fn should_show_oauth_components(&self, ctx: &AppContext) -> bool {
        if let Some(item_id) = self.server_card_item_id {
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
                ServerCardItemId::GalleryMCP(_) | ServerCardItemId::FileBasedMCP(_) => false,
            }
        } else {
            false
        }
    }

    fn render_header(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let title = if self.server_card_item_id.is_none() {
            "Add New MCP Server".to_string()
        } else if let Some(name) = self.server_model.name() {
            format!("Edit {name} MCP Server")
        } else {
            "Edit MCP Server".to_string()
        };

        let ui_builder = appearance.ui_builder().clone();
        let log_out_icon_button = icon_button(
            appearance,
            Icon::LogOut,
            false,
            self.log_out_icon_button_mouse_handle.clone(),
        )
        .with_tooltip(move || ui_builder.tool_tip("Log out".to_string()).build().finish())
        .build()
        .on_click(|ctx, _, _| ctx.dispatch_typed_action(MCPServersEditPageViewAction::LogOut))
        .finish();

        let mut rhs_row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(style::PAGE_SPACING);
        if self.should_show_oauth_components(app) {
            rhs_row.add_child(log_out_icon_button);
        }
        if Self::is_editable(self.server_card_item_id, app) {
            rhs_row.add_child(
                Container::new(ChildView::new(&self.save_button).finish())
                    .with_margin_left(style::EDIT_PAGE_BUTTON_SPACING)
                    .finish(),
            );
        } else if Self::is_reinstallable(self.server_card_item_id, app) {
            rhs_row.add_child(ChildView::new(&self.reinstall_button).finish());
        }

        Container::new(
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(
                    Flex::row()
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .with_child(self.render_back_button(appearance))
                        .with_child(
                            appearance
                                .ui_builder()
                                .wrappable_text(title, true)
                                .with_style(style::header_text())
                                .build()
                                .finish(),
                        )
                        .finish(),
                )
                .with_child(rhs_row.finish())
                .finish(),
        )
        .with_margin_bottom(style::ITEM_BOTTOM_MARGIN)
        .finish()
    }

    fn render_back_button(&self, appearance: &Appearance) -> Box<dyn Element> {
        let button = icon_button(appearance, Icon::ArrowLeft, false, self.back_button.clone());
        Container::new(
            button
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(MCPServersEditPageViewAction::Back);
                })
                .with_cursor(Cursor::PointingHand)
                .finish(),
        )
        .with_margin_right(style::ICON_MARGIN)
        .finish()
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

    fn is_editable(item_id: Option<ServerCardItemId>, app: &AppContext) -> bool {
        match item_id {
            Some(ServerCardItemId::TemplatableMCPInstallation(installation_uuid)) => {
                let template_uuid =
                    TemplatableMCPServerManager::as_ref(app).get_template_uuid(installation_uuid);

                if let Some(template_uuid) = template_uuid {
                    let is_authorized_editor = TemplatableMCPServerManager::as_ref(app)
                        .is_authorized_editor(template_uuid, app);
                    let is_shared = TemplatableMCPServerManager::as_ref(app)
                        .is_server_template_shared(template_uuid, app);

                    is_authorized_editor || !is_shared
                } else {
                    false
                }
            }
            Some(ServerCardItemId::TemplatableMCP(template_uuid)) => {
                let is_shared = TemplatableMCPServerManager::as_ref(app)
                    .is_server_template_shared(template_uuid, app);
                let is_authorized_editor = TemplatableMCPServerManager::as_ref(app)
                    .is_authorized_editor(template_uuid, app);

                is_authorized_editor || !is_shared
            }
            Some(ServerCardItemId::GalleryMCP(_)) | Some(ServerCardItemId::FileBasedMCP(_)) => {
                false
            }
            None => true,
        }
    }

    fn is_reinstallable(item_id: Option<ServerCardItemId>, app: &AppContext) -> bool {
        if let Some(ServerCardItemId::TemplatableMCPInstallation(installation_uuid)) = item_id {
            let installation =
                TemplatableMCPServerManager::as_ref(app).get_installed_server(&installation_uuid);
            if let Some(installation) = installation {
                let has_variables = !installation
                    .templatable_mcp_server()
                    .template
                    .variables
                    .is_empty();
                return has_variables;
            }
        }
        false
    }

    fn is_deletable(item_id: ServerCardItemId, app: &AppContext) -> bool {
        Self::is_editable(Some(item_id), app)
    }

    fn is_unshareable(item_id: ServerCardItemId, app: &AppContext) -> bool {
        let is_shared = Self::is_shared(item_id, app);
        let template_uuid = match item_id {
            ServerCardItemId::TemplatableMCP(template_uuid) => Some(template_uuid),
            ServerCardItemId::TemplatableMCPInstallation(installation_uuid) => {
                TemplatableMCPServerManager::as_ref(app).get_template_uuid(installation_uuid)
            }
            _ => None,
        };
        let is_author = template_uuid
            .map(|template_uuid| {
                TemplatableMCPServerManager::as_ref(app).is_author(template_uuid, app)
            })
            .unwrap_or(false);

        is_author && is_shared
    }

    fn render_editor(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let ui_font_family = appearance.ui_font_family();
        let font_size = appearance.ui_font_size();
        let border_color = internal_colors::neutral_4(theme);

        Container::new(
            Flex::column()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_child(
                    Container::new(
                        Container::new(Text::new("JSON", ui_font_family, font_size).finish())
                            .with_vertical_padding(10.)
                            .with_horizontal_padding(16.)
                            .finish(),
                    )
                    .with_background_color(border_color)
                    .finish(),
                )
                .with_child(
                    Shrinkable::new(
                        1.,
                        Container::new(ChildView::new(&self.json_editor).finish())
                            .with_vertical_padding(style::EDITOR_VERTICAL_PADDING)
                            .finish(),
                    )
                    .finish(),
                )
                .finish(),
        )
        .with_border(Border::all(1.).with_border_color(border_color))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
        .finish()
    }

    fn render_footer(&self, app: &AppContext) -> Box<dyn Element> {
        let mut footer = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(style::EDIT_PAGE_BUTTON_SPACING);

        if let Some(server_card_item_id) = self.server_card_item_id {
            if Self::is_deletable(server_card_item_id, app) {
                footer.add_child(ChildView::new(&self.delete_button).finish());
            }
            if Self::is_unshareable(server_card_item_id, app) {
                footer.add_child(ChildView::new(&self.unshare_button).finish());
            }
        }

        footer.finish()
    }

    fn detect_secrets_in_templatable_mcp_server(
        &self,
        ctx: &mut ViewContext<Self>,
        templatable_mcp_server: &TemplatableMCPServer,
    ) -> Result<(), String> {
        let safe_mode_enabled = *SafeModeSettings::as_ref(ctx).safe_mode_enabled.value();
        let enterprise_enforced =
            UserWorkspaces::as_ref(ctx).is_enterprise_secret_redaction_enabled();
        let contains_secrets =
            !find_secrets_in_text(&templatable_mcp_server.template.json).is_empty();

        if should_block_save_for_secrets(safe_mode_enabled, enterprise_enforced, contains_secrets) {
            let window_id = ctx.window_id();
            ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                toast_stack.add_ephemeral_toast(
                    DismissibleToast::error("This MCP server contains secrets. Visit Settings > Privacy to modify your secret redaction settings.".to_string()),
                    window_id,
                    ctx,
                );
            });
            return Err("This MCP server contains secrets. Visit Settings > Privacy to modify your secret redaction settings.".to_string());
        }

        Ok(())
    }

    fn parse_templatable_json(
        &self,
        ctx: &mut ViewContext<Self>,
        json: &str,
    ) -> Vec<ParsedTemplatableMCPServerResult> {
        let parsed_templatable_mcp_servers =
            match ParsedTemplatableMCPServerResult::from_user_json(json) {
                Ok(parsed_servers) => parsed_servers,
                Err(error) => {
                    let window_id = ctx.window_id();
                    ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                        toast_stack.add_ephemeral_toast(
                            DismissibleToast::error(error.to_string()),
                            window_id,
                            ctx,
                        );
                    });
                    return vec![];
                }
            };

        for parsed_templatable_mcp_server_result in parsed_templatable_mcp_servers.iter() {
            if self
                .detect_secrets_in_templatable_mcp_server(
                    ctx,
                    &parsed_templatable_mcp_server_result.templatable_mcp_server,
                )
                .is_err()
            {
                return vec![];
            }
        }

        // TODO(Pei): Stop and start servers

        parsed_templatable_mcp_servers
    }

    fn build_templatable_mcp_server_result_from_json(
        &self,
        ctx: &mut ViewContext<Self>,
        json: &str,
    ) -> Result<ParsedTemplatableMCPServerResult, String> {
        let parsed_templatable_mcp_servers = self.parse_templatable_json(ctx, json);

        if parsed_templatable_mcp_servers.is_empty() {
            let window_id = ctx.window_id();
            ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                toast_stack.add_ephemeral_toast(
                    DismissibleToast::error("No MCP Server specified.".to_string()),
                    window_id,
                    ctx,
                );
            });

            return Err("No MCP Server specified.".to_string());
        }

        if parsed_templatable_mcp_servers.len() > 1 {
            let window_id = ctx.window_id();
            ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                toast_stack.add_ephemeral_toast(
                    DismissibleToast::error(
                        "Cannot add multiple MCP servers while editing a single server."
                            .to_string(),
                    ),
                    window_id,
                    ctx,
                );
            });

            return Err(
                "Cannot add multiple MCP servers while editing a single server.".to_string(),
            );
        }

        Ok(parsed_templatable_mcp_servers[0].clone())
    }

    fn handle_delete_confirmation_event(
        &mut self,
        event: &DestructiveMCPConfirmationDialogEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            DestructiveMCPConfirmationDialogEvent::Cancel => {
                self.destructive_mcp_confirmation_dialog
                    .update(ctx, |dialog, ctx| {
                        dialog.hide(ctx);
                    });
                ctx.notify();
            }
            DestructiveMCPConfirmationDialogEvent::Confirm(variant) => {
                if let Some(server_card_item_id) = self.server_card_item_id {
                    match variant {
                        DestructiveMCPConfirmationDialogVariant::DeleteLocal
                        | DestructiveMCPConfirmationDialogVariant::DeleteShared => {
                            ctx.emit(MCPServersEditPageViewEvent::Delete(server_card_item_id));
                        }
                        DestructiveMCPConfirmationDialogVariant::Unshare => {
                            match server_card_item_id {
                                ServerCardItemId::TemplatableMCP(template_uuid) => {
                                    TemplatableMCPServerManager::handle(ctx).update(
                                        ctx,
                                        |templatable_manager, ctx| {
                                            templatable_manager
                                                .unshare_templatable_mcp_server(template_uuid, ctx);
                                        },
                                    );
                                    ctx.emit(MCPServersEditPageViewEvent::Back);
                                }
                                ServerCardItemId::TemplatableMCPInstallation(installation_uuid) => {
                                    TemplatableMCPServerManager::handle(ctx).update(
                                        ctx,
                                        |templatable_manager, ctx| {
                                            templatable_manager
                                                .unshare_templatable_mcp_server_installation(
                                                    installation_uuid,
                                                    ctx,
                                                );
                                        },
                                    );
                                    ctx.emit(MCPServersEditPageViewEvent::Back);
                                }
                                _ => {
                                    log::warn!(
                                        "This server is not an installation and cannot be unshared"
                                    );
                                }
                            }
                        }
                    }
                    self.destructive_mcp_confirmation_dialog
                        .update(ctx, |dialog, ctx| {
                            dialog.hide(ctx);
                        });
                    ctx.notify();
                }
            }
        }
    }

    pub fn save_mcp_server_env_vars(mcp_server: MCPServer, ctx: &mut ViewContext<Self>) {
        if let TransportType::CLIServer(cli_server) = &mcp_server.transport_type {
            let env_vars: HashMap<String, String> = cli_server
                .static_env_vars
                .iter()
                .map(|env_var| (env_var.name.clone(), env_var.value.clone()))
                .collect();
            let Ok(env_vars_string) = serde_json::to_string(&env_vars) else {
                report_error!("Could not serialize MCP env vars");
                return;
            };
            let global_resource_handles = GlobalResourceHandlesProvider::as_ref(ctx).get().clone();

            if let Some(model_event_sender) = &global_resource_handles.model_event_sender {
                if let Err(e) =
                    model_event_sender.send(ModelEvent::UpsertMCPServerEnvironmentVariables {
                        mcp_server_uuid: mcp_server.uuid.as_bytes().to_vec(),
                        environment_variables: env_vars_string,
                    })
                {
                    report_error!(anyhow::Error::new(e)
                        .context("Error persisting MCP server env vars to database"));
                };
            }
        }
    }

    fn handle_save_templatable_mcp_server(
        &mut self,
        ctx: &mut ViewContext<Self>,
        template_uuid: Uuid,
    ) -> Result<(), String> {
        let json = self.json_editor.as_ref(ctx).text(ctx).into_string();
        let parsed_result = self.build_templatable_mcp_server_result_from_json(ctx, &json)?;

        let original_template =
            TemplatableMCPServerManager::as_ref(ctx).get_templatable_mcp_server(template_uuid);
        let gallery_data = original_template.and_then(|template| template.gallery_data);

        TemplatableMCPServerManager::handle(ctx).update(ctx, |templatable_manager, ctx| {
            let templatable_mcp_server = TemplatableMCPServer {
                uuid: template_uuid,
                name: parsed_result.templatable_mcp_server.name,
                description: parsed_result.templatable_mcp_server.description,
                template: parsed_result.templatable_mcp_server.template,
                version: parsed_result.templatable_mcp_server.version,
                gallery_data,
            };

            if let Some(old_installation) =
                templatable_manager.get_installation_by_template_uuid(template_uuid)
            {
                templatable_manager
                    .delete_templatable_mcp_server_installation(old_installation.uuid(), ctx);
            }

            templatable_manager.update_templatable_mcp_server(templatable_mcp_server.clone(), ctx);

            if let Some(new_installation) = parsed_result.templatable_mcp_server_installation {
                templatable_manager.install_from_template(
                    templatable_mcp_server.clone(),
                    new_installation.variable_values().clone(),
                    true,
                    ctx,
                );
            }
        });

        Ok(())
    }
}

impl Entity for MCPServersEditPageView {
    type Event = MCPServersEditPageViewEvent;
}

impl View for MCPServersEditPageView {
    fn ui_name() -> &'static str {
        "MCPServersEditPageView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let header = self.render_header(app);
        let editor = self.render_editor(app);
        let footer = self.render_footer(app);

        let mut main_content = Flex::column()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_spacing(style::PAGE_SPACING);
        main_content.add_child(header);
        if !Self::is_editable(self.server_card_item_id, app) {
            main_content.add_child(ChildView::new(&self.editing_disabled_banner).finish());
        }
        main_content.add_child(Shrinkable::new(1., editor).finish());
        main_content.add_child(footer);

        let mut stack = Stack::new();
        stack.add_child(Container::new(main_content.finish()).finish());
        stack.add_positioned_overlay_child(
            ChildView::new(&self.destructive_mcp_confirmation_dialog).finish(),
            OffsetPositioning::offset_from_parent(
                vec2f(0., 0.),
                ParentOffsetBounds::Unbounded,
                ParentAnchor::Center,
                ChildAnchor::Center,
            ),
        );
        stack.finish()
    }
}

impl TypedActionView for MCPServersEditPageView {
    type Action = MCPServersEditPageViewAction;

    fn handle_action(
        &mut self,
        action: &MCPServersEditPageViewAction,
        ctx: &mut ViewContext<Self>,
    ) {
        match action {
            MCPServersEditPageViewAction::Back => {
                ctx.emit(MCPServersEditPageViewEvent::Back);
            }
            MCPServersEditPageViewAction::Delete => {
                let Some(server_card_item_id) = self.server_card_item_id else {
                    return;
                };
                let is_shared = Self::is_shared(server_card_item_id, ctx);

                let variant = if is_shared {
                    DestructiveMCPConfirmationDialogVariant::DeleteShared
                } else {
                    DestructiveMCPConfirmationDialogVariant::DeleteLocal
                };

                self.destructive_mcp_confirmation_dialog
                    .update(ctx, |dialog, ctx| {
                        dialog.show(variant, ctx);
                    });
                ctx.notify();
            }
            MCPServersEditPageViewAction::Unshare => {
                self.destructive_mcp_confirmation_dialog
                    .update(ctx, |dialog, ctx| {
                        dialog.show(DestructiveMCPConfirmationDialogVariant::Unshare, ctx);
                    });
                ctx.notify();
            }
            MCPServersEditPageViewAction::Reinstall => {
                if let Some(ServerCardItemId::TemplatableMCPInstallation(uuid)) =
                    self.server_card_item_id
                {
                    ctx.emit(MCPServersEditPageViewEvent::Reinstall(uuid));
                }
            }
            MCPServersEditPageViewAction::Save => match self.server_card_item_id {
                Some(ServerCardItemId::TemplatableMCP(template_uuid)) => {
                    let result = self.handle_save_templatable_mcp_server(ctx, template_uuid);
                    if result.is_ok() {
                        ctx.emit(MCPServersEditPageViewEvent::Back);
                    }
                }
                Some(ServerCardItemId::TemplatableMCPInstallation(installation_uuid)) => {
                    let template_uuid = TemplatableMCPServerManager::as_ref(ctx)
                        .get_installed_server(&installation_uuid)
                        .map(|installation| installation.template_uuid());

                    if let Some(template_uuid) = template_uuid {
                        let result = self.handle_save_templatable_mcp_server(ctx, template_uuid);
                        if result.is_ok() {
                            ctx.emit(MCPServersEditPageViewEvent::Back);
                        }
                    }
                }
                Some(ServerCardItemId::GalleryMCP(_uuid)) => {
                    log::warn!("Editing of gallery MCP unimplemented");
                }
                Some(ServerCardItemId::FileBasedMCP(_)) => {
                    log::warn!("Editing of file-based MCP unimplemented");
                }
                None => {
                    // This is a new MCP server, we should treat it like a legacy MCP server
                    let json = self.json_editor.as_ref(ctx).text(ctx).into_string();

                    let parsed_servers =
                        match ParsedTemplatableMCPServerResult::from_user_json(&json) {
                            Ok(parsed_templatable_mcp_servers) => parsed_templatable_mcp_servers,
                            Err(error) => {
                                let window_id = ctx.window_id();
                                ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                                    toast_stack.add_ephemeral_toast(
                                        DismissibleToast::error(error.to_string()),
                                        window_id,
                                        ctx,
                                    );
                                });
                                return;
                            }
                        };

                    if parsed_servers.is_empty() {
                        let window_id = ctx.window_id();
                        ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                            toast_stack.add_ephemeral_toast(
                                DismissibleToast::error("No MCP Server specified.".to_string()),
                                window_id,
                                ctx,
                            );
                        });
                        return;
                    }

                    if parsed_servers
                        .iter()
                        .try_for_each(|parsed_server| {
                            self.detect_secrets_in_templatable_mcp_server(
                                ctx,
                                &parsed_server.templatable_mcp_server,
                            )
                        })
                        .is_err()
                    {
                        return;
                    }

                    for parsed_server in parsed_servers {
                        TemplatableMCPServerManager::handle(ctx).update(
                            ctx,
                            |templatable_manager, ctx| {
                                templatable_manager.create_templatable_mcp_server(
                                    parsed_server.templatable_mcp_server.clone(),
                                    Space::Personal,
                                    InitiatedBy::User,
                                    ctx,
                                );
                                if let Some(installation) =
                                    parsed_server.templatable_mcp_server_installation
                                {
                                    templatable_manager.install_from_template(
                                        installation.templatable_mcp_server().clone(),
                                        installation.variable_values().clone(),
                                        true,
                                        ctx,
                                    );
                                }
                            },
                        );
                        send_telemetry_from_ctx!(
                            TelemetryEvent::MCPTemplateCreated {
                                source: MCPTemplateCreationSource::Json,
                                variables: parsed_server.templatable_mcp_server.template.variables,
                                name: parsed_server.templatable_mcp_server.name,
                            },
                            ctx
                        );
                    }

                    ctx.emit(MCPServersEditPageViewEvent::Back);
                }
            },
            MCPServersEditPageViewAction::LogOut => {
                if let Some(item_id) = self.server_card_item_id {
                    ctx.emit(MCPServersEditPageViewEvent::LogOut(
                        item_id,
                        self.server_model.name(),
                    ));
                }
            }
        }
    }
}

/// Decide whether to block saving an MCP server config because secret
/// redaction is in force AND the parsed config contains secret-shaped strings.
///
/// We block only when redaction is actually active — either the user-level
/// Settings > Privacy > Secret redaction toggle is on, or the user's workspace
/// has enterprise enforcement enabled. With both off, the user has explicitly
/// opted to embed secrets in the config and we save it as written (#8761).
fn should_block_save_for_secrets(
    safe_mode_enabled: bool,
    enterprise_enforced: bool,
    contains_secrets: bool,
) -> bool {
    (safe_mode_enabled || enterprise_enforced) && contains_secrets
}

#[cfg(test)]
#[path = "edit_page_tests.rs"]
mod tests;
