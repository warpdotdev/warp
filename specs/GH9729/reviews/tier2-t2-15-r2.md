---
item: tier2-t2-15
commit: 6623e0e
reviewer: R2-quality
spec_ref: tech.md §698 (supplemental)
verdict: pass-with-nits
---

# Spec

t2-15 supplements the same `tech.md` §698 zoom-and-pan bullet that
t2-7, t2-11, t2-12, and t2-13 attached to. It is a UX-polish pass on
the t2-13 zoom toolbar, driven by user manual feedback: (a) the `+`
button's click STILL did not fire after t2-13's restructure, and (b)
the "100%" reset should disappear when the image is already at native
zoom, not grey out. R1 covers correctness; this review is quality-only.

# Findings

- **pass — conditional rendering is the right shape for "hidden, not
  greyed".** Replacing the t2-13 `disabled: zoom == 1.0` flag with an
  `if zoom != 1.0 { … add_positioned_child(...) }` block is the
  idiomatic way to express "this control does not exist at native
  zoom" in this codebase. Other sites in the same file already use the
  same pattern for optional positioned children (`metadata_line` at
  `crates/ui_components/src/lightbox.rs:687`, `description` at `:646`),
  so it reads as native idiom. As a side effect, it removes the
  `Button::Options.disabled` clone-when-skipped allocation the t2-13
  R2 had to chase down (item 5 of `tier2-t2-13-r2.md`); the closure is
  not constructed at all on the `zoom == 1.0` branch.

- **pass — `ZOOM_ICON_BUTTON_SLOT` is named and placed correctly.**
  Renaming `ZOOM_BUTTON_SLOT_WIDTH` to `ZOOM_ICON_BUTTON_SLOT` is a
  truthful rename — the slot is no longer "one slot per button"; it
  is specifically the per-icon-button slot, with the wider label
  button handled separately. The constant sits at module top next to
  `SCRIM_PADDING` / `SCRIM_BUTTON_INSET` / `ZOOM_RESET_GAP_FROM_ICONS`,
  preserving the t2-13 R2 finding (item 7) that lightbox layout magic
  numbers belong in one eyeball-able block. The doc comment retains
  the why-not-Flex history from t2-12 plus the new why-split-the-slot
  rationale, which is what a six-months-out reader needs.

- **minor — `ZOOM_RESET_GAP_FROM_ICONS = 16.` is a third magic number
  in a stack of three layered approximations.** The reset offset is
  computed as `SCRIM_BUTTON_INSET + 2. * ZOOM_ICON_BUTTON_SLOT +
  ZOOM_RESET_GAP_FROM_ICONS` — i.e. "after two icon-button-slots plus
  some breathing room". That arithmetic only matches the visible
  layout if `ZOOM_ICON_BUTTON_SLOT = 32.` happens to be a tight upper
  bound on the actual `Button::Size::Small` icon width. If the icon
  theme changes or `Size::Small` re-tunes its padding, the icons stay
  put (each lives in its own positioned slot) but the gap shifts.
  This is the same brittleness t2-13 R2 (item 1) flagged on
  `ZOOM_BUTTON_SLOT_WIDTH`, now multiplied by an extra term. The
  in-tree comment is honest about being an approximation; the
  follow-up to "replace with a hit-test-correct flex wrapper"
  remains the right long-term fix, and t2-16 (`f960720`) shows the
  author already pursued that. Nit only because the t2-15 commit is
  honest about the limitation, but the layered-approximation
  technical debt is now deeper than t2-13 left it.

- **minor — `zoom != 1.0` repeats the t2-13 `zoom == 1.0` float
  comparison.** Same justification as t2-13 R2 (item 4): the only
  writes to `zoom_factor` are the literal reset to `1.0` and
  `step_zoom(...)` output that never produces exactly 1.0, so strict
  inequality is correct and `EPSILON`-based comparison would be
  looser. But the same justification comment t2-13 R2 asked for is
  still missing, and now there are two sites (`zoom == 1.0` in t2-13
  if it had survived, `zoom != 1.0` here) where a future reader might
  reach for `(zoom - 1.0).abs() > f32::EPSILON`. A single-line
  comment on the `if zoom != 1.0` branch (or on `ZoomDirection::Reset`
  in the enum doc) would resolve this once for the file.

- **minor — block comment above the toolbar render is now somewhat
  speculative-leaning.** The replacement comment (lines around `:499`
  in the post-commit file) lays out the hit-test-overlap hypothesis as
  the diagnosed cause: "the t2-13 layout positioned three buttons at
  fixed 56-px slots, but the '100%' label button rendered wider than
  the slot, overlapping the `+` button's hit area and silently
  swallowing its clicks." The commit message is honest that this is
  "the leading theory but unproven from static analysis." The
  in-source comment is less hedged than the commit message, which is
  the wrong direction — the in-source artifact lives forever, the
  commit message gets read once. If t2-16/t2-17/t2-18 had confirmed
  the hypothesis, the comment would be load-bearing history; if they
  hadn't, it would be misleading. (In fact: t2-18's commit body —
  `e590caf` — says the real `+` bug was something deeper, and t2-19
  resolved zoom by rewriting the viewport layout. The hit-test
  overlap was probably never the cause.) A one-clause hedge in the
  source — "suspected cause, not confirmed" — would have aged better.

- **nit — `on_zoom_in = on_zoom.clone()` followed by the conditional
  block taking ownership of `on_zoom` (no clone, just move into
  `on_zoom_reset = on_zoom`) is a slightly awkward shape.** Either
  every branch clones, or only the last consumer takes ownership; the
  current code clones for `_in` and moves for `_reset`, which works
  but means re-ordering the branches in the future requires
  remembering to flip the clone. Not worth touching, but the slightly
  more uniform shape would be `let on_zoom_in = on_zoom.clone(); /*
  build button */; if zoom != 1.0 { let on_zoom_reset = on_zoom; /*
  build button */ }` — which is what's already written, so this nit
  is purely "be aware the move-on-last-use pattern is intentional and
  fragile."

- **pass — no new tests is the right call for this commit.** The
  R1-side correctness question — "does conditional rendering toggle
  on the `zoom_factor` boundary?" — is one `if` on a public input;
  asserting it would require a renderer fixture or an introspection
  harness on `Box<dyn Element>` that doesn't exist for this surface.
  Same reasoning as t2-13 R2 (item 9). The 18/18 `lightbox_view` test
  count is preserved.

- **pass — no dead code or stray imports left from the swap.** The
  old `ZOOM_BUTTON_SLOT_WIDTH` constant is fully removed (verified by
  grep). No unused imports introduced. The "// Each button sits
  independently…" leftover comment from t2-13 is excised along with
  its constant. Clean diff.

- **observation — the inset asymmetry from t2-13 R2 (item 2) is
  unchanged.** `SCRIM_BUTTON_INSET` is still scoped to the close
  button and the zoom toolbar; prev/next still use literal
  `vec2f(12., 0.)` / `vec2f(-12., 0.)`. Not in scope for t2-15, but
  the asymmetry is now one commit older and worth carrying forward
  as a deferred R2 follow-up rather than letting it accumulate.

# What I checked

- Full `git show 6623e0e` diff.
- `specs/GH9729/tech.md` §698 bullet.
- Post-commit state of `crates/ui_components/src/lightbox.rs` at the
  toolbar render block (verified `if zoom != 1.0 { … }` is the only
  branch wrapping reset, and the `disabled` flag is removed).
- Constant declarations and doc comments at the top of the same
  file (`ZOOM_ICON_BUTTON_SLOT`, `ZOOM_RESET_GAP_FROM_ICONS`,
  `SCRIM_BUTTON_INSET`).
- Field declarations on `Lightbox` (`zoom_out_button`,
  `zoom_in_button`, `zoom_reset_button` — all retained even though
  `zoom_reset_button` is only rendered conditionally, which is fine
  because `button::Button` is a persistent component handle).
- Conditional-positioned-child patterns elsewhere in the file
  (`metadata_line`, `description`, `prev`/`next` blocks) for
  idiomatic-fit comparison.
- t2-13 R2 (`tier2-t2-13-r2.md`) for the prior nits this review
  inherits (slot-width brittleness, float comparison, inset asymmetry).
- Subsequent commits on the branch (`f960720` t2-16, `dff6822` t2-17,
  `e590caf` t2-18) to validate the in-source overlap-hypothesis
  comment against later-discovered cause. The comment is now
  factually questionable per t2-18's commit body — flagged as a minor
  finding above.
- No tests modified by this commit (`git show 6623e0e --stat`); R1
  scope question, but cross-checked.

# Suggestions

The following are candidates for **Deferred R2 follow-ups** rather
than fixes in t2-15:

1. **Track the slot-width / gap approximation as explicit
   technical debt.** A single bullet in `TIER2_TODO.md`'s
   open-issues section or a tracking issue: "Zoom toolbar layout is
   approximated by `ZOOM_ICON_BUTTON_SLOT` + `ZOOM_RESET_GAP_FROM_ICONS`
   magic numbers; replace with a hit-test-correct flex wrapper once
   the underlying Stack/Flex routing bug is resolved." (t2-16 in
   commit `f960720` already attempts this; the follow-up bullet would
   document the throughline.)

2. **Add a one-line justification comment for strict float equality
   on `zoom_factor`.** Either at the `if zoom != 1.0` site here, on
   the t2-13 `zoom == 1.0` if it survives, or on the `ZoomDirection`
   enum. Resolves the recurring "why not EPSILON" reviewer question.

3. **Re-tone the toolbar-render block comment from declarative to
   hedged on the overlap hypothesis.** Per t2-18's findings the
   original cause was something else; the in-source comment now reads
   as a confident diagnosis of a hypothesis that didn't pan out.
   A one-clause "(suspected cause; t2-18 revisited)" patch would
   age this comment correctly.

4. **Extend `SCRIM_BUTTON_INSET` to prev/next button literals.**
   Carried forward from t2-13 R2 (item 2). Trivial, defensible to
   defer, but accumulating across commits.
