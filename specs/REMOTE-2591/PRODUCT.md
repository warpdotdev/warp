# REMOTE-2591: Desktop web text-input bridge for active text carets

## Summary
Warp for Web must expose a focused editable browser element while any editable Warp surface owns an active text caret. This lets MacWhisper-class dictation tools, macOS Dictation, and browser text-input services attach to the active surface and insert text into the canvas-rendered editor.

The requester approved a narrow insertion capability and then chose a general surface rule: activate the bridge for any focused editable surface with an active text caret. The bridge is not a full DOM mirror of the surface.

Figma: none provided. The bridge has no visible UI.

## Behavior
1. On desktop browsers, the bridge is active when the focused Warp view reports an active editable text caret. The rule does not identify views by product name or require each field to opt in.

2. When an editable Warp surface gains focus and reports an active text caret, Warp focuses a real multiline browser text control in the same user interaction. The control stays focused for the full time that the surface owns the active text caret.

3. When a tool such as MacWhisper activates while an editable Warp surface is focused:
   - The tool detects a focused editable text control.
   - The tool can show its normal listening UI.
   - Committed transcription is inserted into the focused Warp surface.
   - One committed transcription produces one insertion.

4. The bridge is transparent and does not add visible text, a native caret, scrolling, layout changes, or a new click target. Its browser-reported bounds follow the active Warp text caret so tool UI and IME candidate UI can anchor near the caret.

5. The bridge reports the native semantics of a multiline text box. Its accessible name is “Warp text input.” It is not hidden from the browser accessibility tree.

6. The focused Warp surface remains the source of truth for text, caret position, and selection. The bridge keeps only its input sentinel and does not expose existing contents as its value.

7. Dictated or otherwise programmatically inserted text lands at the focused surface’s current Warp caret.

8. If the focused surface has a selection, inserted text replaces that selection. Text before and after the selection remains unchanged. After insertion, the Warp caret collapses after the inserted text, following that surface’s existing insertion behavior.

9. If the focused surface already contains text and has no selection, inserted text is added at the current caret. Existing text is not cleared or replaced.

10. Repeated dictation commits append or replace text at the then-current Warp caret. Resetting the bridge sentinel between commits must not move the focused surface’s caret.

11. Hardware keyboard input continues to behave as it does before this change:
    - Printable keys insert once.
    - Enter, Tab, Escape, Backspace, Delete, arrow keys, Home, End, and Page keys keep their existing Warp behavior.
    - Warp keybindings, including Vim-mode keybindings and modified keybindings, keep working.
    - Browser shortcuts that Warp intentionally allows, such as focus-location, new-tab, close-tab, reload, and history navigation, keep working.

12. Paste inserts once through Warp’s existing paste behavior. The browser must not also paste the same text into the bridge and cause a second Warp insertion.

13. CJK and other IME input uses browser composition:
    - In-progress composition is shown through Warp’s existing marked-text UI.
    - Candidate selection does not commit intermediate text.
    - Composition commit replaces the focused surface’s current Warp selection or inserts at its current caret.
    - The final composition text is committed exactly once, even when the browser emits both `compositionend` and a trailing `input` event.

14. If the text surface loses focus during composition, Warp clears unfinished marked text. Warp commits text only when the browser delivered a composition commit before the blur.

15. Clicking within an editable text surface moves its Warp caret or changes its Warp selection using existing canvas hit testing. The bridge remains focused after the click.

16. When focus moves between editable Warp surfaces, the bridge stays focused and moves to the new active caret. Subsequent text goes only to the newly focused surface.

17. When focus moves to a Warp surface that does not report an active editable text caret:
    - The bridge blurs.
    - Dictation tools no longer target a Warp text surface.
    - The canvas or newly focused surface receives normal keyboard interaction.

18. Switching to another browser tab or application does not cause Warp to force focus back to the bridge. When the browser tab becomes active again, Warp restores the bridge only if the focused Warp surface still reports an active editable text caret.

19. A read-only, disabled, viewer-only, or selectable-only surface does not activate the bridge, even if it renders a cursor or selection.

20. Failure to create or focus the bridge does not disable the existing canvas keyboard path. Warp logs the failure without logging existing or inserted text.

21. The existing mobile soft-keyboard behavior remains unchanged on iOS and Android. Mobile continues to use its current hidden-input path, sentinel behavior, soft-keyboard lifecycle, and viewport resize handling.

## Surface coverage
The active-caret rule includes these canvas-rendered surface families when they are editable and focused:
- Terminal, agent, and follow-up prompts, including queued-prompt editing and compact agent inputs.
- Single-line and multiline text fields in search, settings, forms, dialogs, rename flows, workflow editors, environment configuration, sharing, and comments.
- Editable code editors, including JSON/configuration editors and source editors that are available in Warp for Web.
- Editable rich-text surfaces, including notebook bodies, notebook titles, and comment editors.
- The raw terminal surface when it owns the active terminal caret. This matches the native IME contract and permits dictation into shells, REPLs, and terminal applications.

The rule excludes these canvas-rendered surfaces:
- Labels, rendered output, search results, menu items, and other surfaces that only display text.
- Read-only, disabled, viewer-only, and selectable-only editor instances.
- A rich-text surface while it hides its caret for a command selection or a nested link editor. The nested editor can activate the bridge when it owns focus.

## Out of scope
- Full screen-reader reading, selection, or caret navigation is out of scope because the approved bridge does not mirror surface contents.
- Browser autofill and password-manager integration are out of scope because the bridge is not a form or credential field and keeps autocomplete disabled.
- Warp desktop is out of scope. The native macOS client already implements `NSTextInputClient` and exposes a native accessibility text-area role.
- Mobile dictation changes are out of scope. This work must preserve the existing mobile path without changing its behavior.
