use pathfinder_geometry::vector::vec2f;
use warp_core::ui::{self, appearance::Appearance, color::blend::Blend as _};
use warpui::{
    elements::{
        Align, ChildView, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Flex, Icon,
        MainAxisAlignment, MainAxisSize, MouseStateHandle, ParentElement as _, Radius,
    },
    keymap::EditableBinding,
    platform::Cursor,
    ui_components::{
        button::{ButtonVariant, TextAndIcon, TextAndIconAlignment},
        components::{Coords, UiComponent, UiComponentStyles},
    },
    AppContext, Element, Entity, ModelHandle, SingletonEntity as _, TypedActionView, View,
    ViewContext, ViewHandle,
};

use crate::{
    coding_entrypoints::{
        clone_repo_view::{CloneRepoEvent, CloneRepoView},
        create_project_view::{CreateProjectEvent, CreateProjectView},
        project_buttons::{ProjectButtons, ProjectButtonsEvent},
    },
    pane_group::{
        focus_state::PaneFocusHandle, pane::view, BackingView, PaneConfiguration, PaneEvent,
    },
    send_telemetry_from_ctx,
    terminal::TerminalView,
    util::bindings::{keybinding_name_to_display_string, BindingGroup, CustomAction},
    view_components::DismissibleToast,
    workspace::ToastStack,
    workspace::{Workspace, WorkspaceAction},
    TelemetryEvent,
};

pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;

    app.register_editable_bindings([EditableBinding::new(
        "workspace:new_tab",
        "Terminal session",
        GetStartedAction::TerminalSession,
    )
    .with_context_predicate(id!("GetStartedView"))
    .with_group(BindingGroup::Terminal.as_str())
    .with_custom_action(CustomAction::NewTab)]);
}

#[derive(Debug, Default)]
enum ActivePage {
    #[default]
    Main,
    CreateProject,
    CloneRepo,
}

pub struct GetStartedView {
    pane_configuration: ModelHandle<PaneConfiguration>,
    focus_handle: Option<PaneFocusHandle>,
    project_buttons: ViewHandle<ProjectButtons>,
    create_project_view: ViewHandle<CreateProjectView>,
    clone_repo_view: ViewHandle<CloneRepoView>,
    active_page: ActivePage,
    terminal_session_button: MouseStateHandle,
}

impl GetStartedView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let pane_configuration = ctx.add_model(|_ctx| PaneConfiguration::new("Get started"));
        let project_buttons = ctx.add_typed_action_view(ProjectButtons::new);
        ctx.subscribe_to_view(&project_buttons, Self::handle_project_buttons_event);

        let create_project_view =
            ctx.add_typed_action_view(|ctx| CreateProjectView::new(true, ctx));
        ctx.subscribe_to_view(&create_project_view, Self::handle_create_project_event);

        let clone_repo_view = ctx.add_typed_action_view(|ctx| CloneRepoView::new(true, ctx));
        ctx.subscribe_to_view(&clone_repo_view, Self::handle_clone_repo_event);

        Self {
            pane_configuration,
            focus_handle: None,
            project_buttons,
            create_project_view,
            clone_repo_view,
            active_page: Default::default(),
            terminal_session_button: Default::default(),
        }
    }

    fn handle_project_buttons_event(
        &mut self,
        _: ViewHandle<ProjectButtons>,
        event: &ProjectButtonsEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            ProjectButtonsEvent::OpenRepository(path_result) => match path_result {
                Ok(path) => {
                    send_telemetry_from_ctx!(
                        TelemetryEvent::OpenRepoFolderSubmitted { is_ftux: true },
                        ctx
                    );
                    ctx.dispatch_typed_action(&WorkspaceAction::OpenRepository {
                        path: Some(path.clone()),
                    });
                    self.close(ctx);
                }
                Err(err) => {
                    let window_id = ctx.window_id();
                    ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                        toast_stack.add_ephemeral_toast(
                            DismissibleToast::error(format!("{err}")),
                            window_id,
                            ctx,
                        );
                    });
                }
            },
            ProjectButtonsEvent::CreateProject => {
                self.active_page = ActivePage::CreateProject;
                ctx.focus(&self.create_project_view);
                ctx.notify();
            }
            ProjectButtonsEvent::CloneRepository => {
                self.active_page = ActivePage::CloneRepo;
                ctx.focus(&self.clone_repo_view);
                ctx.notify();
            }
        }
    }

    pub fn pane_configuration(&self) -> ModelHandle<PaneConfiguration> {
        self.pane_configuration.clone()
    }

    fn handle_create_project_event(
        &mut self,
        _: ViewHandle<CreateProjectView>,
        event: &CreateProjectEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            CreateProjectEvent::SubmitPrompt(prompt) => {
                self.start_create_new_project(prompt.clone(), ctx);
            }
            CreateProjectEvent::Cancel => {
                self.active_page = Default::default();
                ctx.notify();
            }
        }
    }

    fn handle_clone_repo_event(
        &mut self,
        _: ViewHandle<CloneRepoView>,
        event: &CloneRepoEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            CloneRepoEvent::SubmitPrompt(url) => {
                self.start_clone_repo(url.clone(), ctx);
            }
            CloneRepoEvent::Cancel => {
                self.active_page = ActivePage::Main;
                ctx.notify();
            }
        }
    }

    fn start_create_new_project(&mut self, prompt: String, ctx: &mut ViewContext<Self>) {
        ctx.dispatch_typed_action(&WorkspaceAction::AddTerminalTab {
            hide_homepage: true,
        });
        update_active_terminal(ctx, |terminal, ctx| {
            terminal.create_new_project(prompt, ctx);
        });

        self.close(ctx);
    }

    fn start_clone_repo(&mut self, url: String, ctx: &mut ViewContext<Self>) {
        ctx.dispatch_typed_action(&WorkspaceAction::AddTerminalTab {
            hide_homepage: true,
        });
        update_active_terminal(ctx, |terminal, ctx| {
            terminal.agent_clone_repository(url, ctx);
        });

        self.close(ctx);
    }

    fn render_main_content(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        match self.active_page {
            ActivePage::Main => {}
            ActivePage::CreateProject => {
                return Align::new(
                    ConstrainedBox::new(ChildView::new(&self.create_project_view).finish())
                        .with_max_width(480.)
                        .finish(),
                )
                .finish();
            }
            ActivePage::CloneRepo => {
                return Align::new(
                    ConstrainedBox::new(ChildView::new(&self.clone_repo_view).finish())
                        .with_max_width(480.)
                        .finish(),
                )
                .finish();
            }
        }

        Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_children([
                Container::new(
                    ConstrainedBox::new(
                        Icon::new("bundled/svg/warp-logo-neutral.svg", theme.foreground()).finish(),
                    )
                    .with_height(40.)
                    .with_width(40.)
                    .finish(),
                )
                .with_margin_bottom(12.)
                .finish(),
                appearance
                    .ui_builder()
                    .paragraph(crate::i18n::tr_static(app, "Welcome to Warp"))
                    .with_style(UiComponentStyles {
                        font_size: Some(20.),
                        ..Default::default()
                    })
                    .build()
                    .finish(),
                Container::new(
                    appearance
                        .ui_builder()
                        .paragraph(crate::i18n::tr_static(
                            app,
                            "The Agentic Development Environment",
                        ))
                        .with_style(UiComponentStyles {
                            font_size: Some(14.),
                            font_family_id: Some(appearance.monospace_font_family()),
                            font_color: Some(
                                theme.disabled_text_color(theme.background()).into_solid(),
                            ),
                            ..Default::default()
                        })
                        .build()
                        .finish(),
                )
                .with_margin_top(4.)
                .with_margin_bottom(6.)
                .finish(),
                Container::new(
                    ConstrainedBox::new(ChildView::new(&self.project_buttons).finish())
                        .with_max_width(480.)
                        .with_max_height(70.)
                        .finish(),
                )
                .with_vertical_margin(16.)
                .finish(),
                appearance
                    .ui_builder()
                    .button(ButtonVariant::Text, self.terminal_session_button.clone())
                    .with_style(UiComponentStyles {
                        padding: Some(Coords::uniform(8.)),
                        ..Default::default()
                    })
                    .with_hovered_styles(UiComponentStyles {
                        border_radius: Some(CornerRadius::with_all(Radius::Pixels(4.))),
                        background: Some(
                            theme.background().blend(&theme.surface_overlay_1()).into(),
                        ),
                        ..Default::default()
                    })
                    .with_text_and_icon_label(TextAndIcon::new(
                        TextAndIconAlignment::IconFirst,
                        format!(
                            " New session in {}  {}",
                            dirs::home_dir()
                                .map(|dir| dir.display().to_string())
                                .unwrap_or("~".to_string()),
                            keybinding_name_to_display_string("workspace:new_tab", app)
                                .unwrap_or_default()
                        ),
                        ui::Icon::Terminal.to_warpui_icon(theme.foreground()),
                        MainAxisSize::Min,
                        MainAxisAlignment::Center,
                        vec2f(16., 16.),
                    ))
                    .build()
                    .on_click(|ctx, _, _| {
                        ctx.dispatch_typed_action(GetStartedAction::TerminalSession)
                    })
                    .with_cursor(Cursor::PointingHand)
                    .finish(),
            ])
            .finish()
    }
}

impl Entity for GetStartedView {
    type Event = PaneEvent;
}

#[derive(Debug)]
pub enum GetStartedAction {
    TerminalSession,
}

impl TypedActionView for GetStartedView {
    type Action = GetStartedAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            GetStartedAction::TerminalSession => {
                send_telemetry_from_ctx!(TelemetryEvent::GetStartedSkipToTerminal, ctx);
                ctx.dispatch_typed_action(&WorkspaceAction::AddTerminalTab {
                    hide_homepage: true,
                });
                self.close(ctx);
            }
        }
    }
}

impl View for GetStartedView {
    fn ui_name() -> &'static str {
        "GetStartedView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        Align::new(self.render_main_content(app)).finish()
    }
}

impl BackingView for GetStartedView {
    type PaneHeaderOverflowMenuAction = ();
    type CustomAction = ();
    type AssociatedData = ();

    fn handle_pane_header_overflow_menu_action(
        &mut self,
        _action: &Self::PaneHeaderOverflowMenuAction,
        _ctx: &mut ViewContext<Self>,
    ) {
        // TODO
    }

    fn close(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.emit(PaneEvent::Close);
    }

    fn focus_contents(&mut self, ctx: &mut ViewContext<Self>) {
        match self.active_page {
            ActivePage::CreateProject => ctx.focus(&self.create_project_view),
            ActivePage::CloneRepo => ctx.focus(&self.clone_repo_view),
            ActivePage::Main => ctx.focus(&self.project_buttons),
        }
    }

    fn render_header_content(
        &self,
        _ctx: &view::HeaderRenderContext<'_>,
        _app: &AppContext,
    ) -> view::HeaderContent {
        view::HeaderContent::simple("Get started")
    }

    fn set_focus_handle(&mut self, focus_handle: PaneFocusHandle, _ctx: &mut ViewContext<Self>) {
        self.focus_handle = Some(focus_handle);
    }
}

fn update_active_terminal<F, S>(ctx: &mut ViewContext<GetStartedView>, func: F)
where
    F: FnOnce(&mut TerminalView, &mut ViewContext<TerminalView>) -> S,
{
    let window_id = ctx.window_id();
    if let Some(workspaces) = ctx.views_of_type::<Workspace>(window_id) {
        if let Some(workspace) = workspaces.into_iter().next() {
            workspace.update(ctx, |workspace, ctx| {
                let pane_group = workspace.active_tab_pane_group();
                pane_group.update(ctx, |pane_group, ctx| {
                    if let Some(active_terminal) = pane_group.active_session_view(ctx) {
                        active_terminal.update(ctx, func);
                    }
                });
            });
        }
    }
}
