use super::{TuiClipBounds, TuiScene, TuiScreenPoint, TuiScreenRect, TuiZIndex};
use crate::EntityIdMap;
use crate::elements::tui::{TuiPaintContext, TuiPoint, TuiSize};

#[test]
fn visible_rect_intersects_the_active_clip() {
    let mut scene = TuiScene::default();
    scene.start_layer(TuiClipBounds::BoundedBy(TuiScreenRect::new(
        TuiScreenPoint::new(0, 2, TuiZIndex::Normal(0)),
        TuiSize::new(5, 2),
    )));
    let origin = TuiScreenPoint::new(0, 1, scene.z_index());

    assert_eq!(
        scene.visible_rect(origin, TuiSize::new(5, 3)),
        Some(TuiScreenRect::new(
            TuiScreenPoint::new(0, 2, scene.z_index()),
            TuiSize::new(5, 2),
        ))
    );
}

#[test]
fn higher_hit_layer_covers_lower_points() {
    let mut scene = TuiScene::default();
    let lower = TuiScreenPoint::new(1, 1, scene.z_index());
    scene.start_layer(TuiClipBounds::None);
    scene.record_hit_rect(TuiScreenRect::new(
        TuiScreenPoint::new(0, 0, scene.z_index()),
        TuiSize::new(3, 3),
    ));

    assert!(scene.is_covered(lower));
    assert!(!scene.is_covered(TuiScreenPoint::new(4, 4, TuiZIndex::Normal(0))));
}

/// Verifies that overlay layers occlude every normal layer.
#[test]
fn overlay_layer_covers_normal_layer_below_it() {
    let mut rendered_views = EntityIdMap::default();
    let mut ctx = TuiPaintContext::new(&mut rendered_views);
    ctx.with_overlay_layer(TuiClipBounds::None, |ctx| {
        ctx.scene.record_hit_rect(TuiScreenRect::new(
            TuiScreenPoint::new(0, 0, ctx.scene.z_index()),
            TuiSize::new(3, 3),
        ));
    });
    let (scene, _, _) = ctx.finish();

    assert!(scene.is_covered(TuiScreenPoint::new(1, 1, TuiZIndex::Normal(0))));
}

/// Verifies that later overlay layers occlude earlier overlays.
#[test]
fn higher_overlay_layer_covers_lower_overlay_layer() {
    let mut rendered_views = EntityIdMap::default();
    let mut ctx = TuiPaintContext::new(&mut rendered_views);
    let lower = ctx.with_overlay_layer(TuiClipBounds::None, |ctx| {
        let lower = TuiScreenPoint::new(1, 1, ctx.scene.z_index());
        ctx.with_overlay_layer(TuiClipBounds::None, |ctx| {
            ctx.scene.record_hit_rect(TuiScreenRect::new(
                TuiScreenPoint::new(0, 0, ctx.scene.z_index()),
                TuiSize::new(3, 3),
            ));
        });
        lower
    });
    let (scene, _, _) = ctx.finish();

    assert!(scene.is_covered(lower));
}

/// Verifies that click-through overlays leave lower targets exposed.
#[test]
fn click_through_overlay_does_not_cover_lower_targets() {
    let mut rendered_views = EntityIdMap::default();
    let mut ctx = TuiPaintContext::new(&mut rendered_views);
    ctx.with_overlay_layer(TuiClipBounds::None, |ctx| {
        ctx.set_active_layer_click_through();
        ctx.scene.record_hit_rect(TuiScreenRect::new(
            TuiScreenPoint::new(0, 0, ctx.scene.z_index()),
            TuiSize::new(3, 3),
        ));
    });
    let (scene, _, _) = ctx.finish();

    assert!(!scene.is_covered(TuiScreenPoint::new(1, 1, TuiZIndex::Normal(0))));
}

#[test]
fn disjoint_nested_clip_stays_empty() {
    let mut scene = TuiScene::default();
    scene.start_layer(TuiClipBounds::BoundedBy(TuiScreenRect::new(
        TuiScreenPoint::new(0, 0, TuiZIndex::Normal(0)),
        TuiSize::new(2, 2),
    )));
    scene.start_layer(TuiClipBounds::BoundedByActiveLayerAnd(TuiScreenRect::new(
        TuiScreenPoint::new(4, 4, scene.z_index()),
        TuiSize::new(2, 2),
    )));
    let origin = TuiScreenPoint::new(4, 4, scene.z_index());

    assert_eq!(scene.visible_rect(origin, TuiSize::new(2, 2)), None);
}

#[test]
fn signed_rect_contains_visible_terminal_points() {
    let rect = TuiScreenRect::new(
        TuiScreenPoint::new(-1, 0, TuiZIndex::Normal(0)),
        TuiSize::new(3, 1),
    );

    assert!(rect.contains(TuiPoint::new(0, 0)));
    assert!(!rect.contains(TuiPoint::new(2, 0)));
}

#[test]
fn terminal_cursor_prefers_higher_layers() {
    let mut rendered_views = EntityIdMap::default();
    let mut ctx = TuiPaintContext::new(&mut rendered_views);
    ctx.set_terminal_cursor(TuiScreenPoint::new(1, 0, ctx.scene.z_index()));
    ctx.scene.start_layer(TuiClipBounds::None);
    let higher = TuiScreenPoint::new(2, 0, ctx.scene.z_index());
    ctx.set_terminal_cursor(higher);
    ctx.scene.stop_layer();
    ctx.set_terminal_cursor(TuiScreenPoint::new(3, 0, ctx.scene.z_index()));

    assert_eq!(ctx.terminal_cursor(), Some(higher));
}

#[test]
fn terminal_cursor_uses_later_submission_on_the_same_layer() {
    let mut rendered_views = EntityIdMap::default();
    let mut ctx = TuiPaintContext::new(&mut rendered_views);
    let later = TuiScreenPoint::new(2, 0, ctx.scene.z_index());
    ctx.set_terminal_cursor(TuiScreenPoint::new(1, 0, ctx.scene.z_index()));
    ctx.set_terminal_cursor(later);

    assert_eq!(ctx.terminal_cursor(), Some(later));
}
