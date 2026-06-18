# REMOTE-926: Reopening a closed cloud-mode tab should rejoin the session — Tech Spec

## 1. Problem
`TerminalPane::detach` unconditionally invokes `TerminalManager::on_view_closed` on every detach, including the "reversible" `DetachType::HiddenForClose` case that is used when a tab is closed but remains on the `UndoCloseStack`. For the shared-session viewer `TerminalManager`, `on_view_closed` is destructive: it closes the WebSocket with `close_without_reconnection`, flips `SharedSessionStatus` to `FinishedViewer`, and calls `TerminalView::on_session_share_ended` (which drops `shared_session`, inserts the ended banner, unregisters peers, and makes input read-only). None of this is reversed by `restore_closed_tab` / `PaneGroup::reattach_panes`, so restoring a cloud-mode tab yields a frozen, read-only snapshot instead of a live viewer.

The goal of this spec is to make `HiddenForClose` genuinely non-destructive for the shared-session viewer, so that `attach` on restore needs to do no extra work to bring the view back to a live state. `Closed` remains fully destructive.

## 2. Relevant code
- `app/src/pane_group/pane/mod.rs (516-526)` — `DetachType` enum (`Closed` / `HiddenForClose` / `Moved`).
- `app/src/pane_group/pane/mod.rs:594` — `PaneContent::detach` trait signature.
- `app/src/pane_group/pane/terminal_pane.rs (311-364)` — `TerminalPane::detach`; gates block deletion / conversation clearing on `DetachType::Closed` but calls `on_view_closed` unconditionally (line 333).
- `app/src/pane_group/pane/terminal_pane.rs (208-309)` — `TerminalPane::attach`; re-wires focus + event subscriptions + re-registers with `ActiveAgentViewsModel`.
- `app/src/pane_group/mod.rs (6302-6351)` — `clean_up_panes` (uses `Closed`), `detach_panes` / `detach_panes_for_close` (uses `HiddenForClose`), `reattach_panes`, `attach_pane`.
- `app/src/terminal/mod.rs` / `TerminalManager` trait — `fn on_view_closed(&self, app: &mut AppContext)`. Implemented in:
  - `app/src/terminal/shared_session/viewer/terminal_manager.rs (1414-1441)` — destructive viewer teardown we need to split.
  - `app/src/terminal/local_tty/terminal_manager.rs:2076` — local-tty `on_view_closed` (effectively a no-op for network state).
  - `app/src/terminal/remote_tty/terminal_manager.rs` — remote-tty `on_view_closed`.
  - `app/src/terminal/MockTerminalManager` — used by conversation transcript viewers and tests.
- `app/src/terminal/view/shared_session/view_impl.rs (683-735)` — `TerminalView::on_session_share_ended`, the UI-side teardown routine.
- `app/src/terminal/shared_session/viewer/network.rs (764-777)` — `Network::close` / `close_without_reconnection`.
- `app/src/workspace/view.rs (10104-10120)` — `Workspace::restore_closed_tab`.
- `app/src/workspace/view.rs (9716-9785)` — `Workspace::remove_tab`; calls `detach_panes_for_close` (line 9755).
- `app/src/undo_close/stack.rs (354-388)` — `UndoCloseStack::push_item`; spawns the grace-period expiry task.
- `app/src/undo_close/stack.rs (70-150)` — `ClosedItem::discard` / `clean_up_pane_group`; runs on grace-period expiry and invokes `clean_up_panes` → `DetachType::Closed`.

## 3. Current state
On `Cmd-W` for a cloud-mode tab:

1. `Workspace::remove_tab` → `PaneGroup::detach_panes_for_close` → `detach_panes` → `pane.detach(.., HiddenForClose, ..)`.
2. `TerminalPane::detach`:
   - Skips block deletion / conversation clearing (gated on `DetachType::Closed`).
   - **Still** calls `terminal_manager.on_view_closed(ctx)` for every view in the pane stack.
   - Unregisters from `ActiveAgentViewsModel`, `CLIAgentSessionsModel`, and unsubscribes from pane-stack / view / Manager / history events.
3. Shared-session viewer `TerminalManager::on_view_closed`:
   - `network.close_without_reconnection()` (WS is gone; no reconnect will ever be tried).
   - `model.set_shared_session_status(FinishedViewer)`.
   - `view.on_session_share_ended(ctx)` — drops `shared_session`, inserts ended banner, unregisters remote peers, flips input to `Selectable` (if viewer role).
4. `TabData` is placed on the `UndoCloseStack` with a 60s expiry task (`push_item` at stack.rs:354).

On `Cmd-Shift-T`:
5. `UndoCloseStack::undo_close` → `Workspace::restore_closed_tab` → `PaneGroup::reattach_panes` → `TerminalPane::attach` for each pane.
6. `TerminalPane::attach` just re-wires focus handles + event subscriptions + re-registers with `ActiveAgentViewsModel`. It does nothing to undo steps 3a-3c.

Net: the restored tab shows the frozen close-time snapshot with an ended banner, while the cloud agent continues to run server-side.

The grace-period expiry path is already correctly wired up for full teardown: `ClosedItem::discard` for `ClosedItem::Tab` → `clean_up_pane_group` → `pane_group.clean_up_panes(ctx)` → `pane.detach(.., Closed, ..)`. That path calls `on_view_closed` a second time on the same managers (harmless-but-redundant today; in the new design this becomes the *actual* destructive call).

## 4. Proposed changes

The core change is to thread `DetachType` into the `TerminalManager` detach lifecycle so shared-session viewers can distinguish "suspended, will likely come back" from "permanently closed".

### 4.1 Widen `TerminalManager::on_view_closed` to take a `DetachType`
Rename or extend the trait method. Preferred shape:

```rust path=null start=null
pub trait TerminalManager {
    /// Called when the owning pane is detaching. `detach_type` indicates whether
    /// the detach is reversible (`HiddenForClose`), a permanent close (`Closed`),
    /// or a move (`Moved`).
    fn on_view_detached(&self, detach_type: DetachType, app: &mut AppContext);
    // ... existing methods unchanged
}
```

Rationale for rename: today's `on_view_closed` is a misleading name given we want to call it for non-close detaches too. The new name also opens the door to `Moved` behavior if we ever need to customize it. `DetachType` already lives in `app/src/pane_group/pane/mod.rs` and is appropriate to expose to the terminal layer.

### 4.2 Update `TerminalPane::detach` to pass the detach type
In `app/src/pane_group/pane/terminal_pane.rs`:

```rust path=null start=null
for (manager, view) in contents {
    manager.update(ctx, |terminal_manager, ctx| {
        terminal_manager.on_view_detached(detach_type, ctx);
    });
    ctx.unsubscribe_to_view(&view);
}
```

`ActiveAgentViewsModel::unregister_agent_view_controller` is called unconditionally (including on `HiddenForClose`), so the conversation moves from Active → Past in the conversation list during the grace period. This is intentional: showing an "Active" conversation whose pane is not in any tab group would be misleading — clicking it would not navigate anywhere. On restore, `TerminalPane::attach` calls `register_agent_view_controller` (line 301-308), moving the conversation back to Active. `CLIAgentSessionsModel::remove_session` is also called unconditionally except for `Moved` (where the session is still running in the relocated pane).

### 4.3 Shared-session viewer `TerminalManager` — branch on `DetachType`
In `app/src/terminal/shared_session/viewer/terminal_manager.rs`:

```rust path=null start=null
fn on_view_detached(&self, detach_type: DetachType, app: &mut AppContext) {
    // Keep network alive for non-permanent detaches:
    // - HiddenForClose: may be restored from the undo-close stack.
    // - Moved: the same TerminalManager is reused in the target pane group
    //   (see §4.6 for the code-path analysis).
    if !matches!(detach_type, DetachType::Closed) {
        return;
    }
    // Destructive teardown — only reached on DetachType::Closed.
    let terminal_view_id = self.view.id();
    ActiveAgentViewsModel::handle(app).update(app, |model, ctx| {
        model.unregister_agent_view_controller(terminal_view_id, ctx);
        model.unregister_ambient_session(terminal_view_id, ctx);
    });

    if let NetworkState::Active(ref network) = self.network_state {
        network.update(app, |network, _| {
            network.close_without_reconnection();
        });
    }
    self.model
        .lock()
        .set_shared_session_status(SharedSessionStatus::FinishedViewer);
    self.view
        .update(app, |view, ctx| view.on_session_share_ended(ctx));
}
```

### 4.4 Local-tty / remote-tty / Mock managers — behavior unchanged
For `local_tty::TerminalManager`, `remote_tty::TerminalManager`, and `MockTerminalManager`, forward any value of `DetachType` to today's logic. Those `on_view_closed` bodies are already effectively no-ops for network concerns and don't need to differentiate between detach types. A one-line migration: rename `on_view_closed` → `on_view_detached(_: DetachType, ..)` and ignore the parameter.

### 4.5 `TerminalPane::attach` — re-registration on restore
`attach` re-registers with `ActiveAgentViewsModel` (terminal_pane.rs:301-308). Because `HiddenForClose` unconditionally unregisters the view (§4.2), `attach` on restore performs a fresh registration — the conversation moves back from Past to Active in the conversation list at the moment the tab is restored.

No changes are required in `Workspace::restore_closed_tab` or `PaneGroup::reattach_panes` beyond what `attach` already does — because the shared-session viewer was never torn down, the view picks up where it left off.

### 4.6 Handle `DetachType::Moved` — TerminalManager is reused, not replaced
The move-pane path calls `PaneGroup::remove_pane_for_move`, which calls `pane.detach(.., Moved, ..)` and then returns the same `Box<dyn AnyPaneContent>`. That box is immediately inserted into the target pane group via `init_pane` → `try_attach_pane` → `attach_pane` → `pane.attach(..)`. The same `TerminalPane` (and therefore the same `TerminalManager`) is re-attached; nothing is replaced. Consequently, `on_view_detached(Moved, ..)` should be a no-op for the viewer, just like `HiddenForClose`. The implementation uses `!matches!(detach_type, DetachType::Closed)` so both cases are preserved.

### 4.7 Sanity: do not double-teardown on expiry
Under the new design, when the grace period expires, the discard path calls `clean_up_panes` → `pane.detach(.., Closed, ..)` → `on_view_detached(Closed, ..)`. This is now the *first* destructive call for the shared-session viewer. No redundant work. No behavior changes for local panes.

## 5. End-to-end flow
Close → restore (happy path):
1. User hits `Cmd-W` on a cloud-mode tab.
2. `Workspace::remove_tab` → `detach_panes_for_close` → `detach_panes` → `TerminalPane::detach(.., HiddenForClose, ..)`.
3. `TerminalPane::detach` calls `ActiveAgentViewsModel::unregister_agent_view_controller` and `CLIAgentSessionsModel::remove_session` unconditionally (conversation moves Active→Past); unsubscribes from view events; invokes `TerminalManager::on_view_detached(HiddenForClose, ..)`.
4. Shared-session viewer's `on_view_detached(HiddenForClose, ..)` is a no-op. Network stays `Active`; `SharedSessionStatus` unchanged; `TerminalView::shared_session` still `Some(...)`.
5. `TabData` is pushed onto `UndoCloseStack` with a 60s expiry timer.
6. While the tab is on the stack, `NetworkEvent`s from the server continue to be dispatched into the (now-detached) view, updating the underlying `TerminalModel` / `BlocklistAIHistoryModel`. Visual rendering is skipped because the view is not in any active pane group layout.
7. User hits `Cmd-Shift-T`. `UndoCloseStack::undo_close` pops the item (drop of `ExpiryData` aborts the expiry timer). `Workspace::restore_closed_tab` reinserts the `TabData` and calls `pane_group.reattach_panes(ctx)`.
8. `TerminalPane::attach` re-subscribes to view events, re-registers with `ActiveAgentViewsModel` (conversation moves Past→Active). The view is now rendered again, backed by live model state.

Close → expire (no restore):
1-6 as above.
7'. 60s later, the `spawn_abortable` timer fires in `UndoCloseStack::push_item`'s closure. It removes the item from the stack and calls `ClosedItem::discard`.
8'. `discard` marks conversations historical, then `clean_up_pane_group` → `pane_group.clean_up_panes(ctx)` → `TerminalPane::detach(.., Closed, ..)` → `on_view_detached(Closed, ..)` → full teardown. Matches today's final state.

## 6. Diagrams

```mermaid
sequenceDiagram
    participant User
    participant Workspace
    participant PaneGroup
    participant TerminalPane
    participant TM as SharedSessionViewer<br/>TerminalManager
    participant Net as Network (WS)
    participant Stack as UndoCloseStack

    User->>Workspace: Cmd-W
    Workspace->>PaneGroup: detach_panes_for_close
    PaneGroup->>TerminalPane: detach(HiddenForClose)
    TerminalPane->>TM: on_view_detached(HiddenForClose)
    Note over TM,Net: no-op — network stays Active
    Workspace->>Stack: push_item(TabData, 60s expiry)

    Note over User,Net: Cloud agent keeps emitting events over the live WS

    alt User hits Cmd-Shift-T within 60s
        User->>Stack: undo_close
        Stack->>Workspace: restore_closed_tab
        Workspace->>PaneGroup: reattach_panes
        PaneGroup->>TerminalPane: attach
        Note over TerminalPane,Net: view re-renders;<br/>network was never torn down
    else 60s elapse with no restore
        Stack->>Stack: expiry timer fires
        Stack->>PaneGroup: clean_up_panes
        PaneGroup->>TerminalPane: detach(Closed)
        TerminalPane->>TM: on_view_detached(Closed)
        TM->>Net: close_without_reconnection
        TM->>TM: set SharedSessionStatus::FinishedViewer
        TM->>TM: view.on_session_share_ended
    end
```

## 7. Risks and mitigations
- **Ghost viewer in participant list during grace period.** The sharer and any co-viewers see this user in the participant list for up to 60s after close. Mitigation: accept for v1 (non-goal §4 in PRODUCT.md). Follow-up: protocol-level "suspended" state.
- **Leaked network if expiry task fails to fire.** The grace-period `Timer::after` is driven by the app's async runtime; if the process doesn't crash, expiry always fires. If the process *does* crash, the network dies with it. No new leak risk beyond what exists today for any spawned task.
- **Conversation flickers Active→Past→Active during restore.** When the tab is closed, `unregister_agent_view_controller` fires immediately (conversation moves to Past). On restore, `attach` re-registers (conversation moves back to Active). This brief flicker is acceptable and preferable to showing an "Active" conversation that cannot be navigated to while the pane is off-screen.
- **In-flight `NetworkEvent`s reaching a detached view.** During the grace period, `weak_view_handle.upgrade(ctx)` still succeeds (the view is still alive, just not laid out), so event handlers will keep running. This is desirable — it keeps the model in sync — but we should verify there are no handlers that assume the view is currently visible (e.g. toast-show paths). Mitigation: audit `handle_network_events` (terminal_manager.rs:500-1170) for visibility assumptions; in particular `show_persistent_toast` calls should still be safe. Add an integration test that exercises a `SessionEnded` server message arriving while the tab is hidden.
- **Terminal transcript viewer / conversation viewer tabs.** These use `MockTerminalManager` and do not hold a live network. The rename to `on_view_detached` is a mechanical no-op for them.
- **Move pane path.** `DetachType::Moved` is now a no-op for the shared-session viewer because the `TerminalManager` is reused (not replaced) when a pane is dragged to another tab. Tests around moving a cloud-mode pane across tabs should verify the session stays live after the move.
- **Preview / dogfood rollout.** The change is not feature-flagged; it modifies fundamental detach/attach semantics. Risk is moderate because non-cloud panes should see no behavior change. Mitigation: thorough test coverage in §8 and manual QA of local-terminal close/restore, move-pane, and close-all-tabs-except flows.

## 8. Testing and validation
- **Unit tests**
  - `viewer::TerminalManager::on_view_detached(HiddenForClose)` leaves `network_state` as `Active`, `SharedSessionStatus` unchanged, and does not insert the ended banner on the view.
  - `viewer::TerminalManager::on_view_detached(Closed)` performs today's full teardown.
  - `viewer::TerminalManager::on_view_detached(Moved)` is a no-op (network stays `Active`, `SharedSessionStatus` unchanged) because `TerminalManager` is reused across the move.
  - `local_tty::TerminalManager::on_view_detached` behaves identically for all `DetachType` values (regression).
- **Integration tests** (using the `crates/integration` framework; see skill `warp-integration-test`)
  - Close a shared-session viewer tab, emit a new `DownstreamMessage::OrderedTerminalEvent` from a mock server, restore the tab, assert the new block is present.
  - Close a shared-session viewer tab with editor role, restore the tab, assert input is still `Editable`.
  - Close a shared-session viewer tab, assert no ended banner was inserted, restore, assert no ended banner.
  - Close a shared-session viewer tab, let the grace period expire (use a short override), assert conversation is marked historical, assert `TerminalView` is dropped, assert `SharedSessionStatus::FinishedViewer`.
  - Close a local-terminal tab and restore it — existing tests continue to pass.
  - Close cloud tab, then close another cloud tab, then `Cmd-Shift-T` twice — both restore live.
- **Presubmit**
  - `./script/presubmit` (cargo fmt + clippy + tests). Note the `WARP.md` rule about running this before PR.
- **Manual**
  - Run `cargo run --features with_local_server` against a local `warp-server`, spawn a cloud agent conversation, close the tab mid-agent-response, `Cmd-Shift-T`, verify the view keeps streaming.
  - Verify the conversation list moves the conversation to "Past" immediately on close, then back to "Active" on restore; and that it stays "Past" → finalizes to historical after grace-period expiry.

## 9. Follow-ups
- Send a "viewer suspended" hint to the session-sharing server so the sharer's participant UI can dim / hide the ghost viewer during the grace period. Requires a protocol change (`UpstreamMessage::Suspend` or similar) and server-side handling.
- Once the `DetachType`-aware trait method is in place, consider extending the same pattern to non-terminal panes (code, notebook, workflow) if any of them grow network state that would benefit from suspension.
- Reconsider the 60s default `UndoCloseGracePeriod` specifically for cloud-mode tabs — may want a tighter bound to reduce the ghost-viewer window without penalizing the common local-tab case.
- Long term: reconcile the shared-session viewer detach/attach lifecycle with the deferred-join flow (`new_deferred` + `connect_to_session` in `viewer/terminal_manager.rs (268-292)`) so that a truly re-joinable viewer can be expressed as a first-class `NetworkState::Suspended` variant, making the "HiddenForClose no-op" case an explicit state transition rather than an absence of action.
