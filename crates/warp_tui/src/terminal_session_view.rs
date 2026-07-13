//! Authenticated terminal-session TUI surface.
use std::borrow::Cow;
use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use ai::project_context::model::{ProjectContextModel, ProjectContextModelEvent};
use instant::Instant;
use parking_lot::FairMutex;
use warp::editor::{CodeEditorModel, CodeEditorModelEvent};
use warp::settings::{AISettings, AISettingsChangedEvent};
use warp::tui_export::{
    build_slash_command_mixer, detect_possible_git_repo, record_saved_prompt_accepted,
    record_static_slash_command_accepted, saved_prompt_text_for_id,
    slash_command_is_submitted_as_prompt, slash_command_selection_behavior, slash_commands,
    throttle, AIAgentActionId, AIAgentPtyWriteMode, AcceptSlashCommandOrSavedPrompt, ActiveSession,
    ActiveSessionEvent, AgentInteractionMetadata, AgentViewEntryOrigin, BlocklistAIActionModel,
    BlocklistAIContextModel, BlocklistAIController, BlocklistAIHistoryEvent,
    BlocklistAIHistoryModel, BlocklistAIInputModel, CLISubagentController, CLISubagentEvent,
    CancellationReason, ChangelogModel, ChangelogModelEvent, ChangelogRequestType,
    CommandExecutionSource, ConversationSelection, ConversationSelectionHandle,
    ConversationUsageTotals, ExecuteCommandEvent, GetRelevantFilesController, GitRepoModels,
    GitRepoStatusModel, GitStatusMetadata, LLMPreferences, LLMPreferencesEvent, ModelEvent,
    ParsedSlashCommandInput, PtyIntent, PtyIntentEvent, RepoDetectionSessionType,
    RepoDetectionSource, ShellCommandExecutorEvent, SizeInfo, SizeUpdate, SkillReference,
    SlashCommandDataSource as _, SlashCommandSelectionBehavior, StaticCommand, TerminalMode,
    TerminalModel, TerminalSurface, TerminalSurfaceInit, TuiSlashCommandDataSource,
    TuiSlashCommandDataSourceArgs, TuiZeroStateDataSource, COMMAND_REGISTRY,
    WAKEUP_THROTTLE_PERIOD,
};
use warp_core::settings::Setting;
use warp_editor::model::CoreEditorModel;
use warp_errors::report_error;
use warp_terminal::model::escape_sequences::C0;
use warp_util::local_or_remote_path::LocalOrRemotePath;
use warpui::SingletonEntity;
use warpui_core::elements::tui::{
    TuiChildView, TuiConstrainedBox, TuiContainer, TuiElement, TuiFlex, TuiText,
};
use warpui_core::keymap::macros::*;
use warpui_core::keymap::FixedBinding;
use warpui_core::platform::TerminationMode;
use warpui_core::r#async::Timer;
use warpui_core::{
    AppContext, Entity, EntityId, ModelHandle, TuiView, TypedActionView, ViewContext, ViewHandle,
};

use crate::alt_screen_element::{HostSizeSlot, TuiAltScreenElement, TuiHostSizeProbe};
use crate::autoupdate::{TuiAutoupdater, TuiAutoupdaterEvent};
use crate::clipboard::copy_to_clipboard;
use crate::conversation_selection::TuiConversationSelection;
use crate::exit_confirmation::{ExitConfirmation, CTRL_C_EXIT_WINDOW};
use crate::inline_menu::TuiInlineMenu;
use crate::input::{TuiInputView, TuiInputViewEvent};
use crate::input_mode_policy::{self, TuiInputModePolicy};
use crate::keybindings::TUI_BINDING_GROUP;
use crate::slash_commands::TuiSlashCommandModel;
use crate::transcript_view::{TuiTranscriptView, TuiTranscriptViewEvent};
use crate::transient_hint::{TransientHint, TransientHintTone};
use crate::tui_builder::TuiUiBuilder;
use crate::ui::compact_footer_path;
use crate::usage::UsageToggle;
use crate::warping_indicator::{render_response_summary, render_warping_indicator};
use crate::zero_state::render_zero_state;

/// Width used before the first layout pass pushes the real terminal width into the editor.
const INITIAL_INPUT_WIDTH: u16 = 80;
const MAX_INPUT_TEXT_ROWS: u16 = 6;
const MAX_INLINE_MENU_ROWS: u16 = 10;

/// The footer hint shown while the ctrl-c exit confirmation is armed.
const CTRL_C_EXIT_HINT: &str = "ctrl-c again to exit";

/// Events emitted by the TUI terminal session surface.
pub(crate) enum TuiTerminalSessionEvent {
    ExecuteCommand(Box<ExecuteCommandEvent>),
    WriteAgentInput {
        bytes: Cow<'static, [u8]>,
        mode: AIAgentPtyWriteMode,
    },
    /// Raw user bytes passed straight through to the PTY (alt-screen input).
    WriteToPty(Cow<'static, [u8]>),
    /// Pushes a new winsize to the PTY (alt-screen host-size tracking).
    ResizePty(SizeUpdate),
}

impl PtyIntentEvent for TuiTerminalSessionEvent {
    fn pty_intent(&self) -> Option<PtyIntent> {
        match self {
            Self::ExecuteCommand(event) => Some(PtyIntent::ExecuteCommand((**event).clone())),
            Self::WriteAgentInput { bytes, mode } => Some(PtyIntent::WriteAgentInput {
                bytes: bytes.clone(),
                mode: *mode,
            }),
            Self::WriteToPty(bytes) => Some(PtyIntent::WriteBytes(bytes.clone())),
            Self::ResizePty(size_update) => Some(PtyIntent::Resize(*size_update)),
        }
    }
}

/// Transient hint shown when a shell command is rejected because the PTY is
/// already running a command.
const COMMAND_ALREADY_RUNNING_HINT: &str = "cannot run — command already running";
const NEW_CONVERSATION_COMMAND_RUNNING_HINT: &str =
    "cannot start new conversation while terminal command is running";

/// Footer hint shown while the input is in `!` shell mode.
const SHELL_MODE_HINT: &str = "shell mode · esc to exit";
const COPY_SELECTION_HINT: &str = "copied to clipboard";
/// Keeps an agent-requested command's canonical block out of the TUI's
/// top-level transcript. The shell-command action embeds the block's terminal
/// content inside its own disclosure, so the canonical block must have zero
/// layout height even after the shared CLI-subagent transition unhides it for
/// the GUI's adjacent-block presentation.
fn hide_agent_requested_command_from_top_level(
    model: &Arc<FairMutex<TerminalModel>>,
    action_id: Option<&AIAgentActionId>,
) -> bool {
    let Some(action_id) = action_id else {
        return false;
    };
    model
        .lock()
        .block_list_mut()
        .set_visibility_of_block_for_ai_action(action_id, false);
    true
}

fn raw_prompt_if_not_blank(input: &str) -> Option<&str> {
    (!input.trim().is_empty()).then_some(input)
}

/// Typed actions handled by [`TuiTerminalSessionView`].
#[derive(Debug, Clone)]
pub(crate) enum TuiTerminalSessionAction {
    /// Ctrl-c anywhere in the session surface: cancel the running
    /// conversation, else clear the input; a second press within
    /// [`CTRL_C_EXIT_WINDOW`] exits the TUI. While the alt screen is active,
    /// ctrl-c is forwarded to the running program instead.
    Interrupt,
    /// Click on the footer's usage entry: flips the persisted credits⇄cost
    /// display-mode setting.
    ToggleUsageDisplay,
    /// Encoded input from the alt-screen element, forwarded to the PTY.
    WriteAltScreenInput(Vec<u8>),
}

/// The authenticated terminal/session surface rendered inside [`RootTuiView`].
pub(crate) struct TuiTerminalSessionView {
    transcript: ViewHandle<TuiTranscriptView>,
    input_view: ViewHandle<TuiInputView>,
    inline_menu: TuiInlineMenu,
    slash_commands_source: ModelHandle<TuiSlashCommandDataSource>,
    conversation_selection: ConversationSelectionHandle,
    ai_controller: ModelHandle<BlocklistAIController>,
    /// Read by the footer for the active session's working directory.
    active_session: ModelHandle<ActiveSession>,
    /// Repository currently containing the active session's working directory.
    current_repo_path: Option<LocalOrRemotePath>,
    /// Watcher-backed branch and uncommitted diff metadata for the footer.
    git_repo_status: Option<ModelHandle<GitRepoStatusModel>>,
    /// This view's surface id, used to resolve the active model for the footer
    /// the same way the request path does.
    terminal_surface_id: EntityId,
    /// Armed by a ctrl-c press; a second press while armed exits the TUI.
    /// The footer shows [`CTRL_C_EXIT_HINT`] while armed.
    exit_confirmation: ExitConfirmation,
    /// Credits⇄cost display state for the footer's clickable usage entry.
    usage_toggle: UsageToggle,
    ai_context_model: ModelHandle<BlocklistAIContextModel>,
    ai_input_model: ModelHandle<BlocklistAIInputModel>,
    terminal_model: Arc<FairMutex<TerminalModel>>,
    /// Transient notice shown in the footer's hint slot (e.g. a rejected
    /// shell submission).
    transient_hint: TransientHint,
    /// The host terminal's size in cells, recorded during every layout pass
    /// (by the alt-screen element or the transcript-path probe). Read when
    /// pushing PTY resizes while an alt-screen app is running.
    host_size: HostSizeSlot,
    /// The transcript-mode terminal size captured when the alt screen
    /// activated, restored when the alt-screen app exits.
    pre_alt_screen_size: Option<SizeInfo>,
}

/// Registers the session surface's keybindings. Called once at TUI startup
/// from `keybindings::init`. Ctrl-c is a fixed (non-remappable) binding,
/// mirroring peer agent CLIs that treat it as reserved.
pub(crate) fn init(app: &mut AppContext) {
    app.register_fixed_bindings([FixedBinding::new(
        "ctrl-c",
        TuiTerminalSessionAction::Interrupt,
        id!(TuiTerminalSessionView::ui_name()),
    )
    .with_group(TUI_BINDING_GROUP)]);
}

impl TuiTerminalSessionView {
    /// Builds the transcript-capable terminal surface for a manager-backed session.
    pub(crate) fn new(surface_init: TerminalSurfaceInit, ctx: &mut ViewContext<Self>) -> Self {
        let TerminalSurfaceInit {
            model,
            sessions,
            model_events,
            wakeups_rx,
            ..
        } = surface_init;

        let terminal_surface_id: EntityId = ctx.view_id();
        let active_session =
            ctx.add_model(|ctx| ActiveSession::new(sessions.clone(), model_events.clone(), ctx));
        let conversation_selection = ctx.add_model(|ctx| {
            Box::new(TuiConversationSelection::new(terminal_surface_id, ctx))
                as Box<dyn ConversationSelection>
        });
        let context_model = ctx.add_model(|ctx| {
            BlocklistAIContextModel::new(
                sessions,
                &model_events,
                model.clone(),
                terminal_surface_id,
                conversation_selection.clone(),
                ctx,
            )
        });
        let ai_input_model = ctx.add_model(|ctx| {
            BlocklistAIInputModel::new(
                model.clone(),
                conversation_selection.clone(),
                context_model.clone(),
                Rc::new(TuiInputModePolicy),
                terminal_surface_id,
                ctx,
            )
        });
        let get_relevant_files_controller = ctx.add_model(GetRelevantFilesController::new);
        let action_model = ctx.add_model(|ctx| {
            BlocklistAIActionModel::new(
                model.clone(),
                active_session.clone(),
                &model_events,
                get_relevant_files_controller,
                terminal_surface_id,
                ctx,
            )
        });
        let ai_controller = ctx.add_model(|ctx| {
            BlocklistAIController::new(
                ai_input_model.clone(),
                context_model.clone(),
                conversation_selection.clone(),
                action_model.clone(),
                active_session.clone(),
                model.clone(),
                terminal_surface_id,
                ctx,
            )
        });
        let cli_subagent_controller = ctx.add_model(|ctx| {
            CLISubagentController::new(
                &ai_controller,
                &action_model,
                None,
                model.clone(),
                &model_events,
                terminal_surface_id,
                ctx,
            )
        });
        let model_for_cli_subagent_events = model.clone();
        ctx.subscribe_to_model(&cli_subagent_controller, move |_, _, event, ctx| {
            if let CLISubagentEvent::SpawnedSubagent {
                initial_requested_command_action_id,
                ..
            } = event
            {
                if hide_agent_requested_command_from_top_level(
                    &model_for_cli_subagent_events,
                    initial_requested_command_action_id.as_ref(),
                ) {
                    ctx.notify();
                }
            }
        });
        let transcript = ctx.add_typed_action_tui_view(|ctx| {
            TuiTranscriptView::new(
                terminal_surface_id,
                model.clone(),
                action_model.clone(),
                &model_events,
                ctx,
            )
        });
        let input_editor_model =
            ctx.add_model(|ctx| CodeEditorModel::new_tui(INITIAL_INPUT_WIDTH, ctx));
        let slash_commands_source = ctx.add_model(|ctx| {
            TuiSlashCommandDataSource::new(
                TuiSlashCommandDataSourceArgs {
                    active_session: active_session.clone(),
                    cli_subagent_controller,
                    terminal_view_id: terminal_surface_id,
                },
                ctx,
            )
        });
        let zero_state_source = TuiZeroStateDataSource::new(&slash_commands_source);
        let slash_commands_mixer = ctx.add_model(|ctx| {
            build_slash_command_mixer(slash_commands_source.clone(), zero_state_source, ctx)
        });
        let slash_commands = ctx.add_model(|ctx| {
            TuiSlashCommandModel::new(
                input_editor_model.clone(),
                slash_commands_source.clone(),
                slash_commands_mixer,
                ctx,
            )
        });
        ctx.subscribe_to_model(&slash_commands, |_, _, _, ctx| ctx.notify());
        // Typing after a ctrl-c press disarms the pending exit confirmation.
        // The ctrl-c buffer clear leaves the buffer empty, so the window it
        // arms survives its own clear.
        let editor_for_exit_disarm = input_editor_model.clone();
        ctx.subscribe_to_model(&input_editor_model, move |view, _, event, ctx| {
            if !matches!(event, CodeEditorModelEvent::ContentChanged { .. }) {
                return;
            }
            let is_empty = editor_for_exit_disarm
                .as_ref(ctx)
                .content()
                .as_ref(ctx)
                .is_empty();
            if !is_empty && view.exit_confirmation.disarm() {
                ctx.notify();
            }
        });

        let editor_for_selection = input_editor_model.clone();
        let transcript_for_selection = transcript.clone();
        ctx.subscribe_to_model(&input_editor_model, move |_, _, event, ctx| {
            if !matches!(event, CodeEditorModelEvent::SelectionChanged) {
                return;
            }

            let has_selection = !editor_for_selection
                .as_ref(ctx)
                .buffer_selection_model()
                .as_ref(ctx)
                .first_selection_is_single_cursor();
            if has_selection {
                transcript_for_selection.update(ctx, |transcript, ctx| {
                    transcript.clear_selection(ctx);
                });
            }
        });

        let input_mode_for_input_view = ai_input_model.clone();
        let inline_menu = TuiInlineMenu::SlashCommands(slash_commands.clone());
        let inline_menu_for_input = inline_menu.clone();
        let input_view = ctx.add_typed_action_tui_view(move |ctx| {
            TuiInputView::new(
                input_editor_model,
                input_mode_for_input_view,
                Some(inline_menu_for_input),
                ctx,
            )
        });

        ctx.subscribe_to_view(&transcript, |view, _, event, ctx| match event {
            TuiTranscriptViewEvent::SelectionStarted => {
                view.input_view
                    .update(ctx, |input, ctx| input.clear_selection(ctx));
            }
            TuiTranscriptViewEvent::SelectionEnded(text) => {
                copy_to_clipboard(text);
                view.show_copy_hint(ctx);
            }
        });

        ctx.subscribe_to_view(&input_view, |view, _, event, ctx| match event {
            TuiInputViewEvent::Submitted(text) => view.handle_submitted(text.clone(), ctx),
            TuiInputViewEvent::AcceptedSlashCommand(action) => {
                view.handle_accepted_slash_command(action, ctx);
            }
        });
        // The input box border color and the footer's shell-mode hint depend
        // on the input mode.
        ctx.subscribe_to_model(&ai_input_model, |_, _, _, ctx| ctx.notify());
        // The warping indicator between the transcript and the input box
        // tracks the selected conversation: re-render when its status changes
        // or an exchange starts (the elapsed counter's anchor) on this
        // surface, and when the selected conversation changes.
        ctx.subscribe_to_model(
            &BlocklistAIHistoryModel::handle(ctx),
            |view, _, event, ctx| view.handle_history_event(event, ctx),
        );
        ctx.subscribe_to_model(&conversation_selection, |_, _, _, ctx| ctx.notify());

        // The zero state's "What's new" section: fetch the changelog once at
        // startup and re-render when it arrives. The model no-ops when a
        // changelog is already cached; the other changelog events (request
        // failed, image fetched) don't change what the zero state renders.
        ChangelogModel::handle(ctx).update(ctx, |changelog, ctx| {
            changelog.check_for_changelog(ChangelogRequestType::WindowLaunch, ctx);
        });
        ctx.subscribe_to_model(&ChangelogModel::handle(ctx), |_, _, event, ctx| {
            if let ChangelogModelEvent::ChangelogRequestComplete { .. } = event {
                ctx.notify();
            }
        });
        // The zero state's version line shows the background auto-update
        // status: re-render as the updater progresses.
        ctx.subscribe_to_model(&TuiAutoupdater::handle(ctx), |_, _, event, ctx| {
            let TuiAutoupdaterEvent::StatusChanged = event;
            ctx.notify();
        });
        // The zero state's project section: rules/skills discovery is
        // asynchronous, so re-render as indexed results land. `PathIndexed`
        // accompanies every project-rules mutation (`KnownRulesChanged` is a
        // persistence-oriented duplicate), and `GlobalRulesChanged` covers
        // global rules, which the zero state doesn't show.
        ctx.subscribe_to_model(&ProjectContextModel::handle(ctx), |_, _, event, ctx| {
            if let ProjectContextModelEvent::PathIndexed = event {
                ctx.notify();
            }
        });

        // Bridge shared shell-tool executor events into terminal-manager PTY intents.
        let shell_command_executor = action_model.as_ref(ctx).shell_command_executor(ctx);
        let model_for_shell_events = model.clone();
        ctx.subscribe_to_model(&shell_command_executor, move |view, _, event, ctx| {
            view.handle_shell_command_executor_event(event, &model_for_shell_events, ctx);
        });

        // These events update block metadata or grids the transcript reads.
        // PTY output redraws are driven by `wakeups_rx` below. Alt-screen
        // enter/exit swaps the whole surface between the transcript UI and
        // the alt-screen grid.
        ctx.subscribe_to_model(&model_events, |view, _, event, ctx| match event {
            ModelEvent::BlockCompleted(_)
            | ModelEvent::AfterBlockStarted { .. }
            | ModelEvent::BlockMetadataReceived(_)
            | ModelEvent::BlockWorkingDirectoryUpdated(_)
            | ModelEvent::BackgroundBlockStarted
            | ModelEvent::TerminalClear
            | ModelEvent::PromptUpdated
            | ModelEvent::Typeahead
            | ModelEvent::Handler(_)
            | ModelEvent::FinishUpdate(_) => ctx.notify(),
            ModelEvent::TerminalModeSwapped(mode) => {
                view.handle_terminal_mode_swapped(mode, ctx);
            }
            _ => {}
        });
        // The footer shows the active model, working directory, and usage
        // entry: re-render when the TUI model or usage-display-mode settings
        // change (click or settings-file hot reload), when model display
        // names arrive from the server post-login, or when the session's
        // working directory changes.
        ctx.subscribe_to_model(&AISettings::handle(ctx), |_, _, event, ctx| {
            if matches!(
                event,
                AISettingsChangedEvent::TuiAgentModel { .. }
                    | AISettingsChangedEvent::TuiUsageDisplayMode { .. }
            ) {
                ctx.notify();
            }
        });
        ctx.subscribe_to_model(&LLMPreferences::handle(ctx), |_, _, event, ctx| {
            if let LLMPreferencesEvent::UpdatedAvailableLLMs = event {
                ctx.notify();
            }
        });
        ctx.subscribe_to_model(&active_session, |view, _, event, ctx| match event {
            ActiveSessionEvent::UpdatedPwd => {
                // Run repo detection so project rules and skills follow the
                // session's working directory (the GUI's equivalent lives in
                // `TerminalView::apply_block_metadata_update`). The first
                // post-bootstrap precmd metadata transitions the cwd from
                // `None` to `Some`, so this also covers the launch directory.
                if let Some(cwd) = view
                    .active_session
                    .as_ref(ctx)
                    .current_working_directory()
                    .cloned()
                {
                    let detect_repo = detect_possible_git_repo(
                        RepoDetectionSessionType::Local,
                        &cwd,
                        RepoDetectionSource::TerminalNavigation,
                        ctx,
                    );
                    ctx.spawn(detect_repo, |view, repo_path, ctx| {
                        view.update_git_status_subscription(repo_path, ctx);
                    });
                }
                ctx.notify();
            }
            ActiveSessionEvent::Bootstrapped => {}
        });
        // The footer's usage entry shows the selected conversation's token/cost
        // totals: re-render when that conversation's usage metadata updates.
        ctx.subscribe_to_model(
            &BlocklistAIHistoryModel::handle(ctx),
            |view, _, event, ctx| {
                if let BlocklistAIHistoryEvent::ConversationUsageMetadataUpdated {
                    conversation_id,
                } = event
                {
                    let selected = view
                        .conversation_selection
                        .as_ref(ctx)
                        .selected_conversation_id(ctx);
                    if selected == Some(*conversation_id) {
                        ctx.notify();
                    }
                }
            },
        );

        // A wakeup is also how a running block becomes visible: its height is 0
        // until the long-running render-delay timer fires and sends a wakeup
        // (see `Block::wakeup_after_delay`). Heights are otherwise only
        // recomputed when PTY bytes arrive, so a silent command (e.g. `sleep`)
        // would stay invisible until it finishes. Mirror the GUI's
        // `handle_terminal_wakeup` by throttling the stream and refreshing
        // live block heights here.
        ctx.spawn_stream_local(
            throttle(WAKEUP_THROTTLE_PERIOD, wakeups_rx),
            |view, _, ctx| {
                let is_alt_screen_active = {
                    let mut model = view.terminal_model.lock();
                    let is_alt_screen_active = model.is_alt_screen_active();
                    if !is_alt_screen_active {
                        model.block_list_mut().update_background_block_height();
                        model.block_list_mut().update_active_block_height();
                    }
                    is_alt_screen_active
                };

                // While an alt-screen app is producing output, keep the PTY's
                // winsize tracking the host terminal (covers a host resize
                // that happened after the alt screen was entered).
                if is_alt_screen_active {
                    view.sync_pty_size_to_host(ctx);
                }

                ctx.notify();
            },
            |_, _| {},
        );

        // Focus the input view so the keymap responder chain is
        // [root, session, input]: input bindings win for keys they define,
        // and unbound keys (ctrl-c) fall through to the session/root bindings.
        ctx.focus(&input_view);

        Self {
            transcript,
            input_view,
            inline_menu,
            slash_commands_source,
            conversation_selection,
            ai_controller,
            active_session,
            current_repo_path: None,
            git_repo_status: None,
            terminal_surface_id,
            exit_confirmation: ExitConfirmation::default(),
            usage_toggle: UsageToggle::default(),
            ai_context_model: context_model,
            ai_input_model,
            terminal_model: model,
            transient_hint: TransientHint::default(),
            host_size: Rc::new(Cell::new(None)),
            pre_alt_screen_size: None,
        }
    }

    /// Swaps the surface between the transcript UI and the alt-screen grid.
    ///
    /// Entering the alt screen focuses this view so the input view's editing
    /// keybindings drop out of the keymap responder chain — every keystroke
    /// then reaches the alt-screen element (ctrl-c stays bound here and is
    /// forwarded by [`Self::handle_interrupt`]). The PTY is resized to the
    /// host terminal so the program lays out against the real window, and the
    /// transcript-mode size is restored when the program exits.
    fn handle_terminal_mode_swapped(&mut self, mode: &TerminalMode, ctx: &mut ViewContext<Self>) {
        match mode {
            TerminalMode::AltScreen => {
                self.pre_alt_screen_size =
                    Some(self.terminal_model.lock().block_list().size().to_owned());
                ctx.focus_self();
                self.sync_pty_size_to_host(ctx);
            }
            TerminalMode::BlockList => {
                ctx.focus(&self.input_view);
                if let Some(size) = self.pre_alt_screen_size.take() {
                    self.resize_pty(size.rows(), size.columns(), ctx);
                }
            }
        }
        ctx.notify();
    }

    /// Resizes the PTY (and terminal model) to the host terminal size
    /// recorded during the last layout pass, if it is known and differs.
    fn sync_pty_size_to_host(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(host_size) = self.host_size.get() else {
            return;
        };
        self.resize_pty(
            usize::from(host_size.height),
            usize::from(host_size.width),
            ctx,
        );
    }

    /// Resizes the terminal model and pushes the matching winsize to the PTY
    /// (which delivers `SIGWINCH` to the foreground program). No-ops when the
    /// model already has the requested size.
    fn resize_pty(&mut self, rows: usize, columns: usize, ctx: &mut ViewContext<Self>) {
        let size_update = {
            let mut model = self.terminal_model.lock();
            let last_size = model.block_list().size().to_owned();
            if last_size.rows() == rows && last_size.columns() == columns {
                return;
            }
            let size_update = SizeUpdate::for_tui_host_resize(last_size, rows, columns);
            // Mirrors the GUI's `resize_internal`: the view applies the
            // update to the model, and the emitted intent resizes the PTY.
            model.resize(size_update);
            size_update
        };
        ctx.emit(TuiTerminalSessionEvent::ResizePty(size_update));
        ctx.notify();
    }

    /// Re-renders on history events that can change the warping indicator:
    /// the selected conversation's status changing, or an exchange starting
    /// (which re-anchors the elapsed counter) on this surface.
    fn handle_history_event(
        &mut self,
        event: &BlocklistAIHistoryEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        if event
            .terminal_surface_id()
            .is_some_and(|id| id != self.terminal_surface_id)
        {
            return;
        }
        if matches!(
            event,
            BlocklistAIHistoryEvent::AppendedExchange { .. }
                | BlocklistAIHistoryEvent::UpdatedConversationStatus { .. }
        ) {
            ctx.notify();
        }
    }

    /// Displays `text` in the footer's hint slot for the transient-hint
    /// duration, then reverts to the persistent content.
    fn show_transient_hint(&mut self, text: String, ctx: &mut ViewContext<Self>) {
        self.transient_hint
            .show(text, ctx, |view| &mut view.transient_hint);
    }

    /// Displays success-colored feedback in the transient footer slot.
    fn show_copy_hint(&mut self, ctx: &mut ViewContext<Self>) {
        self.transient_hint
            .show_success(COPY_SELECTION_HINT.to_owned(), ctx, |view| {
                &mut view.transient_hint
            });
    }

    /// Handles a ctrl-c press: a second press within [`CTRL_C_EXIT_WINDOW`]
    /// exits the TUI; otherwise one contextual action runs — cancel the running
    /// conversation if there is one, else clear the input — and the exit
    /// confirmation is (re-)armed, surfacing [`CTRL_C_EXIT_HINT`] in the footer.
    fn handle_interrupt(&mut self, ctx: &mut ViewContext<Self>) {
        // While an alt-screen app is running, ctrl-c belongs to it: forward
        // ETX instead of cancelling/clearing (mirrors a regular terminal).
        if self.terminal_model.lock().is_alt_screen_active() {
            ctx.emit(TuiTerminalSessionEvent::WriteToPty(Cow::Borrowed(&[
                C0::ETX,
            ])));
            return;
        }

        let now = Instant::now();
        if self.exit_confirmation.should_exit(now) {
            ctx.terminate_app(TerminationMode::ForceTerminate, None);
            return;
        }

        if !self.cancel_active_conversation(ctx) {
            self.input_view.update(ctx, |input, ctx| input.clear(ctx));
        }

        // Arm (or re-arm) the confirmation, and disarm + repaint when the
        // window lapses. A re-arm supersedes this (now stale) timer, making
        // its `disarm_expired` a no-op rather than clearing the newer window.
        let window_expires_at = self.exit_confirmation.arm(now);
        ctx.spawn(Timer::after(CTRL_C_EXIT_WINDOW), move |view, _, ctx| {
            if view.exit_confirmation.disarm_expired(window_expires_at) {
                ctx.notify();
            }
        });
        ctx.notify();
    }

    /// Cancels the surface's running conversation (in-flight stream or pending
    /// tool actions), returning whether there was one to cancel.
    fn cancel_active_conversation(&mut self, ctx: &mut ViewContext<Self>) -> bool {
        let terminal_surface_id = ctx.view_id();
        self.ai_controller.update(ctx, |controller, ctx| {
            let conversation_id = BlocklistAIHistoryModel::as_ref(ctx)
                .active_conversation(terminal_surface_id)
                // A brand-new conversation reports `InProgress` before any
                // exchange exists; there is nothing to cancel yet.
                .filter(|conversation| !conversation.is_empty())
                .filter(|conversation| {
                    let status = conversation.status();
                    status.is_in_progress() || status.is_blocked()
                })
                .map(|conversation| conversation.id());
            let Some(conversation_id) = conversation_id else {
                return false;
            };
            controller.cancel_conversation_progress(
                conversation_id,
                CancellationReason::ManuallyCancelled,
                ctx,
            );
            true
        })
    }

    /// Builds the status footer under the input box. The left slot shows one
    /// hint at a time — the ctrl-c exit confirmation while armed, else a
    /// transient notice, else the shell-mode callout; the active model and
    /// working directory are pushed to the right edge behind a flex spacer.
    /// Every child truncates to a single row, so the row lays out one row tall.
    fn render_footer(&self, ctx: &AppContext) -> TuiFlex {
        let builder = TuiUiBuilder::from_app(ctx);
        let muted = builder.muted_text_style();
        let mut left = TuiFlex::row();
        // Left slot, highest priority first: while armed, the ctrl-c hint
        // replaces the other hints in place.
        let hint = if self.exit_confirmation.is_armed() {
            Some((CTRL_C_EXIT_HINT.to_owned(), muted))
        } else if let Some((transient, tone)) = self.transient_hint.current() {
            let style = match tone {
                TransientHintTone::Muted => muted,
                TransientHintTone::Success => builder.success_glyph_style(),
            };
            Some((transient.to_owned(), style))
        } else if self.is_shell_mode(ctx) {
            Some((
                SHELL_MODE_HINT.to_owned(),
                builder.shell_mode_accent_style(),
            ))
        } else {
            None
        };

        if let Some((text, style)) = hint {
            left = left.child(TuiText::new(text).with_style(style).truncate().finish());
        }
        let mut footer = TuiFlex::row().flex_child(left.finish());
        let model_name = LLMPreferences::as_ref(ctx)
            .get_active_base_model(ctx, Some(self.terminal_surface_id))
            .display_name
            .clone();
        footer = footer.child(TuiText::new(" ").truncate().finish()).child(
            TuiText::new(model_name)
                .with_style(builder.primary_text_style())
                .truncate()
                .finish(),
        );
        if let Some(cwd) = self.current_working_directory(ctx) {
            footer = footer.child(
                TuiText::new(format!(" {}", compact_footer_path(&cwd)))
                    .with_style(muted)
                    .truncate()
                    .finish(),
            );
        }
        let git_stats = if let Some(metadata) = self.git_status_metadata(ctx) {
            footer = footer.child(
                TuiText::new(format!(" ↬ {}", metadata.current_branch_name))
                    .with_style(muted)
                    .truncate()
                    .finish(),
            );
            Some(metadata.stats_against_head)
        } else {
            None
        };
        // Usage entry: the selected conversation's credits/cost totals,
        // hidden until any usage has been reported. The displayed unit is the
        // persisted `agents.usage_display_mode` setting; a click dispatches
        // the toggle action (the element pass cannot write settings
        // directly).
        if let Some(totals) = self.selected_conversation_usage_totals(ctx) {
            let mode = AISettings::as_ref(ctx).usage_display_mode;
            footer = footer
                .child(TuiText::new(" • ").with_style(muted).truncate().finish())
                .child(
                    self.usage_toggle
                        .render_entry(mode, totals, ctx, |event_ctx, _| {
                            event_ctx.dispatch_typed_action(
                                TuiTerminalSessionAction::ToggleUsageDisplay,
                            );
                        }),
                );
        }
        if let Some(stats) = git_stats {
            if stats.total_additions > 0 || stats.total_deletions > 0 {
                footer = footer.child(TuiText::new(" • ").with_style(muted).truncate().finish());
                if stats.total_additions > 0 {
                    footer = footer.child(
                        TuiText::new(format!("+{}", stats.total_additions))
                            .with_style(builder.diff_added_style())
                            .truncate()
                            .finish(),
                    );
                }
                if stats.total_deletions > 0 {
                    if stats.total_additions > 0 {
                        footer = footer.child(TuiText::new(" ").truncate().finish());
                    }
                    footer = footer.child(
                        TuiText::new(format!("-{}", stats.total_deletions))
                            .with_style(builder.diff_removed_style())
                            .truncate()
                            .finish(),
                    );
                }
            }
        }
        footer
    }

    /// Updates the watcher-backed git-status subscription after repository
    /// detection completes for the active working directory.
    fn update_git_status_subscription(
        &mut self,
        repo_path: Option<LocalOrRemotePath>,
        ctx: &mut ViewContext<Self>,
    ) {
        if self.current_repo_path == repo_path && self.git_repo_status.is_some() {
            return;
        }
        self.current_repo_path = repo_path.clone();
        self.git_repo_status = None;

        let Some(repo_path) = repo_path else {
            ctx.notify();
            return;
        };
        match GitRepoModels::handle(ctx)
            .update(ctx, |models, ctx| models.subscribe(&repo_path, ctx))
        {
            Ok(handle) => {
                ctx.subscribe_to_model(&handle, |_, _, _, ctx| ctx.notify());
                self.git_repo_status = Some(handle);
            }
            Err(error) => {
                log::warn!("Unable to subscribe TUI footer to git status: {error}");
            }
        }
        ctx.notify();
    }

    fn git_status_metadata<'a>(&self, ctx: &'a AppContext) -> Option<&'a GitStatusMetadata> {
        self.git_repo_status.as_ref()?.as_ref(ctx).metadata(ctx)
    }

    /// Flips the footer usage entry's persisted credits⇄cost display mode.
    /// The settings-changed event re-renders every subscribed surface.
    fn toggle_usage_display(&mut self, ctx: &mut ViewContext<Self>) {
        let next = AISettings::as_ref(ctx).usage_display_mode.toggled();
        AISettings::handle(ctx).update(ctx, |settings, ctx| {
            if let Err(error) = settings.usage_display_mode.set_value(next, ctx) {
                report_error!("failed to persist the TUI usage display mode: {error:#}");
            }
        });
    }

    /// The selected conversation's accumulated usage totals, or `None` (entry
    /// hidden) until any usage has been reported.
    fn selected_conversation_usage_totals(
        &self,
        ctx: &AppContext,
    ) -> Option<ConversationUsageTotals> {
        let totals = self
            .conversation_selection
            .as_ref(ctx)
            .selected_conversation(ctx)?
            .usage_totals();
        (totals != ConversationUsageTotals::default()).then_some(totals)
    }

    /// The session's working directory. The cwd only arrives once shell
    /// metadata flows (warpified sessions); until then fall back to the
    /// process cwd the TUI's shell was spawned with.
    fn current_working_directory(&self, ctx: &AppContext) -> Option<String> {
        self.active_session
            .as_ref(ctx)
            .current_working_directory()
            .cloned()
            .or_else(|| {
                std::env::current_dir()
                    .ok()
                    .map(|cwd| cwd.to_string_lossy().into_owned())
            })
    }

    /// Whether the input is in `!` shell mode (locked shell input).
    fn is_shell_mode(&self, ctx: &AppContext) -> bool {
        input_mode_policy::is_shell_mode(self.ai_input_model.as_ref(ctx))
    }

    /// Routes a submission to shell execution or the agent conversation based
    /// on the input mode.
    fn handle_submitted(&mut self, text: String, ctx: &mut ViewContext<Self>) {
        if self.is_shell_mode(ctx) {
            self.execute_user_command(&text, ctx);
        } else {
            self.handle_submitted_input(&text, ctx);
        }
        ctx.notify();
    }

    /// Executes `command` in the session's PTY as a plain user command.
    ///
    /// Mirrors the GUI's shell-mode submission: rejected while the agent holds
    /// the PTY with an active long-running command (the input keeps its text
    /// and a transient hint is shown), and an in-progress conversation is
    /// cancelled when the command runs. On success the input clears and exits
    /// shell mode back to agent input.
    fn execute_user_command(&mut self, command: &str, ctx: &mut ViewContext<Self>) {
        // A whitespace-only command is a no-op; stay in shell mode. The command
        // itself is sent to the PTY untrimmed, exactly as typed.
        if command.trim().is_empty() {
            return;
        }

        // Keep the lock scope to these reads only (see the terminal-model
        // locking guidance).
        let (is_pty_busy, session_id) = {
            let terminal_model = self.terminal_model.lock();
            let block_list = terminal_model.block_list();
            let active_block = block_list.active_block();
            let is_pty_busy = !block_list.is_bootstrapped()
                || (active_block.is_active_and_long_running()
                    && !active_block.is_in_band_command_block());
            (is_pty_busy, active_block.session_id())
        };
        let Some(session_id) = session_id else {
            log::warn!("Unable to execute TUI user command: no active session");
            return;
        };
        if is_pty_busy {
            self.show_transient_hint(COMMAND_ALREADY_RUNNING_HINT.to_owned(), ctx);
            return;
        }

        // Executing a shell command cancels an in-progress conversation
        // (mirrors the GUI; the running command above is left untouched).
        if let Some(conversation_id) = self
            .conversation_selection
            .as_ref(ctx)
            .selected_conversation_id(ctx)
        {
            let is_in_progress = BlocklistAIHistoryModel::as_ref(ctx)
                .conversation(&conversation_id)
                .is_some_and(|conversation| conversation.status().is_in_progress());
            if is_in_progress {
                self.ai_controller.update(ctx, |controller, ctx| {
                    controller.cancel_conversation_progress(
                        conversation_id,
                        CancellationReason::UserCommandExecuted,
                        ctx,
                    );
                });
            }
        }

        ctx.emit(TuiTerminalSessionEvent::ExecuteCommand(Box::new(
            ExecuteCommandEvent {
                command: command.to_owned(),
                session_id,
                workflow_id: None,
                workflow_command: None,
                should_add_command_to_history: true,
                source: CommandExecutionSource::User,
            },
        )));

        // The submission was accepted: clear the input and return to agent mode.
        self.input_view.update(ctx, |input_view, ctx| {
            input_view.clear(ctx);
            input_view.exit_shell_mode(ctx);
        });
    }

    /// Sends a prompt to the selected conversation, creating one if needed.
    fn send_prompt(&mut self, prompt: String, ctx: &mut ViewContext<Self>) {
        let conversation_id = match self
            .conversation_selection
            .as_ref(ctx)
            .selected_conversation_id(ctx)
        {
            Some(conversation_id) => conversation_id,
            None => match self.conversation_selection.update(ctx, |selection, ctx| {
                selection.try_start_new_conversation(AgentViewEntryOrigin::Tui, ctx)
            }) {
                Ok(conversation_id) => conversation_id,
                Err(error) => {
                    report_error!(
                        anyhow::Error::new(error).context("Failed to create TUI conversation")
                    );
                    return;
                }
            },
        };
        self.ai_controller.update(ctx, |controller, ctx| {
            controller.send_user_query_in_conversation(prompt, conversation_id, None, ctx);
        });
    }

    fn handle_submitted_input(&mut self, input: &str, ctx: &mut ViewContext<Self>) {
        match self
            .slash_commands_source
            .as_ref(ctx)
            .parse_input(input, ctx)
        {
            ParsedSlashCommandInput::SlashCommand(detected_command) => {
                self.execute_tui_slash_command(
                    &detected_command.command,
                    detected_command.argument.as_ref(),
                    ctx,
                );
            }
            ParsedSlashCommandInput::SkillCommand(detected_skill) => {
                self.execute_skill_command(detected_skill.reference, detected_skill.argument, ctx);
            }
            ParsedSlashCommandInput::None | ParsedSlashCommandInput::Composing { .. } => {
                let prompt = raw_prompt_if_not_blank(input);
                self.input_view.update(ctx, |input_view, ctx| {
                    input_view.clear(ctx);
                });
                if let Some(prompt) = prompt {
                    self.send_prompt(prompt.to_owned(), ctx);
                }
            }
        }
    }

    fn execute_skill_command(
        &mut self,
        reference: SkillReference,
        user_query: Option<String>,
        ctx: &mut ViewContext<Self>,
    ) {
        let result = self.ai_controller.update(ctx, |controller, ctx| {
            controller.send_invoke_skill_request(reference, user_query, ctx)
        });
        match result {
            Ok(()) => {
                self.input_view.update(ctx, |input, ctx| input.clear(ctx));
            }
            Err(error) => {
                self.show_transient_hint(error.to_string(), ctx);
            }
        }
    }

    fn handle_accepted_slash_command(
        &mut self,
        action: &AcceptSlashCommandOrSavedPrompt,
        ctx: &mut ViewContext<Self>,
    ) {
        match action {
            AcceptSlashCommandOrSavedPrompt::SlashCommand { id } => {
                let Some(command) = COMMAND_REGISTRY.get_command(id) else {
                    log::debug!("TUI slash command selection is not supported yet: {id:?}");
                    ctx.notify();
                    return;
                };
                self.select_tui_slash_command(command, ctx);
            }
            AcceptSlashCommandOrSavedPrompt::SavedPrompt { id } => {
                let Some(prompt) = saved_prompt_text_for_id(id, ctx) else {
                    log::warn!("Tried to insert saved prompt for id {id:?} but it does not exist");
                    return;
                };
                self.input_view.update(ctx, |input, ctx| {
                    input.set_text(&prompt, ctx);
                });
                record_saved_prompt_accepted(true, ctx);
            }
            AcceptSlashCommandOrSavedPrompt::Skill { name, .. } => {
                self.input_view.update(ctx, |input, ctx| {
                    input.set_text(&format!("/{name} "), ctx);
                });
            }
        }
        ctx.notify();
    }

    fn select_tui_slash_command(&mut self, command: &StaticCommand, ctx: &mut ViewContext<Self>) {
        match slash_command_selection_behavior(command) {
            SlashCommandSelectionBehavior::InsertCommandText(text) => {
                self.input_view.update(ctx, |input, ctx| {
                    input.set_text(&text, ctx);
                });
            }
            SlashCommandSelectionBehavior::Execute => {
                self.execute_tui_slash_command(command, None, ctx);
            }
        }
    }

    fn execute_tui_slash_command(
        &mut self,
        command: &StaticCommand,
        argument: Option<&String>,
        ctx: &mut ViewContext<Self>,
    ) {
        if command.name == slash_commands::AGENT.name || command.name == slash_commands::NEW.name {
            if !self
                .ai_context_model
                .as_ref(ctx)
                .can_start_new_conversation()
            {
                self.show_transient_hint(NEW_CONVERSATION_COMMAND_RUNNING_HINT.to_owned(), ctx);
                return;
            }
            self.cancel_active_conversation(ctx);
            let terminal_surface_id = ctx.view_id();
            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                history.clear_conversations_for_terminal_surface(terminal_surface_id, ctx);
            });
            self.conversation_selection.update(ctx, |selection, ctx| {
                selection.select_new_conversation(AgentViewEntryOrigin::Tui, ctx);
            });
            if let Some(prompt) = argument
                .map(|argument| argument.trim())
                .filter(|argument| !argument.is_empty())
            {
                self.send_prompt(prompt.to_owned(), ctx);
            }
            self.input_view.update(ctx, |input, ctx| input.clear(ctx));
            record_static_slash_command_accepted(command.name, true, ctx);
        } else if slash_command_is_submitted_as_prompt(command) {
            self.input_view.update(ctx, |input, ctx| input.clear(ctx));
            let prompt = argument
                .map(|argument| {
                    if argument.is_empty() {
                        command.name.to_owned()
                    } else {
                        format!("{} {}", command.name, argument)
                    }
                })
                .unwrap_or_else(|| command.name.to_owned());
            self.send_prompt(prompt, ctx);
            record_static_slash_command_accepted(command.name, true, ctx);
        } else {
            log::debug!(
                "TUI slash command selection is not supported yet: {}",
                command.name
            );
        }
    }

    /// Bridges shared shell-tool executor events into terminal-manager PTY intents.
    fn handle_shell_command_executor_event(
        &mut self,
        event: &ShellCommandExecutorEvent,
        model: &Arc<FairMutex<TerminalModel>>,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            ShellCommandExecutorEvent::ExecuteCommand { action_id, command } => {
                let Some((session_id, conversation_id)) = (|| {
                    let model = model.lock();
                    let session_id = model.block_list().active_block().session_id()?;
                    let conversation_id = BlocklistAIHistoryModel::as_ref(ctx)
                        .conversation_id_for_action(action_id, ctx.view_id())?;
                    Some((session_id, conversation_id))
                })() else {
                    log::warn!(
                        "Unable to execute TUI agent-requested command for action {action_id:?}"
                    );
                    return;
                };

                ctx.emit(TuiTerminalSessionEvent::ExecuteCommand(Box::new(
                    ExecuteCommandEvent {
                        command: command.clone(),
                        session_id,
                        workflow_id: None,
                        workflow_command: None,
                        should_add_command_to_history: true,
                        source: CommandExecutionSource::AI {
                            metadata: AgentInteractionMetadata::new_hidden(
                                action_id.clone(),
                                conversation_id,
                            ),
                        },
                    },
                )));
            }
            ShellCommandExecutorEvent::WriteToPty { input, mode } => {
                ctx.emit(TuiTerminalSessionEvent::WriteAgentInput {
                    bytes: Cow::Owned(input.to_vec()),
                    mode: *mode,
                });
            }
            // TODO(tui-agent-cancel): wire `CancelExecution` into the terminal
            // manager so an agent-requested command can be interrupted.
            // Ctrl-c conversation cancellation itself is handled by
            // `handle_interrupt`.
            ShellCommandExecutorEvent::CancelExecution
            | ShellCommandExecutorEvent::TransferControlToUser { .. } => {}
        }
    }
}

impl Entity for TuiTerminalSessionView {
    type Event = TuiTerminalSessionEvent;
}

impl TuiView for TuiTerminalSessionView {
    fn ui_name() -> &'static str {
        "TuiTerminalSessionView"
    }

    fn child_view_ids(&self, _ctx: &AppContext) -> Vec<EntityId> {
        vec![self.transcript.id(), self.input_view.id()]
    }

    fn render(&self, ctx: &AppContext) -> Box<dyn TuiElement> {
        // While the PTY is on the alternate screen (vim, less, htop, …), the
        // alt-screen grid replaces the whole transcript surface, full-bleed.
        // The element also consumes all key/mouse input and forwards it to
        // the running program (see `TuiAltScreenElement`).
        if self.terminal_model.lock().is_alt_screen_active() {
            return TuiAltScreenElement::new(self.terminal_model.clone(), self.host_size.clone())
                .finish();
        }

        let inline_menu = self.inline_menu.render(ctx);
        // The border takes the shell-mode accent while in shell mode.
        let builder = TuiUiBuilder::from_app(ctx);
        let border_style = if self.is_shell_mode(ctx) {
            builder.shell_mode_accent_style()
        } else {
            builder.accent_border_style()
        };
        let input_box = TuiConstrainedBox::new(
            TuiContainer::new(TuiChildView::new(&self.input_view).finish())
                .with_padding_x(1)
                .with_border_style(border_style)
                .finish(),
        )
        .with_max_rows(MAX_INPUT_TEXT_ROWS + 2);

        // Ctrl-c (cancel/clear/exit) is handled by the keymap pass via the
        // fixed binding registered in [`Self::init`], so no element-level key
        // handling is needed here.
        //
        // While the transcript has nothing to show, the zero state fills its
        // slot; the first accepted submission produces a visible block, which
        // swaps the transcript back in.
        let mut content = TuiFlex::column();
        if self.transcript.as_ref(ctx).is_empty() {
            content = content.flex_child(render_zero_state(
                self.current_working_directory(ctx).as_deref(),
                ctx,
            ));
        } else {
            content = content.flex_child(TuiChildView::new(&self.transcript).finish());
        }

        // While the selected conversation is in progress (the GUI warping
        // indicator's core condition), the animated warping indicator sits
        // between the transcript and the input box. Its elapsed counter is
        // anchored to the latest exchange's start so animation survives
        // element-tree rebuilds; the conversation's final status update
        // re-renders the view without it.
        let selected_conversation = self
            .conversation_selection
            .as_ref(ctx)
            .selected_conversation_id(ctx)
            .and_then(|conversation_id| {
                BlocklistAIHistoryModel::as_ref(ctx).conversation(&conversation_id)
            });
        if let Some(conversation) = selected_conversation {
            if conversation.status().is_in_progress() {
                let warping_elapsed = conversation
                    .latest_exchange()
                    .and_then(|exchange| exchange.time_since_start());
                if let Some(elapsed) = warping_elapsed {
                    content = content.child(
                        TuiContainer::new(render_warping_indicator(elapsed, ctx))
                            .with_padding_top(1)
                            .finish(),
                    );
                }
            } else {
                // Once the response completes, the indicator's slot rests on
                // the last response's summary: `∷ {duration} • {credits}`.
                // Wall-to-wall duration is only available once the block's
                // final exchange finished, which also keeps the row hidden
                // for brand-new conversations.
                let wall_to_wall = conversation
                    .wall_to_wall_response_time_since_last_query()
                    .and_then(|ms| u64::try_from(ms).ok())
                    .map(Duration::from_millis);
                if let Some(duration) = wall_to_wall {
                    content = content.child(
                        TuiContainer::new(render_response_summary(
                            duration,
                            conversation.credits_spent_for_last_block(),
                            ctx,
                        ))
                        .with_padding_top(1)
                        .finish(),
                    );
                }
            }
        }
        if let Some(menu) = inline_menu {
            content = content.child(
                TuiConstrainedBox::new(menu)
                    .with_max_rows(MAX_INLINE_MENU_ROWS)
                    .finish(),
            );
        }
        content = content.child(input_box.finish()).child(
            TuiConstrainedBox::new(self.render_footer(ctx).finish())
                .with_max_rows(1)
                .finish(),
        );
        // The probe records the host terminal size every layout pass so the
        // alt-screen path can size the PTY correctly the moment it activates.
        TuiHostSizeProbe::new(
            TuiContainer::new(content.finish())
                .with_padding_x(2)
                .with_padding_top(2)
                .finish(),
            self.host_size.clone(),
        )
        .finish()
    }
}

impl TypedActionView for TuiTerminalSessionView {
    type Action = TuiTerminalSessionAction;

    fn handle_action(&mut self, action: &TuiTerminalSessionAction, ctx: &mut ViewContext<Self>) {
        match action {
            TuiTerminalSessionAction::Interrupt => self.handle_interrupt(ctx),
            TuiTerminalSessionAction::ToggleUsageDisplay => self.toggle_usage_display(ctx),
            TuiTerminalSessionAction::WriteAltScreenInput(bytes) => {
                ctx.emit(TuiTerminalSessionEvent::WriteToPty(Cow::Owned(
                    bytes.clone(),
                )));
            }
        }
    }
}

impl TerminalSurface for TuiTerminalSessionView {
    fn on_shell_determined(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.notify();
    }

    fn on_pty_spawn_failed(&mut self, error: anyhow::Error, ctx: &mut ViewContext<Self>) {
        report_error!(error.context("TUI PTY spawn failed"));
        ctx.notify();
    }
}

#[cfg(test)]
#[path = "terminal_session_view_tests.rs"]
mod tests;
