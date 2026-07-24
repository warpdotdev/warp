# tech.md — CLI agent working-directory changes update the block's directory metadata

Issue: https://github.com/warpdotdev/warp/issues/10031
Product spec: `specs/GH10031/product.md`

## Context

There are two independent "current working directory" tracks in the terminal,
and the CLI-agent track never reaches the block that owns the displayed
directory. All file references are at HEAD on `master`.

### Track 1 — shell / OSC 7 → block directory (works today)

A shell (or a program it launches) reports its directory via an OSC 7
`file://host/path` escape sequence in the PTY stream:

- `app/src/terminal/model/ansi/mod.rs:787` — `osc_dispatch`, the single OSC
  dispatch point.
- `app/src/terminal/model/ansi/mod.rs:839` — the OSC `7` arm; rejoins params,
  calls `parse_osc_7_cwd`, then `self.handler.set_current_working_directory(path)`.
- `app/src/terminal/model/ansi/mod.rs:156` — `parse_osc_7_cwd` / `osc_7_host_is_local`:
  the payload is honored only if its host matches the local hostname; empty host
  and `localhost` are rejected. This gate exists because OSC 7 arrives as
  untrusted bytes in the PTY stream.
- `app/src/terminal/model/terminal_model.rs:2889` —
  `<TerminalModel as ansi::Handler>::set_current_working_directory`: early-returns
  when `self.is_ssh_block()` (`terminal_model.rs:2428`), then routes to
  `self.block_list.set_current_working_directory(path)` (deliberately bypassing
  the `delegate!` alt-screen macro).
- `app/src/terminal/model/block.rs:3417` — `Block::set_current_working_directory`:
  dedupes against the current `pwd`, sets `self.pwd`, and emits
  `Event::BlockWorkingDirectoryUpdated`.
- `app/src/terminal/view.rs:12593` — the view consumes
  `ModelEvent::BlockWorkingDirectoryUpdated`, calls `apply_block_metadata_update`
  and refreshes the `WorkingDirectory` prompt chip.
- `app/src/terminal/view/tab_metadata.rs:16` — `display_working_directory` reads
  **only** the `WorkingDirectory` prompt chip or the block `pwd()`. This is what
  the tab subtitle, git-branch chip, and diff/PR chips derive from.

### Track 2 — CLI-agent cwd (dead-ends today)

- `claude-code-warp/plugins/warp/scripts/build-payload.sh:41,57` — the plugin
  puts the hook's `cwd` into the OSC 777 `warp://cli-agent` JSON payload. It
  never emits OSC 7.
- `app/src/terminal/cli_agent_sessions/event/mod.rs` — the payload is parsed into
  `CLIAgentEvent.cwd`.
- `app/src/terminal/cli_agent_sessions/listener/mod.rs:185` — the per-terminal
  listener holds `terminal_view_id` and forwards parsed events to
  `CLIAgentSessionsModel::update_from_event(view_id, &event, ctx)`.
- `app/src/terminal/cli_agent_sessions/mod.rs:179` — `CLIAgentSession::apply_event`
  stores the directory: `self.session_context.cwd = event.cwd.clone()...`. This
  `session_context.cwd` field is never read by `display_working_directory` and
  never propagated to `Block::pwd`.
- `app/src/terminal/cli_agent_sessions/mod.rs:414` — `update_from_event` emits
  `CLIAgentSessionsModelEvent::StatusChanged` and/or `SessionUpdated`
  (the latter on `SessionStart | PromptSubmit | ToolComplete`).
- `app/src/terminal/view.rs:13387` — `TerminalView::handle_cli_agent_sessions_event`
  consumes those model events. It already gates on
  `*terminal_view_id == self.view_id` and already locks the terminal model to
  mutate the active block in this same handler
  (`let mut model = self.model.lock(); model.block_list_mut().active_block_mut()`,
  `view.rs:13398` / `13407`).

Net: the agent's directory lives in `session_context.cwd` for the agent footer,
but the block's `pwd` (and thus all displayed directory metadata) only updates
from Track 1.

## Proposed changes

Bridge Track 2 into Track 1 so the agent-reported directory flows through the
exact mechanism OSC 7 already uses. No new event types, no new UI, no plugin
change.

### 1. Extract a reusable directory setter on `TerminalModel`

`set_current_working_directory` is currently an `ansi::Handler` trait method
(`terminal_model.rs:2889`), so it can only be called from the terminal parser
and requires the trait in scope. Extract its body into a public inherent method
and have the trait impl delegate:

- New `TerminalModel::set_active_block_working_directory(&mut self, path: String)`
  (inherent, `pub`) containing the existing `is_ssh_block()` guard and the
  `block_list.set_current_working_directory(path)` routing.
- `<TerminalModel as ansi::Handler>::set_current_working_directory` becomes a
  one-line delegate to it.

This keeps the SSH guard and block routing in one place and lets non-parser
callers (the view) invoke it without importing the parser trait. Behavior for
OSC 7 is unchanged.

### 2. Propagate the agent cwd from the view

Add `TerminalView::sync_block_pwd_from_cli_agent(&self, ctx: &AppContext)`:

- Read the reported directory from the model:
  `CLIAgentSessionsModel::as_ref(ctx).session(self.view_id).and_then(|s| s.session_context.cwd.clone())`.
- Skip if absent or empty (invariant 4).
- Skip if it equals `self.pwd()` (invariant 3; `self.pwd()` reads the active
  block metadata, `view.rs:23207`).
- Otherwise `self.model.lock().set_active_block_working_directory(cwd)`.

Call it from `handle_cli_agent_sessions_event` inside the existing block that
already matches `Started | StatusChanged | SessionUpdated | Ended` and is gated
on `event.terminal_view_id() == self.view_id` (`view.rs:13414`), immediately
before `update_pane_configuration` / `ctx.notify()`. Those variants cover the
session start / prompt-submit / tool-complete events that carry the directory
(invariant 5).

Reading `session_context.cwd` from the model rather than from the
`StatusChanged` payload is deliberate: `SessionStart` and non-blocked
`ToolComplete` return `None` from `apply_event` (no `StatusChanged`) but still
set `session_context.cwd` and emit `SessionUpdated`, so the model is the single
source that reflects every reporting event.

### Guards / tradeoffs

- **SSH (invariant 6):** routing through `set_active_block_working_directory`
  inherits the `is_ssh_block()` early-return, so an agent inside an SSH-wrapped
  block cannot move the remote block's directory.
- **Hostname gate:** intentionally *not* applied. `osc_7_host_is_local`
  validates untrusted `file://host/path` bytes from the PTY; the agent cwd is
  already a parsed local path with no host component. Routing it through
  `parse_osc_7_cwd` would be incorrect.
- **Dedup:** `Block::set_current_working_directory` already dedupes, but the view
  also checks against `self.pwd()` first to avoid taking the model lock on the
  common no-change case.

## Testing and validation

- **Invariants 1–2 (integration):** extend the CLI-agent session tests
  (`app/src/terminal/cli_agent_sessions/mod_tests.rs`) — feed a `warp://cli-agent`
  event carrying a new `cwd`, then assert the block's `pwd` and
  `display_working_directory` (`tab_metadata.rs:16`) reflect it. This mirrors the
  existing OSC 7 assertion in `crates/integration/src/test.rs:3474`
  (`test_osc7_updates_current_working_directory`), which already checks that
  `display_working_directory` follows a block-pwd change.
- **Invariant 3 (no-op):** report the same cwd twice; assert only one
  `BlockWorkingDirectoryUpdated` is emitted (dedup at `block.rs:3417`).
- **Invariant 4 (empty):** report an empty/absent cwd; assert `pwd` unchanged.
- **Invariant 5 (timing):** assert the update fires for `SessionStart`,
  `PromptSubmit`, and `ToolComplete` (all emit `SessionUpdated` /
  `StatusChanged`).
- **Invariant 6 (SSH):** on an SSH block, report a cwd; assert the block's
  directory is unchanged (guard inherited from `set_active_block_working_directory`).
- **Manual (invariant 2 end-to-end):** in Warp with the `claude-code-warp`
  plugin, run `claude --worktree <name>`; confirm the tab subtitle, working-
  directory chip, git branch, and diff/PR chips update to the worktree. Capture
  before/after screenshots for the PR.
