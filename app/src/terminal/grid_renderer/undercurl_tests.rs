use std::sync::Arc;

use warpui::color::ColorU;
use warpui::geometry::vector::vec2f;
use warpui::image_cache::StaticImage;

use super::{mask_draw, peak_and_trough};

fn coverage_at(image: &StaticImage, x: u32, y: u32) -> u8 {
    let bytes = image.rgba_bytes();
    let i = ((y * image.width() + x) * 4) as usize;
    bytes[i]
}

#[test]
fn peak_is_above_trough_and_period_joins_at_edges() {
    let cell_width = 10.;
    let cell_height = 16.;
    let thickness = 1.5;
    let (peak_y, trough_y) = peak_and_trough(cell_width, cell_height, thickness);

    assert!(peak_y < trough_y);
    assert!((trough_y - (cell_height - thickness * 0.5)).abs() < f32::EPSILON);
}

#[test]
fn mask_is_a_single_sprite_not_segmented_rects() {
    let draw = mask_draw(
        vec2f(0., 0.),
        12.,
        20.,
        1.,
        1.5,
        ColorU::new(255, 255, 255, 255),
        2.,
    )
    .expect("mask");
    assert!(draw.image.width() > 4);
    assert!(draw.image.height() > 4);
}

#[test]
fn mask_is_not_a_solid_bar() {
    let draw = mask_draw(
        vec2f(0., 0.),
        16.,
        24.,
        1.,
        2.,
        ColorU::new(255, 255, 255, 255),
        2.,
    )
    .expect("mask");
    let w = draw.image.width();
    let h = draw.image.height();
    let edge_x = w / 8;
    let mid_x = w / 2;
    let mut edge_max = 0u8;
    let mut mid_max = 0u8;
    let mut edge_argmax = 0u32;
    let mut mid_argmax = 0u32;
    for y in 0..h {
        let e = coverage_at(&draw.image, edge_x, y);
        let m = coverage_at(&draw.image, mid_x, y);
        if e > edge_max {
            edge_max = e;
            edge_argmax = y;
        }
        if m > mid_max {
            mid_max = m;
            mid_argmax = y;
        }
    }
    assert!(edge_max > 32, "trough column should be inked");
    assert!(mid_max > 32, "peak column should be inked");
    assert!(
        mid_argmax + 1 < edge_argmax,
        "peak should sit above the trough, got mid={mid_argmax} edge={edge_argmax}"
    );
}

#[test]
fn mask_has_antialiased_coverage_segmented_rects_would_not() {
    let draw = mask_draw(
        vec2f(0., 0.),
        16.,
        24.,
        1.,
        2.,
        ColorU::new(255, 255, 255, 255),
        2.,
    )
    .expect("mask");
    let bytes = draw.image.rgba_bytes();
    let partial = bytes
        .chunks(4)
        .filter(|px| px[0] > 0 && px[0] < 255)
        .count();
    assert!(
        partial > 10,
        "stroked cubic should antialias; got {partial} partial pixels"
    );
}

#[test]
fn mask_cache_reuses_the_same_image() {
    let a = mask_draw(
        vec2f(0., 0.),
        10.,
        16.,
        1.,
        1.5,
        ColorU::new(1, 2, 3, 255),
        1.,
    )
    .expect("a");
    let b = mask_draw(
        vec2f(40., 0.),
        10.,
        16.,
        1.,
        1.5,
        ColorU::new(9, 9, 9, 255),
        1.,
    )
    .expect("b");
    assert!(Arc::ptr_eq(&a.image, &b.image));
}

#[test]
fn wide_cell_mask_is_wider_than_a_single_period() {
    let single = mask_draw(
        vec2f(0., 0.),
        10.,
        16.,
        1.,
        1.5,
        ColorU::new(255, 255, 255, 255),
        1.,
    )
    .expect("single");
    let wide = mask_draw(
        vec2f(0., 0.),
        10.,
        16.,
        2.,
        1.5,
        ColorU::new(255, 255, 255, 255),
        1.,
    )
    .expect("wide");
    assert!(wide.logical_size.x() > single.logical_size.x() + 4.);
}
