# Bindable Shortcut To Copy Editor File Path (GH-10290)

## Summary

Add three new bindable Command Palette actions for the Warp editor — "Copy Path" (absolute), "Copy Relative Path" (relative to project root), and "Copy Path with Line" (relative path with `:line[:column]`). Each is rebindable in Settings → Keyboard Shortcuts. All three default to unbound, matching Warp's pattern of leaving optional new actions free for the user to assign.

## Problem

Users frequently need to share or reference the path of the file currently open in Warp's editor — pasting it into a chat, a terminal command, an LLM prompt, or a bug report. Today there is no first-class action to copy the file path; users either drag the file from the tree, copy from the breadcrumb manually, or type the path. This issue specifically asks for a customizable keyboard shortcut to copy the *relative* path of the open editor file (matching the VS Code pattern users already know: `editor.action.copyPath` and `copyRelativeFilePath`).

## Goals

- Provide three discoverable, rebindable actions that copy the active editor file's path in the three formats users actually need.
- Make all three discoverable in the Command Palette regardless of binding state.
- Anchor "relative" to the same project root the file tree uses (git toplevel; fall back to workspace folder).
- Use OS-correct path separators by default.
- Provide brief confirmation toasts and friendly fallbacks when the action cannot complete.

## Non-Goals

- Changing or rebinding existing copy actions on terminal output.
- Copying GitHub-style URL paths (e.g., permalinks). Tracked as future work.
- Multi-file or multi-cursor "copy all open file paths" actions.
- Opening a picker dialog to choose between formats — each format is its own bindable action.

## Behavior Contract

### B1. New actions

Three new actions are registered in the keymap and Command Palette:

- `editor.copy_absolute_path` — copies the absolute path of the active editor file to the clipboard.
- `editor.copy_relative_path` — copies the path relative to the project root.
- `editor.copy_path_with_line` — copies the relative path with a `:line[:column]` suffix derived from the cursor or selection. See B9 for the full numbering and inclusion contract.

### B2. Project root resolution

"Project root" is resolved from the **active workspace's** root, not from
the editor file's parent directory. This separates "what is the project?"
(workspace state) from "where does this file live?" (filesystem path), and
makes the B4 outside-project fallback well-defined: a file is "outside the
project root" iff its absolute path is not a descendant of the resolved
project root.

**Authoritative rule.** The project root is the **git toplevel containing
the active workspace folder when one exists, otherwise the active
workspace folder itself**. Git toplevel takes precedence over the workspace
folder when both are available; the workspace folder is the fallback when
the workspace is not inside any git working tree. This single rule is
authoritative; any earlier wording that called the workspace folder the
"canonical" root is superseded by this section. The reason git toplevel is
preferred is that monorepo users routinely open a sub-package as their
workspace, and "relative path" is far more useful when anchored at the
repo root (`packages/foo/src/x.ts`) than at the sub-package root
(`src/x.ts`) — the former is what teammates expect to paste into reviews,
issue links, and `cd` commands.

Resolution order (matches what the file tree uses; first match wins):

1. **Git toplevel of the active workspace folder.** If Warp has an active
   workspace AND the workspace folder is inside a git working tree, run
   `git rev-parse --show-toplevel` on the **workspace folder** (not on
   the editor file's parent directory) and use the toplevel as the
   project root. This is the common case and the preferred outcome.
2. **Active workspace folder fallback.** If Warp has an active workspace
   but the workspace folder is NOT inside any git working tree (or
   `git rev-parse` fails for any reason — git absent, repo corrupt,
   detached `.git` file pointing at a missing gitdir), the workspace
   folder itself is the project root.
3. **No active workspace.** If Warp has no active workspace at all (e.g.,
   a single-file editor session opened outside any workspace), there is
   no project root. Behavior:
   - `editor.copy_relative_path` and `editor.copy_path_with_line` follow
     B4's outside-project fallback (copy absolute path; preserve `:N[:K]`
     for the line variant) with the B4 toasts.
   - `editor.copy_absolute_path` is unaffected.

The resolved project root is cached per workspace. Cache invalidation rules
(any one of these invalidates the entry):

1. The active workspace changes (different workspace folder selected).
2. The workspace folder moves or is renamed on disk.
3. **Git metadata changes that could shift the toplevel**, even without a
   workspace switch. Specifically: any filesystem event on the workspace
   folder's parent chain that adds, removes, replaces, or modifies a
   `.git` entry (file OR directory). The implementation MUST watch the
   workspace folder and every ancestor up to the filesystem root for:
   - creation of a `.git` file or directory (`git init` in any ancestor;
     adding a submodule that creates a `.git` file in the workspace);
   - deletion of a `.git` entry (rm -rf of a parent repo);
   - replacement of `.git` (directory → gitdir file or vice versa, e.g.,
     adding/removing a submodule worktree, switching to a `git
     worktree`-managed checkout);
   - modification of a `.git` *file*'s contents (the `gitdir:` pointer
     changing destination).
4. An explicit "refresh project root" action (if exposed), or workspace
   reload.

Note 4. is sufficient even without filesystem watching: a user who runs
`git init` in their workspace and then immediately invokes
`editor.copy_relative_path` may see one stale resolution; the next
workspace reload or focus event MUST re-resolve. Implementations SHOULD
prefer event-driven invalidation (rule 3) over polling. TTL-based
invalidation is acceptable as a fallback only with TTL ≤ 5 seconds and
MUST be combined with rule 3 in the supported case.

Cached entries are keyed by the canonical (post-`canonicalize`) workspace
folder path so that a workspace folder reached via a different symlink
does not produce a stale hit.

Edge case — symlinks: both the resolved project root (whichever of #1 or
#2 applied) and the editor file path are canonicalized (resolved through
symlinks via `std::fs::canonicalize` on Unix and the platform equivalent
on Windows) before the inside/outside comparison and before the relative
path is computed. A file reached via a symlink whose canonical target is
inside the canonical project root is treated as inside the project; the
relative path is computed between the two canonicalized paths. If
canonicalization fails for either path (e.g., permission denied,
dangling symlink), the action treats the file as outside the project root
and falls back per B4.

### B3. No file open

If no file is open in the editor when the action fires, it is a no-op and surfaces a brief toast: "No editor file open".

### B4. File outside project root

If the file resolves but is OUTSIDE the project root (e.g., a file opened from `/tmp` while a project is active):

- `editor.copy_relative_path` falls back to the absolute path and surfaces a toast: "File is outside project root — copied absolute path instead".
- `editor.copy_path_with_line` falls back to the absolute path AND PRESERVES the line/column suffix using the same inclusion rules defined in B9. Output is `/abs/path/to/file:N` or `/abs/path/to/file:N:K` (depending on whether column is explicit). Toast: "File is outside project root — copied absolute path with line".
- `editor.copy_absolute_path` is unaffected (it always copies the absolute path).

### B5. Path separator

The copied string uses the OS-native separator:

- macOS / Linux: forward slash (`/`).
- Windows: backslash (`\`).

### B6. Confirmation toast

On success, each action surfaces a brief toast: `Copied: <truncated path>` where the path is truncated with a leading ellipsis if it exceeds 60 characters.

### B7. Editor focus context (key-bound invocation)

When invoked via a configured key binding, the action only fires when the editor pane has focus (`KeyContext = EditorFocused`). Bindings configured in Settings → Keyboard Shortcuts respect this context.

### B8. Command Palette discoverability and target editor

All three actions are visible in the Command Palette UNCONDITIONALLY (independent of editor focus) and are searchable by "copy path". The Command Palette itself moves keyboard focus to the palette input when it opens, so editor focus is necessarily lost while the palette is open. To resolve the resulting ambiguity about which editor the action targets:

- **Last-focused-editor tracking**: Warp tracks the "last focused editor" — the most recent editor pane that held focus before the Command Palette was opened. The application updates this tracker on every editor focus change.
- **Palette invocation target**: when one of the three copy-path actions is invoked from the Command Palette, the action targets the LAST-FOCUSED EDITOR captured at the moment the palette was opened. The palette dismisses on action invocation; focus returns to that editor.
- **No editor was previously focused**: if no editor pane had focus before the palette was opened (e.g., the palette was invoked from the file tree, settings, or a pane with no editor), the action is a no-op and surfaces the B3 toast: "No editor file open".
- **Visibility vs. execution**: actions remain visible in the palette regardless of the last-focused-editor state. Visibility is unconditional; execution requires a non-null last-focused editor with an open file.

### B9. `copy_path_with_line` numbering and suffix contract

`editor.copy_path_with_line` produces a deterministic clipboard string for a given cursor/selection state. The contract:

- **Line numbering**: 1-based — the first line is line `1`. This matches universal editor convention (vim, VS Code, IntelliJ).
- **Column numbering**: 1-based — the first column is column `1`. Columns are **grapheme-cluster INSERTION POSITIONS**, not grapheme cells. A cursor is an insertion point that lives BETWEEN graphemes (or at line start/end), so column K means "the cursor is positioned such that the next inserted text would become the K-th grapheme of line N". Equivalently: column K = (number of grapheme clusters to the LEFT of the cursor on line N) + 1. Grapheme cluster boundaries follow UAX #29 GraphemeBreakProperty; counting is NOT by bytes and NOT by raw codepoints. Concretely:
  - A cursor at the very start of a line (before any character) is column `1`.
  - A cursor immediately after the first ASCII character is column `2`.
  - A cursor immediately after a multi-byte UTF-8 character (e.g., `日`) is column `2`.
  - A cursor positioned in the middle of a multi-codepoint grapheme (e.g., between `e` and the combining `U+0301` of `é`) is NOT a valid insertion position; the editor MUST snap such cursors to the nearest grapheme boundary (the cluster's leading edge for leftward moves, trailing edge for rightward moves) before the column is read. The clipboard column always reflects the snapped position.
  - A cursor immediately after an emoji ZWJ sequence (e.g., 👨‍👩‍👧) is one column past the start of that sequence.
  - Tabs (`\t`) count as one column. Width-aware tab expansion is NOT performed.
- **End-of-line cursor**: a cursor positioned past the last grapheme of line N (i.e., the "after the last character" insertion point, often produced by `End` or by typing through to line end) reports column `G + 1`, where `G` is the count of grapheme clusters on line N. For an empty line, `G = 0` and end-of-line column is `1` — identical to start-of-line, which is correct because both positions are the same insertion point. The clipboard suffix never points past `G + 1`; cursors logically "on the newline" are reported as end-of-line of the line they visually occupy, never as column `1` of line `N + 1`.
- This single grapheme-cluster-insertion-position model supersedes any earlier wording that said "codepoints" or that treated the column as the index of a grapheme cell — the prior phrasing is replaced by this one.
- **When column is included** — driven by an explicit **per-cursor** boolean **`HasExplicitColumn`** that the editor maintains for **each individual cursor** (one flag per cursor, NOT one flag per document). When the editor has a single cursor, "the cursor's flag" is unambiguous. The clipboard suffix is emitted iff the relevant cursor's flag is `true`. Any earlier wording that called this a "per-document" flag is **superseded** — it is per-cursor. The flag transitions are deterministic and source-of-input agnostic; provenance is not tracked at copy time:
  - `HasExplicitColumn := true` on: mouse click placing the cursor; horizontal arrow keys (Left/Right); Home/End; click-drag selection drop; programmatic positioning that supplied a column; type-edit that advances the column on the same line.
  - `HasExplicitColumn := false` on: a "go to line N" command that did not supply a column; opening a file (cursor restored to line 1, col 1 with no column intent); vertical arrow keys (Up/Down) that traveled across lines using the editor's "preferred column" memory without a fresh horizontal action since the last vertical move.
  - The previous wording listed "cursor at start of line (col 1)" and "cursor at end of line" as triggers for omitting the column. That wording was ambiguous because a deliberate click on col 1 and a fresh-from-go-to-line cursor at col 1 are indistinguishable by position alone. It is **superseded** by `HasExplicitColumn`: position alone never controls the suffix; only the boolean does.
- **Single cursor, no selection**: the cursor's line and its `HasExplicitColumn` apply.
- **Single cursor with active selection**: the line and `HasExplicitColumn` of the SELECTION ANCHOR (where the selection began) apply — not the caret's. Reverse selections (anchor below caret) still use the anchor. The selection anchor carries its own `HasExplicitColumn` (snapshot of the cursor flag at the moment the selection started; horizontal/vertical motion of the caret during selection extension does NOT mutate the anchor's flag).
- **Multi-cursor (multiple cursors, no selections)**: there is a single distinguished **PRIMARY CURSOR** — by convention the cursor most recently created or the first cursor in document order if none has been distinguished. The primary cursor's line and its `HasExplicitColumn` are used; secondary cursors are IGNORED for the purposes of this action. The clipboard receives exactly ONE path string regardless of cursor count. Each cursor maintains its own flag; the action reads only the primary's. A toast `Copied: <truncated path>` is emitted exactly once.
- **Multi-cursor with selections** (multiple simultaneous selections, e.g., from "Add Selection to Next Find Match" / column selection): the PRIMARY SELECTION's anchor line and that anchor's `HasExplicitColumn` are used. The primary selection is the selection associated with the primary cursor (same convention as above). All other selections are ignored. The clipboard receives exactly ONE path string; no newline-joined or comma-joined multi-line/multi-column variant is produced by this action.
- **Mixed multi-cursor (some with selections, some without)**: the primary cursor's state alone is consulted — if the primary has a selection, its anchor is used; otherwise the primary cursor's position is used. Other cursors are still ignored.
- **Output forms**: emit `<path>:<line>:<col>` when `HasExplicitColumn == true`, otherwise `<path>:<line>`. Both forms are valid outputs of the same action; which form is emitted is fully determined by `HasExplicitColumn` at invocation time.

## Settings / API surface

Three new keymap entries (default unbound):

- `editor.copy_absolute_path`
- `editor.copy_relative_path`
- `editor.copy_path_with_line`

UI: Settings → Keyboard Shortcuts → searching "copy path" reveals all three. Users assign chords as they would for any other action.

No new user-level toggle is introduced; the actions are always available.

## Acceptance Criteria

Bound-shortcut path:

- A1. Each bound action, invoked while the editor is focused, copies the correct path to the system clipboard.
- A2. `editor.copy_relative_path` produces a path relative to the git toplevel when the file is inside a git repo.
- A3. A file opened from outside the project root falls back to absolute path with the B4 toast (`copy_relative_path` and `copy_path_with_line`).
- A4. With no file open, each action is a no-op and surfaces the B3 toast.
- A5. Copied paths use the OS-native separator.
- A6. `editor.copy_path_with_line` includes the column suffix iff the editor's per-cursor `HasExplicitColumn` flag (B9) is `true`; otherwise it emits `<path>:N` only. When a selection is active, the anchor's flag and line/column are used. With multiple cursors or selections, only the PRIMARY cursor/selection is consulted. Columns are 1-based grapheme-cluster INSERTION positions per UAX #29; an end-of-line cursor on a line with `G` graphemes reports column `G + 1`.
- A7. All three actions appear in the Command Palette without requiring a binding.

Outside-project + line/column path:

- A8. `editor.copy_path_with_line` invoked on a file outside the project root copies the absolute path AND preserves the `:N[:K]` suffix per B9, with the toast "File is outside project root — copied absolute path with line".

Command Palette path:

- A_palette_targets_last_focused_editor. When invoked from the Command Palette, the action targets the last-focused editor pane (captured at the moment the palette opened); the palette dismisses and focus returns to that editor.
- A_palette_no_editor_noop. When invoked from the Command Palette with no last-focused editor (palette opened from file tree, settings, or other non-editor surface), the action is a no-op and surfaces the B3 toast "No editor file open"; the action remains visible in the palette regardless.

## Implementation Pointers

Verified paths:

- Editor module: `app/src/code/editor/` (e.g., `app/src/code/editor/element.rs`, `app/src/code/editor/comment_editor.rs`) — host of editor focus state and active-file metadata.
- Command Palette: `app/src/command_palette.rs` and `app/src/search/command_palette/` — action registration and discoverability.
- Keybinding settings UI: `app/src/settings_view/keybindings.rs` — where users see/edit bindings.
- Existing keybinding view example: `app/src/editor/accept_autosuggestion_keybinding_view.rs`.
- Clipboard write API: `app/src/util/clipboard.rs` plus platform implementations under `crates/warpui/src/platform/mac/clipboard.rs`, `crates/warpui/src/windowing/winit/linux/clipboard.rs`, `crates/warpui/src/windowing/winit/windows/clipboard.rs`.
- Telemetry: existing `command_palette.action_invoked` is reused — extend payload with the new action IDs.

New modules:

- `app/src/code/editor/copy_path_actions.rs` (new) — the three action handlers, project-root resolver, and toast helpers.

## Tests

- T1. `copy_absolute_path` produces the absolute path for the active editor file.
- T2. `copy_relative_path` produces the path relative to the git toplevel.
- T3. `copy_path_with_line` produces `<relative>:line[:column]` correctly.
- T4. File outside project root falls back to absolute path with the B4 toast.
- T5. No-file case is a no-op with the B3 toast.
- T6. macOS/Linux uses forward slashes; Windows uses backslashes (platform-gated tests).
- T7. Rebinding the action in Settings → Keyboard Shortcuts persists across restarts.
- T8. All three actions are discoverable in the Command Palette even when unbound.
- T_line_only_after_goto. Run "Go to line N" with no column → `HasExplicitColumn = false` → output is `<path>:N` with NO column suffix.
- T_line_only_after_open. Open a fresh file; do not interact with the cursor → `HasExplicitColumn = false` → output is `<path>:1`.
- T_line_col_after_click_at_col_1. Mouse-click on line N column 1 → `HasExplicitColumn = true` → output is `<path>:N:1` (column IS emitted because the click set the flag, even though the position is col 1).
- T_line_col_after_arrow_horizontal. Cursor moved mid-line via arrow keys → `HasExplicitColumn = true` → output is `<path>:N:K`.
- T_line_col_after_vertical_only. From `<path>:N` (HasExplicitColumn = false), press Down arrow once → caret moves to next line via preferred-column memory; flag stays false → output is `<path>:N+1` (NO column).
- T_line_only_after_end_of_line_via_goto. Go-to-line places cursor at end of line → `HasExplicitColumn = false` → output is `<path>:N`. Pressing End from there sets the flag → subsequent copy emits `<path>:N:K`.
- T_selection_uses_anchor. With an active selection from anchor (line 5, col 3, HasExplicitColumn = true) to caret (line 12, col 1), output is `<path>:5:3`. Reverse selection (anchor below caret) still uses the anchor's line and flag.
- T_unicode_grapheme_columns. A line containing multi-byte UTF-8 ("héllo", "日本語"), combining marks (`e\u{0301}`), and a ZWJ emoji (`👨\u{200D}👩\u{200D}👧`) — column is the 1-based grapheme-cluster index per UAX #29, not bytes and not codepoints. Cursor after `日` on `日本語` → column 2. Cursor after `é` written as `e\u{0301}` → column 2 (one grapheme, not 3 codepoints). Cursor after the family ZWJ emoji → column 2.
- T_outside_project_with_line. `copy_path_with_line` invoked on a file outside the project root copies `/abs/path/to/file:N` (or `:N:K` if column is explicit) and surfaces the toast "File is outside project root — copied absolute path with line".
- T_palette_targets_last_focused_editor. With editor A focused, open Command Palette, run `editor.copy_relative_path` → clipboard contains the path of the file in editor A; focus returns to editor A after the palette dismisses.
- T_palette_no_editor_noop. With no editor previously focused (palette opened from file tree), run any of the three actions → no clipboard write, B3 toast surfaces, action still appeared in the palette list.

Project-root resolution branches (B2):

- T_root_git_toplevel_in_monorepo. Active workspace is opened on `<repo>/packages/foo` and `<repo>/.git` exists. Editor file is `<repo>/packages/foo/src/x.ts`. `editor.copy_relative_path` produces `packages/foo/src/x.ts` (anchored at git toplevel, not the workspace folder). Verifies B2.1 takes precedence over B2.2.
- T_root_workspace_fallback_no_git. Active workspace is `/Users/u/notes` and that folder is NOT inside any git working tree (no `.git` reachable up the parent chain). Editor file is `/Users/u/notes/today.md`. `editor.copy_relative_path` produces `today.md`. Verifies the B2.2 workspace-folder fallback fires when git toplevel resolution fails because no repo exists.
- T_root_workspace_fallback_git_failure. Active workspace is `/Users/u/proj` containing a corrupt `.git` file (e.g., gitdir pointer to a missing path). `git rev-parse --show-toplevel` exits non-zero. The action falls back to the workspace folder per B2.2 — `editor.copy_relative_path` produces `src/x.ts` for `/Users/u/proj/src/x.ts` rather than erroring or producing the absolute path.
- T_root_no_workspace_fallback. Warp has no active workspace (single-file editor session). Editor file is `/tmp/scratch.txt`. `editor.copy_relative_path` falls back to absolute path `/tmp/scratch.txt` with the B4 outside-project toast. `editor.copy_path_with_line` does the same and preserves `:N[:K]` per B4 and B9. `editor.copy_absolute_path` is unaffected.
- T_root_symlink_canonicalization_inside. Project root is `/Users/u/proj` (canonical) and the editor opens `/Users/u/proj/src/x.ts` via the symlink path `/Users/u/link-to-proj/src/x.ts`. After canonicalizing both root and file path, the file is inside the project; `editor.copy_relative_path` produces `src/x.ts` (relative path computed from canonicalized paths, not from the symlink path that opened the file).
- T_root_symlink_canonicalization_outside. The editor opens `/Users/u/proj/external` which is a symlink whose canonical target is `/Users/u/elsewhere/external`. After canonicalization, the file is outside the project root; `editor.copy_relative_path` falls back to the canonical absolute path `/Users/u/elsewhere/external` with the B4 toast.
- T_root_symlink_canonicalization_failure. Editor opens a dangling symlink (`/Users/u/proj/broken` → missing target). Canonicalization fails; the action treats the file as outside the project root per B2 and falls back per B4 with the absolute (uncanonicalized) symlink path. No panic, no silent success.

Cache invalidation on git metadata changes (B2 cache rules):

- T_cache_invalidated_on_git_init. Active workspace `/Users/u/notes` has no git repo; first `copy_relative_path` invocation resolves the workspace folder as project root. Run `git init` in that folder. After the cache-invalidation event (FS watch event for `.git` creation OR a workspace reload), the next `copy_relative_path` resolves the new git toplevel as the project root and produces paths relative to it.
- T_cache_invalidated_on_git_rm. Active workspace inside a git repo; project root resolves to the git toplevel. The user deletes `.git/`. After invalidation, the next `copy_relative_path` falls back to the workspace folder per B2.2.
- T_cache_invalidated_on_gitdir_flip. Workspace is a worktree managed by `git worktree`; `.git` is a file with a `gitdir:` pointer. The user edits the pointer to a different gitdir (or `git worktree move`s the worktree). After invalidation, the next `copy_relative_path` resolves the new toplevel rather than the cached stale one.

Cursor / multi-cursor / end-of-line column behavior (B9):

- T_end_of_line_column. Line N contains `hello` (5 graphemes). Cursor at the end-of-line insertion point with `HasExplicitColumn = true` (set by pressing End) → output is `<path>:N:6`. No reference to column `1` of line `N + 1`.
- T_empty_line_column. Line N is empty (`G = 0`). Cursor on that line with `HasExplicitColumn = true` → output is `<path>:N:1`.
- T_mid_grapheme_snaps. Programmatic cursor placement targets the byte index between `e` and `U+0301` of `é`. The editor snaps to the cluster boundary BEFORE reading the column; output is the snapped column, not an off-boundary index.
- T_multi_cursor_uses_primary. Two cursors on lines 3 (column 4, `HasExplicitColumn = true`) and 10 (column 2, `HasExplicitColumn = false`). Primary is the line-3 cursor → output is `<path>:3:4`. Exactly one path is written; secondary cursor is ignored. Exactly one toast surfaces.
- T_multi_selection_uses_primary_anchor. Two selections; primary selection's anchor is at line 5 column 3 (`HasExplicitColumn = true`); secondary selection's anchor is at line 20 column 1. Output is `<path>:5:3`. Secondary selection is ignored.
- T_multi_cursor_mixed_no_join. One cursor without selection at line 2 (primary, `HasExplicitColumn = false`), one selection elsewhere at line 8 column 5. Output is `<path>:2` (primary cursor's position; primary has no selection so its anchor is irrelevant; secondary is ignored). The clipboard does not contain a multi-line/multi-column joined string.

## Open Questions

- Should we also expose a POSIX-style normalization toggle on Windows (so users who paste paths into WSL/Git Bash get forward slashes regardless of OS)? Suggest deferring to V1.5 with a per-action setting `editor.copy_path.posix_separator_on_windows` (bool, default `false`).
- Should `editor.copy_path_with_line` also expose an absolute variant? Likely yes in V1.5; not in V1 to avoid action-list bloat.

## Telemetry

Reuse existing `command_palette.action_invoked` event with the action ID payload — no new telemetry events are needed. The three new action IDs become valid values for that event's `action_id` field.
