# Transcript Rehydration + `--conversation` Resume for Claude Code — Product Spec
Linear: [REMOTE-1373](https://linear.app/warpdotdev/issue/REMOTE-1373)
## Summary
Two capabilities for Claude Code cloud runs that together let a Claude session pick up where it left off:
1. **Transcript rehydration** (the foundation) — when a fresh Claude sandbox starts against an existing conversation, restore the prior transcript into `~/.claude/` on disk AND make Claude actually use it. The latter requires a Claude-specific server-side system-prompt body plus a user-turn preamble that overrides Claude's baked-in prompt, which previously caused resumed sessions to ignore saved state and effectively start over (including dropping any uncommitted workspace patches from cloud-to-cloud handoff).
2. **`--conversation <id>` for Claude Code** (the CLI surface) — lets a user deliberately trigger transcript rehydration against a finished Claude conversation, matching what `--conversation` does today for Oz. Works in both `run-cloud` and local `run`.
Transcript rehydration is the more important of the two: it's what makes cloud-to-cloud handoff actually survive on Claude, and `--conversation` is just the user-facing way to invoke it on demand.
## Problem
Cloud Claude runs upload their full state to GCS (`claude_code.json` transcript, `block_snapshot.json`, handoff workspace patches), but nothing on the read side picks it back up:
- The stored transcript is never rewritten into Claude's on-disk layout, so `claude --resume <uuid>` finds nothing and starts fresh.
- On cloud-to-cloud handoff, the existing Oz-style rehydration system prompt gets deprioritized relative to Claude's baked-in system prompt on resumed sessions, so Claude acknowledges the instructions and proceeds as if the workspace were reset — uncommitted changes are silently lost.
- `--conversation` is Oz-only, so a finished Claude conversation is effectively read-only.
## Goals
- **Transcript rehydration works on Claude** — whenever a Claude sandbox is spun up against an existing conversation (cloud-to-cloud handoff, or explicit `--conversation`), the prior transcript lands in `~/.claude/projects/<encoded_cwd>/<uuid>.jsonl` with subagents, todos, and a `sessions-index.json` entry, AND any workspace patches from the prior sandbox are applied before Claude answers the new user turn. The "AND" is the hard part: it requires server-side prompt changes that survive Claude's own system prompt.
- **`--conversation <id>` for Claude Code** — `warp agent run-cloud --harness claude --conversation <id> --prompt "..."` spawns a new cloud run that resumes the prior Claude session; `warp agent run --harness claude --conversation <id> --prompt "..."` does the same locally.
- **Saves continue in place** — follow-up periodic/final saves write to the same server conversation id, same GCS objects, same Warp Drive object, same artifacts list.
## Non-goals
- Transcript rehydration or resume support for third-party harnesses other than Claude Code. The abstraction is harness-agnostic (new CLIs add a `ResumePayload` variant + their own fetch override), but no other harness implements it today.
- Preserving the envelope's original cwd — we rewrite it to the new run's cwd so `claude --resume` finds the jsonl.
- Forking into a new conversation id, or a `--fork-session` branch.
- UI changes to the AI Conversation viewer.
## User experience
### Transcript rehydration (automatic)
Any Claude cloud run that resumes an existing conversation — whether via cloud-to-cloud handoff (REMOTE-1290) or via explicit `--conversation` — goes through the same rehydration path:
1. The sandbox CLI downloads the transcript envelope from `GET /harness-support/transcript`. The endpoint runs under the harness-support workload-token middleware and resolves the conversation server-side from the current task's `agent_conversation_id`, so the caller doesn't pass a conversation id.
2. Envelope's `cwd` is rewritten to the current working directory; main jsonl, subagent jsonls, per-agent todos, and a `sessions-index.json` entry are written under `~/.claude/projects/<encoded_cwd>/...`.
3. Claude is launched with `--resume <session_id>` so it picks up the on-disk transcript.
4. On every turn, `/harness-support/resolve-prompt` appends a third-party-CLI-strength rehydration body to Claude's system prompt and returns a one-line user-turn preamble in the new `resumption_prompt` response field. The Claude harness prepends that preamble to the user-turn prompt it pipes into the CLI, so Claude executes the pre-turn checklist (apply workspace patches, verify dirty files) before answering. Other harnesses can ignore `resumption_prompt`; old clients that don't know the field is there fall through to the unmodified prompt.
5. Periodic and final saves overwrite `<id>/claude_code.json` and `<id>/block_snapshot.json` in GCS so the AI Conversation viewer shows the merged state and artifacts stay attached.
### `--conversation <id>` invocation
```
warp agent run-cloud --harness claude --conversation <id> --prompt "follow-up"
warp agent run       --harness claude --conversation <id> --prompt "follow-up"
```
`--harness claude` is required when resuming a Claude conversation; the default `--harness oz` against a Claude id fails fast with an actionable error. The CLI never silently flips harness mid-flight because harness drives pre-load decisions (task config, CLI validation, server task creation).
The client validates `--harness` against the conversation's stored harness before any task is created, then runs the same rehydration path described above. Local runs skip step 4 (the server's rehydration prompt is a no-op because there's no prior ended execution) but still rehydrate the transcript to disk so Claude's `/resume` picker sees it.
### Error and edge cases
- Non-existent or inaccessible conversation id: fail fast, no task created.
- Harness mismatch (either direction): fail fast with a message naming both sides, e.g. `conversation X was produced by the claude harness, but --harness oz was requested`. No task created.
- Claude conversation with no stored transcript: `conversation <id> has no stored transcript for the claude harness. The prior run may have crashed before saving any state`.
- Transient transcript fetch failures: bounded exponential backoff inside `HarnessSupportClient::fetch_transcript`; permanent 4xx fails fast.
- `claude --resume` failing at runtime (e.g. upstream session-index desync): surface the error and exit non-zero instead of silently starting a fresh session.
- `--conversation` without a prompt/skill: same rejection as today's `has_prompt_source` check.
- In-progress prior run: no special handling — same as Oz `--conversation`.
### Feature-flag gating
- `FeatureFlag::CloudConversations` off → `--conversation` hidden and rejected (unchanged).
- `FeatureFlag::AgentHarness` off → `--harness claude` rejected; the Claude transcript-rehydration client path is also gated on it.
- `CloudToCloudHandoffEnabled` off → server returns the resolved prompt without the rehydration body or user-turn preamble (unchanged gate).
## Success criteria
- **Transcript rehydration**: a Claude cloud run whose sandbox is replaced mid-run resumes with its prior transcript visible to Claude AND its uncommitted workspace patches applied before Claude answers the next turn — verified by the next turn's tool calls / git status, not just by the agent's acknowledgement text.
- **Conversation resuming**: `run-cloud --conversation <id> --prompt "..."` appends to the same AI Conversation in the UI and overwrites `<id>/claude_code.json` / `block_snapshot.json` in place; `agent run --conversation <id>` locally grows `~/.claude/projects/<encoded_cwd>/<uuid>.jsonl` with the prior entries before the new prompt runs.
- New PR / plan / file artifacts from the resumed run attach to the same Warp Drive conversation object as the original.
- Invalid inputs fail cleanly pre-launch with no side effects.
## Validation
- **Transcript rehydration (cloud handoff)**: force a sandbox replacement on a Claude run with uncommitted changes; after handoff, confirm from the next-turn tool calls that `git apply` ran on the expected patches and that the files the patches touched are dirty.
- **Conversation resuming (cloud)**: `run-cloud --harness claude --conversation <id>` with a follow-up; UI merges the conversation, prior uncommitted files reappear, saves overwrite in place.
- **Conversation resuming (local)**: `agent run --harness claude --conversation <id>` locally; `~/.claude/projects/<encoded_cwd>/<uuid>.jsonl` grows and Claude's `/resume` picker lists the session.
- **Errors**: harness-mismatch id, missing-transcript id, non-existent id all fail cleanly pre-launch.
- **Unit tests**: `--resume` vs `--session-id` flag selection, envelope cwd rewrite, session-index upsert, `HarnessSupportClient::fetch_transcript` retry behavior, harness-mismatch + resume-state-missing error paths.
- **Integration tests**: `GET /harness-support/transcript` (Claude 307, Oz 400, no-conversation 400, unauthed 403); worker appends `--conversation` to CLI args when set.
## Resolved decisions
- Transcript rehydration runs on any resumed Claude sandbox, not just `--conversation`; the same code path serves cloud-to-cloud handoff.
- Third-party-CLI rehydration prompt is stronger than Oz's and is also echoed as a user-turn preamble — on resumed Claude sessions the system prompt alone gets treated as background instead of pre-turn action.
- Same conversation id, in place (matches Oz `--conversation`).
- Explicit `--harness claude` required; no auto-detect from metadata.
- Reuse the envelope's Claude `session_id` so the transcript grows linearly instead of fragmenting across saves.
- Rewrite `envelope.cwd` silently (cloud sandboxes routinely change cwd).
- Rehydrate `~/.claude/sessions-index.json` alongside the jsonl so `--resume <uuid>` lookups succeed on recent Claude versions (upstream bugs claude-code#33912, #39667, #5768).
