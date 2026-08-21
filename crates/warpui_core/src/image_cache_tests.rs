use std::borrow::Cow;
use std::rc::Rc;

use rust_embed::RustEmbed;

use super::*;
use crate::AssetProvider;
use crate::r#async::executor::{Background, Foreground};

#[derive(Clone, Copy, RustEmbed)]
#[folder = "test_data"]
pub struct Assets;

// Implement the AssetProvider trait here (required by App::new).
impl AssetProvider for Assets {
    fn get(&self, path: &str) -> Result<Cow<'_, [u8]>> {
        match path {
            "animated.webp" => Ok(Cow::Borrowed(include_bytes!("../test_data/animated.webp"))),
            "cache-test.svg" => Ok(Cow::Borrowed(
                br##"<svg width="20" height="10" viewBox="0 0 20 10" xmlns="http://www.w3.org/2000/svg"><rect width="20" height="10" fill="#ffffff"/></svg>"##,
            )),
            "fit-test.rgba" => Ok(Cow::Borrowed(
                b"warp-img:rgba:4:2:\xff\x00\x00\xff\xff\x00\x00\xff\x00\x00\xff\xff\x00\x00\xff\xff\x00\xff\x00\xff\x00\xff\x00\xff\xff\xff\xff\xff\xff\xff\xff\xff",
            )),
            "numbers-1000ms.gif" => Ok(Cow::Borrowed(include_bytes!(
                "../../warpui/examples/assets/numbers-1000ms.gif"
            ))),
            _ => <Assets as RustEmbed>::get(path)
                .map(|f| f.data)
                .ok_or_else(|| anyhow!("no asset exists at path {}", path)),
        }
    }
}

fn new_asset_cache() -> AssetCache {
    AssetCache::new(
        Box::new(Assets),
        ImageCache::new(),
        Foreground::test().into(),
        Background::default().into(),
    )
}

fn load_bundled_image(
    image_cache: &ImageCache,
    asset_cache: &AssetCache,
    path: &'static str,
    bounds: Vector2I,
    fit_type: FitType,
    animated_image_behavior: AnimatedImageBehavior,
) -> Rc<Image> {
    let image = image_cache.image(
        AssetSource::Bundled { path },
        bounds,
        fit_type,
        animated_image_behavior,
        CacheOption::BySize,
        None,
        asset_cache,
    );
    let AssetState::Loaded { data: image } = image else {
        panic!("Bundled asset should be available immediately!");
    };
    image
}

#[test]
fn test_passes_through_asset_cache_original() {
    let asset_cache = new_asset_cache();
    let image_cache = ImageCache::new();

    let source = AssetSource::Bundled { path: "local.png" };
    let image_asset: AssetState<ImageType> = asset_cache.load_asset(source.clone());
    let AssetState::Loaded { data: image } = image_asset else {
        panic!("Bundled asset should be available immediately!");
    };
    let ImageType::StaticBitmap { image } = image.as_ref() else {
        panic!("Expected static image but got dynamic image!");
    };
    let image_asset_weak = Arc::downgrade(image);

    let bounds = Vector2I::new(1024, 1024);
    let image = image_cache.image(
        source,
        bounds,
        FitType::Cover,
        AnimatedImageBehavior::FullAnimation,
        CacheOption::Original,
        None,
        &asset_cache,
    );
    let AssetState::Loaded { data: image } = image else {
        panic!("Bundled asset should be available immediately!");
    };
    let Image::Static(image) = image.as_ref() else {
        panic!("Expected static image but got dynamic image!");
    };

    // Assert that the image returned from the image cache and the asset stored
    // in the asset cache point to the same underlying data (i.e.: there were
    // no copies made).
    assert!(image_asset_weak.ptr_eq(&Arc::downgrade(image)));
}

#[test]
fn test_passes_through_asset_cache_original_when_target_size_matches_source_size() {
    let asset_cache = new_asset_cache();
    let image_cache = ImageCache::new();

    let source = AssetSource::Bundled { path: "local.png" };
    let image_asset: AssetState<ImageType> = asset_cache.load_asset(source.clone());
    let AssetState::Loaded { data: image } = image_asset else {
        panic!("Bundled asset should be available immediately!");
    };
    let ImageType::StaticBitmap { image } = image.as_ref() else {
        panic!("Expected static image but got dynamic image!");
    };
    let image_asset_weak = Arc::downgrade(image);

    // Load the image with `CacheOption::BySize` but use the source asset's
    // size as the bounds.
    let bounds = image.size();
    let image = image_cache.image(
        source,
        bounds,
        FitType::Cover,
        AnimatedImageBehavior::FullAnimation,
        CacheOption::BySize,
        None,
        &asset_cache,
    );
    let AssetState::Loaded { data: image } = image else {
        panic!("Bundled asset should be available immediately!");
    };
    let Image::Static(image) = image.as_ref() else {
        panic!("Expected static image but got dynamic image!");
    };

    // Assert that the image returned from the image cache and the asset stored
    // in the asset cache point to the same underlying data (i.e.: there were
    // no copies made).
    assert!(image_asset_weak.ptr_eq(&Arc::downgrade(image)));
}

#[test]
fn test_caches_svg_rendered_at_intrinsic_size() {
    let asset_cache = new_asset_cache();
    let image_cache = ImageCache::new();
    let bounds = Vector2I::new(20, 10);

    let image = load_bundled_image(
        &image_cache,
        &asset_cache,
        "cache-test.svg",
        bounds,
        FitType::Contain,
        AnimatedImageBehavior::FullAnimation,
    );
    let image_again = load_bundled_image(
        &image_cache,
        &asset_cache,
        "cache-test.svg",
        bounds,
        FitType::Contain,
        AnimatedImageBehavior::FullAnimation,
    );

    assert!(Rc::ptr_eq(&image, &image_again));
}

#[test]
fn cloned_image_cache_evicts_shared_rendered_images() {
    let image_cache = ImageCache::new();
    let evicting_image_cache = image_cache.clone();
    let asset_cache = AssetCache::new(
        Box::new(Assets),
        image_cache.clone(),
        Foreground::test().into(),
        Background::default().into(),
    );
    let source = AssetSource::Bundled {
        path: "cache-test.svg",
    };
    let bounds = Vector2I::new(20, 10);
    let image = load_bundled_image(
        &image_cache,
        &asset_cache,
        "cache-test.svg",
        bounds,
        FitType::Contain,
        AnimatedImageBehavior::FullAnimation,
    );

    evicting_image_cache.evict_image(&source);

    let image_after_eviction = load_bundled_image(
        &image_cache,
        &asset_cache,
        "cache-test.svg",
        bounds,
        FitType::Contain,
        AnimatedImageBehavior::FullAnimation,
    );
    assert!(!Rc::ptr_eq(&image, &image_after_eviction));
}

#[test]
fn test_different_fit_types_do_not_collide_in_rendered_image_cache() {
    let asset_cache = new_asset_cache();
    let image_cache = ImageCache::new();
    let bounds = Vector2I::new(8, 8);

    let cover = load_bundled_image(
        &image_cache,
        &asset_cache,
        "fit-test.rgba",
        bounds,
        FitType::Cover,
        AnimatedImageBehavior::FullAnimation,
    );
    let stretch = load_bundled_image(
        &image_cache,
        &asset_cache,
        "fit-test.rgba",
        bounds,
        FitType::Stretch,
        AnimatedImageBehavior::FullAnimation,
    );
    assert!(!Rc::ptr_eq(&cover, &stretch));
    let Image::Static(cover) = cover.as_ref() else {
        panic!("Expected static image but got animated image!");
    };
    let Image::Static(stretch) = stretch.as_ref() else {
        panic!("Expected static image but got animated image!");
    };
    assert_eq!(cover.img.dimensions(), (8, 8));
    assert_eq!(stretch.img.dimensions(), (8, 8));
    assert_ne!(cover.rgba_bytes(), stretch.rgba_bytes());
}

#[test]
fn test_respects_max_dimensions_for_cacheoption_original() {
    let asset_cache = new_asset_cache();
    let image_cache = ImageCache::new();

    // We pass a very small value for bounds, which should get ignored due to
    // use of `CacheOption::Original`.
    let bounds = Vector2I::new(10, 10);

    let image = image_cache.image(
        AssetSource::Bundled { path: "local.png" },
        bounds,
        FitType::Cover,
        AnimatedImageBehavior::FullAnimation,
        CacheOption::Original,
        None,
        &asset_cache,
    );
    let AssetState::Loaded { data: image } = image else {
        panic!("Bundled asset should be available immediately!");
    };

    let Image::Static(image) = image.as_ref() else {
        panic!("Expected static image but got dynamic image!");
    };
    // Assert that the image, without resizing or a max dimension, matches our expectations.
    assert_eq!(image.img.dimensions(), (1024, 1024));

    let image = image_cache.image(
        AssetSource::Bundled { path: "local.png" },
        bounds,
        FitType::Cover,
        AnimatedImageBehavior::FullAnimation,
        CacheOption::Original,
        Some(512),
        &asset_cache,
    );
    let AssetState::Loaded { data: image } = image else {
        panic!("Bundled asset should be available immediately!");
    };

    let Image::Static(image) = image.as_ref() else {
        panic!("Expected static image but got dynamic image!");
    };
    // Assert that, when we specify a max dimension of 512, the image is resized accordingly.
    assert_eq!(image.img.dimensions(), (512, 512));
}

#[test]
fn test_first_frame_preview_returns_static_for_animated_gif() {
    let asset_cache = new_asset_cache();
    let image_cache = ImageCache::new();

    let image = load_bundled_image(
        &image_cache,
        &asset_cache,
        "numbers-1000ms.gif",
        Vector2I::new(16, 16),
        FitType::Contain,
        AnimatedImageBehavior::FirstFramePreview,
    );

    let Image::Static(image) = image.as_ref() else {
        panic!("Expected static image but got animated image!");
    };
    assert_eq!(image.img.dimensions(), (16, 16));
}

#[test]
fn test_first_frame_preview_keeps_full_animation_in_asset_cache() {
    let asset_cache = new_asset_cache();
    let image_cache = ImageCache::new();

    for path in ["numbers-1000ms.gif", "animated.webp"] {
        let image = load_bundled_image(
            &image_cache,
            &asset_cache,
            path,
            Vector2I::new(16, 16),
            FitType::Contain,
            AnimatedImageBehavior::FirstFramePreview,
        );

        assert!(matches!(image.as_ref(), Image::Static(_)));

        let asset: AssetState<ImageType> = asset_cache.load_asset(AssetSource::Bundled { path });
        let AssetState::Loaded { data } = asset else {
            panic!("Animated asset should be available immediately!");
        };
        assert!(matches!(data.as_ref(), ImageType::AnimatedBitmap { .. }));
    }
}

#[test]
fn test_first_frame_preview_returns_static_for_animated_webp() {
    let asset_cache = new_asset_cache();
    let image_cache = ImageCache::new();

    let image = load_bundled_image(
        &image_cache,
        &asset_cache,
        "animated.webp",
        Vector2I::new(16, 16),
        FitType::Contain,
        AnimatedImageBehavior::FirstFramePreview,
    );

    let Image::Static(image) = image.as_ref() else {
        panic!("Expected static image but got animated image!");
    };
    assert_eq!(image.img.dimensions(), (16, 16));
}

#[test]
fn test_full_animation_still_returns_animated_for_gif_and_webp() {
    let asset_cache = new_asset_cache();
    let image_cache = ImageCache::new();

    for path in ["numbers-1000ms.gif", "animated.webp"] {
        let image = load_bundled_image(
            &image_cache,
            &asset_cache,
            path,
            Vector2I::new(16, 16),
            FitType::Contain,
            AnimatedImageBehavior::FullAnimation,
        );

        let Image::Animated(image) = image.as_ref() else {
            panic!("Expected animated image but got static image!");
        };
        assert!(image.frames.len() > 1);
    }
}

#[test]
fn test_first_frame_preview_does_not_regress_static_formats() {
    let asset_cache = new_asset_cache();
    let image_cache = ImageCache::new();

    let image = load_bundled_image(
        &image_cache,
        &asset_cache,
        "local.png",
        Vector2I::new(16, 16),
        FitType::Contain,
        AnimatedImageBehavior::FirstFramePreview,
    );

    let Image::Static(image) = image.as_ref() else {
        panic!("Expected static image but got animated image!");
    };
    assert_eq!(image.img.dimensions(), (16, 16));
}

#[test]
fn test_preview_and_full_animation_requests_do_not_collide_in_rendered_image_cache() {
    let asset_cache = new_asset_cache();
    let image_cache = ImageCache::new();
    let bounds = Vector2I::new(16, 16);

    let preview = load_bundled_image(
        &image_cache,
        &asset_cache,
        "numbers-1000ms.gif",
        bounds,
        FitType::Contain,
        AnimatedImageBehavior::FirstFramePreview,
    );
    let full = load_bundled_image(
        &image_cache,
        &asset_cache,
        "numbers-1000ms.gif",
        bounds,
        FitType::Contain,
        AnimatedImageBehavior::FullAnimation,
    );
    let preview_again = load_bundled_image(
        &image_cache,
        &asset_cache,
        "numbers-1000ms.gif",
        bounds,
        FitType::Contain,
        AnimatedImageBehavior::FirstFramePreview,
    );
    let full_again = load_bundled_image(
        &image_cache,
        &asset_cache,
        "numbers-1000ms.gif",
        bounds,
        FitType::Contain,
        AnimatedImageBehavior::FullAnimation,
    );

    assert!(matches!(preview.as_ref(), Image::Static(_)));
    assert!(matches!(full.as_ref(), Image::Animated(_)));
    assert!(Rc::ptr_eq(&preview, &preview_again));
    assert!(Rc::ptr_eq(&full, &full_again));
    assert!(!Rc::ptr_eq(&preview, &full));
}

#[test]
fn test_svg_text_rasterizes_with_loaded_system_fonts() {
    let image_type = ImageType::try_from_bytes(
        br##"<svg width="160" height="40" viewBox="0 0 160 40" xmlns="http://www.w3.org/2000/svg">
  <text x="10" y="24" font-size="20" fill="#000000">Warp</text>
</svg>
"##,
    )
    .expect("SVG should parse");
    let ImageType::Svg { svg } = &image_type else {
        panic!("Expected SVG image type");
    };
    let font_family = svg
        .fontdb()
        .faces()
        .flat_map(|face| face.families.iter().map(|(family, _)| family.as_str()))
        .find(|family| {
            matches!(
                *family,
                "Arial"
                    | "Helvetica"
                    | "Inter"
                    | "DejaVu Sans"
                    | "Liberation Sans"
                    | "Noto Sans"
                    | "Cantarell"
                    | "Segoe UI"
            )
        })
        .or_else(|| {
            svg.fontdb()
                .faces()
                .find_map(|face| face.families.first().map(|(family, _)| family.as_str()))
        })
        .expect("System fonts should be loaded");

    let svg = format!(
        "<svg width=\"160\" height=\"40\" viewBox=\"0 0 160 40\" xmlns=\"http://www.w3.org/2000/svg\">\
  <text x=\"10\" y=\"24\" font-family=\"{font_family}\" font-size=\"20\" fill=\"#000000\">Warp</text>\
</svg>"
    );

    let image_type =
        ImageType::try_from_bytes(svg.as_bytes()).expect("SVG with installed font should parse");
    let image = image_type
        .to_image(
            Vector2I::new(160, 40),
            FitType::Contain,
            true,
            AnimatedImageBehavior::FullAnimation,
        )
        .expect("SVG should rasterize");
    let Image::Static(image) = image else {
        panic!("Expected static image");
    };

    assert!(
        image
            .rgba_bytes()
            .chunks_exact(4)
            .any(|pixel| pixel[3] != 0)
    );
}

#[test]
fn test_svg_text_rasterizes_with_bundled_sans_serif_fallback() {
    let image_type = ImageType::try_from_bytes(
        br##"<svg width="160" height="40" viewBox="0 0 160 40" xmlns="http://www.w3.org/2000/svg">
  <text x="10" y="24" font-family="sans-serif" font-size="20" fill="#000000">Warp</text>
</svg>
"##,
    )
    .expect("SVG should parse");
    let ImageType::Svg { svg } = &image_type else {
        panic!("Expected SVG image type");
    };

    assert!(svg.fontdb().faces().any(|face| {
        face.families
            .iter()
            .any(|(family, _)| family.as_str() == "Roboto")
    }));

    let image = image_type
        .to_image(
            Vector2I::new(160, 40),
            FitType::Contain,
            true,
            AnimatedImageBehavior::FullAnimation,
        )
        .expect("SVG should rasterize");
    let Image::Static(image) = image else {
        panic!("Expected static image");
    };

    assert!(
        image
            .rgba_bytes()
            .chunks_exact(4)
            .any(|pixel| pixel[3] != 0)
    );
}

#[test]
fn test_evict_image_drops_arc_for_resized_bysize() {
    let asset_cache = new_asset_cache();
    let image_cache = ImageCache::new();
    let source = AssetSource::Bundled { path: "local.png" };

    // Request the image at a smaller size than its 1024x1024 source, which forces a resize
    // and allocates a fresh Arc<StaticImage> not shared with AssetCache.
    let bounds = Vector2I::new(64, 64);
    let weak = {
        let image = image_cache.image(
            source.clone(),
            bounds,
            FitType::Cover,
            AnimatedImageBehavior::FullAnimation,
            CacheOption::BySize,
            None,
            &asset_cache,
        );
        let AssetState::Loaded { data: image } = image else {
            panic!("Bundled asset should be available immediately!");
        };
        let Image::Static(arc) = image.as_ref() else {
            panic!("Expected static image!");
        };
        Arc::downgrade(arc)
        // The local Rc<Image> clone is dropped here; only ImageCache holds the entry now.
    };

    assert_eq!(
        weak.strong_count(),
        1,
        "ImageCache should be the sole strong holder after the caller drops its Rc clone"
    );

    // Evicting from ImageCache should make the Arc releasable by TextureCache.
    image_cache.evict_image(&source);
    assert_eq!(
        weak.strong_count(),
        0,
        "After evict_image, the resized Arc should have no strong holders (cascade invariant)"
    );
}

#[test]
fn test_evict_size_drops_arc_only_for_targeted_entry() {
    let asset_cache = new_asset_cache();
    let image_cache = ImageCache::new();
    let source = AssetSource::Bundled { path: "local.png" };

    // Cache the same asset at two distinct sizes.
    let small_bounds = Vector2I::new(32, 32);
    let large_bounds = Vector2I::new(256, 256);

    let weak_small = {
        let image = image_cache.image(
            source.clone(),
            small_bounds,
            FitType::Cover,
            AnimatedImageBehavior::FullAnimation,
            CacheOption::BySize,
            None,
            &asset_cache,
        );
        let AssetState::Loaded { data: image } = image else {
            panic!("Bundled asset should be available immediately!");
        };
        let Image::Static(arc) = image.as_ref() else {
            panic!("Expected static image!");
        };
        Arc::downgrade(arc)
    };

    let weak_large = {
        let image = image_cache.image(
            source.clone(),
            large_bounds,
            FitType::Cover,
            AnimatedImageBehavior::FullAnimation,
            CacheOption::BySize,
            None,
            &asset_cache,
        );
        let AssetState::Loaded { data: image } = image else {
            panic!("Bundled asset should be available immediately!");
        };
        let Image::Static(arc) = image.as_ref() else {
            panic!("Expected static image!");
        };
        Arc::downgrade(arc)
    };

    assert_eq!(weak_small.strong_count(), 1);
    assert_eq!(weak_large.strong_count(), 1);

    // Evict only the small size entry.
    image_cache.evict_size(
        &source,
        small_bounds,
        FitType::Cover,
        AnimatedImageBehavior::FullAnimation,
    );

    assert_eq!(
        weak_small.strong_count(),
        0,
        "Small size Arc should have no strong holders after evict_size"
    );
    assert_eq!(
        weak_large.strong_count(),
        1,
        "Large size Arc should remain alive; only the small size was evicted"
    );
}

#[test]
fn test_image_evicts_lru_size_variant_when_cache_exceeds_capacity() {
    let asset_cache = new_asset_cache();
    let image_cache = ImageCache::new();
    let source = AssetSource::Bundled { path: "local.png" };

    let mut s = DefaultHasher::new();
    source.hash(&mut s);
    let cache_key = s.finish();

    // Simulate a continuous window-resize drag that sweeps through more
    // distinct pixel sizes than MAX_CACHED_SIZES_PER_ASSET.
    let num_sizes = MAX_CACHED_SIZES_PER_ASSET + 4;
    let mut weaks = Vec::new();
    for size in 1..=num_sizes {
        let bounds = Vector2I::new(size as i32 * 10, size as i32 * 10);
        let image = image_cache.image(
            source.clone(),
            bounds,
            FitType::Cover,
            AnimatedImageBehavior::FullAnimation,
            CacheOption::BySize,
            None,
            &asset_cache,
        );
        let AssetState::Loaded { data: image } = image else {
            panic!("Bundled asset should be available immediately!");
        };
        let Image::Static(arc) = image.as_ref() else {
            panic!("Expected static image!");
        };
        weaks.push(Arc::downgrade(arc));
        // Drop the local Rc<Image> clone; only ImageCache should hold a strong
        // reference to entries once they age out.
    }

    let retained = image_cache
        .images
        .read()
        .get(&cache_key)
        .map(|inner_map| inner_map.len())
        .unwrap_or(0);
    assert_eq!(
        retained, MAX_CACHED_SIZES_PER_ASSET,
        "the per-asset cache should never grow past MAX_CACHED_SIZES_PER_ASSET, regardless of how \
         many distinct sizes have been requested over the asset's lifetime"
    );

    // The earliest-requested (least-recently-used) sizes should have been
    // evicted, releasing their Arc<StaticImage>, while the most recently
    // requested sizes remain retained.
    let evicted_count = weaks.len() - MAX_CACHED_SIZES_PER_ASSET;
    for weak in &weaks[..evicted_count] {
        assert_eq!(
            weak.strong_count(),
            0,
            "stale size variants evicted by the LRU pass should have no strong holders left"
        );
    }
    for weak in &weaks[evicted_count..] {
        assert_eq!(
            weak.strong_count(),
            1,
            "recently requested size variants should remain retained by ImageCache"
        );
    }
}

#[test]
fn test_image_does_not_evict_active_size_variants_at_capacity() {
    let asset_cache = new_asset_cache();
    let image_cache = ImageCache::new();
    let source = AssetSource::Bundled { path: "local.png" };

    // Simulate an asset legitimately rendered at exactly
    // MAX_CACHED_SIZES_PER_ASSET distinct fixed sizes at once (e.g. an icon
    // reused at several sizes across the UI).
    let bounds: Vec<Vector2I> = (1..=MAX_CACHED_SIZES_PER_ASSET)
        .map(|size| Vector2I::new(size as i32 * 10, size as i32 * 10))
        .collect();

    let mut originals = Vec::new();
    for bound in &bounds {
        let image = image_cache.image(
            source.clone(),
            *bound,
            FitType::Cover,
            AnimatedImageBehavior::FullAnimation,
            CacheOption::BySize,
            None,
            &asset_cache,
        );
        let AssetState::Loaded { data: image } = image else {
            panic!("Bundled asset should be available immediately!");
        };
        originals.push(image);
    }

    // Simulate several more paint frames re-requesting the same steady-state
    // set of sizes. None should ever be evicted or recomputed: every request
    // is a hit, so the LRU pass never observes a miss that would trigger
    // eviction, and the app doesn't pay a decode-per-frame treadmill cost.
    for _ in 0..5 {
        for (bound, original) in bounds.iter().zip(originals.iter()) {
            let image = image_cache.image(
                source.clone(),
                *bound,
                FitType::Cover,
                AnimatedImageBehavior::FullAnimation,
                CacheOption::BySize,
                None,
                &asset_cache,
            );
            let AssetState::Loaded { data: image } = image else {
                panic!("Bundled asset should be available immediately!");
            };
            assert!(
                Rc::ptr_eq(&image, original),
                "a steady-state size variant within capacity should be served from cache, not \
                 recomputed or evicted"
            );
        }
    }

    // Every round above touched bounds[0] first and bounds[last] last, so
    // bounds[0] is currently the least-recently-used entry and bounds[1] is
    // the next-oldest. Capture weak handles for both, then drop our strong
    // Rc<Image> clones so ImageCache is the sole holder (matching the
    // eviction tests above), before distinguishing them by eviction.
    let weak_oldest = {
        let Image::Static(arc) = originals[0].as_ref() else {
            panic!("Expected static image!");
        };
        Arc::downgrade(arc)
    };
    let weak_next_oldest = {
        let Image::Static(arc) = originals[1].as_ref() else {
            panic!("Expected static image!");
        };
        Arc::downgrade(arc)
    };
    drop(originals);

    // Re-hit the oldest entry once more. This should refresh its recency via
    // the hit path in `image()`, so it's no longer the least-recently-used
    // entry even though it was the first one touched in every round above.
    let refreshed = image_cache.image(
        source.clone(),
        bounds[0],
        FitType::Cover,
        AnimatedImageBehavior::FullAnimation,
        CacheOption::BySize,
        None,
        &asset_cache,
    );
    let AssetState::Loaded { data: refreshed } = refreshed else {
        panic!("Bundled asset should be available immediately!");
    };
    drop(refreshed);

    // Request a brand-new size, pushing the per-asset cache over capacity and
    // forcing eviction of whichever entry is truly least-recently-used.
    let new_bounds = Vector2I::new(
        (MAX_CACHED_SIZES_PER_ASSET as i32 + 1) * 10,
        (MAX_CACHED_SIZES_PER_ASSET as i32 + 1) * 10,
    );
    let new_image = image_cache.image(
        source.clone(),
        new_bounds,
        FitType::Cover,
        AnimatedImageBehavior::FullAnimation,
        CacheOption::BySize,
        None,
        &asset_cache,
    );
    let AssetState::Loaded { data: new_image } = new_image else {
        panic!("Bundled asset should be available immediately!");
    };
    drop(new_image);

    // If cache hits didn't refresh LRU recency, bounds[0] (never re-hit after
    // the loop, in that hypothetical) would have been evicted instead of
    // bounds[1], so these assertions genuinely depend on the hit-path bump.
    assert_eq!(
        weak_oldest.strong_count(),
        1,
        "the entry just re-hit should survive eviction, proving cache hits refresh LRU recency"
    );
    assert_eq!(
        weak_next_oldest.strong_count(),
        0,
        "the entry that wasn't re-hit should be evicted as the true least-recently-used entry"
    );
}

#[test]
fn test_svg_image_size_returns_intrinsic_dimensions() {
    let image_type = ImageType::try_from_bytes(
        br##"<svg width="160" height="40" viewBox="0 0 160 40" xmlns="http://www.w3.org/2000/svg"></svg>"##,
    )
    .expect("SVG should parse");

    assert_eq!(image_type.image_size(), Some(Vector2I::new(160, 40)));
}

#[test]
fn test_respects_max_dimensions_for_cacheoption_bysize() {
    let asset_cache = new_asset_cache();
    let image_cache = ImageCache::new();

    let bounds = Vector2I::new(768, 768);

    let image = image_cache.image(
        AssetSource::Bundled { path: "local.png" },
        bounds,
        FitType::Cover,
        AnimatedImageBehavior::FullAnimation,
        CacheOption::BySize,
        None,
        &asset_cache,
    );
    let AssetState::Loaded { data: image } = image else {
        panic!("Bundled asset should be available immediately!");
    };

    let Image::Static(image) = image.as_ref() else {
        panic!("Expected static image but got dynamic image!");
    };
    // Assert that the image gets resized to match the provided bounds.
    assert_eq!(image.img.dimensions(), (768, 768));

    let image = image_cache.image(
        AssetSource::Bundled { path: "local.png" },
        bounds,
        FitType::Cover,
        AnimatedImageBehavior::FullAnimation,
        CacheOption::BySize,
        Some(512),
        &asset_cache,
    );
    let AssetState::Loaded { data: image } = image else {
        panic!("Bundled asset should be available immediately!");
    };

    let Image::Static(image) = image.as_ref() else {
        panic!("Expected static image but got dynamic image!");
    };
    // Assert that, when we specify a max dimension of 512, the image is resized accordingly.
    assert_eq!(image.img.dimensions(), (512, 512));
}
