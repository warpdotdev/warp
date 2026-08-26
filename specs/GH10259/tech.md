# Support `<details>`/`<summary>` in markdown rendering — Tech Spec

GitHub issue: https://github.com/warpdotdev/warp/issues/10259

Product spec: [product.md](./product.md)

## Context

Markdown rendering runs through a flat, line-oriented intermediate representation, and the editor buffer that some surfaces convert it into.

### The parser IR

- `crates/markdown_parser/src/lib.rs:156-168` — `FormattedTextLine` enumerates the renderable block variants: `Heading`, `Line`, `OrderedList`, `UnorderedList`, `CodeBlock`, `TaskList`, `LineBreak`, `HorizontalRule`, `Embedded`, `Image`, `Table`. There is no container variant, and a document is a flat `VecDeque<FormattedTextLine>` (`FormattedText`, `lib.rs:112-114`).
- `crates/markdown_parser/src/markdown_parser.rs:141-235` — `parse_markdown_internal` drives a nom `alt((…))` block-parser chain (`markdown_parser.rs:149-190`: blank line, horizontal rule, code block, header, image, task list, ordered list, unordered list, GFM-gated table, paragraph) inside a `while !remaining.is_empty()` loop (`markdown_parser.rs:195-232`). The public entry points `parse_markdown` and `parse_markdown_with_gfm_tables` are thin wrappers at `markdown_parser.rs:111-134`.
- **`parse_markdown_internal` carries no threadable parse context.** Its parameters are `(markdown: &str, parse_gfm_tables: bool)` (`markdown_parser.rs:141-144`), and its only state, a `ListIndentationContext`, is constructed fresh and locally at `markdown_parser.rs:145` rather than passed in. Adding details support therefore requires extending that signature, which §2 accounts for.
- **Code fences are backtick-only.** `parse_code_block` (`markdown_parser.rs:888-903`) matches `tag("```")` after `parse_indentation`, and `parse_closing_fence` (`markdown_parser.rs:962-977`) matches a closing fence at the opening indentation or up to three columns deeper. Neither accepts `~~~`, and no other fence syntax appears in the crate's source.
- Inline HTML handling is deliberately narrow: HTML entities (`parse_html_entity`, `markdown_parser.rs:1974`) and `<u>`/`</u>` underline delimiters (`parse_inline_token_underline_start`/`_end`, `markdown_parser.rs:1682`/`1695`). Every other tag flows through as plain text, which is why `<details>` and `<summary>` are visible literals today.
- `crates/markdown_parser/src/html_parser.rs:23`/`:26` — the imported-HTML path (paste from Google Docs, Confluence, and similar) flattens container tags listed in `TOP_LEVEL_ELEMENT_TAGS_TO_SKIP` and treats `PHRASING_ELEMENT_TAGS` as inline. Neither tag appears anywhere in that file, so `<details>`/`<summary>` children are visited and the container semantics are lost.
- `compute_formatted_text_delta` (`lib.rs:66-109`) diffs two `FormattedText` values line by line for streaming updates. Any new variant must implement `LineCount` (`lib.rs:39-41`, `FormattedTextLine` impl at `lib.rs:284-300`) and compare for equality.
- **`inline_fragments` returns one borrowed run.** Its signature is `fn inline_fragments(&self) -> Option<&FormattedTextInline>` (`lib.rs:245`), and `hyperlinks` (`lib.rs:261`) walks that single run accumulating char offsets. Neither can express a variant whose inline content spans a summary plus arbitrarily many body lines, which §1 accounts for.

### The editor buffer

- `crates/editor/src/content/text.rs:541` — the `BufferText` enum: `Text`, `Marker`, `Link(LinkMarker)`, `Color(ColorMarker)`, `Newline`, `BlockItem`, `BlockMarker { marker_type: BufferBlockStyle }`, `Placeholder`.
- A block's style metadata lives in a `BufferText::BlockMarker` sentinel at the block's start (`text.rs:564-566`), typed by `BufferBlockStyle` (`text.rs:867`: `CodeBlock`, `TaskList`, `PlainText`, `Header`, `UnorderedList`, `OrderedList`, `Table`). List blocks carry indent level, ordered-list start number, and checkbox state on that marker. A `BlockMarker` reads as a single `"\n"` in the byte iterator (`text.rs:315`), which is why list metadata never complicates selection.
- **Exactly one `BufferBlockStyle` is active for a given character.** This is the constraint that decides the data model below: a details region coexists with the block styles of its body lines, so it cannot live in that channel.
- `crates/editor/src/content/text.rs:555` — `BufferText::Link(LinkMarker)`, with `LinkMarker { Start(String), End }` at `text.rs:352`, is a top-level **zero-width** start/end pair whose `Start` carries payload. It is counted in the `SumTree` through `StyleSummary::link_counter`/`total_link_marker` (`text.rs:1551`, fields at `text.rs:1559`/`1563`) and queried through the `LinkCount` dimension (`text.rs:1741`). A link span coexists with any block or inline style over its range precisely because it is not itself a style. This is the precedent this spec follows.
- `Table` is the contrasting case: a multi-line block smuggled through an internal code-block format, requiring per-cell offset maps to make selection work. Maintainer guidance on this issue is that the table mechanism is an intermediate implementation kept to leave table editing options open, and must not be replicated here.
- Folding already exists: `HiddenLinesModel` (`crates/editor/src/content/hidden_lines_model.rs:20`) stores hidden ranges as `Vec<(Anchor, Anchor)>` anchor pairs, per editor and independent of buffer content. It is consulted at render time (`crates/editor/src/render/model/mod.rs`, `hidden_line_ranges` projecting the model to logical line ranges at `:861-869`, version-scoped lookup feeding `layout_edit_delta` at `:3332-3347`) and for cursor navigation (`crates/editor/src/selection.rs:414-459`). The click affordance is `hidden_section_clicked` (`crates/editor/src/render/element/mod.rs:379`) rendering a `RenderableHiddenSection` (`crates/editor/src/render/element/hidden_section.rs`).
- **The copy path does not consult `HiddenLinesModel`.** `selected_text_as_plain_text` (`crates/editor/src/content/buffer.rs:1039`) walks `selections_to_offset_ranges` (`crates/editor/src/content/selection_model.rs:129`) into `clipboard_text_in_ranges`/`clipboard_text_in_range` (`buffer.rs:2363`/`:2374`); `crates/editor/src/content/buffer.rs` contains no reference to `HiddenLinesModel` or hidden ranges at all. Selecting across a collapsed region therefore copies its full body today, which is what makes product behavior 10 hold by construction.
- `crates/editor/src/content/markdown.rs` — `to_markdown` (`:61`) and `to_formatted_text` (`:416`) convert between the IR and buffer blocks; `crates/editor/src/content/core.rs` flattens a `FormattedTextLine` into buffer blocks inside `reverse_core_edit_action` (`:472`, match arms from `:571`, list arms at `:743`/`:782`/`:827`).

### Render surfaces

- `FormattedTextElement` (`crates/warpui_core/src/elements/gui/formatted_text_element.rs:145`) is the shared GUI element laying out `FormattedText` for the app's markdown surfaces. It already exposes a builder API (`with_color`, `with_alignment`, `with_no_text_wrapping`, and others) including `disable_mouse_interaction` (`:521`).
- The agent conversation path parses through `parse_markdown_into_text_and_code_sections` (`app/src/ai/agent/util.rs:35`, calling `parse_markdown_with_gfm_tables` at `:189`).
- The notebook/plan editor renders through editor buffers; its view identifier is `RichTextEditorView` (`app/src/notebooks/editor/view.rs:97` and throughout).
- **The TUI is a real second consumer of the IR**, and it is only partly compiler-guarded. `crates/warp_tui/src/tui_markdown.rs` imports `FormattedText`/`FormattedTextLine` (`:8-10`). Inside `render_formatted_text` (`:76-151`) the per-line match at `:85-141` covers all 11 variants with no wildcard arm, so a new variant fails to compile there. `should_insert_blank_row` (`:152-190`) does **not**: it ends in `_ => false` (`:189`), so a new variant silently takes the no-blank-row default. `crates/editor/src/content/core.rs` (arms from `:571`, last arm at `:891`) is the other genuinely exhaustive match outside the parser crate.
- 65 non-test files outside `crates/markdown_parser` reference `FormattedTextLine`, and 24 call `parse_markdown`/`parse_markdown_with_gfm_tables`. Most only construct values or match a single variant; the two matches above are the exhaustive ones.

No `<details>`/`<summary>` handling exists anywhere in the workspace today.

## Proposed changes

### 1. Parser IR: a `Details` container variant

Add to `crates/markdown_parser/src/lib.rs`:

```rust
pub struct FormattedDetails {
	pub summary: DetailsSummary,
	pub body: FormattedText,
	pub default_open: bool,
	pub closed: bool,
}

pub enum DetailsSummary {
	Absent,
	Closed(FormattedTextInline),
	Unclosed(FormattedTextInline),
}
// FormattedTextLine::Details(FormattedDetails)
```

`body` is recursively parsed markdown. There is no verbatim-source field: serialization is canonical re-serialization from parsed structure, matching the rest of the rich-text pipeline. `PartialEq` derives over all fields, so streaming delta comparison compares parsed structure.

**`summary` and `closed` carry source shape, because product behavior 14 makes the source shape observable.** Behavior 14 requires that serialization reproduce each malformed shape as the literal characters the author typed, so the difference between a region that was closed and one that was not — and between a summary that was absent, closed, or unclosed — survives into the output document and cannot be recovered from summary and body text alone. The three summary states are mutually exclusive, so they are one enum rather than a flag beside the run.

| Source shape | Representation | `to_markdown` writes | Behavior |
|---|---|---|---|
| `<details>…</details>` | `closed: true` | opening and closing tag | 1 |
| `<details>` with no `</details>` | `closed: false` | opening tag, no closing tag | 14(a) |
| no `<summary>` element | `DetailsSummary::Absent` | no `<summary>` element; renders the substitute label `Details` | 14(e) |
| `<summary>…</summary>` | `DetailsSummary::Closed(run)` | `<summary>`, the run, `</summary>` | 3 |
| `<summary>` with no `</summary>` | `DetailsSummary::Unclosed(run)` | `<summary>` and the run, no closing tag; body is empty | 14(g) |

`Absent` holds no run: the `Details` label is a rendering substitute rather than content (product behavior 14(e)), so the renderer supplies it and no code path can serialize it into the document by mistake. Renderers read the summary through one accessor that yields the label for `Absent` and the run otherwise, so the substitution lives in one place.

A self-closing `<details/>` (product behavior 14(d)) is not a fourth shape: it degrades under 14(a), so it parses to `closed: false` and re-serializes as a plain opening tag. The canonical-re-serialization non-goal is what permits that spelling change; byte-exactness is not promised, and no tag is lost.

At this layer the whole region is a single `FormattedTextLine`. Start and end marker lines would be wrong here: `FormattedText` consumers are stateless per-line renderers and `compute_formatted_text_delta` diffs lines independently, so nothing at this layer maintains marker balance. The editor buffer is the opposite case — it owns edit machinery that already maintains paired zero-width markers — and gets the paired-marker representation in §4.

The IR-trait implementations split by whether the trait can express nested content:

- `LineCount::num_lines` returns `1 + body line count`, where the `1` is the summary row, which renders regardless of collapse state (collapse is view state, product behavior 9).
- `raw_text()` emits summary text then body text, where a summary-less region's summary text is the substitute label `Details` — matching what the surface renders, since `raw_text` is the rendered-text accessor and tag characters never appear in it (product behavior 13).
- `set_weight` returns `&Self` and mutates in place (`lib.rs:208`), so it recurses into the summary run, when there is one, and into the body.
- `inline_fragments` returns the summary run for `Closed` and `Unclosed`, and `None` for `Absent`, since there is no stored run to borrow. It cannot return the body: its return type is a reference to one `FormattedTextInline` (`lib.rs:245`), and a details region has one inline run per body line. **`hyperlinks` (`lib.rs:261`) therefore gains a `Details` arm that does not go through `inline_fragments`**: it collects the summary's links, then recurses into each body line's own `hyperlinks`, offsetting each result by the char length of everything preceding that line in `raw_text()` order. Without this arm, every link inside a details body would silently vanish from link extraction — the one place where the borrowed-single-run signature would otherwise cause a regression rather than a compile error. The offset base for body links is `raw_text()` order, so a summary-less region offsets by the label's char length, keeping link offsets aligned with the text a user sees.

### 2. Parser: a `parse_details` block branch

Add `parse_details` to the `alt((…))` chain in `parse_markdown_internal` (`markdown_parser.rs:149-190`), ahead of `parse_paragraph`.

**Signature change.** `parse_markdown_internal` (`markdown_parser.rs:141-144`) gains a parse-state parameter carrying the current nesting depth, since it holds no context object today. The public wrappers (`markdown_parser.rs:111-134`) construct the initial state, so their signatures are unchanged.

- **Line-start matching** (product behavior 14(c)): optional leading whitespace, then `<details` matched case-insensitively (product behavior 5), optional attributes with `open` recognized by presence alone regardless of any value (product behavior 4) and the rest ignored, optional whitespace, then `>`. A tag preceded by any non-whitespace character on its line, including a backtick, is not matched and falls through to `parse_paragraph`. This is what makes an inline code span containing the tag inert (product behavior 15) with no separate rule. A `/` before the `>` is accepted and produces a region with no distinct closing tag, which then degrades under product behavior 14(d).
- **Fence-aware delimiting** (product behavior 15): the matching `</details>` is not found by raw text scan. The delimiter walks lines, toggling an in-fence flag using the **backtick-fence recognition the parser actually implements** — an opening ` ``` ` after indentation, closed by a fence at the opening indentation or up to three columns deeper, mirroring `parse_code_block`/`parse_closing_fence` (`markdown_parser.rs:888-903`/`:962-977`). There is no `~~~` fence in this parser, so the delimiter recognizes none either. Tags are recognized only on non-fenced lines at line start, so tags on fenced lines never move the balance counter. A fence that never closes leaves the flag set through end of input, so no later `</details>` is recognized and the region degrades under product behavior 14(a) — product behavior 16, which follows from the flag rather than from a special case.
- **Summary extraction** happens on that same fence-aware walk, so a `<summary>` inside a body fence is likewise inert. Only a `<summary>` opening the body is the summary; any later one is body text (product behavior 14(f)). The walk sets the `DetailsSummary` variant from what it finds: no opening `<summary>` yields `Absent`, an opening tag with a matching `</summary>` yields `Closed`, and an opening tag whose `</summary>` never arrives yields `Unclosed` with the rest of the body as its run and an empty body (product behavior 14(g)).
- **The summary run is scanned as inline text, not blocks.** Between `<summary>` and its `</summary>` the walk looks only for `</summary>`: fence lines inside the summary do not toggle the in-fence flag, and nested `<details>` or `<summary>` tags there open nothing. This is what makes product behavior 14(i) hold — a fence in a summary is literal inline text rather than a region that swallows the rest of the body — and it is why the fence flag governs body lines only. The summary's own text is then parsed with the inline parser, which collapses its line breaks to a single line.
- **Recursive body parse**: `parse_markdown_internal` is applied to the delimited region with depth incremented in the parse state.
- **Nesting guard** (product behavior 13): `const MAX_DETAILS_DEPTH: usize = 64;`. On exceeding it, the branch returns `nom::Err::Error`, so the input deterministically falls through to `parse_paragraph` and renders as literal text. There is no panic path, and recursion is bounded by the constant. The now-unmatched `</details>` is later recognized at its own depth and renders as literal text under product behavior 14(b). Depth needs no backtracking care: it is a value in the parse state passed down by value, so it unwinds with the call.

  The constant is derived from the recursion's measured cost rather than chosen for product reasons. A recursive block parse of this shape costs **~176 bytes of stack per nesting level** (measured, debug profile, aarch64 macOS), and a direct experiment on an explicitly-sized 2 MiB stack — the smallest a spawned thread gets by default, and no thread in the workspace overrides the default — completed 8,000 levels and overflowed somewhere between 8,000 and 16,000. A full `parse_details` frame adds a second frame per level, which would roughly halve that headroom to an estimated ~4,000 levels; that figure is an estimate from the measured per-level cost, not a measurement of the built parser. A release build's frames are smaller, so a shipped build's headroom is at least as good. 64 therefore sits roughly two orders of magnitude below the conservative estimate of the failure point, while exceeding any realistic content — content in the wild rarely nests past 3, and a 64-level document renders every level.
- **Closed and unclosed regions** (product behaviors 1 and 14(a)): reaching the matching `</details>` sets `closed: true`; consuming to the end of the enclosing content without one sets `closed: false`, and the rest of that input becomes the body. Because parsing re-runs on streaming updates, a still-arriving block renders progressively (product behavior 20) — it parses as `closed: false` on every snapshot until its closing tag arrives, at which point the same input parses as `closed: true`. `compute_formatted_text_delta` compares lines positionally, so growth inside the details line changes only that line's own `num_lines` and content; the lines above it compare equal and keep their prefix identity. `closed` flipping is a change to that one line, which the delta already reports as such.
- **Block-level scope** (product behavior 17): the line-start rule means a `<details>` indented under a list item matches as its own block and therefore terminates the list rather than nesting inside it, and a blockquoted `> <details>` never matches and stays literal text. Both diverge from GitHub, which nests in each case; the product spec's "Divergences from GitHub rendering" states the divergence and the representation gap behind it.

### 3. Imported-HTML path

In `crates/markdown_parser/src/html_parser.rs`, handle `details` and `summary` elements explicitly, building the same `FormattedDetails` from the DOM that html5ever already provides. No new scanning is needed, and html5ever normalizes tag case and attribute presence, so product behaviors 4 and 5 hold there without extra work. `MAX_DETAILS_DEPTH` from §2 applies identically.

### 4. Editor buffer: a three-marker span, not a block style

Details regions are a new top-level `BufferText` variant modeled on `Link`, not a `BufferBlockStyle`. A `BufferBlockStyle` is per-character exclusive, so a code block inside a details region could not be both "a code block" and "inside details" if details were a block style. Links already solve exactly this shape by being an independent zero-width marker layer; details replicate that layer at block granularity.

**The markers.** Add `BufferText::Details(DetailsMarker)` alongside `Link` (`text.rs:555`):

```rust
pub enum DetailsMarker {
	Start { default_open: bool },
	EndSummary { closed: bool },
	End,
}
```

**Where each source shape lives in this model.** The three product-observable distinctions of §1 are carried by marker presence wherever the markers can express them, and by payload only where they cannot:

| Distinction | Buffer encoding |
|---|---|
| Region closed vs. unclosed (14(a)) | Marker presence: a `Start` with a matching `End` is closed; an unmatched `Start` is not |
| Summary absent vs. present (14(e)) | Marker presence: a region containing no `EndSummary` has no summary element |
| Summary closed vs. unclosed (14(g)) | `EndSummary { closed }` payload: both shapes place an `EndSummary`, so position alone cannot separate them |

The third row is the only one needing payload. An unclosed `<summary>` absorbs the body and leaves it empty (product behavior 14(g)), so the pairing scan places its `EndSummary` at the end of the region — the same marker sequence a closed summary produces over an empty body. `closed: false` is what makes `to_markdown` emit the opening `<summary>` with no closing tag for that region, and it is the buffer counterpart of `DetailsSummary::Unclosed` in §1. `DetailsSummary::Absent` and `Closed` need no flag, because the markers already separate them.

A region lays out as `Start` · summary text · `EndSummary` · body blocks · `End`:

```
<details:start default_open=true>This is the summary<details:end-summary closed=true>This is the body<details:end>
```

The summary lives inline in the buffer between `Start` and `EndSummary`, not as a `String` on the marker payload. The summary is formatted text — it can carry bold, inline code, and links (product behavior 3) — so a flat string would either lose that formatting or force re-parsing inline markdown on every render. As buffer text, its formatting rides on the same inline-style and link-marker machinery as any other run, and extraction is a seek or short scan from a `Start` to the nearest `EndSummary`.

Each of the three markers is zero-width in the char stream, like `LinkMarker`: added to the zero-width arm of the `Item::summary()` impl (`text.rs:1764`) and left in the skipped catch-all arm of the `Bytes` iterator (`text.rs:328`). None occupies a `\n`. Payload rides on the markers that need it, matching how `LinkMarker::Start` carries a URL; `End` carries none, since region closedness is already marker presence.

**Region and body.** A region is the range between a `Start` and its matching `End`. Body blocks are ordinary blocks, each keeping its own `BufferBlockStyle`. Nesting is nested `Start`/`End` pairs. Because details is orthogonal to block style, a renderer consults the `SumTree` for both "inside a details region, at what depth" and "what is this block's own style", and adjusts per-block styling independently.

**Depth through a SumTree counter.** Add `details_counter` and `total_details_marker` to `StyleSummary` (`text.rs:1551`) and a `DetailsDepth` dimension, mirroring `link_counter`/`total_link_marker` (`text.rs:1559`/`:1563`) and `LinkCount` (`text.rs:1741`). Only `Start` (+1) and `End` (−1) move the counter; `EndSummary` is depth-neutral, since it delimits a run inside an already-open region. The counter value at an offset is the nesting depth, so no depth field is stored on any marker and depth-at-offset is an O(log n) seek.

**Depth beyond the guard in the buffer.** Parsing enforces `MAX_DETAILS_DEPTH` (§2), but editing can nest markers deeper without going through the parser. **The buffer accepts every edit unconditionally.** Markdown is a plain-text format and the editor has no permission to reject a keystroke because of the structure it would produce; an editor that refuses an edit mid-typing makes typing order determine document validity, and it is a worse failure than rendering a deeply nested region as text. The guard therefore lives entirely at the read boundary: `to_formatted_text` emits a `Details` container only while the `DetailsDepth` at that offset is below `MAX_DETAILS_DEPTH`. At or beyond it, the markers convert to their literal characters — `Start` to `<details>` (with ` open` when `default_open`), `EndSummary` to `</summary>` when it carries `closed: true` and to nothing when it does not, `End` to `</details>` — inside ordinary paragraph text, which is the same output the parser produces for an over-depth region under product behavior 13. An `EndSummary { closed: false }` contributes no characters because its source had no `</summary>` to reproduce; that is the one marker-to-literal conversion that writes nothing, and it deletes no authored tag. A depth-65 region therefore renders and round-trips identically whether it arrived through the parser or through editing, and product behavior 21 holds for every edit sequence.

**Selection and copy.** The markers are zero-width and body blocks are ordinary blocks, so there is no offset mapping and no selection special-casing. Because the copy path does not consult `HiddenLinesModel` (Context), copying across a collapsed region yields its full body, satisfying product behavior 10. The regression test in Testing and validation is the enforcement for that property; the prose invariant alone is not a mechanism.

**Edit rebalancing** (product behavior 21). All of these fall out of the pairing scan rather than stored state, so no edit sequence yields an undefined document, exactly as unbalanced link markers degrade rather than corrupt. Each malformed shape resolves to the product behavior that governs the equivalent malformed markdown, and `to_markdown` emits exactly the literal markdown that behavior specifies — an unpaired marker standing for an authored tag is never dropped, because dropping it would delete a tag the user typed:

- An unmatched `Start` owns the range to its enclosing region's end, or to end of buffer at top level. `to_markdown` emits its `<details>` opening tag with no closing tag (product behavior 14(a)).
- An unmatched `End` opens no region and contributes no structure. `to_markdown` emits the literal characters `</details>` at its position (product behavior 14(b)).
- A `Start` whose region contains no `EndSummary` is a region with no summary: the whole run is body, and it renders with the substitute label `Details`. `to_markdown` emits `<details>` and the body with no `<summary>` element at all (product behavior 14(e)). This is the buffer shape of a summary-less `<details>`; behavior 14(g)'s unclosed `<summary>` is a different shape, carrying an `EndSummary { closed: false }` that the pairing scan places at the end of the region so the summary run absorbs the body and the body is empty.
- An `EndSummary` outside any open region contributes no structure. `to_markdown` emits the literal characters `</summary>` at its position (product behavior 14(h)) — the `closed` payload is read only for an `EndSummary` inside a region, since a stray one closes nothing.

Converting a block range to or from a details region goes through the existing `BufferEditAction` path (`set_block_style` at `crates/editor/src/model.rs:1167`, `convert_block` at `:1200`), extended to insert and remove the marker triple.

**Collapse.** Collapsed state reuses `HiddenLinesModel` (`hidden_lines_model.rs:20`): the body range is an anchor-pair hidden region, per editor and outside buffer content, initialized from `default_open`. Hiding is consulted only at render and navigation time (`render/model/mod.rs:861-869`/`:3332-3347`, `selection.rs:414-459`), never by copy or serialization, so toggling cannot dirty deltas or undo history (product behavior 9). Each region owns its own anchor pair, so a nested region's state is independent of its ancestors' and an ancestor's toggle hides it without touching it (product behavior 12). The model is created with the editor view and never persisted, so a closed-and-reopened document starts every region at its `default_open` state (product behavior 9).

**Collapse state across edits and streaming** (product behaviors 20 and 21). The anchor pairs are the mechanism for both rules, and neither needs new state. Anchors move with edits, so an edit inside a collapsed body leaves that body's pair spanning the edited text and the section stays collapsed; an edit to a boundary marker moves the pair with the region the rebalancing rules then define, and a pair whose region no longer exists is dropped when the pairing scan finds no region for it. Across streaming updates the model is keyed per editor and independent of buffer content, so re-parsing a growing document does not reconstruct it: the anchor pair placed when the region first appeared survives each snapshot, which is what keeps a user's toggle from reverting to `default_open` as content arrives. `default_open` is read only when a region's pair is first created, never on a later snapshot of the same region.

**Focus and caret in a collapsed body** (product behavior 11). Cursor navigation already consults `HiddenLinesModel` (`selection.rs:414-459`), which is the mechanism for moving the caret across rather than into a hidden range; a details region's body is an ordinary hidden range there, so it needs no new navigation code. Focus traversal is the half that does: the render element skips focus stops at offsets inside a hidden range, so links in a collapsed body are not tab-reachable. This is the one place where details deliberately diverges from copy, which reads through hidden ranges (product behavior 10) — the divergence is that a selection may legitimately span content the user cannot see, while a focus ring or caret resting there would be invisible.

**Round trip.** In `crates/editor/src/content/markdown.rs`, `to_markdown` (`:61`) emits `<details>` with ` open` iff `default_open` at `Start`, then `<summary>` plus the summary run's serialized inline markdown, then `</summary>` iff the `EndSummary` carries `closed: true`, then the body blocks through their normal arms, then `</details>` at the matching `End`. A region with no `EndSummary` emits no `<summary>` element (product behavior 14(e)). That is the paired case; an unpaired marker emits the literal markdown its governing product behavior specifies, per the rebalancing rules above. Every marker that stands for a tag the author typed produces that tag's characters, so a details tag cannot disappear across a save. The single marker that writes nothing is an `EndSummary { closed: false }`, which stands for a `</summary>` the author never typed; writing one there would fabricate a closing tag, which product behavior 14(g) forbids just as it forbids dropping an authored one.

`to_formatted_text` (`:416`) does the inverse, folding a `Start`/`EndSummary`/`End` triple and the blocks it spans into a `FormattedTextLine::Details`, recursively for nested pairs and subject to the depth boundary above. The two source-shape channels map across in both directions: a matching `End` is `closed: true` and its absence `closed: false`; no `EndSummary` is `DetailsSummary::Absent`, and an `EndSummary` is `Closed` or `Unclosed` per its payload. Because each distinction is representable on both sides, a markdown → IR → buffer → IR → markdown trip preserves every shape in product behavior 14 rather than canonicalizing a malformed region into a well-formed one. `reverse_core_edit_action` in `crates/editor/src/content/core.rs` gains a `Details` arm alongside the list arms (`:743`/`:782`/`:827`) that flattens the container into the marker triple with its summary run and body blocks.

**Render model.** The render `SumTree` gains a details dimension consulted per block (depth plus open state), plus a `render/element/details.rs` that paints the summary run and disclosure indicator above the first body block of a region. The summary is rendered from the formatted run between `Start` and `EndSummary`, never re-parsed from a flat string. Focus, click, Enter/Space toggle, and renderer-generated accessibility identifiers (product behaviors 6 and 18) live here.

Click precedence (product behavior 7) is a hit-test ordering rule in this element: a click on the summary row is offered to the link markers spanning that offset before the row's own toggle handler, and the toggle runs only if no link claims it. The summary's links are already `LinkMarker` spans in the buffer (§4), so the test is a `LinkCount` lookup at the clicked offset rather than new state. Keyboard activation does not go through hit-testing and is unaffected: the section's toggle is bound to its own focus stop, and a link in the summary keeps the focus stop it would have anywhere else.

Drag precedence (product behavior 8) is the ordering in time rather than in depth: the toggle fires on release, not on press, and only when the press and release are on the same row within the surface's existing drag threshold. A press that exceeds the threshold becomes a selection drag through the normal selection path and the toggle never runs. Binding the toggle to release is what keeps summary text selectable, and it is the same press-versus-drag arbitration every other selectable clickable row in the editor already performs.

### 5. Render surfaces

The expanded static fallback is implemented once, at the sinks all consumers render through, rather than as a per-surface obligation.

1. **`FormattedTextElement`** (`crates/warpui_core/src/elements/gui/formatted_text_element.rs:145`) gains layout for the `Details` variant: summary row, then body lines, expanded, with no disclosure indicator — the product behavior 19 fallback. This covers modals, banners, changelog entries, and settings pages. The interactive widget for the agent conversation surface is a dedicated opt-in builder method on this element, consistent with its existing `with_*`/`disable_mouse_interaction` API (`:521`), and is enabled only by the agent conversation path (`app/src/ai/agent/util.rs:35`/`:189`). The agent surface does not reuse the editor's folding machinery; Tradeoffs records why it cannot.
2. **The editor buffer conversion** (`crates/editor/src/content/markdown.rs`) gives buffer-backed consumers the §4 model. Because §4 models details natively in the buffer with `HiddenLinesModel` folding, the notebook/plan editor — the view behind the `RichTextEditorView` identifier used throughout `app/src/notebooks/editor/view.rs` — is interactive, which is the second interactive surface product behavior 6 commits to.
3. **The TUI** (`crates/warp_tui/src/tui_markdown.rs`) is a third consumer and an explicit non-goal for interactivity. The per-line match in `render_formatted_text` (`:85-141`) gains a `Details` arm rendering the summary row followed by the expanded body — the product behavior 19 fallback in the terminal renderer. `should_insert_blank_row` (`:152-190`) gains an explicit `Details` arm grouped with the structural multi-line blocks (`Heading`, `CodeBlock`, `Table`, `Image`), which currently emit a following blank row unless the next line is a `LineBreak` or `HorizontalRule`.

**The compiler guards one of these two sites, not both.** Adding a variant to `FormattedTextLine` breaks the build at `render_formatted_text` (`tui_markdown.rs:85-141`) and at `reverse_core_edit_action` (`content/core.rs`, arms `:571`-`:891`), which are exhaustive. It does **not** break `should_insert_blank_row`, whose `_ => false` arm (`tui_markdown.rs:189`) would silently give a details container no trailing blank row while every comparable block gets one. That site is listed above as required work and is covered by a dedicated test, because nothing else will catch it. Sites that construct values or match a single variant, such as link extraction, need no change; §1's `hyperlinks` arm is the other silent-default site and is specified there for the same reason.

### Tradeoffs considered

- **A table-style internal code block in the buffer** (rejected): the table mechanism is an intermediate implementation kept to leave table editing options open, and it makes the block multi-line, which is what forces per-cell offset mapping onto selection. The §4 span model keeps selection trivial and models the region natively.
- **Details as a `BufferBlockStyle`** (rejected): a block style is per-character exclusive, so details content could not simultaneously carry its own block style. A top-level zero-width span marker makes the two orthogonal.
- **The summary as a `String` on the `Start` payload** (rejected): the summary is formatted text, so a flat string loses inline styling or forces a re-parse on every render. The `EndSummary` marker keeps it as ordinary buffer text at no cost to the depth counter.
- **Start and end marker lines in the parser IR** (rejected at that layer only): IR consumers are stateless per-line renderers with no machinery to maintain balance, so the IR keeps container-as-line. The buffer, which does maintain paired zero-width markers and owns edit semantics, uses the paired representation. This is the §1/§4 split.
- **Rejecting over-deep edits in the buffer** (rejected): blocking an edit because it would nest past `MAX_DETAILS_DEPTH` makes typing order determine document validity and conflicts with product behavior 21. Markdown is a plain-text format and the editor has no permission to refuse a keystroke over the structure it would produce. Applying the guard at the read boundary instead keeps every edit legal and every render defined.
- **Verbatim body-source preservation** (rejected): the rich-text pipeline guarantees canonical re-serialization, not byte-exact preservation, anywhere. Dropping the verbatim field removes duplicated body storage and its equality subtleties.
- **A fully tree-shaped `FormattedText`** (rejected): the correct long-term model, but a cross-cutting rewrite of parser, delta, and editor mapping, far beyond this feature's blast radius.
- **An iterative details parse with an explicit stack** (rejected for this change): a pre-pass that scans the document once, resolves nesting on an explicit stack, and hands only leaf text segments to `parse_markdown_internal`. It removes `MAX_DETAILS_DEPTH` entirely, matches GitHub's unbounded nesting, and touches no existing parser signature. It is rejected here on defect risk rather than size: reassembling a region's body from alternating text runs and completed child regions requires offset bookkeeping and splicing that is the most defect-prone code in either design, and it buys safety the §2 measurement shows is not at risk — the recursion tolerates thousands of levels against a guard of 64. It remains the natural refactor if unbounded nesting is later wanted.
- **A per-document cap on rendered disclosure widgets** (rejected): a count cap defends a cost the render architecture already avoids. Renderable elements are built per visible item through the viewport iterator (`crates/editor/src/render/model/mod.rs:260`), so off-screen widgets construct no elements and the per-widget accessibility and event bookkeeping scales with viewport size rather than document length. No comparable element is capped per document — link markers are counted in the `SumTree` as `total_link_marker` (`text.rs:1563`) with no maximum, and code blocks, list items, and table cells have none either — so a details-specific cap would be the only one in the codebase, and it would silently render a legitimate document's later sections as literal text.
- **Reusing the editor's folding mechanism on the agent conversation surface** (rejected): `HiddenLinesModel` (`hidden_lines_model.rs:20`) is per-editor and anchor-based, and its render element `RenderableHiddenSection` is constructed from a `WeakViewHandle<V: EditorView>` and a `ViewportItem` (`crates/editor/src/render/element/hidden_section.rs:40-47`). The agent conversation path renders through `FormattedTextElement` with no editor view to hold either, so the mechanism is not available there. §5's dedicated opt-in builder is the disclosure surface for that path; the editor keeps folding through `HiddenLinesModel` (§4).

## Testing and validation

### Parser tests

In `crates/markdown_parser/src/markdown_parser_tests.rs` and `html_parser_tests.rs`, mapped to product behaviors:

| Behavior | Test |
|---|---|
| 1, 2 | `<details>` with and without `open` parses to `Details` with the correct `default_open`; a body containing a code block, list, and table parses recursively |
| 3 | a summary carrying bold, inline code, and a link parses to inline fragments with styling preserved; `hyperlinks` returns links from both the summary and the body, with body offsets shifted past the summary |
| 4 | `open`, `open=""`, and `open="false"` all yield `default_open == true`; absent `open` yields false; unknown attributes do not change the parse |
| 5 | `<DETAILS>`, `<Summary>`, and `<details >` parse identically to their canonical lowercase spellings |
| 12, 13 | nested `<details>` at depth ≤ 64 parses as nested `Details`; depth 64 parses as a container whose `raw_text` is its summary and body with no tag characters, while depth 65 falls back to a paragraph whose `raw_text` contains the literal `<details>` and `</details>` characters plus the same summary and body text; the orphaned `</details>` renders as literal text |
| 14(a) | an unclosed `<details>` consumes to end of input and parses with `closed: false`, while the same input with a closing tag parses with `closed: true`; an unclosed nested one ends at its parent's close |
| 14(b), 14(h) | a stray `</details>` and a stray `</summary>` render as literal text, and each survives a parse → `to_markdown` → parse round trip with its tag characters intact |
| 14(c) | a mid-line `<details>` stays plain text; a line that is exactly `` `<details>` `` in backticks stays plain text |
| 14(d) | a self-closing `<details/>` opens a region that runs to end of input |
| 14(e) | a body with no `<summary>` parses to `DetailsSummary::Absent` and renders the literal `Details` label, and a `<summary>Details</summary>` in the source parses to `DetailsSummary::Closed` instead — the two are distinguishable, and only the second serializes a `<summary>` element |
| 14(f) | a `<summary>` after body content renders as literal text; with two leading `<summary>` elements the first is the summary and the second is literal body text |
| 14(g) | an unclosed `<summary>` parses to `DetailsSummary::Unclosed`, consuming the rest of the body as summary and leaving the body empty, and `to_markdown` re-emits the opening `<summary>` with no closing tag so a re-parse yields the same structure; a closed `<summary>` over an empty body parses to `Closed` and re-emits `</summary>`, so the two are not conflated; an unclosed `<summary>` inside an unclosed `<details>` consumes the remainder of the document into a single summary row |
| 14(i) | a code fence, a nested `<details>`, and a nested `<summary>` inside a summary render as literal inline text; a multi-line summary collapses to one line |
| 15 | `<details>`, `</details>`, and `<summary>` inside a fenced code block in the body leave the balance unchanged and appear verbatim in the rendered code block |
| 16 | a code fence opened in the body and never closed swallows a later `</details>`, so the region runs to end of input |
| 17(a) | a `<details>` indented under a list item terminates the list and parses as its own `Details` block, with a following list item starting a new list |
| 17(b) | a `<details>` inside a blockquote parses as literal text, as does its `</details>` |
| 20 | `compute_formatted_text_delta` over successive streaming snapshots keeps `common_prefix_lines` stable for lines above the block, including as the block's own `num_lines` grows |
| 22 | a corpus of existing markdown fixtures containing neither tag parses identically to the pre-change output |

Round-trip coverage (canonical, not byte-exact): parse → buffer → `to_markdown` → parse yields an equal `FormattedText` by structural equality, with `open` preserved, over a nested region and a body containing code fences. The same round trip runs over each malformed shape in product behavior 14 — stray `</details>`, stray `</summary>`, unclosed `<details>`, unclosed `<summary>`, summary-less `<details>` — asserting for each that the output markdown carries the same multiset of details and summary tags as the input, which is the mechanism enforcing that no degradation path deletes a tag or fabricates one. Because the multiset distinguishes an opening tag from a closing one, an implementation that "balanced" an unclosed region by appending `</details>`, or that materialized `<summary>Details</summary>` for a summary-less region, fails the assertion rather than passing a bare count check.

Source-shape coverage runs alongside it: each of the five shapes parses to its distinct `closed`/`DetailsSummary` combination, and each pair of shapes that shares rendered text — summary-less versus `<summary>Details</summary>`, closed versus unclosed region, closed versus unclosed summary over an empty body — is asserted to produce different markdown output. This is what would fail against an IR that cannot tell the members of a pair apart.

### Editor buffer tests

In `crates/editor`, covering the §4 model:

| Behavior | Test |
|---|---|
| pairing | flattening `FormattedTextLine::Details` produces `Start` + summary run + `EndSummary` + body blocks + `End`; nested pairs increment the counter; `EndSummary` is depth-neutral; `DetailsDepth` at an offset matches a linear pairing scan |
| summary formatting (3) | a summary carrying bold, inline code, and a link round-trips through the buffer with formatting intact, tracked by the normal inline-style machinery rather than flattened; extraction is a seek from `Start` to `EndSummary` |
| orthogonality | a code block and a list inside a region keep their own `BufferBlockStyle` while the details counter is nonzero over their range |
| zero-width | all three markers contribute 0 chars, bytes, and lines to the text summary and emit nothing from the `Bytes` iterator; a region's only characters are its summary run and body |
| depth boundary (13, 21) | markers edited to depth 65 are accepted by the buffer with no edit rejected, and `to_formatted_text` emits the depth-65 region as paragraph text containing the literal `<details>`/`</details>` characters rather than a container, byte-identical to the parser's output for the same depth-65 markdown |
| rebalancing (21) | deleting `End` extends the region to its enclosing region's end or to end of buffer; an orphaned `End` serializes as literal `</details>` and an orphaned `EndSummary` as literal `</summary>`, neither dropped; a `Start` whose region has no `EndSummary` yields a summary-less region that serializes with no `<summary>` element and renders with the `Details` label; results agree across edit orders |
| copy (10) | selecting across a region boundary and copying yields summary plus full body with no offset drift; copying a **collapsed** region still yields the full body, regression-guarding that copy ignores `HiddenLinesModel` |
| toggling (9) | fold and unfold leave buffer content, dirty state, and the undo stack unchanged |
| nested state (12) | collapsing an outer region leaves each nested region's own hidden-range entry unchanged, and expanding the outer restores each nested region to the state it held before, including one toggled while its ancestor was collapsed |
| caret and focus (11) | arrow-key and click-to-place caret movement across a collapsed region lands outside it rather than inside; tab traversal reaches no focus stop inside a collapsed body, including a link in it; expanding restores both |
| collapse across edits (21) | an edit inside a collapsed body leaves it collapsed; deleting a boundary marker carries the collapse to the region the rebalancing rules define and drops it when no region remains; no edit resets a region to `default_open` |
| collapse across streaming (20) | over successive streaming snapshots of a growing document, a region the test toggles keeps that state on every later snapshot, including the one where its closing tag first arrives |
| editing | `convert_block` to and from a details region inserts and removes the marker triple and preserves body blocks |

### Renderer and surface tests

- TUI (19): `render_formatted_text` over a collapsed-by-default region emits the summary row and the full body with no disclosure indicator, and nested regions render fully expanded. A separate test asserts `should_insert_blank_row` returns the structural-block result for a `Details` line, since the wildcard arm at `tui_markdown.rs:189` means the compiler cannot catch a missing arm here.
- Static GUI surfaces (19): `FormattedTextElement` without the interactive builder option lays out summary plus expanded body and produces no focusable toggle.
- Interactive surfaces (6, 7, 18): renderer-level tests in the block renderer's existing harness cover click toggle, Enter and Space toggle, focus order, and that the control-association attributes use renderer-generated identifiers rather than input-supplied ones. The assistive-technology half of product behavior 18 — that the control announces its expanded state — is verified manually below, since the existing harness asserts on the render tree rather than on a platform accessibility API.
- Click precedence (7): a click inside the bounds of a link in the summary activates the link and leaves the section's expanded state unchanged; a click on the same row outside those bounds toggles and activates no link; with the section focused, Enter and Space toggle whether or not the summary carries a link.
- Drag precedence (8): a press on the summary row followed by movement past the drag threshold selects summary text and leaves the expanded state unchanged; a press and release within the threshold on the same row toggles; a press on the row released outside it does not toggle.

### Manual proof for the implementation PR

Per CONTRIBUTING, the implementation PR attaches a screen recording of toggling by mouse and by keyboard in both the agent conversation view and the notebook/plan editor, a VoiceOver pass announcing the disclosure control and its expanded and collapsed states, and before/after screenshots of the issue's snippet.

## Risks and mitigations

### One consumer site is a silent default rather than a compile error

`should_insert_blank_row` (`crates/warp_tui/src/tui_markdown.rs:152-190`) ends in `_ => false` at `:189`, so a `Details` variant compiles into the no-blank-row default and the TUI renders details containers with spacing that differs from every comparable block. `hyperlinks` (`crates/markdown_parser/src/lib.rs:261`) has the same shape through `inline_fragments`, where the default silently drops body links. Both are specified as required work (§5, §1) and both have dedicated tests, because the build will not flag either. The compiler does guard `render_formatted_text` and `reverse_core_edit_action`.

### Collapse state is per view, and the buffer is shared

`HiddenLinesModel` is per editor, so the same document open in two panes can show different collapse states. This matches existing hidden-section behavior for code folding and is consistent with collapse being view state (product behavior 9): reopening a document restores every section to its default state.

### Fence tracking is duplicated between the block chain and the details delimiter

The delimiter in §2 re-implements the fence recognition `parse_code_block`/`parse_closing_fence` perform, including their indentation matching. Factoring out a shared line predicate is preferable but is not free: the two call sites consume input differently, and any behavior change to the shared helper alters existing code-block parsing for every document, not just documents containing details regions. The mitigation is to share the fence-open and fence-close predicates only, keep consumption local to each caller, and cover drift directly with the product behavior 15 and 16 tests, which fail if the two paths disagree about where a fence begins or ends.
