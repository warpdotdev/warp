//! Cross-surface equivalence tests for the agent-icon helpers.
//!
//! The invariant under test: for every canonical logical run state, every surface produces
//! the same [`IconWithStatusVariant`]. Surfaces today are:
//! - Terminal view (vertical tabs + pane header) via
//!   [`super::agent_icon_variant_from_terminal_inputs`]
//! - Run cards (conversation list, agent management view) via
//!   [`super::agent_icon_variant_for_run`]
//! - Notification mailbox — exercised in `notifications/item_tests.rs`
//!
//! Adding a new canonical state is a one-enum-variant + one `expected` arm + one `*_inputs`
//! arm change; the table test below enforces every surface agrees.
use chrono::Utc;
use warp_cli::agent::Harness;

use super::{
    agent_conversation_entry_icon_variant, agent_icon_variant_for_run,
    agent_icon_variant_from_terminal_inputs, CLISessionInputs, TerminalIconInputs,
};
use crate::ai::agent::conversation::{AIConversationId, ConversationStatus};
use crate::ai::agent_conversations_model::entry::{
    AgentConversationBackingData, AgentConversationCapabilities, AgentConversationDisplayData,
    AgentConversationIdentity, AgentConversationPrincipal,
};
use crate::ai::agent_conversations_model::{
    AgentConversationEntry, AgentConversationEntryId, AgentConversationProvenance,
    AgentRunDisplayStatus,
};
use crate::terminal::CLIAgent;
use crate::ui_components::icon_with_status::IconWithStatusVariant;

/// Projection of the fields we care about for cross-surface equivalence.
/// [`IconWithStatusVariant`] itself can't derive `PartialEq` because `NeutralElement`
/// carries a `Box<dyn Element>`, so we extract the agent-variant fields here.
#[derive(Debug, Clone, PartialEq, Eq)]
struct AgentIconFields {
    is_cli: bool,
    cli_agent: Option<CLIAgent>,
    status: Option<ConversationStatus>,
    is_ambient: bool,
}

impl AgentIconFields {
    fn from_variant(variant: &IconWithStatusVariant) -> Option<Self> {
        match variant {
            IconWithStatusVariant::OzAgent { status, is_ambient } => Some(Self {
                is_cli: false,
                cli_agent: None,
                status: status.clone(),
                is_ambient: *is_ambient,
            }),
            IconWithStatusVariant::CLIAgent {
                agent,
                status,
                is_ambient,
            } => Some(Self {
                is_cli: true,
                cli_agent: Some(*agent),
                status: status.clone(),
                is_ambient: *is_ambient,
            }),
            IconWithStatusVariant::Neutral { .. }
            | IconWithStatusVariant::NeutralElement { .. }
            | IconWithStatusVariant::CustomAvatar { .. } => None,
        }
    }
}

/// Canonical logical run states. Each represents a conceptually distinct run whose icon must
/// be rendered identically across every surface that can display it.
#[derive(Debug, Clone, Copy)]
enum CanonicalRunState {
    /// Plain terminal, no conversation, no agent activity.
    PlainTerminal,
    /// Local Warp-native (Oz) conversation, in-progress.
    LocalOzInProgress,
    /// Cloud-mode Oz run, in-progress.
    CloudOzInProgress,
    /// Cloud Claude harness selected, pre-dispatch (no session, no status yet).
    /// This is the state the pre-setup icon bug regressed on — the tab must already render
    /// the Claude brand circle even though no CLI session exists yet.
    CloudClaudePreDispatch,
    /// Cloud Claude harness selected, dispatch in flight (status = InProgress, no session).
    CloudClaudeInProgress,
    /// Viewing a finished cloud Codex transcript whose VM has shut down. No live ambient
    /// model exists, so the harness comes from the conversation's server metadata; the icon
    /// must still render as cloud Codex.
    ViewingCloudCodexTranscript,
    /// Local Claude CLI session with a plugin listener (rich status), in-progress.
    LocalClaudePluginInProgress,
    /// Local Claude CLI session with a plugin listener (rich status), blocked.
    LocalClaudePluginBlocked,
    /// Local Claude CLI session detected via command matching only (no listener, no rich status).
    LocalClaudeCommandDetected,
}

impl CanonicalRunState {
    fn all() -> &'static [Self] {
        use CanonicalRunState::*;
        &[
            PlainTerminal,
            LocalOzInProgress,
            CloudOzInProgress,
            CloudClaudePreDispatch,
            CloudClaudeInProgress,
            ViewingCloudCodexTranscript,
            LocalClaudePluginInProgress,
            LocalClaudePluginBlocked,
            LocalClaudeCommandDetected,
        ]
    }

    /// The canonical [`AgentIconFields`] for this state. `None` means no agent icon renders.
    /// Editing an arm here is the deliberate way to evolve the cross-surface contract.
    fn expected(&self) -> Option<AgentIconFields> {
        use CanonicalRunState::*;
        match self {
            PlainTerminal => None,
            LocalOzInProgress => Some(AgentIconFields {
                is_cli: false,
                cli_agent: None,
                status: Some(ConversationStatus::InProgress),
                is_ambient: false,
            }),
            CloudOzInProgress => Some(AgentIconFields {
                is_cli: false,
                cli_agent: None,
                status: Some(ConversationStatus::InProgress),
                is_ambient: true,
            }),
            CloudClaudePreDispatch => Some(AgentIconFields {
                is_cli: true,
                cli_agent: Some(CLIAgent::Claude),
                status: None,
                is_ambient: true,
            }),
            CloudClaudeInProgress => Some(AgentIconFields {
                is_cli: true,
                cli_agent: Some(CLIAgent::Claude),
                status: Some(ConversationStatus::InProgress),
                is_ambient: true,
            }),
            ViewingCloudCodexTranscript => Some(AgentIconFields {
                is_cli: true,
                cli_agent: Some(CLIAgent::Codex),
                status: Some(ConversationStatus::Success),
                is_ambient: true,
            }),
            LocalClaudePluginInProgress => Some(AgentIconFields {
                is_cli: true,
                cli_agent: Some(CLIAgent::Claude),
                status: Some(ConversationStatus::InProgress),
                is_ambient: false,
            }),
            LocalClaudePluginBlocked => Some(AgentIconFields {
                is_cli: true,
                cli_agent: Some(CLIAgent::Claude),
                status: Some(ConversationStatus::Blocked {
                    blocked_action: String::new(),
                }),
                is_ambient: false,
            }),
            LocalClaudeCommandDetected => Some(AgentIconFields {
                is_cli: true,
                cli_agent: Some(CLIAgent::Claude),
                status: None,
                is_ambient: false,
            }),
        }
    }

    /// Terminal-view inputs for this state. Every state has a terminal representation.
    fn terminal_inputs(&self) -> TerminalIconInputs {
        use CanonicalRunState::*;
        match self {
            PlainTerminal => TerminalIconInputs {
                is_ambient: false,
                cli_session: None,
                selected_third_party_cli_agent: None,
                selected_conversation_status: None,
                has_selected_conversation: false,
            },
            LocalOzInProgress => TerminalIconInputs {
                is_ambient: false,
                cli_session: None,
                selected_third_party_cli_agent: None,
                selected_conversation_status: Some(ConversationStatus::InProgress),
                has_selected_conversation: true,
            },
            CloudOzInProgress => TerminalIconInputs {
                is_ambient: true,
                cli_session: None,
                selected_third_party_cli_agent: None,
                selected_conversation_status: Some(ConversationStatus::InProgress),
                has_selected_conversation: false,
            },
            CloudClaudePreDispatch => TerminalIconInputs {
                is_ambient: true,
                cli_session: None,
                selected_third_party_cli_agent: Some(CLIAgent::Claude),
                selected_conversation_status: None,
                has_selected_conversation: false,
            },
            CloudClaudeInProgress => TerminalIconInputs {
                is_ambient: true,
                cli_session: None,
                selected_third_party_cli_agent: Some(CLIAgent::Claude),
                selected_conversation_status: Some(ConversationStatus::InProgress),
                has_selected_conversation: false,
            },
            ViewingCloudCodexTranscript => TerminalIconInputs {
                // VM has shut down: the caller resolves these fields from the conversation's
                // server metadata, so the waterfall sees the same shape as a live run.
                is_ambient: true,
                cli_session: None,
                selected_third_party_cli_agent: Some(CLIAgent::Codex),
                selected_conversation_status: Some(ConversationStatus::Success),
                has_selected_conversation: true,
            },
            LocalClaudePluginInProgress => TerminalIconInputs {
                is_ambient: false,
                cli_session: Some(CLISessionInputs {
                    agent: CLIAgent::Claude,
                    has_listener: true,
                    status: ConversationStatus::InProgress,
                    supports_rich_status: true,
                }),
                selected_third_party_cli_agent: None,
                selected_conversation_status: None,
                has_selected_conversation: false,
            },
            LocalClaudePluginBlocked => TerminalIconInputs {
                is_ambient: false,
                cli_session: Some(CLISessionInputs {
                    agent: CLIAgent::Claude,
                    has_listener: true,
                    status: ConversationStatus::Blocked {
                        blocked_action: String::new(),
                    },
                    supports_rich_status: true,
                }),
                selected_third_party_cli_agent: None,
                selected_conversation_status: None,
                has_selected_conversation: false,
            },
            LocalClaudeCommandDetected => TerminalIconInputs {
                is_ambient: false,
                cli_session: Some(CLISessionInputs {
                    agent: CLIAgent::Claude,
                    has_listener: false,
                    status: ConversationStatus::InProgress,
                    supports_rich_status: false,
                }),
                selected_third_party_cli_agent: None,
                selected_conversation_status: None,
                has_selected_conversation: false,
            },
        }
    }

    /// Run-card inputs for this state, if it can surface as a run card.
    /// Cards only exist for cloud/ambient runs; local states return `None`.
    fn run_inputs(&self) -> Option<(Harness, ConversationStatus, bool)> {
        use CanonicalRunState::*;
        match self {
            CloudOzInProgress => Some((Harness::Oz, ConversationStatus::InProgress, true)),
            CloudClaudePreDispatch | CloudClaudeInProgress => {
                Some((Harness::Claude, ConversationStatus::InProgress, true))
            }
            ViewingCloudCodexTranscript => {
                Some((Harness::Codex, ConversationStatus::Success, true))
            }
            PlainTerminal
            | LocalOzInProgress
            | LocalClaudePluginInProgress
            | LocalClaudePluginBlocked
            | LocalClaudeCommandDetected => None,
        }
    }
}

/// The consistency enforcer: for every canonical state, the terminal-side and task-side
/// helpers must produce the same [`AgentIconFields`] projection.
#[test]
fn every_canonical_state_produces_consistent_icon_across_surfaces() {
    for state in CanonicalRunState::all() {
        let expected = state.expected();

        let terminal_actual = agent_icon_variant_from_terminal_inputs(&state.terminal_inputs())
            .as_ref()
            .and_then(AgentIconFields::from_variant);
        assert_eq!(
            terminal_actual, expected,
            "terminal surface disagreed for {state:?}"
        );

        if let Some((harness, status, is_ambient)) = state.run_inputs() {
            let run_variant = agent_icon_variant_for_run(harness, status.clone(), is_ambient);
            let run_actual = AgentIconFields::from_variant(&run_variant);
            // Run cards always populate status (they derive it from `ConversationOrTask::status`).
            let expected_for_run = expected.clone().map(|mut fields| {
                fields.status = Some(status);
                fields
            });
            assert_eq!(
                run_actual, expected_for_run,
                "run-card surface disagreed for {state:?}"
            );
        }
    }
}

/// Structural invariant: the `is_ambient` flag on the rendered variant must match the
/// `is_ambient` flag on the terminal inputs. Catches accidental drift in the waterfall.
#[test]
fn terminal_is_ambient_matches_inputs_for_every_state() {
    for state in CanonicalRunState::all() {
        let inputs = state.terminal_inputs();
        let Some(variant) = agent_icon_variant_from_terminal_inputs(&inputs) else {
            continue;
        };
        let fields = AgentIconFields::from_variant(&variant)
            .expect("terminal helper must only return agent variants");
        assert_eq!(
            fields.is_ambient, inputs.is_ambient,
            "is_ambient drifted for {state:?}"
        );
    }
}

#[test]
fn cli_agent_from_harness_maps_known_harnesses() {
    assert_eq!(CLIAgent::from_harness(Harness::Oz), None);
    assert_eq!(
        CLIAgent::from_harness(Harness::Claude),
        Some(CLIAgent::Claude)
    );
    assert_eq!(
        CLIAgent::from_harness(Harness::Gemini),
        Some(CLIAgent::Gemini)
    );
    assert_eq!(
        CLIAgent::from_harness(Harness::OpenCode),
        Some(CLIAgent::OpenCode)
    );
}

#[test]
fn run_card_with_oz_or_unknown_harness_renders_as_oz() {
    // Oz harness explicitly: local Oz is the spec-defined fallback.
    let variant = agent_icon_variant_for_run(Harness::Oz, ConversationStatus::Success, true);
    let fields = AgentIconFields::from_variant(&variant).unwrap();
    assert!(!fields.is_cli);
    assert!(fields.is_ambient);

    // Unknown harness (e.g. server surfaced a future variant): also falls back to Oz so we
    // don't render an unbranded gray circle.
    let variant = agent_icon_variant_for_run(Harness::Unknown, ConversationStatus::Success, true);
    let fields = AgentIconFields::from_variant(&variant).unwrap();
    assert!(!fields.is_cli);
    assert!(fields.is_ambient);
}

/// A local Claude session and an ambient Claude run must render with the same CLI agent
/// brand but differ only by `is_ambient`. This answers the product-spec ambiguity about
/// whether those should look different — they should.
#[test]
fn local_claude_vs_cloud_claude_differ_only_by_is_ambient() {
    let local = agent_icon_variant_from_terminal_inputs(
        &CanonicalRunState::LocalClaudePluginInProgress.terminal_inputs(),
    )
    .and_then(|v| AgentIconFields::from_variant(&v))
    .unwrap();
    let cloud = agent_icon_variant_from_terminal_inputs(
        &CanonicalRunState::CloudClaudeInProgress.terminal_inputs(),
    )
    .and_then(|v| AgentIconFields::from_variant(&v))
    .unwrap();

    assert_eq!(local.cli_agent, cloud.cli_agent);
    assert_eq!(local.cli_agent, Some(CLIAgent::Claude));
    assert!(!local.is_ambient);
    assert!(cloud.is_ambient);
}

#[test]
fn non_ambient_entry_uses_display_harness() {
    let conversation_id = AIConversationId::new();
    let entry = AgentConversationEntry {
        id: AgentConversationEntryId::Conversation(conversation_id),
        identity: AgentConversationIdentity {
            local_conversation_id: Some(conversation_id),
            ambient_agent_task_id: None,
            server_conversation_token: None,
            session_id: None,
        },
        provenance: AgentConversationProvenance::CloudSyncedConversation,
        display: AgentConversationDisplayData {
            title: "Codex conversation".to_string(),
            initial_query: None,
            created_at: Utc::now(),
            last_updated: Utc::now(),
            status: AgentRunDisplayStatus::ConversationSucceeded,
            creator: AgentConversationPrincipal::default(),
            executor: None,
            request_usage: None,
            run_time: None,
            session_status: None,
            source: None,
            working_directory: None,
            environment_id: None,
            harness: Some(Harness::Codex),
            artifacts: Vec::new(),
        },
        backing: AgentConversationBackingData {
            has_loaded_conversation: true,
            has_local_persisted_data: true,
            has_cloud_data: true,
            has_ambient_run: false,
        },
        capabilities: AgentConversationCapabilities {
            can_open: true,
            can_copy_link: false,
            can_share: false,
            can_delete: false,
            can_fork_locally: false,
            can_cancel: false,
        },
    };

    let variant = agent_conversation_entry_icon_variant(&entry);
    assert_eq!(
        AgentIconFields::from_variant(&variant).unwrap(),
        AgentIconFields {
            is_cli: true,
            cli_agent: Some(CLIAgent::Codex),
            status: Some(ConversationStatus::Success),
            is_ambient: false,
        }
    );
}
