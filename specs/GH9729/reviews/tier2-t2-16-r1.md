---
item: tier2-t2-16
commit: ae67790
reviewer: R1-correctness
spec_ref: tech.md §698 (supplemental)
verdict: pass-with-nits
---

# Spec

t2-16 is a debug-driven layout iteration on top of t2-15: wrap the
two icon zoom buttons (`−`/`+`) in a `Flex::row` so the row's bbox is
partitioned between cells, and add `log::debug!` lines in each zoom
button's `on_click` closure so the next iteration can confirm whether
the `+` click is actually reaching the closure. The conditional `100%`
reset button stays as a separate positioned child to the right of the
icon cluster, gated on `zoom != 1.0` exactly as t2-15 left it.

# Findings

- [pass] The change adopts `Flex::row().with_spacing(0.0).with_cross_axis_alignment(Center)` as
  described, replacing the two `add_positioned_child` calls for `−`/`+` with one positioned-child
  call carrying the row. Conditional `if zoom != 1.0` reset block is preserved unchanged in
  behavior and re-uses the existing `ZOOM_ICON_BUTTON_SLOT`/`ZOOM_RESET_GAP_FROM_ICONS` constants
  for its offset, so the t2-15 conditional-reset semantics are kept intact.
- [pass] `log::debug!` lines are added in all three button on_click closures
  (`zoom_out_button`, `zoom_in_button`, `zoom_reset_button`). `log` is correctly declared as a
  workspace dependency in `crates/ui_components/Cargo.toml` and `log = { version = "0.4" }` is
  already a workspace dep in the root `Cargo.toml`.
- [pass] No regression for the error-scrim path: the modified block is gated on
  `if let Some(on_zoom) = params.options.on_zoom`, and the error-state nav buttons earlier in
  `Component::render` are untouched. `ZOOM_RESET_GAP_FROM_ICONS` is still referenced by the
  reset-button offset and therefore not dead.
- [major] The commit's *causal theory* does not match how hit-test actually works in this
  codebase. `Hoverable::is_mouse_over_element`
  (`crates/warpui_core/src/elements/hoverable.rs:361`) tests
  `visible_rect(origin, size).contains_point(position) && !is_covered(point)` — there is no
  "hover hit-area beyond visual edge", so it is structurally impossible for `−`'s bbox to extend
  into `+`'s visual area. Stack's `dispatch_event` is `Broadcast` in release
  (`stack/mod.rs:299`) and Waterfall-reverse in debug (`stack/mod.rs:103`), so order alone does
  not "claim" a click; bounds-containment plus z-index occlusion (`is_covered`) decide. With the
  pre-fix `add_positioned_child` order (`−` first, then `+`), the `+` button is painted
  *later* on a new `start_layer` (`stack/mod.rs:236`) → higher z → it is *not* occluded by `−`.
  The Flex::row layout is a fine cleanup but it is unlikely to be the actual fix for the user
  report; whatever fires when the user clicks `+` is more plausibly an issue inside the
  button/keymap/zoom-step layer. This is exactly what the diagnostic `log::debug!` lines will
  reveal in the next iteration — so the path forward is right, but the commit message
  overstates the diagnosis.
- [minor] The diagnostic `log::debug!` lines are not compile-time gated. The repo does not set
  `log` features such as `release_max_level_info`, so in a release build the macro still
  emits records (filtered at runtime by the logger). For a debug iteration this is fine, but
  these three call sites need to come out before any merge to master — the commit body
  acknowledges this implicitly ("if `+` STILL fails…") but there is no `// TODO: remove`
  marker on the log lines themselves. Worth a follow-up to either drop them when the saga
  closes or convert one survivor to a proper `log::trace!` behind a feature.
- [nit] Comment on the reset block ("Since the icon cluster width depends on button rendering
  and we can't measure it from here, the reset's offset is estimated from
  `ZOOM_ICON_BUTTON_SLOT * 2 + ZOOM_RESET_GAP_FROM_ICONS`") concedes a layout estimate, but
  now that the cluster IS a Flex::row, a slightly cleaner alternative — wrapping
  `icon_cluster` + `zoom_reset_button` in an outer `Flex::row` with `ZOOM_RESET_GAP_FROM_ICONS`
  spacing and adding *that* as one positioned child — would remove the magic-offset math.
  Non-blocking for this commit since t2-17/18/etc. were explicitly diagnostic-driven.
- [nit] Tests claim ("lightbox_view tests untouched, still 18/18") is fine for `pass-with-nits`,
  but no test exercises the zoom-toolbar layout, so this is a no-op assertion in terms of
  proving the fix.

# What I checked

- `git show ae67790 --stat` and full diff for `crates/ui_components/src/lightbox.rs`,
  `crates/ui_components/Cargo.toml`, and `Cargo.lock`.
- `crates/ui_components/src/lightbox.rs@ae67790`:
  - Imports of `Flex`/`CrossAxisAlignment` via `warpui::prelude::stack::*` and `prelude::*`
    (used elsewhere at lines 643, 681, etc., so resolution is fine).
  - `ZOOM_ICON_BUTTON_SLOT` and `ZOOM_RESET_GAP_FROM_ICONS` constants (lines 18–34) still
    referenced by the reset-offset math (lines 603–608).
  - `if zoom != 1.0` conditional reset block preserved.
  - All three `log::debug!` sites: 533, 549, 593.
- `crates/ui_components/Cargo.toml`: added `log.workspace = true` in `[dependencies]`.
- Root `Cargo.toml` line 180: `log = { version = "0.4", features = ["serde", "std"] }`,
  no `release_max_level_*` feature anywhere in the repo (so debug records ARE emitted at
  runtime in release builds, just filtered by the logger).
- `crates/warpui_core/src/elements/stack/mod.rs`: `EventDispatchMode` default is `Broadcast`
  in release / `Waterfall` in debug (lines 102–106); `dispatch_event` (lines 290–318) confirms
  Broadcast hits every painted child; per-child `start_layer` in `paint` (line 236)
  monotonically advances z-index in add order.
- `crates/warpui_core/src/elements/flex/mod.rs:498`: `Flex::dispatch_event` also iterates ALL
  children, so the "Flex partitions hit-test" claim in the commit message is true only at the
  *layout* level (cells don't overlap geometrically) — not at the *dispatch* level.
- `crates/warpui_core/src/elements/hoverable.rs:361` (`is_mouse_over_element`) — exact
  containment + `is_covered`; no padding extension beyond `origin`/`size`.
- `specs/GH9729/tech.md` §698: supplemental "Zoom and pan controls" entry; no normative
  layout prescriptions, so t2-16 is non-normative iteration as expected.
- Searched `crates/ui_components/` for log-assertion tests and `assert.*log` — none, so the
  three new `log::debug!` lines don't break any test contract.
- Confirmed error-scrim rendering path is gated separately (earlier `LightboxImageSource::Error`
  branch with its own `add_positioned_child` for nav buttons); the modified block is inside
  the `on_zoom.is_some()` toolbar branch only.

# Suggestions

1. Either rewrite or soften the commit message paragraph that asserts `Button::Size::Small`
   has "interactive hover padding [extending] past the visual edge" — the codebase's
   `Hoverable::is_mouse_over_element` is exact-bounds, so this is not the actual mechanism.
   The honest framing is: "Cluster the two icons under a Flex::row to remove geometric
   ambiguity, and add diagnostic logging so we can verify what `+` actually dispatches to."
2. Add a `// TODO(GH9729 t2-16): remove diagnostic before merge` near each of the three
   `log::debug!` sites so they don't survive into a release commit by accident. (Even though
   `log::debug!` is filtered at runtime by default, the call sites are noise in production
   binaries.)
3. When closing the saga, consider replacing the two-positioned-child layout
   (`icon_cluster` + `zoom_reset_button`) with one outer `Flex::row` carrying both plus a
   `with_spacing(ZOOM_RESET_GAP_FROM_ICONS)` — drops the magic-offset arithmetic on the reset
   button and removes the only remaining "two stack siblings near each other" pattern in the
   toolbar.
