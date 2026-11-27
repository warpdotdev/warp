# Technical Spec: Forward macOS editing shortcuts to Kitty keyboard protocol TUI apps

See `specs/GH9159/product.md` for the product spec.

**Issue:** [warpdotdev/warp#9159](https://github.com/warpdotdev/warp/issues/9159)

## Context

Warp already has the major pieces needed for Kitty keyboard protocol support, but the current encoder leaves gaps for macOS editing shortcuts that combine Cmd/Super or Option/Alt with non-printing keys.

Relevant code inspected at commit `62da4ee72156ac5a8c5952cbb4486b0b426da204`:

- [`crates/warpui/src/platform/mac/event.rs (20-28) @ 62da4ee`](https://github.com/warpdotdev/warp/blob/62da4ee72156ac5a8c5952cbb4486b0b426da204/crates/warpui/src/platform/mac/event.rs#L20-L28) — macOS converts native modifier flags into Warp's `ModifiersState`, including `alt`, `cmd`, `shift`, `ctrl`, and `func`.
- [`crates/warpui/src/platform/mac/event.rs (57-117) @ 62da4ee`](https://github.com/warpdotdev/warp/blob/62da4ee72156ac5a8c5952cbb4486b0b426da204/crates/warpui/src/platform/mac/event.rs#L57-L117) — macOS `KeyDown` events create a `Keystroke` with `alt` and `cmd` bits plus `KeyEventDetails::key_without_modifiers`.
- [`app/src/terminal/block_list_element.rs (4559-4595) @ 62da4ee`](https://github.com/warpdotdev/warp/blob/62da4ee72156ac5a8c5952cbb4486b0b426da204/app/src/terminal/block_list_element.rs#L4559-L4595) — the block-list terminal surface converts focused `KeyDown` events through `KeystrokeWithDetails::to_escape_sequence` before dispatching `TerminalAction::ControlSequence`.
- [`app/src/terminal/alt_screen/alt_screen_element.rs (858-875) @ 62da4ee`](https://github.com/warpdotdev/warp/blob/62da4ee72156ac5a8c5952cbb4486b0b426da204/app/src/terminal/alt_screen/alt_screen_element.rs#L858-L875) — the alternate-screen surface uses the same `KeystrokeWithDetails::to_escape_sequence` path.
- [`app/src/terminal/block_list_element.rs (4647-4660) @ 62da4ee`](https://github.com/warpdotdev/warp/blob/62da4ee72156ac5a8c5952cbb4486b0b426da204/app/src/terminal/block_list_element.rs#L4647-L4660) and [`app/src/terminal/alt_screen/alt_screen_element.rs (963-976) @ 62da4ee`](https://github.com/warpdotdev/warp/blob/62da4ee72156ac5a8c5952cbb4486b0b426da204/app/src/terminal/alt_screen/alt_screen_element.rs#L963-L976) — standalone modifier key press/release events are already routed through `maybe_kitty_keyboard_escape_sequence` when the active mode requires it.
- [`crates/warp_terminal/src/model/escape_sequences.rs (222-253) @ 62da4ee`](https://github.com/warpdotdev/warp/blob/62da4ee72156ac5a8c5952cbb4486b0b426da204/crates/warp_terminal/src/model/escape_sequences.rs#L222-L253) — `KeystrokeWithDetails::to_escape_sequence` first asks the Kitty keyboard protocol encoder for a CSI-u sequence, then falls back to legacy function-key, C0, cursor, Meta, and Backspace encoders.
- [`crates/warp_terminal/src/model/escape_sequences/kitty_keyboard_protocol.rs (13-58) @ 62da4ee`](https://github.com/warpdotdev/warp/blob/62da4ee72156ac5a8c5952cbb4486b0b426da204/crates/warp_terminal/src/model/escape_sequences/kitty_keyboard_protocol.rs#L13-L58) — `maybe_convert_keystroke_to_csi_u` only treats Escape, Ctrl, Meta, non-macOS Alt, and Shift on Enter/Tab/Backspace as ambiguous in disambiguate mode. Cmd/Super is not considered ambiguous, and macOS Option is excluded even for non-printing keys.
- [`crates/warp_terminal/src/model/escape_sequences/kitty_keyboard_protocol.rs (84-221) @ 62da4ee`](https://github.com/warpdotdev/warp/blob/62da4ee72156ac5a8c5952cbb4486b0b426da204/crates/warp_terminal/src/model/escape_sequences/kitty_keyboard_protocol.rs#L84-L221) — `keystroke_to_csi_u` maps Enter, Tab, Escape, Backspace, Space, printable characters, and F13-F35. It does not encode arrows, forward Delete, Insert, PageUp/PageDown, Home/End, or F1-F12 using Kitty's functional-key forms.
- [`crates/warp_terminal/src/model/escape_sequences.rs (500-609) @ 62da4ee`](https://github.com/warpdotdev/warp/blob/62da4ee72156ac5a8c5952cbb4486b0b426da204/crates/warp_terminal/src/model/escape_sequences.rs#L500-L609) — legacy cursor and Backspace fallbacks do not account for Cmd/Super. `backspace_keystroke_to_escape_sequence` intentionally returns `None` when Cmd, Alt, Meta, or Ctrl are present.
- [`crates/warp_terminal/src/model/mode.rs (29-74) @ 62da4ee`](https://github.com/warpdotdev/warp/blob/62da4ee72156ac5a8c5952cbb4486b0b426da204/crates/warp_terminal/src/model/mode.rs#L29-L74) — `TermMode` stores the Kitty keyboard protocol enhancement flags.
- [`crates/warp_terminal/src/model/mode.rs (83-115) @ 62da4ee`](https://github.com/warpdotdev/warp/blob/62da4ee72156ac5a8c5952cbb4486b0b426da204/crates/warp_terminal/src/model/mode.rs#L83-L115) — `KeyboardModes` maps Kitty's progressive enhancement flags into `TermMode`.
- [`app/src/terminal/model/ansi/mod.rs (1459-1487) @ 62da4ee`](https://github.com/warpdotdev/warp/blob/62da4ee72156ac5a8c5952cbb4486b0b426da204/app/src/terminal/model/ansi/mod.rs#L1459-L1487) — the ANSI parser recognizes `CSI = flags ; mode u`, `CSI > flags u`, `CSI < count u`, and `CSI ? u` for Kitty keyboard protocol mode management on non-Windows platforms.
- [`app/src/terminal/model/grid/grid_handler.rs (2221-2270) @ 62da4ee`](https://github.com/warpdotdev/warp/blob/62da4ee72156ac5a8c5952cbb4486b0b426da204/app/src/terminal/model/grid/grid_handler.rs#L2221-L2270) — `GridHandler` stores active keyboard modes and syncs them into `TermMode::KEYBOARD_PROTOCOL`.
- [`app/src/terminal/model/grid/ansi_handler.rs (1423-1458) @ 62da4ee`](https://github.com/warpdotdev/warp/blob/62da4ee72156ac5a8c5952cbb4486b0b426da204/app/src/terminal/model/grid/ansi_handler.rs#L1423-L1458) — the Kitty keyboard protocol feature flag gates setting, pushing, popping, and querying keyboard enhancement flags.

The likely failure mode is therefore centralized in `crates/warp_terminal/src/model/escape_sequences/kitty_keyboard_protocol.rs`: when a TUI enables only the common disambiguate flag (`CSI > 1 u`), Cmd+Backspace, Cmd+Arrow, Option+Backspace, and Option+Arrow do not qualify for CSI-u encoding; the legacy fallback either cannot represent Cmd/Super or refuses modified Backspace, so the terminal surface has no bytes to send.

## Proposed changes

### 1. Introduce a Kitty functional-key encoding table

Add a small internal representation in `crates/warp_terminal/src/model/escape_sequences/kitty_keyboard_protocol.rs` for Kitty functional-key encodings:

- Backspace: key code `127`, final `u`.
- Forward Delete: key code `3`, final `~`.
- Insert: key code `2`, final `~`.
- Left/Right/Up/Down arrows: key code `1`, finals `D`, `C`, `A`, `B`.
- Home/End: key code `1`, finals `H`, `F`.
- PageUp/PageDown: key codes `5`, `6`, final `~`.
- F1/F2/F4: key code `1`, finals `P`, `Q`, `S`.
- F3: key code `13`, final `~`.
- F5-F12: existing tilde key codes `15`, `17`, `18`, `19`, `20`, `21`, `23`, `24`.
- F13-F35: keep using existing private-use CSI-u codes.

The encoder should build the protocol-correct form for the final byte:

- `u` form: `CSI key_code[;modifier]u`, preserving the current omission of `;1` when there are no modifiers and no associated text.
- letter final form: `CSI final` for no modifiers, `CSI 1;modifier final` when modified.
- tilde final form: `CSI key_code~` for no modifiers, `CSI key_code;modifier~` when modified.

This makes the issue's expected Backspace sequences fall out naturally:

- Cmd+Backspace/Delete-left: `ESC[127;9u` (`1 + Super`).
- Option+Backspace/Delete-left: `ESC[127;3u` (`1 + Alt`).

It also produces standard functional-key forms for arrows:

- Cmd+Left: `ESC[1;9D`.
- Cmd+Right: `ESC[1;9C`.
- Cmd+Up: `ESC[1;9A`.
- Cmd+Down: `ESC[1;9B`.
- Option+Left/Right/Up/Down: `ESC[1;3D`, `ESC[1;3C`, `ESC[1;3A`, `ESC[1;3B`.

### 2. Broaden disambiguate-mode eligibility for non-printing keys

Update `maybe_convert_keystroke_to_csi_u` so `TermMode::KEYBOARD_DISAMBIGUATE_ESCAPE` uses enhanced encoding for any key whose legacy encoding would lose meaningful modifier information. The intended rule:

1. Escape remains ambiguous.
2. Ctrl and Meta remain ambiguous.
3. Cmd/Super is ambiguous for all keys that can be encoded by the Kitty keyboard protocol.
4. Shift is ambiguous for non-printing keys where legacy encoding would drop Shift.
5. Alt/Option is ambiguous for non-printing functional/editing keys on macOS, including arrows, Backspace/Delete-left, and forward Delete.
6. Alt/Option on macOS remains excluded for printable characters that use the OS composition/dead-key path, unless the user's Option-as-Meta path sets `keystroke.meta` or `REPORT_ALL_KEYS_AS_ESC` is active.

This keeps the product boundary intact: Option+letter dead keys should not be forced into terminal Alt, while Option+Arrow and Option+Backspace should be forwardable because they are non-printing editing/navigation keys.

### 3. Keep modifier mapping centralized and unchanged

The existing modifier calculation in `keystroke_to_csi_u` already maps:

- `keystroke.alt || keystroke.meta` to Kitty Alt bit (`+2`).
- `keystroke.ctrl` to Kitty Ctrl bit (`+4`).
- `keystroke.cmd` to Kitty Super bit (`+8`).

Keep this mapping, but reuse it for the new functional-key forms. Do not add app-specific translation from Cmd+Arrow to Home/End or Option+Delete to Ctrl+W; Kitty-aware applications should receive the actual modified key event and decide semantics themselves.

### 4. Preserve legacy fallback behavior

Do not change `cursor_movement_keystroke_to_escape_sequence`, `meta_keystroke_to_escape_sequence`, or `backspace_keystroke_to_escape_sequence` for non-Kitty modes except as needed to share helper code safely. In particular:

- `test_shift_backspace_emits_del_sequence` must remain valid.
- Existing xterm-style modified cursor sequences remain valid when Kitty keyboard protocol is not active.
- Existing macOS Option-as-Meta behavior for legacy input remains valid.

### 5. Rely on the existing terminal surfaces

No separate app-level handling should be needed in `block_list_element.rs` or `alt_screen_element.rs`. Both already use `KeystrokeWithDetails::to_escape_sequence`, so centralizing the fix in `crates/warp_terminal` makes the block-list and alternate-screen paths consistent.

Only add surface-level tests if unit tests expose a dispatch-specific gap, such as a `KeyDown` with empty `chars` not being handled after an escape sequence is generated.

## End-to-end flow

1. A TUI app emits `CSI > 1 u` or `CSI = 1 ; 1 u` to enable Kitty disambiguate mode.
2. Warp's ANSI parser handles the sequence and updates the active grid handler's keyboard mode.
3. The user presses Cmd+Delete, Option+Delete, Cmd+Arrow, or Option+Arrow while the terminal surface is focused.
4. macOS event conversion produces a `Keystroke` carrying `cmd` or `alt`, plus the key name (`backspace`, `delete`, `left`, `right`, `up`, or `down`).
5. The focused terminal surface calls `KeystrokeWithDetails::to_escape_sequence`.
6. The Kitty keyboard protocol encoder sees active keyboard protocol mode, recognizes the modified non-printing key as ambiguous, and emits a Kitty-compatible sequence.
7. `TerminalAction::ControlSequence` flows into `TerminalView::control_sequence_on_terminal`, which writes bytes to the PTY for an active long-running or alternate-screen command.
8. The TUI app receives the key event and performs its own editing behavior.

## Testing and validation

1. **Unit tests in `crates/warp_terminal/src/model/escape_sequences_tests.rs` for product invariants 1, 2, 3, 13, and 14.**
   - Add disambiguate-mode cases for:
     - `cmd-backspace` -> `b"\x1b[127;9u"`.
     - `alt-backspace` or `meta-backspace` on macOS Option-as-Meta paths -> `b"\x1b[127;3u"` where applicable.
     - `cmd-left`, `cmd-right`, `cmd-up`, `cmd-down` -> `b"\x1b[1;9D"`, `b"\x1b[1;9C"`, `b"\x1b[1;9A"`, `b"\x1b[1;9B"`.
     - `alt-left`, `alt-right`, `alt-up`, `alt-down` -> `b"\x1b[1;3D"`, `b"\x1b[1;3C"`, `b"\x1b[1;3A"`, `b"\x1b[1;3B"`.
     - `cmd-delete` for forward Delete, if Warp's key naming uses `delete`, -> `b"\x1b[3;9~"`.
     - `alt-delete` for forward Delete -> `b"\x1b[3;3~"`.
   - Add or update tests for `KEYBOARD_REPORT_ALL_AS_ESCAPE` to confirm the same keys still encode there.

2. **Regression tests for product invariants 8, 9, 10, and 15.**
   - Keep `test_shift_backspace_emits_del_sequence` unchanged for non-Kitty legacy mode.
   - Keep existing cursor movement tests unchanged for non-Kitty mode.
   - Keep `test_keyboard_enhancement_mac_option_without_meta_mapping_is_not_disambiguated` for printable Option characters, and add a neighboring test showing that Option+Arrow/Option+Backspace are still encoded because they are non-printing keys.

3. **Optional surface dispatch tests if needed.**
   - If the encoder tests pass but manual testing shows no bytes are written for Cmd+Backspace or Cmd+Arrow, add focused tests around `BlockListElement` and `AltScreenElement` to prove `KeyDown` events with empty `chars` still dispatch `TerminalAction::ControlSequence` when the encoder returns bytes.

4. **Manual macOS validation for product invariants 5, 6, and 7.**
   - Run Claude Code with the fullscreen/no-flicker renderer enabled and verify Cmd+Left/Right, Cmd+Delete, Option+Left/Right, and Option+Delete are no longer dropped.
   - Repeat inside tmux if tmux is configured to pass through Kitty keyboard protocol input.
   - Repeat in an SSH or remote-harness session where the inner application enables the protocol.
   - Compare the received bytes against Ghostty or Kitty using a key-event inspector such as `kitten show-key -m kitty` or a small test program that prints raw bytes.

5. **Repository validation.**
   - Run targeted Rust tests for `warp_terminal` escape sequences.
   - Run the repository's usual formatting and presubmit checks for any implementation PR.

## Parallelization

Parallel sub-agents are not proposed for implementation. The behavioral bug is concentrated in the shared escape-sequence encoder, and the most important tests live next to that code. Splitting implementation across multiple agents would add coordination overhead without meaningfully reducing wall-clock time. A separate manual-validation pass on macOS can run after the central encoder tests pass.

## Risks and mitigations

1. **macOS Option dead-key regressions.** Treat only non-printing Option-modified keys as ambiguous in disambiguate mode; leave printable Option characters on the existing composition path unless Option-as-Meta or all-keys mode explicitly asks otherwise.

2. **Incorrect Kitty functional-key form.** Use a table based on the Kitty functional-key definitions instead of ad hoc string formatting. Tests should assert exact bytes for arrows, Backspace, forward Delete, Home/End, and Page keys.

3. **Cmd/Super conflicts with Warp shortcuts.** This change only affects key events that reach the terminal surface. Warp-owned keybindings should continue to intercept before terminal forwarding when they are intentionally bound.

4. **Apps differ in semantic handling.** Some apps may map Cmd+Arrow differently or not at all. The implementation should only guarantee that Warp forwards a protocol-correct key event.

5. **Windows ConPTY behavior.** The existing ANSI parser disables Kitty keyboard protocol mode management on Windows. Do not broaden this fix to Windows without a separate ConPTY design.

6. **Remote and tmux passthrough variability.** Warp can write the correct bytes, but multiplexers and remote programs may require their own protocol passthrough support. Manual validation should distinguish "Warp sent wrong bytes" from "inner program did not pass through or bind the event."

## Follow-ups

- Consider adding a small raw-key diagnostics command or developer-only helper for comparing Warp's emitted Kitty sequences against reference terminals.
- Audit other Kitty keyboard protocol gaps after this fix, especially keypad keys, F1-F12 with enhancement flags, and event-type reporting for repeat/release beyond standalone modifier keys.
