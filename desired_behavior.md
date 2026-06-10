# Navigation Stack — Desired Behavior

## Overview
IDE-style forward/backward navigation stack, similar to VS Code's Go Back / Go Forward.

## Feature Flag
- The entire feature is gated behind the `NavigationStack` feature flag.
- When the flag is disabled, **nothing** related to this feature is active:
  - Keybindings are not registered.
  - Nav stack buttons do not render.
  - Navigation entries are not recorded.
  - Settings toggle is hidden.

## Keybindings
- Default keybindings match VS Code on all platforms:
  - **macOS**: `Ctrl+-` (back), `Ctrl+Shift+-` (forward)
  - **Windows/Linux**: `Alt+Left` (back), `Alt+Right` (forward)
- Keybindings are only active when the feature flag is enabled.
- Back and Forward should also be accessible from the command palette.

## Tab Bar Buttons
- Back (`<`) and Forward (`>`) chevron buttons are shown in the tab bar, positioned after the grid/layout icon and before the first tab.
- Buttons are only visible when **both**:
  1. The `NavigationStack` feature flag is enabled.
  2. The "Show navigation buttons in tab bar" setting is enabled.
- Each button is visually disabled (greyed out) when there is nothing to navigate to in that direction.

## Settings
- A "Show navigation buttons in tab bar" toggle lives under Settings > Features.
- The setting defaults to **on**.
- The setting is only visible when the `NavigationStack` feature flag is enabled.
- The setting is searchable from the command palette and settings search.

## Initial State
- The navigation stack is **empty** when the app starts up.
- There is no history carried over between app launches.

## Events That Push to the Nav Stack
The following user-initiated actions record a new entry on the navigation stack:

1. **Focusing a different pane** — switching focus from one pane to another within the same tab.
2. **Focusing a different tab** — switching to a different tab within the same window.
3. **Focusing a different window** — bringing a different Warp window to focus.
4. **Scrolling a pane** — when the user scrolls within a pane (e.g. via mouse wheel, trackpad, keyboard scroll).
5. **LSP-initiated navigation** — when the user performs Go to Definition (keyboard, context menu, or Cmd-click) or navigates from a Find References card. The pre-navigation scroll position is captured immediately (not debounced) so the user can navigate back to their original location.

### Scroll Position on Pane Activation
When a pane becomes active (gains focus), the **departing** pane's current scroll position must be captured as part of the navigation entry pushed onto the back stack. This ensures that navigating back restores the previous pane at the exact scroll offset the user was viewing when they left it. This applies to all pane focus changes: splitting panes, clicking a different pane, or any other action that moves focus.

### Intra-Pane Scroll Navigation
When a user scrolls **within a single pane** (mouse wheel, trackpad, PageUp/PageDown, clicking a block header, etc.), the **pre-scroll position** should be recorded so the user can navigate back to where they were.

Because scrolls are continuous (many tiny deltas), recording must be **throttled/debounced**:
1. On the **first** user-scroll event in a series, capture the pane's current scroll position as the "scroll anchor" (this is position A).
2. While the user continues scrolling, keep extending the debounce window but do **not** update the anchor — the anchor stays at position A.
3. Once scrolling stops for a debounce period (~1.5 seconds), push the anchor (position A) onto the back stack.
4. The user is now at position B. Navigating back restores position A.

If the user switches focus (to another pane, tab, or window) before the debounce fires, the pending scroll anchor is flushed immediately — it is pushed onto the back stack before the focus-change entry.

### Scroll Events — User vs. System
- Only **user-initiated** scroll events should add entries to the nav stack.
- **System-triggered** scroll events should **not** create nav stack entries. Examples of system scrolls to ignore:
  - Auto-scroll to bottom when new output appears in a terminal.
  - Scroll position changes caused by restoring a navigation entry (i.e. navigating back/forward itself).
  - Scroll adjustments from window/pane resizing.
  - Programmatic scrolls triggered by search-and-jump or similar features.

### Session Restoration
- Restoring workspace/session state should **not** populate the nav stack.
- Programmatic focus changes that happen as part of restoration should be treated as system-triggered and should not create nav entries.

## Back / Forward Behavior
- **Go Back**: Pops the current entry off the active stack and pushes it onto the forward (redo) stack, then restores the previous entry.
- **Go Forward**: Pops the top of the forward stack and pushes the current state back onto the back stack, then restores the forward entry.
- If a user navigates back and then performs a **new navigation action** (focus change, scroll, etc.), the forward stack is cleared — consistent with standard undo/redo and browser behavior.

## What Gets Restored
When navigating back or forward, the following state is restored:

1. **Window** — the correct window is brought to focus.
2. **Tab** — the correct tab within that window is activated.
3. **Pane** — the correct pane within that tab is focused.
4. **Scroll position** — the pane is scrolled to the recorded position (via scroll snapshot).

If the restored pane is a code editor pane, the editor must regain real text-input focus, not just pane-selection state. The caret should become active/blinking immediately so typing works without an extra click.

## Closed Targets and Cleanup
- If a navigation entry points at a pane, tab, window, or panel that was explicitly closed but is still restorable via undo-close/session restoration, navigating back or forward should restore that target instead of skipping it.
- If that closed target is later permanently cleaned up (for example, the undo-close grace period expires), matching navigation entries should be pruned from both back and forward history.
- Pruning should happen automatically during cleanup so stale entries do not leave back/forward navigation stuck on dead targets.
- Closing a temporary code diff pane must not discard its navigation entries if the underlying `CodeDiffView` can still be reopened.
- If a code diff navigation entry is restored after the temporary pane was closed, Warp should reopen the code diff view and then restore its recorded editor tab/scroll position.

## Best-Effort Scroll Restoration
- Restoring a navigation entry should always try to focus the target even if the recorded scroll position is no longer exact.
- If the recorded scroll position is out of range because content changed (for example after terminal clear / command-K, notebook edits, code editor changes, or diff updates), Warp should clamp to the nearest valid position instead of crashing.
- This best-effort behavior applies to terminal panes, code editor panes, notebook panes, code diff panes, and code review editors.

## Cross-Window Behavior
- Navigation entries include the window ID.
- When the user focuses a different Warp window (e.g. via clicking or Cmd-`), an entry for the previously focused window/tab/pane should be pushed onto the back stack, just like a tab or pane switch.
- Navigating back/forward should work across windows — activating the correct window, tab, pane, and scroll position.
- If the target window no longer exists (was closed), the entry is consumed/skipped without error.

## Code Review Panel
Scroll and LSP navigation events from the code review panel (right panel) also push to the nav stack. The code review panel is not a pane, so entries use a `CodeReview` scroll snapshot variant that carries the panel scroll location. The `pane_id` in the entry is the focused terminal pane at recording time (used for dedup only).

- **Scroll**: User scrolls within a code review editor → debounced push, same as pane scroll.
- **LSP goto-definition**: Before navigating to the definition target, the pre-scroll position is captured and pushed immediately (flush + push).
- **Restoration**: When navigating back to a `CodeReview` entry, Warp should reopen the right panel if needed and restore the recorded editor scroll position best-effort.

## Known Issues / TODO
- **Forward navigation (redo) not working at runtime.** Back navigation works correctly, but pressing the forward keybinding (`Ctrl+Shift+-` on Mac, `Alt+Right` on Linux/Windows) does not navigate forward even when the forward stack has entries. The data model is correct (unit tests pass), so the issue is likely in the keybinding dispatch or the `navigate_forward` call site. Needs debugging.
