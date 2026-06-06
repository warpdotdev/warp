use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use itertools::Itertools as _;
use warp_core::context_flag::ContextFlag;
use warp_core::ui::appearance::Appearance;
use warpui::elements::{
    Align, ChildView, ConstrainedBox, Container, CrossAxisAlignment, Flex, Icon, ParentElement,
};
use warpui::keymap::EditableBinding;
use warpui::platform::FilePickerConfiguration;
use warpui::{
    AppContext, Element, Entity, ModelHandle, SingletonEntity as _, TypedActionView, View,
    ViewContext, ViewHandle, WindowId,
};

use crate::code_review::diff_state::GitDeltaPreference;
use crate::code_review::telemetry_event::CodeReviewPaneEntrypoint;
use crate::pane_group::focus_state::PaneFocusHandle;
use crate::pane_group::pane::view;
use crate::pane_group::{
    BackingView, NewTerminalOptions, PaneConfiguration, PaneEvent, PanesLayout,
};
use crate::projects::ProjectManagementModel;
use crate::search::binding_source::BindingSource;
use crate::search::welcome_palette::{Event as WelcomePaletteEvent, WelcomePalette};
use crate::util::bindings::{keybinding_name_to_display_string, BindingGroup, CustomAction};
use crate::view_components::DismissibleToast;
use crate::workspace::{ToastStack, Workspace};

pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;

    app.register_editable_bindings([
        EditableBinding::new(
            "workspace:new_tab",
            "Terminal session",
            WelcomeViewAction::CreateTerminalSession,
        )
        .with_context_predicate(id!("WelcomeView"))
        .with_group(BindingGroup::Terminal.as_str())
        .with_custom_action(CustomAction::NewTab)
        .with_enabled(|| ContextFlag::CreateNewSession.is_enabled()),
        EditableBinding::new(
            "welcome_view:open_project",
            "Add repository",
            WelcomeViewAction::OpenProject,
        )
        .with_context_predicate(id!("WelcomeView"))
        .with_group(BindingGroup::Folders.as_str())
        .with_mac_key_binding("cmd-shift-N")
        .with_linux_or_windows_key_binding("alt-n"),
    ]);
}

#[derive(Debug, Clone, Copy)]
pub enum WelcomeViewAction {
    CreateTerminalSession,
    OpenProject,
}

pub struct WelcomeView {
    /// Configure which directory to open sessions into as per the "working directory for new
    /// sessions" setting.
    pub startup_directory: Option<PathBuf>,
    pane_configuration: ModelHandle<PaneConfiguration>,
    focus_handle: Option<PaneFocusHandle>,
    palette: ViewHandle<WelcomePalette>,
}

impl WelcomeView {
    pub fn new(startup_directory: Option<PathBuf>, ctx: &mut ViewContext<Self>) -> Self {
        let pane_configuration = ctx.add_model(|_ctx| PaneConfiguration::new("New tab"));
        let window_id = ctx.window_id();
        let view_id = ctx.view_id();
        let palette = ctx.add_typed_action_view(|ctx| {
            let binding_source = BindingSource::View {
                window_id,
                view_id,
                binding_filter_fn: Some(Arc::new(|binding| {
                    binding.action.as_ref().is_some_and(|action| {
                        action
                            .as_any()
                            .downcast_ref::<WelcomeViewAction>()
                            .is_some()
                    }) || binding.name == "workspace:show_settings"
                })),
            };

            let open_project_keybinding =
                keybinding_name_to_display_string("welcome_view:open_project", ctx);

            let terminal_session_keybinding =
                keybinding_name_to_display_string("workspace:new_tab", ctx);

            WelcomePalette::new(
                startup_directory.clone(),
                binding_source,
                open_project_keybinding,
                terminal_session_keybinding,
                ctx,
            )
        });
        ctx.subscribe_to_view(&palette, |me, _, event, ctx| {
            me.handle_palette_event(event, ctx);
        });

        Self {
            startup_directory,
            pane_configuration,
            focus_handle: None,
            palette,
        }
    }

    pub fn pane_configuration(&self) -> ModelHandle<PaneConfiguration> {
        self.pane_configuration.clone()
    }

    fn handle_palette_event(&mut self, event: &WelcomePaletteEvent, ctx: &mut ViewContext<Self>) {
        match event {
            WelcomePaletteEvent::Close => self.close(ctx),
            WelcomePaletteEvent::ParentAction { action } => self.handle_action(action, ctx),
            WelcomePaletteEvent::NewConversationInProject { path } => {
                self.open_project_conversation(path, ctx);
                self.close(ctx);
            }
            _ => {
                // TODO
            }
        }
    }

    fn create_terminal_session(&mut self, ctx: &mut ViewContext<Self>) {
        update_workspace(ctx.window_id(), ctx, |workspace, ctx| {
            workspace.add_tab_with_pane_layout(
                PanesLayout::SingleTerminal(Box::new(
                    NewTerminalOptions::default()
                        .with_initial_directory_opt(self.startup_directory.clone()),
                )),
                Arc::new(HashMap::new()),
                None,
                ctx,
            );
        });
    }

    fn open_project(&mut self, ctx: &mut ViewContext<Self>) {
        let window_id = ctx.window_id();
        ctx.open_file_picker(
            move |result, ctx| match result {
                Ok(paths) => {
                    if let Some(path) = paths.into_iter().next() {
                        save_and_open_project(path, window_id, ctx);
                        ctx.emit(PaneEvent::Close);
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
            FilePickerConfiguration::new().folders_only(),
        );
    }

    fn open_project_conversation(&mut self, path: &String, ctx: &mut ViewContext<Self>) {
        let path_buf = PathBuf::from(path);
        // todo(jparker): What happens if the user deletes a project folder between when this list was generated and now?
        update_workspace(ctx.window_id(), ctx, |workspace, ctx| {
            // Create a new terminal tab with the project path as the initial directory
            workspace.add_tab_with_pane_layout(
                PanesLayout::SingleTerminal(Box::new(
                    NewTerminalOptions::default().with_initial_directory(&path_buf),
                )),
                Arc::new(HashMap::new()),
                None,
                ctx,
            );

            // Start AI mode in the new terminal
            workspace
                .active_tab_pane_group()
                .update(ctx, |pane_group, ctx| {
                    pane_group.start_agent_mode_in_new_pane(None, None, ctx);
                });

            // Open code review pane
            workspace.active_tab_pane_group().update(ctx, |tab, ctx| {
                if let Some(active_terminal) = tab.active_session_view(ctx) {
                    active_terminal.update(ctx, |terminal, ctx| {
                        terminal.toggle_code_review_pane(
                            GitDeltaPreference::OnlyDirty,
                            CodeReviewPaneEntrypoint::Other,
                            None,  // cli_agent
                            false, /* focus_new_pane */
                            ctx,
                        );
                    });
                }
            });

            // Update project accesstime
            ProjectManagementModel::handle(ctx).update(ctx, |projects, ctx| {
                projects.upsert_project(path_buf, ctx);
            });
        });
    }
}

fn update_workspace<F>(window_id: WindowId, ctx: &mut AppContext, update_fn: F)
where
    F: FnOnce(&mut Workspace, &mut ViewContext<Workspace>),
{
    if let Some(workspaces) = ctx.views_of_type::<Workspace>(window_id) {
        if let Ok(workspace) = workspaces.into_iter().exactly_one() {
            workspace.update(ctx, update_fn);
        }
    }
}

impl Entity for WelcomeView {
    type Event = PaneEvent;
}

impl View for WelcomeView {
    fn ui_name() -> &'static str {
        "WelcomeView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        Align::new(
            Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_children([
                    Container::new(
                        ConstrainedBox::new(
                            Icon::new(
                                "bundled/svg/warp-logo-neutral.svg",
                                appearance.theme().foreground(),
                            )
                            .finish(),
                        )
                        .with_height(50.)
                        .with_width(50.)
                        .finish(),
                    )
                    .with_margin_bottom(40.)
                    .finish(),
                    Container::new(ChildView::new(&self.palette).finish())
                        .with_padding_bottom(140.)
                        .finish(),
                ])
                .finish(),
        )
        .finish()
    }
}

impl BackingView for WelcomeView {
    type PaneHeaderOverflowMenuAction = ();
    type CustomAction = ();
    type AssociatedData = ();

    fn handle_pane_header_overflow_menu_action(
        &mut self,
        _action: &Self::PaneHeaderOverflowMenuAction,
        _ctx: &mut ViewContext<Self>,
    ) {
        unimplemented!()
    }

    fn close(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.emit(PaneEvent::Close);
    }

    fn focus_contents(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.focus(&self.palette)
    }

    fn render_header_content(
        &self,
        _ctx: &view::HeaderRenderContext<'_>,
        _app: &AppContext,
    ) -> view::HeaderContent {
        view::HeaderContent::simple("New tab")
    }

    fn set_focus_handle(&mut self, focus_handle: PaneFocusHandle, _ctx: &mut ViewContext<Self>) {
        self.focus_handle = Some(focus_handle);
    }
}

impl TypedActionView for WelcomeView {
    type Action = WelcomeViewAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            WelcomeViewAction::CreateTerminalSession => {
                self.create_terminal_session(ctx);
                self.close(ctx);
            }
            WelcomeViewAction::OpenProject => {
                self.open_project(ctx);
            }
        }
    }
}

/// WARNING - Don't use. The [`crate::workspace::WorkspaceAction::OpenRepository`] is the
/// source-of-truth for this now.
fn save_and_open_project(path: String, window_id: WindowId, ctx: &mut AppContext) {
    ProjectManagementModel::handle(ctx).update(ctx, |projects, ctx| {
        let path_buf = PathBuf::from(&path);
        projects.upsert_project(path_buf.clone(), ctx);
        update_workspace(window_id, ctx, move |workspace, ctx| {
            workspace.add_tab_with_pane_layout(
                PanesLayout::SingleTerminal(Box::new(
                    NewTerminalOptions::default()
                        .with_initial_directory(path)
                        .with_homepage_hidden(),
                )),
                Arc::new(HashMap::new()),
                None,
                ctx,
            );
            workspace.active_tab_pane_group().update(ctx, |tab, ctx| {
                if let Some(active_terminal) = tab.active_session_view(ctx) {
                    active_terminal.update(ctx, |terminal, _ctx| {
                        terminal.maybe_set_pending_repo_init_path(path_buf);
                    });
                }
            });
        });
    });
}
