# Speak selected terminal text on macOS — Product Spec
GitHub issue: https://github.com/warpdotdev/warp/issues/10954
Figma: none provided

## Summary
When a macOS user selects text in a Warp terminal pane and invokes the system Speak Selection shortcut, macOS should speak exactly the selected text. Warp should not cause Speak Selection to start at the top of the terminal pane, read unrelated terminal history, or ignore the active selection.

## Problem
macOS Speak Selection is an accessibility feature that reads the currently selected text in the focused app. In Warp, selecting text in a terminal pane and pressing the configured Speak Selection shortcut currently causes Siri/system speech to begin reading terminal content from the top of the pane instead of the highlighted text. The same user reports that Speak Selection behaves correctly in other terminals, so the Warp terminal should expose its selected text to macOS in the way system accessibility services expect.

## Goals
- Speak Selection reads the active Warp terminal text selection, starting at the selected text's first character.
- The spoken text matches Warp's existing selected-text semantics for copy and AI context where those semantics already define the selection.
- The behavior works for terminal output, prompt/command text, and alternate-screen terminal content.
- Existing VoiceOver announcements, copy-on-select behavior, block selection, and AI-context selection behavior do not regress.
- The fix is transparent: no new setting, prompt, or visible UI is required.

## Non-goals
- Changing macOS Speak Selection configuration or keybindings.
- Adding a Warp-specific text-to-speech command.
- Changing how text selection is visually rendered.
- Changing what normal copy, copy-on-select, or selected text attached to AI context produces except where they currently expose the same selection text to macOS accessibility.
- Making non-terminal Warp surfaces fully compatible with Speak Selection as part of this issue.
- Supporting Speak Selection when there is no active text selection in the terminal pane; existing terminal accessibility content can remain the fallback.

## User experience
1. A user can enable macOS **System Settings > Accessibility > Spoken Content > Speak Selection** and configure a shortcut such as `Option+Esc`.
2. In Warp on macOS, when the user highlights text in a terminal pane and presses the Speak Selection shortcut, macOS starts speaking the highlighted text, not the pane transcript from the top.
3. The first spoken word is the first selected word in document order. For a reversed selection drag, the spoken order is still document order, matching copy behavior.
4. The last spoken word is the last selected word. Speech does not continue into unselected terminal text after the selection ends.
5. Selecting text within command output speaks only the selected output text.
6. Selecting text within the prompt/command area speaks only the selected prompt/command text.
7. Selecting text across multiple terminal blocks speaks only the selected parts of those blocks, in the same order and with the same newline boundaries Warp uses when copying the selection.
8. Selecting text in alternate-screen applications, such as pagers or full-screen terminal programs, speaks the selected alternate-screen text instead of the block-list transcript.
9. Rectangular selections speak the selected rectangle content with row boundaries preserved, matching Warp's existing plain-text selected-text semantics.
10. Double-click word selections and line selections speak the expanded word or line selection that is visibly highlighted, not the raw mouse-down cell.
11. If the selection includes wide characters, emoji, combining marks, or non-Latin text, Speak Selection includes the same characters the user selected. The spoken content should not duplicate spacer cells or omit part of a wide character.
12. If the selected text contains wrapped terminal lines, the spoken text uses the same line breaks Warp exposes for copied selection text. The fix should not introduce extra line breaks at visual wrap boundaries unless copy already does so.
13. If the selected text includes obfuscated secrets, Speak Selection follows the same user-visible secrecy policy as copying selected text from the terminal. It should not reveal hidden secret values through the accessibility layer.
14. If the selection includes rich terminal-adjacent content that already contributes selected text to Warp's terminal selection model, Speak Selection may include that text only when it is part of the active selection. It should not read every visible rich content block.
15. If there is no active text selection in the focused terminal pane, invoking Speak Selection may fall back to the existing macOS/Warp accessibility value for the focused terminal. This spec only requires correctness when a non-empty terminal text selection exists.
16. Block selection is not text selection. If the user selects blocks rather than highlighting terminal text, Speak Selection should not invent a character range from those blocks. Existing VoiceOver/block-selection announcements may remain unchanged.
17. Changing focus to another pane or surface updates the Speak Selection source. A selection in an unfocused terminal pane should not override a focused pane or another focused text surface.
18. Clearing the text selection, starting a new command, switching alternate screen state, resizing the terminal, or otherwise invalidating the selection should prevent macOS from speaking stale previously selected text.
19. The behavior is macOS-specific where Speak Selection exists. Non-macOS platforms should not gain user-visible behavior changes.
20. The fix should work whether VoiceOver is on or off. Speak Selection is a separate macOS accessibility feature and should not require enabling VoiceOver.

## Success criteria
1. On macOS, selecting a word in terminal output and pressing the system Speak Selection shortcut speaks that word first.
2. Selecting a multi-line range in terminal output speaks only the selected lines and stops at the selection end.
3. Selecting text in an alternate-screen app speaks the selected alternate-screen text.
4. Selecting no terminal text no longer incorrectly reuses stale selected text.
5. Existing copy selection behavior still returns the same text as before.
6. Existing VoiceOver accessibility announcements for terminal focus and block/text selection continue to work.

## Validation
- Manually validate on macOS with Speak Selection enabled and VoiceOver disabled.
- Manually validate with VoiceOver enabled to confirm terminal focus and selection announcements still work.
- Add unit coverage for the selected-text snapshot that macOS accessibility reads, covering regular, reversed, multi-block, rectangular, empty, and alternate-screen selections.
- Add macOS accessibility bridge coverage where possible to verify the host view exposes selected text/range only when Warp has an active terminal text selection.

## Open questions
- macOS can query selected text through several `NSAccessibility` text attributes. The technical implementation should confirm the minimum set required for Speak Selection across supported macOS versions, including the reported macOS 26.4.1 environment.
