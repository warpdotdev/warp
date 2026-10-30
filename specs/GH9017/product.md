# Word wrap (soft wrap) in the file editor and code review diff view — Product Spec

GitHub issue: https://github.com/warpdotdev/warp/issues/9017

Figma: none provided

## Summary

Add a persistent word-wrap (soft-wrap) setting to Warp's built-in file editor and the code review diff pane. When enabled, lines longer than the visible pane wrap onto additional visual rows instead of extending past the right edge behind a horizontal scrollbar. Wrapping is visual only: it never inserts newlines, reindents text, or changes what is saved to disk — the same behavior as VS Code's word wrap.

## Goals

- Let users read and edit long-lined files (Markdown source, prose, config dumps, generated files) without horizontal scrolling.
- Let users review diffs whose lines exceed the pane width in the code review / diff view.
- Persist the preference across sessions, and make it quickly toggleable from the keyboard without opening Settings.

## Non-goals

- Wrapping terminal output or the command line (separate long-standing requests; not this feature).
- Wrapping the rendered Markdown preview, or changing the Raw/Rendered toggle behavior.
- Adding a per-file-type or per-file wrap state. One global setting governs the surfaces listed below.
- Hard-wrapping text on save or on any user action.
- Adding a wrap-aware column ruler, wrap indentation guides, or per-paragraph wrap prefixes (VS Code-style `renderIndentGuides`-adjacent features).
- Changing the headless TUI editor.

## Behavior

1. A new setting **Word wrap long lines** appears in Settings → Code → Editor and Code Review, default **off**. It persists across sessions and workspace restores.

2. The setting applies to the raw/source text of every file opened in Warp's built-in file editor (including the Raw mode of the Markdown viewer, which is the same editor surface) and to the diff text in the code review / diff view. When off, behavior is exactly as today: long lines lay out at full width behind the horizontal scrollbar.

3. When on, each logical line that does not fit the visible pane width continues on the following visual row(s), wrapping at word boundaries where possible and at single characters where a single word exceeds the pane width.

4. Wrapping never modifies the file: a line that wraps to N visual rows remains one logical line in the buffer, copy/paste, search, undo history, and on-disk content. Saving a file after viewing it wrapped is byte-identical to saving the same file unwrapped.

5. Wrapping follows the live pane width: resizing the window, the pane, the sidebar, or zooming the font re-wraps immediately, rather than keeping the old break points.

6. The horizontal scrollbar is hidden while wrap is on for the affected surfaces, and returns when wrap is turned off.

7. Line numbers in the gutter count logical lines, not visual rows. A logical line that wraps to five visual rows still shows exactly one number, and every subsequent line keeps the number it would have with wrap off. Turning wrap on or off never changes any gutter number.

8. Relative line-number mode keeps its meaning: numbers are relative distances between logical lines and the cursor's logical line, unchanged by wrapping.

9. Diff decorations stay aligned with their logical lines when wrap is on: an added/removed/changed hunk continues to highlight exactly the lines it highlights with wrap off, and hunk collapse/expand behavior is unchanged.

10. The code review diff view is a single unified column (removed lines render as inline blocks between the surrounding new lines), not a split side-by-side layout. When wrap is on, every row — changed, added, and the inline removed blocks — wraps to the same pane width, so a hunk remains one contiguous, correctly ordered region. Hunk collapse/expand, navigation between hunks, and the added/removed line counters keep their existing semantics because they are computed from the diff, not from layout.

11. Go-to-line (line:column) continues to address logical lines: entering a line number present with wrap off lands on the same content with wrap on.

12. Find-in-file, selection, multi-cursor, vim mode, and keyboard navigation operate on logical content unchanged; moving the cursor down from a wrapped visual row that is not the last of its logical line moves to the next visual row within the same logical line (VS Code-style), not to the next logical line.

13. One editable action **Toggle word wrap** is registered (keyless by default) so users can bind a shortcut such as Alt+Z in Settings → Keyboard Shortcuts and invoke it from the Command Palette. Toggling applies immediately to all open editor and diff panes and updates the persisted setting.

14. Hidden/collapsed sections (fold markers, collapsed diff hunks) render and expand as today; their hidden-line counts are unaffected by wrap.

15. Very narrow panes still wrap: no minimum pane width disables the setting or falls back to horizontal scrolling.

16. Existing editors that embed code-rendering surfaces outside the file editor and diff view (comment boxes, inline AI blocks, find-references cards) keep their current layout behavior and are not governed by this setting.

## Open questions

- None blocking. (Whether the rendered Markdown preview should also gain a wrap-width fix is tracked separately in #10527.)
