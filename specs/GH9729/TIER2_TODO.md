# GH9729 image-preview — Tier 2 follow-up TODO

Authoritative spec: `specs/GH9729/tech.md` §688-713 (do **not** edit).
Branch: `spec/GH9729-image-preview` (this branch).
Predecessor: `IMPLEMENTATION_TODO.md` (v1, complete; awaiting external review).

This file drives a fused ralph-loop for **Tier 2 (UX polish)** of the post-v1
follow-up list in `tech.md` §688-713. The post-v1 *Tier 1* items
(a11y plumbing §692, sibling navigation §693, background-executor decode/stat
§694) are **out of scope** for this loop and will be tracked separately.

## Loop semantics — fused

Each iteration:

1. Read this file. Locate the **Tracker** table.
2. Find the first row with any unchecked box across `Impl | R1 | R2`.
   - If `Impl` is `[ ]`: do just the implementation, commit
     (`GH9729(tier2-impl): <item> — <one-line>`), tick `Impl`, stop.
   - Else if `R1` is `[ ]`: spawn one R1-correctness reviewer, write
     `specs/GH9729/reviews/tier2-<item>-r1.md`, commit
     (`GH9729(tier2-review): <item> R1 — <verdict>`), tick `R1`, stop.
   - Else if `R2` is `[ ]`: same with R2-quality, suffix `-r2.md`.
3. If every row has all three boxes ticked, output
   `<promise>ALL TIER2 ITEMS DONE</promise>` and exit.

Hard rules:

- Touch only the files the current iteration requires. Use the `Explore`
  subagent for codebase lookups; do not grep from the main context window.
- Never edit `specs/GH9729/product.md` or `specs/GH9729/tech.md`.
- If an item's design is under-specified in `tech.md`, surface it in the
  reviewer findings rather than committing a guessed shape. If the impl
  agent cannot proceed without a design call, mark the row's `Impl` cell
  `[blocked]` (not `[x]`) and skip to the next row.
- Run only the narrowest tests for the change. The full presubmit lives in
  the `t2-FINAL` row at the bottom.
- Commit prefix as listed under loop semantics above.
- Reviews use the same frontmatter shape as v1 (`reviewer:`, `verdict:`,
  `spec_ref:`); see `REVIEW_LOOP_PROMPT.md` for the exact template.

## Steps (priority order from `tech.md` §688-713)

- [x] **t2-4.** Convert `ImageType::Unrecognized` to `Err` globally — audit
       every `try_from_bytes` caller, remove the variant, route the error
       through `Result`, update callers to handle the `Err` arm. — `tech.md` §695
- [x] **t2-5.** Adopt `LightboxImageSource::Error` at the artifacts call
       site (`app/src/ai/artifacts/mod.rs:362-365`) so screenshot fetch
       failures use `Error` instead of `Loading + "Failed to load"`. —
       `tech.md` §696
- [x] **t2-6.** Animated GIF/WebP continuous playback. Wire
       `Image::enable_animation_with_start_time(Instant)` into the Lightbox
       image element; drive a per-frame redraw on the focused entry. The
       play/pause sub-layer is deferred — see Deferred R2 follow-ups
       below — because GPUI's `Image` element has no
       `paused_at`/freeze-elapsed primitive today, so a real (continuity-
       preserving) pause needs an upstream API addition rather than a
       call-site hack. — `tech.md` §697
- [x] **t2-7.** Zoom and pan. Extend `lightbox::Params` with zoom/pan state;
       add `+`, `-`, `0`, drag-to-pan keybindings in `lightbox_view.rs`.
       Zoom shipped in this row; drag-to-pan deferred (no
       `Translate`/`Offset` primitive in this GPUI fork — see
       `t2-7-pan` below). — `tech.md` §698
- [x] **t2-8.** Status footer. Extend `lightbox::Params` with an optional
       metadata strip (filename, dimensions, file size, format string)
       rendered below the image. v1 ships dimensions only; filename
       lives in the existing `description` field already, format
       string and file size are deferred (see `t2-8-r2` follow-up). —
       `tech.md` §699
- [x] **t2-9.** EXIF orientation + ICC color profile. Extend the agent-mode
       decoder in `app/src/util/image.rs` and wire into
       `ImageType::try_from_bytes`. EXIF orientation shipped both
       sites in this row; ICC color profile deferred to `t2-9-icc`
       (needs `lcms2`/`qcms` dependency + non-trivial colour-space
       conversion). — `tech.md` §700
- ~~**t2-10.** Visible thumbnail strip — **BLOCKED** on Tier 1 sibling
       navigation (`tech.md` §693). Out of scope for this loop.~~
- [x] **t2-21.** 1.25× zoom step + double-tap zoom-and-center.
       User asked for conventions; mainstream cmd-+ behaviour
       (Preview / Safari / Chrome / Photoshop) is 1.25×, not 1.5×.
       Also adds the macOS Preview / iOS Photos convention of
       double-tap toggling between native and 2× with centering on
       the tap location. New `LightboxViewAction::DoubleTapZoom`
       carries the centering coordinates so zoom and pan apply
       atomically. Math extracted into pure
       `double_tap_zoom_target` helper with 4 new tests. —
       `tech.md` §698.
- [x] **t2-20.** Fix t2-19 pan-state persistence. t2-19 shipped the
       PanClippedImage element but stored `last_drag_position` as a
       plain struct field on the (per-render) element — every
       `Pan` action's `ctx.notify()` rebuilt the element with a fresh
       `None` state, so consecutive drag events lost their context
       and dragging produced one imperceptible step then froze. Move
       drag state to `Arc<Mutex<Option<Vector2F>>>` on the
       *persistent* `Lightbox` struct (same pattern `Button` uses
       for its `MouseStateHandle`). State now survives re-renders.
       — `tech.md` §698.
- [x] **t2-19.** Custom `PanClippedImage` element. Finally fixes the
       t2-7-r1 gotcha that has dogged every zoom iteration since t2-7:
       framework's `ConstrainedBox::layout` won't let a child exceed
       parent's max, so any image already at fit-window-size at zoom
       1.0 can't visibly grow. `PanClippedImage` passes
       `SizeConstraint::strict(zoom*native)` to its child, bypassing
       the parent-max binding. Paint goes through a `ClipBounds`-
       wrapped layer at viewport rect. Drag tracking + cmd+scroll
       collapsed into this single element. Implements the long-
       deferred `t2-7-pan`. — `tech.md` §698 fully addressed.
- [x] **t2-18.** Remove diagnostic logs. Log evidence (11 ZoomIn
       dispatches from a t2-17 run) proved the + button works
       perfectly — closure runs, action dispatches, `zoom_factor`
       updates, footer percentage rises. The user's "nothing happens"
       is the **t2-7-r1 gotcha** I documented and never fixed:
       `ConstrainedBox::layout` tightens the constraint by parent's
       max, so for any image already at fit-window size at zoom 1.0
       (e.g., 1024x1024 in ~1404x800 area = height-bound at 800),
       further zoom can't grow the rendered size. For SMALL images
       (e.g., 200x200 SVG), zoom IS visible up to viewport-bound.
       Proper fix is `t2-7-pan` (let image overflow viewport, clip
       to scrim, drag-to-pan for navigation). — supplements
       `tech.md` §698.
- [x] **t2-17.** Bump diagnostic logs to warn, tighten gaps.
- [x] **t2-16.** Flex::row toolbar + diagnostic logging. After t2-15
       the user reports `+` STILL doesn't fire. The bug persists
       across three layout strategies (Flex::row, individually-positioned
       wide slots, individually-positioned narrow slots). Common
       pattern: `+` is always the rightmost-positioned button and
       always fails. Best theory: separate `add_positioned_child`
       siblings each report a bbox that includes the button's
       interactive hover-padding; the Stack dispatches
       first-added-first, so `−`'s extended bbox swallows clicks
       intended for `+`. The user's "gap" complaint was visible
       evidence of this padding. `Flex::row` partitions its bbox
       precisely between cells, eliminating overlap regardless of
       per-button padding. Diagnostic `log::debug!` lines added in
       each button's on_click closure so a future iteration can see
       whether the click reaches the closure at all. —
       supplements `tech.md` §698.
- [x] **t2-15.** Adjacent zoom icons + conditional reset. User manual
       feedback after t2-13: (a) `+` button STILL doesn't fire even
       after the t2-13 individually-positioned-children restructure —
       most likely cause is the wider "100%" Label button at slot 2
       overlapping into slot 3 (`+`) and swallowing its clicks. (b)
       Layout preference: zoom-out + zoom-in should sit adjacent
       (icon cluster); the "100%" reset should appear only when
       `zoom != 1.0` (not greyed-out — actually hidden). The
       conditional rendering of the wider button independently
       sidesteps the overlap hypothesis: at native zoom the `+`
       button is rightmost with nothing to its right, so its click
       area is unobstructed. — supplements `tech.md` §698.
- [x] **t2-14.** Scrim opacity + toolbar prominence. Manual screenshot
       at 427% zoom showed three layered issues: (a) the scrim's
       current alpha (230/255 = 90%) lets the underlying new-tab
       content bleed through, making the lightbox feel non-modal and
       making the filename / dimensions text hard to read; (b) the
       zoom toolbar blends into the dim scrim with no background
       container, so the `[−] [100%] [+]` cluster is barely visible;
       (c) the t2-7-r1 visual-no-op gotcha is reachable from cmd+scroll
       (footer reports 427% but the image is window-capped) — but
       fixing that needs drag-to-pan (deferred as `t2-7-pan`). This
       row addresses (a) and (b). — supplements `tech.md` §697-699.
- [x] **t2-13.** Polish the t2-12 zoom toolbar after manual feedback:
       (a) Zoom-in `+` button doesn't fire on click — likely a Flex-row
       inside `add_positioned_child` hit-test routing issue, so
       restructure to three individually positioned buttons (mirroring
       the existing prev/next button placement pattern). (b) Replace
       the silly `Icon::Refresh` reset glyph with a text `100%` label
       button. (c) Make the reset button disabled when
       `zoom_factor == 1.0` (no-op state). — supplements `tech.md`
       §698.
- [x] **t2-12.** GUI zoom controls. Manual test revealed
       `cmdorctrl-=` from t2-11 does NOT shadow the workspace
       font-zoom binding in practice — pressing cmd-= zooms the
       terminal font behind the lightbox. R1-t2-11's theoretical
       analysis was wrong (LightboxView likely doesn't actually
       claim keyboard focus on open; escape/left/right work via a
       different routing path that modifier-prefixed keys don't
       take). Solution: drop the keyboard bindings entirely, add
       three GUI buttons (zoom-out, reset, zoom-in) to the
       lightbox toolbar, and add `cmd`+scroll-wheel zoom for power
       users (cmd-modifier prevents accidental trackpad zoom;
       matches macOS Preview convention). — supplements `tech.md`
       §698.
- [x] **t2-11.** Fix t2-7 zoom keybinding routing + add visual zoom
       indicator. `FixedBinding::new("=", ...)` / `"-"` / `"0"` never
       dispatch in a Warp terminal context because unmodified
       character keys route to the terminal stdin layer first; only
       special keys (`escape`/`left`/`right`) and modifier-prefixed
       keys reach view-scoped action bindings. Surfaced by manual
       diagnostic: t2-6 animation + t2-8 footer both work, but
       zoom keys do nothing. Rebind to `cmdorctrl-=` /
       `cmdorctrl--` / `cmdorctrl-0` (matching the existing local
       convention in `app/src/util/bindings.rs`) and drop the
       redundant `shift-=`. Append a zoom-percentage suffix to the
       metadata footer (e.g. `"1024 × 1024 px · 150%"`) when
       `zoom_factor != 1.0` so the user gets visual feedback even
       when the image is window-capped (the t2-7-r1 gotcha). —
       supplements `tech.md` §698.
- [x] **t2-10.** Fix `start_asset_load` synchronous-`FailedToLoad`
       gap. The post-load rewrite callback installed by
       `start_asset_load` only fires for the
       `AssetState::Loading { handle }` branch; a synchronously-
       resolved `FailedToLoad` (reachable after t2-4 made
       `ImageType::try_from_bytes` return `Err` immediately for
       mislabeled tiny files) silently falls through, leaving the
       lightbox stuck on the loading spinner forever. Apply
       `rewrite_image_for_load_state` inline against the initial
       `load_asset` state, and only schedule the spawn for the
       `Loading` arm. Add a unit test for the synchronous
       `FailedToLoad` path. Surfaced by manual test of
       `06-mislabeled.png` post t2-FINAL. — supplements `tech.md`
       §182 / §695.
- [x] **t2-FINAL.** Presubmit. `cargo fmt` applied (cosmetic-only:
       one `view_id` line collapse, one import reorder in
       `app/src/util/image.rs`). `cargo clippy --workspace --exclude
       command-signatures-v2 --all-targets --tests -- -D warnings`
       clean. `cargo nextest run --no-fail-fast --workspace --exclude
       command-signatures-v2` ran 5938 tests, 9 failures — all
       pre-existing environmental tests identical to the v1 FINAL
       pattern (SSH integration × 6, git tag display, settings
       migration marker, plus one flaky `ui_tests::test_active_session_follows_focus`
       that passed on rerun). None of the failures touch
       image-preview code paths.

## Tracker

| # | Item | Impl commit | Impl | R1 | R2 |
|---|------|-------------|------|----|----|
| t2-4 | `Unrecognized` → `Err` globally | `7780d31` | [x] | [x] | [x] |
| t2-5 | adopt `Error` at artifacts call site | `5a8072a` | [x] | [x] | [x] |
| t2-6 | animated playback (continuous; pause deferred) | `f077496` | [x] | [x] | [x] |
| t2-7 | zoom (pan deferred to t2-7-pan) | `6aee220` | [x] | [x] | [x] |
| t2-8 | status footer (dimensions only) | `d9cc0c3` | [x] | [x] | [x] |
| t2-9 | EXIF orientation (ICC deferred to t2-9-icc) | `3e694be` | [x] | [x] | [x] |
| t2-FINAL | presubmit | `611ec2b` | [x] | — | — |
| t2-10 | sync-`FailedToLoad` rewrite | `af7d5f5` | [x] | [x] | [x] |
| t2-11 | zoom keys + visual indicator | `9b51d44` | [x] | [x] | [x] |
| t2-12 | GUI zoom buttons + scroll-zoom | `65b2f56` | [x] | [x] | [x] |
| t2-13 | toolbar polish + fix + button | `a655650` | [x] | [x] | [x] |
| t2-14 | scrim opacity + toolbar prominence | `46f0a2e` | [x] | [x] | [x] |
| t2-15 | adjacent icons + conditional reset | `6623e0e` | [x] | [x] | [x] |
| t2-16 | Flex::row toolbar + logging | `ae67790` | [x] | [x] | [x] |
| t2-17 | warn-level logs + tighter gaps | `dff6822` | [x] | [x] | [x] |
| t2-18 | remove diagnostic logs | `45ccfe2` | [x] | [x] | [x] |
| t2-19 | custom PanClippedImage element | `67f014b` | [x] | [x] | [x] |
| t2-20 | pan state on persistent struct | `c102817` | [x] | [x] | [x] |
| t2-21 | 1.25× step + double-tap zoom-and-center | `d28f6f3` | [x] | [x] | [x] |

Tick `[x]` only after the corresponding artifact (commit for `Impl`, review
file for `R1`/`R2`) exists and contains real content. Empty stubs do not
count.

## Deferred R2 follow-ups

Per the loop's "surface, don't fix" rule, R2-quality nits are recorded
here for an off-loop cleanup pass after the main tier-2 list lands.

- **t2-4-r2.** (1) No regression test loads garbage bytes through
  `ImageType::try_from_bytes` and asserts the resulting "could not
  detect image format" string — future wording drift would silently
  break `sanitize_load_error`'s prefix match. (2) The rewritten test
  `post_load_callback_rewrites_unrecognized_to_error` still carries
  the legacy variant name; rename to reflect the `FailedToLoad` path.
  (3) No direct unit test of `sanitize_load_error` proves the
  "could not detect" branch sits ahead of the generic
  "decode/format" branch — a swap regression would silently widen
  the bucket. — `reviews/tier2-t2-4-r2.md`.
- **t2-6-r1.** Post-asset-load callback in
  `app/src/workspace/lightbox_view.rs::start_asset_load` does NOT
  reset `animation_start_time`. If the bytes take ~600 ms to fetch
  + decode, the animation plays from `elapsed = 600 ms` instead of
  frame 0. One-line fix in the spawn closure; not breaking but
  user-visible on slower networks. — `reviews/tier2-t2-6-r1.md`.
- **t2-6-r2.** Stylistic only: (a) the conditional builder chain
  `let mut image_builder = … ; if let Some(start) = … { … } let
  image = …` is slightly awkward; consider an extension-trait
  helper once a second `enable_animation_with_start_time` consumer
  appears in `ui_components/`. (b) `animation_started_at` would
  mirror the GPUI builder param name marginally better than
  `animation_start_time`. (c) The inline `// GH9729 §697:` comments
  on `NavigatePrevious` / `NavigateNext` partly duplicate the
  field-level doc comment. (d) The `t2-6-pause` rewrite is the
  natural moment to migrate `Option<Instant>` →
  `enum AnimationState { Off, Playing { started_at: Instant },
  Paused { … } }`. — `reviews/tier2-t2-6-r2.md`.
- **t2-7-r1.** (a) Renderer's `params.zoom_factor.clamp(MIN, MAX)`
  does NOT sanitise NaN: `f32::NAN.clamp(0.25, 8.0)` returns NaN
  (per the f32 spec), which would NaN-poison the `ConstrainedBox`
  size if any external caller built `Params` with a non-finite
  zoom. The view's `step_zoom` short-circuits non-finite to 1.0,
  so the in-tree path is safe; the gap is the public `Params`
  contract. Replace the renderer-side `f32::clamp` with a helper
  that mirrors `step_zoom`'s `is_finite` guard, or document the
  contract on `Params::zoom_factor` so external callers must pass
  finite values. (b) `ConstrainedBox::layout` caps the constraint
  at `min(parent_max, self.with_max)`, so zoom-in beyond
  "fill-window" is a *visual no-op* for any image already filling
  the window — pressing `=` on a 4K image inside a 1080p window
  does nothing visible until t2-7-pan ships. Document this on
  `Params::zoom_factor` so it doesn't read as a regression. —
  `reviews/tier2-t2-7-r1.md`.
- **t2-7-r2.** Stylistic / hygiene: (a) `reset_per_image_state` is
  bypassed in `LightboxView::new` (direct field init), splitting
  the "what counts as per-image state" definition across four
  sites; refactor `new` to let-mut-then-call. (b) `MIN_ZOOM_FACTOR`,
  `MAX_ZOOM_FACTOR`, `ZOOM_STEP` ship as `pub const`s — consider
  collapsing to a single `pub fn clamp_zoom_factor(f32) -> f32`
  helper to keep the API surface tight. (c) Add two cheap tests:
  `step_zoom(0.3, Out) == MIN_ZOOM_FACTOR` (targeted boundary
  clamp) and `step_zoom(step_zoom(1.0, In), Out) == 1.0`
  (out-cancels-in round-trip pins the UX expectation). (d) Doc
  comment on `Params::zoom_factor` should pin the non-finite
  contract callers must honour. (e) Adding `pan_offset: Vector2F`
  as a placeholder in `Params` now would make t2-7-pan purely
  behavioural and avoid a future public-API churn. (f) European-
  keyboard `+` ergonomics: the same gap as the workspace-level
  `cmdorctrl-=` zoom — out of scope for t2-7 but worth a
  codebase-wide future fix. —
  `reviews/tier2-t2-7-r2.md`.
- **t2-8-r1.** The metadata strip in `lightbox.rs::content_with_description`
  is gated on `current_description.is_some() && native_size.is_some()`,
  so when an image is loaded with a `None` description (reachable via
  `app/src/ai/artifacts/mod.rs:348-355` for screenshots without a
  description) the metadata strip is silently dropped together with
  the (absent) description. The fix: split the gate so the metadata
  strip renders whenever `native_size.is_some()`, independent of the
  description's presence. — `reviews/tier2-t2-8-r1.md`.
- **t2-8-r2.** (a) Promote `metadata_line: Option<String>` to a
  structured `LightboxMetadata { dimensions, file_size, format }`
  shape when t2-8-r2 adds format and file-size — keeps the
  formatting in the renderer, consistent across callers, testable
  without round-tripping a string. (b) Module-top constants in
  `lightbox.rs` are now split across three locations; one pass to
  regroup. (c) Field name `metadata_line` is singular while §699
  calls it a "metadata strip"; rename. (d) `size.x() as i32`
  truncates toward zero — switch to `.round() as i32`. (e) The
  field's doc comment forward-promises format/size that no v1
  caller supplies; tighten until those land. (f) Add format
  derivation: post-decode `ImageType` doesn't carry the codec, so
  sniff it from the asset-source path extension at the
  `LightboxView::render` site. (g) Add file-size: blocked on
  v1 §694 (background-executor stat) — once that lands, the
  size can come from the same stat the §119 size-cap check
  already does. — `reviews/tier2-t2-8-r2.md`.
- **t2-9-r1.** (a) §700 covers EXIF orientation AND ICC together;
  this commit is a real partial-completion (well-tracked via
  `t2-9-icc`). (b) No regression-guard test for the EXIF path —
  could be done with a hand-rolled `&[u8]` JPEG literal carrying
  an Orientation=6 tag; (c) Small EXIF-tagged JPEGs now incur one
  extra re-encode round-trip (necessary for correctness — worth a
  code-comment note for future maintainers). — `reviews/tier2-t2-9-r1.md`.
- **t2-13-r1.** (a) Prev/next buttons at `lightbox.rs:451, 479`
  still use the raw `12.` literal instead of `SCRIM_BUTTON_INSET`,
  leaving the "one source of truth" consolidation incomplete. (b)
  Commit message claim "step_zoom never produces exactly 1.0 from
  non-1.0 inputs" is wrong — `1.5 / 1.5 = 1.0` exactly in IEEE-754.
  This strengthens (not weakens) the correctness of the
  `zoom == 1.0` disabled check: a user zoom-in/zoom-out round-trip
  lands on exactly 1.0 and the 100% button correctly disables. —
  `reviews/tier2-t2-13-r1.md`.
- **t2-13-r2.** (a) `ZOOM_BUTTON_SLOT_WIDTH = 56.` is a calibrated
  magic number — well-commented technical debt but worth a
  follow-up tracker bullet (proper hit-test-correct flex wrapper
  or measured button width). (b) `disabled: zoom == 1.0` strict
  float equality is provably correct but warrants a one-line
  justification comment to defuse the obvious `f32::EPSILON`
  review question. (c) `SCRIM_BUTTON_INSET` unification is
  incomplete — same as r1.(a). —
  `reviews/tier2-t2-13-r2.md`.
- **t2-12-r1.** Minor nits all non-blocking: (a) cmd+scroll only
  fires when the cursor is over the image rect rather than anywhere
  over the scrim — extending the scroll handler to the scrim is a
  small UX improvement. (b) Toolbar's BottomLeft anchor risks visual
  crowding against the centred description on narrow windows. (c)
  Scroll-direction comment misdescribes natural-scroll semantics. (d)
  Dead-zone is magic-numbered. (e) No unit tests for new wiring
  (acceptable since it mirrors the already-untested on_navigate path).
  — `reviews/tier2-t2-12-r1.md`.
- **t2-12-r2.** (a) Add `SCRIM_BUTTON_INSET = 12.` const to deduplicate
  four literal `12.` offsets in `lightbox.rs` without conflating
  with `SCRIM_PADDING = 48.`. (b) `Icon::Refresh` for zoom-reset is
  the least-bad of available glyphs — the icon enum lacks
  `ZoomReset`/`OneToOne`/`1:1` variants. Right fix is to add a glyph
  rather than change the choice here — OR use a `Content::Label("100%")`
  text button (addressed in t2-13). (c) `SCROLL_ZOOM_DEAD_ZONE = 1.0`
  is plausible for precise (trackpad, pixel-unit) deltas but borderline
  for non-precise (classic wheel, line-unit) deltas. Branching on
  `Event::ScrollWheel.precise` would be cleaner. (d) Extract
  `dy → Option<ZoomDirection>` decision (3 lines around the dead-zone)
  into a pure helper for unit-testability. — `reviews/tier2-t2-12-r2.md`.
- **t2-11-r1.** Commit-message claim that `cmdorctrl-=` "covers both
  `+` and `=` presses" is mechanically wrong. `Keystroke` derives
  strict `Eq` across all five modifier booleans plus the key, so
  `cmd-shift-=` (= literal `cmd-+` on a US layout) is a distinct
  keystroke and won't match `cmdorctrl-=`. Compare
  `app/src/util/bindings.rs:293` `IncreaseFontSize`, which registers
  both `cmdorctrl-=` AND `shift-cmdorctrl-+`. The lightbox loses
  the muscle-memory `cmd-+` zoom-in for the same posture as the
  workspace-level zoom — defensible (matches `CustomAction::IncreaseZoom`
  which is `cmdorctrl-=` only) but worth adding the shift variant
  for keyboard ergonomics parity. — `reviews/tier2-t2-11-r1.md`.
- **t2-11-r2.** (a) Add `use pathfinder_geometry::vector::Vector2F`
  at the top of `lightbox_view.rs` so the deep path doesn't repeat
  in the helper signature + 5 tests. (b) File a t3 follow-up for a
  view-harness regression test that asserts bare `=`/`-`/`0`
  keystrokes do NOT fire zoom (mirroring `image_preview_arm_builds_*`
  at `view_test.rs:3029`) — the dispatch-layer constraint is too
  easy to silently revert otherwise. (c) Backfill the real SHA
  `9b51d44` into the tracker — same loop-hygiene pattern as t2-10. —
  `reviews/tier2-t2-11-r2.md`.
- **t2-10-r1.** (a) The t2-10 commit message claims `06-mislabeled.png`
  hits the synchronous-`FailedToLoad` path on the *first* `load_asset`
  call. R1 verified against `crates/warpui_core/src/assets/asset_cache.rs:320-326`
  that `load_asset` for an `AssetSource::LocalFile` always inserts
  `loading()` and spawns via `load_asynchronously` on a cold cache,
  so the synchronous reach is actually via warm-cache hits,
  `AssetSource::Bundled`, or `AssetSource::Raw`. The fix is still
  necessary (§182 / §695 require it across all sources) but the
  headline repro narrative deserves either a corrected stack or a
  pointer at the warm-cache path. (b) `apply_rewrite_to_slot_leaves_loading_state_alone`
  uses `AssetState::Evicted` as a stand-in for "not Loading,
  not failed" rather than a true `Loading { handle }` — exercises
  the same `_ => None` branch but doesn't pin the `Loading`-specific
  contract. Worth tightening with either a direct-`Loading`-handle
  assertion or an additional `Loaded` no-op test. —
  `reviews/tier2-t2-10-r1.md`.
- **t2-10-r2.** (a) Rename
  `apply_rewrite_to_slot_leaves_loading_state_alone` to
  `apply_rewrite_to_slot_leaves_non_failure_states_alone` — the
  current name over-promises since the body uses `Evicted`, not
  `Loading`. (b) Four-line `// GH9729 §695 / t2-10:` comment inside
  the helper partially duplicates the helper's own doc; collapse.
  (c) Helper placement at the closing of `impl LightboxView`
  separates it from its peer `rewrite_image_for_load_state` higher
  in the file; consider regrouping. (d) "Tiny" framing in the doc
  comment implies a size threshold that isn't the actual mechanism
  (the mechanism is warm-cache, not file size); tighten. (e) No
  end-to-end view-harness regression guard against a future
  contributor removing the inline call — file as a t3 follow-up
  (view-harness exists per `app/src/code/editor/comment_editor_tests.rs:91`).
  — `reviews/tier2-t2-10-r2.md`.
- **t2-9-r2.** (a) `decoder.orientation()?` propagates `ImageError`
  from a corrupt EXIF segment, so a malformed-EXIF-but-otherwise-decodable
  JPEG newly fails. Recommend
  `decoder.orientation().unwrap_or(Orientation::NoTransforms)` at
  both sites. (b) The "preserving v1 zero-copy behaviour" doc
  comment over-promises — v1 skipped the *decode*, the new path
  only skips the *output copy*; tighten the comment. (c) Doc
  comment mentions HEIC, which isn't in `SUPPORTED_IMAGE_MIME_TYPES`;
  remove or qualify. (d) Two-site duplication of the
  `into_decoder → orientation → from_decoder → apply_orientation`
  pattern is acceptable now; revisit if `t2-9-icc` adds a third
  site. (e) `crates/warpui_core/test_data/` already hosts
  `local.png` / `animated.webp`, so a tiny EXIF-tagged JPEG
  fixture is a natural follow-up — the next t2-9 round should
  clear that bar. — `reviews/tier2-t2-9-r2.md`.
- **t2-9-icc.** ICC color profile flattening to sRGB. The `image`
  crate exposes `ImageDecoder::icc_profile() -> ImageResult<Option<Vec<u8>>>`
  for the JPEG/PNG/WebP decoders, so reading the embedded profile is
  free. Applying it requires either `lcms2` / `qcms` (CMYK and named
  colour-space conversion) or a hand-rolled limited path
  (sRGB-→sRGB shortcut for the `cHRM`-only PNG case, plus a generic
  reject for anything else). Either way it's a meaningful crate
  dependency or a new colour-management module under
  `crates/warpui_core/`. Belongs in a separate PR; tracked here so
  v1.x can consider whether to ship without ICC and accept slight
  colour drift on wide-gamut displays. The lightbox decoder in
  `crates/warpui_core/src/image_cache.rs` and the agent-mode decoder
  in `app/src/util/image.rs` should adopt the same path once
  available.
- **t2-7-pan.** Drag-to-pan for a zoomed-in image. This GPUI fork has
  no `Translate`/`Offset`/`Transform` primitive that lets us shift an
  element during paint, so applying a `pan_offset: Vector2F` to the
  lightbox image needs an upstream addition: either a new
  `Translate { dx: f32, dy: f32, child }` element under
  `crates/warpui_core/src/elements/`, or a `paint_at` API on the
  `PaintContext` that lets a wrapper element bias the child's paint
  origin. The drag tracking itself is straightforward — clone
  `crates/warpui_core/src/elements/drag_resize.rs`'s
  `Arc<Mutex<DragState>>` pattern and capture
  `LeftMouseDown`/`LeftMouseDragged`/`LeftMouseUp` on the image's
  `EventHandler`. The blocker is purely the rendering primitive.
  Belongs in a separate PR that adds the GPUI element first; this
  bullet captures the design call needed.
- **t2-6-pause.** Play/pause control for the lightbox's animated
  playback. Real pause-resume needs `Image` (GPUI element in
  `crates/warpui_core/src/elements/image.rs`) to expose either a
  `paused_at: Option<Instant>` field or a frozen-elapsed-millis
  parameter so `paint_animated_image` can hold the current frame
  without skipping `ctx.repaint_after`. The two call-site-only
  workarounds (rebuild `started_at = now() - paused_elapsed`, or
  drop `enable_animation_with_start_time` while paused) either
  silently keep advancing the frame or jump back to frame 0 on
  resume — neither is acceptable as v1.x UX. Belongs in a separate
  PR that touches the GPUI element first; tracked as a sub-bullet
  here so t2-6 can land its primary deliverable.
- **t2-5-r2.** (1) Categorical `LightboxImageSource::Error` messages
  now live in two modules (`lightbox_view.rs::sanitize_load_error`
  plus the artifacts call site); consider a shared catalog once the
  third site lands. (2) `LightboxImage` lacks a constructor helper
  (e.g., `LightboxImage::error(message)`) so each call site uses
  verbose struct-literal form. (3) Test name
  `surfaces_error_variant_for_screenshot_load_errors` is accurate
  about the variant but does not capture the sanitization-of-error-text
  property the body asserts. — `reviews/tier2-t2-5-r2.md`.
