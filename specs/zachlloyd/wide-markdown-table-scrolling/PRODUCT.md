# Wide Markdown Table Scrolling — Product Spec
Linear: none provided
Figma: none provided
Related specs:
- `specs/zachlloyd/markdown-table-consistency/PRODUCT.md`
- `specs/APP-3076/PRODUCT.md`

## Summary
Make rendered Markdown tables scroll horizontally inside their own viewport when they are wider than the surrounding surface, instead of forcing the entire notebook editor or AI block list to scroll horizontally. Selection, cursor, copy, and keyboard navigation continue to work correctly while a table is horizontally scrolled.

## Problem
Warp already renders Markdown tables, but wide-table overflow behavior is not consistently defined across the product. Editor-backed notebook tables render in document flow with no table-local horizontal scroll, so wide tables push their containing surface wider. AI block list structured tables do scroll horizontally, but the contract — and the expected selection behavior while scrolled — is not written down. The result is that wide tables are hard to read and selection/cursor behavior while scrolling is easy to regress silently.

## Behavior

### Scope
1. This feature applies to rendered Markdown tables in notebook/editor-backed Markdown surfaces and in AI block list table sections created from Markdown table output. Other Warp surfaces that inherit the same editor-backed Markdown table renderer pick up the behavior automatically, but notebook/editor and AI block list are the required launch surfaces.

### Overflow rule
2. When a rendered Markdown table fits within the available content width, it renders normally with no horizontal scrolling behavior. Nothing changes for tables that already fit.
3. When a rendered Markdown table is wider than the available content width:
   - The table remains in normal document flow.
   - The visible table region is constrained to the surface width available to that table.
   - The user can horizontally scroll the table to reveal off-screen columns.
   - The table exposes a standard horizontal scrolling affordance (scrollbar) when overflow exists.
   - The rest of the notebook or block list does not need to scroll horizontally to access the wide table.

### Horizontally scrollable container exception
4. When the surrounding surface itself provides horizontal scrolling over the full content area (for example, a code editor that lays out all content at its intrinsic width), wide Markdown tables must not introduce their own table-local horizontal scrolling:
   - The table renders at its full intrinsic width.
   - No table-local horizontal scrollbar, thumb, or overflow affordance is shown.
   - Horizontal scrolling gestures over the table fall through to the surrounding editor.
   - Horizontal overflow is reached entirely through the surrounding editor's existing horizontal scroll.

   This mirrors the vertical scrolling rule: the feature only introduces a nested scrolling axis when the surrounding surface does not already own that axis.

### Vertical scrolling
5. Wide-table support must not introduce nested vertical scrolling. The notebook or surrounding editor continues to own vertical scrolling in editor contexts, the AI block list continues to own vertical scrolling in block list contexts, and the table itself only introduces horizontal overflow behavior.

### Readability while scrolled
6. When horizontally scrolled, cells remain fully rendered — no ellipses, truncation, or degraded rendering solely because content is off-screen. Header and body alignment remain correct. Inline Markdown formatting inside cells continues to render correctly. The table keeps the same styling treatment it would have without overflow.

### Per-cell maximum content width
7. Individual table cells have a maximum content width. A cell whose content fits within the maximum sizes to its content as it does today. A cell whose content exceeds the maximum is laid out at the maximum width and soft-wraps onto additional lines. Alignment, inline Markdown formatting, and header/body styling continue to apply to the wrapped content.
8. The per-cell maximum width applies regardless of whether the surrounding surface owns horizontal scrolling — it is a readability cap on intrinsic cell width, not a scroll affordance. One long cell never dominates the intrinsic width of the entire table.

### Mouse, trackpad, and scrollbar
9. Horizontal scrolling gestures over an overflowing table move the table horizontally. Direct interaction with the horizontal scrollbar (drag, click in track) moves the table horizontally. Normal document-level vertical scrolling continues to work for the surrounding notebook or block list.
10. Mixed-axis trackpad gestures resolve cleanly to either local horizontal table scrolling or surrounding vertical document scrolling, without visible vertical jitter while the table is being panned horizontally.
11. When the table is already pinned at its horizontal scroll edge and the user generates a horizontal wheel delta that would push further in that direction, the event propagates to the surrounding scroller so the user can continue scrolling the document instead of getting stuck at the table edge.
12. The `MouseMoved` event is not consumed by the table scrollbar thumb: even while the pointer hovers over the scrollbar thumb, downstream handlers (hover-link detection, cursor changes) continue to receive the event.

### Selection while horizontally scrolled
13. Clicking inside a horizontally scrolled table places the caret or starts the selection at the correct cell content. Double-clicking a visible word selects the word in the hovered cell — not text from another column in the same row. Dragging across visible table content selects the correct text after the scroll offset is applied.
14. An existing selection remains stable when the user horizontally scrolls the table: the visible highlight moves with the selected content rather than staying pinned to stale viewport coordinates. Selection highlight rectangles stay visually aligned with the selected text while scrolled.
15. Copying a selection copies the selected rendered text, not hidden text outside the selection and not the raw Markdown source. Copying a partial selection copies exactly the selected visible text — it does not convert into a whole-table copy.
16. In block list contexts, selection and copy behavior apply to read-only text selection. In editor contexts, they apply to both selection and caret behavior.

### Editable notebook behavior
17. In editable notebook/editor contexts, existing editing behavior continues to work inside wide tables. The user can still place the caret inside a cell, drag to select text within and across cells, move the caret with the keyboard, and type / delete / paste / otherwise edit table cell content. Horizontal overflow does not make editable tables feel like read-only snapshots.

### Caret visibility during keyboard movement
18. In editable contexts, when keyboard navigation or editing moves the active caret or active selection into an off-screen part of an overflowing table, the table horizontally scrolls just enough to keep the active caret or selection visible. Reveal happens at the table level, not by scrolling the whole surface. This applies to arrow-key movement, Shift+arrow extension, and other common navigation and editing flows.

### Scroll-state stability
19. While the notebook or block list remains open, ordinary rerenders do not reset a table's horizontal scroll position.
20. If the user has an active selection inside a horizontally scrolled table, rerenders while that selection remains active preserve the visual alignment between the highlight and the selected content.
21. If the table content or layout changes so that overflow no longer exists, the table resets back to its leftmost position and no horizontal overflow affordance remains.

### Narrow tables and non-table content
22. This feature does not change the behavior of:
    - Tables that already fit within the available width.
    - Non-table Markdown blocks.
    - Malformed table-like content that falls back to non-table rendering today.
