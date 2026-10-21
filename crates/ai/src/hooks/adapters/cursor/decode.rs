//! Turns what a `beforeShellExecution` process did into effects. Cursor's rules: exit 2 denies;
//! exit 0 means "use the JSON on stdout"; any other exit, a timeout, or a crash follows the
//! definition's failure policy; output that is not a valid decision denies, because this is one of
//! Cursor's permission hooks.

use serde::Deserialize;

use crate::hooks::adapters::DeclaredBinding;
use crate::hooks::model::{
    EmptyOutputPolicy, FailurePolicy, HookEffects, HookReason, HookVerdict, MalformedOutputPolicy,
};
use crate::hooks::runner::{HookProcessOutcome, ProcessExit};

/// The fields Cursor documents for a `beforeShellExecution` response.
#[derive(Debug, Default, Deserialize)]
struct ShellHookOutput {
    #[serde(default)]
    permission: Option<String>,
    #[serde(default)]
    user_message: Option<String>,
    #[serde(default)]
    agent_message: Option<String>,
}

/// Decodes `outcome` for `binding`, which must be a `beforeShellExecution` binding.
pub(super) fn decode_shell_permission_output(
    binding: &DeclaredBinding,
    outcome: &HookProcessOutcome,
) -> HookEffects {
    let decoder = Decoder { binding };
    match &outcome.exit {
        ProcessExit::Code(2) => decoder.deny_from_exit_code(&outcome.stdout),
        ProcessExit::Code(0) => decoder.decode_success(&outcome.stdout),
        ProcessExit::Code(code) => decoder.failed(&format!("exited with code {code}")),
        ProcessExit::Signal(signal) => decoder.failed(&format!("was killed by signal {signal}")),
        ProcessExit::TimedOut => decoder.failed("timed out"),
        ProcessExit::SpawnFailed(error) => decoder.failed(&format!("could not start: {error}")),
        // A cancelled hook never evaluated the command. Its verdict is normally discarded by the
        // caller; if it is ever applied, it must not read as permission.
        ProcessExit::Cancelled => decoder.deny(format!(
            "Hook {} was cancelled before it finished.",
            decoder.binding.command
        )),
    }
}

struct Decoder<'a> {
    binding: &'a DeclaredBinding,
}

impl Decoder<'_> {
    fn deny_from_exit_code(&self, stdout: &[u8]) -> HookEffects {
        // Exit 2 may still carry messages; a script that printed nothing or something else gets
        // the default reason.
        let output = parse_output(stdout).unwrap_or_default();
        let mut effects = self.verdict_effects(HookVerdict::Deny, &output);
        if effects.reasons.is_empty() {
            effects
                .reasons
                .push(self.reason(format!("Denied by hook {} (exit 2).", self.binding.command)));
        }
        effects
    }

    fn decode_success(&self, stdout: &[u8]) -> HookEffects {
        if stdout.iter().all(u8::is_ascii_whitespace) {
            return match self.binding.capabilities.empty_output {
                EmptyOutputPolicy::NoOpinion => HookEffects::default(),
                EmptyOutputPolicy::FollowFailurePolicy => self.failed("printed no decision"),
            };
        }
        let Some(output) = parse_output(stdout) else {
            return self.malformed();
        };
        let verdict = match output.permission.as_deref() {
            Some("allow") => HookVerdict::Allow,
            Some("deny") => HookVerdict::Deny,
            Some("ask") => HookVerdict::Ask,
            // Cursor says a response that does not match the event's schema blocks a permission
            // hook. A missing or unknown `permission` is read that way rather than as consent.
            Some(_) | None => return self.malformed(),
        };
        self.verdict_effects(verdict, &output)
    }

    /// Effects for a decided verdict. On a deny, `agent_message` (or `user_message` when there is
    /// no agent message) becomes the reason the model reads. A `user_message` that is not already
    /// the reason is delivered as a user notice. On other verdicts the messages are observational.
    fn verdict_effects(&self, verdict: HookVerdict, output: &ShellHookOutput) -> HookEffects {
        let agent_message = output.agent_message.as_deref().and_then(non_empty_text);
        let user_message = output.user_message.as_deref().and_then(non_empty_text);
        let mut effects = HookEffects {
            verdict,
            ..Default::default()
        };
        match verdict {
            HookVerdict::Deny => match (agent_message, user_message) {
                (Some(agent), Some(user)) => {
                    effects.reasons.push(self.reason(agent));
                    effects.user_notices.push(user);
                }
                (Some(text), None) | (None, Some(text)) => {
                    effects.reasons.push(self.reason(text));
                }
                (None, None) => {}
            },
            HookVerdict::Allow | HookVerdict::Ask | HookVerdict::NoOpinion | HookVerdict::Defer => {
                effects.model_context.extend(agent_message);
                effects.user_notices.extend(user_message);
            }
        }
        effects
    }

    fn malformed(&self) -> HookEffects {
        match self.binding.capabilities.malformed_output {
            MalformedOutputPolicy::Deny => self.deny(format!(
                "Hook {} printed output that is not a valid permission decision, so the command \
                 was denied.",
                self.binding.command
            )),
            MalformedOutputPolicy::FollowFailurePolicy => {
                self.failed("printed output that is not a valid permission decision")
            }
        }
    }

    /// What a hook that did not produce a decision contributes, by the binding's failure policy.
    fn failed(&self, what_happened: &str) -> HookEffects {
        match self.binding.capabilities.failure {
            FailurePolicy::FailOpen => HookEffects::default(),
            FailurePolicy::FailClosed => self.deny(format!(
                "Hook {} {what_happened}, so the command was denied.",
                self.binding.command
            )),
        }
    }

    fn deny(&self, reason: String) -> HookEffects {
        HookEffects {
            verdict: HookVerdict::Deny,
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

/// Parses stdout as a Cursor response object. Anything that is not a JSON object with the
/// documented field types is `None`.
fn parse_output(stdout: &[u8]) -> Option<ShellHookOutput> {
    let value: serde_json::Value = serde_json::from_slice(stdout).ok()?;
    if !value.is_object() {
        return None;
    }
    serde_json::from_value(value).ok()
}

/// Trims the text and removes control characters (other than newlines and tabs) so a script
/// cannot put terminal escape sequences into a reason or notice. Empty results are dropped.
fn non_empty_text(text: &str) -> Option<String> {
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
