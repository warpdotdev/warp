use std::time::Duration;

use warp::features::FeatureFlag;
use warp::integration_testing::step::new_step_with_default_assertions;
use warp::integration_testing::terminal::util::ExpectedExitStatus;
use warp::integration_testing::terminal::{
    execute_command_for_single_terminal_in_tab, wait_until_bootstrapped_single_pane_for_tab,
};
use warp::integration_testing::view_getters::{single_terminal_view_for_tab, workspace_view};
use warp_multi_agent_api::request::input::tool_call_result::Result as ToolCallResult;
use warp_multi_agent_api::run_shell_command_result::Result as RunShellCommandResult;
use warpui_core::async_assert;
use warpui_core::integration::TestStep;

use super::new_builder;
use crate::Builder;

fn add_shared_ambient_docker_sandbox_tab() -> TestStep {
    new_step_with_default_assertions("Open shared ambient Docker sandbox tab").with_action(
        |app, window_id, _| {
            workspace_view(app, window_id).update(app, |workspace, ctx| {
                workspace.add_shared_ambient_docker_sandbox_tab_for_integration_test(ctx);
            });
        },
    )
}

fn wait_for_tab_count(expected_tab_count: usize) -> TestStep {
    new_step_with_default_assertions("Wait for Docker sandbox tab")
        .set_timeout(Duration::from_secs(30))
        .add_assertion(move |app, window_id| {
            let tab_count =
                workspace_view(app, window_id).read(app, |workspace, _| workspace.tab_count());
            async_assert!(tab_count == expected_tab_count)
        })
}

fn execute_agent_shell_exit(tab_index: usize, command: &'static str) -> TestStep {
    TestStep::new("Execute agent shell exit").with_action(move |app, window_id, _| {
        single_terminal_view_for_tab(app, window_id, tab_index).update(app, |terminal, ctx| {
            terminal
                .execute_cloud_shell_recovery_command_for_integration_test(command.to_owned(), ctx);
        });
    })
}

fn wait_for_recovery(tab_index: usize, expected_attempt: u8) -> TestStep {
    new_step_with_default_assertions("Wait for shell recovery")
        .set_timeout(Duration::from_secs(30))
        .add_assertion(move |app, window_id| {
            let (attempt, pending, active_sharer, shared_ambient) =
                single_terminal_view_for_tab(app, window_id, tab_index).read(app, |terminal, _| {
                    terminal.cloud_shell_recovery_state_for_integration_test()
                });
            async_assert!(
                attempt == expected_attempt && !pending && active_sharer && shared_ambient,
                "attempt={attempt}, pending={pending}, active_sharer={active_sharer}, shared_ambient={shared_ambient}"
            )
        })
}
fn wait_for_shared_ambient_session(tab_index: usize) -> TestStep {
    new_step_with_default_assertions("Wait for shared ambient session")
        .set_timeout(Duration::from_secs(30))
        .add_assertion(move |app, window_id| {
            let (_, _, active_sharer, shared_ambient) =
                single_terminal_view_for_tab(app, window_id, tab_index).read(app, |terminal, _| {
                    terminal.cloud_shell_recovery_state_for_integration_test()
                });
            async_assert!(
                active_sharer && shared_ambient,
                "active_sharer={active_sharer}, shared_ambient={shared_ambient}"
            )
        })
}

fn wait_for_recovered_command_result(
    tab_index: usize,
    expected_status: &'static str,
    expected_exit_code: i32,
) -> TestStep {
    new_step_with_default_assertions("Wait for external recovered command result")
        .set_timeout(Duration::from_secs(30))
        .add_assertion(move |app, window_id| {
            let result = single_terminal_view_for_tab(app, window_id, tab_index)
                .read(app, |terminal, app| {
                    terminal.cloud_shell_recovery_result_for_integration_test(app)
                });
            let Some((delivery_count, ToolCallResult::RunShellCommand(result))) = result else {
                return async_assert!(false, "recovered command result not available");
            };
            let Some(RunShellCommandResult::CommandFinished(command_finished)) = result.result
            else {
                return async_assert!(false, "expected command_finished result");
            };
            let has_expected_status = command_finished.output.contains(expected_status);
            async_assert!(
                delivery_count == 1
                    && has_expected_status
                    && command_finished.exit_code == expected_exit_code,
                "delivery_count={delivery_count}, output={:?}, exit_code={}",
                command_finished.output,
                command_finished.exit_code,
            )
        })
}
pub fn test_cloud_agent_shell_respawn() -> Builder {
    FeatureFlag::LocalDockerSandbox.set_enabled(true);
    FeatureFlag::CreatingSharedSessions.set_enabled(true);
    FeatureFlag::CloudAgentShellRespawn.set_enabled(true);

    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(add_shared_ambient_docker_sandbox_tab())
        .with_step(wait_for_tab_count(2))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(1))
        .with_step(wait_for_shared_ambient_session(1))
        .with_step(wait_for_shared_ambient_session(1))
        .with_step(execute_command_for_single_terminal_in_tab(
            1,
            "mkdir -p /tmp/warp-shell-recovery && cd /tmp/warp-shell-recovery && export WARP_RECOVERY_VALUE=preserved && touch filesystem-marker".to_owned(),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(execute_agent_shell_exit(
            1,
            "printf x >> replay-marker; exit 7",
        ))
        .with_step(wait_for_recovery(1, 1))
        .with_step(wait_for_recovered_command_result(
            1,
            "Observed status: exit code 7",
            7,
        ))
        .with_step(execute_command_for_single_terminal_in_tab(
            1,
            "test \"$PWD\" = /tmp/warp-shell-recovery && test \"$WARP_RECOVERY_VALUE\" = preserved && test -f filesystem-marker && test \"$(wc -c < replay-marker)\" -eq 1 && echo state-preserved".to_owned(),
            ExpectedExitStatus::Success,
            "state-preserved",
        ))
        .with_step(execute_agent_shell_exit(1, "logout"))
        .with_step(wait_for_recovery(1, 2))
        .with_step(execute_agent_shell_exit(1, "kill $$"))
        .with_step(wait_for_recovery(1, 3))
        .with_step(execute_agent_shell_exit(1, "exec false"))
        .with_step(
            new_step_with_default_assertions("Fourth shell death exhausts recovery budget")
                .set_timeout(Duration::from_secs(30))
                .add_assertion(|app, window_id| {
                    let tab_count = workspace_view(app, window_id)
                        .read(app, |workspace, _| workspace.tab_count());
                    async_assert!(tab_count == 1)
                }),
        )
        .with_step(add_shared_ambient_docker_sandbox_tab())
        .with_step(wait_for_tab_count(2))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(1))
        .with_step(execute_agent_shell_exit(1, "exec false"))
        .with_step(wait_for_recovery(1, 1))
        .with_step(execute_agent_shell_exit(
            1,
            "printf 'exit 23\n' > /tmp/sourced-exit.sh; source /tmp/sourced-exit.sh",
        ))
        .with_step(wait_for_recovery(1, 2))
        .with_step(execute_agent_shell_exit(1, "set -e; false"))
        .with_step(wait_for_recovery(1, 3))
        .with_step(execute_command_for_single_terminal_in_tab(
            1,
            "echo follow-up-ok".to_owned(),
            ExpectedExitStatus::Success,
            "follow-up-ok",
        ))
}
