use std::collections::{HashMap, HashSet};
use std::time::Duration;

use warp_multi_agent_api::client_action::Action;
use warp_multi_agent_api::message::Message;
use warp_multi_agent_api::{self as api};
use warpui::{AppContext, ModelHandle, SingletonEntity};

use super::envelope::{
    EnvelopeSite, envelope_for_reasoning_complete, envelope_for_response_complete,
};
use super::{HookRuntime, owns_turn_lifecycle};
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::blocklist::ResponseStreamId;
use crate::terminal::model::session::active_session::ActiveSession;

/// How one client action changes a message's text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TextUpdate {
    Set(String),
    Append(String),
}

/// The parts of a streamed client action the observers care about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StreamObservation {
    Text {
        message_id: String,
        update: TextUpdate,
    },
    Reasoning {
        message_id: String,
        update: TextUpdate,
        finished_duration: Option<Duration>,
    },
    /// A message of another kind started, which ends any text message before it.
    OtherMessage { message_id: String },
    /// Messages after this point belong to a conversation created by the split.
    ConversationSplit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CompletedKind {
    Text,
    Reasoning { duration: Option<Duration> },
}

/// A message the heuristic decided is final, waiting to be emitted.
#[derive(Debug, Clone, PartialEq, Eq)]
struct CompletedMessage {
    /// Absent when the message arrived after a split whose new conversation id was not known yet.
    conversation_id: Option<AIConversationId>,
    kind: CompletedKind,
    message_id: String,
    text: String,
}

/// The text message currently being streamed.
#[derive(Debug)]
struct StreamingText {
    message_id: String,
    text: String,
}

#[derive(Debug, Default)]
struct StreamState {
    conversation_id: Option<AIConversationId>,
    current_text: Option<StreamingText>,
    /// Reasoning text received so far, by message id.
    partial_reasoning: HashMap<String, String>,
    emitted_text: HashSet<String>,
    emitted_reasoning: HashSet<String>,
}

impl StreamState {
    fn complete_current_text(&mut self, completed: &mut Vec<CompletedMessage>) {
        if let Some(StreamingText { message_id, text }) = self.current_text.take() {
            self.emitted_text.insert(message_id.clone());
            completed.push(CompletedMessage {
                conversation_id: self.conversation_id,
                kind: CompletedKind::Text,
                message_id,
                text,
            });
        }
    }

    fn apply(&mut self, observation: StreamObservation, completed: &mut Vec<CompletedMessage>) {
        match observation {
            StreamObservation::Text { message_id, update } => {
                // A late update to a message that was already treated as final is not a new
                // message, so it must not end the one currently streaming.
                if self.emitted_text.contains(&message_id) {
                    return;
                }
                if self
                    .current_text
                    .as_ref()
                    .is_some_and(|current| current.message_id != message_id)
                {
                    self.complete_current_text(completed);
                }
                let text = match (self.current_text.take(), update) {
                    (Some(current), TextUpdate::Append(delta)) => current.text + &delta,
                    (None, TextUpdate::Append(delta)) => delta,
                    (_, TextUpdate::Set(text)) => text,
                };
                self.current_text = Some(StreamingText { message_id, text });
            }
            StreamObservation::Reasoning {
                message_id,
                update,
                finished_duration,
            } => {
                if self.emitted_reasoning.contains(&message_id) {
                    return;
                }
                self.complete_current_text(completed);
                let entry = self
                    .partial_reasoning
                    .entry(message_id.clone())
                    .or_default();
                match update {
                    TextUpdate::Set(text) => *entry = text,
                    TextUpdate::Append(delta) => entry.push_str(&delta),
                }
                if let Some(duration) = finished_duration {
                    let text = self
                        .partial_reasoning
                        .remove(&message_id)
                        .unwrap_or_default();
                    self.emitted_reasoning.insert(message_id.clone());
                    completed.push(CompletedMessage {
                        conversation_id: self.conversation_id,
                        kind: CompletedKind::Reasoning {
                            duration: Some(duration),
                        },
                        message_id,
                        text,
                    });
                }
            }
            StreamObservation::OtherMessage { message_id } => {
                if self
                    .current_text
                    .as_ref()
                    .is_some_and(|current| current.message_id != message_id)
                {
                    self.complete_current_text(completed);
                }
            }
            StreamObservation::ConversationSplit => {
                self.complete_current_text(completed);
                self.conversation_id = None;
            }
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct StreamObserverState {
    streams: HashMap<ResponseStreamId, StreamState>,
}

impl StreamObserverState {
    fn apply_batch(
        &mut self,
        stream_id: &ResponseStreamId,
        conversation_before: AIConversationId,
        conversation_after: Option<AIConversationId>,
        observations: Vec<StreamObservation>,
    ) -> Vec<CompletedMessage> {
        let state = self.streams.entry(stream_id.clone()).or_default();
        if state.conversation_id.is_none() {
            state.conversation_id = Some(conversation_before);
        }
        let mut completed = Vec::new();
        for observation in observations {
            state.apply(observation, &mut completed);
        }
        if state.conversation_id.is_none() {
            state.conversation_id = conversation_after;
        }
        for message in &mut completed {
            if message.conversation_id.is_none() {
                message.conversation_id = conversation_after;
            }
        }
        completed
    }

    fn finish(&mut self, stream_id: &ResponseStreamId, cancelled: bool) -> Vec<CompletedMessage> {
        let Some(mut state) = self.streams.remove(stream_id) else {
            return Vec::new();
        };
        let mut completed = Vec::new();
        if !cancelled {
            state.complete_current_text(&mut completed);
        }
        completed
    }
}

fn proto_duration(duration: &prost_types::Duration) -> Option<Duration> {
    let seconds = u64::try_from(duration.seconds).ok()?;
    let nanos = u32::try_from(duration.nanos).ok()?;
    Some(Duration::new(seconds, nanos))
}

/// How a client action delivered a message.
#[derive(Debug, Clone, Copy)]
enum MessageDelivery {
    /// A message the stream had not shown before.
    Added,
    /// A full replacement of a message already shown.
    Replaced,
    /// Text appended to a message already shown.
    Appended,
}

fn observe_message(
    message: &api::Message,
    delivery: MessageDelivery,
    out: &mut Vec<StreamObservation>,
) {
    let update = |text: &str| match delivery {
        MessageDelivery::Added | MessageDelivery::Replaced => TextUpdate::Set(text.to_owned()),
        MessageDelivery::Appended => TextUpdate::Append(text.to_owned()),
    };
    match &message.message {
        Some(Message::AgentOutput(output)) => out.push(StreamObservation::Text {
            message_id: message.id.clone(),
            update: update(&output.text),
        }),
        Some(Message::AgentReasoning(reasoning)) => out.push(StreamObservation::Reasoning {
            message_id: message.id.clone(),
            update: update(&reasoning.reasoning),
            finished_duration: reasoning
                .finished_duration
                .as_ref()
                .and_then(proto_duration),
        }),
        // Only a message that is new to the stream ends the text before it. An update to an
        // earlier tool call, for example, says nothing about whether the text is finished.
        Some(_) => match delivery {
            MessageDelivery::Added => out.push(StreamObservation::OtherMessage {
                message_id: message.id.clone(),
            }),
            MessageDelivery::Replaced | MessageDelivery::Appended => {}
        },
        None => {}
    }
}

/// Reduces a batch of client actions to what the observers need. Empty when hooks are inactive or
/// the conversation is not driven by this client, so the caller copies nothing on the common path.
pub(crate) fn collect_observations(
    actions: &[api::ClientAction],
    conversation_id: AIConversationId,
    ctx: &AppContext,
) -> Vec<StreamObservation> {
    if !HookRuntime::is_active(ctx) || !owns_turn_lifecycle(conversation_id, ctx) {
        return Vec::new();
    }
    let mut out = Vec::new();
    for action in actions {
        match &action.action {
            Some(Action::AddMessagesToTask(add)) => {
                for message in &add.messages {
                    observe_message(message, MessageDelivery::Added, &mut out);
                }
            }
            Some(Action::UpdateTaskMessage(update)) => {
                if let Some(message) = &update.message {
                    observe_message(message, MessageDelivery::Replaced, &mut out);
                }
            }
            Some(Action::AppendToMessageContent(append)) => {
                if let Some(message) = &append.message {
                    observe_message(message, MessageDelivery::Appended, &mut out);
                }
            }
            Some(Action::StartNewConversation(_)) => out.push(StreamObservation::ConversationSplit),
            Some(
                Action::CreateTask(_)
                | Action::ShowSuggestions(_)
                | Action::UpdateTaskSummary(_)
                | Action::UpdateTaskDescription(_)
                | Action::BeginTransaction(_)
                | Action::CommitTransaction(_)
                | Action::RollbackTransaction(_)
                | Action::UpdateTaskServerData(_)
                | Action::MoveMessagesToNewTask(_),
            )
            | None => {}
        }
    }
    out
}

/// Applies one batch of observations for `stream_id` and fires `response.complete` and
/// `reasoning.complete` for every message the batch finished. `conversation_after` is the
/// conversation the stream maps to once the batch has been applied, which differs from
/// `conversation_before` when the batch split the conversation.
pub(crate) fn observe(
    stream_id: &ResponseStreamId,
    conversation_before: AIConversationId,
    conversation_after: Option<AIConversationId>,
    observations: Vec<StreamObservation>,
    active_session: &ModelHandle<ActiveSession>,
    ctx: &mut AppContext,
) {
    if observations.is_empty() || !HookRuntime::is_active(ctx) {
        return;
    }
    let completed = HookRuntime::handle(ctx).update(ctx, |runtime, _| {
        runtime.stream_observer.apply_batch(
            stream_id,
            conversation_before,
            conversation_after,
            observations,
        )
    });
    emit(completed, active_session, ctx);
}

/// Ends tracking for `stream_id`. A text message still streaming is treated as final unless the
/// stream was cancelled, in which case it was never complete.
pub(crate) fn flush_on_stream_end(
    stream_id: &ResponseStreamId,
    cancelled: bool,
    active_session: &ModelHandle<ActiveSession>,
    ctx: &mut AppContext,
) {
    if !HookRuntime::is_active(ctx) {
        return;
    }
    let completed = HookRuntime::handle(ctx).update(ctx, |runtime, _| {
        runtime.stream_observer.finish(stream_id, cancelled)
    });
    emit(completed, active_session, ctx);
}

fn emit(
    completed: Vec<CompletedMessage>,
    active_session: &ModelHandle<ActiveSession>,
    ctx: &mut AppContext,
) {
    for message in completed {
        let Some(conversation_id) = message.conversation_id else {
            continue;
        };
        let site = EnvelopeSite::new(conversation_id, active_session);
        let envelope = match message.kind {
            CompletedKind::Text => {
                envelope_for_response_complete(message.message_id, message.text, &site, ctx)
            }
            CompletedKind::Reasoning { duration } => envelope_for_reasoning_complete(
                message.message_id,
                message.text,
                duration,
                &site,
                ctx,
            ),
        };
        HookRuntime::as_ref(ctx).dispatch_detached(envelope, ctx);
    }
}

#[cfg(test)]
#[path = "stream_tests.rs"]
mod tests;
