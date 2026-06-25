//! One-shot TUI prompt streaming to stdout.

use std::io::{self, Write};

use anyhow::anyhow;
use pathfinder_geometry::vector::Vector2F;
use warp::tui_export::{
    AIAgentTextSection, AIConversationId, ActiveSession, BannerState, BlocklistAIActionModel,
    BlocklistAIContextModel, BlocklistAIController, BlocklistAIHistoryModel, BlocklistAIInputModel,
    ConversationSelection, ConversationStatus, ConversationStatusUpdate,
    GetRelevantFilesController, IsSharedSessionCreator, LocalTtyTerminalManager, PtyIntent,
    PtyIntentEvent, ServerConversationToken, ShellLaunchData, TerminalManagerTrait,
    TerminalSurface, TerminalSurfaceInit, TerminalSurfaceResult,
};
#[cfg(unix)]
use warp::tui_export::{AfterBlockCompletedEvent, BlockIndex};
use warpui::elements::Empty;
use warpui::platform::{TerminationMode, WindowStyle};
use warpui::{
    AddWindowOptions, AppContext, Element, Entity, EntityId, ModelHandle, SingletonEntity,
    TypedActionView, View, ViewContext, ViewHandle,
};

use super::args::TuiArgs;
use super::conversation_model::{TuiConversationModel, TuiConversationModelEvent};
use super::conversation_selection::TuiConversationSelection;

struct PromptStreamHostView;

impl Entity for PromptStreamHostView {
    type Event = ();
}

impl View for PromptStreamHostView {
    fn ui_name() -> &'static str {
        "PromptStreamHostView"
    }

    fn render(&self, _app: &AppContext) -> Box<dyn Element> {
        Empty::new().finish()
    }
}

impl TypedActionView for PromptStreamHostView {
    type Action = ();
}

struct PromptStreamSurface {
    conversation_model: ModelHandle<TuiConversationModel>,
    last_output: String,
    is_terminating: bool,
}
/// Event type for the prompt-stream terminal surface.
struct PromptStreamEvent;

/// Writes only the newly appended portion of a stream snapshot.
fn write_stream_snapshot_delta<W: Write>(
    last_output: &mut String,
    text: &str,
    output: &mut W,
) -> io::Result<()> {
    if text == last_output {
        return Ok(());
    }

    let delta = text.strip_prefix(last_output.as_str()).unwrap_or(text);
    output.write_all(delta.as_bytes())?;
    output.flush()?;
    last_output.clear();
    last_output.push_str(text);
    Ok(())
}

impl PromptStreamSurface {
    /// Builds the conversation-capable surface from manager-provided terminal session handles.
    fn new(surface_init: TerminalSurfaceInit, ctx: &mut ViewContext<Self>) -> Self {
        let TerminalSurfaceInit {
            model,
            sessions,
            model_events,
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
        let input_model = ctx.add_model(|ctx| {
            BlocklistAIInputModel::new(
                model.clone(),
                conversation_selection.clone(),
                context_model.clone(),
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
                input_model,
                context_model.clone(),
                conversation_selection.clone(),
                action_model,
                active_session,
                model,
                terminal_surface_id,
                ctx,
            )
        });
        let conversation_model = ctx.add_model(|ctx| {
            TuiConversationModel::new(
                terminal_surface_id,
                conversation_selection,
                ai_controller,
                ctx,
            )
        });
        ctx.subscribe_to_model(&conversation_model, |surface, _, event, ctx| {
            surface.handle_conversation_event(event, ctx)
        });
        Self {
            conversation_model,
            last_output: String::new(),
            is_terminating: false,
        }
    }

    /// Prints the server conversation token and final status.
    fn print_final_status(
        &self,
        conversation_id: AIConversationId,
        status: &ConversationStatus,
        ctx: &AppContext,
    ) {
        if !self.last_output.is_empty() && !self.last_output.ends_with('\n') {
            println!();
        }
        if let Some(server_conversation_token) = BlocklistAIHistoryModel::as_ref(ctx)
            .conversation(&conversation_id)
            .and_then(|conversation| conversation.server_conversation_token())
        {
            println!("conversation_id={}", server_conversation_token.as_str());
        }
        println!("status={status:?}");
    }

    /// Submits a new prompt or restores and follows up in an existing conversation.
    fn submit_prompt(
        &mut self,
        prompt: String,
        server_conversation_token: Option<ServerConversationToken>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.conversation_model.update(ctx, |model, ctx| {
            if let Some(server_conversation_token) = server_conversation_token {
                model.restore_conversation_by_server_token_and_send_prompt(
                    prompt,
                    server_conversation_token,
                    ctx,
                );
            } else {
                model.send_prompt(prompt, ctx);
            }
        });
    }

    /// Adapts production model events to the one-shot stdout output.
    fn handle_conversation_event(
        &mut self,
        event: &TuiConversationModelEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        if self.is_terminating {
            return;
        }
        match event {
            TuiConversationModelEvent::SelectedConversationChanged { conversation_id } => {
                let _ = conversation_id;
                self.last_output.clear();
            }
            TuiConversationModelEvent::ConversationStarted { conversation_id } => {
                let _ = conversation_id;
            }
            TuiConversationModelEvent::ConversationUpdated { conversation_id } => {
                self.print_stream_snapshot(*conversation_id, ctx);
            }
            TuiConversationModelEvent::ConversationStatusChanged {
                conversation_id,
                status,
                update: ConversationStatusUpdate::Changed { .. },
            } => {
                self.print_stream_snapshot(*conversation_id, ctx);
                if self.is_terminating {
                    return;
                }
                match status {
                    ConversationStatus::InProgress
                    | ConversationStatus::TransientError
                    | ConversationStatus::WaitingForEvents => {}
                    ConversationStatus::Success => {
                        self.print_final_status(*conversation_id, status, ctx);
                        self.is_terminating = true;
                        ctx.terminate_app(TerminationMode::ForceTerminate, None);
                    }
                    ConversationStatus::Error
                    | ConversationStatus::Cancelled
                    | ConversationStatus::Blocked { .. } => {
                        self.print_final_status(*conversation_id, status, ctx);
                        self.terminate_with_error(
                            anyhow!("TUI prompt streaming ended with status {status:?}"),
                            ctx,
                        );
                    }
                }
            }
            TuiConversationModelEvent::ConversationStatusChanged {
                update: ConversationStatusUpdate::Restored,
                ..
            } => {}
            TuiConversationModelEvent::Error { message } => {
                self.terminate_with_error(anyhow!("{message}"), ctx);
            }
        }
    }
    /// Prints the latest plain-text output when it changes.
    fn print_stream_snapshot(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some((has_actions, text)) = (|| {
            let exchange = BlocklistAIHistoryModel::as_ref(ctx)
                .conversation(&conversation_id)
                .and_then(|conversation| conversation.latest_exchange())?;
            let output = exchange.output_status.output()?;
            let output = output.get();
            let has_actions = output.actions().next().is_some();
            let text = output
                .text_from_agent_output()
                .flat_map(|text| text.sections.iter())
                .filter_map(|section| match section {
                    AIAgentTextSection::PlainText { text } => Some(text.text()),
                    AIAgentTextSection::Code { .. }
                    | AIAgentTextSection::Table { .. }
                    | AIAgentTextSection::Image { .. }
                    | AIAgentTextSection::MermaidDiagram { .. } => None,
                })
                .collect::<Vec<_>>()
                .join("\n");
            Some((has_actions, text))
        })() else {
            return;
        };
        if has_actions {
            self.terminate_with_error(
                anyhow!("TUI prompt streaming does not support tool actions"),
                ctx,
            );
            return;
        }
        let mut stdout = io::stdout().lock();
        if let Err(error) = write_stream_snapshot_delta(&mut self.last_output, &text, &mut stdout) {
            self.terminate_with_error(anyhow!("Failed to write TUI stream output: {error}"), ctx);
        }
    }

    /// Terminates prompt streaming with a user-visible error.
    fn terminate_with_error(&mut self, error: anyhow::Error, ctx: &mut ViewContext<Self>) {
        if self.is_terminating {
            return;
        }
        self.is_terminating = true;
        ctx.terminate_app(TerminationMode::ForceTerminate, Some(Err(error)));
    }
}

impl Entity for PromptStreamSurface {
    type Event = PromptStreamEvent;
}

impl PtyIntentEvent for PromptStreamEvent {
    fn pty_intent(&self) -> Option<PtyIntent> {
        None
    }
}

impl TerminalSurface for PromptStreamSurface {
    #[cfg(unix)]
    fn should_start_password_prompt_polling(&self, _command: &str, _ctx: &AppContext) -> bool {
        false
    }

    #[cfg(unix)]
    fn should_stop_password_prompt_polling(&self, _completed: &AfterBlockCompletedEvent) -> bool {
        false
    }

    fn on_shell_determined(&mut self, _ctx: &mut ViewContext<Self>) {}

    fn on_active_shell_launch_data_updated(
        &mut self,
        _shell_launch_data: Option<ShellLaunchData>,
        _ctx: &mut ViewContext<Self>,
    ) {
    }

    fn on_pty_spawn_failed(&mut self, error: anyhow::Error, _ctx: &mut ViewContext<Self>) {
        log::warn!("PTY spawn failed for the TUI prompt-streaming surface: {error:#}");
    }

    #[cfg(unix)]
    fn on_possible_password_prompt(
        &mut self,
        _block_index: Option<BlockIndex>,
        _ctx: &mut ViewContext<Self>,
    ) {
    }

    #[cfg(unix)]
    fn on_polled_block_completed(
        &mut self,
        _completed: &AfterBlockCompletedEvent,
        _ctx: &mut ViewContext<Self>,
    ) {
    }
}

impl View for PromptStreamSurface {
    fn ui_name() -> &'static str {
        "PromptStreamSurface"
    }

    fn render(&self, _app: &AppContext) -> Box<dyn Element> {
        Empty::new().finish()
    }
}

impl TypedActionView for PromptStreamSurface {
    type Action = ();
}

struct PromptStreamSession {
    _manager: ModelHandle<Box<dyn TerminalManagerTrait>>,
    _surface: ViewHandle<PromptStreamSurface>,
}

impl Entity for PromptStreamSession {
    type Event = ();
}

impl SingletonEntity for PromptStreamSession {}
/// Starts prompt streaming when the TUI frontend received a prompt.
pub(super) fn start(args: TuiArgs, ctx: &mut AppContext) -> bool {
    let Some(prompt) = args.prompt else {
        return false;
    };
    start_prompt_stream(prompt, args.server_conversation_token, ctx);
    true
}

/// Builds a manager-backed terminal session and submits the prompt.
fn start_prompt_stream(
    prompt: String,
    server_conversation_token: Option<ServerConversationToken>,
    ctx: &mut AppContext,
) {
    let (window_id, _) = ctx.add_window(
        AddWindowOptions {
            window_style: WindowStyle::NotStealFocus,
            ..Default::default()
        },
        |_ctx| PromptStreamHostView,
    );
    let banner = ctx.add_model(|_| BannerState::default());
    let terminal_manager = LocalTtyTerminalManager::<PromptStreamSurface>::create_tui_model(
        std::env::current_dir().ok(),
        std::env::vars_os().collect(),
        IsSharedSessionCreator::No,
        None,
        banner,
        Vector2F::new(120., 24.),
        None,
        None,
        ctx,
        move |surface_init, ctx| {
            let surface = ctx.add_typed_action_view(window_id, |ctx| {
                PromptStreamSurface::new(surface_init, ctx)
            });
            TerminalSurfaceResult {
                surface,
                post_wire: |_manager: &mut LocalTtyTerminalManager<PromptStreamSurface>,
                            _surface: &ViewHandle<PromptStreamSurface>,
                            _ctx: &mut AppContext| {},
            }
        },
    );
    let manager = terminal_manager.manager;
    let surface = terminal_manager.surface;
    surface.update(ctx, |surface, ctx| {
        surface.submit_prompt(prompt, server_conversation_token, ctx);
    });
    ctx.add_singleton_model(|_| PromptStreamSession {
        _manager: manager,
        _surface: surface,
    });
}

#[cfg(test)]
#[path = "prompt_stream_tests.rs"]
mod tests;
