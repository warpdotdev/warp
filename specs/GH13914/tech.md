# Tech Spec: Inline code chip background extends past glyph ink on the right

Issue: https://github.com/warpdotdev/warp/issues/13914
Product spec: `specs/GH13914/product.md`

## Context

### Where the background is painted

`Line::paint_internal` in `crates/warpui_core/src/text_layout.rs` walks each `Run` in
a `Line` and, for runs with `run.styles.border.is_some() || run.styles.background_color.is_some()`,
computes a horizontal `visible_left..visible_right` span before delegating to
`paint_run_background`:

- `crates/warpui_core/src/text_layout.rs:1519-1567` — the per-run "simulate the glyph
  walk" loop that determines `visible_left`/`visible_right` for the background,
  mirroring (but not sharing code with) the actual glyph-drawing loop below it.
  `visible_right = visible_right.max(glyph_x + glyph.width)` (line 1550) — so the
  right edge of the background is the position of a glyph's origin (baseline x)
  plus its *advance width* (`glyph.width`), not a measurement of where that
  glyph's ink actually stops drawing.
- `crates/warpui_core/src/text_layout.rs:1202-1285` — `paint_run_background` itself,
  which takes `visible_left`/`visible_right` as already-computed screen-space
  bounds and paints a `RectF` of width `(visible_right - visible_left) + 2. * block_padding`
  (line 1257) at `visible_left - 2. * block_padding` (line 1255). `block_padding`
  (line 1220-1224) is a small kerning-compensation constant (`font_size / 10.`)
  applied symmetrically to both edges — it is not the asymmetry this issue is
  about; the asymmetry comes from `glyph.width` itself.

### What `glyph.width` actually measures

`Glyph.width` is documented in-repo as the glyph's *advance*, not its ink extent:

- `crates/warpui_core/src/text_layout.rs:674-683`:
  ```rust
  pub struct Glyph {
      pub id: GlyphId,
      /// Position of the glyph on its baseline.
      pub position_along_baseline: Vector2F,
      pub index: usize,
      /// The width of the glyph (its advance), in pixels.
      pub width: f32,
  }
  ```

This field is populated per-platform:

- **macOS (CoreText):** `crates/warpui/src/platform/mac/text_layout.rs:769-787`
  builds each `Glyph` inside a `.map` over `itertools::multizip((glyphs, positions,
  string_indices, advances(&run)))`, setting `width: advance.width as f32` at line
  785. The `advances(&run)` helper (`crates/warpui/src/platform/mac/text_layout.rs:993-1017`)
  is a direct wrapper over Apple's `CTRunGetAdvancesPtr`/`CTRunGetAdvances`, i.e.
  the true CoreText advance width for each glyph, which by definition includes
  right-side bearing/spacing baked in by the shaping engine. This is the
  documented root cause: CoreText's advance is deliberately ≥ the glyph's ink
  width so consecutive glyphs don't collide, and that extra space is what shows
  up as excess chip background on the right.
- **winit/cosmic-text (Linux):** `crates/warpui/src/windowing/winit/fonts.rs:846`
  and `:1188-1194` both implement `fn glyph_advance`; the first delegates to
  `self.text_layout_system.glyph_advance(...)`, and the second (used by the
  ttf-parser-backed font-face path) calls `font_face.glyph_hor_advance(glyph_id)`
  directly — an explicit horizontal-advance lookup, not an ink measurement. The
  same advance-based value flows into the shared `Glyph.width` field via the
  cosmic-text-backed `TextLayoutSystem`. Both platforms feed the identical
  advance-based `Glyph.width` into the one shared `paint_run_background` code
  path in `warpui_core`, so the bug and any fix are platform-uniform at the
  `warpui_core::text_layout` layer — no per-platform branching is required in
  the fix itself.

### Why an ink-width alternative is not evenly available (Option B, rejected)

`FontCache::glyph_typographic_bounds` (`crates/warpui_core/src/fonts.rs:377-393`) is
the only ink/bounds-like measurement already plumbed through the cache, but its
two real backends disagree on what it measures:

- **macOS:** `crates/warpui/src/platform/mac/fonts.rs:613`:
  ```rust
  fn glyph_typographic_bounds(&self, font_id: FontId, glyph_id: GlyphId) -> Result<RectI> {
      Ok(self.font(font_id).typographic_bounds(glyph_id)?.to_i32())
  }
  ```
  This calls into `core-text`'s `typographic_bounds`, which wraps CoreText's
  per-glyph typographic bounding box — a font-design-space bounding box, not a
  tight ink/visual bounding box. It is already used elsewhere in this codebase
  for a conceptually different purpose: `Cache::em_width`
  (`crates/warpui_core/src/fonts.rs:506-516`) uses it to measure the width of the
  `'m'` glyph for a monospace em-width estimate, where "close to but not exactly
  ink width" is fine.
- **winit (ttf-parser/cosmic-text):** `crates/warpui/src/windowing/winit/fonts.rs:1199-1226`
  implements the same trait method by calling `font_face.glyph_raster_image(...)`
  (for bitmap/emoji glyphs) or `font_face.glyph_bounding_box(...)` (for outline
  glyphs) via `ttf-parser` — this **is** the true glyph outline bounding box
  (real ink extents), a fundamentally different measurement than CoreText's
  typographic bounds.

Because the two platforms' implementations of the same trait method measure
different things (font-design bounds vs. true ink bounds), computing the
background span from `glyph_typographic_bounds` would make the chip's right-edge
padding visibly different in *size* between macOS and Linux builds for
identical text — trading one asymmetry (left vs. right) for another
(platform-inconsistent). This is why Option B (ink-width-based background
measurement) is documented as a steel-manned alternative but not the chosen
approach; see "Alternatives considered" below.

### Existing clamping behavior this fix must not break

`crates/warpui_core/src/text_layout_tests.rs:463-520`,
`test_run_background_clamped_to_visible_glyph_span`, pins the invariant added by a
prior fix: when a run is partially truncated (e.g. by an ellipsis), the painted
background must be clamped to only the glyphs that are actually drawn
(`visible_left..visible_right`), not the full run width. The fix in this spec
must compose with — not replace — that clamping. Concretely, the padding
adjustment proposed below is applied to the already-clamped `visible_left`/
`visible_right` values, so a truncated run's background is still bounded by
what's visibly drawn, just shifted by the padding correction within that bound.

### The second render surface: TUI Markdown (`warp_tui`)

The ticket asks whether the fix needs to reach a second render surface. It does
not, because that surface does not use this paint path at all:

- `crates/warp_tui/src/tui_markdown.rs:37,56` defines
  `inline_code: builder.accent_text_style()` and, in `fragment_style`
  (`crates/warp_tui/src/tui_markdown.rs:230-259`), applies it via
  `style = style.patch(palette.inline_code);` at line 253. `TuiStyle` here is
  `ratatui`'s cell style type (re-exported through
  `warpui_core::elements::tui::TuiStyle`), which carries foreground/background
  *colors and modifiers* applied per-terminal-cell — there is no `TextBorder`,
  no glyph-advance geometry, and no equivalent of `paint_run_background`'s
  sub-pixel horizontal span math. `TuiStyle` never receives a `background_color`
  for inline code in this file (only `accent_text_style()`'s foreground/modifier
  patch), so today's TUI inline code rendering is foreground-color-only and has
  no visible chip background to be asymmetric in the first place.
- Compare this to the "real" (non-TUI) editor path, where
  `crates/editor/src/render/layout.rs:190-201` explicitly sets both
  `with_background_color(...)` and `with_border(TextBorder { .. })` for
  `text_styles.is_inline_code()` — that's the path that reaches
  `warpui_core::text_layout::Line::paint_run_background` and is affected by this
  bug. The notebook/plan editor and any other Markdown-consuming surface that
  goes through `RichTextStyleContext::style_and_font`
  (`crates/editor/src/render/layout.rs:165-214`, specifically lines 190-201) is
  also affected, since they share this single code path.
- **Conclusion:** the fix, scoped to `warpui_core::text_layout::Line`, reaches
  every consumer of `TextStyle::background_color`/`TextStyle::border` uniformly
  (this includes the `editor` crate's Markdown inline-code rendering and any
  other caller of `style_and_font`/`markdown_inline_to_text_and_style_runs` that
  sets a background/border), but does **not** reach `warp_tui`, because
  `warp_tui` never constructs a `warpui_core::text_layout::TextStyle` with a
  background/border for inline code — it renders through a completely separate,
  cell-grid-based `ratatui` styling path. No change to `warp_tui` is proposed or
  required by this spec (see product spec's Non-goals).

## Proposed changes

### Chosen approach: shift the background left by half the trailing advance padding (Option A, issue author's preferred direction)

Per the issue author's own comment on the ticket, the preferred fix is: **shift
the background decoration left by half of the "available width padding"**,
accepting a subtle sawtooth in stacked code chips, in exchange for preserving
the chip's leading-edge (left) alignment.

Concretely:

1. In the `visible_left`/`visible_right` computation loop at
   `crates/warpui_core/src/text_layout.rs:1519-1567` (and its counterpart used by
   `paint_run_background` directly, since that function also needs the adjusted
   span — see `crates/warpui_core/src/text_layout.rs:1202-1285`), compute a
   per-run **trailing advance padding** value: the difference between the last
   visible glyph's advance width and its... [ink width is unavailable
   platform-uniformly, per "Why an ink-width alternative is not evenly
   available" above]. Because true per-glyph ink width cannot be sourced
   consistently across platforms, the padding heuristic instead uses a
   font-size-relative constant fraction of `self.font_size`, analogous to the
   existing `block_padding` heuristic already used for border kerning
   compensation (`crates/warpui_core/src/text_layout.rs:1220-1224`,
   `self.font_size / 10.`). **This specific fraction requires maintainer
   sign-off — see Open Question 1 in `product.md`** — because, unlike the
   existing `block_padding` (which compensates for a fixed, small kerning
   adjustment), this new padding is standing in for a genuinely
   glyph-dependent, per-character advance/ink gap that a single constant can
   only approximate. The fix should be structured so this constant is a single,
   well-commented, easily-tunable value (or informed by an average/typical
   advance-vs-ink ratio for the font, if that becomes measurable later) rather
   than hard-coded inline in multiple places.
2. Apply the computed padding by shifting `visible_left` and `visible_right`
   both left by `padding / 2.` — NOT by widening or narrowing the span, since
   the intent is to shift the block of color, not resize it (resizing would
   change how the block responds to the existing `block_padding` and clamping
   logic in ways that would need re-deriving from scratch).
3. Do **not** touch `glyph.position_along_baseline`, `caret_positions`, or
   `Glyph.width` anywhere — this is a paint-time-only geometry change, fully
   isolated to the background/border rectangle computed in
   `paint_run_background` and its span pre-computation in `paint_internal`. This
   is required by product spec Success Criteria #2 and #3 (no caret/selection
   regression).
4. The truncation-clamping behavior
   (`test_run_background_clamped_to_visible_glyph_span`) must continue to apply
   to the *shifted* span, i.e. shift first, then clamp to `visible_bounds` via
   `text_rect.intersection(visible_bounds)`
   (`crates/warpui_core/src/text_layout.rs:1263-1267`) — the existing
   intersection call already happens after the rect is constructed, so as long
   as the shift is applied when building `text_rect`'s origin/size (lines
   1254-1261), the intersection-based clamping composes for free.

### Alternatives considered

- **Option B — ink-width-based background measurement (steel-manned, not
  chosen):** measure the true rendered ink extent of the last (and first)
  visible glyph per run, and set `visible_left`/`visible_right` to those ink
  bounds instead of advance-based bounds. This would produce the visually
  "tightest" and most technically correct chip, and is the more principled fix
  in isolation. It is not chosen because:
  - The only currently-plumbed bounds API, `glyph_typographic_bounds`
    (`crates/warpui_core/src/fonts.rs:377-393`), measures different things on
    each platform (CoreText font-design bounds on macOS vs. true ttf-parser
    outline bounds on Linux — see "Why an ink-width alternative is not evenly
    available" above), so adopting it as-is would introduce a new
    platform-dependent visual inconsistency instead of removing an existing
    left/right one.
  - Even where ink bounds are accurately available (Linux), switching the
    *left* edge to ink-based measurement as well would move the chip's leading
    edge whenever the first glyph's left-side bearing is nonzero, which
    product spec Success Criteria #2 explicitly rules out.
  - This remains a valid follow-up if a maintainer decides platform-uniform ink
    measurement is worth adding (e.g., a new CoreText call that returns true
    ink bounds, to bring macOS in line with the Linux ttf-parser behavior)
    before revisiting Option B.
- **Option C — split the difference (shift part, widen part):** apply half the
  correction as a leftward shift and half as a width reduction. This was raised
  by the issue author as a compromise but is not adopted in this initial fix
  because it doubles the number of tunable constants (shift fraction and width
  fraction) without eliminating the sawtooth trade-off, for a marginal
  additional visual improvement. Documented in `product.md` Open Question #3 as
  a possible future refinement once real-world visual feedback on Option A is
  available.

## Testing and validation

- **Update `test_run_background_clamped_to_visible_glyph_span`**
  (`crates/warpui_core/src/text_layout_tests.rs:463-520`) to account for the new
  padding-shift arithmetic: the test currently asserts the painted rect's exact
  `visible_left`..`visible_right` bounds (minus `block_padding`); once the shift
  is introduced, the expected left/right bounds in the assertion must be updated
  to `visible_left - padding/2.` and `visible_right - padding/2.` (composed with
  the existing `block_padding` offset), while the *clamping* behavior itself
  (that a fully-truncated run paints nothing, and a partially-truncated run's
  background does not extend past the drawn glyph span) must still hold.
- **New unit test:** a run with a single glyph of known advance width and a
  configured padding constant should produce a background rect whose left edge
  is `padding/2.` to the left of the un-shifted `visible_left`, and whose width
  is unchanged (only the origin shifts, not the size) — this directly pins
  "shift, don't resize."
- **New unit test:** a two-glyph run (to exercise `visible_right` being
  determined by the last glyph specifically, not an aggregate) confirms only
  the trailing edge's advance padding affects the shift amount as intended by
  the "shift left by half the *available* padding" formulation (i.e., the
  padding source is the last visible glyph, not a sum/average across the run).
- **Regression test:** confirm existing caret/selection tests relying on
  `x_for_index`, `width_for_index`, `caret_position_for_index`, and
  `index_for_x` (`crates/warpui_core/src/text_layout.rs:1031-1171`) are
  unaffected, since none of those read `visible_left`/`visible_right` or the
  background rect at all — they operate purely on
  `glyph.position_along_baseline` and `caret_positions`, which this fix does
  not modify. No new tests are needed here beyond confirming the existing suite
  still passes; this is a "prove the isolation held" check, not new coverage.
- **Manual verification:** render a message with mixed inline code spans ending
  in narrow-ink characters (e.g. `` `foo.bar()` ``, `` `x;` ``) side-by-side
  with wide-ink-ending spans (e.g. `` `foo_bar` ``) and confirm the right-side
  padding reads visually closer to the left-side padding than before the fix,
  on both macOS and a Linux/winit build.
- **Manual verification of the accepted trade-off:** render two consecutive
  lines each starting with an inline code span of different text, and confirm
  the described "subtle sawtooth" (right edges not perfectly column-aligned) is
  present but subtle, matching the product spec's accepted trade-off — this is
  a visual sign-off step, not a pass/fail automated test.
- `cargo fmt` and `cargo clippy --workspace --all-targets --all-features --tests
  -- -D warnings` must pass, per `CONTRIBUTING.md`.
- `cargo nextest run -p warpui_core --no-fail-fast` covering
  `text_layout_tests.rs` must pass with the updated and new tests above.

## Risks and mitigations

- **Risk:** the chosen font-size-relative padding constant does not visually
  match the actual advance-vs-ink gap for most fonts/characters, either
  under- or over-correcting. **Mitigation:** the constant is isolated to a
  single named location (mirroring `block_padding`'s existing pattern) so it
  can be tuned without touching the shift/clamp logic; Open Question #1 in
  `product.md` explicitly flags that the exact value needs maintainer
  sign-off/visual QA before merge, not just code review.
- **Risk:** shifting the background independently per run (rather than per
  line) could, in rare cases (e.g. two adjacent backgrounded runs with a
  visible gap between them, such as two consecutive short inline-code spans
  separated by a space), produce a visible seam or overlap between
  independently-shifted chips. **Mitigation:** each run's background is already
  painted as an independent rect today (no shared/merged rect across runs), so
  this risk is not new — it is the pre-existing per-run painting model, and the
  fix does not change how runs relate to each other, only how each run's own
  rect is positioned.
- **Risk:** a future contributor mistakes the padding shift for a general text
  positioning fix and reuses it outside the background/border path (e.g.
  accidentally applying it to glyph draw positions). **Mitigation:** the
  implementation should keep the padding constant and shift logic scoped
  entirely inside `paint_run_background`'s span computation, with a doc comment
  cross-referencing this issue and explicitly noting it must not be applied to
  glyph or caret geometry.
- **Risk:** RTL text inside a code span could have "leading edge" (first glyph
  drawn) and "trailing edge" (last glyph drawn) not correspond to "visually
  left" and "visually right" the way this spec assumes. **Mitigation:** the
  existing `visible_left`/`visible_right` computation already determines these
  via `.min()`/`.max()` over glyph positions regardless of draw order
  (`crates/warpui_core/src/text_layout.rs:1549-1550`), so `visible_right` is
  already "whichever edge is visually rightmost," not "the last glyph drawn."
  The padding shift must be derived from whichever glyph produced
  `visible_right` (the visually-rightmost glyph), not assumed to be the last
  glyph in iteration order, to remain correct for RTL runs.

## Follow-ups

- Consider adding a true, platform-uniform ink-bounds API (e.g., extending
  `FontDB::glyph_typographic_bounds` or adding a new trait method) if a future
  iteration wants to revisit Option B with matching semantics on macOS and
  Linux.
- Revisit Option C (split shift + resize) once real-world feedback on Option
  A's sawtooth trade-off is available.
- If the font-size-relative padding constant proves visually wrong for
  specific fonts/sizes used in inline code (e.g. the configured monospace code
  font differs meaningfully from the UI font in advance-vs-ink ratio), consider
  sourcing the constant from font metrics specific to the inline-code font
  family rather than a single global fraction of `self.font_size`.
