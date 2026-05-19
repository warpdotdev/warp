---
item: tier2-t2-11
commit: 9b51d44
reviewer: R1-correctness
spec_ref: tech.md §698 + §699 (supplemental)
verdict: pass-with-nits
---

# Spec

Quoted verbatim from `specs/GH9729/tech.md`:

§698:

> - **Zoom and pan controls**: extend `lightbox::Params` with zoom state and `lightbox_view.rs` keybindings (`+`, `-`, `0`, drag-to-pan).

§699:

> - **Status footer** (filename, dimensions, file size, format string): extend `lightbox::Params` with an optional metadata strip rendered below the image.

t2-11 supplements §698 (rebinds the listed `+`/`-`/`0` keys to `cmdorctrl-=` / `cmdorctrl--` / `cmdorctrl-0` because the bare-character variants never dispatch) and §699 (extends the dimension metadata line with a current-zoom percentage suffix).

# Findings

- [pass] **Dispatch chain verified.** `init()` registers three `FixedBinding::new(...)` with `view_id = id!(LightboxView::ui_name())` ("LightboxView"). At keypress time, `core::app::dispatch_keystroke` builds a per-view `context_chain` via `contexts_from_responder_chain` (lines 1830-1857) using each view's `View::keymap_context`. LightboxView does not override `keymap_context`, so its context contains only `{"LightboxView"}` (per `core::view::mod.rs:117-121` default). The matcher iterates the chain innermost-first (`.rev()` in `dispatch_keystroke`, line 2007). On the LightboxView-scoped iteration, `Keymap::bindings()` yields editable bindings then fixed-in-reverse-registration; only those whose `context_predicate.eval` is true on the current `Context` survive. The workspace `EditableBinding "workspace:increase_zoom"` (registered at `app/src/workspace/mod.rs:387` with `id!("Workspace")`) evaluates **false** in the LightboxView context (set lacks "Workspace"). The lightbox `FixedBinding` with predicate `id!("LightboxView")` evaluates true and is selected. Result: `LightboxViewAction::ZoomIn` is dispatched, `handle_action` (lightbox_view.rs:451-456) calls `step_zoom`, mutates `self.zoom_factor`, and calls `ctx.notify()`. End-to-end correct.

- [pass] **No shadowing risk.** The commit message states "the more-specific view scope wins". The actual mechanism is stronger than scope-specificity: contexts are matched **per view in the responder chain**, not merged. When LightboxView is focused, its context contains *only* `"LightboxView"` (not `"Workspace"`), so the workspace zoom binding doesn't even enter the match-set on the innermost iteration. Only if the lightbox iteration returns `MatchResult::None` would the loop move outward to a context containing `"Workspace"`. The lightbox binding will always match `cmdorctrl-=` first, so the workspace font-zoom never fires while the lightbox is focused. (The commit-message phrasing is imprecise but the outcome it claims is correct.)

- [minor] **`cmdorctrl-=` does NOT cover `cmd-shift-=`.** Commit message line: "cmdorctrl-= covers both `+` and `=` presses." This claim is incorrect at the matcher level. `Keystroke` derives strict `Eq` over all five modifier booleans plus `key` (`keymap.rs:319-328`); `cmdorctrl-=` parses to `{cmd: true (mac), shift: false, key: "="}` and matches only the event with exactly those values. If the user presses cmd-shift-= (the literal `cmd-+`), the OS will deliver `{cmd: true, shift: true, key: "=" or "+"}` — neither matches the registered binding. Compare `app/src/util/bindings.rs:293` (`IncreaseFontSize => "shift-cmdorctrl-+"`), which proves this codebase registers shift-+ separately when both presses must dispatch. The dropped `shift-=` from t2-7 covered exactly this case. UX impact: a user who habitually presses cmd-+ (with shift) to zoom in will see nothing happen; they must press cmd-= (no shift). This matches the local convention for `CustomAction::IncreaseZoom` (which also only registers `cmdorctrl-=`), so it's consistent with workspace zoom — but the commit message's coverage claim is wrong, and the dropped `shift-=` does remove the only key the old binding had that would fire for cmd-shift-=. Not blocking because (a) it mirrors workspace zoom precedent and (b) cmd-= alone is the documented behaviour, but worth a follow-up to add `shift-cmdorctrl-+` for parity with `IncreaseFontSize`.

- [pass] **`format_metadata_line` numerics.**
  - `1.0` → 100 → no suffix. ✓
  - `1.5` → 150% ✓
  - `2/3` → 67% (rounds 66.67) ✓
  - `0.5` → 50% ✓
  - `1.001` → `(1.001*100).round() = 100` → no suffix. Acceptable: keystroke-driven zoom can only produce 1.0, 1.5, 2.25, 0.667, 0.444, etc. — none land near 1.001. Reachable only via external-caller poison of `Params::zoom_factor`, where the no-suffix outcome is benign.
  - `0.005` → 1% appears in string, but `MIN_ZOOM_FACTOR = 0.25` clamps `step_zoom` output (lightbox_view.rs:307), so unreachable from the view. The renderer-side `Params::zoom_factor` is a `pub` field and an external caller could supply 0.005; the renderer-side string would show "1%". This is acceptable given the renderer side already trusts `Params` for the image scale itself — a clamping defense in the *string* without clamping the scale would be inconsistent. No action needed; matches the posture of the existing NaN sanitisation (defend the user-visible string, accept that other state could be poisoned).
  - `NaN` / `INFINITY` → finite-guard kicks in, `zoom_pct = 100`, no suffix. ✓ Test covers both.

- [pass] **Fractional dimension rounding.** Change from `.x() as i32` (truncate-toward-zero) to `.round() as i32`. Test `format_metadata_line_rounds_fractional_dimensions` covers 199.7/200.3. No other call site in the codebase asserts a specific dimension string for SVG (or any) fixture — only `crates/ui_components/src/lightbox.rs:144` consumes `metadata_line`, treating it as opaque `Option<String>`. No regression risk.

- [pass] **`escape` / `left` / `right` unchanged.** Lines 19-26 retain the original bindings. No regression.

- [pass] **No new dependencies, no out-of-scope edits.** Diff touches only `app/src/workspace/lightbox_view.rs` and `specs/GH9729/TIER2_TODO.md`. `tech.md` not modified. No new crates.

- [nit] **IMPORTANT comment accuracy.** The comment claims unmodified character keys "route to the terminal stdin layer before reaching the lightbox view's action dispatch". I didn't trace the terminal-input path itself (Explore budget) so I cannot fully validate the *reason*, but the commit message attributes it to a manual diagnostic ("t2-6 animation + t2-8 footer work, zoom keys do nothing"). The mechanism is plausible (Warp embeds a PTY input layer that consumes printable characters before the view-action-dispatch path I traced above). The conclusion ("must use modifier-prefixed keys") is consistent with the special-key carve-out for `escape`/`left`/`right` which are non-printable and therefore never sent to stdin. Comment text accurately reflects the observed behaviour even if I haven't validated the exact intercept point.

- [nit] **State of TIER2_TODO.md row.** The new row for t2-11 has `R1 [ ]` / `R2 [ ]` columns and a placeholder "(see commit msg)" in the commit column. The instructions for TIER2_TODO say to flip `R1` to `[x]` only after the review file exists — appropriate that the implementer left it as `[ ]` for the reviewer to flip. Out of scope for this review to flip it (the prompt forbids editing TIER2_TODO.md).

- [pass] **Test coverage.** Five new tests cover: native (no suffix), non-native (50/150/200), accumulated (67%), NaN/INF (no suffix), fractional dimensions (round). Covers the matrix called out in the lens. The native-zoom test uses 1024×768 which exercises the standard footer path.

# What I checked

- `git show --stat 9b51d44`: 2 files (`app/src/workspace/lightbox_view.rs`, `specs/GH9729/TIER2_TODO.md`), 130/16 line additions, no new files, no new deps.
- `git show 9b51d44`: full diff read. New `format_metadata_line` is a free function in the module (not a method on `LightboxView`); both the renderer-side call site at line 411 and the 5 new tests reach it via `super::format_metadata_line`.
- `specs/GH9729/tech.md` lines 698-699 — quoted verbatim above.
- Dispatch trace:
  - `init()` → `app.register_fixed_bindings([...])` (lightbox_view.rs:18-44)
  - `Keymap::register_fixed_bindings` (`crates/warpui_core/src/keymap.rs:391-400`) appends to `self.fixed_bindings`; iteration in `Keymap::bindings()` returns them in `.rev()` order (line 460-465).
  - `core::app::dispatch_keystroke` (`crates/warpui_core/src/core/app.rs:1998-2034`) iterates `context_chain.iter_mut().enumerate().rev()` — innermost-focused view first.
  - `contexts_from_responder_chain` (lines 1830-1857) calls `view.keymap_context(self)` per view; LightboxView inherits the default `View::default_keymap_context` (`core/view/mod.rs:117-121`) which contains only `Self::ui_name()` = `"LightboxView"`.
  - `Matcher::push_keystroke` (`crates/warpui_core/src/keymap/matcher.rs:303-335`) selects the first binding whose `context_predicate.eval(ctx)` is true. The lightbox `FixedBinding` predicate is `id!("LightboxView")` → true on this context.
  - `dispatch_typed_action` routes to `LightboxView::handle_action` (lightbox_view.rs:432) which calls `step_zoom` (line 299-308) and `ctx.notify()`.
- Workspace zoom registration: `app/src/workspace/mod.rs:306-330` (FixedBinding::custom with `id!("Workspace")`, gated on `FeatureFlag::UIZoom`) and `:384-407` (matching EditableBinding `workspace:increase_zoom`, also `id!("Workspace")`). Both have a context predicate that's false in the LightboxView context — no shadowing.
- `Keystroke::parse` (`crates/warpui_core/src/keymap.rs:897-967`): confirmed strict modifier+key equality; `cmdorctrl-=` produces `{cmd: true, shift: false, key: "="}` on mac and `{ctrl: true, shift: false, key: "="}` elsewhere. `cmd-shift-=` is a *distinct* keystroke that this binding does not match.
- `format_metadata_line` walked numerically through all the edge cases listed in the lens.
- Searched for other consumers of `metadata_line` / `× <n> px` outside the modified file: only `crates/ui_components/src/lightbox.rs:144` (treats it as opaque `Option<String>`) and `crates/ui_components/examples/library.rs` (sets `None`). No test depends on a specific dimension string.
- Existing bindings (`escape`/`left`/`right`) are unchanged in the diff.

# Suggestions

- Update the commit message (in a follow-up amend or follow-up commit) to drop "cmdorctrl-= covers both `+` and `=` presses" — it covers only `=` (without shift). If user research shows people press cmd-shift-= for zoom-in, add a fourth `FixedBinding::new("shift-cmdorctrl-+", LightboxViewAction::ZoomIn, view_id.clone())` to match the precedent set by `CustomAction::IncreaseFontSize`.
- Optional: tighten the `IMPORTANT:` doc-comment phrasing from "route to the terminal stdin layer first" to "are consumed by the focused terminal input layer before reaching view-scoped action dispatch" — sidesteps the implementation-detail claim about stdin specifically, since I didn't validate that exact mechanism.
- Optional: add a regression test that verifies `Keystroke::parse("cmdorctrl-=").unwrap()` does NOT equal the keystroke produced by a `cmd-shift-=` press, to make the modifier strictness explicit at the test layer (would also document the trade-off described above).
