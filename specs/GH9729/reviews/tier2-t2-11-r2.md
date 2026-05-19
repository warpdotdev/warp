---
item: tier2-t2-11
commit: 9b51d44
reviewer: R2-quality
spec_ref: tech.md §698 + §699 (supplemental)
verdict: pass-with-nits
---

# Spec

t2-11 supplements two bullets in `specs/GH9729/tech.md`. Both live in
the "Follow-ups not in v1" list and are the same bullets t2-7 and t2-8
attached to.

§698 (`tech.md:698`, verbatim):

> - **Zoom and pan controls**: extend `lightbox::Params` with zoom
>   state and `lightbox_view.rs` keybindings (`+`, `-`, `0`,
>   drag-to-pan).

§699 (`tech.md:699`, verbatim):

> - **Status footer** (filename, dimensions, file size, format
>   string): extend `lightbox::Params` with an optional metadata
>   strip rendered below the image.

The TIER2_TODO `t2-11` row (`specs/GH9729/TIER2_TODO.md:78-92`) names
the actual contract: rebind to `cmdorctrl-=` / `cmdorctrl--` /
`cmdorctrl-0`, drop the redundant `shift-=`, and append a zoom
percentage to the §699 metadata strip when `zoom_factor != 1.0`. R1
covers correctness against that contract; this review is quality-only.

# Findings

**1. `format_metadata_line` extraction granularity. Right level.**
The chosen shape — a free function `format_metadata_line(Vector2F,
f32) -> String` — is the right grain. The two listed alternatives are
strictly worse here:

- A struct method (`MetadataLine { size, zoom }.format()`) would add a
  one-call-site type for no behavioural reuse. The view doesn't carry
  a long-lived metadata-line state object; it computes the string per
  `render()` from existing fields.
- Two helpers (`format_dimensions(size)` + `format_zoom_suffix(zoom)`
  composed at the call site) would split the *one* invariant that
  matters — "at zoom 1.0 omit the suffix entirely" — across two
  functions, and you'd have to repeat it at every call site or
  reintroduce a tie-breaking glue function. Keeping the conditional
  inside the formatter is exactly what makes the five tests
  trivial-to-write.
- Inline (the pre-t2-11 shape) blocks unit testing of the NaN /
  rounding / suffix-on-non-native cases without a render-test
  harness.

The free function (no `&self`, no `LightboxView` dependency) is also
the right scope: the helper has no view state to read.

**2. Naming. `format_metadata_line` is acceptable; minor
improvement available.** §699 in tech.md calls the surface a
"metadata strip" (not "line"). `Params::metadata_line` in the
existing crate sets the convention — the field name is *line*, so
`format_metadata_line` aligns with the field it feeds. Alternatives:

- `format_status_footer` — t2-11's commit message and the t2-8
  comment both call it the "status footer". Two equally-good names
  in flight. Pick the one that matches the consuming field
  (`metadata_line`), which is what the commit does. Defensible.
- `format_metadata_strip` — closer to tech.md vocabulary but
  diverges from the `Params::metadata_line` field name. Worse.
- `metadata_text` — drops the `format_` prefix; weaker signal that
  this is a pure formatter.

Keep `format_metadata_line`. Not a finding, just an audit.

**3a. `IMPORTANT:` doc comment in `init()`. Right length, fully
accurate.** Eight lines (`lightbox_view.rs:27-39`) is verbose for a
keymap block but earns it: a future contributor *will* be tempted to
revert to `=`/`-`/`0` for ergonomic reasons, and the dispatch-layer
constraint is invisible from the call site. Three specific accuracy
checks:

- "Unmodified character keys (bare "=" / "-" / "0") route to the
  terminal stdin layer before reaching the lightbox view's action
  dispatch" — this is the load-bearing claim and matches the commit
  message's diagnostic. Correct.
- "Only special keys (escape/left/right above) and modifier-prefixed
  keys reach view-scoped FixedBindings" — accurate as a generalization
  *for the terminal-focused context*; mildly imprecise globally
  (other surfaces without a stdin sink can dispatch bare chars), but
  the comment is scoped to the lightbox above a terminal, so this
  scope is implicit. Acceptable.
- "the more-specific view scope wins" — depends on the matcher's
  scope-specificity rule, which I did not re-verify in this pass.
  R1 should have. Flag for R1 if not already covered.

**3b. Doc comment on `format_metadata_line`. Clear.** The 1.0 /
non-1.0 / NaN cases are each named, and the "why-NaN-defends-here"
sentence (`step_zoom` already guards at the input edge but
`Params::zoom_factor` is externally NaN-poisonable) is exactly the
"why this branch exists" justification a future reader needs. The
last clause "renderer-side public `Params::zoom_factor` is
technically NaN-poisonable from external callers" is the right
half-step paranoia for a helper that's one stack frame from a public
field.

**4. Test rigor. Strong; one edge missing.** The five tests cover the
listed cases well. Walking through the suggested 0.999 / 1.001
edges:

- `zoom_factor = 0.999` → `(0.999 * 100.0).round() = 100.0` → `zoom_pct
  == 100` → suffix omitted. Correct: a sub-rounding-error drift from
  native should look native.
- `zoom_factor = 1.001` → `(1.001 * 100.0).round() = 100.0` →
  `zoom_pct == 100` → suffix omitted. Same. Correct, but **arguably
  the wrong behaviour**: if a user types zoom-in and zoom-out and
  hits 1.0001 through floating drift, the suffix vanishing is a
  feature, not a bug. So the test wouldn't catch any defect — it'd
  be pinning a behaviour that already falls out of the existing
  rounding. Not a finding.
- `zoom_factor = 0.995` → `(0.995 * 100.0).round() = 100.0` (`round`
  is half-to-even on `f32` since 1.62, but `99.5_f32 * 1.0` is
  representation-fuzzy enough that `.round()` lands at `100.0` here)
  → suffix omitted. Could surprise a future contributor: they zoom
  out by 0.5% and the indicator still reads "100%"... but
  `MIN_ZOOM_FACTOR` is bounded to a coarse step (1.5×) so this case
  is unreachable in practice. Not a finding.

What *is* genuinely missing is a **suffix-at-exact-rounding-boundary
test** — i.e. the post-multiplication value lands close to an
integer but on the other side of the next-bucket boundary. The
included `1.0/1.5 → 67%` test does double duty here (it lands at
66.66... → 67 rather than 67 → 67 → 67), so the boundary is exercised
even if not explicitly named. Acceptable.

**5. Constants placement. Skip the constant.** A `ZOOM_PERCENTAGE_PRECISION`
or `ZOOM_PERCENTAGE_SCALE` constant for `100.0` would be cargo-cult
naming. `100.0` in the `* 100.0` context is unambiguously
"convert-to-percent" and abstracting it hurts readability. The
`.round() as i32` could be hoisted into a helper if more than one
caller appeared; with one caller it's noise. No finding.

**6. `pathfinder_geometry::vector::Vector2F` import path in helper
signature. Worth a `use` alias.** The fully-qualified path
`pathfinder_geometry::vector::Vector2F` appears three times — once in
the helper signature, twice in the tests via
`pathfinder_geometry::vector::Vector2F::new(...)`. The rest of the
file already imports symbols from `pathfinder_geometry` (search the
file for the existing `use` block); adding `Vector2F` to that block
and writing `fn format_metadata_line(size: Vector2F, zoom_factor:
f32) -> String` would match how the file already refers to the type
elsewhere (e.g. `current_image_native_size` is a `Vector2F` by
inference from the `.x() / .y()` call sites). Low-cost cleanup. Not a
blocker.

**7. Dropped `shift-=` binding. Captured in the commit message;
acceptable.** The commit-message paragraph "The redundant `shift-=`
from t2-7 is dropped — `cmdorctrl-=` covers both `+` and `=` presses"
explicitly documents the deletion. The `IMPORTANT:` block does not
reproduce this deletion rationale, but the surrounding context
("must use modifier-prefixed keys") makes it self-explanatory: an
*unmodified* `shift-=` is itself an unmodified-char binding under the
diagnostic the comment describes (`shift` is not the qualifying
modifier set), and would have been dead code under the same
mechanism that killed `=`. No code reader would re-add a `shift-=`
binding after reading the `IMPORTANT:` block. No follow-up needed.

The "future contributor wonders why `+` doesn't work in isolation"
concern is real but mostly user-facing rather than code-facing: a
user typing `+` (no modifier) in the open lightbox gets nothing,
which is what the t2-7 bare-`=` binding was *meant* to catch but
never could (the same dispatch-layer mechanism kills both). The fix
posture is: `cmd-+` and `cmd-=` both reach dispatch via the
`cmdorctrl-=` binding, which is the convention `app/src/util/bindings.rs:296`
already uses for terminal-font zoom. So the user experience is
consistent with the rest of the app. Acceptable.

**8. ` · ` middot separator. Right typographic choice; codebase
precedent confirmed.** A scan of the rest of `app/src` for ` · `:

- `app/src/settings_view/environments_page.rs:205` —
  `format!("{} · {}", edited, last_used_part)`
- `app/src/settings_view/environments_page.rs:1841` —
  `details_parts.join(" · ")`
- `app/src/settings_view/environments_page.rs:1894` — `" · "` as a
  separator
- `app/src/ai/blocklist/block/view_impl/common.rs:414` — `format!(" ·
  Next check in {formatted}")`
- `app/src/ai/blocklist/block/view_impl/common.rs:957` — `" ·
  Check now"`

` · ` is the established Warp convention for joining metadata fields
in a single text run. Using it for the dimensions / zoom-percentage
join matches the rest of the product. Good call.

**9. No regression-guard test for the binding modifier itself.** The
quality concern is real: nothing prevents a future contributor from
reverting `cmdorctrl-=` → `=` and breaking dispatch silently. The
`IMPORTANT:` comment is the first line of defence; a unit test is the
second.

The keymap module does have introspection (`crates/warpui_core/src/keymap.rs:379`
exposes `get_binding_by_name`), and a registration-style assertion
like the following would catch the revert:

```rust
#[test]
fn zoom_keybindings_require_modifier_prefix() {
    let mut app = AppContext::test();
    init(&mut app);
    let zoom_in = app.keymap().get_binding_by_name(/* ... */);
    assert!(matches!(zoom_in.trigger(), Trigger::Keystroke(k) if !k.modifiers.is_empty()));
}
```

…but wiring it up needs a `LightboxView`-scoped focus context to
make `init`'s registrations addressable. The five existing helper
tests already pin `format_metadata_line`'s contract; pinning the
binding's modifier is the missing rail. **Not a blocker for t2-11**,
but worth filing as a t3 follow-up (parallel to the t2-10 view-test
follow-up logged in `tier2-t2-10-r2.md`). The integration suite
(`app/src/workspace/view_test.rs:3029` already has
`image_preview_arm_builds_*` view-tests) is the right home; a view
test that opens the lightbox, sends the `=` keystroke, and asserts
zoom did *not* change would catch the regression end-to-end.

**10. Commit-message SHA vs. tracker. Same pattern as t2-10.** The
`TIER2_TODO.md` row uses `(see commit msg)` instead of the real SHA;
this is the loop-hygiene artefact you flagged. Not a code finding —
loop-side cleanup. If the loop permits, backfill `9b51d44` into the
tracker row.

# Evidence

- Commit diff: `git show 9b51d44 -- app/src/workspace/lightbox_view.rs`.
- Spec lines: `specs/GH9729/tech.md:698-699`.
- Tracker: `specs/GH9729/TIER2_TODO.md:78-92` for the t2-11 contract;
  line 131 for the row that still reads `(see commit msg)`.
- Middot precedent: `app/src/settings_view/environments_page.rs:205,1841,1894`
  and `app/src/ai/blocklist/block/view_impl/common.rs:414,957`.
- Keymap introspection API: `crates/warpui_core/src/keymap.rs:379`
  (`get_binding_by_name`).
- Existing view-test home for any future binding-regression test:
  `app/src/workspace/view_test.rs:3029` (`image_preview_arm_builds_*`).
- Helper definition and tests:
  `app/src/workspace/lightbox_view.rs:237-264` (helper) and
  `app/src/workspace/lightbox_view.rs:572-625` (the five new tests).

# Suggestions

In priority order, all non-blocking:

1. (Finding 6) Add `use pathfinder_geometry::vector::Vector2F;` (or
   fold into the existing `pathfinder_geometry` `use` block) and
   shorten the helper signature to `fn format_metadata_line(size:
   Vector2F, zoom_factor: f32) -> String`. Same edit applied in the
   three test sites.
2. (Finding 9) File a t3 follow-up for a view-harness regression
   test that opens a lightbox and asserts the bare `=` / `-` / `0`
   keystrokes do *not* fire zoom (mirror of `image_preview_arm_builds_*`).
   The `IMPORTANT:` comment plus this future test would form the
   belt-and-suspenders rail t2-7's gap exposed.
3. (Finding 10) On the next loop turn, backfill the real SHA
   `9b51d44` into the `TIER2_TODO.md` table row for t2-11. Same
   pattern as t2-10.

**Verdict: pass-with-nits.** The keymap-routing fix lands the right
mechanical change with the right load-bearing comment; the
`format_metadata_line` extraction is at the correct granularity,
correctly tested (five cases, including the NaN-input rail), and the
` · ` separator matches existing Warp typographic convention. All
findings above are improvements, not blockers.
