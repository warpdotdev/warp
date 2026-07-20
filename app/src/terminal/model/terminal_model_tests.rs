use std::fs;
use std::sync::Arc;

use base64::engine::general_purpose::STANDARD as BASE64;
use chrono::{DateTime, Local};
use vec1::vec1;
use warp_core::command::ExitCode;
use warp_core::features::FeatureFlag;
use warp_terminal::model::ansi::ClearMode;
use warpui::r#async::executor::Background;
use warpui::text::{SelectionType, str_to_byte_vec};

use super::*;
use crate::terminal::color;
use crate::terminal::event_listener::ChannelEventListener;
use crate::terminal::model::ObfuscateSecrets;
use crate::terminal::model::ansi::{CompletionMetadata, Handler, Processor};
use crate::terminal::model::block::BlockId;
use crate::terminal::model::bootstrap::BootstrapStage;
use crate::terminal::model::grid::Dimensions as _;
use crate::terminal::model::image_map::StoredImageMetadata;
use crate::terminal::model::index::Side;
use crate::terminal::model::selection::ExpandedSelectionRange;
use crate::terminal::model::test_utils::block_size;
use crate::terminal::shared_session::SharedSessionStatus;

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
    assert!(
        !model
            .block_list()
            .is_executing_oz_environment_startup_commands()
    );

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

#[test]
fn is_cloud_agent_conversation_only_true_for_genuine_ambient_sessions() {
    use std::str::FromStr;

    let make_model = || {
        TerminalModel::new_for_shared_session_viewer(
            block_size(),
            color::List::from(&color::Colors::default()),
            ChannelEventListener::new_for_test(),
            Arc::new(Background::default()),
            false,
            false,
            false,
            ObfuscateSecrets::No,
        )
    };
    let task_id = "123e4567-e89b-12d3-a456-426614174000";

    // Baseline: no shared session source and not viewing a transcript.
    let mut model = make_model();
    assert!(!model.is_cloud_agent_conversation());

    // A manually shared *local* (`User`) session carries an orchestrator task id on its
    // `source_task_id` sidecar (QUALITY-726) but is NOT a cloud agent conversation. This is the
    // regression: before the fix, this task id leaked into the cloud agent icon check.
    model.set_shared_session_source(SharedSessionSource::user(Some(task_id.to_owned())));
    assert!(!model.is_cloud_agent_conversation());

    // A shared *ambient* (cloud) session is a cloud agent conversation.
    model.set_shared_session_source(SharedSessionSource::ambient_agent(Some(task_id.to_owned())));
    assert!(model.is_cloud_agent_conversation());

    // Viewing an ambient conversation transcript is a cloud agent conversation.
    let mut model = make_model();
    model.set_conversation_transcript_viewer_status(Some(
        ConversationTranscriptViewerStatus::ViewingAmbientConversation(
            AmbientAgentTaskId::from_str(task_id).expect("valid task id"),
        ),
    ));
    assert!(model.is_cloud_agent_conversation());
}

fn iterm_file_osc(name: &str, inline: bool, payload: &[u8]) -> String {
    let inline = if inline { "1" } else { "0" };
    format!(
        "\x1b]1337;File=name={};inline={}:{}\x07",
        base64::Engine::encode(&BASE64, name),
        inline,
        base64::Engine::encode(&BASE64, payload)
    )
}

fn multipart_iterm_file_osc(name: &str, inline: bool, payload: &[u8]) -> Vec<String> {
    let inline = if inline { "1" } else { "0" };
    let encoded_payload = base64::Engine::encode(&BASE64, payload);
    let midpoint = encoded_payload.len() / 2;
    vec![
        format!(
            "\x1b]1337;MultipartFile=name={};inline={}\x07",
            base64::Engine::encode(&BASE64, name),
            inline,
        ),
        format!("\x1b]1337;FilePart={}\x07", &encoded_payload[..midpoint]),
        format!("\x1b]1337;FilePart={}\x07", &encoded_payload[midpoint..]),
        "\x1b]1337;FileEnd\x07".to_owned(),
    ]
}

fn hex_encoded_json_dcs(payload: &str) -> Vec<u8> {
    let mut bytes = b"\x1bP$d".to_vec();
    bytes.extend(hex::encode(payload).bytes());
    bytes.push(0x9c);
    bytes
}

fn command_finished_and_precmd(terminal: &mut TerminalModel) {
    let completion_metadata = CompletionMetadata {
        exit_code: ExitCode::from(0),
        next_block_id: BlockId::new(),
    };
    terminal.command_finished(CommandFinishedValue {
        completion_metadata: completion_metadata.clone(),
        ..Default::default()
    });
    terminal.precmd_with_completion_metadata(PrecmdValue {
        completion_metadata,
        prompt_metadata: PromptMetadata::default(),
    });
}

fn normal_command_finished_and_precmd(
    terminal: &mut TerminalModel,
    prompt_metadata: PromptMetadata,
) {
    assert_eq!(
        terminal.start_command_execution(),
        StartCommandOutcome::Accepted
    );
    terminal.preexec(PreexecValue {
        command: "completed".to_owned(),
        session_id: None,
    });
    let completion_metadata = CompletionMetadata {
        exit_code: ExitCode::from(0),
        next_block_id: BlockId::new(),
    };
    terminal.command_finished(CommandFinishedValue {
        completion_metadata: completion_metadata.clone(),
        ..Default::default()
    });
    terminal.precmd_with_completion_metadata(PrecmdValue {
        completion_metadata,
        prompt_metadata,
    });
}

#[test]
fn ignores_non_inline_iterm_file_payload_without_overwriting_cwd_file() {
    let temp_dir = tempfile::tempdir().unwrap();
    let target_path = temp_dir.path().join(".zshenv");
    let original_bytes = b"ORIGINAL=1\n";
    let attacker_bytes = b"touch /tmp/warp-pwned\n";
    fs::write(&target_path, original_bytes).unwrap();

    let mut terminal = TerminalModel::mock(None, None);
    terminal.prompt_only_precmd(PromptMetadata {
        pwd: Some(temp_dir.path().to_string_lossy().to_string()),
        ..Default::default()
    });

    let osc = iterm_file_osc(".zshenv", false, attacker_bytes);
    terminal.process_bytes(osc.as_str());

    assert_eq!(fs::read(&target_path).unwrap(), original_bytes);
    assert!(terminal.image_id_to_metadata.is_empty());
}

#[test]
fn ignores_multipart_non_inline_iterm_file_payload_without_overwriting_cwd_file() {
    let temp_dir = tempfile::tempdir().unwrap();
    let target_path = temp_dir.path().join(".zshenv");
    let original_bytes = b"ORIGINAL=1\n";
    let attacker_bytes = b"touch /tmp/warp-pwned\n";
    fs::write(&target_path, original_bytes).unwrap();

    let mut terminal = TerminalModel::mock(None, None);
    terminal.prompt_only_precmd(PromptMetadata {
        pwd: Some(temp_dir.path().to_string_lossy().to_string()),
        ..Default::default()
    });

    for osc in multipart_iterm_file_osc(".zshenv", false, attacker_bytes) {
        terminal.process_bytes(osc.as_str());
    }

    assert_eq!(fs::read(&target_path).unwrap(), original_bytes);
    assert!(terminal.image_id_to_metadata.is_empty());
}

#[test]
fn handles_inline_iterm_image_payload() {
    let mut terminal = TerminalModel::mock(None, None);
    let svg_bytes =
        br#"<svg width="1" height="1" viewBox="0 0 1 1" xmlns="http://www.w3.org/2000/svg"></svg>"#;

    let osc = iterm_file_osc("pixel.svg", true, svg_bytes);
    terminal.process_bytes(osc.as_str());

    assert_eq!(terminal.image_id_to_metadata.len(), 1);
    let StoredImageMetadata::ITerm(metadata) =
        terminal.image_id_to_metadata.values().next().unwrap()
    else {
        panic!("Expected iTerm image metadata");
    };
    assert_eq!(metadata.name, "pixel.svg");
    assert!(metadata.inline);
    assert_eq!(metadata.image_size.x(), 1.0);
    assert_eq!(metadata.image_size.y(), 1.0);
}

// Ensures that an SSH session successfully bootstraps even if the block list is empty and that
// the parent shell resumes after the nested shell exits.
#[test]
fn ssh_bootstraps_if_blocklist_empty_and_reconciles_parent_return() {
    let _recovery_enabled = FeatureFlag::TerminalLifecycleRecovery.override_enabled(true);
    let mut terminal = TerminalModel::mock(None, None);
    command_finished_and_precmd(&mut terminal);
    command_finished_and_precmd(&mut terminal);

    let bootstrapped_value = BootstrappedValue {
        session_id: None,
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
    terminal.command_finished(CommandFinishedValue {
        completion_metadata: CompletionMetadata {
            exit_code: ExitCode::from(0),
            next_block_id: BlockId::new(),
        },
        session_id: None,
    });
    terminal
        .block_list_mut()
        .precmd_with_completion_metadata(PrecmdValue {
            completion_metadata: CompletionMetadata::default(),
            prompt_metadata: PromptMetadata::default(),
        });

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

    command_finished_and_precmd(&mut terminal);
    command_finished_and_precmd(&mut terminal);
    terminal.bootstrapped(bootstrapped_value);
    command_finished_and_precmd(&mut terminal);

    assert!(terminal.is_active_block_bootstrapped());

    let nested_prompt_block_id = terminal.active_block_id().clone();
    terminal.exit_shell(ExitShellValue {
        session_id: 0.into(),
    });
    let parent_next_block_id = BlockId::new();
    let completion_metadata = CompletionMetadata {
        exit_code: ExitCode::from(255),
        next_block_id: parent_next_block_id.clone(),
    };
    terminal.command_finished(CommandFinishedValue {
        completion_metadata: completion_metadata.clone(),
        session_id: None,
    });
    terminal.precmd_with_completion_metadata(PrecmdValue {
        completion_metadata,
        prompt_metadata: PromptMetadata {
            pwd: Some("/parent-return".to_owned()),
            ..Default::default()
        },
    });

    let completed_nested_prompt = terminal
        .block_list()
        .block_with_id(&nested_prompt_block_id)
        .expect("The nested shell's final prompt block should be completed.");
    assert_eq!(
        completed_nested_prompt.state(),
        BlockState::DoneWithExecution
    );
    assert_eq!(completed_nested_prompt.exit_code(), ExitCode::from(255));
    assert_eq!(terminal.active_block_id(), &parent_next_block_id);
    assert_eq!(
        terminal
            .block_list()
            .active_block()
            .pwd()
            .map(String::as_str),
        Some("/parent-return")
    );
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
    terminal.prompt_only_precmd(PromptMetadata::default());

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
    terminal.start_command_execution();
    terminal.preexec(PreexecValue {
        command: "accepted".to_owned(),
        session_id: None,
    });

    terminal.enter_alt_screen(true);

    terminal.command_finished(CommandFinishedValue {
        completion_metadata: CompletionMetadata {
            exit_code: ExitCode::from(0),
            next_block_id: BlockId::new(),
        },
        session_id: None,
    });

    assert!(!terminal.alt_screen_active);
}

#[test]
fn accepted_precmd_and_preexec_target_the_block_list_while_the_alt_screen_is_active() {
    let mut terminal = TerminalModel::mock(None, None);
    terminal.start_command_execution();
    terminal.enter_alt_screen(true);
    terminal.preexec(PreexecValue {
        command: "accepted".to_owned(),
        session_id: None,
    });
    assert_eq!(
        terminal.block_list().active_block().state(),
        BlockState::Executing
    );
    assert!(terminal.alt_screen_active);

    let next_block_id = BlockId::new();
    terminal.command_finished(CommandFinishedValue {
        completion_metadata: CompletionMetadata {
            exit_code: ExitCode::from(0),
            next_block_id: next_block_id.clone(),
        },
        session_id: None,
    });
    terminal.enter_alt_screen(true);
    terminal.precmd_with_completion_metadata(PrecmdValue {
        completion_metadata: CompletionMetadata {
            exit_code: ExitCode::from(0),
            next_block_id,
        },
        prompt_metadata: PromptMetadata {
            pwd: Some("/accepted".to_owned()),
            ..Default::default()
        },
    });
    assert_eq!(
        terminal
            .block_list()
            .active_block()
            .pwd()
            .map(String::as_str),
        Some("/accepted")
    );
    assert!(terminal.alt_screen_active);
}

#[test]
fn test_unset_bracketed_paste_mode_on_command_finished() {
    let mut terminal: TerminalModel = TerminalModel::mock(None, None);
    terminal.start_command_execution();
    terminal.preexec(PreexecValue {
        command: "accepted".to_owned(),
        session_id: None,
    });

    terminal.set_mode(Mode::BracketedPaste);

    terminal.command_finished(CommandFinishedValue {
        completion_metadata: CompletionMetadata {
            exit_code: ExitCode::from(0),
            next_block_id: BlockId::new(),
        },
        session_id: None,
    });

    assert!(!terminal.is_term_mode_set(TermMode::BRACKETED_PASTE));
}

#[test]
fn normal_lifecycle_pipeline_emits_completion_and_prompt_side_effects_once() {
    let (event_tx, event_rx) = async_channel::unbounded();
    let event_proxy = ChannelEventListener::builder_for_test()
        .with_terminal_events_tx(event_tx)
        .build();
    let mut terminal = TerminalModel::mock(None, Some(event_proxy));
    while event_rx.try_recv().is_ok() {}

    let (ordered_tx, ordered_rx) = async_channel::unbounded();
    terminal.set_ordered_terminal_events_for_shared_session_tx(ordered_tx);

    let completed_block_id = terminal.active_block_id().clone();
    let next_block_id = BlockId::new();
    let completion_metadata = CompletionMetadata {
        exit_code: ExitCode::from(7),
        next_block_id: next_block_id.clone(),
    };
    terminal.start_command_execution();
    terminal.preexec(PreexecValue {
        command: "false".to_owned(),
        session_id: None,
    });
    terminal.command_finished(CommandFinishedValue {
        completion_metadata: completion_metadata.clone(),
        session_id: None,
    });
    terminal.precmd_with_completion_metadata(PrecmdValue {
        completion_metadata,
        prompt_metadata: PromptMetadata {
            pwd: Some("/normal-lifecycle".to_owned()),
            ..Default::default()
        },
    });

    let completed_block = terminal
        .block_list()
        .block_with_id(&completed_block_id)
        .expect("The completed block should remain in the block list.");
    assert_eq!(completed_block.state(), BlockState::DoneWithExecution);
    assert_eq!(completed_block.exit_code(), ExitCode::from(7));
    assert_eq!(terminal.active_block_id(), &next_block_id);
    assert_eq!(
        terminal
            .block_list()
            .active_block()
            .pwd()
            .map(String::as_str),
        Some("/normal-lifecycle")
    );

    let events: Vec<_> = std::iter::from_fn(|| event_rx.try_recv().ok()).collect();
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, Event::BlockCompleted(_)))
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, Event::AfterBlockCompleted(_)))
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, Event::BlockMetadataReceived(_)))
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, Event::Handler(HandlerEvent::Preexec)))
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| {
                matches!(
                    event,
                    Event::Handler(HandlerEvent::CommandFinished {
                        command_type: CommandType::User
                    })
                )
            })
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, Event::Handler(HandlerEvent::Precmd { .. })))
            .count(),
        1
    );

    assert!(matches!(
        ordered_rx.try_recv(),
        Ok(OrderedTerminalEventType::CommandExecutionFinished { .. })
    ));
    assert!(ordered_rx.try_recv().is_err());
}

#[test]
fn precmd_with_completion_metadata_records_completion_mismatch_without_overwriting_completed_block()
{
    let (event_tx, event_rx) = async_channel::unbounded();
    let event_proxy = ChannelEventListener::builder_for_test()
        .with_terminal_events_tx(event_tx)
        .build();
    let mut terminal = TerminalModel::mock(None, Some(event_proxy));

    let completed_block_id = terminal.active_block_id().clone();
    let next_block_id = BlockId::new();
    terminal.start_command_execution();
    terminal.preexec(PreexecValue {
        command: "false".to_owned(),
        session_id: None,
    });
    terminal.command_finished(CommandFinishedValue {
        completion_metadata: CompletionMetadata {
            exit_code: ExitCode::from(7),
            next_block_id: next_block_id.clone(),
        },
        session_id: None,
    });
    while event_rx.try_recv().is_ok() {}

    terminal.precmd_with_completion_metadata(PrecmdValue {
        completion_metadata: CompletionMetadata {
            exit_code: ExitCode::from(9),
            next_block_id: next_block_id.clone(),
        },
        prompt_metadata: PromptMetadata::default(),
    });

    let completed_block = terminal
        .block_list()
        .block_with_id(&completed_block_id)
        .expect("The completed block should remain in the block list.");
    assert_eq!(completed_block.exit_code(), ExitCode::from(7));
    assert_eq!(terminal.active_block_id(), &next_block_id);
    let events: Vec<_> = std::iter::from_fn(|| event_rx.try_recv().ok()).collect();
    assert!(!events.iter().any(|event| {
        matches!(
            event,
            Event::BlockCompleted(_) | Event::Handler(HandlerEvent::CommandFinished { .. })
        )
    }));
    assert!(events.iter().any(|event| {
        matches!(
            event,
            Event::LifecycleRecovery(record) if record.completion_mismatch
        )
    }));
}

#[test]
fn precmd_with_completion_metadata_recovers_missing_completion_with_exact_side_effects() {
    let _recovery_enabled = FeatureFlag::TerminalLifecycleRecovery.override_enabled(true);
    let (event_tx, event_rx) = async_channel::unbounded();
    let event_proxy = ChannelEventListener::builder_for_test()
        .with_terminal_events_tx(event_tx)
        .build();
    let mut terminal = TerminalModel::mock(None, Some(event_proxy));
    while event_rx.try_recv().is_ok() {}

    let (ordered_tx, ordered_rx) = async_channel::unbounded();
    terminal.set_ordered_terminal_events_for_shared_session_tx(ordered_tx);
    let completed_block_id = terminal.active_block_id().clone();
    terminal.start_command_execution();
    for c in "missing-finish".chars() {
        terminal.block_list_mut().active_block_for_test().input(c);
    }
    let next_block_id = BlockId::new();
    let completion_metadata = CompletionMetadata {
        exit_code: ExitCode::from(17),
        next_block_id: next_block_id.clone(),
    };
    terminal.precmd_with_completion_metadata(PrecmdValue {
        completion_metadata: completion_metadata.clone(),
        prompt_metadata: PromptMetadata {
            pwd: Some("/recovered".to_owned()),
            ..Default::default()
        },
    });

    let completed_block = terminal
        .block_list()
        .block_with_id(&completed_block_id)
        .expect("The recovered completed block should remain in the block list.");
    assert_eq!(completed_block.state(), BlockState::DoneWithExecution);
    assert_eq!(completed_block.exit_code(), ExitCode::from(17));
    assert_eq!(completed_block.command_to_string(), "missing-finish");
    assert_eq!(terminal.active_block_id(), &next_block_id);
    assert_eq!(
        terminal
            .block_list()
            .active_block()
            .pwd()
            .map(String::as_str),
        Some("/recovered")
    );

    let events: Vec<_> = std::iter::from_fn(|| event_rx.try_recv().ok()).collect();
    for expected in [
        "BlockCompleted",
        "AfterBlockCompleted",
        "BlockMetadataReceived",
        "CommandFinished",
        "Precmd",
    ] {
        let count = events
            .iter()
            .filter(|event| match expected {
                "BlockCompleted" => matches!(event, Event::BlockCompleted(_)),
                "AfterBlockCompleted" => matches!(event, Event::AfterBlockCompleted(_)),
                "BlockMetadataReceived" => matches!(event, Event::BlockMetadataReceived(_)),
                "CommandFinished" => matches!(
                    event,
                    Event::Handler(HandlerEvent::CommandFinished {
                        command_type: CommandType::User
                    })
                ),
                "Precmd" => matches!(event, Event::Handler(HandlerEvent::Precmd { .. })),
                _ => unreachable!("Every expected event kind is handled."),
            })
            .count();
        assert_eq!(count, 1, "Expected exactly one {expected} event.");
    }
    assert!(matches!(
        ordered_rx.try_recv(),
        Ok(OrderedTerminalEventType::CommandExecutionFinished { .. })
    ));
    assert!(ordered_rx.try_recv().is_err());

    terminal.precmd_with_completion_metadata(PrecmdValue {
        completion_metadata,
        prompt_metadata: PromptMetadata {
            pwd: Some("/recovered".to_owned()),
            ..Default::default()
        },
    });
    let repeated_events: Vec<_> = std::iter::from_fn(|| event_rx.try_recv().ok()).collect();
    assert!(!repeated_events.iter().any(|event| {
        matches!(
            event,
            Event::BlockCompleted(_)
                | Event::AfterBlockCompleted(_)
                | Event::BlockMetadataReceived(_)
                | Event::Handler(HandlerEvent::CommandFinished { .. })
                | Event::Handler(HandlerEvent::Precmd { .. })
        )
    }));
}

#[test]
fn precmd_with_completion_metadata_recovery_cleans_up_alt_screen_and_bracketed_paste() {
    let _recovery_enabled = FeatureFlag::TerminalLifecycleRecovery.override_enabled(true);
    let mut terminal = TerminalModel::mock(None, None);
    terminal.start_command_execution();
    terminal.preexec(PreexecValue {
        command: "vim".to_owned(),
        session_id: None,
    });
    let completed_block_id = terminal.active_block_id().clone();
    terminal.set_mode(Mode::BracketedPaste);
    terminal.enter_alt_screen(true);
    assert!(terminal.alt_screen_active);
    assert!(terminal.is_term_mode_set(TermMode::BRACKETED_PASTE));

    let next_block_id = BlockId::new();
    terminal.precmd_with_completion_metadata(PrecmdValue {
        completion_metadata: CompletionMetadata {
            exit_code: ExitCode::from(0),
            next_block_id: next_block_id.clone(),
        },
        prompt_metadata: PromptMetadata::default(),
    });

    let completed_block = terminal
        .block_list()
        .block_with_id(&completed_block_id)
        .expect("The recovered alt-screen block should remain in the block list.");
    assert_eq!(completed_block.state(), BlockState::DoneWithExecution);
    assert_eq!(terminal.active_block_id(), &next_block_id);
    assert!(!terminal.alt_screen_active);
    assert!(!terminal.is_term_mode_set(TermMode::BRACKETED_PASTE));
}

#[test]
fn precmd_with_completion_metadata_completion_recovery_is_disabled_by_default() {
    let _recovery_disabled = FeatureFlag::TerminalLifecycleRecovery.override_enabled(false);
    let mut terminal = TerminalModel::mock(None, None);
    let active_block_id = terminal.active_block_id().clone();
    let block_count = terminal.block_list().blocks().len();

    terminal.precmd_with_completion_metadata(PrecmdValue {
        completion_metadata: CompletionMetadata {
            exit_code: ExitCode::from(17),
            next_block_id: BlockId::new(),
        },
        prompt_metadata: PromptMetadata {
            pwd: Some("/ignored-recovery".to_owned()),
            ..Default::default()
        },
    });

    assert_eq!(terminal.active_block_id(), &active_block_id);
    assert_eq!(terminal.block_list().blocks().len(), block_count);
    assert_eq!(terminal.block_list().active_block().pwd(), None);
}

#[test]
fn precmd_with_completion_metadata_recovers_in_band_completion_and_reuses_cached_prompt() {
    let _recovery_enabled = FeatureFlag::TerminalLifecycleRecovery.override_enabled(true);
    let mut terminal = TerminalModel::mock(None, None);
    normal_command_finished_and_precmd(
        &mut terminal,
        PromptMetadata {
            pwd: Some("/cached-prompt".to_owned()),
            ..Default::default()
        },
    );
    let completed_block_id = terminal.active_block_id().clone();
    assert_eq!(
        terminal.start_in_band_command_execution(),
        StartCommandOutcome::Accepted
    );
    assert!(
        terminal
            .block_list()
            .is_writing_or_executing_in_band_command()
    );

    let next_block_id = BlockId::new();
    terminal.precmd_with_completion_metadata(PrecmdValue {
        completion_metadata: CompletionMetadata {
            exit_code: ExitCode::from(0),
            next_block_id: next_block_id.clone(),
        },
        prompt_metadata: PromptMetadata {
            is_after_in_band_command: true,
            ..Default::default()
        },
    });

    let completed_block = terminal
        .block_list()
        .block_with_id(&completed_block_id)
        .expect("The recovered in-band block should remain in the block list.");
    assert!(completed_block.is_in_band_command_block());
    assert_eq!(completed_block.state(), BlockState::DoneWithExecution);
    assert_eq!(terminal.active_block_id(), &next_block_id);
    assert!(
        !terminal
            .block_list()
            .is_writing_or_executing_in_band_command()
    );
    assert_eq!(
        terminal
            .block_list()
            .active_block()
            .pwd()
            .map(String::as_str),
        Some("/cached-prompt")
    );
}

#[test]
fn empty_and_syntax_error_commands_without_preexec_complete_as_execution() {
    for (command, exit_code) in [("", ExitCode::from(0)), ("if then", ExitCode::from(2))] {
        let mut terminal = TerminalModel::mock(None, None);
        terminal.start_command_execution();
        for c in command.chars() {
            terminal.block_list_mut().active_block_for_test().input(c);
        }
        let completed_block_id = terminal.active_block_id().clone();
        let next_block_id = BlockId::new();
        let completion_metadata = CompletionMetadata {
            exit_code,
            next_block_id: next_block_id.clone(),
        };

        terminal.command_finished(CommandFinishedValue {
            completion_metadata: completion_metadata.clone(),
            session_id: None,
        });
        terminal.precmd_with_completion_metadata(PrecmdValue {
            completion_metadata,
            prompt_metadata: PromptMetadata::default(),
        });

        let completed_block = terminal
            .block_list()
            .block_with_id(&completed_block_id)
            .expect("The completed block should remain in the block list.");
        assert_eq!(completed_block.state(), BlockState::DoneWithExecution);
        assert_eq!(completed_block.exit_code(), exit_code);
        assert_eq!(completed_block.command_to_string(), command);
        assert_eq!(completed_block.has_failed(), exit_code != ExitCode::from(0));
        assert_eq!(terminal.active_block_id(), &next_block_id);
    }
}

#[test]
fn command_finished_recovers_unknown_started_block_with_real_exit_code() {
    let _recovery_enabled = FeatureFlag::TerminalLifecycleRecovery.override_enabled(true);
    let mut terminal = TerminalModel::mock(None, None);
    terminal.lifecycle_coordinator.reset_unknown();
    terminal.block_list_mut().active_block_for_test().start();
    for c in "unknown-command".chars() {
        terminal.block_list_mut().active_block_for_test().input(c);
    }
    let completed_block_id = terminal.active_block_id().clone();
    let next_block_id = BlockId::new();

    terminal.command_finished(CommandFinishedValue {
        completion_metadata: CompletionMetadata {
            exit_code: ExitCode::from(29),
            next_block_id: next_block_id.clone(),
        },
        session_id: None,
    });

    let completed_block = terminal
        .block_list()
        .block_with_id(&completed_block_id)
        .expect("The recovered unknown-state block should remain in the block list.");
    assert_eq!(completed_block.state(), BlockState::DoneWithExecution);
    assert_eq!(completed_block.exit_code(), ExitCode::from(29));
    assert_eq!(completed_block.command_to_string(), "unknown-command");
    assert_eq!(terminal.active_block_id(), &next_block_id);
}

#[test]
fn recovery_advances_finished_active_block_without_republishing_completion() {
    let _recovery_enabled = FeatureFlag::TerminalLifecycleRecovery.override_enabled(true);
    let (event_tx, event_rx) = async_channel::unbounded();
    let event_proxy = ChannelEventListener::builder_for_test()
        .with_terminal_events_tx(event_tx)
        .build();
    let mut terminal = TerminalModel::mock(None, Some(event_proxy));
    let (ordered_tx, ordered_rx) = async_channel::unbounded();
    terminal.set_ordered_terminal_events_for_shared_session_tx(ordered_tx);
    let completed_block_id = terminal.active_block_id().clone();
    terminal
        .block_list_mut()
        .active_block_for_test()
        .finish(ExitCode::from(31));
    while event_rx.try_recv().is_ok() {}
    terminal.lifecycle_coordinator.reset_unknown();

    let next_block_id = BlockId::new();
    let completion_metadata = CompletionMetadata {
        exit_code: ExitCode::from(99),
        next_block_id: next_block_id.clone(),
    };
    terminal.command_finished(CommandFinishedValue {
        completion_metadata: completion_metadata.clone(),
        session_id: None,
    });
    terminal.precmd_with_completion_metadata(PrecmdValue {
        completion_metadata,
        prompt_metadata: PromptMetadata {
            pwd: Some("/advanced".to_owned()),
            ..Default::default()
        },
    });

    let completed_block = terminal
        .block_list()
        .block_with_id(&completed_block_id)
        .expect("The already-finished block should remain in the block list.");
    assert_eq!(completed_block.exit_code(), ExitCode::from(31));
    assert_eq!(terminal.active_block_id(), &next_block_id);
    assert_eq!(
        terminal
            .block_list()
            .active_block()
            .pwd()
            .map(String::as_str),
        Some("/advanced")
    );
    let events: Vec<_> = std::iter::from_fn(|| event_rx.try_recv().ok()).collect();
    assert!(!events.iter().any(|event| {
        matches!(
            event,
            Event::BlockCompleted(_)
                | Event::AfterBlockCompleted(_)
                | Event::Handler(HandlerEvent::CommandFinished { .. })
        )
    }));
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, Event::BlockMetadataReceived(_)))
            .count(),
        1
    );
    assert!(ordered_rx.try_recv().is_err());
}

#[test]
fn repeated_precmd_with_completion_metadata_and_prompt_only_precmd_are_ignored() {
    let _recovery_enabled = FeatureFlag::TerminalLifecycleRecovery.override_enabled(true);
    let (event_tx, event_rx) = async_channel::unbounded();
    let event_proxy = ChannelEventListener::builder_for_test()
        .with_terminal_events_tx(event_tx)
        .build();
    let mut terminal = TerminalModel::mock(None, Some(event_proxy));
    normal_command_finished_and_precmd(
        &mut terminal,
        PromptMetadata {
            pwd: Some("/initial".to_owned()),
            ps1: Some(hex::encode("$ ")),
            honor_ps1: Some(true),
            ..Default::default()
        },
    );
    while event_rx.try_recv().is_ok() {}
    terminal
        .block_list_mut()
        .active_block_for_test()
        .init_command("typed");
    terminal
        .block_list_mut()
        .active_block_for_test()
        .move_backward(2);
    let active_block_id = terminal.active_block_id().clone();
    let active_block_count = terminal.block_list().blocks().len();
    assert_eq!(
        terminal.block_list().active_block().command_to_string(),
        "typed"
    );
    let cursor_point = terminal
        .block_list()
        .active_block()
        .grid_handler()
        .cursor_point();

    terminal.precmd_with_completion_metadata(PrecmdValue {
        completion_metadata: CompletionMetadata {
            exit_code: ExitCode::from(7),
            next_block_id: active_block_id.clone(),
        },
        prompt_metadata: PromptMetadata {
            pwd: Some("/with-completion-metadata".to_owned()),
            session_id: Some(123),
            ..Default::default()
        },
    });

    assert_eq!(terminal.active_block_id(), &active_block_id);
    assert_eq!(terminal.block_list().blocks().len(), active_block_count);
    assert_eq!(
        terminal.block_list().active_block().command_to_string(),
        "typed"
    );
    assert_eq!(
        terminal
            .block_list()
            .active_block()
            .grid_handler()
            .cursor_point(),
        cursor_point
    );
    assert_eq!(
        terminal
            .block_list()
            .active_block()
            .pwd()
            .map(String::as_str),
        Some("/initial")
    );

    let events: Vec<_> = std::iter::from_fn(|| event_rx.try_recv().ok()).collect();
    assert!(!events.iter().any(|event| {
        matches!(
            event,
            Event::BlockCompleted(_)
                | Event::AfterBlockCompleted(_)
                | Event::BlockMetadataReceived(_)
                | Event::BlockWorkingDirectoryUpdated(_)
                | Event::Handler(HandlerEvent::CommandFinished { .. })
                | Event::Handler(HandlerEvent::Precmd { .. })
        )
    }));

    terminal.prompt_only_precmd(PromptMetadata {
        pwd: Some("/prompt-only".to_owned()),
        session_id: Some(123),
        ..Default::default()
    });
    assert_eq!(terminal.active_block_id(), &active_block_id);
    assert_eq!(terminal.block_list().blocks().len(), active_block_count);
    assert_eq!(
        terminal.block_list().active_block().command_to_string(),
        "typed"
    );
    assert_eq!(
        terminal
            .block_list()
            .active_block()
            .grid_handler()
            .cursor_point(),
        cursor_point
    );
    assert_eq!(
        terminal
            .block_list()
            .active_block()
            .pwd()
            .map(String::as_str),
        Some("/initial")
    );
}

#[test]
fn repeated_precmd_with_completion_metadata_and_prompt_only_precmd_are_ignored_when_recovery_is_disabled()
 {
    let _recovery_disabled = FeatureFlag::TerminalLifecycleRecovery.override_enabled(false);
    let mut terminal = TerminalModel::mock(None, None);
    normal_command_finished_and_precmd(
        &mut terminal,
        PromptMetadata {
            pwd: Some("/initial".to_owned()),
            ..Default::default()
        },
    );
    let active_block_id = terminal.active_block_id().clone();
    terminal.precmd_with_completion_metadata(PrecmdValue {
        completion_metadata: CompletionMetadata {
            exit_code: ExitCode::from(7),
            next_block_id: active_block_id.clone(),
        },
        prompt_metadata: PromptMetadata {
            pwd: Some("/with-completion-metadata".to_owned()),
            ..Default::default()
        },
    });
    assert_eq!(terminal.active_block_id(), &active_block_id);
    assert_eq!(
        terminal
            .block_list()
            .active_block()
            .pwd()
            .map(String::as_str),
        Some("/initial")
    );

    terminal.prompt_only_precmd(PromptMetadata {
        pwd: Some("/new".to_owned()),
        ..Default::default()
    });

    assert_eq!(terminal.active_block_id(), &active_block_id);
    assert_eq!(
        terminal
            .block_list()
            .active_block()
            .pwd()
            .map(String::as_str),
        Some("/initial")
    );
}

#[test]
fn repeated_and_executing_command_starts_are_safely_gated() {
    let mut terminal = TerminalModel::mock(None, None);
    let active_block_id = terminal.active_block_id().clone();

    assert_eq!(
        terminal.start_command_execution(),
        StartCommandOutcome::Accepted
    );
    assert_eq!(
        terminal.start_command_execution(),
        StartCommandOutcome::Coalesced
    );
    assert_eq!(terminal.active_block_id(), &active_block_id);

    terminal.preexec(PreexecValue {
        command: "running".to_owned(),
        session_id: None,
    });
    assert_eq!(
        terminal.start_command_execution(),
        StartCommandOutcome::RejectedExecuting
    );
    assert_eq!(terminal.active_block_id(), &active_block_id);
    assert_eq!(
        terminal.block_list().active_block().state(),
        BlockState::Executing
    );
}

#[test]
fn duplicate_and_colliding_completion_evidence_is_ignored() {
    let mut terminal = TerminalModel::mock(None, None);
    terminal.start_command_execution();
    terminal.preexec(PreexecValue {
        command: "first".to_owned(),
        session_id: None,
    });
    let first_block_id = terminal.active_block_id().clone();
    terminal.command_finished(CommandFinishedValue {
        completion_metadata: CompletionMetadata {
            exit_code: ExitCode::from(9),
            next_block_id: first_block_id.clone(),
        },
        session_id: None,
    });
    assert_eq!(terminal.active_block_id(), &first_block_id);
    assert_eq!(
        terminal.block_list().active_block().state(),
        BlockState::Executing
    );

    let second_block_id = BlockId::new();
    terminal.command_finished(CommandFinishedValue {
        completion_metadata: CompletionMetadata {
            exit_code: ExitCode::from(0),
            next_block_id: second_block_id.clone(),
        },
        session_id: None,
    });
    terminal.precmd_with_completion_metadata(PrecmdValue {
        completion_metadata: CompletionMetadata {
            exit_code: ExitCode::from(0),
            next_block_id: second_block_id.clone(),
        },
        prompt_metadata: PromptMetadata::default(),
    });
    terminal.start_command_execution();
    terminal.preexec(PreexecValue {
        command: "second".to_owned(),
        session_id: None,
    });
    terminal.command_finished(CommandFinishedValue {
        completion_metadata: CompletionMetadata {
            exit_code: ExitCode::from(7),
            next_block_id: first_block_id,
        },
        session_id: None,
    });
    assert_eq!(terminal.active_block_id(), &second_block_id);
    assert_eq!(
        terminal.block_list().active_block().state(),
        BlockState::Executing
    );
}

#[test]
fn terminal_exit_absorbs_later_lifecycle_inputs() {
    let mut terminal = TerminalModel::mock(None, None);
    terminal.exit(ExitReason::PtyDisconnected);
    let active_block_id = terminal.active_block_id().clone();
    let block_count = terminal.block_list().blocks().len();
    let pending_session_id = terminal.pending_session_id();

    assert_eq!(
        terminal.start_command_execution(),
        StartCommandOutcome::IgnoredTerminated
    );
    terminal.preexec(PreexecValue {
        command: "ignored".to_owned(),
        session_id: None,
    });
    terminal.command_finished(CommandFinishedValue {
        completion_metadata: CompletionMetadata {
            exit_code: ExitCode::from(1),
            next_block_id: BlockId::new(),
        },
        session_id: None,
    });
    terminal.precmd_with_completion_metadata(PrecmdValue {
        completion_metadata: CompletionMetadata {
            exit_code: ExitCode::from(1),
            next_block_id: active_block_id.clone(),
        },
        prompt_metadata: PromptMetadata::default(),
    });
    terminal.prompt_only_precmd(PromptMetadata::default());
    terminal.init_shell(InitShellValue {
        shell: "bash".to_owned(),
        user: "ignored".to_owned(),
        hostname: "ignored".to_owned(),
        session_id: 42.into(),
        ..Default::default()
    });

    assert_eq!(terminal.active_block_id(), &active_block_id);
    assert_eq!(terminal.block_list().blocks().len(), block_count);
    assert_eq!(terminal.pending_session_id(), pending_session_id);
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
fn viewer_processes_dcs_hook_with_unregistered_session_id() {
    let mut terminal = TerminalModel::mock(None, None);
    terminal.set_shared_session_status(SharedSessionStatus::reader());
    terminal.start_command_execution();
    terminal.command_finished(CommandFinishedValue {
        completion_metadata: CompletionMetadata {
            exit_code: ExitCode::from(0),
            next_block_id: BlockId::new(),
        },
        session_id: None,
    });

    let bytes = hex_encoded_json_dcs(
        r#"{
                "hook": "Precmd",
                "value": {
                    "pwd": "/viewer",
                    "session_id": 999
                }
            }"#,
    );
    terminal.process_bytes(bytes.as_slice());

    assert_eq!(
        terminal
            .block_list()
            .active_block()
            .pwd()
            .map(String::as_str),
        Some("/viewer")
    );
}

#[test]
fn sharer_rejects_dcs_hook_with_unregistered_session_id() {
    let mut terminal = TerminalModel::mock(None, None);
    terminal.set_shared_session_status(SharedSessionStatus::ActiveSharer);
    terminal.start_command_execution();
    terminal.command_finished(CommandFinishedValue {
        completion_metadata: CompletionMetadata {
            exit_code: ExitCode::from(0),
            next_block_id: BlockId::new(),
        },
        session_id: None,
    });

    let bytes = hex_encoded_json_dcs(
        r#"{
                "hook": "Precmd",
                "value": {
                    "pwd": "/sharer",
                    "session_id": 999
                }
            }"#,
    );
    terminal.process_bytes(bytes.as_slice());

    assert_eq!(
        terminal
            .block_list()
            .active_block()
            .pwd()
            .map(String::as_str),
        None
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

#[test]
fn cloud_mode_setup_phase_ended_emits_when_sharing() {
    let mut terminal: TerminalModel = TerminalModel::mock(None, None);
    terminal.set_shared_session_status(SharedSessionStatus::ActiveSharer);
    let (tx, rx) = async_channel::unbounded();
    terminal.set_ordered_terminal_events_for_shared_session_tx(tx);

    terminal.send_cloud_mode_setup_phase_ended_for_shared_session();

    rx.close();
    let events: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
    assert_eq!(events.len(), 1);
    assert!(matches!(
        events[0],
        OrderedTerminalEventType::CloudModeSetupPhaseEnded
    ));
}

#[test]
fn cloud_mode_setup_phase_ended_does_not_emit_when_not_sharing() {
    let mut terminal: TerminalModel = TerminalModel::mock(None, None);
    // No `set_shared_session_status(ActiveSharer)` here — the helper must
    // bail before reaching the channel.
    let (tx, rx) = async_channel::unbounded();
    terminal.set_ordered_terminal_events_for_shared_session_tx(tx);

    terminal.send_cloud_mode_setup_phase_ended_for_shared_session();

    rx.close();
    let events: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
    assert!(events.is_empty());
}
