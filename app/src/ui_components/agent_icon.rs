//! Source-facing helpers that centralize the derivation of the agent-icon shape
//! ([`IconWithStatusVariant`]) from the underlying state models. The invariant the
//! helpers enforce: any single logical agent run renders as the same brand color, glyph,
//! and ambient-vs-local treatment regardless of which surface is rendering it (vertical
//! tabs, pane header, conversation list, notifications mailbox).
//!
//! Each helper is a thin adapter over one data source. Surfaces call the helper for
//! whichever source they hold and feed the resulting variant into
//! [`render_icon_with_status`]. The pure inner functions in this module are exercised
//! directly by the cross-surface consistency tests in `agent_icon_tests.rs`.
use warp_cli::agent::Harness;
use warpui::AppContext;
use warpui::SingletonEntity;

use crate::ai::agent::conversation::ConversationStatus;
use crate::ai::agent_conversations_model::{
    AgentConversationEntry, AgentConversationProvenance, AgentConversationsModel,
    AgentRunDisplayStatus,
};
use crate::terminal::cli_agent_sessions::listener::agent_supports_rich_status;
use crate::terminal::cli_agent_sessions::CLIAgentSessionsModel;
use crate::terminal::view::TerminalView;
use crate::terminal::CLIAgent;
use crate::ui_components::icon_with_status::IconWithStatusVariant;

/// Returns the agent-icon variant for a live [`TerminalView`], or `None` when the terminal is
/// not an agent surface (plain terminal / shell / empty conversation).
///
/// Resolution order:
/// 1. A [`CLIAgentSessionsModel`] session with a known agent wins. Plugin-backed sessions
///    surface rich status; command-detected sessions don't.
/// 2. A task-backed run uses task status and harness so the terminal chrome and the
///    matching conversation list card stay in lockstep.
/// 3. Live ambient pre-dispatch or a selected local conversation falls through to the
///    no-task waterfall.
/// 4. Everything else returns `None` so the caller renders a plain-terminal indicator.
pub(crate) fn terminal_view_agent_icon_variant(
    terminal_view: &TerminalView,
    app: &AppContext,
) -> Option<IconWithStatusVariant> {
    let cli_agent_session = CLIAgentSessionsModel::as_ref(app).session(terminal_view.id());

    // Resolve the ambient task id from [`TerminalView::ambient_agent_task_id_for_details_panel`],
    // falling back to the selected conversation's server metadata for restored cloud transcripts.
    let ambient_task_id = terminal_view
        .ambient_agent_task_id_for_details_panel(app)
        .or_else(|| {
            terminal_view
                .selected_conversation_server_metadata(app)
                .and_then(|m| m.ambient_agent_task_id)
        });
    let task_data = ambient_task_id
        .and_then(|task_id| AgentConversationsModel::as_ref(app).get_task_data(&task_id));

    // Defer to the card helper when we have task data and no CLI session takes precedence.
    if cli_agent_session.is_none() {
        if let Some(task) = task_data.as_ref() {
            let status = AgentRunDisplayStatus::from_task(task, app).to_conversation_status();
            let harness = task
                .agent_config_snapshot
                .as_ref()
                .and_then(|config| config.harness.as_ref())
                .map(|harness| harness.harness_type)
                .unwrap_or(Harness::Oz);
            return Some(agent_icon_variant_for_run(harness, status, true));
        }
    }

    let is_ambient = terminal_view.is_ambient_agent_session(app) || ambient_task_id.is_some();
    let inputs = TerminalIconInputs {
        is_ambient,
        cli_session: cli_agent_session.map(|session| CLISessionInputs {
            agent: session.agent,
            has_listener: session.listener.is_some(),
            status: session.status.to_conversation_status(),
            supports_rich_status: agent_supports_rich_status(&session.agent),
        }),
        selected_third_party_cli_agent: terminal_view
            .ambient_agent_view_model()
            .and_then(|model| model.as_ref(app).selected_third_party_cli_agent()),
        selected_conversation_status: terminal_view.selected_conversation_status_for_display(app),
        has_selected_conversation: terminal_view
            .selected_conversation_display_title(app)
            .is_some(),
    };
    agent_icon_variant_from_terminal_inputs(&inputs)
}

pub(crate) fn agent_conversation_entry_icon_variant(
    entry: &AgentConversationEntry,
) -> IconWithStatusVariant {
    let status = entry.display.status.to_conversation_status();
    let is_ambient = matches!(entry.provenance, AgentConversationProvenance::AmbientRun)
        || entry.backing.has_ambient_run
        || entry.identity.ambient_agent_task_id.is_some();
    agent_icon_variant_for_run(
        entry.display.harness.unwrap_or(Harness::Oz),
        status,
        is_ambient,
    )
}

/// Primitive inputs to the terminal-view waterfall, gathered once from the live
/// [`TerminalView`] / [`AppContext`].
struct TerminalIconInputs {
    is_ambient: bool,
    cli_session: Option<CLISessionInputs>,
    /// Third-party CLI agent for a live ambient run before task data is available (e.g.
    /// Claude pre-dispatch). `None` otherwise; task-derived harnesses are handled upstream.
    selected_third_party_cli_agent: Option<CLIAgent>,
    /// The conversation status that the terminal view would surface in its status-icon slot.
    selected_conversation_status: Option<ConversationStatus>,
    /// Whether the terminal view currently has a selected conversation (ambient or local).
    has_selected_conversation: bool,
}

/// CLI-session-derived inputs for the terminal waterfall.
struct CLISessionInputs {
    agent: CLIAgent,
    /// Whether the session is backed by a plugin listener. Plugin-backed sessions report
    /// rich status; command-detected sessions only know that an agent is running.
    has_listener: bool,
    status: ConversationStatus,
    /// Whether the agent's session handler exposes rich status (plugin-backed handlers report
    /// rich status; Codex's OSC 9 handler does not).
    supports_rich_status: bool,
}

/// Pure waterfall from primitive inputs to an [`IconWithStatusVariant`]. Mirrors the
/// resolution order documented on [`terminal_view_agent_icon_variant`].
fn agent_icon_variant_from_terminal_inputs(
    inputs: &TerminalIconInputs,
) -> Option<IconWithStatusVariant> {
    // 1. CLI session with a known (non-Unknown) agent wins. Status is only meaningful when
    //    the session is plugin-backed and the handler exposes rich status.
    if let Some(session) = inputs
        .cli_session
        .as_ref()
        .filter(|s| !matches!(s.agent, CLIAgent::Unknown))
    {
        let status =
            (session.has_listener && session.supports_rich_status).then(|| session.status.clone());
        return Some(IconWithStatusVariant::CLIAgent {
            agent: session.agent,
            status,
            is_ambient: inputs.is_ambient,
        });
    }

    // 2. Live ambient run with a third-party harness selected, before task data is
    //    available (e.g. Claude pre-dispatch). `Unknown` is filtered so an unrecognized
    //    harness doesn't render as an unbranded gray circle.
    if inputs.is_ambient {
        if let Some(agent) = inputs
            .selected_third_party_cli_agent
            .filter(|agent| !matches!(agent, CLIAgent::Unknown))
        {
            return Some(IconWithStatusVariant::CLIAgent {
                agent,
                status: inputs.selected_conversation_status.clone(),
                is_ambient: true,
            });
        }
    }

    // 3. Selected conversation OR ambient (Oz) terminal: Oz agent variant.
    if inputs.has_selected_conversation || inputs.is_ambient {
        return Some(IconWithStatusVariant::OzAgent {
            status: inputs.selected_conversation_status.clone(),
            is_ambient: inputs.is_ambient,
        });
    }

    None
}

/// Pure run-card logic: maps a [`Harness`], status, and ambient flag into an
/// [`IconWithStatusVariant`]. Falls back to the Oz variant for [`Harness::Oz`] and
/// [`Harness::Unknown`], the latter so a future-server harness this client doesn't
/// recognize doesn't render an unbranded gray circle.
pub(crate) fn agent_icon_variant_for_run(
    harness: Harness,
    status: ConversationStatus,
    is_ambient: bool,
) -> IconWithStatusVariant {
    let cli_agent =
        CLIAgent::from_harness(harness).filter(|agent| !matches!(agent, CLIAgent::Unknown));
    match cli_agent {
        Some(agent) => IconWithStatusVariant::CLIAgent {
            agent,
            status: Some(status),
            is_ambient,
        },
        None => IconWithStatusVariant::OzAgent {
            status: Some(status),
            is_ambient,
        },
    }
}

#[cfg(test)]
#[path = "agent_icon_tests.rs"]
mod tests;
