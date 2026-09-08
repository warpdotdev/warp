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

#[test]
fn animated_image_get_current_frame_advances_with_elapsed_time() {
    let asset_cache = new_asset_cache();
    let image_cache = ImageCache::new();

    // Load an animated GIF with FullAnimation behavior to get an AnimatedImage
    let image = load_bundled_image(
        &image_cache,
        &asset_cache,
        "numbers-1000ms.gif",
        Vector2I::new(16, 16),
        FitType::Contain,
        AnimatedImageBehavior::FullAnimation,
    );

    let Image::Animated(animated) = image.as_ref() else {
        panic!("Expected animated image but got static image!");
    };

    // The numbers-1000ms.gif has multiple frames with various delays
    assert!(
        animated.frames.len() >= 2,
        "Expected at least 2 frames for testing"
    );
    assert!(animated.duration > 0, "Expected positive total duration");

    let (_frame_0, remaining_0) = animated
        .get_current_frame(0)
        .expect("Should get frame at elapsed 0ms");
    let (_frame_100, remaining_100) = animated
        .get_current_frame(100)
        .expect("Should get frame at elapsed 100ms");
    let (_frame_500, remaining_500) = animated
        .get_current_frame(500)
        .expect("Should get frame at elapsed 500ms");

    assert_ne!(
        remaining_0, remaining_100,
        "Remaining delay should differ at different elapsed times"
    );
    assert_ne!(
        remaining_100, remaining_500,
        "Remaining delay should differ at different elapsed times"
    );
}

#[test]
fn animated_image_get_current_frame_wraps_at_duration() {
    let asset_cache = new_asset_cache();
    let image_cache = ImageCache::new();

    let image = load_bundled_image(
        &image_cache,
        &asset_cache,
        "numbers-1000ms.gif",
        Vector2I::new(16, 16),
        FitType::Contain,
        AnimatedImageBehavior::FullAnimation,
    );

    let Image::Animated(animated) = image.as_ref() else {
        panic!("Expected animated image but got static image!");
    };

    let duration = animated.duration;

    let (frame_start, _) = animated
        .get_current_frame(0)
        .expect("Should get frame at start");
    let (frame_end, _) = animated
        .get_current_frame(duration - 1)
        .expect("Should get frame near end");

    // After wrapping (elapsed >= duration), should return to start of animation
    let (frame_wrapped_start, _) = animated
        .get_current_frame(duration)
        .expect("Should get frame after one complete cycle");
    let (frame_wrapped_end, _) = animated
        .get_current_frame(duration * 2 - 1)
        .expect("Should get frame in second cycle");

    assert!(
        Arc::ptr_eq(&frame_start, &frame_wrapped_start),
        "Frame at start should equal frame at wrapped start"
    );

    assert!(
        Arc::ptr_eq(&frame_end, &frame_wrapped_end),
        "Frame at cycle end should equal frame at next cycle end"
    );
}

#[test]
fn static_image_not_affected_by_animation_behavior_change() {
    let asset_cache = new_asset_cache();
    let image_cache = ImageCache::new();

    // Load a static PNG with FullAnimation behavior (should remain static)
    let static_full_animation = load_bundled_image(
        &image_cache,
        &asset_cache,
        "local.png",
        Vector2I::new(512, 512),
        FitType::Cover,
        AnimatedImageBehavior::FullAnimation,
    );

    // Load the same static PNG with FirstFramePreview behavior (should remain static)
    let static_preview = load_bundled_image(
        &image_cache,
        &asset_cache,
        "local.png",
        Vector2I::new(512, 512),
        FitType::Cover,
        AnimatedImageBehavior::FirstFramePreview,
    );

    // Both should be static images
    assert!(matches!(static_full_animation.as_ref(), Image::Static(_)));
    assert!(matches!(static_preview.as_ref(), Image::Static(_)));

    // Extract and verify they're the same image data
    let Image::Static(full_static) = static_full_animation.as_ref() else {
        unreachable!();
    };
    let Image::Static(preview_static) = static_preview.as_ref() else {
        unreachable!();
    };

    // Both should have the same dimensions
    assert_eq!(
        full_static.img.dimensions(),
        preview_static.img.dimensions()
    );
}

#[test]
fn animated_gif_shows_first_frame_preview_when_requested() {
    let asset_cache = new_asset_cache();
    let image_cache = ImageCache::new();

    // Request FirstFramePreview behavior for an animated GIF
    let preview = load_bundled_image(
        &image_cache,
        &asset_cache,
        "numbers-1000ms.gif",
        Vector2I::new(16, 16),
        FitType::Contain,
        AnimatedImageBehavior::FirstFramePreview,
    );

    // Should get a static image (the first frame)
    let Image::Static(_) = preview.as_ref() else {
        panic!("Expected static image for FirstFramePreview");
    };

    // But the underlying asset in asset_cache should still be the full AnimatedBitmap
    let asset: AssetState<ImageType> = asset_cache.load_asset(AssetSource::Bundled {
        path: "numbers-1000ms.gif",
    });
    let AssetState::Loaded { data } = asset else {
        panic!("Bundled asset should be available immediately!");
    };
    assert!(matches!(data.as_ref(), ImageType::AnimatedBitmap { .. }));
}

#[test]
fn animated_image_remaining_delay_decreases_within_frame() {
    let asset_cache = new_asset_cache();
    let image_cache = ImageCache::new();

    let image = load_bundled_image(
        &image_cache,
        &asset_cache,
        "numbers-1000ms.gif",
        Vector2I::new(16, 16),
        FitType::Contain,
        AnimatedImageBehavior::FullAnimation,
    );

    let Image::Animated(animated) = image.as_ref() else {
        panic!("Expected animated image");
    };

    // Assumes the gif's first frame lasts at least 50ms.
    let (_, remaining_at_start) = animated
        .get_current_frame(0)
        .expect("Should get frame at 0ms");
    let (_, remaining_at_50) = animated
        .get_current_frame(50)
        .expect("Should get frame at 50ms");

    assert!(
        remaining_at_50 < remaining_at_start,
        "Remaining delay should decrease as we progress through a frame"
    );
}

#[test]
fn animated_webp_also_advances_frames() {
    let asset_cache = new_asset_cache();
    let image_cache = ImageCache::new();

    let image = load_bundled_image(
        &image_cache,
        &asset_cache,
        "animated.webp",
        Vector2I::new(16, 16),
        FitType::Contain,
        AnimatedImageBehavior::FullAnimation,
    );

    let Image::Animated(animated) = image.as_ref() else {
        panic!("Expected animated image");
    };

    assert!(
        animated.frames.len() > 1,
        "WebP should have multiple frames"
    );

    let (frame_0, _) = animated
        .get_current_frame(0)
        .expect("Should get frame at 0ms");
    let (frame_near_end, _) = animated
        .get_current_frame(animated.duration - 1)
        .expect("Should get frame near end");

    assert!(
        !Arc::ptr_eq(&frame_0, &frame_near_end) || animated.frames.len() == 1,
        "WebP animation should traverse different frames or only have one frame"
    );

    let (frame_wrapped, _) = animated
        .get_current_frame(animated.duration)
        .expect("Should get frame after one cycle");
    assert!(
        Arc::ptr_eq(&frame_0, &frame_wrapped),
        "Frame should wrap back to start after duration"
    );
}
