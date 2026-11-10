//! Mapping between the downscaled screenshot images the model sees and native screen pixels.

use computer_use::{Screenshot, ScreenshotParams, ScreenshotRegion, Target, Vector2I};
use schemars::JsonSchema;
use serde::Deserialize;

use crate::ToolError;

/// Size limits for screenshot images returned to the model.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct ImageConstraints {
    /// The maximum length of the image's longer edge, in pixels.
    pub max_long_edge_px: u32,
    /// The maximum number of pixels in the image.
    pub max_total_px: u32,
}

impl ImageConstraints {
    /// Anthropic's computer-use image limits.
    pub const ANTHROPIC: Self = Self {
        max_long_edge_px: 1568,
        max_total_px: 1_150_000,
    };

    /// OpenAI's computer-use image limits.
    pub const OPENAI: Self = Self {
        max_long_edge_px: 1600,
        max_total_px: 10_240_000,
    };

    pub(crate) fn screenshot_params(self, region: Option<ScreenshotRegion>) -> ScreenshotParams {
        ScreenshotParams {
            max_long_edge_px: Some(self.max_long_edge_px as usize),
            max_total_px: Some(self.max_total_px as usize),
            region,
            target: Target::Screen,
        }
    }

    fn scale_factor(self, width: u32, height: u32) -> f64 {
        computer_use::screenshot_scale_factor(
            width,
            height,
            Some(self.max_long_edge_px as usize),
            Some(self.max_total_px as usize),
        )
    }
}

/// A point in the pixel space of the most recent full screenshot.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Deserialize, JsonSchema)]
pub(crate) struct Point {
    /// Horizontal pixel offset from the left edge of the screenshot.
    pub x: i32,
    /// Vertical pixel offset from the top edge of the screenshot.
    pub y: i32,
}

/// How the most recent full screenshot maps onto the native screen.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ScreenGeometry {
    native_width: u32,
    native_height: u32,
    image_width: u32,
    image_height: u32,
    scale: f64,
}

impl ScreenGeometry {
    pub fn from_screenshot(screenshot: &Screenshot, constraints: ImageConstraints) -> Self {
        let native_width = screenshot.original_width as u32;
        let native_height = screenshot.original_height as u32;
        Self {
            native_width,
            native_height,
            image_width: screenshot.width as u32,
            image_height: screenshot.height as u32,
            scale: constraints.scale_factor(native_width, native_height),
        }
    }

    pub fn image_size(self) -> (u32, u32) {
        (self.image_width, self.image_height)
    }

    pub fn native_size(self) -> (u32, u32) {
        (self.native_width, self.native_height)
    }

    /// Maps a pixel in the screenshot image onto the native screen pixel it was sampled from.
    pub fn to_native(self, point: Point) -> Result<Vector2I, ToolError> {
        if !self.contains(point, false) {
            return Err(self.out_of_bounds(point));
        }
        Ok(Vector2I::new(
            self.unscale(point.x, self.native_width - 1),
            self.unscale(point.y, self.native_height - 1),
        ))
    }

    /// Maps a region corner, which may lie on the image's right or bottom edge, onto the native
    /// screen.
    pub fn corner_to_native(self, point: Point) -> Result<Vector2I, ToolError> {
        if !self.contains(point, true) {
            return Err(self.out_of_bounds(point));
        }
        Ok(Vector2I::new(
            self.unscale(point.x, self.native_width),
            self.unscale(point.y, self.native_height),
        ))
    }

    /// Maps a native screen pixel into the screenshot image's pixel space.
    pub fn to_model(self, native: Vector2I) -> Point {
        Point {
            x: (native.x() as f64 * self.scale).round() as i32,
            y: (native.y() as f64 * self.scale).round() as i32,
        }
    }

    fn contains(self, point: Point, include_far_edges: bool) -> bool {
        let (max_x, max_y) = if include_far_edges {
            (self.image_width as i64, self.image_height as i64)
        } else {
            (self.image_width as i64 - 1, self.image_height as i64 - 1)
        };
        (0..=max_x).contains(&(point.x as i64)) && (0..=max_y).contains(&(point.y as i64))
    }

    fn unscale(self, value: i32, max_native: u32) -> i32 {
        // Truncating matches warp-server's mapping (logic/ai/computer_use/screenshot_region.go).
        ((value as f64 / self.scale) as i32).min(max_native as i32)
    }

    fn out_of_bounds(self, point: Point) -> ToolError {
        format!(
            "Coordinate ({}, {}) is outside the {}x{} screenshot. Coordinates are pixels in the most \
             recent screenshot, measured from its top-left corner.",
            point.x, point.y, self.image_width, self.image_height
        )
        .into()
    }
}

#[cfg(test)]
#[path = "coords_tests.rs"]
mod tests;
