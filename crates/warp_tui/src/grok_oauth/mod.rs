//! View-independent Grok OAuth state used by the `/api-keys` inline menu.

use ai::api_keys::ApiKeyManager;
use ai::grok_subscription::oauth::{
    ManualCodeExchange, OauthAttempt, OauthCancellationHandle, TokenResponse,
};
use uuid::Uuid;
use warpui::SingletonEntity as _;
use warpui_core::{Entity, ModelContext};

const CALLBACK_FAILURE_MESSAGE: &str =
    "Couldn't complete Grok authorization. Press Esc, then select Grok to try again.";
const MANUAL_FAILURE_MESSAGE: &str =
    "Couldn't connect Grok with that code. Check the code and try again.";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TuiGrokOAuthControllerEvent {
    Connected,
    Updated,
}

#[derive(Debug)]
enum TuiGrokOAuthPhase {
    Waiting { manual_error: Option<String> },
    ExchangingManualCode,
    Fatal(String),
}

pub(crate) struct TuiGrokOAuthController {
    active_attempt_id: Option<Uuid>,
    manual_exchange: Option<ManualCodeExchange>,
    cancellation: Option<OauthCancellationHandle>,
    phase: TuiGrokOAuthPhase,
    callback_error: Option<String>,
}

impl TuiGrokOAuthController {
    pub(crate) fn new(attempt: OauthAttempt, ctx: &mut ModelContext<Self>) -> Self {
        let attempt_id = Uuid::new_v4();
        let authorize_url = attempt.authorize_url();
        let manual_exchange = attempt.manual_code_exchange();
        let cancellation = attempt.cancellation_handle();

        ctx.open_url(&authorize_url);
        ctx.spawn(
            async move { attempt.finish().await },
            move |controller, result, ctx| {
                controller.handle_callback_result(attempt_id, result, ctx);
            },
        );

        Self {
            active_attempt_id: Some(attempt_id),
            manual_exchange: Some(manual_exchange),
            cancellation: Some(cancellation),
            phase: TuiGrokOAuthPhase::Waiting { manual_error: None },
            callback_error: None,
        }
    }

    pub(crate) fn is_active(&self) -> bool {
        self.active_attempt_id.is_some()
    }

    pub(crate) fn is_exchanging(&self) -> bool {
        matches!(self.phase, TuiGrokOAuthPhase::ExchangingManualCode)
    }

    pub(crate) fn error(&self) -> Option<&str> {
        match &self.phase {
            TuiGrokOAuthPhase::Waiting { manual_error } => manual_error.as_deref(),
            TuiGrokOAuthPhase::ExchangingManualCode => None,
            TuiGrokOAuthPhase::Fatal(error) => Some(error),
        }
    }

    pub(crate) fn submit_manual_code(&mut self, code: String, ctx: &mut ModelContext<Self>) {
        let Some(attempt_id) = self.active_attempt_id else {
            return;
        };
        if !matches!(self.phase, TuiGrokOAuthPhase::Waiting { .. }) {
            return;
        }
        let Some(exchange) = self.manual_exchange.clone() else {
            return;
        };
        if code.trim().is_empty() {
            self.phase = TuiGrokOAuthPhase::Waiting {
                manual_error: Some(
                    "Enter the code shown in your browser to finish connecting.".to_owned(),
                ),
            };
            ctx.emit(TuiGrokOAuthControllerEvent::Updated);
            return;
        }

        self.phase = TuiGrokOAuthPhase::ExchangingManualCode;
        ctx.spawn(
            async move { exchange.exchange(&code).await },
            move |controller, result, ctx| {
                controller.handle_manual_result(attempt_id, result, ctx);
            },
        );
        ctx.emit(TuiGrokOAuthControllerEvent::Updated);
    }

    pub(crate) fn clear_manual_error(&mut self, ctx: &mut ModelContext<Self>) {
        if let TuiGrokOAuthPhase::Waiting { manual_error } = &mut self.phase
            && manual_error.take().is_some()
        {
            ctx.emit(TuiGrokOAuthControllerEvent::Updated);
        }
    }

    pub(crate) fn cancel(&mut self, ctx: &mut ModelContext<Self>) {
        if self.active_attempt_id.take().is_none() {
            return;
        }
        if let Some(cancellation) = self.cancellation.take() {
            cancellation.cancel();
        }
        self.manual_exchange = None;
        ctx.emit(TuiGrokOAuthControllerEvent::Updated);
    }

    fn handle_callback_result(
        &mut self,
        attempt_id: Uuid,
        result: anyhow::Result<TokenResponse>,
        ctx: &mut ModelContext<Self>,
    ) {
        if self.active_attempt_id != Some(attempt_id) {
            return;
        }
        match result {
            Ok(tokens) => self.finish_success(attempt_id, tokens, ctx),
            Err(_) if self.is_exchanging() => {
                self.callback_error = Some(CALLBACK_FAILURE_MESSAGE.to_owned());
            }
            Err(_) => {
                self.phase = TuiGrokOAuthPhase::Fatal(CALLBACK_FAILURE_MESSAGE.to_owned());
                ctx.emit(TuiGrokOAuthControllerEvent::Updated);
            }
        }
    }

    fn handle_manual_result(
        &mut self,
        attempt_id: Uuid,
        result: anyhow::Result<TokenResponse>,
        ctx: &mut ModelContext<Self>,
    ) {
        if self.active_attempt_id != Some(attempt_id) {
            return;
        }
        match result {
            Ok(tokens) => self.finish_success(attempt_id, tokens, ctx),
            Err(_) => {
                self.phase = match self.callback_error.take() {
                    Some(error) => TuiGrokOAuthPhase::Fatal(error),
                    None => TuiGrokOAuthPhase::Waiting {
                        manual_error: Some(MANUAL_FAILURE_MESSAGE.to_owned()),
                    },
                };
                ctx.emit(TuiGrokOAuthControllerEvent::Updated);
            }
        }
    }

    fn finish_success(
        &mut self,
        attempt_id: Uuid,
        tokens: TokenResponse,
        ctx: &mut ModelContext<Self>,
    ) {
        if self.active_attempt_id != Some(attempt_id) {
            return;
        }
        self.active_attempt_id = None;
        if let Some(cancellation) = self.cancellation.take() {
            cancellation.cancel();
        }
        self.manual_exchange = None;
        ApiKeyManager::handle(ctx).update(ctx, move |manager, ctx| {
            manager.store_grok_tokens(tokens, ctx);
        });
        ctx.emit(TuiGrokOAuthControllerEvent::Connected);
    }
}

impl Drop for TuiGrokOAuthController {
    fn drop(&mut self) {
        if let Some(cancellation) = self.cancellation.take() {
            cancellation.cancel();
        }
    }
}

impl Entity for TuiGrokOAuthController {
    type Event = TuiGrokOAuthControllerEvent;
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
