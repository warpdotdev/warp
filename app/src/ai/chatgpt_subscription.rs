use std::collections::HashMap;
use std::sync::Arc;

use ai::api_keys::{ApiKeyManager, ChatGPTConnectFailure, ChatGPTConnectionStatus};
use settings::Setting as _;
use url::Url;
use uuid::Uuid;
use warp_core::channel::ChannelState;
use warpui::{Entity, ModelContext, SingletonEntity};

use crate::auth::AuthStateProvider;
use crate::server::server_api::ai::AIClient;
use crate::settings::AISettings;

/// Host of the `{scheme}://chatgpt_link` deep link the browser opens once a link flow finishes.
pub const CHATGPT_LINK_URI_HOST: &str = "chatgpt_link";
const CHATGPT_LINK_STATE_PARAM: &str = "state";
const CHATGPT_LINKED_PARAM: &str = "chatgpt_linked";
const CHATGPT_ERROR_PARAM: &str = "chatgpt_error";

/// Keeps [`ApiKeyManager`]'s ChatGPT connection state in sync with warp-server,
/// which owns the account link and any delegated credentials.
pub struct ChatGPTSubscriptionModel {
    ai_client: Arc<dyn AIClient>,
    /// The `state` of the link attempt whose deep-link result is awaited, if any.
    pending_link_state: Option<String>,
}

impl ChatGPTSubscriptionModel {
    pub fn new(ai_client: Arc<dyn AIClient>) -> Self {
        Self {
            ai_client,
            pending_link_state: None,
        }
    }

    /// Starts a link attempt and returns the deep link the browser flow should finish at. The
    /// server-issued start URL arrives via the future; open it in the browser.
    pub fn begin_link_attempt(&mut self) -> impl Future<Output = anyhow::Result<String>> + use<> {
        let state = Uuid::new_v4().to_string();
        let continue_url = format!(
            "{}://{CHATGPT_LINK_URI_HOST}?{CHATGPT_LINK_STATE_PARAM}={state}",
            ChannelState::url_scheme()
        );
        self.pending_link_state = Some(state);
        let ai_client = self.ai_client.clone();
        async move { ai_client.start_chatgpt_link(continue_url).await }
    }

    /// Forgets the awaited link attempt; a later deep link for it is ignored.
    pub fn cancel_link_attempt(&mut self) {
        self.pending_link_state = None;
    }

    /// Settles the awaited link attempt from the deep link the browser flow opened.
    pub fn handle_link_redirect(&mut self, url: &Url, ctx: &mut ModelContext<Self>) {
        let query: HashMap<_, _> = url.query_pairs().into_owned().collect();
        let state = query.get(CHATGPT_LINK_STATE_PARAM);
        if self.pending_link_state.is_none() || state != self.pending_link_state.as_ref() {
            log::warn!("Ignoring ChatGPT link redirect that does not match a pending attempt");
            return;
        }
        self.pending_link_state = None;

        if let Some(error) = query.get(CHATGPT_ERROR_PARAM) {
            let failure = ChatGPTConnectFailure::from_deep_link_code(error);
            ApiKeyManager::handle(ctx).update(ctx, |manager, ctx| {
                manager.fail_chatgpt_oauth(failure, ctx);
            });
            return;
        }
        if !query.contains_key(CHATGPT_LINKED_PARAM) {
            ApiKeyManager::handle(ctx).update(ctx, |manager, ctx| {
                manager.fail_chatgpt_oauth(ChatGPTConnectFailure::Unknown, ctx);
            });
            return;
        }
        self.resolve_pending_link(ctx);
    }

    /// Re-fetches the connection status. Leaves any in-flight connect attempt
    /// untouched, since the user may still be in the browser.
    pub fn refresh(&mut self, ctx: &mut ModelContext<Self>) {
        if !AuthStateProvider::as_ref(ctx).get().is_logged_in() {
            return;
        }
        let ai_client = self.ai_client.clone();
        ctx.spawn(
            async move { ai_client.get_chatgpt_connection().await },
            |_, result, ctx| match result {
                Ok(status) => ApiKeyManager::handle(ctx).update(ctx, |manager, ctx| {
                    manager.set_chatgpt_connection_status(status, ctx);
                }),
                Err(e) => log::warn!("Failed to refresh ChatGPT connection: {e:#}"),
            },
        );
    }

    /// Re-fetches the connection status and uses the result to settle the connect attempt
    /// started from this client.
    fn resolve_pending_link(&mut self, ctx: &mut ModelContext<Self>) {
        if !AuthStateProvider::as_ref(ctx).get().is_logged_in() {
            return;
        }
        let ai_client = self.ai_client.clone();
        ctx.spawn(
            async move { ai_client.get_chatgpt_connection().await },
            |_, result, ctx| {
                let status = match result {
                    Ok(status) => Some(status),
                    Err(e) => {
                        log::warn!("Failed to refresh ChatGPT connection after linking: {e:#}");
                        None
                    }
                };
                ApiKeyManager::handle(ctx).update(ctx, |manager, ctx| {
                    manager.resolve_chatgpt_oauth(status, ctx);
                });
            },
        );
    }

    /// Unlinks the ChatGPT account on the server, then re-fetches so the UI
    /// reflects what the server actually did. A successful unlink also clears the
    /// plan-modal seen marker so a later reconnect re-confirms plan sharing.
    pub fn disconnect(&mut self, ctx: &mut ModelContext<Self>) {
        let ai_client = self.ai_client.clone();
        ctx.spawn(
            async move { ai_client.disconnect_chatgpt().await },
            |me, result, ctx| {
                match result {
                    Ok(()) => AISettings::handle(ctx).update(ctx, |settings, ctx| {
                        if let Err(e) = settings.did_show_chatgpt_plan_modal.set_value(false, ctx) {
                            log::warn!("Failed to reset ChatGPT plan modal seen marker: {e}");
                        }
                    }),
                    Err(e) => {
                        log::warn!("Failed to disconnect ChatGPT: {e:#}");
                        ctx.emit(ChatGPTSubscriptionEvent::DisconnectFailed);
                    }
                }
                me.refresh(ctx);
            },
        );
    }

    /// Forgets the cached status, e.g. on logout.
    pub fn reset(&mut self, ctx: &mut ModelContext<Self>) {
        self.pending_link_state = None;
        ApiKeyManager::handle(ctx).update(ctx, |manager, ctx| {
            manager.set_chatgpt_connection_status(ChatGPTConnectionStatus::Unknown, ctx);
            manager.set_chatgpt_oauth_pending(false, ctx);
        });
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatGPTSubscriptionEvent {
    DisconnectFailed,
}

impl Entity for ChatGPTSubscriptionModel {
    type Event = ChatGPTSubscriptionEvent;
}

impl SingletonEntity for ChatGPTSubscriptionModel {}
