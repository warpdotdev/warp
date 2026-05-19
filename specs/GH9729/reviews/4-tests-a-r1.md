---
item: 4-tests-a
commit: abab042
reviewer: R1-correctness
spec_ref: tech.md §613:640-652
verdict: pass-with-nits
---

# Findings

## Spec fidelity — name-by-name match against §613:640-652

All 9 test names that fall in this item's scope (lines 640-642 + 647-652) are present and named verbatim:

| Spec line | Prescribed name | Implemented |
| --- | --- | --- |
| 640 | `decode_static_rejects_dimensions_over_cap` | yes |
| 641 | `decode_static_rejects_pixels_over_cap` | yes |
| 642 | `decode_static_accepts_normal_photo` | yes |
| 647 | `decode_svg_rejects_intrinsic_dimensions_over_cap` | yes |
| 648 | `decode_svg_accepts_normal_icon` | yes |
| 649 | `decode_svg_rejects_non_xml_prefix` | yes |
| 650 | `decode_svg_accepts_xml_prelude_with_bom_and_whitespace` | yes |
| 651 | `decode_svg_accepts_doctype_prelude` | yes |
| 652 | `decode_svg_accepts_xml_comment_prelude` | yes |

Lines 643-646 (animated) are explicitly carved out into 4-tests-b in the commit message, which is consistent with the IMPLEMENTATION_TODO update in the same diff. No animated names are missing in scope for this item.

## Refactor justification (`decode_static_with_limits_inner`)

§613 does not prescribe the inner-helper pattern for `decode_static_with_limits` (it does not mention `decode_static_with_limits_inner`). However, the analogous animated helper `decode_animated_with_limits_inner` already exists in `image_cache.rs` (lines 357-403, pre-commit), so the refactor follows an established in-codebase convention rather than introducing a new pattern. The wrapper is a one-liner that threads the production constants:

```
fn decode_static_with_limits(data, format) -> ... {
    decode_static_with_limits_inner(data, format, decode_limits(), MAX_DECODE_PIXELS)
}
```

This is a minimal and necessary change for the spec-prescribed dimension-cap and pixel-cap rejection tests, since the production caps are 8192 and 67M and synthesizing fixtures at those sizes inside a unit test would be wasteful. The refactor is in scope and proportionate.

## Behavior preservation

The pre-commit `decode_static_with_limits` body and the new `decode_static_with_limits_inner` body are byte-identical apart from `decode_limits()` / `MAX_DECODE_PIXELS` becoming parameters. The wrapper passes exactly those two values. Production behavior is preserved.

## Test correctness

- `decode_static_rejects_dimensions_over_cap`: 200x100 PNG fed with `max_image_width = max_image_height = 100`. Asserts `result.is_err()`. The assertion verifies rejection but not the cause; an arbitrary decoder error would also pass. Acceptable for the gate-firing intent the spec asks for, but a `to_string().contains(...)` or a downcast on the dimension error would harden it.
- `decode_static_rejects_pixels_over_cap`: 200x100 PNG (20_000 px) with per-axis caps of 1_000 and `max_pixels = 10_000`. The dimension caps cannot fire (200 < 1_000, 100 < 1_000), so rejection must come from the post-decode pixel-count branch (`anyhow::bail!("image is too large to preview")`). This is a tight design — only the targeted code path can produce `Err` here. Asserting `is_err()` is sufficient in practice, though again a substring assertion on the bail message would be stronger.
- `decode_static_accepts_normal_photo`: 200x100 PNG through the production wrapper. `dimensions() == (200, 100)` confirms successful round-trip. Correct.
- `decode_svg_rejects_non_xml_prefix`: predicate-only — `assert!(!looks_like_svg_xml(&[0u8;1024]))`. The spec text on line 649 says "is fed through the SVG path returns `Err` from the content-sanity check before `usvg::Tree::from_data` is invoked. Asserts the prefix check fires." The implementation asserts the predicate directly, not the end-to-end behavior. This is a minor deviation — the test verifies the gate in isolation but does not prove the gate is the active filter inside `try_from_bytes`. In practice, `try_from_bytes` checks `looks_like_svg_xml` first and falls through to `image::guess_format` for non-XML, which would also fail on a NUL buffer, so the spec's "returns Err" assertion would hold but not prove the SVG-path-specific gate fired. Acceptable as a unit test of the predicate; spec-literal would gate end-to-end.
- `decode_svg_accepts_xml_prelude_with_bom_and_whitespace`, `decode_svg_accepts_doctype_prelude`, `decode_svg_accepts_xml_comment_prelude`: all three are predicate-only. The spec text says "passes the content-sanity check," which the predicate-only form does verify. These are spec-faithful.
- `decode_svg_accepts_normal_icon`: end-to-end `try_from_bytes`, asserts `Svg` variant. Correct.
- `decode_svg_rejects_intrinsic_dimensions_over_cap`: end-to-end. 200000x200000 SVG payload runs through `try_from_bytes`, which calls `usvg::Tree::from_data` (succeeds — small XML), then bails on the intrinsic-dimension check (`w > MAX_SVG_RENDER_DIMENSION`). Asserts `is_err()`. The test would also pass if `usvg` rejected the dimensions itself (it does not at present, but a future `usvg` upgrade could mask the cap). A substring assertion on `"svg dimensions exceed render budget"` would harden this.

## Coverage gaps

- **JPEG / WebP-static:** §613:640-642 names PNG specifically in the dimension/pixel cases (lines 640-641 say "synthesized PNG header") and "PNG decodes successfully" on 642. The spec does not prescribe JPEG or WebP-static decode tests at this layer. No gap relative to spec, though the function header on `decode_static_with_limits` explicitly lists "PNG, JPEG, WebP-static" as supported. A grep across the test tree confirms no JPEG/WebP-static decoder tests exist elsewhere — they are simply not prescribed.
- **Mixed-case predicate (`<SVG>`):** the predicate is intentionally case-sensitive (lines 320-323 in `image_cache.rs` document this), and the spec does not prescribe a rejection test for the uppercase form. Not a spec gap, but an implementation choice that a regression test would document.
- **UTF-16 BOM:** predicate strips only UTF-8 BOM; not prescribed by §613, no gap.
- **Empty-input predicate:** not prescribed; no gap.
- **`<image href>` exfiltration (4c R1 secondary finding):** out of scope for §613. Not a gap for this item.

## Determinism, isolation, side effects

- `encode_blank_png(width, height)` produces a deterministic byte sequence (image crate's PNG encoder is deterministic given fixed pixel content, and the fixture is all-zero RGBA). Tests are reproducible.
- Each test creates its own buffer; no shared mutable state.
- No filesystem, network, or subprocess use. Pure in-memory.

# What I checked

- `git show abab042` (full diff and message).
- `specs/GH9729/tech.md` lines 630-665 (one read, covers the prescribed range plus surrounding context for the asset-cache and SVG block).
- `crates/warpui_core/src/image_cache_tests.rs` lines 565-680 (the new test block).
- `crates/warpui_core/src/image_cache.rs` around lines 295-490 to confirm the wrapper/inner refactor preserves behavior, the `looks_like_svg_xml` predicate, and the SVG intrinsic-dimension cap path inside `try_from_bytes`.
- That the inner-helper pattern matches the existing `decode_animated_with_limits_inner` convention (lines 357-403, pre-commit).
- That no JPEG / WebP-static decoder tests are silently expected elsewhere in the repo (none found; spec does not prescribe them at this layer).

# Suggestions

1. (Nit, non-blocking.) Tighten error-cause assertions on the three negative tests by asserting the bail message:
   - `decode_static_rejects_pixels_over_cap` — assert the err string contains `"image is too large to preview"`.
   - `decode_svg_rejects_intrinsic_dimensions_over_cap` — assert the err string contains `"svg dimensions exceed render budget"`.
   This guards against a future `usvg` or `image` crate upgrade silently changing which branch produces the error and masking a regression in the cap.
2. (Nit, non-blocking.) Consider reading `decode_svg_rejects_non_xml_prefix` literally per spec line 649 by routing the NUL buffer through `ImageType::try_from_bytes` and asserting `is_err()` on top of the existing predicate-only assertion. Two-line addition; gives end-to-end coverage that the SVG path is the one rejecting.
3. (Nit, non-blocking.) The `image_cache_tests.rs` module accesses `super::decode_static_with_limits_inner`, `super::decode_static_with_limits`, `super::looks_like_svg_xml`, `super::ImageType`, and `super::MAX_DECODE_PIXELS`. The inner helper is `pub(crate)` only by virtue of being a private fn in the same crate visible to a child module — fine for now. If `image_cache_tests.rs` ever moves out of the same module tree (e.g. into a `tests/` integration target), these would need explicit `pub(crate)` annotations. Worth a one-line comment on `decode_static_with_limits_inner` noting it is intentionally private/test-visible.
