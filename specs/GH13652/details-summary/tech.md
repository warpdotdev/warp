# TECH.md — Markdown viewer: `<details>/<summary>` collapsible sections

Product spec: `specs/GH13652/details-summary/product.md`
GitHub issue: https://github.com/warpdotdev/warp/issues/13652
Preceding spec in the chain: `specs/GH13652/` — `<img>` sizing (PR #13656).

## Context

The Markdown viewer parses via `crates/markdown_parser` (a custom `nom` parser) into
`FormattedText` = a flat sequence of `FormattedTextLine` variants
(`crates/markdown_parser/src/lib.rs:155-168`). The editor (`crates/editor`, package
`warp_editor`) converts these to `BufferBlockItem` / `StyledBufferBlock`, lays them out
in `crates/editor/src/content/edit.rs`, into `BlockItem`s
(`crates/editor/src/render/model/mod.rs:~1172-1226`), drawn by elements under
`crates/editor/src/render/element/`. Raw HTML other than inline `<u>`/`</u>` is currently
dropped to literal text. The full-HTML paste parser
(`crates/markdown_parser/src/html_parser.rs`, html5ever) is **not** wired into the block
grammar.

Unlike `<img>` (a single leaf block), `<details>` needs three things the current
pipeline does not have as a unit:

1. **A body that owns an arbitrary run of block content.**
2. **A user-toggleable collapsed/expanded state that persists across re-layout.**
3. **A clickable disclosure affordance inside a rendered block.**

Reconnaissance of the codebase found precedent for (2) and (3), but a real gap for (1):

### What already exists (reusable)

- **Per-block toggle state keyed by anchor→offset — the mermaid precedent.** A Mermaid
  code block renders as either a diagram or a raw-code fallback, gated on
  `is_mermaid && (render_mermaid_diagrams || is_user_rendered)` where `is_user_rendered`
  reads `layout_options.mermaid_render_offsets.contains(&block_start)`
  (`crates/editor/src/content/edit.rs:761-763`). The offset set lives on
  `RenderLayoutOptions` (`crates/editor/src/render/model/mod.rs:192-195`) with a setter
  at `:~2375`. The source of truth is a per-block child model field
  (`NotebookCommand.mermaid_display_mode`), and `sync_mermaid_render_offsets`
  (`app/src/notebooks/editor/model.rs:397`) rebuilds the `HashSet<CharOffset>` from those
  models, resolving each block's `start_offset` via a **buffer anchor** so it survives
  edits. Click wiring: a footer icon button dispatches
  `EditorViewAction::MermaidDisplayModeSelected { start_anchor, mode }`
  (`app/src/notebooks/editor/notebook_command.rs:685-715`) →
  `view.rs:3064` resolves the anchor → `model.set_mermaid_render_mode(offset, mode)`
  (`model.rs:380`) → `sync_*` → `rebuild_layout`.
- **An in-editor collapse-with-caret affordance — the hidden-section precedent.**
  `crates/editor/src/render/element/hidden_section.rs` renders a collapsed bar
  ("N unmodified lines / Expand all lines") as a `RenderableBlock` built from a
  `MouseStateHandle` + `Hoverable` with `.on_click(...)` dispatching
  `hidden_section_clicked` (`app/src/code/editor/view/actions.rs:~1314`). Its backing
  variant is `BlockItem::Hidden(HiddenBlockConfig)`
  (`crates/editor/src/render/model/mod.rs:~1225`), emitted during layout at
  `edit.rs:~980` and `:~1173`. The "what is hidden" state is a model-level
  `RangeSet<LineCount>` in `HiddenLinesModel`
  (`crates/editor/src/content/hidden_lines_model.rs`), and `set_hidden_lines(...)` drives
  re-layout. This is the closest precedent for "hide a range of lines behind a
  clickable affordance."
- **Chevron icons** (`Icon::ChevronDown` / `Icon::ChevronRight`) used throughout for
  disclosure carets.

### What is missing (the crux)

- **No block owns child blocks.** `FormattedTextLine`, `BufferBlockItem`, and `BlockItem`
  are all single-leaf; none holds a `Vec<…>` sub-sequence. `FormattedTextLine::Embedded(Mapping)`
  is **not** a general nested container — it carries a YAML mapping converted to a single
  self-contained widget (`EmbeddedWorkflow`, `app/src/notebooks/editor/embedded_item.rs:219`),
  with no notion of laying out child Markdown blocks. So a `<details>` body has no native
  representation in the parse→buffer→layout→render pipeline today.

The consequence: a *true nested-container* `<details>` (a block that owns and lays out a
sub-document) is net-new architecture. The pragmatic MVP below **avoids** building that by
treating the body as ordinary top-level blocks whose visibility is gated by a hidden-line
range — reusing the `HiddenLinesModel` / `BlockItem::Hidden` mechanism. This is the key
design decision and its tradeoffs are spelled out under "Design options" and "Risks."

Relevant code:

- `crates/markdown_parser/src/markdown_parser.rs:138-182` — block `alt(( … ))` chain.
- `crates/markdown_parser/src/markdown_parser.rs:1626-1646` — the existing `<u>`/`</u>`
  inline-HTML tag recognition (nearest precedent for tag matching).
- `crates/markdown_parser/src/lib.rs:155-168` — `FormattedTextLine` variants.
- `crates/editor/src/content/edit.rs:747-840` — mermaid two-state layout branch (toggle
  precedent) and `BlockItem::Hidden` emission at `:~980`, `:~1173`.
- `crates/editor/src/content/hidden_lines_model.rs` — hide-range model.
- `crates/editor/src/render/model/mod.rs:192-195` — `RenderLayoutOptions`; `:~1172-1226`
  `BlockItem`; `:~2375` mermaid offset setter.
- `crates/editor/src/render/element/hidden_section.rs` — caret/click affordance.
- `app/src/notebooks/editor/model.rs:380,397,417` — mermaid state sync + `rebuild_layout`.
- `app/src/notebooks/editor/view.rs:3064` — anchor→offset action handler.
- `app/src/notebooks/editor/notebook_command.rs:618,685-715` — footer toggle affordance.

## Design options

**Option A — hidden-range MVP (recommended for the first PR).** Parse `<details>` into a
pair of marker blocks (a summary/disclosure block and an end marker) plus the body left as
ordinary top-level blocks. When collapsed, add the body's line range to a per-section
hide set (reusing `HiddenLinesModel`-style range suppression) so the body blocks are not
laid out. The disclosure block renders the caret + summary and toggles the hide range on
click, mirroring the mermaid anchor→offset→`rebuild_layout` chain.

- Pros: reuses two mature mechanisms (hidden-line ranges + mermaid-style toggle state),
  no new nested-container architecture, body blocks render exactly as top-level ones.
- Cons: the body must be parseable as ordinary top-level Markdown; **nested `<details>`
  and arbitrary raw HTML in the body are not supported** (product invariant 12). The
  summary and body are sibling top-level blocks tied together by a section id, not a true
  parent/child, so boundary bookkeeping (where the body starts/ends) must be tracked
  carefully.

**Option B — true nested container.** Add a `FormattedTextLine::Details { summary,
body: Vec<FormattedTextLine>, open }` and a matching `BlockItem` that owns and lays out
child blocks. This is the "correct" model but is net-new architecture across the whole
pipeline (parse, buffer, layout, render, selection, serialization) and is a much larger
change — likely its own multi-PR effort.

**Recommendation:** ship Option A as the tier-zero `<details>` PR, explicitly bounding
nested/raw-HTML-body cases as limitations, and leave Option B as a documented follow-up if
maintainers want full nesting. The rest of this spec describes Option A. **This choice is
called out for maintainer review** — if the team prefers the nested-container model up
front, this spec becomes a design-only spec and the implementation is materially larger.

## Proposed changes (Option A)

### 1. Parser: recognize a `<details>` region

In `crates/markdown_parser/src/markdown_parser.rs`, add block-level recognition for
`<details …>`, an optional immediately-following `<summary>…</summary>`, and the matching
`</details>`, following the framing of the existing image block parser (block-leading
spaces, own-line, `tag_no_case` for case-insensitive tag names as `<u>` handling already
uses).

Emit new marker variants on `FormattedTextLine` (in `crates/markdown_parser/src/lib.rs`):

- `DetailsStart { summary: FormattedTextInline, open: bool }` — carries the parsed summary
  inline content (empty → default label at render time) and whether the `open` attribute
  was present.
- `DetailsEnd` — the closing marker.

The body between start and end is **not** wrapped; it is parsed by the normal block loop
into ordinary `FormattedTextLine`s that sit between the two markers. This is what lets the
body reuse all existing block rendering (Option A).

`raw_text` / `num_lines` for the markers: `DetailsStart` contributes its summary line (1
line); `DetailsEnd` contributes 0 lines (like `LineBreak`). Update `lib.rs` raw-text /
line-count / set-weight match arms accordingly (every `match` over `FormattedTextLine`
must gain the two arms — the style guide prefers exhaustive matches over `_`).

Fallback (product invariant 10): if `</details>` is never found, the `<details>` start is
re-emitted as literal text (the parser backtracks and the line is parsed by
`parse_paragraph`), so the remainder of the document renders normally.

Add parser tests (see Testing).

### 2. Content model: carry the markers into the buffer

Add `BufferBlockItem::DetailsStart { summary, open }` and `BufferBlockItem::DetailsEnd`
(`crates/editor/src/content/text.rs`), mirroring the `Image` addition pattern from the
`<img>` spec: extend the enum, its manual `PartialEq`, `as_markdown` (re-serialize to
`<details [open]>` / `<summary>…</summary>` / `</details>`), and `to_formatted_text_line`.
Convert from `FormattedTextLine::DetailsStart/DetailsEnd` in
`crates/editor/src/content/core.rs` alongside the existing `FormattedTextLine::Image` arm.

### 3. Section identity + expanded-state model

Introduce a per-section expanded/collapsed state that survives re-layout, mirroring the
mermaid mechanism:

- Each `DetailsStart` gets a **buffer anchor** at its start offset (anchors survive edits,
  as mermaid already relies on).
- A child-model field records each section's expanded flag (default from `open`). Add a
  `sync_details_expanded_offsets` that rebuilds a `HashSet<CharOffset>` of *collapsed*
  section start offsets (or expanded — pick one convention and document it), resolving
  anchors to current offsets. Store it on `RenderLayoutOptions`
  (`render/model/mod.rs:192`) next to `mermaid_render_offsets`, with a setter mirroring
  `set_mermaid_render_offsets` (`:~2375`) that reports whether it changed so the caller
  can `rebuild_layout`.

### 4. Layout: gate the body and emit the disclosure block

In `crates/editor/src/content/edit.rs`:

- When laying out a `DetailsStart`, emit a new disclosure `BlockItem::DetailsSummary`
  (new variant in `render/model/mod.rs`) carrying: the summary inline text, the caret
  state (expanded/collapsed from the offset set), a `MouseStateHandle`, and the section's
  `start_anchor` for the click action.
- Track the body's line range between `DetailsStart` and `DetailsEnd`. When the section is
  collapsed, suppress that range from layout by reusing the hidden-lines mechanism
  (`HiddenLinesModel` / the `BlockItem::Hidden` emission path at `edit.rs:~980,~1173`),
  or by simply skipping emission of the body blocks for a collapsed section. Reusing
  `HiddenLinesModel` is preferred so selection/scrolling already handle the hidden range
  consistently — but skipping emission is simpler if the hidden-lines model proves
  awkward to drive from here; the tech decision is called out for review.
- `DetailsEnd` lays out to nothing (or a small bottom spacer).

### 5. Render element + click wiring

- Add `crates/editor/src/render/element/details_summary.rs` (register in
  `element/mod.rs`) drawing the caret (`Icon::ChevronRight` collapsed /
  `Icon::ChevronDown` expanded) + summary inline text as a `RenderableBlock`, with a
  `Hoverable` + `MouseStateHandle` + `.on_click` dispatching a new
  `EditorViewAction::DetailsToggled { start_anchor }` (add to the action enum at
  `app/src/notebooks/editor/view.rs:~872`). This mirrors `hidden_section.rs` and the
  mermaid footer button.
- Handle the action in `app/src/notebooks/editor/view.rs` (mirroring the
  `MermaidDisplayModeSelected` handler at `:3064`): resolve `start_anchor` → offset →
  `model.set_details_expanded(offset, !current)` → `sync_details_expanded_offsets` →
  `set_details_expanded_offsets` on render state → `rebuild_layout`.
- If the plain file/code viewer (not the notebook editor) also renders Markdown and needs
  its own action impl, add one mirroring `hidden_section_clicked`
  (`app/src/code/editor/view/actions.rs:~1314`).

### 6. Serialization / round-trip

`BufferBlockItem::DetailsStart::as_markdown` emits `<details>` or `<details open>` then,
if a summary is present, `<summary>…</summary>`; `DetailsEnd::as_markdown` emits
`</details>`. Body blocks serialize as their existing Markdown. This preserves the section
on copy/export (product invariant 13). Decide and document whether serialization reflects
live expanded state (`open` when currently expanded) or the source default; recommended:
reflect live state so copy matches what the user sees.

### 7. Security

The parser reads only structural markers and the `open` attribute; all other attributes on
`<details>`/`<summary>` are ignored (product invariant 11). No attribute is executed or
navigated to. Body content is parsed as ordinary Markdown blocks and inherits the viewer's
existing trust boundary; no new asset/source path is introduced here.

## Testing and validation

### Parser unit tests (`crates/markdown_parser/src/markdown_parser_tests.rs`)

- `<details><summary>S</summary>body</details>` on own lines → `DetailsStart{summary:S,
  open:false}`, a body `Line`, `DetailsEnd` (invariants 1, 2, 5).
- `<details open>` → `open:true` (invariant 6).
- `<details>` with no `<summary>` → `DetailsStart` with empty summary (invariant 8).
- Empty `<details></details>` → start+end, no body (invariant 9).
- Unterminated `<details>` (no closing tag) → falls back to literal text; document below
  still parses (invariant 10).
- Nested `<details>` inside body → inner one is literal text / non-interactive; outer
  boundaries and following document intact (invariant 12).
- Ignored attributes: `<details class="x" onclick="y()">` → only `open` considered
  (invariant 11).

### Buffer round-trip (`crates/editor/src/content/text_tests.rs`)

- `<details><summary>S</summary>…</details>` survives markdown → buffer → markdown
  (invariant 13), including the `open` attribute reflecting expanded state per the chosen
  convention.

### Layout / interaction tests (`crates/editor/src/render/model/mod_tests.rs`,
notebook editor tests)

- Collapsed section: body line range is suppressed from layout; expanded: body blocks are
  present (invariants 4, 5).
- Toggling section A's expanded offset does not change section B (invariant 7).
- Caret state matches expanded/collapsed (invariant 3).
- Default collapsed unless `open` (invariant 6).

### Integration / manual

Per CONTRIBUTING, before/after screenshots + a short screen recording of clicking a
`<summary>` to expand/collapse a real README-style `<details>` block (the issue's own
example), plus a `<details open>` starting expanded. Add `crates/integration/` coverage
for opening a Markdown file with a `<details>` and toggling it if the viewer flow is
exercisable there.

## Risks and follow-ups

- **This is the architecturally heaviest tier-zero tag.** The honest feasibility signal:
  per-block toggle state and the caret affordance have clean precedents (mermaid,
  hidden-section), but **there is no existing block that owns child blocks**, so the MVP
  deliberately models the body as sibling top-level blocks gated by a hidden range rather
  than a true nested container. This bounds the feature: nested `<details>` and arbitrary
  raw HTML inside the body are out of scope for the MVP (product invariant 12).
- **Boundary bookkeeping is the main implementation risk.** Because summary and body are
  siblings tied by a section id rather than parent/child, edits that split/merge the
  region, or a body that itself contains block constructs the parser handles specially,
  need careful range tracking. This is the part most likely to grow; if it starts to
  sprawl, the nested-container model (Option B) may actually be cleaner and should be
  reconsidered with maintainers before pressing on.
- **Follow-up:** true nested `<details>` and a general nested-block container (Option B)
  would be a separate, larger effort — possibly worth it if collapsible sections prove
  popular, but explicitly deferred here.
- **Interaction with the tables spec:** once tables land, a `<details>` body containing a
  `<table>` should "just work" under Option A because the body is ordinary top-level
  blocks — but this should be verified once both land.
