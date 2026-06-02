use core::fmt;
use std::collections::HashMap;
use std::error;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::rc::Rc;
use std::sync::{Arc, LazyLock};

use anyhow::{anyhow, Result};
use image::codecs::gif::GifDecoder;
use image::codecs::webp::WebPDecoder;
use image::imageops::FilterType;
use image::{AnimationDecoder, DynamicImage, Frame, ImageBuffer, ImageFormat};
use itertools::Itertools;
use parking_lot::{RwLock, RwLockUpgradableReadGuard};
use pathfinder_geometry::vector::Vector2I;
use resvg::tiny_skia::{self, IntSize};
use resvg::usvg;
use strum_macros::EnumIter;

use crate::assets::asset_cache::{Asset, AssetCache, AssetSource, AssetState};
use crate::util::parse_u32;
use crate::{Entity, SingletonEntity};

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

impl Asset for ImageType {
    fn try_from_bytes(data: &[u8]) -> anyhow::Result<ImageType> {
        // SVGs are not handled by the guess_format helper function, so we have to manually check
        // if it's an SVG ourselves.
        if data.first() == Some(&b'<') {
            let options = usvg::Options {
                fontdb: SVG_FONT_DB.clone(),
                ..Default::default()
            };
            let svg = Rc::new(usvg::Tree::from_data(data, &options)?);
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
                let img = image::ImageReader::with_format(
                    std::io::Cursor::new(data),
                    image::ImageFormat::Jpeg,
                )
                .decode()?
                .into_rgba8();
                Ok(ImageType::StaticBitmap {
                    image: Arc::new(StaticImage { img }),
                })
            }
            Ok(ImageFormat::Png) => {
                let img = image::ImageReader::with_format(
                    std::io::Cursor::new(data),
                    image::ImageFormat::Png,
                )
                .decode()?
                .into_rgba8();
                Ok(ImageType::StaticBitmap {
                    image: Arc::new(StaticImage { img }),
                })
            }
            Ok(ImageFormat::WebP) => {
                let decoder = WebPDecoder::new(std::io::Cursor::new(data))?;
                if decoder.has_animation() {
                    let frames = decoder.into_frames().collect_frames()?;
                    Ok(ImageType::AnimatedBitmap {
                        image: Arc::new(AnimatedImage::from(frames)),
                    })
                } else {
                    let img = DynamicImage::from_decoder(decoder)?.into_rgba8();
                    Ok(ImageType::StaticBitmap {
                        image: Arc::new(StaticImage { img }),
                    })
                }
            }
            Ok(ImageFormat::Gif) => {
                let decoder = GifDecoder::new(std::io::Cursor::new(data))?;
                let frames = decoder.into_frames().collect_frames()?;
                Ok(ImageType::AnimatedBitmap {
                    image: Arc::new(AnimatedImage::from(frames)),
                })
            }
            _ => Ok(ImageType::Unrecognized),
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
            ImageType::Unrecognized => 0,
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

#[derive(Clone)]
pub enum ImageType {
    Svg { svg: Rc<usvg::Tree> },
    StaticBitmap { image: Arc<StaticImage> },
    AnimatedBitmap { image: Arc<AnimatedImage> },
    // TODO: other types
    Unrecognized,
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
            ImageType::Unrecognized => None,
        }
    }

    fn type_str(&self) -> &'static str {
        match self {
            ImageType::Svg { .. } => "ImageType::Svg",
            ImageType::StaticBitmap { .. } => "ImageType::StaticBitmap",
            ImageType::AnimatedBitmap { .. } => "ImageType::AnimatedBitmap",
            ImageType::Unrecognized => "ImageType::Unrecognized",
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
            ImageType::Unrecognized => Err(anyhow!("Unrecognized image format.")),
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

    pub fn image_size(
        &self,
        asset_source: AssetSource,
        asset_cache: &AssetCache,
    ) -> Option<Vector2I> {
        let AssetState::Loaded { data } = asset_cache.load_asset::<ImageType>(asset_source) else {
            return None;
        };
        data.image_size()
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
