use warpui_core::App;

use super::*;
use crate::render::model::RenderState;
use crate::render::model::test_utils::{TEST_STYLES, mock_paragraph};

#[test]
fn a_live_model_is_counted_with_its_content_size() {
    App::test((), |mut app| async move {
        let render_state = app.add_model(|ctx| RenderState::new(TEST_STYLES, false, None, ctx));

        let stats = app.read(live_render_state_stats);

        // A fresh pixel-mode model holds the trailing-newline block: one item, one line, one
        // character. It lands in the first bucket, which is `<= 10` items.
        assert_eq!(stats.live_pixel_models, 1);
        assert_eq!(stats.live_char_cell_models, 0);
        assert_eq!(stats.unresolved_entries, 0);
        assert_eq!(stats.total_items, 1);
        assert_eq!(stats.total_lines, 1);
        assert_eq!(stats.total_chars, 1);
        assert_eq!(stats.largest_model_items, 1);
        assert_eq!(stats.largest_model_lines, 1);
        assert_eq!(stats.models_by_item_count[0], 1);
        assert_eq!(stats.models_above_largest_bucket, 0);

        drop(render_state);
    });
}

#[test]
fn a_dropped_model_is_not_counted_as_live() {
    App::test((), |mut app| async move {
        let render_state = app.add_model(|ctx| RenderState::new(TEST_STYLES, false, None, ctx));
        assert_eq!(app.read(live_render_state_stats).live_pixel_models, 1);

        drop(render_state);

        let stats = app.read(live_render_state_stats);
        assert_eq!(stats.live_pixel_models, 0);
        // Its entry stays in the registry and is reported as unresolved rather than removed, so any
        // shortfall in the live count is visible in the payload.
        assert_eq!(stats.unresolved_entries, 1);
        assert_eq!(registry_len(), 1);
    });
}

#[test]
fn a_model_being_updated_is_counted_again_afterwards() {
    App::test((), |mut app| async move {
        let render_state = app.add_model(|ctx| RenderState::new(TEST_STYLES, false, None, ctx));

        // A model is out of the model map for the whole of its own update, so it cannot be read
        // from inside one. That has to leave the registry intact: treating the failure as death
        // would drop a live model permanently and undercount it for the rest of the process.
        let stats_during_update =
            render_state.update(&mut app, |_, ctx| live_render_state_stats(ctx));
        assert_eq!(stats_during_update.live_pixel_models, 0);
        assert_eq!(stats_during_update.unresolved_entries, 1);

        let stats_after_update = app.read(live_render_state_stats);
        assert_eq!(
            stats_after_update.live_pixel_models, 1,
            "a model unreadable during its own update must still be counted afterwards"
        );
        assert_eq!(stats_after_update.unresolved_entries, 0);

        drop(render_state);
    });
}

#[test]
fn larger_trees_land_in_larger_buckets() {
    App::test((), |mut app| async move {
        // 12 items exceeds the first bucket's bound of 10, so this model is counted one bucket up.
        let render_state = app.add_model(|ctx| {
            let mut render_state = RenderState::new(TEST_STYLES, false, None, ctx);
            let mut content = sum_tree::SumTree::new();
            for _ in 0..12 {
                content.push(mock_paragraph(24., 1., 5));
            }
            render_state.set_content(content);
            render_state
        });

        let stats = app.read(live_render_state_stats);

        assert_eq!(stats.models_by_item_count[0], 0);
        assert_eq!(stats.models_by_item_count[1], 1);
        assert!(
            stats.largest_model_items >= 12,
            "expected at least the 12 pushed items, got {stats:?}"
        );
        assert!(
            stats.largest_model_lines >= 12,
            "each pushed paragraph is a line, got {stats:?}"
        );

        drop(render_state);
    });
}
