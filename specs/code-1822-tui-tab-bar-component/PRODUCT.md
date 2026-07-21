# PRODUCT: Reusable TUI Tab-Bar View
Linear: [CODE-1822 — Orchestration](https://linear.app/warpdotdev/issue/CODE-1822/orchestration)

## Summary
The TUI gains a reusable horizontal tab-bar view that renders an optional main tab and a pageable list of secondary tabs. The view owns retained interaction state and responsive presentation while callers own application selection, focus, and page state.

## Figma
The orchestration designs establish the view states; the view remains domain-neutral:
- Unfocused tab bar: https://www.figma.com/design/yg5nbPZuGoAszHS3Rhvehu/TUI?node-id=806-20498&m=dev
- Focused tab bar: https://www.figma.com/design/yg5nbPZuGoAszHS3Rhvehu/TUI?node-id=806-19947&m=dev
- Truncated final tab and overflow: https://www.figma.com/design/yg5nbPZuGoAszHS3Rhvehu/TUI?node-id=881-21464&m=dev

## Goals
- Give TUI surfaces a reusable view for horizontal tabs, truncation, overflow paging, keyboard targets, and pointer interaction.
- Express tab-bar presentation with the generic TUI element vocabulary.
- Keep application-specific selection, focus, and page state outside the view.
- Match the GUI's size-constraint switch pattern for responsive element-tree variants.

## Non-goals
- Adding a tab-specific element to the TUI element library.
- Knowing about orchestration, conversations, sessions, agents, or Warp-specific navigation.
- Owning application selection, keyboard focus, or persisted page anchors.
- Choosing colors, labels, maximum widths, or keybindings for a caller.
- Rendering context menus, close buttons, drag reordering, or pinned-tab controls.

## Behavior
### Inputs and rendering
1. A caller can provide:
   - An optional main tab.
   - An ordered list of secondary tabs.
   - A stable string key and label for every tab, unique across the configured main and secondary tabs.
   - Optional styled leading text for every tab.
   - The selected tab key, if any.
   - Whether the bar is focused.
   - The current secondary-page anchor.
   - Whether an off-page selected secondary tab should be revealed.
   - Background, leading-label, chrome, normal-tab, focused-selected, and unfocused-selected styles.
   - An optional maximum label width in terminal display cells.
2. The view renders exactly one terminal row and never wraps tabs.
3. The view builds complete row alternatives from generic flex, text, container, and hoverable elements, then uses a size-constraint switch to select the alternative for the assigned width.
4. The main tab, when present, stays at the leading edge and does not participate in secondary paging.
5. The caller controls the optional product label before the tabs. The view supplies one consistent divider and previous/next arrows.
6. An empty secondary list is valid.
7. Duplicate keys across the main and secondary tabs are invalid; creating or reconfiguring the view returns a typed configuration error.

### Ownership
8. The view privately owns stable mouse state for tabs and overflow controls.
9. Re-rendering or resizing does not recreate mouse state for tab keys that remain present.
10. Removed tab keys release their retained mouse state and cannot remain clickable.
11. The view does not mutate application selection, focus, models, or caller-owned tab collections.
12. Pointer interaction emits semantic view events:
    - `SelectTab(key)` when a visible tab is clicked.
    - `PageChanged(anchor_key)` when an overflow control chooses another page.

### Selection and focus presentation
13. The selected key determines which tab uses the selected treatment.
14. Focused and unfocused selected treatments are independently caller-configurable.
15. Focus changes affect presentation only.
16. If the selected key is absent, the view renders no selected tab and continues to lay out and dispatch interactions normally.

### Label width and truncation
17. Width calculations use terminal display cells rather than Unicode scalar count or byte length.
18. When a maximum label width is supplied, every main and secondary label is constrained to that many display cells, including the ellipsis.
19. A maximum label width is invalid when it cannot show the complete label or, for a truncated label, at least one non-zero-width grapheme followed by `...`; creating or reconfiguring the view returns a typed configuration error.
20. A label exceeding its maximum is truncated with `...`.
21. Wide and combining Unicode characters never split into invalid text or corrupt following cell alignment.
22. The last visible secondary tab may be truncated below its configured maximum to preserve an applicable overflow control, but it moves to the next page when there is room for only ellipsis dots and no label content.
23. At narrow widths, fixed leading content and overflow controls take priority over secondary-label content.
24. The view never paints outside its assigned row.

### Paging
25. Responsive composition uses the row width supplied by the layout constraint.
26. Secondary tabs are packed beginning at the caller's page anchor.
27. A next overflow control appears only when later secondary tabs are hidden.
28. A previous overflow control appears only when earlier secondary tabs are hidden.
29. Activating an overflow control emits `PageChanged` with the computed page anchor.
30. Paging does not emit `SelectTab` or change focus.
31. A missing page anchor falls back to the first secondary page.
32. Resizing recomputes visible tabs and page boundaries from the same supplied order and anchor.
33. When selected-tab reveal is enabled, the current page remains stable while the selected secondary tab is visible; only an off-page selection moves the rendered page to that tab.

### Navigation and pointer behavior
34. Previous and next keyboard targets follow the complete semantic order of the main tab followed by secondary tabs and wrap at both ends.
35. First/last-secondary target lookup excludes the main tab.
36. Target lookup returns only a stable key; the caller decides what selecting it means.
37. A tab remains clickable regardless of focused presentation.
38. Activating a tab never changes focus by itself.
39. Hit targets include only the painted tab or overflow-control footprint, not unused trailing row space.
40. Hovering a tab bolds its label without changing selection, page, or focus.
41. Hovering an overflow control bolds the arrow without changing its behavior.
