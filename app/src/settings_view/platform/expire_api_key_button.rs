use crate::{
    i18n::{self, I18nKey},
    server::{ids::ApiKeyUid, server_api::auth::AuthClient},
};
use warp_core::ui::appearance::Appearance;
use warpui::{
    elements::MouseStateHandle, ui_components::components::UiComponent, AppContext, Element,
    Entity, SingletonEntity, TypedActionView, View, ViewContext,
};

use crate::ui_components::{buttons::icon_button, icons::Icon};

#[derive(PartialEq, Eq)]
enum RequestState {
    Idle,
    Pending,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ExpireApiKeyButtonAction {
    ExpireApiKey,
}

pub enum ExpireApiKeyButtonEvent {
    ExpireApiKeySucceeded { uid: ApiKeyUid },
    ExpireApiKeyFailed { message: String },
}

pub struct ExpireApiKeyButton {
    key_uid: ApiKeyUid,
    button_mouse_state: MouseStateHandle,
    request_state: RequestState,
}

impl ExpireApiKeyButton {
    pub fn new(key_uid: ApiKeyUid) -> Self {
        Self {
            key_uid,
            button_mouse_state: Default::default(),
            request_state: RequestState::Idle,
        }
    }

    fn expire_api_key(&mut self, ctx: &mut ViewContext<Self>) {
        if self.request_state == RequestState::Pending {
            return;
        }
        self.request_state = RequestState::Pending;
        ctx.notify();

        let server_api = crate::server::server_api::ServerApiProvider::as_ref(ctx).get();
        let uid_for_req = self.key_uid.clone();
        ctx.spawn(
            async move { server_api.expire_api_key(&uid_for_req).await },
            move |me, res, ctx| match res {
                Ok(
                    warp_graphql::mutations::expire_api_key::ExpireApiKeyResult::ExpireApiKeyOutput(
                        _output,
                    ),
                ) => {
                    me.request_state = RequestState::Idle;
                    ctx.emit(ExpireApiKeyButtonEvent::ExpireApiKeySucceeded {
                        uid: me.key_uid.clone(),
                    });
                    ctx.notify();
                }
                Ok(
                    warp_graphql::mutations::expire_api_key::ExpireApiKeyResult::UserFacingError(e),
                ) => {
                    let _msg = warp_graphql::client::get_user_facing_error_message(e);
                    me.request_state = RequestState::Idle;
                    ctx.emit(ExpireApiKeyButtonEvent::ExpireApiKeyFailed { message: _msg });
                    ctx.notify();
                }
                Ok(warp_graphql::mutations::expire_api_key::ExpireApiKeyResult::Unknown)
                | Err(_) => {
                    me.request_state = RequestState::Idle;
                    ctx.emit(ExpireApiKeyButtonEvent::ExpireApiKeyFailed {
                        message: i18n::tr(ctx, I18nKey::PlatformDeleteApiKeyFailed).to_string(),
                    });
                    ctx.notify();
                }
            },
        );
    }
}

impl View for ExpireApiKeyButton {
    fn ui_name() -> &'static str {
        "ExpireApiKeyButton"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let expire_icon = match self.request_state {
            RequestState::Pending => Icon::Loading,
            RequestState::Idle => Icon::Trash,
        };
        let mut expire_button = icon_button(
            appearance,
            expire_icon,
            false,
            self.button_mouse_state.clone(),
        )
        .build();
        if self.request_state != RequestState::Idle {
            expire_button = expire_button.disable();
        }
        expire_button
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(ExpireApiKeyButtonAction::ExpireApiKey);
            })
            .finish()
    }
}

impl Entity for ExpireApiKeyButton {
    type Event = ExpireApiKeyButtonEvent;
}

impl TypedActionView for ExpireApiKeyButton {
    type Action = ExpireApiKeyButtonAction;

    fn handle_action(&mut self, action: &ExpireApiKeyButtonAction, ctx: &mut ViewContext<Self>) {
        match action {
            ExpireApiKeyButtonAction::ExpireApiKey => {
                self.expire_api_key(ctx);
            }
        }
    }
}
