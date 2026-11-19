//! Cursor's matcher rule: a regex searched, unanchored and case-sensitive, against a value that
//! depends on the event. `""`, `"*"`, and a missing matcher match everything. On the events
//! matched by tool name, each part of the action (one file of a batched read or edit) is tested
//! on its own, so a `Delete` matcher selects the delete in a batch that also writes.

use std::borrow::Cow;

use super::events::{CursorEventSpec, MatchedValue, SUBAGENT_TYPE};
use super::facets::{
    finished_shell_command, mcp_tool_call, pending_shell_command, subagent_launch,
};
use super::parts::{ToolPart, tool_parts};
use super::sources::SourceCache;
use crate::hooks::adapters::DeclaredBinding;
use crate::hooks::model::HookEnvelope;

/// Cursor treats an empty matcher and a bare `*` as "match everything"; neither is a regex.
pub(super) fn matches_everything(pattern: &str) -> bool {
    pattern.is_empty() || pattern == "*"
}

/// Whether `binding`, a definition of `spec`'s event, selects this firing or any part of it. An
/// event Warp does not fire yet never matches, and neither does an envelope the event is not
/// about (a `tool.result` for a file read is not a finished shell command).
pub(super) fn binding_matches(
    sources: &SourceCache,
    spec: &CursorEventSpec,
    binding: &DeclaredBinding,
    envelope: &HookEnvelope,
) -> bool {
    if !spec.fires_in_phase_1 || spec.boundary != Some(envelope.boundary()) {
        return false;
    }
    if spec.matched_value == MatchedValue::ToolName {
        return envelope.payload.action().is_some_and(|action| {
            tool_parts(action)
                .iter()
                .any(|part| part_matches(sources, binding, part))
        });
    }
    let Some(value) = matched_value(spec, envelope) else {
        return false;
    };
    pattern_matches(sources, binding, &value)
}

/// Whether `binding`'s matcher, on an event matched by tool name, selects `part`.
pub(super) fn part_matches(
    sources: &SourceCache,
    binding: &DeclaredBinding,
    part: &ToolPart<'_>,
) -> bool {
    pattern_matches(sources, binding, &part.tool_name())
}

fn pattern_matches(sources: &SourceCache, binding: &DeclaredBinding, value: &str) -> bool {
    match binding.matcher.as_deref() {
        None => true,
        Some(pattern) if matches_everything(pattern) => true,
        Some(pattern) => sources
            .regex(&binding.id.source, pattern)
            .is_some_and(|regex| regex.is_match(value)),
    }
}

/// The text `spec`'s matcher is tested against for `envelope`, or `None` when the envelope is
/// not an occurrence of the event. The tool-name events are matched part by part instead; here
/// they answer with the whole action's name.
pub(super) fn matched_value<'a>(
    spec: &CursorEventSpec,
    envelope: &'a HookEnvelope,
) -> Option<Cow<'a, str>> {
    let payload = &envelope.payload;
    match spec.matched_value {
        MatchedValue::ToolName => payload
            .action()
            .map(|action| ToolPart::Whole(action).tool_name()),
        MatchedValue::CommandText => pending_shell_command(payload)
            .or_else(|| finished_shell_command(payload).map(|finished| finished.command))
            .map(Cow::Borrowed),
        MatchedValue::McpToolName => mcp_tool_call(payload).map(|call| Cow::Borrowed(call.name)),
        MatchedValue::SubagentType => {
            subagent_launch(payload).map(|_| Cow::Borrowed(SUBAGENT_TYPE))
        }
        MatchedValue::Literal(literal) => Some(Cow::Borrowed(literal)),
        MatchedValue::Everything => Some(Cow::Borrowed("")),
    }
}

#[cfg(test)]
#[path = "matcher_tests.rs"]
mod tests;
