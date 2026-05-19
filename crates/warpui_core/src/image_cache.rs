use anyhow::{anyhow, Result};
use core::fmt;
use itertools::Itertools;
use std::{
    collections::HashMap,
    error,
    hash::{DefaultHasher, Hash, Hasher},
    rc::Rc,
    sync::{Arc, LazyLock},
};
use strum_macros::EnumIter;

use crate::{
    assets::asset_cache::{Asset, AssetCache, AssetSource, AssetState},
    util::parse_u32,
    Entity, SingletonEntity,
};
use image::{
    codecs::{gif::GifDecoder, webp::WebPDecoder},
    imageops::FilterType,
    AnimationDecoder, DynamicImage, Frame, ImageBuffer, ImageDecoder, ImageFormat,
};
use parking_lot::{RwLock, RwLockUpgradableReadGuard};
use pathfinder_geometry::vector::Vector2I;
use resvg::{
    tiny_skia::{self, IntSize},
    usvg,
};

const MIN_REFRESH_DELAY_MS: u32 = 50;

static SVG_FONT_DB: LazyLock<Arc<usvg::fontdb::Database>> = LazyLock::new(|| {
    let mut fontdb = usvg::fontdb::Database::new();
    fontdb.load_system_fonts();
    load_svg_fallback_fonts(&mut fontdb);
    Arc::new(fontdb)
});

fn load_svg_fallback_fonts(fontdb: &mut usvg::fontdb::Database) {
    fontdb.load_font_data(
        include_bytes!("../../../app/assets/bundled/fonts/roboto/Roboto-Regular.ttf").to_vec(),
    );
    fontdb.set_sans_serif_family("Roboto");
}

pub fn prewarm_svg_font_db() {
    LazyLock::force(&SVG_FONT_DB);
}

#[derive(EnumIter, Debug)]
pub enum CustomImageFormat {
    Rgb,
    Rgba,
}

impl CustomImageFormat {
    fn create_tag(&self) -> String {
        match self {
            CustomImageFormat::Rgb => "rgb".into(),
            CustomImageFormat::Rgba => "rgba".into(),
        }
    }
}

#[derive(Debug, Clone)]
enum CustomHeaderParsingError {
    ExpectedDataSizeMismatch {
        expected_bytes: usize,
        actual_bytes: usize,
    },
    InvalidCustomHeaderParam {
        param_name: String,
        value: String,
    },
    MissingCustomHeaderParam {
        param_name: String,
    },
    MissingHeaderIdentifier,
}

impl error::Error for CustomHeaderParsingError {}

impl fmt::Display for CustomHeaderParsingError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::ExpectedDataSizeMismatch {
                expected_bytes,
                actual_bytes,
            } => write!(
                f,
                "The custom image's expected bytes ({expected_bytes}) did not match the actual number of bytes ({actual_bytes})."
            ),
            Self::InvalidCustomHeaderParam { param_name, value } => write!(
                f,
                "Custom header had field {param_name} with an invalid value of {value}"
            ),
            Self::MissingCustomHeaderParam { param_name } => {
                write!(f, "Custom header had {param_name} field missing")
            }
            Self::MissingHeaderIdentifier => {
                write!(f, "Image did not contain the 'warp-img:' prefix.")
            }
        }
    }
}

#[derive(Debug, Clone)]
pub enum CustomHeaderCreationError {
    ExpectedDataSizeMismatch {
        expected_bytes: usize,
        actual_bytes: usize,
    },
}

#[derive(Debug)]
pub struct CustomImageHeader {
    pub width: u32,
    pub height: u32,
    pub image_format: CustomImageFormat,
}

impl CustomImageHeader {
    pub fn create_header(&self) -> String {
        format!(
            "warp-img:{}:{}:{}:",
            self.image_format.create_tag(),
            self.width,
            self.height
        )
    }

    pub fn prepend_custom_header(
        mut data: Vec<u8>,
        width: u32,
        height: u32,
        image_format: CustomImageFormat,
    ) -> Result<Vec<u8>, CustomHeaderCreationError> {
        let bytes_per_pixel = match image_format {
            CustomImageFormat::Rgb => 3,
            CustomImageFormat::Rgba => 4,
        };
        let expected_byte_count = (bytes_per_pixel * width * height) as usize;

        if expected_byte_count != data.len() {
            return Err(CustomHeaderCreationError::ExpectedDataSizeMismatch {
                expected_bytes: expected_byte_count,
                actual_bytes: data.len(),
            });
        }

        let custom_header = CustomImageHeader {
            width,
            height,
            image_format,
        };

        data.splice(
            0..0,
            custom_header.create_header().as_bytes().iter().copied(),
        );

        Ok(data)
    }

    fn try_from_bytes(data: &[u8]) -> Result<(CustomImageHeader, &[u8]), CustomHeaderParsingError> {
        if !data.starts_with(b"warp-img:") {
            return Err(CustomHeaderParsingError::MissingHeaderIdentifier);
        }

        let data = match data
            .iter()
            .position(|&byte| byte == b':')
            .map(|position| &data[position + 1..])
        {
            Some(data) => data,
            None => return Err(CustomHeaderParsingError::MissingHeaderIdentifier),
        };

        let (image_type, data) = match data
            .iter()
            .position(|&byte| byte == b':')
            .map(|position| (&data[..position], &data[position + 1..]))
        {
            Some((image_type, data)) => (image_type, data),
            None => {
                return Err(CustomHeaderParsingError::MissingCustomHeaderParam {
                    param_name: "image_type".to_string(),
                });
            }
        };

        let image_type = match image_type {
            b"rgb" => CustomImageFormat::Rgb,
            b"rgba" => CustomImageFormat::Rgba,
            _ => {
                return Err(CustomHeaderParsingError::InvalidCustomHeaderParam {
                    param_name: "image_type".to_string(),
                    value: std::str::from_utf8(image_type)
                        .unwrap_or("Unable to parse utf8")
                        .to_string(),
                });
            }
        };

        let (width, data) = match data
            .iter()
            .position(|&byte| byte == b':')
            .map(|position| (&data[..position], &data[position + 1..]))
        {
            Some((width, data)) => (width, data),
            None => {
                return Err(CustomHeaderParsingError::MissingCustomHeaderParam {
                    param_name: "width".to_string(),
                });
            }
        };

        let width = match parse_u32(width) {
            Some(width) => width,
            None => {
                return Err(CustomHeaderParsingError::InvalidCustomHeaderParam {
                    param_name: "width".to_string(),
                    value: std::str::from_utf8(width)
                        .unwrap_or("Unable to parse utf8")
                        .to_string(),
                });
            }
        };

        let (height, data) = match data
            .iter()
            .position(|&byte| byte == b':')
            .map(|position| (&data[..position], &data[position + 1..]))
        {
            Some((height, data)) => (height, data),
            None => {
                return Err(CustomHeaderParsingError::MissingCustomHeaderParam {
                    param_name: "height".to_string(),
                });
            }
        };

        let height = match parse_u32(height) {
            Some(height) => height,
            None => {
                return Err(CustomHeaderParsingError::InvalidCustomHeaderParam {
                    param_name: "height".to_string(),
                    value: std::str::from_utf8(height)
                        .unwrap_or("Unable to parse utf8")
                        .to_string(),
                });
            }
        };

        let bytes_per_pixel = match image_type {
            CustomImageFormat::Rgb => 3,
            CustomImageFormat::Rgba => 4,
        };
        let expected_byte_count = (bytes_per_pixel * width * height) as usize;

        if expected_byte_count != data.len() {
            return Err(CustomHeaderParsingError::ExpectedDataSizeMismatch {
                expected_bytes: expected_byte_count,
                actual_bytes: data.len(),
            });
        }

        Ok((
            CustomImageHeader {
                width,
                height,
                image_format: image_type,
            },
            data,
        ))
    }
}

/// Maximum width or height in pixels accepted by the static-raster decode
/// path. 8192 sits well above 4K (3840x2160) and covers the 99th-percentile
/// project asset, screenshot, and changelog still. See specs/GH9729/tech.md
/// §217 / §234.
const MAX_DECODE_DIMENSION: u32 = 8_192;

/// Total pixel cap (`MAX_DECODE_DIMENSION * MAX_DECODE_DIMENSION`). Belt-and-
/// suspenders post-decode check against decoders that honor per-axis limits
/// but still materialize a near-cap RGBA buffer. See specs/GH9729/tech.md §234.
const MAX_DECODE_PIXELS: u64 = 67_108_864;

/// Allocation cap forwarded to `image::Limits::max_alloc`. Sized at
/// `MAX_DECODE_PIXELS * 4` (RGBA bytes) so the dimension cap and the alloc
/// cap are internally consistent and the alloc cap is the binding constraint.
/// See specs/GH9729/tech.md §221.
const MAX_DECODE_ALLOC: u64 = 256 * 1024 * 1024;

/// Maximum number of frames accepted from an animated decoder before bailing.
/// `image` 0.25.x animated decoders do not enforce frame counts via
/// `image::Limits`; this cap is applied during frame iteration. See
/// specs/GH9729/tech.md §259.
const MAX_ANIMATED_FRAMES: usize = 256;

/// Total pixel budget summed across all frames of an animated image. With the
/// per-frame allocation reaching ~4 bytes per pixel (RGBA), this caps peak
/// frame-collection memory at ~256 MB regardless of decoder honesty. See
/// specs/GH9729/tech.md §259.
const MAX_ANIMATED_TOTAL_PIXELS: u64 = 67_108_864;

/// Maximum width or height in CSS pixels declared by an SVG's intrinsic size
/// before the renderer is asked to materialize it. Caps the
/// "tiny-byte-payload claims 200000x200000" attack on `resvg`. See
/// specs/GH9729/tech.md §321.
const MAX_SVG_RENDER_DIMENSION: u32 = 8_192;

/// Coarse content-sniff predicate that returns `true` when `data` begins with
/// a UTF-8 prefix consistent with XML or SVG: an optional UTF-8 BOM, optional
/// ASCII whitespace bounded to the first 1 KB, then one of the supported
/// prelude tokens (`<?xml`, `<svg`, `<!--`, `<!DOCTYPE`).
///
/// Two callers:
///   1. The asset-cache `LocalFile` read picks `MAX_SVG_BYTES` (4 MB) over
///      `MAX_PREVIEW_FILE_BYTES` (64 MB) when the on-disk content peek looks
///      like SVG, so a `.png` carrying SVG XML is still tightened to 4 MB.
///      (Implemented in GH9729 item 5a.)
///   2. `ImageType::try_from_bytes` gates `usvg::Tree::from_data` on the same
///      predicate so a binary blob renamed to `.svg` is rejected before
///      `usvg` does the work.
///
/// The predicate is intentionally a coarse sniff, not a full XML lexer; the
/// actual XML/SVG validation is `usvg`'s job. XML is case-sensitive and the
/// standard form of these tokens is fixed-case, so case-insensitive matching
/// is not attempted. See specs/GH9729/tech.md §321.
pub(crate) fn looks_like_svg_xml(data: &[u8]) -> bool {
    // Strip optional UTF-8 BOM.
    let bytes = data.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(data);
    // Skip leading ASCII whitespace, bounded to the first 1 KB to keep the
    // scan O(1) and to refuse pathological "1 GB of whitespace" inputs.
    let scan_end = bytes.len().min(1024);
    let after_ws = bytes[..scan_end]
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .map(|i| &bytes[i..])
        .unwrap_or(&[]);
    after_ws.starts_with(b"<?xml")
        || after_ws.starts_with(b"<svg")
        || after_ws.starts_with(b"<!--")
        || after_ws.starts_with(b"<!DOCTYPE")
}

/// Build the `image::Limits` envelope used by the static-raster decode path
/// (PNG, JPEG, WebP-static). See specs/GH9729/tech.md §234.
fn decode_limits() -> image::Limits {
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_DECODE_DIMENSION);
    limits.max_image_height = Some(MAX_DECODE_DIMENSION);
    limits.max_alloc = Some(MAX_DECODE_ALLOC);
    limits
}

/// Inner animated-decode entry point that takes its decoder limits, frame
/// count cap, and total-pixel cap as parameters. Lets unit tests exercise
/// the rejection paths against modest fixtures with small caps without
/// having to synthesize pathological animated WebPs at runtime. Production
/// callers go through `decode_animated_with_limits` which threads the
/// GH9729 production constants.
fn decode_animated_with_limits_inner(
    data: &[u8],
    format: image::ImageFormat,
    limits: image::Limits,
    max_frames: usize,
    max_total_pixels: u64,
) -> anyhow::Result<Vec<image::Frame>> {
    let mut frames = Vec::new();
    let mut total_pixels: u64 = 0;

    let frame_iter = match format {
        image::ImageFormat::Gif => {
            let mut dec = GifDecoder::new(std::io::Cursor::new(data))?;
            dec.set_limits(limits)?;
            dec.into_frames()
        }
        image::ImageFormat::WebP => {
            let mut dec = WebPDecoder::new(std::io::Cursor::new(data))?;
            dec.set_limits(limits)?;
            dec.into_frames()
        }
        _ => {
            anyhow::bail!("decode_animated_with_limits called with non-animated format")
        }
    };

    for (i, frame) in frame_iter.enumerate() {
        if i >= max_frames {
            anyhow::bail!("animated image has too many frames");
        }
        let frame = frame?;
        let buf = frame.buffer();
        let pixels = (buf.width() as u64).saturating_mul(buf.height() as u64);
        total_pixels = total_pixels.saturating_add(pixels);
        if total_pixels > max_total_pixels {
            anyhow::bail!("animated image exceeds total pixel budget");
        }
        frames.push(frame);
    }

    if frames.is_empty() {
        anyhow::bail!("animated image has no frames");
    }
    Ok(frames)
}

/// Decode an animated image (GIF, animated WebP) under the GH9729 size
/// envelope. Iterates frames and bails as soon as `MAX_ANIMATED_FRAMES`
/// or `MAX_ANIMATED_TOTAL_PIXELS` is breached, before the pathological
/// frame is collected into the output `Vec`.
///
/// `image` 0.25.x animated decoders are weaker than the static path
/// (`GifDecoder` ignores `max_alloc` per frame; `WebPDecoder` does not
/// override `set_limits` at all), so this explicit budget — applied during
/// iteration — is what actually bounds the animated decode envelope.
/// See specs/GH9729/tech.md §259.
fn decode_animated_with_limits(
    data: &[u8],
    format: image::ImageFormat,
) -> anyhow::Result<Vec<image::Frame>> {
    decode_animated_with_limits_inner(
        data,
        format,
        decode_limits(),
        MAX_ANIMATED_FRAMES,
        MAX_ANIMATED_TOTAL_PIXELS,
    )
}

/// Inner static-decode entry point that takes its limits and pixel cap as
/// parameters. Lets unit tests exercise the decode path with small fixtures
/// against small caps without having to materialize 8192-pixel-wide PNGs.
/// Production callers go through `decode_static_with_limits` which threads
/// the GH9729 production constants.
fn decode_static_with_limits_inner(
    data: &[u8],
    format: image::ImageFormat,
    limits: image::Limits,
    max_pixels: u64,
) -> anyhow::Result<image::RgbaImage> {
    let mut reader = image::ImageReader::with_format(std::io::Cursor::new(data), format);
    reader.limits(limits);
    // GH9729 §700: read EXIF orientation from the decoder *before*
    // consuming it into a `DynamicImage`. The `image` crate's
    // per-format decoders (JPEG with the EXIF APP1 segment, PNG with
    // the eXIf chunk, WebP with the EXIF chunk) report the tag here;
    // formats whose decoder doesn't override the trait method get
    // `Orientation::NoTransforms` (the trait default), making
    // `apply_orientation` a no-op for them.
    let mut decoder = reader.into_decoder()?;
    let orientation = decoder.orientation()?;
    let mut img = image::DynamicImage::from_decoder(decoder)?;
    let pixels = (img.width() as u64).saturating_mul(img.height() as u64);
    if pixels > max_pixels {
        anyhow::bail!("image is too large to preview");
    }
    // Apply orientation *after* the pixel-cap check — `apply_orientation`
    // can transpose width/height (Rotate90/Rotate270) but cannot change
    // the total pixel count, so cap correctness is preserved.
    img.apply_orientation(orientation);
    Ok(img.into_rgba8())
}

/// Decode a static-raster image (PNG, JPEG, WebP-static) under the
/// GH9729 size envelope. Returns the decoded `RgbaImage` or an error if the
/// input would breach `decode_limits()` or the post-decode pixel cap.
///
/// The post-decode `pixels > MAX_DECODE_PIXELS` check is a defensive guard
/// against decoders that honor per-axis `max_image_width`/`max_image_height`
/// but still materialize a near-cap RGBA buffer.
fn decode_static_with_limits(
    data: &[u8],
    format: image::ImageFormat,
) -> anyhow::Result<image::RgbaImage> {
    decode_static_with_limits_inner(data, format, decode_limits(), MAX_DECODE_PIXELS)
}

impl Asset for ImageType {
    fn try_from_bytes(data: &[u8]) -> anyhow::Result<ImageType> {
        // SVGs are not handled by the guess_format helper, so we sniff the
        // prefix ourselves. `looks_like_svg_xml` is a coarse content gate
        // shared with the asset-cache byte-cap selection (see specs/GH9729
        // tech.md §321) and rejects the "binary blob renamed to .svg" case
        // before `usvg` is asked to parse.
        if looks_like_svg_xml(data) {
            let options = usvg::Options {
                fontdb: SVG_FONT_DB.clone(),
                ..Default::default()
            };
            let tree = usvg::Tree::from_data(data, &options)?;
            // GH9729 §321 intrinsic-dimension cap: a tiny-byte payload can
            // declare width/height in the millions and OOM the renderer.
            // Reject before any rasterization is attempted.
            let size = tree.size();
            let w = size.width() as u32;
            let h = size.height() as u32;
            if w > MAX_SVG_RENDER_DIMENSION || h > MAX_SVG_RENDER_DIMENSION {
                anyhow::bail!("svg dimensions exceed render budget");
            }
            let pixels = (w as u64).saturating_mul(h as u64);
            if pixels > MAX_DECODE_PIXELS {
                anyhow::bail!("svg dimensions exceed render budget");
            }
            let svg = Rc::new(tree);
            return Ok(ImageType::Svg { svg });
        }

        if data.starts_with(b"warp-img:") {
            let (custom_warp_header, data) = match CustomImageHeader::try_from_bytes(data) {
                Ok((custom_warp_header, data)) => (custom_warp_header, data),
                Err(err) => return Err(anyhow!(err.to_string())),
            };

            let data = data.into();
            let Some(img) = (match custom_warp_header.image_format {
                CustomImageFormat::Rgb => {
                    let dynamic_image = ImageBuffer::from_raw(
                        custom_warp_header.width,
                        custom_warp_header.height,
                        data,
                    )
                    .map(DynamicImage::ImageRgb8);
                    dynamic_image.map(|dynamic_image| dynamic_image.into_rgba8())
                }
                CustomImageFormat::Rgba => {
                    let dynamic_image = ImageBuffer::from_raw(
                        custom_warp_header.width,
                        custom_warp_header.height,
                        data,
                    )
                    .map(DynamicImage::ImageRgba8);
                    dynamic_image.map(|dynamic_image| dynamic_image.into_rgba8())
                }
            }) else {
                return Err(anyhow!(
                    "Could not convert custom warp image into approprate dynamic image."
                ));
            };
            return Ok(ImageType::StaticBitmap {
                image: Arc::new(StaticImage { img }),
            });
        }

        match image::guess_format(data) {
            Ok(ImageFormat::Jpeg) => {
                // GH9729 §234: enforce dimension/alloc/pixel envelope.
                let img = decode_static_with_limits(data, image::ImageFormat::Jpeg)?;
                Ok(ImageType::StaticBitmap {
                    image: Arc::new(StaticImage { img }),
                })
            }
            Ok(ImageFormat::Png) => {
                // GH9729 §234: enforce dimension/alloc/pixel envelope.
                let img = decode_static_with_limits(data, image::ImageFormat::Png)?;
                Ok(ImageType::StaticBitmap {
                    image: Arc::new(StaticImage { img }),
                })
            }
            Ok(ImageFormat::WebP) => {
                let decoder = WebPDecoder::new(std::io::Cursor::new(data))?;
                if decoder.has_animation() {
                    // GH9729 §259: bound frame count and total pixels.
                    drop(decoder);
                    let frames = decode_animated_with_limits(data, image::ImageFormat::WebP)?;
                    Ok(ImageType::AnimatedBitmap {
                        image: Arc::new(AnimatedImage::from(frames)),
                    })
                } else {
                    // GH9729 §234: route the static branch through the
                    // limits-aware helper. The decoder is dropped here and
                    // re-opened by `ImageReader::with_format` inside the
                    // helper; the cost is one extra header parse.
                    drop(decoder);
                    let img = decode_static_with_limits(data, image::ImageFormat::WebP)?;
                    Ok(ImageType::StaticBitmap {
                        image: Arc::new(StaticImage { img }),
                    })
                }
            }
            Ok(ImageFormat::Gif) => {
                // GH9729 §259: bound frame count and total pixels.
                let frames = decode_animated_with_limits(data, image::ImageFormat::Gif)?;
                Ok(ImageType::AnimatedBitmap {
                    image: Arc::new(AnimatedImage::from(frames)),
                })
            }
            // GH9729 §695: every unrecognized format is now an error rather
            // than a sentinel `Ok(Unrecognized)` value. Callers (asset cache,
            // kitty, terminal model) already match `Err` and route it
            // through their existing failure paths.
            _ => Err(anyhow!("could not detect image format")),
        }
    }

    fn size_in_bytes(&self) -> usize {
        match self {
            ImageType::Svg { .. } => 0, // TODO: How do we calculate svg size in bytes?
            ImageType::StaticBitmap { image } => image.rgba_bytes().len(),
            ImageType::AnimatedBitmap { image } => image
                .frames
                .iter()
                .map(|frame| frame.image.rgba_bytes().len())
                .reduce(|acc, bytes| acc + bytes)
                .unwrap_or(0),
        }
    }
}

/// A reference to an image in the asset cache. Can be a static or animated image.
#[derive(Clone)]
pub enum Image {
    Static(Arc<StaticImage>),
    Animated(Arc<AnimatedImage>),
}

/// A representation of an image in the asset cache.
pub struct StaticImage {
    /// The actual RGBA image data, stored as a vector of bytes.
    img: image::RgbaImage,
}

impl StaticImage {
    pub fn size(&self) -> Vector2I {
        Vector2I::new(self.width() as i32, self.height() as i32)
    }

    pub fn width(&self) -> u32 {
        self.img.width()
    }

    pub fn height(&self) -> u32 {
        self.img.height()
    }

    pub fn rgba_bytes(&self) -> &[u8] {
        self.img.as_raw().as_slice()
    }
}

/// A representation of a single frame in an animated image.
pub struct AnimatedImageFrame {
    // The static image representing the current frame of an animated image.
    pub image: Arc<StaticImage>,
    // Delay until the next frame in ms.
    pub delay: u32,
}

/// A representation of an animated image (e.g. gif) in the asset cache.
pub struct AnimatedImage {
    /// The frames of the animated image in sequential order.
    pub frames: Vec<AnimatedImageFrame>,
    /// Total duration of the animated image in ms.
    pub duration: u32,
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum AnimatedImageBehavior {
    #[default]
    FullAnimation,
    FirstFramePreview,
}

#[derive(Clone, Debug, Copy, Eq, Hash, PartialEq)]
pub enum FitType {
    /// Expands the image to fill the entire space while maintaining the aspect ratio.
    Cover,
    /// Resizes the image to maximum size that fully fits in the given bounds,
    /// maintaining the aspect ratio.
    Contain,
    /// Stretches the image to fit the given bounds, ignoring the aspect ratio.
    /// This should likely only be used with SVGs, and not all SVGs are designed
    /// to be stretched.
    Stretch,
}

impl FitType {
    pub fn should_retain_aspect_ratio(&self) -> bool {
        match self {
            FitType::Cover | FitType::Contain => true,
            FitType::Stretch => false,
        }
    }
}

/// A successfully decoded image. Per GH9729 §695, an unrecognized format
/// is surfaced as `Err` from `try_from_bytes` instead of an `Unrecognized`
/// variant — every value of this type represents bytes we know how to
/// render.
#[derive(Clone)]
pub enum ImageType {
    Svg { svg: Rc<usvg::Tree> },
    StaticBitmap { image: Arc<StaticImage> },
    AnimatedBitmap { image: Arc<AnimatedImage> },
    // TODO: other types (HEIC/HEIF/AVIF/BMP/TIFF/ICO — see tech.md §702).
}

impl ImageType {
    /// Returns the size of the underlying asset.
    pub fn image_size(&self) -> Option<Vector2I> {
        match self {
            ImageType::Svg { svg } => Some(Vector2I::new(
                svg.size().width().round() as i32,
                svg.size().height().round() as i32,
            )),
            ImageType::StaticBitmap { image } => Some(image.size()),
            ImageType::AnimatedBitmap { image } => {
                image.frames.first().map(|frame| frame.image.size())
            }
        }
    }

    fn type_str(&self) -> &'static str {
        match self {
            ImageType::Svg { .. } => "ImageType::Svg",
            ImageType::StaticBitmap { .. } => "ImageType::StaticBitmap",
            ImageType::AnimatedBitmap { .. } => "ImageType::AnimatedBitmap",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum CacheOption {
    /// Only the specific used sizes are cached.
    /// Best for situations when the image/asset is used with a fixed size.
    /// Example: icons, theme picker previews.
    BySize,
    /// Only the original asset is cached.
    /// Best for situations when the image/asset doesn't have a fixed size, and it may change
    /// significantly on every window resize.
    /// Example: background image.
    Original,
}

impl From<Vec<Frame>> for AnimatedImage {
    fn from(value: Vec<Frame>) -> Self {
        let mut duration = 0;
        let frames = value
            .into_iter()
            .map(|frame| {
                let (delay_numerator, delay_denominator) = frame.delay().numer_denom_ms();
                let delay_ms = (delay_numerator / delay_denominator).max(MIN_REFRESH_DELAY_MS);
                duration += delay_ms;

                AnimatedImageFrame {
                    image: Arc::new(StaticImage {
                        img: frame.into_buffer(),
                    }),
                    delay: delay_ms,
                }
            })
            .collect_vec();

        AnimatedImage { frames, duration }
    }
}

fn resize_animated_image(
    image: &AnimatedImage,
    bounds: Vector2I,
    fit_type: FitType,
) -> AnimatedImage {
    let resized_frames = image
        .frames
        .iter()
        .map(|frame| AnimatedImageFrame {
            image: Arc::new(StaticImage {
                img: resize_image(&frame.image.img, bounds, fit_type),
            }),
            delay: frame.delay,
        })
        .collect_vec();
    AnimatedImage {
        frames: resized_frames,
        duration: image.duration,
    }
}

fn svg_image(svg: &Rc<usvg::Tree>, bounds: Vector2I, fit_type: FitType) -> Result<Image> {
    let svg_size = &svg.size();

    let svg_has_wider_ratio =
        svg_size.width() / svg_size.height() > bounds.x() as f32 / bounds.y() as f32;
    let fit = match (fit_type, svg_has_wider_ratio) {
        (FitType::Contain, true) | (FitType::Cover, false) => FitTo::Width(bounds.x() as u32),
        (FitType::Contain, false) | (FitType::Cover, true) => FitTo::Height(bounds.y() as u32),
        (FitType::Stretch, _) => FitTo::Bounds(bounds.x() as u32, bounds.y() as u32),
    };
    let svg_size = svg_size.to_int_size();
    let size = fit
        .fit_to_size(svg_size)
        .ok_or_else(|| anyhow!("Unable to fit SVG image to size"))?;
    let transform = fit.fit_to_transform(svg_size);

    let mut pixmap = tiny_skia::Pixmap::new(size.width(), size.height())
        .ok_or_else(|| anyhow!("Could not could create pixmap for bounds {:?}", bounds))?;
    resvg::render(svg.as_ref(), transform, &mut pixmap.as_mut());

    let img = image::RgbaImage::from_vec(pixmap.width(), pixmap.height(), pixmap.take()).ok_or_else(|| anyhow!("Failed to convert tiny_skia::Pixmap into image::ImageBuffer due to buffer size mismatch"))?;

    Ok(Image::Static(Arc::new(StaticImage { img })))
}

fn resize_image(img: &image::RgbaImage, bounds: Vector2I, fit_type: FitType) -> image::RgbaImage {
    // If the image dimensions match the target bounding box, return a simple
    // copy of it.
    if bounds.x() as u32 == img.width() && bounds.y() as u32 == img.height() {
        return img.clone();
    }

    let filter = FilterType::Triangle;
    match fit_type {
        // This logic is adapted from image::DynamicImage::resize_to_fill().
        FitType::Cover => {
            let nwidth = bounds.x() as u32;
            let nheight = bounds.y() as u32;

            // Resize the image, maintaining aspect ratio, such that the
            // smaller dimension equals its bounds.
            let (iwidth, iheight) =
                resize_dimensions(img.width(), img.height(), nwidth, nheight, FitType::Cover);
            let mut intermediate =
                DynamicImage::from(image::imageops::resize(img, iwidth, iheight, filter));

            // Based on the original and new aspect ratios, crop off the excess
            // image data along either the vertical or horizontal axis.
            let aspect_ratio = u64::from(iwidth) * u64::from(nheight);
            let new_aspect_ratio = u64::from(nwidth) * u64::from(iheight);
            let img = if new_aspect_ratio > aspect_ratio {
                intermediate.crop(0, (iheight - nheight) / 2, nwidth, nheight)
            } else {
                intermediate.crop((iwidth - nwidth) / 2, 0, nwidth, nheight)
            };

            img.into_rgba8()
        }
        fit_type => {
            let (new_width, new_height) = resize_dimensions(
                img.width(),
                img.height(),
                bounds.x() as u32,
                bounds.y() as u32,
                fit_type,
            );
            image::imageops::resize(img, new_width, new_height, filter)
        }
    }
}

/// Calculates the width and height an image should be resized to.
/// This preserves aspect ratio, and based on the `fit_type` parameter
/// will either fill the dimensions to fit inside the smaller constraint
/// (will overflow the specified bounds on one axis to preserve
/// aspect ratio), or will shrink so that both dimensions are
/// completely contained within the given `width` and `height`,
/// with empty space on one axis, unless the fit_type is `Stretch`.
///
/// This is adapted from image::math::utils::resize_dimensions().
pub fn resize_dimensions(
    width: u32,
    height: u32,
    nwidth: u32,
    nheight: u32,
    fit_type: FitType,
) -> (u32, u32) {
    use std::cmp::max;

    let wratio = nwidth as f64 / width as f64;
    let hratio = nheight as f64 / height as f64;

    let ratio = match fit_type {
        FitType::Cover => f64::max(wratio, hratio),
        FitType::Contain => {
            // Resize the image, maintaining aspect ratio, such that the larger
            // dimension equals its bounds.
            f64::min(wratio, hratio)
        }
        // Stretch doesn't maintain the aspect ratio
        FitType::Stretch => return (nwidth, nheight),
    };

    let nw = max((width as f64 * ratio).round() as u64, 1);
    let nh = max((height as f64 * ratio).round() as u64, 1);

    if nw > u64::from(u32::MAX) {
        let ratio = u32::MAX as f64 / width as f64;
        (u32::MAX, max((height as f64 * ratio).round() as u32, 1))
    } else if nh > u64::from(u32::MAX) {
        let ratio = u32::MAX as f64 / height as f64;
        (max((width as f64 * ratio).round() as u32, 1), u32::MAX)
    } else {
        (nw as u32, nh as u32)
    }
}

impl ImageType {
    /// Converts the ImageType to the Image structure.
    /// Takes into account bounds, fit_type and whether to resize the image.
    /// If resize is set to true, the image is first resized to either cover or contain fit within
    /// the given bounds. Otherwise, the dimensions are ignored, and the image is converted with
    /// its original size. In this case we may cache the image bytes to avoid repeated conversions.
    fn to_image(
        &self,
        bounds: Vector2I,
        fit_type: FitType,
        resize: bool,
        animated_image_behavior: AnimatedImageBehavior,
    ) -> Result<Image> {
        match self {
            ImageType::StaticBitmap { image } => {
                if resize {
                    let img = resize_image(&image.img, bounds, fit_type);
                    Ok(Image::Static(Arc::new(StaticImage { img })))
                } else {
                    Ok(Image::Static(image.clone()))
                }
            }
            ImageType::AnimatedBitmap { image } => match animated_image_behavior {
                AnimatedImageBehavior::FullAnimation => {
                    if resize {
                        Ok(Image::Animated(Arc::new(resize_animated_image(
                            image.as_ref(),
                            bounds,
                            fit_type,
                        ))))
                    } else {
                        Ok(Image::Animated(image.clone()))
                    }
                }
                AnimatedImageBehavior::FirstFramePreview => {
                    let first_frame = image
                        .frames
                        .first()
                        .ok_or_else(|| anyhow!("Animated image contained no frames"))?
                        .image
                        .clone();
                    if resize {
                        let img = resize_image(&first_frame.img, bounds, fit_type);
                        Ok(Image::Static(Arc::new(StaticImage { img })))
                    } else {
                        Ok(Image::Static(first_frame))
                    }
                }
            },
            ImageType::Svg { svg } => svg_image(svg, bounds, fit_type),
        }
    }
}

impl AnimatedImage {
    /// Calculates the current frame of the animated image based on elapsed time and
    /// returns a pointer to the image along with the remaining delay in the frame.
    /// `elapsed` is the time in ms since the animated image started animating.
    pub fn get_current_frame(&self, elapsed: u32) -> Result<(Arc<StaticImage>, u32)> {
        if self.duration == 0 {
            return Err(anyhow!(
                "Animated image has duration 0, which is not supported"
            ));
        }
        // Linear search for the correct frame, this can be optimized.
        let elapsed = elapsed % self.duration;
        let mut start = 0;
        for frame in self.frames.iter() {
            let end = start + frame.delay;
            if elapsed >= start && elapsed < end {
                let remaining_delay = end - elapsed;
                return Ok((frame.image.clone(), remaining_delay));
            }
            start = end;
        }

        // We should only reach here if self.frames is empty.
        Err(anyhow!("No frame found for elapsed {}", elapsed))
    }
}

/// Image fit options used when rendering an SVG into a bitmap. `resvg` used to support this
/// directly, however it was removed in version `0.34`.
#[derive(Clone, Debug)]
enum FitTo {
    /// Scale to width, preserving aspect ratio.
    Width(u32),
    /// Scale to height, preserving aspect ratio.
    Height(u32),
    /// Stretch to fit the given bounds, ignoring the aspect ratio.
    Bounds(u32, u32),
}

impl FitTo {
    /// Adjusts `size` based on the current value of `FitTo`.
    /// Taken directly from `resvg`:
    /// <https://github.com/RazrFalcon/resvg/blob/0c8a8cd0781d3025659f6de6158d605ca1b752f5/crates/resvg/src/main.rs#L418C8-L439>.
    fn fit_to_size(&self, size: IntSize) -> Option<IntSize> {
        match *self {
            FitTo::Width(w) => size.scale_to_width(w),
            FitTo::Height(h) => size.scale_to_height(h),
            FitTo::Bounds(w, h) => Some(IntSize::from_wh(w, h)?),
        }
    }

    /// Returns a [`tiny_skia::Transform`] that would scale `size` to match the new fitted size
    /// produced via [`FitTo::fit_to_size`].
    /// Taken directly from `resvg`:
    /// <https://github.com/RazrFalcon/resvg/blob/0c8a8cd0781d3025659f6de6158d605ca1b752f5/crates/resvg/src/main.rs#L418C8-L439>.
    fn fit_to_transform(&self, size: IntSize) -> tiny_skia::Transform {
        let original_size = size.to_size();
        let fitted_size = match self.fit_to_size(size) {
            Some(fitted_size) => fitted_size.to_size(),
            None => return tiny_skia::Transform::default(),
        };
        tiny_skia::Transform::from_scale(
            fitted_size.width() / original_size.width(),
            fitted_size.height() / original_size.height(),
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct RenderedImageCacheKey {
    bounds: Vector2I,
    animated_image_behavior: AnimatedImageBehavior,
}

#[derive(Default)]
pub struct ImageCache {
    /// Map of images of any ImageType already scaled to a certain size.
    /// Uses the hashed AssetSource and rendered-image properties as a key.
    images: RwLock<HashMap<u64, HashMap<RenderedImageCacheKey, Rc<Image>>>>,
}

impl ImageCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn evict_image(&self, asset_source: &AssetSource) {
        let mut cache = self.images.write();

        let mut s = DefaultHasher::new();
        asset_source.hash(&mut s);
        let cache_key = s.finish();

        cache.remove(&cache_key);
    }

    /// Removes a single cached size entry for an asset.
    ///
    /// When the removed `Rc<Image>` is the last strong holder of the inner
    /// `Arc<StaticImage>`, that `Arc`'s strong count drops to zero. On the
    /// next call to `TextureCache::end_frame()`, the corresponding GPU texture
    /// will be evicted automatically via the `Weak<StaticImage>` it holds.
    ///
    /// `bounds` must match the resolved bounds used as the cache key inside
    /// `image()` (i.e., after any `max_dimension` adjustment), not the
    /// originally requested bounds.
    // Called by the debounce eviction pass added in the main changeset.
    /// TODO(APP-3877): remove `#[allow(dead_code)]` once the debounce eviction pass wires this up.
    #[allow(dead_code)]
    fn evict_size(
        &self,
        asset_source: &AssetSource,
        bounds: Vector2I,
        animated_image_behavior: AnimatedImageBehavior,
    ) {
        let mut s = DefaultHasher::new();
        asset_source.hash(&mut s);
        let cache_key = s.finish();

        let rendered_key = RenderedImageCacheKey {
            bounds,
            animated_image_behavior,
        };

        let mut cache = self.images.write();
        if let Some(inner_map) = cache.get_mut(&cache_key) {
            inner_map.remove(&rendered_key);
            if inner_map.is_empty() {
                cache.remove(&cache_key);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn image(
        &self,
        asset_source: AssetSource,
        bounds: Vector2I,
        fit_type: FitType,
        animated_image_behavior: AnimatedImageBehavior,
        cache_option: CacheOption,
        max_dimension: Option<u32>,
        asset_cache: &AssetCache,
    ) -> AssetState<Image> {
        let mut s = DefaultHasher::new();
        asset_source.hash(&mut s);
        let cache_key = s.finish();

        match asset_cache.load_asset::<ImageType>(asset_source) {
            AssetState::Loading { handle } => AssetState::Loading { handle },
            AssetState::Evicted => AssetState::Evicted,
            AssetState::FailedToLoad(err) => AssetState::FailedToLoad(err),
            AssetState::Loaded { data } => {
                let (mut needs_resize, mut bounds) = match cache_option {
                    CacheOption::BySize => {
                        // Only store a resized copy of the source asset if a
                        // specific size was requested and it doesn't match the
                        // source asset's size.
                        let needs_resize = data.image_size() != Some(bounds);
                        (needs_resize, bounds)
                    }
                    CacheOption::Original => {
                        // If the caller requested that we cache the asset at
                        // its original size, set its size as the target
                        // bounds.
                        let Some(bounds) = data.image_size() else {
                            return AssetState::FailedToLoad(Rc::new(anyhow!(
                                "Requested CacheOption::Original for {}, which has no inherent size",
                                data.type_str()
                            )));
                        };
                        (false, bounds)
                    }
                };

                // If we need to ensure the image isn't larger than a given
                // size along either dimension, check if a resize is needed
                // and update needs_resize and bounds accordingly.
                if let Some(max_dimension) = max_dimension {
                    let width = bounds.x() as u32;
                    let height = bounds.y() as u32;

                    if width > max_dimension || height > max_dimension {
                        needs_resize = true;
                        let (nwidth, nheight) = resize_dimensions(
                            width,
                            height,
                            max_dimension,
                            max_dimension,
                            fit_type,
                        );
                        bounds = Vector2I::new(nwidth as i32, nheight as i32);
                    }
                }

                let rendered_image_cache_key = RenderedImageCacheKey {
                    bounds,
                    animated_image_behavior,
                };

                // If it's already in the image cache at the target size,
                // return it.
                let cache = self.images.upgradable_read();
                if let Some(inner_map) = cache.get(&cache_key) {
                    if let Some(image) = inner_map.get(&rendered_image_cache_key) {
                        return AssetState::Loaded {
                            data: image.clone(),
                        };
                    }
                }

                // Otherwise, create the correctly-sized image struct and
                // insert it into the cache (if necessary).
                let image =
                    match data.to_image(bounds, fit_type, needs_resize, animated_image_behavior) {
                        Ok(image) => Rc::new(image),
                        Err(err) => return AssetState::FailedToLoad(Rc::new(err)),
                    };
                if needs_resize {
                    let mut images_cache = RwLockUpgradableReadGuard::upgrade(cache);
                    images_cache
                        .entry(cache_key)
                        .or_default()
                        .insert(rendered_image_cache_key, image.clone());
                }

                AssetState::Loaded { data: image }
            }
        }
    }
}

impl Entity for ImageCache {
    type Event = ();
}

impl SingletonEntity for ImageCache {}

#[cfg(test)]
#[path = "image_cache_tests.rs"]
mod tests;

#[cfg(test)]
pub(crate) mod test_utils {
    use super::*;

    /// Creates an `Arc<StaticImage>` with the given dimensions for use in unit tests.
    pub(crate) fn make_static_image(width: u32, height: u32) -> Arc<StaticImage> {
        Arc::new(StaticImage {
            img: image::RgbaImage::new(width, height),
        })
    }
}
