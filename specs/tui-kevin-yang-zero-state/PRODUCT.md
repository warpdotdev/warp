# TUI Zero State: Kevin Yang Face Animation

## Summary

Replace the right-panel starfield animation in the TUI zero state with an animated ASCII art portrait of Kevin Yang — rendered entirely in terminal characters, animating continuously while the zero state is visible, and dismissing cleanly when the user submits their first command.

## Figma

Figma: none provided. Reference image: photo of Kevin Yang (young man in suit and glasses, ivy background) supplied by the user.

## Behavior

### Portrait rendering

1. The right-side animation panel of the TUI zero state displays a grayscale ASCII art portrait of Kevin Yang, generated from the reference photo. The portrait fills the available panel dimensions, scaling proportionally to fit within the terminal cell grid (respecting the ~2:1 cell height-to-width aspect ratio).

2. The portrait is rendered using a ramp of ASCII/Unicode characters ordered from light to dark (e.g. ` `, `.`, `:`, `-`, `=`, `+`, `*`, `#`, `@`, `█`) to approximate luminance regions from the source image. Dark regions (hair, suit jacket, glasses frames) map to denser characters; light regions (face, shirt, background ivy) map to sparser characters.

3. The portrait is statically faithful to the source image's composition: the face is centred in the panel, the glasses are visible as distinct character bands, the suit collar and tie are present in the lower third, and the ivy background is represented in the surrounding region.

4. On terminals narrower than the minimum animation panel width (defined as the same minimum used by the starfield today), the portrait panel renders nothing — same zero-width fallback as the current starfield.

5. On terminals taller or wider than the portrait's natural cell count, the portrait is centred within the panel with blank cells on all sides; it does not stretch beyond its source aspect ratio.

### Animation

6. The portrait animates continuously while the zero state is visible. The animation does not make the face move; instead it applies one or more of the following layered effects that run at ~30 fps (matching the current starfield repaint cadence):

   a. **Scanline shimmer**: a horizontal band of slightly brightened characters (~2–4 rows tall) sweeps slowly downward from the top of the portrait to the bottom, then repeats. The shimmer modifies the characters in the band one brightness step brighter than their base value for the duration of the pass.

   b. **Glyph flicker**: a small percentage (~3–5%) of portrait cells independently swap to an adjacent character in the brightness ramp each frame, producing a subtle "static" or "live signal" texture across the entire face. Cells reset toward their base luminance value after flickering.

   c. **Accent highlight**: a small number of cells (~1–2%) in the lighter luminance regions (face, shirt) are momentarily coloured with the terminal's accent colour (the same colour used for starred stars in the starfield) before fading back to white/default. This matches the visual language of the existing starfield glow effect.

7. The animation effects are additive and independent — scanline shimmer, glyph flicker, and accent highlights all run simultaneously.

8. The animation runs for as long as the zero state is visible. When the user submits a command and the zero state dismisses, all animation stops cleanly with no trailing artefacts.

9. When the zero state reappears (transcript empties), the animation restarts from its initial state (scanline at top, flicker seed reset).

### Responsive behaviour

10. When the terminal is resized while the zero state is visible, the portrait re-renders at the new cell dimensions within one repaint frame. The animation continues uninterrupted through the resize — there is no flash or jump.

11. Portrait cell dimensions are recalculated on each resize. The same source image data is always used regardless of terminal size; the mapping from image pixels to character cells is recomputed from scratch to avoid accumulated scaling artefacts.

12. If the panel is resized below the minimum animation dimensions, the portrait disappears immediately (no partial render). When the panel grows above minimum again, the portrait reappears at the next repaint.

### Coexistence with the left column

13. The portrait occupies only the right-side animation panel. The left text column (title, version, changelog, project context, MCP status) is unaffected — layout, content, and update behaviour are identical to the current zero state.

14. The maximum panel width cap (`MAX_ANIMATION_COLS`) that limits the starfield today applies equally to the portrait panel. On very wide terminals, the portrait centres within that capped width.

### Accessibility and focus

15. The animation panel is purely decorative and not focusable. Screen readers and accessibility tooling receive no additional content from the animation region — the portrait characters are not announced.

16. If the user's terminal does not support colour (e.g. `$TERM` reports `dumb` or colour capability is absent), accent highlight effects are suppressed and the portrait renders in plain character art without colour. Scanline shimmer and glyph flicker are still applied via character substitution only.

### Transition from the starfield

17. The starfield animation is fully replaced by the Kevin Yang portrait. There is no mode-switch, setting, or feature flag to toggle between the two. The starfield code is removed.

18. The new animation must not regress any existing zero-state behaviour: changelog loading, MCP status updates, project context discovery, and autoupdate status all continue to trigger re-renders of the left column independently of the portrait animation on the right.
