---
item: 5-tests
commit: 4ed1e80
reviewer: R1-correctness
spec_ref: tech.md §613:628-636
verdict: pass-with-nits
---

# Findings

- **Spec fidelity: full set of 7 prescribed tests is present, names match exactly.** Cross-checked against tech.md §613 lines 628-636:
  1. `local_file_read_caps_at_max_bytes` (line 629) — present, asset_cache.rs:628
  2. `local_file_read_passes_under_cap` (line 630) — present, asset_cache.rs:614
  3. `local_file_read_rejects_post_open_non_regular_file` (line 631) — present, asset_cache.rs:642
  4. `local_file_read_does_not_block_on_fifo` (`#[cfg(unix)]`, line 632) — present, asset_cache.rs:660, correctly cfg-gated
  5. `local_file_read_caps_svg_at_smaller_limit` (line 633) — present, asset_cache.rs:681; covers both `.svg` and `.bin` per spec
  6. `local_file_read_caps_svg_content_under_png_extension` (line 634) — present, asset_cache.rs:712
  7. `local_file_read_uses_raster_cap_for_non_svg_content` (line 635) — present, asset_cache.rs:731

  No spec-prescribed test is missing.

- **Refactor (`load_local_file_bounded` -> `_inner(path, raster_cap, svg_cap)` thin wrapper) is correct and necessary.** The production wrapper at asset_cache.rs:574-576 threads `MAX_ASSET_LOCAL_FILE_BYTES` (64 MB) and `MAX_SVG_BYTES` (4 MB) and is the only production call site. Verified by diffing 4ed1e80 against 4ed1e80^: the `_inner` body is byte-for-byte identical to the previous monolithic function except `MAX_ASSET_LOCAL_FILE_BYTES` -> `raster_cap` and `MAX_SVG_BYTES` -> `svg_cap` (lines 542-545, 550, 553 in the new code; lines 559-565 in the old code). No semantic change. Behavior preservation confirmed.

  This pattern (thin production wrapper + `_inner` that takes caps as parameters) is the same one used in items 4-tests-a / 4-tests-b for the workspace arm, so it is consistent across the spec implementation.

- **Single production call site updated.** The only invocation is asset_cache.rs:324 (`Box::pin(load_local_file_bounded(path))`), which still calls the wrapper, not `_inner` directly. Production code flow unchanged.

- **`block_on` correctness.** `futures_lite::future::block_on` polls a future to completion on the current thread. `async_fs` 2.1.2 (workspace dep) is built on the `blocking` crate, which dispatches blocking syscalls to a global thread pool and returns futures driven by `Waker` notifications. `block_on` correctly drives those wakers. This is the canonical idiom; no risk here.

- **FIFO test (the critical regression guard).** Without `O_NONBLOCK`, `open(2)` of a FIFO with no writer attached blocks indefinitely (POSIX semantics). With `O_NONBLOCK`, `open()` returns immediately and the post-open `is_file()` check rejects it. The test correctly creates a real FIFO via `libc::mkfifo` and asserts the future returns `Err`. If a future change drops `O_NONBLOCK`, this test will hang.

  **Mitigation found:** `.config/nextest.toml` line 4 sets `slow-timeout = { period = "30s", terminate-after = 2 }`, i.e. nextest will terminate the test at 60s. So a regression dropping `O_NONBLOCK` will not hang the runner indefinitely — it will be killed at 60s and reported as a timeout. The spec text at line 632 ("the future would hang indefinitely and the test would time out") matches this expectation. Concern resolved.

- **Test correctness, per case:**
  - `passes_under_cap`: 5-byte payload under a 1 KB cap, asserts exact bytes. ✓
  - `caps_at_max_bytes`: 200 bytes against a 100-byte cap, asserts `is_err()`. Doesn't verify which bail message fired (size cap vs metadata vs other), but the spec only requires `Err`, so this is in-spec.
  - `rejects_post_open_non_regular_file`: passes a directory path. Comment correctly notes the platform variability of `open()` on a directory; either the open fails or the post-open `is_file()` rejects. Either way `is_err()` holds. ✓ Acceptable simulation per the spec ("on platforms without convenient FIFO support … simulate by opening a path that already resolves to a non-regular file").
  - `does_not_block_on_fifo`: real FIFO via `libc::mkfifo`, mode 0o644, cfg(unix). ✓
  - `caps_svg_at_smaller_limit`: covers both `.svg` and `.bin`; payload is 200 bytes against a 50-byte SVG cap, raster cap 1 KB. ✓
  - `caps_svg_content_under_png_extension`: SVG XML written to `evil.png`, 50-byte SVG cap, 1 KB raster cap. The peek (~200 bytes of `<svg xmlns=...>`) matches `looks_like_svg_xml` regardless of `.png` extension. ✓
  - `uses_raster_cap_for_non_svg_content`: PNG bytes (starting `\x89PNG…`) under `.svg` extension, raster cap = `png_len + 1024`, SVG cap = 50. Confirmed PNG magic bytes do NOT match `looks_like_svg_xml` (it requires `<?xml`, `<svg`, `<!--`, or `<!DOCTYPE` after BOM/whitespace strip — image_cache.rs:335-338). ✓

- **Test fixtures are deterministic:** `svg_payload(N)` deterministically produces N-byte SVG XML; `RgbaImage::new(200, 100)` zero-initializes pixels and `image::ImageFormat::Png` encoding is deterministic for fixed input. ✓

- **Test isolation:** each test creates its own `tempfile::TempDir`, all cleanup via Drop. ✓

- **Cross-platform:** `mkfifo` test gated by `#[cfg(unix)]`. Other tests use only portable APIs. ✓

# What I checked

- `git show 4ed1e80 --stat` and full commit body.
- `tech.md` §613 lines 620-650 (spec for the 7 asset-cache tests + adjacent context).
- `crates/warpui_core/src/assets/asset_cache.rs` lines 500-750 (refactor + tests).
- Pre-refactor `load_local_file_bounded` body via `git show 4ed1e80^:crates/warpui_core/src/assets/asset_cache.rs` to verify behavior preservation.
- `looks_like_svg_xml` impl at `crates/warpui_core/src/image_cache.rs:324-339` to verify SVG-prelude detection (and that PNG magic does NOT match it).
- `.config/nextest.toml` for default per-test timeout (`slow-timeout = 30s, terminate-after = 2` -> 60s hard kill).
- `Cargo.toml` to confirm `async-fs = 2.1.2`, which sits on top of the `blocking` crate and drives correctly under `futures_lite::future::block_on`.
- Production call site for `load_local_file_bounded` (asset_cache.rs:324) — wrapper still used, no production behavior change.

# Suggestions

These are nits, not blockers:

1. **Coverage gap — exact-boundary test (cap and cap+1).** The implementation reads `cap + 1 - peeked` and bails on `buf.len() > cap`. The current `caps_at_max_bytes` test uses 200 bytes / 100-byte cap, which is well over. A future test like `local_file_read_passes_at_exact_cap` (cap = 100, file = 100) and `local_file_read_rejects_just_over_cap` (cap = 100, file = 101) would lock the off-by-one semantics in. Spec doesn't mandate it; nice-to-have.

2. **`caps_at_max_bytes` doesn't distinguish which bail fired.** It asserts `is_err()` only. If a regression made the read fail for a different reason (e.g. pre-`open` failure), the test would still pass. A `result.unwrap_err().to_string().contains("size cap")` would tighten it. Spec doesn't require the message check, so this is consistent with §613 text but a stronger assertion is cheap.

3. **No character-device test.** Not in spec, but `/dev/null` would exercise the post-open `is_file()` check on a different `FileType` variant. Could be added as `#[cfg(unix)]` and would be quick. Skip if scope creep is a concern — the FIFO test already exercises a non-regular file type.

4. **No symlink-to-FIFO swap simulation.** Spec line 631 mentions "race-replace it with a FIFO … between the pre-read metadata stat and the asset-cache open call" — but explicitly allows the simulation form ("simulate by opening a path that already resolves to a non-regular file"), which the directory test does. Acceptable.

5. **`mkfifo` mode 0o644 is fine but `0o600` would be tighter** for a test artifact. Cosmetic.

6. **The FIFO test relies on nextest's 60s slow-timeout.** Under `cargo test` (no nextest) there is no timeout, so a regression dropping `O_NONBLOCK` would hang `cargo test` indefinitely on this single test. Not a blocker — CI runs nextest and developers usually do too — but worth a doc comment on the test itself: "If `O_NONBLOCK` regresses, this test hangs; rely on nextest's slow-timeout (60s) to surface it."
