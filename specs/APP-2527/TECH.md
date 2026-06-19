# MCP Tool Call JSON Tree Rendering — Tech Spec
Linear: APP-2527
Companion product spec: [`specs/APP-2527/PRODUCT.md`](./PRODUCT.md)

## Context
Today an expanded MCP tool-call detail is rendered as a single selectable, monospace, pretty-printed JSON string. Both the request arguments and the response are concatenated into one `String` and shown in one `Text` element. We are replacing that body with an interactive, collapsible, theme-colored JSON tree — built as a generic, reusable `warpui`-style component so it can serve other surfaces (MCP resource results, structured agent outputs, etc.) without re-implementation. The collapsed header row, accept/reject flow, and all non-MCP action rendering are unchanged.

All references pinned to commit `46265f499a3a32a488f640c0fce7565bb763496f`.

Key existing code:
- [`app/src/ai/blocklist/inline_action/requested_command.rs` (1393-1484) @ 46265f4](https://github.com/warpdotdev/warp/blob/46265f499a3a32a488f640c0fce7565bb763496f/app/src/ai/blocklist/inline_action/requested_command.rs#L1393-L1484) — `RequestedCommandView::render`'s `should_render_mcp_content` branch. This is the exact block being replaced.
- [`app/src/ai/blocklist/inline_action/requested_command.rs` (1438-1445) @ 46265f4](https://github.com/warpdotdev/warp/blob/46265f499a3a32a488f640c0fce7565bb763496f/app/src/ai/blocklist/inline_action/requested_command.rs#L1438-L1445) — where `CallMCPToolResult::{Success,Error,Cancelled}` is turned into `result_text`.
- [`app/src/ai/blocklist/inline_action/requested_command.rs` (475-481) @ 46265f4](https://github.com/warpdotdev/warp/blob/46265f499a3a32a488f640c0fce7565bb763496f/app/src/ai/blocklist/inline_action/requested_command.rs#L475-L481) — `RequestedCommandView` fields including `mcp_content_selection_handle` and `mcp_content_selected_text`. Tree expansion state will be added here.
- [`app/src/ai/blocklist/block.rs` (2043-2080) @ 46265f4](https://github.com/warpdotdev/warp/blob/46265f499a3a32a488f640c0fce7565bb763496f/app/src/ai/blocklist/block.rs#L2043-L2080) — where `command_text` is built as `MCP Tool: {name} ({display_input})` including integer-coercion via `coerce_integer_args`. The raw `input: serde_json::Value` and `name` are available here.
- [`app/src/ai/blocklist/action_model/execute/call_mcp_tool.rs` (105-126, 169-286) @ 46265f4](https://github.com/warpdotdev/warp/blob/46265f499a3a32a488f640c0fce7565bb763496f/app/src/ai/blocklist/action_model/execute/call_mcp_tool.rs#L105-L126) — `coerce_integer_args` (`pub(crate)`), already reused by `block.rs`.
- [`crates/ai/src/agent/action_result/mod.rs` (1056-1077) @ 46265f4](https://github.com/warpdotdev/warp/blob/46265f499a3a32a488f640c0fce7565bb763496f/crates/ai/src/agent/action_result/mod.rs#L1056-L1077) — `CallMCPToolResult::Success { result: rmcp::model::CallToolResult }`, `Error(String)`, `Cancelled`. `CallToolResult` carries `structured_content: Option<serde_json::Value>` and `content: Vec<Content>` (text items).
- [`crates/warp_core/src/ui/theme/color.rs` (361-424) @ 46265f4](https://github.com/warpdotdev/warp/blob/46265f499a3a32a488f640c0fce7565bb763496f/crates/warp_core/src/ui/theme/color.rs#L361-L424) — `WarpTheme` ANSI accessors (`ansi_fg_green/yellow/blue/cyan/magenta`) and `internal_colors::{text_main, text_sub, text_disabled}`.
- [`app/src/ai/blocklist/inline_action/inline_action_header.rs` (96-107) @ 46265f4](https://github.com/warpdotdev/warp/blob/46265f499a3a32a488f640c0fce7565bb763496f/app/src/ai/blocklist/inline_action/inline_action_header.rs#L96-L107) — existing chevron/expansion plumbing (`render_expansion_icon`) for visual consistency.

See `PRODUCT.md` for user-visible behavior; this spec does not restate it.

## Design alternatives

### A. Widget architecture: where does the tree component live?

**Option A1 — Generic `warpui`-level component (recommended)**
Add a standalone `JsonTreeView` as a `warpui`-level element (e.g. in `crates/warpui/src/elements/json_tree.rs` or a new `app/src/ui_components/json_tree.rs`). The component takes a `&serde_json::Value`, a `JsonTreeState` (expansion map), a `JsonTreeColors` (pre-resolved theme colors), and callbacks for toggle/copy, and returns a `Box<dyn Element>`. It has no dependency on agent-specific types.

Pros:
- Directly reusable for `ReadMCPResourceResult`, structured agent outputs, settings inspectors, or any future surface showing JSON.
- Clear ownership boundary; agent code calls the component but does not contain rendering logic.
- Testable in isolation without agent scaffolding.

Cons:
- Requires deciding the right crate layer (app-level component vs. warpui crate) before starting — small upfront decision.
- Slightly more initial setup than embedding inline.

**Option A2 — Inline in `requested_command.rs`**
Put the tree rendering functions directly inside `requested_command.rs` or a sibling `mcp_json_tree.rs` in the `inline_action` module.

Pros:
- Zero new crate surface; minimal change to module organization.
- Faster to write initially.

Cons:
- Code is not reusable without copy-paste or moving it later.
- Conflates MCP-specific logic (result parsing, integer coercion) with generic tree rendering.

**Recommendation: A1.** The minimal extra setup pays off immediately — `ReadMCPResourceResult` is the obvious next user. We can house it in `app/src/ui_components/json_tree.rs` (same layer as other reusable view utilities) to avoid a warpui crate dependency on `serde_json`.

---

### B. Element construction: recursive build vs. flattened virtualized list

**Option B1 — Recursive element build (recommended)**
Build a `Flex::column` of rows recursively, traversing only the expanded portion of the tree. Collapsed nodes contribute one row; their children are skipped entirely.

Pros:
- Simple implementation: natural match to the JSON recursive structure.
- Zero per-node overhead for collapsed subtrees — large payloads stay fast as long as users don't expand everything.
- Straightforward to add per-row click handlers, indentation spacers, and formatted-text spans.

Cons:
- If a user expands a very deep/wide tree, all rows are materialized at once. In pathological cases (e.g. 10,000-element flat array fully expanded) this could be slow.

**Option B2 — Flattened virtualized list**
Pre-walk the visible tree into a flat `Vec<TreeRow>`, then render only the rows in the viewport using a virtualized scroll container.

Pros:
- Handles arbitrarily large fully-expanded trees efficiently.

Cons:
- Substantially more complex: requires a virtualization primitive that doesn't exist in `warpui` today.
- MCP payloads are rarely large enough to need this.

**Recommendation: B1**, with a follow-up cap (e.g. "show first N items then a '…show more' row") if real-world payloads prove problematic. The cap can be added entirely inside the `JsonTreeView` component without changing the caller.

---

### C. Expansion state storage: path-keyed vs. node-identity-keyed

**Option C1 — Path-keyed `HashMap<JsonPath, bool>` (recommended)**
A `JsonPath` is a stable sequence of key/index segments (e.g. `["response", "files", 2]`) derived by traversing the tree. State is looked up by path on each render.

Pros:
- Robust to streaming re-parses: the same logical node keeps its expansion state as bytes arrive, because the path is deterministic for a given position in the JSON structure.
- No need to assign stable IDs to tree nodes.

Cons:
- Path derivation adds a small cost per render; negligible for MCP payload sizes.
- Two structurally identical sibling objects share the same path — but in practice this is harmless (toggling either sibling restores the same state for both, which is acceptable).

**Option C2 — Node-identity-keyed (e.g. pointer or arena index)**
Assign each node a stable integer ID at parse time.

Pros:
- O(1) lookup by ID; truly independent state for structurally identical siblings.

Cons:
- Requires an arena allocator or pre-walk step to assign IDs.
- IDs are invalidated on re-parse (streaming), requiring a reconciliation step to preserve expansion state.

**Recommendation: C1.** The streaming-stability advantage is decisive; the structural-sibling limitation is not meaningful in practice.

---

### D. Request data flow: structured value vs. re-parsing the string

**Option D1 — Store coerced `serde_json::Value` on `RequestedCommandView` (recommended)**
Extend `handle_mcp_tool_stream_update` in `block.rs` (lines 2059-2079) to pass the coerced `display_input: serde_json::Value` and `name: String` alongside `command_text`. Add a `mcp_request: Option<McpRequest { name, args }>` field to `RequestedCommandView`.

Pros:
- Clean: no lossy string round-trip; integer coercion is inherited from the existing `coerce_integer_args` path.
- The structured value is already available at the call site.

Cons:
- Requires touching the `handle_mcp_tool_stream_update` call signature.

**Option D2 — Re-parse `command_text` in the view**
Extract the JSON from the `"MCP Tool: name (<value>)"` string at render time.

Pros:
- No changes to call sites.

Cons:
- Fragile: the format string is not stable and the outer wrapper makes clean JSON extraction unreliable.
- Integer coercion would need to be re-applied.

**Recommendation: D1.**

---

### E. Context menu / Copy JSON implementation

**Option E1 — Custom right-click handler with `warpui` Menu (recommended)**
Use `Hoverable::with_on_right_click` (already used in other inline actions) to show a `Menu` element containing "Copy" and "Copy JSON" items. Each row in the tree registers its own right-click handler, capturing the `JsonPath` of that row.

Pros:
- Consistent with existing right-click menus elsewhere in the app.
- Per-row context (the path captured in the handler) allows "Copy JSON" to copy exactly the subtree at that row.

Cons:
- Each rendered row needs a right-click handler, adding a small amount of per-row boilerplate.

**Option E2 — Single root right-click handler + hit-test**
Attach one right-click handler to the whole tree container and determine which row was clicked by hit-testing the mouse position.

Pros:
- Fewer closures.

Cons:
- Hit-testing is non-trivial with the existing element model and would require storing row bounding boxes.

**Recommendation: E1.** Per-row handlers are simpler and follow existing patterns.

## Proposed changes and phasing

The implementation naturally divides into three phases. Each phase is independently reviewable and shippable.

---

### Phase 1 — Generic `JsonTreeView` component and unit tests

**Goal:** A standalone, tested component that renders a `serde_json::Value` as an interactive tree. No changes to any agent or MCP code in this phase.

**Files:**
- `app/src/ui_components/json_tree.rs` (new) — the `JsonTreeView` component. Public surface:
  - `pub struct JsonTreeColors` — resolved `ColorU`s per value type, built from `WarpTheme` (see Design §A1 for mapping).
  - `pub struct JsonTreeState` — `HashMap<JsonPath, bool>` for node expansion + long-string expansion (Design §C1). `JsonPath = Rc<[PathSegment]>`; `PathSegment = Key(String) | Index(usize)`. Methods: `is_expanded(path, depth) -> bool` (default: expanded at depth 0, collapsed deeper), `toggle(path)`.
  - `pub fn render_json_tree(root, root_label, state, colors, on_toggle, on_copy_json, appearance) -> Box<dyn Element>` — builds a `Flex::column` of rows (Design §B1). Each row is a `Flex::row` of: indent spacer (depth × `INDENT_PX = 12.`), chevron icon (only for containers/long strings), `FormattedTextElement` of colored key/value spans, and a right-click `Hoverable` (Design §E1) that opens a `Menu` with Copy and Copy JSON items.
- `app/src/ui_components/mod.rs` — declare `json_tree`.
- `app/src/ui_components/json_tree_tests.rs` (new, `#[cfg(test)]`) — pure logic tests:
  - Annotation formatting: `{} 0/1/N keys`, `[] 0/1/N items` (Behavior 8, 12).
  - Long-string detection at/over threshold and multi-line (Behavior 19-22).
  - Integer rendering: whole-float → integer (Behavior 29); duplicate keys retained (Behavior 30).
  - `JsonTreeState::toggle` independence: toggling one path leaves other paths unchanged (Behavior 9, 15).
  - `mcp_result_to_value` normalization (see Phase 2).

**No changes to agent or MCP code. Reviewable alone.**

---

### Phase 2 — MCP data pipeline: structured value and result normalization

**Goal:** Thread the structured `serde_json::Value` request through to `RequestedCommandView` and normalize `CallMCPToolResult` into a renderable form. Still no visible UI change (the old `Text` render path remains active).

**Files:**
- `app/src/ai/blocklist/inline_action/requested_command.rs`:
  - New fields on `RequestedCommandView`: `mcp_request: Option<McpRequest>` where `McpRequest { name: String, args: serde_json::Value }`.
  - New fields: `mcp_tree_state: JsonTreeState` — covers both request and response trees; paths namespaced by a synthetic root segment (`PathSegment::Key("__request__")` / `PathSegment::Key("__response__")`) so the two trees do not collide.
  - New `RequestedCommandViewAction` variants: `ToggleJsonNode { path: JsonPath }`, `ToggleJsonString { path: JsonPath }`. Handled in `handle_action` by calling `mcp_tree_state.toggle(...)` + `ctx.notify()`.
- `app/src/ai/blocklist/block.rs` (2059-2079) — extend `handle_mcp_tool_stream_update` to also pass `display_input: serde_json::Value` and `name: String` through to the view, populating `mcp_request` (Design §D1). Keep building `command_text` for the collapsed header.
- New helper `fn mcp_result_to_renderable(result: &CallMCPToolResult) -> McpRenderable` where:
  ```
  enum McpRenderable { Tree(serde_json::Value), Error(String), Cancelled }
  ```
  Logic: `Success { result }` → prefer `result.structured_content`; else try `serde_json::from_str` on joined text content; else wrap in a JSON `String` value. `Error(e)` → `McpRenderable::Error(e)`. `Cancelled` → `McpRenderable::Cancelled`.

**Unit tests for `mcp_result_to_renderable` added to `json_tree_tests.rs`.**
**No visible UI change; old `Text` render path still active. Reviewable alone.**

---

### Phase 3 — Replace the render body + context menu

**Goal:** Wire up the `JsonTreeView` component in place of the old `Text` + `serde_json::to_string_pretty`, add the context menu, and ship.

**Files:**
- `app/src/ai/blocklist/inline_action/requested_command.rs` (`should_render_mcp_content` block, lines 1430-1483):
  - Replace `content_text`/single-`Text` with two labeled sections (Request + Response divider, Behavior 4) each calling `render_json_tree(...)`.
  - Request section: `render_json_tree(&self.mcp_request.args, "Request", &self.mcp_tree_state, &colors, ...)` (or `null` indicator when `mcp_request` is absent, Behavior 28).
  - Response section: present only when `action_status.finished_result()` exists; dispatches to tree, error label (`Text` with `ui_error_color`), or cancelled label (Behavior 27).
  - The sections stay inside the current `Container` (padding, background, corner radius) so layout is unchanged (Behavior 32).
  - The `SelectableArea` + `mcp_content_selection_handle` wraps the whole tree column so text selection/copy still works (Behavior 23).
  - Right-click "Copy JSON" in `on_copy_json` callback: serialize the subtree at the received `JsonPath` using `serde_json::to_string_pretty`, write to clipboard.
- Remove the now-dead `.bak` intermediates (cleanup).

**Manual validation checklist** (attached to the PR, to be checked before merging):
- Configure a local MCP server (e.g. filesystem) and expand a tool call: root expanded, nested collapsed (Behavior 13), chevrons toggle independently (Behavior 9), indentation per level (Behavior 10).
- Large/nested response: Request/Response labels + divider visible (Behavior 4), typed colors for all value types (Behavior 16-18), light↔dark theme switch recolors without restart (Behavior 17).
- Long string (file contents): elision preview + chevron, expands/collapses in place without disturbing siblings (Behavior 19-20).
- Error and cancelled tool calls show labeled messages (Behavior 27).
- Right-click → Copy JSON on a collapsed container copies complete JSON (Behavior 25).
- Right-click → Copy JSON on the Request label copies the full request JSON (Behavior 26).
- Text selection and copy across key/value rows works (Behavior 23).
- Collapsed header, accept/reject, and a non-MCP action (shell command) are visually unchanged (Behavior 1, 32-34).
- Screenshots of expanded tree in dark and light themes attached to the PR.

## Testing and validation summary

| Invariant(s) | Test type | Where |
|---|---|---|
| Annotation labels (8, 12) | Unit | `json_tree_tests.rs` |
| Toggle independence (9, 15) | Unit | `json_tree_tests.rs` |
| Long string detection (19-22) | Unit | `json_tree_tests.rs` |
| Integer/unusual values (29-30) | Unit | `json_tree_tests.rs` |
| `mcp_result_to_renderable` (27) | Unit | `json_tree_tests.rs` |
| Null/absent request (28) | Unit | `json_tree_tests.rs` |
| Streaming expansion stability (31) | Unit | `json_tree_tests.rs` |
| All visual/interaction behaviors | Manual | PR checklist (Phase 3) |

## Risks and mitigations
- **Performance on very large payloads.** Mitigated by rendering only expanded nodes (Design §B1); a "show first N / show more" cap can be added inside `JsonTreeView` as a follow-up without changing callers.
- **Selection regression.** The current single-`Text` selection is well-understood; wrapping the tree in the same `SelectableArea`/`SelectionHandle` with `FormattedTextElement` keeps the selection model intact — called out explicitly in the Phase 3 PR for reviewer attention.
- **Streaming flicker.** Path-keyed state (Design §C1) prevents losing expansion when request args stream in; covered by unit tests.
- **Copy JSON clipboard access.** Clipboard writes already work in other right-click menus in the app; same mechanism applies here.

## Follow-ups
- Auto-collapse of very large roots (Behavior 14 open question).
- Reuse `JsonTreeView` for `ReadMCPResourceResult` and other JSON-bearing surfaces (natural next consumer after Phase 3 ships).
- Potential virtualization for pathologically large expanded trees (Design §B2), if needed.
