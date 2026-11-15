use warpui::{Entity, ModelContext, SingletonEntity};

pub use super::auth_manager_types::{
    AuthManagerEvent, LoginGatedFeature, PersistedCurrentUserInformation,
};
use super::auth_view_modal::{AuthRedirectPayload, AuthViewVariant};
use crate::server::telemetry::AnonymousUserSignupEntrypoint;

/// Preserves the model identity for shared subscribers in a local terminal session.
pub struct AuthManager;

impl AuthManager {
    #[cfg(any(test, all(feature = "tui", feature = "test-util")))]
    pub fn new_for_test(_: &mut ModelContext<Self>) -> Self {
        Self
    }

    pub fn initialize_user_from_auth_payload(
        &mut self,
        _: AuthRedirectPayload,
        _: bool,
        _: &mut ModelContext<Self>,
    ) {
    }

    pub fn resume_interrupted_auth_payload(
        &mut self,
        _: AuthRedirectPayload,
        _: &mut ModelContext<Self>,
    ) {
    }

    #[cfg(target_family = "wasm")]
    pub fn initialize_user_from_session_cookie(&self, _: &mut ModelContext<Self>) {}

    pub fn refresh_user(&self, _: &mut ModelContext<Self>) {}

    pub fn authenticate_api_key(&self, _: String, _: &mut ModelContext<Self>) {}

    pub fn authorize_device(&self, _: &mut ModelContext<Self>) {}

    pub(super) fn log_out(&mut self, _: &mut ModelContext<Self>) {}

    pub fn set_needs_reauth(&self, _: bool, _: &mut ModelContext<Self>) {}

    pub fn create_anonymous_user(&self, _: Option<String>, _: &mut ModelContext<Self>) {}

    pub fn attempt_login_gated_feature(
        &self,
        _: LoginGatedFeature,
        _: AuthViewVariant,
        _: &mut ModelContext<Self>,
    ) {
    }

    pub fn anonymous_user_hit_drive_object_limit(&self, _: &mut ModelContext<Self>) {}

    pub fn initiate_anonymous_user_linking(
        &self,
        _: AnonymousUserSignupEntrypoint,
        _: &mut ModelContext<Self>,
    ) {
    }

    pub fn open_url_maybe_with_anonymous_token(
        &self,
        ctx: &mut ModelContext<Self>,
        construct_url: Box<dyn FnOnce(Option<&str>) -> String>,
    ) {
        ctx.open_url(&construct_url(None));
    }

    pub fn copy_anonymous_user_linking_url_to_clipboard(&self, _: &mut ModelContext<Self>) {}

    pub fn sign_up_url(&mut self) -> String {
        String::new()
    }

    pub fn sign_in_url(&mut self) -> String {
        String::new()
    }

    pub fn upgrade_url(&mut self) -> String {
        String::new()
    }

    pub fn link_sso_url(&mut self, _: &str) -> String {
        String::new()
    }

    pub fn set_user_onboarded(&self, _: &mut ModelContext<Self>) {}
}

impl Entity for AuthManager {
    type Event = AuthManagerEvent;
}

impl SingletonEntity for AuthManager {}
