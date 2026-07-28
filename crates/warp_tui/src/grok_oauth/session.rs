//! Grok OAuth lifetime and slash-command integration for a TUI session.

use ai::api_keys::ApiKeyManager;
use ai::grok_subscription::oauth::OauthAttempt;
use warp::tui_export::{UserWorkspaces, record_static_slash_command_accepted};
use warp_core::features::FeatureFlag;
use warpui::SingletonEntity;
use warpui_core::{AppContext, ViewContext, ViewHandle};

use super::TuiTerminalSessionView;
use crate::grok_oauth::{TuiGrokOAuthBlock, TuiGrokOAuthBlockEvent};

const GROK_CONNECTED_HINT: &str = "Grok connected";
const GROK_CLEARED_HINT: &str = "Grok credentials cleared";

impl TuiTerminalSessionView {
    pub(super) fn active_grok_oauth(
        &self,
        ctx: &AppContext,
    ) -> Option<ViewHandle<TuiGrokOAuthBlock>> {
        let block = self.grok_oauth.as_ref()?;
        block.as_ref(ctx).is_active().then(|| block.clone())
    }

    fn grok_oauth_policy_error(&self, ctx: &AppContext) -> Option<&'static str> {
        if !FeatureFlag::SuperGrok.is_enabled() {
            return Some("Grok subscriptions aren't available in this build.");
        }
        let workspaces = UserWorkspaces::as_ref(ctx);
        if !workspaces.is_byo_api_key_enabled(ctx) {
            return Some("Grok subscriptions require BYOK access for this workspace.");
        }
        if !workspaces.are_member_byo_keys_allowed() {
            return Some("Your organization doesn't allow member-provided credentials.");
        }
        None
    }

    pub(super) fn start_grok_oauth(
        &mut self,
        command_name: &'static str,
        ctx: &mut ViewContext<Self>,
    ) {
        if let Some(error) = self.grok_oauth_policy_error(ctx) {
            self.show_error_hint(error.to_owned(), ctx);
            return;
        }
        if ApiKeyManager::as_ref(ctx).has_grok_subscription() {
            self.show_error_hint(
                "Grok is already connected. Run /clear-provider-api-key grok to disconnect."
                    .to_owned(),
                ctx,
            );
            return;
        }
        if let Some(block) = self.active_grok_oauth(ctx) {
            ctx.focus(&block);
            return;
        }
        let Ok(state) = self.session_state(ctx) else {
            return;
        };
        if state.has_blocking_interaction() || !state.input_target().agent_editor_owns_input() {
            self.show_error_hint(
                "Finish the current interaction before connecting Grok.".to_owned(),
                ctx,
            );
            return;
        }
        let attempt = match OauthAttempt::start() {
            Ok(attempt) => attempt,
            Err(error) => {
                self.show_error_hint(error.to_string(), ctx);
                return;
            }
        };

        self.input_view.update(ctx, |input, ctx| input.clear(ctx));
        let block = ctx.add_typed_action_tui_view(move |ctx| TuiGrokOAuthBlock::new(attempt, ctx));
        self.install_grok_oauth_block(block, ctx);
        record_static_slash_command_accepted(command_name, true, ctx);
    }

    pub(super) fn install_grok_oauth_block(
        &mut self,
        block: ViewHandle<TuiGrokOAuthBlock>,
        ctx: &mut ViewContext<Self>,
    ) {
        ctx.subscribe_to_view(&block, |view, _, event, ctx| {
            view.handle_grok_oauth_event(event, ctx);
        });
        self.grok_oauth = Some(block);
        self.refresh_input_focus(ctx);
    }

    pub(super) fn clear_grok_oauth(
        &mut self,
        command_name: &'static str,
        ctx: &mut ViewContext<Self>,
    ) {
        ApiKeyManager::handle(ctx).update(ctx, |manager, ctx| {
            manager.set_grok_tokens(None, ctx);
        });
        self.input_view.update(ctx, |input, ctx| input.clear(ctx));
        self.show_success_hint(GROK_CLEARED_HINT.to_owned(), ctx);
        record_static_slash_command_accepted(command_name, true, ctx);
    }

    pub(super) fn handle_grok_oauth_event(
        &mut self,
        event: &TuiGrokOAuthBlockEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            TuiGrokOAuthBlockEvent::Connected => {
                self.grok_oauth = None;
                self.show_success_hint(GROK_CONNECTED_HINT.to_owned(), ctx);
                self.refresh_input_focus(ctx);
            }
            TuiGrokOAuthBlockEvent::Cancelled => {
                self.grok_oauth = None;
                self.refresh_input_focus(ctx);
            }
            TuiGrokOAuthBlockEvent::LayoutInvalidated => ctx.notify(),
        }
    }
}
