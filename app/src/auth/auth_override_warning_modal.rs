use pathfinder_color::ColorU;
use warp_core::ui::appearance::Appearance;
use warpui::elements::{ChildView, Container, Fill};
use warpui::ui_components::components::{Coords, UiComponentStyles};
use warpui::{
    AppContext, Element, Entity, FocusContext, SingletonEntity, TypedActionView, View, ViewContext,
    ViewHandle,
};

use super::auth_manager::{AuthManager, AuthManagerEvent};
use super::auth_override_warning_body::AuthOverrideWarningBodyEvent;
use crate::auth::auth_override_warning_body::AuthOverrideWarningBody;
use crate::auth::auth_view_modal::AuthRedirectPayload;
use crate::modal::Modal;
use crate::root_view::unthemed_window_border;

pub struct AuthOverrideWarningModal {
    auth_override_warning_modal: ViewHandle<Modal<AuthOverrideWarningBody>>,
    interrupted_auth_payload: Option<AuthRedirectPayload>,
    variant: AuthOverrideWarningModalVariant,
}

pub enum AuthOverrideWarningModalVariant {
    OnboardingView,
    WorkspaceModal,
}

const MODAL_WIDTH: f32 = 364.;

impl AuthOverrideWarningModal {
    pub fn new(ctx: &mut ViewContext<Self>, variant: AuthOverrideWarningModalVariant) -> Self {
        let auth_screen_view = ctx.add_typed_action_view(|_| AuthOverrideWarningBody::new());
        ctx.subscribe_to_view(&auth_screen_view, |me, _, event, ctx| match event {
            AuthOverrideWarningBodyEvent::Close => me.close(ctx),
            AuthOverrideWarningBodyEvent::AllowLogin => {
                if let Some(auth_payload) = me.interrupted_auth_payload.clone() {
                    AuthManager::handle(ctx).update(ctx, |auth_manager, ctx| {
                        auth_manager.resume_interrupted_auth_payload(auth_payload, ctx);
                    });
                }
                ctx.emit(AuthOverrideWarningModalEvent::Close);
            }
            AuthOverrideWarningBodyEvent::BulkExport => {
                ctx.emit(AuthOverrideWarningModalEvent::BulkExport);
            }
        });

        let auth_override_warning_modal = ctx.add_typed_action_view(|ctx| {
            Modal::new(None, auth_screen_view, ctx)
                .with_body_style(UiComponentStyles {
                    padding: Some(Coords::uniform(0.)),
                    ..Default::default()
                })
                .with_modal_style(UiComponentStyles {
                    width: Some(MODAL_WIDTH),
                    border_color: Some(Fill::from(ColorU::transparent_black())), // override default modal border color
                    ..Default::default()
                })
        });

        let auth_manager = AuthManager::handle(ctx);
        ctx.subscribe_to_model(&auth_manager, |me, _, event, ctx| {
            me.handle_auth_manager_event(event, ctx);
        });

        Self {
            auth_override_warning_modal,
            interrupted_auth_payload: None,
            variant,
        }
    }

    fn focus(&self, ctx: &mut ViewContext<Self>) {
        ctx.focus(&self.auth_override_warning_modal);
        ctx.notify();
    }

    fn close(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.emit(AuthOverrideWarningModalEvent::Close);
        self.auth_override_warning_modal.update(ctx, |modal, ctx| {
            modal.body().update(ctx, |body, _| {
                body.reset();
            })
        })
    }

    pub fn set_interrupted_auth_payload(&mut self, auth_payload: AuthRedirectPayload) {
        self.interrupted_auth_payload = Some(auth_payload);
    }

    fn handle_auth_manager_event(&mut self, event: &AuthManagerEvent, ctx: &mut ViewContext<Self>) {
        if let AuthManagerEvent::AuthComplete = event {
            self.interrupted_auth_payload = None;
            self.close(ctx);
        }
        ctx.notify();
    }
}

#[derive(PartialEq, Eq)]
pub enum AuthOverrideWarningModalEvent {
    Close,
    BulkExport,
}

impl Entity for AuthOverrideWarningModal {
    type Event = AuthOverrideWarningModalEvent;
}

impl View for AuthOverrideWarningModal {
    fn ui_name() -> &'static str {
        "AuthOverrideWarningModal"
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() {
            self.focus(ctx);
        }
    }

    fn render(&self, ctx: &AppContext) -> Box<dyn Element> {
        let background_color = match self.variant {
            AuthOverrideWarningModalVariant::OnboardingView => {
                Appearance::as_ref(ctx).theme().background().into()
            }
            AuthOverrideWarningModalVariant::WorkspaceModal => ColorU::transparent_black(),
        };

        Container::new(ChildView::new(&self.auth_override_warning_modal).finish())
            .with_background_color(background_color)
            .with_corner_radius(ctx.windows().window_corner_radius())
            .with_border(unthemed_window_border())
            .finish()
    }
}

impl TypedActionView for AuthOverrideWarningModal {
    type Action = ();
}
