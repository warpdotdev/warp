# MCP Tool Call JSON Tree Rendering — Product Spec
Linear: APP-2527
Figma: none provided (reference screenshot supplied in the originating request showing a generic collapsible JSON tree with chevron expanders and typed value colors; the visual treatment must follow Warp UI conventions, not the screenshot's exact styling)

## Summary
Render the JSON request (arguments) and JSON response of an MCP tool call in the agent UI as an interactive, collapsible tree with chevron expanders and theme-driven colors for keys and values, instead of the current flat pretty-printed JSON blob. Long string values are elided by default and can be expanded in place via a chevron. The underlying JSON tree widget is designed as a generic, reusable UI component.

## Problem
When the agent calls an MCP tool, expanding the tool-call detail currently shows the request arguments and the response as a single unformatted pretty-printed JSON string. For anything beyond a couple of fields this is hard to scan: there is no way to collapse uninteresting sub-objects, no visual distinction between keys and values, and long string values (file contents, logs, base64, stack traces) blow out the height of the view and bury the rest of the structure. Users need to quickly understand what was sent to a tool and what came back.

## Example

The following mock illustrates the intended rendering style for an expanded MCP tool call detail. `▶` is the right-pointing (collapsed) chevron; `▼` is the down-pointing (expanded) chevron. Colors represent value types — keys in cyan/blue, strings in green, numbers in yellow, booleans in magenta, null muted, type annotations in secondary text. Exact palette is determined by Warp theme tokens, not by this mock.

```
Request
  ▼ {} 3 keys
      path:   "/home/user/project"
      depth:  2
    ▼ filters:  [] 2 items
          0:  "*.rs"
          1:  "*.toml"

Response
  ▼ {} 2 keys
      count:  42
    ▶ files:  [] 3 items          ← collapsed; click to expand
```

Long string elision (collapsed → expanded):

```
  summary:  "This is a very long descri…"  ▶
                           ↓ click ▶
  summary:  ▼ "This is a very long description that spans many
               characters and would dominate the view if always shown."
```

## Behavior

### Where this applies
1. The tree rendering applies wherever the expanded detail of an MCP tool call is shown in the agent block list — both the request arguments and the response body. It replaces the current single selectable pretty-printed JSON text for MCP tool calls. It does not change the collapsed header row (the one-line `MCP Tool: <name>` summary), the accept/reject affordances, or any non-MCP action rendering (shell commands, file edits, etc.).
2. The tree is shown only when the MCP tool-call detail is expanded, matching today's behavior where MCP content appears only when the action header is expanded. Collapsing the header hides the tree.

### Request and response sections
3. When expanded, the detail shows the request arguments as a tree. The root of the request tree is the tool's argument object (the JSON passed to the tool).
4. Once a response is available, the detail additionally shows the response as a tree below the request, under a clear visual/label separation between request and response (e.g. "Request" and "Response" labels with a divider). Before a response exists, only the request is shown.
5. If the tool call is still pending (blocked awaiting approval, or running), the request tree renders as soon as the arguments are known; the response section is absent until a result arrives.

### Tree structure and expansion
6. Each JSON value renders as one of: object (`{}`), array (`[]`), string, number, boolean, or null.
7. Objects and arrays are collapsible nodes. Each collapsible node renders a chevron expander at its left edge: pointing right when collapsed, pointing down when expanded. Scalar values (string, number, boolean, null) have no chevron and are not collapsible (except long strings — see Long string elision).
8. A collapsible node's row shows, in order: the chevron, the key (when the node is a member of an object) or the index (when it is an element of an array), and a type/size annotation. The annotation conveys the container type and item count, e.g. `{} 4 keys`, `{} 1 key`, `[] 3 items`, `[] 1 item`, `[] 0 items`, `{} 0 keys`. The annotation count is the sole mechanism for conveying that a node is non-empty; no inline preview of child keys or values is shown on a collapsed row.
9. Clicking anywhere on a collapsible node's row (chevron or label) toggles its expanded/collapsed state. Toggling one node does not change the state of any sibling, ancestor, or descendant node.
10. When a node is expanded, its children render indented one level deeper than the node, vertically stacked, each on its own row. Indentation depth increases by a consistent amount per nesting level so structure is visually obvious.
11. Child rows of an object show `key: value` where the key is the object member name. Child rows of an array show `index: value` where the index is the 0-based position. Scalar children render their value inline on the same row as the key/index; object/array children render as nested collapsible nodes.
12. An empty object renders as `{} 0 keys` and an empty array as `[] 0 items`; they have no chevron and do not respond to click, and never expand to an empty body.

### Default expansion state
13. On first render of a tool call's detail, all nodes in the tree are expanded by default so the user immediately sees the full structure. The tree body is scrollable and height-capped, so a large tree does not push subsequent blocks off-screen.
14. There is no auto-collapse cap based on node count: the tree is fully open by default regardless of size.
15. Expansion state is per tool-call-detail view state. It persists while the conversation stays open (collapsing and re-expanding the action header restores the user's last per-node expansion state for that tool call rather than resetting to defaults). It does not need to persist across app restarts or conversation reloads.
16. If a tool call response arrives while the action header is collapsed, the response data is retained; expanding the header shows both the request and response trees. No data is lost due to the header being collapsed at the time of response arrival.
17. The tree body scrolls vertically when the expanded tree exceeds the height of the action detail container. A maximum height cap is applied to the tree body (consistent with the existing max-height cap used for command editor bodies) so that a fully expanded tree does not push subsequent blocks off-screen. Scrolling the tree does not interfere with scrolling the outer block list.

### Typed colors
18. Keys, and each scalar value type, render in visually distinct colors sourced from the active Warp theme (no hard-coded colors). At minimum these categories are visually distinguishable from each other and from plain body text: object/array keys (and array indices), string values, number values, boolean values, and null values. Container type/size annotations (`{} 4 keys`) render in a muted/secondary text color.
19. The colors adapt to the active theme and remain legible against the detail's background in both light and dark themes; they derive from theme tokens so a theme switch updates them without restart.
20. Punctuation/structural glyphs (braces, brackets, colons, quotes around strings) follow a consistent, readable treatment and must not be mistaken for values.

### Long string elision
21. A string value whose length exceeds a threshold (single-line display length) is elided by default: it shows a truncated preview ending in an ellipsis affordance, with a chevron (or equivalent expander) indicating it can be expanded.
22. Activating a long string's expander reveals the full string value in place (wrapped across lines as needed) without collapsing or disturbing surrounding nodes; activating it again re-collapses to the elided preview. Toggling a long string is independent of object/array node expansion state and follows the same persistence rule as node expansion (invariant 15).
23. Strings at or below the threshold render in full inline with no expander.
24. Multi-line strings (containing newlines) are treated as long for elision purposes: the collapsed preview shows the first line (or a truncated portion) with the expander; expanding shows the full multi-line content.

### Selection, copy, and context menu
25. The user can select text within the rendered tree (keys and values) and copy it with the standard copy shortcut. Copying a selection yields the visible text of the selected region. Copy with no selection is a no-op.
26. Right-clicking a tree node row or a Request/Response section label shows a context menu with at minimum:
    - **Copy** — copies the current text selection. Disabled (greyed out) when nothing is selected.
    - **Copy JSON** — copies the complete raw JSON of the subtree rooted at the right-clicked node (or the full section JSON when the label is right-clicked), formatted as pretty-printed JSON. For a scalar node this copies the scalar value as its JSON representation.
27. "Copy JSON" always copies the complete underlying JSON, regardless of whether the node is collapsed or expanded. This allows extracting a subtree without having to fully expand it first.

### Malformed / edge-case data
28. The response of an MCP tool call may not be a single JSON object — it can be structured content, one or more text content items, or an error. Rendering handles each:
    - Structured/JSON content renders as the tree described above.
    - Plain text content that is not valid JSON renders as a string value (subject to long-string elision), not as a failed/empty tree.
    - An error result renders as a clearly labeled error message (e.g. `Error: <message>`) rather than an empty or misleading tree.
    - A cancelled tool call renders a clear "cancelled" indication rather than an empty tree.
29. If the request arguments are absent or null (a tool called with no arguments), the request tree renders an empty/`null` indication rather than a broken node.
30. Values that are valid JSON but unusual — empty string, very large numbers, numbers that are whole-valued floats, unicode, nested arrays of objects — all render without panicking and without losing data. Whole-number integer arguments display as integers (e.g. `5`, not `5.0`), consistent with how the tool call is actually dispatched.
31. Duplicate object keys (possible in raw JSON) all render; none are silently dropped.

### Streaming
32. While the tool-call request arguments are still streaming in, the request tree may update as more of the structure arrives; partial/in-progress structure renders without flicker that resets the user's expansion state for already-rendered nodes.

### Consistency and non-regression
33. The expanded MCP detail remains inside the same bordered action container it uses today, with the same surrounding spacing, header, and footer behavior; only the body content (formerly a JSON blob) changes to the tree.
34. Keyboard accept/reject/expand behavior of the action header is unchanged.
35. Non-MCP action details (commands, edits, web fetch, etc.) are visually unaffected by this change.
