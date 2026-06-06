use warpui::{EntityId, ModelContext, ModelHandle, SingletonEntity};

use super::{CLIAgentEvent, CLIAgentSessionsModel};
use crate::features::FeatureFlag;
use crate::terminal::cli_agent_sessions::event::{
    parse_event, CLIAgentEventPayload, CLIAgentEventSource, CLIAgentEventType,
};
use crate::terminal::model_events::{ModelEvent, ModelEventDispatcher};
use crate::terminal::CLIAgent;

/// Per-agent handler that filters and transforms parsed CLI agent events.
/// Each CLI agent can have a different implementation depending on which events
/// it cares about.
trait CLIAgentSessionHandler {
    /// Attempt to parse a raw `PluggableNotification` into a typed event.
    /// The default implementation delegates to the structured JSON parser
    /// (`parse_event`); agents with non-JSON notification formats (e.g. Codex
    /// OSC 9 plain text) should override this.
    ///
    /// `plugin_already_active` is true when the session has already received a
    /// structured OSC 777 notification; Codex uses it to drop OSC 9 fallback
    /// once the rich plugin is active. Other handlers ignore it.
    fn try_parse(
        &mut self,
        title: Option<&str>,
        body: &str,
        plugin_already_active: bool,
    ) -> Option<CLIAgentEvent> {
        let _ = plugin_already_active;
        parse_event(title, body)
    }

    /// Decide whether a parsed event should be forwarded to the sessions model.
    /// Returns the event (possibly transformed) if it should be processed.
    fn handle_event(&mut self, event: CLIAgentEvent) -> Option<CLIAgentEvent>;
}

/// Returns `true` if the given CLI agent has a supported session handler.
pub fn is_agent_supported(agent: &CLIAgent) -> bool {
    matches!(
        agent,
        CLIAgent::Claude
            | CLIAgent::OpenCode
            | CLIAgent::Codex
            | CLIAgent::Gemini
            | CLIAgent::Auggie
            | CLIAgent::Pi
    )
}

/// Creates the appropriate handler for the given CLI agent.
fn create_handler(agent: &CLIAgent) -> Option<Box<dyn CLIAgentSessionHandler>> {
    match agent {
        // Auggie and Pi are supported via community-maintained plugins
        // (https://github.com/augmentmoogi/auggie-warp,
        // https://github.com/badlogic/pi-mono), which emit the same
        // structured OSC 777 events as the first-party Claude/OpenCode/Gemini
        // plugins. We don't ship install flows for them — we just listen.
        CLIAgent::Claude
        | CLIAgent::OpenCode
        | CLIAgent::Gemini
        | CLIAgent::Auggie
        | CLIAgent::Pi => Some(Box::new(DefaultSessionListener)),
        CLIAgent::Codex => Some(Box::new(CodexSessionHandler)),
        CLIAgent::Hermes
        | CLIAgent::Amp
        | CLIAgent::Droid
        | CLIAgent::Copilot
        | CLIAgent::CursorCli
        | CLIAgent::Goose
        | CLIAgent::Vibe
        | CLIAgent::Unknown => None,
    }
}

/// Default handler shared by agents whose events need no special filtering
/// beyond skipping the initial `SessionStart`.
struct DefaultSessionListener;

impl CLIAgentSessionHandler for DefaultSessionListener {
    fn handle_event(&mut self, event: CLIAgentEvent) -> Option<CLIAgentEvent> {
        // Skip session_start events (handled during listener construction)
        if event.event == CLIAgentEventType::SessionStart {
            return None;
        }

        Some(event)
    }
}

/// Codex-specific handler that supports both native OSC 9 fallback and structured plugin events.
///
/// Codex sends notifications via OSC 9 (`\x1b]9;message\x07`) with
/// human-readable text. Since there's no way to distinguish notification types from the raw text,
/// OSC 9 fallback notifications are treated as `Stop` (success).
struct CodexSessionHandler;

impl CodexSessionHandler {
    /// Parse a plain-text OSC 9 notification body into a `CLIAgentEvent`.
    /// Returns `None` only for empty bodies.
    fn parse_osc9_text(body: &str) -> Option<CLIAgentEvent> {
        let body = body.trim();
        if body.is_empty() {
            return None;
        }

        Some(CLIAgentEvent {
            v: 1,
            agent: CLIAgent::Codex,
            event: CLIAgentEventType::Stop,
            session_id: None,
            cwd: None,
            project: None,
            payload: CLIAgentEventPayload {
                query: Some(body.to_owned()),
                ..Default::default()
            },
            source: CLIAgentEventSource::CodexOsc9Fallback,
        })
    }
}

impl CLIAgentSessionHandler for CodexSessionHandler {
    /// Before Codex enabled support for hooks, we relied on OSC 9 to trigger notifications in Warp.
    /// Here, we try to parse an OSC 777 event if we can, and remember when we've seen one.
    /// This lets us ignore OSC 9 notifications if we are working with a client that is using
    /// the new plugin, but keeps them intact for legacy clients.
    fn try_parse(
        &mut self,
        title: Option<&str>,
        body: &str,
        plugin_already_active: bool,
    ) -> Option<CLIAgentEvent> {
        if let Some(event) = parse_event(title, body) {
            if event.agent == CLIAgent::Codex {
                if !FeatureFlag::CodexPlugin.is_enabled() {
                    return None;
                }
                return Some(event);
            }
            return None;
        }
        // OSC 9 notifications have no title. Skip OSC 9 once the rich plugin is
        // active, otherwise we'd process both OSC 777 and OSC 9 notifications.
        if title.is_some() || plugin_already_active {
            return None;
        }
        Self::parse_osc9_text(body)
    }

    fn handle_event(&mut self, event: CLIAgentEvent) -> Option<CLIAgentEvent> {
        Some(event)
    }
}

/// Per-agent listener that subscribes to PTY events and forwards them to the
/// sessions model. Stored on [`super::CLIAgentSession`] so its lifetime is
/// tied to the session; dropping the handle cleans up the subscription.
pub struct CLIAgentSessionListener {
    terminal_view_id: EntityId,
    inner: Box<dyn CLIAgentSessionHandler>,
}

impl warpui::Entity for CLIAgentSessionListener {
    type Event = ();
}

impl CLIAgentSessionListener {
    pub fn new(
        terminal_view_id: EntityId,
        agent: CLIAgent,
        model_event_dispatcher: &ModelHandle<ModelEventDispatcher>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        let handler =
            create_handler(&agent).expect("is_agent_supported must be checked before calling new");

        // Subscribe to subsequent OSC events from this terminal's PTY.
        // Parsing is delegated to the handler's `try_parse`; the handler's
        // `handle_event` then filters/transforms the result.
        ctx.subscribe_to_model(model_event_dispatcher, move |me, event, ctx| {
            if let ModelEvent::PluggableNotification { title, body } = event {
                let view_id = me.terminal_view_id;
                let plugin_already_active = CLIAgentSessionsModel::as_ref(ctx)
                    .session(view_id)
                    .is_some_and(|session| session.received_rich_notification);
                let Some(parsed) =
                    me.inner
                        .try_parse(title.as_deref(), body, plugin_already_active)
                else {
                    return;
                };
                if let Some(event) = me.inner.handle_event(parsed) {
                    CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions_model, ctx| {
                        sessions_model.update_from_event(view_id, &event, ctx);
                    });
                }
            }
        });

        Self {
            terminal_view_id,
            inner: handler,
        }
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
