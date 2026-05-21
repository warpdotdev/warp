---
item: tier2-t2-4
commit: 7780d31
reviewer: R1-correctness
spec_ref: tech.md §695
verdict: pass
---

# Spec

> **Convert `ImageType::Unrecognized` to `Err` globally** with an audit
> of every `try_from_bytes` caller. Closes the mislabeled-file rough
> edge for surfaces other than the Lightbox file-tree path (which v1
> already closes locally).

# Findings

- no issues found

# What I checked

- **Spec fidelity (§695).** The diff removes the `ImageType::Unrecognized`
  variant from `crates/warpui_core/src/image_cache.rs` and replaces the
  catch-all `_ => Ok(ImageType::Unrecognized)` in `try_from_bytes`
  (line 574) with `_ => Err(anyhow!("could not detect image format"))`.
  Scope matches the bullet — variant removal + caller audit, nothing more.

- **Caller audit — every `ImageType::try_from_bytes` site routes `Err`.**
  - `crates/warpui_core/src/assets/asset_cache.rs:304` (Bundled): result
    propagated via `.and_then(...)`; `Err(err) => AssetStateInternal::Error(Rc::new(err))`
    at line 316.
  - `crates/warpui_core/src/assets/asset_cache.rs:354` (Raw insert):
    explicit `Err(err)` arm at line 368 logs and stores `AssetStateInternal::Error`.
  - `crates/warpui_core/src/assets/asset_cache.rs:457` (async fetch):
    explicit `Err(err)` arm at line 473 logs and stores
    `AssetStateInternal::Error`.
  - `app/src/terminal/model/terminal_model.rs:3326`: `let Ok(image_type)
    = ... else { return; };` — no behavior change since the previous
    `Ok(Unrecognized)` would have flowed into `image_size() -> None` and
    bailed at line 3329 anyway. The `Err` path now exits one statement
    earlier with the same outcome.
  - `app/src/terminal/model/kitty.rs:921-923`: explicit `Err(err) =>
    return Err(KittyPngError::InvalidBytes(err.to_string()))`.

- **`AssetState::Error -> AssetState::FailedToLoad` mapping.**
  `assets/asset_cache.rs:153` maps `AssetStateInternal::Error(err)` to
  `AssetState::FailedToLoad(err.clone())`, so the error string from
  `try_from_bytes` is preserved verbatim and reaches
  `sanitize_load_error` in the lightbox path.

- **Branch ordering in `sanitize_load_error`.** The "could not detect"
  branch (line 179) is placed BEFORE the generic "decode/format" branch
  (line 185). This ordering is load-bearing because the
  `try_from_bytes` error string `"could not detect image format"` also
  contains the substring `"format"`, which would otherwise match the
  later branch and collapse the user message into `"could not decode
  image"`. The new test `post_load_callback_rewrites_unrecognized_to_error`
  exercises exactly this ordering and asserts the specific
  `"could not detect image format"` output.

- **§182 error-string DoS / leakage posture.** The new branch returns a
  fixed `&'static str` (`"could not detect image format"`). It does not
  interpolate `err`, the underlying `anyhow::Error`, or any path. The
  raw error is logged via `log::warn!` in `rewrite_image_for_load_state`
  (line 208) for the operator only. This matches the §182 rule already
  applied to the other categorical branches.

- **Pattern-match exhaustiveness.** Five exhaustive matches on
  `ImageType` had `Unrecognized` arms; all five had the arm removed and
  none replaced it with `_`:
  - `image_cache.rs:580-583` `size_in_bytes` — 3-arm exhaustive match.
  - `image_cache.rs:684-694` `image_size` — 3-arm exhaustive match.
  - `image_cache.rs:697-699` `type_str` — 3-arm exhaustive match.
  - `image_cache.rs:891-928` `to_image` — 3-arm exhaustive match (note
    the `_` at line 920 is `resize_image(&first_frame.img, ...)` — a
    binding name, not a wildcard arm).
  - `app/src/ai/blocklist/block/view_impl/common.rs:2280-2283`
    `visual_section_max_width` — 3-arm exhaustive match.
  Future variants will fail the exhaustiveness check at compile time —
  good.

- **Test coverage.**
  - `post_load_callback_rewrites_unrecognized_to_error` is rewritten to
    drive `FailedToLoad(anyhow!("could not detect image format"))` and
    still asserts the same user-visible message. Implicitly verifies
    branch ordering in `sanitize_load_error`.
  - No remaining test in the codebase constructs `ImageType::Unrecognized`
    (verified via project-wide grep; the only `Unrecognized` hits are
    unrelated types: `UnrecognizedDisplayHandle`, CLI flag actions, MCP
    autoinstall, etc.).
  - `image_cache_tests.rs` cases that exercise bad bytes
    (`decode_static_rejects_*`, `decode_animated_rejects_*`,
    `decode_svg_rejects_intrinsic_dimensions_over_cap`) already returned
    `Err` from their respective decoder helpers; none asserted
    `Ok(Unrecognized)` and none need adjustment.

- **Async cancellation.** N/A. No new spawns or awaits — the diff is a
  variant-to-error refactor and one new sync branch in
  `sanitize_load_error`.

- **Security.** Decode-bomb / SVG-XXE caps from v1 are unchanged. The
  SVG intrinsic-dimension and pixel caps at `image_cache.rs:480-486`,
  the per-format `decode_static_with_limits` calls at `image_cache.rs:530,
  537, 557`, and the animated frame/pixel cap via
  `decode_animated_with_limits` at `image_cache.rs:547, 565` all sit
  upstream of the new `_ => Err(...)` arm and are reached on the same
  successful-`guess_format` paths as before.

# Suggestions

- None.
