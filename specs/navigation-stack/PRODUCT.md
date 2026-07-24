# Navigation Stack

## 1. Summary

Add IDE-style back/forward navigation to Warp so users can return to prior context after moving between panes, tabs, windows, scroll positions, and code navigation targets. The feature should feel consistent with editors like VS Code while preserving Warp-specific concepts such as multi-pane tabs, cross-window navigation, code review editors, and temporary restorable views.

The feature is fully gated behind the `NavigationStack` feature flag. When the flag is disabled, Warp should behave exactly as it does today: no history is recorded, no related buttons or settings appear, and no navigation commands are available.

## 2. Problem

Warp supports complex navigation across panes, tabs, windows, editors, code review surfaces, and LSP-driven jumps, but users do not have a reliable way to return to where they were previously working. This makes exploration expensive: a user can jump to a definition, switch panes, scroll elsewhere, or move to another window, but then must manually reconstruct their previous context.

Users need a predictable back/forward model that restores their prior working location, including the exact pane or panel and the best available scroll position, without requiring manual recovery.

## 3. Goals

- Let users navigate backward and forward through meaningful workspace context changes.
- Restore the correct window, tab, pane, and scroll position when returning to prior context.
- Match familiar back/forward keyboard shortcuts and expose the actions through multiple entry points.
- Make navigation stack behavior predictable across pane focus changes, scrolling, LSP navigation, and cross-window movement.
- Ensure UI affordances and settings appear only when the feature is enabled.
- Preserve history for targets that were temporarily closed but are still restorable.
- Avoid creating noisy or misleading history entries from system-driven state changes.

## 4. Non-goals

- Persisting navigation history across app launches.
- Recording history while the feature flag is disabled.
- Replacing existing undo-close or session restoration systems; navigation should integrate with them where relevant, not supersede them.
- Capturing every tiny scroll delta as a separate history entry.
- Introducing navigation behavior for system-triggered auto-scroll or restoration-driven state changes.

### Deferred (explicitly out of scope for this release)

- An onboarding, changelog, or coachmark affordance introducing the feature. Verification flagged that nothing proactively introduces back/forward navigation; this is deliberately deferred to a follow-up.

## 5. Figma / Design References

Figma: none provided

## 6. User Experience

### Feature gating

- The entire feature is gated behind the `NavigationStack` feature flag.
- When the flag is disabled:
  - Back and forward keybindings are not registered.
  - Back, forward, and clear-navigation actions are not available from the command palette.
  - Navigation buttons are not shown in the tab bar.
  - Navigation entries are not recorded.
  - The related setting is hidden.

### Entry points

- Back and forward actions are exposed through keyboard shortcuts, the command palette, and optional tab bar buttons.
- A `Clear Navigation Stack` command is exposed through the command palette when the feature is enabled.
- Palette search surfaces the navigation actions for synonym queries: searching "navigate", "navigation", or "history" matches `Go Back`, `Go Forward`, and `Clear Navigation Stack` via search-only keywords attached to the bindings. Keywords affect matching only and are never rendered or highlighted.
- Default keyboard shortcuts match VS Code conventions:
  - macOS:
    - Back: `Ctrl+-`
    - Forward: `Ctrl+Shift+-`
  - Windows/Linux:
    - Back: `Alt+Left`
    - Forward: `Alt+Right`

### Naming

- The feature uses one consistent action name across every surface: `Go Back` / `Go Forward` (sentence case "Go back" / "Go forward" in tooltips).
- The command palette, keyboard-shortcut editor, and tab-bar button tooltips must not use divergent verbs (e.g. "Navigate back") for the same action.

### Keybinding non-interference

- Navigation shortcuts must never intercept keys destined for a foreground terminal program. While the focused pane is running a long-running/interactive command, `Alt+Left` / `Alt+Right` (and any rebinding of Go Back / Go Forward) are delivered to the program, not consumed by navigation.
- While typing in Warp's own input editor, the shortcuts follow IDE convention (navigation wins), matching VS Code; word-movement remains on `Ctrl+Left/Right` on Linux/Windows.
- The shortcuts are remappable from Settings > Keyboard shortcuts.

### Tab bar buttons

- Back (`<`) and forward (`>`) chevron buttons appear in the tab bar after the grid/layout icon and before the first tab.
- In vertical-tabs mode, the same buttons appear in the title bar immediately right of the left toolbar buttons; the feature renders in both layouts and history survives switching layouts mid-session.
- These buttons are visible only when:
  1. `NavigationStack` is enabled.
  2. The user setting to show navigation buttons in the tab bar is enabled.
- Each button is visibly disabled when there is no valid destination in that direction.
- The enabled state must be clearly distinguishable from the disabled state at a glance: enabled chevrons use the same prominent icon color as neighboring toolbar icons rather than the muted sub-text color.
- Buttons show their tooltip (action name plus shortcut) when hovered in both the enabled and disabled states. Both buttons are disabled on every fresh launch, so the disabled tooltip is the primary discovery affordance for new users.

### Settings

- A setting named `Show navigation buttons in tab bar` appears under Settings > Features.
- The setting defaults to on.
- The setting is visible only when `NavigationStack` is enabled.
- The setting is searchable from settings search and the command palette.
- The setting row includes descriptive subtext stating the keyboard shortcuts and clarifying that hiding the buttons does not disable navigation (shortcuts and palette actions keep working).
- The command palette toggle for this setting is state-aware: it reads `Enable …` when the buttons are hidden and `Disable …` when they are shown.

### Initial state

- Navigation history starts empty on app launch.
- No history is restored from prior launches.

### Events that create history

- User-initiated changes that should record a navigation entry include:
  - focusing a different pane within a tab
  - focusing a different tab within a window
  - focusing a different Warp window
  - scrolling within a pane
  - navigating via LSP-driven actions such as Go to Definition, including keyboard, context-menu, and command-click entry points
  - navigating from a Find References card

### Pane activation and departing scroll position

- When focus moves from one pane to another, the entry added to history captures the departing pane’s current scroll position.
- Navigating back to that entry restores the user to the exact pane and the best available approximation of the scroll offset they were viewing when they left.
- This applies regardless of how pane focus changed, including clicking another pane or changing pane focus through layout actions.

### Intra-pane scroll history

- User scrolling within a pane should create history entries based on the position the user is leaving, not the position they end at.
- Scroll recording is debounced so a continuous scroll gesture creates one history anchor rather than many small entries.
- The expected behavior is:
  1. On the first user scroll event in a continuous series, capture the current scroll position as the anchor.
  2. As scrolling continues, extend the debounce window but keep the same anchor.
  3. After scrolling stops for roughly 1.5 seconds, record that anchor as a back-history entry.
  4. Navigating back returns to that anchor position.
- If the user changes focus before the debounce completes, the pending scroll anchor is flushed immediately before the focus-change entry is recorded.
- A minimum scroll delta applies: a trivial or net-zero scroll twitch that ends within a few lines of the previous history anchor must not create a near-duplicate entry. Back should never move the viewport by an imperceptible amount.

### User-driven versus system-driven scrolling

- Only user-initiated scrolls create history.
- System-driven scroll changes must not create entries, including:
  - auto-scroll caused by terminal output
  - scroll changes caused by restoring a history entry
  - scroll changes caused by resizing windows or panes
  - programmatic search/jump behavior and similar system navigation

### Session restoration

- Restoring workspace or session state must not populate navigation history.
- Programmatic focus changes that occur during restoration are treated as system-driven and do not create entries.

### Back / forward semantics

- Back moves from the current state to the previous recorded state and pushes the current state onto the forward stack.
- Forward moves to the next available forward state and pushes the current state back onto the back stack.
- If the user navigates backward and then performs a new user-initiated navigation action, the forward stack is cleared.

### Clear Navigation Stack

- `Clear Navigation Stack` removes all back and forward history entries.
- Running `Clear Navigation Stack` does not change the user’s current window, tab, pane, panel state, or scroll position.
- After clearing, both back and forward navigation are unavailable until the user creates new navigation history through subsequent actions.

### What must be restored

- Restoring a history entry should restore, in order of user-visible effect:
  1. the correct window
  2. the correct tab in that window
  3. the correct pane or panel context
  4. the recorded scroll position, using best-effort restoration if exact restoration is not possible
- If the restored target is a code editor pane, text input focus must be restored fully so the caret is active immediately and typing works without an extra click.
- When a code editor scroll position is restored, the caret is co-located with the restored viewport. The first keystroke after navigating back must not yank the viewport away to a stale caret position; restored scroll holds under continued typing.

### Closed targets and restorable views

- If a history entry points to a pane, tab, window, or panel that was explicitly closed but is still restorable through existing restore flows, back/forward navigation should restore it rather than skip it.
- If that target later becomes permanently non-restorable, matching history entries should be removed from both back and forward history automatically.
- Temporary code diff views should retain useful history while the underlying diff is still reopenable.
- If the user navigates back to a code diff entry after its temporary surface was closed, Warp should reopen the diff view and then restore the recorded editor location and scroll position best-effort.

### Best-effort scroll restoration

- Restoring a history entry should always prioritize getting the user back to the correct target, even if the exact prior scroll position is no longer valid.
- If content changed and the recorded scroll position is out of range, Warp should clamp to the nearest valid position instead of failing.
- This best-effort behavior applies across terminal panes, code editor panes, notebooks, code diff views, and code review editors.

### Cross-window behavior

- History entries include window identity.
- Moving focus to a different Warp window records the previously focused context just like pane and tab switches do.
- Back and forward should work across windows, bringing the correct window to the front and then restoring its tab, pane or panel, and scroll location.
- Cross-window Go Back must preserve the forward stack. The window-focus change caused by the restore itself is system-driven and must not be recorded as a new navigation (which would clear forward history); Go Forward immediately after a cross-window Go Back returns to the departed window.
- If a target window was closed but is still restorable through undo-close, Go Back reopens it, reattaches its panes, and restores the recorded tab/pane/scroll — equivalent in capability to `Reopen Closed Session`.
- If a target window no longer exists and cannot be restored, the entry is consumed safely without surfacing an error.

### Code review panel behavior

- The right-side code review panel participates in navigation history even though it is not a pane.
- User scrolls in the code review editor create debounced history entries using the same scroll-anchor behavior as pane scrolling.
- LSP-driven navigation from the code review panel captures the pre-navigation location immediately so the user can return to it.
- Navigating back to a code review history entry reopens the right panel if necessary and restores the recorded editor scroll position best-effort.

## 7. Success Criteria

1. When the feature flag is off, Warp exposes no user-visible or functional part of the navigation stack feature.
2. Back and forward actions are available through keyboard shortcuts and the command palette when the flag is on.
3. `Clear Navigation Stack` is available from the command palette when the flag is on and unavailable when the flag is off.
4. Tab bar back/forward buttons appear only when both the feature flag and the relevant setting are enabled.
5. The tab bar buttons show a disabled state when there is no valid destination in that direction.
6. Navigation history is empty on launch and does not persist across restarts.
7. Switching focus between panes records the departing pane context, including its current scroll position.
8. Switching tabs records the previous tab context.
9. Switching Warp windows records the previous window/tab/pane context and supports cross-window back/forward restoration.
10. A continuous user scroll within a pane results in a single back-history entry anchored at the pre-scroll position.
11. If focus changes before the scroll debounce completes, the pending scroll anchor is flushed before the focus-change entry is recorded.
12. System-driven scroll changes never create history entries.
13. Session or workspace restoration does not populate navigation history.
14. LSP-driven navigation and Find References navigation capture the pre-navigation location so the user can return to it with Back.
15. Using Back restores the correct window, tab, pane or panel, and best-effort scroll position.
16. Using Forward restores the next forward destination when one exists.
17. Performing a new navigation action after going Back clears the forward stack.
18. Running `Clear Navigation Stack` empties both back and forward history without moving the user away from their current context.
19. Restoring a code editor pane returns real text-input focus immediately.
20. Restorable closed targets are reopened when needed instead of being skipped.
21. Permanently stale history entries are pruned automatically so dead destinations do not accumulate.
22. If a saved scroll location is invalid because content changed, Warp clamps to the nearest valid location without crashing or leaving the user in an unusable state.
23. Code review panel scroll and LSP-driven navigation participate in history and restore correctly.
24. Go Forward works after a cross-window Go Back: the focus change produced by the restore does not clear the forward stack.
25. Go Back reopens a closed-but-restorable window with its panes reattached and functional.
26. After restoring a code editor entry, typing immediately does not move the viewport away from the restored scroll position (caret is co-located).
27. Navigation shortcuts are delivered to a focused foreground terminal program instead of triggering navigation.
28. Hovering a disabled back/forward button shows its tooltip with the action name and shortcut.
29. Enabled and disabled button states are visually distinct; the enabled state matches the prominence of neighboring toolbar icons.
30. All surfaces use the `Go Back` / `Go Forward` naming; the palette toggle for the buttons setting is state-aware (`Enable …` / `Disable …`).
31. A net-zero scroll twitch does not create a history entry; Back after such a twitch is not a near-duplicate no-op.
32. The buttons render and function in vertical-tabs mode, and history survives switching between horizontal and vertical layouts.
33. Searching "navigate", "navigation", or "history" in the command palette surfaces the navigation actions.

## 8. Validation

- Manual: verify that, with the feature flag disabled, no related keybindings, command-palette actions, tab bar buttons, settings entry, or history behavior are present.
- Manual: enable the feature and confirm the default keyboard shortcuts perform Back and Forward on macOS and Windows/Linux respectively.
- Manual: confirm the command palette exposes Back, Forward, and `Clear Navigation Stack` only while the flag is enabled.
- Manual: toggle `Show navigation buttons in tab bar` and confirm the buttons appear and disappear correctly.
- Manual: switch between panes, tabs, and windows, then navigate backward and forward to confirm correct target restoration.
- Manual: scroll within a pane, stop, then use Back to confirm Warp returns to the pre-scroll anchor rather than the post-scroll location.
- Manual: start scrolling and then change pane, tab, or window focus before the debounce window ends; confirm the scroll anchor is still preserved before the focus-change entry.
- Manual: verify that auto-scroll from new terminal output does not create history.
- Manual: perform Go to Definition and Find References navigation, then use Back to return to the original context.
- Manual: close a restorable target, navigate back to it, and confirm Warp restores it instead of skipping it.
- Manual: wait until a formerly restorable target is no longer restorable and confirm stale entries no longer block navigation.
- Manual: change content so the original scroll position is no longer exact, then navigate back and confirm best-effort clamped restoration rather than failure.
- Manual: navigate through the code review panel, including scroll and LSP-driven jumps, and confirm back/forward restoration reopens the panel if needed and restores the recorded location.
- Manual: create some navigation history, run `Clear Navigation Stack`, and confirm both back and forward become unavailable while the current context stays unchanged.
- Manual: with two windows, Go Back across the window boundary and immediately Go Forward; confirm forward returns to the departed window.
- Manual: close a second window, Go Back from the survivor, and confirm the window reopens with working panes (typing works) and correct focus.
- Manual: in a code editor pane, scroll deep, navigate away, Go Back, then type; confirm the viewport stays at the restored position.
- Manual: run a foreground program (e.g. `cat`), press the Go Back shortcut, and confirm the program receives the key.
- Manual: hover both chevrons on a fresh launch (disabled state) and confirm tooltips appear; compare enabled vs disabled brightness.
- Manual: with the buttons visible, confirm the command palette shows `Disable Navigation Buttons in Tab Bar` (and `Enable …` when hidden).
- Manual: nudge the scroll wheel up/down within a second, wait for the debounce, and confirm Back does not perform a near-duplicate micro-jump.
- Manual: search "navigate" and "history" in the command palette and confirm `Go Back` / `Go Forward` / `Clear Navigation Stack` appear in the results.
- Regression: verify that Back and Forward both work at runtime when corresponding history exists.

## 9. Open Questions

None. See the Deferred subsection of Non-goals for intentionally postponed work (onboarding/changelog affordance).
