# PRODUCT.md — Expose Terminal Accessibility Tree for macOS Assistive Apps

Issue: https://github.com/warpdotdev/warp/issues/11160

## Summary

Warp should expose a standard macOS Accessibility (AX) tree for its terminal windows, tabs, and visible terminal buffer text so assistive apps can interoperate with Warp in the same way they interoperate with Terminal.app and iTerm2. The initial version should prioritize a privacy-conscious, opt-in macOS surface that lets tools enumerate Warp windows, identify tabs and the selected tab, and read the currently visible terminal buffer as plain text.

Figma: none provided.

## Problem

Third-party assistive and automation apps such as Echo inspect terminal emulators through macOS Accessibility APIs rather than through app-specific adapters. Today Warp's custom GPU-rendered UI exposes only a minimal accessibility value for the focused view, without a meaningful tree of tab or terminal text elements. As a result, universal terminal tools cannot discover Warp tabs, cannot determine active sessions reliably, and cannot read visible terminal contents through the same AX paths that work for other terminal emulators.

## Goals

- Assistive apps can enumerate Warp windows through the standard application accessibility object and find stable window titles.
- Assistive apps can discover tabs in each Warp window, read each tab's display title, and determine which tab is selected.
- Assistive apps can read the visible terminal buffer for each relevant terminal tab or pane as plain text through a standard text-like AX element.
- Assistive apps can receive or poll enough state to track window lifecycle, tab lifecycle, selected-tab changes, and terminal text updates.
- Users retain explicit control over exposing terminal text to external assistive apps because terminal buffers can contain sensitive data.
- Existing VoiceOver announcements and current Warp accessibility behavior do not regress when the new compatibility surface is disabled.

## Non-goals

- Exposing full scrollback history in the first version. The visible viewport buffer is required; scrollback access is a follow-up.
- Exposing commands to mutate terminal state through AX, such as typing, clicking tabs, closing tabs, or resizing panes.
- Replacing Warp's existing screen-reader announcement system.
- Adding equivalent tree support on Windows, Linux, or web targets.
- Exposing hidden panes, closed tabs, filtered-out content, background agent panes, or other non-visible internal state.
- Bypassing secret redaction or revealing text that Warp intentionally hides or obfuscates in the rendered terminal.

## User experience

1. Warp adds a macOS-only privacy setting in Settings > Privacy named "Expose terminal contents to assistive apps" or equivalent copy. The setting is off by default unless product and security review decide otherwise before implementation.

2. The setting description explains that enabling it lets macOS assistive apps read visible terminal contents, tab titles, and selected-tab state through Accessibility APIs. The description must make clear that terminal contents can include sensitive information.

3. When the setting is off, Warp preserves its current accessibility behavior for focused-view announcements and focused text values. External apps must not gain the new tab tree or terminal-buffer compatibility surface.

4. When the setting is on, each normal Warp application window remains discoverable from `AXUIElementCreateApplication(pid)` via `kAXWindowsAttribute`. Window titles are stable, non-empty, and match the visible window or active tab title as closely as current macOS window behavior allows.

5. Each eligible Warp window exposes a tab container that assistive apps can find as an `AXTabGroup`-style element. The tab container exposes all visible top-level Warp tabs in that window through `kAXTabsAttribute` and/or equivalent AX children.

6. Each exposed tab element has:
   - `kAXTitleAttribute` set to the same display title users see for that tab, including user-renamed tab titles.
   - A selected-state signal, such as `kAXValueAttribute == 1` for the active tab and `0` for inactive tabs, `kAXFocusedAttribute`, `kAXSelectedAttribute`, or an equivalent standard signal.
   - A stable identity for the lifetime of the tab so clients can correlate updates.

7. Tab ordering in the AX tree matches the current visual tab order. Moving tabs left/right or up/down updates the AX order. Closing a tab removes its element and emits a destruction notification when notification support is available.

8. A tab with at least one visible terminal pane exposes one or more text-like child elements using `AXTextArea` or an equivalent text role. Each text element represents a visible terminal pane in that tab. If a compatibility client expects a single text area, the focused or active terminal pane in the tab is sufficient as the primary text area.

9. The text area's `kAXValueAttribute` returns plain text for the currently visible terminal viewport. For normal Warp block mode, this means the rendered visible terminal lines in the pane viewport plus the visible input line when present. For alt-screen applications, this means the current alt-screen grid. Text should be line-broken in visual order and should not include styling, ANSI escape sequences, invisible layout metadata, or hidden UI chrome.

10. In split-pane tabs, each visible terminal pane should be represented separately where practical. The selected or focused terminal pane is identifiable through focus or selected-state signaling. Non-terminal panes do not need to expose terminal text, but they must not cause the tab's terminal text area to return unrelated content.

11. The visible-buffer value updates when terminal output changes, when the user types into the visible input, when the viewport scrolls, when the active pane changes, when the selected tab changes, or when the tab title changes. Polling the value must return current data even if notifications are not delivered.

12. Warp posts standard macOS AX notifications when feasible:
    - Focused window changes use the standard focused-window signal.
    - Text area value changes post `kAXValueChangedNotification` or the AppKit equivalent.
    - Removed tabs, text areas, or windows post `kAXUIElementDestroyedNotification` or the closest available AppKit notification.

13. If notification delivery is incomplete in the first version, polling remains supported. A client that periodically reads `kAXWindowsAttribute`, tab titles, selected state, and text-area values must still be able to track the current state.

14. The AX value must match what the user can currently see in Warp. Secret-redacted text remains redacted, hidden blocks remain hidden, filtered-out content remains omitted, and non-visible scrollback is not included unless a future scrollback extension explicitly adds it.

15. The feature applies only on macOS builds. On other platforms, the setting is hidden or disabled, and no behavior changes.

## Success criteria

- A macOS AX client can start from `AXUIElementCreateApplication(pid)`, read `kAXWindowsAttribute`, and find each normal Warp window.
- For each window with one or more tabs, the client can find tab elements, read their titles, and determine the selected tab without Warp-specific APIs.
- For the selected tab with a visible terminal pane, the client can find a text-like element and read the currently visible terminal buffer from its value.
- In a window with multiple tabs, switching tabs changes the selected-state signal and exposes the newly selected tab's terminal text.
- In a tab with split terminal panes, the focused terminal pane is identifiable, and at least that pane's visible text is readable.
- In alt-screen applications such as `vim`, `less`, or full-screen TUIs, the text value reflects the current visible alt-screen contents rather than stale block-mode output.
- Terminal output, typing, scrolling, tab creation, tab close, tab rename, and selected-tab changes are observable by polling and, where implemented, by AX notifications.
- Disabling the privacy setting removes the new compatibility tree and terminal-buffer surface without breaking existing focused-view accessibility announcements.
- Enabling the setting does not reveal hidden or secret-redacted terminal text.

## Validation

- Build a small macOS AX inspection script or test utility that walks `AXUIElementCreateApplication(pid)`, reads windows, enumerates tabs, reads tab titles and selected state, and reads text-area values.
- Manually verify the utility against:
  - a single Warp window with one tab,
  - one window with multiple tabs,
  - renamed tabs,
  - horizontal or vertical split panes,
  - a normal shell prompt with recent block output,
  - an active alt-screen program,
  - tab creation, tab close, and tab switching,
  - the privacy setting off and on.
- Verify with VoiceOver that existing focused announcements still work when the compatibility surface is disabled and do not become confusing or duplicated when it is enabled.
- Add unit coverage for any Rust-side accessibility snapshot builders, including tab ordering, selected state, visible terminal text selection, split panes, empty/non-terminal panes, and privacy-setting gating.
- Add macOS-specific manual or integration coverage for AX roles, attributes, and notifications where automation is practical in the repository's test infrastructure.

## Open questions

- Should the new compatibility surface be off by default for all users, on by default when macOS screen-reader support is active, or controlled by a broader accessibility preference?
- What final setting name and description should product/security approve?
- Should the first implementation expose one text area per visible terminal pane, only the focused pane per tab, or both a primary compatibility text area plus per-pane text areas?
- Should scrollback access be added later, and if so should it be controlled by a separate privacy setting?
- Should the feature ship behind a temporary feature flag for Dogfood/Preview before Stable?
