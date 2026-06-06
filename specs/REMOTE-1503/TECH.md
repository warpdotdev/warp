# REMOTE-1503: Conversation Resumption for Codex

## Context
Cloud agent runs using the Codex harness need conversation resumption, matching what already exists for Claude Code. The Claude resumption flow — `fetch_resume_payload` → `build_runner` with `ResumePayload` → rehydrate transcript to disk → launch CLI with resume flag — is the template.

Key differences from Claude:
- Codex resumes via `codex resume <session_id> <prompt>`, not `--resume <uuid>`. There is no `--session-id` equivalent for fresh sessions; codex assigns its own UUID on first run.
- Codex's session files live under `~/.codex/sessions/YYYY/MM/DD/rollout-<ts>-<uuid>.jsonl` (vs Claude's `~/.claude/projects/<encoded_cwd>/`). The YYYY/MM/DD path is derived from the `SessionMeta` timestamp.
- Codex doesn't have a `sessions-index.json` equivalent.

Relevant files:
- `app/src/ai/agent_sdk/driver/harness/mod.rs` — `ResumePayload` enum, `ThirdPartyHarness` trait, shared `fetch_transcript_envelope`
- `app/src/ai/agent_sdk/driver/harness/codex.rs` — `CodexHarness`, `CodexHarnessRunner`, `codex_command()`
- `app/src/ai/agent_sdk/driver/harness/codex_transcript.rs` — `CodexTranscriptEnvelope`, `CodexSessionMetadata`, session file I/O
- `app/src/ai/agent_sdk/driver/harness/claude_code.rs` — reference implementation for resume flow
- `app/src/ai/agent_sdk/driver/harness/json_utils.rs` — shared `entries_to_jsonl`
- `app/src/ai/agent_sdk/driver/harness/claude_transcript.rs` — previously owned `entries_to_jsonl`

## Proposed Changes

### 1. Shared `fetch_transcript_envelope<E>` (mod.rs)
Extract the fetch-and-deserialize logic from `ClaudeHarness::fetch_resume_payload` into a generic `fetch_transcript_envelope<E: DeserializeOwned>(harness_label, conversation_id, client)` in `mod.rs`. Both harnesses call it with their envelope type; the 404→`ConversationResumeStateMissing` and parse-error→`ConversationLoadFailed` mapping lives once.

### 2. Shared `entries_to_jsonl` (json_utils.rs)
Move `entries_to_jsonl` from `claude_transcript.rs` into `json_utils.rs` so both the Claude and Codex transcript modules can use it without duplication.

### 3. `ResumePayload::Codex` variant + type-safe extraction (mod.rs)
Add `Codex(CodexResumeInfo)` to `ResumePayload`. Add `TryFrom<ResumePayload>` impls for both `ClaudeResumeInfo` and `CodexResumeInfo` that return `AgentDriverError::InvalidRuntimeState` on variant mismatch — replaces the inline `match` in `ClaudeHarness::build_runner`.

### 4. `CodexResumeInfo` + `session_start_timestamp` (codex_transcript.rs)
New `CodexResumeInfo` struct: `{ conversation_id, session_id, envelope }`. Add `session_start_timestamp: Option<DateTime<Utc>>` to `CodexTranscriptEnvelope` and `CodexSessionMetadata`, parsed from the `SessionMeta` line's `timestamp` field. Used to reconstruct the YYYY/MM/DD directory path when writing back to disk.

### 5. `write_envelope` (codex_transcript.rs)
New function that writes the envelope's JSONL entries back to `<sessions_root>/YYYY/MM/DD/rollout-<ts>-<uuid>.jsonl`. Timestamp comes from `session_start_timestamp`; falls back to `Utc::now()` if absent (codex's lookup is by UUID so the path doesn't need to be exact).

### 6. `CodexHarness::fetch_resume_payload` (codex.rs)
Override `fetch_resume_payload` on `CodexHarness` using the shared `fetch_transcript_envelope::<CodexTranscriptEnvelope>`, wrapping the result into `ResumePayload::Codex`.

### 7. `CodexHarnessRunner` resume path (codex.rs)
In `build_runner`:
- Extract `CodexResumeInfo` via `TryFrom`
- Prepend `resumption_prompt` to the user prompt (mirrors Claude)

In `CodexHarnessRunner::new`, when `resume` is `Some`:
- Write envelope to disk via `write_envelope`
- Pre-populate `session_id`, `transcript_path`, and `session_metadata` `OnceLock`s from the envelope
- Store `preexisting_conversation_id`

### 8. `codex_command` resume subcommand (codex.rs)
`codex_command` gains `session_id: Option<&Uuid>`. When `Some`, emits `codex resume --dangerously-bypass-approvals-and-sandbox <uuid> "$(cat '<prompt>')"`. When `None`, same as before.

### 9. `HarnessRunner::start` reuses conversation ID (codex.rs)
If `preexisting_conversation_id` is set, skip `create_external_conversation` and reuse it. Matches the Claude runner pattern.

## Testing and Validation

### Unit tests (codex_tests.rs)
- `codex_command_with_session_id_invokes_resume_subcommand` — verifies `resume` subcommand format
- `fetch_resume_payload_maps_404_to_resume_state_missing` — mock returns 404, assert `ConversationResumeStateMissing` with harness `"codex"`
- `fetch_resume_payload_maps_other_errors_to_load_failed` — mock returns generic error, assert `ConversationLoadFailed`
- `fetch_resume_payload_returns_codex_variant_on_success` — round-trip envelope through mock, assert `ResumePayload::Codex` fields

### Unit tests (codex_transcript_tests.rs)
- `write_envelope_uses_session_meta_timestamp_for_path` — writes to `2026/04/30/rollout-…-<uuid>.jsonl`
- `write_envelope_round_trip_preserves_entries` — write then `read_envelope`, assert entries match
- `write_envelope_falls_back_to_today_when_timestamp_missing` — no timestamp → still findable by UUID via `find_session_file`
