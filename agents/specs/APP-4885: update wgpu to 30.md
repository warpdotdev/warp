# APP-4885: Update wgpu to 30.0.0 for Linux Vulkan frame pacing

*Proposed change: bump the Warp desktop client's `wgpu` dependency from 29.0.1 to 30.0.0 and migrate the experimental renderer to the wgpu 30 APIs.*

*Summary:* The workspace currently pins `wgpu` 29.0.1 in `Cargo.toml` and `Cargo.lock`; the experimental WarpUI renderer uses APIs whose signatures changed in wgpu 30.0.0. The upgrade is needed to pick up the upstream Vulkan-on-Linux frame-pacing and buffer-mapping crash fixes.

*Key design choices:* Keep the existing workspace dependency declaration and backend feature set, update only the renderer call sites required by the major-version API changes, and validate both compile-time compatibility and a real Vulkan frame/readback path. Preserve existing error propagation and frame ordering rather than weakening checks to make the migration compile.

*Design alternatives:* 
- Stay on wgpu 29.0.1 and workaround the Vulkan/readback defects locally — rejected because it does not receive the upstream fixes and would create renderer-specific maintenance.
- Upgrade wgpu without adapting call sites — rejected because wgpu 30 changes vertex-buffer option types, mapped-range result handling, presentation ownership, and adapter fixture fields.
- Replace the experimental renderer or change its backend feature list — rejected because the ticket is a dependency/API migration and the existing DX12, GLES, Metal, Vulkan, WGSL, `std`, and `parking_lot` support must remain unchanged.

*Root cause / approach:* At commit `a6b677247d9b4649d0af2c9219423dda5942766f`, the workspace dependency is `wgpu = { version = "29.0.1", ... }` (`Cargo.toml:370-376`) and the lockfile resolves wgpu 29.0.1. The renderer constructs three pipelines with bare `VertexBufferLayout` values (`crates/warpui/src/rendering/wgpu/renderer/rect.rs:55-68`, `renderer/glyph.rs:90-112`, and `renderer/image.rs:82-104`); wgpu 30 requires the layouts in `VertexState::buffers` to be wrapped as optional entries. The buffer initialization/readback paths use `get_mapped_range_mut` and `get_mapped_range` as infallible (`renderer/util.rs:78-87` and `renderer.rs:224-256`), while wgpu 30 returns mapping errors that must be propagated through the existing renderer error/string paths. The render completion path calls `SurfaceTexture::present()` (`renderer.rs:128-139`); migrate it to the wgpu 30 queue presentation API while retaining the current “do not present after a scoped validation error” ordering. Update `resources_tests.rs` adapter fixtures for wgpu 30’s optional `AdapterInfo.limit_bucket` and `transient_saves_memory` fields.

*Affected files:* 
- `Cargo.toml` and `Cargo.lock` — resolve wgpu 30.0.0 with the current feature set and transitive lockfile updates.
- `crates/warpui/src/rendering/wgpu/renderer/rect.rs`
- `crates/warpui/src/rendering/wgpu/renderer/glyph.rs`
- `crates/warpui/src/rendering/wgpu/renderer/image.rs`
- `crates/warpui/src/rendering/wgpu/renderer/util.rs`
- `crates/warpui/src/rendering/wgpu/renderer.rs`
- `crates/warpui/src/rendering/wgpu/resources_tests.rs`
- Add focused renderer regression coverage in the existing WarpUI test layout if needed to exercise mapped readback and presentation on a Vulkan adapter; do not broaden unrelated test fixtures.

*Open questions resolved:* The requested target is exactly wgpu 30.0.0 (released 2026-07-01); Rust 1.87 is its minimum supported version and the repository toolchain is Rust 1.92. The renderer remains experimental and feature-gated; no rollout or feature-flag change is required. Full WarpUI tests currently have five unrelated baseline failures, so post-change comparison is against the recorded baseline failure set rather than an expectation of a completely green suite. GPU validation may be unavailable on some CI/agent hosts; when Vulkan hardware is unavailable, record the environment limitation and retain the compile/test evidence, but a Vulkan-capable Linux run is required before merge when an available runner can exercise it.

*Risks / blast radius:* A major wgpu update can change validation behavior, mapped-buffer error timing, surface presentation ordering, adapter selection, or platform-specific backend compilation. Mitigate by retaining all current error scopes and `Result` propagation, preserving the existing backend features, running locked compile/tests on all supported targets available to CI, comparing the complete WarpUI failure set with baseline, and manually exercising Linux Vulkan frame pacing plus capture/readback.

*Validation & verification criteria* (must ALL pass before merge):

1. `Cargo.toml` requests exactly wgpu 30.0.0 with the existing `default-features = false` and `dx12`, `gles`, `metal`, `parking_lot`, `std`, `vulkan`, and `wgsl` features; `Cargo.lock` resolves wgpu 30.0.0 and is internally consistent. Check with `cargo metadata --manifest-path Cargo.toml --locked` and a lockfile diff review.
2. The experimental renderer compiles with no migration errors using `cargo check -p warpui --features experimental-wgpu-renderer --locked`.
3. All three render pipelines compile with `VertexState::buffers` entries matching wgpu 30’s optional vertex-buffer-layout API, and the rect, glyph, and image draw paths still bind their instance buffers and issue their existing indexed draws. Check through the compile above plus the focused renderer test/build target.
4. `create_buffer_init` and `capture_surface_texture` handle mapping results explicitly: mapped-range access errors are returned/logged through the existing error path, buffers are unmapped only after successful access, and no panic/unchecked mapping result is introduced. Add a focused regression test that exercises a successful mapped readback and the relevant failure path where the available wgpu test harness permits it; the test must fail against the pre-migration implementation or document why the upstream-only failure cannot be deterministically reproduced on the available adapter.
5. The render flow submits draw work, invokes the optional capture callback, invokes the pre-present callback, and presents exactly once only when the scoped validation result is successful; the wgpu 30 queue-based presentation API must not reorder these operations. Verify with a focused renderer test or instrumented Vulkan run.
6. Update every `wgpu::AdapterInfo` fixture in `crates/warpui/src/rendering/wgpu/resources_tests.rs` to compile against wgpu 30 (`limit_bucket: None`, `transient_saves_memory: Some(false)` as appropriate) without changing the assertions for old lavapipe or Intel UHD detection. Run `cargo test -p warpui --features experimental-wgpu-renderer resources_tests --locked`.
7. Record the baseline result of `cargo test -p warpui --features experimental-wgpu-renderer --locked` before implementation and rerun it after implementation; the post-change run has no new failures, with only the same five pre-existing unrelated failures (or fewer).
8. On a Linux host with Vulkan support, run the experimental renderer with Vulkan selected (for example, `WGPU_BACKENDS=vulkan`/the repository’s equivalent backend selection) and render a representative scene containing rectangles, glyphs, and images for multiple consecutive frames. Confirm there is no frame-pacing stall or validation/device-loss error and that presentation continues at the expected cadence.
9. In the same Vulkan-capable run, exercise the frame-capture/readback path so a surface texture is copied, mapped, read, and unmapped successfully; confirm the callback receives the expected non-empty dimensions and RGBA bytes, and repeat across multiple frames to exercise the upstream buffer-mapping crash fix.
10. Run `./script/presubmit` from the repository root. Any unavailable GPU/manual criterion must be reported with the host limitation and a reproducible command for a Vulkan-capable runner; it is not a reason to skip the locked compile, focused tests, full-suite comparison, or presubmit.
