# CODE-1793 — CLI coding agent paste being mangled by Warp
Linear: https://linear.app/warpdotdev/issue/CODE-1793/claude-code-native-image-paste-being-bypassed
## Context
When a CLI coding agent like Claude Code runs as a long-running command in a Warp terminal, Warp intercepts Ctrl+V (and the platform paste action) and converts the clipboard to a shell-escaped text paste that is sent to the PTY. Two different user flows break on Windows as a result:
1. **Raw image data in the clipboard (e.g. screenshot from `Win+Shift+S` / Snipping Tool).** The Windows clipboard has only `CF_DIB`, no `CF_HDROP`. `arboard`'s `file_list()` returns no paths, `plain_text` is empty, and Warp's text-paste path sends nothing. Claude Code's native image-paste handler never gets a chance to run and the user sees nothing happen.
2. **Image file copied from Explorer.** The clipboard has a `CF_HDROP` path. Warp reads the path, shell-escapes it via `ShellFamily::escape`, and writes it as text. PowerShell-family escaping uses backtick escapes, which the CLI agent's path-detection does not recognize; Windows Terminal by contrast pastes the path verbatim and the agent's path-detection attaches the image correctly.
CLI coding agents have their own native clipboard-image paste. The keystroke for raw image data differs per platform: `Ctrl+V` on macOS and Linux, `Alt+V` on Windows (see [anthropics/claude-code#18590](https://github.com/anthropics/claude-code/issues/18590)). For image *paths* pasted as text, agents do their own path-detection on the pasted text and load the file from disk — as long as the path is verbatim.
Relevant code:
- `app/src/terminal/view.rs:14156` — `TerminalView::paste`. When the input box isn't focused/visible (true during a long-running command like `claude`/`codex`/`opencode`), this reads the clipboard as text and writes it to the PTY, optionally wrapping it in bracketed paste.
- `app/src/terminal/view.rs:7601` — `TerminalView::read_from_clipboard` → `clipboard_content_with_escaped_paths` at `app/src/util/clipboard.rs:8`. Converts `ClipboardContent.paths` into a shell-escaped space-joined string; falls back to `plain_text` when there are no paths. Image data on `ClipboardContent.images` is ignored on this path.
- `crates/warpui/src/windowing/winit/windows/clipboard.rs:36` — Windows `read()`. `arboard`'s `file_list()` only returns paths when the clipboard carries `CF_HDROP`; screenshot-tool captures do not.
- `crates/warp_util/src/path.rs:218` — `ShellFamily::escape`. Uses backtick escapes for PowerShell; the escaped string isn't the form CLI agents recognize as an image path.
- `app/src/terminal/cli_agent_sessions/mod.rs:296` — `CLIAgentSessionsModel::session(view_id)` gives the active CLI agent (if any) for a terminal.
- `app/src/terminal/cli_agent.rs:108` — `CLIAgent` enum.
Why the existing paste path can't just "pass-through" Ctrl+V: the `terminal:paste` / Windows `ctrl-v` bindings at `app/src/terminal/view/init.rs` intercept the keystroke and dispatch `TerminalAction::Paste`. If `paste()` returns without writing anything, the agent never sees the keystroke at all.
## Implemented changes
Two changes to `TerminalView::paste` in `app/src/terminal/view.rs`, gated on a new `active_cli_agent_handles_image_paste_natively(ctx)` helper that returns `true` whenever `CLIAgentSessionsModel::as_ref(ctx).session(self.view_id).is_some()` — i.e. any active CLI agent session on this terminal. The paste target must also be the PTY (`!should_paste_in_input`) and the event must not be a middle-click (`!middle_click`), since middle-click is an X11/Linux text-paste convention.
### 1. Forward the native paste keystroke for raw clipboard image data
Before the existing text-paste logic, if we're in a CLI-agent paste and `ctx.clipboard().read().has_image_data()` is true, write the platform-appropriate keystroke the CLI agent expects and return early:
- Windows: `ESC 'v'` (`[0x1b, b'v']`) — `Alt+V`.
- macOS / Linux: `[0x16]` — `Ctrl+V` (SYN).
This intentionally fires only when raw image data is present. A clipboard that only has file paths (Explorer copy) falls through to #2 — Claude Code's native `Alt+V` handler only reads raw image bytes and errors out ("No image found in clipboard. Use alt+v to paste images.") if we hand it a path-only clipboard.
### 2. Skip shell-escaping on pasted file paths in CLI-agent pastes
`TerminalView::read_from_clipboard` and `TerminalView::middle_click_paste_content` now take `Option<ShellFamily>` instead of `ShellFamily`. `paste()` passes `None` when `is_cli_agent_paste` is true; otherwise it passes `Some(self.shell_family(ctx))` as before. `clipboard_content_with_escaped_paths` already handled `None` by returning paths verbatim, so no changes were needed there.
The net effect: a path like `C:\Users\andy\Pictures\screenshot.png` is sent to the agent exactly as Windows Terminal would paste it — the agent's file-path detection recognizes it and attaches the image. No PowerShell backtick escaping is applied.
### Scope
`active_cli_agent_handles_image_paste_natively` returns `true` for *any* active CLI agent session, including `CLIAgent::Unknown` (user-configured regex matches). The bar to register a session at all is that Warp's CLI-agent detection matched the command; once matched, the coding-agent contract (verbatim paths, optional native image paste) applies uniformly. No per-agent allowlist is maintained.
Everything else (regular text pastes, pastes into Warp's input editor, middle-click, plain shells without an active CLI agent session) continues through the existing `read_from_clipboard` → bracketed-paste path with shell-escaping unchanged.
A feature flag isn't warranted: the change is scoped by the active CLI agent session and reverts the hijack to faithful pass-through behavior, which is strictly closer to what the agent would see running under a plain terminal emulator.
## Testing and validation
Manual verification on Windows (primary platform for the bug):
1. Run `claude` in a Warp terminal until the Claude Code TUI is active.
2. Capture a screenshot with `Win+Shift+S`. Press `Ctrl+V` in Warp. Claude Code should show the `[Image #N]` attachment chip (it receives `Alt+V` and reads the clipboard itself). Previously: nothing happened.
3. In Explorer, copy an image file (`.png`). Press `Ctrl+V`. Claude Code should attach the image via its path-detection on the unescaped path. Previously: PowerShell-escaped path was pasted as text and the agent didn't recognize it.
4. Repeat 2–3 with `codex` and `opencode` — unescaped file-path paste should attach the image for both. (Raw image data via Alt+V is Claude-specific; Codex/OpenCode paths are the primary case for those agents.)
5. Copy plain text. Press `Ctrl+V`. Text should paste as before; the raw-image early return does not fire and `clipboard_content_with_escaped_paths(..., None)` returns `plain_text` unchanged.
6. Outside any CLI agent (plain `pwsh`), copy a screenshot and Ctrl+V. Behavior is unchanged from today (no CLI agent session → helper returns `false` → shell-escape still applied).
Cross-platform regression checks:
- macOS: `claude` with a screenshot in the clipboard + `Ctrl+V` still attaches the image. On macOS, `Cmd+V` in Warp dispatches the same `TerminalAction::Paste`; the keystroke branch writes `0x16` which matches what macOS Claude Code expects.
- Linux: same as macOS with `Ctrl+V`.
- Middle-click paste on Linux still inserts text (early branch skipped because `middle_click` is true, which also means shell escaping is still applied on that path).
- Pasting into Warp's own input editor (Agent Mode, rich input) is unaffected because `should_paste_in_input` short-circuits before the new branches.
Automated:
- `cargo check -p warp --lib`.
- Existing paste tests in `app/src/terminal/view_test.rs` and `app/src/terminal/input_test.rs` continue to pass — the test helper `read_from_clipboard(ctx)` was updated to pass `Some(ShellFamily::Posix)` to match the new signature, and the new branches only fire when a CLI agent session is active, which those tests don't set up.
## Risks and mitigations
- **Sending `Alt+V` / `Ctrl+V` bytes to a non-Claude TUI that happens to be detected as a CLI agent session.** Only fires when the clipboard actually has raw image data (`has_image_data()` is true), so for normal text/path pastes this branch never runs. Agents that don't handle the keystroke will simply ignore the byte.
- **Future CLI agent updates change the paste keystroke.** The mapping is one `cfg!(windows)` branch inside `paste()`; updating it is a one-line change.
- **Clipboard with both image data and text.** If image data is present we forward the keystroke and don't paste the text. This matches the pre-existing macOS behavior (where the path text was technically inserted but Claude Code attached the image and ignored the text) and is what users asking for native image paste expect.
- **User-defined `CLIAgent::Unknown` regex matches.** These are treated the same as known agents — verbatim paths, keystroke forwarding for raw images. The regex matching is opt-in (user adds the pattern) so the assumption that they want CLI-agent semantics is reasonable.
