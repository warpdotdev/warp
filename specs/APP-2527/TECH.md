# MCP Tool Call JSON Tree Rendering — Tech Spec
Linear: APP-2527
Companion product spec: [`specs/APP-2527/PRODUCT.md`](./PRODUCT.md)

## Context
Today an expanded MCP tool-call detail is rendered as a single selectable, monospace, pretty-printed JSON string. Both the request arguments and the response are concatenated into one `String` and shown in one `Text` element. We are replacing that body with an interactive, collapsible, theme-colored JSON tree. The collapsed header row, accept/reject flow, and all non-MCP action rendering are unchanged.

All references pinned to commit `46265f499a3a32a488f640c0fce7565bb763496f`.

Key existing code:
- [`app/src/ai/blocklist/inline_action/requested_command.rs` (1393-1484) @ 46265f4](https://github.com/warpdotdev/warp/blob/46265f499a3a32a488f640c0fce7565bb763496f/app/src/ai/blocklist/inline_action/requested_command.rs#L1393-L1484) — `RequestedCommandView::render`'s `should_render_mcp_content` branch. This is the exact block being replaced: it computes `content_text` (request `command_text` plus `Response: <to_string_pretty>`) and wraps a single `Text` in a `SelectableArea`.
- [`app/src/ai/blocklist/inline_action/requested_command.rs` (1438-1445) @ 46265f4](https://github.com/warpdotdev/warp/blob/46265f499a3a32a488f640c0fce7565bb763496f/app/src/ai/blocklist/inline_action/requested_command.rs#L1438-L1445) — where `CallMCPToolResult::{Success,Error,Cancelled}` is turned into `result_text`.
- [`app/src/ai/blocklist/inline_action/requested_command.rs` (475-481) @ 46265f4](https://github.com/warpdotdev/warp/blob/46265f499a3a32a488f640c0fce7565bb763496f/app/src/ai/blocklist/inline_action/requested_command.rs#L475-L481) — `RequestedCommandView` fields: `command_text`, `mcp_content_selection_handle: SelectionHandle`, `mcp_content_selected_text`. Tree expansion state will be added here.
- [`app/src/ai/blocklist/block.rs` (2043-2080) @ 46265f4](https://github.com/warpdotdev/warp/blob/46265f499a3a32a488f640c0fce7565bb763496f/app/src/ai/blocklist/block.rs#L2043-L2080) — where the request `command_text` is built as `MCP Tool: {name} ({display_input})`, including the integer-coercion of `display_input` (`coerce_integer_args`). The raw `input: serde_json::Value` and tool `name` are available here; today they are flattened to a string via `handle_mcp_tool_stream_update`.
- [`app/src/ai/blocklist/action_model/execute/call_mcp_tool.rs` (105-126, 169-286) @ 46265f4](https://github.com/warpdotdev/warp/blob/46265f499a3a32a488f640c0fce7565bb763496f/app/src/ai/blocklist/action_model/execute/call_mcp_tool.rs#L105-L126) — `coerce_integer_args` lives here (`pub(crate)`), already reused by `block.rs` for display.
- [`crates/ai/src/agent/action_result/mod.rs` (1056-1077) @ 46265f4](https://github.com/warpdotdev/warp/blob/46265f499a3a32a488f640c0fce7565bb763496f/crates/ai/src/agent/action_result/mod.rs#L1056-L1077) — `CallMCPToolResult::Success { result: rmcp::model::CallToolResult }`, `Error(String)`, `Cancelled`. `CallToolResult` carries `structured_content: Option<serde_json::Value>` and `content: Vec<Content>` (text items).
- [`crates/warp_core/src/ui/theme/color.rs` (361-424) @ 46265f4](https://github.com/warpdotdev/warp/blob/46265f499a3a32a488f640c0fce7565bb763496f/crates/warp_core/src/ui/theme/color.rs#L361-L424) — `WarpTheme` ANSI accessors (`ansi_fg_green/yellow/blue/cyan/magenta/red`) and `internal_colors::{text_main, text_sub, text_disabled}`, the theme tokens that will drive typed coloring.
- [`app/src/ai/blocklist/inline_action/inline_action_header.rs` (96-107) @ 46265f4](https://github.com/warpdotdev/warp/blob/46265f499a3a32a488f640c0fce7565bb763496f/app/src/ai/blocklist/inline_action/inline_action_header.rs#L96-L107) — existing chevron/expansion plumbing (`InteractionMode::ManuallyExpandable`, `render_expansion_icon`) for visual consistency of expanders.

See `PRODUCT.md` for user-visible behavior; this spec does not restate it.

## Proposed changes

### 1. New module: `mcp_json_tree`
Add `app/src/ai/blocklist/inline_action/mcp_json_tree.rs` (declared from `inline_action/mod.rs`). It owns parsing + rendering and contains no view state of its own; expansion state is passed in by the caller so the function stays a pure render of `(value, expansion_state, theme) -> Element`. Public surface:

- `pub struct JsonTreeColors` — resolved `ColorU`s for key, string, number, bool, null, annotation (size/type), and punctuation, built once per render from `WarpTheme`. Mapping (subject to design review):
  - key / index → `ansi_fg_cyan` (or `ansi_fg_blue`)
  - string → `ansi_fg_green`
  - number → `ansi_fg_yellow`
  - bool → `ansi_fg_magenta`
  - null → `text_disabled`
  - type/size annotation + punctuation → `text_sub`
  All sourced from theme tokens (Behavior 16-18); no literals.
- `pub struct JsonTreeState` — per-tree expansion state: a `HashMap<JsonPath, bool>` of node-path → expanded, plus a `HashMap<JsonPath, bool>` for long-string expansion. `JsonPath` is a cheap stable identifier for a node (e.g. `Rc<[PathSegment]>` where `PathSegment` is `Key(String)` or `Index(usize)`), so state survives re-parse on streaming updates (Behavior 15, 29). Provides `is_expanded(path, default)` and `toggle(path)`.
- `pub fn render_json_tree(root: &serde_json::Value, root_label: Option<&str>, state: &JsonTreeState, colors: &JsonTreeColors, selection: &SelectionHandle, appearance: &Appearance, on_toggle: impl Fn(JsonPath) -> ...) -> Box<dyn Element>` — builds a `Flex::column` of rows. Each row is a `Flex::row` of: indentation spacer (depth × INDENT_STEP), chevron (only for containers / long strings), and a `FormattedTextElement` carrying the colored key/value spans. Recurses only into expanded containers, so collapsed subtrees cost nothing.

Row construction details:
- Containers render `<chevron> <key>: {} N keys` / `[] N items` using `colors.annotation`; pluralization handled (`1 key`/`N keys`, `1 item`/`N items`) (Behavior 8, 12).
- Scalars render `<key>: <value>` inline. Strings are quoted; numbers/bools/null unquoted (Behavior 11, 18).
- Long strings (len > `LONG_STRING_THRESHOLD`, or containing `\n`) render an elided preview + chevron; expanded state pulled from `JsonTreeState` long-string map (Behavior 19-22).
- Chevron uses the same icon treatment as `render_expansion_icon` for visual consistency (right when collapsed, down when expanded).
- Row click → `on_toggle(path)`. Because `RequestedCommandView` is a `TypedActionView`, toggles dispatch a new `RequestedCommandViewAction::ToggleJsonNode { path }` (see §3) rather than mutating during render.

The whole tree column is wrapped in the existing `SelectableArea` + `mcp_content_selection_handle` so text selection/copy keeps working (Behavior 23); `FormattedTextElement` content participates in selection the same way `Text` does today.

### 2. Carry structured request + response instead of a flattened string
The renderer needs `serde_json::Value`s, but today only the flattened `command_text` (`MCP Tool: {name} (...)`) reaches the view, and the response is only available via `action_status.finished_result()`.

- Request: extend the MCP path so the view has the structured arguments. Two options:
  - **(A, preferred)** Store the coerced `display_input: serde_json::Value` and `name: String` on `RequestedCommandView` (new fields, e.g. `mcp_request: Option<McpRequest { name, args }>`), populated through the existing `handle_mcp_tool_stream_update` call site in [`block.rs` (2059-2079)](https://github.com/warpdotdev/warp/blob/46265f499a3a32a488f640c0fce7565bb763496f/app/src/ai/blocklist/block.rs#L2059-L2079). Keep building `command_text` for the collapsed header title, but also pass the structured value through. This preserves the existing integer-coercion (`coerce_integer_args`) so `5` renders as `5`, not `5.0` (Behavior 27).
  - (B) Re-parse `command_text` back into JSON in the view. Rejected: lossy and fragile (the string is `MCP Tool: name (<Display of Value>)`, not guaranteed round-trippable JSON).
- Response: in the `should_render_mcp_content` branch, instead of `serde_json::to_string_pretty`, derive a `serde_json::Value` from `CallMCPToolResult`:
  - `Success { result }` → prefer `result.structured_content` when present; else if `result.content` is text item(s), attempt `serde_json::from_str` on the joined text and fall back to a JSON `String` value when it does not parse (Behavior 25). Whole-`CallToolResult` `serde_json::to_value` is the final fallback so nothing is lost.
  - `Error(e)` → render an error label (reuse a `Text` with `ui_error_color`), not a tree (Behavior 25).
  - `Cancelled` → render a "Tool call was cancelled" label (Behavior 25).

A small helper `fn mcp_result_to_value(result: &CallMCPToolResult) -> McpRenderable` (enum of `Tree(Value)` / `Error(String)` / `Cancelled`) keeps this logic testable and out of `render`.

### 3. Expansion state + actions on `RequestedCommandView`
- Add fields: `mcp_request: Option<McpRequest>`, `mcp_tree_state: JsonTreeState` (covers both request and response trees; paths are namespaced by a `Request`/`Response` root segment so the two trees don't collide).
- Add `RequestedCommandViewAction::ToggleJsonNode { path: JsonPath }` and `ToggleJsonString { path: JsonPath }`; handle them in `handle_action` by calling `mcp_tree_state.toggle(...)` + `ctx.notify()`. This mirrors the existing `SelectText` action pattern at [`requested_command.rs` (1616-1618)](https://github.com/warpdotdev/warp/blob/46265f499a3a32a488f640c0fce7565bb763496f/app/src/ai/blocklist/inline_action/requested_command.rs#L1616-L1618).
- Default expansion (root expanded, descendants collapsed) is encoded in `JsonTreeState::is_expanded`'s `default` argument keyed by depth, so no state needs pre-seeding and streaming re-parses keep working (Behavior 13, 15, 29).

### 4. Replace the render body
In the `should_render_mcp_content` block of `RequestedCommandView::render`, replace the `content_text`/single-`Text` construction with:
- a "Request" labeled section rendering `render_json_tree(&self.mcp_request.args, ...)` (or a `null`/empty indicator when absent — Behavior 26), and
- when a finished result exists, a "Response" labeled section + divider rendering either the response tree, the error label, or the cancelled label.
Both sections stay inside the current `Container` (padding, background, bottom corner radius) so container/spacing behavior is unchanged (Behavior 30). The `is_header_expanded`/`extract_mcp_tool_name` collapsed-title behavior outside this branch is untouched (Behavior 2, 31).

### Tradeoffs
- **Custom tree vs. reusing the editor/`CodeEditorView`.** The editor gives syntax highlighting but not collapsible nodes, per-node chevrons, or string elision, and is heavier. A purpose-built element tree in the existing `warpui` `Flex`/`FormattedTextElement` primitives is simpler and matches the interaction model. Chosen.
- **Recursive element build vs. flattened virtualized list.** MCP payloads are usually small; a straightforward recursive build of only-expanded nodes is adequate and far simpler than virtualization. If extremely large payloads become a problem, a follow-up can cap rendered rows with a "show more" affordance (noted in Follow-ups).
- **State keyed by path vs. by node identity.** Path-keyed state is robust to streaming re-parses (the same logical node keeps its expansion state as bytes arrive) at the cost of recomputing paths each render; acceptable for these sizes.

## Testing and validation
Unit tests (`mcp_json_tree_tests.rs`, colocated per repo convention) — pure logic, no view:
- Parsing/normalization: `mcp_result_to_value` for `Success` with `structured_content`, `Success` with JSON text content, `Success` with non-JSON text (→ string), `Error`, `Cancelled` (Behavior 25); request `null`/absent (Behavior 26).
- Annotation/labels: `{} 0 keys`, `{} 1 key`, `{} 4 keys`, `[] 0 items`, `[] 1 item`, `[] 3 items` (Behavior 8, 12).
- Long-string detection at/over threshold and multi-line (Behavior 19-22).
- Number rendering: whole-valued floats display as integers (Behavior 27); duplicate keys all retained (Behavior 28).
- `JsonTreeState` toggle independence: toggling one path leaves siblings/ancestors/descendants unchanged (Behavior 9), and state is stable across a re-parse of an equal-but-new `Value` (Behavior 15, 29).

Manual validation (via `./script/run`, mapping to Behavior):
- Configure a local MCP server (e.g. filesystem) and have the agent call a tool with nested-object arguments. Expand the detail: confirm request tree, root expanded, nested collapsed (3, 13), chevrons toggle correctly (7, 9), indentation per level (10).
- A tool returning a large/nested JSON response: confirm Request/Response sections + divider (4), typed colors for keys/strings/numbers/bools/null and muted annotations (16-18), and that switching theme (e.g. light↔dark) recolors without restart (17).
- A tool returning a long string (file contents): confirm elision + chevron and in-place expand/collapse without disturbing siblings (19-20).
- Error and cancelled tool calls render labeled messages, not empty trees (25).
- Select across keys/values and copy; confirm copied text matches selection (23).
- Confirm collapsed header, accept/reject, and a non-MCP action (shell command / file edit) detail are visually unchanged (2, 30-32).
- Screenshots/video of expanded request+response trees in a dark and a light theme attached to the PR (per template).

## Risks and mitigations
- **Performance on very large payloads.** Mitigated by rendering only expanded nodes; follow-up row cap if needed.
- **Selection regression.** The current single-`Text` selection is well understood; wrapping the multi-row tree in the same `SelectableArea`/`SelectionHandle` and using `FormattedTextElement` keeps the selection model intact — validated manually and called out for review.
- **Streaming flicker resetting expansion.** Path-keyed `JsonTreeState` (not index-/identity-keyed) prevents losing user expansion as request args stream in (Behavior 29); covered by a unit test.

## Follow-ups
- "Copy raw JSON" affordance for a collapsed subtree (Behavior 24 open question).
- Optional auto-collapse of very large roots (Behavior 14 open question).
- Reuse the same tree renderer for `ReadMCPResourceResult` and other JSON-bearing action details if it proves out.
