# TECH.md — Markdown viewer: raw-HTML `<sub>`/`<sup>` support

Product spec: `specs/GH13734/product.md`
GitHub issue: https://github.com/warpdotdev/warp/issues/13734

## Context

Unlike the sibling `<table>` (#13652/tables) and `<img>` splits — where the render path
already existed and the work was wiring a new input format into it — `<sub>`/`<sup>` needs
a capability that **does not exist anywhere in the stack today**: a per-run vertical
baseline offset (and ideally per-run font size). This spec traces that gap from the parser
down to the platform text shaper and proposes a phased implementation.

### Parser: no sub/sup handling exists

- `PHRASING_ELEMENT_TAGS` (`crates/markdown_parser/src/html_parser.rs:26-28`) — the paste
  parser's allow-list of recognized inline tags — is `span, i, code, strong, em, br, a, s,
  u, ins`. Neither `sub` nor `sup` appears.
- The styling match arm that maps a recognized tag name to a `Styling` mutation
  (`html_parser.rs:441-450`, inside `parse_phrasing_content`) has an explicit `_ => ()`
  fallthrough for unknown tags, with an adjacent TODO: *"We need to add more phrasing
  styling we support (e.g. links) here. https://linear.app/warpdotdev/issue/CLD-335/..."*
  — confirming this is a known, general gap rather than sub/sup being deliberately
  excluded.
- `crates/markdown_parser/src/markdown_parser.rs` (the file-viewer's own inline tokenizer,
  independent of the paste-path HTML parser) has no `sub`/`sup` handling either. Its only
  HTML-tag-flavored delimiter pair is `<u>`/`</u>` (underline): `parse_inline_token_
  underline_start`/`_end` (`:1625-1642`) recognize the literal tags `"<u>"`/`"</u>"` as
  `DelimiterKind::UnderlineStart`/`UnderlineEnd` tokens, and `parse_underline`
  (`:1274-1306`) resolves a matched pair by calling `state.backtrack_styles(...,  |styles|
  styles.underline = true)` — i.e. the underline delimiter pair, once matched, sets a
  boolean flag on every fragment's `FormattedTextStyles` in its span. `DelimiterKind`
  (`:1807-1813`) is a fixed enum (`Asterisk, Underscore, LinkStart, Strikethrough,
  UnderlineStart`) with hardcoded arms in `valid_count`/`as_str`/flanking logic — adding
  `SubStart`/`SupStart` here is mechanical and directly mirrors the `UnderlineStart` case.

### Data model: `FormattedTextStyles` has no vertical-offset or font-scale field

`crates/markdown_parser/src/lib.rs:545-552`:

```rust
pub struct FormattedTextStyles {
    pub weight: Option<CustomWeight>,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
    pub inline_code: bool,
    pub hyperlink: Option<Hyperlink>,
}
```

This is the shared per-fragment style struct threaded from parser through to the render
layer. There is no `baseline_offset`, `font_scale`, or `vertical_align`-equivalent field.
Adding sub/sup support means extending this struct (analogous to how `underline: bool` is
a flag consumed downstream) — but, critically, **extending the struct is necessary but not
sufficient**, because the fields on this struct don't currently reach anywhere in the
render pipeline that can act on a vertical offset (see next section).

### Render/paint path: font size and baseline are per-*line*, not per-run

This is the load-bearing finding. Tracing from `FormattedTextStyles` down to the glyphs
that actually get painted:

1. **`crates/editor/src/render/layout.rs`** (`markdown_inline_to_text_and_style_runs`,
   `:246-310` — test-only today, but structurally representative of the real conversion
   path) turns each `FormattedTextFragment` into a `(Range<usize>, StyleAndFont)` style
   run. `properties` (weight/italic) and `TextStyle` (color/underline/strikethrough/
   background) are set per-fragment here. There is **no per-fragment font-size or
   baseline-offset assignment** — nothing to set it *to*, because of the next point.

2. **`StyleAndFont`** (`crates/warpui_core/src/text_layout.rs:587-591`):
   ```rust
   pub struct StyleAndFont {
       pub font_family: FamilyId,
       pub properties: Properties,   // weight, style (italic/oblique) — no size
       pub style: TextStyle,
   }
   ```
   and **`TextStyle`** (`:562-575`) — the per-run style struct actually consumed by the
   platform shaper — has fields for `foreground_color`, `syntax_color`,
   `background_color`, `border`, `error_underline_color`, `show_strikethrough`,
   `underline_color`, `hyperlink_id`. **No size field, no vertical-offset field.** Its
   builder methods (`with_foreground_color`, `with_underline_color`,
   `with_show_strikethrough`, etc., `:603-659`) confirm the exhaustive set — there's
   nothing to extend into for size or offset without adding new fields.

3. **`Line`** (`crates/warpui_core/src/text_layout.rs:491-509`) — the shaped output for one
   visual line — carries a **single scalar** `font_size: f32` (plus `line_height_ratio`,
   `baseline_ratio`, `ascent`, `descent`) that applies to the whole line, alongside
   `runs: Vec<Run>` where each `Run` (`:667-672`) has `styles: TextStyle` (colors/
   decorations only) but **no per-run size**. The doc comment on `baseline_ratio`
   (`:474-475`, also echoed on `LineStyle`) is explicit: *"how far below the origin the
   baseline should fall... within the em-box **for the line**"* — one baseline per line,
   by construction.

4. **`LineStyle`** (`crates/warpui_core/src/platform/mod.rs:77-88`) — the input to the
   platform text shaper — is the same shape: `font_size: f32`, `line_height_ratio: f32`,
   `baseline_ratio: f32`, `fixed_width_tab_size: Option<u8>`. One value each, for the
   entire line being shaped.

5. **`TextLayoutSystem::layout_line`/`layout_text`**
   (`crates/warpui_core/src/fonts/text_layout_system.rs:27-56`) take `text: &str`, one
   `line_style: LineStyle`, and `style_runs: &[(Range<usize>, StyleAndFont)]`, and delegate
   to `self.platform` — the CoreText (macOS) or cosmic-text (winit) backend
   (`crates/warpui_core/src/platform/`). The platform shaper places every glyph's
   `position_along_baseline` (`Glyph`, `:672-679`) using that one `LineStyle`. Per-run
   `StyleAndFont` only ever varies color/weight/decoration in the current call shape —
   never size or vertical position.

**Conclusion:** there is no plumbing today — from the fragment style struct, through the
style-run conversion, through `TextStyle`/`Run`, through `Line`/`LineStyle`, to the
platform shaper call — for a sub-line vertical offset or a sub-line font-size override.
This is a genuine gap in the text-layout engine's data model, not a missing style flag one
layer up. It sits one level deeper than the sibling specs' "extend a match arm" or "add a
DOM reader" changes.

## Feasibility summary (three tiers, honestly sized)

- **(i) Parser recognition of `<sub>`/`<sup>` → a new style flag: SMALL.** Mechanically
  identical to `<u>`: add `SubStart`/`SupStart` to `DelimiterKind`
  (`markdown_parser.rs:1807-1813`), mirror `parse_inline_token_underline_start/end` for
  `<sub>`/`</sub>` and `<sup>`/`</sup>`, mirror `parse_underline`'s backtrack-and-resolve
  logic, add `sub`/`sup` to `PHRASING_ELEMENT_TAGS` (`html_parser.rs:26-28`) and the
  styling match arm (`:441-450`), and add e.g. `pub vertical_align: Option<VerticalAlign>`
  (an enum `{ Sub, Sup }`) to `FormattedTextStyles`. This is well-trodden ground in this
  codebase and low risk.
- **(ii) True per-run baseline shift + font scale in the paint path: LARGE.** This is the
  real cost. It requires: adding size/offset fields to `TextStyle` and/or `StyleAndFont`;
  changing `Line`/`Run` to support more than one `font_size`/baseline per line (or
  introducing a secondary per-run baseline-adjustment applied *after* the platform shaper
  positions glyphs at the line's uniform baseline); and — because CoreText and cosmic-text
  are two independent platform backends — implementing the offset in **both**
  `crates/warpui_core/src/platform/` backends, keeping them visually consistent. This
  touches the lowest, most shared layer of the rendering stack (`Line`/`Run`/`Glyph` are
  used far beyond Markdown — anywhere rich text renders), so it carries real regression
  risk and needs careful review from whoever owns `warpui_core::text_layout`. This is not
  a Markdown-viewer-scoped change; it's a text-engine change that the Markdown viewer
  would be the first consumer of.
- **(iii) MVP without touching the shaper: SMALL–MEDIUM.** Ship the visual "reads as
  sub/superscript" outcome without new shaper plumbing, using one of two approaches (both
  detailed below). This is what makes shipping *something* in a reasonable slice possible
  without waiting on (ii).

Given (ii)'s cost and blast radius, **recommend shipping (i) + (iii) as this slice**, with
(ii) — true glyph-level baseline shift — scoped as an explicit, separately-owned follow-up
against `warpui_core::text_layout`. The product spec's invariant 5 already frames size
reduction as a non-blocking refinement for exactly this reason.

## Proposed changes

### 1. Parser: recognize `<sub>`/`<sup>` (tier i, do this regardless of MVP choice)

- `markdown_parser.rs`: add `SubStart`, `SupStart` to `DelimiterKind` (`:1807-1813`);
  `valid_count` → `count == 1` for both (matching `UnderlineStart`); `as_str` → `"<sub>"`/
  `"<sup>"`. Add `parse_inline_token_sub_start`/`_end` and `..._sup_start`/`_end` mirroring
  `parse_inline_token_underline_start`/`_end` (`:1625-1642`) for the four literal tags.
  Add `parse_sub`/`parse_sup` mirroring `parse_underline` (`:1274-1306`), each backtracking
  the new style flag over its matched span instead of `underline = true`.
- `html_parser.rs`: add `"sub"`, `"sup"` to `PHRASING_ELEMENT_TAGS` (`:26-28`); add match
  arms in the styling switch (`:441-450`) setting the same new flag, right where the
  existing TODO already flags this class of gap.
- `lib.rs`: extend `FormattedTextStyles` (`:545-552`) with a field capturing sub vs. sup —
  recommend a single `pub vertical_align: Option<VerticalAlign>` with `enum VerticalAlign {
  Sub, Sup }` (one flag, mutually exclusive) rather than two independent booleans, since a
  fragment can't sensibly be both at once in this slice (see nesting, below).

### 2. MVP rendering without shaper changes (tier iii — pick one)

**Option A (recommended): true baseline shift via post-shape glyph translation, scoped to
Markdown-viewer paint only.** Rather than teaching the general `Line`/`LineStyle` model a
second baseline, shape the sub/sup-flagged run *and* the rest of the line normally (one
`LineStyle`, unchanged), then, only in the Markdown element painter
(`crates/editor/src/render/element/paint.rs`, where `Decoration`/`RichTextStyles` are
already consumed per the existing underline/strikethrough painting at `:186-190`), apply a
vertical pixel offset when painting the glyphs belonging to a sub/sup-flagged run — shift
down for `Sub`, up for `Sup`, by a fixed fraction of the line's `font_size` (e.g. ±0.3em,
matching typical CSS `vertical-align: sub/super` behavior). This needs the paint code to
know which glyph ranges came from a sub/sup fragment (it already has fragment→range
mapping via the style runs) and translate just those glyphs' paint position before
drawing. No change to `TextStyle`, `Line`, `LineStyle`, or the platform shaper — the offset
is applied at the very last step, in the one place (`paint.rs`) that already special-cases
decorations per style flag (underline/strikethrough do exactly this today, just for line
decorations instead of glyph position). Font size is *not* reduced in this option — this
directly ships product spec invariant 5's stated fallback (full-size, baseline-shifted).
- Risk: shifting glyphs post-shape without adjusting line height can visually clip a
  `<sup>` against the line above it, or a `<sub>` against the line below, if line spacing
  is tight. The implementation must account for the offset when computing line bounding
  boxes used for selection hit-testing and overlap (`table.rs`/`layout.rs`'s line-height
  helpers) — or clamp the offset to stay within the existing line-height ratio's slack.
  This is the concrete risk to validate early with a spike, before committing to Option A
  over B.

**Option B (smaller, more limited): Unicode subscript/superscript character
substitution.** For content whose characters have Unicode sub/superscript codepoints
(digits 0-9, `+ − = ( )`, and a limited set of Latin letters — e.g. U+2080-U+2089 for
subscript digits, U+00B9/U+00B2/U+00B3/U+2074-U+2079 for superscript digits), substitute
the visually-equivalent Unicode character instead of applying any layout offset. This
requires zero paint-path changes — it's a pure text substitution at the fragment level.
**Coverage limits, stated honestly:** this covers the issue's own worked examples (H₂O,
CO₂, footnote¹) but silently fails or degrades for arbitrary content — most letters (only
a handful of Latin letters have Unicode superscript forms; subscript Latin letters are
almost entirely absent), multi-digit numbers rendered digit-by-digit can have inconsistent
kerning/width per font, and it cannot represent *styled* sub/sup content (invariant 4 —
composing with bold/italic/links) since it's swapping characters, not applying a style.
Given the product spec explicitly requires arbitrary inline content to compose with
sub/sup (invariant 4) and doesn't want silent degradation to wrong-looking output for
uncovered characters, **Option B does not fully satisfy the product spec** and is
documented here mainly as the cheaper fallback if Option A's paint-path spike turns out to
be riskier than expected.

**Recommendation: Option A**, with a short paint-path spike (a few hours) to de-risk the
line-height/clipping question before committing the full slice, since it's the one
sub-question that could push this back toward Option B or a smaller MVP.

### 3. Nesting (`<sup>` inside `<sub>`, or any nesting of `<sub>`/`<sup>`)

**Superseded**: an earlier version of this section specified an innermost-wins tie rule
(the inner tag's `backtrack_styles` call overwrites the outer's, mirroring how nested
`<u>`/emphasis resolve). That rule shipped, then was replaced after live verification
surfaced two motivating cases where it (and even a flat same-direction collapse) render a
plausible-looking but factually wrong formula rather than an obviously-broken one:

- **Same-direction towers** — `2<sup>3<sup>4</sup></sup>` is authored to mean 2^(3^4).
  Since `vertical_align` is a single tri-state attribute with no compounding, both
  nesting levels collapse to the identical single-step superscript offset, rendering "34"
  raised — which reads as 2^(34), a different number, not as an obviously degraded
  rendering.
- **Opposite-direction ties nested a level deep** — `2<sup>3<sub>4</sub></sup>` is
  authored to mean 2^(3-sub-4). Innermost-wins renders "4" as a full subscript relative
  to "2"'s own baseline (since the buffer/style model has no notion of "subscript
  relative to an already-superscripted position"), which reads as (2³)₄ — again a
  different, wrong expression that still looks like a plausible formula.

**Current rule (product invariant 7): whole-formula literal bail.** Any nesting of
`<sub>`/`<sup>` — same direction, opposite direction, or depth ≥ 2 — degrades the
**entire outermost span** (its open tag through its matching close tag, all contents,
including any nested tags) to plain literal text, with no partial styling applied
anywhere in the span. Only a single, non-nested `<sub>` or `<sup>` renders styled.
Rationale: a partially-styled nested construct can still misread as a valid (if unusual)
formula even when the styling is wrong or incomplete; showing the whole span as source
text is the only rendering that cannot be mistaken for a real formula. Put simply: it's
better to show the code than to show the wrong math.

Implementation (`crates/markdown_parser/src/markdown_parser.rs`, `parse_vertical_align`
and `Delimiter::vertical_align_poisoned`): nesting is detected when a `<sub>`/`<sup>`
close finds an *outer* vertical-align delimiter still active earlier on the delimiter
stack (meaning this tag opened inside an already-open vertical-align span). When that
happens, this tag's own span reverts to literal (its opening placeholder node — already
literal tag text at push time — is left untouched, and its own literal closing tag text
is pushed alongside it), and the outer delimiter is marked `vertical_align_poisoned` so
that when *it* eventually closes, it also bails its entire span to literal rather than
applying its own alignment. This propagates outward through arbitrarily many enclosing
levels without needing to reconstruct any already-resolved (and already-removed) inner
delimiter state.

True compounding (doubled/scaled offset for doubly-nested sub/sup, so towers and nested
ties render styled instead of bailing) remains out of scope for this slice — documented
as a future direction on #13734 rather than a separate tracked ticket, since it's not
currently planned work. It would require `vertical_align` to become a depth-aware
representation (e.g. a signed integer level rather than a `None`/`Sub`/`Sup` tri-state)
threaded through the buffer summary, the CoreText attribute round-trip, and the paint
path's baseline-offset/font-scale calculation.

Editor-only note: overlapping subscript/superscript buffer markers are unreachable via
Markdown import under this rule (the parser bails any nesting to a single literal
fragment before the buffer ever sees it). They remain reachable only via live
interactive style editing — applying one alignment over an in-buffer selection that
already carries the other, which has no equivalent "is this nested" concept for the
editor to refuse the operation. For that internal case only, `StyleSummary` and the
render iterator must still agree with each other on *some* consistent tie rule
(currently innermost-wins) purely so cursor/toolbar state and rendered runs don't
diverge — this has no visible product behavior, since it's never reachable from parsed
Markdown source.

### 4. Feature gating

No existing feature flag covers inline HTML phrasing tags as a group — `<u>` support
appears ungated. Follow that precedent: no new flag, ship enabled wherever inline HTML
parsing already runs (paste path via `html_parser.rs`; file-viewer path via
`markdown_parser.rs`'s inline tokenizer, confirmed above to have its own independent
`<u>`-style mechanism sub/sup will mirror).

### 5. Copy / export (product invariant 8)

Canonical re-serialization back to Markdown/HTML for a `vertical_align`-flagged fragment
emits `<sub>…</sub>`/`<sup>…</sup>` around the fragment's text, mirroring however `<u>` is
currently canonically re-serialized on export (locate and reuse that existing code path —
it must already exist since underline is already canonically re-serialized today). As
elsewhere in Warp, this is not a byte-exact reproduction of the original source. If Option B
(Unicode substitution) were chosen instead, the substituted character re-serializes as
itself (no tag), which is a strictly worse and lossier export story — another point in
favor of Option A.

## Testing and validation

### Parser unit tests (`crates/markdown_parser/src/markdown_parser_tests.rs`,
`html_parser_tests.rs`)

- `<sub>2</sub>` / `<sup>1</sup>` → fragment with `vertical_align = Some(Sub/Sup)`
  (invariants 1-3).
- `<sub>*n*</sub>` → composed italic + sub flag on the same fragment (invariant 4).
- Unterminated `<sub>` (no closing tag) → literal-text fallback for the unmatched tag,
  rest of document unaffected (invariant 6), mirroring the existing unmatched-`<u>`
  fallback test if one exists.
- `<sup>` nested inside `<sub>` (any direction, any depth) → the entire outermost span
  renders as plain literal text, no panic, zero styled runs (invariant 7; supersedes the
  earlier innermost-wins expectation). Named cases: the same-direction tower
  `2<sup>3<sup>4</sup></sup>` and the opposite-direction nested tie
  `2<sup>3<sub>4</sub></sup>` — both motivating examples for the whole-formula-bail rule,
  both assert the entire construct is literal.
- `<sub class="foo">` → attribute ignored, only tag semantics consulted (invariant 9).

### Paint/render tests (`crates/editor/src/render/element/paint.rs` tests, or
`crates/editor/src/render/model/mod_tests.rs`)

- A line containing a sub/sup-flagged run paints that run's glyphs vertically offset from
  the rest of the line, in the correct direction (down for sub, up for sup).
- Offset sub/sup glyphs do not clip against the adjacent line above/below at default line
  spacing (the risk called out in Option A) — this is the spike's key validation target
  before the full slice is committed.
- Selection/caret hit-testing across a sub/sup run still resolves to the correct character
  offsets despite the glyph paint-position shift (selection logic should operate on
  pre-offset positions/ranges, not the shifted paint coordinates — confirm this separation
  holds).

### Round-trip (export)

- `<sub>2</sub>` / `<sup>1</sup>` → internal format / Markdown export → back, tag
  semantics preserved (invariant 8).

### Integration / manual

Per CONTRIBUTING, before/after screenshots using the issue's own test case (H<sub>2</sub>O,
CO<sub>2</sub>, and the footnote-marker `<sup>1</sup>` example) rendered in the Markdown
viewer, plus a case exercising composed styling (`<sub>*n*</sub>`) and the whole-formula
literal-bail case for nesting.

## Risks and follow-ups

- **The real cost is one layer below the Markdown viewer.** Tiers (i) and (iii)/Option A
  are Markdown-viewer-scoped and low-to-medium risk; true per-run baseline shift in the
  general text-layout engine (tier ii) is a `warpui_core::text_layout` change affecting
  every rich-text consumer in the app, not just Markdown, and should be scoped, owned, and
  reviewed as its own effort rather than folded into this slice.
- **Option A's line-height/clipping risk is unresolved until the spike runs.** If the
  spike shows unacceptable clipping or selection-hit-testing breakage, fall back to Option
  B (Unicode substitution) for a narrower but shippable MVP, with the coverage gaps in the
  product-facing description, or re-scope to require tier (ii) up front.
- **No font-size reduction in the recommended MVP.** Product invariant 5 explicitly allows
  this; if maintainers want the classic "smaller and shifted" look immediately, that
  requires either accepting tier (ii)'s cost now, or layering a coarse font-scale hack onto
  Option A's paint-time approach (e.g. painting the affected glyphs from a pre-scaled glyph
  render — feasibility unconfirmed, would need its own spike; flagged here rather than
  assumed).
- **Interaction with other tier-zero specs:** sub/sup fragments inside a `<table>` cell (per
  the `<table>` spec) or a `<details>` body should compose for free once both land, since
  both are ordinary inline content paths; verify once both are implemented.
- **Depth-aware cumulative rendering is a real future direction, not a hypothetical.** Math
  notation (nested exponents, stacked subscripts) is not uncommon in Markdown on GitHub and
  elsewhere — the whole-formula literal bail (section 3) is a deliberate MVP scope choice,
  not a claim that nesting is unimportant. Lifting the bail for towers and nested ties would
  need `vertical_align` to become depth-aware (signed level rather than tri-state) through
  the buffer, CoreText round-trip, and paint layers. Documented as a future direction on
  #13734 rather than a separate tracked ticket, since it's not currently planned work.
