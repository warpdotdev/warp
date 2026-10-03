# Product Spec: Forward macOS editing shortcuts to Kitty keyboard protocol TUI apps

**Issue:** [warpdotdev/warp#9159](https://github.com/warpdotdev/warp/issues/9159)
**Figma:** none provided

## Summary

When a terminal application running in Warp opts into the Kitty keyboard protocol, Warp should continue forwarding standard macOS editing shortcuts such as Cmd+Arrow, Cmd+Delete, Option+Arrow, and Option+Delete to the application. These shortcuts should behave inside Kitty-aware TUI prompts the same way they behave in comparable terminals such as Ghostty and Kitty, without regressing Warp's normal shell prompt or legacy terminal input behavior.

## Problem

Kitty keyboard protocol mode is increasingly common in fullscreen or alternate-screen TUI apps, including Claude Code's no-flicker renderer, tmux, Vim-like tools, and remote harnesses such as Pi. Users expect macOS editing shortcuts to keep working in those prompts. Today, once the app enables Kitty keyboard protocol input, Warp can silently drop or fail to encode several macOS editing shortcuts, so the app never receives a usable key event even though equivalent control sequences such as Ctrl+A, Ctrl+E, Ctrl+U, and Ctrl+W still work.

## Goals

- Preserve standard macOS text-editing muscle memory inside Kitty-aware TUI applications.
- Forward the affected shortcuts as protocol-correct key events rather than translating them into Warp-local editor actions.
- Keep legacy terminal input, Warp's normal shell prompt behavior, and non-Kitty applications unchanged.
- Cover both block-list long-running commands and alternate-screen/fullscreen TUI applications.
- Make behavior consistent enough that application authors can rely on Warp's Kitty keyboard protocol support for Cmd/Super and Option/Alt modified editing keys.

## Non-goals

- Changing how a TUI application interprets the key once Warp has forwarded it.
- Adding app-specific workarounds for Claude Code, tmux, Pi, Vim, or any one TUI framework.
- Changing Warp's global keybinding precedence for shortcuts intentionally owned by Warp.
- Changing the user's Option-as-Meta setting for printable Option characters and dead keys.
- Enabling Kitty keyboard protocol on Windows paths where Warp intentionally does not forward Kitty keyboard protocol responses.

## Behavior

1. When a foreground terminal application enables Kitty keyboard protocol mode, Warp forwards macOS editing shortcuts that are not claimed by Warp as key events to the foreground application instead of dropping them.

2. The core affected shortcut set includes:
   - Cmd+Left and Cmd+Right.
   - Cmd+Up and Cmd+Down.
   - Cmd+Delete, where Delete is the standard Mac backspace/delete-left key.
   - Option+Left and Option+Right.
   - Option+Up and Option+Down.
   - Option+Delete, where Delete is the standard Mac backspace/delete-left key.

3. If the user's keyboard includes a forward-delete key, or they press a platform shortcut that Warp receives as forward Delete rather than Backspace, Warp forwards the modified forward-delete key consistently with the same Kitty keyboard protocol rules.

4. Warp's responsibility is to forward the correct key event. The foreground application remains responsible for deciding whether Cmd+Left means line start, Cmd+Right means line end, Option+Left/Right means word movement, Cmd+Delete means delete line, Option+Delete means delete previous word, or any other app-specific action.

5. In Claude Code with the fullscreen/no-flicker renderer active, the prompt receives the forwarded shortcuts and can perform the same editing actions that work in Ghostty and Kitty:
   - Cmd+Left/Right moves to the beginning/end of the current prompt line when Claude Code maps those keys that way.
   - Option+Left/Right moves by word when Claude Code maps those keys that way.
   - Cmd+Delete and Option+Delete delete line/word content when Claude Code maps those keys that way.

6. In tmux and other TUI/multiplexer scenarios that opt into or pass through Kitty keyboard protocol input, Warp forwards the same shortcut events to the active terminal program. If tmux or the inner application is not configured to pass through or interpret the protocol, Warp still sends the correct bytes but does not attempt to compensate for the multiplexer configuration.

7. In remote sessions, including SSH-backed workflows and remote harnesses, the behavior is the same as local sessions from the user's perspective: the active application receives the encoded key event over the PTY stream whenever the active terminal state has enabled Kitty keyboard protocol mode.

8. In the normal Warp shell prompt and other states where Warp's own input editor owns text editing, the existing editor behavior remains unchanged. The fix should not cause Cmd+Left, Cmd+Right, Cmd+Delete, Option+Left, Option+Right, or Option+Delete to leak into the shell while the Warp input editor is supposed to handle them locally.

9. In legacy terminal mode, where no foreground application has enabled Kitty keyboard protocol, existing escape sequence behavior remains unchanged. Existing readline-compatible fallbacks such as Ctrl+A, Ctrl+E, Ctrl+U, Ctrl+W, plain arrows, and unmodified Backspace keep their current behavior.

10. Printable Option dead-key behavior on macOS remains unchanged. For example, Option+letter combinations that are intended to produce composed characters or dead keys continue to use the OS text input path unless the user's settings explicitly map Option to terminal Meta behavior or the active Kitty mode requests all keys as escape codes.

11. Warp keybindings and application-level shortcuts that intentionally take precedence continue to do so. This spec applies to key events that reach the terminal surface and would otherwise be forwarded to the active terminal program.

12. The feature is invisible when it works. There is no new setting, toast, banner, or visual state. Users should only observe that the shortcuts no longer do nothing inside Kitty-aware TUI prompts.

13. The fix applies consistently through both terminal render surfaces that can host active programs:
   - Block-list long-running command content.
   - Alternate-screen/fullscreen content.

14. The shortcut events should be encoded in a way that applications using standard Kitty keyboard protocol parsers can distinguish the macOS modifiers:
   - Cmd maps to the Kitty Super modifier.
   - Option maps to the Kitty Alt modifier for non-printing editing/navigation keys.
   - Backspace/Delete-left remains distinguishable from forward Delete.
   - Arrow direction remains distinguishable from Home/End/Page keys.

15. Regression boundaries:
   - Plain Backspace and Shift+Backspace still send the legacy DEL byte when Kitty keyboard protocol is not active.
   - Plain arrows still use existing cursor-key behavior when Kitty keyboard protocol is not active.
   - Ctrl-modified legacy control keys continue to work in both shell prompts and TUI prompts.

## Success Criteria

1. A Kitty-aware TUI app running in Warp receives usable events for Cmd+Arrow, Cmd+Delete, Option+Arrow, and Option+Delete.
2. Claude Code's fullscreen/no-flicker prompt no longer drops the shortcuts described in the issue.
3. Existing terminal input tests for legacy Backspace, cursor keys, Ctrl keys, and Option-as-Meta behavior continue to pass.
4. The fix is covered by unit tests that prove the relevant key combinations produce expected Kitty keyboard protocol sequences when the protocol is active.

## Validation

- **Automated:** Add terminal escape-sequence tests for the affected key combinations in Kitty keyboard protocol mode.
- **Automated:** Keep existing legacy escape-sequence tests passing, especially Shift+Backspace and cursor movement tests.
- **Manual:** Run a Kitty-aware prompt such as Claude Code fullscreen/no-flicker mode on macOS, type text, and verify Cmd+Arrow, Cmd+Delete, Option+Arrow, and Option+Delete are received and acted on by the app.
- **Manual:** Repeat in an alternate-screen or tmux-style workflow to verify the same forwarding path works outside the normal shell prompt.
