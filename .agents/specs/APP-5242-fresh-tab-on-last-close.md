# APP-5242: Open a fresh tab when the final tab closes

## Summary
Add a global, persisted setting that controls what Warp does when the final tab in a window is closed:

- **On** (default): preserve today's behavior and close the window.
- **Off**: keep the window and its chrome open. A close-tab action on a pristine final tab is a silent no-op; closing a non-pristine final tab creates a fresh tab using normal New Tab semantics.

The setting appears in **Settings → Features → General**, immediately below **Quit when all windows are closed**, with the label **Close window when all tabs are closed**. Supporting text reads: **In windows Warp cannot close, closing the last tab opens a new tab instead.**

This is a focused implementation spec for the Dogfood prototype requested in APP-5242. It is independent of APP-5230; do not modify, merge, or reuse APP-5230's branch or PR.

## Current behavior and relevant code
All close entry points converge on `Workspace::close_tab` → `close_tabs` → `remove_tab` in `app/src/workspace/view.rs`. `remove_tab` short-circuits when one tab remains: it closes the window when `ContextFlag::CloseWindow` is enabled and otherwise leaves the tab untouched.

The final-tab affordances are currently inconsistent in a `CloseWindow`-disabled host:

- The horizontal close button and middle-click behavior in `app/src/tab.rs` are gated by `ContextFlag::CloseWindow`.
- The **Close tab** context-menu item in `app/src/tab.rs` is omitted for a single tab.
- The `workspace:close_active_tab` binding in `app/src/workspace/mod.rs` requires either `Workspace_CloseWindow` or `Workspace_MultipleTabs`.
- The vertical-tabs close button is already rendered, while its middle-click path is gated.

Normal tab creation already centralizes default-session-mode, shell/profile, working-directory, and placement behavior in the workspace's New Tab paths. Empty workspaces in hosts that cannot create a terminal session already seed valid host-specific content through `configure_empty_workspace`.

Undo Close stores the removed `TabData` in `UndoCloseStack` and restores it into the originating workspace. Session restore snapshots visible tabs; runtime undo state is not persisted.

For cross-window drag, a one-tab source window currently acts as its own floating preview. A committed handoff transfers its only tab to the target and closes the source window. That path needs an explicit keep-source variant when the new setting is off.

## Behavior contract

### Setting
- Add a boolean general setting with:
  - label: **Close window when all tabs are closed**
  - default: `true`
  - platform support: all platforms
  - persistence in the user settings file
  - global cloud sync that respects the user's sync preference
  - a Command Palette enable/disable entry and matching setting context flag, as required for toggleable settings
- Render it directly below **Quit when all windows are closed** where that setting is present. On platforms where the macOS-only quit setting is absent, retain the same relative position in the General widget list.
- Render the settled supporting text: **In windows Warp cannot close, closing the last tab opens a new tab instead.**
- Changing the setting affects subsequent closes immediately. It does not alter any already-open tab.

### Effective final-tab behavior
Compute one effective keep-window decision at the workspace boundary:

- If `ContextFlag::CloseWindow` is enabled, follow the setting.
- If `ContextFlag::CloseWindow` is disabled, always keep the window, regardless of the setting value.

Then apply one of three outcomes:

1. **Close window:** in a window-capable host with the setting on, preserve today's final-tab behavior. This applies even when the final tab is pristine.
2. **No-op:** in effective keep-window behavior, a close-tab action on a pristine final tab does nothing. The tab and window remain unchanged, no replacement or undo item is created, no confirmation appears, and no toast, animation, or disabled/hidden state is added.
3. **Replace:** in effective keep-window behavior, closing a non-pristine final tab creates a fresh replacement and removes the old tab.

The host divergence is intentional: a `CloseWindow`-disabled host uses keep-window behavior for both setting values. Therefore it no-ops on a pristine final tab and replaces a non-pristine final tab. The Settings supporting text remains the settled copy and summarizes the non-pristine fallback.

When replacement is required:

1. Complete any applicable close confirmation before creating or removing anything. Cancel leaves the original tab and window unchanged.
2. Create the replacement while the closing tab is still available, then remove the closing tab. The workspace must never expose or persist a zero-tab intermediate state.
3. Use the same internal operation as an ordinary **New Tab**, including the configured default session mode/profile or launch configuration, new-tab placement, and working-directory rules.
4. Do not clone the closing tab's profile, cwd, command, environment, title, color, pin state, or pane layout except where ordinary New Tab behavior independently inherits a value.
5. In a host that cannot create a terminal session, use the same host-specific seed selected by `configure_empty_workspace` rather than attempting to launch a shell. This produces a valid fresh home/link-capable tab while retaining workspace chrome.
6. Save the replacement like any normal tab. It has no special session-restore type or filtering.

The decision applies whenever a close operation would remove the final remaining tab, including direct final-tab close and multi-tab/group close operations that eventually reach one tab. Closing a non-final tab is unchanged.

The no-op applies uniformly to the close button, middle-click, **Close tab** context-menu item, and `workspace:close_active_tab`. These affordances remain visible and enabled; workspace logic owns the silent no-op. Repeated Cmd/Ctrl+W on a pristine replacement must not allocate sessions, add undo entries, emit close telemetry, or otherwise accumulate work.

### Close affordances in `CloseWindow`-disabled hosts
For a single-tab workspace, all of the following must be visible/enabled and dispatch the same final-tab close operation:

- horizontal and vertical close button
- middle-click
- **Close tab** context-menu item
- `workspace:close_active_tab`

The affordances remain enabled for both setting values. A pristine final tab silently no-ops and a non-pristine final tab is replaced. Native hosts keep their existing affordances.

### Confirmation
- Setting on in a window-capable host preserves the existing window-close confirmation path.
- Setting off, or any `CloseWindow`-disabled host, uses the existing tab/session confirmation and unsaved-state checks because the window will not close and cannot supply a later window-close confirmation.
- A pristine final-tab no-op is decided before confirmation and presents no prompt.
- The replacement is created only after confirmation succeeds.
- A failed creation must leave the original tab attached and usable; do not remove the original until a valid replacement exists.

## Pristine state and Undo Close contract
Use one pristine definition for both the final-tab no-op and conditional Undo Close. Do not introduce a second empty-session or untouched-state heuristic.

A newly created tab, including an automatic replacement and an ordinary New Tab, starts **pristine** and becomes permanently **non-pristine** on the first user action that mutates that tab's content, identity, or pane structure. A restored/imported tab or any tab for which the runtime did not observe creation starts non-pristine; pristine state is never inferred from visible emptiness.

Actions that make it non-pristine:

- typing, deleting, or pasting terminal/agent input, even if the buffer later becomes empty
- executing a command, submitting an agent prompt, or launching a workflow that writes into the session
- a user-initiated cwd change
- splitting, closing, replacing, moving, or resizing a pane, or any other user-initiated pane-layout mutation
- renaming the tab or one of its sessions/panes
- changing the tab's color, pin state, or group membership, or moving/detaching the tab
- relaunching, replacing, or changing the tab's profile/session

Actions that keep it pristine:

- focus or blur
- switching to or from the tab
- hover
- opening or dismissing menus without selecting a mutating action
- selecting or copying text
- scrolling
- opening, closing, or interacting with a different tab without mutating this tab
- background shell bootstrap, title, prompt, cwd, or rendering updates that are not caused by a user mutation

Do not infer pristine state by comparing the current buffer/layout with its initial value: typing and deleting back to empty must remain non-pristine. Track the one-way runtime transition explicitly and propagate only user-originated mutation signals; generic `AppStateChanged` is unsuitable because focus and background initialization also emit it.

When a pristine tab is the final tab under effective keep-window behavior, all four close-tab entry points silently no-op. A no-op does not alter pristine state.

Closing a non-pristine final tab through the replacement path still adds the closed tab to Undo Close when Undo Close is enabled. Associate that undo item with the exact automatically-created replacement tab.

On Undo Close:

- If the associated replacement still exists in the originating workspace and is pristine, remove and clean it up without adding another undo item, then restore the closed tab at its normal restored position. The result replaces the fresh tab rather than producing two tabs.
- If that exact replacement is missing or non-pristine, restore the closed tab alongside the current tabs. Never discard current user work.
- If Undo Close is disabled or the undo item expires, the replacement remains an ordinary tab.

Closing a pristine non-final tab remains allowed and uses normal Undo Close. If an automatic replacement later becomes non-final and is closed while pristine, normal undo can restore it; an older undo item that references that exact replacement must still apply the conditional replace-vs-restore-alongside rule when it reaches the top of the stack.

The replacement/undo association and pristine marker are runtime-only. They are not serialized. If Warp restarts before undo, session restore brings back the replacement as a normal, conservatively non-pristine tab and no stale conditional-replacement behavior survives.

## Cross-window drag
Cross-window behavior follows the same effective host decision but is not recorded as Undo Close because the dragged tab still exists in the target:

- Setting on in a window-capable host: preserve the current one-tab drag behavior. A handoff into another window transfers the tab and closes the source window.
- Setting off, or a host that cannot close its window: when a one-tab source commits a handoff into another existing window, transfer the dragged tab to the target and leave a normal fresh replacement in the source window.
- This remains allowed when the dragged final tab is pristine. Drag is a move, not a close-tab action, so the pristine final-tab no-op does not apply. Moving/detaching marks the transferred tab non-pristine under the existing one-way mutation contract.
- Create the source replacement only when the handoff commits. Hover, cancellation, dropping back into the source, and other aborted transfers must not leave an extra tab.
- Seed the replacement while the original tab is still available so ordinary working-directory and default-session rules can inspect the previous session.
- The source must retain its original bounds and chrome; it must not be left at the floating preview position, hidden, transparent, or marked as a drag-preview window.
- Dropping a one-tab window on empty desktop space remains a window move, not a tab close, and does not create a replacement.
- Transfers from a source that retains at least one other tab are unchanged.
- Persistence remains paused for the in-flight view transfer and resumes only after source and target each own a unique pane group. No zero-window/zero-tab or duplicate-pane snapshot may be written.

Implement this as an explicit keep-source mode in the single-tab drag state machine. Reusing the current source-window-as-preview path and trying to recreate the source only after transfer is rejected because it loses the original window position and previous-session context. Creating a replacement at drag start is also rejected because canceled drags would leak a tab.

## Implementation outline
1. Add the synced general setting, Settings widget/action/telemetry, Command Palette toggle, and context flag.
2. Centralize a final-tab outcome (`close window`, `no-op pristine`, or `replace non-pristine`) from the setting, `ContextFlag::CloseWindow`, and the tab's one-way pristine marker.
3. Return before confirmation, telemetry, undo, or allocation for the pristine no-op. Refactor the non-pristine replacement path so replacement is created first through the ordinary New Tab/empty-workspace seed path, then the original is removed through existing detach, telemetry, persistence, and undo cleanup.
4. Pair final-tab undo data with the created replacement's identity and add an explicit one-way pristine state to runtime tab data for all newly created tabs. Treat restored/imported/unknown-origin tabs as non-pristine.
5. Emit/handle user-mutation signals at the narrow terminal, pane, and tab actions listed above; do not use broad app-state notifications.
6. Remove the single-tab `CloseWindow` gating from horizontal/vertical close rendering, middle-click, the context menu, and the close-active-tab binding while retaining the effective-behavior guard in workspace logic.
7. Extend the cross-window drag state machine with commit-only replacement of a one-tab source when effective behavior is keep-window; bypass the close-action no-op for committed drag handoff.

## Design alternatives

### Fresh tab content
- **Chosen:** ordinary New Tab semantics, with host-specific empty-workspace seeding where terminal creation is unavailable.
- **Rejected:** clone the closed tab. It risks replaying commands/environment, invents a second launch contract, and conflicts with the requester's explicit choice.

### Undo behavior
- **Chosen:** replace only the exact pristine automatic replacement; otherwise restore alongside it.
- **Rejected:** always restore alongside. It leaves an unwanted empty tab in the common immediate-undo path.
- **Rejected:** always remove the replacement. It can discard user work.
- **Rejected:** compare current state with an initial snapshot. It incorrectly treats type-then-delete and mutate-then-revert as pristine.

### Pristine final-tab close
- **Chosen:** in keep-window behavior, silently no-op all four close actions when the final tab is pristine; keep every affordance visible and enabled.
- **Rejected:** repeatedly close and recreate pristine tabs. Cmd/Ctrl+W spam allocates empty sessions and fills runtime/undo state without changing the visible result.
- **Rejected:** hide or disable affordances while pristine. It conflicts with the settled always-available affordance contract and creates host-specific discoverability churn.
- **Rejected:** show a toast or animation. It adds UI scope and becomes noisy under the repeated-close case this rule is designed to absorb.

### `CloseWindow`-disabled hosts
- **Chosen:** expose all four close affordances and always use keep-window behavior: no-op a pristine final tab and replace a non-pristine final tab, with supporting Settings copy.
- **Rejected:** preserve current affordance gating. It prevents the requested behavior in web/link-only hosts.
- **Rejected:** honor the setting by attempting or simulating window close. These hosts cannot fulfill it; the requester explicitly selected a host fallback even though the global setting is host-dependent.

### Cross-window drag
- **Chosen:** a commit-only keep-source drag mode.
- **Rejected:** create a replacement as soon as drag starts. Canceled or returned drags would produce a spurious tab.
- **Rejected:** close and reconstruct the source after handoff. It loses window identity/position and cannot reliably apply previous-session cwd rules.

## Validation criteria

### Automated behavior
1. The new setting defaults to on, round-trips through the settings file, syncs globally subject to sync preferences, and has working Settings and Command Palette toggles.
2. In a window-capable host with the setting on, each of close button, middle-click, **Close tab**, and `workspace:close_active_tab` closes a one-tab window exactly as today and creates no replacement. Repeat with a pristine final tab to prove pristine state does not alter the default path.
3. In a window-capable host with the setting off, each of those four entry points is a silent no-op on a pristine final tab: the same tab/window remain, no confirmation appears, and session count, undo depth, close telemetry, and persisted workspace state do not change. Repeating the action must remain allocation-free.
4. In a window-capable host with the setting off, each of those four entry points closes a non-pristine final tab and leaves the same window open with exactly one pristine fresh tab and intact chrome.
5. Replacement creation honors:
   - default terminal/profile
   - non-terminal default session modes or launch configurations
   - both new-tab placement values
   - each supported new-tab working-directory rule
   - no copied title, command, environment, pane layout, or explicit cwd from the closed tab outside normal inheritance rules
6. A close operation that starts with multiple tabs and removes the remaining tabs reaches the same on/off and pristine/non-pristine final behavior without an empty-tab panic or stale active index.
7. Closing a non-final tab remains unchanged, including closing and undo-restoring a pristine non-final tab.
8. For replacement behavior, successful tab/session confirmation creates the replacement and removes the original; cancel and replacement-creation failure leave the original unchanged. The pristine no-op bypasses confirmation and creation.
9. In a `CloseWindow`-disabled host, with the setting both on and off:
   - all four close affordances are available
   - closing a pristine final tab silently no-ops
   - closing a non-pristine final tab retains the host window/chrome and seeds a valid host-specific fresh tab
   - no close-window action is attempted
10. A newly created ordinary or automatic tab starts pristine; restored/imported/unknown-origin tabs start non-pristine.
11. Focus, tab switching, hover, menu open/dismiss, selection/copy, and scroll each preserve pristine state and therefore preserve the final-tab no-op.
12. Each enumerated mutation category permanently marks the tab non-pristine and changes its next keep-window final close from no-op to close-and-replace.
13. Type-then-delete back to an empty input remains non-pristine and closes/replaces.
14. A pristine final-tab no-op adds no Undo Close item. After a non-pristine final tab closes/replaces, immediate Undo Close removes the exact pristine replacement and restores the closed tab as the sole tab.
15. If that automatic replacement becomes non-pristine before undo, undo restores the closed tab alongside it. If it becomes non-final and is closed while pristine, normal undo restores it without corrupting the older replacement association.
16. Mutating or removing a different tab does not falsely mark or remove the replacement; missing replacement identity falls back to restore-alongside.
17. Undo disabled and undo expiry leave a usable normal replacement and cannot cause a later unrelated undo to remove it.
18. With session restore enabled, the automatic replacement appears in the saved snapshot and restores as a normal non-pristine tab after restart. Its pristine/undo association does not restore.
19. Cross-window drag with the setting on preserves current behavior for a one-tab source handoff, including a pristine source tab.
20. Cross-window drag with the setting off, or in a `CloseWindow`-disabled host at either setting value, permits a pristine or non-pristine one-tab source to commit the original tab into the target, leaves exactly one normal pristine fresh tab in the source at its original bounds, marks the moved tab non-pristine, and persists both windows without duplicate pane IDs.
21. Canceling, returning, or dropping a one-tab drag on empty desktop space creates no replacement. Multi-tab source transfers remain unchanged.

Add failing-first regression coverage in the existing workspace/tab/undo-close/cross-window test modules. Tests must exercise behavior through production actions and events; do not add a test-only production seam.

### Repository checks
The implementation must pass:

- `./script/format`
- the repository's clippy invocation from `./script/presubmit`
- targeted tests for every touched app module, including workspace close, tab affordances, undo close, settings, and cross-window drag
- `./script/presubmit` as the final local gate
- a successful Dogfood build for the prototype artifact

### Visual proof and prototype
Provide computer-use video proof from the running feature-branch/Dogfood build:

1. Show the setting in **Settings → Features → General**, including the exact label and supporting text.
2. With the setting on, close a pristine only tab and show the window closing.
3. With the setting off, repeatedly close a pristine only tab and show a silent no-op: the exact tab/window remain and no sessions accumulate.
4. In the same keep-window state, mutate the last tab, close it, and show the same window/chrome remain with one fresh tab that follows the configured default and cwd rule.
5. Show immediate Undo Close replacing the pristine fresh tab from the non-pristine close, then repeat after mutating the replacement to show restore-alongside.
6. In a `CloseWindow`-disabled web/link-only host with both setting values, show the final-tab affordances, pristine no-op, and non-pristine host-specific replacement.
7. Show a pristine one-tab cross-window handoff with the setting off, with the original tab in the target and a fresh tab in the unchanged source window.

Attach the video artifact to the implementation PR and task record. Publish a Dogfood/feature-branch prototype build link on APP-5242 so the requester can compare this variant hands-on with APP-5230.

## Out of scope
- Changing the settled setting label, location, default, global scope, or sync behavior.
- Cloning any state from the closed tab beyond ordinary New Tab inheritance.
- Removing workspace chrome or supporting a persistent zero-tab/empty-pane workspace.
- Changing window-close or app-quit behavior beyond the final-tab decision.
- Changing general Undo Close grace-period or stack ordering.
- Introducing a serialized replacement-tab type.
- Modifying APP-5230 or its branch/PR.
