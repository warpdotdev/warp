use float_cmp::{approx_eq, assert_approx_eq};
use warp_core::features::FeatureFlag;
use warpui::elements::DEFAULT_UI_LINE_HEIGHT_RATIO;
use warpui::units::IntoLines;
use warpui::App;

use super::*;
use crate::ai::agent::AIAgentActionId;
use crate::ai::blocklist::agent_view::{
    AgentViewDisplayMode, AgentViewEntryOrigin, AgentViewState,
};
use crate::settings::TerminalSpacing;
use crate::terminal::event::Event;
use crate::terminal::model::ansi::Handler;
use crate::terminal::model::block::AgentInteractionMetadata;
use crate::terminal::model::test_utils;
use crate::terminal::model::test_utils::TestBlockListBuilder;
use crate::terminal::view::{InlineBannerItem, InlineBannerType};
use crate::terminal::{BlockListSettings, SizeUpdateReason};

pub fn input_string(block_list: &mut BlockList, input: &str) {
    for c in input.chars() {
        block_list.input(c);
    }
}

// Returns a block list in the PostBootstrapPrecmd stage. Use this to perform tests
// about the block list, and disregard the behavior of the hidden bootstrapping block.
pub fn new_bootstrapped_block_list(
    block_sizes_override: Option<BlockSize>,
    honor_ps1_override: Option<bool>,
    channel_event_proxy: ChannelEventListener,
) -> BlockList {
    let mut builder = TestBlockListBuilder::new().with_channel_event_proxy(channel_event_proxy);
    if let Some(honor_ps1) = honor_ps1_override {
        builder = builder.with_honor_ps1(honor_ps1);
    }
    if let Some(block_sizes) = block_sizes_override {
        builder = builder.with_block_sizes(block_sizes);
    }

    let mut block_list = builder.build();
    advance_to_bootstrapped(&mut block_list, Default::default());

    assert_eq!(block_list.blocks().len(), 3);
    block_list
}

// Helper function to create dummy blocks
pub fn insert_block(block_list: &mut BlockList, command: &str, output: &str) -> BlockIndex {
    // Create a block.
    block_list.start_active_block();

    let block_index = block_list.active_block_index();

    // Fill the command grid.  This logic splits on newlines, invoking
    // `linefeed()` only when a `\n` character actually appears in the input
    // string.
    let mut lines = command.split('\n');
    if let Some(line) = lines.next() {
        input_string(block_list, line);
    }
    for line in lines {
        block_list.carriage_return();
        block_list.linefeed();
        input_string(block_list, line);
    }
    block_list.preexec(Default::default());

    // Fill the output grid.  This logic splits on newlines, invoking
    // `linefeed()` only when a `\n` character actually appears in the input
    // string.
    let mut lines = output.split('\n');
    if let Some(line) = lines.next() {
        input_string(block_list, line);
    }
    for line in lines {
        block_list.carriage_return();
        block_list.linefeed();
        input_string(block_list, line);
    }
    command_finished_and_precmd(block_list);

    block_index
}

// Helper function to create dummy blocks, with custom prompts.
pub fn insert_block_with_prompt(
    block_list: &mut BlockList,
    prompt: &str,
    command: &str,
    output: &str,
) -> BlockIndex {
    block_list.precmd(PrecmdValue {
        ps1: Some(hex::encode(prompt)),
        honor_ps1: Some(true),
        ..Default::default()
    });

    block_list.prompt_marker(ansi::PromptMarker::StartPrompt {
        kind: ansi::PromptKind::Initial,
    });
    // Fill the prompt grid.  This logic splits on newlines, adding a
    // CR/LF only when a `\n` character actually appears in the input
    // string.
    let mut lines = prompt.split('\n');
    if let Some(line) = lines.next() {
        input_string(block_list, line);
    }
    for line in lines {
        block_list.carriage_return();
        block_list.linefeed();
        input_string(block_list, line);
    }
    block_list.prompt_marker(ansi::PromptMarker::EndPrompt);

    insert_block(block_list, command, output)
}

/// Calling `command_finished` is all that's necessary for tests that only
/// advance the block list and check the state (e.g. like the length of the
/// block list, the bootstrapped state). Tests that check for messages sent to the
/// view need to also call `precmd`.
pub fn command_finished_and_precmd(block_list: &mut BlockList) {
    block_list.command_finished(Default::default());
    block_list.precmd(Default::default());
}
fn drain_terminal_events(events_rx: &async_channel::Receiver<Event>) -> Vec<Event> {
    let mut events = Vec::new();
    while let Ok(event) = events_rx.try_recv() {
        events.push(event);
    }
    events
}

/// Advances the block list to the ScriptExecution stage.
fn advance_to_script_execution(block_list: &mut BlockList) {
    assert!(
        block_list.bootstrap_stage == BootstrapStage::RestoreBlocks
            || block_list.bootstrap_stage == BootstrapStage::WarpInput,
        "Unexpected bootstrap stage: {:?}",
        block_list.bootstrap_stage
    );

    command_finished_and_precmd(block_list);
    assert_eq!(block_list.bootstrap_stage, BootstrapStage::ScriptExecution);
}

/// Advances the block list through bootstrapping (to the PostBootstrapPrecmd
/// stage).
fn advance_to_bootstrapped(block_list: &mut BlockList, data: BootstrappedValue) {
    if block_list.bootstrap_stage == BootstrapStage::RestoreBlocks
        || block_list.bootstrap_stage == BootstrapStage::WarpInput
    {
        advance_to_script_execution(block_list);
    }

    block_list.bootstrapped(data);

    command_finished_and_precmd(block_list);
    assert_eq!(
        block_list.bootstrap_stage,
        BootstrapStage::PostBootstrapPrecmd
    );
}

// This test covers the case where sometimes sumtree could have inconsistency
// where its internal node holds a larger summary than all its children nodes' summary combined
// due to floating point precision error. SumTree should be able to handle this case
// and place the cursor in the right leaf node.
#[test]
fn test_cursor_seeking_in_sumtree_with_floating_point_inconsistency() {
    let mut tree = SumTree::<BlockHeightItem>::new();
    // Heights from an actual error state.
    let heights: Vec<f32> = vec![
        873.19, 0.0, 4.19, 4.19, 4.19, 4.19, 4.19, 4.19, 4.19, 4.19, 4.19, 4.19, 4.19, 4.19, 4.19,
        4.19, 4.19, 4.19, 5.19, 4.19, 4.19, 4.19, 4.19, 4.19, 4.19, 4.19, 4.19, 4.19, 4.19, 4.19,
        4.19, 4.19, 4.19, 4.19, 5.19, 4.19, 4.19, 4.19, 4.19, 4.19, 4.19, 4.19, 4.19, 0.0,
    ];

    let items: Vec<BlockHeightItem> = heights
        .iter()
        .cloned()
        .map(|height| BlockHeightItem::Block(height.into()))
        .collect();

    for item in items {
        tree.push(item);
    }

    // Sum of all the heights.  We use this instead of a manually-computed sum
    // because the IEEE 754 representation of the individual block heights is
    // not exactly the same as the decimal representation, and so we can't use
    // 1046.98 as the total height (the sum of the heights in decimal).
    let total_sum = tree.extent::<BlockHeight>().0;

    let mut cursor = tree.cursor::<BlockHeight, BlockHeightSummary>();
    // Seeking at total sum with bias to the right should put the cursor in the end.
    cursor.seek_clamped(&BlockHeight::from(total_sum), SeekBias::Right);
    assert!(cursor.item().is_none());

    // Seeking at total sum with bias to the left should put the cursor at the last non-zero item.
    cursor.seek_clamped(&BlockHeight::from(total_sum), SeekBias::Left);
    assert_lines_approx_eq!(cursor.item().unwrap().height().into_lines(), 4.19);

    // Seeking at a smaller sum should still work. It should put the cursor at the last non-zero item.
    cursor.seek_clamped(&BlockHeight::from(1046.9797), SeekBias::Right);
    assert_lines_approx_eq!(cursor.item().unwrap().height().into_lines(), 4.19);
}

#[test]
fn test_internal_consistency_of_block_height_summing() {
    let mut tree = SumTree::<BlockHeightItem>::new();
    // Heights taken from an actual crash
    let heights: Vec<f32> = vec![
        7.69, 458.69, 5.69, 7.69, 7.69, 40.69, 944.69, 641.69, 116.69, 65.69, 143.69, 1.5, 0., 0.,
        45.0, 0.,
    ];
    let items: Vec<BlockHeightItem> = heights
        .iter()
        .cloned()
        .map(|height| BlockHeightItem::Block(height.into()))
        .collect();
    tree.extend(items);

    let mut cursor = tree.cursor::<BlockHeight, BlockHeightSummary>();
    // Should seek to between elements 11 and 12
    cursor.seek(&BlockHeight::from(2442.0898), SeekBias::Right);
    // Getting the item here should not cause a crash and should return the correct item.
    assert_lines_approx_eq!(cursor.item().unwrap().height().into_lines(), 1.5);

    let mut cursor = tree.cursor::<BlockHeight, BlockHeightSummary>();
    // Increasing the seek position slightly should seek to between elements 13 and 14
    cursor.seek(&BlockHeight::from(2442.1), SeekBias::Right);
    // Getting the item here should not cause a crash and should return the correct item.
    assert_lines_approx_eq!(cursor.item().unwrap().height().into_lines(), 45.);
}

#[test]
fn test_update_padding_block_heights() {
    App::test((), |app| async move {
        app.add_singleton_model(BlockListSettings::new_with_defaults);
        let mut block_list =
            new_bootstrapped_block_list(None, None, ChannelEventListener::new_for_test());

        // Create two blocks, each with 3 command lines and 3 output lines.
        for _ in 0..2 {
            insert_block(&mut block_list, "foo\nbar\nbazz", "foo\nbar\nbazz");
        }

        let current_block_height = block_list.block_heights().summary().height;

        let spacing = app.read(|ctx| TerminalSpacing::compact(DEFAULT_UI_LINE_HEIGHT_RATIO, ctx));
        block_list
            .update_blockheight_items(spacing.block_padding, spacing.subshell_separator_height);

        let new_block_height = block_list.block_heights().summary().height;
        assert!(!approx_eq!(Lines, current_block_height, new_block_height));
    });
}

// Disabled because it's flaky on CI.
// #[test]
// pub fn test_clear_visible_screen() {
//     let mut block_list = new_bootstrapped_block_list(None, None, ChannelEventListener::new_for_test());

//     // Create two blocks, each with 3 command lines and 3 output lines.
//     for _ in 0..2 {
//         insert_block(&mut block_list, "foo\nbar\nbazz\n", "foo\nbar\nbazz\n");
//     }

//     // Three from the bootstrapped block list, plus two calls to `block_finished`.
//     assert_eq!(block_list.blocks.len(), 5);

//     assert_float_eq!(block_list.blocks[0].height(), 0.);
//     assert_float_eq!(block_list.blocks[1].height(), 0.);
//     assert_float_eq!(block_list.blocks[2].height(), 8.5);
//     assert_float_eq!(block_list.blocks[3].height(), 8.5);
//     assert_float_eq!(block_list.blocks[4].height(), 0.);

//     assert_lines_approx_eq!(block_list.block_heights.summary().height, 17.);
//     block_list.set_next_gap_height_in_lines(17.0.into_lines());

//     // Now clear the visible screen--the number of blocks shouldn't change but total height
//     // should increase by the size of the visible screen (10).
//     block_list.clear_visible_screen();

//     assert_eq!(block_list.blocks.len(), 5);
//     assert_lines_approx_eq!(block_list.block_heights.summary().height, 34.);

//     // The active block should be after the gap within the sumtree.
//     assert_eq!(block_list.block_heights.summary().total_count, 6);
//     assert_eq!(block_list.active_gap.as_ref().unwrap().index, 4);
//     assert_lines_approx_eq!(block_list.active_gap.as_ref().unwrap().current_height, 17.);

//     // Update the height of the active block to now be 5 lines--the active gap should shrink.
//     block_list.start_active_block();
//     input_string(&mut block_list, "foo");
//     block_list.linefeed();
//     input_string(&mut block_list, "bar");
//     block_list.linefeed();
//     input_string(&mut block_list, "bazz");

//     assert_float_eq!(block_list.blocks[4].height(), 5.);
//     assert_lines_approx_eq!(block_list.block_heights.summary().height, 34.);
//     assert_lines_approx_eq!(block_list.active_gap.as_ref().unwrap().current_height, 12.);

//     // Clear the screen again--ensure there's still only one gap that is reset.
//     block_list.clear_visible_screen();
//     assert_lines_approx_eq!(block_list.block_heights.summary().height, 39.);
//     assert_lines_approx_eq!(block_list.active_gap.as_ref().unwrap().current_height, 17.);
//     assert_eq!(block_list.active_gap.as_ref().unwrap().index, 5);

//     // Add a new block with many lines--the active gap should no longer exist.
//     block_list.active_block_mut().finish(0);
//     block_list.update_active_block_height();

//     command_finished_and_precmd(&mut block_list);
//     block_list.start_active_block();
//     for _ in 0..20 {
//         input_string(&mut block_list, "foo");
//         block_list.linefeed();
//     }

//     assert_lines_approx_eq!(block_list.block_heights.summary().height, 44.0);
//     assert!(block_list.active_gap.is_none());
// }

#[test]
pub fn test_script_execution_block() {
    let (events_tx, events_rx) = async_channel::unbounded();
    let channel_event_proxy = ChannelEventListener::builder_for_test()
        .with_terminal_events_tx(events_tx)
        .build();

    let mut block_list = TestBlockListBuilder::new()
        .with_channel_event_proxy(channel_event_proxy.clone())
        .build();
    advance_to_script_execution(&mut block_list);

    // We have the `WarpInput` block and the current script execution block.
    assert_eq!(block_list.blocks.len(), 2);
    assert!(block_list.active_block().started());
    // Ensure that script execution block has a height of 0 if nothing was added to it.
    assert!(block_list
        .active_block()
        .is_empty(&AgentViewState::Inactive));

    advance_to_bootstrapped(&mut block_list, Default::default());

    // We should have three blocks after the last `block_finished`.
    assert_eq!(block_list.blocks.len(), 3);

    let mut block_list = TestBlockListBuilder::new()
        .with_channel_event_proxy(channel_event_proxy)
        .build();
    advance_to_script_execution(&mut block_list);

    assert_eq!(block_list.blocks.len(), 2);
    assert!(block_list.active_block().started());
    assert!(block_list
        .active_block()
        .is_empty(&AgentViewState::Inactive));

    // Add characters to script execution block.
    block_list.input('c');
    block_list.update_active_block_height();

    assert_eq!(block_list.blocks.len(), 2);
    assert!(!block_list
        .active_block()
        .is_empty(&AgentViewState::Inactive));

    advance_to_bootstrapped(&mut block_list, Default::default());

    // Ensure default block was not deleted since characters were added to it.
    assert_eq!(block_list.blocks.len(), 3);

    let mut block_completed_events = Vec::new();
    while let Ok(event) = events_rx.try_recv() {
        if let Event::AfterBlockCompleted(block_completed_type) = event {
            block_completed_events.push(block_completed_type)
        }
    }
    assert_eq!(block_completed_events.len(), 4);
    assert!(matches!(
        block_completed_events[0].block_type,
        BlockType::BootstrapHidden,
    ));
    assert!(matches!(
        block_completed_events[1].block_type,
        BlockType::BootstrapHidden,
    ));
    assert!(matches!(
        block_completed_events[2].block_type,
        BlockType::BootstrapHidden,
    ));
    assert!(matches!(
        block_completed_events[3].block_type,
        BlockType::BootstrapVisible(_)
    ));
}
#[test]
pub fn visible_bootstrap_block_event_fires_when_script_execution_becomes_visible() {
    let (events_tx, events_rx) = async_channel::unbounded();
    let channel_event_proxy = ChannelEventListener::builder_for_test()
        .with_terminal_events_tx(events_tx)
        .build();

    let mut block_list = TestBlockListBuilder::new()
        .with_channel_event_proxy(channel_event_proxy)
        .build();
    advance_to_script_execution(&mut block_list);

    assert!(block_list.active_block().started());
    assert!(block_list
        .active_block()
        .is_empty(&AgentViewState::Inactive));

    let events = drain_terminal_events(&events_rx);
    assert!(!events
        .iter()
        .any(|event| matches!(event, Event::VisibleBootstrapBlock)));

    block_list.input('c');
    let events = drain_terminal_events(&events_rx);
    assert!(!events
        .iter()
        .any(|event| matches!(event, Event::VisibleBootstrapBlock)));

    block_list.update_active_block_height();
    let visible_events = drain_terminal_events(&events_rx)
        .into_iter()
        .filter(|event| matches!(event, Event::VisibleBootstrapBlock))
        .count();
    assert_eq!(visible_events, 1);

    block_list.input('d');
    block_list.update_active_block_height();
    let visible_events = drain_terminal_events(&events_rx)
        .into_iter()
        .filter(|event| matches!(event, Event::VisibleBootstrapBlock))
        .count();
    assert_eq!(visible_events, 0);
}

// Add a few restored blocks and ensure they show up appropriately.
#[test]
pub fn test_restore_completed_blocks() {
    let (events_tx, events_rx) = async_channel::unbounded();
    let channel_event_proxy = ChannelEventListener::builder_for_test()
        .with_terminal_events_tx(events_tx)
        .build();

    let serialized_block: SerializedBlockListItem =
        SerializedBlock::new_for_test("i am".into(), "restored".into()).into();
    let restored_blocks = [serialized_block.clone(), serialized_block];
    let block_list = TestBlockListBuilder::new()
        .with_channel_event_proxy(channel_event_proxy)
        .with_restored_blocks(&restored_blocks)
        .build();

    // We expect to have the two restored blocks, followed by the WarpInput
    // block.
    assert_eq!(block_list.blocks.len(), 3);
    let restored_block_height = 5.5;
    assert_lines_approx_eq!(
        block_list.blocks[0].height(&AgentViewState::Inactive),
        restored_block_height
    );
    assert_lines_approx_eq!(
        block_list.blocks[1].height(&AgentViewState::Inactive),
        restored_block_height
    );
    assert_lines_approx_eq!(
        block_list.block_heights.summary().height,
        2.0 * restored_block_height + RESTORED_BLOCK_SEPARATOR_HEIGHT
    );

    let mut block_completed_events = Vec::new();
    while let Ok(event) = events_rx.try_recv() {
        if let Event::AfterBlockCompleted(block_completed_type) = event {
            block_completed_events.push(block_completed_type)
        }
    }
    assert_eq!(block_completed_events.len(), 2);
    assert!(matches!(
        block_completed_events[0].block_type,
        BlockType::Restored
    ));
    assert!(matches!(
        block_completed_events[1].block_type,
        BlockType::Restored
    ));
}

#[test]
pub fn test_restore_blocks_with_local_status() {
    let (events_tx, events_rx) = async_channel::unbounded();
    let channel_event_proxy = ChannelEventListener::builder_for_test()
        .with_terminal_events_tx(events_tx)
        .build();

    // Create a block that was local (is_local = Some(true))
    let mut local_block = SerializedBlock::new_for_test("local".into(), "block".into());
    local_block.is_local = Some(true);

    // Create a block that was remote (is_local = Some(false))
    let mut remote_block = SerializedBlock::new_for_test("remote".into(), "block".into());
    remote_block.is_local = Some(false);

    // Create a block with unspecified locality (is_local = None)
    let unspecified_block = SerializedBlock::new_for_test("unspecified".into(), "block".into());

    // Create block list with these blocks
    let restored_blocks = [
        local_block.clone().into(),
        remote_block.clone().into(),
        unspecified_block.clone().into(),
    ];

    let block_list = TestBlockListBuilder::new()
        .with_channel_event_proxy(channel_event_proxy)
        .with_restored_blocks(&restored_blocks)
        .build();

    // We should have 3 restored blocks plus the WarpInput block
    assert_eq!(block_list.blocks.len(), 4);

    // Check that the local status was preserved
    assert_eq!(block_list.blocks[0].restored_block_was_local(), Some(true));
    assert_eq!(block_list.blocks[1].restored_block_was_local(), Some(false));
    assert_eq!(block_list.blocks[2].restored_block_was_local(), None);

    // Ensure the event stream is populated correctly
    let mut block_completed_events = Vec::new();
    while let Ok(event) = events_rx.try_recv() {
        if let Event::AfterBlockCompleted(block_completed_type) = event {
            block_completed_events.push(block_completed_type);
        }
    }

    // We should have 3 events for the 3 restored blocks
    assert_eq!(block_completed_events.len(), 3);
    assert!(matches!(
        block_completed_events[0].block_type,
        BlockType::Restored
    ));
}

#[test]
pub fn test_restore_block_that_wasnt_started() {
    let (events_tx, events_rx) = async_channel::unbounded();
    let channel_event_proxy = ChannelEventListener::builder_for_test()
        .with_terminal_events_tx(events_tx)
        .build();

    let block = SerializedBlock::new_active_block_for_test();
    let block_list = TestBlockListBuilder::new()
        .with_channel_event_proxy(channel_event_proxy)
        .with_restored_blocks(&[block.into()])
        .build();

    // Non-started blocks are skipped during the restoration process, so we
    // expect to only have one block - the WarpInput block.
    assert_eq!(block_list.blocks.len(), 1);
    assert_eq!(
        block_list.blocks[0].bootstrap_stage(),
        BootstrapStage::WarpInput
    );
    assert_eq!(
        block_list.blocks[0].height(&AgentViewState::Inactive),
        Lines::zero()
    );

    let mut block_completed_events = Vec::new();
    while let Ok(event) = events_rx.try_recv() {
        if let Event::AfterBlockCompleted(block_completed_type) = event {
            block_completed_events.push(block_completed_type)
        }
    }
    assert_eq!(block_completed_events.len(), 0);
}

#[test]
pub fn test_restore_block_that_wasnt_completed() {
    let (events_tx, events_rx) = async_channel::unbounded();
    let channel_event_proxy = ChannelEventListener::builder_for_test()
        .with_terminal_events_tx(events_tx)
        .build();

    let mut block = SerializedBlock::new_for_test("test".into(), "test".into());
    block.completed_ts = None;
    let block_list = TestBlockListBuilder::new()
        .with_channel_event_proxy(channel_event_proxy)
        .with_restored_blocks(&[block.into()])
        .build();

    // Non-completed blocks are skipped during the restoration process, so we
    // expect to only have one block - the WarpInput block.
    assert_eq!(block_list.blocks.len(), 1);
    assert_eq!(
        block_list.blocks[0].bootstrap_stage(),
        BootstrapStage::WarpInput
    );
    assert_lines_approx_eq!(block_list.blocks[0].height(&AgentViewState::Inactive), 0.0);

    let mut block_completed_events = Vec::new();
    while let Ok(event) = events_rx.try_recv() {
        if let Event::AfterBlockCompleted(block_completed_type) = event {
            block_completed_events.push(block_completed_type)
        }
    }
    assert_eq!(block_completed_events.len(), 0);
}

// Bootstrap with no restored blocks and no script execution.
// There will be a special hidden InitShell block and everything else should be empty.
#[test]
pub fn test_basic_bootstrapping() {
    let (events_tx, events_rx) = async_channel::unbounded();
    let channel_event_proxy = ChannelEventListener::builder_for_test()
        .with_terminal_events_tx(events_tx)
        .build();

    let mut block_list = TestBlockListBuilder::new()
        .with_channel_event_proxy(channel_event_proxy)
        .build();

    // Simulate entering the bootstrap script for WarpInput mode.
    input_string(&mut block_list, "i am the warp input");
    block_list.linefeed();
    block_list.preexec(Default::default());
    // WarpInput -> ScriptExecution
    command_finished_and_precmd(&mut block_list);
    // ScriptExecution -> Bootstrapped
    block_list.bootstrapped(Default::default());

    // Simulate the post-bootstrap precmd sent from the
    // overall bootstrapping script.
    command_finished_and_precmd(&mut block_list);

    // We have four blocks from calling `create_warp_input_block` once and `block_finished` twice.
    assert_eq!(block_list.blocks.len(), 3);
    assert_lines_approx_eq!(block_list.blocks[0].height(&AgentViewState::Inactive), 0.0);
    assert_lines_approx_eq!(block_list.blocks[1].height(&AgentViewState::Inactive), 0.0);
    assert_lines_approx_eq!(block_list.blocks[2].height(&AgentViewState::Inactive), 0.0);
    assert_lines_approx_eq!(block_list.block_heights.summary().height, 0.0);

    let mut block_completed_events = Vec::new();
    while let Ok(event) = events_rx.try_recv() {
        if let Event::AfterBlockCompleted(block_completed_type) = event {
            block_completed_events.push(block_completed_type)
        }
    }
    assert_eq!(block_completed_events.len(), 2);
    assert!(matches!(
        block_completed_events[0].block_type,
        BlockType::BootstrapHidden
    ));
    assert!(matches!(
        block_completed_events[1].block_type,
        BlockType::BootstrapHidden
    ));
}

#[test]
pub fn test_session_restoration_separator() {
    let serialized_block: SerializedBlockListItem =
        SerializedBlock::new_for_test("i am".as_bytes().to_vec(), "restored".as_bytes().to_vec())
            .into();
    let restored_blocks = [serialized_block.clone(), serialized_block];
    let mut block_list = TestBlockListBuilder::new()
        .with_restored_blocks(&restored_blocks)
        .build();

    block_list.set_next_gap_height_in_lines((11. + RESTORED_BLOCK_SEPARATOR_HEIGHT).into_lines());
    assert_eq!(block_list.blocks.len(), 3);
    assert_lines_approx_eq!(block_list.blocks[0].height(&AgentViewState::Inactive), 5.5);
    assert_lines_approx_eq!(block_list.blocks[1].height(&AgentViewState::Inactive), 5.5);
    assert_lines_approx_eq!(block_list.blocks[2].height(&AgentViewState::Inactive), 0.0);

    // We have two blocks at height 5.5 and a separator with height 1.5.
    assert_lines_approx_eq!(
        block_list.block_heights.summary().height,
        11.0 + RESTORED_BLOCK_SEPARATOR_HEIGHT
    );

    // Clear the visible screen and ensure total height increases by 10.
    block_list.clear_visible_screen();
    assert_eq!(block_list.blocks.len(), 3);
    assert_lines_approx_eq!(
        block_list.block_heights.summary().height,
        11.0 + RESTORED_BLOCK_SEPARATOR_HEIGHT
            + block_list
                .next_gap_height()
                .expect("height should be set")
                .as_f64()
    );

    // With the active block still hidden during initialize, the gap is inserted before the active
    // block in clear_visible_screen.
    // Total items: 2 restored blocks + 1 separator + 1 gap + 1 active block = 5.
    assert_eq!(block_list.block_heights.summary().total_count, 5);
    // Gap is at index 3 (before the active block at index 4).
    assert_eq!(block_list.active_gap.as_ref().unwrap().index, 3);
    assert_approx_eq!(
        Lines,
        block_list.active_gap.as_ref().unwrap().current_height,
        block_list
            .next_gap_height()
            .expect("gap height should be set")
    );
}

#[test]
pub fn test_insert_non_block_item() {
    let mut block_list =
        new_bootstrapped_block_list(None, None, ChannelEventListener::new_for_test());

    // Create two blocks, each with 3 command lines and 3 output lines.
    let first_block_index = insert_block(&mut block_list, "foo\nbar\nbazz\n", "foo\nbar\nbazz\n");
    assert_eq!(first_block_index, BlockIndex(2));
    assert_eq!(first_block_index.to_total_index(&block_list), TotalIndex(2));

    insert_block(&mut block_list, "foo\nbar\nbazz\n", "foo\nbar\nbazz\n");

    // This happens to be the block height of such blocks ^.
    let block_height = 8.5;

    // Add a non-block item at the start of the block list (after the hidden blocks).
    let inserted_index = block_list.insert_non_block_item_before_block(
        first_block_index,
        BlockHeightItem::RestoredBlockSeparator {
            height_when_visible: BlockHeight::from(RESTORED_BLOCK_SEPARATOR_HEIGHT),
            is_historical_conversation_restoration: false,
            is_hidden: false,
        },
    );
    assert_eq!(inserted_index, TotalIndex(2));

    // Add a non-block item at the end of the block list.
    let inserted_index = block_list.insert_non_block_item_before_block(
        block_list.active_block_index(),
        BlockHeightItem::RestoredBlockSeparator {
            height_when_visible: BlockHeight::from(RESTORED_BLOCK_SEPARATOR_HEIGHT),
            is_historical_conversation_restoration: false,
            is_hidden: false,
        },
    );
    assert_eq!(inserted_index, TotalIndex(5));

    // The blocks should remain unchanged.
    assert_eq!(block_list.blocks.len(), 5);
    assert_lines_approx_eq!(block_list.blocks[0].height(&AgentViewState::Inactive), 0.);
    assert_lines_approx_eq!(block_list.blocks[1].height(&AgentViewState::Inactive), 0.);
    assert_lines_approx_eq!(
        block_list.blocks[2].height(&AgentViewState::Inactive),
        block_height
    );
    assert_lines_approx_eq!(
        block_list.blocks[3].height(&AgentViewState::Inactive),
        block_height
    );
    assert_lines_approx_eq!(block_list.blocks[4].height(&AgentViewState::Inactive), 0.);

    fn assert_block_height_summary_eq(a: BlockHeightSummary, b: BlockHeightSummary) {
        assert_eq!(a.block_count, b.block_count);
        assert_eq!(a.total_count, b.total_count);
        assert_lines_approx_eq!(a.height, b.height);
    }

    // But the block heights (which encapsulates blocks + nonblocks) should reflect the new items.

    let summaries: Vec<BlockHeightSummary> = block_list
        .block_heights()
        .items()
        .iter()
        .map(|i| i.summary())
        .collect();

    // 2 hidden blocks + non-block + 2 blocks + non-block + active block
    assert_eq!(block_list.block_heights().summary().total_count, 7);

    // The first two items are hidden blocks.
    assert_block_height_summary_eq(
        summaries[0],
        BlockHeightSummary {
            total_count: 1,
            block_count: 1,
            height: Lines::zero(),
        },
    );
    assert_block_height_summary_eq(
        summaries[1],
        BlockHeightSummary {
            total_count: 1,
            block_count: 1,
            height: Lines::zero(),
        },
    );

    // The next item should be the non-block item.
    assert_block_height_summary_eq(
        summaries[2],
        BlockHeightSummary {
            total_count: 1,
            block_count: 0,
            height: RESTORED_BLOCK_SEPARATOR_HEIGHT.into_lines(),
        },
    );

    // The next two items are the blocks.
    assert_block_height_summary_eq(
        summaries[3],
        BlockHeightSummary {
            total_count: 1,
            block_count: 1,
            height: block_height.into_lines(),
        },
    );
    assert_block_height_summary_eq(
        summaries[4],
        BlockHeightSummary {
            total_count: 1,
            block_count: 1,
            height: block_height.into_lines(),
        },
    );

    // The next item should be the non-block item.
    assert_block_height_summary_eq(
        summaries[5],
        BlockHeightSummary {
            total_count: 1,
            block_count: 0,
            height: RESTORED_BLOCK_SEPARATOR_HEIGHT.into_lines(),
        },
    );

    // The last item is the active block.
    assert_block_height_summary_eq(
        summaries[6],
        BlockHeightSummary {
            total_count: 1,
            block_count: 1,
            height: Lines::zero(),
        },
    );

    // Overall, we have two blocks with 8.5 height and two separator with 1.5 height.
    let total_height = 2. * block_height + 2. * RESTORED_BLOCK_SEPARATOR_HEIGHT;
    assert_lines_approx_eq!(block_list.block_heights.summary().height, total_height);

    // Now clear the visible screen--the number of blocks shouldn't change but total height
    // should increase by the size of the visible screen.
    block_list.set_next_gap_height_in_lines(total_height.into_lines());
    block_list.clear_visible_screen();

    assert_eq!(block_list.blocks.len(), 5);
    assert_eq!(block_list.block_heights().summary().total_count, 8);
    assert_lines_approx_eq!(
        block_list.block_heights.summary().height,
        total_height
            + block_list
                .next_gap_height()
                .expect("gap height should be set")
                .as_f64()
    );

    // The active block should be after the gap within the sumtree.
    assert_eq!(block_list.active_gap.as_ref().unwrap().index, 6);
    assert_lines_approx_eq!(
        block_list.active_gap.as_ref().unwrap().current_height,
        total_height
    );
}

#[test]
pub fn test_first_non_hidden_block_by_index_in_range() {
    let mut block_list = TestBlockListBuilder::new().build();
    assert_eq!(block_list.first_non_hidden_block_by_index(), None);

    // add a hidden block
    block_list.start_active_block();
    input_string(&mut block_list, "foo");
    block_list.linefeed();
    block_list.preexec(Default::default());

    advance_to_script_execution(&mut block_list);

    assert_eq!(block_list.first_non_hidden_block_by_index(), None);

    advance_to_bootstrapped(&mut block_list, Default::default());

    // add two non-hidden blocks
    let first_visible_block_index = insert_block(&mut block_list, "foo\n", "foo\n");
    insert_block(&mut block_list, "foo\n", "foo\n");

    assert_eq!(
        block_list.first_non_hidden_block_by_index(),
        Some(first_visible_block_index)
    );
}

#[test]
fn test_matching_block_by_index() {
    let mut block_list =
        new_bootstrapped_block_list(None, None, ChannelEventListener::new_for_test());

    // Add a non-hidden command block, and then a background block.
    insert_block(&mut block_list, "command\n", "output\n");
    input_string(&mut block_list, "background output");

    // The default filter should include background blocks.
    assert_eq!(
        block_list.last_matching_block_by_index(BlockFilter::default()),
        Some(3.into())
    );
    assert_eq!(block_list.last_non_hidden_block_by_index(), Some(3.into()));

    // The command-only filter should exclude background blocks.
    assert_eq!(
        block_list.last_matching_block_by_index(BlockFilter::commands()),
        Some(2.into())
    );

    // It should be possible to include hidden blocks.
    assert_eq!(
        block_list.first_matching_block_by_index(BlockFilter {
            include_hidden: true,
            ..Default::default()
        }),
        Some(0.into())
    );
}

#[test]
fn test_banner_insertion_and_removal() {
    let mut block_list =
        new_bootstrapped_block_list(None, None, ChannelEventListener::new_for_test());

    // Create the following blocklist:
    // block -> banner -> block -> banner -> block -> banner
    let first_block_index = insert_block(&mut block_list, "1", "1");
    insert_block(&mut block_list, "2", "2");
    let last_block_index = insert_block(&mut block_list, "3", "3");

    block_list.insert_inline_banner_after_block(
        first_block_index,
        InlineBannerItem::new(0, InlineBannerType::NotificationsDiscovery),
    );
    block_list.insert_inline_banner_before_block(
        last_block_index,
        InlineBannerItem::new(1, InlineBannerType::NotificationsDiscovery),
        None,
    );
    block_list.append_inline_banner(InlineBannerItem::new(
        2,
        InlineBannerType::NotificationsDiscovery,
    ));

    // Three inserted blocks + three banners + three blocks from bootstrapping
    // Note that in the expected_total_height calculations, the active block
    // has a height of 0 since it hasn't hit preexec
    let total_block_count_after_insertion = 6;
    let total_count_after_insertion = 9;
    assert_eq!(
        block_list.block_heights.summary().block_count,
        total_block_count_after_insertion
    );
    assert_eq!(
        block_list.block_heights.summary().total_count,
        total_count_after_insertion
    );

    let expected_total_height = (block_list.blocks[2]
        .height(&AgentViewState::Inactive)
        .as_f64()
        * 3.
        + 3. * INLINE_BANNER_HEIGHT)
        .into_lines();
    assert_lines_approx_eq!(
        block_list.block_heights.summary().height,
        expected_total_height
    );

    // Remove the first banner
    block_list.remove_inline_banner(0);
    assert_eq!(
        block_list.block_heights.summary().block_count,
        total_block_count_after_insertion
    );
    assert_eq!(
        block_list.block_heights.summary().total_count,
        total_count_after_insertion - 1
    );

    let expected_total_height_after_removal = expected_total_height - INLINE_BANNER_HEIGHT;
    assert_lines_approx_eq!(
        block_list.block_heights.summary().height,
        expected_total_height_after_removal
    );

    // Remove the second banner
    block_list.remove_inline_banner(1);
    assert_eq!(
        block_list.block_heights.summary().block_count,
        total_block_count_after_insertion
    );
    assert_eq!(
        block_list.block_heights.summary().total_count,
        total_count_after_insertion - 2
    );

    let expected_total_height = expected_total_height - 2. * INLINE_BANNER_HEIGHT;
    assert_lines_approx_eq!(
        block_list.block_heights.summary().height,
        expected_total_height
    );
}

/// Regression test for WAR-6056, an issue where removing a banner would leave
/// the active gap in an incorrect state, causing a panic on the next window resize.
#[test]
fn test_gap_after_banner() {
    let mut block_list =
        new_bootstrapped_block_list(None, None, ChannelEventListener::new_for_test());

    // Create the following blocklist:
    // bootstrap block -> bootstrap block -> block -> banner -> gap -> block -> active block
    insert_block(&mut block_list, "cmd", "output");

    block_list.append_inline_banner(InlineBannerItem::new(
        0,
        InlineBannerType::NotificationsDiscovery,
    ));
    block_list.set_next_gap_height_in_lines(17.0.into_lines());
    block_list.clear_visible_screen();

    insert_block(&mut block_list, "cmd2", "output2");
    let baseline_block_height = block_list.blocks[2].height(&AgentViewState::Inactive);

    {
        let summary = block_list.block_heights.summary();
        let active_gap = block_list.active_gap.as_ref().unwrap().clone();
        let gap_height = 17. - baseline_block_height.as_f64();
        // There are 2 bootstrap blocks, 2 blocks, 1 banner, 1 gap, and 1 active block.
        assert_eq!(summary.total_count, 7);
        assert_lines_approx_eq!(active_gap.current_height, gap_height);
        // The gap is after the 2 bootstrap blocks, the first complete block, and the banner.
        assert_eq!(active_gap.index, 4);
        assert_lines_approx_eq!(
            summary.height,
            2. * baseline_block_height.as_f64() + INLINE_BANNER_HEIGHT + gap_height
        );
    }

    // Now, remove the banner and confirm that the gap was updated.
    block_list.remove_inline_banner(0);

    {
        let summary = block_list.block_heights.summary();
        let active_gap = block_list.active_gap.as_ref().unwrap().clone();
        let gap_height = 17. - baseline_block_height.as_f64();
        assert_eq!(summary.total_count, 6);
        assert_lines_approx_eq!(active_gap.current_height, gap_height);
        assert_eq!(active_gap.index, 3);
        assert_lines_approx_eq!(
            summary.height,
            2. * baseline_block_height.as_f64() + gap_height
        );
    }

    // Finally, resizing should update the gap without a panic.
    block_list.update_active_block_height();
    let size_update = SizeUpdate {
        update_reason: SizeUpdateReason::Refresh,
        last_size: *block_list.size(),
        new_size: SizeInfo::new_without_font_metrics(5, 5),
        new_gap_height: Some(5.0.into_lines()),
        natural_rows: 5,
        natural_cols: 5,
    };
    block_list.resize(&size_update, true);

    {
        let active_gap = block_list.active_gap.as_ref().unwrap().clone();
        let new_block_height = block_list.blocks[2].height(&AgentViewState::Inactive);
        assert_lines_approx_eq!(active_gap.current_height, 5.);
        assert_eq!(active_gap.index, 3);

        let mut cursor = block_list
            .block_heights
            .cursor::<TotalIndex, BlockHeightSummary>();
        assert!(cursor.seek(&active_gap.index(), SeekBias::Right));
        match cursor.item() {
            Some(BlockHeightItem::Gap(height)) => assert_lines_approx_eq!(height.0, 5.),
            other => panic!("Expected a Gap, got {other:?}"),
        }
        // The height only includes one block since the gap is before the second one.
        assert_lines_approx_eq!(cursor.end().height, new_block_height + 5.);

        assert_lines_approx_eq!(
            block_list.block_heights.summary().height,
            2. * new_block_height.as_f64() + 5.
        );
    }
}

#[test]
fn test_removed_gap_with_banner() {
    let mut block_list =
        new_bootstrapped_block_list(None, None, ChannelEventListener::new_for_test());

    block_list.set_next_gap_height_in_lines(17.0.into_lines());
    block_list.clear_visible_screen();
    assert!(block_list.active_gap.is_some());

    insert_block(&mut block_list, "cmd", "output");

    block_list.append_inline_banner(InlineBannerItem::new(
        0,
        InlineBannerType::NotificationsDiscovery,
    ));
    // Make sure the banner was inserted.
    assert!(block_list
        .block_heights
        .items()
        .iter()
        .any(|it| matches!(it, BlockHeightItem::InlineBanner { banner, .. } if banner.id == 0)));

    // There's two bootstrap blocks, one gap and one block before the banner.
    assert_eq!(
        block_list
            .removable_blocklist_item_positions
            .get(&RemovableBlocklistItem::InlineBanner(0)),
        Some(&TotalIndex(4))
    );

    // Add output so that the gap is removed by update_active_block_height.
    block_list.active_block_mut().finish(0);
    block_list.update_active_block_height();
    command_finished_and_precmd(&mut block_list);
    block_list.start_active_block();
    for _ in 0..20 {
        input_string(&mut block_list, "text");
        block_list.linefeed();
    }

    // Pretend that we finished processing a chunk of bytes from the PTY so we
    // properly update block heights.
    block_list.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));

    assert!(block_list.active_gap.is_none());
    assert_eq!(
        block_list
            .removable_blocklist_item_positions
            .get(&RemovableBlocklistItem::InlineBanner(0)),
        Some(&TotalIndex(3))
    );

    // We should still be able to remove the banner even though its position has changed.
    block_list.remove_inline_banner(0);
    assert_eq!(
        block_list
            .block_heights
            .items()
            .iter()
            .find(|it| matches!(it, BlockHeightItem::InlineBanner { .. })),
        None
    );
}

#[test]
pub fn test_block_heights_combined_prompt_command_grid_warp_prompt() {
    let mut block_list =
        new_bootstrapped_block_list(None, None, ChannelEventListener::new_for_test());
    let bootstrapped_block_list_len = block_list.blocks().len();

    // Create one block with 3 command lines and 3 output lines.
    let first_block_index = insert_block(&mut block_list, "foo\nbar\nbazz\n", "foo\nbar\nbazz\n");

    let first_block = block_list
        .block_at(first_block_index)
        .expect("block should exist");

    // We created one block.
    assert_eq!(block_list.blocks.len(), bootstrapped_block_list_len + 1);

    // Note that this test is using the Warp prompt, hence the prompt is not included in the combined grid.
    assert_eq!(first_block.prompt_and_command_grid().len(), 3);
    assert_eq!(first_block.output_grid().len(), 3);

    // In this case, we SHOULD consider command_padding_top since we have a combined prompt/command grid BUT
    // we have the built-in Warp prompt, so there's padding between that prompt and the combined grid.
    // The combined grid _just_ has the command in this case! The PS1 is unset!
    // Hence, we expect heights of 8.5.
    assert_lines_approx_eq!(first_block.height(&AgentViewState::Inactive), 8.5);
}

#[test]
pub fn test_block_heights_combined_prompt_command_grid_ps1() {
    let block_sizes = BlockSize {
        // Make sure the grid is wide enough that "prompt2" + "foo" fits on one
        // line without wrapping.
        size: SizeInfo::new_without_font_metrics(10, 20),
        ..test_utils::block_size()
    };
    let mut block_list = new_bootstrapped_block_list(
        Some(block_sizes),
        Some(true),
        ChannelEventListener::new_for_test(),
    );

    let bootstrapped_block_list_len = block_list.blocks().len();

    // Create one block with 3 prompt/command lines (1 prompt line, 1 combined line, 2 command lines) and 3 output lines.
    let first_block_index = insert_block_with_prompt(
        &mut block_list,
        "prompt1\nprompt2",
        "foo\nbar\nbazz\n",
        "foo\nbar\nbazz\n",
    );

    let first_block = block_list
        .block_at(first_block_index)
        .expect("block should exist");

    // We created one block.
    assert_eq!(block_list.blocks.len(), bootstrapped_block_list_len + 1);

    // We have a 2-line prompt, but the second line should be shared with the command!
    // Hence the 2-line prompt and 3-line command result in 4 total lines!
    assert_eq!(first_block.prompt_and_command_grid().len(), 4);
    assert_eq!(first_block.output_grid().len(), 3);

    // We have a 2-line prompt, adding 1 extra line to the combined grid (vs 0.6 default for Warp prompt).
    // Hence, we expect a height of 8.7 rather than 8.3.
    assert_lines_approx_eq!(first_block.height(&AgentViewState::Inactive), 8.7);
}

#[test]
fn test_block_height_update_shifts_indices() {
    let mut block_list =
        new_bootstrapped_block_list(None, None, ChannelEventListener::new_for_test());

    // Create a dummy block.
    let first_block_index = insert_block(&mut block_list, "cmd", "output");
    assert_eq!(first_block_index, BlockIndex(2));
    assert_eq!(first_block_index.to_total_index(&block_list), TotalIndex(2));

    // Insert a gap after the created block. It should be at total index 3 (after the first 2 bootstrap blocks + inserted block).
    block_list.set_next_gap_height_in_lines(17.0.into_lines());
    block_list.clear_visible_screen();
    assert!(block_list.active_gap.is_some());
    assert_eq!(
        block_list.active_gap.as_ref().unwrap().index(),
        TotalIndex(3)
    );

    // Create another dummy block.
    let second_block_index = insert_block(&mut block_list, "cmd", "output");
    assert_eq!(second_block_index, BlockIndex(3));
    assert_eq!(
        second_block_index.to_total_index(&block_list),
        TotalIndex(4)
    );

    // Insert a banner before the first block.
    block_list.insert_inline_banner_before_block(
        first_block_index,
        InlineBannerItem::new(0, InlineBannerType::NotificationsDiscovery),
        None,
    );

    // Make sure the banner was inserted.
    assert!(block_list
        .block_heights
        .items()
        .iter()
        .any(|it| matches!(it, BlockHeightItem::InlineBanner { banner, .. } if banner.id == 0)));
    assert_eq!(
        block_list
            .removable_blocklist_item_positions
            .get(&RemovableBlocklistItem::InlineBanner(0)),
        Some(&TotalIndex(2))
    );

    // Make sure the gap was adjusted.
    assert!(block_list.active_gap.is_some());
    assert_eq!(
        block_list.active_gap.as_ref().unwrap().index(),
        TotalIndex(4)
    );

    // Remove the banner
    block_list.remove_inline_banner(0);

    // Make sure the banner is gone.
    assert!(!block_list
        .block_heights
        .items()
        .iter()
        .any(|it| matches!(&it, BlockHeightItem::InlineBanner { .. })));
    assert!(!block_list
        .removable_blocklist_item_positions
        .contains_key(&RemovableBlocklistItem::InlineBanner(0)));

    // Make sure the gap was adjusted back.
    assert!(block_list.active_gap.is_some());
    assert_eq!(
        block_list.active_gap.as_ref().unwrap().index(),
        TotalIndex(3)
    );
}

#[test]
fn test_remove_rich_content_block() {
    // Populate the blocklist.
    let mut block_list =
        new_bootstrapped_block_list(None, None, ChannelEventListener::new_for_test());

    insert_block(&mut block_list, "cmd", "output");

    let view_id_a = EntityId::new();
    block_list.append_rich_content(RichContentItem::new_for_test(None, view_id_a, None), false);

    let second_block_index = insert_block(&mut block_list, "cmd", "output");

    block_list.insert_inline_banner_before_block(
        second_block_index,
        InlineBannerItem::new(0, InlineBannerType::NotificationsDiscovery),
        None,
    );

    let view_id_b = EntityId::new();
    block_list.append_rich_content(RichContentItem::new_for_test(None, view_id_b, None), false);

    /*
    The blocklist is now:
        (1) first block
        (2) rich content A
        (3) inline banner
        (4) second block
        (5) rich content B
    */

    assert_eq!(
        block_list
            .removable_blocklist_item_positions
            .get(&RemovableBlocklistItem::RichContent(view_id_a)),
        Some(&TotalIndex(3))
    );

    assert_eq!(
        block_list
            .removable_blocklist_item_positions
            .get(&RemovableBlocklistItem::RichContent(view_id_b)),
        Some(&TotalIndex(6))
    );

    // Remove inline banner.
    block_list.remove_inline_banner(0);

    assert_eq!(
        block_list
            .removable_blocklist_item_positions
            .get(&RemovableBlocklistItem::RichContent(view_id_a)),
        Some(&TotalIndex(3))
    );

    assert_eq!(
        block_list
            .removable_blocklist_item_positions
            .get(&RemovableBlocklistItem::RichContent(view_id_b)),
        Some(&TotalIndex(5))
    );

    // Remove first rich content block.
    block_list.remove_rich_content(view_id_a);

    assert_eq!(
        block_list
            .removable_blocklist_item_positions
            .get(&RemovableBlocklistItem::RichContent(view_id_b)),
        Some(&TotalIndex(4))
    );

    // Remove second rich content block.
    block_list.remove_rich_content(view_id_b);

    assert!(!block_list
        .block_heights
        .items()
        .iter()
        .any(|item| matches!(&item, BlockHeightItem::RichContent { .. })));
}

#[test]
fn test_conversation_scoped_rich_content_hidden_outside_fullscreen_agent_view() {
    FeatureFlag::AgentView.set_enabled(true);
    let mut block_list =
        new_bootstrapped_block_list(None, None, ChannelEventListener::new_for_test());
    let conversation_id = AIConversationId::new();
    let view_id = EntityId::new();

    block_list.append_rich_content(
        RichContentItem::new_for_test(None, view_id, Some(conversation_id)),
        false,
    );

    block_list.set_agent_view_state(AgentViewState::Active {
        conversation_id,
        origin: AgentViewEntryOrigin::Input {
            was_prompt_autodetected: false,
        },
        display_mode: AgentViewDisplayMode::FullScreen,
        original_conversation_length: 0,
    });

    let item_visible_in_fullscreen =
        block_list
            .block_heights()
            .items()
            .iter()
            .find_map(|item| match item {
                BlockHeightItem::RichContent(rich_content) if rich_content.view_id == view_id => {
                    Some(*rich_content)
                }
                _ => None,
            });
    assert!(item_visible_in_fullscreen.is_some());
    assert!(item_visible_in_fullscreen.is_some_and(|item| !item.should_hide));
    assert!(item_visible_in_fullscreen
        .is_some_and(|item| item.last_laid_out_height > BlockHeight::zero()));

    block_list.set_agent_view_state(AgentViewState::Inactive);

    let item_hidden_in_terminal_mode =
        block_list
            .block_heights()
            .items()
            .iter()
            .find_map(|item| match item {
                BlockHeightItem::RichContent(rich_content) if rich_content.view_id == view_id => {
                    Some(*rich_content)
                }
                _ => None,
            });
    assert!(item_hidden_in_terminal_mode.is_some());
    assert!(item_hidden_in_terminal_mode.is_some_and(|item| item.should_hide));

    block_list.set_agent_view_state(AgentViewState::Active {
        conversation_id,
        origin: AgentViewEntryOrigin::Input {
            was_prompt_autodetected: false,
        },
        display_mode: AgentViewDisplayMode::Inline,
        original_conversation_length: 0,
    });

    let item_hidden_in_inline =
        block_list
            .block_heights()
            .items()
            .iter()
            .find_map(|item| match item {
                BlockHeightItem::RichContent(rich_content) if rich_content.view_id == view_id => {
                    Some(*rich_content)
                }
                _ => None,
            });
    assert!(item_hidden_in_inline.is_some());
    assert!(item_hidden_in_inline.is_some_and(|item| item.should_hide));
}

#[test]
fn test_clear_user_executed_command_blocks_for_conversation() {
    let mut block_list =
        new_bootstrapped_block_list(None, None, ChannelEventListener::new_for_test());

    let terminal_block_index = insert_block(&mut block_list, "terminal", "output");

    let conversation_id = AIConversationId::new();

    let user_block_index = insert_block(&mut block_list, "user", "output");
    {
        let block = &mut block_list.blocks_mut()[user_block_index.0];
        // User-executed command blocks created inside agent view typically remain in User
        // interaction mode.
        block.set_conversation_id(conversation_id);
    }

    let requested_command_block_index = insert_block(&mut block_list, "requested", "output");
    {
        let block = &mut block_list.blocks_mut()[requested_command_block_index.0];
        block.set_conversation_id(conversation_id);
        let action_id: AIAgentActionId = "action".to_owned().into();
        block.set_agent_interaction_mode(AgentInteractionMetadata::new_hidden(
            action_id,
            conversation_id,
        ));
    }

    let view_id = EntityId::new();
    block_list.append_rich_content(
        RichContentItem::new_for_test(None, view_id, Some(conversation_id)),
        false,
    );

    block_list.set_agent_view_state(AgentViewState::Active {
        conversation_id,
        origin: AgentViewEntryOrigin::LongRunningCommand,
        display_mode: AgentViewDisplayMode::FullScreen,
        original_conversation_length: 0,
    });

    let terminal_block_id = block_list
        .block_at(terminal_block_index)
        .unwrap()
        .id()
        .clone();
    let user_block_id = block_list.block_at(user_block_index).unwrap().id().clone();
    let requested_command_block_id = block_list
        .block_at(requested_command_block_index)
        .unwrap()
        .id()
        .clone();

    block_list.clear_user_executed_command_blocks_for_conversation(conversation_id);

    assert!(block_list.block_index_for_id(&terminal_block_id).is_some());
    assert!(block_list
        .block_index_for_id(&requested_command_block_id)
        .is_some());
    assert!(block_list.block_index_for_id(&user_block_id).is_none());
    assert!(block_list
        .removable_blocklist_item_positions
        .contains_key(&RemovableBlocklistItem::RichContent(view_id)));
}

#[test]
fn test_agent_origin_block_can_be_attached_to_other_conversation() {
    let _agent_view_flag = FeatureFlag::AgentView.override_enabled(true);
    let mut block_list =
        new_bootstrapped_block_list(None, None, ChannelEventListener::new_for_test());

    let expected_origin_conversation_id = AIConversationId::new();
    let other_conversation_id = AIConversationId::new();

    let active_state = |conversation_id| AgentViewState::Active {
        conversation_id,
        origin: AgentViewEntryOrigin::Input {
            was_prompt_autodetected: false,
        },
        display_mode: AgentViewDisplayMode::FullScreen,
        original_conversation_length: 0,
    };

    block_list.set_agent_view_state(active_state(expected_origin_conversation_id));
    let user_block_index = insert_block(&mut block_list, "user", "output");
    let user_block_id = block_list.block_at(user_block_index).unwrap().id().clone();

    let associated = block_list
        .associate_blocks_with_conversation([&user_block_id].into_iter(), other_conversation_id);
    assert_eq!(associated.len(), 1);
    assert_eq!(associated[0].0, user_block_id);
    match &associated[0].1 {
        AgentViewVisibility::Agent {
            origin_conversation_id: observed_origin_conversation_id,
            pending_other_conversation_ids,
            other_conversation_ids,
        } => {
            assert_eq!(
                observed_origin_conversation_id,
                &expected_origin_conversation_id
            );
            assert!(pending_other_conversation_ids.contains(&other_conversation_id));
            assert!(!other_conversation_ids.contains(&other_conversation_id));
        }
        _ => panic!("Expected agent visibility for agent-origin block"),
    }

    block_list.set_agent_view_state(active_state(other_conversation_id));
    let user_block_index = block_list.block_index_for_id(&user_block_id).unwrap();
    let user_block = block_list.block_at(user_block_index).unwrap();
    assert!(!user_block.is_empty(block_list.agent_view_state()));

    let promoted = block_list.promote_blocks_to_attached_from_conversation(other_conversation_id);
    assert_eq!(promoted.len(), 1);
    assert_eq!(promoted[0].0, user_block_id);
    match &promoted[0].1 {
        AgentViewVisibility::Agent {
            pending_other_conversation_ids,
            other_conversation_ids,
            ..
        } => {
            assert!(!pending_other_conversation_ids.contains(&other_conversation_id));
            assert!(other_conversation_ids.contains(&other_conversation_id));
        }
        _ => panic!("Expected agent visibility for agent-origin block"),
    }

    let removed = block_list.remove_pending_context_assocation_for_blocks(
        [&user_block_id].into_iter(),
        other_conversation_id,
    );
    assert!(removed.is_empty());

    block_list.set_agent_view_state(AgentViewState::Inactive);
    let user_block_index = block_list.block_index_for_id(&user_block_id).unwrap();
    let user_block = block_list.block_at(user_block_index).unwrap();
    assert!(user_block.is_empty(block_list.agent_view_state()));
}

#[test]
fn test_finish_startup_commands_at_block_attaches_and_unhides_command_blocks_since_target_block() {
    let _agent_view_flag = FeatureFlag::AgentView.override_enabled(true);
    let mut block_list =
        new_bootstrapped_block_list(None, None, ChannelEventListener::new_for_test());
    block_list.set_is_executing_oz_environment_startup_commands(true);

    let setup_block_index = insert_block(&mut block_list, "setup", "output");
    let harness_block_index = insert_block(&mut block_list, "claude", "output");
    let followup_block_index = insert_block(&mut block_list, "pwd", "output");
    let setup_block_id = block_list.block_at(setup_block_index).unwrap().id().clone();
    let harness_block_id = block_list
        .block_at(harness_block_index)
        .unwrap()
        .id()
        .clone();
    let followup_block_id = block_list
        .block_at(followup_block_index)
        .unwrap()
        .id()
        .clone();
    let conversation_id = AIConversationId::new();

    block_list.set_agent_view_state(AgentViewState::Active {
        conversation_id,
        origin: AgentViewEntryOrigin::ThirdPartyCloudAgent,
        display_mode: AgentViewDisplayMode::FullScreen,
        original_conversation_length: 0,
    });

    block_list
        .finish_oz_environment_startup_commands_at_block(&harness_block_id, Some(conversation_id));

    assert!(!block_list.is_executing_oz_environment_startup_commands());

    for block_id in [&harness_block_id, &followup_block_id] {
        let block = block_list
            .block_with_id(block_id)
            .expect("block should still exist");
        assert!(!block.is_hidden());
        assert!(!block.is_oz_environment_startup_command());
        assert!(!block.should_hide_block(block_list.agent_view_state()));
        match block.agent_view_visibility() {
            AgentViewVisibility::Terminal {
                pending_conversation_ids,
                conversation_ids,
            } => {
                assert!(pending_conversation_ids.is_empty());
                assert!(conversation_ids.contains(&conversation_id));
            }
            AgentViewVisibility::Agent {
                origin_conversation_id,
                pending_other_conversation_ids,
                other_conversation_ids,
            } => panic!(
                "expected terminal visibility, got agent visibility: {origin_conversation_id:?}, {pending_other_conversation_ids:?}, {other_conversation_ids:?}"
            ),
        }
    }

    let setup_block = block_list
        .block_with_id(&setup_block_id)
        .expect("setup block should still exist");
    assert!(setup_block.is_hidden());
    assert!(setup_block.is_oz_environment_startup_command());
    assert!(setup_block.should_hide_block(block_list.agent_view_state()));
}

#[test]
pub fn test_seek_up_to_next_grid() {
    let mut block_list =
        new_bootstrapped_block_list(None, None, ChannelEventListener::new_for_test());

    insert_block(&mut block_list, "foo\nbar\nbazz\n", "");
    for _ in 0..2 {
        insert_block(&mut block_list, "foo\nbar\nbazz\n", "foo\nbar\nbazz\n");
    }

    // Start: Output grid of block 4
    // Expected End: PromptAndCommand grid of block 4
    let a = block_list
        .seek_up_to_next_grid(4.into(), GridType::Output, 0, false)
        .unwrap();
    assert_eq!(a.block_index, 4.into());
    assert_eq!(a.grid, GridType::PromptAndCommand);
    assert_eq!(a.inner.row, 2);
    assert_eq!(a.inner.col, 0);

    // Start: PromptAndCommand grid of block 4
    // Expected End:: Output grid of block 3
    let b = block_list
        .seek_up_to_next_grid(4.into(), GridType::PromptAndCommand, 0, false)
        .unwrap();
    assert_eq!(b.block_index, 3.into());
    assert_eq!(b.grid, GridType::Output);
    assert_eq!(b.inner.row, 2);
    assert_eq!(b.inner.col, 0);

    // Start: PromptAndCommand grid of block 4
    // Expected End:: Output grid of block 3
    let b = block_list
        .seek_up_to_next_grid(4.into(), GridType::PromptAndCommand, 0, false)
        .unwrap();
    assert_eq!(b.block_index, 3.into());
    assert_eq!(b.grid, GridType::Output);
    assert_eq!(b.inner.row, 2);
    assert_eq!(b.inner.col, 0);

    // Start: PromptAndCommand grid of block 3
    // Expected End: PromptAndCommand grid of block 2 (since there's no output grid in block 2)
    let c = block_list
        .seek_up_to_next_grid(3.into(), GridType::PromptAndCommand, 0, false)
        .unwrap();
    assert_eq!(c.block_index, 2.into());
    assert_eq!(c.grid, GridType::PromptAndCommand);
    assert_eq!(b.inner.row, 2);
    assert_eq!(b.inner.col, 0);

    // Start: PromptAndCommand grid of block 3
    // Expected End: PromptAndCommand grid of block 2 (since there's no output grid in block 2)
    let c = block_list
        .seek_up_to_next_grid(3.into(), GridType::PromptAndCommand, 0, false)
        .unwrap();
    assert_eq!(c.block_index, 2.into());
    assert_eq!(c.grid, GridType::PromptAndCommand);
    assert_eq!(b.inner.row, 2);
    assert_eq!(b.inner.col, 0);
}

#[test]
pub fn test_seek_up_to_next_grid_inverted_blocklist() {
    let mut block_list =
        new_bootstrapped_block_list(None, None, ChannelEventListener::new_for_test());

    for _ in 0..2 {
        insert_block(&mut block_list, "foo\nbar\nbazz\n", "foo\nbar\nbazz\n");
    }

    insert_block(&mut block_list, "foo\nbar\nbazz\n", "");

    // We've inserted 3 blocks (indices 2, 3, 4). 4 has no output grid.

    // Start: Output grid of block 3
    // Expected End: PromptAndCommand grid of block 3
    let a = block_list
        .seek_up_to_next_grid(3.into(), GridType::Output, 0, true)
        .unwrap();
    assert_eq!(a.block_index, 3.into());
    assert_eq!(a.grid, GridType::PromptAndCommand);
    assert_eq!(a.inner.row, 2);

    // Start: PromptAndCommand grid of block 2
    // Expected End: Output grid of block 3
    let a = block_list
        .seek_up_to_next_grid(2.into(), GridType::PromptAndCommand, 0, true)
        .unwrap();
    assert_eq!(a.block_index, 3.into());
    assert_eq!(a.grid, GridType::Output);
    assert_eq!(a.inner.row, 2);

    // Start: PromptAndCommand grid of block 4
    // Expected End:: None
    let b = block_list.seek_up_to_next_grid(4.into(), GridType::PromptAndCommand, 0, true);
    assert_eq!(b, None);

    // Start: PromptAndCommand grid of block 3
    // Expected End: PromptAndCommand grid of block 4 (since there's no output grid in block 4)
    let c = block_list
        .seek_up_to_next_grid(3.into(), GridType::PromptAndCommand, 0, true)
        .unwrap();
    assert_eq!(c.block_index, 4.into());
    assert_eq!(c.grid, GridType::PromptAndCommand);
    assert_eq!(c.inner.row, 2);
}

#[test]
pub fn clear_blocks_resets_index() {
    let mut block_list =
        new_bootstrapped_block_list(None, None, ChannelEventListener::new_for_test());

    // The above call adds 3 blocks to the block list.
    assert_eq!(block_list.blocks.len(), 3);

    // Create 4 extra blocks -- the block list should have a total size of 7.
    for _ in 0..4 {
        insert_block(&mut block_list, "foo\nbar\nbazz", "foo\nbar\nbazz");
    }

    assert_eq!(block_list.blocks.len(), 7);
    assert_eq!(block_list.active_block().index(), 6.into());

    // Clear the screen and ensure the block index is reset properly.
    block_list.clear_screen(ClearMode::ResetAndClear);
    assert_eq!(block_list.active_block().index(), 0.into());
}

#[test]
pub fn test_seek_down_to_next_grid() {
    let mut block_list =
        new_bootstrapped_block_list(None, None, ChannelEventListener::new_for_test());

    insert_block(&mut block_list, "foo\nbar\nbazz", "");
    for _ in 0..2 {
        insert_block(&mut block_list, "foo\nbar\nbazz", "foo\nbar\nbazz");
    }

    // Start: PromptAndCommand grid of block 2
    // Expected End: PromptAndCommand grid of block 3 (since there's no ooutput grid in block 2)
    let a = block_list
        .seek_down_to_next_grid(2.into(), GridType::PromptAndCommand, 2, false)
        .unwrap();
    assert_eq!(a.block_index, 3.into());
    assert_eq!(a.grid, GridType::PromptAndCommand);
    assert_eq!(a.inner.row, 0);
    assert_eq!(a.inner.col, 2);

    // Start: Output grid of block 2
    // Expected End: PromptAndCommand grid of block 3
    let b = block_list
        .seek_down_to_next_grid(2.into(), GridType::Output, 0, false)
        .unwrap();
    assert_eq!(b.block_index, 3.into());
    assert_eq!(b.grid, GridType::PromptAndCommand);
    assert_eq!(b.inner.row, 0);
    assert_eq!(b.inner.col, 0);

    // Start: PromptAndCommand grid of block 3
    // Expected End: Output grid of block 3
    let c = block_list
        .seek_down_to_next_grid(3.into(), GridType::PromptAndCommand, 0, false)
        .unwrap();
    assert_eq!(c.block_index, 3.into());
    assert_eq!(c.grid, GridType::Output);
    assert_eq!(b.inner.row, 0);
    assert_eq!(b.inner.col, 0);
}

#[test]
pub fn test_seek_down_to_next_grid_inverted_blocklist() {
    let mut block_list =
        new_bootstrapped_block_list(None, None, ChannelEventListener::new_for_test());

    for _ in 0..2 {
        block_list.start_active_block();
        input_string(&mut block_list, "foo");
        block_list.linefeed();
        input_string(&mut block_list, "bar");
        block_list.linefeed();
        input_string(&mut block_list, "bazz");
        block_list.linefeed();
        block_list.preexec(Default::default());
        input_string(&mut block_list, "foo");
        block_list.linefeed();
        input_string(&mut block_list, "bar");
        block_list.linefeed();
        input_string(&mut block_list, "bazz");
        block_list.linefeed();
        block_list.active_block_mut().finish(0);
        block_list.update_active_block_height();
        command_finished_and_precmd(&mut block_list);
    }

    block_list.start_active_block();
    input_string(&mut block_list, "foo");
    block_list.linefeed();
    input_string(&mut block_list, "bar");
    block_list.linefeed();
    input_string(&mut block_list, "bazz");
    block_list.linefeed();
    block_list.preexec(Default::default());
    block_list.active_block_mut().finish(0);
    block_list.update_active_block_height();
    command_finished_and_precmd(&mut block_list);

    // We've inserted 3 blocks (indices 2, 3, 4). 4 has no output grid.

    // Start: PromptAndCommand grid of block 3
    // Expected End: Output grid of block 3
    let a = block_list
        .seek_down_to_next_grid(3.into(), GridType::PromptAndCommand, 0, true)
        .unwrap();
    assert_eq!(a.block_index, 3.into());
    assert_eq!(a.grid, GridType::Output);
    assert_eq!(a.inner.row, 0);

    // Start: Output grid of block 3
    // Expected End: PromptAndCommand grid of block 2
    let a = block_list
        .seek_down_to_next_grid(3.into(), GridType::Output, 0, true)
        .unwrap();
    assert_eq!(a.block_index, 2.into());
    assert_eq!(a.grid, GridType::PromptAndCommand);
    assert_eq!(a.inner.row, 0);

    // Start: Output grid of block 2
    // Expected End:: None
    let b = block_list.seek_down_to_next_grid(2.into(), GridType::Output, 0, true);
    assert_eq!(b, None);

    // Start: PromptAndCommand grid of block 4
    // Expected End: PromptAndCommand grid of block 4 (since there's no output grid in block 4)
    let c = block_list
        .seek_down_to_next_grid(4.into(), GridType::PromptAndCommand, 0, true)
        .unwrap();
    assert_eq!(c.block_index, 3.into());
    assert_eq!(c.grid, GridType::PromptAndCommand);
    assert_eq!(c.inner.row, 0);
}

#[test]
pub fn test_emits_after_block_completed_event() {
    let (events_tx, events_rx) = async_channel::unbounded();

    let mut block_list = new_bootstrapped_block_list(
        None,
        None,
        ChannelEventListener::builder_for_test()
            .with_terminal_events_tx(events_tx)
            .build(),
    );
    block_list.start_active_block_for_in_band_command();
    block_list.preexec(PreexecValue {
        command: "warp_run_generator_command 1234 foo".to_owned(),
    });
    command_finished_and_precmd(&mut block_list);

    block_list.start_active_block();
    block_list.preexec(PreexecValue {
        command: "some user command".to_owned(),
    });
    command_finished_and_precmd(&mut block_list);

    let mut after_block_completed_events = Vec::new();
    while let Ok(event) = events_rx.try_recv() {
        if let Event::AfterBlockCompleted(event) = event {
            if matches!(event.block_type, BlockType::InBandCommand)
                || matches!(event.block_type, BlockType::User(..))
            {
                after_block_completed_events.push(event);
            }
        }
    }
    assert_eq!(after_block_completed_events.len(), 2);
    assert!(matches!(
        after_block_completed_events[0].block_type,
        BlockType::InBandCommand
    ));
    assert!(matches!(
        after_block_completed_events[1].block_type,
        BlockType::User(..)
    ));
}

#[test]
fn test_background_blocks_finished() {
    let (events_tx, events_rx) = async_channel::unbounded();
    let mut block_list = new_bootstrapped_block_list(
        None,
        None,
        ChannelEventListener::builder_for_test()
            .with_terminal_events_tx(events_tx)
            .build(),
    );
    command_finished_and_precmd(&mut block_list);

    // Flush events from bootstrapping.
    while let Ok(_event) = events_rx.try_recv() {}

    // The block list should contain the bootstrap blocks, one completed user
    // block, a live background block, and the active block.
    insert_block(&mut block_list, "command\n", "output\n");
    input_string(&mut block_list, "background");

    let mut block_completed_events = vec![];
    while let Ok(event) = events_rx.try_recv() {
        if let Event::AfterBlockCompleted(event_content) = event {
            block_completed_events.push(event_content);
        }
    }

    // At this point, the user block has completed, but not the background block.
    assert_eq!(block_completed_events.len(), 1);
    match &block_completed_events[0].block_type {
        BlockType::User(block) => {
            assert_eq!(&block.command, "command");
            assert_eq!(&block.output_truncated_with_obfuscated_secrets, "output");
        }
        other => panic!("Expected BlockType::User, but was {other:?}"),
    }

    // Running an in-band command should not finish the background block.
    block_list.start_active_block_for_in_band_command();
    block_list.preexec(PreexecValue {
        command: "warp_run_generator_command abc".to_owned(),
    });
    command_finished_and_precmd(&mut block_list);

    while let Ok(event) = events_rx.try_recv() {
        if let Event::AfterBlockCompleted(event_content) = event {
            assert!(matches!(event_content.block_type, BlockType::InBandCommand));
        }
    }

    // When this block finishes, it also finishes the previous background block.
    insert_block(&mut block_list, "next command\n", "next command output\n");

    while let Ok(event) = events_rx.try_recv() {
        if let Event::AfterBlockCompleted(event_content) = event {
            block_completed_events.push(event_content);
        }
    }
    // There's now a completion event for the first user block, one for the
    // background block, and one for the second user block. Likewise, the block
    // list now contains the bootstrap blocks, the first user block, the background
    // block, the in-band generator block, the second user block, and the active block.
    assert_eq!(block_completed_events.len(), 3);
    assert_eq!(block_list.blocks().len(), 8);

    match &block_completed_events[1].block_type {
        BlockType::Background(block) => {
            assert!(block.is_background);
            assert!(block.stylized_command.is_empty());
            assert_eq!(
                std::str::from_utf8(&block.stylized_output),
                Ok("background\r\n")
            );
        }
        other => panic!("Expected BlockType::BackgroundOutput, but was {other:?}"),
    }

    match &block_completed_events[2].block_type {
        BlockType::User(block) => {
            assert_eq!(&block.command, "next command");
            assert_eq!(
                &block.output_truncated_with_obfuscated_secrets,
                "next command output"
            );
        }
        other => panic!("Expected BlockType::User, but was {other:?}"),
    }
}

#[test]
fn test_interleaves_background_with_gaps() {
    let mut block_list =
        new_bootstrapped_block_list(None, None, ChannelEventListener::new_for_test());
    block_list.set_next_gap_height_in_lines(17.0.into_lines());

    insert_block(&mut block_list, "some background command &\n", "\n");
    input_string(&mut block_list, "bg1");
    block_list.carriage_return();
    block_list.linefeed();
    block_list.clear_visible_screen();
    assert_lines_approx_eq!(block_list.active_gap().unwrap().height(), 17.0);
    input_string(&mut block_list, "bg2");
    block_list.carriage_return();
    block_list.linefeed();
    block_list.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));

    // There are 2 bootstrap blocks, 1 command block, 2 background blocks, and 1 active block.
    assert_eq!(block_list.blocks.len(), 6);

    assert_eq!(
        &block_list.blocks[2].command_to_string(),
        "some background command &"
    );
    assert_eq!(&block_list.blocks[3].output_to_string(), "bg1");
    assert!(block_list.blocks[3].is_background());
    assert!(block_list.blocks[3].finished());
    // The second background block isn't finished, so the trailing \n has not
    // been stripped off yet.
    assert_eq!(&block_list.blocks[4].output_to_string(), "bg2\n");
    assert!(block_list.blocks[4].is_background());
    assert!(!block_list.blocks[4].finished());

    let expected_heights = [
        BlockHeightItem::Block(0.0.into()),
        BlockHeightItem::Block(0.0.into()),
        BlockHeightItem::Block(7.5.into()),
        // The first background block should be before the gap.
        BlockHeightItem::Block(2.2.into()),
        // The second background block should have shrunk the gap.
        BlockHeightItem::Gap(14.6.into()),
        BlockHeightItem::Block(2.4.into()),
        // The active block has 0 height.
        BlockHeightItem::Block(0.0.into()),
    ];
    assert_eq!(
        block_list.block_heights.items().len(),
        expected_heights.len()
    );
    for (actual, expected) in block_list
        .block_heights
        .items()
        .iter()
        .zip(expected_heights.iter())
    {
        // Make sure the items are of the same type.
        assert_eq!(
            std::mem::discriminant(actual),
            std::mem::discriminant(expected)
        );
        // Make sure the heights are the same.
        assert_approx_eq!(
            Lines,
            actual.height().into_lines(),
            expected.height().into_lines()
        );
    }
}

#[test]
fn test_remove_background_block_with_active_gap() {
    let mut block_list =
        new_bootstrapped_block_list(None, None, ChannelEventListener::new_for_test());

    block_list.set_next_gap_height_in_lines(17.0.into_lines());
    block_list.clear_visible_screen();

    input_string(&mut block_list, "echo foo");

    assert!(block_list.active_gap().is_some());

    // This is the real part of the test: if the fix is not working properly,
    // this will panic on a debug_assert
    block_list.remove_background_block();

    assert!(block_list.active_gap().is_some());
}

#[test]
fn test_device_status_uses_active_block_if_no_typeahead() {
    let mut block_list =
        new_bootstrapped_block_list(None, None, ChannelEventListener::new_for_test());

    insert_block(&mut block_list, "command\n", "output\n");
    let active_block = block_list.active_block_mut();
    let grid = active_block
        .grid_of_type_mut(active_block.active_grid_type())
        .expect("should have grid");
    grid.grid_handler_mut().update_cursor(|cursor| {
        cursor.point.col = 20;
    });
    assert_eq!(
        block_list.active_block().grid_handler().cursor_point(),
        Point { row: 0, col: 20 }
    );

    let mut writer = Vec::new();

    block_list.device_status(&mut writer, 6);

    assert_eq!(writer, "\x1b[1;21R".as_bytes());
}
