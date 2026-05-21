use super::*;
use crate::terminal::color;
use crate::terminal::model::ansi::{Handler, Processor};
use crate::terminal::model::block::BlockId;
use crate::terminal::model::bootstrap::BootstrapStage;
use crate::terminal::model::grid::Dimensions as _;
use crate::terminal::model::index::Side;
use crate::terminal::model::selection::ExpandedSelectionRange;
use crate::terminal::model::test_utils::block_size;
use crate::terminal::model::ObfuscateSecrets;
use crate::terminal::{event_listener::ChannelEventListener, shared_session::SharedSessionStatus};
use chrono::{DateTime, Local};
use std::sync::Arc;
use vec1::vec1;
use warp_core::command::ExitCode;
use warp_terminal::model::ansi::ClearMode;
use warpui::r#async::executor::Background;
use warpui::text::str_to_byte_vec;
use warpui::text::SelectionType;

/// Helper function to create a SerializedBlock with default values,
/// including the new is_local field.
fn create_default_serialized_block() -> SerializedBlock {
    SerializedBlock {
        id: BlockId::new(),
        stylized_command: Default::default(),
        stylized_output: Default::default(),
        pwd: None,
        git_head: None,
        git_branch_name: None,
        virtual_env: None,
        conda_env: None,
        node_version: None,
        exit_code: ExitCode::from(0),
        did_execute: false,
        start_ts: Some(Local::now()),
        completed_ts: Some(Local::now()),
        ps1: None,
        rprompt: None,
        honor_ps1: false,
        session_id: None,
        shell_host: None,
        is_background: false,
        prompt_snapshot: None,
        ai_metadata: None,
        is_local: None,
        agent_view_visibility: None,
    }
}

#[test]
fn cloud_mode_deferred_terminal_model_starts_view_pending() {
    let mut model = TerminalModel::new_for_cloud_mode_shared_session_viewer(
        block_size(),
        color::List::from(&color::Colors::default()),
        ChannelEventListener::new_for_test(),
        Arc::new(Background::default()),
        false,
        false,
        false,
        ObfuscateSecrets::No,
    );

    assert!(matches!(
        model.shared_session_status(),
        SharedSessionStatus::ViewPending
    ));
    assert!(model.shared_session_status().is_viewer());
    assert!(model.is_dummy_cloud_mode_session());
    assert!(!model
        .block_list()
        .is_executing_oz_environment_startup_commands());

    let restored_block = SerializedBlock {
        id: BlockId::new(),
        stylized_command: str_to_byte_vec("setup-looking-command"),
        stylized_output: str_to_byte_vec("output"),
        did_execute: true,
        start_ts: Some(Local::now()),
        completed_ts: Some(Local::now()),
        ..Default::default()
    };
    model
        .block_list_mut()
        .insert_restored_block(&restored_block);

    let restored_command_block = model
        .block_list()
        .blocks()
        .iter()
        .find(|block| block.command_to_string() == "setup-looking-command")
        .expect("restored command block should exist");
    assert!(!restored_command_block.is_hidden());
    assert!(!restored_command_block.is_oz_environment_startup_command());
}

#[test]
fn generic_shared_session_viewer_model_starts_view_pending() {
    let model = TerminalModel::new_for_shared_session_viewer(
        block_size(),
        color::List::from(&color::Colors::default()),
        ChannelEventListener::new_for_test(),
        Arc::new(Background::default()),
        false,
        false,
        false,
        ObfuscateSecrets::No,
    );

    assert!(matches!(
        model.shared_session_status(),
        SharedSessionStatus::ViewPending
    ));
    assert!(model.shared_session_status().is_viewer());
}

// Ensures that an ssh session successfully bootstraps even if the block list is empty.
#[test]
fn ssh_bootstraps_if_blocklist_empty() {
    let mut terminal = TerminalModel::mock(None, None);
    terminal.command_finished(Default::default());
    terminal.precmd(Default::default());
    terminal.command_finished(Default::default());
    terminal.precmd(Default::default());

    let bootstrapped_value = BootstrappedValue {
        histfile: None,
        shell: String::from("bash"),
        home_dir: None,
        path: None,
        cdpath: None,
        editor: None,
        env_var_names: None,
        aliases: None,
        abbreviations: None,
        function_names: None,
        builtins: None,
        keywords: None,
        shell_version: None,
        shell_options: None,
        shell_plugins: None,
        rcfiles_start_time: None,
        rcfiles_end_time: None,
        vi_mode_enabled: None,
        os_category: None,
        linux_distribution: None,
        wsl_name: None,
        shell_path: None,
    };
    terminal.bootstrapped(bootstrapped_value.clone());
    terminal.command_finished(Default::default());
    terminal.block_list_mut().precmd(Default::default());

    assert!(terminal.is_active_block_bootstrapped());

    // Clear all the blocks in the blocklist.
    terminal.clear_screen(ClearMode::ResetAndClear);

    terminal.ssh(SSHValue::default());
    terminal.init_shell(InitShellValue {
        shell: "bash".into(),
        user: "zach".to_owned(),
        hostname: "sf".to_owned(),
        session_id: 0.into(),
        ..Default::default()
    });

    // The active block should no longer be considered bootstrapped after the init shell call.
    assert!(!terminal.is_active_block_bootstrapped());

    terminal.command_finished(Default::default());
    terminal.precmd(PrecmdValue::default());
    terminal.command_finished(Default::default());
    terminal.precmd(Default::default());
    terminal.bootstrapped(bootstrapped_value);
    terminal.command_finished(Default::default());
    terminal.precmd(Default::default());

    assert!(terminal.is_active_block_bootstrapped());
}

#[test]
// An empty block that is restored should have a nonzero height and it should not get deleted.
pub fn test_restored_empty_command_block() {
    let restored_blocks = [create_default_serialized_block().into()];
    let model = TerminalModel::mock(Some(&restored_blocks), None);
    let restored_block = &model.block_list().blocks()[0];
    assert_eq!(
        restored_block.bootstrap_stage(),
        BootstrapStage::RestoreBlocks
    );
    assert!(
        !restored_block.is_command_empty(),
        "The empty block should have nonzero length"
    );
    // The mocked terminal model comes with a WarpInput block and the active block.
    assert_eq!(model.block_list().blocks().len(), 3);
}

/// Saved blocks that run on ANY hosts/shells should still get restored to the block list
/// during session restoration. This test makes sure blocks from various hosts/shells get
/// restored.
#[test]
fn test_restored_blocks_on_different_host() {
    let restored_blocks = [
        SerializedBlock {
            id: BlockId::new(),
            stylized_command: str_to_byte_vec("echo $TERM_PROGRAM"),
            stylized_output: str_to_byte_vec("WarpTerminal"),
            pwd: Some("/".to_owned()),
            git_head: None,
            git_branch_name: None,
            virtual_env: None,
            conda_env: None,
            node_version: None,
            exit_code: ExitCode::from(0),
            did_execute: true,
            completed_ts: Some(
                DateTime::from_timestamp(1671210994, 100)
                    .unwrap()
                    .with_timezone(&Local),
            ),
            start_ts: Some(
                DateTime::from_timestamp(1671210994, 0)
                    .unwrap()
                    .with_timezone(&Local),
            ),
            ps1: None,
            rprompt: None,
            honor_ps1: false,
            session_id: None,
            shell_host: Some(ShellHost {
                shell_type: ShellType::Zsh,
                user: "local:user".to_owned(),
                hostname: "local:host".to_owned(),
            }),
            is_background: false,
            prompt_snapshot: None,
            ai_metadata: None,
            is_local: Some(true),
            agent_view_visibility: None,
        }
        .into(),
        SerializedBlock {
            id: BlockId::new(),
            stylized_command: str_to_byte_vec("pwd"),
            stylized_output: str_to_byte_vec("/"),
            pwd: Some("/".to_owned()),
            git_head: None,
            git_branch_name: None,
            virtual_env: None,
            conda_env: None,
            node_version: None,
            exit_code: ExitCode::from(0),
            did_execute: true,
            completed_ts: Some(
                DateTime::from_timestamp(1671210995, 100)
                    .unwrap()
                    .with_timezone(&Local),
            ),
            start_ts: Some(
                DateTime::from_timestamp(1671210995, 0)
                    .unwrap()
                    .with_timezone(&Local),
            ),
            ps1: None,
            rprompt: None,
            honor_ps1: false,
            session_id: None,
            shell_host: Some(ShellHost {
                shell_type: ShellType::Bash,
                user: "local:user".to_owned(),
                hostname: "local:host".to_owned(),
            }),
            is_background: false,
            prompt_snapshot: None,
            ai_metadata: None,
            is_local: Some(true),
            agent_view_visibility: None,
        }
        .into(),
        SerializedBlock {
            id: BlockId::new(),
            stylized_command: str_to_byte_vec("uname"),
            stylized_output: str_to_byte_vec("Linux"),
            pwd: Some("/".to_owned()),
            git_head: None,
            git_branch_name: None,
            virtual_env: None,
            conda_env: None,
            node_version: None,
            exit_code: ExitCode::from(0),
            did_execute: true,
            completed_ts: Some(
                DateTime::from_timestamp(1671210996, 100)
                    .unwrap()
                    .with_timezone(&Local),
            ),
            start_ts: Some(
                DateTime::from_timestamp(1671210996, 0)
                    .unwrap()
                    .with_timezone(&Local),
            ),
            ps1: None,
            rprompt: None,
            honor_ps1: false,
            session_id: None,
            shell_host: Some(ShellHost {
                shell_type: ShellType::Zsh,
                user: "root".to_owned(),
                hostname: "webserver".to_owned(),
            }),
            is_background: false,
            prompt_snapshot: None,
            ai_metadata: None,
            is_local: Some(false),
            agent_view_visibility: None,
        }
        .into(),
        SerializedBlock {
            id: BlockId::new(),
            stylized_command: str_to_byte_vec("mkdir secrets"),
            stylized_output: str_to_byte_vec("secrets"),
            pwd: Some("/etc".to_owned()),
            git_head: None,
            git_branch_name: None,
            virtual_env: None,
            conda_env: None,
            node_version: None,
            exit_code: ExitCode::from(0),
            did_execute: true,
            completed_ts: Some(
                DateTime::from_timestamp(1671210997, 100)
                    .unwrap()
                    .with_timezone(&Local),
            ),
            start_ts: Some(
                DateTime::from_timestamp(1671210997, 0)
                    .unwrap()
                    .with_timezone(&Local),
            ),
            ps1: None,
            rprompt: None,
            honor_ps1: false,
            session_id: None,
            shell_host: None,
            is_background: false,
            prompt_snapshot: None,
            ai_metadata: None,
            is_local: Some(true),
            agent_view_visibility: None,
        }
        .into(),
    ];
    let model = TerminalModel::mock(Some(&restored_blocks), None);
    // The mocked terminal model comes with a WarpInput block and the active block.
    assert_eq!(model.block_list().blocks().len(), restored_blocks.len() + 2);
    for restored_block in model
        .block_list()
        .blocks()
        .iter()
        .take(restored_blocks.len())
    {
        assert_eq!(
            restored_block.bootstrap_stage(),
            BootstrapStage::RestoreBlocks,
        );
    }
    let blocks = model.block_list().blocks();
    assert_eq!(blocks[0].command_to_string(), "echo $TERM_PROGRAM",);
    assert_eq!(blocks[0].output_to_string(), "WarpTerminal",);
    assert_eq!(blocks[1].command_to_string(), "pwd",);
    assert_eq!(blocks[1].output_to_string(), "/",);
    assert_eq!(blocks[2].command_to_string(), "uname",);
    assert_eq!(blocks[2].output_to_string(), "Linux",);
    assert_eq!(blocks[3].command_to_string(), "mkdir secrets",);
    assert_eq!(blocks[3].output_to_string(), "secrets",);
}

#[test]
fn test_selected_block_range_contains() {
    let range = SelectedBlockRange {
        pivot: 10.into(),
        tail: 2.into(),
    };
    assert!(range.contains(4.into()));
    assert!(!range.contains(1.into()));
}

#[test]
fn test_selected_blocks_is_selected() {
    let selected_blocks = SelectedBlocks {
        ranges: vec![
            SelectedBlockRange {
                pivot: 3.into(),
                tail: 4.into(),
            },
            SelectedBlockRange {
                pivot: 0.into(),
                tail: 1.into(),
            },
        ],
    };

    assert!(selected_blocks.is_selected(0.into()));
    assert!(!selected_blocks.is_selected(2.into()));
}

#[test]
fn test_selected_blocks_tail() {
    let selected_blocks = SelectedBlocks {
        ranges: vec![
            SelectedBlockRange {
                pivot: 3.into(),
                tail: 4.into(),
            },
            SelectedBlockRange {
                pivot: 0.into(),
                tail: 1.into(),
            },
        ],
    };

    assert_eq!(selected_blocks.tail(), Some(1.into()));
}

#[test]
fn test_selected_blocks_reset() {
    let mut selected_blocks = SelectedBlocks {
        ranges: vec![
            SelectedBlockRange {
                pivot: 3.into(),
                tail: 4.into(),
            },
            SelectedBlockRange {
                pivot: 0.into(),
                tail: 1.into(),
            },
        ],
    };

    // Reset to nothing.
    selected_blocks.reset();
    assert!(selected_blocks.is_empty());
    assert!(selected_blocks.tail().is_none());

    // Reset to single.
    selected_blocks.reset_to_single(5.into());
    assert!(!selected_blocks.is_empty());
    assert_eq!(selected_blocks.tail(), Some(5.into()));
}

#[test]
fn test_selected_blocks_range_select() {
    let mut selected_blocks = SelectedBlocks {
        ranges: vec![
            SelectedBlockRange {
                pivot: 0.into(),
                tail: 1.into(),
            },
            SelectedBlockRange {
                pivot: 3.into(),
                tail: 5.into(),
            },
        ],
    };

    // Range select should reset to a single selection with
    // same pivot as most recent selection, and new tail.
    selected_blocks.range_select(6.into());
    assert_eq!(selected_blocks.ranges().len(), 1);
    assert_eq!(selected_blocks.ranges().last().unwrap().pivot, 3.into());
    assert_eq!(selected_blocks.tail(), Some(6.into()));

    // Range select reversed should work similarly.
    selected_blocks.ranges = vec![
        SelectedBlockRange {
            pivot: 0.into(),
            tail: 1.into(),
        },
        SelectedBlockRange {
            pivot: 3.into(),
            tail: 5.into(),
        },
    ];
    selected_blocks.range_select(0.into());
    assert_eq!(selected_blocks.ranges().len(), 1);
    assert_eq!(selected_blocks.ranges().last().unwrap().pivot, 3.into());
    assert_eq!(selected_blocks.tail(), Some(0.into()));
}

#[test]
fn test_selected_blocks_sorted_ranges() {
    let selected_blocks = SelectedBlocks {
        ranges: vec![
            SelectedBlockRange {
                pivot: 0.into(),
                tail: 1.into(),
            },
            SelectedBlockRange {
                pivot: 3.into(),
                tail: 5.into(),
            },
        ],
    };

    let sorted_ranges = selected_blocks.sorted_ranges(BlockSortDirection::MostRecentLast);
    assert_eq!(2, sorted_ranges.len());
    assert_eq!(sorted_ranges.first().unwrap().start(), 0.into());
    assert_eq!(sorted_ranges.first().unwrap().end(), 1.into());
    assert_eq!(sorted_ranges.last().unwrap().start(), 3.into());
    assert_eq!(sorted_ranges.last().unwrap().end(), 5.into());

    let reverse_sorted = selected_blocks.sorted_ranges(BlockSortDirection::MostRecentFirst);
    assert_eq!(2, sorted_ranges.len());
    assert_eq!(
        reverse_sorted
            .first()
            .unwrap()
            .range(Some(BlockSortDirection::MostRecentFirst))
            .next()
            .unwrap(),
        5.into()
    );
    assert_eq!(
        reverse_sorted
            .last()
            .unwrap()
            .range(Some(BlockSortDirection::MostRecentFirst))
            .last()
            .unwrap(),
        0.into()
    );
}

#[test]
fn test_selected_blocks_toggle_on() {
    let mut selected_blocks = SelectedBlocks {
        ranges: vec![SelectedBlockRange {
            pivot: 0.into(),
            tail: 3.into(),
        }],
    };

    // Toggle a disjoint selection - should toggle ON
    selected_blocks.toggle(6.into(), Some(7.into()), Some(5.into()));
    assert_eq!(selected_blocks.ranges().len(), 2);
    assert_eq!(selected_blocks.ranges().last().unwrap().pivot, 6.into());
    assert_eq!(selected_blocks.ranges().last().unwrap().tail, 6.into());
    assert!(selected_blocks.is_selected(6.into()));

    // Toggle a selection before another, should merge
    selected_blocks.toggle(5.into(), Some(6.into()), Some(4.into()));
    assert_eq!(selected_blocks.ranges().len(), 2);
    assert_eq!(selected_blocks.ranges().last().unwrap().pivot, 6.into());
    assert_eq!(selected_blocks.ranges().last().unwrap().tail, 5.into());
    assert!(selected_blocks.is_selected(5.into()));

    // Toggle a selection at the end of another, should merge
    selected_blocks.toggle(7.into(), Some(8.into()), Some(6.into()));
    assert_eq!(selected_blocks.ranges().len(), 2);
    assert_eq!(selected_blocks.ranges().last().unwrap().pivot, 7.into());
    assert_eq!(selected_blocks.ranges().last().unwrap().tail, 5.into());
    assert!(selected_blocks.is_selected(7.into()));

    // Toggle a selection between two existing selections, should merge
    selected_blocks.toggle(4.into(), Some(5.into()), Some(3.into()));
    assert_eq!(selected_blocks.ranges().len(), 1);
    assert_eq!(selected_blocks.ranges().last().unwrap().pivot, 0.into());
    assert_eq!(selected_blocks.ranges().last().unwrap().tail, 7.into());
    assert!(selected_blocks.is_selected(4.into()));
}

#[test]
fn test_selected_blocks_toggle_off() {
    let mut selected_blocks = SelectedBlocks {
        ranges: vec![
            SelectedBlockRange {
                pivot: 2.into(),
                tail: 2.into(),
            },
            SelectedBlockRange {
                pivot: 0.into(),
                tail: 1.into(),
            },
            SelectedBlockRange {
                pivot: 25.into(),
                tail: 20.into(),
            },
            SelectedBlockRange {
                pivot: 3.into(),
                tail: 10.into(),
            },
        ],
    };

    // Case 1: deselect the entire selection range
    selected_blocks.toggle(2.into(), Some(3.into()), Some(1.into()));
    assert_eq!(selected_blocks.ranges().len(), 3);
    assert!(!selected_blocks.is_selected(2.into()));

    // Case 2: deselect the pivot and tail for non-reversed range
    selected_blocks.toggle(25.into(), Some(26.into()), Some(24.into())); // deselect pivot
    selected_blocks.toggle(20.into(), Some(21.into()), Some(19.into())); // deselect tail
    assert_eq!(selected_blocks.ranges().len(), 3);
    assert_eq!(selected_blocks.ranges()[1].pivot, 24.into());
    assert_eq!(selected_blocks.ranges()[1].tail, 21.into());
    assert!(!selected_blocks.is_selected(25.into()));
    assert!(!selected_blocks.is_selected(20.into()));

    // Case 2: deselect the pivot and tail for reversed range
    selected_blocks.toggle(3.into(), Some(4.into()), Some(2.into())); // deselect pivot
    selected_blocks.toggle(10.into(), Some(11.into()), Some(9.into())); // deselect tail
    assert_eq!(selected_blocks.ranges().len(), 3);
    assert_eq!(selected_blocks.ranges()[2].pivot, 4.into());
    assert_eq!(selected_blocks.ranges()[2].tail, 9.into());
    assert!(!selected_blocks.is_selected(3.into()));
    assert!(!selected_blocks.is_selected(10.into()));

    // Case 3: deselect in the middle of range
    selected_blocks.toggle(6.into(), Some(7.into()), Some(5.into()));
    assert_eq!(selected_blocks.ranges().len(), 4);
    assert_eq!(selected_blocks.ranges()[2].pivot, 7.into());
    assert_eq!(selected_blocks.ranges()[2].tail, 9.into());
    assert_eq!(selected_blocks.ranges()[3].pivot, 4.into());
    assert_eq!(selected_blocks.ranges()[3].tail, 5.into());
    assert!(!selected_blocks.is_selected(6.into()));
}

fn validate_title_event(event: Result<Event, async_channel::TryRecvError>, expected_title: String) {
    match event {
        Ok(Event::Title(title)) => assert_eq!(expected_title, title),
        _ => panic!("Expected Event::Title({expected_title}), got: {event:?}"),
    }
}

#[test]
fn set_custom_title() {
    let (event_tx, event_rx) = async_channel::unbounded();
    let event_proxy = ChannelEventListener::builder_for_test()
        .with_terminal_events_tx(event_tx)
        .build();
    let mut terminal = TerminalModel::mock(None, Some(event_proxy));

    // Empty all the events that could've been sent to this channel prior to us changing the
    // title for tests.
    while !event_rx.is_empty() {
        let _ = event_rx.try_recv();
    }

    // Validate that we send the title event when the new custom title is set.
    let custom_title = "Custom title".to_string();
    terminal.set_custom_title(Some(custom_title.clone()));
    assert!(terminal.are_any_events_pending());
    validate_title_event(event_rx.try_recv(), custom_title);

    // Verify that while regular `set_title` will change the self.title, it will NOT emit a new
    // Title event.
    let default_title = "Default title".to_string();
    terminal.set_title(Some(default_title.clone()));
    assert!(!terminal.are_any_events_pending());

    // Validate that setting a custom title again works
    let another_custom_title = "Another custom title".to_string();
    terminal.set_custom_title(Some(another_custom_title.clone()));
    assert!(terminal.are_any_events_pending());
    validate_title_event(event_rx.try_recv(), another_custom_title);

    // Validate that setting the custom title to None will emit the default title as the event
    terminal.set_custom_title(None);
    assert!(terminal.are_any_events_pending());
    validate_title_event(event_rx.try_recv(), default_title);
}

#[test]
fn compare_within_block_points() {
    let a = WithinBlock::new(Point::new(4, 5), 1.into(), GridType::PromptAndCommand);
    let b = WithinBlock::new(Point::new(1, 0), 2.into(), GridType::PromptAndCommand);
    assert!(a < b);

    let c = WithinBlock::new(Point::new(1, 5), 2.into(), GridType::Output);
    let d = WithinBlock::new(Point::new(4, 0), 2.into(), GridType::PromptAndCommand);
    assert!(d < c);

    let e = WithinBlock::new(Point::new(1, 5), 2.into(), GridType::PromptAndCommand);
    let f = WithinBlock::new(Point::new(4, 0), 2.into(), GridType::PromptAndCommand);
    assert!(e < f);

    let g = WithinBlock::new(Point::new(1, 5), 2.into(), GridType::PromptAndCommand);
    let h = WithinBlock::new(Point::new(1, 4), 2.into(), GridType::PromptAndCommand);
    assert!(h < g);

    let i = WithinBlock::new(Point::new(1, 5), 2.into(), GridType::PromptAndCommand);
    let j = WithinBlock::new(Point::new(1, 5), 2.into(), GridType::PromptAndCommand);
    assert!(i == j);
}

#[test]
fn test_alt_screen_toggle() {
    let mut terminal = TerminalModel::mock(None, None);

    terminal.set_mode(ansi::Mode::SwapScreen {
        save_cursor_and_clear_screen: true,
    });
    assert!(terminal.alt_screen_active);

    // Some programs send the control codes to enter/exit the alternate
    // screen multiple times (such as `info`). This should still leave the
    // screen in the expected state (instead of flipping back and forth).
    terminal.set_mode(ansi::Mode::SwapScreen {
        save_cursor_and_clear_screen: true,
    });
    assert!(terminal.alt_screen_active);

    terminal.unset_mode(ansi::Mode::SwapScreen {
        save_cursor_and_clear_screen: true,
    });
    assert!(!terminal.alt_screen_active);

    terminal.unset_mode(ansi::Mode::SwapScreen {
        save_cursor_and_clear_screen: true,
    });
    assert!(!terminal.alt_screen_active);
}

#[test]
fn test_reset_state() {
    let mut terminal: TerminalModel = TerminalModel::mock(None, None);

    terminal.set_custom_title(Some("This is a title".to_owned()));
    terminal.set_title(Some("Other title".to_owned()));

    terminal.reset_state();

    // Make sure that the custom title is not reset.
    assert_eq!(terminal.title, None);
    assert_eq!(terminal.custom_title, Some("This is a title".to_owned()));
}

#[test]
fn test_exit_alt_screen_on_command_finished() {
    let mut terminal: TerminalModel = TerminalModel::mock(None, None);

    terminal.enter_alt_screen(true);

    terminal.command_finished(CommandFinishedValue {
        exit_code: ExitCode::from(0),
        next_block_id: BlockId::new(),
    });

    assert!(!terminal.alt_screen_active);
}

#[test]
fn test_unset_bracketed_paste_mode_on_command_finished() {
    let mut terminal: TerminalModel = TerminalModel::mock(None, None);

    terminal.set_mode(Mode::BracketedPaste);

    terminal.command_finished(CommandFinishedValue {
        exit_code: ExitCode::from(0),
        next_block_id: BlockId::new(),
    });

    assert!(!terminal.is_term_mode_set(TermMode::BRACKETED_PASTE));
}

#[test]
fn test_alt_screen_selection_tracks_scroll() {
    let mut terminal: TerminalModel = TerminalModel::mock(None, None);
    terminal.enter_alt_screen(true);
    assert!(terminal.is_alt_screen_active());

    let semantic_selection = SemanticSelection::mock(false, "");

    // Select an arbitrary range in the middle of the visible window.
    terminal
        .alt_screen
        .start_selection(Point::new(3, 1), SelectionType::Simple, Side::Left);
    terminal
        .alt_screen
        .update_selection(Point::new(7, 5), Side::Right);
    assert_eq!(
        terminal.alt_screen.selection_range(&semantic_selection),
        Some(ExpandedSelectionRange::Regular {
            start: Point { row: 3, col: 1 },
            end: Point { row: 7, col: 5 },
            reversed: false
        })
    );

    // Move the cursor to the last visible row, and then add a line to that, triggering "scroll
    // down".
    terminal.alt_screen.goto_line(VisibleRow(9));
    terminal.alt_screen.linefeed();
    assert_eq!(
        terminal.alt_screen().selection_range(&semantic_selection),
        Some(ExpandedSelectionRange::Regular {
            start: Point { row: 2, col: 1 },
            end: Point { row: 6, col: 5 },
            reversed: false
        })
    );

    // Move the cursor to the top, and go up 3 times, scrolling up.
    terminal.alt_screen.goto_line(VisibleRow(0));
    terminal.alt_screen.reverse_index();
    terminal.alt_screen.reverse_index();
    terminal.alt_screen.reverse_index();
    assert_eq!(
        terminal.alt_screen().selection_range(&semantic_selection),
        Some(ExpandedSelectionRange::Regular {
            start: Point { row: 5, col: 1 },
            end: Point { row: 9, col: 5 },
            reversed: false
        })
    );

    // Scroll up some more, pushing the end of the selection past the end of the viewport.
    terminal.alt_screen.reverse_index();
    terminal.alt_screen.reverse_index();
    terminal.alt_screen.reverse_index();

    let grid = terminal.alt_screen().grid_handler();
    let max_point = Point {
        row: grid.visible_rows() - 1,
        col: grid.columns() - 1,
    };
    assert_eq!(
        terminal.alt_screen().selection_range(&semantic_selection),
        Some(ExpandedSelectionRange::Regular {
            start: Point { row: 8, col: 1 },
            end: max_point,
            reversed: false
        })
    );

    // Test an explicit "scroll up".
    terminal.alt_screen.scroll_up(2);
    assert_eq!(
        terminal.alt_screen().selection_range(&semantic_selection),
        Some(ExpandedSelectionRange::Regular {
            start: Point { row: 6, col: 1 },
            end: Point {
                row: 7,
                col: max_point.col
            },
            reversed: false
        })
    );

    terminal.alt_screen.scroll_down(1);
    assert_eq!(
        terminal.alt_screen().selection_range(&semantic_selection),
        Some(ExpandedSelectionRange::Regular {
            start: Point { row: 7, col: 1 },
            end: Point {
                row: 8,
                col: max_point.col
            },
            reversed: false
        })
    );
}

#[test]
fn test_rect_selection_in_alt_screen() {
    let mut terminal: TerminalModel = TerminalModel::mock(None, None);
    terminal.enter_alt_screen(true);
    assert!(terminal.is_alt_screen_active());

    let semantic_selection = SemanticSelection::mock(false, "");

    // Start a rect selection.
    terminal
        .alt_screen
        .start_selection(Point::new(2, 2), SelectionType::Rect, Side::Left);
    terminal
        .alt_screen
        .update_selection(Point::new(4, 4), Side::Right);
    assert_eq!(
        terminal.alt_screen.selection_range(&semantic_selection),
        Some(ExpandedSelectionRange::Rect {
            rows: vec1![
                (Point { row: 2, col: 2 }, Point { row: 2, col: 4 }),
                (Point { row: 3, col: 2 }, Point { row: 3, col: 4 }),
                (Point { row: 4, col: 2 }, Point { row: 4, col: 4 }),
            ],
        })
    );

    // Scroll down and verify the selection adjusts.
    terminal.alt_screen.scroll_down(1);
    assert_eq!(
        terminal.alt_screen.selection_range(&semantic_selection),
        Some(ExpandedSelectionRange::Rect {
            rows: vec1![
                (Point { row: 3, col: 2 }, Point { row: 3, col: 4 }),
                (Point { row: 4, col: 2 }, Point { row: 4, col: 4 }),
                (Point { row: 5, col: 2 }, Point { row: 5, col: 4 }),
            ],
        })
    );

    // Scroll up and verify the selection adjusts back.
    terminal.alt_screen.scroll_up(1);
    assert_eq!(
        terminal.alt_screen.selection_range(&semantic_selection),
        Some(ExpandedSelectionRange::Rect {
            rows: vec1![
                (Point { row: 2, col: 2 }, Point { row: 2, col: 4 }),
                (Point { row: 3, col: 2 }, Point { row: 3, col: 4 }),
                (Point { row: 4, col: 2 }, Point { row: 4, col: 4 }),
            ],
        })
    );
}

#[test]
fn test_synchronized_output_sharing_session() {
    let mut terminal: TerminalModel = TerminalModel::mock(None, None);

    // Configure the terminal model for a shared session.
    terminal.set_shared_session_status(SharedSessionStatus::ActiveSharer);
    let (tx, rx) = async_channel::unbounded();
    terminal.set_ordered_terminal_events_for_shared_session_tx(tx);

    // Process bytes including synchronized output markers.
    terminal.process_bytes(&b"Before\x1b[?2026hsynchronized\x1b[?2026lafter"[..]);

    // Bytes are flushed every time synchronized output toggles, plus the trailing bytes.
    rx.close();
    let events: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
    assert_eq!(events.len(), 3);

    let OrderedTerminalEventType::PtyBytesRead { bytes } = &events[0] else {
        panic!("Expected PtyBytesRead, got {:?}", events[0]);
    };
    assert_eq!(bytes.as_slice(), b"Before\x1b[?2026h");

    let OrderedTerminalEventType::PtyBytesRead { bytes } = &events[1] else {
        panic!("Expected PtyBytesRead, got {:?}", events[1]);
    };
    assert_eq!(bytes.as_slice(), b"synchronized\x1b[?2026l");

    let OrderedTerminalEventType::PtyBytesRead { bytes } = &events[2] else {
        panic!("Expected PtyBytesRead, got {:?}", events[2]);
    };
    assert_eq!(bytes.as_slice(), b"after");
}

/// Tests the split-batch case where synchronized output markers arrive in separate
/// `parse_bytes` calls on a persistent [`Processor`], preserving sync output state across calls.
#[test]
fn test_synchronized_output_sharing_session_split_batch() {
    let mut terminal: TerminalModel = TerminalModel::mock(None, None);

    // Configure the terminal model for a shared session.
    terminal.set_shared_session_status(SharedSessionStatus::ActiveSharer);
    let (tx, rx) = async_channel::unbounded();
    terminal.set_ordered_terminal_events_for_shared_session_tx(tx);

    // Use a single Processor so that synchronized output state is preserved across calls.
    let mut processor = Processor::new();

    // First batch: contains the sync output start marker but not the end marker.
    processor.parse_bytes(
        &mut terminal,
        &b"Before\x1b[?2026hsync"[..],
        &mut std::io::sink(),
    );

    // Second batch: contains the sync output end marker.
    processor.parse_bytes(
        &mut terminal,
        &b"hronized\x1b[?2026lafter"[..],
        &mut std::io::sink(),
    );

    // Bytes are flushed at each toggle point and at the end of each parse_bytes call.
    rx.close();
    let events: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
    assert_eq!(events.len(), 4);

    // First batch flushes at the sync start toggle, then the remaining bytes.
    let OrderedTerminalEventType::PtyBytesRead { bytes } = &events[0] else {
        panic!("Expected PtyBytesRead, got {:?}", events[0]);
    };
    assert_eq!(bytes.as_slice(), b"Before\x1b[?2026h");

    let OrderedTerminalEventType::PtyBytesRead { bytes } = &events[1] else {
        panic!("Expected PtyBytesRead, got {:?}", events[1]);
    };
    assert_eq!(bytes.as_slice(), b"sync");

    // Second batch flushes at the sync end toggle, then the remaining bytes.
    let OrderedTerminalEventType::PtyBytesRead { bytes } = &events[2] else {
        panic!("Expected PtyBytesRead, got {:?}", events[2]);
    };
    assert_eq!(bytes.as_slice(), b"hronized\x1b[?2026l");

    let OrderedTerminalEventType::PtyBytesRead { bytes } = &events[3] else {
        panic!("Expected PtyBytesRead, got {:?}", events[3]);
    };
    assert_eq!(bytes.as_slice(), b"after");
}
