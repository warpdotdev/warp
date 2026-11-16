# Transcript Rehydration + `--conversation` Resume for Claude Code — Tech Spec
Product spec: `specs/REMOTE-1373/PRODUCT.md`
## Problem
Two related gaps for Claude Code cloud runs: (1) a fresh Claude sandbox resuming an existing conversation doesn't actually pick up the prior state — the stored `ClaudeTranscriptEnvelope` isn't rewritten into the on-disk layout `claude --resume` expects, and on cloud-to-cloud handoff the Oz-style rehydration system prompt gets deprioritized by Claude so workspace patches don't get applied; and (2) `--conversation <id>` is Oz-only, so there's no user-facing surface to resume a finished Claude conversation in a new cloud or local run.
## Architecture
```mermaid
sequenceDiagram
    participant User
    participant CLI as warp CLI
    participant Srv as warp-server
    participant Wrk as oz-agent-worker
    participant Sand as Sandbox CLI
    participant GCS
    participant Cld as claude
    User->>CLI: warp agent run --harness claude --conversation X
    CLI->>Srv: list_ai_conversation_metadata([X]) → CLAUDE_CODE
    Note over Wrk,Sand: Cloud-to-cloud followups (Slack/Linear) reach the sandbox via the existing\nworker→sandbox path; this PR only changes how the sandbox CLI consumes the id.
    Wrk->>Sand: oz agent run --task-id <t> --harness claude --conversation X --sandboxed
    Sand->>Srv: GET /harness-support/transcript (workload token → task.AgentConversationID) → signed URL
    Sand->>GCS: GET claude_code.json → ClaudeTranscriptEnvelope
    Sand->>Sand: write_envelope(cwd=<sandbox_cwd>) + sessions-index entry → ~/.claude/projects/...
    Sand->>Cld: claude --resume <uuid> --dangerously-skip-permissions < (resumption_prompt + prompt)
    loop per-turn
        Cld->>Srv: POST /harness-support/resolve-prompt
        Srv-->>Cld: prompt, system_prompt (+ rehydration body), resumption_prompt (preamble for the harness to surface on resumed runs)
        Cld->>Cld: Claude harness prepends resumption_prompt to the user-turn prompt before piping into the CLI
    end
    loop periodic + final
        Sand->>GCS: PUT <X>/claude_code.json + block_snapshot.json (overwrite)
    end
```
Local resume is the user's CLI doing the fetch, rehydrate, and launch directly. Cloud spawn-with-resume from the Rust CLI (`run-cloud --conversation`) is intentionally out of scope for this PR — see the follow-ups section.
## Implementation
### Client CLI — `warp-internal`
#### CLI arg shape
`RunAgentArgs` in `crates/warp_cli/src/agent.rs` accepts both `--task-id` and `--conversation` simultaneously. Eventually we want them mutually exclusive (`--task-id` already implies a server-side task whose `conversation_id` is the conversation to resume), but the worker still appends `--conversation <id>` alongside `--task-id` to the embedded CLI for Slack/Linear followups, so adding a `conflicts_with` on `--task-id` would break those during the rollout window. The enforcement is deferred to a follow-up that lands after the worker stops appending `--conversation`. When both are set, the runtime merge in `setup_and_run_driver` prefers the explicit `--conversation` value over the task's stored `conversation_id`.
#### Shared harness validation
`fetch_and_validate_conversation_harness` in `app/src/ai/agent_sdk/common.rs` fetches conversation metadata via `list_ai_conversation_metadata` and compares its `AIAgentHarness` against the caller's `warp_cli::agent::Harness`. A mismatch returns `AgentDriverError::ConversationHarnessMismatch { conversation_id, expected, got }` before any task/config side effects. The local path (`setup_and_run_driver` in `app/src/ai/agent_sdk/mod.rs`) calls it up front when `--conversation` is passed.
#### Effective conversation id resolution
`setup_and_run_driver` in `app/src/ai/agent_sdk/mod.rs` resolves the effective conversation id from up to two sources:
- `--conversation <id>`: validated up front (no task side effects on mismatch).
- `--task-id <id>`: `fetch_secrets_and_attachments` reads the task's `conversation_id` off the fetched `AmbientAgentTask` and returns it; the harness check against the task's stored harness happens inside `fetch_secrets_and_attachments`, before any conversation-load side effects.
When both are passed, the explicit `--conversation` value wins via `resume_conversation_id.or(task_conversation_id)`.
#### Harness-aware resume dispatch
`load_conversation_information` in `app/src/ai/agent_sdk/mod.rs` now takes the resolved `&HarnessKind` and dispatches:
- `HarnessKind::Oz` keeps the existing path: `get_ai_conversation` + `driver_options.conversation_restoration`.
- `HarnessKind::ThirdParty(h)` parses the id to `AIConversationId`, grabs an `Arc<dyn HarnessSupportClient>` via `ServerApiProvider::get_harness_support_client()`, and calls `h.fetch_resume_payload(&conversation_id, harness_support_client).await?`, stashing the returned `Option<ResumePayload>` on `driver_options.resume_payload`. The previous `ServerConversationToken` and `&ServerAIConversationMetadata` arguments are gone now that the server resolves the conversation from the task's `agent_conversation_id`.
`resume_payload` and `conversation_restoration` are mutually exclusive on `AgentDriverOptions`. `prepare_harness` in `app/src/ai/agent_sdk/driver.rs` takes the payload off `AgentDriver` via `me.resume_payload.take()` and forwards it to `ThirdPartyHarness::build_runner`.
#### Harness-agnostic resume payload
`app/src/ai/agent_sdk/driver/harness/mod.rs` defines a single harness-dispatched enum that the driver itself never inspects:
```rust path=null start=null
pub(crate) enum ResumePayload {
    Claude(ClaudeResumeInfo),
    // Future CLI harnesses add their own variant here.
}
```
`ResumePayload` is what `ThirdPartyHarness` implementations return from `fetch_resume_payload`. There is no harness-dispatched `TranscriptEnvelope` enum: each harness fetches raw bytes from `HarnessSupportClient::fetch_transcript` and deserializes them into its own envelope type directly, which keeps the abstraction local to each harness module.
`ThirdPartyHarness` is `#[async_trait]` and exposes the resume-shaped methods plus the build hook:
- `async fn fetch_resume_payload(&self, _conversation_id: &AIConversationId, _harness_support_client: Arc<dyn HarnessSupportClient>) -> Result<Option<ResumePayload>, AgentDriverError>` — defaults to `Ok(None)`; Gemini uses the default.
- `fn build_runner(&self, prompt: &str, system_prompt: Option<&str>, resumption_prompt: Option<&str>, working_dir: &Path, server_api: Arc<ServerApi>, terminal_driver: ModelHandle<TerminalDriver>, resume: Option<ResumePayload>) -> Result<Box<dyn HarnessRunner>, AgentDriverError>` — implementors match on their own variant and ignore others, and decide how to surface the optional `resumption_prompt`. The driver never inspects either argument; the abstraction ends at `build_runner` and each runner stays harness-shaped internally.
#### Claude Code
`app/src/ai/agent_sdk/driver/harness/claude_code.rs`:
- `ClaudeHarness::fetch_resume_payload` calls `harness_support_client.fetch_transcript()`, deserializes the bytes into `ClaudeTranscriptEnvelope` directly via `serde_json::from_slice`, maps a 404 (string match on `status 404`) to `AgentDriverError::ConversationResumeStateMissing { harness: "claude", conversation_id }`, and wraps the envelope into `ResumePayload::Claude(ClaudeResumeInfo { conversation_id, session_id: envelope.uuid, envelope })`.
- `ClaudeHarness::build_runner` destructures `resume.map(|ResumePayload::Claude(info)| info)` and, when `resumption_prompt` is non-empty, prepends `"{preamble}\n\n"` to the user-turn `prompt` before passing it to `ClaudeHarnessRunner::new`. Claude treats the user-turn message as immediate intent, so a local prepend at runner construction is the most reliable way to land the preamble; other harnesses can pick a different placement (or ignore it).
- `ClaudeHarnessRunner::new`, when resuming: rewrites `envelope.cwd = working_dir`, calls `write_envelope` under `claude_config_dir()`, calls `write_session_index_entry` (best-effort), reuses the envelope's session uuid, and stashes `Some(conversation_id)` on a new `preexisting_conversation_id` field. Jsonl-write failures return `AgentDriverError::ConfigBuildFailed` so the user gets a real error instead of a silent start-from-scratch.
- `claude_command(..., resuming: bool)` picks `--resume <uuid>` when resuming and `--session-id <uuid>` otherwise.
- `HarnessRunner::start` skips `create_external_conversation` when `preexisting_conversation_id.is_some()` and uses the stored id. `save_conversation` is unchanged; reusing `(conversation_id, session_id)` makes periodic/final saves overwrite the same GCS objects.
#### Claude transcript module
`app/src/ai/agent_sdk/driver/harness/claude_transcript.rs` is a sibling module that owns `ClaudeTranscriptEnvelope`, `ClaudeResumeInfo`, `encode_cwd`, `claude_config_dir`, `read_envelope`, `write_envelope`, and `write_session_index_entry`. It's extracted from `claude_code.rs` purely to keep the on-disk layout helpers separate from the runner; both modules import from it as needed. `write_envelope` lost its `#[expect(dead_code)]` — it's live now.
`write_session_index_entry(session_id, cwd, config_root)` upserts an entry keyed on the session uuid into `~/.claude/sessions-index.json` with `sessionId`, `cwd`, `projectPath` (= encoded cwd), and `transcriptPath`. Existing entries and unknown fields are preserved; missing/malformed files are created/overwritten. Best-effort: failures log warn and continue (upstream Claude versions vary on how they consume this index).
#### Transcript fetch client
`HarnessSupportClient::fetch_transcript` in `app/src/server/server_api/harness_support.rs` downloads from `GET harness-support/transcript` via `get_public_api_response`, reads bytes, and returns them to the harness. The conversation is resolved server-side from the current task's `agent_conversation_id`, so the call takes no parameters. The endpoint sits behind the harness-support workload-token middleware; only cloud-agent contexts can reach it. Transient retries reuse the shared `with_bounded_retry` helper from `agent_sdk::retry` (3 attempts, `500ms * 2^n` backoff), classifying 5xx / 408 / 429 as transient via `HttpStatusError` in the error chain. The previous `AIClient::get_transcript` trait method and the `server_api/claude_transcript.rs` module were removed; each harness owns deserialization for its own envelope shape.
#### Resolved-prompt response
`ResolvedHarnessPrompt` in `app/src/server/server_api/harness_support.rs` adds an optional `resumption_prompt: Option<String>` field (deserialized with `#[serde(default)]` so older servers that don't set it still parse cleanly). The driver in `prepare_harness` extracts the field alongside `prompt` and `system_prompt` and forwards it to `ThirdPartyHarness::build_runner`. Each harness picks how to surface it. Today only the Claude harness consumes it (prepended to the user-turn prompt); Gemini ignores it via `_resumption_prompt`.
#### Error variants
`AgentDriverError` in `app/src/ai/agent_sdk/driver.rs` gains two variants (both classified in `app/src/ai/agent_sdk/driver/error_classification.rs`):
- `ConversationHarnessMismatch { conversation_id, expected, got }` → `EnvironmentSetupFailed`.
- `ConversationResumeStateMissing { harness, conversation_id }` → `ResourceNotFound`. Harness-neutral on purpose; each harness tags the variant with its own label (`"claude"` today).
### Server — `warp-server`
- `router/handlers/public_api/harness_support.go` registers `GET /harness-support/transcript` on the existing harness-support group (which already runs `ValidateAmbientTask` + `RequireCloudAgent`) and implements `GetTranscriptDownloadHandler`: pulls `AmbientRequestInfo` off the gin context, 400s on missing `info.Task.AgentConversationID`, resolves the principal + conversation data store, and 307-redirects to the URL returned by `conversation_transcript.GetConversationRawTranscriptDownloadURL`. That underlying function already returns `InvalidRequestError` for non-`GenericCLIHarnessTranscript` manifests, so Oz conversations get a 400 for free. The previously-proposed `GET /agent/conversations/:conversation_id/third-party-transcript` route was removed in favor of this one so callers don't have to pass a conversation id and the route lives next to the rest of the harness-support endpoints.
- `public_api/openapi.yaml` adds the `get` operation under `/harness-support/transcript` (sibling of the existing `post` upload-target operation) with the standard error responses, and `ResolvePromptResponse` gains an optional `resumption_prompt` string with a doc comment explaining the contract. Types are regenerated into `public_api/types/types.gen.go` via `go generate ./public_api/types/`.
- `logic/ai/ambient_agents/handoff_rehydration.go` introduces `RehydrationAgentKind` (`RehydrationForOz` / `RehydrationForThirdPartyCLI`) and a second prompt body, `HandoffRestoreInstructionsForThirdPartyCLI`, that is ordered as an unconditional pre-turn checklist with explicit verbatim `cat` / `git apply` commands. It also exports `HandoffRestoreUserPromptPreambleForThirdPartyCLI`, a one-line user-turn nudge pointing Claude back at the system-prompt checklist. `ResolveHandoffRehydrationPrompt` and `formatHandoffRehydrationPrompt` take the kind and dispatch on it.
- `router/handlers/public_api/harness_support.go`'s `ResolvePromptHandler` calls `ResolveHandoffRehydrationPrompt(..., RehydrationForThirdPartyCLI)`. When the body is non-empty it appends the body to `systemPrompt` (via `appendSystemPromptSection`) and stores `HandoffRestoreUserPromptPreambleForThirdPartyCLI` in a new `resumptionPrompt` local that's returned in `api.ResolvePromptResponse.ResumptionPrompt`. The handler no longer mutates `prompt` server-side — each harness chooses how to surface the preamble (Claude prepends it; Gemini ignores). Old clients that don't deserialize `resumption_prompt` simply skip it and behave as before.
- `logic/ai/multi_agent/runtime/interceptors/input.go` passes `RehydrationForOz` so the Oz runtime keeps its softer UserQuery-style body.
- `logic/ai/ambient_agents/workers/selfhosted/websocket.go` drops the redundant `ConversationID` field from `TaskAssignmentMessage`; the worker now reads `task.AgentConversationID` directly off the embedded `*types.Task` (already serialized as `agent_conversation_id`).
- `test/integration/external_conversation_test.go` includes `TestGetConversationRawTranscriptDownloadURL_OzRejected` (referenced from the new handler's doc comment) covering the Oz 400 path that the handler relies on.
### Worker — `oz-agent-worker`
- `internal/types/messages.go` adds `AgentConversationID *string \`json:"agent_conversation_id,omitempty"\`` on `Task`. This replaces the removed `TaskAssignmentMessage.ConversationID` as the canonical conversation-id source for resumed runs.
- `internal/common/task_utils.go`'s `AugmentArgsForTask` appends `--conversation <id>` from `task.AgentConversationID` when set, so the embedded warp CLI can resume the conversation's state (Oz or Claude Code). The CLI accepts `--task-id` + `--conversation` together while the deferred `conflicts_with` migration is outstanding (see follow-ups); when both are present, the CLI's runtime merge prefers the explicit `--conversation`.
- `internal/worker/worker.go` stops reading the removed `assignment.ConversationID`; the CLI args are built entirely off `task.AgentConversationID` via `AugmentArgsForTask`.
## Feature flags
- `FeatureFlag::CloudConversations` gates `--conversation` on the CLI (unchanged).
- `FeatureFlag::AgentHarness` gates `--harness claude` and the Claude resume path (unchanged).
- `CloudToCloudHandoffEnabled` gates the server-side rehydration body + user-turn preamble in `ResolvePromptHandler`. Off → the resolved prompt is returned unmodified.
- Worker `AgentConversationID` is transport-level; inert when unset, so old workers + new server degrade silently to "no resume".
## Risks and mitigations
- **cwd mismatch**: `--resume` is scoped to `~/.claude/projects/<encoded_cwd>/`. The envelope's cwd is unconditionally rewritten to `working_dir` before `write_envelope`. Unit-tested.
- **`sessions-index.json`**: recent Claude versions key `--resume <uuid>` off the index, not by scanning jsonl directly (claude-code#33912, #39667, #5768). We upsert an entry alongside the jsonl, preserving other entries. Best-effort — index failures surface as normal resume errors, not rehydration aborts.
- **Weak system-prompt adherence on resumed Claude**: the stronger third-party-CLI body + user-turn preamble in `/resolve-prompt` is specifically to overcome Claude's baked-in prompt dominating on resumed sessions. The preamble lives in the new `resumption_prompt` response field; the harness decides where to inject it (Claude prepends it to the user-turn prompt fed into the CLI).
- **Old server + new client**: `GET /harness-support/transcript` returns 404; surfaced as a clear `ConversationResumeStateMissing` error.
- **Old client + new server (resumption_prompt)**: clients that don't deserialize the new `resumption_prompt` field simply ignore it; their resumed runs lose only the user-turn preamble nudge, not any rehydration behaviour.
- **Old worker + new server**: the server no longer emits the top-level `TaskAssignmentMessage.ConversationID`; old workers that read that field (instead of `task.AgentConversationID`) will stop appending `--conversation` and silently degrade to "no resume" on both fresh `--conversation` invocations AND pre-existing Slack/Linear follow-ups. Follow-ups still run, they just lose conversation continuity until self-hosted workers are rebuilt. Worth sequencing worker rollout ahead of server rollout.
- **Concurrent resumed runs**: last-write-wins on `<X>/claude_code.json`, same hazard as Oz `--conversation` today; not addressed here.
## Testing and validation
### Unit tests (warp-internal)
- `claude_code_tests.rs`: `--session-id` vs `--resume` flag selection, stdin-redirect + `--dangerously-skip-permissions`, resume writes envelope under current cwd, resume runner skips `create_external_conversation`, `fetch_resume_payload` happy path, `fetch_resume_payload` 404 → `ConversationResumeStateMissing`.
- `claude_transcript_tests.rs`: `encode_cwd`, `read_envelope` / `write_envelope` round-trips, `write_session_index_entry` create/preserve-others/overwrite-same-session/overwrite-malformed.
- `harness_support_tests.rs`: `HarnessSupportClient::fetch_transcript` envelope round-trip + transient-error retry.
- `mod.rs`: harness-mismatch pre-spawn (both directions), `HarnessKind::ThirdParty` populates `resume_payload`.
### Integration tests
- warp-server: `TestResolvePromptHandler_HandoffRehydrationNoPriorExecution` pins empty `prompt` / `system_prompt` / `resumption_prompt` when no prior ended execution exists; `TestGetConversationRawTranscriptDownloadURL_OzRejected` covers the Oz 400 path that the new `GET /harness-support/transcript` handler relies on; existing tests cover the upload side.
- oz-agent-worker: `AugmentArgsForTask` forwards `--conversation <id>` to the embedded CLI when `task.AgentConversationID` is set, and omits the flag otherwise.
### Manual
1. Short Claude cloud agent → note `<id>`.
2. `agent run --harness claude --conversation <id>` locally → jsonl grows, Claude `/resume` lists the session.
3. Harness mismatch / missing transcript / missing id → clean errors pre-launch.
## Follow-ups
- Wire up cloud spawn-with-resume from the Rust CLI: add `conversation_id: Option<String>` to `SpawnAgentRequest`, forward `args.conversation` from `run-cloud --conversation` (with the up-front `fetch_and_validate_conversation_harness` call inside `spawn_future`), and ship as part of the broader local→cloud handoff design. The server already accepts the field and the worker already forwards `task.AgentConversationID`, so the wiring is small; deferring keeps this PR focused on local resume + the worker/server transcript path.
- Add `conflicts_with = "conversation"` to `--task-id` in `RunAgentArgs` once the worker stops appending `--conversation` alongside `--task-id` for Slack/Linear followups. Until then, both flags can be passed; the runtime merge prefers the explicit `--conversation`.
- Add a second `ResumePayload` variant + per-harness `fetch_transcript` deserializer when another CLI harness (Gemini / Codex / opencode) gains resume support. The generic surface (raw-bytes fetch on `HarnessSupportClient`, harness-decided `resumption_prompt` injection) is already in place.
- Reconcile the duplicated `types.Task` fields between warp-server and oz-agent-worker.
- Retry/fallback semantics on `write_envelope` failure (today: hard error).
- Auto-detect `--harness` from metadata once harness reading moves below the conversation-fetch step in `build_merged_config_and_task` and `ambient.rs`.
