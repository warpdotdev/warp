# TUI Thinking Blocks
## Summary
Render the agent's reasoning ("thinking") inside the TUI transcript. A thinking block shows a `Thinking...` header while the agent reasons and its reasoning text below it, then becomes a collapsed `Thought for N seconds` header once reasoning finishes. A chevron on the header lets the user collapse or expand the reasoning body.
## Figma
- Starting to think: https://www.figma.com/design/yg5nbPZuGoAszHS3Rhvehu/TUI?node-id=126-4851&m=dev
- In-progress thinking: https://www.figma.com/design/yg5nbPZuGoAszHS3Rhvehu/TUI?node-id=126-4899&m=dev
- In-progress thinking (more): https://www.figma.com/design/yg5nbPZuGoAszHS3Rhvehu/TUI?node-id=126-4935&m=dev
- Finished thinking: https://www.figma.com/design/yg5nbPZuGoAszHS3Rhvehu/TUI?node-id=71-9495&m=dev
Note: the Figma shows a chevron only on the finished header; this feature intentionally shows a chevron in both the streaming and finished states.
## Behavior
1. When an agent exchange in the transcript contains reasoning, that reasoning renders as a distinct "thinking block" positioned in message order relative to the exchange's other output (plain text, tool calls, etc.), not appended at the end.
2. A thinking block has a header line and a collapsible reasoning body beneath it.
3. While the agent is still reasoning (reasoning not finished), the header reads `Thinking...`.
4. Once reasoning has finished, the header reads `Thought for N seconds`, where the duration is the time the agent spent reasoning, with correct pluralization (`Thought for 1 second`, `Thought for 15 seconds`).
5. The header always ends with a chevron disclosure indicator in both states: a right-pointing triangle when collapsed and a down-pointing triangle when expanded, placed after the header text.
6. While reasoning is streaming, the block is expanded by default: the reasoning body renders in full below the header. Body lines are indented (four spaces) beneath the header and wrap to the transcript width. There is no internal scroll region or height cap on the body; the transcript's own scrolling handles long reasoning.
7. As reasoning text streams in, the body updates incrementally and the transcript continues to follow the newest content the same way it does for other streaming output.
8. When reasoning finishes, the block auto-collapses (header only, body hidden) under the default display mode, provided the user has not manually toggled that block (see 10).
9. Clicking anywhere on the header line toggles the block between collapsed and expanded. Collapsing hides the reasoning body and shows only the header; expanding restores the full body. The chevron always reflects the current state.
10. A manual toggle wins over auto-collapse: if the user has toggled a block while it was still streaming, that block does not auto-collapse when reasoning finishes — it stays in the state the user chose.
11. Toggling is mouse-only in this version. There is no keyboard shortcut for collapse/expand.
12. Collapsing or expanding a block reflows the transcript correctly: the block's height changes to match its new state and surrounding content shifts accordingly, with no clipped or overlapping rows.
13. Reasoning is always rendered: shown expanded while streaming and auto-collapsed on finish (per 6–10). There is no user setting to change this behavior in this version (see Non-goals).
14. An exchange may contain more than one reasoning segment. Each renders as its own independent thinking block with its own header, chevron, and collapse state; collapsing one does not affect another.
15. A thinking block appears as soon as the exchange has reasoning, even before any reasoning text has streamed: the `Thinking...` header shows with an empty body area until text arrives.
16. Header text and reasoning body use the terminal theme's bright-black (dim) color; there are no hard-coded colors.
17. Reasoning is the only content rendered as a collapsible thinking block. Transcript spacing is uniform: every rendered section (input, plain text, tool call, thinking) is followed by one blank row. This replaces the previous spacing scheme (a single gap at the input→output boundary with consecutive output sections packed tightly), so exchanges with multiple output sections render with a blank row between each section.
## Non-goals
- Honoring the thinking display setting (`Never show` / `Always show` / `Show & collapse`). This version always shows reasoning expanded while streaming and auto-collapses on finish; wiring the setting is deferred.
- Keyboard-driven collapse/expand (mouse only for now).
- An internal scroll region or height cap on the streaming reasoning body (the transcript scrolls instead).
- Rendering non-plaintext reasoning content (code blocks, tables, images, diagrams) inside the body; only plain text is shown initially.
