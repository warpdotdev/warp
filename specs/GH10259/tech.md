# GH10259: Support `<details>`/`<summary>` in markdown rendering — Tech Spec

Issue: https://github.com/warpdotdev/warp/issues/10259
Product spec: [product.md](./product.md)

## Context

Markdown rendering is built on a flat, line-oriented document model:

- `crates/markdown_parser/src/lib.rs:156-168` — `FormattedTextLine` is the enum of renderable block variants (`Heading`, `Line`, `OrderedList`, `UnorderedList`, `CodeBlock`, `TaskList`, `LineBreak`, `HorizontalRule`, `Embedded`, `Image`, `Table`). There is no container variant; the document is a flat `VecDeque<FormattedTextLine>` (`FormattedText`, lib.rs:111-114).
- `crates/markdown_parser/src/markdown_parser.rs` — `parse_markdown` drives a nom `alt((...))` chain of block parsers (code block, header, image, task/ordered/unordered list, table, paragraph) inside a `while !remaining.is_empty()` loop (~lines 130-215). Inline HTML handling is deliberately narrow: HTML entities (`parse_html_entity`, ~line 1515) and `<u>`/`</u>` underline delimiters (`parse_inline_token_underline_start`/`_end`, lines 1626-1645). All other tags flow through as plain text.
- `crates/markdown_parser/src/html_parser.rs:23-28` — the imported-HTML path (paste from GDocs/Confluence/etc.) flattens container tags via `TOP_LEVEL_ELEMENT_TAGS_TO_SKIP` and treats a fixed set of `PHRASING_ELEMENT_TAGS` as inline. `<details>`/`<summary>` are in neither list, so their children are visited and the container semantics are lost.
- `crates/editor/src/content/markdown.rs` — maps `FormattedTextLine` variants to editor `BufferBlockStyle`s and back (`to_formatted_text`, lines ~416-520; `to_markdown`, lines ~61-160). The buffer model this spec builds on: a block's style metadata lives in a `BufferText::BlockMarker { marker_type: BufferBlockStyle }` sentinel at the block's start (`crates/editor/src/content/text.rs:541-571`, enum at text.rs:867). List blocks carry their metadata (indent level, ordered-list start number, task-list checkbox state) on that marker; to selection, copy, and every offset dimension the marker reads as a single `"\n"` (text.rs:~626), which is why list metadata never complicates text selection. Table, by contrast, is a multi-line block smuggled through an internal code-block format and needs the `TableCache` per-cell offset maps to make selection work — maintainer guidance on this PR is explicitly that the table mechanism is an intermediate implementation and must not be replicated for details.
- Span markers orthogonal to block style: `BufferText::Link(LinkMarker::Start(String)/End)` (text.rs:351) is a top-level, **zero-width** start/end pair whose `Start` carries payload, counted in the `SumTree` via `StyleSummary::link_counter`/`total_link_marker` and the `LinkCount` dimension (text.rs:1550/1743). A link span coexists with any block/inline style over its range because it is not a style itself — the precedent this spec follows for details (a span, not a block style). Contrast the data-less inline-style pair `Marker { marker_type, dir }` and the per-`\n` `BlockMarker`/`BufferBlockStyle`.
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

In the parser IR (the read-only rendering model), the whole `<details>…</details>` region is a single `FormattedTextLine`. Start/end marker lines are wrong *at this layer*: `FormattedText` consumers are stateless per-line renderers and `compute_formatted_text_delta` diffs lines independently, so nothing maintains marker balance here. The editor buffer is the opposite case — it has edit machinery that already maintains paired zero-width span markers (links) — and gets a delimited-marker representation instead (§4).

- `LineCount::num_lines` returns `1 + body line count`, where the `1` is the summary/header line the container renders above its body (present regardless of collapse — collapsed is view state, spec invariant 9).
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

### 4. Editor buffer: details as a span marker, not a block style (link-marker precedent)

Details markers are a **new top-level `BufferText` variant modeled on `Link`**, not a `BufferBlockStyle`. This is the correction from review: a `BufferBlockStyle` is per-character-exclusive (one active block style per position), so a code block *inside* a details region could not be both "a code block" and "inside details" if details were a block style. Links already solve exactly this — a link span coexists with whatever inline styles its text carries — by being an independent zero-width marker layer. Details replicate that layer at block granularity.

- **The marker**: add `BufferText::Details(DetailsMarker)` (text.rs:541), mirroring `Link(LinkMarker)`:

  ```rust
  pub enum DetailsMarker {
      Start { default_open: bool }, // `open` attribute; summary is the buffer text up to EndSummary
      EndSummary,                   // separates summary text from body blocks
      End,
  }
  ```

  Three markers rather than two, so the summary carries live formatting: the buffer structure is `<details:start default_open=true>` summary text `<details:end-summary>` body blocks `<details:end>`. All three are **zero-width in the char stream**, like `LinkMarker`: each is added to the zero-width arm of `Item::summary()` (text.rs:1817, next to `Marker`/`Link`/`Color`) and left in the skipped `_ => ()` arm of the `Bytes` iterator (text.rs:301). None occupies a `\n`. **The summary is the ordinary styled buffer text between `Start` and `EndSummary`** — so summary formatting (bold, inline code, links) is tracked by the buffer's normal inline-style and link-marker machinery exactly like body text, and editing the summary is ordinary inline character editing rather than a marker-data edit. This is the correction from the last review note: a `String` summary on the `Start` marker could not carry live formatting spans; giving the summary its own delimited text range does, at the cost of one extra zero-width marker and no change to the depth counter — only `Start`/`End` are counted, `EndSummary` is depth-neutral. Extracting the summary for render or serialization is a seek/linear scan from a `Start` to its matching `EndSummary`.

- **Region and body**: a details region spans `Start` … `End`, split by its `EndSummary` into a *summary* sub-range (`Start` … `EndSummary`, inline styled text — no block markers, per product invariant 8f) and a *body* sub-range (`EndSummary` … `End`). The body is whatever blocks fall in that sub-range — **each keeping its own `BufferBlockStyle`** (paragraph, code block, list, table). Nesting is just nested `Start`/`End` pairs. Because details is orthogonal to block style, a renderer can consult the `SumTree` for *both* "am I inside a details span, and at what depth" *and* "what is this block's own style," and adjust per-block styling (colors/fonts) for details content — the capability the block-style model foreclosed.

- **Depth via a SumTree counter (link precedent)**: `LinkMarker` already feeds a balanced counter and a total in `StyleSummary` (`link_counter`, `total_link_marker`, text.rs:1550), queryable as the `LinkCount` dimension (text.rs:1743). Add the analogous `details_counter`/`total_details_marker` fields plus a `DetailsDepth` dimension. Only `Start` (+1) and `End` (−1) feed `details_counter`; `EndSummary` is depth-neutral. The counter value at an offset *is* the nesting depth — no depth field is stored on the marker; depth is a summary seek, O(log n), and structurally correct by construction. `MAX_DETAILS_DEPTH` is enforced on conversion into the buffer.

- **Selection/copy stay trivial and preserve the full body**: the markers are zero-width and body blocks are ordinary blocks, so there is no offset mapping and no selection special-casing. Copy is verified to walk raw buffer offsets over the selection range and to **not** consult `HiddenLinesModel` (`selected_text_as_plain_text` → `clipboard_text_in_range`, buffer.rs:1039/2374; selection anchors resolve to real offsets, selection_model.rs:128) — so selecting across a *collapsed* details region copies the full hidden summary and body (the summary being real buffer text, it is copied by the same offset walk, no special-casing). This is what makes product invariant 9 hold, and it is an invariant the implementation must preserve: if copy is ever changed to respect folds, details would need to opt out.

- **Edit invariants (deterministic rebalancing, mirroring product 8a/8b)**: an unmatched `Start` (its `End` was deleted) owns the range to end of buffer — the 8(a) rule; an unmatched `End` is inert and skipped on serialization — the 8(b) rule. For the third marker: a `Start` with no matching `EndSummary` before its `End` (or buffer end) has an empty summary and renders the literal `Details` label — the product invariant-4 missing-summary rule; a stray `EndSummary` outside any `Start`…`End` span is inert and skipped on serialization. All fall out of the counter/pairing scan, not stored state, so no edit sequence yields an undefined document — the same way unbalanced link markers degrade rather than corrupt. Converting a block range to/from a details region goes through the existing `BufferEditAction` path (`set_block_style`/`convert_block`, model.rs:1142/1175) extended to insert/remove the marker trio (`Start`/`EndSummary`/`End`).

- **Collapse = existing folding**: collapsed state reuses `HiddenLinesModel` (hidden_lines_model.rs:20) — the body range is an anchor-pair hidden region, per-editor and outside buffer content, initialized from `default_open`. Hiding is view state consulted only at render/navigation time (render/model/mod.rs:2691, selection.rs:414-459), never by copy/serialization; toggling does not edit the buffer, so it cannot dirty deltas or undo history.

- **Round-trip** (`crates/editor/src/content/markdown.rs`): `to_markdown` (lines ~61-160) emits `<details>` (` open` iff `default_open`) + `<summary>` + the summary sub-range serialized through the normal inline arms + `</summary>`, serializes the body blocks through their normal arms, and emits `</details>` at the matching `End` — canonical re-serialization, no verbatim source (product invariant 10). `to_formatted_text` (lines ~416-520) does the inverse, folding the summary sub-range into the container's `summary: FormattedTextInline` and the body blocks into `body`, yielding a `FormattedTextLine::Details` (recursively for nested pairs); `core.rs` gains parse arms alongside the list arms (core.rs:~742/781) that flatten a `FormattedTextLine::Details` into `Start` marker + summary inline text + `EndSummary` marker + body blocks + `End` marker on the way in.

- **Render model**: the render `SumTree` gains a details-span dimension consulted per block (depth + open state) plus a `render/element/details.rs` that paints the summary line and disclosure triangle above the first body block of each span (the summary is the styled text in the `Start`…`EndSummary` range, laid out with its own inline formatting). Focus, click, Enter/Space toggle, and renderer-generated accessibility IDs live here.

### 5. Rendering surfaces (product invariant 11)

The static fallback is not an enumerated per-surface obligation — it is implemented once, at the two sinks all consumers render through, so every current and future consumer gets defined behavior by construction:

1. **`FormattedTextElement`** (`crates/warpui_core/src/elements/gui/formatted_text_element.rs:144`) — the shared GUI element that lays out `FormattedText` for the app's markdown surfaces (modals, banners, changelog, settings pages, conversation list, etc.). It gains layout for the `Details` variant: summary line, then body lines, expanded, no disclosure indicator — the invariant-11 fallback. The interactive widget is an opt-in builder method on this element (consistent with its existing `with_*`/`disable_mouse_interaction` builder API), enabled only by the agent conversation block path (`app/src/ai/agent/util.rs:35`, `parse_markdown_into_text_and_code_sections` via `parse_markdown_with_gfm_tables` at util.rs:189).
2. **The editor buffer conversion** (`crates/editor/src/content/markdown.rs`) — consumers that push `FormattedText` into editor buffers get the §4 buffer-native mapping. Because §4 models details directly in the buffer with `HiddenLinesModel` folding, the notebook/plan editor (`RichTextEditorView`, `app/src/notebooks/editor/view.rs:1029`) is an interactive-tier surface (product invariant 11): its buffers fold and toggle details natively, which is the second surface the product spec commits to.

Additionally, adding a variant to `FormattedTextLine` is a compile-time forcing function: every exhaustive `match` on the enum fails to compile until the new variant is handled, so no consumer can silently mis-render. The implementation audits the non-parser match sites (57 non-test files reference `FormattedTextLine` outside `crates/markdown_parser` at time of writing) and routes each to sink 1 or 2; sites that only pattern-match specific variants (e.g. hyperlink extraction) need no change.

For completeness, the current non-test `parse_markdown`/`parse_markdown_with_gfm_tables` call sites (18 files): `app/src/ai/agent/{mod,util}.rs`, `app/src/ai/blocklist/agent_view/zero_state_block.rs`, `app/src/ai/blocklist/block/numbered_button.rs`, `app/src/ai/blocklist/inline_action/{ask_user_question_view,inline_action_header,requested_command_attribution}.rs`, `app/src/ai_assistant/utils.rs`, `app/src/changelog_model.rs`, `app/src/code/language_server_extension.rs`, `app/src/notebooks/{editor/view,mod}.rs`, `app/src/settings_view/mcp_servers/installation_modal.rs`, `app/src/terminal/cli_agent.rs`, `app/src/workspace/view/launch_modal/mod.rs`, `crates/editor/src/content/text.rs`, `crates/editor/src/model.rs`, `crates/warpui/examples/formatted-text/root_view.rs`. The agent path and the notebook/plan editor are the interactive tier; the rest render through the sinks above. Promoting any other surface to the interactive tier later is additive and needs no parser or spec change.

**Open question for maintainers:** for the agent-conversation tier (which renders through `FormattedTextElement`, not editor buffers), whether the disclosure interaction reuses the existing block-folding machinery (as used for command blocks) or introduces a dedicated disclosure component — the spec constrains behavior (product invariants 1-3, 11), not the component choice. The editor tier has no such question: it folds via `HiddenLinesModel` (§4).

### Tradeoffs considered

- **Table-style internal code block in the buffer** (rejected, maintainer guidance): the `warp-markdown-table` mechanism is an intermediate implementation kept to leave table *editing* options open, and it makes the block multi-line — which is exactly what forces the `TableCache` per-cell offset-mapping apparatus onto selection. The §4 span-marker model keeps selection trivial and models the region natively.
- **Details as a `BufferBlockStyle`** (rejected, review feedback): a block style is per-character-exclusive, so details content could not simultaneously carry its own block style (code block, list). Modeling details as a top-level zero-width span marker (the `Link` precedent) makes the two orthogonal, so body blocks keep their styles and a renderer can query both dimensions independently.
- **Start/End marker lines in the parser IR** (rejected at that layer only): `FormattedText` consumers are stateless per-line renderers with no edit machinery to maintain balance, so the IR keeps container-as-line. The buffer, which *does* maintain paired zero-width span markers (links) and owns edit semantics, uses the delimited-marker representation (`Start`/`EndSummary`/`End`) — the split in §1/§4.
- **Verbatim body-source preservation** (dropped, maintainer guidance): the rich-text pipeline does not guarantee byte-exact markdown preservation anywhere; round-trip is canonical re-serialization (product invariant 10), which removes the duplicated body storage and its equality subtleties.
- **Full tree-shaped `FormattedText`** (rejected): correct long-term model but a cross-cutting rewrite of parser, delta, and editor mapping — far beyond this feature's blast radius.
- **Depth 8 / count 512**: the limits bound what each widget adds beyond its content — parser recursion depth, and per-widget focus/accessibility/interaction bookkeeping — not content size (product invariant 7). GFM content in the wild rarely nests past 3; 8 is generous. Both are constants so behavior is reproducible (Oz review of PR #10462 flagged non-deterministic "soft caps" — these are hard). In the buffer, depth is not stored per marker but read from the details `SumTree` counter (the `LinkCount` precedent); the parse-time constant is the only enforcement point.

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
| pairing | flattening `FormattedTextLine::Details` produces `Details(Start{..})` + summary inline text + `Details(EndSummary)` + body blocks (each with its own `BufferBlockStyle`) + `Details(End)`; nested pairs increment the details counter; `EndSummary` leaves the counter unchanged; depth-at-offset via the `DetailsDepth` `SumTree` dimension matches the pairing scan |
| summary formatting | a summary containing bold / inline code / a link round-trips through buffer → `to_markdown` → parse with its inline styles and link markers intact (the property the `String`-on-marker model lost); the summary sub-range carries real inline-style spans, not marker payload |
| orthogonality | a code block / list inside a details span keeps its own `BufferBlockStyle`; the details counter is nonzero over its range while the block style is unchanged (the property the block-style model lost) |
| zero-width | all three `Details` markers contribute 0 chars/bytes/lines to `TextSummary` and emit nothing from the `Bytes` iterator; a details span adds no `\n` to the char stream (the summary's own text still counts as ordinary chars) |
| rebalancing | deleting the `Details(End)` marker → span extends to end of buffer (8a semantics); an orphaned `Details(End)` is inert and not serialized (8b semantics); deleting the `Details(EndSummary)` → empty summary + literal `Details` label (invariant-4 semantics); an orphaned `Details(EndSummary)` is inert and not serialized; all deterministic across edit orders |
| selection/copy | selecting across a details boundary and copying yields summary + full body with no offset drift; copying a **collapsed** span still yields the full body (regression-guards that copy ignores `HiddenLinesModel`, invariant 9) |
| editing | `convert_block` to/from a details span inserts/removes the marker trio (`Start`/`EndSummary`/`End`) and preserves body blocks; undo history unaffected by fold/unfold toggles |

Interaction/accessibility invariants (3, 11) are validated with renderer-level tests in the block renderer's existing test harness, plus manual testing evidence (screen recording of toggle via mouse and keyboard, VoiceOver announcement) attached to the implementation PR as required by CONTRIBUTING.md.
