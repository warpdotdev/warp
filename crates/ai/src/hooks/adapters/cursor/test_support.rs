//! Builders shared by the adapter's test modules: the checked-in fixtures, an envelope with fixed
//! common fields that the `stdin.json` fixtures were written against, and sample actions.

use std::path::PathBuf;
use std::time::Duration;

use ai_types::{AIAgentActionId, AIConversationId};
use uuid::Uuid;
use warp_terminal::model::BlockId;

use super::config::{ParsedDefinition, parse_cursor_file};
use super::events::{CursorEvent, declare_bindings};
use crate::LLMId;
use crate::agent::action::AIAgentActionType;
use crate::agent::action_result::{AIAgentActionResultType, RequestCommandOutputResult};
use crate::hooks::adapters::DeclaredBinding;
use crate::hooks::model::{CanonicalToolName, HookEnvelope, HookPayload};
use crate::hooks::runner::{HookProcessOutcome, ProcessExit};
use crate::hooks::test_util::{test_envelope, test_source_id};

pub(super) const FULL_CONFIG: &str = include_str!("fixtures/full_config.json");
pub(super) const INVALID_MATCHER: &str = include_str!("fixtures/invalid_matcher.json");
pub(super) const UNKNOWN_EVENT: &str = include_str!("fixtures/unknown_event.json");
pub(super) const PROMPT_HOOK: &str = include_str!("fixtures/prompt_hook.json");
pub(super) const CLAUDE_SHAPE_OUTPUT: &str = include_str!("fixtures/claude_shape_output.json");

/// The checked-in files for one event: its `hooks.json`, the stdin Warp encodes for
/// [`fixture_envelope`], and a documented stdout.
pub(super) struct EventFixture {
    pub event: CursorEvent,
    pub hooks: &'static str,
    pub stdin: &'static str,
    pub stdout: &'static str,
}

macro_rules! event_fixture {
    ($event:ident, $dir:literal) => {
        EventFixture {
            event: CursorEvent::$event,
            hooks: include_str!(concat!("fixtures/", $dir, "/hooks.json")),
            stdin: include_str!(concat!("fixtures/", $dir, "/stdin.json")),
            stdout: include_str!(concat!("fixtures/", $dir, "/stdout.json")),
        }
    };
}

/// One fixture per event Warp fires in Phase 1.
pub(super) static EVENT_FIXTURES: [EventFixture; 12] = [
    event_fixture!(SessionStart, "sessionStart"),
    event_fixture!(PreToolUse, "preToolUse"),
    event_fixture!(PostToolUse, "postToolUse"),
    event_fixture!(PostToolUseFailure, "postToolUseFailure"),
    event_fixture!(BeforeShellExecution, "beforeShellExecution"),
    event_fixture!(AfterShellExecution, "afterShellExecution"),
    event_fixture!(BeforeMcpExecution, "beforeMCPExecution"),
    event_fixture!(AfterMcpExecution, "afterMCPExecution"),
    event_fixture!(BeforeSubmitPrompt, "beforeSubmitPrompt"),
    event_fixture!(Stop, "stop"),
    event_fixture!(AfterAgentResponse, "afterAgentResponse"),
    event_fixture!(AfterAgentThought, "afterAgentThought"),
];

pub(super) fn event_fixture(event: CursorEvent) -> &'static EventFixture {
    EVENT_FIXTURES
        .iter()
        .find(|fixture| fixture.event == event)
        .expect("a fixture exists for every Phase 1 event")
}

pub(super) const FIXTURE_CONVERSATION_ID: &str = "11111111-1111-1111-1111-111111111111";
pub(super) const FIXTURE_MCP_SERVER_ID: &str = "22222222-2222-2222-2222-222222222222";

pub(super) fn fixture_conversation_id() -> AIConversationId {
    AIConversationId::try_from(FIXTURE_CONVERSATION_ID.to_owned()).expect("a uuid")
}

/// An envelope with the common fields the `stdin.json` fixtures were written against.
pub(super) fn fixture_envelope(payload: HookPayload) -> HookEnvelope {
    let mut envelope = test_envelope(fixture_conversation_id(), payload);
    envelope.cwd = Some(PathBuf::from("/work/repo/src"));
    envelope.roots = vec![PathBuf::from("/work/repo")];
    envelope.model_id = Some(LLMId::from("claude-sonnet".to_owned()));
    envelope.user_email = Some("dev@example.com".to_owned());
    envelope.warp_version = "v0.2026.01.01".to_owned();
    envelope.action_id = Some(AIAgentActionId::from("call_1".to_owned()));
    envelope
}

pub(super) fn shell_action(command: &str) -> AIAgentActionType {
    AIAgentActionType::RequestCommandOutput {
        command: command.to_owned(),
        is_read_only: None,
        is_risky: None,
        wait_until_completion: true,
        uses_pager: None,
        rationale: None,
        citations: Vec::new(),
    }
}

pub(super) fn mcp_action(name: &str, input: serde_json::Value) -> AIAgentActionType {
    AIAgentActionType::CallMCPTool {
        server_id: Some(Uuid::parse_str(FIXTURE_MCP_SERVER_ID).expect("a uuid")),
        name: name.to_owned(),
        input,
    }
}

pub(super) fn tool_pre(action: AIAgentActionType) -> HookPayload {
    let tool_name = CanonicalToolName::for_action(&action);
    HookPayload::ToolPre { action, tool_name }
}

pub(super) fn tool_result(
    action: AIAgentActionType,
    result: AIAgentActionResultType,
    duration: Option<Duration>,
) -> HookPayload {
    let tool_name = CanonicalToolName::for_action(&action);
    HookPayload::ToolResult {
        action,
        tool_name,
        result,
        duration,
    }
}

pub(super) fn tool_failure(
    action: AIAgentActionType,
    result: AIAgentActionResultType,
    cancellation_reason: Option<&str>,
) -> HookPayload {
    let tool_name = CanonicalToolName::for_action(&action);
    HookPayload::ToolFailure {
        action,
        tool_name,
        result,
        cancellation_reason: cancellation_reason.map(str::to_owned),
        duration: None,
    }
}

pub(super) fn completed_shell(
    command: &str,
    output: &str,
    exit_code: i32,
) -> AIAgentActionResultType {
    AIAgentActionResultType::RequestCommandOutput(RequestCommandOutputResult::Completed {
        block_id: BlockId::from("block-1".to_owned()),
        command: command.to_owned(),
        output: output.to_owned(),
        exit_code: exit_code.into(),
        start_ts: None,
        completed_ts: None,
    })
}

/// A binding for `event` with the given `failClosed`, declared the way the adapter declares one.
pub(super) fn binding_for(event: CursorEvent, fail_closed: bool) -> DeclaredBinding {
    let spec = event.spec();
    let definition = ParsedDefinition {
        event,
        index: 0,
        command: "./check.sh".to_owned(),
        matcher: None,
        timeout: spec.default_timeout(),
        fail_closed,
        loop_limit: spec.may_continue().then_some(5),
    };
    declare_bindings(&[definition], &test_source_id("hooks.json"))
        .pop()
        .expect("the event has a boundary")
}

/// The bindings a `hooks.json` text declares, in order.
pub(super) fn bindings_from(text: &str) -> Vec<DeclaredBinding> {
    let (_, parsed) = parse_cursor_file(text.as_bytes()).expect("parses");
    declare_bindings(&parsed.definitions, &test_source_id("hooks.json"))
}

pub(super) fn outcome(exit: ProcessExit, stdout: &str) -> HookProcessOutcome {
    HookProcessOutcome {
        exit,
        stdout: stdout.as_bytes().to_vec(),
        stderr_tail: String::new(),
        duration: Duration::from_millis(1),
    }
}

pub(super) fn json(text: &str) -> serde_json::Value {
    serde_json::from_str(text).expect("fixture is JSON")
}
