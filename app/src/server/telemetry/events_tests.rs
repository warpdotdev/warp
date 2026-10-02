use serde_json::json;
use warp_core::telemetry::{TelemetryEvent as _, TelemetryEventDesc};

use super::{FileTreeSource, NotificationAgentVariant, TelemetryEvent};
use crate::ai::agent_management::notifications::NotificationSourceAgent;
use crate::code_review::telemetry_event::{CodeReviewPaneEntrypoint, CodeReviewTelemetryEvent};
use crate::terminal::CLIAgent;
use crate::terminal::view::NotificationsTrigger;

#[derive(Debug)]
enum TelemetryEventPropertyError {
    // The variant data is never directly read, but it's used for error formatting if the test
    // below fails.
    EmptyName(#[expect(dead_code)] Box<dyn TelemetryEventDesc>),
    EmptyDescription(#[expect(dead_code)] Box<dyn TelemetryEventDesc>),
}

/// Checks that all telemetry events have a non-empty name and description.
///
/// The name and description are intended to be user-facing and are used to populate
/// our [exhaustive telemetry table](https://docs.warp.dev/support-and-community/privacy-and-security/privacy#exhaustive-telemetry-table).
#[test]
#[cfg(not(target_family = "wasm"))]
fn telemetry_events_have_nonempty_name_and_description() -> Result<(), TelemetryEventPropertyError>
{
    for event in warp_core::telemetry::all_events() {
        if event.name().is_empty() {
            return Err(TelemetryEventPropertyError::EmptyName(event));
        }
        if event.description().is_empty() {
            return Err(TelemetryEventPropertyError::EmptyDescription(event));
        }
    }
    Ok(())
}

#[test]
fn cli_agent_toolbar_payload_uses_analytics_name() {
    let event = TelemetryEvent::CLIAgentToolbarShown {
        cli_agent: CLIAgent::CursorCli.telemetry_name(),
    };

    assert_eq!(event.payload(), Some(json!({"agent_name": "Cursor"})));
}

#[test]
fn cli_agent_notification_payload_preserves_nested_variant() {
    let source = NotificationSourceAgent::CLI {
        agent: CLIAgent::CursorCli,
        is_ambient: false,
    };
    let event = TelemetryEvent::AgentNotificationShown {
        agent_variant: source.into(),
    };

    assert_eq!(
        event.payload(),
        Some(json!({"agent_variant": {"c_l_i_agent": "Cursor"}}))
    );
}

#[test]
fn notification_sent_payload_preserves_optional_agent_variant() {
    let event = TelemetryEvent::NotificationSent {
        trigger: NotificationsTrigger::NeedsAttention,
        agent_variant: Some(NotificationAgentVariant::CLIAgent(
            CLIAgent::CursorCli.telemetry_name(),
        )),
    };
    let no_agent_event = TelemetryEvent::NotificationSent {
        trigger: NotificationsTrigger::NeedsAttention,
        agent_variant: None,
    };

    assert_eq!(
        event.payload(),
        Some(json!({
            "trigger": "NeedsAttention",
            "agent_variant": {"c_l_i_agent": "Cursor"},
        }))
    );
    assert_eq!(
        no_agent_event.payload(),
        Some(json!({"trigger": "NeedsAttention", "agent_variant": null}))
    );
}

#[test]
fn file_tree_payload_preserves_optional_cli_agent() {
    let event = TelemetryEvent::FileTreeToggled {
        source: FileTreeSource::CLIAgentView,
        is_code_mode_v2: true,
        cli_agent: Some(CLIAgent::CursorCli.telemetry_name()),
    };
    let no_agent_event = TelemetryEvent::FileTreeToggled {
        source: FileTreeSource::AgentToolbelt,
        is_code_mode_v2: true,
        cli_agent: None,
    };

    assert_eq!(
        event.payload(),
        Some(json!({
            "source": "CLIAgentView",
            "is_code_mode_v2": true,
            "cli_agent": "Cursor",
        }))
    );
    assert_eq!(
        no_agent_event.payload(),
        Some(json!({
            "source": "AgentToolbelt",
            "is_code_mode_v2": true,
            "cli_agent": null,
        }))
    );
}

#[test]
fn code_review_payload_preserves_optional_cli_agent() {
    let event = CodeReviewTelemetryEvent::PaneOpened {
        is_local: Some(true),
        entrypoint: CodeReviewPaneEntrypoint::CLIAgentView,
        is_code_mode_v2: true,
        cli_agent: Some(CLIAgent::CursorCli.telemetry_name()),
    };
    let no_agent_event = CodeReviewTelemetryEvent::PaneOpened {
        is_local: None,
        entrypoint: CodeReviewPaneEntrypoint::Other,
        is_code_mode_v2: true,
        cli_agent: None,
    };

    assert_eq!(
        event.payload(),
        Some(json!({
            "is_local": true,
            "entrypoint": "cli_agent_view",
            "is_code_mode_v2": true,
            "agent_name": "Cursor",
        }))
    );
    assert_eq!(
        no_agent_event.payload(),
        Some(json!({
            "is_local": null,
            "entrypoint": "other",
            "is_code_mode_v2": true,
            "agent_name": null,
        }))
    );
}
