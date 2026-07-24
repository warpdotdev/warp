# TECH.md — Expose Terminal Accessibility Tree for macOS Assistive Apps

Issue: https://github.com/warpdotdev/warp/issues/11160
Product spec: `specs/GH11160/product.md`

## Problem

Warp's custom UI framework renders through a native macOS host view instead of AppKit controls, so macOS does not automatically see a meaningful Accessibility hierarchy for Warp tabs or terminal buffers. The current macOS surface exposes the host view as a single text area whose value is derived from the focused view. That helps focused screen-reader announcements, but it does not provide the window > tab group > tab > text area structure expected by external terminal-aware assistive apps.

The implementation needs a macOS-only AX compatibility tree that is grounded in Warp's Rust-side workspace, tab, pane, and terminal state while preserving the existing focused-view accessibility path.

## Relevant code

- `crates/warpui/src/platform/mac/objc/host_view.m (368-397)` — `WarpHostView` currently declares itself as an accessibility element, returns `NSAccessibilityTextAreaRole`, and returns `warp_get_accessibility_contents(self)` from `accessibilityValue`.
- `crates/warpui/src/platform/mac/window.rs (1244-1262)` — `warp_get_accessibility_contents` bridges from Objective-C to `AppContext::focused_view_accessibility_data(window_id)`.
- `crates/warpui_core/src/core/app.rs (1232-1255)` — `focused_view_accessibility_data` walks the responder chain and returns the first focused view's `AccessibilityData`.
- `crates/warpui_core/src/core/view/mod.rs (130-138)` — `View::accessibility_data` is the current generic hook for view-provided accessibility tree data.
- `app/src/terminal/view.rs (27151-27189)` — `TerminalView::accessibility_data` returns alt-screen output or recent visible blocks plus input text for the focused terminal.
- `app/src/terminal/model/alt_screen.rs (309-324)` — `AltScreen::output_to_string` serializes the current alt-screen grid.
- `app/src/terminal/model/block.rs (2211-2241)` — block output and block contents can be serialized to plain text.
- `app/src/pane_group/mod.rs (2185-2210)` — `PaneGroup` can resolve focused pane ID and active session ID from focus state.
- `app/src/pane_group/mod.rs (5441-5461)` — `PaneGroup::display_title` resolves a tab's displayed title from custom tab title or focused pane title.
- `app/src/pane_group/mod.rs (7105-7135)` — `PaneGroup` can resolve active/focused terminal views and terminal views by pane ID.
- `app/src/pane_group/mod.rs (8210-8239)` — `PaneGroup::render` renders the pane tree, including the focused/maximized pane behavior that the accessibility snapshot must mirror.
- `app/src/workspace/view.rs (10194-10239)` — `Workspace` owns tab activation and active-tab index state.
- `app/src/workspace/action.rs (90-139)` — `WorkspaceAction` contains tab activation, movement, rename, and close actions that should invalidate or notify the AX tree.
- `app/src/settings/accessibility.rs (1-17)` — current accessibility settings contain only screen-reader announcement verbosity.
- `app/src/settings_view/privacy_page.rs (1796-1888)` — existing Privacy settings widgets provide a precedent for privacy-sensitive settings.
- `crates/warpui/src/platform/mac/objc/window.m (813-843)` — existing AppKit notification helper posts accessibility announcements, but not tree/value/destroyed notifications for child elements.

## Current state

Today, AppKit can enumerate Warp application windows through standard `NSWindow` behavior. Once an accessibility client descends into a Warp window, it mainly sees the `WarpHostView` Metal-backed content view as a single `NSAccessibilityTextAreaRole`. Its value is fetched on demand from Rust by looking at the currently focused view's `accessibility_data`.

This has several limitations for the requested use case:

- There is no explicit AX tab group or tab element list.
- There is no per-tab or per-pane text area identity.
- A client cannot read inactive tab titles and selected state from AX.
- A client cannot correlate lifecycle updates for tab creation, tab close, tab rename, pane focus, or terminal text changes.
- `TerminalView::accessibility_data` currently returns recent block content for normal mode, not a strict visible viewport snapshot.
- The current surface is optimized for screen-reader announcements, not for a persistent accessibility object tree consumed by external apps.

## Proposed changes

### 1. Add an opt-in setting

Add a macOS-only boolean setting under `AccessibilitySettings` or `PrivacySettings`, with a privacy-page toggle in `app/src/settings_view/privacy_page.rs`.

Recommended internal shape:

- Setting group: prefer `PrivacySettings` if the setting is framed as external data exposure; otherwise `AccessibilitySettings` if product wants it grouped with accessibility controls.
- Storage key: `ExposeTerminalAccessibilityTree`.
- TOML path: `privacy.expose_terminal_accessibility_tree` or `accessibility.expose_terminal_accessibility_tree`.
- Default: `false`.
- Supported platforms: macOS only.
- Sync: default to local-only unless product/security approve syncing this privacy-sensitive preference.

The macOS AX child tree should be hidden when the setting is off. Existing `WarpHostView` focused accessibility behavior should continue regardless of this setting.

### 2. Introduce Rust-side accessibility snapshot types

Extend `warpui_core::accessibility` with structured snapshot data separate from the existing announcement-focused `AccessibilityContent`:

- `AccessibilityTreeSnapshot`
  - `window_id`
  - `tabs: Vec<AccessibilityTabSnapshot>`
  - `active_tab_index`
  - `generation`
- `AccessibilityTabSnapshot`
  - stable tab identifier, derived from the `PaneGroup` `EntityId`
  - tab index
  - title
  - selected boolean
  - `terminal_text_areas: Vec<AccessibilityTextAreaSnapshot>`
- `AccessibilityTextAreaSnapshot`
  - stable pane identifier, derived from `PaneId` or terminal `EntityId`
  - title or label
  - focused boolean
  - visible buffer text
  - optional frame if a reliable element position is available later

Add an `AppContext` accessor such as `accessibility_tree_snapshot(window_id)` that returns this tree only when the setting is enabled and the current root view can provide it. Keep this accessor distinct from `focused_view_accessibility_data` so the existing screen-reader path is not forced to serve compatibility-tree clients.

### 3. Have `Workspace` produce the window snapshot

Add a `View` hook or Workspace-specific callback to build the tree from the current `Workspace` state:

- Iterate `self.tabs` in visual order.
- Use `self.active_tab_index` to set selected state.
- Use each tab's `pane_group.as_ref(ctx).display_title(ctx)` for the AX tab title.
- Use each `PaneGroup` to enumerate visible terminal panes. If a public visible-pane iterator is not already available, add a narrow method on `PaneGroup` that returns visible terminal pane IDs and their `ViewHandle<TerminalView>` without exposing hidden panes.
- Mark the focused text area using `PaneGroup::focused_pane_id(ctx)` and/or active session ID.

This keeps tab and pane semantics in `app/src/workspace` and `app/src/pane_group`, where they already live, and keeps macOS-specific AX code out of product state.

### 4. Add a visible-buffer terminal snapshot method

Add a new method on `TerminalView`, for example `visible_accessibility_buffer_text(&self, ctx: &AppContext) -> String`, instead of reusing `accessibility_data` directly.

The first implementation should:

- For alt-screen mode, use the current alt-screen grid serialization path.
- For normal block mode, prefer the currently visible viewport rather than the last five visible blocks. Use `viewport_state`, block heights, and visible block filtering to compute the block/row range intersecting the pane viewport.
- Include the visible input prompt and input buffer only when the input box is visible.
- Preserve existing secret-obfuscation behavior used by terminal string serializers.
- Omit ANSI escape sequences, style metadata, hidden blocks, filtered-out content, hidden child-agent panes, and non-visible scrollback.

If exact visible-row extraction proves too risky for the first implementation, the implementer may start by reusing the current recent-block text as a prototype, but the spec should remain updated to call out that this does not yet satisfy the product requirement.

### 5. Implement native macOS AX child elements

Add Objective-C classes under `crates/warpui/src/platform/mac/objc/` or a Rust-backed Objective-C bridge in `crates/warpui/src/platform/mac/window.rs`:

- `WarpAccessibilityTabGroup`
  - Role: `NSAccessibilityTabGroupRole`.
  - Exposes tabs through `accessibilityTabs` or the appropriate AppKit attribute path.
  - Children include tab elements and, if AppKit requires, selected tab content.
- `WarpAccessibilityTab`
  - Role: `NSAccessibilityTabRole` if available, or the closest standard role that maps to `AXTab`.
  - Title: snapshot title.
  - Value or selected attribute: `1` for selected, `0` for not selected.
  - Children: text areas for visible terminal panes in the tab, or at least the primary focused text area.
- `WarpAccessibilityTextArea`
  - Role: `NSAccessibilityTextAreaRole`.
  - Value: snapshot visible buffer text.
  - Focused/selected: true for the focused terminal pane when applicable.

`WarpHostView` should override `accessibilityChildren` and related AppKit accessors to expose these child objects when the setting is enabled. The host view can continue to be an accessibility element for backward compatibility, but its child tree should make the richer structure discoverable by generic AX clients.

### 6. Cache and invalidate snapshots safely

Native AX objects need stable identity but current values. Use stable IDs from tab `PaneGroup` handles and pane IDs to reuse child objects across snapshot refreshes while updating their backing data.

Recommended flow:

- `WarpHostView` owns or references a `WarpAccessibilityTreeController`.
- On AX attribute reads, the controller asks Rust for the latest snapshot for the owning `window_id`.
- The controller reconciles tab/text-area objects by stable IDs.
- Snapshot building must avoid nested `TerminalModel` locks and keep lock scopes short, following `WARP.md` terminal locking guidance.
- Large text values should be bounded to the visible viewport; no additional line-limit should truncate the visible viewport.

### 7. Post AX notifications from existing state-change points

Add macOS notification helpers for:

- tab tree structural changes,
- tab selected-state changes,
- text-area value changes,
- text-area or tab destruction.

Likely invalidation points:

- `WorkspaceAction::ActivateTab`, `ActivateNextTab`, `ActivatePrevTab`, `MoveTabLeft`, `MoveTabRight`, `RenameTab`, `SetActiveTabName`, `CloseTab`, `AddDefaultTab`, and related tab creation/close actions.
- `PaneGroup` focus changes and pane add/remove/move events.
- terminal output processing, input buffer changes, scroll-position changes, alt-screen changes, and size/viewport changes in `TerminalView`.

Avoid posting high-frequency AppKit notifications for every byte of terminal output. Coalesce value-change notifications per frame or with a short debounce so assistive apps can update efficiently without flooding the main thread.

### 8. Feature flag and rollout

Introduce a runtime feature flag if product wants Dogfood/Preview rollout before Stable. The setting should only have effect when both the feature flag and user setting are enabled. If no feature flag is used, the setting itself is the rollout gate.

## End-to-end flow

1. User enables the macOS privacy setting.
2. An assistive app obtains the Warp application AX element with `AXUIElementCreateApplication(pid)`.
3. The app reads `kAXWindowsAttribute`; AppKit returns Warp `NSWindow` accessibility elements.
4. The app descends into a window's content view and reads the tab group children.
5. `WarpHostView` asks Rust for the latest `AccessibilityTreeSnapshot` for its `window_id`.
6. Rust builds a snapshot from `Workspace.tabs`, each tab's `PaneGroup`, and each visible terminal pane's `TerminalView`.
7. Native AX tab objects expose titles and selected state from the snapshot.
8. Native AX text-area objects expose visible terminal text from the corresponding terminal snapshot.
9. When Warp output, tab state, pane focus, or window state changes, Warp invalidates the tree and posts coalesced AX notifications. Polling clients still receive current values on the next attribute read.

## Risks and mitigations

- Risk: terminal buffer text can expose secrets to any macOS app that has Accessibility permission. Mitigation: gate behind an explicit privacy setting, default off, use clear setting copy, preserve secret redaction, and avoid scrollback in the first version.
- Risk: adding AX reads introduces terminal-model deadlocks or UI jank. Mitigation: keep snapshot lock scopes short, never call into rendering while holding a terminal lock, and build bounded visible-viewport strings only.
- Risk: high-frequency output floods the main thread with AX notifications. Mitigation: coalesce `AXValueChanged` notifications and make polling authoritative.
- Risk: tab and pane object identity changes on every read, breaking external clients. Mitigation: reuse native AX child objects keyed by stable tab and pane IDs.
- Risk: split panes create ambiguous "per-tab text" semantics. Mitigation: expose the focused pane as the primary text area and expose additional visible terminal panes where practical.
- Risk: AppKit role support differs by macOS version. Mitigation: prefer standard AppKit roles, test with Accessibility Inspector and AX clients, and document any role fallbacks.
- Risk: current `TerminalView::accessibility_data` is not a strict visible viewport. Mitigation: add a separate visible-buffer API for this feature rather than overloading the existing announcement path.

## Testing and validation

- Add unit tests for Rust snapshot builders:
  - no tree when the setting is disabled,
  - tabs are ordered by `Workspace.tabs`,
  - selected tab matches `active_tab_index`,
  - renamed tabs use `PaneGroup::display_title`,
  - hidden panes are omitted,
  - focused terminal pane is marked focused,
  - non-terminal panes do not produce terminal text areas,
  - alt-screen and normal-mode terminal text use the expected serializer.
- Add terminal model tests for visible-buffer extraction from normal block mode, including scrolled viewports, filtered/hidden blocks, input-visible and input-hidden states, and secret-redacted text.
- Add macOS-specific manual or automated AX validation:
  - inspect roles and attributes with Accessibility Inspector,
  - run a small AX script that traverses windows, tabs, selected state, and text areas,
  - verify `AXValueChanged` notifications are coalesced and delivered for terminal output and typing,
  - verify destroyed notifications on tab close and window close.
- Run `cargo fmt`.
- Run targeted Rust tests for `warpui_core`, `warpui` mac bindings where available, `warp_app` workspace/pane/terminal snapshot tests, and any settings tests touched by the new toggle.
- Run `cargo clippy --workspace --all-targets --all-features --tests -- -D warnings` before implementation PR review.

## Follow-ups

- Add optional scrollback exposure behind a separate privacy-controlled surface.
- Add richer frames for tab and text-area elements once WarpUI element position plumbing can provide reliable bounds for accessibility children.
- Add AX actions for selecting a tab only if product explicitly wants external apps to control Warp through Accessibility.
- Consider non-macOS accessibility tree equivalents after the macOS AX compatibility path is proven.
