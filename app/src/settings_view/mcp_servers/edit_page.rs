use std::{collections::HashMap, path::Path};

use pathfinder_geometry::vector::vec2f;
use uuid::Uuid;
use warp_core::ui::{appearance::Appearance, theme::color::internal_colors};
use warp_editor::{
    content::buffer::InitialBufferState, render::element::VerticalExpansionBehavior,
};
use warpui::{
    elements::{
        Border, ChildAnchor, ChildView, Container, CornerRadius, CrossAxisAlignment, Flex,
        MainAxisAlignment, MainAxisSize, MouseStateHandle, OffsetPositioning, ParentAnchor,
        ParentElement, ParentOffsetBounds, Radius, Shrinkable, Stack, Text,
    },
    platform::Cursor,
    ui_components::components::UiComponent,
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};

use crate::{
    ai::{
        blocklist::secret_redaction::find_secrets_in_text,
        mcp::{
            parsing::{prettify_json, resolve_json, ParsedTemplatableMCPServerResult},
            MCPServer, TemplatableMCPServer, TemplatableMCPServerInstallation,
            TemplatableMCPServerManager, TransportType,
        },
    },
    banner::{Banner, BannerTextContent},
    code::editor::view::{CodeEditorRenderOptions, CodeEditorView},
    persistence::ModelEvent,
    settings_view::mcp_servers::{
        destructive_mcp_confirmation_dialog::{
            DestructiveMCPConfirmationDialog, DestructiveMCPConfirmationDialogEvent,
            DestructiveMCPConfirmationDialogVariant,
        },
        style, ServerCardItemId,
    },
    ui_components::{buttons::icon_button, icons::Icon},
    view_components::{
        action_button::{ActionButton, DangerSecondaryTheme, PrimaryTheme},
        DismissibleToast,
    },
    workspace::ToastStack,
    GlobalResourceHandlesProvider,
};

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
    LogOut,
}

#[allow(clippy::large_enum_variant)]
pub enum ServerModel {
    TemplatableMCPServer(TemplatableMCPServer),
    LocalTemplatableMCPInstallation(TemplatableMCPServerInstallation),
    None,
}

impl ServerModel {
    pub fn name(&self) -> Option<String> {
        match self {
            ServerModel::TemplatableMCPServer(templatable_server) => {
                Some(templatable_server.display_name())
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
    back_button: MouseStateHandle,
    json_editor: ViewHandle<CodeEditorView>,
    destructive_mcp_confirmation_dialog: ViewHandle<DestructiveMCPConfirmationDialog>,
    log_out_icon_button_mouse_handle: MouseStateHandle,
    editing_disabled_banner: ViewHandle<Banner<()>>,
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

        let json_editor = ctx.add_typed_action_view(|ctx| {
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
            editor.set_language_with_path(Path::new("mcp.json"), ctx);
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

        Self {
            server_card_item_id: None,
            server_model: ServerModel::None,
            save_button,
            reinstall_button,
            delete_button,
            back_button: Default::default(),
            json_editor,
            destructive_mcp_confirmation_dialog,
            log_out_icon_button_mouse_handle: Default::default(),
            editing_disabled_banner,
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
                let templatable_mcp_server = TemplatableMCPServerManager::as_ref(ctx)
                    .get_templatable_mcp_server(template_uuid);

                if let Some(templatable_mcp_server) = templatable_mcp_server {
                    self.server_model =
                        ServerModel::TemplatableMCPServer(templatable_mcp_server.clone());
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
                ServerCardItemId::FileBasedMCP(_) => false,
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
        .with_tooltip(move || {
            ui_builder
                .tool_tip("Disconnect".to_string())
                .build()
                .finish()
        })
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

    fn is_editable(item_id: Option<ServerCardItemId>, _app: &AppContext) -> bool {
        match item_id {
            Some(ServerCardItemId::TemplatableMCPInstallation(_))
            | Some(ServerCardItemId::TemplatableMCP(_)) => true,
            Some(ServerCardItemId::FileBasedMCP(_)) => false,
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
        }

        footer.finish()
    }

    fn detect_secrets_in_templatable_mcp_server(
        &self,
        ctx: &mut ViewContext<Self>,
        templatable_mcp_server: &TemplatableMCPServer,
    ) -> Result<(), String> {
        let contains_secrets =
            !find_secrets_in_text(&templatable_mcp_server.template.json).is_empty();

        if contains_secrets {
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
                        DestructiveMCPConfirmationDialogVariant::DeleteLocal => {
                            ctx.emit(MCPServersEditPageViewEvent::Delete(server_card_item_id));
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
                log::error!("Could not serialize MCP env vars");
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
                    log::error!("Error persisting MCP server env vars to database: {e:?}");
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

        TemplatableMCPServerManager::handle(ctx).update(ctx, |templatable_manager, ctx| {
            let templatable_mcp_server = TemplatableMCPServer {
                uuid: template_uuid,
                name: parsed_result.templatable_mcp_server.name,
                description: parsed_result.templatable_mcp_server.description,
                template: parsed_result.templatable_mcp_server.template,
                version: parsed_result.templatable_mcp_server.version,
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
                if self.server_card_item_id.is_none() {
                    return;
                }
                self.destructive_mcp_confirmation_dialog
                    .update(ctx, |dialog, ctx| {
                        dialog.show(DestructiveMCPConfirmationDialogVariant::DeleteLocal, ctx);
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

                    for parsed_server in parsed_servers {
                        TemplatableMCPServerManager::handle(ctx).update(
                            ctx,
                            |templatable_manager, ctx| {
                                templatable_manager.create_templatable_mcp_server(
                                    parsed_server.templatable_mcp_server.clone(),
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
