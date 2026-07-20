# PRODUCT: TUI Orchestration Conversation Tab Bar
Linear: [CODE-1822 — Orchestration](https://linear.app/warpdotdev/issue/CODE-1822/orchestration)
Component: [specs/code-1822-tui-tab-bar-component/PRODUCT.md](../code-1822-tui-tab-bar-component/PRODUCT.md)

## Summary
The Warp TUI shows a tab bar for an orchestration tree so users can see and switch among the orchestrator and its navigable child-agent conversations. The bar supports keyboard and mouse navigation, preserves each session's state, and paginates predictably when all child tabs do not fit.

## Figma
- Orchestration bar visible but unfocused: https://www.figma.com/design/yg5nbPZuGoAszHS3Rhvehu/TUI?node-id=806-20498&m=dev
- Orchestration bar focused: https://www.figma.com/design/yg5nbPZuGoAszHS3Rhvehu/TUI?node-id=806-19947&m=dev
- Focused bar with a truncated tab and overflow: https://www.figma.com/design/yg5nbPZuGoAszHS3Rhvehu/TUI?node-id=881-21464&m=dev

## Goals
- Make every navigable conversation in a local TUI orchestration tree directly reachable without opening the conversation picker.
- Keep keyboard navigation fast while preserving mouse access to every visible tab and overflow control.

## Non-goals
- Pinning or unpinning agents from the TUI. Existing pin state may affect ordering for parity with the GUI.
- Showing conversations that do not have a retained, navigable TUI session.
- Restoring or materializing a missing session when its conversation would otherwise appear in the tree.
- Killing a selected child with `Ctrl+C`. This will be implemented in a follow-up PR above the tab-bar PR; this PR retains the existing session-level `Ctrl+C` behavior and does not advertise killing in the focused footer.
- Adding GUI-style per-agent menus such as opening a child in another pane or tab.

## Behavior
### Visibility and contents
1. The tab bar appears at the top of the normal TUI session surface when the focused conversation belongs to an orchestration tree with at least one navigable child-agent conversation.
2. The same orchestration tree is shown while the orchestrator or any navigable descendant session is focused. Switching among members never changes which tree the bar represents.
3. The bar contains:
   - The `Agents:` label.
   - An optional main tab for the orchestrator, fixed at the leading edge.
   - A divider after the orchestrator.
   - One tab for each navigable child-agent conversation.
4. A conversation is included only while it maps to a retained TUI session that can be focused immediately. Failed, synthetic, unloaded, removed, or otherwise non-navigable conversations are omitted.
5. The orchestrator tab is labeled `orchestrator`. Each child tab uses the child's stable agent identity glyph and agent name.
6. The active conversation is the selected tab. There is no second pending or highlighted selection distinct from the active conversation.
7. When the bar is unfocused, the selected tab remains visibly emphasized without using the focused selection background. When the bar is focused, the selected tab uses the focused magenta selection treatment from the designs.
8. Tab colors and text styles adapt to the active terminal theme. The bar remains legible in dark, light, and custom themes without fixed foreground or background colors.

### Dynamic ordering
9. Child tabs use the same canonical ordering as the GUI orchestration pill bar. The TUI must not maintain a separate ordering policy.
10. The GUI ordering is applied exactly:
   - Pinned children precede unpinned children.
   - Within each pin bucket, blocked children come first.
   - Errored children follow blocked children.
   - In-progress, transient-error, and waiting children follow errored children.
   - Successful and cancelled children follow active children, ordered by most recent modification first.
   - Spawn order breaks remaining ties.
11. The row reorders as status, recency, or persisted pin state changes. Labels, selection, keyboard navigation, and pagination all use the same updated order.
12. Reordering never changes the active conversation by itself.

### Entering and leaving keyboard focus
13. While an orchestration tab bar is available and the input is focused, the normal footer shows the `Shift + ↑ sub-agents` hint from the design.
14. `Shift+Up` focuses the tab bar only when:
   - The input cursor is on the first visual row of the wrapped input (display row zero), and
   - The input has no active text selection.
15. On any other input row, or while text is selected, `Shift+Up` keeps its existing text-selection behavior.
16. Focusing the bar initially selects the already-active conversation; focus alone never switches sessions.
17. `Shift+Down` leaves the tab bar and focuses the active session's normal interaction target. This is normally the input, but an active blocking interaction or full-screen terminal surface retains its existing focus precedence.
18. While the bar is focused, its footer shows: `Tab or ← → to navigate  Shift + ← → to go to start/end  Shift + ↓ to send a message`.

### Keyboard navigation
19. While the bar is focused, `Right` and `Tab` immediately switch to the next conversation in the current canonical order.
20. While the bar is focused, `Left` and `Shift+Tab` immediately switch to the previous conversation in the current canonical order.
21. Previous/next navigation wraps across the complete sequence of the orchestrator followed by the ordered child tabs.
22. `Shift+Left` switches to the first child-agent conversation.
23. `Shift+Right` switches to the last child-agent conversation.
24. The orchestrator is excluded from the `Shift+Left` and `Shift+Right` destinations.
25. A keyboard-driven session switch keeps the tab bar focused in the target session so repeated navigation continues without another `Shift+Up`.
26. Keyboard navigation always starts from the active conversation in the complete canonical order, even when an explicitly selected overflow page does not contain the active tab. `Tab` behaves like `Right`, and `Shift+Tab` behaves like `Left`.
27. Once keyboard navigation switches conversations, the newly active tab becomes the navigation origin and is kept visible.

### Mouse navigation
28. Every visible orchestrator or child tab is clickable whether or not the tab bar currently has keyboard focus.
29. Clicking a tab immediately switches to that conversation's retained session.
30. If the bar was focused before the click, it remains focused after the switch.
31. If the bar was not focused before the click, the target session's normal interaction target receives focus. The click does not force keyboard focus onto the tab bar.
32. Clicking the already-active tab does not reset its transcript, input draft, cursor, selection, scroll position, or running work.

### Width, truncation, and overflow
33. The orchestrator remains fixed at the leading edge. Only the child-tab region paginates.
34. The orchestration bar configures the reusable component with a maximum label width of 20 terminal display cells, including the ellipsis.
35. A child label wider than its maximum is truncated with `...`. Truncation is display-cell aware so wide Unicode characters never corrupt alignment.
36. The final visible tab on a page may be truncated further when needed to preserve the applicable overflow arrow, matching the supplied narrow/overflow design.
37. A right overflow arrow appears when later child tabs are hidden. A left overflow arrow appears when earlier child tabs are hidden. An arrow that has no page in its direction is not shown as actionable.
38. Clicking an overflow arrow changes only the visible child page:
   - It does not switch conversations.
   - It does not select a tab.
   - It does not change keyboard focus. In particular, clicking an arrow while the input is focused leaves the input focused.
39. The selected page is shared across all sessions in the same orchestration tree. Switching conversations does not reset it.
40. The active tab is automatically revealed after session selection or dynamic reordering unless the user explicitly paged away with an overflow arrow. Selecting another tab already on the visible page does not shift or re-anchor that page.
41. After an explicit overflow click, the chosen page remains visible even if the active tab is on another page. The next keyboard selection follows the active conversation's canonical neighbor per (26), not the visible page edge.
42. Selecting a tab clears the explicit paged-away state and keeps the selected tab visible.
43. Terminal resizing recomputes which complete or truncated tabs fit, preserves a valid page when possible, and never clips an overflow arrow or writes outside the row.
44. At very narrow widths, the bar prioritizes the `Agents:` label, orchestrator tab, divider, and an applicable overflow control before child labels. It remains a single row.

### Session and lifecycle behavior
45. Switching tabs focuses the existing retained session; it does not rebuild, restore, clone, or move the conversation.
46. Each session preserves its own transcript position, input draft, cursor, text selection, input scroll, blocking interaction, PTY state, and running agent state while another tab is active.
47. A newly materialized navigable child appears without requiring the user to leave the current session.
48. A removed or failed session disappears without leaving a clickable stale tab. The remaining tabs, page boundaries, and selection are recomputed immediately.
49. Status-driven reordering and child additions/removals never steal keyboard focus from the input or tab bar.
50. The tab bar remains available above normal blocking interactions. Existing alternate-screen behavior continues to own the complete terminal surface while active.
