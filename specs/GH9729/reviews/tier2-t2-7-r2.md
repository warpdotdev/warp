---
item: tier2-t2-7
commit: 6aee220
reviewer: R2-quality
spec_ref: tech.md §698
verdict: pass-with-nits
---

# Spec

The relevant tech.md §698 bullet sits in the v1.x follow-ups list of
`specs/GH9729/tech.md`. Verbatim:

> **Zoom and pan controls**: extend `lightbox::Params` with zoom state and `lightbox_view.rs` keybindings (`+`, `-`, `0`, drag-to-pan).

# Findings

- [minor] **`step_zoom` extraction is the right call; not worth
  splitting into `step_zoom_in` / `step_zoom_out`.** A
  `(current, ZoomDirection)` shape pulls the shared NaN sanitization,
  the multiplicative step lookup, and the `clamp(MIN, MAX)` tail into
  exactly one place; splitting it into two free functions would
  duplicate the NaN guard and the clamp on both branches, or push the
  duplication-removing helper down a level (which is the same shape
  this code already has, just renamed). The `enum ZoomDirection { In,
  Out }` does add one type that exists for one consumer, but it pays
  for itself by giving the test names a stable parameter to bind —
  `step_zoom(1.0, ZoomDirection::In)` reads cleaner than a boolean
  `step_zoom(1.0, true)` would, and the callers in
  `LightboxViewAction::ZoomIn` / `ZoomOut` (lines 380, 387 of
  `app/src/workspace/lightbox_view.rs`) read symmetrically. Verdict:
  the chosen factoring is appropriate; flag a microscopic alternative
  (`fn next_zoom_factor(current: f32, factor: f32) -> f32` taking the
  raw multiplier) only as a future option if a non-keystroke caller
  ever needs a different step size — not relevant in v1.

- [minor] **Constants placement — option (a) (current shape) is
  defensible but option (b) is cleaner.** The diff makes
  `MIN_ZOOM_FACTOR`, `MAX_ZOOM_FACTOR`, and `ZOOM_STEP` `pub` in
  `crates/ui_components/src/lightbox.rs:18-29` and the view layer in
  `app/src/workspace/lightbox_view.rs:218-237` reaches in to
  `lightbox::ZOOM_STEP` and `lightbox::MIN_ZOOM_FACTOR` /
  `lightbox::MAX_ZOOM_FACTOR` directly. The current `lightbox::Lightbox`
  Component already clamps inside its own paint (lines 211-216 of
  `lightbox.rs`), so the *lightbox crate's* invariant doesn't actually
  depend on the view layer cooperating. That makes the public
  constants a *contract* that says "if you want your UI to feel
  consistent with the renderer's clamp, here are the bounds." Option
  (b) — keep the constants private, expose
  `pub fn clamp_zoom_factor(f32) -> f32` — would shrink the public API
  surface to "one clamp function" and would prevent a second
  consumer from drifting if the constants are ever retuned. Option
  (c) (per-view consts in `lightbox_view.rs`) loses the synchronized-
  retune property entirely. Net: the chosen shape works but adds
  three `pub const`s where one `pub fn` would do. Not blocking;
  worth a follow-up cleanup before any second consumer materialises
  (e.g. when the §699 status footer or t2-7-pan land and need their
  own zoom-aware UI).

- [nit] **`reset_per_image_state` not used at construction is a real
  inconsistency, but the cost of fixing is one rebinding.** Lines
  108-113 of `app/src/workspace/lightbox_view.rs` (the `new`
  constructor) directly initialise `animation_start_time:
  Instant::now()` and `zoom_factor: 1.0`, while `update_params`,
  `NavigatePrevious`, and `NavigateNext` all route through
  `reset_per_image_state`. The struct-init form is required at
  construction (the helper takes `&mut self`), so the natural fix
  is the let-mut-then-call pattern that already exists in `new`:
  set the fields to whatever placeholder, then call
  `view.reset_per_image_state();` between the struct literal and
  `view.start_asset_loads(ctx);`. That centralises the "what counts
  as per-image state" definition into the helper body and means a
  future per-image field (pan offset for t2-7-pan, EXIF orientation
  override, etc.) only needs to touch the helper, not four call
  sites. Strictly stylistic in v1 because the construction default
  trivially happens to match the reset value; will start mattering
  the moment the reset value diverges from the construction default
  (e.g. if a future per-image field has different "fresh" vs "reset"
  semantics). Worth fixing now while the helper is fresh.

- [nit] **Naming.** `step_zoom` reads marginally awkwardly as
  verb-then-noun; `apply_zoom_step` or `next_zoom_factor` would scan
  more naturally and `zoom_step` (noun) would mirror the
  `lightbox::ZOOM_STEP` constant. `ZoomDirection` is fine and matches
  the GPUI vocabulary used elsewhere (e.g.
  `LightboxViewAction::NavigatePrevious`/`NavigateNext` use direction-
  shaped names). `MIN_ZOOM_FACTOR` and `MAX_ZOOM_FACTOR` are clear.
  `ZOOM_STEP` could read more precisely as `ZOOM_STEP_FACTOR` to
  emphasise it's multiplicative, not additive — a future contributor
  who reads only the constant name might reasonably guess "step by
  1.5 zoom-factor units per press" (additive) rather than "multiply
  by 1.5 per press." The existing doc comment on the constant
  (`lightbox.rs:25-29`) is unambiguous, so this is a name-only nit;
  the doc carries the contract. Strictly stylistic; not worth a
  rename.

- [nit] **Doc comment on `Params::zoom_factor` is complete on the
  happy path; one sentence on the non-finite contract would close
  the loop.** The doc at `crates/ui_components/src/lightbox.rs:119-132`
  correctly explains `1.0` native, `>1` zoom in, `<1` shrink, and
  references `t2-7-pan` for the deferred pan deliverable. What it
  doesn't explicitly say is what happens if a caller passes
  `f32::NAN` or `f32::INFINITY`: the implementation at line 215 of
  `lightbox.rs` calls `params.zoom_factor.clamp(MIN_ZOOM_FACTOR,
  MAX_ZOOM_FACTOR)`, and `f32::clamp` of NaN is NaN (per the std
  contract — clamp does not sanitise non-finite inputs). The view
  side's `step_zoom` *does* sanitise NaN to `1.0`, so in practice
  the view never feeds a non-finite value through. The renderer is
  therefore relying on the view-side guarantee, not its own clamp.
  Suggest one sentence at the bottom of the doc: "Callers must pass
  a finite value; non-finite inputs propagate through the renderer's
  clamp and will NaN-poison the layout." This documents the
  contract for the next consumer who isn't `LightboxView` (e.g. a
  unit test, an example, or the t2-7-pan PR). Minor — the bug
  surface is closed today by the view; this is documentation of an
  invariant rather than a code change.

- [minor] **Test rigor — five tests cover the obvious axes; two
  small additions would be high-value-per-line.** The five existing
  tests (`step_zoom_in_multiplies_by_step`, `step_zoom_out_divides_by_step`,
  `step_zoom_in_clamps_to_max`, `step_zoom_out_clamps_to_min`,
  `step_zoom_recovers_from_non_finite_input` at lines 464-510 of
  `app/src/workspace/lightbox_view.rs`) cover multiply, divide, both
  clamps, and NaN/Inf. The two missing cases that I'd recommend
  adding:

  1. **Targeted near-min clamp.** The current
     `step_zoom_out_clamps_to_min` test spams 50 iterations to reach
     the floor. A more readable test would assert that
     `step_zoom(0.3, Out)` returns exactly `MIN_ZOOM_FACTOR` (0.25),
     since `0.3 / 1.5 = 0.2` which clamps up to 0.25. This pins the
     clamp behaviour at the boundary — if someone later refactors
     the clamp to `if raw < MIN_ZOOM_FACTOR { return MIN_ZOOM_FACTOR; }`
     and the boundary check goes off-by-one, the spam test still
     passes (the loop saturates regardless) but the targeted test
     would catch it.

  2. **Round-trip cancellation.** `step_zoom(step_zoom(1.0, In), Out)`
     should return exactly `1.0` (since `(1.0 * 1.5) / 1.5 == 1.0`
     in IEEE-754 for these specific values, no precision loss
     because `1.5` is exactly representable). This pins the UX
     expectation that "one zoom-out cancels one zoom-in" and would
     catch a regression where someone retunes `ZOOM_STEP` to a
     non-power-of-two value (e.g. `1.4`) where the round-trip would
     drift. If the test fails with a future tuning, it's a signal
     to either pin a specific tuning or document the round-trip
     drift as expected.

  Both are one-line tests; not blocking, but cheap.

- [minor] **Keybinding registration — `shift-=` parses correctly
  but only because of a peculiarity in
  `crates/warpui_core/src/keymap.rs::Keystroke::parse`.** I traced the
  parser at `crates/warpui_core/src/keymap.rs:897-967`: splitting
  `"shift-="` on `-` gives `["shift", "="]`. The first component
  matches the `"shift"` modifier arm; the second falls into the
  `_` arm and `is_valid_key("=")` (line 834) returns `true` because
  `"="` is a single character. The debug-build assertion at lines
  940-963 then checks "shift + lowercase letter" — `=` is not a
  letter so `is_lowercase()` is false, and the assertion is skipped.
  So `shift-=` is a valid keystroke string. (For comparison, `-`
  registers via the special "trailing dash" handling at lines
  925-931: splitting `"-"` on `-` gives `["", ""]`, and the
  `source.ends_with('-')` branch at line 925 sets `key = "-"`.)
  Both bindings register; the FixedBinding registration would not
  silently fail. **Future ergonomics** to flag (not blocking):
  European keyboards (DE, FR, NO, etc.) where `+` is unmodified —
  the spec calls for `+` and the current bindings cover the US
  layout but not DE/NO unmodified `+`. Worth a follow-up to also
  register `"plus"` if the keymap supports a named key, or to
  document the layout assumption explicitly. The existing
  workspace-level `cmdorctrl-=` precedent (`app/src/util/bindings.rs:296`,
  `app/src/workspace/mod.rs:393, 442`, `app/src/notebooks/notebook.rs:169`)
  has the same layout sensitivity, so this is a codebase-wide
  ergonomics gap rather than something t2-7 introduced.

- [nit] **Comment density is on the heavy side; two
  `// GH9729 §698:` comments restate what the code already says.**
  The comments inventory:

  - `app/src/workspace/lightbox_view.rs:31-34` (keybinding
    registration) — *justified*. Explains the `=` vs `shift-=` split
    and references the workspace-level convention, which a future
    contributor would not derive from the code alone.
  - `app/src/workspace/lightbox_view.rs:73, 75, 77` (action enum
    docstrings) — *acceptable as docstrings* but the `GH9729 §698:`
    prefix is noise on a public-doc surface; prefer "Zoom the current
    image in by one step." without the issue-prefix on a docstring
    that's part of the type's public API.
  - `app/src/workspace/lightbox_view.rs:92-98` (field doc comment) —
    *justified*. The field's invariants (`[MIN, MAX]`, reset on
    image change) live here.
  - `app/src/workspace/lightbox_view.rs:117-122`
    (`reset_per_image_state` doc) — *justified*. Names the two pieces
    of state and the reset trigger.
  - `app/src/workspace/lightbox_view.rs:217, 220-225` (free-function
    doc) — *justified*. NaN sanitization rationale is non-obvious.
  - `crates/ui_components/src/lightbox.rs:17, 21, 26` (constant docs) —
    *acceptable* but the `GH9729 §698:` prefix is again redundant on
    a docstring; the doc body itself carries the meaning.
  - `crates/ui_components/src/lightbox.rs:211-214` (clamp comment) —
    *justified*. Names the NaN-poisoning failure mode the clamp
    defends against.

  Net: most are justified; the `GH9729 §698:` prefix on docstrings
  is noise. Not blocking.

- [nit] **Unit-of-measure ambiguity is closed by the doc comment.**
  A future contributor reading only `pub zoom_factor: f32` on the
  field declaration could plausibly assume percentages (`100.0` =
  native) given how `f32`-shaped UI scale fields work in some other
  frameworks. The doc comment at `lightbox.rs:119-132` explicitly
  says "`1.0` renders at native size" and "Values `> 1.0` scale the
  `ConstrainedBox` linearly", which closes the ambiguity for anyone
  who reads the doc. Field name `zoom_factor` (rather than
  `zoom_level` or `zoom_percent`) reinforces the multiplicative
  interpretation. No change needed.

- [minor] **`pan_offset` placeholder — preference for adding it
  now.** The deferred t2-7-pan follow-up (`TIER2_TODO.md:126-140`)
  will add `pan_offset: Vector2F` to `Params`. Without a placeholder,
  that's a public-API change to `Params` (every existing literal in
  `app/src/workspace/lightbox_view.rs:331` and the two examples in
  `crates/ui_components/examples/library.rs:567, 602` will need
  `pan_offset: Vector2F::default()` added). The cost of adding it
  now is exactly the same — three call sites — and avoids the
  semver / churn ceremony when t2-7-pan lands. Counter-argument: a
  placeholder `pan_offset` that the renderer ignores is dead code,
  and Rust's `#[non_exhaustive]` on `Params` (not currently set,
  worth verifying) would already let new fields land
  non-breakingly. Verdict: lean toward adding `pan_offset:
  Vector2F` now (defaulted to zero, ignored by the renderer with a
  `// GH9729 t2-7-pan: not yet consumed` comment), so the t2-7-pan
  PR is purely behavioural. Not blocking; either choice is
  defensible.

# What I checked

- Read the full diff via `git show 6aee220`.
- Confirmed the spec section: tech.md §698 is the "Zoom and pan
  controls" bullet in the v1.x follow-ups list of
  `specs/GH9729/tech.md` (verbatim above).
- `crates/ui_components/src/lightbox.rs:17-29` — three new `pub
  const`s with doc comments. Values: `MIN = 0.25`, `MAX = 8.0`,
  `STEP = 1.5`. The doc on `ZOOM_STEP` correctly states the step
  reaches `MAX` in five `+` presses (`1.0 * 1.5^5 = 7.59`, clamped
  to 8.0 on the next press) and `MIN` in four `-` presses
  (`1.0 / 1.5^4 = 0.197`, clamped up to 0.25 on the way down).
  Numbers check out.
- `crates/ui_components/src/lightbox.rs:119-132` — `Params::zoom_factor`
  doc comment. Accurate on the happy path; non-finite contract is
  not explicit (see finding #5).
- `crates/ui_components/src/lightbox.rs:208-225` — renderer clamp
  and `with_max_width(native_size.x() * zoom)` /
  `with_max_height(native_size.y() * zoom)` wiring. Correct
  multiplicative scaling of the bounding box.
- `app/src/workspace/lightbox_view.rs:31-39` — four new
  `FixedBinding`s. Traced parser at
  `crates/warpui_core/src/keymap.rs:834, 897-967` — all four parse
  correctly (see finding #7).
- `app/src/workspace/lightbox_view.rs:72-77` — three new
  `LightboxViewAction` variants with docstrings.
- `app/src/workspace/lightbox_view.rs:92-98` — `zoom_factor` field
  with doc comment.
- `app/src/workspace/lightbox_view.rs:108-113` — `new` constructor
  initialises `animation_start_time` and `zoom_factor` directly,
  bypassing `reset_per_image_state` (see finding #3).
- `app/src/workspace/lightbox_view.rs:117-126` —
  `reset_per_image_state` helper. Two-field body matches the diff.
- `app/src/workspace/lightbox_view.rs:217-237` — `ZoomDirection`
  enum and `step_zoom` free function with NaN/Inf guard, multiply/
  divide branch, and final clamp.
- `app/src/workspace/lightbox_view.rs:331` — `zoom_factor:
  self.zoom_factor` wiring into `lightbox::Params`.
- `app/src/workspace/lightbox_view.rs:374-393` — three new action
  arms (`ZoomIn`, `ZoomOut`, `ZoomReset`) with `if next != current`
  notify-only-on-change guards. Correct.
- `app/src/workspace/lightbox_view.rs:464-510` — five new tests.
  Coverage analysis in finding #6.
- `crates/ui_components/examples/library.rs:567, 602` — two
  `Params` literals updated with `zoom_factor: 1.0`. Both present.
- `specs/GH9729/TIER2_TODO.md:59-63` — t2-7 row flipped to `[x]`
  with "drag-to-pan deferred" annotation.
- `specs/GH9729/TIER2_TODO.md:84` — table row updated to
  `t2-7 | zoom (pan deferred to t2-7-pan) | _pending_ | [x] | [ ] | [ ]`.
  Note the `_pending_` placeholder for the commit hash — R1
  presumably noted this; flagging for completeness.
- `specs/GH9729/TIER2_TODO.md:126-140` — new `t2-7-pan` follow-up
  bullet. Correctly identifies the GPUI fork's missing
  `Translate`/`Offset` primitive as the blocker, names the
  `crates/warpui_core/src/elements/drag_resize.rs` precedent for
  drag tracking, and scopes the follow-up to a separate PR. Matches
  the commit message rationale.
- `crates/warpui_core/src/keymap.rs:834, 897-967` — keystroke
  parser. Verified that `"="`, `"shift-="`, `"-"`, and `"0"` all
  parse without error in both debug and release builds.
- `app/src/util/bindings.rs:296`,
  `app/src/workspace/mod.rs:393, 442`,
  `app/src/notebooks/notebook.rs:169` — pre-existing
  `cmdorctrl-=` zoom bindings, confirming the workspace-level
  convention referenced in the keybinding-registration comment.

# Suggestions

1. (Optional) Route construction through `reset_per_image_state`
   (set placeholder field values in the struct literal, then call
   the helper before `start_asset_loads`) so all four reset sites
   share one definition. Centralises future per-image fields.

2. (Optional) Add a `clamp_zoom_factor(f32) -> f32` to the
   `lightbox` crate and make the three constants private; or drop
   the `// GH9729 §698:` prefix from docstrings. Both reduce
   public-API surface / docstring noise without changing
   behaviour.

3. (Optional) One-sentence addition to the
   `Params::zoom_factor` doc comment noting that callers must pass
   a finite value (the renderer's `clamp` does not sanitise NaN).

4. (Optional) Two cheap test additions: targeted near-min clamp
   (`step_zoom(0.3, Out) == MIN_ZOOM_FACTOR`) and round-trip
   cancellation (`step_zoom(step_zoom(1.0, In), Out) == 1.0`).
   Pin the boundary and the UX expectation respectively.

5. (Optional / future) Add `pan_offset: Vector2F` to `Params` now
   (defaulted to zero, renderer ignores it with a TODO comment) so
   the t2-7-pan PR is purely behavioural rather than
   API-shape-changing.

6. (Optional / future) Register a `"plus"` or layout-aware
   keystroke alongside `=` / `shift-=` once a second European-
   layout user reports that `+` doesn't work for them; same gap
   exists at the workspace-level `cmdorctrl-=` so address codebase-
   wide.

7. (Optional, name-only) Consider `apply_zoom_step` or
   `next_zoom_factor` over `step_zoom`, and `ZOOM_STEP_FACTOR` over
   `ZOOM_STEP`, to emphasise the multiplicative semantics. Pure
   churn; mention only if a name-only pass lands.

# Summary

Verdict: **pass-with-nits**. The diff cleanly delivers the zoom
half of §698, defers drag-to-pan with a well-scoped `t2-7-pan`
follow-up that correctly identifies the missing GPUI primitive,
and lands five focused tests on the extracted `step_zoom` helper.
The headline quality calls — the `step_zoom` extraction, the
`ZoomDirection` enum, and the `reset_per_image_state` helper — are
all the right shape. Two small inconsistencies are worth tightening
before close: `reset_per_image_state` not used at construction
leaves a four-site definition where three would do, and three
`pub const`s in the `lightbox` module ship a slightly larger public
API than a single `pub fn clamp_zoom_factor` would. Documentation
is complete on the happy path but doesn't pin the non-finite
contract on `Params::zoom_factor` (the view sanitises NaN; the
renderer doesn't, so the contract is "callers must pass a finite
value"). Test coverage is solid; two one-line additions (targeted
near-min clamp, round-trip cancellation) would close the obvious
gaps. Nothing here blocks merge — every nit is either a future-
proofing call or a strictly stylistic preference.
