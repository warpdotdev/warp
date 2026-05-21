---
item: 4-tests-b
commit: 57be862
reviewer: R1-correctness
spec_ref: tech.md §613:643-646
verdict: pass-with-nits
---

# Findings

- Spec fidelity (test names): all four prescribed names appear verbatim in `image_cache_tests.rs`:
  `decode_animated_rejects_too_many_frames`, `decode_animated_rejects_total_pixel_budget`,
  `decode_animated_constructs_bitmap_for_legitimate_input`,
  `decode_animated_constructs_bitmap_for_legitimate_webp`. Match §613:643-646.
- Refactor justification: `decode_animated_with_limits` is split into a thin production
  wrapper plus `decode_animated_with_limits_inner(data, format, limits, max_frames,
  max_total_pixels)`. Mirrors the 4-tests-a split of `decode_static_with_limits`. Justified:
  testing rejection paths against existing 600x600 / 2x2 fixtures requires sub-production
  caps, and synthesizing a pathological multi-million-frame animated WebP at runtime would
  be far worse than parameterizing the helper. The wrapper's only job is to thread the
  three GH9729 production constants, so behavior preservation is mechanical and obvious by
  inspection.
- Behavior preservation: the wrapper passes `decode_limits()`, `MAX_ANIMATED_FRAMES`,
  `MAX_ANIMATED_TOTAL_PIXELS` — the exact three values the original function read directly.
  Loop body, bail order, and empty-frames check are byte-identical to the pre-refactor
  code. Both production call sites (`image_cache.rs:547` GIF, `:565` WebP) still go through
  the wrapper. No orphan callers; `_inner` is only reached from the wrapper or the tests.
- Test 1 (`too_many_frames`): correct. `numbers-1000ms.gif` has many frames (file inspection
  shows ~441 image descriptor markers). With `max_frames = 1`, iteration index 1 trips
  `if i >= 1` on the second frame BEFORE the second frame is decoded or pushed. The error
  is exactly `"animated image has too many frames"` and the substring assertion is on the
  spec-contract message. Note this confirms the helper produces the right error text
  internally; it does not exercise the user-facing UX layer (sanitize_load_error /
  ImagePreview Error variant) — out of scope for this item but worth flagging that 4b's
  upstream concern about "too many frames" surfacing to the user is NOT covered by this
  test.
- Test 2 (`total_pixel_budget`): passes, but the comment is wrong. The fixture
  `animated.webp` is a 2x2 canvas (verified via VP8X chunk: width=2, height=2, animated=1,
  two ANMF frames). Trace:
  - i=0 < usize::MAX → continue.
  - frame 0 decode succeeds; pixels = 2*2 = 4.
  - total_pixels = 0.saturating_add(4) = 4.
  - `4 > 1` → bail with `"animated image exceeds total pixel budget"`. Frame 0 is NOT
    pushed.
  The empty-frames branch is unreachable here because the loop exits via `bail!`, not
  natural iterator exhaustion. The commit body's "the empty-frames branch catches it"
  and the in-test comment "the empty-frames branch catches it" are both technically
  incorrect — the pixel-budget bail catches it directly. The test still asserts only
  `is_err()`, so it passes either way, but the comment misleads future readers and (worse)
  the test would silently still pass if the pixel-budget bail were ever removed and the
  empty-frames bail caught it instead. Tightening the assertion to
  `msg.contains("exceeds total pixel budget")` would (a) fix the documentation drift and
  (b) lock the test to the actual code path the spec wants exercised.
- Test 3 (legitimate GIF): correct regression check. Asserts `frames.len() > 1` against the
  441-frame fixture under production caps (`MAX_ANIMATED_FRAMES = 512` per the constants;
  fixture's 441 < 512 so it is in-budget). Catches a 4b regression where frame collection
  could truncate.
- Test 4 (legitimate WebP): correct same shape; the 2-frame `animated.webp` fixture
  satisfies `frames.len() > 1` under production caps.
- Determinism / isolation / side effects: pure in-memory `include_bytes!`, no FS/network,
  no shared state. `image::GifDecoder` and `image::WebPDecoder` are deterministic. Tests
  are independent.
- `Frame: Debug` workaround: `result.err().expect(msg)` is sound — `.err()` returns
  `Option<E>` regardless of the Ok-side `Debug` bound, then `.expect` unwraps. Idiomatic
  fix.
- Coverage gaps (acceptable, not blocking):
  - No "too many frames" test for WebP (only GIF). Mixed-format coverage is asymmetric.
  - No "total pixel budget" test for GIF (only WebP). Same observation, opposite axis.
  - No test exercises the empty-frames branch (`"animated image has no frames"`).
    The branch is reachable only if a decoder yields 0 items via natural iterator
    exhaustion — hard to synthesize and rare in practice. Acceptable as untested.
  - No test for the `_ => bail!("decode_animated_with_limits called with non-animated
    format")` arm. Programmer-error guard; not worth a test.

# What I checked

- `git show 57be862` — full diff of image_cache.rs and image_cache_tests.rs.
- `specs/GH9729/tech.md` §613:643-646 — confirmed test names and behaviors match the four
  prescribed tests.
- `crates/warpui_core/src/image_cache.rs:340-430` — verified the wrapper / `_inner` split,
  parameter threading, and that the original behavior is preserved by reading both halves
  side-by-side.
- `crates/warpui_core/src/image_cache_tests.rs:678-755` — verified test bodies against
  spec.
- `grep` for `decode_animated_with_limits(_inner)?` across the workspace — confirmed two
  production call sites still route through the wrapper, `_inner` is only used by tests,
  no orphan refactor.
- Fixture inspection:
  - `numbers-1000ms.gif`: GIF89a, 600x600, ~441 image descriptors → multi-frame ✓.
  - `animated.webp`: VP8X with animated flag set, 2x2 canvas, 2 ANMF chunks → animated,
    2 frames ✓. Critically: 2x2 = 4 pixels, which exceeds the test's `max_total_pixels=1`
    cap on the very first frame, confirming the pixel-budget bail (not empty-frames) is
    what fires in test 2.
- Verified `Frame: Debug` workaround (`.err().expect(...)`) is sound.

# Suggestions

- Tighten test 2 to assert the actual error text:
  ```rust
  let err = result.err().expect("expected total-pixel cap to fire");
  let msg = format!("{err}");
  assert!(
      msg.contains("exceeds total pixel budget"),
      "expected total-pixel error, got: {msg}",
  );
  ```
  Reasons: (a) corrects the misleading "the empty-frames branch catches it" comment that
  is factually wrong for this fixture (2x2 first frame triggers pixel-budget bail directly,
  iterator never exhausts), (b) prevents silent pass-through if the pixel-budget bail were
  ever removed, (c) brings test 2 into structural parity with test 1 which already
  string-matches its bail message.
- Fix the comment in test 2 to describe the actual control flow ("the first frame's pixel
  count exceeds the budget, so the pixel-budget bail fires before the frame is pushed")
  and update the commit body's narrative if it is preserved into a squashed merge commit.
- Optional (not blocking): consider adding a WebP "too many frames" test and a GIF
  "total pixel budget" test for symmetric format coverage. Spec only requires one of each
  axis, so this is at the author's discretion.
- Optional: the 4-tests-b commit confirms the helper produces the substring `"too many
  frames"` internally, but R1 noted in 4b that the user-facing UX layer was not verified.
  This test does not close that gap (correctly — that is a UX layer concern, not a decoder
  helper concern), but the IMPLEMENTATION_TODO check should not be read as covering the
  user-facing path.
