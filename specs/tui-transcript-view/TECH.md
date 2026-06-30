# TUI transcript view — TECH
## Context
This PR builds the first production-shaped conversation transcript view for Warp's TUI. It proves the transcript container and canonical ordering path with two intentionally simple block renderers:
- an agent block that renders user input and streamed plain-text agent output
- a terminal block that renders command/input and streamed terminal output
Bare `warp-tui` launches a real login-gated TUI root. Once authenticated, the root delegates to an authenticated terminal session view containing an editor-backed input docked at the bottom and a transcript above it. Submitting the input sends a prompt to the surface's conversation, streaming the response into the transcript as an agent block.

Rich block content and interactive block affordances are outside this PR. Those features must extend the block-render boundary established here rather than alter the transcript container or introduce a TUI-specific blocklist.
The generalized, content-agnostic TUI viewport this transcript renders into (the virtualized list, scroll/anchor model, height reconciliation, and wheel/event plumbing) is a dependency provided by the downstack branch and specified in [`specs/tui-viewport/TECH.md`](../tui-viewport/TECH.md). This spec covers only the terminal-backed transcript built on top of it.

The current TUI prompt path is owned by `crates/warp_tui/src/terminal_session_view.rs`. `TuiTerminalSessionView` creates the production AI context/input/action/controller models for its terminal surface, owns a `TuiConversationSelection`, and sends submitted prompts through `BlocklistAIController::send_user_query_in_conversation`. The earlier one-shot stdout prompt-streaming path and separate `TuiConversationModel` are removed in this PR.

WarpUI already has a TUI-specific element/view/presenter stack. [`TuiElement`](https://github.com/warpdotdev/warp/blob/e36e8ddf823d6a25a5225251a7db60698f5da74d/crates/warpui_core/src/elements/tui/mod.rs#L96-L140) defines the normal layout, rendering, presentation, event, and cursor lifecycle, while [`TuiPresenter`](https://github.com/warpdotdev/warp/blob/e36e8ddf823d6a25a5225251a7db60698f5da74d/crates/warpui_core/src/presenter/tui.rs#L81-L208) retains laid-out trees and records child-view embeddings. The transcript must return normal visible `TuiElement` trees so this lifecycle remains intact; it must not use a context-free raw-buffer row renderer.

`TerminalModel::BlockList` is the canonical ordered presentation model for a terminal surface. Its heterogeneous [`BlockHeightItem`](https://github.com/warpdotdev/warp/blob/e36e8ddf823d6a25a5225251a7db60698f5da74d/app/src/terminal/model/blocks.rs#L121-L196) sum tree orders terminal blocks and rich content and tracks accumulated height, count, and block count in [`BlockList`](https://github.com/warpdotdev/warp/blob/e36e8ddf823d6a25a5225251a7db60698f5da74d/app/src/terminal/model/blocks.rs#L225-L270). Terminal output updates model-authoritative block heights, while view-measured rich content uses dirty marking and height writeback ([`mark_rich_content_dirty`](https://github.com/warpdotdev/warp/blob/e36e8ddf823d6a25a5225251a7db60698f5da74d/app/src/terminal/model/blocks.rs#L1173-L1180), [`update_rich_content_heights`](https://github.com/warpdotdev/warp/blob/e36e8ddf823d6a25a5225251a7db60698f5da74d/app/src/terminal/model/blocks.rs#L2349-L2351)). The GUI follows the same canonical-order model by inserting one rich-content AI block per exchange in [`TerminalView::handle_ai_history_model_event`](https://github.com/warpdotdev/warp/blob/e36e8ddf823d6a25a5225251a7db60698f5da74d/app/src/terminal/view.rs#L6030-L6220).

The TUI transcript will use this existing order. It will not own a second transcript order or introduce a `TUIBlocklistElement`.
## Proposed changes
### TUI transcript composition root
Change the no-prompt TUI frontend callback in `crates/warp_tui/src/lib.rs`: after app-side authentication, bare `warp-tui` starts a real TUI session instead of printing the authenticated user ID and exiting.

Add a root TUI view that owns login branching only. When logged in, it renders a `TuiTerminalSessionView` child. The authenticated session view renders a transcript above a bordered bottom input:
```rust
TuiColumn::new()
    .flex_child(TuiChildView::new(&transcript_view))
    .child(bordered_input)
```
The transcript fills remaining rows above the input. Short transcript content is bottom-aligned so it grows upward from the input; once content reaches the top of the transcript region, the existing viewport scrolling behavior takes over. The bordered input uses the real layout width minus the session padding, not a fixed input width. The input border uses the TUI accent color.

`TuiTerminalSessionView` is the `TerminalSurface` driven by the normal local terminal manager, so its transcript reads the same `TerminalModel` that receives shell output. `RootTuiView` remains only the login-gated app shell. `crates/warp_tui/src/session.rs` keeps the terminal manager and the `spawn_tui_driver` handle alive in a TUI-session singleton.

This PR also relies on small TUI-core support changes: `TuiViewportedList` supports `TuiViewportVerticalAlignment::GrowFromBottom` for short transcript content, `TuiEventContext::set_origin_view` is public for TUI event tests, `AppContext::subscribe_to_view` can subscribe to any `Entity`-backed `ViewHandle` rather than only GUI `View`s, and `TuiContainer` supports uniform, per-axis, and per-side padding so callers can express one-sided spacing without local spacer elements.

### App integration surface
The `warp_tui` crate accesses app-owned terminal and AI types through `app/src/tui_export.rs`. This PR expands that export boundary for the transcript, including:
- agent exchange/input/output types, `AIBlockModel`, and `AIBlockModelImpl`
- terminal block/grid/list/rich-content types and terminal colors
- `CommandExecutionSource`, `ExecuteCommandEvent`, and `should_show_task_in_blocklist`

The app-side trait bounds are relaxed where TUI surfaces now share production models:
- `AIBlockModelImpl<V>` is bounded by `Entity` rather than GUI `View`, allowing `TuiAgentBlockView` to reuse the production exchange model.
- `TerminalSurface` is bounded by `Entity` rather than GUI `View`, with default no-op callbacks for optional GUI-oriented hooks.
- selected terminal model test helpers are exposed behind `test-util` for `warp_tui` tests.

### Interactive input hookup
`TuiTerminalSessionView` embeds the editor-backed [`TuiInputView`](../../crates/warp_tui/src/input/view.rs) (a `warp::editor::CodeEditorModel` in char-cell mode) as the fixed bottom child. It subscribes to `TuiInputViewEvent::Submitted`; on submit it trims the text, ignores empty prompts, creates a selected `AgentViewEntryOrigin::Cli` conversation through `TuiConversationSelection` if needed, and sends through `BlocklistAIController`. `TuiInputView::submit` already clears the editor buffer, so the input resets after each send.
The input box is rendered with a styled `TuiContainer` border. The input view reports cursor coordinates through the normal `TuiElement::cursor_position` path.
The input view drives agent prompts only. Running shell commands from the TUI is future work, so the terminal session view emits no PTY intents; terminal-block rendering is exercised by tests that drive `TerminalModel` directly rather than by interactive input.

### TUI block-list viewport source
Add a `TuiBlockListViewportSource` adapter under `crates/warp_tui/src/` over the canonical `TerminalModel::BlockList` sum tree.

The adapter maps canonical entries to owned TUI transcript descriptors:
```rust
enum TuiBlockListViewportItemId {
    TerminalBlock(BlockId),
    AgentBlock(EntityId),
}
enum TuiBlockListVisibleItem {
    TerminalBlock { block_id: BlockId },
    AgentBlock {
        registration: AgentBlockRegistration,
    },
}
```

The adapter uses the `BlockList` sum tree as the source of truth and currently walks canonical entries from the start of the block list when collecting visible descriptors. It skips unsupported blocklist item kinds in this PR rather than rendering placeholders for them. A future optimization can seek closer to the requested scroll window if large transcripts make this traversal measurable.

`TuiBlockListViewportSource` consumes `BlockList`'s existing dirty rich-content queue, measures dirty registered TUI agent blocks at the current viewport width, and writes the resulting heights back through `BlockList::update_rich_content_heights`. It then collects owned descriptors while holding the terminal-model lock, releases that lock, and only then permits the viewport to invoke the item-render function.

`BlockList::take_dirty_rich_content_items` is public so the TUI viewport source can consume pending rich-content height invalidations. The `warp_tui` crate accesses that helper and other app-owned model types only through the narrow `warp::tui_export` boundary.

### Transcript view and exchange lifecycle
Add a TUI transcript view under `crates/warp_tui/src/` that owns the generalized viewport state and the terminal-history integration. The terminal session view embeds it as the flex child above the bottom input in this PR. It subscribes to terminal-surface-scoped `BlocklistAIHistoryEvent`s and mirrors the existing GUI model-level lifecycle:
- `AppendedExchange` creates a simple TUI agent block view and inserts one `RichContentItem` into the canonical `BlockList`.
- `UpdatedStreamingExchange` marks the corresponding canonical rich-content item dirty and notifies the transcript.
- `ReassignedExchange` updates the block's conversation association.
- removal, deletion, clear, and transfer events remove the affected TUI agent rich-content entries.
TUI agent rich-content entries intentionally leave `agent_view_conversation_id` unset. That field encodes GUI Agent View filtering; setting it while the TUI block list remains in `AgentViewState::Inactive` causes the shared `BlockList` height-update path to hide the entry. The TUI transcript keeps its conversation/exchange association in its own registration map while retaining canonical outer ordering in `BlockList`.

The transcript wraps `TuiViewportedList` in `TuiScrollable` for mouse-wheel handling, renders `TuiBlockListViewportSource` through that viewport, and stores viewport position in its view-owned handle:
```rust
let source = TuiBlockListViewportSource::new(
    self.model.clone(),
    self.agent_blocks.clone(),
);
TuiScrollable::new(
    TuiViewportedList::new(self.viewport.clone(), source)
        .with_vertical_alignment(TuiViewportVerticalAlignment::GrowFromBottom),
)
```

### Simple terminal block
Add a simple terminal-block renderer under `crates/warp_tui/src/`. It renders only the requested visible rows from the block's prompt/command grid followed by its output grid. The renderer reads/copies the required grid data under a short terminal-model lock and performs TUI element rendering after the lock is released.

The renderer preserves terminal cell glyphs and styles and supports incremental output because terminal block heights and grid contents are already updated by `TerminalModel`.

### Simple agent block
Add a simple TUI agent block view keyed by `(AIConversationId, AIAgentExchangeId)`. It reads the current exchange from `BlocklistAIHistoryModel` and extracts logical `TuiAgentBlockSection`s:
- the exchange's displayable user input
- concatenated streamed `AIAgentTextSection::PlainText` output

The view builds a normal TUI element tree from those sections. User input is rendered with `TuiContainer` and `TuiText` using a full-width highlighted background and the `≫ ` prompt prefix. Plain-text output is rendered with `TuiText`. Input/output separation and bottom padding are expressed with `TuiContainer` padding in the composed element tree, not with a separate manual height formula.

The rich-content height adapter measures the same composed element tree at the actual viewport width and writes that height back to `BlockList`. The viewport renders the agent block as a normal child view and clips it through the viewport item boundary. It intentionally omits all non-plain-text agent output rather than inventing placeholder production behavior in this PR.
## End-to-end flow
```mermaid
flowchart TD
  Init["bare warp-tui"] --> Root["RootTuiView<br/>login shell"]
  Root --> Driver["Invalidation-driven<br/>TUI driver"]
  Root --> Session["TuiTerminalSessionView<br/>TerminalSurface"]
  Session --> Input["TuiInputView<br/>bottom bordered"]
  Input -->|submit prompt| Selection["TuiConversationSelection"]
  Selection --> Controller["BlocklistAIController"]
  Controller --> History
  History["BlocklistAIHistoryEvent"] --> Transcript["TUI transcript view"]
  Session --> Transcript
  TerminalModel --> BlockList["TerminalModel::BlockList<br/>canonical SumTree"]
  Transcript -->|append/update/remove agent rich content| BlockList
  BlockList --> Index["TuiBlockListViewportSource<br/>canonical adapter"]
  Index -->|owned visible descriptors| Viewport["TuiViewportedList"]
  Viewport --> Render["Injected item renderer"]
  Render --> Terminal["Simple terminal block<br/>visible rows"]
  Render --> Agent["Simple agent block<br/>visible rows"]
  Terminal --> Presenter["Normal TuiElement lifecycle"]
  Agent --> Presenter
  Presenter --> Frame["TUI frame"]
  Driver --> Presenter
```
## Testing and validation
### Generalized viewport tests
The generalized viewport element, scroll/anchor model, height reconciliation, and wheel/event conversion are tested in the downstack branch; see [`specs/tui-viewport/TECH.md`](../tui-viewport/TECH.md).

### Block renderer tests
Current focused `warp_tui` crate unit tests cover:
- agent block rendering of user input and streamed plain-text output
- agent block width-dependent height measurement through composed TUI layout
- omission of unsupported agent sections until the TUI renders them intentionally
- terminal grid snapshot preservation of theme colors, RGB colors, and common style flags
- terminal block row slicing through the block-list viewport source
- directional and axis-specific `TuiContainer` padding

### Transcript integration tests
Current `warpui::App::test` and `TuiPresenter` coverage verifies:
- terminal and simple agent blocks appear in canonical `BlockList` order
- terminal block visible-row slicing
- TUI agent rich content stays visible without GUI Agent View state
- transcript rendering from canonical terminal blocks
- agent rich-content dirty marking and removal from canonical `BlockList`
- mouse-wheel scrolling while key events remain unhandled by the transcript viewport
- `TuiViewportedList` grow-from-bottom alignment for short content

Additional validation that still needs attention before treating the TUI transcript as production-complete:
- `AppendedExchange`, reassignment, clear, deletion, and transfer events through real `BlocklistAIHistoryModel` fixtures
- submitted input prompt produces a streamed agent block in canonical `BlockList` order
- resize reflows agent text, updates rich-content height, and stabilizes the current frame
- follow-bottom remains pinned while streaming and anchored scrolling remains stable away from the bottom
- terminal session/root composition tests for login-gated child-view ownership

### Manual validation
- Run bare `cargo run -p warp_tui`; verify it enters the alternate screen and displays a bordered input docked at the bottom, with the transcript above it.
- Type a prompt and press Enter; verify the input clears and an agent block with streamed plain-text output appears.
- Create enough blocks to overflow the screen, then use the mouse wheel; verify the transcript preserves its anchor away from the bottom and resumes following after scrolling back to the end.
- Resize the terminal; verify the transcript reflows and preserves/follows its anchor as appropriate.
- Exit with Ctrl-C; verify the alternate screen and terminal mode restore cleanly.

Run:
- `./script/format`
- `cargo nextest run -p warpui_core -E 'test(tui)'`
- focused `cargo nextest run -p warp -E 'test(tui)'`
- `cargo check -p warp -p warp_tui`
- `cargo check -p warp --tests`
- `cargo clippy -p warp -p warp_tui --all-targets -- -D warnings`
## Parallelization
Parallel implementation agents are not proposed. The generalized viewport API, TUI block-list viewport source, block renderers, and transcript lifecycle are tightly coupled through evolving associated types and height/locking contracts; parallel branches would spend significant time restacking and reconciling the same interfaces. Implement sequentially on `harry/tui-transcript-view`, then run focused validation in parallel where the test runner permits it.
## Risks and mitigations
- **A second transcript order diverges from the terminal model.** Use `TerminalModel::BlockList` as the only canonical order; `TuiBlockListViewportSource` is an adapter, not storage.
- **Viewport abstraction leaks terminal or agent types.** Keep descriptors opaque to `TuiViewportedList`; all type-specific rendering stays in the injected app-layer function.
- **Terminal-model deadlock or UI stall.** End scoped index traversal before item rendering; snapshot only required terminal grid rows; batch height writeback under one short lock.
- **Hidden O(N) traversal defeats virtualization.** The current adapter walks from the start of the canonical block list to collect visible descriptors. Keep this visible as an explicit performance review point and add scroll-window seeking if large transcripts make it measurable.
- **Streaming height changes cause visual jumps.** Preserve stable anchors, batch height feedback, and stabilize visible layout in the current pass.
- **TUI and GUI behavior regress together.** Keep the new viewport TUI-specific; reuse backend-neutral pure algorithms only when their contracts truly match.
- **Simple test blocks become accidental production taxonomy.** Keep their scope explicit and verify the block-render seam rather than expanding content behavior in this PR.
- **TUI launch leaves the host terminal in raw/alternate-screen mode.** Tie terminal restoration to the owned driver handle and cover teardown in runtime tests.
## Outside this PR
- final production agent/terminal block styling and content taxonomy
- rich or interactive block affordances
- production-grade input affordances beyond submitting a prompt (history, completions, richer multi-line UX)
