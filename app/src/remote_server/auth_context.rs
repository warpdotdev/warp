use std::sync::Arc;

use remote_server::auth::RemoteServerAuthContext;
use warpui::r#async::BoxFuture;

use crate::auth::auth_state::AuthState;
use crate::server::server_api::auth::AuthClient;

/// Builds the app-wide auth context used by remote-server connections.
pub fn server_api_auth_context(
    auth_state: Arc<AuthState>,
    auth_client: Arc<dyn AuthClient>,
    crash_reporting_enabled: bool,
) -> RemoteServerAuthContext {
    let token_auth_state = auth_state.clone();
    let token_auth_client = auth_client;
    let identity_auth_state = auth_state.clone();
    let user_id_auth_state = auth_state.clone();
    let user_email_auth_state = auth_state;

    let user_id = user_id_auth_state
        .user_id()
        .map(|uid| uid.as_string())
        .unwrap_or_default();
    let user_email = user_email_auth_state.user_email().unwrap_or_default();

    RemoteServerAuthContext::new(
        move || -> BoxFuture<'static, Option<String>> {
            if !use_authenticated_user_identity(&token_auth_state) {
                return Box::pin(async { None });
            }

            let auth_client = token_auth_client.clone();
            Box::pin(async move {
                match auth_client.get_or_refresh_access_token().await {
                    Ok(token) => token.bearer_token(),
                    Err(_) => None,
                }
            })
        },
        move || remote_server_identity_key(&identity_auth_state),
        user_id,
        user_email,
        crash_reporting_enabled,
    )
}

fn use_authenticated_user_identity(auth_state: &AuthState) -> bool {
    auth_state.is_logged_in() && !auth_state.is_user_anonymous().unwrap_or(true)
}

fn remote_server_identity_key(auth_state: &AuthState) -> String {
    if use_authenticated_user_identity(auth_state) {
        auth_state
            .user_id()
            .map(|uid| uid.as_string())
            .unwrap_or_else(|| auth_state.anonymous_id())
    } else {
        auth_state.anonymous_id()
    }
}
