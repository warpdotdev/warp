---
item: tier2-t2-9
commit: 3e694be
reviewer: R1-correctness
spec_ref: tech.md §700
verdict: pass-with-nits
---

# Spec

> - **EXIF orientation and ICC color profile**: extend the agent-mode decoder in `app/src/util/image.rs` and wire into `ImageType::try_from_bytes`.

# Findings

- [minor] §700 enumerates EXIF orientation AND ICC together. This commit ships EXIF only and explicitly defers ICC to a new `t2-9-icc` row in `TIER2_TODO.md`. The defer is well-justified (requires `lcms2`/`qcms` or a colour-management module), and the row is tracked, but the t2-9 line is being marked `[x]` Impl despite covering only half of §700. The `TIER2_TODO.md` edit relabels the row as "EXIF orientation (ICC deferred to t2-9-icc)" so the partial state is visible — acceptable, but flagging that the §700 bullet itself remains a partial-completion until t2-9-icc lands.
- [nit] §700 says "extend the agent-mode decoder in `app/src/util/image.rs` and wire into `ImageType::try_from_bytes`." The commit extends `app/src/util/image.rs::resize_image` (which is what `ImageType::try_from_bytes` ultimately funnels image bytes through) AND additionally extends the lightbox decoder at `crates/warpui_core/src/image_cache.rs::decode_static_with_limits_inner`. The lightbox-side extension is over-and-above what §700 explicitly mentions but is consistent with the spec's intent (the lightbox is the user-visible site where sideways photos were the original complaint). No problem; just noting the commit does slightly more than the literal §700 bullet.
- [nit] Lossy re-encode hazard: a small JPEG with `Orientation != NoTransforms` now falls through to the re-encode path (identity-resize branch in `app/src/util/image.rs`), incurring one extra JPEG round-trip. Acceptable for agent mode (which already re-encodes oversized images), and necessary for correctness — there's no way to flatten orientation without re-encoding. Worth a one-line code comment future-readers can find; the commit message mentions the path but the code does not. Not blocking.
- [nit] No new EXIF-specific tests. The commit message correctly observes that fixturing an EXIF JPEG would require a binary asset; a small hand-rolled JPEG with a single APP1 segment and an `Orientation=6` tag could be embedded as a `&[u8]` literal (~700 bytes) without adding a fixture file, which would let a test assert that `resize_image` flips width/height. Not blocking — t2-FINAL presubmit will exercise the no-EXIF paths and the existing `test_resize_image_small_image_unchanged` still passes — but a regression-guard test would be cheap insurance for the one feature the commit actually adds. Flagged for R2 / follow-up only.

# What I checked

- `git show 3e694be` and `git show --stat 3e694be` — three files: `app/src/util/image.rs` (+48/-15), `crates/warpui_core/src/image_cache.rs` (+14/-1), `specs/GH9729/TIER2_TODO.md` (+19/-4).
- Verified §700 against `specs/GH9729/tech.md:700` and quoted it verbatim above. Confirmed it lists EXIF and ICC together in the same bullet, so the deferral is a real partial.
- §234 pixel-cap interaction in `image_cache.rs:436-458`: the diff places `pixels > max_pixels` check BEFORE `img.apply_orientation(orientation)`. Confirmed Rotate90/Rotate270 transpose `(w, h)` but never change `w*h`, so the cap stays correct regardless of order. The diff comment ("can transpose width/height but cannot change the total pixel count") is accurate.
- `image = "0.25.9"` API surface, verified in `~/.cargo/registry/src/index.crates.io-*/image-0.25.9/`:
  - `ImageReader::into_decoder` exists at `src/io/image_reader_type.rs:219` and DOES forward `Limits` via both `make_decoder(..., self.limits.clone())` and a subsequent `decoder.set_limits(self.limits)?`. §234/§259 caps preserved.
  - `ImageDecoder::orientation` trait default at `src/io/decoder.rs:53` returns `Ok(NoTransforms)` when `exif_metadata()` returns `None`. JPEG (`codecs/jpeg/decoder.rs:140`), WebP (`codecs/webp/decoder.rs:93`), and TIFF (`codecs/tiff/`) override it; PNG inherits the default (which still picks up `eXIf` chunks via the generic `exif_metadata()` path).
  - `DynamicImage::from_decoder` at `src/images/dynimage.rs:243` and `DynamicImage::apply_orientation` at `src/images/dynimage.rs:1161` both exist.
- No-EXIF formats: PNG without `eXIf`, GIF, etc. take the trait default and yield `NoTransforms`, so `apply_orientation` is a no-op. Confirmed.
- Animated path: `crates/warpui_core/src/image_cache.rs:413` (`decode_animated_with_limits` → `decode_animated_with_limits_inner` at line 357) is a separate function the diff does NOT touch. The static dispatch in `image_cache.rs:556-578` routes animated WebP/GIF through that path. Animated WebP/GIF are correctly unaffected by the orientation change.
- Agent-mode fast path: `app/src/util/image.rs` early-return at the `current_pixels <= MAX_IMAGE_PIXELS && !needs_orientation_flatten` gate. Synthesised PNGs (used by `test_resize_image_small_image_unchanged`) carry no EXIF, so `orientation == NoTransforms`, so the early-return still fires and the test still returns original bytes verbatim. Confirmed.
- Tests claim "6/6 `util::image::tests`, 27/27 `image_cache::tests`". Not re-run here; t2-FINAL will re-verify.

# Suggestions

- Optional R2 follow-up: add a regression-guard test using a hand-rolled tiny EXIF JPEG (`&[u8]` literal, no fixture file). Asserting `resize_image(input).len() != input.len()` for an `Orientation=6` payload, or — stronger — round-tripping through `image::load_from_memory` and asserting transposed dimensions, would lock the wire-up against regressions.
- Optional comment in `app/src/util/image.rs::resize_image` noting that the identity-resize branch incurs one JPEG re-compression for EXIF-tagged photos. The commit message has this; the code does not.
- For the t2-9 TIER2_TODO row, consider marking the Impl column `[~]` rather than `[x]` until `t2-9-icc` lands, or split into two rows; the current "EXIF orientation (ICC deferred to t2-9-icc)" label is workable but reads as fully-done at a glance.
