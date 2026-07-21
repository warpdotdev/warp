# Tech Spec: Inline code chip background extends past glyph ink on the right

Issue: https://github.com/warpdotdev/warp/issues/13914
Product spec: `specs/GH13914/product.md`

## Context

### Where the background is painted

`Line::paint_internal` in `crates/warpui_core/src/text_layout.rs` walks each `Run` in
a `Line` and, for runs with `run.styles.border.is_some() || run.styles.background_color.is_some()`,
computes a horizontal `visible_left..visible_right` span before delegating to
`paint_run_background`:

- The per-run "simulate the glyph walk" loop in `paint_internal` determines
  `visible_left`/`visible_right` for the background, mirroring (but not sharing
  code with) the actual glyph-drawing loop below it. It accumulates
  `visible_left = visible_left.min(glyph_x)` and
  `visible_right = visible_right.max(glyph_x + glyph.width)` — so the right edge of
  the background is a glyph's origin (baseline x) plus its *advance width*
  (`glyph.width`), not a measurement of where that glyph's ink actually stops
  drawing.
- `paint_run_background` takes `visible_left`/`visible_right` as already-computed
  screen-space bounds and paints a `RectF` of width
  `(visible_right - visible_left) + 2. * block_padding` at
  `visible_left - 2. * block_padding` (`crates/warpui_core/src/text_layout.rs:1401-1405`).
  Expanding the rect's right edge (`origin.x() + width`) shows this is **not**
  symmetric: `(visible_left - 2·block_padding) + (visible_right - visible_left) +
  2·block_padding = visible_right` exactly — the `2·block_padding` term cancels out
  of the right edge algebraically and only ever shifts the *left* edge outward.
  `block_padding` (`font_size / 10.` when a border is present) is therefore a
  **left-only** kerning-compensation offset, not a symmetric one, despite reading
  like a per-edge inset at first glance. This is a second, independent asymmetry
  from the one this issue targets (which comes from `glyph.width`/advance itself,
  below) — see "Interaction with the legacy `block_padding` offset" for how the two
  compose.

### What `glyph.width` actually measures

`Glyph.width` is documented in-repo as the glyph's *advance*, not its ink extent:

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

Advance is the sum of a glyph's left-side bearing, its ink, and its right-side
bearing — the pen movement to position the *next* glyph. Anchoring the chip's left
edge at the first glyph's advance origin and its right edge at the last glyph's
advance endpoint therefore leaves roughly one left-side bearing of gap on the left
and one right-side bearing on the right; for most fonts these differ, producing the
visible left/right asymmetry (worst with narrow trailing glyphs whose right-side
bearing is large relative to their ink).

This field is populated per-platform from a true advance lookup on each backend
(CoreText `CTRunGetAdvances` on macOS; `glyph_hor_advance`/`glyph_advance` on the
winit/cosmic-text path), so the advance-based bug — and any fix at the
`warpui_core::text_layout` layer — is platform-uniform; no per-platform branching is
required in the fix.

## The chosen fix: derive chip edges from glyph ink, not advance

The chip's horizontal edges are computed from the true **ink** of the visible
first/last glyphs, plus a fixed design padding:

```
background_left  = first_visible_glyph.ink_left  - pad
background_right = last_visible_glyph.ink_right + pad
```

where `ink_left`/`ink_right` are the glyph's rasterized ink extents in paint space
and `pad` is a fixed fraction of the font size. Interior glyph positions stay
advance-based (they are the text itself). This is a paint-time-only change to the
background/border rectangle; glyph draw positions
(`glyph.position_along_baseline`), `Glyph.width`, and `caret_positions` are never
touched (product spec Success Criteria #2 and #3).

### Interaction with the legacy `block_padding` offset (composes, does not replace)

The ink-derived edges are substituted for `visible_left`/`visible_right` as *local*
values immediately before the existing `block_padding` math runs unchanged
(`crates/warpui_core/src/text_layout.rs:1362-1368` reassigns `visible_left`/
`visible_right`; the `text_rect` construction that consumes them at
`text_layout.rs:1401-1405` is untouched):

```
visible_left  = ink_left_of(first_visible_glyph)  - CODE_CHIP_INK_PADDING_RATIO * font_size
visible_right = ink_right_of(last_visible_glyph)  + CODE_CHIP_INK_PADDING_RATIO * font_size
# ... then, unchanged from before this fix:
rect.origin.x = visible_left  - 2 * block_padding
rect.width    = (visible_right - visible_left) + 2 * block_padding
# ⇒ rect's right edge = rect.origin.x + rect.width = visible_right   (block_padding cancels)
# ⇒ rect's left edge  = visible_left - 2 * block_padding             (block_padding does NOT cancel)
```

So the fix **composes with** the legacy `block_padding` offset; it does not replace,
bypass, or otherwise adjust it. Concretely, for inline code (which always has
`run.styles.border.is_some()`, so `block_padding = font_size / 10.` is nonzero):

- The **right** edge is exactly `ink_right + CODE_CHIP_INK_PADDING_RATIO * font_size`
  — pure ink-edge geometry, `block_padding` has zero effect (it cancels out of the
  right edge algebraically, as shown above).
- The **left** edge is `ink_left - CODE_CHIP_INK_PADDING_RATIO * font_size - 2 *
  block_padding`, i.e. `ink_left - font_size/12 - font_size/5` at the values in this
  spec — a further `font_size / 5` (≈1.08px at a 13px font) beyond the intended ink
  padding, entirely from the pre-existing, uncorrected `block_padding` behavior.

**This means the promised "equal ink padding on both sides" (Goal 1 / Success
Criterion 1) is not fully achieved by the ink-edge fix alone**: the *advance-vs-ink*
asymmetry this issue targets is eliminated (both edges are now ink-anchored plus a
fixed pad), but the *pre-existing, unrelated* `block_padding` left-only skew survives
underneath it, unadjusted. The net left edge is `font_size / 5` further left than the
net right edge is right, for any bordered inline-code chip.

This residual is small (~1px at typical UI font sizes) and strictly *less* than the
advance-vs-ink gap this issue removes (which was on the order of a full right-side
bearing, several pixels for many fonts) — so the fix is a strict visual improvement,
not a regression. But it is **not** the "left edge does not move / right edge reads
equal to left" outcome implied by Goal 2 and Success Criterion 1 taken literally: the
left edge was already `2 * block_padding` further from the ink than the right edge
before this fix, and remains so after it. Callers of `paint_run_background` do not
have a lever to fix this within the scope of this change, because `block_padding`'s
left-only application predates GH13914 and is unrelated to advance-vs-ink; correcting
it is out of scope here (see Follow-ups) — but the open question it raises (should
`block_padding` apply to both edges, e.g. `rect.width` also gaining a `-
2·block_padding`-equivalent on the right, or should the right edge explicitly add
`+ block_padding` to match) needs an explicit maintainer decision, since either
resolution changes the border's visual footprint for every bordered/backgrounded run
in the codebase, not just inline code.

### Per-glyph ink IS derivable cross-platform (the key finding)

An earlier draft of this spec claimed no platform-uniform per-glyph ink measurement
existed. That claim was specifically about **one** bounds wrapper,
`FontCache::glyph_typographic_bounds` (`crates/warpui_core/src/fonts.rs:377-393`),
whose two backends measure different things (documented as a dead end below). It was
not a fundamental platform limitation.

The platform font abstraction seam is the `platform::FontDB` trait
(`crates/warpui_core/src/platform/mod.rs:347`). That trait already exposes a second,
distinct measurement: **`glyph_raster_bounds`** (declared at
`crates/warpui_core/src/platform/mod.rs:405`; wrapped and cached by
`FontCache::glyph_raster_bounds` at `crates/warpui_core/src/fonts.rs:395-418`, keyed
by `(GlyphKey, scale)`). It returns the true **rasterized ink bounds** of a glyph,
in device pixels, and both real backends implement it with matching semantics:

- **macOS:** `crates/warpui/src/platform/mac/fonts.rs` → the font-kit rasterizer at
  `crates/warpui/src/fonts/font_kit.rs:42-78`, which calls font-kit
  `Font::raster_bounds(glyph, point_size, Transform2F::from_scale(scale),
  HintingOptions::None, GrayscaleAa)` — the true rasterized ink box (CoreGraphics/
  CoreText) in device pixels.
- **Linux (winit/cosmic-text):** `crates/warpui/src/windowing/winit/fonts.rs` → the
  swash rasterizer at `crates/warpui/src/fonts/swash_rasterizer.rs`, which returns
  `RectI::new(vec2i(image.placement.left, -image.placement.top),
  vec2i(image.placement.width, image.placement.height))` — the true swash-rasterized
  ink placement in device pixels.
- **Test backend:** `crates/warpui_core/src/platform/test/delegate.rs` implements the
  same trait method; for tests it returns deterministic synthetic ink so the ink-edge
  derivation is unit-exercisable, and a zero-width box for a designated zero-ink
  glyph to exercise the fallback path.

Because `glyph_raster_bounds` reports true ink on both backends, no new trait method
and no new platform plumbing are needed — the existing seam suffices.

### Device-pixel → paint-space conversion (required)

`glyph_raster_bounds` returns device pixels (paint-space × the scene scale factor),
relative to the glyph origin. The background rect is built in scaled paint-space
`f32` (the same space as `glyph.position_along_baseline` and `glyph.width`), so the
raster bounds must be divided by the scale factor and offset by the glyph's paint-
space origin `glyph_x`:

```
ink_left  = glyph_x + raster_bounds.min_x() / scale_factor
ink_right = glyph_x + raster_bounds.max_x() / scale_factor
```

The scale factor and the `GlyphConfig` that `glyph_raster_bounds` needs are both
read from the `scene` argument already passed to `paint_run_background`
(`scene.scale_factor()` and `scene.rendering_config().glyphs`), so no signature or
call-site plumbing beyond the paint path is required.

### The fixed padding constant

`pad = font_size / 12` (`CODE_CHIP_INK_PADDING_RATIO`). It is a single, well-
commented, easily-tunable constant, chosen to approximate a typical monospace left-
side bearing so that `ink_left - pad` lands near the old advance-based left edge —
i.e. the fix preserves the chip's current left-edge appearance (product spec
Success Criterion #2) and reads as "the right edge got tightened," not "the chip was
redesigned." Unlike the earlier advance-fraction heuristic, this constant is *not*
standing in for a missing measurement (the ink edges are exact); mis-tuning it only
changes the uniform padding, and can never re-introduce the left/right asymmetry.
The exact fraction is a maintainer taste question (product spec Open Question #1).

### RTL / edge-glyph identification

The visible-span loop already determines `visible_left`/`visible_right` via
`.min()`/`.max()` over glyph positions regardless of draw order, so it is correct for
RTL runs. The fix captures, alongside those extrema, *which* glyph produced each edge
(its glyph id and paint-space origin) — order-independently — and derives that
glyph's ink for the corresponding edge. The rightmost edge uses the visually-
rightmost glyph's ink, not "the last glyph in iteration order," so RTL runs remain
correct.

### Fallback rules (no chip collapse)

`ink_left`/`ink_right` are `Option`s. The edge falls back to the advance-based
`visible_left`/`visible_right` when:

- the edge glyph has no ink — `glyph_raster_bounds` reports a zero-width box (e.g. a
  trailing space in a code span); or
- the raster-bounds lookup errors, or the scale factor is non-positive.

This guarantees the chip never collapses to zero/negative width on whitespace-
terminated spans, and degrades to today's advance-based behavior if ink is ever
unavailable.

### Composition with existing truncation clamping

The ink edges are derived from the *visible* first/last glyphs (the same glyphs the
span loop already clamps to under ellipsis/width truncation). The existing
`text_rect.intersection(visible_bounds)` clamp in `paint_run_background` still runs
after the rect is built, so a truncated run's background is still bounded to
`visible_bounds`. The prior-fix invariant pinned by
`test_run_background_clamped_to_visible_glyph_span` continues to hold (its expected
bounds are updated to the ink-edge geometry without weakening the truncation
assertion).

## Alternatives considered (documented dead ends)

- **`glyph_typographic_bounds` (the mis-analyzed API — do not re-propose):**
  `FontCache::glyph_typographic_bounds` (`crates/warpui_core/src/fonts.rs:377-393`)
  is a bounds-like measurement, but its two backends disagree on what they measure:
  - **macOS:** `crates/warpui/src/platform/mac/fonts.rs` calls font-kit
    `Font::typographic_bounds`, a font-design-space *typographic* bounding box
    (includes side bearings), not tight ink.
  - **Linux:** `crates/warpui/src/windowing/winit/fonts.rs` calls ttf-parser
    `Face::glyph_bounding_box`, the tight outline (ink) bounding box.

  Computing the chip span from this wrapper would make the chip's right-edge padding
  visibly different in *size* between macOS and Linux for identical text — trading
  one asymmetry (left vs. right) for another (platform-inconsistent). This is why the
  original spec concluded "ink is not evenly available." The conclusion was correct
  *for this wrapper only*; `glyph_raster_bounds` (above) is the right API and does
  have parity. This dead end is kept documented so reviewers don't re-propose
  `glyph_typographic_bounds`.

- **A new design-space ink API on macOS (CoreText
  `CTFontGetBoundingRectsForGlyphs`) — not adopted:** CoreText exposes per-glyph
  design-space ink rects via `CTFontGetBoundingRectsForGlyphs` (available in the
  `core-text` crate the repo depends on), which would be the design-space analog of
  ttf-parser's `glyph_bounding_box`, letting the chip span be computed in font units
  scaled by ppem with no device-pixel/subpixel dependence. It is **not used anywhere
  in the repo today** and is not adopted here, because `glyph_raster_bounds` already
  gives cross-platform ink parity with zero new plumbing. Adding a design-space ink
  path is a possible future refinement if the ~1px macOS rasterizer AA residual
  (below) ever proves visually significant.

## Risks and residuals

- **macOS right-edge AA fudge (~1 device px).** The macOS rasterizer deliberately
  inflates raster bounds by 1px on the right to avoid clipping anti-aliased glyphs
  (`crates/warpui/src/fonts/font_kit.rs:66-77`: `size + vec2i(1, 1)`, origin offset
  only vertical). The Linux swash path has no such fudge. So the derived `ink_right`
  reads up to ~1 device pixel wider on macOS than on Linux for identical text. This
  is an order of magnitude smaller than the advance gap being removed and is
  cosmetically negligible, but it is a real cross-platform residual flagged for
  visual sign-off (product spec Open Question #2). If ever unacceptable, the
  design-space CoreText path above removes it.
- **Italic/overhang glyphs — incidental improvement, not a regression.** For a glyph
  whose ink overshoots its advance box (e.g. an italic `f`), today's advance-based
  right edge can clip the overshoot; the ink-based right edge contains it. This is a
  correct-direction side effect.
- **Per-run independence (unchanged model).** Each run's background is already
  painted as an independent rect; the fix does not change how runs relate to each
  other, only how each run's own rect is positioned. There is no cross-run "sawtooth"
  because each chip's edges come from its own glyphs' ink (this supersedes the
  earlier advance-shift approach, which accepted a sawtooth trade-off — now moot).
- **Perf.** The extra `glyph_raster_bounds` calls are only for backgrounded/bordered
  runs (inline code), and only for the first and last visible glyph of each such run.
  The result is cached in `FontCache` keyed by `(GlyphKey, scale)`; the drawn glyphs
  are rasterized anyway, so the bounds are typically already computed. The cost is
  bounded and does not touch the common (non-backgrounded) text hot path.

## The second render surface: TUI Markdown (`warp_tui`) — unaffected

The ticket asks whether the fix needs to reach a second render surface. It does not,
because that surface does not use this paint path at all:

- `crates/warp_tui/src/tui_markdown.rs` defines `inline_code:
  builder.accent_text_style()` and applies it in `fragment_style` via
  `style = style.patch(palette.inline_code);`. `TuiStyle` here is `ratatui`'s cell
  style type (re-exported through `warpui_core::elements::tui::TuiStyle`), which
  carries foreground/background *colors and modifiers* applied per-terminal-cell —
  there is no `TextBorder`, no glyph-advance geometry, and no equivalent of
  `paint_run_background`'s sub-pixel horizontal span math. `TuiStyle` never receives a
  `background_color` for inline code in this file (only `accent_text_style()`'s
  foreground/modifier patch), so TUI inline code is foreground-color-only and has no
  chip background to be asymmetric in the first place.
- Compare the "real" (non-TUI) editor path, where
  `crates/editor/src/render/layout.rs:190-201` sets both `with_background_color(...)`
  and `with_border(TextBorder { .. })` for `text_styles.is_inline_code()` — that is
  the path reaching `warpui_core::text_layout::Line::paint_run_background` and
  affected by this bug. The notebook/plan editor and any other Markdown-consuming
  surface going through `RichTextStyleContext::style_and_font`
  (`crates/editor/src/render/layout.rs:165-214`) shares this single code path and is
  fixed by the same change.
- **Conclusion:** the fix, scoped to `warpui_core::text_layout::Line`, reaches every
  consumer of `TextStyle::background_color`/`TextStyle::border` uniformly (including
  the `editor` crate's Markdown inline-code rendering), but does **not** reach
  `warp_tui`, which renders through a separate cell-grid `ratatui` path. No change to
  `warp_tui` is proposed or required (see product spec Non-goals).

## Testing and validation

- **Update `test_run_background_clamped_to_visible_glyph_span`** to the ink-edge
  geometry: with ink-bearing glyphs, the painted rect's left/right come from the
  first/last *visible* glyph's ink (± `pad`), and the right edge must still be
  bounded to the visible glyph span (not the full run width) — the truncation-
  clamping invariant this test pins must still hold.
- **New unit test — ink edges used, not advance:** a mid-line single-glyph chip
  whose painted left edge is `ink_left - pad` and right edge is `ink_right + pad`,
  with the right edge proven strictly less than the advance endpoint
  (`glyph_x + advance`) — i.e. the trailing advance bearing is trimmed.
- **New unit test — zero-ink fallback:** a zero-ink edge glyph (whitespace) yields
  advance-edge ± `pad`, confirming the chip does not collapse.
- **New unit test — first/last glyph specifically:** a two-glyph run confirms the
  left edge comes from the first visible glyph's ink and the right edge from the
  *last* visible glyph's ink (edges key off their own glyph, not an aggregate; RTL-
  safe extrema selection).
- **Regression (isolation):** existing caret/selection tests relying on
  `x_for_index`, `width_for_index`, `caret_position_for_index`, and `index_for_x`
  are unaffected — they read `glyph.position_along_baseline`/`caret_positions`, which
  this fix does not modify. No new tests needed beyond confirming the suite passes;
  this is a "prove the isolation held" check.
- **Manual verification:** render mixed inline code spans ending in narrow-ink
  characters (`` `foo.bar()` ``, `` `x;` ``) beside wide-ink-ending spans
  (`` `foo_bar` ``) and confirm right-side padding now equals left-side padding, on
  both macOS and a Linux/winit build (also check the macOS AA residual, Open
  Question #2). Confirm the left edge did not visibly move (the padding calibration
  target). Check dark and light themes.
- `cargo fmt` and `cargo clippy --workspace --all-targets --all-features --tests
  -- -D warnings` must pass, per `CONTRIBUTING.md`.
- `cargo nextest run -p warpui_core -p warp_editor` (CI parity) must pass with the
  updated and new tests.

## Follow-ups

- **`block_padding` left-only asymmetry (new, found while speccing this fix — not
  fixed here).** As detailed in "Interaction with the legacy `block_padding`
  offset" above, `block_padding` (`font_size / 10.` when a border is present) only
  ever offsets the rect's *left* edge — it algebraically cancels out of the right
  edge in the existing `text_rect` construction
  (`crates/warpui_core/src/text_layout.rs:1401-1405`). This predates GH13914 and is
  independent of the advance-vs-ink bug this issue fixes, but it means a bordered
  inline-code chip's left padding is `font_size / 5` (≈1px at 13px) wider than its
  right padding even after this fix lands. Needs a maintainer decision on whether to
  apply `block_padding` to both edges (and the resulting visual footprint change for
  every bordered/backgrounded run, not just inline code) before it's addressed.
- If the ~1px macOS rasterizer AA residual (Open Question #2) ever proves visually
  significant, add a design-space per-glyph ink path (CoreText
  `CTFontGetBoundingRectsForGlyphs` to match the Linux ttf-parser outline semantics),
  removing the device-pixel/subpixel dependence entirely.
- If the fixed padding proves visually wrong for a specific inline-code font family
  whose advance-vs-ink ratio differs meaningfully from the UI font, consider sourcing
  the padding from that font's metrics rather than a single global fraction of
  `font_size`.
