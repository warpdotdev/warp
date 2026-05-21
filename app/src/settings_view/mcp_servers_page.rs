use std::collections::HashMap;
use uuid::Uuid;
use warpui::{
    elements::{ChildView, Container},
    ui_components::components::{Coords, UiComponentStyles},
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};

use crate::{
    ai::mcp::{
        gallery::MCPGalleryManager, templatable_installation::VariableValue, FileBasedMCPManager,
        TemplatableMCPServer, TemplatableMCPServerInstallation, TemplatableMCPServerManager,
    },
    appearance::Appearance,
    cloud_object::Space,
    i18n::I18nKey,
    modal::{Modal, ModalViewState},
    server::cloud_objects::update_manager::InitiatedBy,
    settings_view::{
        mcp_servers::{
            edit_page::{MCPServersEditPageView, MCPServersEditPageViewEvent},
            installation_modal::{InstallationModalBody, InstallationModalBodyEvent},
            list_page::{MCPServersListPageView, MCPServersListPageViewEvent},
            style, ServerCardItemId,
        },
        settings_page::{MatchData, PageType, SettingsPageMeta, SettingsWidget},
        SettingsSection,
    },
    view_components::DismissibleToast,
    workspace::ToastStack,
};

/// Describes where an MCP install request originated.
///
/// Used to decide whether an install request is allowed to bypass the
/// installation modal. In-app gestures (gallery card click, reinstall button)
/// are implicitly confirmed by the click itself. Deeplink-triggered installs
/// are untrusted and must always route through the installation modal so the
/// user can explicitly confirm before any installation or server spawn occurs.
/// See `specs/GH686/product.md`.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum InstallOrigin {
    /// Triggered by a user gesture inside Warp (gallery card click,
    /// reinstall button, programmatic in-app flows, etc.).
    InApp,
    /// Triggered by a `warp://settings/mcp?autoinstall=...` deeplink; must be
    /// gated by an explicit in-app confirmation before install or spawn.
    Deeplink,
}

#[derive(Debug, Default, Copy, Clone)]
pub enum MCPServersSettingsPage {
    #[default]
    List,
    Edit {
        item_id: Option<ServerCardItemId>,
    },
}

#[derive(Debug, Clone)]
pub enum MCPServersSettingsPageEvent {
    ShowModal,
    HideModal,
}

pub struct MCPServersSettingsPageView {
    page: PageType<Self>,
    current_page: MCPServersSettingsPage,
    list_view: ViewHandle<MCPServersListPageView>,
    edit_view: ViewHandle<MCPServersEditPageView>,
    installation_modal_state: ModalViewState<Modal<InstallationModalBody>>,
}

impl MCPServersSettingsPageView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let list_view = ctx.add_typed_action_view(MCPServersListPageView::new);
        ctx.subscribe_to_view(&list_view, |me, _, event, ctx| {
            me.handle_list_view_event(event, ctx);
        });

        let edit_view = ctx.add_typed_action_view(MCPServersEditPageView::new);
        ctx.subscribe_to_view(&edit_view, |me, _, event, ctx| {
            me.handle_edit_view_event(event, ctx);
        });

        let installation_modal_body =
            ctx.add_typed_action_view(|_ctx| InstallationModalBody::new());
        ctx.subscribe_to_view(&installation_modal_body, |me, _, event, ctx| {
            me.handle_installation_modal_body_event(event, ctx);
        });

        let installation_modal = ctx.add_typed_action_view(|ctx| {
            Modal::new(None, installation_modal_body, ctx).with_body_style(UiComponentStyles {
                padding: Some(Coords::uniform(0.)),
                ..Default::default()
            })
        });
        let installation_modal_state = ModalViewState::new(installation_modal);

        Self {
            page: PageType::new_monolith_i18n_title(
                MCPServersSettingsWidget::default(),
                Some(I18nKey::SettingsPageTitleMcpServers),
                true,
            ),
            current_page: MCPServersSettingsPage::default(),
            list_view,
            edit_view,
            installation_modal_state,
        }
    }

    pub fn update_page(&mut self, page: MCPServersSettingsPage, ctx: &mut ViewContext<Self>) {
        self.current_page = page;
        if let MCPServersSettingsPage::Edit { item_id } = page {
            self.edit_view.update(ctx, |edit_view, ctx| {
                edit_view.set_mcp_server(item_id, ctx);
            });
        }
        self.focus(ctx);
        ctx.notify();
    }

    pub fn focus(&mut self, ctx: &mut ViewContext<Self>) {
        match self.current_page {
            MCPServersSettingsPage::List => ctx.focus(&self.list_view),
            MCPServersSettingsPage::Edit { .. } => ctx.focus(&self.edit_view),
        }
    }

    fn add_toast(&mut self, message: &str, ctx: &mut ViewContext<Self>) {
        let window_id = ctx.window_id();
        ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
            toast_stack.add_ephemeral_toast(
                DismissibleToast::default(message.to_string()),
                window_id,
                ctx,
            );
        });
    }

    fn handle_log_out(
        &mut self,
        item_id: ServerCardItemId,
        server_name: Option<String>,
        ctx: &mut ViewContext<Self>,
    ) {
        let message = match server_name {
            Some(name) => format!("Successfully logged out of {name} MCP server"),
            None => "Successfully logged out of MCP server".to_string(),
        };
        match item_id {
            ServerCardItemId::TemplatableMCP(_) => {
                log::error!("Logging out is not supported for template MCP servers.");
            }
            ServerCardItemId::TemplatableMCPInstallation(uuid) => {
                TemplatableMCPServerManager::handle(ctx).update(ctx, |manager, ctx| {
                    manager.delete_credentials_from_secure_storage(uuid, ctx);
                    manager.shutdown_server(uuid, ctx);
                });
                self.add_toast(&message, ctx);
            }
            ServerCardItemId::GalleryMCP(_) => {
                log::error!("Logging out is not supported for gallery MCP servers.");
            }
            ServerCardItemId::FileBasedMCP(uuid) => {
                if let Some(installation) =
                    FileBasedMCPManager::as_ref(ctx).get_installation_by_uuid(uuid)
                {
                    if let Some(hash) = installation.hash() {
                        TemplatableMCPServerManager::handle(ctx).update(ctx, |manager, ctx| {
                            manager.shutdown_server(uuid, ctx);
                            manager.purge_file_based_server_credentials(&vec![hash], ctx);
                        });
                    }
                }
                self.add_toast(&message, ctx);
            }
        }
    }

    fn start_server_installation(
        &mut self,
        templatable_mcp_server: TemplatableMCPServer,
        instructions_in_markdown: Option<String>,
        origin: InstallOrigin,
        ctx: &mut ViewContext<Self>,
    ) {
        let has_variables = !templatable_mcp_server.template.variables.is_empty();
        let has_instructions = instructions_in_markdown.is_some();
        let should_show_modal =
            Self::should_show_install_modal(origin, has_variables, has_instructions);

        if should_show_modal {
            self.installation_modal_state
                .view
                .update(ctx, |modal, ctx| {
                    modal.body().update(ctx, |body, ctx| {
                        body.set_templatable_mcp_server(
                            Some(templatable_mcp_server),
                            instructions_in_markdown,
                            ctx,
                        )
                    });
                });
            self.installation_modal_state.open();
            ctx.focus(&self.installation_modal_state.view);
            ctx.emit(MCPServersSettingsPageEvent::ShowModal);
        } else {
            self.process_server_installation(&templatable_mcp_server, HashMap::new(), ctx);
        }
        ctx.notify();
    }

    /// Decides whether an install request should route through the installation
    /// modal. Deeplink-origin requests always require explicit confirmation via
    /// the modal, regardless of template shape. In-app requests keep the
    /// pre-existing heuristic where the modal is only shown when the template
    /// has variables or markdown instructions; the click gesture itself is the
    /// user confirmation. See `specs/GH686/product.md`.
    pub(crate) fn should_show_install_modal(
        origin: InstallOrigin,
        has_variables: bool,
        has_instructions: bool,
    ) -> bool {
        match origin {
            InstallOrigin::Deeplink => true,
            InstallOrigin::InApp => has_variables || has_instructions,
        }
    }

    fn process_server_installation(
        &mut self,
        templatable_mcp_server: &TemplatableMCPServer,
        variable_values: HashMap<String, VariableValue>,
        ctx: &mut ViewContext<Self>,
    ) -> Option<TemplatableMCPServerInstallation> {
        TemplatableMCPServerManager::handle(ctx).update(ctx, |templatable_manager, ctx| {
            if templatable_manager
                .get_cloud_server(templatable_mcp_server.uuid, ctx)
                .is_none()
            {
                templatable_manager.create_templatable_mcp_server(
                    templatable_mcp_server.clone(),
                    Space::Personal,
                    InitiatedBy::User,
                    ctx,
                );
            }

            let installation = templatable_manager.install_from_template(
                templatable_mcp_server.clone(),
                variable_values.clone(),
                true,
                ctx,
            );
            ctx.notify();
            installation
        })
    }

    pub fn reinstall_server(&mut self, installation_uuid: Uuid, ctx: &mut ViewContext<Self>) {
        let template_uuid =
            TemplatableMCPServerManager::as_ref(ctx).get_template_uuid(installation_uuid);
        if let Some(template_uuid) = template_uuid {
            let templatable_mcp_server =
                TemplatableMCPServerManager::as_ref(ctx).get_templatable_mcp_server(template_uuid);

            if let Some(templatable_mcp_server) = templatable_mcp_server {
                // Reinstall is always an in-app action triggered from the edit page
                // reinstall button; it must not pick up the deeplink confirmation gating.
                self.start_server_installation(
                    templatable_mcp_server.clone(),
                    None,
                    InstallOrigin::InApp,
                    ctx,
                );
            }
        }
    }

    /// Emits an error toast in the current window.
    fn add_error_toast(&mut self, message: String, ctx: &mut ViewContext<Self>) {
        let window_id = ctx.window_id();
        ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
            toast_stack.add_ephemeral_toast(DismissibleToast::error(message), window_id, ctx);
        });
    }

    /// Auto-installs an MCP server from the gallery.
    ///
    /// This is the single sink for `warp://settings/mcp?autoinstall=<title>`
    /// deeplinks; callers must therefore treat the `autoinstall_param` as
    /// untrusted input. The `autoinstall_param` is matched case-insensitively
    /// against gallery titles.
    ///
    /// Every deeplink autoinstall is routed through the installation modal and
    /// requires an explicit user confirmation before any installation or
    /// server spawn occurs — even for templates with no variables and no
    /// markdown instructions. See `specs/GH686/product.md`.
    pub fn autoinstall_from_gallery(
        &mut self,
        autoinstall_param: &str,
        ctx: &mut ViewContext<Self>,
    ) {
        log::info!("Received MCP deeplink autoinstall for value '{autoinstall_param}'");

        // Concurrent deeplink guard: if a prior installation modal is still
        // open, surface a toast and bail so we do not silently overwrite the
        // current modal contents with a different template. See product
        // invariant 7 in specs/GH686/product.md.
        if self.installation_modal_state.is_open() {
            log::warn!(
                "Ignoring MCP deeplink autoinstall for '{autoinstall_param}': installation modal already open"
            );
            self.add_error_toast(
                "Finish the current MCP install before opening another install link.".to_string(),
                ctx,
            );
            return;
        }

        let autoinstall_lower = autoinstall_param.to_lowercase();
        let gallery_server = MCPGalleryManager::as_ref(ctx)
            .get_gallery()
            .into_iter()
            .find(|item| item.title().to_lowercase() == autoinstall_lower);
        let Some(gallery_server) = gallery_server else {
            log::warn!(
                "Unrecognized autoinstall value '{autoinstall_param}': no matching gallery item found"
            );
            self.add_error_toast(format!("Unknown MCP server '{autoinstall_param}'"), ctx);
            return;
        };

        // Skip if this gallery item is already installed.
        let gallery_uuid = gallery_server.uuid();
        let already_installed = TemplatableMCPServerManager::as_ref(ctx)
            .get_installed_templatable_servers()
            .values()
            .any(|installation| installation.gallery_uuid() == Some(gallery_uuid));
        if already_installed {
            log::info!(
                "Gallery MCP server '{}' is already installed, skipping autoinstall",
                gallery_server.title()
            );
            return;
        }

        let instructions = gallery_server.instructions_in_markdown().cloned();
        let gallery_title = gallery_server.title().to_string();
        let Ok(templatable_mcp_server) = TemplatableMCPServer::try_from(gallery_server) else {
            log::warn!(
                "Failed to convert gallery item '{autoinstall_param}' to TemplatableMCPServer"
            );
            // Invariant 5 (specs/GH686/product.md): the match succeeded but the
            // gallery entry cannot be turned into a valid template. Surface the
            // failure to the user rather than silently returning.
            self.add_error_toast(
                format!("MCP server '{gallery_title}' cannot be installed from this link."),
                ctx,
            );
            return;
        };
        log::info!("Opening MCP install confirmation for deeplink gallery title '{gallery_title}'");
        self.start_server_installation(
            templatable_mcp_server,
            instructions,
            InstallOrigin::Deeplink,
            ctx,
        );
    }

    fn handle_list_view_event(
        &mut self,
        event: &MCPServersListPageViewEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            MCPServersListPageViewEvent::Edit(mcp_item_id) => {
                self.update_page(
                    MCPServersSettingsPage::Edit {
                        item_id: Some(*mcp_item_id),
                    },
                    ctx,
                );
            }
            MCPServersListPageViewEvent::Add => {
                self.update_page(MCPServersSettingsPage::Edit { item_id: None }, ctx);
            }
            MCPServersListPageViewEvent::LogOut(server_card_item_id, server_name) => {
                self.handle_log_out(*server_card_item_id, Some(server_name.clone()), ctx);
            }
            MCPServersListPageViewEvent::StartInstallation {
                templatable_mcp_server: template,
                instructions_in_markdown,
                origin,
            } => {
                self.start_server_installation(
                    template.clone(),
                    instructions_in_markdown.clone(),
                    *origin,
                    ctx,
                );
            }
            MCPServersListPageViewEvent::ShowModal => {
                ctx.emit(MCPServersSettingsPageEvent::ShowModal);
            }
            MCPServersListPageViewEvent::HideModal => {
                ctx.emit(MCPServersSettingsPageEvent::HideModal);
            }
        }
    }

    fn handle_edit_view_event(
        &mut self,
        event: &MCPServersEditPageViewEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            MCPServersEditPageViewEvent::Back => {
                self.update_page(MCPServersSettingsPage::List, ctx);
            }
            MCPServersEditPageViewEvent::Reinstall(template_uuid) => {
                self.reinstall_server(*template_uuid, ctx);
            }
            MCPServersEditPageViewEvent::Delete(item_id) => {
                self.list_view.update(ctx, |list_view, ctx| {
                    list_view.delete_server(*item_id, ctx);
                });
                self.update_page(MCPServersSettingsPage::List, ctx);
            }
            MCPServersEditPageViewEvent::LogOut(server_card_item_id, server_name) => {
                self.handle_log_out(*server_card_item_id, server_name.clone(), ctx);
            }
        }
    }

    pub fn get_modal_content(&self, app: &AppContext) -> Option<Box<dyn Element>> {
        if self.installation_modal_state.is_open() {
            Some(self.installation_modal_state.render())
        } else {
            match self.current_page {
                MCPServersSettingsPage::List => self
                    .list_view
                    .read(app, |list_view, _| list_view.get_modal_content()),
                MCPServersSettingsPage::Edit { .. } => None,
            }
        }
    }

    fn handle_installation_modal_body_event(
        &mut self,
        event: &InstallationModalBodyEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            InstallationModalBodyEvent::Install(templatable_mcp_server, variable_values) => {
                // Uninstall the old copy with outdated variable values
                TemplatableMCPServerManager::handle(ctx).update(ctx, |manager, ctx| {
                    let old_installation =
                        manager.get_installation_by_template_uuid(templatable_mcp_server.uuid);
                    if let Some(old_installation) = old_installation {
                        let old_installation_uuid = old_installation.uuid();
                        manager
                            .delete_templatable_mcp_server_installation(old_installation_uuid, ctx);
                        ctx.notify();
                    };
                });

                // Install the copy with new variables
                let new_installation = self.process_server_installation(
                    templatable_mcp_server,
                    variable_values.clone(),
                    ctx,
                );

                // When we re-install, the installation uuid changes, so we should load the edit page with the new installation uuid
                if let Some(new_installation) = new_installation {
                    self.edit_view.update(ctx, |edit_page, ctx| {
                        edit_page.set_mcp_server(
                            Some(ServerCardItemId::TemplatableMCPInstallation(
                                new_installation.uuid(),
                            )),
                            ctx,
                        );
                    });
                }

                self.installation_modal_state
                    .view
                    .update(ctx, |modal, ctx| {
                        modal.body().update(ctx, |body, ctx| {
                            body.set_templatable_mcp_server(None, None, ctx)
                        });
                    });
                self.installation_modal_state.close();
                ctx.emit(MCPServersSettingsPageEvent::HideModal);

                ctx.notify();
            }
            InstallationModalBodyEvent::Cancel => {
                self.installation_modal_state.close();
                ctx.emit(MCPServersSettingsPageEvent::HideModal);
                ctx.notify();
            }
        }
    }
}

impl Entity for MCPServersSettingsPageView {
    type Event = MCPServersSettingsPageEvent;
}

impl View for MCPServersSettingsPageView {
    fn ui_name() -> &'static str {
        "MCPServersSettingsPageView"
    }

    fn render(&self, _app: &AppContext) -> Box<dyn Element> {
        match self.current_page {
            MCPServersSettingsPage::List => self.page.render(self, _app),
            MCPServersSettingsPage::Edit { item_id: _ } => {
                // The edit view needs to be constrained so we will render it directly
                // instead of rendering inside the settings widget
                Container::new(ChildView::new(&self.edit_view).finish())
                    .with_uniform_padding(style::PAGE_PADDING)
                    .finish()
            }
        }
    }
}

impl TypedActionView for MCPServersSettingsPageView {
    type Action = ();
}

impl SettingsPageMeta for MCPServersSettingsPageView {
    fn section() -> SettingsSection {
        SettingsSection::MCPServers
    }

    fn should_render(&self, _ctx: &AppContext) -> bool {
        true
    }

    fn update_filter(&mut self, query: &str, ctx: &mut ViewContext<Self>) -> MatchData {
        self.page.update_filter(query, ctx)
    }

    fn scroll_to_widget(&mut self, widget_id: &'static str) {
        self.page.scroll_to_widget(widget_id)
    }

    fn clear_highlighted_widget(&mut self) {
        self.page.clear_highlighted_widget()
    }
}

#[cfg(test)]
#[path = "mcp_servers_page_tests.rs"]
mod tests;

#[derive(Default)]
pub struct MCPServersSettingsWidget {
    // No state yet
}

impl SettingsWidget for MCPServersSettingsWidget {
    type View = MCPServersSettingsPageView;

    fn search_terms(&self) -> &str {
        "mcp servers"
    }

    fn render(
        &self,
        view: &Self::View,
        _appearance: &Appearance,
        _app: &AppContext,
    ) -> Box<dyn Element> {
        // The settings widget will always return list view
        // The edit view needs to be constrained so we will render it directly
        // instead of rendering inside the settings widget
        ChildView::new(&view.list_view).finish()
    }
}
