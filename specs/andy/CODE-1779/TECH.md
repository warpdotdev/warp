# TECH.md — CODE-1779: Drag-and-drop file paths in WSL (and Git Bash)

See `PRODUCT.md` for user-visible behavior.

## Context

There are two drop paths relevant to this ticket:

1. **Drop onto the terminal grid** (e.g. during a long-running command). Handled by `TerminalView::drag_and_drop_files` in `app/src/terminal/view.rs (23007-23085)`. This path already applies `warp_util::path::convert_windows_path_to_wsl` when `session.is_wsl()` before shell-escaping and writing to the PTY. For MSYS2 long-running it deliberately keeps Windows-native paths and skips shell escaping (so native Windows binaries receive the form they expect). **Not changing this path.**

2. **Drop onto the input editor** (typical case when there's no long-running command). The editor element's `drag_and_drop_file` in `app/src/editor/view/element.rs (663-681)` dispatches `EditorAction::DragAndDropFiles`, which routes to `EditorView::drag_and_drop_files` in `app/src/editor/view/mod.rs (7893-7914)`. That function emits `Event::DroppedImageFiles` for image paths (terminal input handles attachment) and, for everything else, calls `warpui::clipboard_utils::escaped_paths_str` followed by `self.user_insert`. **No path conversion happens — this is the bug for both WSL and MSYS2.**

Why the fix doesn't belong inside `EditorView`: `EditorView` is a general-purpose text editor used across dozens of surfaces (notebooks, settings, code editors, modals, etc. — see the many `EditorView::new(...)` / `single_line(...)` call sites). It already carries `shell_family` for escaping, which is a shell concern but not tied to any particular parent; WSL / MSYS2 path conversion is a terminal-session concern and must not leak into the editor.

Existing helpers:
- `warp_util::path::convert_windows_path_to_wsl` in `crates/warp_util/src/path.rs (673-691)`, with tests at `crates/warp_util/src/path_test.rs (629-649)`.
- **No equivalent exists yet for MSYS2.** We'll add `convert_windows_path_to_msys2` alongside it (same shape, `/<drive>/…` instead of `/mnt/<drive>/…`).
- `Session::is_wsl()` at `app/src/terminal/model/session.rs:972` and `Session::is_msys2()` at `app/src/terminal/model/session.rs:980` both already exist and return `false` on non-Windows platforms.
- `TerminalInput::active_session` is available in `app/src/terminal/input.rs:11011`.

Shell-family setup on the editor already happens in `TerminalInput::set_active_block_metadata` at `app/src/terminal/input.rs (13034-13065)`, which is called on every active-block (and therefore session) change. That's the natural place to install/clear the transformer too.

## Proposed changes

### 1. Add `convert_windows_path_to_msys2` to `warp_util::path`

In `crates/warp_util/src/path.rs`, alongside `convert_windows_path_to_wsl`, add:

```rust path=null start=null
/// Converts a Windows-native path string to an MSYS2 / Git Bash POSIX-style path.
///
/// Drive-letter paths (e.g. `C:\Users\aloke\file.txt`) are mapped to
/// `/<drive>/Users/aloke/file.txt`. Paths that don't start with a drive letter
/// are returned as-is with backslashes replaced by forward slashes.
pub fn convert_windows_path_to_msys2(windows_path: &str) -> String { /* ... */ }
```

Implementation mirrors `convert_windows_path_to_wsl` but emits `/<drive>` instead of `/mnt/<drive>`. Consider factoring both into a shared internal helper that takes a `&'static str` drive prefix (`"/mnt/"` vs `"/"`) to avoid duplication.

Add unit tests in `crates/warp_util/src/path_test.rs` matching PRODUCT.md invariant (2):
- `C:\Users\andy\file.txt` → `/c/Users/andy/file.txt`
- Spaces preserved
- `C:\` and `C:` both → `/c`
- Uppercase drive lowercased
- UNC fallback converts backslashes to slashes

### 2. Add a generic "path transformer" hook to `EditorView`

In `app/src/editor/view/mod.rs`:

- Add a public type alias `pub type PathTransformerFn = Rc<dyn Fn(&str) -> String>;` near the existing `CursorColorsFn` (line ~1587). `Rc` is already imported.
- Add `pub drag_drop_path_transformer: Option<PathTransformerFn>` to `EditorOptions` (default `None` in both `Default for EditorOptions` and `From<SingleLineEditorOptions> for EditorOptions`).
- Add a mirrored private field on `EditorView`, initialize it from `options.drag_drop_path_transformer` in `new_internal`, and add a `set_drag_drop_path_transformer` setter next to the existing `set_shell_family`. No public getter is needed: the only caller that would want to read the transformer back is the parent that installed it, and that parent already knows which transformation applies to the current session.
- In `EditorView::drag_and_drop_files`, after the image-files branch returns early, run each remaining (non-image) path through the transformer if present, then pass the transformed list — not `paths_as_strings` — into `escaped_paths_str`. Image paths stay untransformed because `Event::DroppedImageFiles` consumers (terminal input) need to read the original file from the host.

Design note: we deliberately picked a closure over an event/delegation flag. The editor already takes other domain-agnostic closures (`render_decorator_elements`, `cursor_colors_fn`, `keymap_context_modifier`), and a pure `Fn(&str) -> String` hook is the smallest, most focused extension point for what amounts to "rewrite each path string before inserting." It keeps image attachment, shell escaping, and insertion flow inside the editor unchanged, and it imposes no terminal vocabulary on the editor. A single closure type also naturally covers multiple concrete transformations (WSL, MSYS2, and anything else we add later).

### 3. Install the transformer from `TerminalInput`

In `app/src/terminal/input.rs`:

- In `set_active_block_metadata` (line ~13034), alongside the existing `editor.set_shell_family(...)` call, select a transformer based on the session and install or clear it:
  - `session.is_wsl()` → `Rc::new(|p: &str| warp_util::path::convert_windows_path_to_wsl(p))`.
  - `session.is_msys2()` → `Rc::new(|p: &str| warp_util::path::convert_windows_path_to_msys2(p))`.
  - Otherwise → `None`.
  Both session predicates return `false` off-Windows, so no `cfg!(windows)` gating is needed at the call site. Check `is_wsl()` before `is_msys2()` for clarity (they are mutually exclusive in practice, but the order documents priority).
- In the existing `EditorEvent::DroppedImageFiles` handler (line ~9493), when the image-attach fallback inserts paths as text, apply the same WSL / MSYS2 conversion to `image_filepaths` (branching on `session.is_wsl()` / `session.is_msys2()` just as step above does) before calling `escaped_paths_str`. This preserves PRODUCT.md invariant (7) for both WSL and MSYS2.

We rely on `set_active_block_metadata` being called whenever the active session changes; it already updates `shell_family` and `path_separators`, so the transformer follows the same lifecycle and always reflects the currently active session.

### 4. No changes needed elsewhere

- `TerminalView::drag_and_drop_files` (terminal grid path) already converts correctly for WSL, and deliberately does not convert for MSYS2 long-running commands. Per PRODUCT.md invariant (8), leave it alone.
- No other `EditorView::new(...)` call site sets the new option, so the default `None` transformer means identical behavior for notebooks, settings, etc.

## Testing and validation

Covers the invariants in `PRODUCT.md`.

- **Unit tests for `convert_windows_path_to_msys2` (invariant 2, MSYS2 cases).** In `crates/warp_util/src/path_test.rs`, mirror the existing `test_convert_windows_path_to_wsl` test with MSYS2-equivalent expectations (`/c/...`, `/d/...`, etc.).

- **Existing coverage for the WSL conversion itself (invariant 2, WSL cases).** `test_convert_windows_path_to_wsl` in `crates/warp_util/src/path_test.rs (629-649)` already verifies drive-letter lowercasing, spaces, UNC, and empty-suffix behavior. No new tests needed there.

- **Unit test — transformer wiring (invariants 1, 2, 5, 9).** In `app/src/editor/view/mod_test.rs`, add a test that:
  - Creates an `EditorView` with `shell_family: Some(Posix)` and runs two scenarios:
    1. `drag_drop_path_transformer: Some(Rc::new(|p| warp_util::path::convert_windows_path_to_wsl(p)))` — WSL behavior.
    2. `drag_drop_path_transformer: Some(Rc::new(|p| warp_util::path::convert_windows_path_to_msys2(p)))` — MSYS2 behavior.
  - Each scenario dispatches `EditorAction::DragAndDropFiles` with representative inputs (`C:\foo`, `D:\bar baz.txt`, a UNC path, and a path with no drive letter) and asserts the buffer contents match PRODUCT.md invariant (2) with shell escaping applied on top.
  - A third scenario clears the transformer and asserts the original Windows paths are inserted verbatim (PRODUCT.md invariant 5).

- **Image-attach fallback (invariants 6, 7).** In `app/src/terminal/input_test.rs`, if there is already coverage for the `DroppedImageFiles` fallback path, extend it to assert that when a transformer is installed, the fallback text is the transformed paths. If no such test exists yet, add a focused one that mocks an over-limit scenario and inspects the buffer. Exercise both a WSL transformer and an MSYS2 transformer. Image attachment itself is unchanged and needs no new test.

- **Manual verification (invariants 1, 3, 8, 10).** On a Windows machine with both a WSL distro and Git Bash installed:
  - Drag a file from Explorer into a WSL tab's input editor → buffer reads `/mnt/c/…` with shell escaping.
  - Drag a file from Explorer into a Git Bash tab's input editor → buffer reads `/c/…` with shell escaping.
  - Drag the same file onto a WSL tab while a long-running command is active → grid receives `/mnt/c/…` (regression check for existing behavior).
  - Drag the same file onto a Git Bash tab while a long-running command is active → grid receives `C:\…` without shell escaping (regression check: deliberate existing behavior per PRODUCT.md invariant 8).
  - Drag into a local PowerShell tab → buffer reads `C:\…` unchanged.
  - Drag into a notebook editor → buffer reads `C:\…` unchanged.
  - Drag an image file in Agent Mode with an empty buffer (WSL or Git Bash) → image attaches normally; no text inserted.

## Risks and mitigations

- **Stale transformer after session switch.** Because we install/clear the transformer in `set_active_block_metadata`, it always matches the active session. If that function is ever bypassed for a session change, the transformer could lag. Mitigation: piggy-back on the exact same call site as `set_shell_family`, which already has this assumption and has proven stable.

- **Cross-platform builds.** `convert_windows_path_to_wsl` is pure string manipulation and compiles on all platforms, as does `Session::is_wsl()` (returns `false` off-Windows). No `cfg!(windows)` gating needed at the call site, though in practice the transformer will only ever be non-`None` on Windows because no other platform has WSL sessions.
