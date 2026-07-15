# PRODUCT: Reusable TUI Tab-Bar Component
Linear: [CODE-1822 — Orchestration](https://linear.app/warpdotdev/issue/CODE-1822/orchestration)

## Summary
The TUI element library gains a reusable horizontal tab-bar component that renders an optional main tab and a pageable list of secondary tabs. The component owns width-dependent layout and interaction geometry while callers own application selection, focus, and page state through semantic inputs and callbacks.

## Figma
The orchestration designs establish the component states; the component itself remains domain-neutral:
- Unfocused tab bar: https://www.figma.com/design/yg5nbPZuGoAszHS3Rhvehu/TUI?node-id=806-20498&m=dev
- Focused tab bar: https://www.figma.com/design/yg5nbPZuGoAszHS3Rhvehu/TUI?node-id=806-19947&m=dev
- Truncated final tab and overflow: https://www.figma.com/design/yg5nbPZuGoAszHS3Rhvehu/TUI?node-id=881-21464&m=dev

## Goals
- Give TUI surfaces one reusable component for horizontal tabs, truncation, overflow paging, and pointer interaction.
- Keep application-specific selection and page state outside the component.
- Keep width-derived visible ranges and page boundaries private to the component.

## Non-goals
- Knowing about orchestration, conversations, sessions, agents, or Warp-specific navigation.
- Owning application selection, keyboard focus, or persisted page anchors.
- Choosing colors, icons, labels, maximum widths, or keybindings for a caller.
- Rendering context menus, close buttons, drag reordering, or pinned-tab controls.

## Behavior
### Inputs and output
1. A caller can provide:
   - An optional main tab.
   - An ordered list of secondary tabs.
   - A stable key and label for every tab.
   - An optional leading glyph and glyph style for every tab.
   - The selected tab key, if any.
   - Whether the bar is focused.
   - Focused, unfocused, selected, background, divider, and overflow styles.
   - An optional maximum label width in terminal display cells.
   - The current secondary-page anchor.
2. The component renders exactly one terminal row. It never wraps tabs onto another row.
3. The main tab, when present, is fixed at the leading edge and does not participate in secondary-tab paging.
4. The caller controls any label or divider surrounding the tabs; the component does not hard-code product copy.
5. An empty secondary list is valid. The component renders the supplied main tab and surrounding chrome without overflow controls.

### Ownership
6. The component privately owns:
   - Stable mouse state for tabs and overflow controls.
   - Width-dependent tab packing.
   - The settled visible secondary range.
   - Previous and next page boundaries.
   - Hit-test geometry.
7. The caller cannot read or mutate the settled visible range or page-boundary geometry.
8. The component does not mutate application selection, page state, focus, models, or caller-owned collections.
9. The component communicates only semantic outcomes:
   - `SelectTab(key)` when a visible tab is clicked.
   - `PageChanged(anchor_key)` when an overflow control chooses another page.
   - A target tab key when the caller requests previous or next keyboard navigation.
10. Rebuilding or resizing the element does not recreate mouse state for tab keys that remain present.
11. Removed tab keys release their retained component state and cannot remain clickable.

### Selection and focus presentation
12. The selected key determines which tab uses the selected treatment. The component has no pending selection distinct from the caller's selected key.
13. Focused and unfocused selected treatments are independently caller-configurable.
14. Focus changes affect presentation only. Focusing the component does not select a different tab or change the page.
15. If the selected key is absent, the component renders no selected tab and continues to lay out and dispatch interactions normally.

### Label width and truncation
16. Width calculations use terminal display cells rather than Unicode scalar count or byte length.
17. When a maximum label width is supplied, every label is constrained to that many display cells, including the ellipsis.
18. A label exceeding its maximum is truncated with `...`.
19. A label within its maximum is rendered in full.
20. Wide and combining Unicode characters never split into invalid text or corrupt following cell alignment.
21. The last visible secondary tab may be truncated below its configured maximum when required to preserve an applicable overflow control.
22. At narrow widths, fixed leading content and applicable overflow controls take priority over secondary-label content. The component never paints outside its assigned row.

### Paging
23. The component packs secondary tabs beginning at the caller's page anchor.
24. A next overflow control appears when later secondary tabs are hidden.
25. A previous overflow control appears when earlier secondary tabs are hidden.
26. A control with no page in its direction is not actionable.
27. Activating an overflow control emits `PageChanged` with the anchor computed from the component's settled layout.
28. Paging does not emit `SelectTab`.
29. Paging does not change focus.
30. When the caller supplies a new page anchor, the next layout settles the visible range from that anchor and clamps an unavailable anchor to a valid page.
31. Resizing recomputes visible tabs and page boundaries from the same supplied order and anchor.

### Navigation and pointer behavior
32. Activating a visible tab emits `SelectTab` for that tab's stable key.
33. A tab remains clickable regardless of the bar's focused presentation.
34. Activating a tab never changes focus by itself.
35. The component can resolve previous and next navigation from its private settled layout:
   - When the selected tab is visible, navigation uses the complete supplied order and wraps at both ends.
   - When the selected tab is off-page, previous resolves to the last visible secondary tab and next resolves to the first visible secondary tab.
36. Resolved navigation returns only the target key; the caller decides what selecting that key means.
37. Hit targets include only the painted tab or overflow-control footprint, not unused trailing row space.
38. Pointer press/release outside a target does not invoke its callback.
