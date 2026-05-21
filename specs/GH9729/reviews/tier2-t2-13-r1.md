---
item: tier2-t2-13
commit: a655650
reviewer: R1-correctness
spec_ref: tech.md §698 (supplemental)
verdict: pass-with-nits
---

# Spec

`tech.md:698`:

> - **Zoom and pan controls**: extend `lightbox::Params` with zoom state and `lightbox_view.rs` keybindings (`+`, `-`, `0`, drag-to-pan).

t2-13 is a supplemental polish on the zoom toolbar already added in t2-12 under the same one-line spec entry; the relevant authoritative description is the commit message and the supplemental notes in `tech.md` follow-ups.

# Findings

- [pass] `+` button hit-test fix. The three buttons are now siblings of the close/prev/next buttons under the same `Stack`, each routed through its own `content.add_positioned_child(...)` call (`crates/ui_components/src/lightbox.rs:553-585`). This mirrors the prev/next pattern at `crates/ui_components/src/lightbox.rs:448-484`, which is already proven to route clicks correctly. The Flex-inside-positioned-parent shape that lost the rightmost click in t2-12 is gone — no remaining `Flex::row()` over the three zoom buttons exists in the file.
- [pass] Offset math. Each button anchored `BottomLeft → BottomLeft`:
  - zoom-out: `vec2f(SCRIM_BUTTON_INSET, -SCRIM_BUTTON_INSET) = (12., -12.)`
  - reset:    `vec2f(SCRIM_BUTTON_INSET + ZOOM_BUTTON_SLOT_WIDTH, -SCRIM_BUTTON_INSET) = (68., -12.)`
  - zoom-in:  `vec2f(SCRIM_BUTTON_INSET + 2. * ZOOM_BUTTON_SLOT_WIDTH, -SCRIM_BUTTON_INSET) = (124., -12.)`
  Matches the values you listed.
- [pass] No overlap between the three buttons. `Button::Size::Small` has `height = 24`, `font_size = 12`, `horizontal_padding = 8` (`crates/ui_components/src/button/params.rs:142-157`). Icon-only buttons (Minus, Plus) are forced square at 24×24 by `button.rs:119-136`. The middle label button widens to fit "100%" plus `2 * 8 = 16` horizontal padding plus a 4-glyph 12-pt run; even rounding up generously this is comfortably under the 56-px slot. No two buttons can occupy the same horizontal region.
- [pass] Label content path. `button::Content::Label(Cow<'static, str>)` is defined at `crates/ui_components/src/button/params.rs:56`; `"100%".into()` produces `Cow::Borrowed("100%")`. The Label branch in `button.rs:68-82` renders the text via `Text::new_inline` with the small font properties and skips the icon `ConstrainedBox` branch, so the slot stays text-shaped rather than icon-square — fits the wider middle slot fine.
- [pass] Disabled gate.
  - `disabled: zoom == 1.0` at `crates/ui_components/src/lightbox.rs:527` is wired into `button::Options.disabled`.
  - `button.rs:155-160` shows `on_click` is wired only when `!params.options.disabled`, so a disabled reset is a true no-op (no cursor change, no callback).
  - `button.rs:28-32` swaps to `themes::Disabled`, so the visual signal is also correct.
- [nit] The spec's parenthetical "step_zoom never produces exactly 1.0 from non-1.0 inputs" is slightly inaccurate — `step_zoom(1.5, Out) = 1.5 / 1.5 = 1.0` exactly in IEEE-754, so a `+` followed by a `-` does land back at exactly `1.0`. That is the *desired* outcome (reset becomes disabled at native size), so the `==` comparison is still correct, just for a stronger reason than the spec claim. Worth noting in case future audits rely on that property.
- [pass] NaN/Inf does not regress here. `step_zoom` in `app/src/workspace/lightbox_view.rs:303-312` collapses non-finite inputs back to `1.0`, and `LightboxView::default()` initializes `zoom_factor: 1.0` (`lightbox_view.rs:107-120`), so no NaN path reaches `params.zoom_factor` from the in-tree caller. The renderer-side `clamp` on line 293 still propagates NaN if a future caller passes one in — that's a pre-existing concern from R1-t2-7, *not* introduced or worsened by this commit. For the reset button specifically, `NaN == 1.0` is `false`, so the button stays enabled and the user can click "reset" to recover — defensible behavior.
- [pass] `ZOOM_TOOLBAR_SPACING` fully removed. `grep -rn ZOOM_TOOLBAR_SPACING` over `crates/ui_components/` and `specs/` returns no hits.
- [pass] Helpers untouched. The commit modifies `crates/ui_components/src/lightbox.rs` and `specs/GH9729/TIER2_TODO.md` only. `app/src/workspace/lightbox_view.rs` (which owns `step_zoom`, `format_metadata_line`, `apply_rewrite_to_slot`) is not in the diff, so the 18 unit tests remain on the same code path.
- [pass] Examples unaffected. `crates/ui_components/examples/library.rs:559-607` already passes `on_zoom: None`, so the new `let zoom = params.zoom_factor` and the disabled wiring sit inside the `if let Some(on_zoom) = params.options.on_zoom` branch and are unreachable for the example surfaces.
- [nit] `SCRIM_BUTTON_INSET` is good consolidation for close and zoom-out (the two corner-anchored buttons), but the prev/next buttons at lines 451 and 479 still use the raw literal `12.` for their horizontal inset. If the intent is "one source of truth for corner/edge insets," sweeping those two call sites in a follow-up commit would finish the job. Not blocking.
- [nit] `ZOOM_BUTTON_SLOT_WIDTH = 56.` is a hand-tuned constant that depends on the rendered width of `"100%"` at small font size. Localized variants ("100 %", "100%" rendered with a different glyph set) or a future label change would silently overlap. The constant doc-comment already calls this out, which is the right outcome for "approximation until a proper flex hit-test fix lands." Worth keeping on the t2 cleanup list.

# What I checked

- `git show a655650 --stat` and full file diff — confirmed only `lightbox.rs` (+62/-17) and `TIER2_TODO.md` (+10) were touched; no test file, no lightbox_view.rs.
- `crates/ui_components/src/lightbox.rs:417-425` — close button still uses corner inset, now via constant; same anchor.
- `crates/ui_components/src/lightbox.rs:448-484` — confirmed prev/next use the same individually-positioned-child pattern (no Flex).
- `crates/ui_components/src/lightbox.rs:488-585` — new zoom toolbar block: three buttons, three `add_positioned_child` calls, BottomLeft anchor, accumulating x offsets, `disabled: zoom == 1.0` on reset, `Content::Label("100%".into())` on reset.
- `crates/ui_components/src/button.rs:28-32` — disabled theme swap.
- `crates/ui_components/src/button.rs:48-82` — Label rendering branch.
- `crates/ui_components/src/button.rs:119-136` — icon-button squaring vs label-button width.
- `crates/ui_components/src/button.rs:155-160` — `on_click` skipped when `disabled`.
- `crates/ui_components/src/button/params.rs:54-60` — `Content::Label(Cow<'static, str>)`.
- `crates/ui_components/src/button/params.rs:142-157` — `SMALL_SIZE` sizing (height 24, font 12, padding 8).
- `app/src/workspace/lightbox_view.rs:303-312` — `step_zoom` non-finite guard and clamp; verified caller flow can't leak NaN into `zoom_factor`.
- `crates/ui_components/src/lightbox.rs:285-300` — renderer clamp on `zoom_factor`; pre-existing NaN propagation noted, not regressed.
- `grep -rn ZOOM_TOOLBAR_SPACING` — confirmed full removal.
- `crates/ui_components/examples/library.rs:559-607` — both `lightbox::Params` literals pass `on_zoom: None`.

# Suggestions

- Optional follow-up: replace the raw `12.` literals at `lightbox.rs:451` and `lightbox.rs:479` with `SCRIM_BUTTON_INSET` so the constant is the single source of truth for edge insets, matching the intent described in the commit body.
- Tighten the commit-message claim about `step_zoom` never returning to exactly `1.0` — it can, via the `1.5 / 1.5` round-trip, and that's actually the desired behavior. Not worth a re-commit on its own; fold into the next zoom-related change.
