# Structured apply-diff errors
## Summary
Move the LLM-facing wording of `edit_files` (apply-file-diffs) failures off the Warp client and onto warp-server. The client reports *what* failed as a structured, typed value on the wire; the server decides *how* that failure is worded to the agent, per harness/model. The agent keeps receiving an error at least as actionable as today, but the exact prose becomes server-controlled and changeable without a client release.
## Problem
Today the client renders the entire failure message and ships it as a single opaque string (`ApplyFileDiffsResult.Error.message`); warp-server copies that string into the conversation verbatim. Because the wording is baked into the client binary, any change — rephrasing the retry instruction, per-model or per-harness tuning, A/B testing, localization — requires a client release that rolls out over weeks.
The failure already has well-defined categories on the client (`DiffApplicationError`), plus per-search-block detail (the failed search text and the line range it was expected to match). All of that structure is flattened to text at the API boundary and never reaches the server. This mirrors reviewer feedback on PR #11841: the error should be a structured type at the API level so the harness owns the verbiage.
## Why this shape (and why `edit_files` specifically)
There are two existing patterns for tool-result errors in the codebase:
- **Client-rendered string.** `read_files` reports `ReadFilesResult.FailedRead { path, message }` where `message` is built on the client and forwarded unchanged. `edit_files` does the same today with a single `Error.message`. Wording stays frozen in the client.
- **Server-rendered from structure.** The server already owns per-model tool formatting (e.g. `anthropic_text_editor.go`, `openai_apply_patch.go`), so it is the natural place to word tool output per harness.
`edit_files` is the stronger case for server-side wording than `read_files`: its message is active retry-coaching ("make each failed search block match the file exactly, then retry") — a prompt-engineering surface we want to iterate on quickly and tune per model. This spec moves `edit_files` to the server-rendered pattern. `read_files`/`search_codebase` are intentionally left on the client-rendered pattern for now and called out as a convergence follow-up so the two paths do not diverge permanently.
## Goals
- The client communicates apply-diff failures as a structured, typed value rather than a prerendered sentence.
- The server owns the agent-facing wording and can change it independently of client releases, including per-harness/per-model variation.
- No regression: for every failure category, the agent-facing message stays at least as specific and actionable as the current client wording.
- Safe rollout: mixed client/server versions during deploy never produce a worse message than today, and never drop the failure.
## Non-goals
- Changing *which* situations count as apply-diff failures, or the diff-matching algorithm itself. This is purely about how an already-detected failure is represented and worded.
- Changing how successful edits are reported.
- Changing the client's own local display of failures (pane UI, SDK transcript). Those keep rendering locally; only the agent-facing wire form changes.
- Migrating `read_files`/`search_codebase` to the structured contract (tracked as a follow-up).
## Behavior
Consumers of this contract are (a) warp-server harness code, which receives the structured failure and produces the agent-facing message, and (b) the agent/LLM, which reads that message and retries.
### Failure categories
1. When the client cannot apply one or more requested edits, it reports a non-empty, ordered list of typed failures, preserving the order in which the client produced them.
2. Each failure is exactly one of the following categories. Each carries the data the server needs to word it; the server must render a useful message from that data alone, without re-deriving anything from a prerendered string.
   - **Unmatched diffs** — one or more search blocks for a file could not be located. Carries: the file path; a count of how many search blocks failed to match; and, for each unmatched search block, its search text, a truncation flag, and (when known) the 1-indexed inclusive line range it was expected to match.
   - **Changes already applied (no-op)** — the requested change for a file is already present. Carries: the file path. May co-occur with unmatched diffs for the same file.
   - **Missing file** — an edit targeted a file that does not exist. Carries: the file path.
   - **Read failed** — a target file exists but could not be read (I/O, permissions, remote connectivity). Carries: the file path.
   - **Already exists** — a file the agent tried to create already exists. Carries: the file path.
   - **Multiple create attempts** — more than one attempt to create the same file. Carries: the file path.
   - **Multiple rename attempts** — more than one attempt to rename the same file. Carries: the file path.
   - **Mutated a deleted file** — the request both deleted and edited the same file. Carries: the file path.
   - **No diffs applicable** — the request produced no applicable diffs. Carries no additional data.
   - **Remote file operations unsupported** — file read/edit is unavailable on the current remote session. Carries no additional data.
   - **Opaque** — a prerendered message the server surfaces verbatim. Used for failures that are not (yet) categorized — for example errors saving accepted files to disk — and as the carrier for messages produced by clients that predate this contract. Carries: the message text.
3. The agent-facing message for each category must communicate at least the same actionable information as the current client wording (the baseline to meet or beat):
   - Unmatched diffs: identify the file, instruct the agent to make each failed search block match the file exactly and retry, and enumerate the failed search blocks (with expected line numbers when known).
   - No-op: state that the changes to the file were already made.
   - Missing file: name the file and prompt the agent to check the path.
   - Read failed: state the file could not be read.
   - Already exists: state the file could not be created because it already exists.
   - Multiple create / multiple rename: state there can only be one attempt to create / rename the file.
   - Mutated a deleted file: state a deleted file cannot also be edited.
   - No diffs applicable: state that no diffs could be applied.
   - Remote unsupported: state the file read/edit tool is unavailable on this remote session and suggest a different tool.
   - Opaque: surface the carried message unchanged.
4. When multiple failures are reported, the agent-facing message presents all of them in the order received, visually separated so the agent can act on each. A single failure is presented without list decoration.
### Invariants
5. The agent-facing wording is determined entirely by the server. Two clients of different versions reporting the same structured failure to the same server version yield the same agent-facing message.
6. Changing apply-diff failure wording must not require a client change. A server-only change can alter the exact prose for any category except Opaque (whose text originates upstream).
7. Search-block text and file paths are user content. They must be marked sensitive wherever the failure is transmitted or stored, and must never be written to logs or telemetry in cleartext — including via a derived `Debug`.
8. The size of transmitted failure detail is bounded. Search-block text is capped client-side; the contract preserves a truncation indicator so the server can show that a block was shortened.
9. The agent must always receive a non-empty failure message when an apply-diff fails. No category may render to an empty string; an unrecognized category falls back to a generic non-empty message rather than emitting nothing.
### Compatibility (observable during rollout)
10. Old client + new server: the client sends only a prerendered message (Opaque). The agent-facing message equals that message — identical to today. No regression.
11. New client + old server: the server reads only the prerendered message the new client still includes alongside the structured data. The agent-facing message equals today's wording. No regression.
12. New client + new server: the server renders from the structured data and ignores the prerendered message. This is the only configuration in which server-controlled wording takes effect.
13. Reading back a persisted/historical failure must not error. A stored failure with only a prerendered message reconstructs as Opaque; one with structured data reconstructs into its category.
