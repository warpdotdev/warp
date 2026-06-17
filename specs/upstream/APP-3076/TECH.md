# Block List Markdown Table Rendering — Tech Spec
Product spec: `specs/APP-3076/PRODUCT.md`

## Problem
The AI block list already detects GFM-style tables, but it degrades them into a single preformatted text blob. That loses the structured table model we already have elsewhere in the repo, prevents reuse of notebook-style inline cell formatting, and makes copy semantics awkward because the same string is currently used for rendering and clipboard export.

The implementation needs to satisfy four constraints at once:
- reuse shared Markdown table parsing rather than maintaining a second table parser
- render with a read-only UI component appropriate for the block list rather than the editor-specific notebook renderer
- preserve source Markdown for block-level copy actions
- support block-list-native interaction rules: horizontal-only local scrolling, no nested vertical scrolling, and correct text selection

## Relevant Code
- `specs/APP-3076/PRODUCT.md` — approved product behavior
- `app/Cargo.toml` — compile-time feature declarations for feature-flagged app functionality
- `app/src/lib.rs (2363-2561)` — compile-time-to-runtime `FeatureFlag` wiring for app features
- `app/src/ai/agent/util.rs:24-145` — `parse_markdown_into_text_and_code_sections`; current AI-output section splitter
- `app/src/ai/agent/mod.rs (1281-1300)` — `AIAgentTextSection`; current table payload is `content: String`
- `app/src/ai/agent/mod.rs (1547-1588)` — `Display for AIAgentOutputMessage`; current copy/export path prints table `content`
- `app/src/ai/blocklist/block.rs (4941-5002)` — AI block copy helpers
- `app/src/ai/blocklist/block.rs (5923-5937)` — `Copy`, `CopyOutput`, and related clipboard actions
- `app/src/ai/blocklist/block/view_impl/common.rs (1139-1188)` — `render_table_section`; current table renderer is a `Text` element inside a horizontal scroller
- `app/src/ai/blocklist/block/find.rs:70-90` — find currently searches table `content` directly
- `markdown_parser/src/markdown_parser.rs:106-189` — `parse_markdown` and `parse_markdown_with_gfm_tables`
- `markdown_parser/src/markdown_parser.rs (333-458)` — structured GFM table parsing (`parse_table`, separator parsing, inline cell parsing)
- `markdown_parser/src/lib.rs (338-438)` — `FormattedTable` and `TableAlignment`
- `editor/src/content/text.rs (61-109)` — `parse_table_cell_markdown_inline`
- `editor/src/content/text.rs:261-279` — `table_from_internal_format_with_inline_markdown`; current notebook-side table reconstruction helper
- `editor/src/render/model/mod.rs:454-476` — notebook `TableStyle`
- `editor/src/render/model/mod.rs (1140-1234)` — `LaidOutTable`; notebook table layout and selection model
- `editor/src/render/element/table.rs:1-220` — notebook table painting path
- `ui/src/elements/table/mod.rs (117-240)` — shared `Table`, `TableConfig`, and column sizing API
- `ui/src/elements/table/mod.rs (517-980)` — `Table` layout behavior, including intrinsic measurement and viewport sizing
- `ui/src/elements/table/mod.rs (1002-1478)` — selection and scroll behavior for the shared table element
- `warp_core/src/features.rs` — `FeatureFlag` definitions and dogfood/default channel enablement

## Current State

### AI block list path
The block list does not use the shared Markdown table model. `parse_markdown_into_text_and_code_sections` in `app/src/ai/agent/util.rs:24-145` uses a custom helper from `ai::gfm_table` to detect a table-shaped region while scanning the response line-by-line. Once it finds one, it emits `AIAgentTextSection::Table { content: String }`, where `content` is a normalized pipe-delimited string rather than a structured table object.

Rendering then happens in `render_table_section` in `app/src/ai/blocklist/block/view_impl/common.rs (1139-1188)`, which places that string into a single `Text` node wrapped in a horizontal scroller. This satisfies the current “readable monospace dump” behavior, but it does not preserve:
- structured column alignment
- notebook-style inline formatting inside cells
- a clean separation between source Markdown and rendered representation

Because `AIAgentTextSection::Table` only stores the rendered string, copy/export and find also operate on that same string.

### Notebook path
The notebook/editor stack already has a real structured table model:
- `markdown_parser` can parse GFM tables into `FormattedTable`
- `editor/src/content/text.rs` has helpers for reconstructing a `FormattedTable` from the editor’s internal representation while preserving inline Markdown styles
- `editor/src/render/model/mod.rs` and `editor/src/render/element/table.rs` lay out and paint tables with alignment, padding, selection mapping, and per-cell text frames

That renderer is not a good direct fit for the block list because it is tied to the editor buffer/render model, offset maps, and editor-specific interaction flow.

### Shared UI table component
The shared `ui::elements::Table` is much closer to what the block list needs: it supports arbitrary cell elements, per-column sizing, selection delegation, and independent composition with surrounding UI.

However, its current defaults do not match the product requirements for the block list:
- it assumes an internal vertical viewport and implements vertical scrolling itself
- selection over virtualized rows is intentionally incomplete when rows are off-screen
- intrinsic width measurement only looks at headers, not row content

Those defaults are reasonable for a general-purpose scrollable table, but not for a block-list table that must stay vertically integrated with the surrounding response.

## Proposed Changes

### 0. Gate the feature behind `BlocklistMarkdownTableRendering`
Add a dedicated feature flag named `BlocklistMarkdownTableRendering` and wire it through the normal Warp feature-flag plumbing:
- add `blocklist_markdown_table_rendering` to `app/Cargo.toml`
- map that Cargo feature to `FeatureFlag::BlocklistMarkdownTableRendering` in `app/src/lib.rs`
- add the new enum variant in `warp_core/src/features.rs`
- enable it by default for dogfood builds via `DOGFOOD_FLAGS`

The new structured table rendering should only activate when this flag is enabled. When disabled, AI block list responses should continue to detect tables and render them with the pre-feature monospace scrollable table block.

### 1. Replace string-backed table sections with a structured payload
Introduce a dedicated AI-output table type in `app/src/ai/agent/mod.rs`, for example:

```rust
pub struct AgentOutputTable {
    pub markdown_source: String,
    pub table: FormattedTable,
}
```

Then change `AIAgentTextSection::Table` from:

```rust
Table { content: String }
```

to:

```rust
Table { table: AgentOutputTable }
```

This gives the block list two representations of the same table:
- `markdown_source` for response-level copy/export
- `FormattedTable` for rendering and selection-aware display

This is the key ownership boundary for the feature. We should not try to derive clipboard Markdown back from the rendered UI.

### 2. Reuse `markdown_parser` for table parsing
Stop using the custom `ai::gfm_table::maybe_parse_gfm_table` path as the source of truth for parsed table structure.

Instead, add a small shared helper in `markdown_parser` that parses a contiguous GFM table block into `FormattedTable`. The block list section splitter in `app/src/ai/agent/util.rs` should continue to own boundary detection between:
- plain text
- fenced code blocks with metadata
- tables

but once it has collected a candidate table block, it should hand the raw Markdown to `markdown_parser`, not re-parse inline cell content itself.

Concretely:
- keep the existing line-oriented section splitter in `app/src/ai/agent/util.rs` so code-block metadata parsing remains unchanged
- replace the current custom table-formatting helper with a new shared parser entry point from `markdown_parser`
- store the exact raw table Markdown in `markdown_source`, preserving spacing and source syntax for copy/export

This reuses the repo’s actual GFM table parsing logic, including:
- alignment parsing
- inline Markdown in cells
- links, inline code, bold, italic, and strikethrough handling

### 3. Preserve source Markdown in copy/export flows
Update the copy/export paths that currently rely on `AIAgentOutputMessage` display formatting to use the table’s `markdown_source` rather than a rendered or normalized text serialization.

The affected flows are the existing AI block and conversation export paths in:
- `app/src/ai/agent/mod.rs (1547-1588)`
- `app/src/ai/agent/conversation.rs:1082`
- `app/src/ai/blocklist/block.rs (4941-5002, 5923-5937)`

The rule is:
- block-level copy actions use `markdown_source`
- rendered-text selection continues to come from the UI layer

This keeps block-level copy behavior aligned with the product spec without complicating the table renderer.

### 4. Add a block-list table renderer built on the shared UI `Table`
Add a new renderer for AI-output Markdown tables in the block list, either as a helper in `app/src/ai/blocklist/block/view_impl/common.rs` or as a dedicated view/component in the same module tree.

The renderer should:
- take `&AgentOutputTable`
- build a `ui::elements::Table`
- create one header element per `FormattedTable::headers` entry
- create one row per `FormattedTable::rows` entry
- wrap the table in the existing horizontal `NewScrollable` / `ClippedScrollStateHandle` composition used for block-list tables today

Each cell should be rendered with `FormattedTextElement`, using a one-line `FormattedText` built from the cell’s `FormattedTextInline` fragments. This preserves the same inline Markdown primitives already supported by notebooks without embedding the editor renderer.

This renderer should intentionally stay read-only. No editor state, offset map, or notebook-specific block model should be introduced into the block list.

### 5. Extend the shared UI `Table` for block-list usage
The shared UI table needs one opt-in mode for this feature.

Add an explicit vertical sizing mode to `ui/src/elements/table/mod.rs` rather than another boolean toggle. The shared `TableConfig` should expose an enum that distinguishes the default viewported behavior from a full-content mode, e.g. `TableVerticalSizing::Viewported` vs `TableVerticalSizing::ExpandToContent`.

In `ExpandToContent` mode:
- the table expands to its full content height
- table-local vertical scrolling is disabled
- the parent scroll container owns vertical scrolling

This keeps the API clear about the underlying layout model rather than asking callers to infer semantics from a boolean.
- existing behavior remains the default `Viewported` mode
- the block list opts into `ExpandToContent`
- layout measures all rows, not just visible rows
- the element reports full content height
- `ScrollableElement` does not capture vertical wheel scrolling for the table
- the table no longer behaves like its own vertical viewport

This change is necessary for two reasons:
1. it enforces the product rule that tall tables scroll with the block list, not inside a nested scroller
2. it removes the current virtualization-related selection limitation for off-screen rows

### 6. Make intrinsic widths account for row content in block-list mode
If we render block-list tables with the current `ui::elements::Table` intrinsic sizing behavior, only header content contributes to intrinsic widths. That is likely to produce visibly different results from notebook tables when body cells are wider than their headers.

To keep the block-list result visually close to notebook tables, add an opt-in width measurement path for the shared `Table` so intrinsic column widths can include body cells when desired.

This should be scoped narrowly:
- preserve current default behavior for existing `Table` users
- enable body-cell-aware intrinsic sizing only for block-list Markdown tables in `TableVerticalSizing::ExpandToContent` mode

Because block-list tables will already be in `ExpandToContent` mode, we can avoid a separate measurement-only render pass: render each row once in the full-content layout path, measure unconstrained intrinsic widths from those already-instantiated body cells, then lay those same row elements out with the final computed column widths. This removes the extra `render_fn` pass for intrinsic body-width measurement while keeping the change scoped to the block-list path.

### 7. Separate find/search text from source Markdown
After the table payload becomes structured, the block list should not use `markdown_source` for find matching. That would make the find surface operate on Markdown syntax instead of rendered text.

Add a helper on `AgentOutputTable` that flattens the parsed table into plain find/selection text in row-major order, using tab-separated cells and newline-separated rows. Then update `app/src/ai/blocklist/block/find.rs:70-90` to search that derived plain text instead of the raw Markdown source.

This keeps find behavior aligned with the rendered content while leaving clipboard export source-accurate.

## End-to-End Flow
1. The AI response streams in as Markdown text.
2. `parse_markdown_into_text_and_code_sections` in `app/src/ai/agent/util.rs` continues scanning line-by-line.
3. When it encounters a candidate table region, it collects the raw Markdown block and hands it to a shared `markdown_parser` helper.
4. The parser returns a `FormattedTable`.
5. The block list stores that as `AIAgentTextSection::Table { table: AgentOutputTable { markdown_source, table } }`.
6. The block renderer sees the table section and builds a read-only WarpUI `Table`.
7. The WarpUI table renders inline cell formatting via `FormattedTextElement`.
8. The block list wraps the table in a horizontal scroller only.
9. Vertical scrolling stays with the surrounding block list.
10. Block-level copy actions export `markdown_source`; selection copy comes from the rendered table elements.

## Implementation Plan

### Phase 1: Data model and parsing
- Add `BlocklistMarkdownTableRendering` feature-flag plumbing and gate the block-list table behavior behind it
- Add `AgentOutputTable` and update `AIAgentTextSection`
- Add a shared GFM-table parsing helper in `markdown_parser`
- Update `app/src/ai/agent/util.rs` to emit structured table sections with preserved `markdown_source`
- Remove or stop using the custom `ai::gfm_table` helper

### Phase 2: Copy/find behavior
- Update AI output formatting and block/conversation copy flows to use `markdown_source`
- Add a plain-text flattening helper for find
- Update `app/src/ai/blocklist/block/find.rs` to search rendered table text rather than raw Markdown

### Phase 3: UI table support
- Replace the expand-to-content boolean with an explicit `TableVerticalSizing` enum on `TableConfig`
- Extend `ui::elements::Table` with an `ExpandToContent` mode that disables local vertical scrolling
- Extend intrinsic measurement so body cells can participate when requested, using the single full-content layout pass in `ExpandToContent` mode
- Keep existing behavior as the default for current users of the component

### Phase 4: Block-list rendering
- Replace the current monospace `render_table_section` with a structured renderer built on WarpUI `Table`
- Reuse current horizontal scroll handle plumbing
- Match notebook table styling as closely as practical via block-list table theme helpers

## Risks and Mitigations

### Risk: nested vertical scrolling or incomplete selection
Using the shared `Table` without modification would keep the current vertical viewport and virtualization behavior, which conflicts with the product spec.

Mitigation:
- add an explicit `TableVerticalSizing::ExpandToContent` mode for block-list tables
- disable table-local vertical scrolling in that mode

### Risk: copy/export regressions
Today the table section’s rendered string is also what gets copied. Moving to structured tables could accidentally change clipboard output.

Mitigation:
- make `markdown_source` a first-class field on the table payload
- route copy/export through that field explicitly
- add unit coverage for block-level and conversation-level copy

### Risk: visual mismatch with notebook tables
If block-list tables use header-only intrinsic sizing or different theme tokens, they may look noticeably different from notebook tables.

Mitigation:
- add body-cell-aware intrinsic sizing for block-list mode
- define a small style translation helper that mirrors notebook table border, padding, alternating-row, and header treatments as closely as practical

### Risk: performance on very large tables
Expanding to full height and measuring all rows is more expensive than a virtualized viewport, even after removing the extra intrinsic-width render pass.

Mitigation:
- accept the tradeoff for the first version because AI-response tables are typically modest in size
- keep the expand-to-content mode opt-in and local to this feature
- revisit with profiling only if large-table responses become a real issue

## Testing and Validation

### Parser and data-model tests
- Add `markdown_parser` tests for the new shared table-block parser:
  - simple tables
  - alignment parsing
  - inline formatting in cells
  - links
  - strikethrough
  - escaped pipes
  - invalid/non-table input
- Add `app/src/ai/agent/util_tests.rs` coverage that verifies:
  - table sections preserve exact `markdown_source`
  - code blocks that contain table-looking text are not parsed as tables
  - prose before/after a table still produces the correct section ordering

### Copy and find tests
- Add tests covering `AIAgentOutputMessage` / exchange formatting to verify block-level copy uses original Markdown table syntax
- Add block-list find tests to verify searches match rendered cell text rather than Markdown syntax

### UI table tests
- Add WarpUI table tests for the new expand-to-content mode:
  - no local vertical scroll behavior
  - full content height is returned
  - selection spans all rows because no rows are virtualized away
- Add table sizing tests covering body-cell-aware intrinsic measurement

### Block-list rendering validation
- Manual validation that wide tables get a local horizontal scrollbar
- Manual validation that tall tables scroll with the block list and do not show a nested vertical scrollbar
- Manual validation that selection works within cells, across cells, across rows, and across prose/table boundaries
- Manual validation that Markdown links in cells remain clickable while click-drag still selects text
- Manual validation that rendered output visually matches notebook tables closely for alignment, padding, borders, and row treatment

## Follow-ups
- Generalize the block-list table renderer into a reusable read-only Markdown table view if other surfaces need it
- Consider adding autodetected file-path/URL highlighting inside table cells if we decide block-list tables should match plain-text-section link detection behavior as well
- If future AI outputs include very large tables, revisit whether the shared `Table` should support a hybrid mode that preserves block-list vertical scrolling while still reducing layout cost
