use std::time::Duration;

use warpui::r#async::SpawnedFutureHandle;
use warpui::{Entity, ModelContext, ModelHandle};

use super::shell::ShellType;
use super::{ModelEvent, ModelEventDispatcher};
use crate::terminal::model::session::Sessions;
use crate::terminal::model_events::AnsiHandlerEvent;

/// The duration after a precmd/end prompt hook (depending on the shell type) to wait before
/// assuming the shell's line editor is active again. If we receive a preexec hook in that time,
/// we assume there are multiple queued typeahead commands and wait to request input.
/// This prevents Warp sending an escape sequence to an arbitrary running program.
///
/// To avoid flickering, this must be less than `BACKGROUND_OUTPUT_RENDER_DELAY_MS`,
/// which is the delay before a background block is rendered.
const LINE_EDITOR_ACTIVATION_DELAY: Duration = Duration::from_millis(50);

/// Model representing the status of the shell's line editor.
///
/// This model also emits events when the line editor becomes active/inactive.
pub struct LineEditorStatus {
    model_event_dispatcher: ModelHandle<ModelEventDispatcher>,
    is_line_editor_active: bool,
    mark_line_editor_active_abort_handle: Option<SpawnedFutureHandle>,
    sessions: ModelHandle<Sessions>,

    /// `true` if the active session is zsh and precmd was received before the most recent end
    /// prompt marker.
    ///
    /// When receiving an end prompt marker in zsh, this is used as a proxy to determine if the
    /// session is bootstrapped -- the prompt markers are emitted by zsh regardless of whether or
    /// not its a Warpified session, so to in order properly signal downstream that the line editor
    /// (for Warpified sessions) is active, we must check if there was a corresponding precmd
    /// emitted prior to the end prompt marker.
    ///
    /// Precmd is always emitted before prompt markers.
    did_receive_zsh_precmd: bool,
}

impl LineEditorStatus {
    pub fn new(
        model_event_dispatcher: ModelHandle<ModelEventDispatcher>,
        sessions: ModelHandle<Sessions>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        ctx.subscribe_to_model(&model_event_dispatcher, |me, _, event, ctx| {
            me.handle_model_event(event, ctx);
        });
        LineEditorStatus {
            model_event_dispatcher,
            is_line_editor_active: false,
            mark_line_editor_active_abort_handle: None,
            did_receive_zsh_precmd: false,
            sessions,
        }
    }

    pub fn is_line_editor_active(&self) -> bool {
        self.is_line_editor_active
    }

    /// Marks the line editor as inactive.
    ///
    /// This is meant to be called when a command is written to the PTY.
    pub(super) fn did_execute_command(&mut self, ctx: &mut ModelContext<Self>) {
        self.set_line_editor_inactive(ctx);
    }

    fn handle_model_event(&mut self, event: &ModelEvent, ctx: &mut ModelContext<Self>) {
        let Some(active_session_id) = self.model_event_dispatcher.as_ref(ctx).active_session_id()
        else {
            return;
        };

        let Some(active_session) = self.sessions.as_ref(ctx).get(active_session_id) else {
            return;
        };

        let is_active_session_zsh = active_session.shell().shell_type() == ShellType::Zsh;
        match event {
            ModelEvent::Handler(AnsiHandlerEvent::Precmd) => {
                if is_active_session_zsh {
                    self.did_receive_zsh_precmd = true;
                } else if !self.is_line_editor_active {
                    self.mark_line_editor_active_after_delay(LINE_EDITOR_ACTIVATION_DELAY, ctx);
                    self.did_receive_zsh_precmd = false;
                }
            }
            ModelEvent::Handler(AnsiHandlerEvent::EndPrompt)
                if is_active_session_zsh && self.did_receive_zsh_precmd =>
            {
                if !self.is_line_editor_active {
                    self.mark_line_editor_active_after_delay(LINE_EDITOR_ACTIVATION_DELAY, ctx);
                }
                self.did_receive_zsh_precmd = false;
            }
            ModelEvent::Handler(AnsiHandlerEvent::Preexec) => {
                self.set_line_editor_inactive(ctx);
            }
            _ => (),
        }
    }

    fn mark_line_editor_active_after_delay(
        &mut self,
        delay: Duration,
        ctx: &mut ModelContext<Self>,
    ) {
        abort_if_running(self.mark_line_editor_active_abort_handle.take());

        // For zsh, we use this heuristic -- 10ms after EndPrompt -- to approximate when the line
        // editor is active.
        let abort_handle = ctx.spawn_abortable(
            async move { warpui::r#async::Timer::after(delay).await },
            |me, _, ctx| {
                me.is_line_editor_active = true;
                ctx.emit(LineEditorStatusEvent::Active);
            },
            |_, _| (),
        );
        self.mark_line_editor_active_abort_handle = Some(abort_handle);
    }

    fn set_line_editor_inactive(&mut self, ctx: &mut ModelContext<Self>) {
        abort_if_running(self.mark_line_editor_active_abort_handle.take());

        if !self.is_line_editor_active {
            return;
        }
        self.is_line_editor_active = false;
        ctx.emit(LineEditorStatusEvent::Inactive);
    }
}

/// Aborts the given `abort_handle` if it is `Some()`.
fn abort_if_running(abort_handle: Option<SpawnedFutureHandle>) {
    if let Some(abort_handle) = abort_handle {
        abort_handle.abort();
    }
}

#[derive(Clone, Copy, Debug)]
pub enum LineEditorStatusEvent {
    Active,
    Inactive,
}

impl Entity for LineEditorStatus {
    type Event = LineEditorStatusEvent;
}
