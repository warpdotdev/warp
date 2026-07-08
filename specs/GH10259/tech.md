# GH10259: Support `<details>`/`<summary>` in markdown rendering — Tech Spec

Issue: https://github.com/warpdotdev/warp/issues/10259
Product spec: [product.md](./product.md)

## Context

Markdown rendering is built on a flat, line-oriented document model:

- `crates/markdown_parser/src/lib.rs:156-168` — `FormattedTextLine` is the enum of renderable block variants (`Heading`, `Line`, `OrderedList`, `UnorderedList`, `CodeBlock`, `TaskList`, `LineBreak`, `HorizontalRule`, `Embedded`, `Image`, `Table`). There is no container variant; the document is a flat `VecDeque<FormattedTextLine>` (`FormattedText`, lib.rs:111-114).
- `crates/markdown_parser/src/markdown_parser.rs` — `parse_markdown` drives a nom `alt((...))` chain of block parsers (code block, header, image, task/ordered/unordered list, table, paragraph) inside a `while !remaining.is_empty()` loop (~lines 130-215). Inline HTML handling is deliberately narrow: HTML entities (`parse_html_entity`, ~line 1515) and `<u>`/`</u>` underline delimiters (`parse_inline_token_underline_start`/`_end`, lines 1626-1645). All other tags flow through as plain text.
- `crates/markdown_parser/src/html_parser.rs:23-28` — the imported-HTML path (paste from GDocs/Confluence/etc.) flattens container tags via `TOP_LEVEL_ELEMENT_TAGS_TO_SKIP` and treats a fixed set of `PHRASING_ELEMENT_TAGS` as inline. `<details>`/`<summary>` are in neither list, so their children are visited and the container semantics are lost.
- `crates/editor/src/content/markdown.rs` — maps `FormattedTextLine` variants to editor `BufferBlockStyle`s and back (`to_formatted_text`, lines ~416-520; `to_markdown`, lines ~61-160). The buffer model this spec builds on: a block's style metadata lives in a `BufferText::BlockMarker { marker_type: BufferBlockStyle }` sentinel at the block's start (`crates/editor/src/content/text.rs:541-571`, enum at text.rs:867). List blocks carry their metadata (indent level, ordered-list start number, task-list checkbox state) on that marker; to selection, copy, and every offset dimension the marker reads as a single `"\n"` (text.rs:~626), which is why list metadata never complicates text selection. Table, by contrast, is a multi-line block smuggled through an internal code-block format and needs the `TableCache` per-cell offset maps to make selection work — maintainer guidance on this PR is explicitly that the table mechanism is an intermediate implementation and must not be replicated for details.
- Nesting-depth precedents: `ListIndentLevel` (`crates/warpui_core/src/elements/gui/list.rs:20`) stores per-marker indent; `ListNumbering` (list.rs:96) recomputes visible list numbers with a bounded backward scan and a per-level counter stack; the paired inline-style markers aggregate counter dimensions in the buffer's `SumTree` summaries (`StyleSummary`, text.rs:1551), so counter-style dimensions with O(log n) seeks are an established pattern.
- Folding already exists: `HiddenLinesModel` (`crates/editor/src/content/hidden_lines_model.rs:20`) tracks hidden line ranges as anchor pairs, per-editor and independent of buffer content, rendered as collapsed sections.
- `compute_formatted_text_delta` (lib.rs:66-109) diffs two `FormattedText` values line-by-line for streaming updates; any new variant must implement `LineCount` (lib.rs:284-300) and be comparable for equality.

## Proposed changes

### 1. New `FormattedTextLine::Details` variant (container-as-line in the parser IR)

Add to `crates/markdown_parser/src/lib.rs`:

```rust
pub struct FormattedDetails {
    pub summary: FormattedTextInline,     // parsed inline markdown; "Details" literal if absent
    pub body: FormattedText,              // recursively parsed markdown
    pub default_open: bool,               // `open` attribute present
}
// FormattedTextLine::Details(FormattedDetails)
```

There is no verbatim-source field: serialization is canonical re-serialization from parsed structure (product invariant 10), consistent with the rest of the rich-text pipeline, so `body` is the only body representation. `PartialEq` derives over all fields, so streaming delta comparisons compare parsed structure.

In the parser IR (the read-only rendering model), the whole `<details>…</details>` region is a single `FormattedTextLine`. Start/end marker lines are wrong *at this layer*: `FormattedText` consumers are stateless per-line renderers and `compute_formatted_text_delta` diffs lines independently, so nothing maintains marker balance here. The editor buffer is the opposite case — it has edit machinery that already maintains paired markers — and gets a paired representation instead (§4).

- `LineCount::num_lines` returns `1 + body line count` (content lines, independent of collapsed state — collapsed is view state, spec invariant 9).
- `raw_text()` emits summary then body text.
- `set_weight`/`inline_fragments`/`hyperlinks` recurse into summary and body.

### 2. Parser: new block branch in `parse_markdown`

Add `parse_details` to the `alt((...))` block chain in `crates/markdown_parser/src/markdown_parser.rs`, before `parse_paragraph`:

- Matches only at block start (spec invariant 8c): optional leading spaces, `<details` + optional attributes (`open` recognized, others ignored) + `>`.
- **Fence-aware matching (invariant 8g)**: the closing `</details>` is not found by raw text scan. The body region is delimited line-by-line with the same fence tracking the block parser already performs: the delimiter walks lines, toggling an in-fence flag on code-fence lines (the ` ``` `/`~~~` recognition `parse_code_block` uses), and recognizes `<details`/`</details>` tags only on lines outside a fence and only at line start. Tags on fenced lines are body content and never affect the balance counter. `parse_markdown` is then applied recursively to the delimited region with a `depth` parameter. Extracting the first top-level `<summary>…</summary>` (invariant 4) happens on the same fence-aware walk.
- Depth and count limits (invariants 6-7): `const MAX_DETAILS_DEPTH: usize = 8; const MAX_DETAILS_PER_DOC: usize = 512;` threaded through the existing parse context. On exceeding either, the branch returns `nom::Err::Error` so the input deterministically falls through to `parse_paragraph` (plain text) — no panic path, bounded recursion.
- Unclosed `<details>` (invariant 8a): treat rest of the current parse input as body. Because parsing is re-run on streaming updates, a still-streaming block naturally renders progressively (invariant 12); `compute_formatted_text_delta` sees the growing `Details` line as the changed suffix, so preceding lines keep their prefix identity.

### 3. Imported-HTML path

In `crates/markdown_parser/src/html_parser.rs`, handle `details`/`summary` elements explicitly: build the same `FormattedDetails` from the DOM (html5ever already provides the tree, so no new scanning is needed) with the same depth/count constants applied.

### 4. Editor buffer: details modeled directly in the buffer (list precedent)

Details regions are first-class buffer blocks, following the ordered-list precedent (per maintainer guidance: the `FormattedTable` internal-code-block mechanism is an intermediate implementation and is not replicated here — it makes the block multi-line and forces `TableCache`-style offset mapping onto selection).

- **Summary line**: a details region starts with a one-line block styled `BufferBlockStyle::DetailsSummary { depth: u8, default_open: bool }` (new variants alongside `OrderedList`/`TaskList`, text.rs:867). Exactly like an ordered-list item, the marker (`BufferText::BlockMarker`) carries the block metadata while the line's *text content is the summary itself* — editable, selectable inline text, the analogue of list-item text. The disclosure indicator is a render-time margin decoration like the list number (drawn by the render element, never present in the char stream). The list-number analogy for metadata is `depth` and `default_open`; the checkbox precedent (`TaskList { complete }`) shows a togglable bool on the marker already exists.
- **Region extent**: the body is the run of ordinary blocks following the summary line, terminated by a zero-content end sentinel block `BufferBlockStyle::DetailsEnd { depth: u8 }`. Begin/end markers form pairs the same way paired inline-style markers do (`StyleSummary`'s +1/−1 counters, text.rs:1551): a counter field added to the buffer summary increments on `DetailsSummary` and decrements on `DetailsEnd`, so nesting depth at any offset is an O(log n) `SumTree` summary seek, and the `depth` field on each marker is a cached copy of that structural depth (as `indent_level` is for lists).
- **Selection/copy stay trivial**: both markers read as a single `"\n"` in the byte iterator (text.rs:~626), exactly like every other `BlockMarker`; the summary is ordinary line text and body blocks are ordinary blocks. No offset mapping, no selection special cases — the property the table mechanism loses.
- **Nesting**: a nested details region is simply a nested begin/end pair among the body blocks. `MAX_DETAILS_DEPTH` applies on conversion into the buffer; depth never multiplies block types.
- **Edit invariants (deterministic rebalancing, mirroring product 8a/8b)**: an unmatched `DetailsSummary` (its `DetailsEnd` was deleted) owns blocks to the end of the buffer — the 8(a) rule; an unmatched `DetailsEnd` is inert and skipped on serialization — the 8(b) rule. Both are properties of the pairing scan, not stored state, so no edit sequence can produce an undefined document. `line_break_behavior()` (text.rs:904): Enter on a summary line starts a new plain body block below it (the marker case, like lists); Enter in the body behaves normally. Converting a block to/from details styles goes through the existing `BufferEditAction::StyleBlock` path (`set_block_style`/`convert_block`, model.rs:1142/1175), which already shows how to preserve marker metadata across conversions.
- **Collapse = existing folding**: collapsed state reuses `HiddenLinesModel` (hidden_lines_model.rs:20) — the body range is an anchor-pair hidden region, per-editor and outside buffer content, initialized from `default_open`. This satisfies product invariant 9 by construction: hiding is view state; copy, `raw_text()`, and serialization walk the buffer, which always contains the full body. Toggling does not edit the buffer, so it cannot dirty deltas or undo history.
- **Round-trip** (`crates/editor/src/content/markdown.rs`): `to_markdown` (lines ~61-160) emits `<details>` (` open` iff `default_open`) + `<summary>…</summary>` with the summary line's inline content re-serialized as inline markdown when it reaches a `DetailsSummary` block, serializes body blocks through their normal arms, and emits `</details>` at the paired `DetailsEnd` — canonical re-serialization, no verbatim source (product invariant 10). `to_formatted_text` (lines ~416-520) does the inverse: it consumes the blocks between paired markers into a `FormattedTextLine::Details` container (recursively for nested pairs), and `core.rs` gains parse arms alongside the list arms (core.rs:~742/781) that flatten a `FormattedTextLine::Details` into marker + body blocks + end marker on the way in.
- **Render model**: the render `SumTree` gains `BlockItem::Details { depth, open, .. }` (render/model/mod.rs:927) and a `render/element/details.rs` modeled on `ordered_list.rs` — the disclosure triangle paints in the margin the way the list number does. Focus, click, Enter/Space toggle, and renderer-generated accessibility IDs live here.

### 5. Rendering surfaces (product invariant 11)

The static fallback is not an enumerated per-surface obligation — it is implemented once, at the two sinks all consumers render through, so every current and future consumer gets defined behavior by construction:

1. **`FormattedTextElement`** (`crates/warpui_core/src/elements/gui/formatted_text_element.rs:144`) — the shared GUI element that lays out `FormattedText` for the app's markdown surfaces (modals, banners, changelog, settings pages, conversation list, etc.). It gains layout for the `Details` variant: summary line, then body lines, expanded, no disclosure indicator — the invariant-11 fallback. The interactive widget is an opt-in builder method on this element (consistent with its existing `with_*`/`disable_mouse_interaction` builder API), enabled only by the agent conversation block path (`app/src/ai/agent/util.rs:35`, `parse_markdown_into_text_and_code_sections` via `parse_markdown_with_gfm_tables` at util.rs:189).
2. **The editor buffer conversion** (`crates/editor/src/content/markdown.rs`) — consumers that push `FormattedText` into editor buffers get the §4 buffer-native mapping. Because §4 models details directly in the buffer with `HiddenLinesModel` folding, the notebook/plan editor (`RichTextEditorView`, `app/src/notebooks/editor/view.rs:1029`) is an interactive-tier surface (product invariant 11): its buffers fold and toggle details natively, which is the second surface the product spec commits to.

Additionally, adding a variant to `FormattedTextLine` is a compile-time forcing function: every exhaustive `match` on the enum fails to compile until the new variant is handled, so no consumer can silently mis-render. The implementation audits the non-parser match sites (57 non-test files reference `FormattedTextLine` outside `crates/markdown_parser` at time of writing) and routes each to sink 1 or 2; sites that only pattern-match specific variants (e.g. hyperlink extraction) need no change.

For completeness, the current non-test `parse_markdown`/`parse_markdown_with_gfm_tables` call sites (18 files): `app/src/ai/agent/{mod,util}.rs`, `app/src/ai/blocklist/agent_view/zero_state_block.rs`, `app/src/ai/blocklist/block/numbered_button.rs`, `app/src/ai/blocklist/inline_action/{ask_user_question_view,inline_action_header,requested_command_attribution}.rs`, `app/src/ai_assistant/utils.rs`, `app/src/changelog_model.rs`, `app/src/code/language_server_extension.rs`, `app/src/notebooks/{editor/view,mod}.rs`, `app/src/settings_view/mcp_servers/installation_modal.rs`, `app/src/terminal/cli_agent.rs`, `app/src/workspace/view/launch_modal/mod.rs`, `crates/editor/src/content/text.rs`, `crates/editor/src/model.rs`, `crates/warpui/examples/formatted-text/root_view.rs`. The agent path and the notebook/plan editor are the interactive tier; the rest render through the sinks above. Promoting any other surface to the interactive tier later is additive and needs no parser or spec change.

**Open question for maintainers:** for the agent-conversation tier (which renders through `FormattedTextElement`, not editor buffers), whether the disclosure interaction reuses the existing block-folding machinery (as used for command blocks) or introduces a dedicated disclosure component — the spec constrains behavior (product invariants 1-3, 11), not the component choice. The editor tier has no such question: it folds via `HiddenLinesModel` (§4).

### Tradeoffs considered

- **Table-style internal code block in the buffer** (rejected, maintainer guidance): the `warp-markdown-table` mechanism is an intermediate implementation kept to leave table *editing* options open, and it makes the block multi-line — which is exactly what forces the `TableCache` per-cell offset-mapping apparatus onto selection. The §4 marker model keeps selection trivial and models the region natively.
- **Start/End marker lines in the parser IR** (rejected at that layer only): `FormattedText` consumers are stateless per-line renderers with no edit machinery to maintain balance, so the IR keeps container-as-line. The buffer, which *does* maintain paired markers (inline styles) and owns edit semantics, uses the paired representation — the split in §1/§4.
- **Verbatim body-source preservation** (dropped, maintainer guidance): the rich-text pipeline does not guarantee byte-exact markdown preservation anywhere; round-trip is canonical re-serialization (product invariant 10), which removes the duplicated body storage and its equality subtleties.
- **Full tree-shaped `FormattedText`** (rejected): correct long-term model but a cross-cutting rewrite of parser, delta, and editor mapping — far beyond this feature's blast radius.
- **Depth 8 / count 512**: the limits bound what each widget adds beyond its content — parser recursion depth, and per-widget focus/accessibility/interaction bookkeeping — not content size (product invariant 7). GFM content in the wild rarely nests past 3; 8 is generous. Both are constants so behavior is reproducible (Oz review of PR #10462 flagged non-deterministic "soft caps" — these are hard). Note `depth` in the buffer is `u8`-sized per marker; the parse-time constant is the only enforcement point.

## Testing and validation

Unit tests in `crates/markdown_parser/src/markdown_parser_tests.rs` (parser) and `html_parser_tests.rs` (imported HTML), mapped to product invariants:

| Invariant | Test |
|---|---|
| 1, 2 | `<details>` with/without `open` parses to `Details` with correct `default_open`; body markdown (code block, list, table) parses recursively |
| 4 | missing `<summary>` → literal summary; multiple `<summary>` siblings → first is summary, rest are body lines |
| 5 | nested `<details>` at depth ≤ 8 parses as nested `Details` |
| 6 | depth 9 falls back to plain text; depth 8 still renders; content identical either way (`raw_text` equality) |
| 7 | 512th widget renders, 513th falls back; deterministic across repeated parses |
| 8a | unclosed `<details>` consumes to end of input; unclosed nested consumes to parent close |
| 8b | stray `</details>` / `<summary>` outside a details body render as plain text (existing-behavior regression test) |
| 8c | mid-line `<details>` stays plain text |
| 8d | unclosed `<summary>` consumes the rest of the details body as summary; body empty; `raw_text` preserved |
| 8e | `</summary>` without opener inside a body renders as body plain text |
| 8f | `<summary>` attributes ignored; multi-line summary collapses line breaks; code fence / nested `<details>` inside summary render as literal inline text |
| 8g | `</details>` / `<details>` / `<summary>` inside a fenced code block in the body are code content: balance unchanged, tags appear verbatim in the rendered code block; `</details>` mid-paragraph does not close the container |
| 9 | `raw_text()` includes summary + body; `LineCount` independent of open state |
| 10 | canonical round-trip: parse → buffer → `to_markdown` → parse yields an equal `FormattedText` (structure equality, not byte equality), `open` attribute preserved; covers a nested details and a body containing code fences |
| 12 | `compute_formatted_text_delta` over successive streaming snapshots keeps `common_prefix_lines` stable for lines above the details block |

Editor buffer tests in `crates/editor` (the §4 model):

| Behavior | Test |
|---|---|
| pairing | flattening `FormattedTextLine::Details` produces `DetailsSummary` + body blocks + `DetailsEnd`; nested pairs preserve depth; depth-at-offset via the summary counter matches the pairing scan |
| rebalancing | deleting the `DetailsEnd` line → region extends to end of buffer (8a semantics); an orphaned `DetailsEnd` is inert and not serialized (8b semantics); both deterministic across edit orders |
| selection/copy | both markers read as one `"\n"`; selecting across a details boundary and copying yields summary text + body text with no offset drift; collapsed state does not change copy output (invariant 9, via `HiddenLinesModel` being view-only) |
| editing | Enter on a summary line starts a plain body block; `convert_block` to/from details preserves body blocks; undo history unaffected by fold/unfold toggles |

Interaction/accessibility invariants (3, 11) are validated with renderer-level tests in the block renderer's existing test harness, plus manual testing evidence (screen recording of toggle via mouse and keyboard, VoiceOver announcement) attached to the implementation PR as required by CONTRIBUTING.md.
