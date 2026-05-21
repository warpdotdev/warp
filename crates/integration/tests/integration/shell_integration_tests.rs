//! Tests that need to run against every supported shell.
//!
//! Add a test to this module if any of the following are true:
//! * It needs to run against every shell.
//! * It needs to run against a _specific_ shell or set of shells.
//!
//! When adding a test to this module, please add a brief comment indicating
//! why it belongs here.

use super::integration_tests;

integration_tests! {
    // Test command execution works.
    test_single_command,
    // Test shell process terminates when session is closed.
    test_add_and_close_session,
    // Test powerlevel10k detection (via bootstrap script logic).
    test_detect_powerlevel10k,
    // Test properties of bootstrap.
    test_bootstrap_with_no_script_execution_block,
    test_rc_files_only_sourced_once_during_bootstrapping,
    // Test ctrl-c terminates long-running commands.
    test_ctrl_c,
    // Test copying a block's command gives us the expected command string.
    test_open_context_menu_and_execute_command,
    // Test we get the right metadata from a bootstrapped shell.
    test_block_metadata_received,
    // Test typeahead behavior.
    test_typeahead,
    // Test input reporting behavior.
    test_input_reporting_posix_shells,
    test_input_reporting_powershell,
    // Test background output behavior.
    test_background_output,
    // Must run against zsh.
    test_zshrc_keypress,
    // Tests bash- and zsh-specific behavior.
    test_alias_guards_on_ps1_set,
    // Tests prompt information from shell.
    test_ps1_value_not_null_or_exit,
    // Tests bash-specific behavior.
    test_custom_ps1_expansion_bash,
    // Tests zsh-specific behavior.
    test_auto_title,
    // Tests zsh-specific behavior.
    test_warp_auto_title_disabled,
    // Tests bash-specific behavior.
    test_warp_honors_user_title_bash,
    // Tests zsh-specific behavior.
    test_warp_honors_user_title_zsh,
    // Tests shell-specific "autocd" behavior.
    test_completions_with_autocd,
    // Tests bootstrap reports completable executables.
    test_executable_completions,
    // Tests bootstrap reports completable functions.
    test_function_completions,
    // Tests bootstrap reports completable builtins.
    test_builtin_completions,
    // Tests bootstrap reports completable keywords.
    test_keyword_completions,
    // Tests bash-specific behavior.
    test_histcontrol_env_var,
    // Tests initial working directory behavior.
    test_create_session_with_new_tab_while_bootstrapping,
    // Tests initial working directory behavior.
    test_start_shell_in_deleted_directory,
    // Tests prompt information (sent during precmd).
    test_git_prompt,
    // Tests shell initialization.
    test_terminal_announces_capabilities_to_shell,
    // Tests bash-specific behavior.
    test_bash_bootstraps_with_prompt_command_array,
    test_bash_bootstraps_with_prompt_command_array_that_sets_ps1,
    // Test runs only on zsh.
    test_color_overrides_in_prompt_dont_crash,
    // Tests zsh-specific behavior with nounset option.
    test_zsh_bootstraps_with_nounset_option,

    // Tests of ssh wrapper logic from bootstrap script.
    test_legacy_ssh_into_bash,
    test_legacy_ssh_into_zsh,
    test_tmux_ssh_into_bash,
    test_tmux_ssh_into_zsh,
    // TODO(vorporeal): Reenable fish once we actually support it as a remote
    // shell.
    // test_ssh_into_fish,
    test_ssh_into_sh,
    test_ssh_into_ash,

    // Tests of remote server behavior.
    test_remote_server_connect_bash,
    test_remote_server_connect_zsh,
    test_remote_server_navigate_to_repo,
    test_remote_server_completions,
    test_remote_server_file_operations,
    test_remote_server_lazy_load_directory,

    // Tests of custom prompt behavior.
    test_copy_prompt_from_block_honor_ps1_enabled,
    test_copy_prompt_from_input_honor_ps1_enabled,

    // Disabled due to flakiness on CI.
    #[ignore]
    test_copy_rprompt_from_input_honor_ps1_enabled,

    // Tests of subshell logic from bootstrap script.
    #[ignore = "Affected by agent_view feature flag UI changes"]
    test_can_bootstrap_local_bash_subshell,
    #[ignore = "Affected by agent_view feature flag UI changes"]
    test_can_bootstrap_local_zsh_subshell,
    // Disabled due to flakiness on CI.
    #[ignore]
    test_can_bootstrap_local_fish_subshell,

    // Tests loading command history from shell histfile.
    test_command_search_loads_history,
    test_histfile_left_joined_with_persisted_history,

    // Tests default prompt behavior.
    test_context_chips_prompt_at_bootstrap,

    // CTRL-D tests.
    test_ctrl_d_eot,
    test_ctrl_d_exit,
    test_ctrl_d_handled_by_read_during_bootstrapping,
    test_ctrl_d_during_bootstrapping_exits_shell_upon_completion,

    #[ignore = "Affected by agent_view feature flag UI changes"]
    test_git_prompt_chips,
}
