# Tech Spec: Render command block output as Markdown
Issue: https://github.com/warpdotdev/warp/issues/12691
Product spec: `specs/GH12691/product.md`
Code references inspected at commit: [`a66337f4faede85e681e856ac734333a3ee62d3f`](https://github.com/warpdotdev/warp/tree/a66337f4faede85e681e856ac734333a3ee62d3f)
## Context
The feature should be implemented as an alternate presentation for a normal terminal command block, not as a new rich-content block inserted after a command. Normal command blocks already own prompt/command/output grids, context-menu behavior, copy/share semantics, filters, find, persistence, and block-height accounting. Rich content already supports variable-height child views, but using a separate rich-content item for command output would split selection, context menus, block chrome, and copy/share behavior away from the command block that owns the output.
Relevant current code:
- [`app/src/terminal/model/block.rs:295-329 @ a66337f`](https://github.com/warpdotdev/warp/blob/a66337f4faede85e681e856ac734333a3ee62d3f/app/src/terminal/model/block.rs#L295-L329) defines `Block` with `output_grid`, visibility flags, and block metadata.
- [`app/src/terminal/model/block.rs:1430-1492 @ a66337f`](https://github.com/warpdotdev/warp/blob/a66337f4faede85e681e856ac734333a3ee62d3f/app/src/terminal/model/block.rs#L1430-L1492) computes block height from prompt/command height plus `output_grid_displayed_height`, footer, and padding.
- [`app/src/terminal/model/block.rs:1780-1999 @ a66337f`](https://github.com/warpdotdev/warp/blob/a66337f4faede85e681e856ac734333a3ee62d3f/app/src/terminal/model/block.rs#L1780-L1999) exposes output grid accessors, find helpers, output offsets, and raw output height.
- [`app/src/terminal/model/block.rs:2180-2226 @ a66337f`](https://github.com/warpdotdev/warp/blob/a66337f4faede85e681e856ac734333a3ee62d3f/app/src/terminal/model/block.rs#L2180-L2226) exposes `command_to_string`, `output_to_string`, and related raw-output extraction helpers.
- [`app/src/terminal/block_list_element.rs:2340-2724 @ a66337f`](https://github.com/warpdotdev/warp/blob/a66337f4faede85e681e856ac734333a3ee62d3f/app/src/terminal/block_list_element.rs#L2340-L2724) paints prompt/command and output grids inside a single block and advances `grid_origin` by the output grid's displayed height.
- [`app/src/terminal/block_list_element.rs:2978-3176 @ a66337f`](https://github.com/warpdotdev/warp/blob/a66337f4faede85e681e856ac734333a3ee62d3f/app/src/terminal/block_list_element.rs#L2978-L3176) shows the existing variable-height rich-content measurement pattern: dirty rich-content items are laid out, measured, and fed back into block-list height accounting.
- [`app/src/terminal/model/blocks.rs:2163-2309 @ a66337f`](https://github.com/warpdotdev/warp/blob/a66337f4faede85e681e856ac734333a3ee62d3f/app/src/terminal/model/blocks.rs#L2163-L2309) rebuilds the `BlockHeightItem` `SumTree`, optionally applying measured rich-content heights.
- [`app/src/terminal/view.rs:16434-16847 @ a66337f`](https://github.com/warpdotdev/warp/blob/a66337f4faede85e681e856ac734333a3ee62d3f/app/src/terminal/view.rs#L16434-L16847) builds the single-block context menu, including `Copy output`, `Find within block`, filters, bookmarks, and scroll actions.
- [`app/src/terminal/view.rs:24489-24530 @ a66337f`](https://github.com/warpdotdev/warp/blob/a66337f4faede85e681e856ac734333a3ee62d3f/app/src/terminal/view.rs#L24489-L24530) dispatches `ContextMenuAction` variants for selected command blocks.
- [`app/src/terminal/block_list_element.rs:1284-1482 @ a66337f`](https://github.com/warpdotdev/warp/blob/a66337f4faede85e681e856ac734333a3ee62d3f/app/src/terminal/block_list_element.rs#L1284-L1482) maps right-clicks to command-block, rich-content, text-selection, or outside-block context-menu sources.
- [`app/src/terminal/model/block/serialized_block.rs:145-199 @ a66337f`](https://github.com/warpdotdev/warp/blob/a66337f4faede85e681e856ac734333a3ee62d3f/app/src/terminal/model/block/serialized_block.rs#L145-L199) defines `SerializedBlock`, which currently stores stylized command/output bytes and metadata but no output display-mode field.
- [`app/src/persistence/block_list.rs:233-352 @ a66337f`](https://github.com/warpdotdev/warp/blob/a66337f4faede85e681e856ac734333a3ee62d3f/app/src/persistence/block_list.rs#L233-L352) persists command blocks into SQLite columns, including `stylized_output`, but no display-mode column.
- [`app/src/terminal/model/blocks.rs:3034-3119 @ a66337f`](https://github.com/warpdotdev/warp/blob/a66337f4faede85e681e856ac734333a3ee62d3f/app/src/terminal/model/blocks.rs#L3034-L3119) restores serialized command blocks by replaying stylized command/output bytes through the terminal processor.
- [`app/src/ai/agent/util.rs:35-173 @ a66337f`](https://github.com/warpdotdev/warp/blob/a66337f4faede85e681e856ac734333a3ee62d3f/app/src/ai/agent/util.rs#L35-L173) parses Markdown into AI text/code/table sections, including GFM table extraction and code-fence splitting.
- [`app/src/ai/agent/mod.rs:1456-1655 @ a66337f`](https://github.com/warpdotdev/warp/blob/a66337f4faede85e681e856ac734333a3ee62d3f/app/src/ai/agent/mod.rs#L1456-L1655) defines `AgentOutputText`, `AIAgentTextSection`, and table/image/Mermaid section data used by blocklist Markdown rendering.
- [`app/src/ai/blocklist/block/view_impl/common.rs:1091-1290 @ a66337f`](https://github.com/warpdotdev/warp/blob/a66337f4faede85e681e856ac734333a3ee62d3f/app/src/ai/blocklist/block/view_impl/common.rs#L1091-L1290) renders AI output sections into rich text, code, tables, images, and Mermaid diagrams.
- [`app/src/ai/blocklist/block/view_impl/common.rs:1524-1589 @ a66337f`](https://github.com/warpdotdev/warp/blob/a66337f4faede85e681e856ac734333a3ee62d3f/app/src/ai/blocklist/block/view_impl/common.rs#L1524-L1589) renders parsed Markdown text via `FormattedTextElement`, with theme-aware inline code, selection color, link handling, secret redaction, and find highlighting.
- [`crates/markdown_parser/src/lib.rs:108-181 @ a66337f`](https://github.com/warpdotdev/warp/blob/a66337f4faede85e681e856ac734333a3ee62d3f/crates/markdown_parser/src/lib.rs#L108-L181) defines `FormattedText` / `FormattedTextLine` and raw-text extraction for parsed Markdown.
- [`crates/warp_features/src/lib.rs:553-568 @ a66337f`](https://github.com/warpdotdev/warp/blob/a66337f4faede85e681e856ac734333a3ee62d3f/crates/warp_features/src/lib.rs#L553-L568) already has Markdown table, image, Mermaid, and blocklist Markdown feature flags.
- [`app/src/view_components/markdown_toggle_view.rs:12-126 @ a66337f`](https://github.com/warpdotdev/warp/blob/a66337f4faede85e681e856ac734333a3ee62d3f/app/src/view_components/markdown_toggle_view.rs#L12-L126) provides a rendered/raw segmented control for notebook panes, but the product request is a per-block context-menu toggle rather than an always-visible control.
## Proposed changes
### 1. Add per-block output display state
Add a small model enum near `Block`:
- `BlockOutputDisplayMode::TerminalGrid`
- `BlockOutputDisplayMode::Markdown`
Store it on `Block`, defaulting to `TerminalGrid`. Add accessors and setters:
- `output_display_mode()`
- `is_output_markdown_rendered()`
- `set_output_display_mode(mode)`
- `toggle_output_display_mode()`
Keep the state in memory for the initial implementation. Do not add a SQLite migration unless the product open question about persistence is resolved in favor of restart durability. Restored and older blocks should therefore default to `TerminalGrid`, satisfying product behavior 7. If restart durability is later required, extend `SerializedBlock` with a serde-defaulted field and add a nullable SQLite column or metadata JSON path in the same implementation PR.
### 2. Add context-menu action and labels
Extend `ContextMenuAction` with a single-block action such as `SetBlockOutputDisplayMode { block_index, mode }` or `ToggleBlockOutputMarkdown { block_index }`. Add the menu item in `TerminalView::context_menu_items` only when:
- the menu resolves to exactly one selected command block,
- the source is a block or overflow menu rather than selected-text-only,
- the block has non-empty output after filtering rules are applied, and
- the block is not a rich-content item.
Use label `Render output as Markdown` when the block is raw and `Show raw output` when the block is rendered. Place the item near `Copy output`, because it is an output-specific action. `TerminalView::context_menu_action` should mutate the block through `TerminalModel` / `BlockList`, update the block height, clear incompatible grid text selection if needed, close the context menu, and request a redraw.
### 3. Build a reusable blocklist Markdown output renderer
Do not make command blocks depend directly on the AI block model. Instead, extract or wrap the reusable Markdown pieces currently embedded in AI rendering:
- Move `parse_markdown_into_text_and_code_sections` from `app/src/ai/agent/util.rs` to a `pub(crate)` shared module, or expose it behind a neutral `visual_markdown` helper.
- Extract rendering helpers from `app/src/ai/blocklist/block/view_impl/common.rs` into a neutral module that can render `AIAgentTextSection` or an equivalent shared `MarkdownOutputSection` without requiring `AIBlockModel`, action-model handles, conversation state, or AI-specific controls.
- Keep AI-specific affordances, such as code edit buttons and action statuses, outside the shared renderer. Command output only needs read-only rendering, selection, links, tables, code blocks, horizontal scrolling, theme colors, and optional find/secret-redaction hooks.
The renderer should accept:
- the raw output text,
- the available width,
- the block's current working directory/session context for future relative link/image handling,
- theme/font settings from `Appearance`,
- selection/find context where available,
- an obfuscation mode and secret-redaction metadata, and
- a feature-gated policy for tables/images/Mermaid.
For the first version, treat images and Mermaid diagrams conservatively: reuse existing blocklist rendering only if asset resolution and layout hooks can be shared safely for command blocks; otherwise render those constructs as links/code blocks and keep images/Mermaid as follow-ups.
### 4. Cache parsed Markdown and measured height
Markdown parsing and layout should not run from scratch on every paint. Add command-output Markdown cache state either on `Block` or in a `BlockListElement`-owned cache keyed by `(BlockId, output_revision, width_bucket, theme/font inputs)`:
- raw source text or a cheap fingerprint/length,
- parsed sections,
- last measured height in pixels/lines,
- parse error or fallback state.
For a running command, invalidate the cache when the output grid changes. The current model already updates active/background block heights after PTY chunks; hook Markdown invalidation into the same flow or derive it from output text length/fingerprint during layout. For finished blocks, cache should remain stable until filter, display mode, width, or theme changes.
### 5. Measure Markdown output height inside normal block layout
Update the block height model so `Block::height()` can use the Markdown output height when `output_display_mode == Markdown`:
- Add a field such as `markdown_output_height: Option<Lines>` or `BlockOutputRenderMetrics` to `Block`.
- Add `Block::output_displayed_height()` that returns either `output_grid_displayed_height()` or the measured Markdown height.
- Replace direct height calculations in `Block::height`, `full_content_height_with_display_options` if applicable, and block-section offsets with the new output height helper.
- Add `BlockList::update_markdown_output_height(block_index, height)` or include markdown heights in an `update_blocks_and_sumtree` pass, analogous to the rich-content height feedback in `BlockListElement::layout`.
In `BlockListElement::layout`, for visible Markdown-mode command blocks:
1. Build or fetch the rendered Markdown element for the block's current output text.
2. Lay it out with the same available width as the output area and unconstrained vertical height.
3. Convert measured pixels to `Lines` using `size_info.cell_height_px`.
4. Feed the updated height into the model before visible-item calculation settles.
5. Ensure repeated layouts converge and avoid infinite relayout loops by updating only when the measured line height materially changes.
### 6. Paint Markdown in place of the output grid
In `BlockListElement::draw_block`, keep prompt/command painting unchanged. At the output branch:
- If `TerminalGrid`, execute the existing `block.output_grid().draw(...)` path.
- If `Markdown`, skip `output_grid().draw(...)`, paint the cached/rendered Markdown element at the same output origin, and advance `grid_origin` by the measured Markdown height plus footer/padding.
The Markdown output should live inside the command block's background, selection border, snackbar clipping, failure stripe, bookmark/scroll positions, and surrounding block chrome. This is why an in-place rendering mode is preferred over inserting a separate `RichContentItem`.
### 7. Preserve raw data paths
Do not change:
- `Block::output_to_string`
- `Block::command_and_output_to_string`
- `ContextMenuAction::CopyBlockOutputs`
- block share/session share payloads
- AI context attachment
- `SerializedBlock::from(&Block)` stylized output serialization
- SQLite persistence of `stylized_output`
Any new Markdown renderer must consume a text projection of the output and never replace the `BlockGrid`. Product behavior 17 depends on this boundary.
### 8. Filters, find, links, and secrets
Filters: Markdown mode should render the same output text that `Copy filtered output` and filtered output display use. If the existing API only exposes raw full output text, add a helper that returns the displayed/filtered output text without changing copy semantics.
Find: Keep the existing raw-grid find path for raw mode. For Markdown mode, start with block-level navigation and visible text highlighting where the shared Markdown renderer exposes line/cell text offsets. If exact mapping is not ready, document the limitation in the implementation PR and keep raw mode as the exact grid-highlight fallback.
Links: Use the Markdown renderer's default URL click handlers for rendered Markdown links. Do not merge raw terminal link hit-testing with rendered Markdown link hit-testing; each mode owns its own visible link model.
Secrets: Before rendering Markdown, ensure the source text respects the same obfuscation policy as the visible raw output. Prefer feeding already-obfuscated displayed text into the Markdown renderer, then keep AI-style secret redaction hooks as defense in depth.
### 9. Feature flag and rollout
Add a dedicated feature flag such as `CommandBlockMarkdownOutput` unless product/engineering decides this can ride an existing Markdown rendering flag. The flag should gate:
- context-menu item visibility,
- model actions, and
- Markdown layout/rendering paths.
Existing Markdown table/image/Mermaid feature flags continue to control those individual constructs inside the rendered output.
## End-to-end flow
1. User runs a command; `Block` accumulates output in `output_grid` exactly as today.
2. User opens a single-block context menu; `TerminalView::context_menu_items` sees non-empty output and raw display mode, then adds `Render output as Markdown`.
3. User selects the item; `TerminalView::context_menu_action` sets that block's display mode to `Markdown`, invalidates output layout/cache, updates block heights, and redraws.
4. `BlockListElement::layout` sees the visible Markdown-mode block, parses/caches output text, measures the rendered element, and feeds the measured height back into the block height model.
5. `BlockListElement::paint` draws the same block chrome and prompt/command grid, then paints rendered Markdown at the output origin instead of drawing the output grid.
6. User chooses `Show raw output`; the display mode returns to `TerminalGrid`, the cached Markdown element can be retained or dropped, and the existing output-grid paint/height path takes over.
## Testing and validation
Automated tests should map to `product.md` behaviors:
- Model/unit tests in `app/src/terminal/model/block_tests.rs` or `blocks_tests.rs` for default `TerminalGrid`, toggling one block without affecting siblings, active/running block updates, height helper behavior, and restored-block defaulting. Covers behaviors 1, 5-7, and 9.
- Serialization tests in `app/src/terminal/model/block/serialized_block_tests.rs` proving raw stylized output is unchanged and, if no persistence field is added, display mode is not serialized. Covers behaviors 7 and 17.
- Context-menu tests in `app/src/terminal/view_tests.rs` covering menu label, action dispatch, disabled/omitted states for multi-selection, selected text, empty output, rich content, and block overflow. Covers behaviors 2, 4, 8, and 24.
- Renderer tests for the shared Markdown output module covering headings, lists, code fences, inline code, links, tables under the table flag, malformed Markdown fallback, ANSI-heavy text fallback/no crash, long tables, and secret-obfuscated input. Covers behaviors 10-13, 15, 20, and 22.
- Block-list layout tests covering measured Markdown height, resize reflow, scroll-to-top/bottom, no overlapping blocks, and returning to raw grid height. Covers behaviors 14-16 and 25.
- Filter tests covering Markdown rendering of filtered output and refresh when filters change. Covers behavior 21.
- Find tests covering at least block-level navigation in Markdown mode and exact raw-grid highlighting after switching back to raw. Covers behavior 19.
Focused commands to run during implementation:
```bash
cargo nextest run -p warp --lib terminal::model::block::tests
cargo nextest run -p warp --lib terminal::model::blocks::tests
cargo nextest run -p warp --lib terminal::view_tests
cargo nextest run -p warp --lib ai::agent::util::tests
cargo nextest run -p markdown_parser
./script/format --check
cargo clippy --workspace --all-targets --all-features --tests -- -D warnings
```
Manual validation:
1. Run a command that prints Markdown with headings, bullets, fenced code, inline code, a link, and a GFM table. Toggle Markdown on/off from right-click and overflow menus.
2. Repeat with ANSI-colored `ls`, `grep`, or test output and verify raw mode remains the escape hatch.
3. Run a streaming command that prints an opening code fence, delays, then closes it; verify no crash or overlapping layout while streaming.
4. Resize the pane narrower/wider while Markdown mode is active; verify block height and table/code overflow behavior.
5. Apply and clear a block filter while Markdown mode is active.
6. Copy output, copy the whole block, share block/session if available, and attach as AI context; verify raw output is preserved.
7. Test light/dark themes and secret obfuscation enabled.
## Parallelization
Parallel sub-agents can help after the specs are approved because the implementation has separable model/action, rendering, and validation work. Use local worktrees from `master` to avoid conflicts, then merge into one implementation branch.
- `model-actions` owns `Block` display state, `TerminalAction`/`ContextMenuAction`, context-menu labels, and raw data invariants. Use local worktree `../warp-gh12691-model-actions` on branch `oz-agent/gh12691-model-actions`; hand off a patch and focused unit/context-menu test results.
- `markdown-renderer` owns extracting a reusable Markdown output renderer and parser/cache helpers from AI rendering without introducing AI-model dependencies into terminal block rendering. Use local worktree `../warp-gh12691-markdown-renderer` on branch `oz-agent/gh12691-markdown-renderer`; hand off changed renderer files and parser/renderer tests.
- `layout-validation` owns block-list height measurement, resize behavior, filters/find integration tests, and manual validation artifacts. Use local worktree `../warp-gh12691-layout-validation` on branch `oz-agent/gh12691-layout-validation`; hand off tests and any layout fixes.
Ordering:
1. `model-actions` and `markdown-renderer` can start in parallel after the product/tech specs are approved.
2. `layout-validation` should start once the renderer API shape is known, but can prepare tests in parallel.
3. A lead integrator merges all work into one implementation PR, resolves conflicts in `BlockListElement` and terminal view actions, then runs final validation.
## Risks and mitigations
- **Performance regression from parsing large output.** Cache parsed sections and measured layout by block/output revision and width. Consider a size threshold with a user-visible fallback if an output is too large to render interactively.
- **Layout loops from measured Markdown height changing during layout.** Update stored height only when it changes beyond a small epsilon and ensure the second layout pass converges.
- **Secret leakage through Markdown rendering.** Feed obfuscated displayed output to the renderer and reuse secret-redaction hooks for rendered text/table cells.
- **Over-coupling terminal blocks to AI block rendering.** Extract neutral renderer utilities instead of importing AI block model/state into terminal rendering.
- **Raw terminal fidelity confusion.** Keep raw mode default and make `Show raw output` always available for rendered blocks.
- **Persistence scope ambiguity.** Initial implementation should default restored blocks to raw unless schema persistence is explicitly added. If persistence is added, it must be serde-defaulted and backward compatible.
## Follow-ups / open technical questions
- Decide whether restart-durable display mode is worth a SQLite/serialization migration.
- Decide whether command-output Markdown should render local/remote images and Mermaid diagrams in v1 or defer them until asset resolution and security review are complete.
- Consider adding a keyboard command for the selected block after the context-menu UX proves useful.
