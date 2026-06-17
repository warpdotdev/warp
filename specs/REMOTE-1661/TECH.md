# Recoverable Shared-Session Viewer Initial Joins

Issue: [REMOTE-1661](https://linear.app/warpdotdev/issue/REMOTE-1661/errors-joining-a-cloud-mode-session-can-leave-the-pane-in-a-broken)

Product spec: [PRODUCT.md](./PRODUCT.md)

## Context

`PRODUCT.md` defines the user-visible contract: transient failure before first display is retried within a bounded interval, terminal/exhausted failure renders an actionable state, and post-join reconnect behavior remains unchanged.

The current implementation has a lifecycle gap at this boundary:

- [`app/src/terminal/shared_session/viewer/network.rs:57-75 @ a572a2d9df85616f3514495864f8c063f0e6188d`](https://github.com/warpdotdev/warp/blob/a572a2d9df85616f3514495864f8c063f0e6188d/app/src/terminal/shared_session/viewer/network.rs#L57-L75) defines exponential retry only for reconnection after a session has been established.

- [`app/src/terminal/shared_session/viewer/network.rs:157-199 @ a572a2d9df85616f3514495864f8c063f0e6188d`](https://github.com/warpdotdev/warp/blob/a572a2d9df85616f3514495864f8c063f0e6188d/app/src/terminal/shared_session/viewer/network.rs#L157-L199) creates a viewer in `Stage::BeforeJoined` and begins the initial websocket attempt.

- [`app/src/terminal/shared_session/viewer/network.rs:308-423 @ a572a2d9df85616f3514495864f8c063f0e6188d`](https://github.com/warpdotdev/warp/blob/a572a2d9df85616f3514495864f8c063f0e6188d/app/src/terminal/shared_session/viewer/network.rs#L308-L423) starts heartbeat after the transport connects. Connection establishment failures emit `FailedToJoin`, but a stream ending before join acknowledgement closes the socket and schedules no replacement attempt or terminal event.

- [`app/src/terminal/shared_session/viewer/network.rs:426-488 @ a572a2d9df85616f3514495864f8c063f0e6188d`](https://github.com/warpdotdev/warp/blob/a572a2d9df85616f3514495864f8c063f0e6188d/app/src/terminal/shared_session/viewer/network.rs#L426-L488) implements resume reconnect and requires an `event_loop`; it cannot recover an initial attempt that has not joined.

- [`app/src/terminal/shared_session/viewer/network.rs:536-649 @ a572a2d9df85616f3514495864f8c063f0e6188d`](https://github.com/warpdotdev/warp/blob/a572a2d9df85616f3514495864f8c063f0e6188d/app/src/terminal/shared_session/viewer/network.rs#L536-L649) creates `event_loop` only on `JoinedSuccessfully` and passes explicit server `FailedToJoin` to the manager.

- [`app/src/terminal/shared_session/network/heartbeat.rs:10-96 @ a572a2d9df85616f3514495864f8c063f0e6188d`](https://github.com/warpdotdev/warp/blob/a572a2d9df85616f3514495864f8c063f0e6188d/app/src/terminal/shared_session/network/heartbeat.rs#L10-L96) owns ping and idle timers but currently has no explicit stop API for an abandoned initial attempt.

- [`app/src/terminal/shared_session/viewer/terminal_manager.rs:415-499 @ a572a2d9df85616f3514495864f8c063f0e6188d`](https://github.com/warpdotdev/warp/blob/a572a2d9df85616f3514495864f8c063f0e6188d/app/src/terminal/shared_session/viewer/terminal_manager.rs#L415-L499) installs pending network/write state and sets `SharedSessionStatus::ViewPending` before acknowledgement. [`terminal_manager.rs:918-935 @ a572a2d9df85616f3514495864f8c063f0e6188d`](https://github.com/warpdotdev/warp/blob/a572a2d9df85616f3514495864f8c063f0e6188d/app/src/terminal/shared_session/viewer/terminal_manager.rs#L918-L935) currently displays a failure toast while deliberately leaving that status pending.

- [`app/src/terminal/shared_session/mod.rs:99-199 @ a572a2d9df85616f3514495864f8c063f0e6188d`](https://github.com/warpdotdev/warp/blob/a572a2d9df85616f3514495864f8c063f0e6188d/app/src/terminal/shared_session/mod.rs#L99-L199) has pending, active, and finished viewer states but no representation for a viewer that never completed initial join. [`app/src/terminal/view.rs:23373-23405 @ a572a2d9df85616f3514495864f8c063f0e6188d`](https://github.com/warpdotdev/warp/blob/a572a2d9df85616f3514495864f8c063f0e6188d/app/src/terminal/view.rs#L23373-L23405) and [`app/src/terminal/view.rs:27020-27045 @ a572a2d9df85616f3514495864f8c063f0e6188d`](https://github.com/warpdotdev/warp/blob/a572a2d9df85616f3514495864f8c063f0e6188d/app/src/terminal/view.rs#L27020-L27045) render pending/loading surfaces from those states.

## Proposed changes

### 1. Distinct bounded initial-join retry lifecycle

In `app/src/terminal/shared_session/viewer/network.rs`, add a separate initial-join retry operation distinct from the existing post-join `reconnect_websocket` method:

- Create an initial-join retry strategy with bounded backoff (e.g., exponential backoff starting at ~1s, capped at 15-30s total window).
- On socket termination during `Stage::BeforeJoined`, classify the failure as retryable (transport/timeout/EOF) or terminal (explicit protocol rejection, session not found, access denied).
- Retry only transport failures; do not retry explicit `FailedToJoin` protocol responses.
- On each retry, re-send the original `Initialize` message.
- Preserve Cloud Mode `initial_load_mode` and connection parameters across retries.
- Emit `NetworkEvent::FailedToJoin` when retries are exhausted or a terminal failure is encountered, preserving whether a failed state can be retried manually.

### 2. Complete and race-safe cleanup

- Add an explicit cancel/stop API to `app/src/terminal/shared_session/network/heartbeat.rs` so that an abandoned initial-join attempt can shut down heartbeat timers promptly.
- Per retry attempt, replace websocket proxy channels to prevent stale messages from prior attempts.
- Guard against late async completions from prior join attempts that may fire after a new attempt has begun.

### 3. Failed-initial-join viewer state and UI

In `app/src/terminal/shared_session/mod.rs` and `app/src/terminal/view.rs`:

- Add a failed-initial-join viewer state distinct from `ViewPending` and `FinishedViewer`.
- On terminal failure, clear `SharedSessionStatus::ViewPending` and transition to the new failed state.
- Render an explicit error/recovery surface that communicates the join could not be completed and offers a same-pane retry action only for retryable failures (for example, exhausted transport attempts or an internal server error).
- On retry activation from a retryable failed state, start a new initial-join attempt and return to joining/loading UI; reject the retry action for terminal failures such as unavailable sessions, invalid links, access denial, or capacity limits.

### 4. Post-join behavior unchanged

- Preserve all existing post-join reconnect, session-ended, and access-control semantics.
- Do not change the server protocol or server-side join/auth behavior.

## Tests and validation

### Unit/integration tests

- `app/src/terminal/shared_session/viewer/network_tests.rs`: test socket closure during `Stage::BeforeJoined`, verify retry classification, verify terminal/exhausted transitions.
- `app/src/terminal/shared_session/viewer/terminal_manager_tests.rs`: test failed-initial-join state transition, verify viewer state/UI rendering, test same-pane retry action.
- `app/src/terminal/view/shared_session/view_impl_tests.rs` or `app/src/terminal/view_tests.rs`: test pending→failed-join→loading→joined or pending→failed-join UI transitions.

### Manual validation

- During implementation run targeted `cargo nextest`, `./script/format --check`, `./script/presubmit`.
- Manually verify pre-join EOF/socket close produces a failed-join surface and retry succeeds.
- Manually verify terminal server rejections (session not found, access denied) do not retry and render appropriate error.
- Verify Cloud Mode context and `initial_load_mode` are preserved across a successful retry.

## Risks

- **Duplicate sockets or late completions**: without careful async task cleanup, stale completion handlers from prior attempts could re-establish a socket or incorrectly update state. Use task cancellation and generation/ID tagging per attempt.

- **Stale heartbeat reconnect**: if heartbeat timers are not explicitly stopped on an abandoned attempt, a heartbeat timeout could trigger a reconnect using the old attempt's state. Implement explicit heartbeat cancellation.

- **Terminal error misclassification**: classify server protocol errors too narrowly and some retryable failures may be treated as terminal and vice versa. Research and test against actual server failure modes.

- **Cloud Mode local context loss**: incorrect scope/cleanup could discard pane-local context (e.g., `initial_load_mode`, provisional state) across retries. Preserve all local state across retry boundaries.

## Parallelization

No child implementation is proposed. The network/state/render contract is tightly coupled and requires a single coordinated change.
