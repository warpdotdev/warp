# macOS Speak Selection reads the active Warp text selection
GitHub issue: https://github.com/warpdotdev/warp/issues/10954
Figma: none provided

## Summary
On macOS, when a user selects text in Warp and invokes the native Speak Selection shortcut, macOS should speak the selected text rather than starting from the top of the terminal pane. The behavior should match Warp's existing selected-text semantics for input text, terminal output, wrapped lines, rectangular selections, alt-screen selections, rich content blocks, and obfuscated secrets.

## Problem
Users who rely on macOS Speak Selection currently cannot trust Warp to speak the text they highlighted. The reported behavior is that Siri starts reading from the top of the pane, which makes Warp inconsistent with other terminals and breaks an accessibility workflow for listening to long command output, agent plans, diffs, and analyses without copying content into another app.

## Goals
- Make macOS Speak Selection read the active selected text in the focused Warp surface.
- Preserve Warp's existing copy semantics for what counts as selected text.
- Avoid stale speech when selection changes, clears, or focus moves.
- Keep the fix focused on native macOS accessibility behavior; no new user-visible Warp UI or setting is required.
- Preserve existing VoiceOver announcements and selection-copy behavior.

## Non-goals
- Changing copy, copy-on-select, insert-selected-text, Ask AI selected-context behavior, or block selection semantics.
- Adding a cross-platform selected-text accessibility bridge for Linux, Windows, or web surfaces in this issue.
- Reworking the full terminal accessibility transcript or making every terminal cell individually addressable.
- Adding a Warp text-to-speech feature, menu item, command, prompt, or settings toggle.
- Changing how terminal applications that own mouse reporting receive unmodified drags; existing Shift-drag behavior remains the Warp-owned selection escape hatch.

## Behavior
1. When the focused Warp terminal has a non-empty text selection and the user invokes macOS Speak Selection, macOS speaks that selected text starting at the first selected character.

2. Speak Selection must not start reading earlier terminal contents, such as the top of the pane, the beginning of the accessibility transcript, or the beginning of the current block, when a non-empty text selection exists.

3. If the focused terminal input/editor contains selected text, that input-editor selection takes precedence over any terminal-output selection in the same pane. This matches the user's focus: selected text in the currently editable command area is what macOS should speak.

4. If the focused terminal input/editor has no selected text, Speak Selection uses the focused terminal output selection. A selection dragged bottom-up or right-to-left is spoken in document order, matching Warp copy behavior rather than drag direction.

5. Multi-line selections speak only the selected range and stop at the end of that range. Speech does not continue into unselected rows, earlier terminal output, later terminal output, prompts, or unrelated blocks.

6. Word, semantic, and line selections speak the expanded highlighted selection, not only the original mouse-down cell or cursor point.

7. Rectangular selections preserve row boundaries and ordering the same way Warp copy does. Rows are spoken in their selected row order, with line breaks where copy would include line breaks.

8. Wrapped-line selections use Warp copy's line-break semantics. Visual wraps do not introduce extra spoken line breaks unless the same selected text copied to the clipboard would include them.

9. Alt-screen selections speak only the selected alt-screen text. In mouse-reporting terminal UIs where a normal drag belongs to the terminal app, holding Shift while dragging continues to let Warp own the terminal text selection; Speak Selection then reads that Warp-owned selection.

10. Rich content selections inside terminal block-list content, including AI or agent response text where Warp already supports selection/copy, speak the same selected plain text that copy would produce.

11. Obfuscated secrets remain obfuscated through the selected-text accessibility path. Speak Selection must not reveal hidden secret values that Warp copy would hide.

12. With no active text selection, with only block selection active, or with an empty selection, Warp does not report selected text to macOS. It must not reuse a previous selection. Any fallback macOS behavior in the no-selection case is unchanged by this issue.

13. Clearing or invalidating a selection through user actions, new command execution, alt-screen transitions, resizing, scrollback truncation, selection cleanup, or focus changes is reflected in the next Speak Selection query.

14. Only the focused Warp surface contributes selected text. A selection in an unfocused pane, background window, or inactive rich-content view does not override the focused terminal/input surface.

15. Wide characters, emoji, combining marks, and non-Latin text are spoken as the selected glyphs. The selected-text path must not duplicate wide-character spacer cells, split combining sequences differently from copy, or truncate non-ASCII text.

16. Existing VoiceOver focus and action announcements continue to work. Supporting Speak Selection must not add duplicate VoiceOver announcements during normal selection, remove existing block-text announcements, or change role labels that screen-reader users already hear.

17. The fix applies to macOS native Speak Selection regardless of the user's configured shortcut. The issue's Option+Esc shortcut is only an example of the system setting.

## Success criteria
- A macOS user can select terminal output in Warp, invoke Speak Selection, and hear only the highlighted text.
- Users who listen to long command output or AI-generated content no longer need to copy text into another app solely to use macOS text-to-speech.
- Existing selection copy behavior remains unchanged for normal, semantic, line, rectangular, wrapped, alt-screen, and rich-content selections.
- No stale selected text is spoken after the selection is cleared or focus moves to another pane.

## Validation
- Manual macOS validation covers Speak Selection with terminal output, command input, reversed selections, multi-line selections, rectangular selections, wrapped lines, alt-screen selection, Shift-drag in a mouse-reporting TUI, rich content selection, obfuscated secrets, wide characters, no selection, cleared selection, and focus changes between panes.
- Automated tests cover the selected-text source of truth that the macOS bridge uses, including input-selection precedence and the existing terminal `selection_to_string` semantics.
- Existing VoiceOver smoke validation confirms selection announcements are not duplicated or removed.

## Open questions
- Should a future follow-up expose a richer full-text accessibility provider for terminal contents, including nonzero character counts and range-based APIs, so macOS accessibility features beyond Speak Selection can navigate the transcript more precisely?
- Should non-macOS accessibility bridges gain comparable selected-text support after the macOS regression is fixed?
