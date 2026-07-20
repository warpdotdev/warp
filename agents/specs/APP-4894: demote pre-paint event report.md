*Proposed change: Demote the pre-paint EventHandler report to trace logging*

*Summary:* `EventHandler::dispatch_event` can legitimately run before its child has painted, leaving `child_max_z_index` unset. The current recoverable path calls `report_error!` on the `MouseMoved` hot path and floods Sentry (APP-4894). Replace that report with one static `log::trace!` message while preserving the existing safe `false` return and event propagation behavior.

*Key design choices:*
1. **Logging-only at `trace!`** — this is an expected lifecycle state on a very high-volume mouse-event path; it should remain available to local diagnostics but must not create a Sentry issue or breadcrumb.
2. **Preserve dispatch semantics** — keep dispatching to the child before checking paint state and keep returning `false` when no paint-derived z-index exists; do not add a new paint gate.
3. **Static, non-sensitive message** — use the existing stable text without event payloads or other per-instance data; `OncePerRun` is unnecessary because `trace!` is local-only and disabled by default.

*Design alternatives:*
- **`log::debug!` instead of `log::trace!`** — viable, but `trace!` better matches a per-`MouseMoved` hot path and the repo guidance for very fine-grained diagnostics.
- **Paint-gate event dispatch or initialize a fallback z-index** — rejected. The current element contract allows parents to dispatch into children before their first paint, and changing hit-testing/propagation would broaden the fix and risk dropping legitimate events. The existing `false` result is already the correct recoverable behavior.
- **Keep `report_error!` with `OncePerRun`, or demote to `warn!`/`error!`** — rejected. This state is not an actionable invariant failure, and `warn!`/`error!` still creates high-volume breadcrumbs; `report_error!` is the direct cause of the Sentry flood.

*Root cause / approach:* The `paint` implementation populates `child_max_z_index` (`crates/warpui_core/src/elements/gui/event_handler.rs:235-239 @ c86f67db3d61d5487301787e135ba890a7b9f243`), but retained or newly laid-out GUI subtrees can receive events before that first paint. In `dispatch_event`, the `None` branch at `crates/warpui_core/src/elements/gui/event_handler.rs:254-259 @ c86f67db3d61d5487301787e135ba890a7b9f243` should therefore be treated as an expected, handled-no-op state: emit the exact static message with `log::trace!`, then return `false`. Remove the now-unused `warp_errors::report_error` and event-discriminant plumbing if the implementation no longer references them.

*Affected files:*
- `crates/warpui_core/src/elements/gui/event_handler.rs` — replace the Sentry report in the pre-paint branch with static `log::trace!` logging and remove imports made obsolete by that change.
- `crates/warpui_core/src/elements/gui/event_handler_tests.rs` — add a focused regression test that dispatches before paint and verifies the recoverable return plus trace-only logging; retain all existing propagation and hover tests.

*Open questions resolved:* None. Triage confirmed the Sentry issue, the pre-paint reproduction, the recoverable `false` behavior, and the 5/5 focused-test baseline. The logging treatment follows `.agents/skills/logging-and-error-reporting/SKILL.md`: hot-path diagnostics use `debug!`/`trace!`, while Sentry reports are reserved for actionable failures.

*Risks / blast radius:* The change affects only diagnostics for an already-safe early return. The child-first dispatch order, z-index hit testing after paint, callback invocation, and propagation behavior remain unchanged. The trace message contains no user data, paths, IDs, prompts, or other PII, and trace logs are not uploaded to Sentry.

*Validation & verification criteria* (must ALL pass before merge):
1. **Pre-paint regression test (fails before, passes after):** add a named test in `crates/warpui_core/src/elements/gui/event_handler_tests.rs` (for example, `test_dispatch_before_paint_is_recoverable_and_trace_only`) that constructs an `EventHandler`, dispatches a `MouseMoved` event before `paint` has run (`child_max_z_index == None`), and verifies the call returns `false` without invoking a callback. Install/use the existing test logger pattern to assert that this call produces no `warp_errors::LOG_TARGET` report entry and, if the test captures the module log, exactly one `Trace` record with the unchanged static message `Dispatching event on EventHandler element which was never painted`.
2. **No Sentry reporting remains at the emitting site:** `event_handler.rs` no longer imports or invokes `report_error!` for this branch; the regression test and a source review confirm there is no `warp_errors::LOG_TARGET` report event or `extra: { "event" => ... }` payload from the pre-paint path.
3. **Normal EventHandler behavior is unchanged:** `CARGO_HOME=/tmp/warp-cargo-home cargo test --manifest-path /workspace/warp/Cargo.toml -p warpui_core event_handler --lib` passes, including the existing five tests for layered clicks, hover behavior, coverage, and propagation plus the new pre-paint regression.
4. **Original symptom is addressed:** replaying the confirmed pre-paint `MouseMoved` path no longer calls Sentry capture/reporting and remains a non-consuming `false` result; the only diagnostic is the static trace message, which is filtered out of Sentry under the logging guidelines.
5. **Presubmit:** `./script/presubmit` passes from `/workspace/warp` with formatting, linting, build, and the workspace's applicable tests clean. No computer-use screenshot is required because this is a logging-only/headless behavior change with no rendered UI difference.
