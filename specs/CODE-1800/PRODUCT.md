# PRODUCT: Inline diff viewer for file-edit tool calls in the TUI (CODE-1800)

## Summary

When the agent edits files in the TUI, each edited file renders as its own transcript section: a header row (`✓ Updated components.js +31 −12 ▾`) followed by a read-only, hunk-only inline diff — line-number gutter, dim context lines, removed lines in red, added lines in green. When a tool call edits multiple files, the per-file sections nest, indented, under one collapsible summary header (`✓ Edited 3 files +34 −15 ▾`). The diff appears as soon as the edits are resolved (before execution completes) so the user can see what the agent is changing. It is not editable and has no approval affordances.

## Figma

Figma: https://www.figma.com/design/yg5nbPZuGoAszHS3Rhvehu/TUI?node-id=323-17499

## Non-goals

- Editing the diff, accepting/rejecting changes, or any permissions/approval UI (the TUI has no permissions model).
- Live token-by-token diff growth while the agent is still generating the tool call arguments.
- Syntax highlighting and intra-line (word-level) diff highlights.
- Keyboard navigation or keyboard toggling within the transcript.

## Behavior

### Per-file sections

1. Each file touched by a single file-edit tool call gets its own section in the transcript: one header row plus (once available) one diff body. When the tool call edits two or more files, these sections nest under an aggregate summary header (see Multi-file aggregation below); a single-file edit renders its file section alone, with no summary header.

2. The header row reads `{status glyph} {verb} {filename} +{added} −{removed} {caret}`, in bold. The verb reflects the operation: `Updated` for content edits, `Created` for new files, `Deleted` for deletions. A rename renders the filename as `old_name → new_name`.

3. `+{added}` renders in the theme's green and `−{removed}` in the theme's red. The counts are the total added/removed line counts of the same computed diff that colors the body, so they always agree with the body's green/red rows (though elision may hide none of them — changed lines are never elided).

### Lifecycle states

4. While the agent is still generating the tool call and while edits are being resolved, the header shows in-progress copy consistent with the existing per-state tool-call labels (e.g. "Preparing edits…"), and no body or caret is shown.

5. The moment edits are resolved — which happens before the edit finishes executing — the body appears fully formed and the header switches to the format in (2). The body does not grow incrementally.

6. The header's leading glyph reflects the action state using the same glyph set and colors as other tool-call rows (○ pending, ● running, ✓ success, ✗ failure, ■ awaiting approval/cancelled). If the action fails or is cancelled after the body appeared, the body remains visible unchanged. If resolution itself fails, the header shows the failure state and no body is shown.

### Diff body content

7. The body is hunk-only: up to 3 unchanged context lines render on each side of each hunk. Unchanged regions between hunks collapse into a single dim separator row reading `… {M} lines`. Elided unchanged regions at the start/end of the file render nothing (the body just starts/ends at the outermost context line, as in the mock).

8. Changed lines are never elided and there is no overall height cap: a diff with many changed lines renders all of them. Only unchanged lines are ever hidden.

9. Each body line renders as: a right-aligned line-number gutter, a two-space gap, then the line content. Gutter width accommodates the largest displayed line number.

10. Coloring is foreground-only (no background fills, no `+`/`-` sign column):
    - Context lines, including their line numbers, render dim (theme bright-black).
    - Removed lines render entirely in theme red.
    - Added lines, including their line numbers, render entirely in theme green.

11. Added and context lines show their new-file line numbers. Removed lines show no line number — their gutter renders blank — consistent with the GUI's diff rendering. (This intentionally diverges from the mock, which shows an old-file number on the removed line.)

12. Created files render their full content as added (green) lines with new-file numbers. Deleted files render their full former content as removed (red) lines with blank gutters.

13. Lines longer than the available width soft-wrap; there is no horizontal scrolling. Continuation rows render a blank gutter with content aligned to the content column, and keep the color of the line they continue.

14. A resolved edit with zero changed lines shows the header with `+0 −0` and no body or caret.

### Collapse

15. The body is expanded by default when it appears.

16. Clicking the header row toggles that file's body between expanded and collapsed (same interaction as the thinking-section toggle, including hover affordance). The caret shows `▾` expanded, `▸` collapsed. Each file's collapse state is independent.

### Transcript integration and invariants

17. Diff rows are ordinary transcript content: they scroll with the transcript, and when a body appears, collapses, or expands, subsequent transcript content shifts accordingly with no stale or clipped rows.

18. The body is read-only: clicks inside it do nothing, it never takes focus, and the input box keeps keyboard focus at all times.

19. If diff data is unavailable for a completed action (e.g. a restored conversation without stored diffs), the view falls back to a single aggregate result line (e.g. "Edited 2 files (+40 −12)"), with no per-file headers, body, or caret.

20. All colors come from the active theme's standard tokens (red, green, bright-black, default foreground). No hard-coded colors; the view renders correctly across themes.

### Multi-file aggregation

21. When a tool call edits two or more files, a summary header reading `{status glyph} Edited {N} files +{added} −{removed} {caret}` precedes the per-file sections, which render indented two cells beneath it. The glyph, bold label, colored counts, and hover affordance follow the same rules as file headers (2–3, 6); per-file headers keep their full format from (2)–(3) unchanged.

22. The summary counts are the sums across all files and appear only once every file's diff has computed, so the totals never tick up incrementally. The caret is always shown.

23. Clicking the summary header collapses or expands the whole group, with the same interaction as file headers (caret `▾` expanded, `▸` collapsed; expanded by default). Collapsing hides all per-file sections; each file's own collapse state is independent and is preserved and restored when the group is re-expanded.
