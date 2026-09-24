use std::time::Duration;

use warp::features::FeatureFlag;
use warp::integration_testing::cloud_shell_recovery::{
    add_shared_ambient_bash_tab, execute_agent_command, execute_agent_shell_exit,
    sync_session_environment_variable, wait_for_agent_command_result,
    wait_for_recovered_command_result, wait_for_recovery, wait_for_shared_ambient_session,
    wait_for_tab_count,
};
use warp::integration_testing::step::new_step_with_default_assertions;
use warp::integration_testing::terminal::wait_until_bootstrapped_single_pane_for_tab;
use warp::integration_testing::view_getters::workspace_view;
use warpui_core::async_assert;

use super::new_builder;
use crate::Builder;

pub fn test_cloud_agent_shell_respawn() -> Builder {
    FeatureFlag::CreatingSharedSessions.set_enabled(true);
    FeatureFlag::CloudAgentShellRespawn.set_enabled(true);

    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(add_shared_ambient_bash_tab())
        .with_step(wait_for_tab_count(2))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(1))
        .with_step(wait_for_shared_ambient_session(1))
        .with_step(execute_agent_command(
            1,
            "mkdir -p \"/tmp/warp-shell-recovery-$$\" && cd \"/tmp/warp-shell-recovery-$$\" && export RECOVERY_VALUE=preserved && touch filesystem-marker",
            "setup_action",
        ))
        .with_step(wait_for_agent_command_result(1, "setup_action", ""))
        .with_step(sync_session_environment_variable(
            1,
            "setup_action",
            "RECOVERY_VALUE",
            "preserved",
        ))
        .with_step(execute_agent_shell_exit(
            1,
            "sleep 0.2; printf x >> replay-marker; exit 7",
        ))
        .with_step(wait_for_recovery(1))
        .with_step(wait_for_recovered_command_result(
            1,
            "Observed status: exit code 7",
            7,
        ))
        .with_step(execute_agent_command(
            1,
            "printf 'pwd=%s env=%s filesystem=%s replay-bytes=%s\\n' \"$PWD\" \"$RECOVERY_VALUE\" \"$(test -f filesystem-marker && echo present)\" \"$(wc -c < replay-marker)\"; case \"$PWD\" in /tmp/warp-shell-recovery-?*) ;; *) false ;; esac && test \"$RECOVERY_VALUE\" = preserved && test -f filesystem-marker && test \"$(wc -c < replay-marker)\" -eq 1 && echo state-preserved",
            "state_action",
        ))
        .with_step(wait_for_agent_command_result(
            1,
            "state_action",
            "state-preserved",
        ))
        .with_step(execute_agent_shell_exit(1, "sleep 0.2; exit 0"))
        .with_step(wait_for_recovery(1))
        .with_step(execute_agent_shell_exit(1, "sleep 0.2; exit 2"))
        .with_step(wait_for_recovery(1))
        .with_step(execute_agent_shell_exit(1, "sleep 0.2; exec false"))
        .with_step(
            new_step_with_default_assertions("Fourth shell death exhausts recovery budget")
                .set_timeout(Duration::from_secs(30))
                .add_assertion(|app, window_id| {
                    let tab_count = workspace_view(app, window_id)
                        .read(app, |workspace, _| workspace.tab_count());
                    async_assert!(tab_count == 1)
                }),
        )
}
