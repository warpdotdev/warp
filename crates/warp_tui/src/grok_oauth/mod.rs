//! Input-blocking Grok OAuth interaction for the active TUI session.

use ai::api_keys::ApiKeyManager;
use ai::grok_subscription::oauth::{
    ManualCodeExchange, OauthAttempt, OauthCancellationHandle, TokenResponse,
};
use uuid::Uuid;
use warpui::SingletonEntity;
use warpui_core::elements::CrossAxisAlignment;
use warpui_core::elements::tui::{TuiChildView, TuiContainer, TuiElement, TuiFlex, TuiText};
use warpui_core::keymap::macros::*;
use warpui_core::keymap::{self, FixedBinding};
use warpui_core::{
    AppContext, Entity, EntityId, FocusContext, TuiView, TypedActionView, ViewContext, ViewHandle,
};

use crate::editor_view::{TuiEditorView, TuiEditorViewEvent};
use crate::keybindings::TUI_BINDING_GROUP;
use crate::transcript_view::BLOCK_TOP_PADDING_ROWS;
use crate::tui_builder::TuiUiBuilder;

const TITLE: &str = "Connect Grok";
const CALLBACK_FAILURE_MESSAGE: &str =
    "Couldn't complete Grok authorization. Press Esc, then run /add-api-key grok to try again.";
const MANUAL_FAILURE_MESSAGE: &str =
    "Couldn't connect Grok with that code. Check the code and try again.";

pub(crate) fn init(app: &mut AppContext) {
    let context = id!(TuiGrokOAuthBlock::ui_name());
    app.register_fixed_bindings([
        FixedBinding::new(
            "enter",
            TuiGrokOAuthBlockAction::SubmitManualCode,
            context.clone(),
        )
        .with_group(TUI_BINDING_GROUP),
        FixedBinding::new(
            "numpadenter",
            TuiGrokOAuthBlockAction::SubmitManualCode,
            context.clone(),
        )
        .with_group(TUI_BINDING_GROUP),
        FixedBinding::new("escape", TuiGrokOAuthBlockAction::Cancel, context)
            .with_group(TUI_BINDING_GROUP),
    ]);
}

#[derive(Clone)]
pub(crate) enum TuiGrokOAuthBlockEvent {
    Connected,
    Cancelled,
    LayoutInvalidated,
}

#[derive(Clone, Debug)]
pub(crate) enum TuiGrokOAuthBlockAction {
    SubmitManualCode,
    Cancel,
}

enum TuiGrokOAuthPhase {
    Waiting { manual_error: Option<String> },
    ExchangingManualCode,
    Fatal(String),
}

pub(crate) struct TuiGrokOAuthBlock {
    active_attempt_id: Option<Uuid>,
    manual_exchange: Option<ManualCodeExchange>,
    cancellation: Option<OauthCancellationHandle>,
    code_editor: ViewHandle<TuiEditorView>,
    phase: TuiGrokOAuthPhase,
    callback_error: Option<String>,
}

impl TuiGrokOAuthBlock {
    pub(crate) fn new(attempt: OauthAttempt, ctx: &mut ViewContext<Self>) -> Self {
        let attempt_id = Uuid::new_v4();
        let authorize_url = attempt.authorize_url();
        let manual_exchange = attempt.manual_code_exchange();
        let cancellation = attempt.cancellation_handle();
        let code_editor = ctx.add_typed_action_tui_view(TuiEditorView::single_line);
        ctx.subscribe_to_view(&code_editor, |block, _, event, ctx| {
            if matches!(event, TuiEditorViewEvent::Changed(_))
                && let TuiGrokOAuthPhase::Waiting { manual_error } = &mut block.phase
            {
                manual_error.take();
                ctx.emit(TuiGrokOAuthBlockEvent::LayoutInvalidated);
                ctx.notify();
            }
        });

        ctx.open_url(&authorize_url);
        ctx.spawn(
            async move { attempt.finish().await },
            move |block, result, ctx| {
                block.handle_callback_result(attempt_id, result, ctx);
            },
        );

        Self {
            active_attempt_id: Some(attempt_id),
            manual_exchange: Some(manual_exchange),
            cancellation: Some(cancellation),
            code_editor,
            phase: TuiGrokOAuthPhase::Waiting { manual_error: None },
            callback_error: None,
        }
    }

    pub(crate) fn is_active(&self) -> bool {
        self.active_attempt_id.is_some()
    }

    fn handle_callback_result(
        &mut self,
        attempt_id: Uuid,
        result: anyhow::Result<TokenResponse>,
        ctx: &mut ViewContext<Self>,
    ) {
        if self.active_attempt_id != Some(attempt_id) {
            return;
        }
        match result {
            Ok(tokens) => self.finish_success(attempt_id, tokens, ctx),
            Err(_) if matches!(&self.phase, TuiGrokOAuthPhase::ExchangingManualCode) => {
                self.callback_error = Some(CALLBACK_FAILURE_MESSAGE.to_owned());
            }
            Err(_) => {
                self.phase = TuiGrokOAuthPhase::Fatal(CALLBACK_FAILURE_MESSAGE.to_owned());
                ctx.emit(TuiGrokOAuthBlockEvent::LayoutInvalidated);
                ctx.notify();
            }
        }
    }

    fn submit_manual_code(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(attempt_id) = self.active_attempt_id else {
            return;
        };
        if !matches!(&self.phase, TuiGrokOAuthPhase::Waiting { .. }) {
            return;
        }
        let Some(exchange) = self.manual_exchange.clone() else {
            return;
        };
        let code = self.code_editor.as_ref(ctx).text(ctx);
        if code.trim().is_empty() {
            self.phase = TuiGrokOAuthPhase::Waiting {
                manual_error: Some(
                    "Enter the code shown in your browser to finish connecting.".to_owned(),
                ),
            };
            ctx.emit(TuiGrokOAuthBlockEvent::LayoutInvalidated);
            ctx.notify();
            return;
        }

        self.code_editor
            .update(ctx, |editor, ctx| editor.set_text("", ctx));
        self.phase = TuiGrokOAuthPhase::ExchangingManualCode;
        ctx.spawn(
            async move { exchange.exchange(&code).await },
            move |block, result, ctx| {
                block.handle_manual_result(attempt_id, result, ctx);
            },
        );
        ctx.emit(TuiGrokOAuthBlockEvent::LayoutInvalidated);
        ctx.notify();
    }

    fn handle_manual_result(
        &mut self,
        attempt_id: Uuid,
        result: anyhow::Result<TokenResponse>,
        ctx: &mut ViewContext<Self>,
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
                ctx.emit(TuiGrokOAuthBlockEvent::LayoutInvalidated);
                ctx.notify();
            }
        }
    }

    fn finish_success(
        &mut self,
        attempt_id: Uuid,
        tokens: TokenResponse,
        ctx: &mut ViewContext<Self>,
    ) {
        if self.active_attempt_id != Some(attempt_id) {
            return;
        }
        self.active_attempt_id = None;
        if let Some(cancellation) = self.cancellation.take() {
            cancellation.cancel();
        }
        self.manual_exchange = None;
        self.code_editor
            .update(ctx, |editor, ctx| editor.set_text("", ctx));
        ApiKeyManager::handle(ctx).update(ctx, move |manager, ctx| {
            manager.store_grok_tokens(tokens, ctx);
        });
        ctx.emit(TuiGrokOAuthBlockEvent::Connected);
    }

    fn cancel(&mut self, ctx: &mut ViewContext<Self>) {
        if self.active_attempt_id.take().is_none() {
            return;
        }
        if let Some(cancellation) = self.cancellation.take() {
            cancellation.cancel();
        }
        self.manual_exchange = None;
        self.code_editor
            .update(ctx, |editor, ctx| editor.set_text("", ctx));
        ctx.emit(TuiGrokOAuthBlockEvent::Cancelled);
    }

    fn render_body(&self, builder: &TuiUiBuilder) -> Box<dyn TuiElement> {
        let mut body = TuiFlex::column()
            .child(
                TuiText::new("Complete sign-in in the browser window that just opened.")
                    .with_style(builder.primary_text_style())
                    .finish(),
            )
            .child(
                TuiText::new("If xAI shows an authorization code, paste it below:")
                    .with_style(builder.muted_text_style())
                    .finish(),
            );

        match &self.phase {
            TuiGrokOAuthPhase::Waiting { manual_error } => {
                body = body.child(
                    TuiContainer::new(TuiChildView::new(&self.code_editor).finish())
                        .with_border_style(builder.grok_oauth_accent_style())
                        .with_padding_x(1)
                        .finish(),
                );
                if let Some(error) = manual_error {
                    body = body.child(
                        TuiText::new(error.clone())
                            .with_style(builder.error_text_style())
                            .finish(),
                    );
                }
            }
            TuiGrokOAuthPhase::ExchangingManualCode => {
                body = body.child(
                    TuiText::new("Connecting…")
                        .with_style(builder.muted_text_style())
                        .finish(),
                );
            }
            TuiGrokOAuthPhase::Fatal(error) => {
                body = body.child(
                    TuiText::new(error.clone())
                        .with_style(builder.error_text_style())
                        .finish(),
                );
            }
        }
        body.finish()
    }
}

impl Drop for TuiGrokOAuthBlock {
    fn drop(&mut self) {
        if let Some(cancellation) = self.cancellation.take() {
            cancellation.cancel();
        }
    }
}

impl Entity for TuiGrokOAuthBlock {
    type Event = TuiGrokOAuthBlockEvent;
}

impl TuiView for TuiGrokOAuthBlock {
    fn ui_name() -> &'static str {
        "TuiGrokOAuthBlock"
    }

    fn child_view_ids(&self, _ctx: &AppContext) -> Vec<EntityId> {
        vec![self.code_editor.id()]
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() && matches!(&self.phase, TuiGrokOAuthPhase::Waiting { .. }) {
            ctx.focus(&self.code_editor);
        }
    }

    fn keymap_context(&self, _ctx: &AppContext) -> keymap::Context {
        let mut context = keymap::Context::default();
        context.set.insert(Self::ui_name());
        context
    }

    fn render(&self, ctx: &AppContext) -> Box<dyn TuiElement> {
        let builder = TuiUiBuilder::from_app(ctx);
        let header = TuiContainer::new(
            TuiText::from_spans([
                ("■ ".to_owned(), builder.grok_oauth_accent_style()),
                (TITLE.to_owned(), builder.primary_text_style()),
            ])
            .finish(),
        )
        .with_background(builder.grok_oauth_header_background())
        .with_padding_x(1)
        .finish();
        let body = TuiContainer::new(self.render_body(&builder))
            .with_background(builder.grok_oauth_surface_background())
            .with_padding_x(3)
            .with_padding_y(1)
            .finish();
        let footer = TuiText::from_spans([
            ("Esc ".to_owned(), builder.primary_text_style()),
            ("to close".to_owned(), builder.muted_text_style()),
        ])
        .finish();
        TuiContainer::new(
            TuiFlex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(header)
                .child(body)
                .child(TuiContainer::new(footer).with_padding_top(1).finish())
                .finish(),
        )
        .with_padding_top(BLOCK_TOP_PADDING_ROWS)
        .finish()
    }
}

impl TypedActionView for TuiGrokOAuthBlock {
    type Action = TuiGrokOAuthBlockAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            TuiGrokOAuthBlockAction::SubmitManualCode => self.submit_manual_code(ctx),
            TuiGrokOAuthBlockAction::Cancel => self.cancel(ctx),
        }
    }
}

#[cfg(test)]
pub(crate) use tests::new_block;
#[cfg(test)]
#[path = "tests.rs"]
mod tests;
