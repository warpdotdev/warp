//! Turns what a hook process did into effects under Cursor's rules. Exit 2 blocks on any event
//! that can block; exit 0 means "read the JSON on stdout" and each event honors only the fields
//! Cursor documents for it; empty stdout, output that does not fit the event, and every other way
//! a process can end follow the binding's policies. A field that asks for an effect the event
//! cannot produce is reported rather than applied, except a rewrite request on a permission
//! hook, which denies the action because a rewrite can never be dropped silently. Under the
//! strict hook policy every failure of a hook that can block becomes a block, and a `preToolUse`
//! `ask` is honored instead of ignored.

use serde_json::{Map, Value};

use super::events::{AskRule, ControlEffect, CursorEvent, CursorEventSpec, VerdictRules};
use crate::hooks::adapters::{DeclaredBinding, DecodePolicy};
use crate::hooks::model::{
    ContinuationRequest, EffectKind, EmptyOutputPolicy, FailurePolicy, HookEffects, HookReason,
    HookVerdict, MalformedOutputPolicy, UnsupportedEffect,
};
use crate::hooks::runner::{HookProcessOutcome, ProcessExit};

pub(super) fn decode_output(
    spec: &CursorEventSpec,
    binding: &DeclaredBinding,
    outcome: &HookProcessOutcome,
    policy: &DecodePolicy,
) -> HookEffects {
    let decoder = Decoder {
        spec,
        binding,
        strict: policy.strict && !matches!(spec.control, ControlEffect::ObserveOnly),
    };
    match &outcome.exit {
        ProcessExit::Code(2) => decoder.blocked_by_exit_two(&outcome.stdout),
        // What the runner kept is not the whole response, so it cannot be read as a decision.
        ProcessExit::Code(0) if outcome.stdout_truncated => decoder.malformed(),
        ProcessExit::Code(0) => decoder.decode_success(&outcome.stdout),
        ProcessExit::Code(code) => decoder.failed(&format!("exited with code {code}")),
        ProcessExit::Signal(signal) => decoder.failed(&format!("was killed by signal {signal}")),
        ProcessExit::TimedOut => decoder.failed("timed out"),
        ProcessExit::SpawnFailed(error) => decoder.failed(&format!("could not start: {error}")),
        // A cancelled hook never evaluated the operation. Its verdict is normally discarded by
        // the caller; if it is ever applied, it must not read as permission.
        ProcessExit::Cancelled => decoder.block("was cancelled before it finished"),
    }
}

/// Output that does not fit the event's documented schema.
struct Malformed;

type Decoded = Result<HookEffects, Malformed>;

struct Decoder<'a> {
    spec: &'a CursorEventSpec,
    binding: &'a DeclaredBinding,
    /// The strict hook policy applies to this binding: it can block, and the setting is on.
    strict: bool,
}

impl Decoder<'_> {
    fn verdict_rules(&self) -> Option<VerdictRules> {
        match self.spec.control {
            ControlEffect::Verdict(rules) => Some(rules),
            ControlEffect::HaltTurn | ControlEffect::ObserveOnly => None,
        }
    }

    fn failure_policy(&self) -> FailurePolicy {
        if self.strict {
            FailurePolicy::FailClosed
        } else {
            self.binding.capabilities.failure
        }
    }

    fn malformed_output_policy(&self) -> MalformedOutputPolicy {
        if self.strict {
            MalformedOutputPolicy::Deny
        } else {
            self.binding.capabilities.malformed_output
        }
    }

    fn blocked_by_exit_two(&self, stdout: &[u8]) -> HookEffects {
        // Exit 2 may still carry messages; a script that printed nothing or something else gets
        // the default reason.
        let output = parse_object(stdout).unwrap_or_default();
        match self.spec.control {
            ControlEffect::Verdict(_) => {
                let (agent_message, user_message) =
                    self.permission_messages(&output).unwrap_or((None, None));
                let mut effects =
                    self.verdict_effects(HookVerdict::Deny, agent_message, user_message);
                if effects.reasons.is_empty() {
                    effects
                        .reasons
                        .push(self.reason(format!("Denied by hook {} (exit 2).", self.command())));
                }
                effects
            }
            ControlEffect::HaltTurn => {
                let message = text_field(&output, "user_message").ok().flatten();
                self.halt(
                    message
                        .unwrap_or_else(|| format!("Blocked by hook {} (exit 2).", self.command())),
                )
            }
            ControlEffect::ObserveOnly => self.failed("exited with code 2"),
        }
    }

    /// The configured command with control characters stripped, or the binding id if nothing
    /// printable remains.
    fn command(&self) -> String {
        clean_text(&self.binding.command).unwrap_or_else(|| self.binding.id.to_string())
    }

    fn decode_success(&self, stdout: &[u8]) -> HookEffects {
        if stdout.iter().all(u8::is_ascii_whitespace) {
            return match self.binding.capabilities.empty_output {
                EmptyOutputPolicy::NoOpinion => HookEffects::default(),
                EmptyOutputPolicy::FollowFailurePolicy => self.failed("printed no decision"),
            };
        }
        let Some(output) = parse_object(stdout) else {
            return self.malformed();
        };
        let unsupported = self.report_undocumented_fields(&output);
        let decoded = match self.spec.event {
            CursorEvent::SessionStart => self.decode_session_start(&output),
            CursorEvent::PreToolUse
            | CursorEvent::SubagentStart
            | CursorEvent::BeforeShellExecution
            | CursorEvent::BeforeMcpExecution
            | CursorEvent::BeforeReadFile
            | CursorEvent::BeforeTabFileRead => self.decode_permission(&output),
            CursorEvent::PostToolUse => self.decode_post_tool_use(&output),
            CursorEvent::PostToolUseFailure => self.decode_additional_context(&output),
            CursorEvent::SubagentStop | CursorEvent::Stop => self.decode_follow_up(&output),
            CursorEvent::BeforeSubmitPrompt => self.decode_before_submit_prompt(&output),
            CursorEvent::PreCompact => self.decode_pre_compact(&output),
            // Cursor documents no output for these, or output Warp cannot use.
            CursorEvent::SessionEnd
            | CursorEvent::AfterShellExecution
            | CursorEvent::AfterMcpExecution
            | CursorEvent::AfterFileEdit
            | CursorEvent::AfterAgentResponse
            | CursorEvent::AfterAgentThought
            | CursorEvent::AfterTabFileEdit
            | CursorEvent::WorkspaceOpen => Ok(HookEffects::default()),
        };
        let mut effects = decoded.unwrap_or_else(|Malformed| self.malformed());
        effects.unsupported = unsupported;
        effects
    }

    fn decode_permission(&self, output: &Map<String, Value>) -> Decoded {
        let claude = self.claude_output(output)?;
        let permission = raw_text_field(output, "permission")?.or_else(|| {
            claude
                .as_ref()
                .and_then(|claude| claude.permission_decision.clone())
        });
        let Some(permission) = permission else {
            return Err(Malformed);
        };
        let (agent_message, user_message) = self.permission_messages(output)?;
        let user_message = user_message.or_else(|| {
            claude
                .as_ref()
                .and_then(|claude| claude.permission_decision_reason.clone())
        });

        let mut effects = match permission.as_str() {
            "allow" => self.verdict_effects(HookVerdict::Allow, agent_message, user_message),
            "deny" => {
                let mut effects =
                    self.verdict_effects(HookVerdict::Deny, agent_message, user_message);
                if effects.reasons.is_empty() {
                    effects
                        .reasons
                        .push(self.reason(format!("Denied by hook {}.", self.command())));
                }
                effects
            }
            "ask" => self.ask_effects(agent_message, user_message)?,
            // Cursor says a response that does not match the event's schema blocks a permission
            // hook. An unknown value is read that way rather than as consent.
            _ => return Err(Malformed),
        };

        // A rewrite is carried on every permission hook, not only `preToolUse` where Cursor
        // documents it, because the permission policy denies an action whose input a hook asked
        // to change and cannot be honored; dropping the request would run the original input
        // against the hook's intent. Claude's nested spelling is already `None` on events whose
        // table entry does not document it.
        let updated_input = object_field(output, "updated_input")?
            .or_else(|| claude.and_then(|claude| claude.updated_input));
        effects.replacement_input = updated_input.map(Value::Object);
        Ok(effects)
    }

    fn ask_effects(&self, agent_message: Option<String>, user_message: Option<String>) -> Decoded {
        let Some(rules) = self.verdict_rules() else {
            return Err(Malformed);
        };
        match rules.ask {
            AskRule::Honor => {
                Ok(self.verdict_effects(HookVerdict::Ask, agent_message, user_message))
            }
            AskRule::IgnoreUnlessStrict => {
                if self.strict {
                    return Ok(self.verdict_effects(HookVerdict::Ask, agent_message, user_message));
                }
                log::info!(
                    "hook {} answered ask on {}, which Cursor does not enforce; it was ignored",
                    self.binding.id,
                    self.spec.name
                );
                Ok(self.verdict_effects(HookVerdict::NoOpinion, agent_message, user_message))
            }
            AskRule::Deny => {
                let mut effects =
                    self.verdict_effects(HookVerdict::Deny, agent_message, user_message);
                effects.reasons.insert(
                    0,
                    self.reason(format!(
                        "Hook {} answered ask, which {} treats as deny.",
                        self.command(),
                        self.spec.name
                    )),
                );
                Ok(effects)
            }
            AskRule::NotInSchema => Err(Malformed),
        }
    }

    /// The `agent_message` and `user_message` the event documents, when present and non-empty.
    /// A field the event does not document is left alone here and reported as undocumented.
    fn permission_messages(
        &self,
        output: &Map<String, Value>,
    ) -> Result<(Option<String>, Option<String>), Malformed> {
        let read = |field: &str| {
            if self.spec.documented_output.contains(&field) {
                text_field(output, field)
            } else {
                Ok(None)
            }
        };
        Ok((read("agent_message")?, read("user_message")?))
    }

    /// Effects for a decided verdict. On a deny, and on an ask (which a surface with no one to
    /// ask turns into a denial), `agent_message` (or `user_message` when there is no agent
    /// message) becomes the reason the model reads, and a `user_message` that is not already the
    /// reason is delivered as a user notice. On any other verdict `user_message` is a notice
    /// unless the event documents it as deny-only, and `agent_message` has no documented meaning,
    /// so it is logged and dropped.
    fn verdict_effects(
        &self,
        verdict: HookVerdict,
        agent_message: Option<String>,
        user_message: Option<String>,
    ) -> HookEffects {
        let mut effects = HookEffects {
            verdict,
            ..Default::default()
        };
        match verdict {
            HookVerdict::Deny | HookVerdict::Ask => match (agent_message, user_message) {
                (Some(agent), Some(user)) => {
                    effects.reasons.push(self.reason(agent));
                    effects.user_notices.push(user);
                }
                (Some(text), None) | (None, Some(text)) => {
                    effects.reasons.push(self.reason(text));
                }
                (None, None) => {}
            },
            HookVerdict::Allow | HookVerdict::NoOpinion | HookVerdict::Defer => {
                let deny_only = self
                    .verdict_rules()
                    .is_some_and(|rules| rules.user_message_on_deny_only);
                if deny_only {
                    if user_message.is_some() {
                        log::info!(
                            "hook {} set user_message without denying; {} shows it only on a \
                             denial, so it was ignored",
                            self.binding.id,
                            self.spec.name
                        );
                    }
                } else {
                    effects.user_notices.extend(user_message);
                }
                if agent_message.is_some() {
                    log::info!(
                        "hook {} set agent_message without denying; Cursor sends it to the agent \
                         only on a denial, so it was ignored",
                        self.binding.id
                    );
                }
            }
        }
        effects
    }

    fn decode_session_start(&self, output: &Map<String, Value>) -> Decoded {
        let mut effects = HookEffects::default();
        if let Some(env) = output.get("env") {
            let Some(env) = env.as_object() else {
                return Err(Malformed);
            };
            for (key, value) in env {
                let Some(value) = value.as_str() else {
                    return Err(Malformed);
                };
                effects.session_env.push((key.clone(), value.to_owned()));
            }
        }
        effects
            .model_context
            .extend(text_field(output, "additional_context")?);
        Ok(effects)
    }

    fn decode_post_tool_use(&self, output: &Map<String, Value>) -> Decoded {
        let mut effects = self.decode_additional_context(output)?;
        effects.replacement_result =
            object_field(output, "updated_mcp_tool_output")?.map(Value::Object);
        Ok(effects)
    }

    fn decode_additional_context(&self, output: &Map<String, Value>) -> Decoded {
        let mut effects = HookEffects::default();
        effects
            .model_context
            .extend(text_field(output, "additional_context")?);
        Ok(effects)
    }

    /// `followup_message`, or the Claude shapes Cursor translates: `decision: "block"` with a
    /// `reason`, flat or under `hookSpecificOutput`. An empty message is no follow-up.
    fn decode_follow_up(&self, output: &Map<String, Value>) -> Decoded {
        let claude = self.claude_output(output)?;
        let decision = raw_text_field(output, "decision")?
            .or_else(|| claude.as_ref().and_then(|claude| claude.decision.clone()));
        let reason = text_field(output, "reason")?
            .or_else(|| claude.as_ref().and_then(|claude| claude.reason.clone()));
        let follow_up = match text_field(output, "followup_message")? {
            Some(text) => Some(text),
            None if decision.as_deref() == Some("block") => reason,
            None => None,
        };
        Ok(HookEffects {
            continuation: follow_up.map(|text| ContinuationRequest {
                text,
                loop_limit: self.binding.loop_limit,
            }),
            ..Default::default()
        })
    }

    fn decode_before_submit_prompt(&self, output: &Map<String, Value>) -> Decoded {
        let Some(proceed) = bool_field(output, "continue")? else {
            return Err(Malformed);
        };
        let user_message = text_field(output, "user_message")?;
        if proceed {
            let mut effects = HookEffects::default();
            effects.user_notices.extend(user_message);
            return Ok(effects);
        }
        Ok(self
            .halt(user_message.unwrap_or_else(|| format!("Blocked by hook {}.", self.command()))))
    }

    fn decode_pre_compact(&self, output: &Map<String, Value>) -> Decoded {
        let mut effects = HookEffects::default();
        effects
            .user_notices
            .extend(text_field(output, "user_message")?);
        Ok(effects)
    }

    /// Claude's nested response shape, which Cursor accepts on `preToolUse`, `stop`, and
    /// `subagentStop`. Elsewhere the key is undocumented and ignored.
    fn claude_output(
        &self,
        output: &Map<String, Value>,
    ) -> Result<Option<ClaudeOutput>, Malformed> {
        if !self.spec.documented_output.contains(&"hookSpecificOutput") {
            return Ok(None);
        }
        let Some(nested) = output.get("hookSpecificOutput") else {
            return Ok(None);
        };
        let Some(nested) = nested.as_object() else {
            return Err(Malformed);
        };
        Ok(Some(ClaudeOutput {
            permission_decision: raw_text_field(nested, "permissionDecision")?,
            permission_decision_reason: text_field(nested, "permissionDecisionReason")?,
            updated_input: object_field(nested, "updatedInput")?,
            decision: raw_text_field(nested, "decision")?,
            reason: text_field(nested, "reason")?,
        }))
    }

    /// Fields the event does not document. One that names an effect Cursor defines on some other
    /// event, and whose value asks for that effect, is a control effect the binding cannot
    /// produce, so it is reported for the source's diagnostics; anything else is only logged.
    fn report_undocumented_fields(&self, output: &Map<String, Value>) -> Vec<UnsupportedEffect> {
        let mut unsupported = Vec::new();
        for (field, value) in output {
            if self.spec.documented_output.contains(&field.as_str()) {
                continue;
            }
            let effect = match classify_undocumented_control(field, value) {
                None => {
                    log::warn!(
                        "hook {} returned {field}, which {} does not use; it was ignored",
                        self.binding.id,
                        self.spec.name
                    );
                    continue;
                }
                Some(UndocumentedControl::NoOp) => {
                    log::info!(
                        "hook {} returned {field}: {value}, which {} does not use and which asks \
                         for nothing; it was ignored",
                        self.binding.id,
                        self.spec.name
                    );
                    continue;
                }
                Some(UndocumentedControl::Active(effect)) => effect,
            };
            let rewrite_on_permission_hook = (effect == EffectKind::ReplacementInput)
                .then(|| self.verdict_rules())
                .flatten();
            let reason = match rewrite_on_permission_hook {
                Some(rules) => format!(
                    "{} does not document updated_input, and a rewrite cannot be honored, so {} \
                     was denied instead of rewritten",
                    self.spec.name, rules.subject
                ),
                None => format!("{} does not use {field}; it was ignored", self.spec.name),
            };
            unsupported.push(UnsupportedEffect {
                binding: self.binding.id.clone(),
                effect,
                reason,
            });
        }
        unsupported
    }

    fn malformed(&self) -> HookEffects {
        let what_happened = match self.spec.control {
            ControlEffect::Verdict(_) => "printed output that is not a valid permission decision",
            ControlEffect::HaltTurn => "printed output that is not a valid response",
            ControlEffect::ObserveOnly => "printed output that does not fit the event",
        };
        match self.malformed_output_policy() {
            MalformedOutputPolicy::Deny => self.block(what_happened),
            MalformedOutputPolicy::FollowFailurePolicy => self.failed(what_happened),
        }
    }

    /// What a hook that did not produce a decision contributes, by the failure policy.
    fn failed(&self, what_happened: &str) -> HookEffects {
        match self.failure_policy() {
            FailurePolicy::FailOpen => HookEffects::default(),
            FailurePolicy::FailClosed => self.block(what_happened),
        }
    }

    /// What blocking means for this event: a denial where the event decides permission, a halted
    /// prompt where it decides submission, and nothing where it only observes.
    fn block(&self, what_happened: &str) -> HookEffects {
        let command = self.command();
        match self.spec.control {
            ControlEffect::Verdict(rules) => self.deny(format!(
                "Hook {command} {what_happened}, so {} was denied.",
                rules.subject
            )),
            ControlEffect::HaltTurn => self.halt(format!(
                "Hook {command} {what_happened}, so the prompt was not sent."
            )),
            ControlEffect::ObserveOnly => HookEffects::default(),
        }
    }

    fn deny(&self, reason: String) -> HookEffects {
        HookEffects {
            verdict: HookVerdict::Deny,
            reasons: vec![self.reason(reason)],
            ..Default::default()
        }
    }

    fn halt(&self, reason: String) -> HookEffects {
        HookEffects {
            halt_turn: true,
            reasons: vec![self.reason(reason)],
            ..Default::default()
        }
    }

    fn reason(&self, text: String) -> HookReason {
        HookReason {
            text,
            binding: Some(self.binding.id.clone()),
        }
    }
}

/// The fields of Claude's `hookSpecificOutput` object that Cursor translates.
struct ClaudeOutput {
    permission_decision: Option<String>,
    permission_decision_reason: Option<String>,
    updated_input: Option<Map<String, Value>>,
    decision: Option<String>,
    reason: Option<String>,
}

/// Parses stdout as a JSON object. Anything else, including stdout that is not UTF-8, is `None`.
fn parse_object(stdout: &[u8]) -> Option<Map<String, Value>> {
    match serde_json::from_slice(stdout).ok()? {
        Value::Object(object) => Some(object),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Array(_) => {
            None
        }
    }
}

/// An output key that carries a control effect on the events Cursor defines it for, found on an
/// event that does not.
enum UndocumentedControl {
    /// The value asks for the effect, which this binding cannot produce.
    Active(EffectKind),
    /// The value is the one that means "nothing to do" where the key is defined, so the hook has
    /// not asked for anything.
    NoOp,
}

/// Classifies an undocumented output key by the control effect it names. Claude-shaped output
/// routinely carries `continue: true`, and a hook that copies it onto an event that does not
/// define the key has not asked for anything, so the no-op values (`continue: true`,
/// `permission: "allow"`, a `decision` other than `"block"`, a blank `followup_message`) are told
/// apart from the asking ones. A rewrite, a result replacement, and Claude's nested object always
/// ask for something.
fn classify_undocumented_control(field: &str, value: &Value) -> Option<UndocumentedControl> {
    let (effect, is_no_op) = match field {
        "permission" => (EffectKind::Verdict, value.as_str() == Some("allow")),
        "hookSpecificOutput" => (EffectKind::Verdict, false),
        "updated_input" => (EffectKind::ReplacementInput, false),
        "updated_mcp_tool_output" => (EffectKind::ReplacementResult, false),
        "continue" => (EffectKind::HaltTurn, value.as_bool() == Some(true)),
        "followup_message" => (
            EffectKind::Continuation,
            value
                .as_str()
                .is_some_and(|message| clean_text(message).is_none()),
        ),
        "decision" => (
            EffectKind::Continuation,
            value.as_str().is_some_and(|decision| decision != "block"),
        ),
        _ => return None,
    };
    Some(if is_no_op {
        UndocumentedControl::NoOp
    } else {
        UndocumentedControl::Active(effect)
    })
}

/// A string field, cleaned for display. A present field that is not a string is malformed; an
/// empty or whitespace-only string counts as absent.
fn text_field(output: &Map<String, Value>, field: &str) -> Result<Option<String>, Malformed> {
    match output.get(field) {
        None => Ok(None),
        Some(Value::String(text)) => Ok(clean_text(text)),
        Some(_) => Err(Malformed),
    }
}

/// Reads an optional string field verbatim, without trimming or stripping control characters.
fn raw_text_field(output: &Map<String, Value>, field: &str) -> Result<Option<String>, Malformed> {
    match output.get(field) {
        None => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(_) => Err(Malformed),
    }
}

fn bool_field(output: &Map<String, Value>, field: &str) -> Result<Option<bool>, Malformed> {
    match output.get(field) {
        None => Ok(None),
        Some(Value::Bool(value)) => Ok(Some(*value)),
        Some(_) => Err(Malformed),
    }
}

fn object_field(
    output: &Map<String, Value>,
    field: &str,
) -> Result<Option<Map<String, Value>>, Malformed> {
    match output.get(field) {
        None => Ok(None),
        Some(Value::Object(object)) => Ok(Some(object.clone())),
        Some(_) => Err(Malformed),
    }
}

/// Trims the text and removes control characters (other than newlines and tabs) so a script
/// cannot put terminal escape sequences into a reason or notice. Empty results are dropped.
fn clean_text(text: &str) -> Option<String> {
    let cleaned = text
        .chars()
        .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
        .collect::<String>();
    let trimmed = cleaned.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

#[cfg(test)]
#[path = "decode_tests.rs"]
mod tests;
