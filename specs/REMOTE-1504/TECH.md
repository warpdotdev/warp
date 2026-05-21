# REMOTE-1504: Save and Upload Codex Conversation Transcript

## Context
Cloud agent runs using the Codex harness already upload block snapshots on each save, but the session transcript (the JSONL rollout Codex writes to disk) was not being captured. Claude Code already does this — `claude_code.rs` calls `upload_transcript` alongside `upload_current_block_snapshot` via `futures::try_join!`, reading the session JSONL from `~/.claude/projects/…/<uuid>.jsonl`. Codex stores its rollouts differently: `~/.codex/sessions/YYYY/MM/DD/rollout-<ts>-<uuid>.jsonl`, so transcript capture needs its own envelope format and file-discovery logic.

### Relevant files
- `app/src/ai/agent_sdk/driver/harness/codex.rs` — `CodexHarnessRunner` impl, owns the per-run state and `save_conversation`
- `app/src/ai/agent_sdk/driver/harness/claude_code.rs (504-528)` — Claude's `upload_transcript`, the pattern being mirrored
- `app/src/ai/agent_sdk/driver/harness/claude_transcript.rs` — `ClaudeTranscriptEnvelope`, `read_envelope`, `read_jsonl`
- `app/src/terminal/cli_agent_sessions/mod.rs` — `CLIAgentSessionsModel`, singleton that tracks CLI agent session context including `session_id`
- `app/src/ai/agent_sdk/driver/harness/mod.rs` — `HarnessRunner` trait, `upload_current_block_snapshot`, `handle_session_update`

## Proposed changes

### New module: `codex_transcript.rs`
Parallel to `claude_transcript.rs`. Contains:

- **`CodexTranscriptEnvelope`** — on-wire JSON shape: `{ cwd, session_id, codex_version?, entries }`. Simpler than Claude's envelope (no subagents/todos). `entries` is the parsed JSONL content.
- **`CodexSessionMetadata`** — `{ cwd, codex_version }` extracted from the first JSONL line (`SessionMeta`). Cached via `OnceLock` on the runner so subsequent saves skip reparsing.
- **`codex_sessions_root()`** — resolves `$CODEX_HOME/sessions` or `~/.codex/sessions`.
- **`find_session_file(sessions_root, session_id)`** — walks `YYYY/MM/DD/` dirs looking for `rollout-*-<uuid>.jsonl`. Returns `Ok(None)` when root doesn't exist or no match found. The walk is unavoidable since Codex names files with timestamps we don't control; acceptable cost on cloud agents where the sessions dir is small. The path is cached on the runner after first discovery.
- **`parse_session_meta(first_entry)`** — pulls `cwd` and `cli_version` from the first JSONL entry's `payload` object. Constant for session lifetime so callers cache the result.

Reuses `read_jsonl` from `claude_transcript` for the actual JSONL parsing.

### Changes to `CodexHarnessRunner` (`codex.rs`)
Three `OnceLock` fields added for lazy, set-once caching:
- `session_id: OnceLock<Uuid>` — captured from `CLIAgentSessionsModel` when hooks emit `SessionStart`
- `transcript_path: OnceLock<PathBuf>` — resolved by `find_session_file` on first save, cached thereafter

This caching pattern differs from Claude Code, which re-reads the config dir every save. Done here for consistency with the immutable-once-set nature of Codex's `SessionMeta` line, and because the `YYYY/MM/DD` dir walk is more expensive than Claude's direct path lookup.

**`handle_session_update`** — new override. Reads session ID from `CLIAgentSessionsModel` (the singleton that receives events from the Codex hooks plugin). Parses the string into a `Uuid` and stores it in the `OnceLock`. No-ops once set. The session ID is needed to find the rollout file.

**`save_conversation`** — now runs `upload_current_block_snapshot` and `upload_transcript` concurrently via `futures::try_join!`, matching Claude's pattern. `upload_transcript` is a standalone async fn that:
1. Returns early if session ID or transcript path aren't available yet (early periodic saves before hooks fire)
2. Reads + parses JSONL in `spawn_blocking`
3. Uses cached metadata or parses it from the first entry
4. Builds `CodexTranscriptEnvelope`, serializes, uploads via `get_transcript_upload_target` + `upload_to_target`
5. Returns newly-parsed metadata (if any) so the caller can cache it

### Design note: dir walk vs timestamp-based path
Codex filenames embed `rollout-<ts>-<uuid>.jsonl`. An alternative to the walk would be computing the expected `YYYY/MM/DD` from the session start time. Rejected because timezone/midnight-boundary bugs make it fragile — a session starting at 23:59 local might land in tomorrow's dir depending on Codex's clock handling. The walk is safe and runs once per session.

## Testing and validation
- `codex_transcript_tests.rs` — unit tests covering:
  - `codex_sessions_root` honors `$CODEX_HOME` env var override
  - `find_session_file` walks a synthetic `YYYY/MM/DD` tree and matches the right UUID
  - `find_session_file` returns `None` for non-matching UUID
  - `find_session_file` returns `None` when root is missing
  - `read_envelope` round-trip: writes a synthetic rollout with `SessionMeta` + event lines, recovers `cwd`, `codex_version`, correct entry count
  - `read_envelope` returns `None` for missing session
- Manual: run a cloud agent with `--harness codex`, verify transcript appears in GCS after save
