use super::slash_command_is_submitted_as_prompt;
use crate::features::FeatureFlag;
use crate::search::slash_command_menu::static_commands::{
    Availability, SlashCommandKind, commands,
};

const BASELINE_AVAILABILITY: Availability = Availability::AGENT_VIEW
    .union(Availability::AI_ENABLED)
    .union(Availability::NO_LRC_CONTROL);

/// The centralized classifier must mark only the prompt-submitting commands (/compact, /plan,
/// /orchestrate) as "submitted as a prompt". Every other slash command emits an immediate action
/// and must be treated as "run now" by the prompt-queue gate and the shared-session viewer path.
#[test]
fn slash_command_is_submitted_as_prompt_only_for_prompt_commands() {
    assert!(slash_command_is_submitted_as_prompt(&commands::COMPACT));
    assert!(slash_command_is_submitted_as_prompt(&commands::PLAN));
    assert!(slash_command_is_submitted_as_prompt(&commands::ORCHESTRATE));

    for command in [
        &*commands::FORK,
        &*commands::FORK_AND_COMPACT,
        &commands::FORK_FROM,
        &*commands::CONTINUE_LOCALLY,
        &*commands::COMPACT_AND,
        &*commands::MODEL,
        &commands::AUTO_APPROVE,
        &commands::REWIND,
        &commands::CONVERSATIONS,
        &*commands::QUEUE,
        &commands::MCP,
    ] {
        assert!(!slash_command_is_submitted_as_prompt(command));
    }
}

#[test]
fn auto_approve_is_an_exact_no_argument_command() {
    use super::{SlashCommandSelectionBehavior, slash_command_selection_behavior};

    assert_eq!(commands::AUTO_APPROVE.kind, SlashCommandKind::AutoApprove);
    assert_eq!(
        slash_command_selection_behavior(&commands::AUTO_APPROVE),
        SlashCommandSelectionBehavior::Execute
    );
    assert!(commands::AUTO_APPROVE.argument.is_none());
}

#[test]
fn tui_commands_have_typed_identities_and_explicit_surface_support() {
    for (command, expected) in [
        (&*commands::AGENT, SlashCommandKind::Agent),
        (&*commands::NEW, SlashCommandKind::New),
        (&*commands::COMPACT, SlashCommandKind::Compact),
        (&commands::COST, SlashCommandKind::Cost),
        (&*commands::PLAN, SlashCommandKind::Plan),
        (&*commands::MODEL, SlashCommandKind::Model),
        (
            &*commands::CREATE_NEW_PROJECT,
            SlashCommandKind::CreateNewProject,
        ),
        (
            &commands::EXPORT_TO_CLIPBOARD,
            SlashCommandKind::ExportToClipboard,
        ),
        (&*commands::EXPORT_TO_FILE, SlashCommandKind::ExportToFile),
        (&commands::AUTO_APPROVE, SlashCommandKind::AutoApprove),
        (&commands::MCP, SlashCommandKind::Mcp),
        (&commands::EXIT, SlashCommandKind::Exit),
        (&commands::LOGOUT, SlashCommandKind::Logout),
        (&commands::VERSION, SlashCommandKind::Version),
        (&commands::VIEW_LOGS, SlashCommandKind::ViewLogs),
    ] {
        assert_eq!(
            command.kind, expected,
            "{} should have its typed command identity",
            command.name
        );
        assert!(command.supports_surface(settings::SettingsMode::Tui));
    }

    let command = &*commands::ORCHESTRATE;
    assert_eq!(command.kind, SlashCommandKind::Orchestrate);
    assert!(!command.supports_surface(settings::SettingsMode::Tui));
}

#[test]
fn model_command_is_supported_in_tui_without_becoming_a_prompt_command() {
    assert_eq!(commands::MODEL.kind, SlashCommandKind::Model);
    assert!(!slash_command_is_submitted_as_prompt(&commands::MODEL));
    assert!(commands::MODEL.argument.is_none());
}

#[test]
fn exit_command_executes_immediately_and_takes_no_argument() {
    use super::{SlashCommandSelectionBehavior, slash_command_selection_behavior};

    assert_eq!(commands::EXIT.kind, SlashCommandKind::Exit);
    assert!(commands::EXIT.argument.is_none());
    assert!(!slash_command_is_submitted_as_prompt(&commands::EXIT));
    assert_eq!(
        slash_command_selection_behavior(&commands::EXIT),
        SlashCommandSelectionBehavior::Execute
    );
    assert_eq!(commands::EXIT.availability, Availability::ALWAYS);
}

#[test]
fn logout_command_executes_immediately_and_takes_no_argument() {
    use super::{SlashCommandSelectionBehavior, slash_command_selection_behavior};

    assert_eq!(commands::LOGOUT.kind, SlashCommandKind::Logout);
    assert!(commands::LOGOUT.argument.is_none());
    assert!(!slash_command_is_submitted_as_prompt(&commands::LOGOUT));
    assert_eq!(
        slash_command_selection_behavior(&commands::LOGOUT),
        SlashCommandSelectionBehavior::Execute
    );
    assert_eq!(commands::LOGOUT.availability, Availability::ALWAYS);
}

#[test]
fn version_command_executes_immediately_and_takes_no_argument() {
    use super::{SlashCommandSelectionBehavior, slash_command_selection_behavior};

    assert_eq!(commands::VERSION.kind, SlashCommandKind::Version);
    assert!(commands::VERSION.argument.is_none());
    assert!(!slash_command_is_submitted_as_prompt(&commands::VERSION));
    assert_eq!(
        slash_command_selection_behavior(&commands::VERSION),
        SlashCommandSelectionBehavior::Execute
    );
    assert_eq!(commands::VERSION.availability, Availability::ALWAYS);
    assert!(commands::VERSION.supports_surface(settings::SettingsMode::Tui));
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

#[test]
fn natural_language_detection_command_is_supported_in_tui() {
    let command = &commands::NATURAL_LANGUAGE_DETECTION;
    assert_eq!(command.kind, SlashCommandKind::NaturalLanguageDetection);
    assert!(command.supports_surface(settings::SettingsMode::Tui));
    // The toggle command runs immediately and is never reiterated as a prompt.
    assert!(command.argument.is_none());
    assert!(!slash_command_is_submitted_as_prompt(command));
}

#[cfg(all(feature = "local_fs", windows))]
mod windows {
    use std::sync::Arc;

    use super::super::*;
    use crate::terminal::ShellLaunchData;
    use crate::terminal::model::session::SessionInfo;
    use crate::terminal::model::session::command_executor::testing::TestCommandExecutor;
    use crate::terminal::shell::ShellType;

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
