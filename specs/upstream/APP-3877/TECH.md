# APP-3877: Tech Spec — Prep 1: TextureCache Cascade Eviction

## Context

This is the first preparatory changeset for the image cache debounce strategy described in the broader APP-3877 plan. It establishes the per-size eviction API on `ImageCache` and verifies that GPU memory is freed correctly when an `ImageCache` entry is dropped.

### Relevant files
- `crates/warpui_core/src/image_cache.rs:799` — `ImageCache`, stores `RwLock<HashMap<u64, HashMap<RenderedImageCacheKey, Rc<Image>>>>`. Whole-asset eviction exists (`evict_image`); per-size eviction does not yet exist.
- `crates/warpui_core/src/rendering/texture_cache.rs:26` — `TextureCache<T>`, stores a `Vec<TextureInfo<T>>` where each entry holds `Weak<StaticImage>` and a `last_accessed_frame` counter.
- `crates/warpui_core/src/elements/image.rs:302` — `Image::paint()` calls `ImageCache::image()`, receives `Rc<Image>`, immediately unwraps the inner `Arc<StaticImage>`, and pushes it to the `Scene`. The `Rc<Image>` is never stored beyond `paint()`.
- `crates/warpui_core/src/scene.rs:109` — `Scene::Image { asset: Arc<StaticImage> }` — the scene stores a strong reference during the current frame only.
- `crates/warpui/src/rendering/wgpu/renderer/image.rs:53` — `Pipeline::texture_cache: TextureCache<TextureInfo>`, populated from `scene.images[*].asset` each frame.

### How the cascade currently works

`TextureCache::end_frame()` already evicts entries in two cases:
1. `asset.strong_count() == 0` — the backing `Arc<StaticImage>` has no remaining strong holders; the asset has been dropped entirely.
2. `frame_index - last_accessed_frame >= MAX_UNUSED_FRAMES` — the texture has not been rendered in 10+ frames, regardless of whether the asset is still alive.

For `CacheOption::BySize` resized images, `ImageCache` allocates a fresh `Arc<StaticImage>` per `(asset, size)` entry that is shared with no other subsystem. After `Image::paint()` returns and the scene for that frame is discarded, the sole persistent strong holder is `ImageCache`. Dropping that entry — via either `evict_image` or the new `evict_size` — reduces the strong count to zero, which causes `end_frame()` to evict the corresponding GPU texture on the next frame.

No changes to `TextureCache` are required. The `Weak<StaticImage>` mechanism already provides the correct cascade behavior.

## Proposed changes

### 1. Add `evict_size` to `ImageCache`

Add a private method that removes a single `(cache_key, RenderedImageCacheKey)` entry, cleaning up the outer map if the inner map becomes empty. The main changeset will call this from inside `image()` during its lazy eviction pass.

```rust
fn evict_size(&self, cache_key: u64, rendered_key: RenderedImageCacheKey) {
    let mut cache = self.images.write();
    if let Some(inner_map) = cache.get_mut(&cache_key) {
        inner_map.remove(&rendered_key);
        if inner_map.is_empty() {
            cache.remove(&cache_key);
        }
    }
}
```

### 2. Add a test-only `StaticImage` constructor

`StaticImage` currently has no public constructor. `TextureCache` tests need `Arc<StaticImage>` to exercise `get_or_insert_by_asset`. Add a `pub(crate) mod test_utils` under `#[cfg(test)]` in `image_cache.rs` with a `make_static_image(width, height)` helper.

### 3. Add tests

**`image_cache_tests.rs`**
- `test_evict_image_drops_arc_for_resized_bysize`: load a `BySize` image at a size different from the source (forcing a resize and a fresh `Arc`), capture a `Weak<StaticImage>`, drop the local `Rc<Image>` clone, assert `strong_count == 1` (only `ImageCache` holds it), call `evict_image`, assert `strong_count == 0`.
- `test_evict_size_drops_arc_for_single_entry`: same setup with two different sizes; call `evict_size` for one entry and assert that only the targeted size's `Arc` is released while the other remains alive.

**`texture_cache_tests.rs`** (new file, wired in via `#[path]`)
- `test_end_frame_evicts_when_asset_dropped`: insert an asset, drop the `Arc`, call `end_frame`, verify the entry count drops to zero.
- `test_end_frame_evicts_after_max_unused_frames`: insert an asset, keep the `Arc` alive, advance `frame_index` past `MAX_UNUSED_FRAMES` by calling `end_frame` repeatedly without re-accessing the texture, verify the entry is evicted.
- `test_end_frame_retains_recently_used_entry`: insert, re-access each frame, advance past `MAX_UNUSED_FRAMES`, verify the texture is retained because it was used.

## Testing and validation

All new behavior is verified by unit tests above. No rendering hardware is required — `TextureCache<T>` is generic and tests use `T = ()`. Run with:

```
cargo nextest run -p warpui_core
```
