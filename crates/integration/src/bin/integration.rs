use std::{collections::HashMap, env};

use anyhow::Result;
use clap::Parser;
use integration::test::*;
use integration::Builder;
use warp_cli::WorkerCommand;
use warp_core::channel::{Channel, ChannelConfig, ChannelState, OzConfig, WarpServerConfig};
use warp_core::AppId;

/// The Warp integration test runner.
#[derive(Debug, Default, Parser, Clone)]
#[command(name = "warp-integration-test")]
#[clap(args_conflicts_with_subcommands = true)]
pub struct Args {
    #[command(subcommand)]
    command: Option<WorkerCommand>,

    /// Integration test name.
    #[arg(value_name = "INTEGRATION_TEST_NAME", required = true)]
    // This is an Option<String> because it's not set if a subcommand is requested.
    integration_test_name: Option<String>,
}

pub fn main() -> Result<()> {
    ChannelState::set(ChannelState::new(
        Channel::Integration,
        ChannelConfig {
            app_id: AppId::new(
                "dev",
                "warp",
                if cfg!(target_os = "macos") {
                    "Warp-Integration"
                } else {
                    "WarpIntegration"
                },
            ),
            logfile_name: "warp_integration.log".into(),
            server_config: WarpServerConfig {
                firebase_auth_api_key: "".into(),
                // Use an IP in the IANA testing range, with the TCP discard port, to
                // black-hole server traffic.
                server_root_url: "http://192.0.2.0:9".into(),
                rtc_server_url: "ws://192.0.2.0:9/graphql/v2".into(),
                session_sharing_server_url: None,
            },
            oz_config: OzConfig {
                // Use an IP in the IANA testing range, with the TCP discard port, to
                // black-hole server traffic.
                oz_root_url: "http://192.0.2.0:9".into(),
                workload_audience_url: None,
            },
            telemetry_config: None,
            crash_reporting_config: None,
            autoupdate_config: None,
            mcp_static_config: None,
        },
    ));

    let args = Args::parse();

    if let Some(command) = &args.command {
        match command {
            #[cfg(unix)]
            WorkerCommand::TerminalServer(args) => {
                // If we were asked to run as a terminal server (as opposed to the main
                // GUI application), do so.  This must occur before init_logging, as the
                // terminal server sets up its own logger, and attempting to set a second
                // logger leads to a panic.
                warp::terminal::local_tty::server::run_terminal_server(args);
                return Ok(());
            }
            // This is a catch-all to handle the plugin host, which the integration test crate doesn't have a feature flag for.
            #[allow(unreachable_patterns)]
            other => panic!("Worker not supported in integration tests: {other:?}"),
        }
    }

    let tests = register_tests();
    let test_name = args
        .integration_test_name
        .as_deref()
        .expect("Integration test name is required");

    println!("Running integration test: {test_name}");
    let Some(builder) = tests.get(test_name).map(|func| func()) else {
        panic!("test not found for args: {:#?}", env::args());
    };
    #[cfg_attr(not(unix), allow(unused_variables))]
    let driver = builder.build(test_name, true);

    // Before actually running the test, make sure we won't accidentally stop
    // on any of the real user's configuration or rcfiles.
    cfg_if::cfg_if! {
        if #[cfg(unix)] {
            let home =
                std::env::var("HOME").expect("Should have a value for the HOME environment variable");
            let original_home = std::env::var("ORIGINAL_HOME").expect(
                "Integration test binary should have set an ORIGINAL_HOME environment variable",
            );
            assert_ne!(home, original_home, "HOME should not be the same as ORIGINAL_HOME!");
        } else {
            unimplemented!("Need to add support for hermetic integration tests for the current platform!");
        }
    }

    #[cfg_attr(not(unix), allow(unreachable_code))]
    warp::run_integration_test(driver)
}

/// Type of a function that produces an integration test builder.
type BoxedBuilderFn = Box<dyn Fn() -> Builder>;

fn register_tests() -> HashMap<&'static str, BoxedBuilderFn> {
    let mut tests: HashMap<&str, BoxedBuilderFn> = HashMap::new();

    // A tiny macro to simplify the act of registering a test.  This avoids
    // any inconsistencies between the test function name and the key in the
    // map, and makes it easier to change how we register the tests (if we
    // decide to do so in the future).
    macro_rules! register_test {
        ($name:ident) => {
            tests.insert(stringify!($name), Box::new(|| $name()));
        };
    }

    // Add new tests here
    register_test!(test_single_command);
    register_test!(test_add_and_close_session);
    register_test!(test_add_many_sessions);
    register_test!(test_ctrl_tab_session_switching);
    register_test!(test_ctrl_d_eot);
    register_test!(test_ctrl_d_exit);
    register_test!(test_ctrl_d_handled_by_read_during_bootstrapping);
    register_test!(test_ctrl_d_during_bootstrapping_exits_shell_upon_completion);
    register_test!(test_hover_over_menu);
    register_test!(test_zshrc_keypress);
    register_test!(test_bootstrap_with_no_script_execution_block);
    register_test!(test_instant_prompt_bootstrap);
    register_test!(test_rc_files_only_sourced_once_during_bootstrapping);
    register_test!(test_unescaped_prompt_bootstraps);
    register_test!(test_detect_powerlevel10k);
    register_test!(test_open_and_close_resource_center);
    register_test!(test_block_based_snackbar_scroll_to_top);
    register_test!(test_block_based_snackbar_small_window);
    register_test!(test_block_based_snackbar_appears_for_running_command_input_at_bottom);
    register_test!(test_block_based_snackbar_not_visible_for_pager_command_input_at_bottom);
    register_test!(test_block_based_snackbar_appears_for_running_command_pinned_to_top);
    register_test!(test_block_based_snackbar_not_visible_for_pager_command_pinned_to_top);
    register_test!(test_block_based_snackbar_appears_for_running_command_waterfall_mode);
    register_test!(test_block_based_snackbar_not_visible_pager_command_waterfall_mode);
    register_test!(test_shell_reinitializing);
    register_test!(test_exit_multiple_tabs);
    register_test!(test_open_context_menu_and_execute_command);
    register_test!(test_open_and_close_context_menu_with_keybinding);
    register_test!(test_block_metadata_received);
    register_test!(test_scroll_to_hidden_block_and_open_context_menu_with_keybinding);
    register_test!(test_block_navigation);
    register_test!(test_execute_multiple_cursor_command);
    register_test!(test_undo_redo);
    register_test!(test_add_windows_correct_position_and_cascade);
    register_test!(test_typeahead);
    register_test!(test_input_reporting_posix_shells);
    register_test!(test_input_reporting_powershell);
    register_test!(test_background_output);
    register_test!(test_home_key_should_not_appear_in_input);
    register_test!(test_change_font_size);
    register_test!(test_long_running_block_height_updated);
    register_test!(test_unnecessary_resizes);
    register_test!(test_open_and_close_settings);
    register_test!(test_suggestions_menu_positioning);
    register_test!(test_open_and_close_theme_creator_modal);
    register_test!(test_removing_tabs_out_of_order);
    register_test!(test_ctrl_c);
    register_test!(test_click_on_prompt_to_focus_input);
    register_test!(test_text_input_on_block_list);
    register_test!(test_text_input_on_block_list_while_composing);
    register_test!(test_clear);
    register_test!(test_waterfall_input);
    register_test!(test_waterfall_input_text_selection);
    register_test!(test_waterfall_input_scrolling);
    register_test!(test_waterfall_input_after_command_execution);
    register_test!(test_waterfall_input_alt_grid);
    register_test!(test_find_within_block);
    register_test!(test_case_sensitive_find);
    register_test!(test_find_bar_autoselects_text);
    register_test!(test_disabling_action_dispatching);
    register_test!(test_session_restoration);
    register_test!(test_restored_blocks_on_different_hosts);
    register_test!(test_restore_snapshot_with_deleted_cwd);
    register_test!(test_session_restoration_with_multiple_shells);
    register_test!(test_restore_snapshot_with_background_output);
    register_test!(test_restore_snapshot_with_notebooks);
    register_test!(test_restore_snapshot_with_workflows);
    register_test!(test_restore_snapshot_with_test_json_object);
    register_test!(test_restore_snapshot_with_common_shareable_metadata_ids);
    register_test!(test_restore_snapshot_with_markdown_file);
    register_test!(test_restore_snapshot_with_code_file);
    register_test!(test_restore_snapshot_with_settings_page);
    register_test!(test_multi_block_selections);
    register_test!(test_alias_guards_on_ps1_set);
    register_test!(test_ps1_value_not_null_or_exit);
    register_test!(test_custom_ps1_expansion_bash);
    register_test!(test_completions_with_autocd);
    register_test!(test_auto_title);
    register_test!(test_warp_auto_title_disabled);
    register_test!(test_warp_honors_user_title_bash);
    register_test!(test_warp_honors_user_title_zsh);
    register_test!(test_input_focused_after_executing_command);
    register_test!(test_new_session_focuses_input);
    register_test!(test_executable_completions);
    register_test!(test_function_completions);
    register_test!(test_builtin_completions);
    register_test!(test_keyword_completions);
    register_test!(test_with_launch_config);
    register_test!(test_command_xray_hover);
    register_test!(test_command_xray_for_partial_command);
    register_test!(test_ctrl_r_multi_cursor);
    register_test!(test_histcontrol_env_var);
    register_test!(test_session_navigation_recency_change_tab);
    register_test!(test_session_navigation_recency_navigate_to_tab);
    register_test!(test_session_navigation_recency_click_on_window);
    register_test!(test_session_navigation_recency_navigate_to_window);
    register_test!(test_completions_as_you_type);
    register_test!(test_completions_as_you_type_one_matching_entry_tab);
    register_test!(test_completions_as_you_type_execute_on_enter);
    register_test!(test_accepting_completion_inserts_space);
    register_test!(test_create_session_with_split_pane_while_bootstrapping);
    register_test!(test_create_session_with_new_tab_while_bootstrapping);
    register_test!(test_add_theme_to_warp_config);
    register_test!(test_palette_opens_when_theme_chooser_is_open);
    #[cfg(target_os = "macos")]
    register_test!(test_preview_config_dir_migration);
    register_test!(test_launch_warp_with_theme_in_warp_config);
    register_test!(test_add_launch_config_to_warp_config);
    register_test!(test_add_workflows_to_warp_config);
    register_test!(test_loading_project_workflows);
    register_test!(test_cmd_enter);
    register_test!(test_alias_expansion_has_limit);
    register_test!(test_command_corrections);
    register_test!(test_start_shell_in_deleted_directory);
    register_test!(test_new_window_inherits_previous_session_directory);
    register_test!(test_preferred_shell);
    register_test!(test_git_prompt);
    register_test!(test_terminal_announces_capabilities_to_shell);
    register_test!(test_open_new_tab_with_specific_shell_from_new_session_menu);
    register_test!(test_open_launch_config_from_add_tab_menu_legacy);
    register_test!(test_open_launch_config_with_custom_size);
    register_test!(test_launch_config_single_child_branch);
    register_test!(test_open_launch_config_in_active_window);
    register_test!(test_with_launch_config_with_active_tab_index);
    register_test!(test_with_launch_config_with_active_pane);
    register_test!(test_with_launch_config_with_no_active_pane);
    register_test!(test_find_query_not_evaluated_on_terminal_mode_change);
    register_test!(test_bash_bootstraps_with_prompt_command_array);
    register_test!(test_bash_bootstraps_with_prompt_command_array_that_sets_ps1);
    register_test!(test_zsh_bootstraps_with_nounset_option);
    register_test!(test_legacy_ssh_into_bash);
    register_test!(test_legacy_ssh_into_zsh);
    register_test!(test_tmux_ssh_into_bash);
    register_test!(test_tmux_ssh_into_zsh);
    register_test!(test_ssh_into_fish);
    register_test!(test_ssh_into_sh);
    register_test!(test_ssh_into_ash);
    register_test!(test_ssh_with_shell_override);

    // Remote server integration tests
    register_test!(test_remote_server_connect_bash);
    register_test!(test_remote_server_connect_zsh);
    register_test!(test_remote_server_navigate_to_repo);
    register_test!(test_remote_server_completions);
    register_test!(test_remote_server_file_operations);
    register_test!(test_remote_server_lazy_load_directory);
    register_test!(test_custom_open_completions_menu_binding);
    register_test!(test_color_overrides_in_prompt_dont_crash);
    register_test!(test_copy_prompt_from_block_honor_ps1_disabled);
    register_test!(test_copy_prompt_from_block_honor_ps1_enabled);
    register_test!(test_copy_prompt_from_input_honor_ps1_disabled);
    register_test!(test_copy_prompt_from_input_honor_ps1_enabled);
    register_test!(test_copy_rprompt_from_input_honor_ps1_enabled);
    register_test!(test_rprompt_doesnt_show_when_not_enough_space);
    register_test!(test_block_cursor_navigation_using_escape_codes);
    register_test!(test_block_bulk_deletion_using_escape_codes);
    register_test!(test_escape_sequences_sent_to_focused_terminal);
    register_test!(test_open_input_context_menu);
    register_test!(test_copy_all_from_input_context_menu);
    register_test!(test_cut_paste_from_input_context_menu);
    register_test!(test_paste_and_type_characters_before_bootstrap);
    register_test!(test_code_review_scroll_anchor_preserved_when_inserting_above);
    register_test!(test_code_review_scroll_anchor_unchanged_when_inserting_below);
    register_test!(test_code_review_scroll_preserved_second_file);
    register_test!(test_code_review_scroll_preserved_deleted_range);
    register_test!(test_code_review_scroll_preserved_header_range);
    register_test!(test_code_review_scroll_preserved_footer_range);
    register_test!(test_alt_screen_context_menu_with_sgr_with_mouse_reporting);
    register_test!(test_alt_screen_context_menu_with_sgr_without_mouse_reporting);
    register_test!(test_alt_screen_context_menu_without_sgr_with_mouse_reporting);
    register_test!(test_alt_screen_context_menu_without_sgr_without_mouse_reporting);
    register_test!(test_pane_group_state_single_pane);
    register_test!(test_pane_group_state_multi_pane);
    register_test!(test_pane_group_state_close_pane);
    register_test!(test_pane_group_state_clear_blocks);

    register_test!(test_input_syncing_is_off_by_default);
    register_test!(test_can_sync_input_editor_text_in_tab);
    register_test!(test_can_run_command_in_synced_panes_in_tab);
    register_test!(test_synced_panes_long_running_commands);
    register_test!(test_synced_inputs_terminal_mode_change_view_focus);

    register_test!(test_can_bootstrap_local_bash_subshell);
    register_test!(test_can_bootstrap_local_fish_subshell);
    register_test!(test_can_bootstrap_local_zsh_subshell);
    register_test!(test_can_bootstrap_remote_bash_subshell);
    register_test!(test_can_bootstrap_remote_zsh_subshell);

    register_test!(test_can_auto_bootstrap);

    register_test!(test_ask_warp_ai_keybinding_for_selected_block);
    register_test!(test_create_folder_from_command_palette);

    register_test!(test_tab_behavior_setting);

    register_test!(test_private_public_settings_routing_with_flag_enabled);
    register_test!(test_private_settings_preloaded_and_not_leaked_to_toml);

    register_test!(test_command_search_loads_history);
    register_test!(test_histfile_left_joined_with_persisted_history);
    register_test!(test_history_command_is_linked_to_local_workflow);
    register_test!(test_up_arrow_history_enters_shift_tab_for_workflow);

    register_test!(test_websocket_does_not_begin_on_startup);
    register_test!(test_websocket_begins_on_startup);
    register_test!(test_websocket_begins_after_joining_a_team);
    register_test!(test_websocket_begins_after_creating_an_object);

    register_test!(test_secret_is_obfuscated_on_copy);
    register_test!(test_secret_tooltip_shows_on_click);
    register_test!(test_secret_tooltip_respects_safe_mode_setting);
    register_test!(test_copy_secret_respects_safe_mode_setting);
    register_test!(test_alt_screen_secret_detection);
    register_test!(test_secret_case_sensitivity);
    register_test!(test_secrets_are_always_redacted_in_ai_inputs);

    register_test!(test_context_chips_prompt_at_bootstrap);

    register_test!(test_active_session_follows_focus);
    register_test!(test_tab_context_menu_copies_metadata);
    register_test!(test_vertical_tab_context_menu_copies_metadata);
    register_test!(test_vertical_pane_context_menu_copies_metadata);

    register_test!(test_focus_panes_on_hover);

    register_test!(test_close_tab_with_long_running_process);
    register_test!(test_reorder_tabs_with_drag);
    register_test!(test_detach_tab_to_new_window_with_drag);
    register_test!(test_attach_tab_to_other_window_and_continue_drag);
    register_test!(test_single_tab_handoff_continues_drag);

    register_test!(test_restore_single_closed_pane);
    register_test!(test_restore_multiple_closed_panes);
    register_test!(test_undo_close_grace_period_cleanup);
    register_test!(test_closed_panes_cleared_on_rearrangement);
    register_test!(test_tab_closes_when_last_visible_pane_closed);

    register_test!(test_notebook_pane_tracking);
    register_test!(test_close_notebook_tab);
    register_test!(test_open_in_warp_banner);
    register_test!(test_close_notebook_window);
    register_test!(test_backspace_inside_raw_mermaid_block_edits_text_without_removing_block);

    // Workflow tests
    register_test!(test_open_workflow_in_pane);
    register_test!(test_create_personal_workflow_pane_from_command_palette);
    register_test!(test_create_team_workflow_pane_from_command_palette);

    register_test!(test_block_filtering_keybinding);
    register_test!(test_block_filtering_keybinding_with_long_running_command);
    register_test!(test_block_filtering_toolbelt_icon);
    register_test!(test_block_filtering_context_menu);
    register_test!(test_block_filtering_toggle_filter);
    register_test!(test_block_filtering_toggle_filter_while_find_active);
    register_test!(test_block_filtering_filter_then_find);
    register_test!(test_block_filtering_with_secrets);
    register_test!(test_block_filtering_active_block);
    register_test!(test_block_filtering_clear_blocklist);

    register_test!(test_autosuggestions_are_hidden_when_opening_tab_completions);
    register_test!(test_latest_buffer_operations);

    register_test!(test_pass_control_sequences_to_long_running_block);
    register_test!(test_settings_file_migration_from_native_store);
    register_test!(test_settings_file_hot_reload_applies_new_values);

    register_test!(test_settings_error_banner_on_startup_with_invalid_toml);
    register_test!(test_settings_error_banner_on_startup_with_invalid_value);
    register_test!(test_settings_error_banner_on_reload_with_invalid_toml);
    register_test!(test_settings_error_banner_on_reload_with_invalid_value);

    register_test!(test_middle_click_paste);

    register_test!(test_selection_first_to_last_through_ai_simple);
    register_test!(test_copy_on_select_first_to_last_through_ai_simple);
    register_test!(test_selection_first_to_last_through_ai_semantic);
    register_test!(test_selection_first_to_last_through_ai_lines);
    register_test!(test_selection_last_to_first_through_ai_simple);
    register_test!(test_selection_last_to_first_through_ai_semantic);
    register_test!(test_selection_last_to_first_through_ai_lines);
    register_test!(test_selection_first_to_ai_simple);
    register_test!(test_selection_first_to_ai_semantic);
    register_test!(test_selection_first_to_ai_lines);
    register_test!(test_selection_ai_to_first_simple);
    register_test!(test_selection_ai_to_first_semantic);
    register_test!(test_selection_ai_to_first_lines);
    register_test!(test_selection_ai_to_last_simple);
    register_test!(test_selection_ai_to_last_semantic);
    register_test!(test_selection_ai_to_last_lines);
    register_test!(test_selection_last_to_ai_simple);
    register_test!(test_selection_last_to_ai_semantic);
    register_test!(test_selection_last_to_ai_lines);
    register_test!(test_restored_ai_block_renders_mermaid_and_local_images);

    register_test!(test_agent_mode_pane_minimum_size);
    register_test!(test_git_prompt_chips);

    // These tests are only invoked manually, and not included in the
    // automatic integration test suite.
    register_test!(test_with_24_bit_color);
    register_test!(test_with_long_line);
    register_test!(make_1000_blocks_memory_benchmark);

    register_test!(test_rule_creation);
    register_test!(test_rule_update);
    register_test!(test_rule_pane_opening);
    register_test!(test_undo_close_stack_timeout_cleanup);

    // File tree tests
    register_test!(test_file_tree_opens_files_in_warp);
    register_test!(test_file_tree_open_in_new_pane);
    register_test!(test_file_tree_open_in_new_tab);
    register_test!(test_file_tree_keyboard_navigation);
    register_test!(test_file_tree_non_openable_files);
    register_test!(test_file_tree_nested_file_opening);

    // Go to Line tests
    register_test!(test_goto_line_dialog_open_close);
    register_test!(test_goto_line_jumps_to_line);
    register_test!(test_goto_line_with_column);
    register_test!(test_goto_line_clamps_out_of_range);

    // Keyboard protocol tests
    register_test!(test_keyboard_protocol_disabled_shift_enter);
    register_test!(test_keyboard_protocol_enabled_shift_enter);
    register_test!(test_keyboard_protocol_enabled_shifted_symbol_uses_unshifted_keycode);
    register_test!(test_keyboard_protocol_query_and_apply_modes);
    register_test!(test_keyboard_protocol_report_all_keys_printable_and_cursor);
    register_test!(test_keyboard_protocol_event_types);
    register_test!(test_keyboard_protocol_modifier_key_reporting);
    register_test!(test_keyboard_protocol_modifier_self_bit);
    register_test!(test_keyboard_protocol_alternate_keys_and_text);

    // Video recording test (manual only)
    register_test!(test_video_recording);

    tests
}
