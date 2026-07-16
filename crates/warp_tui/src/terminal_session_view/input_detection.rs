//! TUI input detection coordination and Agent-biased classification gating.
//!
//! The parser, completion context, and
//! [`BlocklistAIInputModel`](warp::tui_export::BlocklistAIInputModel) classifier
//! are shared with the GUI input. The orchestration remains frontend-specific:
//! GUI input detection is coupled to command decorations, parsed-token caching,
//! shared-session edits, and GUI suggestion modes, while the TUI owns a
//! separate editor, inline menus, future lifecycle, and short-input Agent bias.
//! Keeping those coordinators separate avoids a broad callback abstraction
//! while preserving the classification behavior in the shared model.

use std::time::Duration;

use warp::tui_export::{
    parse_current_commands_and_tokens, tui_completion_context_has_exact_command,
    tui_completion_session_context, InputType,
};
use warp_editor::model::CoreEditorModel;
use warpui_core::r#async::{SpawnedFutureHandle, Timer};
use warpui_core::ViewContext;

use super::TuiTerminalSessionView;
use crate::inline_menu::active_inline_menu;

const INPUT_AUTODETECTION_DEBOUNCE: Duration = Duration::from_millis(10);
const MIN_STANDALONE_COMMAND_CHARS: usize = 2;

#[derive(Default)]
pub(super) struct InputDetectionState {
    future: Option<SpawnedFutureHandle>,
}

/// The next action the TUI input-detection coordinator should take for a
/// buffer snapshot. This is not classifier confidence: it prevents empty or
/// weak partial input from reaching the shared classifier while allowing
/// non-empty input to be parsed and strong command candidates to be classified.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InputDetectionDecision {
    /// Return unlocked input to Agent without running detection.
    ResetToAgent,
    /// Parse the non-empty buffer to gather command evidence.
    Parse,
    /// Run the shared Agent/Shell classifier on the parsed snapshot.
    Classify,
}

fn input_detection_decision(
    buffer_text: &str,
    parsed_token_count: Option<usize>,
    first_token_char_count: usize,
    first_token_has_command_evidence: bool,
) -> InputDetectionDecision {
    if buffer_text.trim().is_empty() {
        return InputDetectionDecision::ResetToAgent;
    }
    let Some(parsed_token_count) = parsed_token_count else {
        return InputDetectionDecision::Parse;
    };
    // Multi-token input has enough context to send through the shared classifier. A single token
    // is classified only when it has at least two characters and exactly matches command evidence
    // from the live shell, completion description, or command-signature registry. This keeps short
    // or partial input biased toward Agent while preserving known standalone commands.
    if parsed_token_count >= 2
        || (parsed_token_count == 1
            && first_token_char_count >= MIN_STANDALONE_COMMAND_CHARS
            && first_token_has_command_evidence)
    {
        InputDetectionDecision::Classify
    } else {
        InputDetectionDecision::ResetToAgent
    }
}

fn should_reset_input_to_agent(
    decision: InputDetectionDecision,
    is_input_type_locked: bool,
) -> bool {
    !is_input_type_locked && decision == InputDetectionDecision::ResetToAgent
}

/// Returns whether an asynchronous parse result still applies to the live input.
///
/// This only validates snapshot freshness and that the buffer is not serving as an inline-menu
/// query. [`input_detection_decision`] separately decides whether valid parsed input should reset
/// to Agent or run through the shared classifier.
fn parsed_result_is_applicable(
    parsed_buffer_text: &str,
    current_buffer_text: &str,
    has_active_inline_menu: bool,
) -> bool {
    parsed_buffer_text == current_buffer_text && !has_active_inline_menu
}

impl TuiTerminalSessionView {
    pub(super) fn handle_input_content_changed(
        &mut self,
        is_user_edit: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        self.abort_input_detection(ctx);
        if is_user_edit {
            self.schedule_input_detection(ctx);
        }
    }

    fn abort_input_detection(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(future) = self.input_detection.future.take() {
            future.abort();
        }
        self.ai_input_model.update(ctx, |input_mode, _| {
            input_mode.abort_in_progress_detection();
        });
    }

    fn reset_input_to_agent(
        &mut self,
        decision: InputDetectionDecision,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        let is_input_type_locked = self.ai_input_model.as_ref(ctx).is_input_type_locked();
        if !should_reset_input_to_agent(decision, is_input_type_locked) {
            return false;
        }
        self.ai_input_model.update(ctx, |input_mode, ctx| {
            input_mode.enable_autodetection(InputType::AI, ctx);
        });
        true
    }

    pub(super) fn schedule_input_detection(&mut self, ctx: &mut ViewContext<Self>) {
        self.abort_input_detection(ctx);
        let buffer_text = {
            let editor = self.input_view.as_ref(ctx).model().as_ref(ctx);
            editor.content().as_ref(ctx).text().into_string()
        };
        let decision = input_detection_decision(&buffer_text, None, 0, false);
        if self.reset_input_to_agent(decision, ctx) {
            return;
        }
        if !self
            .ai_input_model
            .as_ref(ctx)
            .should_run_input_autodetection(ctx)
        {
            return;
        }
        let Some(current_working_directory) = self.current_working_directory(ctx) else {
            return;
        };
        let Some(completion_context) = tui_completion_session_context(
            self.active_session.as_ref(ctx),
            current_working_directory,
            ctx,
        ) else {
            return;
        };
        let completion_session = completion_context.session.clone();

        self.input_detection.future = Some(ctx.spawn_abortable(
            async move {
                Timer::after(INPUT_AUTODETECTION_DEBOUNCE).await;
                let parsed =
                    parse_current_commands_and_tokens(buffer_text.clone(), &completion_context)
                        .await;
                (buffer_text, parsed, completion_context)
            },
            move |view, (parsed_buffer_text, parsed, completion_context), ctx| {
                view.input_detection.future = None;
                let current_buffer_text = {
                    let editor = view.input_view.as_ref(ctx).model().as_ref(ctx);
                    editor.content().as_ref(ctx).text().into_string()
                };
                let has_active_inline_menu = active_inline_menu(
                    &view.inline_menus,
                    view.suggestions_mode.as_ref(ctx).mode(),
                    ctx,
                )
                .is_some();
                if !parsed_result_is_applicable(
                    &parsed_buffer_text,
                    &current_buffer_text,
                    has_active_inline_menu,
                ) {
                    return;
                }
                let first_token = parsed.parsed_tokens.first();
                let first_token_has_command_evidence = first_token.is_some_and(|token| {
                    token.token_description.is_some()
                        || tui_completion_context_has_exact_command(
                            &completion_context,
                            token.token.as_str(),
                        )
                });
                let decision = input_detection_decision(
                    &parsed.buffer_text,
                    Some(parsed.parsed_tokens.len()),
                    first_token
                        .map(|token| token.token.chars().count())
                        .unwrap_or_default(),
                    first_token_has_command_evidence,
                );
                if view.reset_input_to_agent(decision, ctx) {
                    return;
                }
                if decision != InputDetectionDecision::Classify {
                    return;
                }
                let session_id = completion_context.session.id();
                view.ai_input_model.update(ctx, |input_mode, ctx| {
                    input_mode.detect_and_set_input_type(
                        parsed,
                        completion_context,
                        Some(session_id),
                        ctx,
                    );
                });
            },
            move |_, _| {
                completion_session.cancel_active_commands();
            },
        ));
    }
}

#[cfg(test)]
#[path = "input_detection_tests.rs"]
mod tests;
