//! Crop geometry for a capture substrate that can only record a whole display.
//!
//! Such a substrate records a single window by cropping the display frame. The
//! window's bounds arrive in display points while the captured frame is in
//! physical pixels, so the rectangle has to be scaled, clamped into the frame,
//! and aligned to a chroma boundary before ffmpeg's `crop` filter can use it.

/// A rectangle in display points with a top-left origin.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PointRect {
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) width: f64,
    pub(crate) height: f64,
}

impl PointRect {
    fn is_usable(&self) -> bool {
        self.x.is_finite()
            && self.y.is_finite()
            && self.width.is_finite()
            && self.height.is_finite()
            && self.width > 0.0
            && self.height > 0.0
    }
}

/// An even, fully in-frame rectangle of the captured display frame, in physical
/// pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CaptureCrop {
    pub(crate) x: u32,
    pub(crate) y: u32,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

impl CaptureCrop {
    /// Renders the rectangle as ffmpeg's `crop` filter argument.
    pub(crate) fn filter_arg(&self) -> String {
        let Self {
            x,
            y,
            width,
            height,
        } = self;
        format!("crop={width}:{height}:{x}:{y}")
    }
}

/// How far outside the captured frame a converted edge may land before it counts
/// as real out-of-frame geometry rather than point-to-pixel rounding error.
const ROUNDING_TOLERANCE_PX: i64 = 1;
/// Smallest crop the encoder accepts once both axes are rounded down to even.
const MIN_CROP_EDGE_PX: i64 = 2;

/// Resolves `window`'s point-space bounds into a crop of the captured frame.
///
/// `display_origin` is the captured display's origin in the same global point
/// space as `window`, `scale` its backing pixels per point, and `frame` the
/// captured frame's physical pixel size. Fails rather than silently recording
/// the wrong pixels when the window does not fit the frame or is too small to
/// encode.
pub(crate) fn window_crop_in_capture_space(
    window: PointRect,
    display_origin: (f64, f64),
    scale: f64,
    frame: (u32, u32),
) -> Result<CaptureCrop, String> {
    if !window.is_usable() {
        return Err(format!(
            "window bounds {}x{} at ({}, {}) are not a usable rectangle",
            window.width, window.height, window.x, window.y
        ));
    }
    if !scale.is_finite() || scale <= 0.0 {
        return Err(format!("display backing scale {scale} is not usable"));
    }
    let (frame_width, frame_height) = (i64::from(frame.0), i64::from(frame.1));
    if frame_width == 0 || frame_height == 0 {
        return Err(format!("capture frame {}x{} is empty", frame.0, frame.1));
    }

    let to_pixels = |points: f64| (points * scale).round() as i64;
    let left = to_pixels(window.x - display_origin.0);
    let top = to_pixels(window.y - display_origin.1);
    let right = to_pixels(window.x + window.width - display_origin.0);
    let bottom = to_pixels(window.y + window.height - display_origin.1);

    if left < -ROUNDING_TOLERANCE_PX
        || top < -ROUNDING_TOLERANCE_PX
        || right > frame_width + ROUNDING_TOLERANCE_PX
        || bottom > frame_height + ROUNDING_TOLERANCE_PX
    {
        return Err(format!(
            "window pixel rect ({left},{top})-({right},{bottom}) is not contained in the \
             {frame_width}x{frame_height} capture frame"
        ));
    }

    // yuv420p subsamples chroma 2x2, so the offset and the size both have to be
    // even for the crop to land on a chroma sample boundary.
    let x = left.clamp(0, frame_width) & !1;
    let y = top.clamp(0, frame_height) & !1;
    let width = (right.clamp(0, frame_width) - x) & !1;
    let height = (bottom.clamp(0, frame_height) - y) & !1;
    if width < MIN_CROP_EDGE_PX || height < MIN_CROP_EDGE_PX {
        return Err(format!(
            "window crop {width}x{height} is too small to encode"
        ));
    }

    Ok(CaptureCrop {
        x: x as u32,
        y: y as u32,
        width: width as u32,
        height: height as u32,
    })
}

#[cfg(test)]
#[path = "window_crop_tests.rs"]
mod tests;
