# Tech Spec: Horizontal scrolling per terminal pane (GH-9828)

> **Spec convention.** This file is the technical half of the
> two-file spec convention `specs/GH<issue>/{product.md, tech.md}`.
> See `specs/GH9828/product.md` for the product requirements. No
> `SPEC.md` exists for this issue — downstream spec-discovery
> tooling should locate this spec via the `product.md` + `tech.md`
> pair.

## Behavior contract

- B1. Horizontal scrolling is gated on:
  - The pane's `terminal.line_wrap = false` setting (existing).
  - At least one visible row exceeding pane width.
  When either condition is false, the pane behaves as today (no
  horizontal scroll, no scrollbar).
- B1a. **Terminology.** This spec uses two strictly distinct
  operations on `h_offset`:
  - **Clamp** — re-evaluate `h_offset` against the current valid
    range `[0, max_valid]` where
    `max_valid = max(max_observed_columns - viewport_columns, 0)`.
    A clamp may **incidentally** produce the value `0` when
    `max_valid == 0`, but this is not a "reset"; it is the
    boundary value of the clamp range and is treated identically
    to any other clamp outcome.
  - **Reset** — unconditionally set `h_offset := 0` regardless
    of the current valid range. Reset is triggered only by the
    explicit, exhaustive list in
    "Reset to 0 ONLY on explicit triggers" below.
  Every rule in this spec that affects `h_offset` is one of these
  two operations and nothing else. There is no third "reset
  because the range collapsed" operation; range collapse is
  handled by the clamp rule.

  `h_offset` is a logical column offset clamped to
  `max(visible_longest_line_columns - visible_pane_columns, 0)`.
  Recompute that maximum whenever the visible buffer range changes,
  the pane resizes, line-wrap toggles, or rendered line contents
  change. The general clamp rule is:
  ```
  h_offset := clamp(h_offset, 0,
                    max(max_observed_columns - viewport_columns, 0))
  ```
  - **Boundary case (max-cleared clamp).** When
    `max(max_observed_columns - viewport_columns, 0) == 0`, the
    valid range collapses to `{0}` and the general clamp above
    forces `h_offset` to 0. The horizontal scrollbar is hidden.
    This is **not a separate "reset" rule** — it is the general
    clamp applied to its boundary value. The list of reset
    triggers in "Reset to 0 ONLY on explicit triggers" below is
    the canonical, exhaustive set; the max-cleared clamp does
    NOT appear there because it is a clamp, not a reset. When
    `max_observed_columns - viewport_columns` later becomes
    positive again, the offset is recomputed by the
    preserve-and-restore rule for wrap toggles, or remains at
    the current clamped value otherwise.
  - **Wrap-on suspension.** If line-wrap is toggled ON,
    **preserve `h_offset` internally** without applying it to
    rendering (the offset is internal state while wrap is on).
    The horizontal scrollbar is hidden. See
    "Line-wrap toggle: preserve-and-restore" below.
  - **Shrink clamp.** If the maximum shrinks but remains
    positive, clamp `h_offset` down to the new maximum so blank
    shifted columns are never rendered.
- B2. Horizontal scroll inputs:
  - Trackpad two-finger horizontal swipe (native macOS, Windows,
    and Linux). On Linux, two-finger horizontal swipe is sourced
    from libinput's horizontal scroll axis, matching the existing
    Warp Linux trackpad gesture support. If a Linux platform/session
    does not deliver horizontal-axis events from the windowing system
    (e.g. constrained Wayland compositors, some X11 driver setups),
    Shift + scroll wheel and the horizontal scrollbar drag remain the
    documented fallback inputs and the behavior contract is otherwise
    unchanged.
  - Shift + scroll wheel.
  - A new horizontal scrollbar at the bottom of the pane,
    visible only when content exceeds pane width.
- B3. Scroll position is per-pane and persists across the pane's
  lifetime (resets on pane close).
- B4. Vertical scroll behavior is unchanged. The two axes are
  independent.
- B5. Selection and click-to-position respect the horizontal
  offset — the user clicks where they see, not where the
  underlying buffer column is. Hit-testing uses the same terminal
  grid column model as rendering (see B8).
- B6. Auto-scroll on new output: the pane snaps back
  to horizontal offset 0 only when all of these are true:
  - `terminal.horizontal_scroll.snap_on_new_output = true`
    (per-pane override `pane.horizontal_snap_on_new_output` takes
    precedence when set).
  - The terminal is pinned to the live bottom when the output event
    is applied, not viewing scrollback.
  - The pane is on the primary screen, not the alternate screen.
    Alternate-screen / TUI applications (vim, htop, less, etc.) own
    their column model and never trigger snap.
  - The output event commits a completed rendered row strictly
    below the current bottom-of-buffer position — i.e. a true
    line-append. Tailing logs that emit `\n`-terminated full lines
    DO trigger snap. Excluded: partial writes to the current row
    (no `\n` yet), CR-then-rewrite prompt redraws (cursor returns
    to column 0 of the same row), in-place line edits via `\x1b[K`
    or other erase sequences, cursor-only updates, and any output
    that arrives while the user is viewing scrollback.
  Implementation hint: distinguish "new line committed below
  viewport" from "in-place line update" by tracking the cursor row's
  relationship to bottom-of-buffer at the moment the output event is
  applied — only transitions that move the committed bottom row
  index forward count as line-append.
- B7. Persist `terminal.horizontal_scroll.snap_on_new_output` in the
  existing `TerminalSettings` settings group
  (`app/src/terminal/settings.rs`) with TOML path
  `terminal.horizontal_scroll.snap_on_new_output`, boolean,
  default `true`, with the same global sync behavior as other
  non-private terminal settings. V1 exposes it through the
  settings file/schema and surfaces a first-class toggle under
  Settings → Terminal → Scrolling labelled "Snap horizontal scroll
  on new output". A per-pane override is available via tab/pane
  config `pane.horizontal_snap_on_new_output` (boolean, optional;
  when present, takes precedence over the global setting for that
  pane).
- B8. Offset and column model. `h_offset` is measured in TERMINAL
  GRID COLUMNS, not characters or bytes. The column model matches
  Warp's existing terminal grid:
  - Wide glyphs (CJK, emoji, double-width box drawing) occupy 2
    grid columns.
  - Combining marks and zero-width joiners occupy 0 grid columns
    (they attach to the preceding cell).
  - Tabs expand to the next tab stop based on the terminal's
    existing tab-width configuration.
  An `h_offset` of N columns scrolls each rendered row so that grid
  column N is the leftmost visible column. If column N falls inside
  a wide glyph (i.e. the glyph started at column N-1 and spans
  columns N-1 and N), the partially-clipped left edge renders per
  the existing terminal grid render rules — typically a half-block
  or whitespace placeholder cell — not an attempt to redraw the
  truncated half of the glyph. Selection, hit-testing, and the
  scrollbar thumb all use this same column model so the offset →
  buffer-column mapping is bijective for whole-cell positions and
  consistently rounded for fractional positions.

### Offset clamping & reset rules

Per-pane horizontal scroll state is `(h_offset, max_observed_columns)`
where `max_observed_columns` is the longest visible row's grid-column
length within the current visible buffer range. Clamping rules:

- On pane resize: re-clamp via the general clamp rule in B1a.
  The boundary case (no row exceeds pane width) hides the
  scrollbar and forces `h_offset = 0` per the B1a max-cleared
  clamp.
- On vertical scroll: do NOT auto-reset `h_offset` to 0. Recompute
  `max_observed_columns` against the new visible window's longest
  line, then apply the B1a general clamp. Auto-reset to 0 happens
  only when explicitly required by the snap setting (B6) — never
  silently on vertical scroll.

### Line-wrap toggle: preserve-and-restore (uniform invariant)

- **Wrap ON.** When line-wrap is toggled ON, the horizontal
  scrollbar is hidden and `h_offset` is NOT applied to rendering
  (every row rewraps inside the viewport). The previous
  `h_offset` value is **preserved internally** as part of the
  pane's per-pane horizontal scroll state.
- **Wrap OFF.** When line-wrap is toggled OFF again, the
  preserved `h_offset` is **restored** and re-clamped against
  the current `max_observed_columns` using the same general
  clamp rule from B1a:
  ```
  h_offset := clamp(preserved_h_offset, 0,
                    max(max_observed_columns - viewport_columns, 0))
  ```
  If the current `max_observed_columns` is smaller than the
  preserved offset's required range, the offset clamps down (it
  is never silently snapped to 0 unless the clamp resolves to
  0 because nothing exceeds the viewport — i.e. the B1a
  max-cleared boundary clamp).
- **Reset to 0 ONLY on explicit triggers.** `h_offset` is reset
  to 0 only when one of the following fires:
  1. The user explicitly scrolls to the leftmost column (manual
     reset via input).
  2. Snap-on-new-output triggers per B6 (line-append while
     pinned to live bottom, primary screen, snap setting on).
  3. The pane is closed (state discarded with the pane).
  Toggling line-wrap is NOT one of these triggers — wrap-toggle
  uses preserve-and-restore semantics, never a reset. The B1a
  max-cleared clamp is also NOT a reset trigger — it is the
  general clamp applied at its boundary. This list is the
  canonical, exhaustive set of reset triggers.
- **Rationale.** A user who scrolled horizontally to inspect
  long output, then toggled wrap on for a moment to re-read
  with wrapping, expects to land back where they were when
  wrap is toggled off again. Resetting to 0 on wrap toggle
  loses that position and is a regression of intent.
- On long-line set changes (rows appended, edited in place, or
  evicted from scrollback): recompute `max_observed_columns` against
  the current visible buffer range and apply the B1a clamp.
  Never render blank shifted columns.

## Implementation pointers

- Terminal grid render lives in `app/src/terminal/...`. The grid
  already tracks per-pane viewport state; horizontal offset is a
  new field there.
- Selection logic in `app/src/terminal/model/blocks/selection.rs`
  uses buffer coordinates (column index), so the horizontal-offset
  → buffer-column mapping is the only adjustment in the click /
  drag path.
- Add the snap setting beside other terminal settings in
  `app/src/terminal/settings.rs`, using generated settings schema
  and TOML persistence rather than a custom config path.

## Test plan

- T1. Render with a long line + line-wrap-off + h-offset = 5;
  rendered cells start at column 5.
- T2. Two-finger horizontal swipe event increments h_offset.
- T3. Selection across an h-scrolled region returns logical text.
- T4. While pinned to live bottom, completed new output snaps back
  to offset 0 with the default snap setting.
- T5. With line-wrap on, h-scroll inputs are ignored.
- T6. With snap disabled, completed new output preserves h_offset.
- T7. Resize and vertical-scroll to shorter content clamp
  `h_offset` to the new max and hide the scrollbar when max
  offset is 0. Vertical scroll to shorter content NEVER resets
  to 0 silently (per the offset-clamping rules).
- T_line_wrap_toggle_round_trip. **Wrap-toggle preserve-and-
  restore.** Set `h_offset = 120` with line-wrap OFF. Toggle
  line-wrap ON: scrollbar hides; rendering rewraps inside
  viewport; pane state still records the preserved offset of
  120. Toggle line-wrap OFF again. The post-toggle value of
  `h_offset` is asserted with **exact equality (`==`), never
  inequality (`>=`)**. Inequality assertions like `>=` are
  banned in this test (and in T_line_wrap_toggle_clamps_on_shrink,
  T_wrap_toggle_no_reset below) because an inequality would
  pass even if the implementation silently snapped to a smaller
  value — which is exactly the regression these tests must
  catch. Note that the **preconditions** below use comparisons
  like "at least" / "less than" to describe the test fixture
  state; the **assertions** in each case are strict `==`:
  - **Case A — preserved offset fits the current range**
    (precondition: `max_valid` is at least 120, where
    `max_valid = max(max_observed_columns - viewport_columns, 0)`).
    Assertion: `h_offset == 120` exactly. The preserved offset
    is restored unchanged.
  - **Case B — preserved offset exceeds the current range,
    but the range is non-empty** (precondition: `max_valid`
    is greater than 0 and strictly less than 120; the longest
    line shrank while wrap was on but the viewport still has a
    long row). Assertion: `h_offset == max_valid` exactly. The
    preserved offset is clamped down to the new `max_valid`,
    no further.
  - **Case C — current range collapsed to {0}** (precondition:
    `max_valid == 0`; longest line now fits viewport).
    Assertion: `h_offset == 0` exactly. This is the
    max-cleared clamp from B1a, not a reset trigger; the
    preserved value is held in pane state in case the longest
    line later regrows under wrap toggles or new output.
  All three case assertions use `==`. The scrollbar's
  visibility follows from whether `max_valid` is greater
  than 0.
- T_line_wrap_toggle_clamps_on_shrink. Set `h_offset = 200`
  with `max_observed_columns - viewport_columns = 220`. While
  wrap is ON, content evicts such that
  `max_observed_columns - viewport_columns = 80`. Toggle wrap
  OFF: `h_offset` is restored and clamped to 80, not 200.
  Assertion: `h_offset == 80` exactly. Inequality assertions
  (e.g. `h_offset >= 80`, `h_offset <= 200`) are forbidden —
  they would pass even if the implementation silently snapped
  to 0.
- T_wrap_toggle_no_reset. Set `h_offset = 50`. Toggle wrap on
  then off five times. After every cycle, assert `h_offset == 50`
  (clamped to current max). The toggle is never a reset.
- T8. Partial-row writes, prompt redraws (CR-then-rewrite), in-place
  line edits via `\x1b[K`, alternate-screen updates, and output
  received while viewing scrollback all preserve h_offset.
- T9. Wide glyph (CJK / emoji), combining mark, and tab column
  accounting: with h_offset landing inside a wide glyph, the
  partially-clipped left edge renders per existing grid rules and
  selection at that column resolves to the correct logical buffer
  column.
- T10. Per-pane override: when `pane.horizontal_snap_on_new_output`
  is set, it overrides the global
  `terminal.horizontal_scroll.snap_on_new_output` for that pane
  only; other panes follow the global setting.
- T11. Linux trackpad: with libinput horizontal-axis events
  available, two-finger swipe scrolls; with horizontal events
  absent, Shift+wheel and scrollbar drag scroll and the gesture
  is silently ignored (no errors).
