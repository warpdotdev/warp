# Resume cloud agent runs on transient transport failures — tech spec

Linear: [REMOTE-1894](https://linear.app/warpdotdev/issue/REMOTE-1894/oz-cloud-runs-error-mid-execution-from-connectivitytransport-failures) · Product spec: [PRODUCT.md](./PRODUCT.md) · PR: [warp#12431](https://github.com/warpdotdev/warp/pull/12431)

References are pinned to `84276e07` (the implementation branch); behavior invariants `I<n>` refer to PRODUCT.md.

## Context

Every agent turn is one streaming MAA request (`POST /ai/multi-agent`) wrapped by `ResponseStream`, which emits events to `BlocklistAIController`; the controller updates the conversation in `BlocklistAIHistoryModel`, whose status drives the headless driver (cloud runs), `LocalAgentTaskSyncModel` (run-state reporting), and the UI.

On master, recovery was broken in four compounding ways (the failure modes in Roland's run links):

- Retry required `is_online` and was abandoned otherwise; the "attempt 1/3" path could fail without ever recovering.
- A stream that ended in a clean EOF without a `StreamFinished` event (how mid-body `ConnectionReset`/`close_notify` aborts surface through reqwest) was hardcoded as a non-resumable terminal error in the controller.
- When a resume *was* scheduled, the sync model still reported terminal ERROR immediately — tearing down the cloud execution before the resume could land — and the conversion in `ambient_agents` dropped the structured error (and its resume flag) in favor of the stringly `status_error_message`.
- Recovery state was smuggled through a `will_attempt_resume` rendering flag on the error rather than modeled in the conversation state machine.

Key files:

- [`app/src/ai/blocklist/controller/response_stream.rs:41 @ 84276e07`](https://github.com/warpdotdev/warp/blob/84276e0732860798fa49eb7372e6ee90cdf1728b/app/src/ai/blocklist/controller/response_stream.rs#L41) — request lifecycle, retry/resume decision
- [`app/src/ai/blocklist/controller.rs:2783 @ 84276e07`](https://github.com/warpdotdev/warp/blob/84276e0732860798fa49eb7372e6ee90cdf1728b/app/src/ai/blocklist/controller.rs#L2783) — stream-event handling, resume spawn, cancellation
- [`app/src/ai/agent/conversation.rs:4205 @ 84276e07`](https://github.com/warpdotdev/warp/blob/84276e0732860798fa49eb7372e6ee90cdf1728b/app/src/ai/agent/conversation.rs#L4205) — conversation status state machine
- [`app/src/ai/agent_sdk/driver.rs:2793 @ 84276e07`](https://github.com/warpdotdev/warp/blob/84276e0732860798fa49eb7372e6ee90cdf1728b/app/src/ai/agent_sdk/driver.rs#L2793) — cloud-run lifecycle decisions
- [`app/src/ai/blocklist/local_agent_task_sync_model.rs:322 @ 84276e07`](https://github.com/warpdotdev/warp/blob/84276e0732860798fa49eb7372e6ee90cdf1728b/app/src/ai/blocklist/local_agent_task_sync_model.rs#L322) — conversation → run-state mapping

## Implementation

### `ConversationStatus::TransientError` (client-only state)

[`conversation.rs:4205`](https://github.com/warpdotdev/warp/blob/84276e0732860798fa49eb7372e6ee90cdf1728b/app/src/ai/agent/conversation.rs#L4205): new non-terminal status ("Reconnecting", in-progress icon treatment — I5). `mark_request_completed_with_error` takes `recovery_pending: bool` and sets `TransientError` vs `Error`; the exchange itself is still marked finished-with-error so the structured error is preserved for rendering and restore. Consumers updated exhaustively (no wildcard arms):

- Driver ([`driver.rs:2793`](https://github.com/warpdotdev/warp/blob/84276e0732860798fa49eb7372e6ee90cdf1728b/app/src/ai/agent_sdk/driver.rs#L2793)): `TransientError` → `end_run_after(AUTO_RESUME_TIMEOUT = 120s)` with the last structured error (I11); a recovery flips status back to `InProgress`, cancelling the deadline, and the next failure arms a fresh deadline — so the window is per attempt, not per turn. The old `will_attempt_resume` check in the Error arm is gone — `Error` is always terminal now.
- Sync model ([`local_agent_task_sync_model.rs:322`](https://github.com/warpdotdev/warp/blob/84276e0732860798fa49eb7372e6ee90cdf1728b/app/src/ai/blocklist/local_agent_task_sync_model.rs#L322)): `TransientError` → `IN_PROGRESS` with no status message (I6); `task_update_for_conversation_error` ignores the `will_attempt_resume` rendering hint (terminal classification only).
- `ambient_agents::conversation_output_status_from_conversation`: early `None` for `TransientError`; for terminal `Error` it now prefers the structured exchange error over `status_error_message`.
- Run lists / pill bar / aggregation / notifications / queued-prompt gating treat it as working (I7, I8, I19).
- Restore-from-disk derives status from exchanges, so `TransientError` restores as terminal `Error` (I18, accepted).

### Strict retry/resume split

[`response_stream.rs:41`](https://github.com/warpdotdev/warp/blob/84276e0732860798fa49eb7372e6ee90cdf1728b/app/src/ai/blocklist/controller/response_stream.rs#L41): pure `recovery_action(has_received_client_actions, is_recoverable, recovery, is_online) -> {Retry, RetryWhenOnline, Resume, Fail(reason)}`:

- No actions yet + retryable + budget → retry, verbatim re-send after a backoff (I2). Offline parks the retry instead (`RetryWhenOnline`).
- Actions received + transient + resume-eligible + budget → `ResumeConversation` (I3).
- Everything else → `Fail(reason)` (terminal), where the reason distinguishes a non-transient error, an exhausted budget, and a request that may not resume at all. Recovery eligibility (retry and resume) uses `AIApiError::is_recoverable()` ([`server_api.rs:341`](https://github.com/warpdotdev/warp/blob/84276e0732860798fa49eb7372e6ee90cdf1728b/app/src/server/server_api.rs#L341)) (I12).

Both kinds of recovery draw from one `RecoveryBudget` (I9), a `Copy` value the request carries: a retry charges it in place, and a resume hands the charged budget to the `ResumeConversation` request the controller sends next, as a `PendingResume` (`pending_resume` → `schedule_auto_resume_after_error`). The scope is one request and its recoveries, not one agent turn — `send_follow_up_for_conversation` and the other outbound paths each start a `RecoveryBudget::fresh()`, as they did when the budget was a per-stream `retry_count`. `resume_allowed = false` now means only "this request kind may never resume" (the passive-request clamp, I17), not "recovery is over". Every attempt waits `retry_strategies::backoff_after_attempts`, the same jittered exponential schedule `with_bounded_retry` uses (I10); the resume's wait is decided with the decision and carried on `PendingResume` rather than recomputed at send time, since the schedule is jittered and a recomputed value would not be the one that was logged. Raised from the original one-shot resume by [REMOTE-2269](https://linear.app/warpdotdev/issue/REMOTE-2269/allow-multiple-resume-attempts).

The controller passes `recovery_pending = should_resume_conversation_after_stream_finished()` into the history model; `will_attempt_resume` on `RenderableAIError` remains rendering-only.

### Silent-EOF synthesis (`AIApiError::UnexpectedEof`)

The server always sends `StreamFinished`, but a transport cut between chunks reaches the stream layer as a clean EOF with no error event — empirically confirmed (both e2e scenarios below exercised only this path; the `Err`-event arm never fired). `ResponseStream` tracks `stream_finished_received` / `error_event_emitted` and, on completion without either, synthesizes `UnexpectedEof` (retryable + transient) and runs the same `recovery_action` decision ([`response_stream.rs:388`](https://github.com/warpdotdev/warp/blob/84276e0732860798fa49eb7372e6ee90cdf1728b/app/src/ai/blocklist/controller/response_stream.rs#L388)). This replaces the controller's hardcoded-terminal EOF fallback (now a defensive warn). HTTP send failures do not take this path — they arrive as in-stream error events; request-conversion failures (no stream ever created) are surfaced terminally with the original error instead of being synthesized as `UnexpectedEof`. Non-retried failures from both paths report to Sentry via a shared helper with classification tags.

### Offline parking

`defer_retry_until_online` parks a retry on `NetworkStatus::wait_until_online()`, suppressing completion of the failed attempt (`deferred_retry_pending`) and emitting `WaitingForNetwork { waiting }`; the controller mirrors it onto the conversation (`TransientError` ↔ `InProgress`) without finishing any exchange (I13). The resume spawn in `AfterStreamFinished` also waits for connectivity (I14). Parked work is invalidated by `current_request_id` on cancellation/supersession.

### Cancellation and replacement

`cancel_conversation_progress` aborts the parked resume handle and, when no active stream remains, flips a `TransientError` conversation to `Cancelled` directly (I15). `send_request_input` aborts the pending resume for the conversation before sending (I16) and clamps the budget with `RecoveryBudget::without_resume` for passive requests (I17).

## Testing and validation

Unit (all in-tree, `cargo nextest run -p warp --lib`):

- `response_stream_tests.rs` — exhaustive `recovery_action` matrix: retry/park/fail pre-actions (I2, I9, I13), resume gating post-actions (I3, I9), budget exhaustion and non-retryable → fail (I10, I12); the shared budget across a retry/resume mix and across repeated resumes, plus a bound tying the backoff schedule to `AUTO_RESUME_TIMEOUT` (I9, I10, I11).
- `server_api_tests.rs` — `is_recoverable` classification: 5xx/timeout/transport and app-level (quota/overload/misc/JSON) recoverable, other 4xx not (I12); `UnexpectedEof` recoverable (I1).
- `history_model_tests.rs` — `recovery_pending` → `TransientError` and no terminal derived outcome (I5, I6 upstream); structured exchange error preserved through conversion; non-recoverable error stays terminal.
- `local_agent_task_sync_model_tests.rs` — `TransientError` → `IN_PROGRESS` with no status message (I6); `will_attempt_resume` hint ignored for terminal classification (I10, I12).

E2e (oz-local + warp-server `TransportReset` LLM mock, `simulate_maa_transport_reset: true`):

- Recovery (I1, I3–I6, I11): single mid-stream reset → client log shows the synthesized truncation → conversation `TransientError` → driver "automatic recovery pending — waiting up to 120s" → resume fires → run completes with the mocked `finish_task`. Verified 2026-06-10.
- Bounded failure (I9, I10): cycling resets → exactly one resume, then terminal ERROR with the friendly lost-connection message; 2 LLM calls total, no retry storm. Verified 2026-06-10, against the pre-REMOTE-2269 one-shot resume; the shared budget makes this 3 spaced attempts before the same terminal error.
- The pre-action retry paths (I2) were not exercised by these scenarios, so they are covered by the unit matrix plus the request-send-failure path. The mock cut the stream at the LLM call, by which point the server had already emitted that request's user-message-append `ClientActions` — so it could not produce a failure before the first `ClientActions` event, which is what the retry paths need. That is a limit of where the mock injected the reset, **not** a property of cloud runs: `has_received_client_actions` flips only when a `ClientActions` event actually arrives, so any failure ahead of the first one (initial connection error, pre-header 5xx, early reset) does take the retry path with the full budget. [REMOTE-2269](https://linear.app/warpdotdev/issue/REMOTE-2269/allow-multiple-resume-attempts) records one from a real cloud run: `MultiAgent request failed, retrying (attempt 1/3) - Error: ErrorStatus(503, "upstream connect error or disconnect/reset before headers. reset reason: remote connection failure")`.

Lint/build gates: `cargo fmt`, `cargo clippy -p warp --lib --tests -- -D warnings`, `cargo check -p warp --lib --features crash_reporting`.

Not covered by automated tests (manual/review only): controller cancellation-during-recovery (I15, I16), notification suppression (I8), and the offline `wait_until_online` integration (I13, I14) — exercised via the unit matrix decision but not a live network flap.

## Risks and mitigations

- **Stuck "Reconnecting"**: any path that sets `TransientError` and never delivers a recovery would hang the UI state. Mitigated by the driver's 120s per-attempt deadline (cloud), the shared recovery budget, and the cancellation flip to `Cancelled`; restore-from-disk degrades it to `Error`.
- **Behavior change**: quota/overload failures no longer set the resume flag (master did). Intentional — a resume would fail identically — but visible to anyone who relied on the old flag.
- **Sentry volume**: synthesized truncations now report (with `will_attempt_resume`/`is_recoverable` tags) — expect new `UnexpectedEof` events that previously appeared as the generic EOF fallback.

## Parallelization

Not proposed: the change is a single tightly-coupled state machine (status variant + decision function + consumers), already implemented and validated on one branch (PR #12431). Splitting it across agents would have created merge conflicts in `controller.rs`/`response_stream.rs` without wall-clock benefit.
