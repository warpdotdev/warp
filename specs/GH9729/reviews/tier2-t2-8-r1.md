---
item: tier2-t2-8
commit: d9cc0c3
reviewer: R1-correctness
spec_ref: tech.md §699
verdict: pass-with-nits
---

# Spec

> - **Status footer** (filename, dimensions, file size, format string): extend `lightbox::Params` with an optional metadata strip rendered below the image.

(`specs/GH9729/tech.md` line 699, in the v1.x follow-up section.)

# Findings

- **Spec fidelity — partial coverage with documented deferrals (acceptable).** §699
  enumerates four data points: filename, dimensions, file size, format string.
  The diff ships dimensions only and pushes the rest to `t2-8-r2`. The deferral
  rationales hold up:
  - **Filename** is already surfaced via `LightboxImage::description`
    (set to the file basename in `app/src/workspace/view.rs:23899/23905`),
    so the strip would only duplicate it.
  - **Format string** — confirmed `crates/warpui_core/src/image_cache.rs:673-678`
    declares `enum ImageType { Svg, StaticBitmap, AnimatedBitmap }` with no
    PNG-vs-JPEG-vs-WebP carrier; the original codec is genuinely lost after decode.
    Sniffing from the `AssetSource::LocalFile { path }` extension would be the cheapest
    follow-up but is non-trivial (the extension can lie — see §403's content-keying
    discussion) and out of scope here.
  - **File size** — confirmed `tech.md` §694 explicitly defers extending the
    foreground `std::fs::metadata` posture to v1.x. Adding a synchronous stat
    (or even a fresh async one) here without the background-executor migration
    would cut against that constraint, so deferring is the right call.

- **Render gating — `central_content` short-circuit when `description` is `None`
  drops the metadata strip too (nit, behavioural).** The added block sits inside
  `if let (Some(description), Some(_)) = (current_description, params.current_image_native_size)`
  in `crates/ui_components/src/lightbox.rs:293`. When the image is fully loaded
  but `description` is `None`, the entire wrapper column is skipped and
  `central_content` is rendered directly, which means **the metadata strip is
  hidden even though the gating condition for it (native size known) is met.**

  This is reachable in production via the artifacts path: in
  `app/src/ai/artifacts/mod.rs:348-355`, a Resolved screenshot whose
  `data.description` is empty produces `description: None`; once dimensions are
  known, the user sees the image but no footer. From the spec's perspective the
  strip is independent of the description, so I'd argue it should still render.
  Two cheap fixes: (a) split the condition so the column wrapper is built
  whenever **either** description **or** metadata_line is `Some` (with native
  size known), or (b) anchor on `params.current_image_native_size.is_some()`
  alone and append children conditionally. The `LightboxView` callers in
  `app/src/workspace/view.rs` always pass `Some(filename)` so this doesn't
  affect the file-tree entry point — but the artifacts path does. Recommend
  decoupling in `t2-8-r2` rather than reshipping; flagging as a nit, not a block.

- **Render gating — load/error coverage is correct.** Verified:
  - Loading (no native size): `if let (.., Some(_))` is false ⇒ falls through
    to `central_content` only ⇒ no footer next to the spinner.
  - Error variant: the `(Some(LightboxImageSource::Error { .. }), _)` match arm
    at `lightbox.rs:261-278` builds and `finish()`-es its own column inside
    `central_content`; control then flows to the description-gating block, but
    `current_image_native_size` is forced to `None` for Error in
    `app/src/workspace/lightbox_view.rs:322`, so the gate skips and the error
    panel survives unmodified. No regression.
  - No images: the `_ if image_count == 0` arm hits before description gating
    and the unchanged early structure carries through. No regression.

- **Dimensions formatting — exact in practice.** `current_image_native_size`
  is built from `ImageType::image_size()` (which returns `Vector2I`) by
  `Vector2F::new(size.x() as f32, size.y() as f32)` at
  `app/src/workspace/lightbox_view.rs:315`, and the SVG arm of `image_size`
  already `.round()`s to integer at `image_cache.rs:684-686`. So the
  `size.x() as i32` cast in the format expression sees only integer-valued
  f32s and the truncation is exact for every variant. Acceptable.

- **Unicode `×` character.** The format string uses U+00D7 MULTIPLICATION
  SIGN. `appearance.ui_font_family()` is the workspace UI font family
  (`crates/warp_core/src/ui/appearance.rs:301`), which already renders Latin-1
  Supplement glyphs throughout the existing UI text (e.g., copyright glyphs
  in legal text). No glyph-fallback concern surfaced.

- **Color and size of the footer.**
  - `METADATA_TEXT_ALPHA = 178` math: `255 * 0.7 = 178.5`, truncated to 178.
    Matches the literal and the comment.
  - `METADATA_TEXT_SIZE_REDUCTION = 2.0` with `.max(8.0)` floor. Default
    `ui_font_size = 12` (`crates/warp_core/src/ui/appearance.rs:12`) plus
    `LIGHTBOX_TEXT_SIZE_DELTA = 4` gives `lightbox_text_size = 16`, so
    metadata renders at 14. Floor at 8 only matters for hypothetical
    sub-default appearances; reasonable.

- **Examples — both `lightbox::Params` literals updated.** `library.rs:565`
  and `library.rs:601` both pass `metadata_line: None`. Verified the diff
  touches both.

- **String safety.** `LightboxView` is the only v1 caller producing a
  `Some(...)` value, and the string is `format!("{} × {} px", i32, i32)` —
  there is no path for filename or attacker-controlled bytes to reach the
  rendered glyphs. The doc comment on the field explicitly puts caller
  responsibility for sanitization/localization on record, so future callers
  are warned. Good.

# What I checked

- `git show d9cc0c3` and `git show --stat d9cc0c3` (4 files, +65/-6).
- `specs/GH9729/tech.md` §699, §694 (foreground stat constraint), §698
  (status-footer follow-up vs. zoom).
- `crates/warpui_core/src/image_cache.rs` `enum ImageType` and `image_size()`.
- `crates/ui_components/src/lightbox.rs` description-gating block, Error arm,
  loading-element pathway, and the new constants/helpers.
- `app/src/workspace/lightbox_view.rs` `current_image_native_size` derivation
  and the new `metadata_line` mapping; verified Error paths force `None`.
- `app/src/ai/artifacts/mod.rs` LightboxImage construction sites (the path
  most likely to produce `description: None` with native size known).
- `crates/warp_core/src/ui/appearance.rs` `DEFAULT_UI_FONT_SIZE = 12.0` and
  `ui_font_family()`.
- `crates/ui_components/examples/library.rs` to confirm both `Params`
  literals were updated.
- `specs/GH9729/TIER2_TODO.md` row + checklist update.

# Suggestions

- In `t2-8-r2`, consider decoupling the metadata strip's render condition
  from the description's `Some` so an artifact image with no description but
  a known native size still gets the footer. Either rebuild the column when
  `(metadata_line.is_some() || description.is_some()) && native_size.is_some()`,
  or refactor to a single column whose children are pushed conditionally with
  the spinner only short-circuiting when `native_size.is_none()`.
- When format-string sniffing lands in `t2-8-r2`, prefer reading from the
  `AssetSource::LocalFile { path }` extension *before* falling back to
  `ImageType` discrimination — the extension is the only place the original
  codec survives without a fresh content peek. Note that `AssetSource::Bundled`
  and `url_source` paths can also carry a recognisable extension, so the
  same helper covers all three Resolved sources.

# Summary

Verdict: **pass-with-nits.** The diff cleanly extends `lightbox::Params` with
an `Option<String> metadata_line`, populates it in `LightboxView` from
`current_image_native_size` (which the renderer already needs), and stays inside
the load gate so the strip never appears next to a spinner. Spec fidelity is
explicitly partial: the §699 quartet ships as dimensions-only with the
filename living in `description`, and format-string and file-size deferrals
are accurately backed by `ImageType` losing the codec post-decode and by §694's
foreground-stat constraint. The Error and no-images arms are unaffected. One
behavioural nit worth chasing in `t2-8-r2`: when `description` is `None` but
the image is loaded, the metadata strip is also dropped because both gate on
the same `if let` — reachable today via `app/src/ai/artifacts/mod.rs` when an
artifact has no description text. Constants, color math, and the `×` glyph
all check out.
