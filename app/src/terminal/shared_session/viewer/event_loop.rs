use std::collections::HashMap;
use std::io::{sink, Sink};
use std::sync::Arc;

use parking_lot::FairMutex;
use session_sharing_protocol::common::{
    OrderedTerminalEvent, OrderedTerminalEventType, Scrollback, WindowSize,
};
use warpui::{Entity, ModelContext, SingletonEntity, WeakViewHandle};

use crate::ai::agent::AIAgentActionId;
use crate::ai::blocklist::block::cli_controller::LongRunningCommandControlState;
use crate::ai::blocklist::history_model::BlocklistAIHistoryModel;
use crate::features::FeatureFlag;
use crate::terminal::event_listener::ChannelEventListener;
use crate::terminal::model::ansi::{self};
use crate::terminal::model::block::AgentInteractionMetadata;
use crate::terminal::shared_session::ai_agent::decode_agent_response_event;
use crate::terminal::shared_session::{decode_scrollback, SharedSessionStatus};
use crate::terminal::{TerminalModel, TerminalView};

/// If we end up buffering more than this many events,
/// this is an indication that we're too far ahead and
/// could indicate an issue.
const TOO_MANY_BUFFERED_EVENTS: usize = 50;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SharedSessionInitialLoadMode {
    /// Replace the viewer's placeholder block list with the scrollback snapshot from the session
    /// being joined.
    ReplaceFromSessionScrollback,
    /// Add only the new blocks from a follow-up session while preserving the existing shared
    /// ambient-agent transcript.
    AppendFollowupScrollback,
}

/// The event loop is used to process a stream of events
/// originating from the sender.
pub struct EventLoop {
    terminal_model: Arc<FairMutex<TerminalModel>>,

    /// We need a reference to the view in the event loop
    /// to ensure that any events which require updating the
    /// view and model happen in lockstep. For example,
    /// resize requires updating the view and model.
    /// If we just dispatched an event, we could potentially
    /// have other [`OrderedTerminalEvent`]s race which would
    /// break the invariant of the event loop.
    #[allow(dead_code)]
    terminal_view: WeakViewHandle<TerminalView>,

    parser: ansi::Processor,

    /// We use a sink as a no-op writer to swallow any writes when the ansi handler needs
    /// to write back to the PTY after reading (e.g. to identify itself).
    /// We assume that the sharer will perform these write-backs.
    sink: Sink,

    channel_event_listener: ChannelEventListener,

    /// The next event number we need from the server.
    next_event_no: usize,

    /// The latest event no of the session the viewer needs to catch up to, at the time of joining.
    catching_up_to_event_no: Option<usize>,

    /// A buffer to maintain events we receive from the server that are unordered.
    buffer: HashMap<usize, OrderedTerminalEventType>,

    should_suppress_existing_agent_conversation_replay: bool,
}

impl EventLoop {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        terminal_model: Arc<FairMutex<TerminalModel>>,
        terminal_view: WeakViewHandle<TerminalView>,
        channel_event_listener: ChannelEventListener,
        window_size: WindowSize,
        scrollback: Scrollback,
        catching_up_to_event_no: Option<usize>,
        load_mode: SharedSessionInitialLoadMode,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        let scrollback_blocks = decode_scrollback(&scrollback);
        let is_alt_screen_active = scrollback.is_alt_screen_active;
        {
            let mut terminal_model = terminal_model.lock();
            match load_mode {
                SharedSessionInitialLoadMode::ReplaceFromSessionScrollback => {
                    terminal_model.load_shared_session_scrollback(scrollback_blocks.as_slice());
                }
                SharedSessionInitialLoadMode::AppendFollowupScrollback => {
                    terminal_model
                        .append_followup_shared_session_scrollback(scrollback_blocks.as_slice());
                }
            }
            if is_alt_screen_active {
                terminal_model.enter_alt_screen(true);
            }
        }

        // When we load scrollback, we might not actually complete a block (e.g. shared session started
        // without any scrollback except active block). In this case, we want to make sure the input
        // is aware of what the latest block ID is.
        if let Some(terminal_view) = terminal_view.upgrade(ctx) {
            terminal_view.update(ctx, |terminal_view, ctx| {
                terminal_view.input().update(ctx, |input, ctx| {
                    input.refresh_deferred_remote_operations(ctx);
                });
            });
        }

        log::debug!(
            "[orch-viewer] EventLoop::new \
             catching_up_to_event_no={catching_up_to_event_no:?} load_mode={load_mode:?}"
        );
        if catching_up_to_event_no.is_none() {
            terminal_model
                .lock()
                .set_shared_session_status(SharedSessionStatus::ActiveViewer {
                    role: Default::default(),
                });
        }

        let mut event_loop = Self {
            terminal_model,
            terminal_view,
            parser: ansi::Processor::new(),
            sink: sink(),
            channel_event_listener,
            // Eventually once we have pagination, the server might need to tell us this.
            next_event_no: 0,
            buffer: HashMap::new(),
            catching_up_to_event_no,
            should_suppress_existing_agent_conversation_replay: matches!(
                load_mode,
                SharedSessionInitialLoadMode::AppendFollowupScrollback
            ),
        };

        // Respect the sharer's window size.
        event_loop.process_resize_event(window_size, ctx);

        event_loop
    }

    fn process_resize_event(&mut self, new_window_size: WindowSize, ctx: &mut ModelContext<Self>) {
        if let Some(view) = self.terminal_view.upgrade(ctx) {
            view.update(ctx, |view, ctx| {
                view.resize_from_sharer_update(new_window_size, ctx);
            });
        }
    }

    /// Returns None if we haven't received any events yet.
    pub fn last_received_event_no(&self) -> Option<usize> {
        if self.next_event_no == 0 {
            return None;
        }
        Some(self.next_event_no - 1)
    }

    pub fn process_ordered_terminal_event(
        &mut self,
        event: OrderedTerminalEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        log::trace!(
            "[orch-viewer] EventLoop received OrderedTerminalEvent \
             event_no={} event_type={:?} (next_event_no={} \
             catching_up_to_event_no={:?} buffer_len_before={})",
            event.event_no,
            event.event_type,
            self.next_event_no,
            self.catching_up_to_event_no,
            self.buffer.len(),
        );
        // Add the event to the buffer.
        self.buffer.insert(event.event_no, event.event_type);

        // If we get too far ahead, let's log a warning for better debugging.
        if self.buffer.len() >= TOO_MANY_BUFFERED_EVENTS {
            log::warn!(
                "Viewer is more than {TOO_MANY_BUFFERED_EVENTS} events ahead of next_event_no"
            );
        }

        // Flush out as many contiguous events as we can.
        while let Some(next_event) = self.buffer.remove(&self.next_event_no) {
            match next_event {
                OrderedTerminalEventType::PtyBytesRead { bytes } => {
                    let mut model = self.terminal_model.lock();
                    let decompressed = lz4_flex::block::decompress_size_prepended(&bytes)
                        .expect("Should be able to decompress the PtyBytesRead event");
                    self.parser
                        .parse_bytes(&mut *model, &decompressed, &mut self.sink);
                }
                OrderedTerminalEventType::CommandExecutionStarted {
                    participant_id,
                    ai_metadata,
                } => {
                    // When a non-agent command starts, clear the loading state and input buffer.
                    // We don't clear for agent commands because the viewer may be typing a follow-up.
                    if ai_metadata.is_none() {
                        if let Some(view) = self.terminal_view.upgrade(ctx) {
                            view.update(ctx, |view, ctx| {
                                view.input().update(ctx, |input, ctx| {
                                    input.unfreeze_and_clear_agent_input(ctx);
                                });
                            });
                        }
                    }

                    // If we have AI metadata, map the tool_call_id back to the owning conversation
                    let reconstructed_ai_metadata = ai_metadata.and_then(|ai_metadata| {
                        let action_id: AIAgentActionId = ai_metadata.tool_call_id.into();

                        // Try to map the action back to its owning conversation.
                        let Some(conversation_id) =
                            self.terminal_view.upgrade(ctx).and_then(|view| {
                                view.read(ctx, |view, app| {
                                    let terminal_view_id = view.id();
                                    let history = BlocklistAIHistoryModel::as_ref(app);

                                    // Try to map the action back to its owning conversation.
                                    history
                                        .conversation_id_for_action(&action_id, terminal_view_id)
                                        // Fallback to active conversation if no exact match is found.
                                        .or_else(|| {
                                            history.active_conversation_id(terminal_view_id)
                                        })
                                })
                            })
                        else {
                            // If we can't find the conversation ID, we can't reconstruct the AI metadata.
                            return None;
                        };

                        Some(AgentInteractionMetadata::new(
                            Some(action_id),
                            conversation_id,
                            None,
                            // If the sharer started this as an agent-monitored long-running command,
                            // reflect that in the viewer's metadata so the command can be rendered as an agent long-running command.
                            // Further state will be inferred from the sharer's agent events.
                            ai_metadata.is_agent_monitored.then_some(
                                LongRunningCommandControlState::Agent {
                                    is_blocked: false,
                                    should_hide_responses: false,
                                },
                            ),
                            false,
                            true,
                        ))
                    });

                    self.terminal_model
                        .lock()
                        .start_command_execution_for_shared_session(
                            participant_id,
                            reconstructed_ai_metadata.clone(),
                        );

                    // Notify the action model that the action is now executing on the sharer's side
                    // This allows the viewer's UI to show the command as running rather than queued
                    // (which is essential for long running commands to be expandable in the UI).
                    if let Some(ai_metadata) = reconstructed_ai_metadata {
                        if let Some(view) = self.terminal_view.upgrade(ctx) {
                            if let Some(action_id) = ai_metadata.requested_command_action_id() {
                                view.update(ctx, |view, ctx| {
                                    view.ai_controller().update(ctx, |controller, ctx| {
                                        controller
                                            .mark_action_as_remotely_executing_in_shared_session(
                                                action_id,
                                                *ai_metadata.conversation_id(),
                                                ctx,
                                            );
                                    });
                                });
                            }
                        }
                    }
                }
                OrderedTerminalEventType::Resize { window_size } => {
                    self.process_resize_event(window_size, ctx)
                }
                OrderedTerminalEventType::CommandExecutionFinished { .. } => (),
                OrderedTerminalEventType::AgentResponseEvent {
                    response_initiator,
                    response_event,
                    forked_from_conversation_token,
                } => {
                    if FeatureFlag::AgentSharedSessions.is_enabled() {
                        match decode_agent_response_event(&response_event) {
                            Ok(resp) => {
                                if let Some(view) = self.terminal_view.upgrade(ctx) {
                                    let event_clone = resp.clone();
                                    let forked_from_token = forked_from_conversation_token.clone();
                                    view.update(ctx, move |view, ctx| {
                                        view.ai_controller().update(ctx, |c, ctx| {
                                            // Set the participant who initiated this response
                                            if let Some(response_initiator) = response_initiator {
                                                c.set_current_response_initiator(
                                                    response_initiator,
                                                );
                                            }

                                            // For forked conversations, update the viewer's conversation
                                            // to use the new server token (only sent once per fork).
                                            if let Some(forked_from) = forked_from_token {
                                                c.link_forked_conversation_token(
                                                    &forked_from,
                                                    &event_clone,
                                                    ctx,
                                                );
                                            }

                                            c.handle_shared_session_response_event(
                                                event_clone.clone(),
                                                ctx,
                                            );
                                        });
                                    });
                                }
                            }
                            Err(err) => {
                                log::warn!("Failed to decode agent response event: {err}");
                            }
                        }
                    }
                }
                OrderedTerminalEventType::AgentConversationReplayStarted => {
                    self.terminal_model
                        .lock()
                        .set_is_receiving_agent_conversation_replay(true);
                    if let Some(view) = self.terminal_view.upgrade(ctx) {
                        let should_suppress_existing_replay =
                            self.should_suppress_existing_agent_conversation_replay;
                        view.update(ctx, |view, ctx| {
                            view.ai_controller().update(ctx, |controller, _| {
                                controller.set_should_suppress_existing_agent_conversation_replay(
                                    should_suppress_existing_replay,
                                );
                            });
                        });
                    }
                }
                OrderedTerminalEventType::AgentConversationReplayEnded => {
                    self.terminal_model
                        .lock()
                        .set_is_receiving_agent_conversation_replay(false);
                    if let Some(view) = self.terminal_view.upgrade(ctx) {
                        view.update(ctx, |view, ctx| {
                            view.ai_controller().update(ctx, |controller, _| {
                                controller
                                    .set_should_suppress_existing_agent_conversation_replay(false);
                            });
                        });
                    }
                }
            }

            self.channel_event_listener.send_wakeup_event();

            self.next_event_no += 1;

            // Catch-up gate: transition ViewPending → ActiveViewer once
            // we've processed enough events to match the server's
            // catch-up target.
            //
            // Empirically the target value `latest_event_no` from
            // `JoinedSuccessfully` is the count of events to consume,
            // not the highest event_no — a server-reported 335 means
            // events 0..334 arrive (335 events). We compare against
            // the *post-increment* `next_event_no` with `>=` so the
            // gate fires once we've handled that many events, and is
            // robust to slight overshoots when buffered live events
            // are drained immediately after.
            //
            // We clear `catching_up_to_event_no` only after a successful
            // transition: if the presence_manager / role isn't ready yet
            // (the share-join adapter hasn't installed it), the next
            // event re-evaluates the gate and retries.
            if self
                .catching_up_to_event_no
                .is_some_and(|target| self.next_event_no >= target)
            {
                let view_handle = self.terminal_view.upgrade(ctx);
                let presence_manager = view_handle.as_ref().and_then(|view| {
                    view.read(ctx, |view, _| view.shared_session_presence_manager())
                });
                // Read role from the presence manager rather than caching
                // it so a role change that lands during catch-up is not
                // clobbered by a stale value.
                let role = presence_manager
                    .as_ref()
                    .and_then(|pm| pm.as_ref(ctx).role());
                if let Some(role) = role {
                    log::info!(
                        "[orch-viewer] EventLoop: catch-up complete \
                         (next_event_no={} target={:?}); transitioning \
                         ViewPending → ActiveViewer role={role:?}",
                        self.next_event_no,
                        self.catching_up_to_event_no,
                    );
                    self.terminal_model
                        .lock()
                        .set_shared_session_status(SharedSessionStatus::ActiveViewer { role });
                    self.catching_up_to_event_no = None;
                } else {
                    log::debug!(
                        "[orch-viewer] EventLoop: catch-up target reached \
                         (next_event_no={} target={:?}) but role not yet \
                         available (view_handle.is_some={} \
                         presence_manager.is_some={}); retrying on next event",
                        self.next_event_no,
                        self.catching_up_to_event_no,
                        view_handle.is_some(),
                        presence_manager.is_some(),
                    );
                }
            }
        }
    }
}

impl Entity for EventLoop {
    type Event = ();
}

#[cfg(test)]
#[path = "event_loop_tests.rs"]
mod tests;
