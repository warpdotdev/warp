---
item: 5-tests
commit: 4ed1e80
reviewer: R2-quality
spec_ref: tech.md §613:628-636
verdict: pass-with-nits
---

# Findings

- The 7 tests prescribed in `tech.md §613:628-636` are present and named
  exactly per the spec (`local_file_read_passes_under_cap`,
  `local_file_read_caps_at_max_bytes`,
  `local_file_read_rejects_post_open_non_regular_file`,
  `local_file_read_does_not_block_on_fifo` (with `#[cfg(unix)]`),
  `local_file_read_caps_svg_at_smaller_limit`,
  `local_file_read_caps_svg_content_under_png_extension`,
  `local_file_read_uses_raster_cap_for_non_svg_content`).
  Spec-driven naming convention (no `test_` prefix) is consistent with
  prior test commits in this branch.
- Refactor splits `load_local_file_bounded` into a thin wrapper plus a
  parameterized `load_local_file_bounded_inner(path, raster_cap, svg_cap)`.
  Mechanical extraction; production constants are threaded by the wrapper
  unchanged. No behavior change for the production call site.
- Test driver is `futures_lite::future::block_on`. `futures-lite` is
  already a runtime dep of `warpui_core` (Cargo.toml:39), and
  `block_on` is already used elsewhere in the crate
  (`async/wasm/mod.rs:9`). `async_fs` is built on `futures-lite`, so
  `block_on` is the correct executor for these futures. No new dep.
- `tempfile` and `libc` are already dependencies of `warpui_core`
  (Cargo.toml:50, 76 in `[dependencies]`; `tempfile` is also redeclared
  in `[dev-dependencies]` at 87, a pre-existing duplication, not a
  regression introduced here). Tests compile under the existing dep
  surface — no new dependency added by this commit.
- The FIFO test (`local_file_read_does_not_block_on_fifo`) is the
  highest-leverage test here: it is a real regression guard for the
  `O_NONBLOCK` flag. Without `O_NONBLOCK`, `block_on` would park
  indefinitely on the `open()` syscall and the test would hang rather
  than fail; with `O_NONBLOCK` it returns Err quickly. Minimal
  `unsafe { libc::mkfifo(...) }` block, justified, with rc-checked
  assertion. Idiomatic.
- `svg_payload(target_bytes)` produces a deterministic, well-formed SVG
  byte sequence whose first 1 KB matches `looks_like_svg_xml`, then pads
  to size with `<g/>` tags and a final whitespace top-up. Smart helper.
- `small_png_bytes()` builds a 200x100 RGBA PNG via `image::DynamicImage::write_to`.
  Same shape as the `encode_blank_png` helper used in the 4-tests-a
  commit but local to this `mod tests`. Could be hoisted into a shared
  test util later — but at one call-site here, hoisting now would be
  premature.
- `path.to_string_lossy().into_owned()` matches the production
  call-site's owned-`String` argument convention.
- `assert!(result.is_err(), "msg")` consistently across rejection
  tests with custom messages.

# What I checked

- Read the full diff of commit `4ed1e80` and the resulting test module
  in `crates/warpui_core/src/assets/asset_cache.rs:498-750`.
- Cross-referenced each test against the spec in
  `specs/GH9729/tech.md §613:628-636`. Names, fixtures, and expected
  outcomes match. The use of small caps via the `_inner` wrapper is a
  reasonable interpretation of the spec's "slightly larger than"
  language: spec asks for caps to fire; testing at 200 bytes vs a
  100-byte cap is materially equivalent to 64 MB+1 vs a 64 MB cap and
  much faster.
- Confirmed `futures-lite` and `async-fs` are existing workspace deps
  in `crates/warpui_core/Cargo.toml`, so `block_on` is an in-tree
  driver and not a new dep.
- Confirmed `tempfile` and `libc` are already declared in
  `[dependencies]`, so `#[cfg(test)]` use does not require any
  Cargo.toml change in this commit (and indeed none was made).
- Confirmed `#[cfg(unix)]` is correctly applied only to the FIFO test;
  the other six tests are cross-platform.

# Suggestions

- (nit) `local_file_read_caps_at_max_bytes` only asserts `is_err()`. It
  doesn't verify the rejection came from the size-cap branch
  specifically (vs, e.g., an unrelated I/O error). Same observation
  was raised in the 4-tests-b R2 review. Optional improvement: assert
  on the error message ("local asset exceeds size cap") so a future
  refactor that silently dropped the cap check would be caught here.
- (nit) The literal `50` for the SVG cap is repeated four times across
  the SVG-cap tests. A `const SMALL_SVG_CAP: u64 = 50;` near the
  helpers would document intent ("smaller than the smallest SVG XML
  payload we generate") at trivial cost. Trade-off is fine either way.
- (nit) `local_file_read_caps_at_max_bytes` could add a
  boundary-equal-to-cap case (write `cap` bytes against `cap` cap and
  assert Ok) to lock down the exact `MAX + 1` semantics. The current
  test catches "200 vs 100" which is comfortably over but doesn't
  exercise the boundary. Spec doesn't require it; would be additive.
- (nit) The wrapper `load_local_file_bounded` (production-constant
  thread-through, one-liner) is not directly exercised. Wrapper-bypass
  is the same trade-off accepted in 4-tests-a. The wrapper's only
  behavior beyond `_inner` is "passes the right two constants" — a
  trivial argument. Acceptable.
- (nit) `tempfile` appears in both `[dependencies]` and
  `[dev-dependencies]` of `warpui_core/Cargo.toml`. Pre-existing
  duplication, not introduced by this commit. Worth flagging once for
  cleanup in a follow-up but not blocking.
