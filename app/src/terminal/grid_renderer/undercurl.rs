//! Cached monochrome undercurl sprite, tinted at draw time.
//!
//! Warp's scene has no stroked-path primitive, so the curve is rasterized
//! into an alpha mask and drawn with `draw_icon`.

use std::collections::HashMap;
use std::sync::Arc;

use image::{Rgba, RgbaImage};
use parking_lot::Mutex;
use warpui::color::ColorU;
use warpui::geometry::vector::{Vector2F, vec2f};
use warpui::image_cache::StaticImage;

/// Curvature of the two cubics that form one period.
const CURLY_CURVATURE: f32 = 0.4;
const SAMPLES_PER_CUBIC: usize = 32;

#[derive(Clone, Copy, Hash, Eq, PartialEq)]
struct UndercurlKey {
    cell_px: u32,
    thickness_px: u32,
    periods: u32,
}

#[derive(Clone)]
struct CachedMask {
    image: Arc<StaticImage>,
    logical_size: Vector2F,
    origin_offset: Vector2F,
}

static UNDERCURL_MASKS: Mutex<Option<HashMap<UndercurlKey, CachedMask>>> = Mutex::new(None);

pub(super) struct UndercurlDraw {
    pub origin: Vector2F,
    pub logical_size: Vector2F,
    pub image: Arc<StaticImage>,
    pub color: ColorU,
}

pub(super) fn peak_and_trough(cell_width: f32, cell_height: f32, thickness: f32) -> (f32, f32) {
    let amplitude = cell_width / std::f32::consts::PI;
    let trough_y = cell_height - thickness * 0.5;
    let peak_y = (trough_y - amplitude).max(thickness * 0.5);
    (peak_y, trough_y)
}

pub(super) fn mask_draw(
    cell_origin: Vector2F,
    cell_width: f32,
    cell_height: f32,
    column_span: f32,
    thickness: f32,
    color: ColorU,
    scale_factor: f32,
) -> Option<UndercurlDraw> {
    let thickness = thickness.max(1.);
    let periods = column_span.max(1.).round() as u32;
    let cell_px = (cell_width * scale_factor).round().max(1.) as u32;
    let thickness_px = (thickness * scale_factor).round().max(1.) as u32;
    let key = UndercurlKey {
        cell_px,
        thickness_px,
        periods,
    };

    let cached = {
        let mut guard = UNDERCURL_MASKS.lock();
        let cache = guard.get_or_insert_with(HashMap::new);
        if let Some(cached) = cache.get(&key) {
            cached.clone()
        } else {
            let cached = rasterize_mask(cell_width, cell_height, thickness, periods, scale_factor)?;
            cache.insert(key, cached.clone());
            cached
        }
    };

    Some(UndercurlDraw {
        origin: cell_origin + cached.origin_offset,
        logical_size: cached.logical_size,
        image: cached.image,
        color,
    })
}

fn rasterize_mask(
    cell_width: f32,
    cell_height: f32,
    thickness: f32,
    periods: u32,
    scale_factor: f32,
) -> Option<CachedMask> {
    let (peak_y, trough_y) = peak_and_trough(cell_width, cell_height, thickness);
    let radius = thickness * 0.5;
    let pad = radius.max(1.);
    let span_width = cell_width * periods as f32;
    let mask_x0 = -pad;
    let mask_y0 = peak_y - radius - pad;
    let mask_width = span_width + pad * 2.;
    let mask_height = (trough_y + radius + pad) - mask_y0;
    if mask_width <= 0. || mask_height <= 0. {
        return None;
    }

    let px_w = (mask_width * scale_factor).round().max(1.) as u32;
    let px_h = (mask_height * scale_factor).round().max(1.) as u32;
    let points = stroked_centerline(cell_width, peak_y, trough_y, periods);
    let radius_px = radius * scale_factor;
    let mut img = RgbaImage::new(px_w, px_h);

    for y in 0..px_h {
        for x in 0..px_w {
            let lx = mask_x0 + (x as f32 + 0.5) / scale_factor;
            let ly = mask_y0 + (y as f32 + 0.5) / scale_factor;
            let dist = min_distance_to_polyline(lx, ly, &points);
            let coverage = ((radius_px + 0.5 - dist * scale_factor).clamp(0., 1.) * 255.) as u8;
            img.put_pixel(x, y, Rgba([coverage, 0, 0, coverage]));
        }
    }

    Some(CachedMask {
        image: Arc::new(StaticImage::from_rgba(img)),
        logical_size: vec2f(mask_width, mask_height),
        origin_offset: vec2f(mask_x0, mask_y0),
    })
}

fn stroked_centerline(cell_width: f32, peak_y: f32, trough_y: f32, periods: u32) -> Vec<[f32; 2]> {
    let mut points = Vec::with_capacity(periods as usize * SAMPLES_PER_CUBIC * 2 + 1);
    for period in 0..periods {
        let x0 = period as f32 * cell_width;
        append_period_cubics(x0, cell_width, peak_y, trough_y, &mut points);
    }
    points
}

fn append_period_cubics(
    x0: f32,
    width: f32,
    peak_y: f32,
    trough_y: f32,
    points: &mut Vec<[f32; 2]>,
) {
    let center = 0.5 * width;
    let r = CURLY_CURVATURE;
    let start = [x0, trough_y];
    if points.last() != Some(&start) {
        points.push(start);
    }
    sample_cubic(
        start,
        [x0 + center * r, trough_y],
        [x0 + center - center * r, peak_y],
        [x0 + center, peak_y],
        points,
    );
    sample_cubic(
        [x0 + center, peak_y],
        [x0 + center + center * r, peak_y],
        [x0 + width - center * r, trough_y],
        [x0 + width, trough_y],
        points,
    );
}

fn sample_cubic(
    p0: [f32; 2],
    p1: [f32; 2],
    p2: [f32; 2],
    p3: [f32; 2],
    points: &mut Vec<[f32; 2]>,
) {
    for i in 1..=SAMPLES_PER_CUBIC {
        let t = i as f32 / SAMPLES_PER_CUBIC as f32;
        points.push(cubic_point(p0, p1, p2, p3, t));
    }
}

fn cubic_point(p0: [f32; 2], p1: [f32; 2], p2: [f32; 2], p3: [f32; 2], t: f32) -> [f32; 2] {
    let u = 1. - t;
    let uu = u * u;
    let uuu = uu * u;
    let tt = t * t;
    let ttt = tt * t;
    [
        uuu * p0[0] + 3. * uu * t * p1[0] + 3. * u * tt * p2[0] + ttt * p3[0],
        uuu * p0[1] + 3. * uu * t * p1[1] + 3. * u * tt * p2[1] + ttt * p3[1],
    ]
}

fn min_distance_to_polyline(x: f32, y: f32, points: &[[f32; 2]]) -> f32 {
    let mut min_d = f32::MAX;
    for window in points.windows(2) {
        min_d = min_d.min(distance_to_segment(x, y, window[0], window[1]));
    }
    min_d
}

fn distance_to_segment(x: f32, y: f32, a: [f32; 2], b: [f32; 2]) -> f32 {
    let abx = b[0] - a[0];
    let aby = b[1] - a[1];
    let len2 = abx * abx + aby * aby;
    let t = if len2 == 0. {
        0.
    } else {
        ((x - a[0]) * abx + (y - a[1]) * aby) / len2
    }
    .clamp(0., 1.);
    let px = a[0] + t * abx - x;
    let py = a[1] + t * aby - y;
    (px * px + py * py).sqrt()
}

#[cfg(test)]
#[path = "undercurl_tests.rs"]
mod tests;
