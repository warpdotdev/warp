# APP-3632: Code Review Header UI Refactor

## Problem
The updated Figma designs for the code review panel introduce git operation buttons (commit, push, create PR) in the inner header. The existing header layout doesn't have room for these — it already contains the branch name, diff stats, discard button, add-context button, and diff mode dropdown. This PR clears space by relocating contextual info upward and consolidating actions into the overflow menu.

## Feature Flag
All UI changes are gated behind `FeatureFlag::GitOperationsInCodeReview`. The flag is **not** in `DOGFOOD_FLAGS` — it is off everywhere by default. Both the inner header (`code_review_header.rs`) and the right panel header (`right_panel.rs`) maintain legacy render paths that match master when the flag is off.

To test locally: `cargo run --features git_operations_in_code_review`

## Overview of Changes
When the flag is **on**, this PR restructures the code review header into two layers:

1. **Right panel header** (top-level): shows repo context — repo path, branch name, and diff stats
2. **Inner code review header**: simplified to just the diff mode selector, file nav button, and an overflow menu

Actions that were previously standalone buttons (discard all, add diff set as context) are consolidated into the overflow menu. The file navigation toggle button is now an `ActionButton` with `PaneHeaderTheme` that appears in both wide and compact layouts.

When the flag is **off**, both headers render identically to master.

## File-by-File Changes

### `app/src/workspace/view/right_panel.rs`
**Purpose**: Redesign the panel header to show contextual git info (flag-gated).

- Call site dispatches to `render_header` (flag on) or `render_header_legacy` (flag off)
- **New layout** replaces the static "Code review" title with:
  - **Repo path** — tilde-shortened (e.g. `~/Repos/warp-internal:`), rendered in semibold sub-text color
  - **Branch name** — read from `DiffStateModel` via `get_diff_state_model()`
  - **Diff stats** — read from `CodeReviewView::loaded_diff_stats()`
- Uses shared `CONTENT_LEFT_MARGIN` / `CONTENT_RIGHT_MARGIN` constants so the header aligns with the content area below
- **Legacy layout** preserves the "Code review" title with `PANE_HEADER_HEIGHT` and `HEADER_EDGE_PADDING`

### `app/src/code_review/code_review_header.rs`
**Purpose**: Simplify the inner header to only layout concerns, with legacy fallback.

- Master's code (`render`, `render_wide_layout`, `render_compact_layout`, and all helpers) is **untouched** — zero deletions from master's version aside from adding `FilterableDropdown` to imports.
- **New path (`render_new`)**: added at the bottom of the file. Renders diff mode dropdown (left) + git operations button + file nav button + overflow menu (right). Compact layout is a single row.
- `render_header` in `CodeReviewView` checks the flag and calls `render_new` or `render`

### `app/src/code_review/code_review_view.rs`
**Purpose**: Restyle the dropdown, relocate buttons, consolidate menu items.

- **File navigation button**: `ViewHandle<ActionButton>` with `PaneHeaderTheme`, created once in `new()`. Passed to the header via `CodeReviewHeaderFields.file_nav_button` so both wide and compact layouts can render it. Tooltip updates dynamically when sidebar state changes.
- **Diff mode dropdown**: flag-gated styling in `new()`. Flag on: `ButtonVariant::Text` with semibold larger font. Flag off: master's bordered outline/accent style.
- **`header_menu_items()`**: dispatches to `header_menu_items_new` (flag on) or `header_menu_items_legacy` (flag off, matches master). New version adds "Discard all" and `AISettings` check.
- **`loaded_diff_stats()`**: new public accessor for the right panel header
- **Shared margin constants**: `CONTENT_LEFT_MARGIN` (16px) and `CONTENT_RIGHT_MARGIN` (4px) exported as `pub(crate)`
- **`render_header`**: takes `state` and `app` params, dispatches to `render_new` or `render` based on flag
- **`render_file_navigation_button`**: public function retained from master (used by legacy right panel header)

### `app/src/pane_group/working_directories.rs`
**Purpose**: Expose diff state for the panel header to read.

- Adds `get_diff_state_model(&self, repo_path: &Path) -> Option<ModelHandle<DiffStateModel>>`

### `app/src/lib.rs`
- Wired `git_operations_in_code_review` cargo feature to `FeatureFlag::GitOperationsInCodeReview` runtime flag

### `crates/warp_features/src/lib.rs`
- `GitOperationsInCodeReview` removed from `DOGFOOD_FLAGS` (flag is off everywhere by default)

## Design Decisions

- **Dual render paths behind feature flag**: the header restructuring is a prerequisite for git operation buttons (added in child branches). Since this PR may merge before the rest of the stack, both old and new rendering coexist. The legacy code is clearly marked and will be deleted when the flag is promoted.
- **File nav button as `ActionButton`**: uses `PaneHeaderTheme` to match the three-dots and maximize buttons. Created as a `ViewHandle` in `new()` (not inline during render) so it can appear in both wide and compact layouts via `ChildView`.
- **Branch name reads from `DiffStateModel` directly** rather than being passed through `CodeReviewView`, because the panel header renders independently of whether diffs have loaded.
- **Diff stats still read from `CodeReviewView::loaded_diff_stats()`** because they depend on the loaded diff state, which only `CodeReviewView` owns.
- **Overflow menu is always rendered** (no longer gated on `FileAndDiffSetComments`). Individual items are independently gated, so the menu gracefully degrades to empty when all flags are off.
