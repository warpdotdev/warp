# GH10235: Tech Spec — Keep a CLI agent pane's identity across shell job control
Product spec: `specs/GH10235/product.md`
Issue: https://github.com/warpdotdev/warp/issues/10235
Code references inspected at commit: `a6b42da2714ae26e066eebfac1d21cbbf1053798`

## Context
A CLI agent session is stored per terminal view in `CLIAgentSessionsModel`. Its lifetime follows the block that launched the agent, not the agent process. Warp has no shell job-control awareness: there is no handling for `fg`, SIGTSTP, or stopped jobs anywhere in `app/` or `crates/`.

What happens today for `claude` → Ctrl+Z → `fg %1`:

1. `claude` runs as a `User` block. After `LONG_RUNNING_COMMAND_DURATION_MS`, `detect_cli_agent_from_model` matches the command and `set_session` creates the session, emitting `Started`. If the Claude plugin is installed, its first OSC 777 event upgrades the session with a listener.
2. Ctrl+Z stops the job. The shell prints its "suspended" line and runs its prompt hooks, so Warp completes the `claude` block. The block-completed handler closes rich input and calls `remove_session`, which emits `Ended` and drops the listener, draft, and context.
3. `fg %1` starts a new `User` block. When it becomes long-running, `CLIAgent::detect` looks at the first word (`fg`), finds no agent, and creates no session. The vertical tabs row therefore falls back to the plain-terminal icon and the shell title (`claude`).
4. If the plugin later sends an event, `register_cli_agent_listener_from_event` sees no listener and builds a fresh session through `register_listener` → `set_session`. That session has no draft and none of the earlier context.

Relevant code:
- [`app/src/terminal/view.rs:12762-12771 @ a6b42da2`](https://github.com/warpdotdev/warp/blob/a6b42da2714ae26e066eebfac1d21cbbf1053798/app/src/terminal/view.rs#L12762-L12771): on any `User` block completion, closes CLI agent rich input and calls `remove_session`. This is where a suspend is treated as an exit.
- [`app/src/terminal/view.rs:12931-12995 @ a6b42da2`](https://github.com/warpdotdev/warp/blob/a6b42da2714ae26e066eebfac1d21cbbf1053798/app/src/terminal/view.rs#L12931-L12995): long-running timer that detects the agent from the command and calls `set_session`, and registers the Codex/Grok listener without a `SessionStart` event.
- [`app/src/terminal/view/use_agent_footer/mod.rs:369-401 @ a6b42da2`](https://github.com/warpdotdev/warp/blob/a6b42da2714ae26e066eebfac1d21cbbf1053798/app/src/terminal/view/use_agent_footer/mod.rs#L369-L401): `detect_cli_agent_from_model` reads the active block's command text, resolves aliases, and calls `CLIAgent::detect`.
- [`app/src/terminal/cli_agent.rs:504-535 @ a6b42da2`](https://github.com/warpdotdev/warp/blob/a6b42da2714ae26e066eebfac1d21cbbf1053798/app/src/terminal/cli_agent.rs#L504-L535): `CLIAgent::detect` matches on the first command word, so `fg ...` never matches.
- [`app/src/terminal/cli_agent_sessions/mod.rs:149-179 @ a6b42da2`](https://github.com/warpdotdev/warp/blob/a6b42da2714ae26e066eebfac1d21cbbf1053798/app/src/terminal/cli_agent_sessions/mod.rs#L149-L179): `CLIAgentSession` holds the agent, status, context, input state, listener, and draft text.
- [`app/src/terminal/cli_agent_sessions/mod.rs:318-345 @ a6b42da2`](https://github.com/warpdotdev/warp/blob/a6b42da2714ae26e066eebfac1d21cbbf1053798/app/src/terminal/cli_agent_sessions/mod.rs#L318-L345): `CLIAgentSessionsModelEvent::{Started, Ended}`.
- [`app/src/terminal/cli_agent_sessions/mod.rs:395-509 @ a6b42da2`](https://github.com/warpdotdev/warp/blob/a6b42da2714ae26e066eebfac1d21cbbf1053798/app/src/terminal/cli_agent_sessions/mod.rs#L395-L509): the model's `sessions` map, `register_listener`, which upgrades an existing session or creates a new one, and `remove_session`.
- [`app/src/terminal/cli_agent_sessions/mod.rs:788-812 @ a6b42da2`](https://github.com/warpdotdev/warp/blob/a6b42da2714ae26e066eebfac1d21cbbf1053798/app/src/terminal/cli_agent_sessions/mod.rs#L788-L812): `set_session` closes input, resets Ctrl-C state, and emits `Ended` for a replaced session plus `Started`.
- [`app/src/terminal/view.rs:14123-14201 @ a6b42da2`](https://github.com/warpdotdev/warp/blob/a6b42da2714ae26e066eebfac1d21cbbf1053798/app/src/terminal/view.rs#L14123-L14201): OSC 777 handling. `register_cli_agent_listener_from_event` creates a session from any event when the pane has no listener.
- [`app/src/terminal/view.rs:14311-14351 @ a6b42da2`](https://github.com/warpdotdev/warp/blob/a6b42da2714ae26e066eebfac1d21cbbf1053798/app/src/terminal/view.rs#L14311-L14351): `handle_cli_agent_sessions_event`. `Started` enables full-grid-clear redraw on the active block, and `Started`/`Ended` refresh pane configuration and git status.
- [`app/src/terminal/view.rs:3931-3943 @ a6b42da2`](https://github.com/warpdotdev/warp/blob/a6b42da2714ae26e066eebfac1d21cbbf1053798/app/src/terminal/view.rs#L3931-L3943): `Ended` auto-stops session sharing that was auto-started for the CLI run.
- [`app/src/ai/agent_management/agent_management_model.rs:122-139 @ a6b42da2`](https://github.com/warpdotdev/warp/blob/a6b42da2714ae26e066eebfac1d21cbbf1053798/app/src/ai/agent_management/agent_management_model.rs#L122-L139): `Ended` removes the pane's CLI agent notifications.
- [`app/src/workspace/view/vertical_tabs.rs:4382-4410 @ a6b42da2`](https://github.com/warpdotdev/warp/blob/a6b42da2714ae26e066eebfac1d21cbbf1053798/app/src/workspace/view/vertical_tabs.rs#L4382-L4410): `terminal_agent_text` derives the row's agent identity and title from `CLIAgentSessionsModel::session`.
- [`app/src/workspace/view/vertical_tabs.rs:4274-4320 @ a6b42da2`](https://github.com/warpdotdev/warp/blob/a6b42da2714ae26e066eebfac1d21cbbf1053798/app/src/workspace/view/vertical_tabs.rs#L4274-L4320) and [`3335-3360`](https://github.com/warpdotdev/warp/blob/a6b42da2714ae26e066eebfac1d21cbbf1053798/app/src/workspace/view/vertical_tabs.rs#L3335-L3360): without a session the row uses the long-running shell title and `WarpIcon::Terminal`. These match the `>_ claude` row in the issue.
- [`app/src/terminal/model/terminal_model.rs:543-556 @ a6b42da2`](https://github.com/warpdotdev/warp/blob/a6b42da2714ae26e066eebfac1d21cbbf1053798/app/src/terminal/model/terminal_model.rs#L543-L556) and [`2101`](https://github.com/warpdotdev/warp/blob/a6b42da2714ae26e066eebfac1d21cbbf1053798/app/src/terminal/model/terminal_model.rs#L2101): `ShellProcessInfo { pid, pty_leader_fd }` is already available on the terminal model.
- [`app/src/ai/blocklist/action_model/execute/lrc_activity/sampler.rs:212-232 @ a6b42da2`](https://github.com/warpdotdev/warp/blob/a6b42da2714ae26e066eebfac1d21cbbf1053798/app/src/ai/blocklist/action_model/execute/lrc_activity/sampler.rs#L212-L232) and [`app/src/ai/agent_sdk/driver/harness/process_control.rs:55-63`](https://github.com/warpdotdev/warp/blob/a6b42da2714ae26e066eebfac1d21cbbf1053798/app/src/ai/agent_sdk/driver/harness/process_control.rs#L55-L63): two existing private helpers that read the PTY's foreground process group with `tcgetpgrp(pty_leader_fd)`.

The key observation is that the shell already knows exactly which job is suspended and which one `fg` resumed, and it tells the kernel: `fg` makes the job's process group the PTY's foreground group. Warp owns the PTY leader and already reads that value in two places. Tracking the agent by process group lets Warp follow job control without parsing `fg` arguments or the shell's job table.

## Proposed changes

### 1. Shared foreground process-group helper
Extract the duplicated `tcgetpgrp(pty_leader_fd)` logic into one helper, for example `terminal::process_group::foreground_pgid(&ShellProcessInfo) -> Option<u32>`. Use it from `lrc_activity/sampler.rs` and `harness/process_control.rs`, keeping their current ancestry checks. On non-Unix it returns `None`.

Add `ProcessGroupProbe` with one method, `leader_state(pgid) -> Option<LeaderState>`, where `LeaderState` is `{ start_time, stopped: bool }`. It is backed by `sysinfo` (`ProcessStatus::Stop`) in production and faked in tests. The start time guards against PID reuse.

### 2. Record the agent's process group on the session
Add `process_group: Option<AgentProcessGroup>` to `CLIAgentSession`, where `AgentProcessGroup { pgid: u32, leader_start_time: u64 }`.

- Fill it in where sessions are created for a local pane: the long-running detection path (`view.rs:12931-12995`) and `register_cli_agent_listener_from_event`. At both points the agent is the foreground job, so `foreground_pgid` returns its group. Accept the value only if it differs from the shell's own pid, so Warp never records the shell.
- Leave it `None` when `remote_host.is_some()`. The local PTY's foreground group is `ssh` there, not the agent. Also leave it `None` on non-Unix and when the probe fails.

### 3. Suspend instead of remove when the agent is stopped
In `CLIAgentSessionsModel`:
- Add `suspended: HashMap<EntityId, Vec<SuspendedCLIAgentSession>>`, where `SuspendedCLIAgentSession { session: CLIAgentSession, was_input_open: bool }`.
- Add `suspend_session(terminal_view_id, was_input_open, ctx)`. It aborts any pending Ctrl-C window, moves the session from `sessions` into `suspended`, and emits `Ended { reason: EndReason::Suspended }`. The draft is already in the session: the block-completed handler calls `close_cli_agent_rich_input` first, which saves the buffer through `set_draft` ([`use_agent_footer/mod.rs:603-608`](https://github.com/warpdotdev/warp/blob/a6b42da2714ae26e066eebfac1d21cbbf1053798/app/src/terminal/view/use_agent_footer/mod.rs#L603-L608)). Today that draft is dropped by `remove_session` right afterward. The listener handle moves with the session, so the PTY subscription stays alive. A stopped process emits nothing, so this is inert.
- Leave `remove_session` unchanged except that it emits `Ended { reason: EndReason::Exited }`.

In the block-completed handler (`view.rs:12762-12771`), read `CLIAgentSessionsModel::is_input_open` before closing rich input, and keep closing it as today. Then call `suspend_session` with that value instead of `remove_session` when both of these hold:
- the session has a `process_group`, and
- the probe reports that group's leader alive, with the recorded start time, and `stopped`.

Otherwise call `remove_session` as today.

Using the process state rather than the exit code makes this independent of shell conventions. SIGTSTP is 18 on macOS and 20 on Linux, so the shell reports 146 or 148, and fish reports it differently again. A legitimate exit status of 146 can no longer be mistaken for a suspend.

### 4. Resume by matching the foreground process group
Add `resume_session(terminal_view_id, foreground_pgid, probe, ctx) -> bool` to `CLIAgentSessionsModel`:
- Drop any suspended entries whose leader is gone or whose start time no longer matches. Emit `Ended { reason: Exited }` for each, so notification cleanup and sharing auto-stop run once, at real exit.
- If an entry's `pgid` equals `foreground_pgid`, remove it from `suspended` and insert its session into `sessions` without going through `set_session`. That keeps the Ctrl-C state reset and input-close side effects that belong to a fresh session out of the resume path. Restore `draft_text`, and reopen input if `was_input_open`. Emit `Started { reason: StartReason::Resumed }`, then return `true`.

In the long-running timer (`view.rs:12931-12995`), call `resume_session` with the current `foreground_pgid` before `detect_cli_agent_from_model`. If it returns `true`, skip command detection and the Codex/Grok listener registration, because the restored session already has its listener. Then run the existing `maybe_show_use_agent_footer_in_blocklist`. Because the timer only fires for blocks that stay long-running, a failed `fg` that exits immediately never reaches this path (product behavior 10).

`Started { reason: Resumed }` still reaches `handle_cli_agent_sessions_event`, which calls `enable_full_grid_clear_behavior()` on the `fg` block. That gives the resumed TUI the same in-place redraw as at launch (product behavior 8).

### 5. Plugin fallback for remote shells
In `register_cli_agent_listener_from_event`, before creating a listener, check the pane's suspended entries for one with the same `agent` and the same non-empty `session_context.session_id` as the incoming event. If one matches, restore it through the same internal path as `resume_session`, and let `update_from_event` apply the event to the restored session. This covers SSH, where there is no process group (product behavior 11). It runs only when no live session exists, so it never steals events from a running session.

### 6. Event reasons and subscribers
Add `reason` fields to `Started { .., reason: StartReason }` (`New | Resumed`) and `Ended { .., reason: EndReason }` (`Exited | Suspended`). The compiler will flag every match site. Only these subscribers need to look at the reason:
- `TerminalView` auto-stop sharing (`view.rs:3931-3943`): stop only on `EndReason::Exited` (product behavior 4).
- Plugin-detected telemetry and `maybe_auto_open_cli_agent_rich_input`: these are driven by `SessionStart` events and the detection path, which the resume path bypasses, so they need no change. Add a regression test that a resume does not call them.
- `AgentManagementModel` (`agent_management_model.rs:132-139`): keep removing notifications on both reasons. A "needs input" notification for a stopped agent is stale.

Everything else can keep treating `Started`/`Ended` as they do today: `Workspace`, the vertical tabs, `terminal_view_adaptor`, the agent input footer, and `pending_cli_harness_prompt_queue`. Hiding agent chrome while suspended and showing it again on resume is exactly what product behaviors 2, 3, and 6 ask for.

### 7. Cleanup
- When the terminal view is dropped (`impl Drop for TerminalView`) or the shell exits, clear the pane's `suspended` entries and emit `Ended { reason: Exited }` for each (product behavior 12).
- Cap `suspended` at 8 entries per pane and drop the oldest. This bounds memory if the probe can't observe process exit.

### Alternatives considered
- **Parse `fg` / `%N` job specs from the command text.** `fg` and `%` map to the most recent stopped job, `%name` to a prefix match, and so on. This fails for `%N` when more than one job is stopped, for jobs killed while suspended, for background jobs that became the current job, and for aliases or functions that wrap `fg`. Matching the foreground process group avoids all of these.
- **Have the shell integration report `jobs -l` in precmd.** Exact, but it touches every bootstrap script and adds work to every prompt, and Warp still has to map job numbers to sessions. The process group already carries that mapping.
- **Detect suspend by exit code.** This depends on the shell and platform (146 vs 148, fish) and gives false positives for legitimate exit statuses. It's kept only as a possible debug log.

## Testing and validation
Unit tests (`app/src/terminal/cli_agent_sessions/mod_tests.rs`, with a fake `ProcessGroupProbe`):
- `suspend_session` moves the session out of `sessions`, keeps the draft and `was_input_open`, and emits `Ended { Suspended }`. Covers behaviors 2, 3, 5.
- `resume_session` with a matching pgid restores the identical agent, status, context, listener handle, and draft, and emits `Started { Resumed }`. Covers 6 and 7.
- With two suspended sessions, each pgid restores only its own session, in any order. Covers 9.
- A non-matching pgid restores nothing. An entry whose leader is gone or has a different start time is reaped with `Ended { Exited }`. Covers 10.
- The plugin fallback restores by `(agent, session_id)` and ignores a different `session_id` or agent. Covers 11.
- After a resume, `remove_session` emits `Ended { Exited }` exactly once. Covers 13.

View and integration tests:
- In the `TerminalView` tests, completing a `User` block whose session leader the fake probe reports as stopped keeps the session suspended rather than removed, and does not auto-stop sharing. Covers 4.
- `Started { Resumed }` enables full-grid-clear redraw on the new active block. Covers 8.
- In `vertical_tabs_tests.rs`, `terminal_agent_text` returns the agent identity and title after a resume. With a session suspended it returns plain terminal text, still one row per pane. Covers 1, 3, 6.
- Closing the pane emits `Ended { Exited }` for suspended sessions. Covers 12.

Commands:
```bash
cargo nextest run --package warp terminal::cli_agent_sessions
cargo nextest run --package warp workspace::view::vertical_tabs
cargo nextest run --package warp terminal::view
./script/format --check
cargo clippy --workspace --all-targets --all-features --tests -- -D warnings
./script/presubmit
```

Manual validation, with a screen recording for the PR:
- macOS zsh and bash, Linux bash, and fish: run `claude` and then `codex`, press Ctrl+Z, run `fg`. Check one row with the agent icon, title, and status, the footer back, and the draft kept.
- Resume variants: `fg %1`, `%1`, `%claude`, and an alias `alias f=fg`.
- Two agents suspended in one pane, resumed in both orders.
- Suspend, `kill -9 %1`, then `fg` another job. The agent must not be restored.
- An auto-started shared session survives suspend/resume.
- SSH (warpified) with the Claude plugin. The session is restored on the first plugin event after `fg`.

## Risks and mitigations
- **PID or process-group reuse.** Matching requires the recorded leader start time, and entries are reaped whenever the probe says the leader is gone.
- **Agent re-execs into a new process group.** Then the foreground group won't match after `fg` and Warp falls back to today's detection or plugin path. This is no worse than now.
- **`sysinfo` refresh cost on block completion.** Probe only the single recorded pid, and only for panes that have a CLI agent session.
- **New event fields touch many match sites.** The changes are mechanical and the compiler finds every site. The default handling is unchanged, and only the sharing auto-stop branches on the reason.

## Follow-ups
- Optional "suspended" hint on the pane row (see the product spec's open questions).
- Reuse the same process-group tracking for Warp's own long-running-command UI if suspended non-agent jobs need similar treatment.
