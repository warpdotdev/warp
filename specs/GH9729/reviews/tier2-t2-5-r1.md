---
item: tier2-t2-5
commit: 5a8072a
reviewer: R1-correctness
spec_ref: tech.md §696
verdict: pass
---

# Spec

Verbatim quote of `specs/GH9729/tech.md` line 696:

> - **Adopt `LightboxImageSource::Error` at the artifacts call site** (`app/src/ai/artifacts/mod.rs:362-365`) so screenshot fetch failures use `Error` instead of the `Loading + "Failed to load"` description workaround.

Cross-reference, §182 (lines 188 and 199-200 of `tech.md`) — the `Error`
variant contract:

> `Error { message: String },`
> ...
> `AssetState::FailedToLoad(err)` → rewrite to `LightboxImageSource::Error { message: sanitize(err) }` where `sanitize` collapses to a small set of categorical strings (`"could not read image"`, `"could not decode image"`, `"image is too large to preview"`) and never interpolates the raw error or any path. The original error is logged with `log::warn!` for the operator.

# Findings

- [pass] The diff at `app/src/ai/artifacts/mod.rs:360-375` does exactly
  what §696 asks: replaces the `Loading + Some("Failed to load")` shape
  with `LightboxImageSource::Error { message: "could not load
  screenshot".to_string() }` and `description: None`. Spec fidelity is
  exact.
- [pass] §182 contract — sanitized categorical message: the new message
  string `"could not load screenshot"` is a fixed, lowercase, categorical
  human-readable string with no interpolation. It does not exactly match
  any of the three example strings enumerated in §182 line 200 (those
  three are the *load/decode-pipeline* sanitization buckets handled by
  `sanitize_load_error` in `lightbox_view.rs`); the artifacts call site
  is a *fetch* failure (signed-URL request via the AI client), which is
  a different failure surface, so introducing a new categorical
  bucket for it is faithful to §182's intent rather than a violation.
- [pass] §182 contract — no OS errors / paths interpolated. `message` is
  a hard-coded string literal; the test at `mod_tests.rs:67-80` actively
  asserts that "network" and "connection reset" do not appear in
  `message`.
- [pass] §182 contract — original error logged via `log::warn!`. The
  pre-existing `log::warn!("Failed to load screenshot artifact {index}:
  {e}")` at `mod.rs:368` is preserved, exactly as required.
- [pass] `description: None` is renderer-safe. Verified at
  `crates/ui_components/src/lightbox.rs:179-196`: the `Error` arm uses
  `if let Some(description) = current_description.clone()` and only
  emits the filename row when present. With `None`, the panel renders
  just the `message` row. The §207-211 description-below-image branch
  is also gated on `current_image_native_size.is_some()`, which is
  always `None` for an `Error` entry (no native size), so it is also
  not entered. No renderer panic, no missing column, no layout regression.
- [pass] No other call site reads this `LightboxImage`'s description on
  the error path. The single non-test caller of
  `screenshot_lightbox_image_from_download_result` is at `mod.rs:329-335`,
  which forwards the value into `WorkspaceAction::UpdateLightboxImage`;
  the handler at `view.rs:21781-21788` calls
  `LightboxView::update_image_at` (`lightbox_view.rs:93-110`), which
  only branches on `source` (`Resolved` → start asset load; otherwise
  do nothing) and never inspects `description`. Safe.
- [pass] Sibling failure paths in the artifacts module: ripgrep across
  `app/src/ai/artifacts/` for `"Failed to load"` and for the
  `Loading + description` placeholder pattern returns only the now-
  removed shape and its replacement comments. No other site in this
  module silently builds a placeholder `LightboxImage` on a load
  failure. Nothing missed.
- [pass] Test coverage assertions:
  - The renamed test `surfaces_error_variant_for_screenshot_load_errors`
    matches on `LightboxImageSource::Error { message }` and panics on
    any other variant (`mod_tests.rs:67-80`). Variant assertion ✓.
  - It asserts `message == "could not load screenshot"` exactly
    (`mod_tests.rs:69`). Sanitized-message assertion ✓.
  - It seeds the input error with `"network error: connection reset by
    peer"` and asserts neither `"network"` nor `"connection reset"`
    appears in `message` (`mod_tests.rs:60-77`). Raw-error-leak
    assertion ✓.
  - It asserts `image.description.is_none()` (`mod_tests.rs:81`).
    Description-None assertion ✓.
- [pass] No surviving legacy assertions. `grep -rn 'Failed to load' app
  crates --include='*.rs'` finds no test still asserting `"Failed to
  load"` for this code path; the only remaining hits are explanatory
  comments in `mod.rs:363` and `mod_tests.rs:58` plus an unrelated
  GitHub-auth string in `update_environment_form_tests.rs`.
- [pass] Async cancellation / spawn behavior unchanged. The `Err` arm
  runs in the `ctx.spawn` callback at `mod.rs:326-338` and only
  constructs the `LightboxImage` synchronously; no future is registered
  per failed entry. Downstream, `update_image_at` extracts an
  `asset_source` only from the `Resolved` arm
  (`lightbox_view.rs:102-105`); for `Error`, no `start_asset_load` is
  called and no `when_loaded` future is registered. The previous
  `Loading` shape likewise registered no asset load (no `Resolved`
  source to load), so cancellation semantics are identical: zero
  pending decode/load handles before, zero after.
- [pass] No caller of `screenshot_lightbox_image_from_download_result`
  relies on `LightboxImageSource::Loading` as a failure sentinel. The
  one production caller (`mod.rs:329-335`) only checks `is_some()`. The
  three test callers (`mod_tests.rs:30, 60, 86`) are exactly the unit
  tests for this helper itself. Variant-meaning change is contained.
- [nit] §182 line 192 describes the `Error` panel as rendering the
  entry's `description` (filename) on one line and the `message` on the
  next. With `description: None`, only the `message` line renders. The
  renderer tolerates this, and there is no filename available at the
  artifacts call site (a screenshot fetched by signed URL has no
  user-meaningful filename). This is consistent with the spec, but a
  follow-up might pass `description: Some(artifact_uid)` or similar so
  the user can correlate which artifact failed when multiple are loading
  in one lightbox session. Not in scope for §696. Flagging only.
- [nit] `TIER2_TODO.md` records the impl commit as `6cb2fc6`, but the
  reviewed commit is `5a8072a`. Bookkeeping mismatch only; per the
  review hard rules I did not edit `TIER2_TODO.md`. The author should
  reconcile this in a follow-up housekeeping commit so the table in
  `TIER2_TODO.md` matches the actual commit graph.

# What I checked

- `git show 5a8072a` and `git show --stat 5a8072a` — three files
  touched: `app/src/ai/artifacts/mod.rs` (+11/−2),
  `app/src/ai/artifacts/mod_tests.rs` (+23/−5),
  `specs/GH9729/TIER2_TODO.md` (+2/−2). No production code outside the
  artifacts call site was modified.
- `app/src/ai/artifacts/mod.rs:342-377` — the modified function body.
  Confirmed only the `Err` arm changed; `Ok(Screenshot)` and `Ok(File)`
  arms are byte-identical.
- `crates/ui_components/src/lightbox.rs:27-51, 130-205` —
  `LightboxImageSource::Error` definition and the `Error` render arm in
  `Lightbox::render`. Confirmed `description: None` is handled with
  `if let Some(...)`.
- `app/src/workspace/lightbox_view.rs:93-110` (`update_image_at`) and
  `:145-168` (`start_asset_load`) — confirmed the `Error` source skips
  asset-load spawning, so no leaked future for the failed entry.
- `app/src/workspace/view.rs:21781-21788` — `UpdateLightboxImage`
  handler dispatches to `update_image_at`; does not read `description`.
- `app/src/ai/artifacts/mod_tests.rs:54-82` — full text of the renamed
  test; verified the four required assertions (variant, exact message,
  no leak, `description.is_none`).
- `grep -rn "screenshot_lightbox_image_from_download_result" app crates`
  — three test call sites and one production call site. None depended
  on `Loading` as a failure sentinel.
- `grep -rn '"Failed to load"' app crates --include='*.rs'` — the
  legacy assertion is gone; only comments remain.
- `awk 'NR==696' specs/GH9729/tech.md` and the §182 block at lines
  179-206 — quoted verbatim above.

# Suggestions

- Consider, in a follow-up (not blocking for §696), passing
  `description: Some(artifact_uid)` on the error path so a user with
  multiple artifacts open can identify which one failed. The renderer
  already supports this and §182 documents the description-as-filename
  convention; the artifact UID is the closest analog the artifacts
  call site has.
- Reconcile the `TIER2_TODO.md` impl-commit cell (`6cb2fc6` → `5a8072a`)
  in a separate housekeeping commit.
