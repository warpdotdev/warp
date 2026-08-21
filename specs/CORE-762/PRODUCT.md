# Shift+click extends block-list text selection

## Summary
Warp must extend an existing text highlight when the user holds Shift and left-clicks in terminal text or agent rich content. The interaction must keep the original selection endpoint fixed, move only the active endpoint, and preserve Warp's existing whole-block Shift+click behavior when text extension does not apply.

Linear: [CORE-762](https://linear.app/warpdotdev/issue/CORE-762/shiftclick-does-not-extend-text-selection-in-terminal-or-rich-content)

Figma: none provided. This change reuses the existing selection highlight.

## Behavior
1. Shift+left-click extends a current non-empty text selection when the clicked text belongs to a selection surface that can represent the current selection.
   - A terminal block-list selection can extend to terminal text in the same command block or another command block.
   - A terminal block-list selection can cross intervening command blocks and agent rich-content blocks.
   - A terminal block-list selection can end inside an agent rich-content block when the existing cross-block drag path can represent that endpoint.
   - A selection that starts inside one agent rich-content `SelectableArea` can extend within that same area.

2. The original fixed endpoint does not move. The Shift+click moves only the active endpoint to the clicked cell or rich-text position.

3. Repeated Shift+clicks continue to use the same fixed endpoint. A later click may move the active endpoint to either side of the fixed endpoint and reverse the selection.

4. Terminal selection is not limited to one command block. Warp's point-based block-list selection already spans multiple command blocks during drag. It also coordinates intervening agent rich-content highlights. This behavior is the required Shift+click behavior, not a new limitation.

5. A rich-content-only selection cannot extend into a different rich-content area or into terminal text in v1. Each `SelectableArea` owns coordinates relative to itself, and a rich-content-only selection has no block-list point that can serve as a cross-surface anchor.

6. Text extension is not applicable in these cases:
   - There is no current non-empty text selection.
   - The prior interaction produced an empty selection that Warp discarded on mouse-up.
   - An interactive child consumes the mouse-down, such as a button or nested editor that owns its own selection.
   - A rich-content-only selection exists, but the click is outside the `SelectableArea` that owns it.
   - The click is on a surface excluded from v1, including alt-screen or fullscreen TUI mouse-reporting content.

7. When text extension is not applicable, Warp uses the current click behavior.
   - With selected whole blocks and no applicable text selection, Shift+click range-selects whole blocks.
   - With no text selection or whole-block selection, Shift+click behaves as a normal click.
   - Warp does not retain an invisible anchor after an empty click.

8. When both a non-empty text selection and whole-block selection state are present, applicable text extension wins. Warp extends the text and does not range-select blocks.

9. After an applicable Shift+mouse-down, dragging continues to move the active text endpoint. Holding Shift during that drag does not switch to whole-block selection.

10. A plain left-click without Shift keeps the current behavior. It starts a new selection and replaces or clears the old selection.

11. Shift+click uses simple cell or character extension.
   - A completed word or line selection first keeps its visible fixed boundary, then extends to the clicked cell or character.
   - Shift+double-click does not add word-extension behavior.
   - Shift+triple-click does not add line-extension behavior.

12. Mouse-up keeps the extended selection when it is non-empty. Existing copy-on-select, explicit copy, and selected-text-as-agent-context behavior use the updated selection.

13. Keyboard selection extension and input-editor Shift+click behavior do not change.

## Decisions
- **Text selection versus whole-block selection:** Text extension wins only when a non-empty text selection exists and the destination is applicable. Otherwise, Warp preserves whole-block range selection. This is the requester-approved rule and supersedes the unresolved alternatives in DES-282.
- **Anchor after a plain click:** Warp does not add hidden anchor state. This avoids changing the meaning of an empty click.
- **Terminal and rich content:** Both paths ship together with the same fixed-endpoint semantics. They use separate internal mechanisms because their coordinate and ownership models differ.
- **Fixed endpoint versus nearest endpoint:** The original endpoint remains fixed. Moving whichever boundary is nearest to the click, as proposed in external PR [#11933](https://github.com/warpdotdev/warp/pull/11933), would change the anchor after repeated Shift+clicks and is not the selected behavior.

## Out of scope
- Alt-screen and fullscreen TUI mouse-reporting selection.
- Shift+double-click word extension.
- Shift+triple-click line extension.
- A new global coordinate or hidden-anchor model for selections that start in rich content.
- Changes to selection colors or other visual styling.

## Assumptions
- Existing child-first mouse event ownership remains unchanged. A nested editor or control that consumes Shift+click does not also extend its parent rich-content selection.
- The implementation does not require a feature flag because the fallback preserves current behavior whenever text extension is not applicable.
