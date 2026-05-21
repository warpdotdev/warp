use warp_core::semantic_selection::SemanticSelection;
use warpui::App;

use crate::terminal::{
    block_filter::BlockFilterQuery,
    event_listener::ChannelEventListener,
    model::{
        block::SerializedBlock,
        blocks::BlockListPoint,
        index::{Point, Side},
        terminal_model::WithinBlock,
    },
    shared_session::tests::terminal_model_for_viewer,
    GridType, SizeInfo, SizeUpdate, SizeUpdateReason, TerminalModel,
};
use warpui::text::SelectionType;

/// Creates a [`SelectionType::Simple`], left-to-right text selection
/// from `start` to `end` in the `model`'s blocklist.
fn create_simple_text_selection(
    model: &mut TerminalModel,
    start: WithinBlock<Point>,
    end: WithinBlock<Point>,
) {
    let start_block_point = BlockListPoint::from_within_block_point(&start, model.block_list());
    let end_block_point = BlockListPoint::from_within_block_point(&end, model.block_list());
    model
        .block_list_mut()
        .start_selection(start_block_point, SelectionType::Simple, Side::Left);
    model
        .block_list_mut()
        .update_selection(end_block_point, Side::Right);
}

fn create_sharer_and_viewer_models_with_same_block(
    input: &str,
    output: &str,
) -> (TerminalModel, TerminalModel) {
    let mut sharer_model = TerminalModel::mock(None, None);
    sharer_model.simulate_block(input, output);

    let channel_event_proxy = ChannelEventListener::new_for_test();
    let mut viewer_model = terminal_model_for_viewer(channel_event_proxy);
    let block = sharer_model.block_list().last_non_hidden_block().unwrap();
    let serialized_block = SerializedBlock::from(block);
    viewer_model.load_shared_session_scrollback(&[
        serialized_block,
        SerializedBlock::new_active_block_for_test(),
    ]);

    assert_eq!(
        viewer_model
            .block_list()
            .last_non_hidden_block()
            .unwrap()
            .output_grid()
            .contents_to_string(true, None)
            .trim(),
        output,
    );

    (sharer_model, viewer_model)
}

#[test]
fn test_selections_across_different_filtered_blocklists() {
    App::test((), |app| async move {
        app.read(|ctx| {
            // Create a model for sharer / viewer with the following block:
            // the command grid in this test should simply be "ls", and
            // the output grid should look like
            // ```
            // foo
            // bar
            // baz
            // ```
            let semantic_selection = SemanticSelection::mock(false, "");
            let (mut sharer_model, mut viewer_model) =
                create_sharer_and_viewer_models_with_same_block("ls", "foo\r\nbar\r\nbaz");

            // Suppose the sharer filters the block down with filter="ba".
            let block_index = sharer_model
                .block_list()
                .last_non_hidden_block_by_index()
                .unwrap();
            sharer_model.update_filter_on_block(
                block_index,
                BlockFilterQuery::new_for_test(String::from("ba")),
            );
            assert_eq!(
                sharer_model
                    .block_list()
                    .num_matched_lines_in_filter_for_block(block_index),
                Some(2)
            );

            // Now, suppose the sharer selects "bar", which is on the first row of the output grid, post-filter.
            create_simple_text_selection(
                &mut sharer_model,
                WithinBlock {
                    block_index,
                    grid: GridType::Output,
                    inner: Point::new(0, 0),
                },
                WithinBlock {
                    block_index,
                    grid: GridType::Output,
                    inner: Point::new(0, 2),
                },
            );
            assert_eq!(
                sharer_model
                    .selection_to_string(&semantic_selection, false, ctx)
                    .unwrap(),
                "bar"
            );

            // Convert the selection to session-sharing-compatible points
            // that the viewer can apply locally.
            let (start, end, _) = sharer_model
                .block_list()
                .text_selection_range(&semantic_selection, false)
                .unwrap();
            let start_converted = start
                .to_session_sharing_block_point(sharer_model.block_list())
                .unwrap();
            let end_converted = end
                .to_session_sharing_block_point(sharer_model.block_list())
                .unwrap();

            // Suppose the viewer receives these points; convert them to local points.
            let viewer_start = WithinBlock::<Point>::from_session_sharing_block_point(
                start_converted.clone(),
                viewer_model.block_list(),
            )
            .unwrap();
            let viewer_end = WithinBlock::<Point>::from_session_sharing_block_point(
                end_converted.clone(),
                viewer_model.block_list(),
            )
            .unwrap();
            create_simple_text_selection(&mut viewer_model, viewer_start, viewer_end);
            assert_eq!(
                viewer_model
                    .selection_to_string(&semantic_selection, false, ctx)
                    .unwrap(),
                "bar"
            );

            // Even if the viewer filters further, the selection should still be stable.
            let block_index = viewer_model
                .block_list()
                .last_non_hidden_block_by_index()
                .unwrap();
            viewer_model.block_list_mut().clear_selection();
            viewer_model.update_filter_on_block(
                block_index,
                BlockFilterQuery::new_for_test(String::from("ba")),
            );

            let viewer_start = WithinBlock::<Point>::from_session_sharing_block_point(
                start_converted,
                viewer_model.block_list(),
            )
            .unwrap();
            let viewer_end = WithinBlock::<Point>::from_session_sharing_block_point(
                end_converted,
                viewer_model.block_list(),
            )
            .unwrap();
            create_simple_text_selection(&mut viewer_model, viewer_start, viewer_end);
            assert_eq!(
                viewer_model
                    .selection_to_string(&semantic_selection, false, ctx)
                    .unwrap(),
                "bar"
            );
        })
    })
}

#[test]
fn test_selection_of_undisplayed_row() {
    App::test((), |app| async move {
        app.read(|ctx| {
            // Create a model for the sharer and create the following block:
            // the command grid in this test should simply be "ls", and
            // the output grid should look like
            // ```
            // foo
            // bar
            // baz
            // ```
            let semantic_selection = SemanticSelection::mock(false, "");
            let (mut sharer_model, mut viewer_model) =
                create_sharer_and_viewer_models_with_same_block("ls", "foo\r\nbar\r\nbaz");

            // Suppose the viewer filters down to lines with "ba".
            let block_index = viewer_model
                .block_list()
                .last_non_hidden_block_by_index()
                .unwrap();
            viewer_model.block_list_mut().clear_selection();
            viewer_model.update_filter_on_block(
                block_index,
                BlockFilterQuery::new_for_test(String::from("bar")),
            );

            // Suppose the sharer selects "foo".
            let block_index = sharer_model
                .block_list()
                .last_non_hidden_block_by_index()
                .unwrap();
            create_simple_text_selection(
                &mut sharer_model,
                WithinBlock {
                    block_index,
                    grid: GridType::Output,
                    inner: Point::new(0, 0),
                },
                WithinBlock {
                    block_index,
                    grid: GridType::Output,
                    inner: Point::new(0, 2),
                },
            );
            assert_eq!(
                sharer_model
                    .selection_to_string(&semantic_selection, false, ctx)
                    .unwrap(),
                "foo"
            );

            // Convert the selection to session-sharing-compatible points
            // that the viewer can apply locally.
            let (start, end, _) = sharer_model
                .block_list()
                .text_selection_range(&semantic_selection, false)
                .unwrap();
            let start_converted = start
                .to_session_sharing_block_point(sharer_model.block_list())
                .unwrap();
            let end_converted = end
                .to_session_sharing_block_point(sharer_model.block_list())
                .unwrap();

            // Suppose the viewer receives these points and tries to convert them to local points.
            let viewer_start = WithinBlock::<Point>::from_session_sharing_block_point(
                start_converted.clone(),
                viewer_model.block_list(),
            );
            let viewer_end = WithinBlock::<Point>::from_session_sharing_block_point(
                end_converted.clone(),
                viewer_model.block_list(),
            );

            // These points shouldn't exist because the viewer has a filter that
            // excludes "foo" from the output grid.
            assert!(viewer_start.is_none());
            assert!(viewer_end.is_none());
        })
    })
}

#[test]
fn test_selections_from_larger_grid_to_smaller_grid() {
    App::test((), |app| async move {
        app.read(|ctx| {
            let semantic_selection = SemanticSelection::mock(false, "");
            let (mut sharer_model, mut viewer_model) =
                create_sharer_and_viewer_models_with_same_block(
                    "ls",
                    "\
                    this is some long line\r\n\
                    short line\r\n\
                    this is another long line\r\n\
                    short line 2",
                );

            // Make sure the viewer only has 10 columns, while the sharer has 50.
            let sharer_update = SizeUpdate {
                update_reason: SizeUpdateReason::Refresh,
                last_size: *sharer_model.block_list().size(),
                new_size: SizeInfo::new_without_font_metrics(100, 50),
                new_gap_height: None,
                natural_rows: 100,
                natural_cols: 50,
            };
            sharer_model.block_list_mut().resize(&sharer_update, true);

            let viewer_update = SizeUpdate {
                update_reason: SizeUpdateReason::Refresh,
                last_size: *viewer_model.block_list().size(),
                new_size: SizeInfo::new_without_font_metrics(100, 10),
                new_gap_height: None,
                natural_rows: 100,
                natural_cols: 10,
            };
            viewer_model.block_list_mut().resize(&viewer_update, true);

            // Suppose the sharer selects "line" on the third line.
            let block_index = sharer_model
                .block_list()
                .last_non_hidden_block_by_index()
                .unwrap();
            create_simple_text_selection(
                &mut sharer_model,
                WithinBlock {
                    block_index,
                    grid: GridType::Output,
                    inner: Point::new(2, 21),
                },
                WithinBlock {
                    block_index,
                    grid: GridType::Output,
                    inner: Point::new(2, 25),
                },
            );
            assert_eq!(
                sharer_model
                    .selection_to_string(&semantic_selection, false, ctx)
                    .unwrap(),
                "line"
            );

            // Convert the selection to session-sharing-compatible points
            // that the viewer can apply locally.
            let (start, end, _) = sharer_model
                .block_list()
                .text_selection_range(&semantic_selection, false)
                .unwrap();
            let start_converted = start
                .to_session_sharing_block_point(sharer_model.block_list())
                .unwrap();
            let end_converted = end
                .to_session_sharing_block_point(sharer_model.block_list())
                .unwrap();

            // Suppose the viewer receives these points and tries to convert them to local points.
            let viewer_start = WithinBlock::<Point>::from_session_sharing_block_point(
                start_converted,
                viewer_model.block_list(),
            )
            .unwrap();
            let viewer_end = WithinBlock::<Point>::from_session_sharing_block_point(
                end_converted,
                viewer_model.block_list(),
            )
            .unwrap();
            create_simple_text_selection(&mut viewer_model, viewer_start, viewer_end);
            assert_eq!(
                viewer_model
                    .selection_to_string(&semantic_selection, false, ctx)
                    .unwrap(),
                "line"
            );
        })
    })
}

#[test]
fn test_selections_from_smaller_grid_to_larger_grid() {
    App::test((), |app| async move {
        app.read(|ctx| {
            let semantic_selection = SemanticSelection::mock(false, "");
            let (mut sharer_model, mut viewer_model) =
                create_sharer_and_viewer_models_with_same_block(
                    "ls",
                    "\
                this is some long line\r\n\
                short line\r\n\
                this is another long line\r\n\
                short line 2",
                );

            // Make sure the sharer only has 10 columns, while the viewer has 50.
            let sharer_update = SizeUpdate {
                update_reason: SizeUpdateReason::Refresh,
                last_size: *sharer_model.block_list().size(),
                new_size: SizeInfo::new_without_font_metrics(100, 10),
                new_gap_height: None,
                natural_rows: 100,
                natural_cols: 10,
            };
            sharer_model.block_list_mut().resize(&sharer_update, true);

            let viewer_update = SizeUpdate {
                update_reason: SizeUpdateReason::Refresh,
                last_size: *viewer_model.block_list().size(),
                new_size: SizeInfo::new_without_font_metrics(100, 50),
                new_gap_height: None,
                natural_rows: 100,
                natural_cols: 50,
            };
            viewer_model.block_list_mut().resize(&viewer_update, true);

            // Suppose the sharer selects "line" on the third line
            // (which is actually the 6th line due to wrapping).
            let block_index = sharer_model
                .block_list()
                .last_non_hidden_block_by_index()
                .unwrap();
            create_simple_text_selection(
                &mut sharer_model,
                WithinBlock {
                    block_index,
                    grid: GridType::Output,
                    inner: Point::new(6, 1),
                },
                WithinBlock {
                    block_index,
                    grid: GridType::Output,
                    inner: Point::new(6, 4),
                },
            );
            assert_eq!(
                sharer_model
                    .selection_to_string(&semantic_selection, false, ctx)
                    .unwrap(),
                "line"
            );

            // Convert the selection to session-sharing-compatible points
            // that the viewer can apply locally.
            let (start, end, _) = sharer_model
                .block_list()
                .text_selection_range(&semantic_selection, false)
                .unwrap();
            let start_converted = start
                .to_session_sharing_block_point(sharer_model.block_list())
                .unwrap();
            let end_converted = end
                .to_session_sharing_block_point(sharer_model.block_list())
                .unwrap();

            // Suppose the viewer receives these points and tries to convert them to local points.
            let viewer_start = WithinBlock::<Point>::from_session_sharing_block_point(
                start_converted.clone(),
                viewer_model.block_list(),
            )
            .unwrap();
            let viewer_end = WithinBlock::<Point>::from_session_sharing_block_point(
                end_converted,
                viewer_model.block_list(),
            )
            .unwrap();
            create_simple_text_selection(&mut viewer_model, viewer_start, viewer_end);
            assert_eq!(
                viewer_model
                    .selection_to_string(&semantic_selection, false, ctx)
                    .unwrap(),
                "line"
            );
        })
    })
}
