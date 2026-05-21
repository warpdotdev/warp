---
item: tier2-t2-4
commit: 7780d31
reviewer: R2-quality
spec_ref: tech.md §695
verdict: pass-with-nits
---

# Spec

Quoted verbatim from `specs/GH9729/tech.md` line 695:

> - **Convert `ImageType::Unrecognized` to `Err` globally** with an audit of every `try_from_bytes` caller. Closes the mislabeled-file rough edge for surfaces other than the Lightbox file-tree path (which v1 already closes locally).

# Findings

- [nit] The `anyhow!("could not detect image format")` in `image_cache.rs:574` is a stringly-typed sentinel that another module (`lightbox_view.rs::sanitize_load_error`, line 179) classifies by substring-matching `"could not detect"`. Both call-sites carry comments naming the cross-module contract (the producer comment at `image_cache.rs:570-573` and the consumer comment at `lightbox_view.rs:180-183`), and the test at lines 339-359 exercises the exact wording end-to-end, so the coupling is documented and guarded. For a v1.x follow-up this is acceptable; longer-term a typed error in `warpui_core` (e.g. `pub enum ImageDecodeError { Unrecognized, …}` re-exported and then matched on by consumers via downcast) would be more idiomatic Rust and remove the substring contract entirely. Worth a TIER3 follow-up note but not blocking.

- [nit] The test name `post_load_callback_rewrites_unrecognized_to_error` (line 340) preserves the pre-refactor concept ("Unrecognized") in its identifier even though the variant no longer exists and the test now drives `FailedToLoad`. The test body's comment explains the rename context, but a future reader grepping for `Unrecognized` in test names will be momentarily confused. Suggest `post_load_callback_rewrites_unrecognized_format_err_to_error` or `…rewrites_could_not_detect_to_detect_error` to keep the "what we're guarding" semantics without naming a removed variant.

- [minor] **Branch-ordering test gap.** `sanitize_load_error_picks_decode_category` (line 371) confirms `"png decode error: invalid IHDR chunk"` maps to `"could not decode image"` — this still holds and is meaningful. However there is **no negative test** that asserts `"could not detect image format"` does NOT collapse into the `decode/format` bucket. The whole correctness of the refactor rests on the new `else if s.contains("could not detect")` branch sitting BEFORE the `s.contains("decode") || s.contains("format")` branch. If a future contributor reorders the branches (e.g. tidies the chain alphabetically), `post_load_callback_rewrites_unrecognized_to_error` would catch it because the asserted string would then be `"could not decode image"` instead of `"could not detect image format"` — but that's an end-to-end test going through `rewrite_image_for_load_state`, not a direct unit test of `sanitize_load_error`. Recommend adding `sanitize_load_error_picks_unrecognized_before_decode` that calls `sanitize_load_error(&anyhow!("could not detect image format"))` directly, parallel in shape to the existing three `sanitize_load_error_picks_*` tests. This keeps each branch covered by a dedicated unit test.

- [minor] **Brittleness of the producer wording.** If a future contributor edits the message in `image_cache.rs:574` to `"Could not detect image format."` (capital C, trailing period) or `"unrecognised image format"` (British spelling, common drift), the test `post_load_callback_rewrites_unrecognized_to_error` would *not* fail directly — that test constructs its own `anyhow!("could not detect image format")` in test code rather than calling `ImageType::try_from_bytes`. So the producer-side string drift is silent until someone manually loads a mislabeled file. There is no end-to-end test in this commit asserting that `ImageType::try_from_bytes(<garbage>)` produces an error whose `to_string()` contains `"could not detect"`. Recommend adding a one-line test in `image_cache_tests.rs` of the form: `assert!(ImageType::try_from_bytes(b"not an image").unwrap_err().to_string().to_lowercase().contains("could not detect"))`. This is the missing guard for the cross-module string contract — without it, the substring coupling really is undefended.

- [nit] The `// TODO: other types` comment on the enum was correctly updated to `// TODO: other types (HEIC/HEIF/AVIF/BMP/TIFF/ICO — see tech.md §702).` (line 678). Accurate, no longer implies `Unrecognized` is the path for new formats. Good fix.

- [nit] The doc comment added to `pub enum ImageType` ("A successfully decoded image. Per GH9729 §695, …") is clear and accurate. The updated `start_asset_load` doc (lines 136-144) and `rewrite_image_for_load_state` doc (lines 192-202) correctly explain why the `Unrecognized` arm is gone. Good.

- [nit] Convention check: other v1 GH9729 inline comments use the form `// GH9729 §NNN:` (e.g. `image_cache.rs:474, 529, 545, 552, 564`). This commit uses `// GH9729 §695:` consistently in both `image_cache.rs:570` and `lightbox_view.rs:180`. Doc-comment refs use `tech.md §695` (without `GH9729` prefix), which matches the prior `tech.md §182` convention in the same file. Consistent.

- [nit] `visual_section_max_width` in `view_impl/common.rs` correctly drops its `Unrecognized` arm. The match was already exhaustive over the remaining variants, so the fix is mechanical and safe. No surprises.

- [nit] Dead code check: no remaining live references to `ImageType::Unrecognized` exist after this commit; the only hits in the tree are in comments/strings inside `image_cache.rs` and `lightbox_view.rs` documenting the refactor, plus unrelated `Unrecognized*` symbols in `warp_completer`, `windowing/system.rs`, etc. Clean.

# What I checked

- §695 of `tech.md` (line 695, the Follow-ups bullet) — quoted verbatim above.
- Full diff of commit `7780d31` via `git show`.
- `lightbox_view.rs:130-200` (updated doc comments + `sanitize_load_error` branch ordering + `rewrite_image_for_load_state`).
- `lightbox_view.rs:339-381` (updated and pre-existing tests).
- `image_cache.rs:560-589` (new `Err` arm in `try_from_bytes` + `size_in_bytes` cleanup).
- `image_cache.rs:668-680` (new doc comment on `ImageType` + updated `// TODO` comment).
- `image_cache.rs:683-700` (cleaned-up `image_size` and `type_str` matches).
- `view_impl/common.rs:2283` (dropped `Unrecognized` arm).
- `app/src/terminal/model/kitty.rs:920-924` (consumer routes `Err` → `KittyPngError::InvalidBytes`).
- `app/src/terminal/model/terminal_model.rs:3326` (consumer uses `let Ok(image_type) = ... else { return; }`).
- Whole-tree grep for `ImageType::Unrecognized` and lowercase `Unrecognized` to verify no residual live references.
- `// GH9729 §NNN:` code-comment convention by greppping all `*.rs` for `GH9729 §`.

# Suggestions

1. Add `sanitize_load_error_picks_unrecognized_before_decode` as a direct unit test of the branch order.
2. Add a producer-side guard test in `image_cache_tests.rs` asserting `try_from_bytes(b"not an image")` produces an error containing `"could not detect"` (lower-cased). This is the missing end-to-end defense for the cross-module substring contract.
3. Rename `post_load_callback_rewrites_unrecognized_to_error` to something like `…rewrites_could_not_detect_to_detect_error` to remove the dead `Unrecognized` reference from the test identifier.
4. (TIER3 follow-up) Consider replacing the `anyhow!("could not detect image format")` sentinel with a typed error variant exported from `warpui_core` (e.g. `ImageDecodeError::UnrecognizedFormat`) and have `sanitize_load_error` downcast on it. Would remove the substring coupling entirely and is the v1.x-clean version of this v1.x-acceptable shortcut.
