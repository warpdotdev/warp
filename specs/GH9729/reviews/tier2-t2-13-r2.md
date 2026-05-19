---
item: tier2-t2-13
commit: a655650
reviewer: R2-quality
spec_ref: tech.md §698 (supplemental)
verdict: pass-with-nits
---

# Spec

t2-13 supplements one bullet in `specs/GH9729/tech.md` (Follow-ups not
in v1), the same line t2-7, t2-11, and t2-12 attached to.

§698 (`tech.md:698`, verbatim):

> - **Zoom and pan controls**: extend `lightbox::Params` with zoom
>   state and `lightbox_view.rs` keybindings (`+`, `-`, `0`,
>   drag-to-pan).

t2-13 is a polish pass on the t2-12 zoom toolbar — three corrections
from manual feedback: fix the `+` button click that never fired in
t2-12, replace `Icon::Refresh` reset with a `Label("100%")` button,
and disable reset when `zoom_factor == 1.0`. R1 covers correctness;
this review is quality-only.

# Findings

**1. `ZOOM_BUTTON_SLOT_WIDTH = 56.` is the right shape of fix at the
wrong layer.** The constant exists because the t2-12 `Flex::row`
wrapped-inside-`add_positioned_child` pattern was hit-test broken for
the rightmost child, and the cheapest unblock is "give up on Flex,
position each button by hand at a fixed step". The doc comment is
honest about that ("approximation calibrated for `Button::Size::Small`
with a `Label("100%")` middle slot; needs re-tuning if a future icon
set changes the rendered button width"). That honesty does not change
the fact that **a hard-coded slot width is a future-bug magnet**:
themes change, font metrics change, the middle label changes from
"100%" to "150%" (which is wider in proportional fonts), and the row
quietly overlaps or gaps. The right long-term fix is whichever of
(a) figure out the t2-12 Flex-routing bug and re-enable Flex, or
(b) thread a hit-test-aware row primitive through the positioning
layer. The doc comment says as much ("on a future tighter layout pass
it could be replaced with a hit-test-correct flex wrapper"), so this
is documented technical debt rather than hidden technical debt. Pass,
with the nit that a follow-up bullet in `TIER2_TODO.md` or
`tech.md`'s Follow-ups list (or a tracking issue) would harden the
commitment — a comment in a constant tends to drift out of grep
attention.

**2. Asymmetric corner-vs-middle inset handling. Worth a small
follow-up, not a blocker.** `SCRIM_BUTTON_INSET = 12.` cleanly unifies
the close button (top-right) and the three zoom buttons (bottom-left)
— same magic number, same justification ("corner stays symmetric"),
one source of truth. Good. But the prev/next buttons one block up
(`crates/ui_components/src/lightbox.rs:451` and `:479`) still use
literal `vec2f(12., 0.)` / `vec2f(-12., 0.)` against `MiddleLeft` /
`MiddleRight` anchors. The R1 doc-trail asks whether these are
"semantically the same inset" or different. My read: they **are**
semantically the same — the spec intent everywhere is "inset 12 px
from a scrim edge". Whether the anchor is corner-anchored or
middle-anchored is a layout-mechanics detail, not a design decision
about gutter width. Extending `SCRIM_BUTTON_INSET` to the prev/next
pair would be one more touch site on a tighter scope; the t2-13
commit chose to leave it out, which is defensible (scope discipline).
But it leaves the inset half-de-magic-numbered: a future reader will
see the constant on lines 31/33, find it used at the close button and
three zoom buttons, then hit the prev/next literals and wonder why
those didn't migrate. A trailing nit, not a blocker.

**3. `button::Content::Label("100%".into())` is idiomatic.** The
`.into()` converts `&'static str` to `Cow<'static, str>`, matching the
in-repo doc example at `crates/ui_components/src/lib.rs:48`
(`content: button::Content::Label("Click me".into())`). The only other
production `Label` site is t2-13's own. No nit on style.

**4. `disabled: zoom == 1.0` float equality is fine in context, but a
one-line justification comment would future-proof it.** Clippy's
`float_cmp` lint (and most code-review intuition) flags `==` on f32 as
a smell. In this codebase the lint may be project-allowed or this
particular site may sit on an `#[allow]`, but a reviewer landing on
this line in three months without t2-13's context will reasonably
ask "why isn't this `(zoom - 1.0).abs() < f32::EPSILON`?". The
factual answer (verified by reading `step_zoom` at
`app/src/workspace/lightbox_view.rs:303` and the zoom-reset assignment
at `:485` — the only writes to `zoom_factor` are the literal `1.0`,
`step_zoom(...)` results that never step **to** exactly 1.0 from a
non-1.0 input, and the clamp inside `step_zoom`) is that the value
**is** bit-exactly `1.0` when reset, so strict equality is correct
and `EPSILON` would be looser, not safer. Worth one trailing comment
like `// exact: zoom_factor is only ever set to literal 1.0 (reset)
or step_zoom() output, which never returns exactly 1.0`. Nit, not
blocker.

**5. `on_zoom_reset` clone-when-disabled is not a leak. Confirmed.**
The disabled branch in `button.rs:155` skips both
`with_cursor(...)` and `.on_click(on_click)`. The `on_click` is a
`Box<dyn Fn(...)>` that owns the cloned `Arc<...>` — when the branch
is skipped, the `Box` is dropped at the end of the `render` call,
which drops the closure, which drops the `Arc` clone. Per-frame
allocation and drop, no leak. Trivial concern resolved.

**6. Commit-message claim that `ZOOM_TOOLBAR_SPACING = 8.` is
removed: verified.** `grep ZOOM_TOOLBAR_SPACING` against the working
tree returns zero hits; the diff shows the const removed at the same
location where `ZOOM_BUTTON_SLOT_WIDTH` is added. Claim accurate.

**7. `ZOOM_BUTTON_SLOT_WIDTH` placement is correct.** Sitting at
module top (line 25-ish) next to `SCRIM_PADDING` and
`SCRIM_BUTTON_INSET` puts all the lightbox layout magic numbers in
one block where they can be eyeballed and re-tuned together. Moving
it next to the toolbar render code (a 60-line span down at line ~530)
would split the constant from its peers (`SCRIM_PADDING` is also
"used far from declaration") for no readability win. Keep where it
is.

**8. Doc comment length on `ZOOM_BUTTON_SLOT_WIDTH`: appropriate.**
The 8-line comment explains *why* individual `add_positioned_child`
calls instead of `Flex::row` (the t2-12 hit-test bug) and *why* the
slot-width approach is brittle (needs re-tune on icon changes). Both
pieces of context are non-obvious from the code alone and saving a
future reader from re-deriving them is worth 8 lines. The comment
does *not* over-recount t2-12's history — it states the symptom
("rightmost button's click did not fire") and the hypothesis
("suspected hit-test routing bug when multiple children share one
positioned parent") and stops. Calibrated right.

**9. No new tests is acceptable for this commit.** The disabled-state
contract is `button::Options.disabled` (set on line 525), not
behavior owned by `Lightbox`. Asserting "the rendered button has
`disabled` matching `zoom == 1.0`" requires either a renderer
fixture or a structural-test harness that introspects the
`Box<dyn Element>` tree, neither of which exists for this surface.
The 18/18 `lightbox_view` test count is preserved because the
zoom-action plumbing it covers is unchanged. Reasonable scope.

**10. SHA-after-amend hygiene.** The tracker row in `TIER2_TODO.md`
lists `_pending_` (correct — no review SHAs yet). The implementation
commit currently sits at `0d1dcac` on the branch (the original
`a655650` referenced in this review's frontmatter was amended once;
`git diff a655650 0d1dcac -- crates/ui_components/src/lightbox.rs`
returns zero, so the amend was metadata-only and the code in this
review is the code on the branch). If the review-commit step updates
the tracker, use the current `0d1dcac` SHA.

# Verdict

**pass-with-nits.** The three corrections from manual t2-12 feedback
(plus-button fix, text-100% reset, disabled gate) land cleanly and
are well-commented. Two real nits — the `ZOOM_BUTTON_SLOT_WIDTH`
approximation should be tracked as follow-up debt rather than only
inline-commented, and the prev/next button literals should either
adopt `SCRIM_BUTTON_INSET` for consistency or get a one-line "these
look like the same magic number but the anchors are different"
disclaimer. Plus the float-equality comment from item 4. None block
this commit.
