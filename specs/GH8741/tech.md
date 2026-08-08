# Tech Spec: Wait for a file view opened by `warpctrl`

Issue: https://github.com/warpdotdev/Warp/issues/8741
Product spec: `specs/GH8741/product.md`

## Context

- `FileOpenArgs` and `run_file_command` define the CLI and map it to the existing
  `file.open` action (`crates/warp_cli/src/local_control/mod.rs:720`,
  `crates/warp_cli/src/local_control/commands.rs:682`). `FileOpenParams` is the
  strict wire type (`crates/local_control/src/protocol.rs:95`).
- The client makes one blocking HTTP request with
  `reqwest::blocking::Client::new()` (`crates/local_control/src/client.rs:54`).
  The blocking wrapper applies a 30-second default timeout per operation even
  though the async client defaults to none. The server is bare Axum with no
  timeout layers (`app/src/local_control/mod.rs:252`) and holds a request open
  indefinitely. It dispatches through `LocalControlBridge`, which currently
  returns a response synchronously from the WarpUI model thread
  (`app/src/local_control/mod.rs:508`, `app/src/local_control/bridge.rs:40`,
  `app/src/local_control/handlers/app_state.rs:748`).
- `open_file_notebook` and `open_code` create or focus the selected Markdown
  pane or code-editor tab (`app/src/workspace/view.rs:8415`,
  `app/src/workspace/view.rs:8507`).
- Two types are named `TabData`: the workspace tab (`app/src/tab.rs:170`), which
  holds a `PaneGroup`, and the code-editor tab (`app/src/code/view.rs:207`),
  which represents one file inside a `CodeView`. This spec always means the
  editor tab. `CodeView` holds `tab_group: Vec<TabData>`
  (`app/src/code/view.rs:237`), so one code pane can own several open files.
- `PaneView<P: BackingView>` is the generic backing-view wrapper
  (`app/src/pane_group/pane/view/mod.rs:84`), and `Pane::detach` receives a
  `DetachType` that distinguishes moves from closes
  (`app/src/pane_group/pane/mod.rs:541`). `PaneGroup` stores panes as
  `Box<dyn AnyPaneContent>` (`app/src/pane_group/mod.rs:893`), so generic
  lifecycle access needs an accessor on the `PaneContent` trait.
- Editor-tab moves rebuild the destination from `CodeSource::Link` rather than
  carrying the tab (`CodeView::remove_tab_for_move`,
  `app/src/code/view.rs:1194`). Drag-merges deduplicate same-path tabs in
  `CodeView::merge_tabs` (`app/src/code/view.rs:2089`), reached from the
  `DroppedOnTabBar` path (`app/src/workspace/view.rs:16457`).
- `warp_util::sync::Condition` is cloneable, set-once, and safe for multiple or
  late waiters (`crates/warp_util/src/sync.rs:30`).
- `warpctrl` is the app binary re-invoked via `--warpctrl`
  (`app/src/lib.rs:701`). The bundled macOS wrapper `exec`s the binary
  (`script/macos/create_warpctrl_wrapper`), so terminal signals reach the CLI
  process directly.

## Proposed changes

### 1. Extend the existing CLI and wire parameter

- Add a Clap `--wait` boolean to `FileOpenArgs`.
- Add `wait: bool` with a Serde default to `FileOpenParams`; omit it when false.
- Pass it through the existing `ActionKind::FileOpen` request.
- Document `file open --wait` and Ctrl+C behavior in
  `resources/bundled/skills/warpctrl/SKILL.md`.

Do not add an action, endpoint, result type, or protocol version. Default
requests retain their wire shape. An older strict server rejects `wait: true`
with `InvalidParams` instead of silently ignoring it.

### 2. Represent one private logical lifetime

Add `app/src/pane_group/lifecycle.rs` with:

- `ViewLifecycle`, a cloneable handle built on `Condition` with an idempotent
  `close()` and a late-waiter-safe `wait_until_closed()`;
- `redirect(target)`, which re-points the lifecycle's current and future
  waiters at `target` so a merged-away view's waiters follow the surviving
  view. Waiting follows redirect chains; `close()` on a redirected lifecycle is
  a no-op; and
- `FileOpenReceipt`, which returns the selected lifecycle to local control.

The lifecycle has no UUID, path registry, serialization, or save status. UI
removal paths close it explicitly; `Drop` does not. App teardown therefore ends
the transport instead of impersonating a user close.

### 3. Return the lifecycle selected by open/focus

Change `WorkspaceView::open_file_with_target` and its code/Markdown helpers to
return the lifecycle of the exact view they create or focus:

- `open_file_notebook` returns the `PaneView` lifecycle of the matching
  existing `FilePane` or of the newly created one.
- `open_code` and `CodeView::open_or_focus_existing` return the selected
  editor-tab lifecycle; new editor tabs receive a new lifecycle.
- Existing non-control callers may ignore the receipt. A missing receipt is an
  internal error for `file.open`, whose target resolver guarantees an in-Warp
  code or Markdown view.

Selection and receipt retrieval occur in the same model-thread update, avoiding
an open-then-search race and path ambiguity.

### 4. Two-level ownership: every pane, then editor tabs

**Level 1 — `PaneView`.** Add a `ViewLifecycle` to `PaneView`, exposed through
an accessor on the `PaneContent` trait so `PaneGroup`'s type-erased storage can
reach it. `detach` closes it for `DetachType::HiddenForClose` and
`DetachType::Closed` and preserves it for `DetachType::Moved`. This covers
every `BackingView` implementation. For Markdown it is the whole story: a
`FilePane` is one pane showing one file. Reopening returns the existing
lifecycle; rendered/raw mode changes preserve it; replacing the viewer with a
distinct code-editor view closes it. Loading, reload, error display, and file
watching remain unchanged.

**Level 2 — editor tabs.** Add a `ViewLifecycle` to the editor `TabData`,
because a `CodeView` multiplexes several files in one pane and a pane-scoped
wait would outlive an individual file's tab.

- Reorders and transfers preserve it. `remove_tab_for_move` extracts the
  lifecycle and passes it into the pane rebuilt from `CodeSource::Link` instead
  of using the close-oriented removal path.
- Actual removal after unchanged close, Save, or Discard calls `close()`;
  Cancel does not.
- When the owning pane's lifecycle closes, all of its editor-tab lifecycles
  close with it.
- If merge deduplication removes the source editor tab in favor of an existing
  same-path destination tab (`merge_tabs` and the `DroppedOnTabBar` path),
  redirect the source lifecycle to the destination's: the file is still on
  screen, so the wait continues until the surviving tab closes (product
  invariant 4).

### 5. Defer only waiting responses

Let `LocalControlBridge::handle_request` return either:

- `Ready(ResponseEnvelope)` for existing actions and non-waiting opens; or
- a private deferred result containing the request ID, acknowledgement data,
  and selected lifecycle.

The bridge still validates and mutates UI state synchronously. The Axum request
task awaits the lifecycle off the UI thread, then emits the ordinary
acknowledgement. Disconnecting never dispatches a close or stores mutable
per-client state on the view. Server or app shutdown drops the connection and
uses the existing nonzero transport error.

**Client timeout.** The blocking reqwest client's 30-second default would fail
every wait at 30 seconds with a transport error, so the client builds waiting
requests with `.timeout(None)`. One-shot actions keep the default, where the
deadline is a feature. The server needs no change; it already holds a request
open indefinitely.

**Held connection.** `--wait` becomes the first local control action to hold
its connection open; every current `ActionKind` is one-shot. Keep the single
held request rather than adding a long-poll or SSE endpoint, and validate
idle-connection behavior — notably across macOS sleep/wake — before calling
the implementation done.

**Ctrl+C.** Because the bundled wrapper `exec`s the CLI binary, the default
SIGINT disposition kills the waiting process and drops the connection, which
the server observes as a disconnect with no view change. Product invariant 8
holds without a signal handler.

## Testing and validation

| Area | Coverage |
| --- | --- |
| CLI and protocol | Parse default/true values; compose with existing arguments and selectors; omit false from serialization; round-trip true; keep only `file.open`. Covers invariants 1, 2, 10, and 11. |
| Lifecycle unit tests | Multiple and late waiters, idempotent close, canceling one waiter, and redirects: waiters complete when the final target closes, and `close()` on a redirected source is a no-op. Covers invariants 4, 5, and 7. |
| Code and Markdown ownership | New/reused views, duplicate paths, reorder, move, merge redirect to the surviving tab, containing-view close, Save, Discard, and Cancel. Covers invariants 2–7 and 11. |
| Local control and client | Immediate default response, delayed response, no client timeout on waiting requests, two waiters, dropped caller, and server/transport loss. Covers invariants 1 and 7–10. |

Manual validation must exercise code and Markdown views, existing and duplicate
paths, moves, merge deduplication, Save/Discard/Cancel, two waiters, Ctrl+C
through the bundled wrapper, Warp shutdown, and a held wait that stays idle
across macOS sleep/wake. Before pushing implementation, run the focused tests,
`./script/format`, and the repository-required Clippy command.

## Risks and mitigations

- **False completion during moves:** carry the lifecycle through move-specific
  extraction; never use close-oriented removal.
- **False completion during merges:** redirect source waiters to the surviving
  tab's lifecycle instead of closing.
- **Wrong duplicate:** return the lifecycle from the atomic open/focus operation
  instead of looking it up later.
- **Lost notification:** use `Condition` so close state survives races and late
  registration.
- **Client deadline:** the blocking client defaults to a 30-second timeout;
  set `.timeout(None)` on waiting requests only.
- **Idle held connection:** hours-long loopback requests are untested across
  macOS sleep/wake; validate before release, with long-poll or SSE as the
  documented fallback if the held connection proves unreliable.
- **Blocked UI:** await only in the Axum request task.
- **App exit reported as close:** complete only from explicit UI removal, never
  `Drop`.
