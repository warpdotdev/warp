# Folder Tree View for Code Review File List (GH-10340)

## Summary

Add a VS Code-style folder-grouped tree view to the Code Review changed-files
list, with a header toggle between Flat and Tree modes. Tree mode supports
collapsible folders, compact single-child folder chains, folder-level change
counts and aggregate `+/-` line counts, and preserves selection across mode
switches.

## Problem

The Code Review changed-files list is currently flat. For PRs that touch many
files spread across nested directories, this flat layout makes it hard to
scan changes by area, hard to scope review per-package, and hides the shape of
the change. VS Code's Explorer-style tree view is the expected mental model
for a hierarchical changed-files list.

## Goals

- Tree view of changed files grouped by directory.
- Collapsible folders with expand/collapse affordance.
- User-visible toggle to switch between Flat and Tree views.
- Preserve current selection when switching between modes.
- Folder-level summaries: count of changed files and aggregate `+N -M` line
  counts where available.
- Compact-folder behavior matching VS Code (collapse single-child chains).

## Non-Goals

- Not a full file explorer; this is scoped to the Code Review changed-files
  list only.
- No new git operations exposed from the tree (stage/discard/etc.).
- Not synchronizing folder expansion state across sessions in V1; tracked as
  V1.5 (see Open Questions).
- No symbol-level / hunk-level grouping; entries remain at file granularity.

## Behavior Contract

### B1. Mode toggle

A header button group "Flat | Tree" switches the file list between modes.
Default is Flat to preserve current behavior. The user's choice persists per
user (across restarts).

### B2. Tree rendering and compact folders

In Tree mode the file list renders as a hierarchy of folders rooted at the
review's repo root. Single-child folder chains collapse into a compact label
matching VS Code's Explorer (e.g. `app/src/foo/bar/baz.rs` displays under a
compact folder row `app/src/foo/bar` when no other siblings exist at the
intermediate levels). A "Compact folders" overflow option (default ON)
toggles this behavior. When OFF, every directory level renders as its own
row.

### B3. Folder rows

Each folder row shows:

- folder name (or compact path),
- count of changed files contained (recursive), and
- aggregate `+N -M` line counts when the diff stats are available.

### B4. File rows

Each file row under a folder shows:

- filename,
- change kind indicator (M / A / D / R),
- per-file `+N -M` line counts, and
- status icon.

Selecting a file row opens the diff using the same routing as Flat mode.

### B5. Expand / collapse

- Click on a folder row toggles expand/collapse.
- Right arrow expands a collapsed folder; on an expanded folder it moves to
  the first child.
- Left arrow collapses an expanded folder; on a collapsed folder or file it
  moves to the parent.
- Cmd-click (macOS) / Ctrl-click (Windows/Linux) on a folder row toggles all
  descendants.

### B6. File-list filter contract

**TL;DR for implementers.** The file-list filter is a case-insensitive
plain-text substring match against the **basename only** of each changed
file. It never matches folder rows, full paths, intermediate segments, or
the non-attributed side of a rename. Filter-driven folder auto-expansion
is **transient** (overlay layer) and never mutates the persisted
expanded-state cache. Two normative subsections below specify these
rules.

This spec introduces a **new file-list filter** that is distinct from Code
Review's existing content-based find bar. The two are independent:

- The **existing find bar** continues to operate inside the open diff editor
  as a content search across diff text. It is unchanged by this spec.
- The **new file-list filter** is a filename-substring filter (case-
  insensitive, plain text — no regex in V1) embedded in the file-list panel
  header.

UI placement (authoritative — the file-list panel is the panel that displays
the changed-file list, distinct from the Code Review header):

The file-list panel header lays out, top to bottom:

1. **Filter input** — left-aligned, full width minus toolbar actions.
2. **Flat/Tree toggle** — right-aligned button group on the same row as
   (or directly below, depending on width) the filter input.
3. **Kebab/overflow menu** — sits at the right edge of the toolbar row and
   contains:
   - "Compact folders" (toggle, default ON; B2)
   - "Group renames by old path" (toggle, default OFF; B9)
   - "Hide non-matching files when filtering" (toggle, default OFF; B6c)

ALL toggles (Flat/Tree, filter input, Compact folders, rename grouping,
hide-non-matches) live in the **file-list panel header** — NOT in the Code
Review header component. See Implementation Pointers for the exact module.

Filter behavior:

- Filters the changed-files set only; it never searches diff content.

#### B6a-match. Match scope (authoritative)

The filter performs a **case-insensitive plain-text substring match
against the file's basename only** — the final path segment of the
attributed path. For example, with files `app/src/foo/bar.rs`,
`app/src/foo/baz.txt`, and `unrelated/foo.rs`:

| Filter input | Matches |
|---|---|
| `"bar"` | `app/src/foo/bar.rs` (basename `bar.rs` contains `bar`) |
| `"baz"` | `app/src/foo/baz.txt` |
| `"foo"` | ONLY `unrelated/foo.rs` (the `foo` in `app/src/foo/` is a directory segment, not a basename) |
| `"src"` | NOTHING (no basename contains `src`) |
| `"app/src"` | NOTHING (full paths are never matched) |

The filter does **NOT** match against:

1. **Full relative paths** (e.g. `app/src/foo/bar.rs`) — never considered.
2. **Intermediate directory segments** (e.g. the `src` in
   `app/src/foo/bar.rs`) — never considered.
3. **Folder row labels** in Tree mode (e.g. the row labeled `app/src/foo`
   or the compact label `app/src/foo/bar`) — folders themselves are
   never "matched"; they are only auto-expanded when a descendant file's
   basename matches.
4. **Compact folder labels** specifically (e.g. `app/src/foo/bar` as a
   single compact row) — the embedded path segments are not matched even
   though they appear in the visible label.
5. **The non-attributed side of a rename** — for a renamed file
   `a/old.rs → b/new.rs`, the path that appears as the *subtitle* under
   the row title is never considered for matching.

For renamed files, the match is evaluated against the **basename of the
attribution path**:

- When `code.review.file_list.group_renames_by_old_path = false`
  (default): match the **new-path basename** (`new.rs` in the example
  above). Filter `"new"` matches; filter `"old"` does not.
- When `code.review.file_list.group_renames_by_old_path = true`: match
  the **old-path basename** (`old.rs`). Filter `"old"` matches; filter
  `"new"` does not.

The non-attributed side (rendered as the subtitle) is **not** considered
for matching under either setting.
- In Tree mode, files whose names match the filter cause their parent
  folders to auto-expand. Non-matching siblings are **dimmed** by default
  (see B6c for the toggle that hides them entirely).
- In Flat mode, non-matching files are **dimmed** by default. (Same toggle
  in B6c hides them when ON.)

#### B6b-expansion. Filter-driven auto-expansion semantics (authoritative)

Filter-driven auto-expansion is **transient — it never mutates the
review's cached expansion state**. The renderer maintains two layers:

1. **Base expanded-state map** (persistent for the lifetime of the review
   session). Keyed by folder path. **Mutated ONLY** by:
   - User click on a folder row.
   - Keyboard Enter on a focused folder row.
   - Left/Right arrow expand/collapse on a folder row.
   - `Cmd`/`Ctrl`-click-toggle-all-descendants from B5.
   This is the same expanded-state cache referenced in Implementation
   Pointers as the "per-`(repo, review)` expansion state".
2. **Transient filter-expansion overlay** (an in-memory set of folder
   paths). Populated when the filter is non-empty with the ancestors of
   every file whose basename matches the filter. **Mutated ONLY** by:
   - The filter input value changing to a non-empty value (recompute
     overlay).
   - The filter input becoming empty or losing focus while empty (clear
     overlay).

The visible expanded state of a folder is the **set union** of the two
layers (base ∪ overlay) — a folder is rendered expanded if it is
expanded in either layer.

Concrete cases (normative):

- **Clearing the filter** (input value → empty, or focus lost while
  empty) clears the overlay. The visible expanded state reverts to the
  base map exactly. Folders auto-expanded only because of the filter
  collapse back to their pre-filter state.
- **Manual expansion while the filter is active.** If the user clicks
  a folder row (or otherwise triggers a B5 expand action) while the
  filter is non-empty, that action mutates the **base map** as usual.
  When the filter is later cleared, that folder remains expanded
  because the base map remembers it.
- **Manual collapse of an auto-expanded folder.** If the filter
  auto-expanded folder `X` and the user clicks `X` to collapse it
  while the filter is still active, the base map is updated to mark
  `X` as collapsed AND the overlay's entry for `X` is removed for the
  remainder of the current filter span. `X` is rendered collapsed
  for the rest of the filter session (the overlay does not re-expand
  it). On the next change of the filter input value, the overlay is
  recomputed from the new filter results, which MAY re-add `X`.
- **Switching modes while filtering.** Toggling Flat ↔ Tree preserves
  both the base map and the overlay; the filter value is preserved
  per B6 and the overlay is re-applied on entering Tree mode.
- **Closing the review session.** Both layers are discarded; nothing
  persists across review sessions in V1 (see Open Questions for
  V1.5 persistence).

The overlay is never written to disk; it is never included in any
persisted setting; it is never visible to telemetry as a distinct
event.
- The filter value is preserved when the user toggles Flat ↔ Tree.
- The filter is window-local and cleared when the review session closes;
  it is not persisted across reviews or restarts.

### B6c. Hide-non-matches toggle

Setting key: `code.review.file_list.hide_non_matches_when_filtering`
(`bool`, default `false`).

- When `false` (default): non-matching files/folders are **dimmed** but
  remain visible in both Flat and Tree modes. This applies in both modes
  identically.
- When `true`: non-matching files are **hidden** entirely from the list. In
  Tree mode, folders whose entire descendant set is non-matching are also
  hidden; folders with at least one matching descendant remain visible
  (auto-expanded as in B6).
- Surfaced as the "Hide non-matching files when filtering" entry in the
  file-list panel header kebab menu (see B6 UI placement).
- Persisted per user across restarts.

### B6a. Selected file definition

"Selected file" in the Code Review file list is defined as **the file whose
diff is currently open in the main pane**. This matches today's Flat-mode
behavior: clicking a file row opens its diff and that file becomes the
selection. The selection is **window-local**; switching reviews/PRs or
closing the review session resets it.

Initial state: when a review opens with no diff yet selected, no file is
selected. Toggling Flat ↔ Tree in this state does not preserve a selection;
the new mode scrolls to the top of the list.

### B7. Keyboard navigation

- Up/Down arrows navigate rows in display order (folders and files
  interleaved per the tree).
- Enter on a file opens the diff.
- Enter on a folder toggles its expanded state.
- Type-ahead jumps to the next row whose label matches the typed prefix.

### B8. Selection preservation across mode switch

Switching between Flat and Tree preserves the currently selected file (as
defined in B6a — the file whose diff is open in the main pane). Its row is
auto-revealed in the new mode (parents expanded as needed in Tree mode; row
scrolled into view in Flat mode). If no file is selected (no diff open),
toggling the mode does not preserve a selection and the new mode scrolls to
the top of the list.

### B9. Renames and cross-directory placement

Files with change kind `R` (renamed) — including cross-directory renames —
are placed in the tree as follows by default:

- The renamed file is grouped under the **parent folder of its NEW path**.
- The row label shows the new filename, with the old path rendered as a
  small subtitle, e.g. `bar.rs (was foo/bar.rs)`.
- The rename counts **once** toward the new folder's "changes" count and
  contributes its `+/-` line counts to the new folder's aggregate.
- The rename is **never** double-counted (it does not appear in, nor
  contribute stats to, the old folder).

An overflow toggle "Group renames by old path" (default OFF) inverts this
attribution: when ON, renamed files are grouped under the parent folder of
their OLD path, the new path appears as the subtitle, and stats accrue to
the old folder.

**Authoritative persistence contract for this toggle.** The toggle is
backed by the persisted setting
`code.review.file_list.group_renames_by_old_path` (`bool`, default
`false`) defined in Settings / API surface. It persists across restarts
on a per-user basis, identical to the other kebab-menu toggles (Compact
folders, Hide non-matching files when filtering). Earlier wording that
described this toggle as "window-local and not persisted" is superseded
by this contract — the persisted setting is the single source of truth.

## Settings / API surface

- `code.review.file_list_mode` — `"flat"` | `"tree"`, default `"flat"`.
- `code.review.tree_compact_folders` — `bool`, default `true`.
- `code.review.file_list.hide_non_matches_when_filtering` — `bool`, default
  `false`. Controls B6c.
- `code.review.file_list.group_renames_by_old_path` — `bool`, default
  `false`. Controls the rename-grouping inversion described in B9.

UI placement (all in the **file-list panel header**, NOT the Code Review
header — see B6 for the authoritative layout):

- Filter input (B6).
- "Flat | Tree" button group.
- Kebab/overflow menu entries:
  - "Compact folders" (toggle, default ON).
  - "Group renames by old path" (toggle, default OFF).
  - "Hide non-matching files when filtering" (toggle, default OFF).

## Acceptance Criteria

- A1. Toggle Flat ↔ Tree switches the rendering correctly.
- A2. Default is Flat; existing users see no change until they opt in.
- A3. Compact-folders ON collapses single-child chains; OFF renders one row
  per level.
- A4. Folder counts and aggregate `+N -M` are correct (sum of descendants).
- A5. Currently selected file is preserved across mode toggle and revealed.
- A6. Search filter works in both modes; in Tree mode, parent folders of
  matching files auto-expand.
- A7. Keyboard navigation (arrows, Enter, Cmd-click) behaves per B5–B7.
- A8. `code.review.file_list_mode`, `code.review.tree_compact_folders`,
  `code.review.file_list.hide_non_matches_when_filtering`, and
  `code.review.file_list.group_renames_by_old_path` all persist across
  restart.
- A9_hide_non_matches_default. With the filter active and
  `hide_non_matches_when_filtering = false` (default), non-matching files
  remain visible but dimmed in both Flat and Tree modes.
- A10_hide_non_matches_on. With the filter active and
  `hide_non_matches_when_filtering = true`, non-matching files are hidden
  entirely; in Tree mode, folders containing zero matching descendants are
  also hidden, while folders with at least one matching descendant remain
  visible and auto-expanded.
- A_rename_grouping_default. With
  `group_renames_by_old_path = false` (default), renamed files appear under
  their NEW path's parent folder, with the old path shown as subtitle, and
  contribute exactly once to the new folder's count and aggregate.
- A_rename_grouping_toggle. With
  `group_renames_by_old_path = true`, renamed files appear under their OLD
  path's parent folder, with the new path shown as subtitle, and contribute
  exactly once to the old folder's count and aggregate.
- A_telemetry_pane_opened. When the Code Review pane opens, the existing
  `CodeReview.PaneOpened` event is emitted exactly once and includes a
  `file_list_mode` field whose value matches the active mode at open time
  (`"flat"` or `"tree"`).
- A_telemetry_mode_toggle. Toggling Flat ↔ Tree mid-session emits exactly
  one `CodeReview.FileListModeToggled` event per toggle, with payload
  `{ from, to }` reflecting the transition (and `from != to`). Switching
  reviews/PRs without toggling does NOT emit this event.
- A_telemetry_filter_debounce. Using the file-list filter emits exactly
  one `CodeReview.FileListFiltered` event per filter session — defined
  as a contiguous span of typing terminated by either (a) clearing the
  filter input or (b) the file-list filter input losing focus. Rapid
  keystrokes within a session coalesce into a single event whose
  `results_count` is the result count at session end. Payload is
  `{ mode, results_count }` and contains no other fields. The event is
  NOT emitted when the filter input is empty.

## Implementation Pointers

**Where the existing sidebar lives today.** The current Code Review
changed-files sidebar is rendered from
`app/src/code_review/code_review_view.rs` (the existing flat-list
renderer). This spec does **not** delete or replace that file; instead
it carves the file-list rendering out into a new sibling module so the
two modes can coexist. After this refactor:

- `app/src/code_review/code_review_view.rs` remains the top-level Code
  Review pane. It loses its inline file-row rendering loop but keeps
  layout glue, the existing toolbar host, and the call site that
  invokes the new file-list module.
- `app/src/code_review/file_list/` (NEW DIRECTORY introduced by this
  spec) hosts the Flat and Tree renderers, the filter input, the
  Flat/Tree toggle, and the kebab menu.

Implementers should expect to:

1. Create the new module tree at `app/src/code_review/file_list/` (new
   directory) containing:
   - `mod.rs` — public surface for the file-list panel.
   - `tree.rs` — tree data model (build, compact-folder collapse,
     aggregate counts, expanded-state cache layered as base map +
     transient filter overlay; see B6 filter-driven auto-expansion).
   - `renderer.rs` — both Flat-mode and Tree-mode renderers, parameterized
     on `code.review.file_list_mode`. The Flat-mode renderer is moved
     from `code_review_view.rs` into this module verbatim (no behavior
     change) so both modes share the same call site.
   - `header.rs` — the file-list panel header that hosts the filter
     input, Flat/Tree toggle, and kebab/overflow menu (see B6).
2. Update `code_review_view.rs` to delegate sidebar rendering to
   `file_list::render(...)` instead of inlining the flat list. After this
   change, `code_review_view.rs` should contain no file-row rendering
   code — only layout glue and the call into the new `file_list`
   module.
3. Build a tree from the flat path list once per file-list update; cache
   expanded-state by path for the active review (in `tree.rs`). Two
   layers: a persisted-shape base map mutated only by user clicks /
   keyboard, and a transient filter-expansion overlay cleared on filter
   empty/blur (see B6).
4. Reuse existing diff-open routing on file-row activation — keep the
   activation callback identical to today's flat list to satisfy A5 /
   T3 (selection preservation across mode toggle).
5. Wire the filter input, Flat/Tree button group, and kebab/overflow menu
   into `file_list::header`. These controls do **not** live in
   `app/src/code_review/code_review_header/` — that module remains the
   Code Review pane's top-level header (PR selector, etc.) and is
   unmodified by this spec.

## Tests

- T1. Tree construction from a flat path list (correctness of structure,
  parent links, file counts).
- T2. Compact-folder collapsing of single-child chains.
- T3. Mode toggle preserves selection (flat → tree → flat).
- T4. Search auto-expands ancestor folders of matching files.
- T5. Keyboard arrow navigation in Tree mode (including parent/child moves).
- T6. Folder-level `+/-` aggregation matches sum of descendants.
- T7. Cmd-click on folder toggles all descendants.
- T8. Settings persistence across restart for all four keys
  (`file_list_mode`, `tree_compact_folders`,
  `hide_non_matches_when_filtering`, `group_renames_by_old_path`).
- T9. Tree construction performance: ≤16ms for 1000 changed files.
- T_filter_match_basename_only. Given files `app/src/foo/bar.rs`,
  `app/src/foo/baz.txt`, `unrelated/foo.rs`: filter `"foo"` matches ONLY
  `unrelated/foo.rs` (basename match). Filter `"src"` matches NOTHING
  (no basename contains `src`). Filter `"bar"` matches ONLY `bar.rs`.
- T_filter_no_match_on_folder_label. With a compact folder row labeled
  `app/src/foo/bar`, filter `"src"` does NOT mark that folder row as
  matched directly — the folder is only auto-expanded if a descendant
  file's basename matches.
- T_filter_match_rename_attribution_path. Renamed file
  `a/old.rs → b/new.rs` with `group_renames_by_old_path = false`:
  filter `"new"` MATCHES (basename of attribution path = `new.rs`);
  filter `"old"` does NOT match. With
  `group_renames_by_old_path = true`: filter `"old"` matches; filter
  `"new"` does not.
- T_filter_auto_expansion_transient. Tree mode, all folders collapsed.
  Type filter `"bar"` → ancestor folders of `bar.rs` auto-expand. Clear
  the filter → those folders revert to collapsed (cached base map
  unchanged). Re-type filter `"bar"` → same folders auto-expand again.
- T_filter_manual_expansion_persists. Tree mode, filter `"bar"` is
  active and auto-expanded folder `app/src/foo/`. User then manually
  expands a sibling folder `app/src/qux/` (no matching descendants).
  Clear the filter. Auto-expanded `app/src/foo/` reverts to collapsed,
  but manually-expanded `app/src/qux/` REMAINS expanded.
- T_filelist_module_split. After implementation, `code_review_view.rs`
  contains no file-row rendering code; the sidebar is rendered by a
  call into `app/src/code_review/file_list/`.
- T_hide_non_matches_default. Filter active,
  `hide_non_matches_when_filtering = false` → non-matches dimmed and
  visible in both Flat and Tree modes.
- T_hide_non_matches_on. Filter active,
  `hide_non_matches_when_filtering = true` → non-matches hidden; Tree-mode
  folders with no matching descendants are hidden, while folders with at
  least one matching descendant remain visible and auto-expanded.
- T_rename_grouping_default. Renamed file `a/old.rs → b/new.rs` with
  `group_renames_by_old_path = false` → row appears under `b/` with title
  `new.rs (was a/old.rs)`; counts/stats accrue only to `b/`, not `a/`.
- T_rename_grouping_toggled. Same renamed file with
  `group_renames_by_old_path = true` → row appears under `a/` with title
  `old.rs (now b/new.rs)`; counts/stats accrue only to `a/`, not `b/`.
- T_telemetry_pane_opened_flat. Open the Code Review pane with
  `code.review.file_list_mode = "flat"`; assert exactly one
  `CodeReview.PaneOpened` event is emitted with
  `file_list_mode == "flat"`.
- T_telemetry_pane_opened_tree. Same as above with
  `code.review.file_list_mode = "tree"`; assert
  `file_list_mode == "tree"`.
- T_telemetry_mode_toggle. Open in Flat, toggle to Tree, toggle back to
  Flat; assert exactly two `CodeReview.FileListModeToggled` events with
  payloads `{ from: "flat", to: "tree" }` then
  `{ from: "tree", to: "flat" }`.
- T_telemetry_filter_debounce. Type "fo", "foo", "foob" rapidly into the
  filter input; clear the input. Assert exactly ONE
  `CodeReview.FileListFiltered` event was emitted, with `mode` matching
  the active mode and `results_count` equal to the count after the last
  keystroke before clearing. Repeat with focus-loss as the terminator
  (instead of clearing); same single-event invariant.
- T_telemetry_filter_no_event_on_empty. Focus the filter input, type
  nothing, then blur. Assert NO `CodeReview.FileListFiltered` event
  was emitted.

## Open Questions

- Should folder expansion state persist across review sessions for the same
  PR? Suggested: yes, in V1.5, keyed by `(repo, pr_id, folder_path)`.
- Should folder rows expose a "review folder" affordance (mark all files in
  folder as reviewed)? Out of scope for V1.

## Telemetry

This spec extends the **existing** `CodeReview.PaneOpened` event and adds
two new events. (Earlier drafts referenced an event name that did not match
the current event taxonomy; the names below are the authoritative targets.)

- `CodeReview.PaneOpened` (existing, extended): add an optional
  `file_list_mode: "flat" | "tree"` field reflecting the active mode at the
  time the pane opened.
- `CodeReview.FileListModeToggled` (NEW): fired when the user toggles
  between Flat and Tree mid-session. Payload: `{ from: "flat" | "tree",
  to: "flat" | "tree" }`.
- `CodeReview.FileListFiltered` (NEW): fired when the new file-list filter
  (B6) is used. Debounced so it fires at most once per filter session
  (i.e. once per contiguous span of typing terminated by clear or focus
  loss). Payload: `{ mode: "flat" | "tree", results_count: number }`.

No other event types are introduced.
