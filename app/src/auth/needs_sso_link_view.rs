use super::auth_manager::AuthManager;
use crate::{appearance::Appearance, auth::login_error_modal::LoginErrorModal};
use warpui::elements::{Align, MouseStateHandle, Shrinkable};
use warpui::ui_components::button::ButtonVariant;
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::{AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext};

#[derive(Debug)]
pub enum NeedsSsoLinkViewAction {
    ClickedLinkSsoButton,
}

pub struct NeedsSsoLinkView {
    email: Option<String>,
    mouse_state_handles: MouseStateHandles,
}

#[derive(Default)]
struct MouseStateHandles {
    link_sso_handle: MouseStateHandle,
}

impl NeedsSsoLinkView {
    pub fn new() -> Self {
        Self {
            email: None,
            mouse_state_handles: Default::default(),
        }
    }

    pub fn set_email(&mut self, email: String) {
        self.email = Some(email);
    }
}

impl Entity for NeedsSsoLinkView {
    type Event = ();
}

impl View for NeedsSsoLinkView {
    fn ui_name() -> &'static str {
        "NeedsSsoLinkView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let ui_builder = appearance.ui_builder();

        let link_sso_button = Shrinkable::new(
            1.,
            Align::new(
                ui_builder
                    .button(
                        ButtonVariant::Accent,
                        self.mouse_state_handles.link_sso_handle.clone(),
                    )
                    .with_text_label(crate::i18n::tr_static(app, "Link SSO").to_string())
                    .with_style(UiComponentStyles {
                        padding: Some(Coords {
                            top: 10.,
                            bottom: 10.,
                            left: 40.,
                            right: 40.,
                        }),
                        ..Default::default()
                    })
                    .build()
                    .on_click(move |ctx, _, _| {
                        ctx.dispatch_typed_action(NeedsSsoLinkViewAction::ClickedLinkSsoButton);
                    })
                    .finish(),
            )
            .finish(),
        )
        .finish();

        LoginErrorModal::new(app)
            .with_header("Your organization has enabled SSO for your account")
            .with_detail("Click the button below to link your Warp account to your SSO provider.")
            .with_action(link_sso_button)
            .build()
            .finish()
    }
}

impl TypedActionView for NeedsSsoLinkView {
    type Action = NeedsSsoLinkViewAction;

    fn handle_action(&mut self, action: &NeedsSsoLinkViewAction, ctx: &mut ViewContext<Self>) {
        match action {
            NeedsSsoLinkViewAction::ClickedLinkSsoButton => {
                let email = self.email.as_deref().unwrap_or("");

                AuthManager::handle(ctx).update(ctx, |auth_manager, ctx| {
                    let url = auth_manager.link_sso_url(email);
                    ctx.open_url(&url);
                });
            }
        }
    }
}
