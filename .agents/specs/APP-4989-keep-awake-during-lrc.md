# APP-4989: keep the computer awake during agent long-running command execution

## PRODUCT

**Summary:** The Warp client already has keep-awake logic (`crates/prevent_sleep`), but
today a wake assertion is held only for the lifetime of a single multi-agent SSE
request (one turn). It is dropped at `StreamFinished`, so the machine can idle-sleep
while the client executes tool calls locally between turns — most importantly while a
long-running shell command runs across turns. This change closes that gap by holding a
wake assertion continuously across the whole in-progress agent conversation — including
the local command-execution window between `StreamFinished` and the next follow-up turn
(`WriteToLongRunningShellCommand` / `ReadShellCommandOutput` polling) — and releasing it
promptly when the conversation reaches a terminal state, is cancelled, goes idle waiting
for user input, **or exceeds a refreshable max-duration safety cap**. The per-request
guard in `http_client`/`warp_multi_agent_client` (the only existing guard site) is
**removed** because the conversation-scoped guard supersedes it and the per-request guard
added no coverage the conversation guard does not. The change is client-side in
`warpdotdev/warp`; no server change is required. Per the requester's explicit direction,
this first change is scoped to the long-running command (LRC) case and to *system* sleep
only.

**User-visible invariants:**

1. While an in-app agent conversation is running (status `InProgress`), a system
   idle-sleep wake assertion is held continuously — across every turn and across the
   local command-execution gap between turns — so the machine does not idle-sleep while
   the agent runs a long-running local command (e.g. `sleep 600`).
2. The assertion is held across transient failures (`TransientError`, while an automatic
   retry/resume is pending) so recovery is not interrupted by sleep.
3. The assertion is released promptly when the conversation stops running: on a terminal
   status (`Success`, `Error`, `Cancelled`), when the agent yields to wait for events or
   user input (`WaitingForEvents`), or when an action is blocked pending user approval
   (`Blocked`). It is re-acquired if the conversation resumes (returns to `InProgress`).
4. The assertion is released when the conversation is cleaned up (pane cleared,
   conversation removed/deleted) so closing the pane or deleting the conversation never
   leaves a stray wake assertion.
5. **Max-duration safety cap (refreshable):** the assertion is held for at most a bounded
   window since the last *refresh event*. Each new turn (SSE `Init`/`StreamInit`), each
   locally-executed tool-call action result, each `ReadShellCommandOutput` /
   `WriteToLongRunningShellCommand` poll, and any user input that resumes the
   conversation refreshes the timer, so a genuinely-active long conversation (many turns,
   or a long-running command polled repeatedly across turns) stays awake the whole time. A
   single stuck conversation or stuck LRC that receives no refresh event for longer than
   the cap eventually releases the assertion even while still nominally `InProgress`.
6. The now-redundant per-request `prevent_sleep` guard on the multi-agent SSE request is
   **removed**. The conversation-scoped guard is the single source of wake protection for
   agent runs; `http_client`'s generic `prevent_sleep` opt-in remains available for any
   future non-agent caller that wants it, but no caller sets it after this change.
7. Auto-handoff-on-sleep behavior is unchanged: the existing long-running-command skip
   (`AutoCloudHandoffSkipReason::LongRunningCommand`) is not altered, and no new setting
   or flag is introduced that affects handoff gating. Covering the LRC gap here is
   complementary to handoff (which already skips LRCs).
8. On Linux the keep-awake backend remains a no-op (out of scope for this change); the
   new model still compiles and runs on Linux and on wasm without changing behavior
   (wasm guard is a no-op).

**Key design choices:** (a) Scope the wake assertion to the agent *conversation* lifecycle
(held across turns, including the local command-execution gap), not per-request. (b) Add a
**refreshable max-duration cap** (default 20 minutes) so a stuck conversation cannot hold
the assertion indefinitely; each turn / tool-call activity / command-output poll / user
input refreshes the cap, and cap expiry releases the assertion (with a log + telemetry
signal) even while still `InProgress`. (c) **Remove** the existing per-request guard — it
is only exercised by the agent SSE path and is fully superseded by the conversation guard.
(d) Manage the guard in one small dedicated model keyed by conversation id, subscribed to
`BlocklistAIHistoryModel` status + cleanup events plus turn/activity refresh signals, so
the acquire/refresh/release logic is centralized and unit-testable. (e) Use a distinct
reason string (`"Agent Mode run in-progress"`) from the removed per-request one.

## TECH

**Current context (all references pinned to `fb22b1920c59bc6e6aa969d81c797ec24e7b1830`, current `master` HEAD):**

- `crates/prevent_sleep/src/lib.rs:17` exposes `prevent_sleep::prevent_sleep(reason: &'static str) -> Guard`;
  the guard releases the assertion on `Drop`. `Guard` is `Send + Sync` on macOS
  (`crates/prevent_sleep/src/mac.rs:16-17`) and sendable on Windows (the guard holds an
  `mpsc::Sender`, `crates/prevent_sleep/src/windows.rs:143-146`). `Stream::wrap`
  (`lib.rs:33`) wraps a stream with an optional guard.
- `crates/prevent_sleep/src/mac.rs:34` uses `NSActivityOptions::UserInitiated` (includes
  `IdleSystemSleepDisabled`, prevents idle *system* sleep; does **not** include
  `IdleDisplaySleepDisabled`). `crates/prevent_sleep/src/windows.rs:59-63` uses
  `ES_CONTINUOUS | ES_AWAYMODE_REQUIRED | ES_SYSTEM_REQUIRED` (no `ES_DISPLAY_REQUIRED`).
  `crates/prevent_sleep/src/noop.rs` is a no-op `Guard` for Linux/other/wasm
  (`crates/prevent_sleep/build.rs:6`: `noop: { not(any(macos, windows)) }`).
- **Neither platform's assertion self-expires.** macOS
  `NSProcessInfo::beginActivityWithOptions:reason:` returns a token that remains in
  effect until `endActivity:` is called (or the token is deallocated) — there is no
  built-in timeout (Apple: "be careful to end activities that disable sleep ...
  failing to end these activities for an extended period of time can have significant
  negative impacts"). Windows `SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED)`
  "continuously prevents the system from sleeping until explicitly cleared"; Microsoft
  explicitly warns "Avoid holding `ES_SYSTEM_REQUIRED | ES_CONTINUOUS` indefinitely. On
  Modern Standby devices, this drains the battery rapidly even when the lid is closed."
  So **the old per-request guard had no max time limit** — its only bound was the SSE
  stream ending (guard `Drop`). This is why a refreshable cap is needed for the new
  conversation-scoped guard, which is intended to span a much longer window.
- The only callers that request a guard are in `crates/http_client/src/lib.rs`:
  `execute_inner` (`lib.rs:381`, non-streaming round-trip) and `eventsource()`
  (`lib.rs:553-557`, wraps the SSE stream via `prevent_sleep::Stream::wrap`). The guard is
  only acquired when `prevent_sleep_reason` is `Some`, which is set only by
  `RequestBuilder::prevent_sleep(reason)` (`lib.rs:656-660`). **The only caller of that
  method is `crates/warp_multi_agent_client/src/lib.rs:59`:
  `.prevent_sleep("Agent Mode request in-progress")` on the multi-agent SSE request.** No
  other `http_client` caller (lsp, graphql, drive, telemetry, etc.) ever sets a reason, so
  none of them get a guard today. `prevent_sleep` is a workspace dependency
  (`Cargo.toml:65`) currently used only by `http_client`
  (`crates/http_client/Cargo.toml:21`); the `app` crate does **not** currently depend on
  it. Conclusion: the `http_client` `prevent_sleep` hook is *generic opt-in
  infrastructure*, but in practice it is **only exercised by the agent SSE path**, so
  removing that one call site loses no other caller's protection and the conversation
  guard fully supersedes it.
- The agent runs as a sequence of turns. Each turn = one SSE request = one
  `ResponseStream`. `app/src/ai/blocklist/controller.rs:2490-2505` constructs
  `ResponseStream::new(...)` per turn; `controller.rs:2542-2547` sets the conversation to
  `InProgress` when a request is sent. After a turn's stream finishes,
  `controller.rs:3035+` (`AfterStreamFinished`) queues the turn's actions for local
  execution, and `controller.rs:1572-1667` (`send_follow_up_for_conversation`) sends a
  *new* turn once actions complete. The conversation stays `InProgress` during local
  action execution between turns.
- `app/src/ai/blocklist/action_model/execute/shell_command.rs:38-49` (`ShellCommandExecutor`)
  acquires **no** `prevent_sleep` guard. Its poll future (`action_result_future`,
  `shell_command.rs:513`) waits on timers up to `MAX_AGENT_DELAY_DURATION` (120s,
  `shell_command.rs:56`) or `MAX_WAIT_DURATION` (2s, `:52`) and on block-completion
  signals. `ReadShellCommandOutput` (`:349-389`) and `WriteToLongRunningShellCommand`
  (`:285-348`) poll across turns, so the inter-turn gap can be many minutes and recur
  many times for one command. `supports_long_running_commands: true` is advertised at
  `app/src/ai/agent/api/impl.rs:82`.
- `app/src/ai/agent/conversation.rs:4589-4613` defines `ConversationStatus`:
  `InProgress`, `Success`, `Error`, `TransientError`, `Cancelled`, `Blocked { .. }`,
  `WaitingForEvents`. `is_done()` (`:4713-4718`) covers `Success`/`Error`/`Cancelled`;
  `is_waiting_for_events()` (`:4722-4724`) covers `WaitingForEvents`.
  `update_status_with_error` (`conversation.rs:973-997`) emits
  `BlocklistAIHistoryEvent::UpdatedConversationStatus { conversation_id, terminal_surface_id, update: ConversationStatusUpdate::Changed { prev_status }, new_status }` on every status set.
- `app/src/ai/blocklist/history_model.rs:2938-2945` defines
  `BlocklistAIHistoryEvent::UpdatedConversationStatus`; `:2883-2886` defines
  `ConversationStatusUpdate { Restored, Changed { prev_status } }` (`Restored` is emitted
  on conversation re-load, `history_model.rs:1149`). Cleanup events:
  `ClearedConversationsForTerminalSurface` (`:2959-2965`, carries `cleared_conversation_ids`),
  `RemoveConversation` (`:2987-2991`), `DeletedConversation` (`:2993-3001`).
- Turn/activity refresh signals available to the new model: a new SSE turn starting
  (`ResponseStreamEvent::ReceivedEvent` with `StreamInit`, observed via the controller's
  existing `handle_response_stream_event`); locally-executed action results drained in
  `send_follow_up_for_conversation` (`controller.rs:1588-1590`,
  `drain_finished_action_results`); `ReadShellCommandOutput` /
  `WriteToLongRunningShellCommand` poll completions (the `ShellCommandExecutor` action
  result futures, `shell_command.rs:331-347` and `:375-388`); and user input that
  resumes a `WaitingForEvents`/`Blocked` conversation (the conversation returning to
  `InProgress`).
- `app/src/workspace/auto_handoff.rs:198-200` handles `CpuWillSleep`; `:328-330` skips
  handoff with `AutoCloudHandoffSkipReason::LongRunningCommand` when
  `has_active_long_running_command()`. `app/src/settings/ai.rs:1930-1939`:
  `auto_handoff_on_sleep_enabled` defaults to `false`, macOS only.

**Trace/reproduction evidence:** The gap is structural and confirmed by call-graph
tracing (carried forward from the APP-4989 triage findings): `prevent_sleep` is depended
on only by `http_client`; the only guard request site is the multi-agent SSE request;
`ShellCommandExecutor` and the terminal command path acquire no guard; each turn is a
separate `ResponseStream`/SSE stream. Manual corroboration (macOS): run an agent turn
that executes a long-running local command (e.g. ask the agent to run `sleep 600`);
while the command is running, `pmset -g assertions` shows **no** `PreventUserIdleSystemSleep`
assertion from Warp during the inter-turn gap (the assertion appears only while an SSE
turn is actively streaming), and the machine will idle-sleep during the command. After
this change the assertion must be present continuously through the `sleep 600` run.

### Proposed changes

1. **Add a conversation-scoped, refresh-capped wake-assertion model.**
   - Add a new model (e.g. `AgentRunSleepGuardModel`) that owns, per conversation id, a
     `prevent_sleep::Guard` plus a refresh deadline (`Instant` = last refresh + cap). It
     subscribes to `BlocklistAIHistoryModel` events and to turn/activity refresh signals
     (see the refresh-events list above). Register it in the app model graph alongside
     the other `BlocklistAI*` singletons/models, on both native and wasm targets.
   - **Acquire:** on `UpdatedConversationStatus` with `new_status` `InProgress` or
     `TransientError` and no guard held for that conversation, acquire one with reason
     `"Agent Mode run in-progress"` and set the deadline to `now + CAP`. Only acquire when
     no guard exists for the id (never double-acquire for one conversation).
   - **Refresh:** each refresh event (new SSE turn `StreamInit`, locally-executed action
     result drained, `ReadShellCommandOutput`/`WriteToLongRunningShellCommand` poll
     completion, user input resuming the conversation) advances the deadline to
     `now + CAP` for that conversation. Refreshing does **not** drop+re-acquire the guard
     (the OS assertion stays continuous); only the deadline moves. A conversation that
     keeps producing turns or polling an LRC stays awake indefinitely, which is the
     desired behavior for an active run.
   - **Cap expiry:** a periodic timer (e.g. waked every 60s, or on each refresh event)
     checks each held guard's deadline. If `now > deadline` while the conversation is
     still nominally `InProgress`/`TransientError`, **release the guard** (drop it), emit
     a `log::warn!` ("Agent Mode sleep guard cap expired for conversation {id}; releasing
     wake assertion") and a telemetry event (e.g. an `AgentRunSleepGuardCapExpired`
     telemetry event via `send_telemetry_from_ctx!`) so the expiry is observable. The
     conversation status is **not** changed by cap expiry (it stays `InProgress`); only
     the wake assertion is released. The guard is re-acquired (and the deadline reset) on
     the next refresh event — i.e. the next turn, action result, or poll — so a stalled
     conversation that resumes re-acquires wake protection automatically.
   - **Release on status:** on `UpdatedConversationStatus` with `new_status`
     `Success`/`Error`/`Cancelled`/`WaitingForEvents`/`Blocked`, drop the guard for that
     conversation (if held). Re-acquire on a later return to `InProgress`.
   - **Release on cleanup:** on `ClearedConversationsForTerminalSurface` /
     `RemoveConversation` / `DeletedConversation`, drop guards for the affected
     conversation id(s) so pane close / deletion never leaks an assertion.
   - **Restored conversations:** on `ConversationStatusUpdate::Restored` with an active
     status (`InProgress`/`TransientError`): conservatively acquire with a fresh deadline
     (the run may resume on app restart). If a restored `InProgress` conversation does not
     actually resume, the cap bounds the held duration (it expires after `CAP` with no
     refresh events).
   - The model's own `Drop` releases all held guards as a final safety net.
   - **Cap default:** `CAP = 20 minutes` (`const CAP: Duration = Duration::from_secs(20 * 60)`).
     Rationale: a single turn + local action execution rarely exceeds a few minutes; an
     LRC poll cycle is bounded by `MAX_AGENT_DELAY_DURATION` (120s). 20 min is well above
     normal inter-turn gaps, comfortably covers a long LRC polled every few minutes, yet
     short enough that a truly stuck conversation (no turns, no polls, no user input)
     releases the assertion in a bounded window rather than draining battery until the
     user notices. The cap is a `const`, not a setting (no new setting is introduced — see
     non-goals), but is easy to tune in one place.

2. **Wire the `prevent_sleep` dependency into the `app` crate.**
   - Add `prevent_sleep.workspace = true` to `app/Cargo.toml`. No change to the
     `prevent_sleep` crate itself is required — `Guard` is already constructible via
     `prevent_sleep::prevent_sleep(reason)` and is `Send`/`Sync` (mac) / sendable (win),
     so it can be stored in a model. On wasm/Linux the guard is a no-op, so the model is
     harmless there.

3. **Remove the now-redundant per-request guard.**
   - Remove `.prevent_sleep("Agent Mode request in-progress")` at
     `crates/warp_multi_agent_client/src/lib.rs:59`. The conversation-scoped guard is
     acquired on `InProgress` (before the first turn's SSE request is even sent, since
     `controller.rs:2542-2547` sets `InProgress` when a request is sent) and held across
     the whole run, so it strictly supersedes the per-request guard — the per-request
     guard added no window the conversation guard does not cover, and the conversation
     guard additionally covers the inter-turn local-execution gap that the per-request
     guard missed.
   - The `prevent_sleep::Stream::wrap` and `execute_inner` guard paths in
     `crates/http_client/src/lib.rs` (`lib.rs:381`, `:553-557`) are **generic opt-in
     infrastructure** keyed off `prevent_sleep_reason: Option<&'static str>`; with no
     caller setting a reason they become inert (`None` → no guard acquired). They are left
     in place for any future non-agent caller that wants per-request wake protection —
     removing the `http_client` plumbing is out of scope and would needlessly widen the
     blast radius. (If a future audit confirms no caller will ever use it, the
     `prevent_sleep` dependency can be dropped from `http_client` in a separate cleanup
     PR; not required here.)
   - Net effect: exactly one wake assertion per active agent conversation (the
     conversation-scoped one), instead of one-per-turn that left inter-turn gaps.

**Design alternatives:**

- **Where to attach the conversation-level guard** — (a) *selected:* a new dedicated
  model subscribed to `BlocklistAIHistoryModel` status + cleanup events plus
  turn/activity refresh signals, keyed by conversation id. Centralizes the lifecycle
  (including the refresh cap) in one testable owner and reuses the existing status event
  surface. (b) Acquire/release the guard directly at every status-transition site in
  `BlocklistAIController`. More invasive, scatters the logic (and the cap timer) across
  many call sites, and is error-prone (easy to miss a transition or a refresh). (c)
  Acquire the guard inside `ShellCommandExecutor` per LRC action. Narrower, but it does
  not cover the gap between `StreamFinished` and the first LRC action, nor the gap
  between LRC completion and the next follow-up turn, and requires per-action
  acquire/release that still leaves sub-turn gaps — it does not fully close the gap the
  requester named. (d) Extend `ResponseStream` to hold a conversation-level guard.
  `ResponseStream` is per-turn/per-SSE-stream, so it is the wrong scope (it drops at
  `StreamFinished` — the same hole).
- **Max-duration cap design** — (a) *selected:* a refreshable deadline per conversation,
  refreshed on turn/activity/output/user-input events, with cap expiry releasing the
  guard (log + telemetry) without changing the conversation status, and re-acquire on the
  next refresh. Keeps active long conversations awake indefinitely while bounding a
  stuck one. (b) A hard absolute max (e.g. "never hold > 2h regardless of activity").
  Rejects legitimately long but active runs (a multi-hour LRC-polled agent run is a real
  case the requester wants awake). (c) No cap (hold until terminal/cancel/idle). This is
  the status quo for the *per-request* guard (no self-expiry on either platform) and is
  exactly the requester's worry — a stuck conversation or stuck LRC would hold the
  assertion until the user intervenes. (d) Cap implemented by drop+re-acquire on each
  refresh (re-issuing the OS assertion). Rejected: on Windows that briefly drops the
  power request, and on macOS it needlessly churns `NSProcessInfo` activities; the
  assertion must stay *continuous* across refreshes, so only the deadline moves.
- **Keep vs remove the per-request guard** — (a) *selected:* **remove** the one
  agent-path call site (`.prevent_sleep(...)` in `warp_multi_agent_client`). It is only
  exercised by the agent SSE path (no other `http_client` caller sets a reason), and the
  conversation-scoped guard strictly supersedes it (it covers the SSE window *and* the
  inter-turn gap the per-request guard missed). Removing it eliminates redundancy and the
  confusion of two overlapping assertions with two reason strings. (b) Keep it as
  defense-in-depth. Rejected: it provides no additional coverage (the conversation guard
  is acquired before the first turn and held across all turns), and "defense in depth"
  against a *missing* wake assertion is not a real failure mode here — the risk is a
  *leaked* assertion, and two guards double the leak surface. (c) Remove the
  `prevent_sleep` plumbing from `http_client` entirely. Out of scope: the plumbing is
  generic opt-in infra, removing it widens the blast radius, and it harms nothing when no
  caller sets a reason. Leave the plumbing; remove the one call site.
- **Release conditions** — (a) *selected:* release on terminal (`Success`/`Error`/
  `Cancelled`), on idle-waiting-for-user-input (`WaitingForEvents`, `Blocked`), and on
  cap expiry; hold across `InProgress` and `TransientError`. Matches the requester's
  "release on terminal / cancellation / idle-waiting-for-user-input" direction plus the
  cap-safety request. (b) Hold until terminal only (keep through `WaitingForEvents`/
  `Blocked`). Keeps the assertion while the user is away from a parked conversation, but
  drains battery when the agent is genuinely idle waiting for input and no command is
  running — rejected for the LRC scope. (c) Release on every `StreamFinished` and
  re-acquire on the next turn. This is the status quo and is exactly the hole being
  closed.

**Open questions resolved:**

- *System-only vs system+display sleep* — per the requester's scope, this change
  prevents *system* sleep only. The macOS `IdleDisplaySleepDisabled` flag and the Windows
  `ES_DISPLAY_REQUIRED` flag are explicitly **out of scope** (deferred). The existing
  `NSActivityOptions::UserInitiated` and `ES_SYSTEM_REQUIRED` behavior is unchanged.
- *Aggressive whole-conversation prevention vs minimal gap-covering* — per the requester,
  hold the guard across the conversation/turn lifecycle spanning the LRC gap (the local
  command-execution window between turns), releasing on terminal/cancel/idle-waiting-for-
  user-input, **bounded by a refreshable max-duration cap** so a stuck run releases.
- *Did the old per-request guard have a max time limit?* — **No.** Neither macOS
  `NSProcessInfo::beginActivityWithOptions:reason:` nor Windows
  `SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED)` self-expire; the only
  bound was the SSE stream ending (guard `Drop`). The new conversation-scoped guard is
  intended to span a much longer window, so it adds the refreshable cap the old guard
  never had.
- *Cap value* — 20 minutes (const, not a setting). Well above normal inter-turn gaps and
  LRC poll cycles, short enough to bound a stuck run. Tunable in one place.
- *What refreshes the cap* — new SSE turn (`StreamInit`), locally-executed action result
  drained, `ReadShellCommandOutput`/`WriteToLongRunningShellCommand` poll completion, and
  user input resuming the conversation. These are exactly the signals that the
  conversation is genuinely active.
- *Cap-expiry semantics while still InProgress* — release the guard only (drop it); do not
  change the conversation status; emit a `log::warn!` + a telemetry event
  (`AgentRunSleepGuardCapExpired`); re-acquire (with a fresh deadline) on the next
  refresh event, so a stalled-then-resumed run re-acquires wake protection.
- *Remove the existing per-request guard?* — **yes.** It is only the agent SSE path, and
  the conversation guard strictly supersedes it (covers the SSE window plus the inter-turn
  gap). The `http_client` generic plumbing stays (no other caller sets a reason, so it is
  inert), but the one agent call site is removed.
- *Interaction with auto-handoff-on-sleep* — out of scope to redesign; this change adds
  no new handoff behavior or setting. It is complementary: handoff already skips LRCs
  (`auto_handoff.rs:328-330`), so covering the LRC gap here does not change handoff's
  decision. The validation criteria require confirming no regression.
- *Linux keep-awake backend* — out of scope (deferred). Linux stays a no-op; the new
  model compiles and runs on Linux (no-op guard) and the unit tests run on Linux.
- *`Blocked` state* — release the guard there (the agent is waiting for the user to
  approve an action; no local command is executing). Re-acquire if the conversation
  returns to `InProgress` after approval.
- *Restored `InProgress` conversations on app restart* — conservatively acquire with a
  fresh deadline (the run may resume). Bounded by the cap: if a restored `InProgress`
  conversation never produces a refresh event, the guard expires after `CAP` and is
  re-acquired only if the conversation resumes.
- *Reason string* — `"Agent Mode run in-progress"` (the per-request
  `"Agent Mode request in-progress"` is removed with the per-request guard).

**Risks / mitigations:**

- *Guard leak (battery drain)* — three layers of defense: (1) release on any non-active
  status and on all cleanup events; (2) the refreshable cap releases a stuck-but-still-
  `InProgress` conversation after `CAP` with no refresh events; (3) the model's `Drop`
  releases all guards. The unit tests cover the status, cleanup, and cap-expiry paths.
- *Cap expiry drops protection for a genuinely-active run* — mitigated by choosing refresh
  events that fire on every real activity (turn, action result, LRC poll, user input). A
  run that is producing any of these stays awake. A run that produces none of them for
  `CAP` is, by definition, stalled and should release. The telemetry event makes
  false-positive expiries observable and tunable.
- *Double-acquire / panic* — the model only acquires when no guard exists for the
  conversation id. On Windows the `State` thread handles `AddTask`/`RemoveTask` by task
  id (`windows.rs:99-118`); on macOS multiple `NSProcessInfo` activities stack. No panic
  path.
- *Removing the per-request guard regresses streaming-turn wake protection* — it does
  not: the conversation guard is acquired on `InProgress` (set when a request is sent,
  `controller.rs:2542-2547`) before the first turn's SSE stream opens, and held across
  every turn. The per-request guard's window is a strict subset. Validation criterion 6
  confirms via the existing `http_client`/`warp_multi_agent_client` tests plus reasoning.
- *Restored stale guard* — a restored `InProgress` conversation that never resumes is
  bounded by the cap (expires after `CAP` with no refresh). Better than the unbounded
  hold the original spec proposed.
- *Wasm / Linux* — guard is a no-op there; ensure the model compiles on wasm and that no
  native-only API is unconditionally called (gate any native-only behavior behind
  `cfg(not(target_family = "wasm"))` if needed, though the no-op `Guard` makes this
  likely unnecessary). The cap timer must be implemented with a `Timer`/`Timer::after`
  that works on wasm (the `warpui` `Timer` used by `ShellCommandExecutor` is the existing
  pattern to follow).
- *Battery impact of holding across the whole conversation* — intended behavior per the
  requester (close the gap). Mitigated by releasing on `WaitingForEvents`/`Blocked` and by
  the refreshable cap, so an idle or stuck conversation does not hold the assertion.

## Validation & verification criteria (all must pass before merge)

1. *Reproduction fixed (manual, macOS)* — ask the agent to run a long-running local
   command (e.g. `sleep 600`). During the command run — specifically in the inter-turn
   gap after the `RunShellCommand` turn's SSE stream finishes and before the follow-up
   turn — run `pmset -g assertions` in another terminal and confirm a
   `PreventUserIdleSystemSleep` assertion from Warp is present **continuously** through
   the `sleep 600` run (not only while an SSE turn is streaming). This carries forward
   the triage's manual corroboration as the repro. (Behavioral proof is the OS power
   assertion output, not a screenshot — this change has no visible UI delta.)
2. *Reproduction fixed (manual, Windows)* — equivalent on Windows: `powercfg /requests`
   shows a `SYSTEM` request from Warp continuously through a long-running command run,
   including the inter-turn gap.
3. *Regression test (unit) — guard lifecycle* — add a unit test (e.g.
   `agent_run_sleep_guard_model_lifecycle` in the new model's `*_tests.rs`) that drives
   `BlocklistAIHistoryModel` conversation status transitions and asserts the guard map:
   acquires on `InProgress`; holds across `TransientError`; drops on
   `Success`/`Error`/`Cancelled`/`WaitingForEvents`/`Blocked`; re-acquires on a
   subsequent return to `InProgress`. The test must fail before the change (no model →
   no guard held during the `InProgress` local-execution window) and pass after. The
   implementation may add a test seam (e.g. a guard acquire/release counter or a
   mockable backend) to make the presence/absence assertion concrete on the Linux/no-op
   target.
4. *Regression test (unit) — cancel releases* — a unit test asserting that the
   cancellation path (`cancel_conversation_progress` → `Cancelled`) on an `InProgress`
   conversation drops the guard for that conversation.
5. *No leak on cleanup (unit)* — a unit test asserting guards are dropped on
   `ClearedConversationsForTerminalSurface` (for each `cleared_conversation_id`),
   `RemoveConversation`, and `DeletedConversation`, so closing the pane / deleting the
   conversation does not hold a wake assertion.
6. *Per-request guard removed, no regression (unit + tests)* — the
   `.prevent_sleep("Agent Mode request in-progress")` call at
   `crates/warp_multi_agent_client/src/lib.rs:59` is **removed** (the diff shows it gone;
   a `grep` for `.prevent_sleep(` over the workspace returns only the spec file and the
   `http_client` `RequestBuilder::prevent_sleep` method definition). The existing
   `http_client` and `warp_multi_agent_client` tests still pass
   (`cargo nextest run -p http_client -p warp_multi_agent_client`). Reasoning: the
   conversation-scoped guard is acquired on `InProgress` (set when a request is sent,
   before the SSE stream opens) and held across all turns, so it strictly supersedes the
   per-request guard — removing the per-request call site loses no wake-protection
   window. The `http_client` `prevent_sleep` plumbing stays (inert when no caller sets a
   reason).
7. *Refresh behavior (unit) — active conversation stays awake past the cap* — a unit
   test that simulates a long conversation: acquire on `InProgress`, then fire refresh
   events (new turn `StreamInit`, an action-result drain, an LRC poll completion, a
   user-input resume) at intervals **shorter** than `CAP` (use an injected/mock clock or
   a small test-only `CAP`), advancing simulated time past the original deadline each
   time. Assert the guard remains held continuously (no release) across `>` `CAP` of
   simulated time as long as refresh events keep coming. This verifies an active
   multi-turn / LRC-polled conversation stays awake indefinitely.
8. *Cap-expiry behavior (unit) — stuck conversation releases after the cap* — a unit
   test that acquires the guard on `InProgress`, fires **no** refresh events, advances
   simulated time past `CAP`, and asserts: the guard is dropped (released) even though
   the conversation status is still `InProgress`; a `log::warn!` is emitted; and the
   `AgentRunSleepGuardCapExpired` telemetry event is recorded (assert via a test
   telemetry recorder). Then fire a refresh event (e.g. a new turn) and assert the guard
   is re-acquired with a fresh deadline. This verifies a stuck conversation/LRC releases
   the assertion in a bounded window and re-acquires if it resumes.
9. *Auto-handoff-on-sleep not regressed* — the LRC skip at
   `app/src/workspace/auto_handoff.rs:328-330` is unchanged and no new setting/flag
   affecting handoff gating is introduced. Checked by: the existing `auto_handoff` tests
   pass; reasoning that the change is complementary (handoff already skips LRCs, so
   covering the LRC gap here does not alter handoff's decision).
10. *Wasm no-op / compiles* — the new model compiles and registers on wasm without
    changing behavior (the guard is a no-op on wasm). Checked by: a wasm build of the
    `app` crate (the repo's documented wasm target/check) succeeds.
11. *Repository checks (scope-proportional per the factory-verification mandate)* —
    `./script/format` (or `cargo fmt --all --check`) passes; `cargo clippy --workspace
    --all-targets --all-features --tests -- -D warnings` passes on the touched crates; the
    focused nextest suite for the touched modules (the new guard model, the
    `app/src/ai/blocklist` controller/history_model wiring, `prevent_sleep`,
    `warp_multi_agent_client`) passes. The repo's `./script/presubmit` / PR CI is the
    full-suite backstop.
