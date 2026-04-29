use crate::ai::blocklist::agent_view::AgentViewState;
use crate::terminal::model::block::SerializedBlock;
use crate::terminal::shared_session::tests::terminal_model_for_viewer;
use crate::terminal::TerminalView;
use crate::terminal::{
    event_listener::ChannelEventListener, shared_session::viewer::event_loop::EventLoop,
};
use crate::test_util::add_window_with_terminal;
use crate::test_util::terminal::initialize_app_for_terminal_view;

use parking_lot::FairMutex;
use session_sharing_protocol::common::{
    OrderedTerminalEvent, OrderedTerminalEventType, Scrollback, ScrollbackBlock, WindowSize,
};
use std::sync::Arc;
use warpui::units::Lines;
use warpui::{App, ViewHandle};

fn ordered_terminal_event_from_bytes(
    bytes: impl Into<Vec<u8>>,
    event_no: usize,
) -> OrderedTerminalEvent {
    let compressed = lz4_flex::block::compress_prepend_size(&bytes.into());
    OrderedTerminalEvent {
        event_no,
        event_type: OrderedTerminalEventType::PtyBytesRead { bytes: compressed },
    }
}

fn terminal_view(app: &mut App) -> ViewHandle<TerminalView> {
    initialize_app_for_terminal_view(app);
    add_window_with_terminal(app, None)
}

#[test]
fn test_terminal_model_is_correct() {
    App::test((), |mut app| async move {
        let channel_event_proxy = ChannelEventListener::new_for_test();
        let model = Arc::new(FairMutex::new(terminal_model_for_viewer(
            channel_event_proxy.clone(),
        )));

        let terminal_view = terminal_view(&mut app);
        let event_loop = app.add_model(|ctx| {
            EventLoop::new(
                model.clone(),
                terminal_view.downgrade(),
                channel_event_proxy.clone(),
                WindowSize {
                    num_rows: 0,
                    num_cols: 0,
                },
                Scrollback {
                    blocks: vec![],
                    is_alt_screen_active: false,
                },
                None,
                ctx,
            )
        });

        // Before we receive any events, the block list only contains hidden blocks.
        assert!(model
            .lock()
            .block_list()
            .blocks()
            .iter()
            .all(|block| block.height(&AgentViewState::Inactive) == Lines::zero()));

        // Load shared session scrollback.
        let scrollback = &[
            SerializedBlock::new_for_test("block1".into(), "block1".into()),
            SerializedBlock::new_active_block_for_test(),
        ];
        {
            let mut model = model.lock();
            model.load_shared_session_scrollback(scrollback, false);
            // A hidden block, a completed scrollback block, then the active block.
            assert_eq!(model.block_list().blocks().len(), 3);
            assert_eq!(
                model.block_list().blocks()[0].height(&AgentViewState::Inactive),
                Lines::zero()
            );
            assert_ne!(
                model.block_list().blocks()[1].height(&AgentViewState::Inactive),
                Lines::zero()
            );
            assert_eq!(
                model.block_list().blocks()[2].height(&AgentViewState::Inactive),
                Lines::zero()
            );
        }

        // Write some PTY events after starting active block.
        model.lock().start_command_execution();
        event_loop.update(&mut app, |event_loop, ctx| {
            event_loop
                .process_ordered_terminal_event(ordered_terminal_event_from_bytes("a", 0), ctx);
        });

        let model = model.lock();
        // After writing bytes, active block should no longer have height 0.
        assert_eq!(model.block_list().blocks().len(), 3);
        assert_eq!(
            model.block_list().blocks()[0].height(&AgentViewState::Inactive),
            Lines::zero()
        );
        assert_ne!(
            model.block_list().blocks()[1].height(&AgentViewState::Inactive),
            Lines::zero()
        );
        assert_ne!(
            model.block_list().blocks()[2].height(&AgentViewState::Inactive),
            Lines::zero()
        );
    })
}

#[test]
fn test_out_of_order_buffering() {
    App::test((), |mut app| async move {
        let channel_event_proxy = ChannelEventListener::new_for_test();
        let model = Arc::new(FairMutex::new(terminal_model_for_viewer(
            channel_event_proxy.clone(),
        )));

        let terminal_view = terminal_view(&mut app);
        let active_block: SerializedBlock = model.lock().block_list().active_block().into();
        let event_loop = app.add_model(|ctx| {
            EventLoop::new(
                model.clone(),
                terminal_view.downgrade(),
                channel_event_proxy.clone(),
                WindowSize {
                    num_rows: 0,
                    num_cols: 0,
                },
                Scrollback {
                    blocks: vec![ScrollbackBlock {
                        raw: serde_json::to_vec(&active_block).unwrap(),
                    }],
                    is_alt_screen_active: false,
                },
                None,
                ctx,
            )
        });

        // Simulate the real event flow: CommandExecutionStarted (event_no 0) arrives first,
        // then PTY bytes (event_no 1-3) potentially in out-of-order sequence.
        event_loop.update(&mut app, |event_loop, ctx| {
            // First: sharer sends CommandExecutionStarted when user executes a command
            event_loop.process_ordered_terminal_event(
                OrderedTerminalEvent {
                    event_no: 0,
                    event_type: OrderedTerminalEventType::CommandExecutionStarted {
                        participant_id: Default::default(),
                        ai_metadata: None,
                    },
                },
                ctx,
            );

            // Then: PTY bytes arrive (potentially out of order)
            event_loop
                .process_ordered_terminal_event(ordered_terminal_event_from_bytes("c", 3), ctx);
            event_loop
                .process_ordered_terminal_event(ordered_terminal_event_from_bytes("b", 2), ctx);
            event_loop
                .process_ordered_terminal_event(ordered_terminal_event_from_bytes("a", 1), ctx);
        });

        // Ensure the events were applied in the right order.
        let command_grid = model
            .lock()
            .block_list()
            .active_block()
            .command_to_string()
            .trim()
            .to_string();
        assert_eq!(command_grid, "abc");
    })
}

#[test]
fn test_pty_bytes_buffered_before_command_execution_started() {
    App::test((), |mut app| async move {
        let channel_event_proxy = ChannelEventListener::new_for_test();
        let model = Arc::new(FairMutex::new(terminal_model_for_viewer(
            channel_event_proxy.clone(),
        )));

        let terminal_view = terminal_view(&mut app);
        let active_block: SerializedBlock = model.lock().block_list().active_block().into();
        let event_loop = app.add_model(|ctx| {
            EventLoop::new(
                model.clone(),
                terminal_view.downgrade(),
                channel_event_proxy.clone(),
                WindowSize {
                    num_rows: 0,
                    num_cols: 0,
                },
                Scrollback {
                    blocks: vec![ScrollbackBlock {
                        raw: serde_json::to_vec(&active_block).unwrap(),
                    }],
                    is_alt_screen_active: false,
                },
                None,
                ctx,
            )
        });

        // Edge case: PTY bytes arrive BEFORE CommandExecutionStarted.
        // The event loop should buffer the PTY bytes until CommandExecutionStarted arrives,
        // then process them in order.
        event_loop.update(&mut app, |event_loop, ctx| {
            // PTY bytes arrive first (event_no 0-2, out of order)
            event_loop
                .process_ordered_terminal_event(ordered_terminal_event_from_bytes("c", 2), ctx);
            event_loop
                .process_ordered_terminal_event(ordered_terminal_event_from_bytes("a", 0), ctx);

            // CommandExecutionStarted arrives later (event_no 3)
            event_loop.process_ordered_terminal_event(
                OrderedTerminalEvent {
                    event_no: 3,
                    event_type: OrderedTerminalEventType::CommandExecutionStarted {
                        participant_id: Default::default(),
                        ai_metadata: None,
                    },
                },
                ctx,
            );

            // More PTY bytes arrive after CommandExecutionStarted (event_no 4)
            event_loop
                .process_ordered_terminal_event(ordered_terminal_event_from_bytes("b", 1), ctx);
        });

        // Ensure the buffering worked correctly and bytes were applied in the right order.
        // Note: The first two bytes (0, 2) arrive before CommandExecutionStarted,
        // but since the block isn't started until event 3, they should be buffered.
        // After CommandExecutionStarted, the block is started and we process in order: 0, 1, 2.
        let command_grid = model
            .lock()
            .block_list()
            .active_block()
            .command_to_string()
            .trim()
            .to_string();
        assert_eq!(command_grid, "abc");
    })
}
