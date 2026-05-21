---
item: tier2-t2-5
commit: 5a8072a
reviewer: R2-quality
spec_ref: tech.md §696
verdict: pass-with-nits
---

# Spec

Quoted verbatim from `specs/GH9729/tech.md` line 696:

> - **Adopt `LightboxImageSource::Error` at the artifacts call site** (`app/src/ai/artifacts/mod.rs:362-365`) so screenshot fetch failures use `Error` instead of the `Loading + "Failed to load"` description workaround.

(Cross-reference §182 + §586 + §206 establish the `Error { message }` variant
as the renderer-supported failure surface and explicitly identify the
artifacts site as the one in-tree caller still on the `Loading + description`
workaround.)

# Findings

- [nit] **Function name unchanged is correct.** `screenshot_lightbox_image_from_download_result` is still semantically accurate after this commit — the function still maps a download `Result` to an `Option<LightboxImage>`, only the failure-arm shape changed. No rename is warranted; renaming it (e.g. to "...maybe_error") would imply a behaviour change that did not happen at this layer. Keep as-is.

- [nit] **Test rename is reasonable and a slight upgrade.** The old name `returns_failure_placeholder_for_screenshot_load_errors` baked in the legacy "placeholder" framing (the legacy code stuffed a description into a `Loading` variant — that *was* a placeholder). The new name `surfaces_error_variant_for_screenshot_load_errors` accurately names the variant being asserted. The new name does not, however, capture the additional load-bearing claim of the test — that the message is sanitized and the raw error does not leak. A name like `surfaces_sanitized_error_variant_for_screenshot_load_errors` would describe the body more fully. Not blocking.

- [minor] **Sanitized message string is consistent in voice; placement deserves a follow-up.** The new `"could not load screenshot"` matches the lowercased "could not …" form used by the four existing strings in `sanitize_load_error` (`lightbox_view.rs:177-189`):
  - "image is too large to preview"
  - "could not detect image format"
  - "could not decode image"
  - "could not read image"
  - "could not load screenshot" (new)

  Voice and casing match. However, the catalog is now split across two modules with no shared definition: the four `sanitize_load_error` strings sit in `app/src/workspace/lightbox_view.rs` and the new fifth lives in `app/src/ai/artifacts/mod.rs`. They are cousins — every one is the user-visible payload of `LightboxImageSource::Error.message` — but only the `lightbox_view.rs` set is collected together. A future contributor adding a sixth `Error` site has no central catalog to consult and will plausibly invent a sixth voice ("Failed to load image", "Image error", "Cannot show image", etc.). Recommend a follow-up to consolidate (e.g. a `mod errors { pub const COULD_NOT_LOAD_SCREENSHOT: &str = …; pub const COULD_NOT_DECODE_IMAGE: &str = …; }` next to the `LightboxImageSource::Error` definition in `crates/ui_components/src/lightbox.rs`, or a doc-comment table on the variant itself listing the canonical phrases). Not blocking for v1.x — one-off-per-call-site is fine while there are only two call sites — but it should be tracked.

- [nit] **Comment format matches the in-tree v1 GH9729 convention.** The new inline comment `// GH9729 §696: …` at `mod.rs:361` matches the format used at `image_cache.rs:474`, `image_cache.rs:529`, `image_cache.rs:545`, `image_cache.rs:552`, `image_cache.rs:564`, `image_cache.rs:570`, and `lightbox_view.rs:180`. Multi-line block comment, leading `// GH9729 §NNN:` then prose. Good. The cross-reference to `§182` for the rendering contract is also consistent with the established style.

- [nit] **Test rigor — the substring negative is sufficient at this layer.** The new test asserts the message equals `"could not load screenshot"` exactly (`mod_tests.rs:69`) AND asserts it does not contain `"network"` (line 71) or `"connection reset"` (line 75). The exact-equality assertion is the strongest form: it already implies that no character of `format!("{e}")` leaks, because the equality holds against a hardcoded constant. The two `!message.contains(...)` checks are redundant with the `assert_eq!` (a longer message would have failed the equality first), but they add a useful documentation-style "this is the property under test" signal for readers, and they would survive if someone ever softened the equality to `starts_with` or `contains`. A third assertion of the form `!message.contains(&format!("{e}"))` is not missing — it would be strictly weaker than the existing exact-equality check. Sufficient as written.

- [nit] **No constructor/helper exists on `LightboxImage`.** Inspecting `crates/ui_components/src/lightbox.rs` (lines 47-52): `LightboxImage` is a plain struct with two public fields, no `impl` block, no `LightboxImage::new` / `::loading` / `::error` constructors. The struct-literal form `LightboxImage { source: …, description: None }` used here is the only available shape and matches every other call site in tree (`mod.rs:306`, `mod.rs:348`). Idiomatic for the current codebase. A `LightboxImage::error(message: impl Into<String>) -> Self` helper is a natural follow-up if the central catalog above gets adopted, but it is not pre-existing today and adding it here would be out-of-scope for t2-5. No action.

- [nit] **No leftover `Loading`-as-failure references.** Confirmed by grep on `app/src/ai/artifacts/mod.rs`: the only remaining `LightboxImageSource::Loading` use is at `mod.rs:307` inside `open_screenshot_lightbox`, where it is the legitimate transient-state placeholder used while the download is in flight (the `description: None` shape there is also clean — no "Failed to load" or other failure-flavored string). Pre-resolution `Loading` and post-failure `Error` are now correctly disjoint; `Loading` is no longer a failure sentinel anywhere in this module. Clean.

- [nit] **Cross-module string-matching consumer: confirmed none.** Unlike `t2-4` where `"could not detect image format"` is *produced* in `image_cache.rs:574` and *substring-matched* in `lightbox_view.rs:179`, the new `"could not load screenshot"` is only consumed by `Lightbox::render` at `crates/ui_components/src/lightbox.rs:179` (the `(Some(LightboxImageSource::Error { message }), _) =>` arm), which prints `message` verbatim into the inline error panel. Grep confirms no `contains("could not load screenshot")`, no `== "could not load screenshot"`, and no telemetry/error-categorization site reads the message field. The string is rendered, not parsed. Safe. If a future telemetry feature wants to bucket `Error` variants by reason (e.g. `lightbox_error_reason: "screenshot_load_failed" | "image_too_large" | …`), it should be driven by a typed enum carried alongside `message`, not by string-matching the message; that would be a v1.x follow-up that subsumes the catalog suggestion above.

# What I checked

- `git show --stat 5a8072a` and `git show 5a8072a` — three files, +36/-9, scoped to the artifacts module + spec status.
- `specs/GH9729/tech.md` line 696 (verbatim quote above) — the v1 follow-up bullet is the authoritative spec.
- `specs/GH9729/tech.md` lines 206 and 586 — corroborate that the artifacts site was always the documented future adopter of `Error`.
- `app/src/ai/artifacts/mod.rs` lines 358-377 — the new `Error` arm; the comment block at lines 361-367; `description: None`.
- `app/src/ai/artifacts/mod.rs` lines 299-310 — confirms the surviving `LightboxImageSource::Loading` is the legitimate transient placeholder, not a failure sentinel.
- `app/src/ai/artifacts/mod_tests.rs` lines 53-82 — the renamed and rewritten test.
- `app/src/workspace/lightbox_view.rs` lines 175-190 — `sanitize_load_error` and the four existing categorical strings, for voice/casing comparison.
- `app/src/workspace/lightbox_view.rs` line 180 — the in-tree v1 `// GH9729 §NNN:` comment-format convention; matches the new comment in `mod.rs:361`.
- `crates/ui_components/src/lightbox.rs` lines 26-52 — `LightboxImageSource::Error { message }` and the `LightboxImage` struct definition; confirms no `impl LightboxImage` constructor exists.
- `crates/ui_components/src/lightbox.rs` line 179 — the only consumer of `Error.message`, renders verbatim.
- `grep -n "could not load screenshot"` across the workspace — exactly two hits: the producer at `mod.rs:371` and the test at `mod_tests.rs:69`. No string-matching consumer.
- `specs/GH9729/TIER2_TODO.md` — both the bullet and the matrix row are flipped to `[x]` for impl. Status accurate.

# Suggestions

1. **Tighten the test name** to `surfaces_sanitized_error_variant_for_screenshot_load_errors` so the identifier captures both load-bearing claims (variant + sanitization).

2. **Track the catalog consolidation** as a Tier 3 follow-up: collect `"could not load screenshot"` + the four `sanitize_load_error` strings into a single `mod errors` (or doc-comment table on `LightboxImageSource::Error`) so a sixth call site has an obvious place to look. Pairs naturally with introducing a typed `LightboxErrorReason` enum if/when telemetry wants to bucket failures programmatically.

3. **Optional `LightboxImage::error(impl Into<String>) -> Self` helper** alongside (2). Would eliminate the four-line struct-literal at every error site and centralize the `description: None` invariant. Out of scope for t2-5; only worth doing if a third `Error` site appears.
