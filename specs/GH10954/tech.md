# Speak selected terminal text on macOS — Tech Spec
Product spec: `specs/GH10954/product.md`
GitHub issue: https://github.com/warpdotdev/warp/issues/10954

## Context
Warp's terminal text selection model already knows how to turn the active visual selection into plain text for copy, copy-on-select, accessibility announcements during selection, and AI context. macOS Speak Selection, however, queries the native accessibility text surface rather than Warp's internal selection model directly.

Relevant code:
- `crates/warpui/src/platform/mac/objc/host_view.m:368` — `WarpHostView` declares itself as an accessibility element with `NSAccessibilityTextAreaRole`.
- `crates/warpui/src/platform/mac/objc/host_view.m:382` — `accessibilityValue` returns `warp_get_accessibility_contents(self)`.
- `crates/warpui/src/platform/mac/objc/host_view.m:389` — `accessibilityNumberOfCharacters` currently returns `0`; no selected-text or selected-range accessibility methods are implemented.
- `crates/warpui/src/platform/mac/window.rs:1244` — `warp_get_accessibility_contents` asks the focused view for `AccessibilityData` and returns its `content` as an `NSString`.
- `crates/warpui_core/src/core/app.rs:1351` — `focused_view_accessibility_data` walks the focused responder chain and returns the first view-provided `AccessibilityData`.
- `crates/warpui_core/src/core/view/mod.rs:133` — `View::accessibility_data` is the current framework hook for exposing focused-view text content to platform accessibility.
- `app/src/terminal/view.rs:26957` — `TerminalView::accessibility_data` exposes alt-screen output or the last five visible blocks plus input text as the terminal accessibility content.
- `app/src/terminal/model/terminal_model.rs:1783` — `TerminalModel::selection_to_string` delegates selected-text extraction to alt screen or block list depending on terminal mode.
- `app/src/terminal/model/blocks/selection.rs:871` — `BlockList::selection_to_string` expands regular, rectangular, and rich-content-inclusive block-list selections into selected text.
- `app/src/terminal/model/blocks/selection.rs:1214` — `BlockList::expand_selection` turns the stored selection anchors into ordered grid ranges for rendering and text extraction.
- `app/src/terminal/model/alt_screen.rs:240` — alt-screen selection can be converted to selected text through its own `selection_to_string` path.
- `app/src/terminal/view.rs:16864` — `TerminalView::end_text_selection` reads `selection_to_string`, clears empty selections, optionally copies, and clears block selections.
- `app/src/terminal/view.rs:17297` — `maybe_copy_selection_to_clipboard` uses `TerminalModel::selection_to_string` as the source of truth for copy-on-select.
- `app/src/terminal/view.rs:24744` — `TerminalView::action_accessibility_contents` emits selected text as an accessibility announcement for `BlockTextSelect`.

Current macOS bridge behavior explains the bug: when Speak Selection asks the `WarpHostView` text area for accessibility text, Warp provides a broad `accessibilityValue` transcript but does not provide the native selected text/range attributes. Because `accessibilityNumberOfCharacters` is `0` and selected-text methods are absent, macOS can fall back to reading from the text area's value rather than the active highlighted range.

## Proposed changes

### 1. Introduce selected-text accessibility data in warpui_core
Extend the cross-platform accessibility data model so a focused view can expose both broad readable content and an optional selected-text snapshot:

- Add `selected_text: Option<String>` to `AccessibilityData`, or introduce a sibling `AccessibilityTextData` struct that contains:
  - `content: String`
  - `selected_text: Option<String>`
  - optionally `selected_range: Option<Range<usize>>` if the native bridge needs a range into `content`.

Keep the default `View::accessibility_data` hook returning `None` so non-participating views do not change behavior. If `selected_range` is added, document that it is byte/character-indexed only after converting to the exact string returned to the platform; macOS `NSRange` must use UTF-16 code units, so the platform bridge should not treat Rust byte indices as native character offsets.

Tradeoff:
- A minimal implementation could add a separate `focused_view_accessibility_selected_text` callback only for macOS. Prefer extending `AccessibilityData` because the selected-text concept belongs with the existing focused-view accessibility payload, and future platforms or text surfaces can use the same hook.

### 2. Make TerminalView expose the active terminal selection
Update `TerminalView::accessibility_data` so it includes `selected_text` when the focused terminal has a non-empty active text selection:

1. Compute the existing `content` exactly as today for the fallback accessibility value.
2. Before returning, get `SemanticSelection::as_ref(ctx)` and call:
   - `model.selection_to_string(semantic_selection, self.is_inverted_blocklist(ctx), ctx)` for block-list or alt-screen selection.
   - Use the existing alt-screen delegation in `TerminalModel::selection_to_string` rather than adding a parallel alt-screen path.
3. Filter out `None` and empty strings.
4. Do not return selected text for block selection unless there is also an active text selection in the model.
5. Respect existing secret-obfuscation behavior by reusing `selection_to_string` and `bounds_to_string` paths rather than reading grid storage directly.
6. Avoid stale selections by relying on existing selection clearing:
   - `BlockList::clear_selection` emits `TextSelectionChanged`.
   - `TerminalView::end_text_selection` clears empty text selections.
   - alt-screen selection is cleared on relevant terminal-mode transitions.

This keeps copy, AI context, selection announcements, and Speak Selection aligned on the same selected-text source.

### 3. Add native macOS selected-text accessibility methods
Update `WarpHostView` in `crates/warpui/src/platform/mac/objc/host_view.m` to expose selected text to AppKit:

- Implement `-accessibilitySelectedText` to return a new Rust FFI callback such as `warp_get_accessibility_selected_text(self)`.
- Implement `-accessibilitySelectedTextRange` to return the selected range when available.
  - If the bridge only exposes a selected-text snapshot and no range into `accessibilityValue`, return `NSMakeRange(NSNotFound, 0)` or omit the method if testing shows Speak Selection only needs `accessibilitySelectedText`.
  - If a range is exposed, compute it in UTF-16 code units against the string returned by `accessibilityValue`.
- Update `-accessibilityNumberOfCharacters` to return the UTF-16 length of `accessibilityValue` when no selected-text range is available, or the actual content length if range APIs are implemented.
- Consider implementing `-accessibilityStringForRange:` if Speak Selection on supported macOS versions asks for the selected range first and then asks for the string at that range.

Add corresponding Rust FFI in `crates/warpui/src/platform/mac/window.rs`:

- `warp_get_accessibility_selected_text(object: &mut Object) -> id`
  - Resolve the window id from `get_window_state`.
  - Call into the app context, using the same focused responder chain as `warp_get_accessibility_contents`.
  - Return an empty/nil-equivalent string when no selected text exists.
- If range support is added, return a small C-compatible range representation or expose a helper that Obj-C can call for location/length.

The Obj-C implementation should not cache selected text itself. It should query Rust each time so the answer reflects focus changes, selection clearing, and terminal updates.

### 4. Preserve existing announcement behavior
Do not replace `set_accessibility_contents` or `ActionAccessibilityContent` behavior. Those APIs produce VoiceOver-style announcements after actions. Speak Selection should instead be served through the host view's text accessibility attributes.

Keep `BlockTextSelect` action accessibility in `TerminalView::action_accessibility_contents` unless testing shows it creates duplicate speech when VoiceOver and Speak Selection are both active. If duplicate speech occurs only with VoiceOver enabled, gate announcement changes narrowly and document the VoiceOver behavior.

### 5. Add tests around selected accessibility data
Add Rust unit tests close to the selection model and terminal view where possible:

- `app/src/terminal/model/blocks/selection_tests.rs`
  - Existing tests already cover many selection expansion behaviors. Add assertions for selections that should become the `selected_text` snapshot if a helper is introduced.
- `app/src/terminal/model/alt_screen_tests.rs`
  - Add or reuse coverage proving alt-screen `selection_to_string` returns the selected text required by Speak Selection.
- `app/src/terminal/view_tests.rs`
  - Add a terminal view accessibility-data test if fixtures can construct a focused terminal with selected text. Assert:
    - `content` remains the same fallback transcript shape.
    - `selected_text` is `Some(...)` for a non-empty text selection.
    - `selected_text` is `None` for no selection and for empty selections.
    - block selection alone does not populate `selected_text`.
- `crates/warpui_core` test
  - Add a small test for any new `AccessibilityData` helper or default construction.

For the native macOS bridge:
- If Obj-C accessibility methods are covered by an existing macOS unit/integration test target, add tests for `accessibilitySelectedText`, no-selection fallback, and UTF-16 range length with non-ASCII text.
- If no automated native test target exists, add a compact Rust-side test for the FFI string helper and require manual macOS validation.

## End-to-end flow
1. User highlights terminal text.
2. Existing terminal selection state updates in `BlockList` or alt screen.
3. macOS Speak Selection invokes accessibility APIs on `WarpHostView`.
4. `WarpHostView` asks Rust for the focused view's selected accessibility text.
5. `AppContext::focused_view_accessibility_data` reaches the focused `TerminalView`.
6. `TerminalView::accessibility_data` returns the current terminal transcript plus the active selected-text snapshot.
7. `WarpHostView` returns the snapshot through selected-text accessibility methods.
8. macOS speaks the snapshot instead of starting from the top of `accessibilityValue`.

## Testing and validation
Behavior-to-verification mapping:
- Product behavior 2, 5, 6: manual macOS Speak Selection validation on selected output and selected prompt/command text.
- Product behavior 3, 7, 9, 10: unit tests for regular, reversed, multi-block, rectangular, word, and line selections using the existing selection model.
- Product behavior 8: unit/manual coverage for alt-screen selection.
- Product behavior 11: selected-text test containing wide characters and emoji; native bridge validation should account for UTF-16 lengths if ranges are implemented.
- Product behavior 13: test or manual validation with an obfuscated secret-like selection to verify the selected text matches copy behavior.
- Product behavior 15, 16, 18: tests for no selection, block selection only, and cleared selection returning no selected-text snapshot.
- Product behavior 20: manual validation with VoiceOver off and then on.

Suggested manual validation on macOS:
1. Enable Speak Selection and assign `Option+Esc`.
2. Run a command that prints several lines.
3. Select a word in the middle of the output and press `Option+Esc`; confirm speech starts at that word.
4. Select a multi-line output range and confirm speech stops at the selected range.
5. Select text in the command/prompt area and confirm only that text is spoken.
6. Open an alternate-screen app, select visible text, and confirm only that selection is spoken.
7. Clear the selection and press `Option+Esc`; confirm Warp does not speak the previous selection.
8. Repeat with VoiceOver enabled and confirm existing focus/selection announcements still work.

Repository validation:
- Run `cargo fmt`.
- Run the smallest relevant Rust test targets added or changed for:
  - `app/src/terminal/model/blocks/selection_tests.rs`
  - `app/src/terminal/model/alt_screen_tests.rs`
  - `app/src/terminal/view_tests.rs` if terminal view accessibility coverage is added
  - any `crates/warpui_core` accessibility data tests
- If native macOS tests are added, run the macOS-specific test target in a macOS environment.

## Parallelization
Parallel implementation is useful but should be limited because the core behavior depends on one shared accessibility data contract.

- Agent `core-selected-text`
  - Execution mode: local or remote macOS-capable environment; local is preferable if the implementer needs to run macOS-specific tests.
  - Worktree/branch: `../warp-GH10954-core` on branch `oz-agent/GH10954-core-selected-text`.
  - Owns `crates/warpui_core/src/core/view/mod.rs`, `crates/warpui_core/src/core/app.rs`, `app/src/terminal/view.rs`, and Rust unit tests for selected accessibility data.
- Agent `mac-bridge`
  - Execution mode: macOS-capable local environment; required for validating `NSAccessibility` behavior.
  - Worktree/branch: `../warp-GH10954-mac-bridge` on branch `oz-agent/GH10954-mac-bridge`.
  - Owns `crates/warpui/src/platform/mac/objc/host_view.m`, `crates/warpui/src/platform/mac/window.rs`, and any macOS bridge tests.

Merge strategy: one combined implementation PR after merging or cherry-picking both branches into a final branch. The `core-selected-text` agent should land the Rust data contract first; `mac-bridge` should consume that contract and should not invent a separate terminal-selection path. Coordination boundary is the `AccessibilityData` shape and any FFI helper names.

Sequential dependency:
1. Agree on the `AccessibilityData` selected-text/range shape.
2. Implement terminal selected-text population.
3. Implement the macOS native bridge.
4. Run unit tests and macOS manual validation.

## Risks and mitigations
- Risk: macOS Speak Selection requires selected range APIs rather than only selected text. Mitigation: test on supported macOS versions and add `accessibilitySelectedTextRange` plus `accessibilityStringForRange:` if needed.
- Risk: Rust byte offsets are accidentally exposed as macOS `NSRange` offsets. Mitigation: compute native ranges in UTF-16 code units or avoid range APIs until they can be represented correctly.
- Risk: selected text becomes stale after focus or selection changes. Mitigation: query Rust live for each accessibility request and reuse existing selection-clearing paths.
- Risk: fixing Speak Selection changes VoiceOver announcements. Mitigation: keep action announcements separate from selected-text attributes and manually verify with VoiceOver enabled.
- Risk: accessibility content leaks secrets. Mitigation: do not read raw grid storage; reuse `selection_to_string` / `bounds_to_string` paths that already enforce selection copy semantics.

## Follow-ups
- If non-terminal Warp text surfaces also fail macOS Speak Selection, extend the same `AccessibilityData` selected-text hook to those views in separate issues.
- If manual validation shows macOS needs precise selection geometry, add `accessibilityFrameForRange:` using existing rendered selection bounds, but keep that out of the first fix unless required for Speak Selection correctness.
