# Tech Spec: Wait for a file view opened by `warpctrl`

Issue: https://github.com/warpdotdev/Warp/issues/8741
Product spec: `specs/GH8741/product.md`

## Context

Warp Control already exposes one typed file action:

- `crates/warp_cli/src/local_control/mod.rs` defines
  `FileCommand::Open(FileOpenArgs)`. `FileOpenArgs` contains the path, optional
  line and column, `new_tab`, and the standard target selectors.
- `crates/warp_cli/src/local_control/commands.rs::run_file_command` maps that
  command to the existing `ActionKind::FileOpen` request.
- `crates/local_control/src/protocol.rs::FileOpenParams` is the strict wire
  parameter type. `crates/local_control/src/catalog.rs` declares `file.open`
  with an acknowledgement result.
- `crates/local_control/src/client.rs::send_request` performs one blocking HTTP
  request and reads one response. It has no client-side request timeout, so the
  existing transport can carry a long-lived wait without polling or streaming.
- `app/src/local_control/mod.rs::handle_control_request` authenticates the
  request, dispatches it to the main-thread `LocalControlBridge`, awaits the
  bridge result, and serializes one response.
- `app/src/local_control/bridge.rs::LocalControlBridge::handle_request` is
  synchronous on the WarpUI model thread and currently returns a
  `ResponseEnvelope` immediately.
- `app/src/local_control/handlers/app_state.rs::file_open` validates the
  parameters and target, calls `WorkspaceView::open_file_with_target`, and
  immediately returns the acknowledgement.

The selected in-Warp file surface depends on existing settings and state:

- `app/src/util/openable_file_type.rs::resolve_file_target_to_open_in_warp`
  guarantees that local-control `file.open` selects either Warp's code editor
  or its Markdown viewer, even if the user's general editor setting points to
  an external application.
- `app/src/workspace/view.rs::open_file_notebook` focuses an existing Markdown
  pane for the same path or creates a new `FilePane`.
- `app/src/workspace/view.rs::open_code` and
  `app/src/code/view.rs::CodeView::open_or_focus_existing` focus an existing
  code-editor tab or create a new one.
- A logical code-editor file is a `TabData` inside `CodeView`, not necessarily
  the containing pane. A logical Markdown file view is a `FileNotebookView`
  owned by `FilePane`.
- `app/src/pane_group/pane/mod.rs::DetachType` distinguishes a real close
  (`HiddenForClose` or `Closed`) from `Moved`. The undo-close path hides a pane
  before destroying it.
- Code-tab movement is not uniform. Reordering and most merges carry `TabData`,
  while `CodeView::remove_tab_for_move` currently reconstructs a new
  `CodePane` from its path. Waiting therefore cannot rely only on a view handle,
  pane ID, tab index, or path.
- `crates/warp_util/src/sync.rs::Condition` is a cloneable, set-once async
  condition. It records completion before notifying, supports multiple and late
  waiters, and does not require a per-waiter registry.

## Proposed changes

### 1. Extend the existing CLI and wire parameter

In `crates/warp_cli/src/local_control/mod.rs`:

- Add a Clap `--wait` boolean to `FileOpenArgs`.

In `crates/local_control/src/protocol.rs`:

- Add `wait: bool` to `FileOpenParams` with a Serde default.
- Omit `wait` when false so requests from the default command retain their
  current serialized shape.

In `crates/warp_cli/src/local_control/commands.rs`:

- Pass the parsed flag through the existing `ActionKind::FileOpen` request.
- Keep `run_action_with_params`, instance selection, target selection, output
  formatting, and the success payload unchanged.

Do not add an `ActionKind`, parameter-spec variant, result-spec variant,
endpoint, or protocol version. A new CLI using `--wait` against an older strict
server will receive `InvalidParams` rather than silently behaving as
nonblocking. Normal packaged usage invokes the channel-matched Warp binary, and
an old CLI request without the field remains accepted by the new server.

Update `resources/bundled/skills/warpctrl/SKILL.md` with a `file open --wait`
example and the rule that Ctrl+C cancels the caller without closing the view.

### 2. Add one private logical file-view lifecycle

Create `app/src/file_view_lifecycle.rs` as the single owner of the private wait
contract. It defines:

- `FileViewLifecycle`, a cloneable wrapper around
  `warp_util::sync::Condition`.
- `FileViewLifecycle::close()`, an idempotent transition to closed.
- `FileViewLifecycle::wait_until_closed()`, which completes immediately for a
  late waiter or asynchronously for an open view.
- `FileOpenReceipt`, which carries the lifecycle selected by an open/focus
  operation back to local control.

The lifecycle has no UUID, path lookup, serialization, save status, or public
protocol representation. Exact identity comes from returning a clone of the
lifecycle owned by the selected logical UI object.

Do not complete the lifecycle from `Drop`. Normal UI removal paths complete it
explicitly. This prevents app teardown from being misreported as a successful
user close; teardown instead drops the HTTP transport and produces the existing
nonzero client error.

### 3. Return the exact lifecycle from file open/focus

Change `WorkspaceView::open_file_with_target` to return an optional
`FileOpenReceipt`:

- `FileTarget::CodeEditor` and `FileTarget::MarkdownViewer` return the receipt
  for the exact newly-created or focused logical view.
- Targets without an in-Warp lifecycle return `None`. Existing non-control
  callers may ignore the result.
- `app_state::file_open` treats a missing receipt as an internal error because
  `resolve_file_target_to_open_in_warp` guarantees an in-Warp target for this
  action.

Make the same return value available from the internal helpers:

- `open_file_notebook` returns the existing `FileNotebookView` lifecycle when
  it focuses a matching pane, or the new pane's lifecycle after creation.
- `open_code` and `CodeView::open_or_focus_existing` return the selected
  `TabData` lifecycle. New `TabData` receives a new lifecycle when built.

Selection and lifecycle retrieval happen in the same main-thread update. There
is no open-then-path-search gap in which an immediate close could be missed or
an unrelated duplicate could become the target.

### 4. Preserve or complete code-editor tab lifecycles at existing ownership seams

Add a `FileViewLifecycle` field to `app/src/code/view.rs::TabData`.

- Tab reordering and ordinary `TabData` transfer/clone paths preserve the same
  lifecycle.
- Replace the close-oriented use of `remove_tab_data_index` in
  `remove_tab_for_move` with a move-specific extraction path. Reconstructing a
  `CodePane` for that move must pass the extracted lifecycle into the new
  `TabData` instead of allocating a replacement lifecycle.
- Removing a tab after an unchanged close, successful Save, or Discard calls
  `close()` at the same point the tab actually leaves the view. Cancel does not
  remove the tab and therefore does not complete the lifecycle.
- Closing a whole `CodePane` completes every contained tab lifecycle when the
  pane is detached as `HiddenForClose` or `Closed`. `DetachType::Moved` carries
  the existing pane and does not complete any lifecycle.
- When a move/merge discards a source `TabData` because the destination already
  contains the same path, explicitly complete the discarded source lifecycle.
  The destination is a different pre-existing logical view, matching Product
  Behavior #4.

The lifecycle remains attached to the logical file tab rather than
`CodeManager`, a pane ID, an index, or a path-keyed registry.

### 5. Preserve or complete Markdown viewer lifecycles at pane seams

Add a `FileViewLifecycle` to `FileNotebookView` and expose a clone to
`WorkspaceView::open_file_notebook` through `FilePane`.

- Focusing an already-open Markdown pane returns that pane's existing
  lifecycle.
- Creating a new `FilePane` returns its newly-created lifecycle.
- `FilePane::detach` completes the lifecycle for `HiddenForClose` and `Closed`
  and preserves it for `Moved`.
- Rendered/Raw changes within the same `FileNotebookView` keep the lifecycle.
  Replacing that viewer with a distinct code-editor view closes the original
  viewer lifecycle.

Existing file loading, error display, reload, and file-watching behavior is not
part of the lifecycle contract and remains unchanged.

### 6. Defer only the `--wait` HTTP response off the UI thread

Introduce an app-private bridge dispatch result in
`app/src/local_control/bridge.rs` with two states:

- `Ready(ResponseEnvelope)` for all existing actions and `file.open` without
  `wait`.
- A deferred file-close response containing the request ID, existing
  acknowledgement data, and selected `FileViewLifecycle` for `file.open` with
  `wait`.

`LocalControlBridge::handle_request` still performs validation, target
resolution, and UI mutation synchronously on the WarpUI model thread. It must
not await the user there. It returns the deferred value to
`handle_control_request`, whose Axum task awaits `wait_until_closed()` on the
local-control runtime and then creates the ordinary successful
`ResponseEnvelope`.

If the CLI is interrupted or disconnects, no close action is dispatched. The
server may cancel the abandoned handler immediately or finish its pending wait
when the view eventually closes; either way, it owns only a lifecycle clone and
does not register mutable per-client state on the view. If Warp or the
local-control runtime exits first, the connection drops and
`crates/local_control/src/client.rs` maps the failure to the existing nonzero
transport error.

## Testing and validation

### Protocol and CLI tests

- `crates/warp_cli/src/local_control_tests.rs`
  - `file open PATH --wait` parses `wait = true` and still maps to
    `ActionKind::FileOpen`.
  - Omitting the flag produces `wait = false`.
  - `--wait` composes with line, column, new-tab, and target selectors.
- `crates/local_control/src/protocol_tests.rs`
  - A missing `wait` field decodes as false.
  - False is omitted from serialization; true round-trips.
  - Unknown fields remain rejected and the catalog still contains only the
    existing `file.open` action.

### Lifecycle and UI ownership tests

- Unit tests for `FileViewLifecycle` verify multiple waiters, a late waiter,
  idempotent close, and dropping one waiter without closing the lifecycle.
- `app/src/workspace/view_tests.rs`
  - Extend `test_open_file_notebook_focuses_existing_markdown_pane` to assert
    that reopening returns the original lifecycle.
  - Add equivalent code-editor tests for new and already-open tabs and for an
    unrelated duplicate path not satisfying the receipt.
- Add `app/src/code/view_tests.rs` (using the repository's separate test-module
  convention) to verify unchanged close, Save, Discard, canceled close,
  reorder, single-tab move, whole-pane move, and same-path merge/deduplication.
- `app/src/notebooks/file/mod_tests.rs` and
  `app/src/pane_group/mod_tests.rs` verify `Moved` remains pending while
  `HiddenForClose` and `Closed` complete, including containing tab/window close.

### Local-control tests

- `app/src/local_control/mod_tests.rs`
  - A non-waiting request returns the existing acknowledgement immediately.
  - A waiting request remains pending while the selected lifecycle is open and
    returns the same acknowledgement after close.
  - Two requests can wait on one lifecycle.
  - Dropping a waiting request leaves the lifecycle open.
  - Stopping the bridge/server before close produces an error, not a successful
    acknowledgement.
- `crates/local_control/src/client_tests.rs`
  - The blocking client accepts a delayed valid response.
  - Premature server/transport disappearance maps to `TransportUnavailable`.

### Manual validation

1. Start a development Warp build with Warp Control enabled.
2. Run `warpctrl file open <existing-temp-file> --wait`; confirm the CLI stays
   pending, edits can be saved, and it exits successfully only after the exact
   view closes.
3. Repeat for a Markdown viewer pane and a code-editor tab, with default split
   layout and `--new-tab`.
4. Reorder and move the target; confirm the CLI remains pending. Close an
   unrelated duplicate path and confirm it remains pending.
5. Exercise Save, Discard, and Cancel in the unsaved-changes dialog.
6. Start two waiters for the same existing view and confirm both exit on close.
7. Press Ctrl+C while waiting and confirm the view remains open and editable.
8. Quit Warp while waiting and confirm the CLI exits nonzero.

Before pushing an implementation update, run the focused tests above followed
by the repository-required `./script/format` and `cargo clippy` commands.

## Risks and mitigations

- **False completion during a move.** Some code-tab moves rebuild a pane from a
  path. A move-specific extraction path must carry the lifecycle and must not
  call the close-oriented removal helper.
- **Waiting on the wrong duplicate.** Paths, pane IDs, and tab indices are not
  sufficient identities. Returning the lifecycle directly from the atomic
  open/focus operation avoids a later lookup.
- **Lost close notification.** `Condition` records its set state and checks it
  before and after listener registration, so a close that races with the Axum
  task beginning its wait cannot hang.
- **Blocking the UI thread.** Only the Axum request task awaits the lifecycle;
  the WarpUI bridge returns the deferred result immediately after dispatch.
- **App exit reported as a normal close.** The lifecycle is completed only by
  explicit view-removal paths, not `Drop`; server shutdown therefore ends the
  transport instead of synthesizing success.
- **Protocol compatibility.** False is omitted and remains compatible with the
  previous request shape. An older strict server rejects a true `wait` field
  explicitly rather than silently ignoring it.

## Follow-ups

None required. Public file-view handles, a separate wait action, explicit mode
or layout flags, and broader Warp automation should be proposed independently
if concrete use cases require them.
