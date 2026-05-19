use std::path::PathBuf;
use std::sync::Arc;
use warp_terminal::shell::{ShellLaunchData, ShellType};
use warp_util::path::ShellFamily;
use warpui::App;

use super::*;

fn initialize_app(app: &App) {
    app.add_singleton_model(WarpConfig::mock);
}

#[test]
fn test_filter_global_workflows_for_session() {
    App::test((), |app| async move {
        initialize_app(&app);

        let local_workflows = app.add_singleton_model(LocalWorkflows::new);

        // Create a session that has "tr" as the only available command.
        let session = Session::test();
        session.set_external_commands(["tr"]);

        local_workflows.read(&app, move |local_workflows, _| {
            // Verify that all workflows either start with "tr" or punctuation.
            local_workflows
                .global_workflows(Some(Arc::new(session)))
                .for_each(|workflow| {
                    let command = workflow.command().expect("Workflow is Command Workflow");
                    assert!(
                        command.starts_with("tr")
                            || command.starts_with(|c: char| c.is_ascii_punctuation()),
                        "found workflow that should have been filtered: {command}"
                    );
                });
        });
    });
}

#[test]
fn test_workflow_by_command_name() {
    App::test((), |app| async move {
        initialize_app(&app);

        let local_workflows = app.add_singleton_model(LocalWorkflows::new);

        let global_workflow_command = r#"{{command}} 1> {{file}}"#;
        local_workflows.read(&app, move |local_workflows, ctx| {
            // Verify that all workflows either start with punctuation or with "tr".
            let Some((workflow_source, workflow)) =
                local_workflows.workflow_with_command(ctx, global_workflow_command)
            else {
                panic!("Did not find workflow with command {global_workflow_command}");
            };
            assert_eq!(workflow.name(), "Redirect stdout");
            assert_eq!(workflow_source, WorkflowSource::Global);
        });
    });
}

#[test]
fn tail_command_for_shell_converts_windows_log_paths_for_wsl_sessions() {
    let shell_launch_data = ShellLaunchData::WSL {
        distro: "Ubuntu".to_owned(),
    };

    let command = tail_command_for_shell(
        ShellFamily::Posix,
        &PathBuf::from(r"C:\Users\test\AppData\Roaming\Warp\logs\mcp\server.log"),
        Some(&shell_launch_data),
    );

    assert_eq!(
        command,
        r#"tail -f "/mnt/c/Users/test/AppData/Roaming/Warp/logs/mcp/server.log""#
    );
}

#[test]
fn tail_command_for_shell_converts_windows_log_paths_for_msys2_sessions() {
    let shell_launch_data = ShellLaunchData::MSYS2 {
        executable_path: PathBuf::from(r"C:\msys64\usr\bin\bash.exe"),
        shell_type: ShellType::Bash,
    };

    let command = tail_command_for_shell(
        ShellFamily::Posix,
        &PathBuf::from(r"C:\Users\test\AppData\Roaming\Warp\logs\mcp\server.log"),
        Some(&shell_launch_data),
    );

    assert_eq!(
        command,
        r#"tail -f "/c/Users/test/AppData/Roaming/Warp/logs/mcp/server.log""#
    );
}

#[test]
fn tail_command_for_shell_leaves_windows_log_paths_for_powershell_sessions() {
    let command = tail_command_for_shell(
        ShellFamily::PowerShell,
        &PathBuf::from(r"C:\Users\test\AppData\Roaming\Warp\logs\mcp\server.log"),
        None,
    );

    assert_eq!(
        command,
        r#"Get-Content -Wait -Tail 10 -Path "C:\Users\test\AppData\Roaming\Warp\logs\mcp\server.log""#
    );
}
