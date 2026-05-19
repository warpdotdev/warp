---
item: 4-tests-b
commit: 57be862
reviewer: R2-quality
spec_ref: tech.md §613:643-646
verdict: pass-with-nits
---

# Findings

- **Fixture paths resolve correctly.** `include_bytes!` is resolved relative to the file containing the macro. From `crates/warpui_core/src/image_cache_tests.rs`:
  - `../../warpui/examples/assets/numbers-1000ms.gif` → `crates/warpui/examples/assets/numbers-1000ms.gif` (exists, 120568 bytes).
  - `../test_data/animated.webp` → `crates/warpui_core/test_data/animated.webp` (exists, 140 bytes).
  Both paths are also already used identically in the same file (lines 20-22) by the `Assets` impl, so this is established prior art, not a new pattern. Not brittle in practice.
- **`Frame: !Debug` is real.** Confirmed in `image-0.25.9/src/animation.rs` line 38: `pub struct Frame { ... }` has no `#[derive(Debug)]` and no manual `impl Debug`. The `.err().expect(...)` workaround is the right tool; `unwrap_err()` would not compile. Commit body documents this; in-source there is no inline comment for the next reader who hits the same wall.
- **Refactor scope.** `decode_animated_with_limits` was split into a parameterized `_inner` plus a thin production wrapper that threads `decode_limits()`, `MAX_ANIMATED_FRAMES`, `MAX_ANIMATED_TOTAL_PIXELS`. Mechanical and parallel to the static-decode split landed in 4-tests-a; the same precedent applies here. Production caller surface unchanged.
- **All 4 spec-required tests are present** with the spec-mandated names: `decode_animated_rejects_too_many_frames`, `decode_animated_rejects_total_pixel_budget`, `decode_animated_constructs_bitmap_for_legitimate_input`, `decode_animated_constructs_bitmap_for_legitimate_webp`. Naming follows the spec's `decode_animated_*` prefix; drift from the rest of the file's `test_*` style is spec-driven, not stylistic.
- **Error-message verification is inconsistent.** `decode_animated_rejects_too_many_frames` asserts on `msg.contains("too many frames")`. `decode_animated_rejects_total_pixel_budget` only asserts `is_err()` — it does *not* verify the total-pixel branch fired (a `set_limits` rejection or a decoder error would also satisfy `is_err()`). Tightening the second test to `msg.contains("total pixel budget")` would make the rejection cause unambiguous and parallel test 1.
- **`Limits::default()` is empty in the rejection tests.** Intentional: the test wants `max_frames` / `max_total_pixels` to fire, not the decoder's per-frame `max_alloc`. Idiomatic for these tests.
- **`> 1` frame assertion is loose-but-correct.** Roundtrip tests assert `frames.len() > 1`, not the exact frame count. Per the lens, this is tight enough — it catches accidental truncation to one frame (the regression in scope per the commit body) while remaining robust to fixture changes. Not a nit.
- **`u64::MAX` / `usize::MAX` as "no cap" sentinels.** Correct and idiomatic for tests; pairs cleanly with the inner-fn parameter shape.
- **Comment quality is good.** The leading comments add real detail (e.g. test 1's "BEFORE that frame is appended", test 2's note about the iteration bailing before the push). Worth keeping for security-critical code where the *order of operations* is what matters.
- **Module-scoped `ANIMATED_GIF_BYTES` / `ANIMATED_WEBP_BYTES` constants.** Reasonable since two tests share each fixture; inlining would duplicate the `include_bytes!` macro and the long path string. Acceptable.
- **`include_bytes!` cost claim.** Accurate; the macro is compile-time and the constants are zero-cost references into `.rodata`.
- **Cross-crate fixture path (`../../warpui/...`) duplication.** `numbers-1000ms.gif` lives under `crates/warpui/examples/assets/` and is referenced from `crates/warpui_core` via a four-segment relative path. Already-existing duplication in the file; moving the fixture into a shared `crates/warpui_core/test_data/` is out of scope for a tests commit, but worth a follow-up.
- **No `assert_err_contains` helper.** Only one test uses the `format!("{err}")` + `contains` pattern in this commit and only one other test in the file matches a related shape (line 711). Not enough repetition to motivate extracting a helper yet; revisit if a third instance appears.

# What I checked

- `git show 57be862` — full diff and commit body.
- `crates/warpui_core/src/image_cache_tests.rs` lines 670-755 — refactor target plus the appended tests.
- `crates/warpui_core/src/image_cache.rs` — wrapper/inner split is mechanical, production constants threaded correctly through the wrapper.
- Filesystem existence of both fixtures and their relative-path resolution from the test file's location (both resolve cleanly; fixture sizes confirm non-trivial test data).
- `image-0.25.9/src/animation.rs` confirmed `Frame` has no `Debug` derive or impl, so `.err().expect(...)` is required (not preference).
- Existing `Assets` impl in the same test file (lines 20-22) uses the identical relative `include_bytes!` paths, so the cross-crate path is established prior art.
- `Cargo.toml` and `Cargo.lock` confirm `image = "0.25.9"`, the version under which `Frame: !Debug` was verified.
- `specs/GH9729/tech.md` §613:643-646 — all 4 named tests present, spec-mandated naming honored.
- Existing test conventions in the file — `result.is_err()` is the dominant pattern (lines 601, 621, 686); the message-contains pattern is new and used in only test 1 of this commit.

# Suggestions

- **Tighten `decode_animated_rejects_total_pixel_budget`** to verify the error message: `let err = result.err().expect("expected total-pixel cap to fire"); assert!(format!("{err}").contains("total pixel budget"), ...);`. This makes the rejection cause unambiguous and matches the rigor of test 1. Low risk, high signal.
- **Add a one-line inline comment** at the `.err().expect(...)` in test 1 explaining why `unwrap_err()` is unavailable, e.g. `// image::Frame does not derive Debug, so unwrap_err is unavailable here.`. The commit body documents it but the next reader of the test file does not see commit bodies.
- **Follow-up (out of scope here):** consider relocating `numbers-1000ms.gif` into `crates/warpui_core/test_data/` so the fixture path stops crossing crate boundaries. Pure cleanup; do not block on it.
- **Defer extracting** an `assert_err_contains(result, needle)` helper until a third call site materializes; one local instance is below the threshold.
