# TECH: TUI Warping Indicator (CODE-1816)

## Context

While an agent turn is running, the TUI shows no in-progress indicator. Per the Figma design ([TUI file, node 323-17216](https://www.figma.com/design/yg5nbPZuGoAszHS3Rhvehu/TUI?node-id=323-17216&m=dev)), a `⋮ Warping (1s)` row should render at the bottom of the streaming agent block, directly above the input box:

- Layout: `<spinner> <Warping> <(Ns)>` on one row, one space (1 cell) between parts, one blank row above (the existing inter-section padding).
- Spinner: cycles through `⋮ ⋰ ⋱ ⋮ ⋱ ⋰` (from the design's 7 CharAni variants; the 7th is `⋮`, the loop point), non-bold, ANSI normal yellow.
- "Warping": bold, base ANSI normal yellow, with a shimmer band sweeping left→right whose peak is ANSI bright white. The Figma hex values (`#fefdc2`, `#feffff`, `#8e8e8e`) are exactly the dark theme's `normal.yellow` / `bright.white` / `bright.black` ([`app/src/themes/default_themes.rs:11-30 @ fbe26f9`](https://github.com/warpdotdev/warp/blob/fbe26f99c2b508c2e450db4ceccfbdb2a0c8766e/app/src/themes/default_themes.rs#L11-L30)) — so colors come from the theme, not hardcoded hex.
- `(Ns)`: elapsed whole seconds since the request started, ANSI bright black (the existing muted style).

There is no `PRODUCT.md`; the Figma node above is the behavior source of truth.

### Current state

- GUI shimmer: `ShimmeringTextElement` computes a moving band center and per-glyph intensity, then lerps base→shimmer color per glyph ([`crates/warpui_core/src/elements/gui/shimmering_text.rs:119-182 @ fbe26f9`](https://github.com/warpdotdev/warp/blob/fbe26f99c2b508c2e450db4ceccfbdb2a0c8766e/crates/warpui_core/src/elements/gui/shimmering_text.rs#L119-L182)); config in [`shimmering_text/config.rs @ fbe26f9`](https://github.com/warpdotdev/warp/blob/fbe26f99c2b508c2e450db4ceccfbdb2a0c8766e/crates/warpui_core/src/elements/gui/shimmering_text/config.rs) (period 3s, radius 6, padding 8). The math is plain `f32`/`Duration`; only the glyph/ligature mapping is GUI-specific. `elements::gui` is always compiled in `warpui_core`, so these types are already visible to `warp_tui`.
- GUI warping indicator ([`app/src/ai/blocklist/block/view_impl/common.rs:203,551 @ fbe26f9`](https://github.com/warpdotdev/warp/blob/fbe26f99c2b508c2e450db4ceccfbdb2a0c8766e/app/src/ai/blocklist/block/view_impl/common.rs#L203)) uses gray→white shimmer colors and has no rotating spinner; the yellow + `⋮⋰⋱` look is new to the TUI design, so only the shimmer math is reusable.
- TUI text: `TuiText` applies one `TuiStyle` to the whole string ([`crates/warpui_core/src/elements/tui/text.rs @ fbe26f9`](https://github.com/warpdotdev/warp/blob/fbe26f99c2b508c2e450db4ceccfbdb2a0c8766e/crates/warpui_core/src/elements/tui/text.rs)) — no per-character color today, but `TuiElement::render` writes `ratatui` cells directly and `Color::Rgb` is supported.
- Agent blocks: `TuiAIBlock` extracts sections and composes them in `render_element` ([`crates/warp_tui/src/agent_block.rs:241-276 @ fbe26f9`](https://github.com/warpdotdev/warp/blob/fbe26f99c2b508c2e450db4ceccfbdb2a0c8766e/crates/warp_tui/src/agent_block.rs#L241-L276)); per-section renderers live in [`agent_block_sections.rs @ fbe26f9`](https://github.com/warpdotdev/warp/blob/fbe26f99c2b508c2e450db4ceccfbdb2a0c8766e/crates/warp_tui/src/agent_block_sections.rs). The block holds `model: Rc<dyn AIBlockModel>`, which already exposes `status(app).is_streaming()` ([`app/src/ai/blocklist/block/model.rs:151-155 @ fbe26f9`](https://github.com/warpdotdev/warp/blob/fbe26f99c2b508c2e450db4ceccfbdb2a0c8766e/app/src/ai/blocklist/block/model.rs#L151-L155)) and `time_since_request_start(app)` ([`model.rs:179-184 @ fbe26f9`](https://github.com/warpdotdev/warp/blob/fbe26f99c2b508c2e450db4ceccfbdb2a0c8766e/app/src/ai/blocklist/block/model.rs#L179-L184), pure wall-clock) — no new state plumbing needed.
- Redraws are invalidation-driven only ([`spawn_tui_driver`, `crates/warpui_core/src/runtime/mod.rs:381-471 @ fbe26f9`](https://github.com/warpdotdev/warp/blob/fbe26f99c2b508c2e450db4ceccfbdb2a0c8766e/crates/warpui_core/src/runtime/mod.rs#L381-L471)); the TUI paint path has no repaint scheduling. The GUI's generalizable animation mechanism is `PaintContext::repaint_after`/`repaint_at` with earliest-deadline-wins coalescing ([`crates/warpui_core/src/presenter.rs:604-629 @ fbe26f9`](https://github.com/warpdotdev/warp/blob/fbe26f99c2b508c2e450db4ceccfbdb2a0c8766e/crates/warpui_core/src/presenter.rs#L604-L629)) plus the `LiveElement` wrapper ([`crates/warpui_core/src/elements/gui/live.rs @ fbe26f9`](https://github.com/warpdotdev/warp/blob/fbe26f99c2b508c2e450db4ceccfbdb2a0c8766e/crates/warpui_core/src/elements/gui/live.rs)); `ShimmeringTextElement` self-schedules the same way.
- Every TUI draw re-executes `TuiElement::render` over the retained element tree into a fresh buffer ([`TuiPresenter`, `crates/warpui_core/src/presenter/tui.rs:133-251 @ fbe26f9`](https://github.com/warpdotdev/warp/blob/fbe26f99c2b508c2e450db4ceccfbdb2a0c8766e/crates/warpui_core/src/presenter/tui.rs#L133-L251)), so elements that derive visuals from the wall clock inside `render()` animate correctly on cached-element repaints, with no view re-render or relayout.
- Streaming updates already redraw via `TuiTranscriptView::handle_history_event` → `mark_exchange_dirty` ([`crates/warp_tui/src/transcript_view.rs:107-109,193-201 @ fbe26f9`](https://github.com/warpdotdev/warp/blob/fbe26f99c2b508c2e450db4ceccfbdb2a0c8766e/crates/warp_tui/src/transcript_view.rs#L107-L109)), so the indicator appears/disappears on status changes; only the animation clock is missing.
- Theme/styles: the TUI is pinned to `dark_theme()` ([`crates/warp_tui/src/session.rs:64-66 @ fbe26f9`](https://github.com/warpdotdev/warp/blob/fbe26f99c2b508c2e450db4ceccfbdb2a0c8766e/crates/warp_tui/src/session.rs#L64-L66)); styles come from `TuiUiBuilder` ([`crates/warp_tui/src/tui_builder.rs @ fbe26f9`](https://github.com/warpdotdev/warp/blob/fbe26f99c2b508c2e450db4ceccfbdb2a0c8766e/crates/warp_tui/src/tui_builder.rs)), which reads `terminal_colors().normal/bright`.

## Proposed changes

### 1. Shared shimmer math in `warpui_core`

- New feature-independent module `crates/warpui_core/src/elements/shimmer_math.rs`: move `ShimmerConfig` here plus free functions for the band center (glyph count + elapsed time + config), per-index intensity, and the base→peak `ColorU` lerp, operating on plain `usize`/`f32` (no `GlyphIndex`).
- Refactor `ShimmeringTextElement` to call the shared functions (mapping its `GlyphIndex` at the call site) and re-export `ShimmerConfig` from its current path so `warpui::elements::shimmering_text::ShimmerConfig` consumers (e.g. `app/src/ai/loading/shimmering_warp_loading_text.rs`) are unaffected.

### 2. Generalizable TUI repaint scheduling (framework, mirrors the GUI)

- Paint-context plumbing: give the TUI paint pass `repaint_after(Duration)` with earliest-deadline-wins coalescing, mirroring the GUI `PaintContext`. Preferred shape: a dedicated `TuiPaintContext` (wrapping `rendered_views` + `repaint_at: Option<Instant>`) passed to `TuiElement::render`, mirroring the GUI's LayoutContext/PaintContext split; this is mechanical signature churn across `crates/warpui_core/src/elements/tui/*`. (Lighter alternative if churn is unwanted: add the accumulator to the existing `TuiLayoutContext`, which `render` already receives.)
- Surface the deadline: `TuiPresenter::paint` collects the winning deadline into `TuiFrame` (new `repaint_at: Option<Instant>` field). Elements speak relative (`repaint_after`); the frame/driver speak absolute (`repaint_at`), so post-paint scheduling doesn't drift by paint duration.
- Drive it: in `spawn_tui_driver`, after each draw, if the frame carries a deadline, schedule/replace a single one-shot foreground timer that calls `screen.draw(ctx)` at that instant. This is a paint-only redraw over retained elements — no view invalidation, no transcript relayout. Self-sustaining like the GUI: each draw reschedules only if some element requested another repaint, so the TUI is fully idle when nothing animates.
- `TuiLiveElement`: a TUI mirror of the GUI `LiveElement` — wraps any child and calls `repaint_after(interval)` on every render. This is the reusable primitive for future TUI animation (blinking cursor, progress, etc.).
- There must be exactly one native animation mechanism: no per-view tick timers. (A prior TUI spinner effort showed a manual per-view tick forcing full transcript relayout alongside native repaint scheduling causes redundant redraws and jank.)

### 3. New TUI shimmering-text element

- `TuiShimmeringText` in `crates/warpui_core/src/elements/tui/` (gated by the existing `tui` feature, alongside `text.rs`): renders a single-line string writing one cell per char with `fg: Color::Rgb(lerp(base, peak, intensity))` plus optional `Modifier::BOLD`.
- Colors are computed inside `render()` from the wall clock (an animation-anchor `Instant` passed at construction) and a `ShimmerConfig`, and the element calls `repaint_after` each render — mirroring the GUI `ShimmeringTextElement`, minus the glyph-layout state handle (TUI cells are 1:1 with chars).

### 4. Warping indicator section in `warp_tui`

- `crates/warp_tui/src/tui_builder.rs`: add semantic styles — warping yellow (`terminal_colors().normal.yellow`), shimmer peak (`bright.white`); the timer suffix reuses `muted_text_style`.
- `crates/warp_tui/src/agent_block_sections.rs`: new `render_warping_indicator_section(elapsed, app)` composing a `TuiFlex::row()`: spinner element (frame chosen from the wall clock against the same animation anchor), a space, `TuiShimmeringText::new("Warping")` (bold, yellow→bright-white), a space, and the `(Ns)` counter. Spinner + counter are wrapped in `TuiLiveElement` for their repaint cadence; the counter derives its seconds at paint time (elapsed-at-build + time since build) so it ticks on cached-element repaints.
- `crates/warp_tui/src/agent_block.rs`: in `render_element`, when `self.model.status(app).is_streaming()`, append the indicator as a final section (the composer already inserts the one-row gap between sections), anchored to `time_since_request_start` so animation survives element-tree rebuilds. Removal needs no timer: the exchange's final streaming/status events already mark the block dirty and re-render it without the section.
- `TuiTranscriptView` needs no changes.

### 5. Constants

Spinner frame list (`["⋮","⋰","⋱","⋮","⋱","⋰"]`), spinner frame duration (200ms), shimmer repaint interval (~100ms), and shimmer config (`ShimmerConfig::default()`: 3s period, radius 6, padding 8) live together in `agent_block_sections.rs` (or a small `warping_indicator` module) with doc comments. The Figma prototype's frame timings aren't readable via the MCP API, so these are tunable defaults to adjust by eye. (The GUI shimmer repaints at 32ms/30fps; a TUI band over a 7-char string moves ~1 cell per 140ms at the 3s period, so ~100ms repaints are visually equivalent and cheaper.)

Open question: elapsed format for long turns — the design shows `(1s)`; the proposal keeps plain seconds (`(94s)`) rather than `1m 34s`.

## Testing and validation

- Build/lint: `cargo check -p warp_tui -p warpui_core`, then `./script/format` and the presubmit clippy invocation.
- Unit tests (`*_tests.rs` convention):
  - Shimmer-math extraction parity: the GUI element produces identical color overrides before/after the refactor.
  - Spinner frame selection from elapsed time (loop order and frame duration).
  - `TuiShimmeringText` cell colors: all-base at band-outside positions, peak lerp mid-band.
  - `TuiAIBlock` sections include the indicator iff `is_streaming()` (extend `crates/warp_tui/src/agent_block_tests.rs`).
  - Repaint plumbing: `TuiFrame` carries the earliest requested deadline; `TuiLiveElement` requests one (extend `crates/warpui_core/src/presenter/tui_tests.rs`).
- Manual: `script/run-tui`, run a prompt, verify spinner rotation, shimmer sweep, ticking `(Ns)`, spacing/colors against the Figma node, the row disappearing when the turn completes, and that an idle TUI schedules no repaints (no busy redraw loop).

## Parallelization

Not proposed. The work is a single dependency chain (shimmer math → repaint plumbing/`TuiShimmeringText` → `warp_tui` integration) inside two crates with shared signatures (`TuiElement::render` churn touches everything the later steps build on); splitting it across agents would serialize on merge conflicts rather than save wall-clock time. Steps 1 and 2 are mutually independent and could be done in either order, but each is small.

## Risks and mitigations

- `TuiElement::render` signature change touches every TUI element and its tests: purely mechanical; do it as its own commit so review is trivial.
- Runaway repaint loops (an element always requesting repaints): the indicator only renders while streaming, and the driver schedules exactly one coalesced timer per frame; the manual idle check above guards regressions.
- Elapsed-counter width changes (`(9s)` → `(10s)`) shift row layout by one cell mid-animation: acceptable per design (counter is the row's last element, nothing to its right).
