use warp_core::send_telemetry_from_ctx;
use warpui::{Entity, ModelContext, ModelHandle};

use crate::ai::blocklist::agent_view::{AgentViewController, AgentViewControllerEvent};
use crate::server::telemetry::TelemetryEvent;
use crate::terminal::input::buffer_model::InputBufferModel;

/// Model responsible for managing state required to conditionally render the shortcuts view.
pub struct AgentShortcutViewModel {
    is_shortcut_view_open: bool,
}

impl AgentShortcutViewModel {
    pub fn new(
        input_buffer_model: ModelHandle<InputBufferModel>,
        agent_view_controller: ModelHandle<AgentViewController>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        ctx.subscribe_to_model(&input_buffer_model, |me, _, event, ctx| {
            if me.is_shortcut_view_open && !event.new_content.is_empty() {
                me.hide_shortcut_view(ctx);
            }
        });
        ctx.subscribe_to_model(&agent_view_controller, |me, _, event, ctx| {
            if matches!(event, AgentViewControllerEvent::ExitedAgentView { .. }) {
                me.hide_shortcut_view(ctx);
            }
        });

        Self {
            is_shortcut_view_open: false,
        }
    }

    pub fn is_shortcut_view_open(&self) -> bool {
        self.is_shortcut_view_open
    }

    pub fn open_shortcut_view(&mut self, ctx: &mut ModelContext<Self>) {
        self.set_shortcut_view_visibility(true, ctx);
    }

    pub fn hide_shortcut_view(&mut self, ctx: &mut ModelContext<Self>) {
        self.set_shortcut_view_visibility(false, ctx);
    }

    fn set_shortcut_view_visibility(&mut self, is_open: bool, ctx: &mut ModelContext<Self>) {
        if is_open == self.is_shortcut_view_open {
            return;
        }
        self.is_shortcut_view_open = is_open;
        ctx.emit(AgentShortcutEvent::ToggledViewVisibility {
            is_visible: is_open,
        });
        send_telemetry_from_ctx!(
            TelemetryEvent::AgentShortcutsViewToggled {
                is_visible: is_open,
            },
            ctx
        );
    }
}

impl Entity for AgentShortcutViewModel {
    type Event = AgentShortcutEvent;
}

pub enum AgentShortcutEvent {
    ToggledViewVisibility { is_visible: bool },
}
