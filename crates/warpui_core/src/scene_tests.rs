use pathfinder_geometry::vector::vec2f;

use super::*;
use crate::image_cache::test_utils::make_static_image;
use crate::rendering;

#[test]
fn test_hit_rect_recording() {
    let mut scene = Scene::new(1., rendering::Config::default());
    assert_eq!(ZIndex::new(0), scene.z_index());

    scene.draw_rect_with_hit_recording(RectF::new(vec2f(0., 0.), vec2f(100., 100.)));
    assert_eq!(ZIndex::new(0), scene.z_index());
    assert!(!scene.is_covered(Point::new(10., 10., ZIndex::new(0))));

    scene.start_layer(ClipBounds::ActiveLayer);
    scene.draw_rect_with_hit_recording(RectF::new(vec2f(50., 50.), vec2f(100., 100.)));
    assert_eq!(ZIndex::new(1), scene.z_index());
    assert!(!scene.is_covered(Point::new(10., 10., ZIndex::new(0))));
    assert!(scene.is_covered(Point::new(60., 60., ZIndex::new(0))));

    scene.start_layer(ClipBounds::ActiveLayer);
    scene.draw_rect_with_hit_recording(RectF::new(vec2f(0., 0.), vec2f(100., 100.)));
    assert_eq!(ZIndex::new(2), scene.z_index());
    assert!(scene.is_covered(Point::new(10., 10., ZIndex::new(0))));
    assert!(scene.is_covered(Point::new(60., 60., ZIndex::new(1))));
}

#[test]
fn test_nested_clip_bounds_with_intersection() {
    let mut scene = Scene::new(1., rendering::Config::default());

    let bounds1 = RectF::new(Vector2F::zero(), Vector2F::new(10., 10.));
    scene.start_layer(ClipBounds::BoundedBy(bounds1));

    let bounds2 = RectF::new(Vector2F::new(5., 5.), Vector2F::new(10., 10.));
    scene.start_layer(ClipBounds::BoundedByActiveLayerAnd(bounds2));

    assert_eq!(
        scene.active_layer().clip_bounds,
        Some(RectF::new(Vector2F::new(5., 5.), Vector2F::new(5., 5.)))
    );
}

#[test]
fn test_nested_clip_bounds_with_no_intersection() {
    let mut scene = Scene::new(1., rendering::Config::default());

    let bounds1 = RectF::new(Vector2F::zero(), Vector2F::new(10., 10.));
    scene.start_layer(ClipBounds::BoundedBy(bounds1));

    let bounds2 = RectF::new(Vector2F::new(100., 100.), Vector2F::new(10., 10.));
    scene.start_layer(ClipBounds::BoundedByActiveLayerAnd(bounds2));

    // We should have explicit bounds for this layer.  (None represents a lack
    // of clipping, not clipping they layer down to nothingness.)
    assert!(scene.active_layer().clip_bounds.is_some());
    // The clip bounds should have an area of zero.
    assert!(scene.active_layer().clip_bounds.unwrap().is_empty());
}

#[test]
fn test_click_through_layer_does_not_cover_lower_layers() {
    let mut scene = Scene::new(1., rendering::Config::default());

    scene.start_layer(ClipBounds::ActiveLayer);
    scene.set_active_layer_click_through();
    scene.draw_rect_with_hit_recording(RectF::new(vec2f(0., 0.), vec2f(100., 100.)));

    assert!(!scene.is_covered(Point::new(10., 10., ZIndex::new(0))));
}

#[test]
fn test_click_through_layer_with_image_does_not_cover_lower_layers() {
    let mut scene = Scene::new(1., rendering::Config::default());

    // The layer an element such as the terminal grid paints into.
    scene.draw_rect_with_hit_recording(RectF::new(vec2f(0., 0.), vec2f(100., 100.)));
    let grid_z_index = scene.z_index();

    // An image painted on top of it, on a click-through layer.
    scene.start_layer(ClipBounds::ActiveLayer);
    scene.set_active_layer_click_through();
    scene.draw_image(
        RectF::new(vec2f(20., 20.), vec2f(40., 40.)),
        make_static_image(40, 40),
        1.,
        CornerRadius::default(),
    );

    // A point inside the image's bounds still reaches the layer below it.
    assert!(!scene.is_covered(Point::new(30., 30., grid_z_index)));
}

#[test]
fn test_image_on_regular_layer_covers_lower_layers() {
    let mut scene = Scene::new(1., rendering::Config::default());

    scene.draw_rect_with_hit_recording(RectF::new(vec2f(0., 0.), vec2f(100., 100.)));
    let grid_z_index = scene.z_index();

    scene.start_layer(ClipBounds::ActiveLayer);
    scene.draw_image(
        RectF::new(vec2f(20., 20.), vec2f(40., 40.)),
        make_static_image(40, 40),
        1.,
        CornerRadius::default(),
    );

    assert!(scene.is_covered(Point::new(30., 30., grid_z_index)));
    assert!(!scene.is_covered(Point::new(80., 80., grid_z_index)));
}
