//! Warp input editor logic related to decorating the input's text, such as
//! applying syntax highlighting and error underlining.

use std::collections::HashMap;
use std::ops::Range;

use settings::Setting as _;
use string_offset::{ByteOffset, CharOffset};
pub use warp_completer::completer::SuggestionTypeName;
pub use warp_completer::util::parse_current_commands_and_tokens;
pub use warp_completer::{ParsedTokenData, ParsedTokensSnapshot};
use warp_core::features::FeatureFlag;
use warpui::{AppContext, SingletonEntity, ViewContext};

use super::Input;
use crate::appearance::Appearance;
use crate::completer::{EmptyCompletionContext, SessionContext};
use crate::editor::TextStyleOperation;
use crate::settings::InputSettings;
use crate::themes::theme::{AnsiColorIdentifier, AnsiColors};

/// Options to enable/disable command decoration and/or AI input background tasks spawned on input
/// edits.
#[derive(Default, Clone, Copy)]
pub struct InputBackgroundJobOptions {
    command_decoration: bool,
    ai_input_detection: bool,
}

impl InputBackgroundJobOptions {
    pub fn with_command_decoration(mut self) -> Self {
        self.command_decoration = true;
        self
    }

    pub fn with_ai_input_detection(mut self) -> Self {
        self.ai_input_detection = true;
        self
    }

    /// Returns `true` if there are no input background jobs to run. Returns `false` if there is at
    /// least one job to run.
    fn no_jobs_to_run(self) -> bool {
        !self.command_decoration && !self.ai_input_detection
    }
}

// Characters that will make us ignore commands for error underlining - largely
// a temporary solution till our parser improves (to handle special edge cases).
// Note that some of these are redundant since we split on tokens such as "&"
// but including them to be defensive.
// TODO: Remove "," once we differentiate between brace expansion and grouped commands
// at the parsing level.
const INVALID_SYMBOLS_COMMAND_ERROR_UNDERLINING: [char; 22] = [
    '~', '`', '#', '$', '&', '*', '(', ')', '\\', '|', '[', ']', '{', '}', ';', '\'', '\"', '<',
    '>', '?', '!', ',',
];

enum CompletionSessionContext {
    Session(SessionContext),
    Empty(EmptyCompletionContext),
}

/// Returns boolean indicating whether we should attempt to red underline
/// the command or not (this is a stop-gap since our parser doesn't cover
/// all the edge cases for commands currently e.g. "!!"). We don't want
/// to incorrectly red underline a valid command. In other words,
/// we would rather miss red underlining an invalid command compared to
/// incorrectly red underlining a valid command.
fn valid_command_for_error_underline(command: &str) -> bool {
    command
        .chars()
        .all(|x| !INVALID_SYMBOLS_COMMAND_ERROR_UNDERLINING.contains(&x))
}

impl Input {
    fn completion_session_context_or_empty_context(
        &self,
        ctx: &AppContext,
    ) -> CompletionSessionContext {
        self.completion_session_context(ctx)
            .map(CompletionSessionContext::Session)
            .unwrap_or_else(|| CompletionSessionContext::Empty(EmptyCompletionContext::new()))
    }

    /// Whether or not any decorations should be computed and applied to the
    /// input text.
    pub fn should_apply_decorations(&self, ctx: &ViewContext<Self>) -> bool {
        self.should_show_syntax_highlighting(ctx) || self.should_show_error_underlining(ctx)
    }

    /// Whether or not syntax highlighting should be computed and applied to the
    /// input text.
    fn should_show_syntax_highlighting(&self, ctx: &ViewContext<Self>) -> bool {
        *InputSettings::as_ref(ctx).syntax_highlighting.value()
    }

    /// Whether or not error underlining should be computed and applied to the
    /// input text.
    fn should_show_error_underlining(&self, ctx: &ViewContext<Self>) -> bool {
        *InputSettings::as_ref(ctx).error_underlining.value()
    }

    fn run_input_mode_detection(
        &self,
        completion_context: SessionContext,
        ctx: &mut ViewContext<Self>,
    ) {
        if let Some(parsed_token) = self.last_parsed_tokens.clone() {
            let session_id = completion_context.session.id();
            self.ai_input_model.update(ctx, |ai_input_model, ctx| {
                ai_input_model.detect_and_set_input_type(
                    parsed_token,
                    completion_context,
                    Some(session_id),
                    ctx,
                )
            })
        }
    }

    /// Applies background highlighting to slash command and skill command prefixes that should be
    /// syntax highlighted.
    fn apply_slash_command_prefix_highlighting(
        &mut self,
        buffer_text: &str,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        let highlighted_prefix_len = self
            .slash_command_model
            .as_ref(ctx)
            .state()
            .command_prefix_highlight_len(buffer_text);

        let Some(highlighted_prefix_len) = highlighted_prefix_len else {
            return false;
        };

        let theme = Appearance::as_ref(ctx).theme();
        let color = theme.ansi_fg_magenta();
        self.editor.update(ctx, |editor, ctx| {
            editor.update_buffer_styles(
                vec![CharOffset::from(0)..CharOffset::from(highlighted_prefix_len)],
                TextStyleOperation::default().set_syntax_color(color),
                ctx,
            )
        });
        true
    }

    /// Computes information about the currently-entered command in a background
    /// task and then uses it to decorate the input, specifically applying
    /// styles for syntax highlighting and error underlining.
    /// Includes a short-circuit that lets us clear formatting and return without parsing the input.
    pub fn run_input_background_jobs(
        &mut self,
        mode: InputBackgroundJobOptions,
        ctx: &mut ViewContext<Self>,
    ) {
        if mode.no_jobs_to_run() {
            return;
        }

        let mut mode = mode;

        // We don't show input command decorations in AI mode, but we keep slash command prefix highlighting.
        let buffer_text = self.editor.as_ref(ctx).buffer_text(ctx);
        if self.ai_input_model.as_ref(ctx).is_ai_input_enabled()
            || (FeatureFlag::AgentView.is_enabled()
                && self
                    .slash_command_model
                    .as_ref(ctx)
                    .state()
                    .is_detected_command_or_skill())
        {
            self.clear_decorations(ctx);
            self.apply_slash_command_prefix_highlighting(&buffer_text, ctx);
            mode.command_decoration = false;

            // Return early because there are no input background jobs to run.
            if mode.no_jobs_to_run() {
                return;
            }
        }

        match self.completion_session_context_or_empty_context(ctx) {
            CompletionSessionContext::Session(completion_context) => {
                let editor = self.editor.as_ref(ctx);
                let buffer_text = editor.buffer_text(ctx);

                if matches!(&self.last_parsed_tokens, Some(last_parsed_tokens) if buffer_text == last_parsed_tokens.buffer_text)
                {
                    if mode.ai_input_detection {
                        self.run_input_mode_detection(completion_context, ctx);
                    }

                    if mode.command_decoration {
                        self.apply_decorations(ctx);
                    }

                    return;
                }

                if let Some(handle) = self.decorations_future_handle.take() {
                    handle.abort_handle().abort();
                }

                let completion_session = completion_context.session.clone();

                self.decorations_future_handle = Some(ctx.spawn_abortable(
                    async move {
                        (
                            parse_current_commands_and_tokens(buffer_text, &completion_context)
                                .await,
                            completion_context,
                        )
                    },
                    move |input, (parsed_tokens, completion_context), ctx| {
                        input.last_parsed_tokens = Some(parsed_tokens);

                        if mode.ai_input_detection {
                            input.run_input_mode_detection(completion_context, ctx);
                        }

                        if mode.command_decoration {
                            input.apply_decorations(ctx);
                        }
                    },
                    move |_, _| {
                        completion_session.cancel_active_commands();
                    },
                ));
            }
            CompletionSessionContext::Empty(detection_ctx) => {
                if mode.ai_input_detection {
                    // No session context available (e.g., shared session viewer).
                    // Use a dedicated detection context that does not expose top-level commands.
                    let buffer_text = self.editor.as_ref(ctx).buffer_text(ctx);
                    let ai_input_model = self.ai_input_model.clone();
                    ctx.spawn(
                        async move {
                            parse_current_commands_and_tokens(buffer_text, &detection_ctx).await
                        },
                        move |_input, parsed_tokens, ctx| {
                            ai_input_model.update(ctx, |model, ctx| {
                                model.detect_and_set_input_type(
                                    parsed_tokens,
                                    EmptyCompletionContext::new(),
                                    None,
                                    ctx,
                                );
                            });
                        },
                    );
                }
            }
        }
    }

    /// Applies error underlining and/or syntax highlighting as appropriate,
    /// using the result of the last parse operation.
    fn apply_decorations(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(parsed_tokens_snapshot) = &self.last_parsed_tokens else {
            return;
        };
        let buffer_text = self.editor.as_ref(ctx).buffer_text(ctx);
        if buffer_text != parsed_tokens_snapshot.buffer_text {
            // Our state is out-of-date (parsed_tokens no longer applies to the
            // updated state of the buffer) and this should be a no-op since
            // another async callback is likely handling or already handled
            // this (for a later state) i.e. race condition.
            return;
        }

        // Clear all decorations before applying the updated ones.
        self.clear_decorations(ctx);

        let theme = Appearance::as_ref(ctx).theme();
        let terminal_colors_normal = theme.terminal_colors().normal;

        if self.should_show_syntax_highlighting(ctx) {
            self.apply_colors_syntax_highlighting_all_tokens(terminal_colors_normal, ctx);
        }
        if self.should_show_error_underlining(ctx) {
            self.apply_colors_error_underlining_all_tokens(terminal_colors_normal, ctx);
        }
    }

    /// Removes decorations (error underlining, syntax highlighting, and background colors) from the input buffer.
    pub(super) fn clear_decorations(&mut self, ctx: &mut ViewContext<Self>) {
        self.editor.update(ctx, |editor, ctx| {
            editor.update_buffer_styles(
                vec![CharOffset::from(0)..editor.buffer_size(ctx)],
                TextStyleOperation::default().clear_decorations(),
                ctx,
            )
        });
    }

    /// Applies error underlining appropriately to all given tokens, given the
    /// parsed tokens data.
    ///
    /// This does not unset any existing error underline decorations in the
    /// editor buffer.
    fn apply_colors_error_underlining_all_tokens(
        &mut self,
        terminal_colors_normal: AnsiColors,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(parsed_tokens_snapshot) = &self.last_parsed_tokens else {
            return;
        };

        let mut ranges = vec![];
        for token_data in &parsed_tokens_snapshot.parsed_tokens {
            if token_data.token_description.is_none()
                && token_data.token_index == 0
                && valid_command_for_error_underline(&token_data.token.item)
            {
                ranges.push(
                    ByteOffset::from(token_data.token.span.start())
                        ..ByteOffset::from(token_data.token.span.end()),
                );
            }
        }

        if !ranges.is_empty() {
            self.editor.update(ctx, |editor, ctx| {
                editor.update_buffer_styles(
                    ranges,
                    TextStyleOperation::default().set_error_underline_color(
                        AnsiColorIdentifier::Red
                            .to_ansi_color(&terminal_colors_normal)
                            .into(),
                    ),
                    ctx,
                )
            });
        }
    }

    /// Applies syntax highlighting colors appropriately to all given tokens,
    /// given the parsed tokens data.
    ///
    /// This does not unset any existing syntax highlighting decorations in the
    /// editor buffer.
    fn apply_colors_syntax_highlighting_all_tokens(
        &mut self,
        terminal_colors_normal: AnsiColors,
        ctx: &mut ViewContext<Input>,
    ) {
        let Some(parsed_tokens_snapshot) = &self.last_parsed_tokens else {
            return;
        };

        let mut ranges: HashMap<SuggestionTypeName, Vec<Range<ByteOffset>>> = HashMap::new();
        for token_data in &parsed_tokens_snapshot.parsed_tokens {
            if let Some(description) = &token_data.token_description {
                let suggestion_type = description.suggestion_type;
                let range = ByteOffset::from(token_data.token.span.start())
                    ..ByteOffset::from(token_data.token.span.end());
                ranges
                    .entry(suggestion_type.to_name())
                    .or_default()
                    .push(range);
            }
        }

        for (suggestion_type, ranges) in ranges {
            let color: AnsiColorIdentifier = suggestion_type.into();
            self.editor.update(ctx, |editor, ctx| {
                editor.update_buffer_styles(
                    ranges,
                    TextStyleOperation::default()
                        .set_syntax_color(color.to_ansi_color(&terminal_colors_normal).into()),
                    ctx,
                )
            });
        }
    }
}

#[cfg(test)]
#[path = "decorations_tests.rs"]
mod tests;
