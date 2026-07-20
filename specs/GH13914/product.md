# Product Spec: Inline code chip background extends past glyph ink on the right

**Issue:** [warpdotdev/warp#13914](https://github.com/warpdotdev/warp/issues/13914)
**Figma:** none provided

## Summary

Inline code chips (the pill-shaped background behind `` `code` `` spans in Markdown-rendered text) paint their background rectangle using each glyph's advance width, not its visual ink width. On the right edge of the chip, this leaves visible empty background color between the last glyph's ink and the chip's right border/edge — the chip looks asymmetric, with tighter padding on the left than on the right.

## Problem

When Warp renders Markdown inline code (for example, in agent conversation panes, README/chat message rendering, or any rich text using the `warpui_core` text layout system), the background/border decoration behind the code span is computed from the sum of each glyph's *advance width* — the distance the text cursor moves after drawing the glyph, which includes the font's built-in right-side bearing/spacing for that glyph. Advance width is deliberately wider than a glyph's visible ink so that adjacent glyphs don't visually collide; it is not designed to be a tight visual bounding box.

Because the chip's left edge is anchored at the first glyph's origin (no equivalent left-side padding is added or removed) while the right edge is anchored at the last glyph's advance-based endpoint, the chip is not visually balanced: the gap between the last character's ink and the chip's right edge is consistently larger than the gap between the chip's left edge and the first character's ink. This is most noticeable with narrow-ink final characters (e.g., `code)`, `foo;`, or any run ending near a period, comma, or narrow letter) where the extra advance spacing after the glyph becomes visible whitespace inside the colored chip.

This is purely a rendering/paint issue — it does not affect text selection, caret placement, copy/paste, or the underlying Markdown content, all of which continue to use the existing glyph geometry unchanged.

## Goals

- The inline code chip's background/border no longer shows a visually larger gap on the right side of the chip than on the left.
- The fix preserves left-edge (leading) alignment of code chips relative to surrounding text and other chips on the same line — no chip should visually shift its starting position.
- The fix applies uniformly regardless of platform (macOS/CoreText or Linux-winit/cosmic-text-cum-ttf-parser text layout backends), since both currently source glyph advance width the same way into the shared paint path.

## Non-goals

- Re-deriving the background span from true per-glyph ink/visual bounding boxes. Investigation found this alternative is not evenly supported: the macOS backend's available "typographic bounds" API returns font-design bounds (not tight ink bounds), while the Linux/cosmic-text backend's equivalent returns the true glyph outline bounding box — the two platforms would visually disagree on chip width if we measured ink per platform. This is documented as an alternative in `tech.md`, not adopted here.
- Changing the border radius, border width, block padding, or color of inline code chips (only the horizontal positioning of the existing background/border rectangle is affected).
- Changing font metrics, hinting, ligature shaping, or any other text layout behavior unrelated to the background/border paint step.
- Any change to the terminal UI (TUI) Markdown renderer (`crates/warp_tui/src/tui_markdown.rs`). The TUI renders inline code with a foreground-color-only style patch (no `background_color`/border) over `ratatui`'s per-cell grid, which has no advance-vs-ink distinction to begin with — this issue does not reproduce there and no change is proposed for it.
- Fixing analogous background/border asymmetry for other backgrounded text styles outside inline code (e.g., search-match highlights), unless they share the exact same paint path and are trivially covered by the same fix — see `tech.md` for confirmation of blast radius.

## User experience

### Current behavior (broken)

1. Warp renders a chat/agent message (or any rich-text surface) containing inline code, e.g. `` This is `code`. ``.
2. The code chip's background is visibly wider on the right of the last glyph (`e`) than the gap on the left before the first glyph (`c`).
3. This asymmetry is most visible with narrow trailing glyphs and is a subtle but persistent visual defect across every inline code span in every rendered Markdown surface.

### Expected behavior (after fix)

1. The same message renders with the code chip's background visually centered around the glyph ink, i.e. the left and right padding read as equal (or as close to equal as achievable without shifting the chip's leading edge — see trade-off below).
2. The chip's left edge remains anchored at the same horizontal position as before (no shift in leading alignment), so code spans at the start of a line, or immediately following another inline element, do not move.
3. Known, accepted trade-off (preferred direction, see `tech.md` "Chosen approach"): because the fix shifts the *painted background rectangle* leftward by half of the per-glyph advance padding without changing glyph or caret positions, two vertically-stacked code chips of different text can display a subtle "sawtooth" — their right edges will not perfectly line up column-to-column the way their (unchanged) advance widths would suggest, because the padding-based shift is applied independently per run. This is judged an acceptable trade-off versus shifting the visible text rightward, which would break leading-alignment expectations for code chips that start a line.
4. Also expected: shifting the background only (not the glyph or caret positions) means the extra space that this fix "gives back" was, and remains, the space between two words — e.g. in `This is `code``, the visual glyph gap between "is" and "`code`" is unchanged; only the region of that gap that renders as chip-background color moves.

### Edge cases

- **Empty code span (`` `` ``):** no glyphs to measure; the chip should not attempt to paint a malformed or negative-width background. Existing empty-run handling (no first glyph → no background painted) is preserved.
- **Ligatures:** a run with a shaped ligature (e.g. a code font ligature for `->` or `!=`) has one glyph covering multiple characters; the ligature's own advance-width padding is treated the same as any other glyph's — no special-casing is introduced or required.
- **Right-to-left (RTL) text inside a code span:** the fix must not assume left-to-right glyph ordering when determining which edge is "leading" vs "trailing." See `tech.md` for how `visible_left`/`visible_right` are computed order-independently today, and how the fix preserves that.
- **Emoji or wide glyphs inside code spans:** wide glyphs (e.g. emoji) typically have less disproportionate advance-vs-ink padding relative to their size, so the visual effect is smaller, but the same halving logic applies uniformly; no separate threshold or cutoff is introduced.
- **Truncated/ellipsized code chip (chip cut off by line-width clipping):** the existing `visible_left`/`visible_right` clamping to the drawn glyph span (added for a prior fix, see `test_run_background_clamped_to_visible_glyph_span`) continues to apply; this fix composes with that clamping rather than replacing it.
- **Chip is the border style only (background color absent, e.g. some future style variant):** the fix applies to the horizontal span computation shared by both border and background painting, so both are shifted consistently and remain visually aligned with each other.

## Success criteria

1. For a representative set of inline code spans ending in narrow-ink characters (`.`, `,`, `i`, `l`, `)`, `;`), the visually-measured right-side chip padding is within a small, consistent tolerance of the left-side chip padding (exact tolerance defined in `tech.md`).
2. The chip's left edge does not move by more than a sub-pixel rounding amount compared to before the fix, for any code span (verified by a geometry-level test, not just visual inspection).
3. No regression to caret placement, text selection, or copy/paste behavior for text inside or adjacent to inline code spans (these are driven by `caret_positions`/glyph `position_along_baseline`, which this fix does not touch).
4. No change to the TUI Markdown rendering of inline code (confirmed unaffected, per Non-goals).
5. Existing test `test_run_background_clamped_to_visible_glyph_span` in `crates/warpui_core/src/text_layout_tests.rs` continues to pass (updated if necessary to reflect the new geometry, without weakening its truncation-clamping assertion).

## Open questions

1. **Preferred implementation (this spec's recommendation, per issue author):** shift the painted background/border rectangle left by half of the available per-run advance-vs-ink padding, keeping the glyph/caret geometry untouched. This accepts the sawtooth trade-off described above. See `tech.md` "Chosen approach" for the precise padding source and formula, since there is no existing per-glyph ink measurement to subtract from — the spec needs maintainer sign-off on the specific padding heuristic proposed in `tech.md` before implementation.
2. Should this fix also apply to any other `TextStyle::border`/`background_color` consumer beyond inline code (e.g., is there another caller of this same paint path where the identical asymmetry would be user-visible)? `tech.md` documents what was found; a maintainer should confirm whether any of those are in scope.
3. Is the accepted sawtooth trade-off (Option A) acceptable as the final shipped behavior, or should a future follow-up explore the "split the difference" compromise (Option C, described in `tech.md`) once the padding heuristic is refined? Not blocking for this spec, but worth deciding explicitly rather than by default.
