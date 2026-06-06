use crate::features::FeatureFlag;
use crate::search::slash_command_menu::static_commands::{commands, Availability};

use super::{
    normalize_rename_conversation_title, RenameConversationTitleError,
    RENAME_CONVERSATION_TITLE_MAX_CHARS,
};
const BASELINE_AVAILABILITY: Availability = Availability::AGENT_VIEW
    .union(Availability::AI_ENABLED)
    .union(Availability::NO_LRC_CONTROL);

#[test]
fn not_cloud_agent_commands_are_only_active_outside_cloud_mode() {
    let local_context = BASELINE_AVAILABILITY | Availability::NOT_CLOUD_AGENT;
    assert!(commands::AGENT.is_active(local_context));
    assert!(commands::NEW.is_active(local_context));

    let cloud_context = BASELINE_AVAILABILITY;
    assert!(!commands::AGENT.is_active(cloud_context));
    assert!(!commands::NEW.is_active(cloud_context));

    let _cloud_mode_input_v2 = FeatureFlag::CloudModeInputV2.override_enabled(true);
    let cloud_mode_v2_context = BASELINE_AVAILABILITY | Availability::CLOUD_AGENT_V2;
    assert!(!commands::AGENT.is_active(cloud_mode_v2_context));
    assert!(!commands::NEW.is_active(cloud_mode_v2_context));
}

#[test]
fn cloud_mode_v2_commands_are_active_only_in_cloud_mode_v2_context() {
    let cloud_context = BASELINE_AVAILABILITY;
    assert!(!commands::HARNESS.is_active(cloud_context));

    let _cloud_mode_input_v2 = FeatureFlag::CloudModeInputV2.override_enabled(true);
    let cloud_mode_v2_context = BASELINE_AVAILABILITY | Availability::CLOUD_AGENT_V2;
    assert!(commands::PLAN.is_active(cloud_mode_v2_context));
    assert!(commands::MODEL.is_active(cloud_mode_v2_context));
    assert!(commands::HARNESS.is_active(cloud_mode_v2_context));
}

#[test]
fn rename_conversation_title_validation_trims_and_preserves_internal_whitespace() {
    assert_eq!(
        normalize_rename_conversation_title(Some("  New   title  ")).as_deref(),
        Ok("New   title"),
    );
}

#[test]
fn rename_conversation_title_validation_rejects_missing_or_whitespace_only_title() {
    assert_eq!(
        normalize_rename_conversation_title(None),
        Err(RenameConversationTitleError::Missing),
    );
    assert_eq!(
        normalize_rename_conversation_title(Some("   \t\n")),
        Err(RenameConversationTitleError::Missing),
    );
}

#[test]
fn rename_conversation_title_validation_counts_unicode_scalars_for_limit() {
    let accepted = "🦀".repeat(RENAME_CONVERSATION_TITLE_MAX_CHARS);
    let rejected = "🦀".repeat(RENAME_CONVERSATION_TITLE_MAX_CHARS + 1);

    assert_eq!(
        normalize_rename_conversation_title(Some(&accepted)).as_deref(),
        Ok(accepted.as_str()),
    );
    assert_eq!(
        normalize_rename_conversation_title(Some(&rejected)),
        Err(RenameConversationTitleError::TooLong),
    );
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
