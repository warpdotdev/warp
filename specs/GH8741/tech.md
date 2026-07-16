# Tech Spec: Wait for a file view opened by `warpctrl`

Issue: https://github.com/warpdotdev/Warp/issues/8741
Product spec: `specs/GH8741/product.md`

## Context

- `FileOpenArgs` and `run_file_command` define the CLI and map it to the existing
  `file.open` action (`crates/warp_cli/src/local_control/mod.rs:720`,
  `crates/warp_cli/src/local_control/commands.rs:682`). `FileOpenParams` is the
  strict wire type (`crates/local_control/src/protocol.rs:95`).
- The client already makes one blocking HTTP request. The server dispatches it
  through `LocalControlBridge`, which currently returns a response synchronously
  from the WarpUI model thread (`app/src/local_control/mod.rs:508`,
  `app/src/local_control/bridge.rs:40`,
  `app/src/local_control/handlers/app_state.rs:748`).
- `open_file_notebook` and `open_code` create or focus the selected Markdown pane
  or code tab (`app/src/workspace/view.rs:8415`,
  `app/src/workspace/view.rs:8507`). A code file's logical owner is `TabData`,
  while a Markdown view is owned by `FileNotebookView`.
- View identity cannot be inferred from a path, pane ID, tab index, or view
  handle. Code-tab moves may rebuild a pane from its path
  (`app/src/code/view.rs:1194`), while `DetachType` distinguishes moves from
  closes (`app/src/pane_group/pane/mod.rs:541`).
- `warp_util::sync::Condition` is cloneable, set-once, and safe for multiple or
  late waiters (`crates/warp_util/src/sync.rs:30`).

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

Add `app/src/file_view_lifecycle.rs` with:

- `FileViewLifecycle`, a cloneable wrapper around `Condition`;
- idempotent `close()` and late-waiter-safe `wait_until_closed()` methods; and
- `FileOpenReceipt`, which returns the selected lifecycle to local control.

The lifecycle has no UUID, path registry, serialization, or save status. UI
removal paths close it explicitly; `Drop` does not. App teardown therefore ends
the transport instead of impersonating a user close.

### 3. Return the lifecycle selected by open/focus

Change `WorkspaceView::open_file_with_target` and its code/Markdown helpers to
return the lifecycle of the exact view they create or focus:

- `open_file_notebook` returns the matching existing pane's lifecycle or the
  newly created pane's lifecycle.
- `open_code` and `CodeView::open_or_focus_existing` return the selected
  `TabData` lifecycle; new tabs receive a new lifecycle.
- Existing non-control callers may ignore the receipt. A missing receipt is an
  internal error for `file.open`, whose target resolver guarantees an in-Warp
  code or Markdown view.

Selection and receipt retrieval occur in the same model-thread update, avoiding
an open-then-search race and path ambiguity.

### 4. Carry code-tab lifecycles through ownership changes

Add a `FileViewLifecycle` to `TabData`.

- Reorders and transfers preserve it.
- `remove_tab_for_move` extracts and passes it into the rebuilt pane instead of
  using the close-oriented removal path.
- Actual removal after unchanged close, Save, or Discard calls `close()`; Cancel
  does not.
- Detaching a containing `CodePane` as `HiddenForClose` or `Closed` closes all
  tab lifecycles; `Moved` preserves them.
- If merge deduplication removes the source tab in favor of an existing
  same-path destination, close the source lifecycle.

### 5. Apply the same contract to Markdown panes

Add a `FileViewLifecycle` to `FileNotebookView` and expose it through
`FilePane`.

- Reopening returns the existing lifecycle; creation returns a new one.
- `FilePane::detach` closes it for `HiddenForClose` and `Closed`, but not
  `Moved`.
- Rendered/raw mode changes preserve it. Replacing the viewer with a distinct
  code-editor view closes it.

Loading, reload, error display, and file watching remain unchanged.

### 6. Defer only waiting responses

Let `LocalControlBridge::handle_request` return either:

- `Ready(ResponseEnvelope)` for existing actions and non-waiting opens; or
- a private deferred result containing the request ID, acknowledgement data,
  and selected lifecycle.

The bridge still validates and mutates UI state synchronously. The Axum request
task awaits the lifecycle off the UI thread, then emits the ordinary
acknowledgement. Disconnecting never dispatches a close or stores mutable
per-client state on the view. Server or app shutdown drops the connection and
uses the existing nonzero transport error.

## Testing and validation

| Area | Coverage |
| --- | --- |
| CLI and protocol | Parse default/true values; compose with existing arguments and selectors; omit false from serialization; round-trip true; keep only `file.open`. Covers invariants 1, 2, 10, and 11. |
| Lifecycle unit tests | Multiple and late waiters, idempotent close, and canceling one waiter. Covers invariants 5 and 7. |
| Code and Markdown ownership | New/reused views, duplicate paths, reorder, move, merge, containing-view close, Save, Discard, and Cancel. Covers invariants 2–7 and 11. |
| Local control and client | Immediate default response, delayed response, two waiters, dropped caller, and server/transport loss. Covers invariants 1 and 7–10. |

Manual validation must exercise code and Markdown views, existing and duplicate
paths, moves, Save/Discard/Cancel, two waiters, Ctrl+C, and Warp shutdown. Before
pushing implementation, run the focused tests, `./script/format`, and the
repository-required Clippy command.

## Risks and mitigations

- **False completion during moves:** carry the lifecycle through move-specific
  extraction; never use close-oriented removal.
- **Wrong duplicate:** return the lifecycle from the atomic open/focus operation
  instead of looking it up later.
- **Lost notification:** use `Condition` so close state survives races and late
  registration.
- **Blocked UI:** await only in the Axum request task.
- **App exit reported as close:** complete only from explicit UI removal, never
  `Drop`.
