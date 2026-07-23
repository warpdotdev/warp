# Reliable Ctrl+Tab switching on rapid release — technical specification

GitHub issue: [#11221](https://github.com/warpdotdev/warp/issues/11221)

## Context

This plan implements the user-visible invariants in [PRODUCT.md](./PRODUCT.md) against Warp commit [`abea51cd1e102b363935f1b25ef03d335bc7b36f`](https://github.com/warpdotdev/warp/commit/abea51cd1e102b363935f1b25ef03d335bc7b36f).

The current flow is:

1. [`Workspace::cycle_session`](https://github.com/warpdotdev/warp/blob/abea51cd1e102b363935f1b25ef03d335bc7b36f/app/src/workspace/view.rs#L11722-L11768) handles both MRU settings. On the first cycle action it marks the Ctrl+Tab palette open, configures its data source and initial selection, and requests a redraw.
2. [`Workspace::open_ctrl_tab_palette`](https://github.com/warpdotdev/warp/blob/abea51cd1e102b363935f1b25ef03d335bc7b36f/app/src/workspace/view.rs#L14309-L14365) prepares the sessions or tabs query and its forward/reverse offset. The tab and session sources used here are synchronous today.
3. [`Workspace::open_palette`](https://github.com/warpdotdev/warp/blob/abea51cd1e102b363935f1b25ef03d335bc7b36f/app/src/workspace/view.rs#L14499-L14552) correctly chooses the Ctrl+Tab handle for setup, but then unconditionally focuses the regular `palette` handle.
4. The Ctrl+Tab view is inserted only when [`is_ctrl_tab_palette_open` is rendered](https://github.com/warpdotdev/warp/blob/abea51cd1e102b363935f1b25ef03d335bc7b36f/app/src/workspace/view.rs#L27045-L27051). Its [`on_modifier_state_changed` listener](https://github.com/warpdotdev/warp/blob/abea51cd1e102b363935f1b25ef03d335bc7b36f/app/src/search/command_palette/view.rs#L188-L218) converts a Control release into `Action::CtrlPressed(false)`; the action then accepts the selected result.

This creates a first-frame gap: if Control is released after the cycle action changes workspace state but before a frame containing the Ctrl+Tab child is painted, the release is dispatched through the old element tree and the palette listener cannot observe it. WarpUI intentionally sends events to children before parents, and only painted children participate: see [`EventHandler::dispatch_event`](https://github.com/warpdotdev/warp/blob/abea51cd1e102b363935f1b25ef03d335bc7b36f/crates/warpui_core/src/elements/gui/event_handler.rs#L241-L260) and [`Stack::dispatch_event`](https://github.com/warpdotdev/warp/blob/abea51cd1e102b363935f1b25ef03d335bc7b36f/crates/warpui_core/src/elements/gui/stack/mod.rs#L288-L315).

The closed implementation PR [#11222](https://github.com/warpdotdev/warp/pull/11222) identified the same focus and event-order issues. It received no inline technical review; it was blocked because the issue was not then labeled `ready-to-implement` and later closed as stale. Its approach is useful prior art, not an approved implementation.

## Proposed changes

### 1. Focus the palette that was opened

In `Workspace::open_palette`, retain the existing `active_palette` selection and focus that handle after mode-specific setup. Normal palette sources continue focusing `self.palette`; `PaletteSource::CtrlTab` focuses `self.ctrl_tab_palette`.

This fixes the incorrect responder target after the Ctrl+Tab palette reaches the scene, but it does not by itself close the pre-render event gap.

### 2. Add a workspace-level fallback for the pre-render Control release

Add a narrowly named `WorkspaceAction` for committing an active Ctrl+Tab interaction after a release that reaches the workspace root. Classify it as not requiring app-state persistence in `WorkspaceAction::should_save_app_state_on_action`.

Attach a modifier-state listener to the existing workspace-root `EventHandler` in `Workspace::render`:

- Dispatch the fallback action only for `ControlLeft` or `ControlRight` with `KeyState::Released`.
- Return `PropagateToParent`; this listener is a fallback, not a general modifier-event sink.
- Do not react to Shift, Alt, Super, Fn, or Control events whose effective state is still `Pressed`.

The existing child-first propagation provides the deduplication rule. Once the Ctrl+Tab palette is painted, its listener accepts the result and returns `StopPropagation`, so the workspace fallback does not run. Before the child is painted, the event reaches the workspace listener and is recovered there.

### 3. Forward the fallback commit without re-entering Workspace

In the new workspace action handler:

- No-op unless `current_workspace_state.is_ctrl_tab_palette_open` is still true. This prevents a late release from acting after Escape, a direct row selection, or another overlay transition has closed the interaction.
- Forward `command_palette::Action::CtrlPressed(false)` to `ctrl_tab_palette` as a deferred typed action.

Deferral is required because accepting a session or tab can dispatch through `RootView` and update the same `Workspace` that is currently handling the fallback. WarpUI removes a view from storage while it is being updated and panics on re-entry as documented by [`AppContext::update_view`](https://github.com/warpdotdev/warp/blob/abea51cd1e102b363935f1b25ef03d335bc7b36f/crates/warpui_core/src/core/app.rs#L4619-L4648). [`dispatch_typed_action_deferred`](https://github.com/warpdotdev/warp/blob/abea51cd1e102b363935f1b25ef03d335bc7b36f/crates/warpui_core/src/core/view/context.rs#L413-L423) queues the palette action until the current update is complete.

Keep selection and close behavior in the command-palette code path rather than duplicating navigation in Workspace. [`handle_result_accepted`](https://github.com/warpdotdev/warp/blob/abea51cd1e102b363935f1b25ef03d335bc7b36f/app/src/search/command_palette/view.rs#L715-L1002) already owns session/tab activation, telemetry, reset, and close emission.

For PRODUCT invariant 11, make the CtrlTab release path explicitly close when no selected result exists. Do not change Enter or result-click behavior in normal palette mode.

### 4. Keep scope local

No new setting, keybinding, feature flag, telemetry event, persistence field, or UI element is needed. The implementation should remain confined to the workspace action/view, the Ctrl+Tab release path, and focused tests.

## Testing and validation

### Automated regression coverage

Extend the existing GUI integration coverage at [`test_ctrl_tab_session_switching`](https://github.com/warpdotdev/warp/blob/abea51cd1e102b363935f1b25ef03d335bc7b36f/crates/integration/src/test.rs#L2130-L2249):

- Preserve the existing slow-release assertions for forward, reverse, and wraparound behavior (PRODUCT 2–4, 13).
- Add a same-frame regression step that dispatches `CustomAction::CycleNextSession` and returns `Event::ModifierKeyChanged { ControlLeft, Released }` from one `with_event_fn` callback, before the next rendered frame can mount the palette. Assert that the expected MRU session is focused and `Workspace::is_palette_open()` is false (PRODUCT 5–7).
- Repeat the same-frame case with `CtrlTabBehavior::CycleMostRecentTab` and assert tab activation rather than pane/session activation (PRODUCT 1, 5, 14).
- Create two windows, dispatch the same-frame cycle-and-release sequence only to one window, and assert that it commits and closes there without changing the other window's active destination or Ctrl+Tab interaction state (PRODUCT 14).
- Hold both physical Control keys, release one while the other remains pressed, and assert that no destination is committed and the switcher stays open; then release the remaining Control key and assert exactly one commit and a closed switcher (PRODUCT 8).
- Cover reverse direction with `CyclePrevSession` and Control release in the same frame (PRODUCT 3–5).
- Close the switcher before emitting a Control release and assert no navigation occurs (PRODUCT 10, 12).
- Exercise the one-destination case and assert the switcher closes without changing the active destination (PRODUCT 11).

If the integration harness cannot express the pre-render ordering without producing an intervening frame, add a focused WarpUI test fixture whose painted tree initially omits the Ctrl+Tab child, dispatches the cycle state change and release, and then paints. Do not replace the real integration assertion with sleeps; the test must deterministically encode event order.

### Existing suites and repository gates

- Run the focused Ctrl+Tab GUI integration test on macOS.
- Run `./script/format` and the Clippy command selected by `./script/presubmit` before publishing any implementation update.
- Run the relevant app/unit tests plus `cargo nextest run --no-fail-fast --workspace --exclude command-signatures-v2` and `cargo test --doc`, subject to the repository's documented external-fork exclusions.

### Manual and visual validation

On a local macOS Warp build with at least three tabs and multiple sessions:

1. Record the current bug on the base revision with **Cycle most recent session** and a rapid Ctrl+Tab tap.
2. On the implementation branch, perform at least 20 rapid forward taps and 20 rapid reverse taps; every interaction must commit and leave no switcher visible (PRODUCT 3–7).
3. Repeat with **Cycle most recent tab**, including tabs with multiple panes (PRODUCT 14).
4. Verify a slow hold, repeated Tab presses, Escape cancellation, direct row selection, and the **Activate previous/next tab** setting (PRODUCT 1, 4, 10, 12, 13).
5. Attach a short after-fix recording to the implementation PR. No new static UI screenshot is required because visual appearance is unchanged.

## Parallelization

Parallel implementation is not recommended. The action enum, workspace event fallback, palette commit behavior, and deterministic regression test describe one tightly coupled event sequence, and splitting them across worktrees would increase merge and diagnosis cost. Use one branch and worktree (suggested: `rasitakyol/gh11221-ctrl-tab-fix` in `warp-wt-11221-impl`), implement code first, then run automated and manual validation sequentially. A separate reviewer may inspect the event-order assumptions after the focused test is passing, but should not author overlapping files in parallel.

## Risks and mitigations

- **Double commit after the palette is visible:** rely on child-first dispatch and the palette's existing `StopPropagation`; retain the open-state guard in the fallback action.
- **Circular view update during navigation:** dispatch the palette release action only through the deferred typed-action queue.
- **Incorrect behavior with two physical Control keys:** act only on a `Released` state supplied by WarpUI's effective modifier-state conversion, not on every raw Control key transition.
- **Closing without a result:** explicitly close without navigation so an empty or transiently unavailable result set cannot recreate the stuck palette.
- **Regression in normal command palette focus:** select the focus target from `PaletteSource`; do not alter normal palette mode or its listeners.
- **Timing test that passes accidentally:** encode cycle and release before a paint boundary in one harness callback or a purpose-built event-tree fixture; do not use an arbitrary short delay as a proxy for the race.
