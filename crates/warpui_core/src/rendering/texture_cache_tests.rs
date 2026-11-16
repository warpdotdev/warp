use super::*;
use crate::image_cache::test_utils::make_static_image;

#[test]
fn test_end_frame_evicts_when_asset_dropped() {
    let mut cache = TextureCache::<()>::new();
    let asset = make_static_image(4, 4);
    let weak = Arc::downgrade(&asset);

    cache.get_or_insert_by_asset(&asset, |_| ());

    // Dropping the only strong reference makes strong_count == 0.
    drop(asset);
    assert_eq!(weak.strong_count(), 0);

    // end_frame should detect strong_count == 0 and evict the entry.
    cache.end_frame();
    assert_eq!(
        cache.textures.len(),
        0,
        "TextureCache should evict entries whose backing asset has been dropped (cascade invariant)"
    );
}

#[test]
fn test_end_frame_retains_asset_in_use() {
    let mut cache = TextureCache::<()>::new();
    let asset = make_static_image(4, 4);

    cache.get_or_insert_by_asset(&asset, |_| ());

    // The Arc is still alive; the texture should be retained.
    cache.end_frame();
    assert_eq!(
        cache.textures.len(),
        1,
        "TextureCache should retain entries whose backing asset is still alive"
    );

    drop(asset);
}

#[test]
fn test_end_frame_evicts_after_max_unused_frames() {
    let mut cache = TextureCache::<()>::new();
    let asset = make_static_image(4, 4);

    cache.get_or_insert_by_asset(&asset, |_| ());

    // Advance past MAX_UNUSED_FRAMES without re-accessing the texture.
    // The asset is still alive (strong_count > 0), but the texture goes stale.
    for _ in 0..TextureCache::<()>::MAX_UNUSED_FRAMES {
        cache.end_frame();
        assert_eq!(
            cache.textures.len(),
            1,
            "Texture should still be present before MAX_UNUSED_FRAMES is exceeded"
        );
    }

    // One more end_frame tips it over the threshold.
    cache.end_frame();
    assert_eq!(
        cache.textures.len(),
        0,
        "TextureCache should evict stale entries after MAX_UNUSED_FRAMES unused frames"
    );

    drop(asset);
}

#[test]
fn test_end_frame_retains_recently_used_entry() {
    let mut cache = TextureCache::<()>::new();
    let asset = make_static_image(4, 4);

    cache.get_or_insert_by_asset(&asset, |_| ());

    // Re-access the texture each frame to keep it fresh.
    for _ in 0..=TextureCache::<()>::MAX_UNUSED_FRAMES {
        cache.get_or_insert_by_asset(&asset, |_| ());
        cache.end_frame();
    }

    assert_eq!(
        cache.textures.len(),
        1,
        "Texture accessed every frame should never be evicted by the frame-count check"
    );

    drop(asset);
}
