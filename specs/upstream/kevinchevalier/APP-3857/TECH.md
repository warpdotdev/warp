# APP-3857: Memory-safe animated image rendering in markdown
See `specs/kevinchevalier/APP-3857/PRODUCT.md` for the product spec.

## Problem
Markdown image blocks currently pay for full animated-image decoding even though the markdown renderer does not actually play animations. `RenderableImage` builds a plain `warpui::elements::Image` without `enable_animation_with_start_time`, so the markdown surface effectively shows only the first frame today (`crates/editor/src/render/element/image.rs:13-49`, `crates/warpui_core/src/elements/image.rs:107-172`). However, the shared image asset path still decodes animated GIF/WebP assets into `AnimatedImage { frames: Vec<...> }`, and the resize path then creates resized copies of every decoded frame (`crates/warpui_core/src/image_cache.rs (259-359)`, `crates/warpui_core/src/image_cache.rs (457-511)`). On large assets this produces the multi-GB memory spike seen in `APP-3857`.

## Relevant code
- `crates/editor/src/content/edit.rs:66-97` — resolves markdown image source strings into `AssetSource`.
- `crates/editor/src/content/edit.rs:677-759` — converts `BufferBlockItem::Image` into a laid-out `BlockItem::Image` with default markdown image sizing.
- `crates/editor/src/render/element/image.rs:13-49` — markdown render block that constructs `warpui::elements::Image` from the resolved asset source.
- `crates/warpui_core/src/elements/image.rs:107-172` — animation only occurs when callers explicitly provide a start time; otherwise the first frame is painted.
- `crates/warpui_core/src/image_cache.rs (259-359)` — `ImageType::try_from_bytes` currently uses `collect_frames()` for animated GIF and animated WebP inputs.
- `crates/warpui_core/src/image_cache.rs (457-511)` — animated resize path duplicates every frame at the target render size.
- `crates/warpui_core/src/image_cache.rs:666-772` — shared image-cache entry point; caches rendered images by asset source hash plus render properties and is where preview-vs-animation behavior can diverge without changing the underlying asset type.
- `crates/warpui_core/src/assets/asset_cache.rs:284-320` — asset cache keys include both `AssetSource` and `TypeId`; this implementation continues to use the existing `ImageType` asset path and keeps full animated assets there.
- `app/src/resource_center/section_views/changelog_section.rs:152` — an existing real app surface that explicitly enables animation and therefore must keep the current full-animation path.
- `crates/warpui_core/src/image_cache_tests.rs (1-261)` and `crates/editor/src/content/markdown_tests.rs (270-337)` — the closest existing test coverage for image decoding/cache behavior and markdown image handling.

## Current state
- Markdown image syntax is parsed as a block item and stored as `BufferBlockItem::Image { alt_text, source }`, then laid out as `BlockItem::Image { asset_source, config, .. }` using the shared editor/render pipeline (`crates/editor/src/content/text.rs:302-373`, `crates/editor/src/content/edit.rs:677-759`).
- The markdown render element constructs a generic `warpui::elements::Image` with `.contain()` and no animation start time (`crates/editor/src/render/element/image.rs:42-49`).
- The `Image` element will animate only if a caller explicitly opts in via `enable_animation_with_start_time`; otherwise it repeatedly paints frame 0 (`crates/warpui_core/src/elements/image.rs:107-172`).
- Despite that static markdown UX, the shared asset loader eagerly decodes animated GIF/WebP files into an in-memory `AnimatedImage` containing all RGBA frames (`crates/warpui_core/src/image_cache.rs (259-359)`).
- `ImageCache::image` then resizes animated assets by resizing every frame, which creates a second full set of frame buffers at markdown display size (`crates/warpui_core/src/image_cache.rs (457-511)`, `crates/warpui_core/src/image_cache.rs:666-772`).
- This is why the current surface is especially wasteful: markdown gets only a first-frame preview, but pays the memory cost of full animation decode plus resized-frame duplication.

## Proposed changes
- Add an explicit animated-image behavior to `warpui::elements::Image`, with two modes:
  - `FullAnimation` — current behavior for surfaces that intentionally animate.
  - `FirstFramePreview` — render animated sources as a static first-frame image.
- Keep `FullAnimation` as the default so existing animated callers continue to work unchanged. Markdown will opt into `FirstFramePreview` at the call site.
- Keep a single `ImageType` asset path in `crates/warpui_core/src/image_cache.rs`. Animated GIF/WebP assets will continue to decode into `AnimatedImage` in the asset cache, but `ImageCache::image` will materialize `FirstFramePreview` requests as a static first-frame render result instead of resizing every decoded animation frame.
- Thread the new behavior into `ImageCache::image` so the rendered-image cache chooses between a static preview and a full animation from the same underlying `ImageType` asset.
- Replace the current rendered-image cache key shape with a struct that includes, at minimum:
  - asset source hash
  - target bounds
  - animated-image behavior

  This avoids collisions where the same source and bounds are requested once as a full animation and once as a static preview.
- Update `crates/editor/src/render/element/image.rs` so markdown image blocks always construct `Image::new(asset_source).contain().first_frame_preview()` (or equivalent builder naming). No markdown storage, export, or copy behavior changes are needed.
- Leave explicit animated callers unchanged. `app/src/resource_center/section_views/changelog_section.rs:152` should continue to opt into animation, and the `warpui` animated-image example should still exercise the full-animation path.
- Do not implement a bounded frame-buffer animation system in this ticket. That is the browser / media-viewer style solution, but it is unnecessary for this markdown surface because markdown does not animate today. The simpler preview-only split matches the current product behavior and removes the blow-up at the source.
- This mirrors common approaches elsewhere:
  - browser/media stacks like Chromium separate decode from caching and leave room for smarter bounded frame caches
  - native image libraries often provide either a rolling frame buffer for true animation (for example Gifu) or a first-frame-only preview mode for list/thumbnail surfaces (for example Kingfisher)

## End-to-end flow
```mermaid
graph TD
  A[Markdown ![](...)] --> B[BufferBlockItem::Image]
  B --> C[LayoutTask::Image resolves AssetSource]
  C --> D[BlockItem::Image]
  D --> E[RenderableImage]
  E --> F[warpui::elements::Image with FirstFramePreview]
  F --> G[ImageCache preview path]
  G --> H[AssetCache.load_asset<ImageType>]
  H --> I[Decode AnimatedImage once]
  I --> J[ImageCache extracts and resizes only frame 0]
  J --> K[Paint as static markdown image]

  L[Changelog / explicit animated surface] --> M[Image.enable_animation_with_start_time]
  M --> N[ImageCache full-animation path]
  N --> O[AssetCache.load_asset<ImageType>]
  O --> P[AnimatedImage]
```

## Risks and mitigations
- **Risk: preview/full cache collisions.**
  - Mitigation: include animated-image behavior in the rendered-image cache key rather than relying on `AssetSource + bounds` only.
- **Risk: the asset cache still holds full animated frame data.**
  - Mitigation: acceptable for this version of the fix. The user-facing memory win comes from avoiding resized copies of every frame in `ImageCache`, and we add tests that preview requests still render static output while the asset cache keeps the full animated asset. If this is still too expensive in practice, a follow-up can teach `AssetCache` to cache raw bytes or accept decode parameters.
- **Risk: accidental regression for existing animated surfaces.**
  - Mitigation: keep `FullAnimation` as the default, make preview behavior opt-in at the markdown call site, and add tests that the explicit animated path still produces `Image::Animated`.
- **Risk: very large single-frame assets remain expensive.**
  - Mitigation: acceptable for this ticket because the current failure is multi-frame explosion. Follow-up work can add explicit decode limits or dimension guards if needed.
- **Risk: first-frame choice differs slightly from a browser’s composited “current frame” semantics for malformed files.**
  - Mitigation: the product spec only requires a stable first visible frame preview, not browser-perfect animation fidelity.

## Testing and validation
- Add `warpui_core` unit tests with small animated GIF and animated WebP fixtures:
  - preview policy returns `Image::Static`
  - full-animation policy still returns `Image::Animated`
  - preview policy does not regress static formats
- Add an image-cache test that preview and full-animation requests for the same source/bounds do not collide in the rendered-image cache.
- Add/extend markdown image tests only where needed to confirm markdown export and serialization still preserve the authored `![](...)` reference.
- Manually validate against the Petra repro markdown from `APP-3857`:
  - open the document
  - scroll through it
  - confirm the old multi-GB spike is gone
  - confirm animated image sources render as stable first-frame previews
- Run the usual repo validation before implementation is considered done:
  - `cargo fmt`
  - targeted Rust tests for the touched editor / warpui_core modules
  - wasm build check for this repo, since image/rendering code is shared across desktop and wasm-adjacent targets

## Follow-ups
- If product later wants real markdown animation, add a bounded frame-buffer animation path instead of reusing the eager all-frames asset.
- Add explicit decode limits / pixel-budget guards for pathological single-frame assets if the Petra fix reveals a second-order issue there.
- Consider reusing the preview-only animated-image behavior in any other list, thumbnail, or markdown-adjacent surfaces that currently do not need full animation.
