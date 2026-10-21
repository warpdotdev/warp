//! Cursor's 21 hook events and what the adapter knows about each one: the Warp boundary it
//! projects onto, whether Warp fires it in Phase 1, what its matcher is tested against, which
//! output fields Cursor documents for it, and how it fails. Every other part of the adapter reads
//! an event's facts from this table and matches on `CursorEvent` only to pick per-event code.

use std::collections::HashSet;
use std::time::Duration;

use strum_macros::EnumCount;

use super::config::ParsedDefinition;
use crate::hooks::adapters::DeclaredBinding;
use crate::hooks::model::{
    BindingCapabilities, EffectKind, EmptyOutputPolicy, FailurePolicy, HandlerKind, HookBindingId,
    HookBoundary, HookSourceId, MalformedOutputPolicy, MatcherTarget, POST_AND_OTHER_TIMEOUT,
};

/// Cursor's default per-definition cap on the follow-ups a stop hook may request.
pub(super) const DEFAULT_LOOP_LIMIT: u32 = 5;

/// Warp's cap on hook-requested follow-ups in one continuation chain. A definition that asks for
/// more, or for no limit, gets this value.
pub(super) const LOOP_LIMIT_HOST_CAP: u32 = 8;

/// One of the event names Cursor's `hooks.json` accepts, in the order Cursor's reference lists
/// them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, EnumCount)]
pub(super) enum CursorEvent {
    SessionStart,
    SessionEnd,
    PreToolUse,
    PostToolUse,
    PostToolUseFailure,
    SubagentStart,
    SubagentStop,
    BeforeShellExecution,
    AfterShellExecution,
    BeforeMcpExecution,
    AfterMcpExecution,
    BeforeReadFile,
    AfterFileEdit,
    BeforeSubmitPrompt,
    PreCompact,
    Stop,
    AfterAgentResponse,
    AfterAgentThought,
    BeforeTabFileRead,
    AfterTabFileEdit,
    WorkspaceOpen,
}

impl CursorEvent {
    pub(super) fn from_name(name: &str) -> Option<Self> {
        CURSOR_EVENTS
            .iter()
            .find(|spec| spec.name == name)
            .map(|spec| spec.event)
    }

    #[cfg(test)]
    pub(super) fn name(self) -> &'static str {
        self.spec().name
    }

    pub(super) fn spec(self) -> &'static CursorEventSpec {
        CURSOR_EVENTS
            .iter()
            .find(|spec| spec.event == self)
            .expect("every Cursor event has a table entry")
    }
}

/// The value a definition's `matcher` is tested against when its event fires. Finer than the
/// shared `MatcherTarget` it is reported as: the adapter must tell an MCP tool's bare name from
/// the canonical `MCP:<tool>` (both are `MatcherTarget::ToolName`) and must know which events have
/// nothing to match (`Everything`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MatchedValue {
    /// The canonical tool name (`Shell`, `Read`, `MCP:<tool>`, and Warp's own names).
    ToolName,
    /// The model's full shell command.
    CommandText,
    /// The MCP tool's own name, without the `MCP:` prefix.
    McpToolName,
    /// The subagent type. No Phase 1 payload carries one.
    SubagentType,
    /// A fixed word Cursor documents for the event.
    Literal(&'static str),
    /// Cursor documents no value; a matcher on the event is ignored.
    Everything,
}

/// Whether a hook on the event can change what the agent does, and how.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ControlEffect {
    /// The hook returns a permission decision.
    Verdict,
    /// The hook can stop a prompt from being sent.
    HaltTurn,
    /// Nothing the hook returns changes what the agent does.
    ObserveOnly,
}

/// What the adapter knows about one event.
#[derive(Debug)]
pub(super) struct CursorEventSpec {
    pub event: CursorEvent,
    /// The event name as `hooks.json` spells it.
    pub name: &'static str,
    /// The Warp boundary the event projects onto; `None` when Warp has no such moment.
    pub boundary: Option<HookBoundary>,
    /// Whether Warp fires the event in Phase 1. False for a facet whose boundary fires but whose
    /// payload does not exist yet (`afterFileEdit`) as well as for events whose boundary does not
    /// fire.
    pub fires_in_phase_1: bool,
    pub matched_value: MatchedValue,
    /// The output fields Cursor documents for the event. Anything else on stdout is ignored.
    pub documented_output: &'static [&'static str],
    /// The effect channels those fields feed.
    pub may_produce: &'static [EffectKind],
    pub malformed_output: MalformedOutputPolicy,
    pub empty_output: EmptyOutputPolicy,
    /// Cursor documents `user_message` on this event as shown only when the hook denies, so on
    /// an allow it is dropped. The shell and MCP hooks document it as "shown in client" with no
    /// such condition.
    pub user_message_on_deny_only: bool,
    /// The noun Warp's own denial reasons use for what was blocked, as in "so the command was
    /// denied".
    pub subject: &'static str,
}

impl CursorEventSpec {
    pub(super) fn control(&self) -> ControlEffect {
        if self.may_produce.contains(&EffectKind::Verdict) {
            ControlEffect::Verdict
        } else if self.may_produce.contains(&EffectKind::HaltTurn) {
            ControlEffect::HaltTurn
        } else {
            ControlEffect::ObserveOnly
        }
    }

    pub(super) fn may_continue(&self) -> bool {
        self.may_produce.contains(&EffectKind::Continuation)
    }

    pub(super) fn has_matcher_target(&self) -> bool {
        self.matched_value != MatchedValue::Everything
    }

    pub(super) fn matcher_target(&self) -> MatcherTarget {
        match self.matched_value {
            MatchedValue::ToolName | MatchedValue::McpToolName => MatcherTarget::ToolName,
            MatchedValue::CommandText => MatcherTarget::CommandText,
            MatchedValue::SubagentType => MatcherTarget::SubagentType,
            MatchedValue::Literal(literal) => MatcherTarget::Literal(literal.to_owned()),
            MatchedValue::Everything => MatcherTarget::None,
        }
    }

    pub(super) fn default_timeout(&self) -> Duration {
        self.boundary
            .map(|boundary| boundary.default_timeout())
            .unwrap_or(POST_AND_OTHER_TIMEOUT)
    }

    pub(super) fn capabilities(&self, fail_closed: bool) -> BindingCapabilities {
        BindingCapabilities {
            may_produce: self.may_produce.iter().copied().collect::<HashSet<_>>(),
            failure: if fail_closed {
                FailurePolicy::FailClosed
            } else {
                FailurePolicy::FailOpen
            },
            malformed_output: self.malformed_output,
            empty_output: self.empty_output,
            matcher_target: self.matcher_target(),
            handler: HandlerKind::Command,
        }
    }
}

const NO_OUTPUT: &[&str] = &[];
const NO_EFFECTS: &[EffectKind] = &[];
const PERMISSION_OUTPUT: &[&str] = &["permission", "user_message", "agent_message"];
const PERMISSION_EFFECTS: &[EffectKind] = &[EffectKind::Verdict, EffectKind::UserNotice];
/// The follow-up fields for `stop` and `subagentStop`: Cursor's own `followup_message` plus the
/// Claude shapes it translates.
const FOLLOW_UP_OUTPUT: &[&str] = &[
    "followup_message",
    "hookSpecificOutput",
    "decision",
    "reason",
];

/// Cursor's events, in the order its reference lists them. Bindings from one source are declared
/// in this order, so two events that project onto the same boundary merge in a fixed order.
pub(super) static CURSOR_EVENTS: [CursorEventSpec; 21] = [
    CursorEventSpec {
        event: CursorEvent::SessionStart,
        name: "sessionStart",
        boundary: Some(HookBoundary::SessionStart),
        fires_in_phase_1: true,
        matched_value: MatchedValue::Everything,
        documented_output: &["env", "additional_context", "continue", "user_message"],
        may_produce: &[EffectKind::SessionEnv, EffectKind::ModelContext],
        malformed_output: MalformedOutputPolicy::FollowFailurePolicy,
        empty_output: EmptyOutputPolicy::NoOpinion,
        user_message_on_deny_only: false,
        subject: "the session",
    },
    CursorEventSpec {
        event: CursorEvent::SessionEnd,
        name: "sessionEnd",
        boundary: Some(HookBoundary::SessionEnd),
        fires_in_phase_1: false,
        matched_value: MatchedValue::Everything,
        documented_output: NO_OUTPUT,
        may_produce: NO_EFFECTS,
        malformed_output: MalformedOutputPolicy::FollowFailurePolicy,
        empty_output: EmptyOutputPolicy::NoOpinion,
        user_message_on_deny_only: false,
        subject: "the session",
    },
    CursorEventSpec {
        event: CursorEvent::PreToolUse,
        name: "preToolUse",
        boundary: Some(HookBoundary::ToolPre),
        fires_in_phase_1: true,
        matched_value: MatchedValue::ToolName,
        documented_output: &[
            "permission",
            "user_message",
            "agent_message",
            "updated_input",
            "hookSpecificOutput",
        ],
        may_produce: &[
            EffectKind::Verdict,
            EffectKind::UserNotice,
            EffectKind::ReplacementInput,
        ],
        malformed_output: MalformedOutputPolicy::Deny,
        empty_output: EmptyOutputPolicy::FollowFailurePolicy,
        user_message_on_deny_only: true,
        subject: "the tool call",
    },
    CursorEventSpec {
        event: CursorEvent::PostToolUse,
        name: "postToolUse",
        boundary: Some(HookBoundary::ToolResult),
        fires_in_phase_1: true,
        matched_value: MatchedValue::ToolName,
        documented_output: &["updated_mcp_tool_output", "additional_context"],
        may_produce: &[EffectKind::ReplacementResult, EffectKind::ModelContext],
        malformed_output: MalformedOutputPolicy::FollowFailurePolicy,
        empty_output: EmptyOutputPolicy::NoOpinion,
        user_message_on_deny_only: false,
        subject: "the tool result",
    },
    CursorEventSpec {
        event: CursorEvent::PostToolUseFailure,
        name: "postToolUseFailure",
        boundary: Some(HookBoundary::ToolFailure),
        fires_in_phase_1: true,
        matched_value: MatchedValue::ToolName,
        documented_output: &["additional_context"],
        may_produce: &[EffectKind::ModelContext],
        malformed_output: MalformedOutputPolicy::FollowFailurePolicy,
        empty_output: EmptyOutputPolicy::NoOpinion,
        user_message_on_deny_only: false,
        subject: "the tool result",
    },
    CursorEventSpec {
        event: CursorEvent::SubagentStart,
        name: "subagentStart",
        boundary: Some(HookBoundary::SubagentStart),
        fires_in_phase_1: false,
        matched_value: MatchedValue::SubagentType,
        documented_output: &["permission", "user_message"],
        may_produce: PERMISSION_EFFECTS,
        malformed_output: MalformedOutputPolicy::Deny,
        empty_output: EmptyOutputPolicy::FollowFailurePolicy,
        user_message_on_deny_only: true,
        subject: "the subagent",
    },
    CursorEventSpec {
        event: CursorEvent::SubagentStop,
        name: "subagentStop",
        boundary: Some(HookBoundary::SubagentStop),
        fires_in_phase_1: false,
        matched_value: MatchedValue::SubagentType,
        documented_output: FOLLOW_UP_OUTPUT,
        may_produce: &[EffectKind::Continuation],
        malformed_output: MalformedOutputPolicy::FollowFailurePolicy,
        empty_output: EmptyOutputPolicy::NoOpinion,
        user_message_on_deny_only: false,
        subject: "the subagent",
    },
    CursorEventSpec {
        event: CursorEvent::BeforeShellExecution,
        name: "beforeShellExecution",
        boundary: Some(HookBoundary::ToolPre),
        fires_in_phase_1: true,
        matched_value: MatchedValue::CommandText,
        documented_output: PERMISSION_OUTPUT,
        may_produce: PERMISSION_EFFECTS,
        malformed_output: MalformedOutputPolicy::Deny,
        empty_output: EmptyOutputPolicy::FollowFailurePolicy,
        user_message_on_deny_only: false,
        subject: "the command",
    },
    CursorEventSpec {
        event: CursorEvent::AfterShellExecution,
        name: "afterShellExecution",
        boundary: Some(HookBoundary::ToolResult),
        fires_in_phase_1: true,
        matched_value: MatchedValue::CommandText,
        documented_output: NO_OUTPUT,
        may_produce: NO_EFFECTS,
        malformed_output: MalformedOutputPolicy::FollowFailurePolicy,
        empty_output: EmptyOutputPolicy::NoOpinion,
        user_message_on_deny_only: false,
        subject: "the command",
    },
    CursorEventSpec {
        event: CursorEvent::BeforeMcpExecution,
        name: "beforeMCPExecution",
        boundary: Some(HookBoundary::ToolPre),
        fires_in_phase_1: true,
        matched_value: MatchedValue::McpToolName,
        documented_output: PERMISSION_OUTPUT,
        may_produce: PERMISSION_EFFECTS,
        malformed_output: MalformedOutputPolicy::Deny,
        empty_output: EmptyOutputPolicy::FollowFailurePolicy,
        user_message_on_deny_only: false,
        subject: "the MCP tool call",
    },
    CursorEventSpec {
        event: CursorEvent::AfterMcpExecution,
        name: "afterMCPExecution",
        boundary: Some(HookBoundary::ToolResult),
        fires_in_phase_1: true,
        matched_value: MatchedValue::McpToolName,
        documented_output: NO_OUTPUT,
        may_produce: NO_EFFECTS,
        malformed_output: MalformedOutputPolicy::FollowFailurePolicy,
        empty_output: EmptyOutputPolicy::NoOpinion,
        user_message_on_deny_only: false,
        subject: "the MCP tool call",
    },
    CursorEventSpec {
        event: CursorEvent::BeforeReadFile,
        name: "beforeReadFile",
        boundary: Some(HookBoundary::FileExposure),
        fires_in_phase_1: false,
        matched_value: MatchedValue::Literal("Read"),
        documented_output: &["permission", "user_message"],
        may_produce: PERMISSION_EFFECTS,
        malformed_output: MalformedOutputPolicy::Deny,
        empty_output: EmptyOutputPolicy::FollowFailurePolicy,
        user_message_on_deny_only: true,
        subject: "the file read",
    },
    CursorEventSpec {
        event: CursorEvent::AfterFileEdit,
        name: "afterFileEdit",
        boundary: Some(HookBoundary::ToolResult),
        fires_in_phase_1: false,
        matched_value: MatchedValue::Literal("Write"),
        documented_output: NO_OUTPUT,
        may_produce: NO_EFFECTS,
        malformed_output: MalformedOutputPolicy::FollowFailurePolicy,
        empty_output: EmptyOutputPolicy::NoOpinion,
        user_message_on_deny_only: false,
        subject: "the file edit",
    },
    CursorEventSpec {
        event: CursorEvent::BeforeSubmitPrompt,
        name: "beforeSubmitPrompt",
        boundary: Some(HookBoundary::PromptSubmit),
        fires_in_phase_1: true,
        matched_value: MatchedValue::Literal("UserPromptSubmit"),
        documented_output: &["continue", "user_message"],
        may_produce: &[EffectKind::HaltTurn, EffectKind::UserNotice],
        malformed_output: MalformedOutputPolicy::FollowFailurePolicy,
        empty_output: EmptyOutputPolicy::FollowFailurePolicy,
        user_message_on_deny_only: false,
        subject: "the prompt",
    },
    CursorEventSpec {
        event: CursorEvent::PreCompact,
        name: "preCompact",
        boundary: Some(HookBoundary::CompactionPre),
        fires_in_phase_1: false,
        matched_value: MatchedValue::Everything,
        documented_output: &["user_message"],
        may_produce: &[EffectKind::UserNotice],
        malformed_output: MalformedOutputPolicy::FollowFailurePolicy,
        empty_output: EmptyOutputPolicy::NoOpinion,
        user_message_on_deny_only: false,
        subject: "the compaction",
    },
    CursorEventSpec {
        event: CursorEvent::Stop,
        name: "stop",
        boundary: Some(HookBoundary::TurnStop),
        fires_in_phase_1: true,
        matched_value: MatchedValue::Literal("Stop"),
        documented_output: FOLLOW_UP_OUTPUT,
        may_produce: &[EffectKind::Continuation],
        malformed_output: MalformedOutputPolicy::FollowFailurePolicy,
        empty_output: EmptyOutputPolicy::NoOpinion,
        user_message_on_deny_only: false,
        subject: "the turn",
    },
    CursorEventSpec {
        event: CursorEvent::AfterAgentResponse,
        name: "afterAgentResponse",
        boundary: Some(HookBoundary::ResponseComplete),
        fires_in_phase_1: true,
        matched_value: MatchedValue::Literal("AgentResponse"),
        documented_output: NO_OUTPUT,
        may_produce: NO_EFFECTS,
        malformed_output: MalformedOutputPolicy::FollowFailurePolicy,
        empty_output: EmptyOutputPolicy::NoOpinion,
        user_message_on_deny_only: false,
        subject: "the response",
    },
    CursorEventSpec {
        event: CursorEvent::AfterAgentThought,
        name: "afterAgentThought",
        boundary: Some(HookBoundary::ReasoningComplete),
        fires_in_phase_1: true,
        matched_value: MatchedValue::Literal("AgentThought"),
        documented_output: NO_OUTPUT,
        may_produce: NO_EFFECTS,
        malformed_output: MalformedOutputPolicy::FollowFailurePolicy,
        empty_output: EmptyOutputPolicy::NoOpinion,
        user_message_on_deny_only: false,
        subject: "the reasoning",
    },
    CursorEventSpec {
        event: CursorEvent::BeforeTabFileRead,
        name: "beforeTabFileRead",
        boundary: None,
        fires_in_phase_1: false,
        matched_value: MatchedValue::Literal("TabRead"),
        documented_output: &["permission"],
        may_produce: &[EffectKind::Verdict],
        malformed_output: MalformedOutputPolicy::Deny,
        empty_output: EmptyOutputPolicy::FollowFailurePolicy,
        user_message_on_deny_only: false,
        subject: "the file read",
    },
    CursorEventSpec {
        event: CursorEvent::AfterTabFileEdit,
        name: "afterTabFileEdit",
        boundary: None,
        fires_in_phase_1: false,
        matched_value: MatchedValue::Literal("TabWrite"),
        documented_output: NO_OUTPUT,
        may_produce: NO_EFFECTS,
        malformed_output: MalformedOutputPolicy::FollowFailurePolicy,
        empty_output: EmptyOutputPolicy::NoOpinion,
        user_message_on_deny_only: false,
        subject: "the file edit",
    },
    CursorEventSpec {
        event: CursorEvent::WorkspaceOpen,
        name: "workspaceOpen",
        boundary: None,
        fires_in_phase_1: false,
        matched_value: MatchedValue::Everything,
        documented_output: &["pluginPaths"],
        may_produce: NO_EFFECTS,
        malformed_output: MalformedOutputPolicy::FollowFailurePolicy,
        empty_output: EmptyOutputPolicy::NoOpinion,
        user_message_on_deny_only: false,
        subject: "the workspace",
    },
];

/// Declares one binding per parsed definition whose event has a Warp boundary. Definitions are
/// already in table order, then array order, which is the order they merge in.
pub(super) fn declare_bindings(
    definitions: &[ParsedDefinition],
    source: &HookSourceId,
) -> Vec<DeclaredBinding> {
    definitions
        .iter()
        .filter_map(|definition| {
            let spec = definition.event.spec();
            let boundary = spec.boundary?;
            Some(DeclaredBinding {
                id: HookBindingId {
                    source: source.clone(),
                    event: spec.name.to_owned(),
                    index: definition.index,
                },
                boundary,
                vendor_event: spec.name.to_owned(),
                capabilities: spec.capabilities(definition.fail_closed),
                matcher: definition.matcher_text().map(str::to_owned),
                command: definition.command.clone(),
                timeout: definition.timeout,
                loop_limit: definition.loop_limit,
            })
        })
        .collect()
}

#[cfg(test)]
#[path = "events_tests.rs"]
mod tests;
