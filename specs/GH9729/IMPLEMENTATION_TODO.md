# GH9729 image-preview — implementation TODO

Authoritative spec: `specs/GH9729/tech.md` (do **not** edit).
Branch: `spec/GH9729-image-preview` (this branch).

This file is the single source of truth for the Ralph loop driving the
implementation. The loop reads it each iteration, picks the first unchecked
item, does **only** that item, flips its checkbox in the same commit, and
stops. The next iteration sees the updated state and picks the next item.

## Hard rules for every iteration

- Touch only the files the current item requires. Never re-read the whole
  `tech.md` — open just the section the item points to.
- Use the `Explore` subagent for codebase lookups; do not grep from the main
  context window.
- Run only the narrowest tests relevant to the change. Save the
  full-workspace `cargo fmt` / `clippy -D warnings` / `nextest` for the FINAL
  step.
- Keep diffs small (≤ ~150 lines where reasonable). Split if larger.
- Commit prefix: `GH9729(impl):` followed by a one-line summary.
- Do not edit `specs/GH9729/product.md` or `specs/GH9729/tech.md`.
- After finishing the item, mark its checkbox `[x]` in the same commit and
  stop. Do **not** start the next item.

## Steps

- [x] 1a. Add `FileTarget::ImagePreview` variant — `tech.md` §74 (`### 1.`)
- [x] 1b. Resolver short-circuit in `resolve_file_target_with_editor_choice`
       (PNG/JPEG/GIF/WebP/SVG ahead of markdown probe and binary fall-through)
       — `tech.md` §74
- [x] 1c. Unit tests for resolver: each supported extension, precedence over
       markdown, non-image binary still `SystemGeneric` — `tech.md` §613
**Spec narrowing note (commit `112e581`).** v1 is single-image, no sibling
navigation. The `list_sibling_images_natural_sorted` helper, `MAX_SIBLING_IMAGES`
cap, hidden-file filter, and natural-sort tests that the original PR body
mentioned are explicitly out of scope per `tech.md` §119 ("Single-element Vec...
v1 always passes a one-element vec") and the descope listed in `product.md`.
The original 2a / 2c bullets are struck through and superseded.

- [x] 3a. `LightboxImageSource::Error { message }` variant in
       `crates/ui_components/src/lightbox.rs` — `tech.md` §182 (`### 3.`).
       Promoted ahead of the workspace arm because §119 references it.
- [x] 3b. Render the `Error` variant inline in `Lightbox::render` (filename
       on one line, message on the next; non-blocking, dismissal still works)
       — `tech.md` §182. Adopting `Error` at `app/src/ai/artifacts/mod.rs:362-365`
       is explicitly a follow-up per §182 and is NOT in this bullet.
- [x] 2-arm. Workspace `FileTarget::ImagePreview` arm: synchronous
       `metadata` size/regular-file check (`MAX_PREVIEW_FILE_BYTES = 64 MB`),
       `truncate_message` helper (`MAX_ERROR_MESSAGE_LEN = 256`), single-element
       `OpenLightbox` dispatch — `tech.md` §119
- [x] 2-tests. Workspace-arm tests per `tech.md` §613 (lines 623-626):
       `image_preview_arm_builds_resolved_when_under_size_cap`,
       `image_preview_arm_builds_error_when_over_size_cap`,
       `image_preview_arm_builds_error_when_metadata_fails`,
       `image_preview_arm_builds_error_for_non_regular_file`
- ~~[ ] 2a. `list_sibling_images_natural_sorted` helper + `MAX_SIBLING_IMAGES`
       cap (1024) + hidden-file filter — out of scope (single-image v1)~~
- ~~[ ] 2c. Unit tests for natural sort, hidden-file filter, sibling cap —
       out of scope (single-image v1)~~
- [x] 4a. Static-decode caps in `ImageType::try_from_bytes`:
       `image::Limits` (max dimension, max alloc) + `MAX_DECODE_PIXELS = 67M`
       — `tech.md` §234 (Static raster, under §217)
- [x] 4b. Animated decode caps: `MAX_ANIMATED_FRAMES`, `MAX_ANIMATED_TOTAL_PIXELS`
       — `tech.md` §259 (Animated WebP / GIF, under §217)
- [x] 4c. SVG content-sanity prefix check + intrinsic-dimension cap —
       `tech.md` §321
- [x] 4-tests-a. Static-raster + SVG decoder tests per `tech.md` §613
       (lines 640-642 and 647-652): 3 static + 6 SVG = 9 tests.
- [x] 4-tests-b. Animated decoder tests per `tech.md` §613 (lines 643-646):
       4 tests (frame-count, total-pixel, GIF roundtrip, WebP roundtrip).
       Split out because animated-WebP fixture synthesis is non-trivial.
- [x] 5a. Bound the `LocalFile` asset-cache read with content-keyed cap
       (raster vs SVG by 1 KB content peek), post-open `is_file()` check,
       and `O_NONBLOCK` regression guard — `tech.md` §400 (`### 5.`)
- [x] 5b. `lightbox_view.rs` post-load callback rewrites `FailedToLoad` /
       `Unrecognized` to `Error` — `tech.md` §613 (lines 660-661)
- [x] 5-tests. Asset-cache tests per `tech.md` §613 (lines 628-636).
- [x] 7.  Telemetry serialization test for the existing `CodePanelsFileOpened`
       event — `tech.md` §517 (`### 7.`). NOTE: §517 explicitly says "no
       additional event, no additional enum, no aggregation change"; the
       v1 telemetry posture is the existing event's `target` field
       distinguishing image opens via the stable string `"image_preview"`,
       wired in item 1a. This bullet adds the acceptance test only.
- [x] FINAL. `cargo fmt` applied; `cargo clippy --workspace --exclude
       command-signatures-v2 --all-targets --tests -- -D warnings` clean;
       `cargo nextest run --no-fail-fast --workspace --exclude
       command-signatures-v2` ran 5933 tests with all 38 GH9729-touched
       tests passing. The 7 non-GH9729 failures (SSH integration, git tag
       display, settings migration marker) reproduce on master and are
       pre-existing environmental issues unrelated to this work.

## Notes

- §6 in the tech spec is intentionally a no-op ("No change to
  `crates/warp_util/src/file_type.rs`"); it is omitted from this list.
- If a step's tests reveal the change is too large, split it into smaller
  follow-up commits but keep the checkbox unchecked until all sub-changes
  for that bullet land.

## Reviews

Driven by a separate Ralph loop (see `specs/GH9729/REVIEW_LOOP_PROMPT.md`).
Each implementation bullet above gets **two parallel reviews** per iteration:

- **R1 — Correctness lens:** spec-fidelity vs `tech.md`/`product.md`, edge
  cases, error paths, security/DoS/resource caps (decode bombs, SVG sanity,
  size caps, allocator limits).
- **R2 — Quality lens:** idiomatic Rust, naming, structure, test rigor,
  dead code, comment quality.

Notes are written to `specs/GH9729/reviews/<item>-r<N>.md`.

| # | Item | Commit | R1 | R2 |
|---|------|--------|----|----|
| 1a | `FileTarget::ImagePreview` variant | `fa5336b` | [x] | [x] |
| 1b | resolver short-circuit | `c15c1be` | [x] | [x] |
| 1c | resolver unit tests | `256d31d` | [x] | [x] |
| 3a | `LightboxImageSource::Error` variant | `c0b5a7c` | [x] | [x] |
| 3b | render `Error` inline in `Lightbox::render` | `328c333` | [x] | [x] |
| 2-arm | workspace `FileTarget::ImagePreview` arm | `38aec6e` | [x] | [x] |
| 2-tests | workspace-arm tests | `32cee73` | [x] | [x] |
| 4a | static-decode caps in `ImageType::try_from_bytes` | `926dedf` | [x] | [x] |
| 4b | animated-decode caps (WebP, GIF) | `a6aaf00` | [x] | [x] |
| 4c | SVG content-sanity gate + intrinsic-dimension cap | `294fa95` | [x] | [x] |
| 4-tests-a | static-raster + SVG decoder tests | `abab042` | [x] | [x] |
| 4-tests-b | animated decoder tests | `57be862` | [x] | [x] |
| 5a | bounded `LocalFile` asset-cache read | `e338655` | [x] | [x] |
| 5b | post-load callback rewrites failures to `Error` | `d5fdacc` | [x] | [x] |
| 5-tests | asset-cache tests | `4ed1e80` | [x] | [x] |
| 7 | telemetry serialization test | `ec6ffb1` | [x] | [x] |
| FINAL | presubmit (fmt + clippy + nextest) | `3bf5148`, `f743be1`, `8a5c2e6` | [x] | [x] |

Tick `[x]` only after the corresponding `reviews/<item>-r<N>.md` exists and
contains real findings (or an explicit "no issues found, here's what I
checked" sign-off — empty stubs do not count).
