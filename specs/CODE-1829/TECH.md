# TUI agent task list — TECH

Linear: [CODE-1829](https://linear.app/warpdotdev/issue/CODE-1829/agent-task-list)
Design: [Figma — TUI / Task list (node 323:17832)](https://www.figma.com/design/yg5nbPZuGoAszHS3Rhvehu/TUI?node-id=323-17832)

## Context

The agent's todo list is invisible in the TUI transcript. The Figma design renders it inline in the conversation as a collapsible block: a bold `☰ Tasks 3 ▾` header row followed by one row per task — a yellow `•` glyph for the in-progress item, `◌` for pending items, white titles indented under the header.

All of the data already flows through production-shaped models the TUI consumes:

- `app/src/ai/agent/mod.rs:1716` — `TodoOperation` (`UpdateTodos { todos }` / `MarkAsCompleted { completed_todos }`) arrives in exchange output as `AIAgentOutputMessageType::TodoOperation`.
- `app/src/ai/agent/todos/mod.rs` — `AIAgentTodoList` (pending + completed items) is the aggregate list state.
- `app/src/ai/agent/conversation.rs (3256-3299)` — `AIConversation` owns the `todo_lists` stack; `active_todo_list()` returns the latest, and `todo_status(&AIAgentTodoId)` derives per-item `TodoStatus` (`Pending`, `InProgress`, `Completed`, `Cancelled`, `Stopped`), including cancelling items from superseded lists.
- `app/src/ai/blocklist/block/view_impl/todos.rs` — the GUI reference: `render_todos` renders `UpdateTodos` as a collapsible "Tasks" card with per-item status icons from `conversation.todo_status()` (falling back to `Cancelled` for unknown ids), and `render_completed_todo_items` renders `MarkAsCompleted` as a one-line `✓ Completed <title> (n/m)` row using the active list for index/length.
- `app/src/ai/blocklist/block.rs (573-587, 6761-6765)` — GUI per-message `TodoListElementState` defaults to expanded; collapse only changes on manual toggle.

On the TUI side:

- `crates/warp_tui/src/agent_block.rs:325` — `TuiAIBlock::sections` walks `output.messages` in order and currently drops `TodoOperation` (line 385) in the unsupported-message arm.
- `crates/warp_tui/src/agent_block.rs (54-102)` — `ThinkingBlockStates` is the existing per-message collapse/hover state pattern (manual override map + owned `MouseStateHandle`s keyed by `MessageId`).
- `crates/warp_tui/src/agent_block_sections.rs` — pure per-section render functions; `render_thinking_section` already composes `TuiUiBuilder::collapsible` for a toggleable header + indented body, and `render_fallback_tool_call_section` establishes the glyph-gutter row style (colored state glyph, then label).
- `app/src/tui_export.rs` — the narrow export boundary; `BlocklistAIHistoryModel` is already exported, but none of the todo types are.

Per the design review: mirror the GUI's full `TodoStatus` taxonomy (not just the two states visible in the Figma frame); default expanded with manual-toggle-only collapse, matching the GUI. `MarkAsCompleted` progress rows are included (matching the GUI). The GUI's "Outdated" badge is deferred — superseded lists just show cancelled-styled items.

## Proposed changes

### tui_export additions

Export the todo data types `warp_tui` must name: `TodoOperation`, `AIAgentTodo`, `AIAgentTodoId`, `AIAgentTodoList`, and `TodoStatus`.

Do not export `AIConversation`. Instead, add two narrow projections on the already-exported `BlocklistAIHistoryModel`, following the `ConversationUsageTotals` precedent of keeping conversation internals out of the boundary:

```rust
pub fn todo_status(
    &self,
    conversation_id: &AIConversationId,
    todo_id: &AIAgentTodoId,
) -> Option<TodoStatus>;

pub fn active_todo_list(
    &self,
    conversation_id: &AIConversationId,
) -> Option<&AIAgentTodoList>;
```

Both delegate to the conversation's existing methods.

### Section adapter

Add two variants to `TuiAIBlockSection` and matching arms in `TuiAIBlock::sections`:

```rust
TodoList {
    message_id: MessageId,
    todos: Vec<AIAgentTodo>,
},
CompletedTodos {
    completed: Vec<AIAgentTodo>,
},
```

- `TodoOperation::UpdateTodos { todos }` with non-empty `todos` becomes `TodoList`; empty lists are ignored (matching the GUI's guard).
- `TodoOperation::MarkAsCompleted { completed_todos }` becomes `CompletedTodos`.

Ordering stays sourced from the single pass over `output.messages`, per the established adapter rules in `specs/tui-agent-tool-calls/TECH.md`.

### Collapse state

Rename `ThinkingBlockStates` to a shared `CollapsibleSectionStates` and reuse the same instance for task lists. The struct's semantics already fit both consumers: `is_collapsed(message_id, default_collapsed)` returns the manual override if recorded, else the default. Thinking sections keep passing `finished` as the default; task lists pass `false` (always default-expanded, manual toggle wins permanently — the GUI behavior). `MessageId`s are unique across message kinds, so one map per block cannot collide.

### Renderers

Add to `crates/warp_tui/src/agent_block_sections.rs`:

`render_todo_list_section(states, message_id, todos, conversation_id, app)`:
- Header: `☰ Tasks {todos.len()}` rendered through `TuiUiBuilder::collapsible` with the block's persistent hover state, bold/primary styling per the Figma. The collapsible helper owns the `▾`/`▸` affordance and toggle callback (record override, `event_ctx.notify()`), exactly like the thinking section.
- Body: one row per todo, indented under the header, in a glyph gutter + title layout mirroring `render_fallback_tool_call_section`. Status comes from the new `BlocklistAIHistoryModel::todo_status` projection, falling back to `Cancelled` like the GUI. Glyph/style mapping:
  - `InProgress` — `•` with `attention_glyph_style` (yellow), title in `primary_text_style` (Figma).
  - `Pending` — `◌` and title in `primary_text_style` (Figma).
  - `Completed` — `✓` with `success_glyph_style`, title in `primary_text_style`.
  - `Cancelled` — cancelled glyph from the tool-call glyph conventions, title in `muted_text_style` with the crossed-out modifier if `TuiText` styling supports it; otherwise muted color alone.
  - `Stopped` — stop glyph, `muted_text_style`.

  Reuse/extend the glyph table in `crates/warp_tui/src/tool_call_labels.rs` rather than introducing a second glyph vocabulary.

`render_completed_todos_section(completed, active_list, app)`:
- A single muted row: `✓ Completed <title> (n/m)`, joining multiple completions with commas, using `active_todo_list` for index/length and omitting `(n/m)` when the item is not in the active list — a direct port of the GUI's `render_completed_todo_items` text logic. Returns nothing when the text would be empty.

`TuiAIBlock::render_element` dispatches the new variants; `TuiAIBlock` passes its `conversation_id` through to the todo-list renderer for status lookups.

### Redraw and heights

- New `TodoOperation` messages enter through the existing response-stream path; `UpdatedStreamingExchange` dirties the owning rich-content item, and `TuiBlockListViewportSource` re-measures via `desired_height`.
- Per-item statuses are projections of conversation-wide state. `UpdatedTodoList` therefore dirties every agent block on the terminal surface: a new list can make rows in older exchanges `Cancelled`. `UpdatedConversationStatus` dirties blocks for that conversation because the first pending row switches between `InProgress` and `Stopped`.
- Collapse toggles change height and follow the same notify path the thinking section already uses.

## Testing and validation

Unit tests in `crates/warp_tui/src/agent_block_tests.rs`:
- `UpdateTodos` produces a `TodoList` section in message order between adjacent text/tool-call sections; empty `UpdateTodos` is ignored.
- `MarkAsCompleted` produces a `CompletedTodos` section; the rendered text includes `(n/m)` when the item is in the active list and omits it otherwise.
- Glyph/style mapping per `TodoStatus`, including the `Cancelled` fallback for ids missing from the conversation.
- Default-expanded behavior and manual collapse override via `CollapsibleSectionStates`, and `desired_height` differing between expanded and collapsed.
- Existing thinking-section tests keep passing after the state-struct rename.

App-side test for the two `BlocklistAIHistoryModel` projections (status delegation and active-list lookup) next to existing history-model tests.

Manual validation: run `cargo run -p warp_tui`, prompt the agent into a multi-step task that creates todos, and verify the Tasks block matches the Figma (header count, glyphs, indentation), updates as items complete, and collapses/expands on header click.

Run:
- `cargo nextest run -p warp_tui`
- `cargo nextest run -p warp history_model`
- `./script/format`
- `cargo clippy --workspace --all-targets --all-features --tests -- -D warnings`

## Parallelization

No child agents. This is a small, tightly coupled change inside one crate plus a narrow export addition — the adapter variant, renderers, and state rename all touch the same files, so parallel branches would only create merge overhead. Implement sequentially on `ian/code-1829-agent-task-list`.

## Follow-ups

- "Outdated" indicator for superseded task lists (GUI parity), deferred from v1.
- Strikethrough support in `TuiText` styling if the crossed-out modifier is not currently plumbed through, so cancelled items can match GUI treatment fully.
