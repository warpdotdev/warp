# Product Spec: Inline code chip background extends past glyph ink on the right

**Issue:** [warpdotdev/warp#13914](https://github.com/warpdotdev/warp/issues/13914)
**Figma:** none provided

## Summary

Inline code chips (the pill-shaped background behind `` `code` `` spans in Markdown-rendered text) paint their background rectangle using each glyph's advance width, not its visual ink width. On the right edge of the chip, this leaves visible empty background color between the last glyph's ink and the chip's right border/edge — the chip looks asymmetric, with tighter padding on the left than on the right.

The fix stops deriving the chip's horizontal edges from advance width. Instead it anchors them to the first and last visible glyph's true **ink** extents, plus a fixed design padding on each side. Interior glyph positions remain advance-based (that is the text itself, unchanged). This is a paint-time-only change to the background/border rectangle; it does not affect text selection, caret placement, copy/paste, or the underlying Markdown content.

## Problem

When Warp renders Markdown inline code (for example, in agent conversation panes, README/chat message rendering, or any rich text using the `warpui_core` text layout system), the background/border decoration behind the code span is computed from each glyph's *advance width* — the distance the text cursor moves after drawing the glyph, which is the sum of the glyph's left-side bearing, its ink, and its right-side bearing. Advance width is a spacing metric for positioning the *next* glyph, deliberately wider than a glyph's visible ink so adjacent glyphs don't collide; it is not a visual bounding box.

Because the chip's left edge is anchored at the first glyph's origin (advance origin, roughly one left-side bearing to the left of the ink) while the right edge is anchored at the last glyph's advance endpoint (roughly one right-side bearing past the ink), the chip is not visually balanced: the gap between the last character's ink and the chip's right edge is consistently larger than the gap between the chip's left edge and the first character's ink. This is most noticeable with narrow-ink final characters (e.g., `code)`, `foo;`, or any run ending near a period, comma, or narrow letter) where the extra advance spacing after the glyph becomes visible whitespace inside the colored chip.

This is purely a rendering/paint issue — it does not affect text selection, caret placement, copy/paste, or the underlying Markdown content, all of which continue to use the existing glyph geometry unchanged.

## Goals

- The inline code chip's background/border no longer shows a visually larger gap on the right side of the chip than on the left; both sides read as a fixed, equal design padding around the code's ink.
- The chip's left-edge (leading) appearance is preserved relative to today's rendering — the fix is calibrated to keep the left edge where it currently sits, so no chip visually shifts its starting position and the change reads as "the right edge got tightened," not "the chip was redesigned."
- The fix applies uniformly regardless of platform (macOS/CoreText or Linux-winit/cosmic-text text layout backends), because it sources per-glyph ink from a single measurement that both backends already implement with matching semantics (see `tech.md`).

## Non-goals

- Changing the border radius, border width, block padding, or color of inline code chips (only the horizontal positioning of the existing background/border rectangle is affected).
- Changing font metrics, hinting, ligature shaping, glyph draw positions, or caret geometry — the ink measurement is read *only* to position the background/border rectangle; nothing about how text is laid out or where the caret lands changes.
- Any change to the terminal UI (TUI) Markdown renderer (`crates/warp_tui/src/tui_markdown.rs`). The TUI renders inline code with a foreground-color-only style patch (no `background_color`/border) over `ratatui`'s per-cell grid, which has no advance-vs-ink distinction to begin with — this issue does not reproduce there and no change is proposed for it (confirmed in `tech.md`).
- Fixing analogous background/border asymmetry for other backgrounded text styles outside inline code (e.g., search-match highlights), unless they share the exact same paint path and are trivially covered by the same fix — see `tech.md` for confirmation of blast radius.
- Adding a *design-space* per-glyph ink API (e.g. a new CoreText `CTFontGetBoundingRectsForGlyphs` binding). The fix uses the already-plumbed rasterized-ink measurement; a design-space alternative is noted in `tech.md` as a possible future refinement, not adopted here.

## User experience

### Current behavior (broken)

1. Warp renders a chat/agent message (or any rich-text surface) containing inline code, e.g. `` This is `code`. ``.
2. The code chip's background is visibly wider on the right of the last glyph (`e`) than the gap on the left before the first glyph (`c`).
3. This asymmetry is most visible with narrow trailing glyphs and is a subtle but persistent visual defect across every inline code span in every rendered Markdown surface.

### Expected behavior (after fix)

1. The same message renders with the code chip's background hugging the code's ink, with a fixed, equal design padding on the left and right. The left and right padding read as equal.
2. The chip's left edge remains at (approximately) the same horizontal position as before (the fixed padding is calibrated to preserve the current left-edge look), so code spans at the start of a line, or immediately following another inline element, do not move.
3. Because each chip's edges are derived from its own glyphs' ink, chips are independently correct — there is no cross-chip "sawtooth." (An earlier candidate approach shifted the whole rect by a fraction of advance and accepted a subtle sawtooth in vertically-stacked chips; the ink-edge approach makes that trade-off moot because each chip gets its own exact correction.)
4. The extra space the fix "gives back" on the right was, and remains, the space between two words — e.g. in `This is `code``, the visual glyph gap between "is" and "`code`" is unchanged; only the region of that gap that renders as chip-background color moves.

### Edge cases

- **Empty code span (`` `` ``):** no glyphs to measure; the chip should not attempt to paint a malformed or negative-width background. Existing empty-run handling (no first glyph → no background painted) is preserved.
- **Zero-ink edge glyph (e.g. a trailing space inside a code span):** a glyph with no ink has no ink edge to snap to. The affected edge (left or right) falls back to that glyph's advance-based edge, so the chip never collapses to zero or negative width on whitespace-terminated spans. See `tech.md` for the exact fallback rule.
- **Ink overshooting the advance (e.g. an italic `f` whose ink extends past its advance box):** ink-based edges contain such overshoot naturally. Note that today's advance-based right edge can actually *clip* such overshoot; the ink-edge fix removes that incidental clipping. This is a small, correct-direction side effect, not a regression.
- **Ligatures:** a run with a shaped ligature (e.g. a code font ligature for `->` or `!=`) has one glyph covering multiple characters; its ink is measured the same as any other glyph's — no special-casing is introduced or required.
- **Right-to-left (RTL) text inside a code span:** the fix must not assume left-to-right glyph ordering when determining which edge is "leading" vs "trailing." The visually-leftmost and visually-rightmost glyphs are identified order-independently (see `tech.md`), and their ink is what drives the left/right edges.
- **Emoji or wide glyphs inside code spans:** wide glyphs typically have less disproportionate advance-vs-ink padding relative to their size, so the visual effect is smaller, but ink measurement applies uniformly; no separate threshold or cutoff is introduced.
- **Truncated/ellipsized code chip (chip cut off by line-width clipping):** the existing `visible_left`/`visible_right` clamping to the drawn glyph span (added for a prior fix, see `test_run_background_clamped_to_visible_glyph_span`) continues to apply; the ink edges are derived from the *visible* first/last glyphs, and the existing intersection clamp still bounds the painted rect to `visible_bounds`.
- **Chip is the border style only (background color absent, e.g. some future style variant):** the fix applies to the horizontal span computation shared by both border and background painting, so both are positioned consistently and remain visually aligned with each other.

## Success criteria

1. For a representative set of inline code spans ending in narrow-ink characters (`.`, `,`, `i`, `l`, `)`, `;`), the visually-measured right-side chip padding closely approximates the left-side chip padding (both are anchored to the same fixed design padding around the ink) — the multi-pixel advance-vs-ink gap this issue targets is eliminated. A small residual (~1px at typical UI font sizes) remains on the left edge from a separate, pre-existing `block_padding` behavior unrelated to this issue (see `tech.md` "Interaction with the legacy `block_padding` offset" and Open Question #4); this residual is not eliminated by this fix.
2. The chip's left edge does not move by more than a sub-pixel amount compared to before the fix, for any code span (the fixed padding is calibrated to preserve the current left edge; verified by a geometry-level test, not just visual inspection).
3. No regression to caret placement, text selection, or copy/paste behavior for text inside or adjacent to inline code spans (these are driven by `caret_positions`/glyph `position_along_baseline`, which this fix does not touch).
4. No change to the TUI Markdown rendering of inline code (confirmed unaffected, per Non-goals).
5. Existing test `test_run_background_clamped_to_visible_glyph_span` in the `warpui_core` text-layout tests continues to pass (updated to reflect the new ink-edge geometry, without weakening its truncation-clamping assertion).
6. A zero-ink trailing glyph (e.g. a code span ending in a space) still paints a well-formed chip via the advance-edge fallback (verified by a unit test).

## Open questions

1. **Padding value (design/taste, not correctness).** The chip's per-side padding is a fixed fraction of the font size — currently `font_size / 12`, chosen to approximate a typical monospace left-side bearing so the padded ink-left lands near today's advance-based left edge (preserving the current left-edge look, per Goal/Success #2). This is a single, well-commented, easily-tunable constant. The exact fraction is a maintainer taste question (how much "air" around inline code reads best) and merits a visual sign-off, but — unlike the earlier advance-fraction heuristic — it is no longer standing in for a missing measurement: the ink edges themselves are exact (see `tech.md`), so mis-tuning this constant only changes the uniform padding, never re-introduces the left/right asymmetry.
2. **macOS right-edge AA residual (visual sign-off).** The rasterized-ink measurement on macOS includes a deliberate ~1px anti-aliasing fudge on the right that the Linux backend does not (see `tech.md`). This makes the right ink edge read up to ~1 device pixel wider on macOS than on Linux for identical text — far smaller than the advance gap being removed, but worth a cross-platform visual check before merge.
3. **Scope: other `TextStyle::border`/`background_color` consumers.** Should the ink-edge treatment apply to any backgrounded/bordered run beyond inline code (e.g. a hypothetical search-match highlight sharing this paint path)? Because the fix lives in the shared span computation, it already reaches every consumer of this path uniformly; `tech.md` documents what was found. A maintainer should confirm whether that uniform reach is desired for all such consumers or should be scoped to inline code specifically.
4. **Pre-existing `block_padding` left-only offset (found while speccing this fix, not caused by it).** Independent of the advance-vs-ink bug this issue targets, the existing `block_padding` (kerning-compensation) term only offsets the chip's left edge — it algebraically cancels out of the right edge in the current rect construction (see `tech.md`). This leaves a small residual left/right imbalance (~1px at typical UI font sizes) even after this fix, on every bordered inline-code chip. This fix does not correct it (out of scope; changing `block_padding`'s application changes the visual footprint of every bordered/backgrounded run in the codebase, not just inline code). A maintainer should decide whether to file this as a separate, explicitly-scoped follow-up fix.
