# File Tree Search (GH-10320)

## Summary

Add an in-tree search box to the File Tree / Project Explorer, openable via
`Cmd+F` (`Ctrl+F` on Windows/Linux) when the File Tree has focus, and via a
discoverable magnifier button in the tree header. Live-filters the tree to
filename and path-component matches, auto-expands folders containing matches,
and supports next/previous-match navigation. Matches VS Code's Explorer
search behavior at the V1 level.

## Problem

The File Tree currently has no in-panel search. Users with large repos must
scroll or use the global Find (which is content search, not filename) to
locate files in the tree. The expected interaction is `Cmd+F` while the
File Tree is focused, plus a visible magnifier button — both consistent with
VS Code's Explorer.

## Goals

- In-tree search box accessible via `Cmd+F` when the tree has focus.
- Header magnifier button as a discoverable affordance for the same search.
- Live-filter the tree to nodes matching filename or relative-path
  components.
- Auto-expand folders that contain matches.
- Jump-to-match navigation (next / previous).
- Preserve prior selection and prior expand/collapse state when search is
  cleared.

## Non-Goals

- No full-text content search; the existing global Find continues to handle
  that.
- No regex or glob matching in V1; deferred to V1.5 (see Open Questions).
- No cross-workspace search.
- No fuzzy code-symbol search; this is filename / path search only.

## Behavior Contract

### B1. Activation

When the File Tree is the active focus context:

- `Cmd+F` (macOS) / `Ctrl+F` (Windows/Linux) opens a search box pinned at
  the top of the tree panel.
- A magnifier icon button in the tree header opens the same search box.
- `Esc` closes the search box and restores focus to the previously focused
  tree row.

The shortcut MUST NOT trigger the File Tree search when focus is in the
terminal, an editor, or other panes — those keep their existing `Cmd+F`
behaviors.

### B2. Match semantics

- Case-insensitive substring match.
- **Match target is the row's own name only — NOT the full relative path
  string.** Concretely: for a file row, the substring match runs against
  the **file basename** (e.g. `a.rs`). For a folder row, the substring
  match runs against the **folder name** (e.g. `src`). The full relative
  path (e.g. `src/a.rs`) is NEVER used as a match target. This avoids the
  ambiguity in which a file `src/a.rs` would otherwise "directly match"
  the query `src` purely because its ancestor's name appears in its full
  path; under this contract, that case is unambiguously a folder match
  on `src` with `a.rs` included as a folder-match descendant per B2a —
  it is NOT a direct match on `a.rs`. Earlier drafts that said "matched
  against … the full relative path string" are superseded by this rule;
  B2a remains the single source of truth for how descendants are pulled
  into the match set.
- **Direct match (per row).** A row is a "direct match" if and only if
  the query is a substring of that row's OWN name (folder name for
  folder rows; basename for file rows). Direct matches receive substring
  highlighting (per B2a.3) and are counted exactly once in the match
  total (per B2a.2).
- **Folder-match-descendant (per row).** Any descendant (file or folder)
  of a directly-matched folder is included in the **union match set** as
  a folder-match-descendant, regardless of whether the descendant's own
  name matches the query. Folder-match-descendants are visible (per
  B2a.1), counted once (per B2a.2), and do NOT receive highlighting
  unless their own names also directly match (per B2a.3). See B2a for
  the full descendant contract.
- **Union match set.** The set of "matching rows" used for visibility
  (B3), counting (B2a.2), highlighting (B2a.3), and navigation order
  (B2a.4) is the UNION of (a) direct matches and (b) folder-match
  descendants, with each row appearing at most once. This same wording
  is reused verbatim by the Settings / API surface (B6) so implementers
  read the contract identically there.
- Matching is performed after NFC normalization, then full Unicode default
  case folding. See B2b for the precise Unicode rules.

### B2a. Folder-match descendant semantics

When a folder name matches the query, the folder is itself a match and its
**descendants are included** in the match set as "in-context" matches. The
following rules govern visibility, counting, highlighting, and navigation
of folder-match descendants. They apply consistently in both visibility
modes (B3 dim mode and `dim_non_matches = false` hide mode).

#### B2a.1. Visibility

- **Dim mode** (`file_tree.search.dim_non_matches = true`, default): the
  matched folder and ALL its descendants render at FULL emphasis (NOT
  dimmed) — they are part of the match set. Other (non-matching, non-
  descendant) tree nodes follow the normal dim rules in B3.
- **Hide mode** (`file_tree.search.dim_non_matches = false`): the matched
  folder and ALL its descendants render normally (visible). They are NOT
  hidden, regardless of whether each individual descendant's name matches
  the query.

#### B2a.2. Match count (no double-count)

A descendant counted via folder-match is NOT additionally counted if its
own name also matches the query. Counting is the **union** of the direct-
match set and the folder-match-descendant set, not their sum:

- A direct file match counts once.
- A direct folder match counts once for the folder itself; its descendants
  each count once via the folder-match-descendant rule.
- If a descendant's own name also matches the query, it still counts
  exactly once.

Total match count rendered next to the search box (B3) reflects this
union cardinality.

#### B2a.3. Highlighting

- The matched folder name itself is highlighted (the matched substring is
  emphasized in the rendered name).
- Descendant rows brought in via folder-match are NOT individually
  highlighted — they appear at full emphasis (per B2a.1) but no character-
  range highlight is rendered on their names. (If a descendant's own name
  ALSO directly matches the query, its name IS highlighted as a direct
  match.) This prevents descendants from appearing as if they had directly
  matched the query.

#### B2a.4. Navigation

`F3` / `Enter` (B4) cycles through the **union** match set: direct file
matches AND descendants of folder matches (with the no-double-count rule
in B2a.2 applied so each row is visited at most once per cycle). The cycle
order is **depth-first, lexicographic** by path. Wraparound semantics are
unchanged from B4.

### B2b. Unicode normalization and case folding

Both the query string and each candidate target string (path component or
full relative path) are processed identically before substring matching:

1. **NFC normalization** is applied to both query and target.
2. **Unicode Default Case Folding (Unicode TR#21)** is then applied. This
   is **full case folding** — i.e., uses the Unicode case folding mappings
   with status `C` (common) PLUS status `F` (full). It is locale-
   independent.
   - **Implementation contract — DO NOT use `str::to_lowercase()`.**
     Lowercasing and case folding are different operations: case folding
     applies the `CaseFolding.txt` C+F mappings (e.g. `ß` → `ss`,
     `ﬃ` → `ffi`), while `str::to_lowercase()` applies the
     `SpecialCasing.txt` lowercase mappings, which preserve `ß` as `ß`
     and do not collapse final-sigma `ς` and middle sigma `σ` to a
     single form. An earlier draft of this spec said `to_lowercase()`
     was equivalent — that was wrong, and would break the `ß` and sigma
     examples below.
   - **Required implementation.** Use a Unicode case-folding crate that
     implements C+F default folding (e.g. `caseless::default_caseless_match_str`
     or `caseless::Caseless::default_case_fold`) on the NFC-normalized
     strings. If the chosen crate already performs an internal
     normalization step compatible with NFC, the explicit NFC pass MAY
     be elided; otherwise it is required. The implementation MUST NOT
     substitute `str::to_lowercase()` even as a fallback.
   - **Test gate.** Implementations MUST pass T_unicode_german_ss and
     T_unicode_greek_sigma; passing these tests is the authoritative
     check that the right folding is being applied.
   - Special locale-specific case mappings (notably the Turkish dotted
     and dotless I, Azerbaijani, etc.) are **NOT** applied. The folding
     is locale-independent default folding only.

Notable consequences:

- German `ß` (U+00DF) folds to `ss`. Therefore the query `MAUSS` matches
  the file `Mauß.txt`, and the query `mauss` likewise matches.
- Greek final-form sigma `ς` (U+03C2), middle/initial sigma `σ` (U+03C3),
  and capital `Σ` (U+03A3) all fold to the same lowercase form under
  default case folding. Therefore the query `Σ` matches `ς` and `σ`.
- Turkish I/ı/İ/i: default case folding maps `I` → `i` and `İ` → `i̇`
  (i + combining dot above) per Unicode default rules; the Turkish-
  locale-specific mapping `I` → `ı` and `İ` → `i` is NOT applied. Users
  who require Turkish-specific folding must obtain it via an explicit
  future setting; V1 uses default folding only.

Both NFC and case folding are applied once per keystroke to the query and
once per (re-)indexed target string; matching is then byte-by-byte
substring comparison on the folded forms.

### B3. Live filter

The tree updates as the user types:

- Folders containing matches auto-expand.
- The visibility of non-matching siblings is controlled by the boolean
  setting `file_tree.search.dim_non_matches` (default `true`):
  - When `true` (default): non-matching siblings remain visible but
    **dimmed** (de-emphasized) so the user keeps spatial context.
  - When `false`: non-matching siblings are **hidden** entirely. The
    set of rendered nodes in this mode is:
    1. every node in the **union match set** (direct file/folder
       matches + descendants of folder matches per B2a), AND
    2. every **ancestor folder** of any node in (1), auto-expanded
       so the matched rows are reachable.

    Concretely: a folder match keeps **ALL** of its descendants
    visible (because they are in the union match set per B2a), even
    if those descendants' own names do not match the query. This is
    NOT in conflict with the "only matching files plus ancestors
    render" wording — the union match set INCLUDES folder-match
    descendants, so they qualify as "matching" for B3's hide rule.
    Earlier drafts that read "only filename matches plus their
    ancestors" are superseded by this rule; B2a's union semantics is
    the authoritative match set.
- A match count appears next to the search box (e.g. "12 matches").

### B4. Navigation

- `F3` or `Enter` jumps to the next match.
- `Shift+F3` or `Shift+Enter` jumps to the previous match.
- The active match is visually highlighted distinctly from other matches.
- The active match scrolls into view if not already visible.
- Wraps from last → first and first → last.

### B5. Clear and restore

Clearing the search input (deleting all characters) restores:

- the prior tree expand/collapse state, and
- the prior selected row.

Closing the search box via `Esc` does the same.

### B6. Empty match

When the query yields no matches:

- A "No matches" indicator appears next to the search box.
- With `file_tree.search.dim_non_matches = true` (default): the tree is
  fully dimmed.
- With `file_tree.search.dim_non_matches = false`: the tree renders empty
  (only the panel chrome remains visible).

### B7. Persistence

The search query is NOT persisted across closing and reopening the tree
panel. Each panel-open starts with an empty search.

## Settings / API surface

- `file_tree.search.dim_non_matches` — `bool`, default `true`. When
  `true`, rows NOT in the **union match set** (direct matches +
  folder-match descendants per B2 / B2a) remain visible but **dimmed**
  (de-emphasized) so the user keeps spatial context. When `false`, rows
  NOT in the union match set are **hidden entirely**; the rendered tree
  in this mode is exactly: (1) every row in the union match set (which
  INCLUDES descendants of any directly-matched folder, even if those
  descendants' own names do not match the query — per B2a) PLUS (2)
  every ancestor folder of any row in (1), auto-expanded so the
  matched rows are reachable. This summary intentionally uses the same
  "union match set" wording as B2 / B2a so implementers do not read
  hide mode as filename-only filtering.
- Keybinding registration for `Cmd+F` (mac) / `Ctrl+F` (win/linux), scoped
  to a new keymap context flag `"FileTreeFocused"` (added via
  `context.set.insert("FileTreeFocused")` in the file-tree view's
  `keymap_context` impl) so it does not collide with editor or terminal
  `Cmd+F`.

## Acceptance Criteria

- A1. `Cmd+F` opens the search box when the File Tree has focus.
- A2. The magnifier button in the tree header opens the search box.
- A3. Live-filter updates per keystroke within ≤16ms for a 5,000-node tree.
- A4. Folders auto-expand to reveal matches as the user types.
- A5. Setting `file_tree.search.dim_non_matches` switches between dimming
  (`true`, default — non-matches visible but dimmed) and hiding (`false`
  — non-matches hidden entirely).
- A6. `Enter` / `F3` cycles to the next match; `Shift+Enter` / `Shift+F3`
  cycles to the previous; wraps at the ends.
- A7. `Esc` closes the search and restores prior focus and state.
- A8. Clearing the input restores the prior selection and expand state.
- A9. The match count is visible while the search box is open.
- A_folder_match_visibility_dim. Folder name matches in dim mode
  (`dim_non_matches = true`) → matched folder and ALL its descendants
  render at full emphasis; descendants are NOT dimmed (per B2a.1).
- A_folder_match_visibility_hide. Folder name matches in hide mode
  (`dim_non_matches = false`) → matched folder and ALL its descendants
  remain visible regardless of whether their individual names match (per
  B2a.1).
- A_folder_match_count_union. The match count reflects the **union** of
  direct matches and folder-match descendants, with each row counted at
  most once even if it qualifies via both rules (per B2a.2).
- A_folder_match_highlight. Only the matched folder name itself receives
  substring highlighting; descendant rows do NOT receive direct-match
  highlighting unless their own names also match (per B2a.3).
- A_folder_match_navigation. `F3` / `Enter` cycles through the union match
  set in depth-first lexicographic order, visiting each row at most once
  per cycle (per B2a.4).
- A_unicode_default_folding. Case folding is Unicode Default Case Folding
  (TR#21, status `C` + `F`) applied AFTER NFC normalization, locale-
  independent (per B2b). Examples: query `MAUSS` matches `Mauß`; query
  `Σ` matches `ς` and `σ`; Turkish-locale-specific I↔ı mapping is NOT
  applied.

## Implementation Pointers

> Paths verified against worktree at commit `86940541`. If reorganizing,
> update tooling accordingly (no behavior change).

- **New module** `app/src/code/file_tree/search.rs` for the matching
  pipeline (substring match, NFC normalization, ancestor expansion
  tracking) and match-list state.
- **Update existing view** `app/src/code/file_tree/view.rs`
  (`FileTreeView`, ~3,170 lines) for the filter + highlight render
  pipeline, auto-expand on match, prior-state snapshot, and search-box
  state hookup. The render path lives in
  `app/src/code/file_tree/view/render.rs`.
- **Magnifier button**: there is no separate `header.rs` today — header
  rendering is inlined inside `app/src/code/file_tree/view.rs` (search for
  the existing toolbar / header row; the magnifier button is added as a
  new icon child of that row).
- **Container view**: `app/src/workspace/view/left_panel.rs`
  (`LeftPanelView`) hosts the `FileTreeView`; no changes there beyond
  forwarding focus state if needed.
- **Keymap context**: register the new context-set flag
  `"FileTreeFocused"` via the `keymap_context` impl associated with
  `FileTreeView` (pattern: `context.set.insert("FileTreeFocused")` —
  consistent with existing flags like `"EditorFocused"` set in
  `app/src/terminal/view.rs:26640`). Bind `Cmd+F` / `Ctrl+F` to the new
  search action under predicate `id!("FileTreeFocused")` so it does not
  collide with editor / terminal `Cmd+F`.
- **Snapshot prior state**: snapshot expand/collapse state and selected
  row when the search box opens; restore on `Esc` / clear (per B5). Reuse
  the existing `FileTreeView` expansion-state container.
- **Settings**: add `file_tree.search.dim_non_matches: bool` (default
  `true`) to the file-tree settings group. Surface a toggle in
  `app/src/settings_view/code_page.rs` under the file-tree section
  (there is no separate `editor_page.rs`; `code_page.rs` houses
  code/file-tree settings).
- **No persistence migration** — the new setting is additive with a
  default.

## Tests

- T1. `Cmd+F` with file-tree focus opens the search box; with terminal /
  editor focus it does not.
- T2. Substring match across both file basenames and folder names.
- T3. Auto-expand of all ancestor folders on a match.
- T4. Next / previous navigation cycles, including wraparound.
- T5. `Esc` restores prior focus and tree state.
- T6. Setting `file_tree.search.dim_non_matches` switches between dimming
  (`true`) and hiding (`false`) behavior. Default value is `true`.
- T7. Clearing the input restores the prior selection and expand state.
- T8. Performance: live filter updates ≤16ms per keystroke on a
  5,000-node tree.
- T9. Unicode + locale-insensitive matching: NFC normalization is applied
  to both query and target, then full Unicode Default Case Folding (TR#21,
  status `C` + `F`).
- T_unicode_german_ss. Tree contains `Mauß.txt`. Query `MAUSS` matches
  it; query `mauss` matches it; query `Mauß` matches it. (Default folding:
  `ß` → `ss`.)
- T_unicode_greek_sigma. Tree contains `final_ς.txt` and `middle_σ.rs`.
  Query `Σ` (capital sigma) matches both. Query `σ` matches both. Query
  `ς` (final-form sigma) matches both. (Default folding maps all three to
  the same lowercased form.)
- T_unicode_turkish_default_folding. Tree contains `İstanbul.txt` and
  `istanbul.rs`. With default folding (locale-independent), query
  `istanbul` matches `istanbul.rs`; query `i̇stanbul` (i + combining dot
  above, the default-folded form of `İ`) matches `İstanbul.txt`. The
  query `istanbul` does NOT match `İstanbul.txt` (Turkish-locale rule
  `İ`→`i` is NOT applied), confirming locale-independent behavior.
- T_folder_match_dim_visibility. With `dim_non_matches = true` (default),
  query `src` against tree `src/{a.rs, b.rs}, other/{c.rs}` → folder
  `src` matches; both `a.rs` and `b.rs` render at full emphasis (NOT
  dimmed); `other/` and `c.rs` are dimmed.
- T_folder_match_hide_visibility. With `dim_non_matches = false`, the
  same query and tree → `src/{a.rs, b.rs}` are visible; `other/` and
  `c.rs` are hidden.
- T_folder_match_count_union. Query `src` against tree
  `src/{a.rs, src_helper.rs}` → match count is 3 (folder `src` itself,
  plus descendants `a.rs` and `src_helper.rs`). `src_helper.rs` is
  counted once, NOT twice (once as folder-match-descendant, once as
  direct match).
- T_folder_match_highlight. Same tree as above → folder `src` row shows
  `src` highlighted; `a.rs` row renders at full emphasis without any
  highlight; `src_helper.rs` row shows `src` highlighted within its name
  (because its own name directly matches).
- T_folder_match_navigation_order. Tree
  `proj/{src/{a.rs, b.rs}, tests/{a.rs}}`, query `src` → cycle order
  with `F3`: folder `src` itself, then `src/a.rs`, then `src/b.rs`. (No
  `tests/a.rs` — it is not in the union match set.)

## Open Questions

- Should results also support diff / git status filters (e.g. "show only
  modified files")? Suggested: V1.5 as a separate filter row, not part of
  the substring search.
- Should the search support a `path:` prefix to scope matching to path
  components only? Deferred.

## Telemetry

The actual existing telemetry event for file-tree open/close is
`TelemetryEvent::FileTreeToggled` (label "File Tree Toggled" — see
`app/src/server/telemetry/events.rs:5658`); there is no
`file_tree.opened` event today.

Two options, in order of preference:

1. **Preferred — extend the existing `FileTreeToggled` variant** in
   `app/src/server/telemetry/events.rs` with an optional
   `search_used: bool` field that is `true` if the user opened the
   search box at any point during the tree-panel session. No new event
   type introduced.
2. **Alternative — add a net-new event** `FileTreeSearchUsed` (label
   `"FileTree.SearchUsed"`) emitted once per tree-panel session if the
   user opened the search box. Use this only if extending
   `FileTreeToggled` is awkward for the existing call sites.

Pick option 1 unless implementation-time review shows the existing
variant is consumed in a way that makes adding the field risky.
