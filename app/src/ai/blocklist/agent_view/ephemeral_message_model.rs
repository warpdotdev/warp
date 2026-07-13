use std::borrow::Cow;
use std::time::Duration;

use warpui::r#async::SpawnedFutureHandle;
use warpui::{Entity, ModelContext};

use super::agent_message_bar::AgentMessageArgs;
use crate::terminal::input::message_bar::{Message, MessageProvider};

const DEFAULT_MESSAGE_DURATION: Duration = Duration::from_millis(1500);

pub struct EphemeralMessage {
    /// Optional id that may be used to identify the message.
    id: Option<Cow<'static, str>>,

    /// The message to be displayed.
    message: Message,

    /// The strategy to be used to determine when to stop showing the message.
    dismissal: DismissalStrategy,
}

#[derive(Clone, Copy)]
pub enum DismissalStrategy {
    /// Persists until explicitly dismissed by the input.
    UntilExplicitlyDismissed,

    /// Auto-dismiss after a duration elapses.
    Timer(Duration),
}

impl EphemeralMessage {
    pub fn new(message: Message, dismissal: DismissalStrategy) -> Self {
        Self {
            id: None,
            message,
            dismissal,
        }
    }

    pub fn with_id(mut self, id: impl Into<Cow<'static, str>>) -> Self {
        self.id = Some(id.into());
        self
    }

    pub fn with_duration(mut self, duration: Duration) -> Self {
        self.dismissal = DismissalStrategy::Timer(duration);
        self
    }

    pub fn id(&self) -> Option<&str> {
        self.id.as_ref().map(|id| id.as_ref())
    }
}

/// Manages messages that are dismissed either explicitly by the input or after a fixed duration.
#[derive(Default)]
pub struct EphemeralMessageModel {
    current_message: Option<EphemeralMessage>,
    clear_timer_handle: Option<SpawnedFutureHandle>,
}

#[derive(Debug, Clone, Copy)]
pub enum EphemeralMessageModelEvent {
    MessageChanged,
}

impl EphemeralMessageModel {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn current_message(&self) -> Option<&EphemeralMessage> {
        self.current_message.as_ref()
    }
    /// Shows a message with the given dismissal strategy.
    pub fn show_ephemeral_message(
        &mut self,
        message: EphemeralMessage,
        ctx: &mut ModelContext<Self>,
    ) {
        if let Some(handle) = self.clear_timer_handle.take() {
            handle.abort();
        }

        let dismissal = message.dismissal;
        self.current_message = Some(message);

        // If we are dismissing via timer, start the timer.
        if let DismissalStrategy::Timer(duration) = dismissal {
            let abort_handle = ctx.spawn_abortable(
                async move { warpui::r#async::Timer::after(duration).await },
                |me, _, ctx| {
                    me.current_message = None;
                    me.clear_timer_handle = None;
                    ctx.emit(EphemeralMessageModelEvent::MessageChanged);
                },
                |_, _| (),
            );
            self.clear_timer_handle = Some(abort_handle);
        }

        ctx.emit(EphemeralMessageModelEvent::MessageChanged);
    }

    pub fn show_info_ephemeral_message(
        &mut self,
        message: impl Into<Cow<'static, str>>,
        ctx: &mut ModelContext<Self>,
    ) {
        self.show_ephemeral_message(
            EphemeralMessage::new(
                Message::from_text(message),
                DismissalStrategy::Timer(DEFAULT_MESSAGE_DURATION),
            ),
            ctx,
        );
    }

    /// Dismisses the current message if it is not timer-based.
    pub fn try_dismiss_explicit_message(&mut self, ctx: &mut ModelContext<Self>) {
        let should_dismiss = self
            .current_message
            .as_ref()
            .is_some_and(|m| matches!(m.dismissal, DismissalStrategy::UntilExplicitlyDismissed));
        if should_dismiss {
            self.current_message = None;
            ctx.emit(EphemeralMessageModelEvent::MessageChanged);
        }
    }

    /// Unconditionally clears the current message and cancels any active timer.
    pub fn clear_message(&mut self, ctx: &mut ModelContext<Self>) {
        if let Some(handle) = self.clear_timer_handle.take() {
            handle.abort();
        }

        if self.current_message.take().is_some() {
            ctx.emit(EphemeralMessageModelEvent::MessageChanged);
        }
    }
}

impl Entity for EphemeralMessageModel {
    type Event = EphemeralMessageModelEvent;
}

impl MessageProvider<AgentMessageArgs<'_>> for EphemeralMessageModel {
    fn produce_message(&self, _args: AgentMessageArgs<'_>) -> Option<Message> {
        self.current_message()
            .map(|ephemeral_message| ephemeral_message.message.clone())
    }
}
