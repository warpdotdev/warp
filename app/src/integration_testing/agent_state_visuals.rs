use warpui::{App, AppContext, EntityId, SingletonEntity, ViewHandle};

use crate::terminal::TerminalView;
use crate::terminal::cli_agent_sessions::event::CLI_AGENT_NOTIFICATION_SENTINEL;
use crate::terminal::cli_agent_sessions::{CLIAgentDisplayState, CLIAgentSessionsModel};
use crate::terminal::model_events::ModelEvent;
use crate::user_config::WarpConfig;
use crate::user_config::agent_tab_styles::{AgentTabColor, AgentTabStyles};

pub fn agent_tab_styles_path() -> std::path::PathBuf {
    crate::user_config::agent_tab_styles_path()
}

pub fn cli_agent_display_state(
    app: &AppContext,
    terminal_view_id: EntityId,
) -> Option<&'static str> {
    CLIAgentSessionsModel::as_ref(app)
        .display_state(terminal_view_id)
        .map(|state| match state {
            CLIAgentDisplayState::Idle => "idle",
            CLIAgentDisplayState::Processing => "processing",
            CLIAgentDisplayState::Success => "success",
            CLIAgentDisplayState::NeedsAttention => "needs_attention",
        })
}

pub fn agent_tab_styles_are_default(app: &AppContext) -> bool {
    WarpConfig::as_ref(app).agent_tab_styles() == &AgentTabStyles::default()
}

pub fn agent_tab_style_color(app: &AppContext, state: &str) -> &'static str {
    let styles = &WarpConfig::as_ref(app).agent_tab_styles().states;
    let color = match state {
        "idle" => styles.idle.color,
        "processing" => styles.processing.color,
        "success" => styles.success.color,
        "needs_attention" => styles.needs_attention.color,
        other => panic!("unknown agent-tab state {other}"),
    };
    match color {
        AgentTabColor::Red => "red",
        AgentTabColor::Green => "green",
        AgentTabColor::Yellow => "yellow",
        AgentTabColor::Blue => "blue",
        AgentTabColor::Magenta => "magenta",
        AgentTabColor::Cyan => "cyan",
    }
}

pub fn emit_cli_agent_notification(
    app: &mut App,
    terminal: &ViewHandle<TerminalView>,
    body: String,
) {
    let dispatcher = terminal.read(app, |terminal, _| terminal.model_event_dispatcher().clone());
    dispatcher.update(app, |_, ctx| {
        ctx.emit(ModelEvent::PluggableNotification {
            title: Some(CLI_AGENT_NOTIFICATION_SENTINEL.to_string()),
            body,
        });
    });
}
