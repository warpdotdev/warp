# GH10259: Support `<details>`/`<summary>` in markdown rendering — Tech Spec

Issue: https://github.com/warpdotdev/warp/issues/10259
Product spec: [product.md](./product.md)

## Context

Markdown rendering is built on a flat, line-oriented document model:

- `crates/markdown_parser/src/lib.rs:156-168` — `FormattedTextLine` is the enum of renderable block variants (`Heading`, `Line`, `OrderedList`, `UnorderedList`, `CodeBlock`, `TaskList`, `LineBreak`, `HorizontalRule`, `Embedded`, `Image`, `Table`). There is no container variant; the document is a flat `VecDeque<FormattedTextLine>` (`FormattedText`, lib.rs:111-114).
- `crates/markdown_parser/src/markdown_parser.rs` — `parse_markdown` drives a nom `alt((...))` chain of block parsers (code block, header, image, task/ordered/unordered list, table, paragraph) inside a `while !remaining.is_empty()` loop (~lines 130-215). Inline HTML handling is deliberately narrow: HTML entities (`parse_html_entity`, ~line 1515) and `<u>`/`</u>` underline delimiters (`parse_inline_token_underline_start`/`_end`, lines 1626-1645). All other tags flow through as plain text.
- `crates/markdown_parser/src/html_parser.rs:23-28` — the imported-HTML path (paste from GDocs/Confluence/etc.) flattens container tags via `TOP_LEVEL_ELEMENT_TAGS_TO_SKIP` and treats a fixed set of `PHRASING_ELEMENT_TAGS` as inline. `<details>`/`<summary>` are in neither list, so their children are visited and the container semantics are lost.
- `crates/editor/src/content/markdown.rs` — maps `FormattedTextLine` variants to editor `BufferBlockStyle`s and back (`to_formatted_text`, lines ~416-520; `to_markdown`, lines ~61-160). Precedent: `FormattedTable` round-trips through a dedicated internal representation (`warp-markdown-table` code blocks, `to_internal_format`/`from_internal_format`, lib.rs:361-430) rather than extending the buffer's line model.
- `compute_formatted_text_delta` (lib.rs:66-109) diffs two `FormattedText` values line-by-line for streaming updates; any new variant must implement `LineCount` (lib.rs:284-300) and be comparable for equality.

## Proposed changes

### 1. New `FormattedTextLine::Details` variant (container-as-line, Table precedent)

Add to `crates/markdown_parser/src/lib.rs`:

```rust
pub struct FormattedDetails {
    pub summary: FormattedTextInline,     // parsed inline markdown; "Details" literal if absent
    pub body: FormattedText,              // recursively parsed markdown
    pub default_open: bool,               // `open` attribute present
}
// FormattedTextLine::Details(FormattedDetails)
```

The whole `<details>…</details>` region becomes a single `FormattedTextLine`, like `Table`. Rationale over start/end marker lines: markers would let intervening edits produce unbalanced documents, and every renderer/serializer consumer would need to track container state; a single variant keeps the flat model's invariants intact.

- `LineCount::num_lines` returns `1 + body line count` (content lines, independent of collapsed state — collapsed is view state, spec invariant 9).
- `raw_text()` emits summary then body text.
- `set_weight`/`inline_fragments`/`hyperlinks` recurse into summary and body.

### 2. Parser: new block branch in `parse_markdown`

Add `parse_details` to the `alt((...))` block chain in `crates/markdown_parser/src/markdown_parser.rs`, before `parse_paragraph`:

- Matches only at block start (spec invariant 8c): optional leading spaces, `<details` + optional attributes (`open` recognized, others ignored) + `>`.
- Scans forward for the matching `</details>` with a tag-balance counter; on success, extracts the first top-level `<summary>…</summary>` (invariant 4) and recursively calls `parse_markdown` on the body with a `depth` parameter.
- Depth and count limits (invariants 6-7): `const MAX_DETAILS_DEPTH: usize = 8; const MAX_DETAILS_PER_DOC: usize = 512;` threaded through the existing parse context. On exceeding either, the branch returns `nom::Err::Error` so the input deterministically falls through to `parse_paragraph` (plain text) — no panic path, bounded recursion.
- Unclosed `<details>` (invariant 8a): treat rest of the current parse input as body. Because parsing is re-run on streaming updates, a still-streaming block naturally renders progressively (invariant 12); `compute_formatted_text_delta` sees the growing `Details` line as the changed suffix, so preceding lines keep their prefix identity.

### 3. Imported-HTML path

In `crates/markdown_parser/src/html_parser.rs`, handle `details`/`summary` elements explicitly: build the same `FormattedDetails` from the DOM (html5ever already provides the tree, so no new scanning is needed) with the same depth/count constants applied.

### 4. Rendering and editor mapping

The buffer round-trip follows the `FormattedTable` mechanism exactly. Tables survive the flat buffer model as dedicated internal code blocks (`TABLE_BLOCK_MARKDOWN_LANG = "warp-markdown-table"`, markdown_parser.rs:39, with `to_internal_format`/`from_internal_format`, lib.rs:361-430); details blocks get the analogous `DETAILS_BLOCK_MARKDOWN_LANG = "warp-markdown-details"` internal representation:

- **Internal format**: line 1 is a header carrying the `open` flag and the summary serialized as inline markdown (tab-separated, like the table format); lines 2..N are the body's markdown source verbatim — including any nested `<details>` markup as literal text. `FormattedDetails::to_internal_format`/`from_internal_format` in `crates/markdown_parser/src/lib.rs` own this, mirroring `FormattedTable`.
- **Boundaries**: the enclosing internal code block's fence delimits the details region in the buffer, so body boundaries are exact regardless of body content. When the body itself contains code fences, the outer fence uses the standard CommonMark longer-fence rule (as `to_markdown` already must for code blocks).
- **Nesting**: nested details exist inside the buffer only as literal markdown text within the parent's internal block; they are re-materialized by `parse_markdown` on conversion back to `FormattedText` (`from_internal_format` → recursive parse, subject to the same `MAX_DETAILS_DEPTH`). This means nesting depth never multiplies buffer block types and the flat buffer model is untouched.
- In `crates/editor/src/content/markdown.rs`: `to_formatted_text` (lines ~416-520) maps `warp-markdown-details` code blocks to `FormattedTextLine::Details`, exactly where `BufferBlockStyle::Table` is handled today (line ~489); `to_markdown` (lines ~61-160) emits `<details>`/`<summary>` markup, adding ` open` when `default_open` (invariant 10).
- The disclosure widget itself (collapse/expand, focus, click, Enter/Space handling, renderer-generated accessibility IDs) lives in the block renderer layer. Open/collapsed is per-view UI state initialized from `default_open`, not part of `FormattedDetails` equality — so toggling does not dirty the text delta.

### 5. Rendering surfaces (product invariant 11)

`FormattedTextLine::Details` flows to every `parse_markdown` consumer; the rendering tier is decided per surface, with the static fallback as the default so no consumer renders in an undefined state.

**Interactive tier (initial implementation)** — the agent conversation block path: `app/src/ai/agent/util.rs:35` (`parse_markdown_into_text_and_code_sections`, via `parse_markdown_with_gfm_tables` at util.rs:189). This is where agent output renders and where the disclosure widget is implemented.

**Static-fallback tier** — all other current `parse_markdown` call sites render summary-then-body expanded with no widget. Enumerated at time of writing:
- `app/src/ai/blocklist/inline_action/inline_action_header.rs:232` (action titles), `.../requested_command_attribution.rs:60`, `.../ask_user_question_view.rs:1913` (option text), `app/src/ai/blocklist/block/numbered_button.rs:151` (button labels) — single-line label contexts; a block-level details cannot meaningfully collapse here.
- `app/src/ai/blocklist/agent_view/zero_state_block.rs:628`, `app/src/changelog_model.rs:83,188`, `app/src/settings_view/mcp_servers/installation_modal.rs:334`, `app/src/workspace/view/launch_modal/mod.rs:406`, `app/src/ai_assistant/utils.rs:403` — static content panels.

Promoting any fallback surface to the interactive tier later is additive and needs no parser or spec change.

**Open question for maintainers:** whether the interactive tier reuses the existing block-folding interaction machinery (as used for command blocks) or introduces a dedicated disclosure component — the spec constrains behavior (product invariants 1-3, 11), not the component choice.

### Tradeoffs considered

- **Start/End marker lines** (rejected): keeps parser trivial but leaks balancing invariants to every consumer; delta/edit paths can split containers.
- **Full tree-shaped `FormattedText`** (rejected): correct long-term model but a cross-cutting rewrite of parser, delta, and editor mapping — far beyond this feature's blast radius.
- **Depth 8 / count 512**: GFM content in the wild rarely nests past 3; 8 is generous while keeping recursion trivially bounded. 512 bounds widget/interaction bookkeeping per document. Both are constants so behavior is reproducible (Oz review of PR #10462 flagged non-deterministic "soft caps" — these are hard).

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
| 9 | `raw_text()` includes summary + body; `LineCount` independent of open state |
| 10 | markdown round-trip preserves tags and `open` attribute (editor `to_markdown` test); `warp-markdown-details` internal format round-trips `FormattedDetails` including a nested details (parse → buffer → markdown → parse equality) |
| 12 | `compute_formatted_text_delta` over successive streaming snapshots keeps `common_prefix_lines` stable for lines above the details block |

Interaction/accessibility invariants (3, 11) are validated with renderer-level tests in the block renderer's existing test harness, plus manual testing evidence (screen recording of toggle via mouse and keyboard, VoiceOver announcement) attached to the implementation PR as required by CONTRIBUTING.md.
