//! Footer bar for "Use agent" functionality during long-running commands.
//!
//! This module provides a footer that appears at the bottom of active long running blocks,
//! offering users the option to bring in the agent. For CLI agent commands (e.g., Claude Code,
//! Gemini CLI, Codex), it displays a specialized footer with additional functionality.

use crate::ai::agent::ImageContext;
use crate::ai::blocklist::agent_view::agent_input_footer::{
    AgentInputFooter, AgentInputFooterEvent,
};
use crate::terminal::cli_agent_sessions::listener::agent_supports_rich_status;
use crate::terminal::cli_agent_sessions::{CLIAgentInputEntrypoint, CLIAgentSessionsModel};
use crate::terminal::shared_session::{SharedSessionActionSource, SharedSessionScrollbackType};
use crate::util::image::{infer_mime_type, MAX_IMAGE_SIZE_BYTES_FOR_CLI_AGENT, MIME_SNIFF_BYTES};
use base64::Engine;
use session_sharing_protocol::sharer::SessionSourceType;
use warpui::clipboard::{ClipboardContent, ImageData};
mod warpify_footer;

pub use crate::terminal::CLIAgent;
use warpify_footer::{WarpifyFooterView, WarpifyFooterViewEvent};

use std::path::Path;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use warpui::r#async::Timer;

use crate::code_review::diff_state::GitDeltaPreference;
use crate::code_review::telemetry_event::CodeReviewPaneEntrypoint;
use anyhow::anyhow;
use parking_lot::FairMutex;
use pathfinder_color::ColorU;
use warp_core::{
    features::FeatureFlag,
    report_error, send_telemetry_from_ctx,
    settings::Setting,
    ui::{
        appearance::Appearance,
        color::contrast::{
            high_enough_contrast, pick_best_foreground_color, MinimumAllowedContrast,
        },
        theme::{color::internal_colors, Fill as ThemeFill},
    },
};

use warpui::{
    elements::{
        ChildView, Container, CrossAxisAlignment, Empty, Expanded, Flex, MainAxisSize,
        ParentElement,
    },
    keymap::Keystroke,
    AppContext, Element, Entity, EntityId, ModelHandle, SingletonEntity, TypedActionView, View,
    ViewContext, ViewHandle,
};

use crate::{
    ai::blocklist::{agent_view::agent_view_bg_fill, block::cli_controller::CLISubagentEvent},
    cmd_or_ctrl_shift,
    server::telemetry::{CLIAgentType, CLISubagentControlState, TelemetryEvent},
    settings::{
        AISettings, AISettingsChangedEvent, CompiledCommandsForCodingAgentToolbar,
        InputModeSettings,
    },
    terminal::cli_agent_sessions::CLIAgentRichInputCloseReason,
    terminal::{
        model_events::{ModelEvent, ModelEventDispatcher},
        TerminalModel,
    },
    ui_components::{blended_colors, icons::Icon},
    view_components::action_button::{
        ActionButton, ActionButtonTheme, ButtonSize, KeystrokeSource, TooltipAlignment,
    },
};

use warp_terminal::model::escape_sequences::{BRACKETED_PASTE_END, BRACKETED_PASTE_START};

use super::{RichContentInsertionPosition, TerminalAction, TerminalView};
use crate::terminal::view::block_banner::WarpificationMode;

/// Small delay inserted between separate PTY writes to CLI agents.
/// (Used both for the mode-switch prefix split and for the `DelayedEnter`
/// submit strategy so each write is delivered as a distinct stdin read.)
const CLI_AGENT_PTY_WRITE_DELAY: Duration = Duration::from_millis(50);

/// Longer delay for agents (like Copilot) that need extra time after a
/// bracketed paste before they will accept a submit keystroke.
const CLI_AGENT_BRACKETED_PASTE_ENTER_DELAY: Duration = Duration::from_millis(300);

/// Longer delay between clipboard image pastes (Ctrl+V) to CLI agents.
/// The CLI agent needs time to read from the system clipboard before
/// we overwrite it with the next image.
const CLI_AGENT_IMAGE_PASTE_DELAY: Duration = Duration::from_millis(300);

/// ASCII prefixes that CLI agents use to switch input modes (e.g. `!` for bash
/// mode in Claude Code). When the rich input starts with one of these, the
/// prefix byte is written to the PTY separately so the agent can process it
/// before the rest of the command arrives.
#[allow(clippy::byte_char_slices)]
const CLI_AGENT_MODE_SWITCH_PREFIXES: &[u8] = &[b'!', b'&'];

/// Bytes that simulate a "paste image from clipboard" keystroke for the
/// foreground CLI agent. `0x16` is `Ctrl+V` (SYN); on Windows Claude Code
/// listens for `Alt+V` (`ESC` + `'v'`) instead. Mirrored from the equivalent
/// branch in `TerminalView::paste`.
fn cli_agent_paste_keystroke_bytes() -> Vec<u8> {
    if cfg!(windows) {
        vec![0x1b, b'v']
    } else {
        vec![0x16]
    }
}

/// How rich input delivers text + Enter to the CLI agent's PTY.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RichInputSubmitStrategy {
    /// Send text bytes followed by `\r` in a single write.
    /// Works for agents whose input layer processes the carriage return as
    /// a submit even when it arrives in the same buffer as the preceding text.
    Inline,
    /// Wrap text in bracketed paste escape sequences, then send `\r` separately.
    /// Required for agents like Codex whose paste-burst heuristics would
    /// otherwise suppress a rapid Enter after a character stream.
    BracketedPaste,
    /// Send text first, then `\r` after a short delay.
    /// For agents that don't respond to `\r` when it arrives in the same
    /// buffer as the text and don't support bracketed paste reliably.
    DelayedEnter,
    /// Wrap text in bracketed paste (reliable buffer insertion), then send
    /// `\r` after a delay. For agents like Copilot that need bracketed paste
    /// for reliable text delivery but also need a separate delayed Enter.
    BracketedPasteDelayedEnter,
}

/// Returns the strategy for submitting rich input text to a CLI agent's PTY.
fn rich_input_submit_strategy(agent: CLIAgent) -> RichInputSubmitStrategy {
    match agent {
        CLIAgent::Codex => RichInputSubmitStrategy::BracketedPaste,
        CLIAgent::Copilot => RichInputSubmitStrategy::BracketedPasteDelayedEnter,
        CLIAgent::Claude
        | CLIAgent::OpenCode
        | CLIAgent::Gemini
        | CLIAgent::Auggie
        | CLIAgent::CursorCli => RichInputSubmitStrategy::DelayedEnter,
        CLIAgent::Amp
        | CLIAgent::Droid
        | CLIAgent::Pi
        | CLIAgent::Goose
        | CLIAgent::Hermes
        | CLIAgent::Vibe
        | CLIAgent::Unknown => RichInputSubmitStrategy::Inline,
    }
}

static USE_AGENT_KEYSTROKE: LazyLock<Keystroke> =
    LazyLock::new(|| Keystroke::parse(cmd_or_ctrl_shift("enter")).expect("valid keystroke"));

impl TerminalView {
    pub(super) fn register_subscriptions_for_use_agent_footer(
        &mut self,
        ctx: &mut ViewContext<Self>,
    ) {
        let ai_settings = AISettings::handle(ctx);
        ctx.subscribe_to_model(&ai_settings, |me, _, event, ctx| match event {
            AISettingsChangedEvent::IsAnyAIEnabled { .. }
            | AISettingsChangedEvent::ShouldRenderCLIAgentToolbar { .. } => {
                me.maybe_show_use_agent_footer_in_blocklist(ctx);
            }
            AISettingsChangedEvent::ShouldRenderUseAgentToolbarForUserCommands { .. } => {
                // When the setting is re-enabled (e.g. from the AI settings page),
                // reset the pane-scoped dismissal so the footer can reappear.
                if *AISettings::as_ref(ctx)
                    .should_render_use_agent_footer_for_user_commands
                    .value()
                {
                    me.use_agent_footer.update(ctx, |footer, _| {
                        footer.did_user_dismiss = false;
                    });
                }
                me.maybe_show_use_agent_footer_in_blocklist(ctx);
            }
            AISettingsChangedEvent::CLIAgentToolbarEnabledCommands { .. } => {
                me.maybe_show_use_agent_footer_in_blocklist(ctx);
            }
            _ => (),
        });

        ctx.subscribe_to_view(&self.use_agent_footer, |me, _, event, ctx| {
            me.handle_use_agent_footer_event(event, ctx);
        });

        let input_mode_settings = InputModeSettings::handle(ctx);
        let mut was_pinned_to_top = input_mode_settings
            .as_ref(ctx)
            .input_mode
            .is_pinned_to_top();
        ctx.subscribe_to_model(&input_mode_settings, move |me, settings_handle, _, ctx| {
            let is_pinned_to_top = settings_handle.as_ref(ctx).is_pinned_to_top();
            if was_pinned_to_top != is_pinned_to_top {
                was_pinned_to_top = is_pinned_to_top;
                me.maybe_show_use_agent_footer_in_blocklist(ctx);
            }
        });

        ctx.subscribe_to_model(
            &self.cli_subagent_controller,
            |me, _, event, ctx| match event {
                CLISubagentEvent::SpawnedSubagent { .. } => {
                    me.hide_use_agent_footer_in_blocklist(ctx);
                }
                CLISubagentEvent::UpdatedControl { .. } => {
                    me.maybe_show_use_agent_footer_in_blocklist(ctx);
                }
                _ => (),
            },
        );
    }

    fn handle_use_agent_footer_event(
        &mut self,
        event: &UseAgentToolbarEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            UseAgentToolbarEvent::Dismiss => {
                self.hide_use_agent_footer_in_blocklist(ctx);
                send_telemetry_from_ctx!(TelemetryEvent::AgentToolbarDismissed, ctx);
                ctx.notify();
            }
            UseAgentToolbarEvent::WriteToPty(text) => {
                // Route like user-typed terminal input so shared-session viewers
                // forward the write request to the sharer instead of only
                // emitting a local PTY write event.
                self.write_user_bytes_to_pty(text.as_bytes().to_vec(), ctx);
            }
            UseAgentToolbarEvent::InsertIntoRichInput(text) => {
                self.input.update(ctx, |input, ctx| {
                    input.insert_into_cli_agent_rich_input(text, ctx);
                });
            }
            UseAgentToolbarEvent::ToggleCodeReviewPane(cli_agent) => {
                self.toggle_code_review_pane(
                    GitDeltaPreference::Always,
                    CodeReviewPaneEntrypoint::CLIAgentView,
                    Some(*cli_agent),
                    true, // focus_new_pane
                    ctx,
                );
            }
            UseAgentToolbarEvent::ToggleFileExplorer(cli_agent) => {
                self.toggle_file_tree(Some((*cli_agent).into()), ctx);
            }
            UseAgentToolbarEvent::StartRemoteControl { scrollback_type } => {
                self.auto_stop_sharing_on_cli_end =
                    *scrollback_type == SharedSessionScrollbackType::None;
                self.attempt_to_share_session(
                    *scrollback_type,
                    Some(SharedSessionActionSource::FooterChip),
                    SessionSourceType::default(),
                    true,
                    ctx,
                );
            }
            UseAgentToolbarEvent::StopRemoteControl => {
                self.auto_stop_sharing_on_cli_end = false;
                self.stop_sharing_session(SharedSessionActionSource::FooterChip, ctx);
            }
            UseAgentToolbarEvent::OpenRichInput => {
                if self.has_active_cli_agent_input_session(ctx) {
                    self.close_cli_agent_rich_input_and_disable_auto_toggle(ctx);
                } else {
                    self.open_cli_agent_rich_input(CLIAgentInputEntrypoint::FooterButton, ctx);
                }
            }
            UseAgentToolbarEvent::HideRichInput => {
                self.close_cli_agent_rich_input_and_disable_auto_toggle(ctx);
            }
            UseAgentToolbarEvent::Warpify { mode } => {
                self.hide_use_agent_footer_in_blocklist(ctx);
                match mode {
                    WarpificationMode::Ssh { .. } => {
                        self.handle_action(&TerminalAction::WarpifySSHSession, ctx);
                    }
                    WarpificationMode::Subshell { .. } => {
                        self.handle_action(&TerminalAction::TriggerSubshellBootstrap, ctx);
                    }
                }
                send_telemetry_from_ctx!(
                    TelemetryEvent::WarpifyFooterAcceptedWarpify {
                        is_ssh: mode.is_ssh()
                    },
                    ctx
                );
            }
            UseAgentToolbarEvent::UseAgent => {
                self.hide_use_agent_footer_in_blocklist(ctx);
                self.handle_action(&TerminalAction::SetInputModeAgent, ctx);
            }
        }
    }

    pub(super) fn has_active_cli_agent_input_session(&self, app: &AppContext) -> bool {
        CLIAgentSessionsModel::as_ref(app).is_input_open(self.view_id)
    }

    /// Checks if the footer should be rendered.
    /// Reads the CLI agent from the sessions model (single source of truth).
    pub(super) fn should_render_use_agent_footer(
        &self,
        model: &TerminalModel,
        app: &AppContext,
    ) -> bool {
        let ai_settings = AISettings::as_ref(app);

        // If a warpify mode is set, that means ssh or subshell is detected and we should show the footer.
        if self
            .use_agent_footer
            .as_ref(app)
            .warpify_mode(app)
            .is_some()
        {
            return true;
        }

        let active_block = model.block_list().active_block();
        let cli_agent = CLIAgentSessionsModel::as_ref(app)
            .session(self.view_id)
            .map(|s| s.agent);

        // Check the appropriate setting based on whether this is a CLI agent command
        if cli_agent.is_some() {
            // For CLI agent commands, only check the CLI agent footer setting.
            // This is independent of the global AI toggle so that users who
            // disable Warp AI still get the footer for third-party coding agents.
            if !*ai_settings.should_render_cli_agent_footer {
                return false;
            }

            // If a CLIAgent is active, we always want to show the agent footer.
            return true;
        }

        // All other footer variants require the global AI setting to be on.
        if !ai_settings.is_any_ai_enabled(app) {
            return false;
        }

        if !active_block.is_eligible_for_agent_handoff() {
            // For regular commands (not agent handoff), check the "Use Agent" footer setting.
            // Agent handoff blocks always show the footer regardless of this setting.
            let is_user_command = active_block.requested_command_action_id().is_none();
            if is_user_command
                && (self.use_agent_footer.as_ref(app).did_user_dismiss()
                    || !*ai_settings.should_render_use_agent_footer_for_user_commands)
            {
                return false;
            }
        }

        // Don't show the use agent footer during LRCs in setup phase of ambient agent sessions.
        let is_shared_ambient_session = model.is_shared_ambient_agent_session();

        !self.is_input_box_visible(model, app)
            && ((active_block.is_eligible_to_tag_in_agent() && !is_shared_ambient_session)
                || active_block.is_eligible_for_agent_handoff())
    }

    /// Returns the detected CLI agent for the active block's command, if any.
    ///
    /// This method resolves aliases before detecting the CLI agent. For example,
    /// if a user has aliased `foo` to `claude`, running `foo` will detect Claude.
    /// Falls back to user-configured toolbar command patterns, returning the
    /// assigned agent (or `CLIAgent::Unknown` for unassigned patterns).
    ///
    /// The second tuple element is the custom command prefix (the first word of
    /// the command), present only when the agent was resolved via a custom
    /// toolbar command pattern rather than native detection.
    pub(super) fn detect_cli_agent_from_model(
        &self,
        model: &TerminalModel,
        ctx: &AppContext,
    ) -> Option<(CLIAgent, Option<String>)> {
        let active_block = model.block_list().active_block();

        if !active_block.is_active_and_long_running() {
            return None;
        }

        let command = active_block.command_with_secrets_obfuscated(false);

        let detected = self.active_block_session_id().and_then(|session_id| {
            self.sessions.read(ctx, |sessions, _| {
                let session = sessions.get(session_id)?;
                CLIAgent::detect(
                    &command,
                    Some(session.shell_family().escape_char()),
                    Some(session.aliases()),
                    ctx,
                )
            })
        });

        if let Some(agent) = detected {
            return Some((agent, None));
        }

        CompiledCommandsForCodingAgentToolbar::matched_agent(ctx, &command).map(|agent| {
            let prefix = command.split_whitespace().next().map(str::to_owned);
            (agent, prefix)
        })
    }

    /// Updates the UI during a long running command to agent "tagged-in state".
    ///
    /// An agent may be "tagged in" during a _user-executed_ long running command, where being
    /// 'tagged in' means the input is visible and locked in agent mode, presumably awaiting user
    /// submission of a prompt for the agent to interact with the command.
    pub(super) fn tag_in_agent_for_user_long_running_command(
        &mut self,
        ctx: &mut ViewContext<Self>,
    ) {
        if self
            .model
            .lock()
            .block_list()
            .active_block()
            .is_agent_tagged_in()
            || !self
                .model
                .lock()
                .block_list()
                .active_block()
                .is_eligible_to_tag_in_agent()
        {
            return;
        }

        self.model
            .lock()
            .block_list_mut()
            .active_block_mut()
            .set_is_agent_tagged_in(true);

        if !self.model.lock().is_alt_screen_active() {
            self.use_agent_footer.update(ctx, |footer, ctx| {
                footer.clear_warpify_mode(ctx);
            });
            self.hide_use_agent_footer_in_blocklist(ctx);
        }

        self.input.update(ctx, |input, ctx| {
            input.set_input_mode_agent(true, ctx);
            input.clear_buffer_and_reset_undo_stack(ctx);
        });
        ctx.notify();

        let model = self.model.lock();
        let active_block = model.block_list().active_block();
        let conversation_id = active_block.ai_conversation_id();
        let block_id = active_block.id().clone();
        send_telemetry_from_ctx!(
            TelemetryEvent::CLISubagentControlStateChanged {
                conversation_id,
                block_id,
                control_state: CLISubagentControlState::AgentTaggedIn,
            },
            ctx
        );
    }

    /// Tags the agent "out". See docs on `tag_in_agent_for_user_long_running_command` for
    /// 'tagged-in' semantics.
    ///
    /// Hides the agent input and re-shows the 'Use agent' footer at the bottom of the block.
    pub(super) fn tag_out_agent_for_user_long_running_command(
        &mut self,
        ctx: &mut ViewContext<Self>,
    ) {
        if !self
            .model
            .lock()
            .block_list()
            .active_block()
            .is_agent_tagged_in()
        {
            return;
        }

        self.model
            .lock()
            .block_list_mut()
            .active_block_mut()
            .set_is_agent_tagged_in(false);

        if !self.model.lock().is_alt_screen_active() {
            self.maybe_show_use_agent_footer_in_blocklist(ctx);
        }

        self.input.update(ctx, |input, ctx| {
            input.set_input_mode_terminal(false, ctx);
        });
        self.redetermine_terminal_focus(ctx);

        ctx.notify();

        let model = self.model.lock();
        let active_block = model.block_list().active_block();
        let conversation_id = active_block.ai_conversation_id();
        let block_id = active_block.id().clone();
        send_telemetry_from_ctx!(
            TelemetryEvent::CLISubagentControlStateChanged {
                conversation_id,
                block_id,
                control_state: CLISubagentControlState::AgentTaggedOut,
            },
            ctx
        );
    }

    pub(super) fn maybe_show_use_agent_footer_in_blocklist(&mut self, ctx: &mut ViewContext<Self>) {
        // This is a bit of a hack- but it ensures we never show more than one footer in the
        // blocklist.
        self.hide_use_agent_footer_in_blocklist(ctx);
        let (should_render_footer, is_alt_screen_active) = {
            let model = self.model.lock();
            (
                self.should_render_use_agent_footer(&model, ctx),
                model.is_alt_screen_active(),
            )
        };
        if is_alt_screen_active || !should_render_footer {
            return;
        }

        let should_insert_after_block = !InputModeSettings::as_ref(ctx).is_pinned_to_top();

        // Send telemetry when showing CLI agent footer
        if let Some(session) = CLIAgentSessionsModel::as_ref(ctx).session(self.view_id) {
            let cli_agent_type: CLIAgentType = session.agent.into();
            send_telemetry_from_ctx!(
                TelemetryEvent::CLIAgentToolbarShown {
                    cli_agent: cli_agent_type,
                },
                ctx
            );
        }

        self.insert_rich_content(
            None,
            self.use_agent_footer.clone(),
            None,
            RichContentInsertionPosition::Append {
                insert_below_long_running_block: should_insert_after_block,
            },
            ctx,
        );
    }

    pub(super) fn hide_use_agent_footer_in_blocklist(&mut self, ctx: &mut ViewContext<Self>) {
        let mut model = self.model.lock();
        let block_list = model.block_list_mut();
        block_list.remove_rich_content(self.use_agent_footer.id());
        ctx.notify();
    }

    /// Closes the CLI agent rich input session. Side effects (input config restore,
    /// buffer clear, hint text) are handled reactively by subscribers to
    /// `CLIAgentSessionsModelEvent::InputSessionChanged`.
    pub(in crate::terminal) fn close_cli_agent_rich_input(
        &mut self,
        reason: CLIAgentRichInputCloseReason,
        ctx: &mut ViewContext<Self>,
    ) {
        self.close_cli_agent_rich_input_impl(true, reason, ctx);
    }

    pub(in crate::terminal) fn close_cli_agent_rich_input_and_disable_auto_toggle(
        &mut self,
        ctx: &mut ViewContext<Self>,
    ) {
        self.close_cli_agent_rich_input_impl(false, CLIAgentRichInputCloseReason::Manual, ctx);
    }

    fn close_cli_agent_rich_input_impl(
        &mut self,
        should_auto_toggle_input: bool,
        reason: CLIAgentRichInputCloseReason,
        ctx: &mut ViewContext<Self>,
    ) {
        if !self.has_active_cli_agent_input_session(ctx) {
            return;
        }

        // Save the current buffer text as a draft before closing, so it can
        // be restored if the user reopens the composer.
        let draft = self.input.as_ref(ctx).buffer_text(ctx);
        let view_id = self.view_id;
        CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions_model, ctx| {
            sessions_model.set_draft(view_id, draft);
            sessions_model.close_input(view_id, should_auto_toggle_input, ctx);
        });

        let cli_agent_type: Option<CLIAgentType> = CLIAgentSessionsModel::as_ref(ctx)
            .session(self.view_id)
            .map(|s| s.agent.into());
        if let Some(cli_agent) = cli_agent_type {
            send_telemetry_from_ctx!(
                TelemetryEvent::CLIAgentRichInputClosed { cli_agent, reason },
                ctx
            );
        }

        self.redetermine_terminal_focus(ctx);
        ctx.notify();
    }

    /// Conditionally closes CLI agent rich input after a prompt submission.
    /// When auto-toggle is active with a plugin listener that emits rich
    /// status, rich input stays open (status-change events manage visibility
    /// instead). Otherwise, respects the auto-dismiss-after-submit setting.
    fn maybe_close_rich_input_after_submit(&mut self, ctx: &mut ViewContext<Self>) {
        let session = CLIAgentSessionsModel::as_ref(ctx).session(self.view_id);
        let has_plugin = session.as_ref().is_some_and(|s| {
            s.listener.is_some()
                && s.should_auto_toggle_input
                && agent_supports_rich_status(&s.agent)
        });
        let ai_settings = AISettings::as_ref(ctx);

        let should_close = if has_plugin && *ai_settings.auto_toggle_rich_input {
            false
        } else {
            *ai_settings.auto_dismiss_rich_input_after_submit
        };

        if should_close {
            self.close_cli_agent_rich_input(CLIAgentRichInputCloseReason::Submit, ctx);
        } else {
            self.input.update(ctx, |input, ctx| {
                input.clear_buffer_and_reset_undo_stack(ctx);
            });
        }
    }

    pub(super) fn submit_cli_agent_rich_input(
        &mut self,
        text: String,
        ctx: &mut ViewContext<Self>,
    ) {
        if !self.has_active_cli_agent_input_session(ctx) {
            return;
        }
        if text.trim().is_empty() {
            return;
        }

        let prompt_length = text.chars().count();
        let cli_agent: Option<CLIAgentType> = CLIAgentSessionsModel::as_ref(ctx)
            .session(self.view_id)
            .map(|s| s.agent.into());
        if let Some(cli_agent) = cli_agent {
            send_telemetry_from_ctx!(
                TelemetryEvent::CLIAgentRichInputSubmitted {
                    cli_agent,
                    prompt_length,
                },
                ctx
            );
        }

        // Clear any saved draft so submitted text isn't restored on the next open.
        let view_id = self.view_id;
        CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions_model, _| {
            sessions_model.clear_draft(view_id);
        });

        let strategy = CLIAgentSessionsModel::as_ref(ctx)
            .session(self.view_id)
            .map(|s| rich_input_submit_strategy(s.agent))
            .unwrap_or(RichInputSubmitStrategy::Inline);

        let text_bytes = text.into_bytes();

        // Clear the buffer eagerly so that any close path (auto-dismiss,
        // auto-toggle, or a deferred timer) sees an empty buffer and doesn't
        // re-save the submitted text as a draft.
        self.input.update(ctx, |input, ctx| {
            input.clear_buffer_and_reset_undo_stack(ctx);
        });

        // Extract pending image attachments and clear them from the context
        // model before submission.
        let images: Vec<_> = self
            .ai_context_model
            .as_ref(ctx)
            .pending_images()
            .into_iter()
            .cloned()
            .collect();
        if !images.is_empty() {
            self.ai_context_model.update(ctx, |model, ctx| {
                model.clear_pending_images(ctx);
            });
        }

        // When the input starts with a known mode-switch prefix (e.g. `!` for
        // bash mode, `&` for background mode), write the prefix byte separately
        // with a small delay before the rest of the command. This gives CLI
        // agents like Claude Code time to recognise the prefix and switch modes
        // before the command text arrives.
        //
        // Only applied to known ASCII prefixes to avoid splitting multi-byte
        // UTF-8 characters.
        if text_bytes.len() > 1 && CLI_AGENT_MODE_SWITCH_PREFIXES.contains(&text_bytes[0]) {
            self.write_user_bytes_to_pty(vec![text_bytes[0]], ctx);
            let rest = text_bytes[1..].to_vec();
            ctx.spawn(
                Timer::after(CLI_AGENT_PTY_WRITE_DELAY),
                move |me, _, ctx| {
                    me.paste_images_then_submit_text(images, rest, strategy, ctx);
                },
            );
        } else {
            self.paste_images_then_submit_text(images, text_bytes, strategy, ctx);
        }
    }

    /// Submits `text` as a prompt to the active CLI agent on this terminal by
    /// writing it to the PTY using the agent-specific submission strategy
    /// (the same pipeline as the CLI agent rich input composer).
    ///
    /// Intended for callers that produce prompts outside the rich input
    /// editor (e.g. shared-session viewer follow-up prompts). Returns
    /// without writing if there is no active CLI agent session or the text
    /// is empty.
    #[cfg(feature = "local_tty")]
    pub(crate) fn submit_text_to_cli_agent_pty(
        &mut self,
        text: String,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(agent) = CLIAgentSessionsModel::as_ref(ctx)
            .session(self.view_id)
            .map(|s| s.agent)
        else {
            return;
        };

        let text_bytes = text.into_bytes();
        if text_bytes.is_empty() {
            return;
        }

        let strategy = rich_input_submit_strategy(agent);
        self.write_cli_agent_text_then_submit(text_bytes, strategy, ctx);
    }

    /// Simulates clipboard image paste for each pending image attachment by
    /// writing the image to the system clipboard and sending Ctrl+V to the PTY.
    /// After all images are pasted, the text prompt is sent via the normal
    /// submission strategy.
    ///
    /// Uses a single async task that hops back to the view context via
    /// [`ViewSpawner`] for each image, rather than chaining per-image timers.
    /// If the rich input session closes mid-paste, the loop exits early so we
    /// don't leak Ctrl+V bytes into an unrelated PTY context.
    fn paste_images_then_submit_text(
        &mut self,
        images: Vec<ImageContext>,
        text_bytes: Vec<u8>,
        strategy: RichInputSubmitStrategy,
        ctx: &mut ViewContext<Self>,
    ) {
        // Bail if the rich input session was closed before we got here.
        if !self.has_active_cli_agent_input_session(ctx) {
            return;
        }

        if images.is_empty() {
            self.write_cli_agent_text_then_submit(text_bytes, strategy, ctx);
            return;
        }

        let spawner = ctx.spawner();
        ctx.spawn(
            async move {
                for image in images {
                    // Decode off the main thread; log and skip on failure.
                    let raw_bytes =
                        match base64::engine::general_purpose::STANDARD.decode(&image.data) {
                            Ok(bytes) => bytes,
                            Err(_) => {
                                log::error!(
                                    "Failed to decode base64 image data for {}",
                                    image.file_name
                                );
                                continue;
                            }
                        };

                    // Hop back to the view to write the clipboard + Ctrl+V.
                    // Returns false if the input session has closed, in which
                    // case we stop pasting and skip the final text submit.
                    let should_continue = spawner
                        .spawn(move |me, ctx| {
                            if !me.has_active_cli_agent_input_session(ctx) {
                                return false;
                            }
                            ctx.clipboard().write(ClipboardContent {
                                images: Some(vec![ImageData {
                                    data: raw_bytes,
                                    mime_type: image.mime_type,
                                    filename: Some(image.file_name),
                                }]),
                                ..Default::default()
                            });
                            me.write_user_bytes_to_pty(cli_agent_paste_keystroke_bytes(), ctx);
                            true
                        })
                        .await;

                    if !matches!(should_continue, Ok(true)) {
                        return false;
                    }

                    // Give the CLI agent time to read from the clipboard before
                    // we overwrite it with the next image (or send the text).
                    Timer::after(CLI_AGENT_IMAGE_PASTE_DELAY).await;
                }
                true
            },
            move |me, ok, ctx| {
                if !ok || !me.has_active_cli_agent_input_session(ctx) {
                    return;
                }
                me.write_cli_agent_text_then_submit(text_bytes, strategy, ctx);
            },
        );
    }

    /// Mirrors the CLI-agent Cmd+V image-paste path in `TerminalView::paste`
    /// for dropped image files: reads each file, writes its bytes to the
    /// system clipboard as image data, and sends the agent's paste keystroke
    /// to the PTY so the agent reads the image directly. This produces the
    /// same outcome as if the user had copied the image to their clipboard
    /// and pressed Cmd+V over the agent's TUI.
    pub(super) fn paste_dropped_images_to_cli_agent(
        &mut self,
        image_filepaths: Vec<String>,
        ctx: &mut ViewContext<Self>,
    ) {
        if image_filepaths.is_empty() {
            return;
        }
        let spawner = ctx.spawner();
        ctx.spawn(
            async move {
                for path_str in image_filepaths {
                    // Stat first so a multi-GB drop doesn't load into memory
                    // before we reject it. CLI agents handle their own
                    // compression, so the cap only exists to bound memory use.
                    match async_fs::metadata(&path_str).await {
                        Ok(meta) if (meta.len() as usize) > MAX_IMAGE_SIZE_BYTES_FOR_CLI_AGENT => {
                            let filename = Path::new(&path_str)
                                .file_name()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_else(|| path_str.clone());
                            let limit_mb = MAX_IMAGE_SIZE_BYTES_FOR_CLI_AGENT / 1_000_000;
                            let msg = format!(
                                "{filename} is too large to send to the agent (limit {limit_mb}MB)."
                            );
                            let _ = spawner
                                .spawn(move |me, ctx| {
                                    me.show_error_toast(msg, ctx);
                                })
                                .await;
                            continue;
                        }
                        Ok(_) => {}
                        Err(e) => {
                            log::error!("Failed to stat dropped image {path_str}: {e}");
                            continue;
                        }
                    }

                    let bytes = match async_fs::read(&path_str).await {
                        Ok(b) => b,
                        Err(e) => {
                            log::error!("Failed to read dropped image {path_str}: {e}");
                            continue;
                        }
                    };
                    let path = Path::new(&path_str);
                    let filename = path.file_name().map(|n| n.to_string_lossy().into_owned());
                    let sniff_len = bytes.len().min(MIME_SNIFF_BYTES);
                    let mime_type = infer_mime_type(path, &bytes[..sniff_len]);

                    // Hop back to the view to write the clipboard + paste
                    // keystroke. Bail if the CLI agent session disappeared,
                    // OR if the agent's long-running block exited while we
                    // were reading off-thread — without that second check
                    // the paste byte would leak into the shell after the
                    // agent quit, since the session entry can outlive its
                    // foreground block.
                    let should_continue = spawner
                        .spawn(move |me, ctx| {
                            if !me.has_active_cli_agent_session(ctx) {
                                return false;
                            }
                            let still_long_running = me
                                .model
                                .lock()
                                .block_list()
                                .active_block()
                                .is_active_and_long_running();
                            if !still_long_running {
                                return false;
                            }
                            ctx.clipboard().write(ClipboardContent {
                                images: Some(vec![ImageData {
                                    data: bytes,
                                    mime_type,
                                    filename,
                                }]),
                                ..Default::default()
                            });
                            me.write_user_bytes_to_pty(cli_agent_paste_keystroke_bytes(), ctx);
                            true
                        })
                        .await;

                    if !matches!(should_continue, Ok(true)) {
                        return;
                    }

                    // Give the CLI agent time to read from the clipboard
                    // before we overwrite it with the next image.
                    Timer::after(CLI_AGENT_IMAGE_PASTE_DELAY).await;
                }
            },
            |_, _, _| {},
        );
    }

    /// Writes the input text to the PTY and then sends a carriage return to
    /// submit it, using the agent-specific strategy. After the submission is
    /// complete (synchronously for the inline strategies, after a timer for
    /// the delayed strategies), closes the rich input if the user's settings
    /// request auto-dismissal.
    fn write_cli_agent_text_then_submit(
        &mut self,
        text_bytes: Vec<u8>,
        strategy: RichInputSubmitStrategy,
        ctx: &mut ViewContext<Self>,
    ) {
        match strategy {
            RichInputSubmitStrategy::Inline => {
                let mut bytes = text_bytes;
                bytes.extend_from_slice(b"\r");
                self.write_user_bytes_to_pty(bytes, ctx);
                self.maybe_close_rich_input_after_submit(ctx);
            }
            RichInputSubmitStrategy::BracketedPaste => {
                let mut bytes = Vec::with_capacity(
                    BRACKETED_PASTE_START.len() + text_bytes.len() + BRACKETED_PASTE_END.len(),
                );
                bytes.extend_from_slice(BRACKETED_PASTE_START);
                bytes.extend_from_slice(&text_bytes);
                bytes.extend_from_slice(BRACKETED_PASTE_END);
                self.write_user_bytes_to_pty(bytes, ctx);
                self.write_user_bytes_to_pty(b"\r".to_vec(), ctx);
                self.maybe_close_rich_input_after_submit(ctx);
            }
            RichInputSubmitStrategy::DelayedEnter => {
                self.write_user_bytes_to_pty(text_bytes, ctx);
                ctx.spawn(
                    Timer::after(CLI_AGENT_PTY_WRITE_DELAY),
                    move |me, _, ctx| {
                        me.write_user_bytes_to_pty(b"\r".to_vec(), ctx);
                        me.maybe_close_rich_input_after_submit(ctx);
                    },
                );
            }
            RichInputSubmitStrategy::BracketedPasteDelayedEnter => {
                let mut bytes = Vec::with_capacity(
                    BRACKETED_PASTE_START.len() + text_bytes.len() + BRACKETED_PASTE_END.len(),
                );
                bytes.extend_from_slice(BRACKETED_PASTE_START);
                bytes.extend_from_slice(&text_bytes);
                bytes.extend_from_slice(BRACKETED_PASTE_END);
                self.write_user_bytes_to_pty(bytes, ctx);
                ctx.spawn(
                    Timer::after(CLI_AGENT_BRACKETED_PASTE_ENTER_DELAY),
                    move |me, _, ctx| {
                        me.write_user_bytes_to_pty(b"\r".to_vec(), ctx);
                        me.maybe_close_rich_input_after_submit(ctx);
                    },
                );
            }
        }
    }

    pub(in crate::terminal) fn open_cli_agent_rich_input(
        &mut self,
        entrypoint: CLIAgentInputEntrypoint,
        ctx: &mut ViewContext<Self>,
    ) {
        if !FeatureFlag::CLIAgentRichInput.is_enabled()
            || self.has_active_cli_agent_input_session(ctx)
        {
            return;
        }

        // The Ctrl-G binding and footer button are both gated on an active CLI
        // agent session, so the session should always exist here.
        let Some(cli_agent) = CLIAgentSessionsModel::as_ref(ctx)
            .session(self.view_id)
            .map(|session| session.agent)
        else {
            return;
        };

        let ai_input_model = self.ai_input_model.as_ref(ctx);
        let previous_input_config = ai_input_model.input_config();
        let previous_was_lock_set_with_empty_buffer =
            ai_input_model.was_lock_set_with_empty_buffer();

        let view_id = self.view_id;
        CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions_model, ctx| {
            sessions_model.open_input(
                view_id,
                entrypoint,
                previous_input_config,
                previous_was_lock_set_with_empty_buffer,
                true,
                ctx,
            );
        });

        send_telemetry_from_ctx!(
            TelemetryEvent::CLIAgentRichInputOpened {
                cli_agent: cli_agent.into(),
                entrypoint,
            },
            ctx
        );

        // Input mode switch, buffer clear, draft restoration, and hint text
        // are handled reactively by Input's subscription to InputSessionChanged.
        self.redetermine_terminal_focus(ctx);
        ctx.notify();
    }
}

/// Footer rendered at the bottom of the active long running block or alt screen element.
///
/// For regular commands, displays a 'Use agent' keystroke button to enter agent mode.
/// For CLI agent commands (e.g., Claude Code, Gemini CLI, Codex), displays a specialized
/// footer with image attachment, voice input, file explorer, view changes, and share buttons.
pub struct UseAgentToolbar {
    terminal_view_id: EntityId,
    terminal_model: Arc<FairMutex<TerminalModel>>,

    // Standard "Use agent" UI
    button: ViewHandle<ActionButton>,
    give_control_back_button: ViewHandle<ActionButton>,
    dismiss_button: ViewHandle<ActionButton>,
    dont_show_again_button: ViewHandle<ActionButton>,

    // Shared agent input footer (renders CLI agent mode when a CLI session is active).
    agent_input_footer: ViewHandle<AgentInputFooter>,

    // Warpify footer UI (shown when a subshell/SSH command is detected).
    warpify_footer_view: ViewHandle<WarpifyFooterView>,

    // `true` if the user has dismissed the footer.
    //
    // Footer dismissal is terminal pane-scoped, e.g. dismissal hides the footer for this
    // specific terminal pane for the lifetime of the pane.
    did_user_dismiss: bool,
}

impl UseAgentToolbar {
    pub(crate) fn new(
        terminal_view_id: EntityId,
        terminal_model: Arc<FairMutex<TerminalModel>>,
        model_event_dispatcher: &ModelHandle<ModelEventDispatcher>,
        agent_input_footer: ViewHandle<AgentInputFooter>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let button_size = ButtonSize::XSmall;

        let button = ctx.add_typed_action_view(|ctx| {
            ActionButton::new(
                "Use agent",
                AgentFooterButtonTheme::new(Some(terminal_model.clone())),
            )
            .with_icon(Icon::Oz)
            .with_keybinding(KeystrokeSource::Fixed(USE_AGENT_KEYSTROKE.clone()), ctx)
            .with_size(button_size)
            .with_tooltip("Ask the Warp agent to assist")
            .with_tooltip_alignment(TooltipAlignment::Left)
            .on_click(|ctx| {
                ctx.dispatch_typed_action(TerminalAction::SetInputModeAgent);
            })
        });
        let give_control_back_button = ctx.add_typed_action_view(|ctx| {
            ActionButton::new(
                "Give control back to agent",
                AgentFooterButtonTheme::new(Some(terminal_model.clone())),
            )
            .with_icon(Icon::Oz)
            .with_keybinding(KeystrokeSource::Fixed(USE_AGENT_KEYSTROKE.clone()), ctx)
            .with_size(button_size)
            .with_tooltip("Ask the Warp agent to resume")
            .with_tooltip_alignment(TooltipAlignment::Left)
            .on_click(|ctx| {
                ctx.dispatch_typed_action(TerminalAction::SetInputModeAgent);
            })
        });
        let dismiss_button = ctx.add_typed_action_view(|_| {
            ActionButton::new(
                "Dismiss",
                AgentFooterButtonTheme::new(Some(terminal_model.clone())),
            )
            .on_click(|ctx| {
                ctx.dispatch_typed_action(UseAgentToolbarAction::Dismiss { permanently: false });
            })
            .with_size(button_size)
        });
        let dont_show_again_button = ctx.add_typed_action_view(|_| {
            ActionButton::new(
                "Don't show again",
                AgentFooterButtonTheme::new(Some(terminal_model.clone())),
            )
            .on_click(|ctx| {
                ctx.dispatch_typed_action(UseAgentToolbarAction::Dismiss { permanently: true });
            })
            .with_size(button_size)
        });

        // Subscribe to agent input footer events to forward CLI-relevant ones.
        ctx.subscribe_to_view(&agent_input_footer, |me, _, event, ctx| {
            me.handle_agent_input_footer_event(event, ctx);
        });

        let warpify_footer_view =
            ctx.add_typed_action_view(|ctx| WarpifyFooterView::new(terminal_model.clone(), ctx));

        ctx.subscribe_to_view(&warpify_footer_view, |me, _, event, ctx| {
            me.handle_warpify_footer_event(event, ctx);
        });

        ctx.subscribe_to_model(model_event_dispatcher, |me, _, event, ctx| {
            if let ModelEvent::TerminalModeSwapped(..) = event {
                me.notify_and_notify_children(ctx);
            }
        });

        // Re-render when the CLI agent session state changes (e.g. status updates
        // from the plugin, session started/ended).
        let cli_agent_sessions = CLIAgentSessionsModel::handle(ctx);
        ctx.subscribe_to_model(&cli_agent_sessions, move |me, _, event, ctx| {
            if event.terminal_view_id() != terminal_view_id {
                return;
            }
            me.notify_and_notify_children(ctx);
        });

        Self {
            terminal_view_id,
            button,
            give_control_back_button,
            dismiss_button,
            dont_show_again_button,
            agent_input_footer,
            warpify_footer_view,
            terminal_model,
            did_user_dismiss: false,
        }
    }

    fn handle_agent_input_footer_event(
        &mut self,
        event: &AgentInputFooterEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        // Forward CLI-relevant events from the shared agent input footer.
        match event {
            AgentInputFooterEvent::WriteToPty(text) => {
                ctx.emit(UseAgentToolbarEvent::WriteToPty(text.clone()));
            }
            AgentInputFooterEvent::InsertIntoCLIRichInput(text) => {
                ctx.emit(UseAgentToolbarEvent::InsertIntoRichInput(text.clone()));
            }
            AgentInputFooterEvent::ToggleCodeReviewPane(agent) => {
                ctx.emit(UseAgentToolbarEvent::ToggleCodeReviewPane(*agent));
            }
            AgentInputFooterEvent::ToggleFileExplorer(agent) => {
                ctx.emit(UseAgentToolbarEvent::ToggleFileExplorer(*agent));
            }
            AgentInputFooterEvent::StartRemoteControl => {
                let scrollback_type = if self.cli_agent(ctx).is_some() {
                    SharedSessionScrollbackType::None
                } else {
                    SharedSessionScrollbackType::All
                };
                ctx.emit(UseAgentToolbarEvent::StartRemoteControl { scrollback_type });
            }
            AgentInputFooterEvent::StopRemoteControl => {
                ctx.emit(UseAgentToolbarEvent::StopRemoteControl);
            }
            AgentInputFooterEvent::OpenRichInput => {
                ctx.emit(UseAgentToolbarEvent::OpenRichInput);
            }
            AgentInputFooterEvent::HideRichInput => {
                ctx.emit(UseAgentToolbarEvent::HideRichInput);
            }
            // Non-CLI events are handled by Input's subscription, not here.
            _ => {}
        }
    }

    fn handle_warpify_footer_event(
        &mut self,
        event: &WarpifyFooterViewEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            WarpifyFooterViewEvent::Warpify { mode } => {
                ctx.emit(UseAgentToolbarEvent::Warpify { mode: mode.clone() });
            }
            WarpifyFooterViewEvent::UseAgent => {
                ctx.emit(UseAgentToolbarEvent::UseAgent);
            }
            WarpifyFooterViewEvent::Dismiss => {
                ctx.emit(UseAgentToolbarEvent::Dismiss);
            }
        }
    }

    pub(in crate::terminal) fn notify_and_notify_children(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.notify();
        self.agent_input_footer.update(ctx, |_, ctx| ctx.notify());
        self.warpify_footer_view.update(ctx, |_, ctx| ctx.notify());
        self.button.update(ctx, |_, ctx| ctx.notify());
        self.give_control_back_button
            .update(ctx, |_, ctx| ctx.notify());
        self.dismiss_button.update(ctx, |_, ctx| ctx.notify());
        self.dont_show_again_button
            .update(ctx, |_, ctx| ctx.notify());
    }

    /// Returns whether the user has dismissed this footer.
    pub fn did_user_dismiss(&self) -> bool {
        self.did_user_dismiss
    }

    fn cli_agent(&self, app: &AppContext) -> Option<CLIAgent> {
        CLIAgentSessionsModel::as_ref(app)
            .session(self.terminal_view_id)
            .map(|session| session.agent)
    }

    /// Sets the current warpification mode. When set, the footer shows the
    /// warpify view instead of the CLI agent or regular "Use agent" views.
    pub(in crate::terminal) fn set_warpify_mode(
        &mut self,
        mode: WarpificationMode,
        ctx: &mut ViewContext<Self>,
    ) {
        self.warpify_footer_view.update(ctx, |view, ctx| {
            view.set_mode(mode, ctx);
        });
        ctx.notify();
    }

    /// Clears the warpification mode so the footer reverts to its default behavior.
    pub(in crate::terminal) fn clear_warpify_mode(&mut self, ctx: &mut ViewContext<Self>) {
        self.warpify_footer_view.update(ctx, |view, ctx| {
            view.clear_mode(ctx);
        });
        ctx.notify();
    }

    /// Returns the current warpification mode, if set.
    pub(in crate::terminal) fn warpify_mode(&self, app: &AppContext) -> Option<WarpificationMode> {
        self.warpify_footer_view.as_ref(app).mode().cloned()
    }

    /// Returns whether there's a current CLI agent (like Claude Code).
    #[cfg(feature = "voice_input")]
    pub fn has_cli_agent(&self, app: &AppContext) -> bool {
        self.cli_agent(app).is_some()
    }
}

/// Events emitted by UseAgentToolbar.
pub enum UseAgentToolbarEvent {
    /// The footer was dismissed.
    Dismiss,
    /// Write text to the PTY (from CLI agent view).
    WriteToPty(String),
    /// Insert text into CLI agent rich input.
    InsertIntoRichInput(String),
    /// Toggle the code review pane (from CLI agent view).
    ToggleCodeReviewPane(CLIAgent),
    /// Toggle the file explorer (from CLI agent view).
    ToggleFileExplorer(CLIAgent),
    /// Start remote control (one-click share without modal).
    StartRemoteControl {
        scrollback_type: SharedSessionScrollbackType,
    },
    /// Stop remote control (stop the active shared session).
    StopRemoteControl,
    /// Open the rich input editor for composing a prompt.
    OpenRichInput,
    /// Hide the rich input editor (same as Escape).
    HideRichInput,
    /// User chose to warpify the subshell/SSH session.
    Warpify { mode: WarpificationMode },
    /// User chose to use the agent.
    UseAgent,
}

impl Entity for UseAgentToolbar {
    type Event = UseAgentToolbarEvent;
}

impl View for UseAgentToolbar {
    fn ui_name() -> &'static str {
        "UseAgentToolbar"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        // If a warpify mode is set, delegate rendering to the warpify footer view.
        if self.warpify_footer_view.as_ref(app).mode().is_some() {
            return ChildView::new(&self.warpify_footer_view).finish();
        }

        // Hide the toolbar entirely when CLI rich input is open,
        // since the Input view renders its own footer in that state.
        if CLIAgentSessionsModel::as_ref(app).is_input_open(self.terminal_view_id) {
            return Empty::new().finish();
        }

        // If a CLI agent is detected, delegate rendering to the CLI agent footer view.
        // Wrap with horizontal padding matching the terminal view padding so the footer
        // aligns consistently with the input context (which inherits terminal padding).
        if self.cli_agent(app).is_some() {
            let mut container = Container::new(ChildView::new(&self.agent_input_footer).finish())
                .with_horizontal_padding(*super::PADDING_LEFT);

            // Apply the alt screen background on this outer container so it covers
            // the horizontal padding area as well, preventing a visible color mismatch
            // between the padding and the footer content.
            let terminal_model = self.terminal_model.lock();
            if terminal_model.is_alt_screen_active() {
                if let Some(bg_color) = terminal_model.alt_screen().inferred_bg_color() {
                    container = container.with_background(bg_color);
                }
            }

            return container.finish();
        }

        let terminal_model = self.terminal_model.lock();

        let active_block = terminal_model.block_list().active_block();
        let show_give_control_back_button = active_block.is_eligible_for_agent_handoff();
        let show_dismiss_actions = active_block.requested_command_action_id().is_none();

        let mut button_row = Flex::row()
            .with_spacing(4.)
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(
                ChildView::new(if show_give_control_back_button {
                    &self.give_control_back_button
                } else {
                    &self.button
                })
                .finish(),
            );

        if show_dismiss_actions {
            button_row = button_row
                .with_child(Expanded::new(1., Empty::new().finish()).finish())
                .with_child(ChildView::new(&self.dismiss_button).finish());

            if !show_give_control_back_button {
                button_row =
                    button_row.with_child(ChildView::new(&self.dont_show_again_button).finish());
            }
        }

        let mut container = Container::new(button_row.finish())
            .with_horizontal_padding(*super::PADDING_LEFT)
            .with_vertical_padding(4.);

        if terminal_model.is_alt_screen_active() {
            if let Some(bg_color) = terminal_model.alt_screen().inferred_bg_color() {
                container = container.with_background(bg_color);
            }
        } else if terminal_model.block_list().agent_view_state().is_inline() {
            container = container.with_background(agent_view_bg_fill(app));
        }

        container.finish()
    }
}

#[derive(Debug, Clone)]
pub enum UseAgentToolbarAction {
    Dismiss { permanently: bool },
}

impl TypedActionView for UseAgentToolbar {
    type Action = UseAgentToolbarAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        let UseAgentToolbarAction::Dismiss { permanently } = action;
        self.did_user_dismiss = true;
        ctx.emit(UseAgentToolbarEvent::Dismiss);

        if *permanently {
            AISettings::handle(ctx).update(ctx, |settings, ctx| {
                if let Err(e) = settings
                    .should_render_use_agent_footer_for_user_commands
                    .set_value(false, ctx)
                {
                    report_error!(anyhow!("{e:?}")
                        .context("Failed to set `ShouldRenderUseAgentToolbarForUserCommands`"));
                }
            });
        }

        ctx.notify();
    }
}

#[derive(Clone)]
pub(super) struct AgentFooterButtonTheme {
    /// When set, enables alt-screen contrast adjustment for text and border.
    terminal_model: Option<Arc<FairMutex<TerminalModel>>>,
}

impl AgentFooterButtonTheme {
    pub fn new(terminal_model: Option<Arc<FairMutex<TerminalModel>>>) -> Self {
        Self { terminal_model }
    }

    /// Returns the inferred background colour of the alt screen, if active.
    fn inferred_alt_screen_bg(&self) -> Option<ColorU> {
        let terminal_model = self.terminal_model.as_ref()?;
        let terminal_model = terminal_model.lock();
        terminal_model
            .is_alt_screen_active()
            .then(|| terminal_model.alt_screen().inferred_bg_color())
            .flatten()
    }

    /// Picks a colour that contrasts well against `bg`, choosing between two
    /// neutral candidates.
    fn contrast_adjusted_color(
        bg: ColorU,
        default: ColorU,
        contrast: MinimumAllowedContrast,
        appearance: &Appearance,
    ) -> ColorU {
        if high_enough_contrast(default, bg, contrast) {
            default
        } else {
            pick_best_foreground_color(
                bg,
                blended_colors::neutral_2(appearance.theme()),
                blended_colors::neutral_6(appearance.theme()),
                contrast,
            )
        }
    }
}

impl ActionButtonTheme for AgentFooterButtonTheme {
    fn background(&self, hovered: bool, appearance: &Appearance) -> Option<ThemeFill> {
        if hovered {
            Some(internal_colors::fg_overlay_2(appearance.theme()))
        } else {
            None
        }
    }

    fn border(&self, appearance: &Appearance) -> Option<ColorU> {
        let color = appearance.theme().outline().into_solid();
        if let Some(bg) = self.inferred_alt_screen_bg() {
            return Some(Self::contrast_adjusted_color(
                bg,
                color,
                MinimumAllowedContrast::NonText,
                appearance,
            ));
        }
        Some(color)
    }

    fn text_color(
        &self,
        _hovered: bool,
        _background: Option<ThemeFill>,
        appearance: &Appearance,
    ) -> ColorU {
        let color = appearance
            .theme()
            .sub_text_color(appearance.theme().surface_1())
            .into_solid();

        // If rendered in the alt screen, the footer is rendered with the inferred background color
        // of the alt screen output grid (if there is one). In such cases, we have to ensure that
        // the text within the footer is high-contrast enough to be legible, since the background
        // color can essentially be anything.
        if let Some(bg) = self.inferred_alt_screen_bg() {
            return Self::contrast_adjusted_color(
                bg,
                color,
                MinimumAllowedContrast::Text,
                appearance,
            );
        }
        color
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
