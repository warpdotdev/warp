# Product Spec: Built-in `/review` slash command

**Issue:** [warpdotdev/warp#9606](https://github.com/warpdotdev/warp/issues/9606)
**Figma:** none provided

## Summary

Add a built-in `/review` slash command in Agent input that gathers the current working tree's uncommitted changes and asks the Agent for a concise code review focused on logic bugs, security concerns, performance, simplicity, reuse, and maintainability. The command becomes available whenever the user is in a Git repository with at least one tracked-file change (staged or unstaged) and the agent is enabled.

This complements — does not replace — Warp's existing **Code Review** panel and the existing `/open-code-review` slash command, which are oriented toward inspecting and managing diffs rather than generating AI feedback. `/review` is the proactive, agent-facing entry point users currently improvise with hand-written prompts and ad-hoc diff attachment.

## Problem

Per the issue: users who want AI feedback on their uncommitted changes today have to:

1. Manually prompt the Agent ("can you review my changes?") and rely on the agent inferring how to gather the diff, OR
2. Paste a diff into the conversation, OR
3. Build their own reusable skill or saved prompt.

This makes a high-value, frequently-repeated workflow undiscoverable. The `/agent`, `/fork`, `/open-code-review`, `/index`, `/init`, and `/pr-comments` commands all already live as built-in static commands. A `/review` command sits naturally in this set and short-circuits the manual-prompt pattern.

## Goals

- A user in a Git repo with uncommitted changes types `/review` in Agent input and gets a focused code review back as the next agent turn — no manual diff gathering, no template prompt to memorize.
- The command surfaces in the existing static-command palette (`/`-prefix dropdown) when the conditions are met, with an icon and short description matching today's `/open-code-review` UX.
- The command is discoverable from the same surface as every other built-in agent command (no new menu, no new keybinding required for V1).
- The diff payload is bounded — a runaway 100k-line refactor doesn't blow the agent's context window or the user's token budget.
- The command is gracefully unavailable when the preconditions (Git repo, AI enabled, has changes) aren't met, with a clear reason instead of a silent failure.

## Non-goals (V1 — explicitly deferred to follow-ups)

- **Reviewing committed changes / specific commits / branches.** V1 covers uncommitted changes (working tree + index). Commit-range review (`/review HEAD~3..HEAD`, `/review main..feature`) is a follow-up gated on the V1 prompt template proving useful.
- **Reviewing PRs from GitHub.** The existing `/pr-comments` command pulls PR review comments; `/review` is local-changes-focused. No overlap.
- **User-customizable review prompt template.** V1 ships a fixed prompt focusing on the issue's stated priorities (logic bugs, security, performance, simplicity, reuse, maintainability). A per-project override file is a follow-up.
- **Auto-applying suggested fixes.** `/review` produces feedback; applying it is the existing agent edit flow. No new auto-apply affordance.
- **Per-file review filtering** (`/review src/foo.rs`). V1 reviews the entire diff. File scoping is a follow-up.
- **Streaming / incremental review.** V1 sends the full diff in one agent turn. Streaming summary→deep-dive is a follow-up.

## User experience

### Invoking the command

1. User is in a Warp tab whose CWD is inside a Git repository, AI is enabled, and there is at least one uncommitted change (staged or unstaged) to a tracked file.
2. User types `/r` in Agent input. The static-command palette shows `/review` alongside other matching commands.
3. User hits Enter (or clicks). Warp:
   - Synchronously runs `git diff HEAD --no-color` (or equivalent) to gather staged + unstaged changes against `HEAD`.
   - If the diff is over the configured size cap (default 50k bytes ≈ 1k lines), Warp truncates per-file (longest files first) and includes a note in the prompt: *"This diff was truncated from `<n>` files / `<n>` bytes to fit the review budget."*
   - Constructs an agent prompt containing the (possibly truncated) diff and a fixed review template (see "Prompt template" below).
   - Submits the prompt as a normal agent turn, opening or reusing the agent conversation per existing convention.

### Subsequent turns

1. The agent's response renders inline in the conversation as it streams back, with the same affordances as any other agent turn (apply suggestions, copy snippets, follow-up questions).
2. The user can type a follow-up turn (`"focus on the changes in src/auth/"`) and the agent has the diff in conversation context — no need to re-run `/review`.

### Unavailability scenarios

The command should fail loudly and predictably, not silently:

1. **Not in a Git repository.** The command does not appear in the slash-command palette. Same surface today's `/open-code-review` already uses (`Availability::REPOSITORY`).
2. **In a Git repo but no uncommitted changes.** The command appears in the palette but is disabled with a tooltip *"`/review` needs at least one uncommitted change."* Selecting it (e.g. via keyboard) shows the same tooltip toast and does not submit a turn.
3. **AI disabled** (the user has turned AI off, or is offline / unauthenticated). Command does not appear; same surface today's `/index` and `/init` already use (`Availability::AI_ENABLED`).
4. **Diff exceeds the truncation cap by an order of magnitude** (e.g. ≥ 500k bytes, indicating a vendored-blob commit or generated-code dump). The command runs but the prompt notes truncation explicitly. Users can re-narrow scope with a follow-up; the V1 command does not pre-emptively refuse.

## Configuration shape

The setting lives under the existing **AI** group. Default values keep V1 ergonomics defensible without exposing the user to runaway cost:

```toml
[ai]
review_command_max_diff_bytes = 51200    # ~50k bytes, ~1k lines of diff
```

| Field | Default | Notes |
|---|---|---|
| `review_command_max_diff_bytes` | 51200 | Truncation cap. When the gathered diff exceeds this, Warp prefers shorter files and notes truncation in the prompt. Tunable so users on unusually large refactors can opt up; minimum 1024, maximum 1048576. |

V1 deliberately ships **no other knobs** — review focus list, prompt template, included file globs are all hardcoded. Each is a follow-up if the V1 command proves popular.

## Prompt template (fixed in V1)

The agent prompt assembled by `/review` is, byte-for-byte:

```
Please review the following uncommitted changes in this Git repository.

Focus on, in priority order:
1. Logic bugs (off-by-one, null/empty handling, race conditions, incorrect state transitions)
2. Security concerns (injection, missing input validation, secrets in code, unsafe deserialization)
3. Performance issues (obvious O(n²) hot paths, unbounded queries, sync I/O on hot paths)
4. Simplicity and reuse (existing helpers being re-implemented, dead branches, code that can be deleted)
5. Maintainability (poor naming, missing tests for new logic, public API drift)

Skip stylistic nits unless they materially hurt readability. Group findings by file and severity. If a finding is uncertain, say so.

If the diff is short, also briefly note what looks correct.

[BEGIN DIFF]
{diff_content}
[END DIFF]
```

Justification: the priority list mirrors the issue's stated focus (*"logic bugs, security concerns, performance problems, simplicity, reuse, and maintainability"*). The "skip nits" line keeps the V1 review tight enough to be useful in a single agent turn. The "if uncertain, say so" line avoids the most common AI-review failure mode (confidently-wrong feedback).

## Testable behavior invariants

Numbered list — each maps to a verification path in the tech spec:

1. With a Git repo open, AI enabled, and at least one tracked-file change (staged OR unstaged), `/review` is present and selectable in the slash-command palette.
2. With no Git repo OR no AI OR a Git repo with zero tracked-file changes, `/review` is either absent from the palette (no repo / no AI) or visibly disabled with a tooltip (no changes).
3. Selecting `/review` from the palette gathers diff via `git diff HEAD --no-color` (against `HEAD`, including staged and unstaged), constructs the fixed V1 prompt with the diff inlined, and submits it as a single agent turn.
4. When the gathered diff exceeds `ai.review_command_max_diff_bytes`, the prompt's diff content is truncated (longest files first), and the prompt body contains the literal substring `"This diff was truncated"`.
5. The diff content is wrapped between the literal markers `[BEGIN DIFF]` and `[END DIFF]` exactly once each.
6. The command's `Availability` flags include `REPOSITORY | AI_ENABLED` (matching the pattern set by `/init`).
7. If the user has uncommitted changes only in untracked (new) files — i.e. files Git does not yet know about — the command still gathers them (via `git diff HEAD --no-color --binary` plus the file contents for new files via the existing change-detection path).
8. Submitting `/review` records a telemetry event of the existing slash-command type with `command_name = "/review"`.
9. After the V1 turn completes, follow-up agent turns in the same conversation have the diff in conversation context (no special handling — this falls out of the existing agent turn flow).
10. The V1 command does not silently re-invoke or re-attach the diff on follow-up turns; it is a one-shot invocation that opens the conversation. Follow-ups are normal agent turns.

## Open questions

- **Untracked-file handling.** `git diff HEAD` does not include untracked (new) files. Does the V1 command include them? Recommend yes — they are part of "uncommitted changes" colloquially, and excluding them would silently drop new-file changes from review. Tech spec details the gathering path.
- **Conversation-routing semantics.** Does `/review` always start a *new* conversation (like `/agent`) or use the active conversation if one exists (like the bare prompt path)? Recommend reuse-active-or-create-new, matching the default agent prompt path. This keeps follow-ups attached without proliferating conversations.
- **Should the command surface a "Re-review" affordance** after diff changes? Recommend no for V1 — users can re-type `/review` if they want a fresh pass with new diff state. Avoids over-design for a feature whose usage we don't yet have data on.
