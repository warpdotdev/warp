# GH9816: Configurable code editor line number modes
## Summary
Add a configurable line numbering mode for Warp code editors so users can choose Absolute or Relative line numbers. The setting is independent of Vim mode, defaults to today’s absolute numbering, and makes Vim-style vertical motion counts easier to read when users choose relative numbering.
## Problem
Warp code editors currently show only absolute line numbers. Vim users commonly rely on relative-style line numbers to choose motions like `5j` and `12k` without mentally subtracting the current line from nearby line numbers. Because the existing input editor surfaces do not show line-number gutters, this feature should improve the code editor without implying a new gutter in command input.
## Goals
1. Let users choose how line numbers are displayed in code editor gutters.
2. Preserve the current absolute numbering behavior by default.
3. Make relative mode update immediately as the active cursor line changes.
4. Keep the setting usable by all code editor users, not only users who enable Vim keybindings.
5. Avoid adding line numbers to command input editors or rich-text notebook editors as part of this change.
## Non-goals
1. Do not add line-number gutters to the terminal input editor, AI input editor, or other command-entry surfaces.
2. Do not add Vim `:set number`, `:set relativenumber`, or `:set norelativenumber` commands in this iteration.
3. Do not change Vim motion behavior, cursor movement, selections, search, find-references, or diff navigation.
4. Do not change whether a particular code editor surface chooses to show or hide its gutter; the new setting only affects gutters that already render line numbers.
5. Do not redesign the gutter width, diff hunk controls, hidden-section controls, or inline review comment controls beyond the minimum needed to display the selected numbering mode.
## Figma
Figma: none provided. The feature reuses the existing code editor gutter and settings UI patterns.
## Behavior
1. Warp exposes a line numbering mode setting for code editors with exactly two choices:
   - **Absolute**: each line shows its absolute, one-based line number. This is the current behavior and the default for all users.
   - **Relative**: the active cursor line shows its absolute, one-based line number; every other visible line shows the absolute distance in lines from the active cursor line. Warp does not expose any additional line-number option in this iteration.
2. The setting is available from Settings under the Text Editing area, near the existing code/text editing controls. It is not nested under the Vim-mode toggle and remains visible whether Vim keybindings are enabled or disabled.
3. The setting persists like other public editor settings and is restored for future Warp windows and sessions. If the user has not chosen a value, Warp behaves exactly as it does today: Absolute mode.
4. Changing the setting updates all currently open code editor gutters without requiring the user to reopen files, restart Warp, toggle Vim mode, or refocus the editor.
5. Absolute mode is behaviorally identical to the current code editor gutter:
   - The first file line displays `1`, the second displays `2`, and so on.
   - Code editor surfaces that start numbering from a caller-provided starting line continue to use that starting line.
   - Hidden sections, diff hunk controls, and gutter action buttons behave as they do today.
6. Relative mode uses the active cursor line as the origin while keeping the active line absolute:
   - If the cursor is on line 10, line 10 displays `10`, line 9 displays `1`, line 11 displays `1`, line 5 displays `5`, and line 22 displays `12`.
   - Moving the cursor, clicking another line, selecting text, or using keyboard navigation recomputes the displayed distances immediately.
   - Non-active relative distances are always positive integers; lines above and below the cursor both show positive distances.
   - In normal code editor surfaces, Relative mode uses the editor’s current primary selection head even when the editor has just opened or focus temporarily moves elsewhere; losing focus does not force normal code editor gutters back to Absolute mode.
7. For multiple cursors or multiple selections, the active cursor line is the primary selection head used by the editor for cursor-position reporting. The gutter uses that single active line as the relative origin until a future design intentionally supports multiple relative origins.
8. For visual selections, the active line remains the selection head, not the selection anchor or the full selected range. The displayed numbers update as the selection head moves.
9. The line number mode is independent of Vim mode:
    - Users can choose Relative before enabling Vim keybindings.
    - Enabling or disabling Vim keybindings does not reset or hide the chosen line numbering mode.
    - Vim status bar and clipboard settings remain separate from line numbering.
10. Code editor line numbers correspond to logical file lines, not soft-wrap rows. A long line that visually wraps still has one line number, and wrapped continuation rows do not introduce extra relative counts.
11. Hidden/collapsed code regions keep their existing hidden-section gutter affordances. The hidden section itself does not need to display a relative count for every hidden line.
12. Diff and review gutters keep their current affordances:
    - Diff and review editors that are not currently focused continue to show absolute line numbers, even when the global code editor setting is Relative.
    - When a diff or review editor is focused and Relative is selected, current-buffer lines across that editor that already display a line number use Relative mode from the active cursor line.
    - Removed/temporary diff lines that currently omit a line number continue to omit one unless a separate diff design changes that behavior.
    - Diff hunk buttons, comment buttons, hover hit targets, and collapse/expand interactions continue to work.
13. The visual style of line numbers remains consistent with the existing gutter: same font family, size, colors, selection behavior, and alignment unless a small width or alignment adjustment is necessary to prevent relative values from clipping.
14. The gutter reserves enough width for the largest value that can appear in the current mode:
    - Absolute must fit the largest absolute line number for the surface.
    - Relative must fit both the active line’s absolute number and the largest visible relative distance where practical, and must never overlap editor text or gutter controls.
15. Command input editors, terminal prompt editors, AI input editors, and rich-text notebook editors do not show line numbers after this change. Their Vim status indicators and Vim keybindings continue to work as they do today.
16. The settings UI is searchable with terms such as `line number`, `relative line`, `vim`, and `gutter`.
17. If a settings file contains an invalid line numbering value, Warp falls back to Absolute mode using the existing settings validation/error behavior rather than failing to render editors.
## Success criteria
1. A new user or upgraded user with no explicit setting sees the same absolute code editor line numbers as before.
2. Selecting Relative mode shows the active cursor line’s absolute number and distances on surrounding numbered code editor lines.
3. Moving the cursor with mouse, arrow keys, search, Vim motions, or goto-line updates relative gutters immediately.
4. The setting is visible and usable when Vim keybindings are disabled.
5. Enabling or disabling Vim keybindings does not change the selected line numbering mode.
6. Terminal input and AI input surfaces still do not render line-number gutters.
7. Diff hunk controls, inline comment buttons, hidden-section controls, and find-references anchoring still work in code editors with each line numbering mode; inactive diff/review editors continue to show absolute line numbers until that editor is focused.
## Validation
1. Manually verify a multi-line file in the code editor with Absolute and Relative modes selected.
2. In Relative mode, move the cursor above and below visible lines using mouse, arrow keys, goto-line, and Vim motions; verify displayed values update correctly.
3. Verify the setting persists after closing and reopening Warp or reloading settings.
4. Verify the setting remains visible when Vim mode is disabled and that toggling Vim mode does not reset it.
5. Verify terminal command input, AI input, and rich-text notebook editors do not gain line-number gutters.
6. Verify code review/diff editors still show diff decorations and gutter buttons correctly in both modes, show absolute line numbers while the diff/review editor is not focused, and apply Relative numbering across the focused editor.
