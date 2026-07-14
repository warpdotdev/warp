use super::{
    slash_command_is_submitted_as_prompt, slash_command_is_supported_in_tui, TuiSlashCommand,
};
use crate::features::FeatureFlag;
use crate::search::slash_command_menu::static_commands::{commands, Availability};
const BASELINE_AVAILABILITY: Availability = Availability::AGENT_VIEW
    .union(Availability::AI_ENABLED)
    .union(Availability::NO_LRC_CONTROL);

/// The centralized classifier must mark only the prompt-submitting commands (/compact, /plan,
/// /orchestrate) as "submitted as a prompt". Every other slash command emits an immediate action
/// and must be treated as "run now" by the prompt-queue gate and the shared-session viewer path.
#[test]
fn slash_command_is_submitted_as_prompt_only_for_prompt_commands() {
    // Prompt-submitting commands reiterate their text into the conversation.
    assert!(slash_command_is_submitted_as_prompt(&commands::COMPACT));
    assert!(slash_command_is_submitted_as_prompt(&commands::PLAN));
    assert!(slash_command_is_submitted_as_prompt(&commands::ORCHESTRATE));

    // Action-emitting commands run immediately and are never queued / forwarded as prompts.
    assert!(!slash_command_is_submitted_as_prompt(&commands::FORK));
    assert!(!slash_command_is_submitted_as_prompt(
        &commands::FORK_AND_COMPACT
    ));
    assert!(!slash_command_is_submitted_as_prompt(&commands::FORK_FROM));
    assert!(!slash_command_is_submitted_as_prompt(
        &commands::CONTINUE_LOCALLY
    ));
    assert!(!slash_command_is_submitted_as_prompt(
        &commands::COMPACT_AND
    ));
    assert!(!slash_command_is_submitted_as_prompt(&commands::MODEL));
    assert!(!slash_command_is_submitted_as_prompt(&commands::REWIND));
    assert!(!slash_command_is_submitted_as_prompt(
        &commands::CONVERSATIONS
    ));
    assert!(!slash_command_is_submitted_as_prompt(&commands::QUEUE));
}

#[test]
fn tui_supports_the_selected_low_effort_commands_but_not_cost_or_orchestrate() {
    for (command, expected) in [
        (&*commands::AGENT, TuiSlashCommand::Agent),
        (&*commands::NEW, TuiSlashCommand::New),
        (&*commands::COMPACT, TuiSlashCommand::Compact),
        (&*commands::PLAN, TuiSlashCommand::Plan),
        (&commands::MODEL, TuiSlashCommand::Model),
        (
            &*commands::CREATE_NEW_PROJECT,
            TuiSlashCommand::CreateNewProject,
        ),
        (
            &commands::EXPORT_TO_CLIPBOARD,
            TuiSlashCommand::ExportToClipboard,
        ),
        (&*commands::EXPORT_TO_FILE, TuiSlashCommand::ExportToFile),
    ] {
        assert_eq!(
            TuiSlashCommand::from_static_command(command),
            Some(expected),
            "{} should map to its TUI command",
            command.name
        );
        assert!(
            slash_command_is_supported_in_tui(command),
            "{} should be supported in TUI",
            command.name
        );
    }

    for command in [&commands::COST, &*commands::ORCHESTRATE] {
        assert_eq!(TuiSlashCommand::from_static_command(command), None);
        assert!(!slash_command_is_supported_in_tui(command));
    }
}

#[test]
fn model_command_is_supported_in_tui_without_becoming_a_prompt_command() {
    assert!(slash_command_is_supported_in_tui(&commands::MODEL));
    assert!(!slash_command_is_submitted_as_prompt(&commands::MODEL));
    assert!(commands::MODEL.argument.is_none());
}

#[test]
fn not_cloud_agent_commands_are_only_active_outside_cloud_mode() {
    let local_context = BASELINE_AVAILABILITY | Availability::NOT_CLOUD_AGENT;
    assert!(commands::AGENT.is_active(local_context));
    assert!(commands::NEW.is_active(local_context));

    let cloud_context = BASELINE_AVAILABILITY;
    assert!(!commands::AGENT.is_active(cloud_context));
    assert!(!commands::NEW.is_active(cloud_context));

    let _cloud_mode_input_v2 = FeatureFlag::CloudModeInputV2.override_enabled(true);
    let cloud_mode_v2_context = BASELINE_AVAILABILITY | Availability::CLOUD_MODE_V2_COMPOSER;
    assert!(!commands::AGENT.is_active(cloud_mode_v2_context));
    assert!(!commands::NEW.is_active(cloud_mode_v2_context));
}

#[test]
fn cloud_mode_v2_commands_are_active_only_in_cloud_mode_v2_context() {
    let cloud_context = BASELINE_AVAILABILITY;
    assert!(!commands::HARNESS.is_active(cloud_context));

    let _cloud_mode_input_v2 = FeatureFlag::CloudModeInputV2.override_enabled(true);
    let cloud_mode_v2_context = BASELINE_AVAILABILITY | Availability::CLOUD_MODE_V2_COMPOSER;
    assert!(commands::PLAN.is_active(cloud_mode_v2_context));
    assert!(commands::MODEL.is_active(cloud_mode_v2_context));
    assert!(commands::HARNESS.is_active(cloud_mode_v2_context));
}

#[cfg(all(feature = "local_fs", windows))]
mod windows {
    use std::sync::Arc;

    use super::super::*;
    use crate::terminal::model::session::command_executor::testing::TestCommandExecutor;
    use crate::terminal::model::session::SessionInfo;
    use crate::terminal::shell::ShellType;
    use crate::terminal::ShellLaunchData;

    fn wsl_session() -> Session {
        Session::new(
            SessionInfo::new_for_test().with_shell_type(ShellType::Bash),
            Arc::new(TestCommandExecutor::default()),
        )
        .with_shell_launch_data(ShellLaunchData::WSL {
            distro: "Ubuntu".to_owned(),
        })
    }

    #[test]
    fn open_file_command_converts_wsl_paths_to_host_paths() {
        let session = wsl_session();
        let cases = [
            (
                "/home/ubuntu",
                "subdir/test.txt",
                r"\\WSL$\Ubuntu\home\ubuntu\subdir\test.txt",
                None,
            ),
            (
                "/home/ubuntu/project",
                "../test.txt",
                r"\\WSL$\Ubuntu\home\ubuntu\test.txt",
                None,
            ),
            (
                "/home/ubuntu",
                "subdir/file\\ name.txt",
                r"\\WSL$\Ubuntu\home\ubuntu\subdir\file name.txt",
                None,
            ),
            (
                "/home/ubuntu",
                "subdir/test.txt:4:2",
                r"\\WSL$\Ubuntu\home\ubuntu\subdir\test.txt",
                Some(LineAndColumnArg {
                    line_num: 4,
                    column_num: Some(2),
                }),
            ),
        ];

        for (current_dir, raw_arg, expected_path, expected_line_col) in cases {
            let (path, line_col) = open_file_command_path(&session, current_dir, raw_arg);

            assert_eq!(path, PathBuf::from(expected_path));
            assert_eq!(line_col, expected_line_col);
        }
    }
}
