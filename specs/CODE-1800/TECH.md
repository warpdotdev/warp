# TECH: Inline diff viewer for file-edit tool calls in the TUI (CODE-1800)

See [`PRODUCT.md`](./PRODUCT.md) for behavior. Code references are repo-relative `path:line` on this branch.

This branch builds the diff viewer on top of the core TUI editor element introduced by its parent branch (`specs/tui-editor-element/TECH.md`). That spec covers the element, the char-cell display-row surface, and why structural overlays (gutter, ghost rows, hidden ranges) are core-element mechanisms configured by consumers; this spec covers only the diff feature itself: the per-file view, the model pipeline that feeds it, the transcript integration, and two shared-model fixes the feature surfaced.

## Context

Diff data already reaches the TUI. `RequestFileEditsExecutor::preprocess_action` (`app/src/ai/blocklist/action_model/execute/request_file_edits.rs`) resolves the tool args once, atomically, into `Vec<FileDiff>` (`app/src/ai/blocklist/diff_types.rs:27`) — full pre-edit content (`DiffBase`) plus line-range `DiffDelta`s per file (`Create`/`Update{rename}`/`Delete`) — and delivers them to the surface-registered storage. In the TUI that is `TuiDiffStorage` (`crates/warp_tui/src/tui_diff_storage.rs`), owned by `TuiFileEditsView` (`crates/warp_tui/src/tui_file_edits_view.rs`), created per `RequestFileEdits` action by `TuiAIBlock::sync_action_views` (`crates/warp_tui/src/agent_block.rs`). Resolution is one-shot at preprocess time, so the body appears fully formed when diffs land; streaming diffs during arg generation is out of scope (PRODUCT non-goal).

Before this branch, `TuiFileEditsView` rendered a one-line aggregate summary ("Edited N file(s) (+a −r)").

## Architecture

`TuiFileEditsView` is the TUI analogue of the GUI's `EditorWrapper`: pure policy and chrome over the core editor element. All diff semantics come from the existing `CodeEditorModel`/`DiffModel` pipeline; all render data (ghost rows, hidden ranges, decorations) flows model → render state → element. The view never walks diff hunks, never computes hidden ranges, never builds rows.

### Per-file model pipeline

For each `FileDiff`, the view runs the whole setup fire-and-forget at seeding time; the model completes everything itself when the async diff computation lands:

```
CodeEditorModel::new_tui
  reset_content(base content)              // buffer = pre-edit, doubles as DiffModel base
  apply_diffs(deltas)                      // model.rs — buffer becomes post-edit, diff recomputes
  hide_lines_outside_of_active_diff(3)     // hunk ± 3 context, model-side (see fixes below)
  expand_diffs()                           // refresh_diff_state pushes render data when the diff lands
```

The seeding mirrors the GUI's `CodeDiffView` (`reset_content` → `apply_diffs`); when the diff computes, `refresh_diff_state` pushes removed-line ghost blocks into the char-cell render state, the model recalculates hidden line ranges (hunks ± context), and the delayed-rendering flush re-runs the refresh under the expanded state — no further consumer involvement. The view's `DiffUpdated` subscription is pure bookkeeping: mark header counts ready, re-render.

### Shared-model fixes (`app/src/code/editor/model.rs`)

The TUI is the first consumer combining delta-seeded content with `DiffModel`-driven hunk-context hiding — the GUI either loads content from disk (code review: a `ContentReplaced` triggers recalculation), precomputes hidden ranges view-side (code review's deleted-file path, `app/src/code_review/hidden_lines.rs:30`), or doesn't elide at all (the GUI `RequestFileEdits` inline diff renders full content in a capped scrollable). Two model changes were required:

- **A unified recalculation trigger.** Every reason to recalculate hidden lines — enabling hiding, replacing content, changing the diff base, or editing an existing hidden range — now calls `request_hidden_lines_recalculation_after_diff(buffer_version)`. `DiffModel` only reports the version it computed; the editor recalculates when that version reaches the latest requested version, then clears the request. This replaces the former mix of a boolean carried through each diff computation and a separate one-shot request. Version gating prevents an in-flight stale diff (such as the seeding reset's empty diff) from consuming the request early, while coalescing repeated requests preserves the latest required content version.
- **An off-by-one fix in `calculate_hidden_lines`.** It subtracted 1 from `modified_lines()` ranges under a comment claiming they are 1-indexed; they are 0-based, so every window rendered `context+1` lines above and `context−1` below each hunk. Empirically confirmed on the unmodified GUI pipeline and fixed; windows are now exactly `± context_lines` (GUI code review shifts one whole context line from above each hunk to below it). Regression test: `test_hidden_lines_window_is_symmetric_around_changes`.

The `delay_rendering` interplay is validated end-to-end by `test_char_cell_diff_pipeline_populates_ghosts_and_hidden_ranges` (`app/src/code/editor/model_tests.rs`): the exact wrapper sequence, asserting ghosts and hidden ranges land with no consumer calls after setup.

### The view (`crates/warp_tui/src/tui_file_edits_view.rs`)

- Renders the core element per file: no `.editable()`/`.on_action` (read-only, PRODUCT 18), `.with_gutter(...)` (new-file numbers, PRODUCT 9/11), `.hide_trailing_empty_line()` (PRODUCT 7), and diff styles — context dim, added green via line classification from `DiffModel::added_or_changed_lines`, ghost red, gap dim (PRODUCT 10; styles from `tui_builder.rs`'s new `diff_added_style`/`diff_removed_style`).
- Owns chrome: header row (state glyph per `tool_call_labels.rs` conventions, verb from `DiffType`, `+a −r` from `diff_status().get_diff_lines()` so header and body derive from the same computed diff — PRODUCT 2-6), per-file collapse (`MouseStateHandle` + click, caret `▾`/`▸`, PRODUCT 15-16), blank-row spacing between files.
- Multi-file aggregation (PRODUCT 21-23): a single generic `render_section` renders every collapsible level — keyed header (`SectionKey::Summary | File(index)` into one shared collapse/hover state map) over a lazily built body. Two-plus files render as one summary section (`Edited N files`, counts summed once every file's diff is ready) whose body is the indented per-file column; one file renders the per-file column directly.
- Fallback: when the storage was never seeded (failed/cancelled actions, restored conversations), a single aggregate label from the action's recorded result (PRODUCT 19); zero-change diffs render `+0 −0`, no body or caret (PRODUCT 14).

### Transcript integration

Per-file bodies must measure and refresh correctly as ordinary transcript content (PRODUCT 17):

- Agent-block roots resolve through the presenter cache: the viewport source emits `TuiChildView` nodes for agent blocks in both measurement and render paths (mirroring the GUI, where rich-content roots are `ChildView`s), so unchanged blocks are not re-rendered per frame. Measurement threads the live `TuiLayoutContext` through `TuiViewportedElement::visible_items` so `TuiChildView` children measure real heights (`tui_block_list_viewport_source.rs`, `viewported_list.rs`).
- Staleness is prevented by routing invalidations to block views: `TuiAIBlock` notifies itself on exchange output updates, and `TuiTranscriptView::notify_action_owner` notifies the owning block on action-status transitions and on `ModelEvent::AfterBlockStarted`/`BlockCompleted` for agent-requested command blocks (command rows read the terminal block's ground truth at render time).

## Testing and validation

- `app/src/code/editor/model_tests.rs`: the fully-automatic char-cell pipeline test and the symmetric hidden-lines window regression test (both above).
- `tui_file_edits_view_tests.rs`: header verb/name per `DiffType` incl. renames (PRODUCT 1-3), delta extraction per op, end-to-end pipeline (seed → apply → diff → expand → assert added ranges + ghost blocks), fallback states (PRODUCT 14/19).
- Transcript/viewport tests: presenter-cache resolution of agent blocks, height re-measurement, invalidation routing.
- Suites: `cargo nextest run -p warp_tui -p warp_editor` plus `cargo nextest run -p warp code::editor code_review`; `./script/format` and the presubmit clippy invocation before PR.
- Manual verification in the TUI binary: single/multi-file edits, multi-hunk separators, long wrapped lines, create/delete/rename, collapse toggling, failed edit, theme switch (PRODUCT 17-20).

## Risks and mitigations

- Two shared-model behavior changes are deliberate and GUI-visible: the ±context off-by-one fix (each code-review hunk shows one less context line above, one more below — now matching the configured context exactly) and the one-shot recalculation (a no-op in existing GUI flows). Both are covered by model tests; the full GUI editor/code-review suites pass.
- Pathological diffs (full-file rewrites) render all changed lines by design (PRODUCT 8); a cap can be added later without architectural change.
