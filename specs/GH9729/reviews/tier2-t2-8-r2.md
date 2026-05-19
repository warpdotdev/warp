---
item: tier2-t2-8
commit: d9cc0c3
reviewer: R2-quality
spec_ref: tech.md §699
verdict: pass-with-nits
---

# Spec

> **Status footer** (filename, dimensions, file size, format string): extend `lightbox::Params` with an optional metadata strip rendered below the image.

# Findings

- **API shape (Q1).** `Option<String>` is a defensible v1 choice — at this
  scope there's exactly one caller and one renderer, and option (b) the
  structured `LightboxMetadata { dimensions, file_size, format }` is the
  shape this will *want* to be once t2-8-r2 lands. The downside of the
  current shape is real but small: when format and file-size plumb in,
  the formatter lives in `lightbox_view.rs` rather than in the renderer,
  so each future caller (artifacts, chat-image, future agent surfaces)
  re-implements `"<dim> · <fmt> · <size>"` and they will inevitably drift.
  Not a blocker for this commit, but worth flagging in TIER2_TODO that
  t2-8-r2 should probably promote this to a struct rather than just
  appending more fields to the same `format!`. Nit.
- **Naming (Q2).** `metadata_line` is singular and the spec calls it a
  "metadata strip" — `metadata_strip` would mirror the spec exactly and
  not foreclose a multi-line layout. `status_footer` is also reasonable
  and matches the commit subject. The current name isn't *wrong*, just
  slightly narrower than the concept. Nit.
- **Constants placement (Q3).** Inconsistent. `DESCRIPTION_SPACING`,
  `LIGHTBOX_TEXT_SIZE_DELTA`, the zoom constants, and `SCRIM_PADDING`
  live in a module-top band; the new `METADATA_TEXT_SIZE_REDUCTION`
  /`METADATA_TEXT_ALPHA` sit between `SCRIM_PADDING` and the zoom
  constants while their sibling visual constants
  (`DESCRIPTION_SPACING`, `LIGHTBOX_TEXT_SIZE_DELTA`) sit *after* the
  zoom constants. Result: the file now has the metadata visual
  constants at lines 18-32 and the description visual constants at
  lines 52-53, with zoom constants between them. Either group all
  visual constants together (preferred) or all the §699 constants
  together. Not load-bearing but it's the kind of thing the next
  contributor will spend a minute trying to parse. Nit. They are not
  worth being `pub` — there's no external caller that needs to match
  this styling, and exposing them locks a v1-internal aesthetic
  decision into the public API.
- **Helper functions (Q4).** Reasonable. Each is one expression today
  but the `.max(8.0)` floor on `metadata_text_size` is a real piece of
  logic (prevents unreadably small text on minimum UI font sizes), and
  collecting both behind named helpers means the call site reads as
  "metadata text" rather than as raw arithmetic. The pattern matches
  the existing `lightbox_text_size` / `scrim_color` helpers in the
  same file. Pass.
- **Truncate-toward-zero cast (Q5).** `size.x() as i32` truncates
  toward zero. For a Vector2F coming from a decoded image,
  `current_image_native_size` is sourced from
  `Image::size().to_f32()` paths and will normally be integral, so
  truncation matches reality for raster. SVG renders via the rasterized
  pixmap and is also integral. The fractional-pixel concern is
  theoretically real but not exercised by any current source.
  `.round() as i32` would be a one-character defensive improvement and
  closes the door on a future SVG path that rasterizes at fractional
  scale. Nit.
- **Code organisation (Q6).** See Q3. The mix of visual + logic
  constants at module top is already mildly disorderly pre-this-commit
  (DESCRIPTION_SPACING sits *after* the zoom constants), and this
  commit makes the ordering slightly worse by inserting the metadata
  constants in a *third* location. Worth one cleanup pass that groups
  constants by axis. Nit.
- **Test rigor (Q7).** Searched `crates/ui_components/` for any
  `#[test]` / `#[gpui::test]` invocations and any element-tree
  assertion harness — none exist. The crate has no test files at all.
  Element-tree assertions would require either a mock `RenderContext`
  or a snapshot harness, neither of which the v1 Lightbox work has
  introduced. The deferral is fine and the commit message correctly
  scopes this to v1.x's broader rendering harness. Pass.
- **Comment density (Q8).** The doc comment on `metadata_line`
  (8 lines) and the inline §699 comment at the call site in
  `lightbox_view.rs` (10 lines explaining why format + size are
  deferred) overlap on the "what". The call-site comment is justified
  because it's the *deferral rationale* (why it's `None` for those
  two fields), which doesn't belong on the field doc. So the overlap
  is small and intentional — keep both. Pass.
- **Doc comment promises future fields (Q9).** "the strip typically
  carries `<width>×<height>` plus format / size when the caller knows
  them" — strictly accurate (the field is a free-form `Option<String>`
  so a caller *could* put format/size in it today), but a reader of the
  v1 API will look for format/size and not find them in any caller.
  Reads as forward-looking without being false. A one-word tweak
  ("when a caller chooses to") would make it cleaner; not a blocker.
  Nit.
- **`reset_per_image_state` interaction (Q10).** Confirmed.
  `metadata_line` is computed every render from
  `current_image_native_size`, which is itself recomputed from the
  current entry. There is no per-image cached state for the footer
  to reset. No change needed.

# What I checked

- `git show d9cc0c3` — three files touched, +47/-3.
- `specs/GH9729/tech.md` §699 verbatim and t2-8-r2 follow-up wording.
- Module-top constant ordering in
  `crates/ui_components/src/lightbox.rs` (lines 12-53).
- All call sites of `lightbox::Params` (`lightbox_view.rs:333` and the
  two example sites in `examples/library.rs:565,601`) — all three
  populate `metadata_line` consistently with their data availability.
- `reset_per_image_state` in `lightbox_view.rs:133` — confirmed it
  resets only animation_start_time and zoom_factor; metadata derives
  from `current_image_native_size` per render.
- Searched `crates/ui_components/` for any test files or element-tree
  assertion harness — none exist; the deferral of unit tests is
  consistent with the crate's existing posture.

# Suggestions

- (Optional, t2-8-r2) Promote `metadata_line: Option<String>` to
  `metadata: Option<LightboxMetadata>` so the renderer owns the
  formatting and future callers don't drift. Mention in the t2-8-r2
  TIER2_TODO entry.
- (Optional, this commit follow-up or t2-8-r2) Group module-top
  constants by axis: visual constants (SCRIM_PADDING, DESCRIPTION_*,
  LIGHTBOX_TEXT_*, METADATA_TEXT_*) above logic constants
  (MIN/MAX/ZOOM_STEP).
- (Optional) `.round() as i32` for the dimension formatter — closes a
  theoretical fractional-pixel rounding inconsistency at zero cost.
- (Optional) Tighten the `metadata_line` doc comment to "when a caller
  chooses to" so it doesn't read as forward-promising format/size.
