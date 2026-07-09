use pathfinder_color::ColorU;
use warpui::elements::{
    Align, ChildView, ClippedScrollStateHandle, ClippedScrollable, CrossAxisAlignment, Dismiss,
    Element, Flex, MouseStateHandle, ParentElement, ScrollbarWidth,
};
use warpui::ui_components::components::UiComponent;
use warpui::{
    AppContext, Entity, FocusContext, SingletonEntity, TypedActionView, View, ViewContext,
    ViewHandle,
};

use crate::ai::ambient_agents::github_auth_url::{AuthSource, GithubAuthRedirectTarget};
use crate::ai::cloud_environments;
use crate::appearance::Appearance;
use crate::modal::MODAL_BACKDROP_OPACITY;
use crate::report_error;
use crate::server::cloud_objects::update_manager::UpdateManager;
use crate::server::ids::{ClientId, SyncId};
use crate::settings_view::update_environment_form::{
    EnvironmentFormInitArgs, UpdateEnvironmentForm, UpdateEnvironmentFormEvent,
};
use crate::ui_components::buttons::icon_button;
use crate::ui_components::dialog::{dialog_styles, Dialog};
use crate::ui_components::icons::Icon;

const DIALOG_WIDTH: f32 = 600.;

#[derive(Clone, Copy, Debug)]
enum HandoffEnvironmentCreationModalContext {
    Handoff,
    Orchestration,
}

#[derive(Debug, Clone)]
pub(crate) enum HandoffEnvironmentCreationModalEvent {
    Created { env_id: SyncId },
    Cancelled,
    CreationFailed { error_message: String },
}

#[derive(Debug, Clone)]
pub(crate) enum HandoffEnvironmentCreationModalAction {
    Cancel,
}

pub(crate) struct HandoffEnvironmentCreationModal {
    environment_form: ViewHandle<UpdateEnvironmentForm>,
    close_button_mouse_state: MouseStateHandle,
    scroll_state: ClippedScrollStateHandle,
}

impl HandoffEnvironmentCreationModal {
    pub(crate) fn new(ctx: &mut ViewContext<Self>) -> Self {
        Self::new_impl(HandoffEnvironmentCreationModalContext::Handoff, ctx)
    }

    pub(crate) fn new_for_orchestration(ctx: &mut ViewContext<Self>) -> Self {
        Self::new_impl(HandoffEnvironmentCreationModalContext::Orchestration, ctx)
    }

    fn new_impl(
        context: HandoffEnvironmentCreationModalContext,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let environment_form = ctx.add_typed_action_view(move |ctx| {
            let mut form = UpdateEnvironmentForm::new(EnvironmentFormInitArgs::Create, ctx);
            form.set_github_auth_redirect_target(GithubAuthRedirectTarget::FocusCloudMode);
            form.set_show_header(false, ctx);
            form.set_should_handle_escape_from_editor(true);
            form.set_auth_source(AuthSource::CloudSetup);
            match context {
                HandoffEnvironmentCreationModalContext::Handoff => {}
                HandoffEnvironmentCreationModalContext::Orchestration => {
                    form.configure_for_orchestration_modal(ctx);
                }
            }
            form
        });

        ctx.subscribe_to_view(&environment_form, |me, _, event, ctx| {
            me.handle_environment_form_event(event, ctx);
        });

        Self {
            environment_form,
            close_button_mouse_state: MouseStateHandle::default(),
            scroll_state: ClippedScrollStateHandle::default(),
        }
    }

    pub(crate) fn show(&mut self, ctx: &mut ViewContext<Self>) {
        self.scroll_state = ClippedScrollStateHandle::default();
        self.environment_form.update(ctx, |form, ctx| {
            form.set_mode(EnvironmentFormInitArgs::Create, ctx);
            form.focus(ctx);
        });
        ctx.notify();
    }

    fn handle_environment_form_event(
        &mut self,
        event: &UpdateEnvironmentFormEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            UpdateEnvironmentFormEvent::Created {
                environment,
                share_with_team,
            } => {
                let owner = if *share_with_team {
                    cloud_environments::owner_for_new_environment(ctx)
                } else {
                    cloud_environments::owner_for_new_personal_environment(ctx)
                };

                let Some(owner) = owner else {
                    report_error!("Unable to create environment: not logged in");
                    ctx.emit(HandoffEnvironmentCreationModalEvent::CreationFailed {
                        error_message: "Not logged in".to_string(),
                    });
                    return;
                };

                let client_id = ClientId::default();
                let create_future =
                    UpdateManager::handle(ctx).update(ctx, |update_manager, ctx| {
                        update_manager.create_ambient_agent_environment_online(
                            environment.clone(),
                            client_id,
                            owner,
                            ctx,
                        )
                    });

                ctx.spawn(create_future, |_me, result, ctx| match result {
                    Ok(server_id) => {
                        let env_id = SyncId::ServerId(server_id);
                        ctx.emit(HandoffEnvironmentCreationModalEvent::Created { env_id });
                    }
                    Err(err) => {
                        report_error!(&err);
                        ctx.emit(HandoffEnvironmentCreationModalEvent::CreationFailed {
                            error_message: err.to_string(),
                        });
                    }
                });
            }
            UpdateEnvironmentFormEvent::Cancelled => {
                ctx.emit(HandoffEnvironmentCreationModalEvent::Cancelled);
            }
            UpdateEnvironmentFormEvent::Updated { .. }
            | UpdateEnvironmentFormEvent::DeleteRequested { .. } => {}
        }
    }

    #[cfg(test)]
    pub(crate) fn uses_orchestration_form_configuration_for_test(&self, app: &AppContext) -> bool {
        self.environment_form
            .as_ref(app)
            .uses_orchestration_modal_configuration_for_test()
    }

    fn render_dialog(&self, appearance: &Appearance, app: &AppContext) -> Box<dyn Element> {
        let theme = appearance.theme();

        let close_button = icon_button(
            appearance,
            Icon::X,
            false,
            self.close_button_mouse_state.clone(),
        )
        .build()
        .on_click(|ctx, _, _| {
            ctx.dispatch_typed_action(HandoffEnvironmentCreationModalAction::Cancel);
        })
        .finish();

        let form_content = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(ChildView::new(&self.environment_form).finish())
            .finish();

        let scrollable_form = ClippedScrollable::vertical(
            self.scroll_state.clone(),
            form_content,
            ScrollbarWidth::Auto,
            theme.nonactive_ui_text_color().into(),
            theme.active_ui_text_color().into(),
            warpui::elements::Fill::None,
        )
        .finish();

        let padded_form = warpui::elements::Container::new(scrollable_form)
            .with_uniform_padding(8.)
            .finish();

        let dialog = Dialog::new(
            "Create environment".to_string(),
            None,
            dialog_styles(appearance),
        )
        .with_close_button(close_button)
        .with_child(padded_form)
        .with_width(DIALOG_WIDTH)
        .build();

        let dialog = Dismiss::new(dialog.finish())
            .prevent_interaction_with_other_elements()
            .on_dismiss(|ctx, _app| {
                ctx.dispatch_typed_action(HandoffEnvironmentCreationModalAction::Cancel);
            })
            .finish();

        warpui::elements::Container::new(Align::new(dialog).finish())
            .with_background_color(ColorU::new(0, 0, 0, MODAL_BACKDROP_OPACITY))
            .with_corner_radius(app.windows().window_corner_radius())
            .finish()
    }
}

impl Entity for HandoffEnvironmentCreationModal {
    type Event = HandoffEnvironmentCreationModalEvent;
}

impl TypedActionView for HandoffEnvironmentCreationModal {
    type Action = HandoffEnvironmentCreationModalAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            HandoffEnvironmentCreationModalAction::Cancel => {
                ctx.emit(HandoffEnvironmentCreationModalEvent::Cancelled);
            }
        }
    }
}

impl View for HandoffEnvironmentCreationModal {
    fn ui_name() -> &'static str {
        "HandoffEnvironmentCreationModal"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        self.render_dialog(appearance, app)
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() {
            ctx.focus(&self.environment_form);
        }
    }
}
