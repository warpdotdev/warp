//! The adapter for Cursor's `hooks.json` format. This first version understands only
//! `beforeShellExecution`, which runs before a shell command and may deny it.

mod config;
mod decode;

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use regex::Regex;
use serde_json::json;

use self::config::{
    BEFORE_SHELL_EXECUTION, matches_everything, parse_cursor_document, parse_cursor_file,
};
use self::decode::decode_shell_permission_output;
use crate::agent::action::AIAgentActionType;
use crate::hooks::adapters::{
    DeclaredBinding, HookConfigError, HookFormatAdapter, ParsedHookConfig,
};
use crate::hooks::model::{
    BindingCapabilities, DiagnosticSeverity, EffectKind, EmptyOutputPolicy, FailurePolicy,
    HandlerKind, HookBindingId, HookBoundary, HookDiagnostic, HookEffects, HookEnvelope,
    HookFormat, HookPayload, HookSourceId, MalformedOutputPolicy, MatcherTarget,
};
use crate::hooks::runner::HookProcessOutcome;

#[derive(Default)]
pub struct CursorAdapter {
    /// Compiled matchers by pattern text. Patterns were validated when the file was parsed, so
    /// every entry here compiled once already.
    matchers: Mutex<HashMap<String, Arc<Regex>>>,
}

impl CursorAdapter {
    pub fn new() -> Self {
        Self::default()
    }

    fn matcher(&self, pattern: &str) -> Option<Arc<Regex>> {
        let mut matchers = self.matchers.lock().expect("matcher cache lock");
        if let Some(regex) = matchers.get(pattern) {
            return Some(regex.clone());
        }
        let regex = Arc::new(Regex::new(pattern).ok()?);
        matchers.insert(pattern.to_owned(), regex.clone());
        Some(regex)
    }
}

/// What a `beforeShellExecution` definition may do and how it fails: it is one of Cursor's
/// permission hooks, so unreadable output denies even when the definition fails open.
pub(crate) fn shell_permission_capabilities(fail_closed: bool) -> BindingCapabilities {
    BindingCapabilities {
        may_produce: HashSet::from([
            EffectKind::Verdict,
            EffectKind::UserNotice,
            EffectKind::ModelContext,
        ]),
        failure: if fail_closed {
            FailurePolicy::FailClosed
        } else {
            FailurePolicy::FailOpen
        },
        malformed_output: MalformedOutputPolicy::Deny,
        empty_output: EmptyOutputPolicy::FollowFailurePolicy,
        matcher_target: MatcherTarget::CommandText,
        handler: HandlerKind::Command,
    }
}

/// The shell command a `tool.pre` envelope is about, when it is about one.
fn shell_command(envelope: &HookEnvelope) -> Option<&str> {
    match &envelope.payload {
        HookPayload::ToolPre {
            action: AIAgentActionType::RequestCommandOutput { command, .. },
            ..
        } => Some(command),
        HookPayload::ToolPre { .. }
        | HookPayload::SessionStart { .. }
        | HookPayload::PromptSubmit { .. }
        | HookPayload::ToolResult { .. }
        | HookPayload::ToolFailure { .. }
        | HookPayload::TurnStop { .. }
        | HookPayload::ResponseComplete { .. }
        | HookPayload::ReasoningComplete { .. } => None,
    }
}

impl HookFormatAdapter for CursorAdapter {
    fn format(&self) -> HookFormat {
        HookFormat::Cursor
    }

    fn parse(
        &self,
        bytes: &[u8],
        source: &HookSourceId,
    ) -> Result<ParsedHookConfig, HookConfigError> {
        let (document, parsed) = parse_cursor_file(bytes)?;
        let warning = |message: String| HookDiagnostic {
            source: source.clone(),
            severity: DiagnosticSeverity::Warning,
            message,
        };
        let warnings = parsed
            .unknown_events
            .iter()
            .map(|event| {
                warning(format!(
                    "{event} hooks are not supported yet; they were skipped"
                ))
            })
            .chain(parsed.skipped_definitions.into_iter().map(warning))
            .collect();
        Ok(ParsedHookConfig {
            source: source.clone(),
            format: HookFormat::Cursor,
            document,
            warnings,
        })
    }

    fn bindings(&self, config: &ParsedHookConfig) -> Vec<DeclaredBinding> {
        let parsed = match parse_cursor_document(&config.document) {
            Ok(parsed) => parsed,
            Err(error) => {
                log::error!(
                    "hook source {} stopped parsing after it was accepted: {error}",
                    config.source
                );
                return Vec::new();
            }
        };
        parsed
            .shell_hooks
            .into_iter()
            .map(|definition| DeclaredBinding {
                id: HookBindingId {
                    source: config.source.clone(),
                    event: BEFORE_SHELL_EXECUTION.to_owned(),
                    index: definition.index,
                },
                boundary: HookBoundary::ToolPre,
                vendor_event: BEFORE_SHELL_EXECUTION.to_owned(),
                capabilities: shell_permission_capabilities(definition.fail_closed),
                matcher: definition.matcher,
                command: definition.command,
                timeout: definition.timeout,
            })
            .collect()
    }

    fn matches(&self, binding: &DeclaredBinding, envelope: &HookEnvelope) -> bool {
        let Some(command) = shell_command(envelope) else {
            return false;
        };
        match binding.matcher.as_deref() {
            None => true,
            Some(pattern) if matches_everything(pattern) => true,
            Some(pattern) => self
                .matcher(pattern)
                .is_some_and(|regex| regex.is_match(command)),
        }
    }

    fn encode_input(
        &self,
        binding: &DeclaredBinding,
        envelope: &HookEnvelope,
    ) -> serde_json::Value {
        let conversation_id = envelope.conversation_id.to_string();
        let generation_id = envelope
            .turn_id
            .map(|turn| turn.as_uuid().to_string())
            .unwrap_or_else(|| conversation_id.clone());
        let cwd = envelope.cwd.as_deref().map(|cwd| cwd.to_string_lossy());
        let workspace_roots = if envelope.roots.is_empty() {
            cwd.iter().map(ToString::to_string).collect::<Vec<_>>()
        } else {
            envelope
                .roots
                .iter()
                .map(|root| root.to_string_lossy().into_owned())
                .collect()
        };

        let mut input = json!({
            "conversation_id": conversation_id,
            "generation_id": generation_id,
            "hook_event_name": binding.vendor_event,
            "cursor_version": envelope.warp_version,
            "workspace_roots": workspace_roots,
            "user_email": envelope.user_email,
            "transcript_path": serde_json::Value::Null,
        });
        if let Some(model) = &envelope.model_id {
            input["model"] = json!(model.to_string());
        }
        if let Some(action_id) = &envelope.action_id {
            input["tool_use_id"] = json!(action_id.to_string());
        }
        if let Some(command) = shell_command(envelope) {
            input["command"] = json!(command);
        }
        if let Some(cwd) = cwd {
            input["cwd"] = json!(cwd);
        }
        input
    }

    fn decode_output(
        &self,
        binding: &DeclaredBinding,
        outcome: &HookProcessOutcome,
    ) -> HookEffects {
        decode_shell_permission_output(binding, outcome)
    }
}

#[cfg(test)]
#[path = "mod_test.rs"]
mod tests;
