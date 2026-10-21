use std::sync::Mutex;

use serde::Deserialize;
use serde_json::json;

use crate::hooks::adapters::{
    DecodePolicy, DeclaredBinding, HookConfigError, HookFormatAdapter, ParsedHookConfig,
};
use crate::hooks::model::{
    ContinuationRequest, HookEffects, HookEnvelope, HookFormat, HookPayload, HookReason,
    HookSourceId, HookVerdict,
};
use crate::hooks::runner::{HookProcessOutcome, ProcessExit};

/// The JSON a scripted hook writes on stdout in tests.
#[derive(Debug, Default, Deserialize)]
struct TestHookOutput {
    #[serde(default)]
    verdict: Option<String>,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    notices: Vec<String>,
    #[serde(default)]
    halt: bool,
    #[serde(default)]
    continuation: Option<String>,
    #[serde(default)]
    replacement_input: Option<serde_json::Value>,
}

/// An adapter for tests. Matching compares the binding's matcher text to the canonical tool name
/// exactly; an absent, empty, or `*` matcher matches everything. Decoding treats exit code 2 as a
/// deny (stdout is the reason), exit code 0 with JSON stdout as a scripted verdict, and anything
/// else as no opinion.
#[derive(Default)]
pub struct RecordingAdapter {
    bindings: Mutex<Vec<DeclaredBinding>>,
    encoded: Mutex<Vec<HookEnvelope>>,
    decode_policies: Mutex<Vec<DecodePolicy>>,
}

impl RecordingAdapter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_bindings(bindings: Vec<DeclaredBinding>) -> Self {
        Self {
            bindings: Mutex::new(bindings),
            encoded: Mutex::new(Vec::new()),
            decode_policies: Mutex::new(Vec::new()),
        }
    }

    /// Every envelope passed to `encode_input`, in order.
    pub fn encoded_envelopes(&self) -> Vec<HookEnvelope> {
        self.encoded.lock().expect("adapter lock").clone()
    }

    /// The policy passed to each `decode_output` call, in order.
    pub fn decode_policies(&self) -> Vec<DecodePolicy> {
        self.decode_policies.lock().expect("adapter lock").clone()
    }

    pub fn encoded_count(&self) -> usize {
        self.encoded.lock().expect("adapter lock").len()
    }

    pub fn clear_encoded(&self) {
        self.encoded.lock().expect("adapter lock").clear();
    }
}

impl HookFormatAdapter for RecordingAdapter {
    fn format(&self) -> HookFormat {
        HookFormat::Cursor
    }

    fn parse(
        &self,
        _bytes: &[u8],
        source: &HookSourceId,
    ) -> Result<ParsedHookConfig, HookConfigError> {
        Ok(ParsedHookConfig {
            source: source.clone(),
            format: HookFormat::Cursor,
            document: serde_json::Value::Null,
            warnings: Vec::new(),
        })
    }

    fn bindings(&self, _config: &ParsedHookConfig) -> Vec<DeclaredBinding> {
        self.bindings.lock().expect("adapter lock").clone()
    }

    fn matches(&self, binding: &DeclaredBinding, envelope: &HookEnvelope) -> bool {
        let Some(matcher) = binding.matcher.as_deref() else {
            return true;
        };
        if matcher.is_empty() || matcher == "*" {
            return true;
        }
        match envelope.payload.tool_name() {
            Some(tool_name) => tool_name.as_str() == matcher,
            None => true,
        }
    }

    fn encode_input(
        &self,
        binding: &DeclaredBinding,
        envelope: &HookEnvelope,
    ) -> serde_json::Value {
        self.encoded
            .lock()
            .expect("adapter lock")
            .push(envelope.clone());
        let mut input = json!({
            "event": binding.vendor_event,
            "boundary": envelope.boundary().canonical_name(),
            "conversation_id": envelope.conversation_id.to_string(),
            "occurrence_id": envelope.occurrence_id.to_string(),
            "invocation_id": envelope.invocation_id.to_string(),
        });
        if let Some(action_id) = &envelope.action_id {
            input["action_id"] = json!(action_id.to_string());
        }
        if let Some(tool_name) = envelope.payload.tool_name() {
            input["tool_name"] = json!(tool_name.as_str());
        }
        if let HookPayload::ToolPre { action, .. } = &envelope.payload {
            input["tool_input"] = json!(action.to_string());
        }
        input
    }

    fn decode_output(
        &self,
        binding: &DeclaredBinding,
        outcome: &HookProcessOutcome,
        policy: &DecodePolicy,
    ) -> HookEffects {
        self.decode_policies
            .lock()
            .expect("adapter lock")
            .push(*policy);
        let stdout = String::from_utf8_lossy(&outcome.stdout).trim().to_owned();
        let reason = |text: String| HookReason {
            text,
            binding: Some(binding.id.clone()),
        };
        match outcome.exit {
            ProcessExit::Code(2) => HookEffects {
                verdict: HookVerdict::Deny,
                reasons: if stdout.is_empty() {
                    Vec::new()
                } else {
                    vec![reason(stdout)]
                },
                ..Default::default()
            },
            ProcessExit::Code(0) => {
                if stdout.is_empty() {
                    return HookEffects::default();
                }
                let Ok(output) = serde_json::from_str::<TestHookOutput>(&stdout) else {
                    return HookEffects {
                        verdict: HookVerdict::Deny,
                        reasons: vec![reason("malformed hook output".to_owned())],
                        ..Default::default()
                    };
                };
                let verdict = match output.verdict.as_deref() {
                    Some("allow") => HookVerdict::Allow,
                    Some("deny") => HookVerdict::Deny,
                    Some("ask") => HookVerdict::Ask,
                    Some("defer") => HookVerdict::Defer,
                    Some(_) | None => HookVerdict::NoOpinion,
                };
                HookEffects {
                    verdict,
                    reasons: output.reason.into_iter().map(reason).collect(),
                    replacement_input: output.replacement_input,
                    replacement_result: None,
                    model_context: Vec::new(),
                    user_notices: output.notices,
                    session_env: Vec::new(),
                    halt_turn: output.halt,
                    continuation: output.continuation.map(|text| ContinuationRequest {
                        text,
                        loop_limit: None,
                    }),
                }
            }
            ProcessExit::Code(_)
            | ProcessExit::Signal(_)
            | ProcessExit::TimedOut
            | ProcessExit::Cancelled
            | ProcessExit::SpawnFailed(_) => HookEffects::default(),
        }
    }
}
