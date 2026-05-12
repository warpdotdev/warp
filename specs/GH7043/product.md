# Side-by-Side Diff Layout in the Code Review Pane - Product Spec
GitHub issue: https://github.com/warpdotdev/warp/issues/7043
Roadmap reference: https://github.com/warpdotdev/warp/issues/9233 ("Improved code review - Side by side diffs", listed under "Seeking community drivers")
Figma: none provided

## Summary
Add a side-by-side diff layout as an alternative to the existing inline diff in the Code Review pane only. The user picks the layout in Settings -> Code under a new "Diff layout" subsection, and the choice is persisted as a synced `code.editor.diff_layout` setting. Inline remains the default. Side-by-side renders the baseline on the left and the modified file on the right, with hunk-aligned padding so corresponding lines sit on the same row.

V1 applies only to Code Review. AI block-list diffs, inline banner diffs, and settings-menu surfacing for those surfaces are explicitly deferred to v2.

## Problem
Warp renders Code Review file diffs as a single-column inline view: deletions on `-` rows, additions on `+` rows, both interleaved. The Code Review pane consumes editor and diff primitives through `LocalCodeEditorView`, `CodeReviewEditorState`, and `CodeEditorView`, and currently has no layout knob.

Two real costs follow from that:

- Side-by-side is the de-facto standard for diff review across GitHub, GitLab, Phabricator, JetBrains IDEs, VS Code, and tools like Beyond Compare. Users with wide displays expect it, and the absence shows up repeatedly in user feedback, including the request that opened this issue: "since we aren't writing the code ourselves," the reviewer needs to read both versions in parallel rather than reconstructing them mentally from a single column.
- Warp's review surface is shaped by AI-generated edits more than by hand-written commits. An AI agent regularly produces diffs that touch dozens of unrelated regions in one block. In an inline layout, a 30-line modification with reordered code is hard to follow, because the same logical block appears as `-` lines in one place and `+` lines in another. Side-by-side puts those next to each other.

The two related open issues - #9017 (word wrap in diff and Markdown) and #9040 (auto line wrap in diff) - assume a single diff layout and argue about wrapping inside it. They are out of scope for this spec but become more useful once a side-by-side layout exists, since wrap behavior interacts with column width.

## Goals
- Introduce a `DiffLayout` choice with two values: `Inline` (today's behavior) and `SideBySide`.
- Default `DiffLayout` to `Inline` for every existing user; the change is opt-in.
- Surface the choice in Settings -> Code under a new "Diff layout" subsection for the Code Review pane.
- Persist the choice as a synced user setting `code.editor.diff_layout` so it carries across sessions and machines.
- Apply the chosen layout to the Code Review pane only in v1.
- In `SideBySide`, render the baseline on the left, the modified file on the right, with hunk-aligned padding so that unchanged context lines, modifications, and pure additions or deletions all sit at the same vertical position on both sides.
- Keep edits in the diff constrained to the right, modified pane. The left, baseline pane is never editable.
- Keep the text cursor in the right, modified pane only. The left pane does not expose an insertion cursor.
- Allow selection on the left pane for copy, including deleted-line ranges.
- Preserve every existing Code Review diff feature in both layouts: hunk navigation (`f`/`F`), accept and reject, save, revert to base, comment threads, hidden lines, find-in-diff, and the existing nav bar.
- Keep `Inline` byte-for-byte identical to today. No regression to the default code path.

## Non-goals
- Applying side-by-side layout to AI block-list diffs in v1. That surface is deferred to v2.
- Applying side-by-side layout to inline banner diffs in v1. That surface is deferred to v2.
- Adding settings-menu surfacing for AI block-list or inline banner diffs in v1.
- Word-level or character-level diff highlighting on changed lines. Issue #9017 and #9040 are the natural follow-up specs for that.
- A vertical "stacked" layout (baseline on top, modified below). The roadmap and the issue specifically ask for side-by-side; a stacked variant could be a separate spec.
- Cross-pane selection. Selection in `SideBySide` is per-pane, matching GitHub Desktop and GitLab MR review.
- Per-pane width control. The split is fixed at 50/50 in this change; resizable splits can follow.
- A separate layout choice per surface. V1 has only one participating surface: Code Review.
- Mobile/wasm-only behavior changes beyond what falls out naturally from layout symmetry.
- Changing the existing `DiffMode` enum (`Head` / `MainBranch` / `OtherBranch(String)`) in `app/src/code_review/diff_state.rs:266`. That enum controls *what* is being compared; the new `DiffLayout` enum controls *how* the comparison is rendered. The two are orthogonal.

## Behavior

1. When `code.editor.diff_layout` is unset or `inline`, Code Review diffs in Warp render identically to today. No regression to the default form.

2. When `code.editor.diff_layout` is `side_by_side`, Code Review renders with the baseline on the left and the modified file on the right.
   - The split is a 50/50 vertical split with a single 1-pixel divider in the panel chrome.
   - The two panes share vertical scroll position and inherit horizontal scrollbars independently, since wrap-vs-no-wrap behavior is per-pane.
   - The baseline pane shows the file content prior to the diff, with deletions visible. The modified pane shows the post-diff file content, with additions visible. Unchanged context lines appear in both panes at the same vertical position.
   - AI block-list diffs and inline banner diffs continue to render inline in v1 regardless of the stored setting.

3. Hunk-aligned padding keeps corresponding lines aligned across panes. The alignment algorithm pairs deleted lines with added lines within a hunk so that a modification renders on a single shared row across the two panes:
   - Unchanged lines: rendered at the same row on both sides with the same content.
   - Modifications: within each hunk, deleted lines and added lines are paired in order. For a hunk of `D` deleted lines followed by `A` added lines, the first `min(D, A)` pairs render on shared rows: the deleted line on the left at row N, the added line on the right at row N. The shared row is what makes "before/after" review readable.
   - Excess deletions (when `D > A`): the trailing `D - A` deleted-only lines render on the left at consecutive rows; the right pane shows blank padding at the same rows. This is the pure-deletion case for unpaired suffixes.
   - Excess additions (when `A > D`): the trailing `A - D` added-only lines render on the right at consecutive rows; the left pane shows blank padding at the same rows. This is the pure-addition case for unpaired suffixes.
   - The padding rows are visually the same height as a normal line and use the same gutter as the matching pane, so vertical positions on the two panes always agree.
   - Word-level or character-level highlighting on a paired modification row is out of scope (see Non-goals); a row pair shows the deleted line in full on the left and the added line in full on the right.

4. Synchronized vertical scrolling:
   - A scroll wheel event in either pane drives both panes by the same delta.
   - Cursor-up or cursor-down moves only the right pane cursor. The baseline pane scrolls without exposing or moving a cursor so the corresponding line stays in view.
   - Hunk navigation actions (`f` / `F` / "Next change" / "Previous change") move the focused hunk on both panes simultaneously, focusing the matching row in the modified pane.
   - Search and find-next in code review keeps both panes scrolled to the matched line, with the match highlighted on the pane that contains it.

5. Selection is per-pane:
   - Mouse drag selection on one pane never extends into the other.
   - Cmd-A in the modified pane selects only modified-pane content.
   - Cmd-A in the baseline pane selects only baseline-pane content that is selectable for copy.
   - Copy from a pane copies only that pane's selected text. The clipboard text is the rendered pane's content (no diff markers added).
   - Deleted-line ranges in the baseline pane are selectable for copy.
   - This matches GitHub Desktop and GitLab MR review behavior. Cross-pane selection is out of scope.

6. Settings -> Code is the entry point for the layout choice:
   - The Code settings page gains a "Diff layout" subsection with a two-option segmented control: "Inline" and "Side by side".
   - Selecting either option updates `code.editor.diff_layout` and refreshes visible Code Review diffs without re-fetching diff data or losing the current scroll position.
   - The currently active layout is shown as the selected segment.
   - The diff toolbar continues to expose per-view ephemeral controls such as whitespace visibility, but it does not expose `code.editor.diff_layout`; the layout is a user-level preference for Code Review v1.

7. The setting is read by the Code Review diff host at diff-construction time and on every change:
   - Code Review construction reads `code.editor.diff_layout` and dispatches to inline rendering or `DiffLayout::SideBySide` rendering.
   - The Code Review pane subscribes to setting updates and applies the new layout to every visible diff. Diffs that are scrolled out of view rebuild lazily on next render.
   - Switching the setting while a diff is open preserves the current scroll position and cursor row in both layouts. The user does not need to scroll back to where they were.

8. Hunk navigation, accept, reject, save, and revert behave identically across layouts:
   - Accept (write the modified file) writes the same content that the inline layout would have written.
   - Reject discards the modification on both panes; the result is the baseline.
   - Revert-to-base restores the editor to the baseline content; the side-by-side renderer then shows two identical panes (which rebuild as a no-op diff).
   - Save writes the modified content via `FileModel`, the same as today.

9. Comment threads in Code Review render in `SideBySide` next to the line they target:
   - Comments authored on a line in the baseline pane render under that line in the baseline pane.
   - Comments authored on a line in the modified pane render under that line in the modified pane.
   - The corresponding pane shows a small "comment marker" gutter glyph at the same row to indicate that the other side has a thread there.
   - Multi-line comment ranges that span both deleted and added regions render on the side they were originally authored against, the same as today.

10. Hidden lines (collapsed unchanged context) work identically across layouts:
    - The same "Show N more lines" affordance appears at the same vertical position on both panes.
    - Expanding hidden lines on either pane expands them on both, since context lines exist in both files.

11. Find-in-diff in Code Review highlights matches in both panes when both panes contain the search term, and only the matching pane when only one does. Cycling through matches with `cmd-G` / `cmd-shift-G` advances the search position across panes; focus remains on the modified pane when the next match lives on the baseline side.

12. The Code Review header (`app/src/code_review/code_review_header/`) and the existing diff menu's `DiffMode` selector ("Head" / "Main Branch" / "Other Branch") are unchanged. They control the comparison base; layout is orthogonal.

13. Telemetry: when the user changes the layout in Settings -> Code, emit a `CodeReviewTelemetryEvent` carrying the new layout value. Layout-change rate is the metric for adoption.

14. Accessibility:
    - The modified pane participates in normal edit-focused tab order. The baseline pane can receive focus for copy selection only and never exposes edit actions.
    - The cursor is announced only in the modified pane.
    - Screen-reader output for a side-by-side row should read as "Original: <text> ... Modified: <text>" so a non-sighted reviewer can still hear both sides.
    - Color is not the only signal of change: every changed row carries a `+` or `-` gutter glyph in the corresponding pane, identical to today's inline gutter.

15. Performance budget:
    - Switching layouts on a 5,000-line diff completes in under 200ms on an M1 MacBook Air, measured from settings change to first paint of the new layout.
    - Memory overhead for `SideBySide` is bounded to the additional state needed to render the baseline pane alongside the modified pane.

16. Feature flag gating:
    - The change ships behind a `SideBySideDiffLayout` `FeatureFlag` defined in `crates/warp_features/src/lib.rs` (the canonical flag enum, re-exported from `app/src/features.rs`) that defaults to off in shipping builds and on in dogfood/preview builds. Once stabilized, the flag is removed and the setting becomes the user-facing control.
    - When the flag is off, the Settings page does not render the "Diff layout" widget, and the setting is treated as `Inline` regardless of stored value.

## Mockup placeholder

The mockup placeholder for this spec is the Settings -> Code page. It should show a "Diff layout" subsection under Code settings with an "Inline" / "Side by side" segmented control and explanatory helper text that the setting applies to Code Review.
