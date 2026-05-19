use std::{borrow::Cow, rc::Rc};

use rust_embed::RustEmbed;

use crate::{
    r#async::executor::{Background, Foreground},
    AssetProvider,
};

use super::*;

#[derive(Clone, Copy, RustEmbed)]
#[folder = "test_data"]
pub struct Assets;

// Implement the AssetProvider trait here (required by App::new).
impl AssetProvider for Assets {
    fn get(&self, path: &str) -> Result<Cow<'_, [u8]>> {
        match path {
            "animated.webp" => Ok(Cow::Borrowed(include_bytes!("../test_data/animated.webp"))),
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

    assert!(image
        .rgba_bytes()
        .chunks_exact(4)
        .any(|pixel| pixel[3] != 0));
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

    assert!(image
        .rgba_bytes()
        .chunks_exact(4)
        .any(|pixel| pixel[3] != 0));
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
    image_cache.evict_size(&source, small_bounds, AnimatedImageBehavior::FullAnimation);

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

// GH9729: decoder-budget tests (specs/GH9729/tech.md §613:640-652).
// Static raster + SVG live here. Animated tests (lines 643-646) live under
// item 4-tests-b in a follow-up commit.

/// Encode an `(width, height)` blank RGBA image as PNG into an in-memory
/// buffer. Used by the static-decode tests to feed real PNG bytes through
/// the production-shaped decode path. Sized small (caller's responsibility)
/// so the test runner does not allocate the production-cap envelope.
fn encode_blank_png(width: u32, height: u32) -> Vec<u8> {
    let img = image::RgbaImage::new(width, height);
    let mut out: Vec<u8> = Vec::new();
    image::DynamicImage::ImageRgba8(img)
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .expect("PNG encode for test fixture");
    out
}

#[test]
fn decode_static_rejects_dimensions_over_cap() {
    // 200x100 PNG against a max-width cap of 100. The decoder reads the IHDR,
    // observes width 200 > 100, and bails before any pixel is decompressed.
    let png = encode_blank_png(200, 100);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(100);
    limits.max_image_height = Some(100);
    let result = super::decode_static_with_limits_inner(
        &png,
        image::ImageFormat::Png,
        limits,
        super::MAX_DECODE_PIXELS,
    );
    assert!(
        result.is_err(),
        "expected dimension cap to reject 200x100 PNG"
    );
}

#[test]
fn decode_static_rejects_pixels_over_cap() {
    // 200x100 PNG (20_000 pixels) against a per-pixel cap of 10_000. Dimensions
    // pass the per-axis caps; the post-decode pixel-count check is what fires.
    let png = encode_blank_png(200, 100);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(1_000);
    limits.max_image_height = Some(1_000);
    let result = super::decode_static_with_limits_inner(
        &png,
        image::ImageFormat::Png,
        limits,
        /* max_pixels */ 10_000,
    );
    assert!(
        result.is_err(),
        "expected pixel cap to reject 20k-pixel PNG"
    );
}

#[test]
fn decode_static_accepts_normal_photo() {
    // 200x100 PNG passes both caps cleanly under the production constants.
    let png = encode_blank_png(200, 100);
    let result = super::decode_static_with_limits(&png, image::ImageFormat::Png);
    let img = result.expect("normal photo should decode");
    assert_eq!(img.dimensions(), (200, 100));
}

// `looks_like_svg_xml` direct-predicate tests.

#[test]
fn decode_svg_rejects_non_xml_prefix() {
    // 1 KB buffer starting with NUL bytes never reaches `usvg::Tree::from_data`.
    let buf = vec![0u8; 1024];
    assert!(!super::looks_like_svg_xml(&buf));
}

#[test]
fn decode_svg_accepts_xml_prelude_with_bom_and_whitespace() {
    let mut buf: Vec<u8> = vec![0xEF, 0xBB, 0xBF]; // UTF-8 BOM
    buf.extend_from_slice(
        b"\n  <?xml version=\"1.0\"?><svg xmlns=\"http://www.w3.org/2000/svg\"/>",
    );
    assert!(super::looks_like_svg_xml(&buf));
}

#[test]
fn decode_svg_accepts_doctype_prelude() {
    let buf = b"<!DOCTYPE svg PUBLIC \"-//W3C//DTD SVG 1.1//EN\" \"http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd\">\n<svg xmlns=\"http://www.w3.org/2000/svg\"/>";
    assert!(super::looks_like_svg_xml(buf));
}

#[test]
fn decode_svg_accepts_xml_comment_prelude() {
    let buf = b"<!-- generated by Inkscape -->\n<svg xmlns=\"http://www.w3.org/2000/svg\"/>";
    assert!(super::looks_like_svg_xml(buf));
}

// SVG end-to-end through `ImageType::try_from_bytes`.

#[test]
fn decode_svg_accepts_normal_icon() {
    let svg = b"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"256\" height=\"256\"><circle cx=\"128\" cy=\"128\" r=\"64\"/></svg>";
    let result = super::ImageType::try_from_bytes(svg).expect("normal icon decodes");
    match result {
        super::ImageType::Svg { .. } => {}
        _ => panic!("expected Svg variant"),
    }
}

#[test]
fn decode_svg_rejects_intrinsic_dimensions_over_cap() {
    // Tiny byte payload but declares 200000 x 200000 - the renderer would
    // OOM trying to materialize this. The intrinsic-dimension cap fires
    // after `usvg::Tree::from_data` returns and before any rasterization.
    let svg =
        b"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"200000\" height=\"200000\"><g/></svg>";
    let result = super::ImageType::try_from_bytes(svg);
    assert!(
        result.is_err(),
        "expected SVG intrinsic-dimension cap to reject 200000x200000",
    );
}

// GH9729: animated decoder tests (specs/GH9729/tech.md §613:643-646).
// Reuses the existing animated.webp and numbers-1000ms.gif fixtures that
// the asset-loading tests above already include via the `Assets` impl.

const ANIMATED_GIF_BYTES: &[u8] = include_bytes!("../../warpui/examples/assets/numbers-1000ms.gif");
const ANIMATED_WEBP_BYTES: &[u8] = include_bytes!("../test_data/animated.webp");

#[test]
fn decode_animated_rejects_too_many_frames() {
    // Cap frames at 1; a multi-frame GIF must trip the count cap on the
    // second frame iteration BEFORE that frame is appended to the output.
    let limits = image::Limits::default();
    let result = super::decode_animated_with_limits_inner(
        ANIMATED_GIF_BYTES,
        image::ImageFormat::Gif,
        limits,
        /* max_frames */ 1,
        /* max_total_pixels */ u64::MAX,
    );
    let err = result.err().expect("expected frame-count cap to fire");
    let msg = format!("{err}");
    assert!(
        msg.contains("too many frames"),
        "expected frame-count error, got: {msg}",
    );
}

#[test]
fn decode_animated_rejects_total_pixel_budget() {
    // Cap total pixels at 1; the very first frame's pixel count exceeds the
    // budget, so the helper bails before the frame Vec ever grows past 0
    // entries (the iteration accumulates into total_pixels and then bails
    // BEFORE pushing the frame; the empty-frames branch catches it).
    let limits = image::Limits::default();
    let result = super::decode_animated_with_limits_inner(
        ANIMATED_WEBP_BYTES,
        image::ImageFormat::WebP,
        limits,
        /* max_frames */ usize::MAX,
        /* max_total_pixels */ 1,
    );
    assert!(result.is_err(), "expected total-pixel cap to fire");
}

#[test]
fn decode_animated_constructs_bitmap_for_legitimate_input() {
    // GIF roundtrip: production caps accept the existing fixture and return
    // every frame the decoder yielded (regression check against item 4b
    // accidentally truncating frame collection to one frame).
    let frames = super::decode_animated_with_limits(ANIMATED_GIF_BYTES, image::ImageFormat::Gif)
        .expect("legitimate animated GIF should decode");
    assert!(
        frames.len() > 1,
        "fixture is animated; expected > 1 frame, got {}",
        frames.len(),
    );
}

#[test]
fn decode_animated_constructs_bitmap_for_legitimate_webp() {
    // Animated WebP roundtrip; same shape as the GIF roundtrip above.
    let frames = super::decode_animated_with_limits(ANIMATED_WEBP_BYTES, image::ImageFormat::WebP)
        .expect("legitimate animated WebP should decode");
    assert!(
        frames.len() > 1,
        "fixture is animated; expected > 1 frame, got {}",
        frames.len(),
    );
}
