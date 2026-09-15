# Tech Spec: Resume CLI-agent conversations per project after restart

**Issue:** [warpdotdev/warp#14720](https://github.com/warpdotdev/warp/issues/14720)

**Product spec:** [`specs/GH14720/product.md`](product.md)

**Provenance:** documents the implementation from [#14734](https://github.com/warpdotdev/warp/pull/14734)
by @samithaj (closed by stale-PR automation before code review), forward-ported to current
`master`. Sections describe the code as it exists on this branch.

## Context

Before this change, CLI-agent session state lived only in memory:
`CLIAgentSessionsModel` (`app/src/terminal/cli_agent_sessions/mod.rs`) tracks a session from the
plugin's OSC events and drops it when the agent exits. Panes have a durable identity already —
`terminal_panes.uuid` in SQLite — but nothing ties an agent session to it. The tab sidebar
(`app/src/workspace/view/vertical_tabs.rs`) lists tabs, not conversations, and has no notion of
a project.

Claude Code keeps its own durable state in `~/.claude/projects/<encoded-cwd>/*.jsonl`, one
transcript per session; `app/src/ai/agent_sdk/driver/harness/claude_transcript.rs` already knows
this layout for the agent-sdk driver.

## Architecture

Three layers with an explicit authority split, plus the rail UI:

### 1. Durable session handles (`agent_session_handles` table)

- Migration `crates/persistence/migrations/2026-08-04-000000_add_agent_session_handles/`:
  columns `(agent, session_id, cwd, pane_uuid, title, created_at, last_seen_at)` with two
  partial unique indexes that encode the state machine —
  `(agent, session_id) WHERE session_id IS NOT NULL` (task identity: one row per upstream
  session, whichever pane resumed it) and `(pane_uuid, agent) WHERE session_id IS NULL` (at most
  one in-flight, not-yet-identified launch per pane per agent). The table header documents it as
  a **rebuildable index**: the agent's transcript directory is the source of truth.
- Writes go through the existing SQLite writer thread as `AgentSessionHandleOp`s
  (`app/src/persistence/agent_session_handles.rs`); rows are recorded at spawn from
  `CLIAgentSessionsModel`, which learned a `pane_uuids: HashMap<EntityId, Vec<u8>>` registry so
  handles key on the durable pane uuid, never the per-run `EntityId`.
- `AgentSessionHandlesModel` (`app/src/terminal/cli_agent_sessions/handle_store.rs`) is the
  UI-thread read model: hydrated once at startup (`initialize_app` in `app/src/lib.rs` passes
  `sqlite_data.agent_session_handles` through), then kept consistent by applying every op that
  is enqueued for the writer. It tracks identified handles only; in-flight rows always belong
  to a live pane the rail already shows as a live task.

### 2. Transcript discovery and naming (Claude Code)

- `session_scan.rs` scans `~/.claude/projects/<encoded-cwd>/*.jsonl` for the directories of
  open projects; a transcript's existence *is* the session (`ScannedSession`,
  `ClaudeSessionScanModel::refresh`). UUID-shaped filename filtering excludes the `<uuid>/`
  subdirectories the project dir also contains.
- `transcript_naming.rs` derives name candidates per transcript. The subtlety (measured on a
  real corpus, documented in the module header): Claude Code's `/rename` **broadcasts** the new
  title into other live sessions' transcripts under the file's own `sessionId`, so no per-record
  check can spot contamination. The rule that works is **uniqueness across sessions**, not
  recency: `TranscriptNames::resolve` accepts a title only when `is_unique` confirms no other
  session claims it (`TitleClaims` in `session_scan.rs` provides the cross-session view). All
  reads are seek-bounded (head/tail windows), never whole-file.
- **`encode_cwd` fix** (`claude_transcript.rs`): the old encoding replaced only `/` and `.`,
  but Claude mangles every non-alphanumeric character. Measured against 47 populated project
  dirs the two rules disagreed on 18 — session envelopes were written where Claude never reads,
  silently breaking `claude --resume` for any path containing `_` or a space. Fixed to match
  Claude's rule; standalone regression value even without the rest of this PR.

### 3. Authority split (invariant, enforced structurally)

- The **handle store owns pane binding**: only it knows which `terminal_panes.uuid` ran a
  session, so only it can resume *in place* — and only when both pane uuid **and** cwd still
  match (panes `cd` away).
- The **scan owns names and existence** of unwitnessed sessions. A scanned session has no pane
  binding to look up, so the resume path cannot fabricate one: scanned rows
  (`DormantTaskOrigin::Scanned`) always resume into a fresh tab at the scanned directory.
- Join key between the two sources is the session UUID.

### 4. Projects × Tasks rail

- `project_key.rs`: `ProjectKey` keys a project by the repo's shared (common) `.git` directory
  via the existing `repo_metadata` detection, so every worktree of a repo collapses into one
  project; no new git plumbing.
- `project_layout.rs`: `ProjectLayout::compute` is a pure projection of the open tabs (plus
  handles and scanned sessions via `compute_with_handles`) into `ProjectEntry` groups with
  live/dormant/discovered `DormantTask` rows; the sidebar rail, top tab bar, and navigation all
  consult this single projection rather than filtering `Workspace::tabs` in place.
- Rendering in `workspace/view/vertical_tabs.rs`; row titles via `workspace/tab_title.rs`;
  actions `WorkspaceAction::{SelectProject, ActivateTaskByPaneGroupId, ResumeDormantAgentTask}`
  (`workspace/action.rs`) — the resume action carries `(agent, session_id)` so it cannot go
  stale if the handle list reorders between paint and click.
- Resume commands: `terminal/cli_agent_resume.rs` — `resume_command` / `continue_command`
  return `Some` only for agents with a verified verb (Claude, Codex, Cursor CLI, OpenCode);
  variants are matched exhaustively so a new `CLIAgent` forces a decision; `is_valid_session_id`
  gates ids before they are embedded in a command. The command is prefilled, never executed.
- Settings (`workspace/tab_settings.rs`, surfaced in `settings_view/appearance_page.rs`):
  `UseProjectLayout` ("Group sessions by project"), `RailShowTasks` ("List tasks in the project
  rail"), `RailTaskInfo` ("Task rows show"), `TabLineCount` ("Show two lines on tabs").
- Flags (`crates/warp_features/src/lib.rs`): `ResumeProjectTasks` (dogfood; depends on
  `Projects`) gates handles/scan/resume; `LazyShellStartup` (dogfood) defers a restored tab's
  shell spawn until the tab is opened, so restoring a window full of tabs doesn't start every
  shell at once.

## End-to-end flow

1. User runs `claude` in a tab → `CLIAgentSessionsModel` records an in-flight handle
   `(pane_uuid, cwd, agent)`; the plugin reports the session id → the row is identified and the
   `(agent, session_id)` identity is upserted; the mirror updates and the rail shows a live task.
2. The agent exits → the session leaves `CLIAgentSessionsModel`, but the handle remains; the
   rail row turns dormant, named from the handle title or the transcript.
3. User clicks the dormant row → pane uuid + cwd still match → the same pane gets
   `claude --resume <id>` prefilled. Pane gone or cwd changed → fresh tab at the recorded cwd.
4. Warp restarts → handles hydrate from SQLite into the mirror; restored tabs defer shell spawn
   (`LazyShellStartup`); the scan lists unwitnessed sessions for open projects; rows resolve
   names tail-first with the uniqueness rule.

## Forward-port notes (this branch vs. the original PR)

- 3 call sites of `TerminalPane::new` added upstream since August now pass the two new
  durable-session args (`app/src/pane_group/child_agent/hydration.rs`).
- `CLIAgent::Grok` (new upstream) added to the exhaustive resume/continue matches as
  non-resumable, per the module's "adding a variant forces a decision" convention.
- Upstream's `OrchestrationUnifiedStack` restoration structure kept in
  `pane_group/child_agent/restoration.rs`.
- The original branch's test-only OSS flag enable in `app/src/bin/oss.rs` was dropped.

## Testing and validation

Unit tests (all in-tree, run with `cargo nextest run -p warp`):
- `transcript_naming_tests.rs` — naming cascade: tail-rename-wins, head tier, injected-wrapper
  and `Caveat:` rejection, junk-name rejection, UUID filename filter.
- `session_scan_tests.rs` — discovery, ordering, seek-bounding.
- `handle_store_tests.rs` / `agent_session_handles_tests.rs` — mirror ops and store CRUD
  against the partial unique indexes.
- `project_key_tests.rs`, `project_layout_tests.rs`, `tab_title_tests.rs`,
  `cli_agent_resume_tests.rs`, `claude_transcript_tests.rs` (encode_cwd), plus
  `pane_group/mod_tests.rs` hydration-decision and `LazyShellStartup` restore tests.

Manual (from the original PR, re-verified on this forward-port): rail rows validated against
real transcripts; resume clicked on a dormant row after agent exit and after full restart;
a mid-session `/rename` picked up.

## Risks and mitigations

- **Unreviewed origin.** The original PR never received code review (only the readiness-label
  gate). This spec + the revived PR route it through the normal Oz + SME review.
- **Scan cost.** Bounded: only directories of open projects are scanned, reads are
  seek-bounded, and results are cached in a model refreshed explicitly.
- **Naming heuristics.** The uniqueness rule is justified by measured data in the module docs;
  worst case is a fallback to a less specific name, never a wrong pane binding.
- **Schema.** Additive migration with a `down.sql`; the table is documented as rebuildable.

## Follow-ups

- Discovery for agents beyond Claude Code once their on-disk formats are verified.
- Retention/aging policy for old handles and scanned sessions.
- Optional split into three PRs (store+resume, discovery+naming, rail UI) if maintainers prefer
  smaller reviews.
