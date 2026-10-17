use warp_core::features::FeatureFlag;

use crate::SizeInfo;
use crate::event_listener::ChannelEventListener;
use crate::model::ansi::{self, Handler};
use crate::model::blockgrid::{BlockGrid, CursorDisplayPoint};
use crate::model::grid::Dimensions;
use crate::model::grid::grid_handler::PerformResetGridChecks;
use crate::model::image_map::VirtualPlacement;
use crate::model::index::{Point, VisibleRow};
use crate::model::kitty::{
    CursorMovementPolicy, DisplayStoredImage, KittyAction, KittyPlacementData,
};
use crate::model::secrets::ObfuscateSecrets;
use crate::test_util::{
    mock_blockgrid, test_kitty_image_metadata_map, test_kitty_store_and_display_action,
};

#[test]
pub fn test_finish_truncates_grid_basic() {
    let size = SizeInfo::new_without_font_metrics(10, 7);
    let mut block_grid = BlockGrid::new(
        size,
        1000, /* max_scroll_limit */
        ChannelEventListener::new_for_test(),
        ObfuscateSecrets::No,
        PerformResetGridChecks::default(),
    );

    for c in "hello".chars() {
        block_grid.input(c);
    }
    block_grid.linefeed();
    block_grid.finish();

    assert_eq!(block_grid.len(), 2);
    assert_eq!(block_grid.grid_handler().total_rows(), 2);
    assert_eq!(block_grid.grid_handler().columns(), size.columns);
}

#[test]
pub fn test_finish_truncates_grid_cursor_at_bottom() {
    let size = SizeInfo::new_without_font_metrics(10, 7);
    let mut block_grid = BlockGrid::new(
        size,
        1000, /* max_scroll_limit */
        ChannelEventListener::new_for_test(),
        ObfuscateSecrets::No,
        PerformResetGridChecks::default(),
    );

    for _ in 0..300 {
        block_grid.input('a');
        block_grid.linefeed();
        block_grid.carriage_return();
    }

    block_grid.finish();

    assert_eq!(block_grid.len(), 300);
    assert_eq!(block_grid.grid_handler().total_rows(), 301);
    assert_eq!(block_grid.grid_handler().columns(), size.columns);
}

#[test]
pub fn test_resize_finished_block() {
    let size = SizeInfo::new_without_font_metrics(10, 7);
    let mut block_grid = BlockGrid::new(
        size,
        1000, /* max_scroll_limit */
        ChannelEventListener::new_for_test(),
        ObfuscateSecrets::No,
        PerformResetGridChecks::default(),
    );

    for _ in 0..5 {
        for c in "hello".chars() {
            block_grid.input(c);
        }

        block_grid.linefeed();
        block_grid.carriage_return();
    }

    block_grid.finish();

    assert_eq!(block_grid.len(), 5);
    assert_eq!(block_grid.grid_handler().total_rows(), 6);

    block_grid.resize(SizeInfo::new_without_font_metrics(10, 10));

    // The len and total rows of the grid should be unchanged even after resize.
    assert_eq!(block_grid.len(), 5);
    assert_eq!(block_grid.grid_handler().total_rows(), 6);

    // Resize so that the contents of can no longer fit on one line. The length of the grid
    // should increase because each row is now soft-wrapped.
    block_grid.resize(SizeInfo::new_without_font_metrics(10, 3));
    assert_eq!(block_grid.len(), 10);
    // Each of the 5 "hello" rows should be split into "hel" and "lo" (for a total
    // of 10 rows), plus one final empty row at the end.
    assert_eq!(block_grid.grid_handler().total_rows(), 11);

    // Resizing the height in either direction should have no effect on the total grid height.
    block_grid.resize(SizeInfo::new_without_font_metrics(3, 7));
    assert_eq!(block_grid.len(), 5);
    assert_eq!(block_grid.grid_handler().total_rows(), 6);

    block_grid.resize(SizeInfo::new_without_font_metrics(300, 7));
    assert_eq!(block_grid.len(), 5);
    assert_eq!(block_grid.grid_handler().total_rows(), 6);
}

#[test]
pub fn test_resize_finished_softwrapped_block() {
    let size = SizeInfo::new_without_font_metrics(10, 3);
    let mut block_grid = BlockGrid::new(
        size,
        1000, /* max_scroll_limit */
        ChannelEventListener::new_for_test(),
        ObfuscateSecrets::No,
        PerformResetGridChecks::default(),
    );

    for _ in 0..5 {
        for c in "hello".chars() {
            block_grid.input(c);
        }

        block_grid.linefeed();
        block_grid.carriage_return();
    }

    block_grid.finish();

    assert_eq!(block_grid.len(), 10);
    assert_eq!(block_grid.grid_handler().total_rows(), 11);

    // Resize so each item can only fit on a given line.
    block_grid.resize(SizeInfo::new_without_font_metrics(10, 10));

    assert_eq!(block_grid.len(), 5);
    assert_eq!(block_grid.grid_handler().total_rows(), 6);
}

#[test]
pub fn test_content_summary() {
    let blockgrid = mock_blockgrid("1\r\n2\r\n3\r\n4\r\n5\r\n6\r\n7\r\n8\r\n9\r\n10\r\n");

    // Test a summary that omits the middle of the grid.
    let summary = blockgrid.content_summary(1, 2, false);
    assert_eq!(summary, "1\n...(truncated)...\n9\n10\n");

    // Test a summary that perfectly covers the entire grid.
    let summary = blockgrid.content_summary(7, 3, false);
    assert_eq!(summary, "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n");

    // Test a summary where the total range exceeds the grid size.
    let summary = blockgrid.content_summary(7, 6, false);
    assert_eq!(summary, "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n");
}

#[test]
pub fn test_trim_trailing_blank_rows_uses_active_floor_for_blank_started_grid() {
    let size = SizeInfo::new_without_font_metrics(10, 10);
    let mut block_grid = BlockGrid::new(
        size,
        1000,
        ChannelEventListener::new_for_test(),
        ObfuscateSecrets::No,
        PerformResetGridChecks::default(),
    );

    block_grid.start();
    block_grid.goto(VisibleRow(4), 0);
    block_grid.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));
    block_grid.set_trim_trailing_blank_rows(true);

    assert_eq!(block_grid.len(), 5);
    assert_eq!(block_grid.len_displayed(), 1);
    assert_eq!(
        block_grid.cursor_display_point(),
        Some(CursorDisplayPoint::HiddenCache(Point::new(0, 0)))
    );
}

#[test]
pub fn test_non_moving_kitty_image_keeps_finished_grid_visible() {
    let _kitty_images = FeatureFlag::KittyImages.override_enabled(true);
    let size = SizeInfo::new_without_font_metrics(10, 10);
    let mut block_grid = BlockGrid::new(
        size,
        1000,
        ChannelEventListener::new_for_test(),
        ObfuscateSecrets::No,
        PerformResetGridChecks::default(),
    );
    let mut metadata = test_kitty_image_metadata_map(1);
    let mut action = test_kitty_store_and_display_action(1, 1);
    let KittyAction::StoreAndDisplay(store_and_display) = &mut action else {
        panic!("expected StoreAndDisplay action");
    };
    store_and_display.placement_data.cursor_movement_policy = CursorMovementPolicy::DoNotMoveCursor;

    block_grid
        .handle_completed_kitty_action(action, &mut metadata)
        .expect("kitty action should be handled")
        .expect("kitty image should render");

    assert_eq!(block_grid.grid_handler().cursor_point(), Point::new(0, 0));
    assert!(block_grid.grid_handler().has_visible_images());
    assert!(!block_grid.should_show_as_empty_when_finished());
}

#[test]
pub fn test_virtual_kitty_placement_is_recorded_without_placing_or_moving_cursor() {
    let _kitty_images = FeatureFlag::KittyImages.override_enabled(true);
    let size = SizeInfo::new_without_font_metrics(10, 10);
    let mut block_grid = BlockGrid::new(
        size,
        1000,
        ChannelEventListener::new_for_test(),
        ObfuscateSecrets::No,
        PerformResetGridChecks::default(),
    );
    let mut metadata = test_kitty_image_metadata_map(1);
    let mut action = test_kitty_store_and_display_action(1, 7);
    let KittyAction::StoreAndDisplay(store_and_display) = &mut action else {
        panic!("expected StoreAndDisplay action");
    };
    store_and_display.placement_data.unicode_placeholder = true;

    block_grid
        .handle_completed_kitty_action(action, &mut metadata)
        .expect("kitty action should be handled")
        .expect("virtual placement should be accepted");

    let grid = block_grid.grid_handler();
    assert_eq!(grid.cursor_point(), Point::new(0, 0));
    assert!(!grid.has_visible_images());
    assert_eq!(
        grid.virtual_image_placement(1),
        Some(VirtualPlacement {
            placement_id: 7,
            cols: 1,
            rows: 1
        })
    );

    block_grid.grid_handler_mut().evict_image(1);
    assert_eq!(block_grid.grid_handler().virtual_image_placement(1), None);
}

#[test]
pub fn test_virtual_kitty_placement_of_stored_image_is_deleted_by_its_placement_id() {
    let _kitty_images = FeatureFlag::KittyImages.override_enabled(true);
    let size = SizeInfo::new_without_font_metrics(10, 10);
    let mut block_grid = BlockGrid::new(
        size,
        1000,
        ChannelEventListener::new_for_test(),
        ObfuscateSecrets::No,
        PerformResetGridChecks::default(),
    );
    let mut metadata = test_kitty_image_metadata_map(1);
    for placement_id in [7, 8] {
        let action = KittyAction::DisplayStoredImage(DisplayStoredImage {
            placement_data: KittyPlacementData {
                cols: Some(2),
                rows: Some(3),
                unicode_placeholder: true,
                ..Default::default()
            },
            image_id: 1,
            placement_id,
        });
        block_grid
            .handle_completed_kitty_action(action, &mut metadata)
            .expect("kitty action should be handled")
            .expect("virtual placement should be accepted");
    }

    // The newest placement is the one placeholder cells show.
    let placement_8 = Some(VirtualPlacement {
        placement_id: 8,
        cols: 2,
        rows: 3,
    });
    let grid = block_grid.grid_handler();
    assert_eq!(grid.cursor_point(), Point::new(0, 0));
    assert!(!grid.has_visible_images());
    assert_eq!(grid.virtual_image_placement(1), placement_8);

    block_grid.grid_handler_mut().evict_placement(1, 7);
    assert_eq!(
        block_grid.grid_handler().virtual_image_placement(1),
        placement_8
    );
    block_grid.grid_handler_mut().evict_placement(1, 8);
    assert_eq!(block_grid.grid_handler().virtual_image_placement(1), None);
}

#[test]
pub fn test_virtual_kitty_placement_box_follows_columns_and_rows() {
    let _kitty_images = FeatureFlag::KittyImages.override_enabled(true);
    let size = SizeInfo::new_without_font_metrics(10, 10);
    let mut block_grid = BlockGrid::new(
        size,
        1000,
        ChannelEventListener::new_for_test(),
        ObfuscateSecrets::No,
        PerformResetGridChecks::default(),
    );
    // A 10 x 10 pixel image, in cells of 1 x 1 pixel.
    let mut metadata = test_kitty_image_metadata_map(1);
    for ((cols, rows), expected) in [
        ((Some(4), Some(2)), (4, 2)),
        ((Some(4), None), (4, 4)),
        ((None, Some(3)), (3, 3)),
        ((None, None), (10, 10)),
    ] {
        let action = KittyAction::DisplayStoredImage(DisplayStoredImage {
            placement_data: KittyPlacementData {
                cols,
                rows,
                unicode_placeholder: true,
                ..Default::default()
            },
            image_id: 1,
            placement_id: 7,
        });
        block_grid
            .handle_completed_kitty_action(action, &mut metadata)
            .expect("kitty action should be handled")
            .expect("virtual placement should be accepted");

        let placement = block_grid.grid_handler().virtual_image_placement(1);
        assert_eq!(
            placement.map(|placement| (placement.cols, placement.rows)),
            Some(expected),
            "c={cols:?}, r={rows:?}"
        );
    }
}

#[test]
pub fn test_cursor_display_point_hidden_when_cursor_below_trimmed_content() {
    let size = SizeInfo::new_without_font_metrics(10, 10);
    let mut block_grid = BlockGrid::new(
        size,
        1000,
        ChannelEventListener::new_for_test(),
        ObfuscateSecrets::No,
        PerformResetGridChecks::default(),
    );

    block_grid.start();
    block_grid.input('a');
    block_grid.input('b');
    block_grid.input('c');
    block_grid.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));
    block_grid.set_trim_trailing_blank_rows(true);

    assert_eq!(block_grid.len_displayed(), 1);
    assert_eq!(
        block_grid.cursor_display_point(),
        Some(CursorDisplayPoint::Visible(Point::new(0, 3)))
    );

    block_grid.goto(VisibleRow(4), 2);
    block_grid.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));

    assert_eq!(block_grid.len(), 5);
    assert_eq!(block_grid.len_displayed(), 1);
    assert_eq!(
        block_grid.cursor_display_point(),
        Some(CursorDisplayPoint::HiddenCache(Point::new(0, 2)))
    );
}

#[test]
pub fn test_cursor_display_point_not_clipped_when_trimming_disabled() {
    let size = SizeInfo::new_without_font_metrics(10, 10);
    let mut block_grid = BlockGrid::new(
        size,
        1000,
        ChannelEventListener::new_for_test(),
        ObfuscateSecrets::No,
        PerformResetGridChecks::default(),
    );

    block_grid.start();
    block_grid.input('a');
    block_grid.input('b');
    block_grid.input('c');
    block_grid
        .grid_handler_mut()
        .set_marked_text("12345678", &(0..0));

    assert_eq!(block_grid.len_displayed(), 1);
    assert_eq!(
        block_grid.cursor_display_point(),
        Some(CursorDisplayPoint::Visible(Point::new(1, 1)))
    );

    block_grid.set_trim_trailing_blank_rows(true);

    assert_eq!(block_grid.len_displayed(), 1);
    assert_eq!(
        block_grid.cursor_display_point(),
        Some(CursorDisplayPoint::HiddenCache(Point::new(0, 1)))
    );
}
