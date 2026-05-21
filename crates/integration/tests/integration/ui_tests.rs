//! Tests that cover application UI interactions and not external integrations.
//!
//! If any of the following are true, a test DOES NOT belong here:
//! * It needs to run against every shell.
//! * It needs to run against a _specific_ shell or set of shells.

use super::integration_tests;

integration_tests! {
    test_add_many_sessions,
    test_ctrl_tab_session_switching,
    test_hover_over_menu,
    test_shell_reinitializing,
    test_exit_multiple_tabs,
    test_execute_multiple_cursor_command,
    test_home_key_should_not_appear_in_input,
    test_change_font_size,
    test_long_running_block_height_updated,
    test_instant_prompt_bootstrap,
    test_unescaped_prompt_bootstraps,
    test_unnecessary_resizes,
    test_removing_tabs_out_of_order,
    #[ignore = "Affected by agent_view feature flag UI changes"]
    test_suggestions_menu_positioning,
    test_open_and_close_theme_creator_modal,
    #[ignore = "Affected by agent_view feature flag UI changes"]
    test_click_on_prompt_to_focus_input,
    #[ignore = "Affected by agent_view feature flag UI changes"]
    test_text_input_on_block_list,
    #[ignore = "Affected by agent_view feature flag UI changes"]
    test_text_input_on_block_list_while_composing,
    #[ignore]
    test_open_and_close_resource_center,
    test_open_and_close_context_menu_with_keybinding,
    test_open_and_close_settings,
    #[ignore = "Affected by agent_view feature flag UI changes"]
    test_scroll_to_hidden_block_and_open_context_menu_with_keybinding,
    #[ignore = "Affected by agent_view feature flag UI changes"]
    test_block_navigation,
    #[ignore = "Affected by agent_view feature flag UI changes"]
    test_waterfall_input,
    #[ignore]
    test_waterfall_input_text_selection,
    #[ignore = "Affected by agent_view feature flag UI changes"]
    test_waterfall_input_scrolling,
    #[ignore = "Flakes in CI"]
    test_waterfall_input_after_command_execution,
    test_waterfall_input_alt_grid,
    test_undo_redo,
    #[cfg(target_os="macos")]
    // TODO(alokedesai): Add support for cascading windows when opening new windows via winit.
    test_add_windows_correct_position_and_cascade,
    #[ignore = "Affected by agent_view feature flag UI changes"]
    test_find_within_block,
    test_case_sensitive_find,
    test_find_bar_autoselects_text,
    test_disabling_action_dispatching,
    test_session_restoration,
    test_restored_blocks_on_different_hosts,
    test_restore_snapshot_with_deleted_cwd,
    test_session_restoration_with_multiple_shells,
    test_restore_snapshot_with_background_output,
    test_restore_snapshot_with_notebooks,
    test_restore_snapshot_with_workflows,
    test_restore_snapshot_with_test_json_object,
    test_restore_snapshot_with_common_shareable_metadata_ids,
    test_restore_snapshot_with_markdown_file,
    test_restore_snapshot_with_settings_page,
    // TODO(kevin): figure out why the file name doesn't match.
    #[ignore]
    test_restore_snapshot_with_code_file,
    #[ignore = "Affected by agent_view feature flag UI changes"]
    test_multi_block_selections,
    test_input_focused_after_executing_command,
    // TODO(alokedesai): Determine why this test doesn't reliably pass on CI.
    #[cfg_attr(target_os="linux", ignore)]
    test_with_launch_config,
    test_command_xray_hover,
    test_command_xray_for_partial_command,
    test_ctrl_r_multi_cursor,
    test_session_navigation_recency_change_tab,
    test_session_navigation_recency_navigate_to_tab,
    // Temporarily disable while we investigate why this test is failing on CI.
    #[ignore]
    test_session_navigation_recency_click_on_window,
    // TODO: Figure out why it is flakey.
    #[ignore]
    test_session_navigation_recency_navigate_to_window,
    test_block_based_snackbar_scroll_to_top,
    test_block_based_snackbar_small_window,
    test_block_based_snackbar_appears_for_running_command_input_at_bottom,
    test_block_based_snackbar_not_visible_for_pager_command_input_at_bottom,
    test_block_based_snackbar_appears_for_running_command_pinned_to_top,
    test_block_based_snackbar_not_visible_for_pager_command_pinned_to_top,
    test_block_based_snackbar_appears_for_running_command_waterfall_mode,
    test_block_based_snackbar_not_visible_pager_command_waterfall_mode,
    #[ignore = "Affected by agent_view feature flag UI changes"]
    test_accepting_completion_inserts_space,
    test_palette_opens_when_theme_chooser_is_open,
    test_launch_warp_with_theme_in_warp_config,
    #[cfg(target_os="macos")]
    test_preview_config_dir_migration,
    #[ignore = "Flakes in CI"]
    test_add_launch_config_to_warp_config,
    #[ignore = "Flakes in CI"]
    test_add_workflows_to_warp_config,
    #[ignore = "Flakes in CI"]
    test_add_theme_to_warp_config,
    test_loading_project_workflows,
    test_completions_as_you_type,
    test_completions_as_you_type_one_matching_entry_tab,
    test_completions_as_you_type_execute_on_enter,
    #[ignore = "Affected by agent_view feature flag UI changes"]
    test_cmd_enter,
    test_alias_expansion_has_limit,
    test_command_corrections,
    test_new_window_inherits_previous_session_directory,
    test_preferred_shell,
    test_open_new_tab_with_specific_shell_from_new_session_menu,
    test_open_launch_config_from_add_tab_menu_legacy,
    test_open_launch_config_with_custom_size,
    test_launch_config_single_child_branch,
    test_open_launch_config_in_active_window,
    test_with_launch_config_with_active_tab_index,
    test_with_launch_config_with_active_pane,
    test_with_launch_config_with_no_active_pane,
    test_find_query_not_evaluated_on_terminal_mode_change,
    test_custom_open_completions_menu_binding,
    test_ssh_with_shell_override,

    #[ignore = "Affected by agent_view feature flag UI changes"]
    test_copy_prompt_from_block_honor_ps1_disabled,
    #[ignore = "Affected by agent_view feature flag UI changes"]
    test_copy_prompt_from_input_honor_ps1_disabled,
    #[ignore = "Affected by agent_view feature flag UI changes"]
    test_rprompt_doesnt_show_when_not_enough_space,
    test_block_cursor_navigation_using_escape_codes,
    test_block_bulk_deletion_using_escape_codes,
    test_escape_sequences_sent_to_focused_terminal,
    test_open_input_context_menu,
    test_copy_all_from_input_context_menu,
    test_cut_paste_from_input_context_menu,
    test_paste_and_type_characters_before_bootstrap,
    #[ignore = "Flaking on CI - KC looking into 3/31/26"]
    test_code_review_scroll_anchor_preserved_when_inserting_above,
    #[ignore = "Flaking on CI - KC looking into 3/31/26"]
    test_code_review_scroll_anchor_unchanged_when_inserting_below,
    #[ignore = "Flaking on CI - KC looking into 3/31/26"]
    test_code_review_scroll_preserved_second_file,
    #[ignore = "Flaking on CI - KC looking into 3/31/26"]
    test_code_review_scroll_preserved_deleted_range,
    #[ignore = "Flaking on CI - KC looking into 3/31/26"]
    test_code_review_scroll_preserved_header_range,
    #[ignore = "Flaking on CI - KC looking into 3/31/26"]
    test_code_review_scroll_preserved_footer_range,
    test_pane_group_state_single_pane,
    test_pane_group_state_multi_pane,
    test_pane_group_state_close_pane,
    test_pane_group_state_clear_blocks,

    test_alt_screen_context_menu_with_sgr_with_mouse_reporting,
    test_alt_screen_context_menu_with_sgr_without_mouse_reporting,
    test_alt_screen_context_menu_without_sgr_with_mouse_reporting,
    test_alt_screen_context_menu_without_sgr_without_mouse_reporting,

    test_input_syncing_is_off_by_default,
    test_can_sync_input_editor_text_in_tab,
    test_can_run_command_in_synced_panes_in_tab,
    test_synced_panes_long_running_commands,
    test_synced_inputs_terminal_mode_change_view_focus,

    #[ignore = "Affected by agent_view feature flag UI changes"]
    test_can_bootstrap_remote_bash_subshell,
    #[ignore = "Affected by agent_view feature flag UI changes"]
    test_can_bootstrap_remote_zsh_subshell,

    #[ignore = "Affected by agent_view feature flag UI changes"]
    test_can_auto_bootstrap,

    // Disabled due to flakiness on CI.
    #[ignore]
    test_create_session_with_split_pane_while_bootstrapping,

    // For some reason, disabling the `AgentMode` flag does not actually disable Agent Mode in the test
    // run. Ignore for now.
    #[ignore]
    test_ask_warp_ai_keybinding_for_selected_block,

    test_create_folder_from_command_palette,

    test_tab_behavior_setting,

    test_private_public_settings_routing_with_flag_enabled,
    test_private_settings_preloaded_and_not_leaked_to_toml,
    test_history_command_is_linked_to_local_workflow,
    #[ignore = "Affected by agent_view feature flag UI changes"]
    test_up_arrow_history_enters_shift_tab_for_workflow,

    test_websocket_begins_on_startup,
    test_websocket_does_not_begin_on_startup,
    test_websocket_begins_after_joining_a_team,
    test_websocket_begins_after_creating_an_object,

    test_secret_is_obfuscated_on_copy,
    test_secret_tooltip_respects_safe_mode_setting,
    test_copy_secret_respects_safe_mode_setting,
    test_alt_screen_secret_detection,
    test_secret_case_sensitivity,
    test_secrets_are_always_redacted_in_ai_inputs,

    test_active_session_follows_focus,
    test_tab_context_menu_copies_metadata,
    test_vertical_tab_context_menu_copies_metadata,
    test_vertical_pane_context_menu_copies_metadata,

    test_focus_panes_on_hover,

    test_close_tab_with_long_running_process,
    test_reorder_tabs_with_drag,
    test_detach_tab_to_new_window_with_drag,
    test_attach_tab_to_other_window_and_continue_drag,
    test_single_tab_handoff_continues_drag,

    test_restore_single_closed_pane,
    test_restore_multiple_closed_panes,
    test_undo_close_grace_period_cleanup,
    test_closed_panes_cleared_on_rearrangement,
    test_tab_closes_when_last_visible_pane_closed,

    test_notebook_pane_tracking,
    test_close_notebook_tab,
    test_open_in_warp_banner,
    test_close_notebook_window,
    test_backspace_inside_raw_mermaid_block_edits_text_without_removing_block,

    test_open_workflow_in_pane,
    test_create_personal_workflow_pane_from_command_palette,
    test_create_team_workflow_pane_from_command_palette,

    // TODO(alokedesai): Fix this on the latest version of Bash.
    #[ignore]
    test_up_arrow_history,

    test_block_filtering_keybinding,
    test_block_filtering_toolbelt_icon,
    test_block_filtering_context_menu,
    test_block_filtering_toggle_filter,
    test_block_filtering_toggle_filter_while_find_active,
    test_block_filtering_filter_then_find,
    test_block_filtering_with_secrets,
    test_block_filtering_active_block,
    test_block_filtering_clear_blocklist,
    test_autosuggestions_are_hidden_when_opening_tab_completions,
    test_latest_buffer_operations,

    test_pass_control_sequences_to_long_running_block,
    test_settings_file_migration_from_native_store,
    test_settings_file_hot_reload_applies_new_values,

    test_settings_error_banner_on_startup_with_invalid_toml,
    test_settings_error_banner_on_startup_with_invalid_value,
    test_settings_error_banner_on_reload_with_invalid_toml,
    test_settings_error_banner_on_reload_with_invalid_value,

    #[ignore = "Affected by agent_view feature flag UI changes"]
    test_selection_first_to_last_through_ai_simple,
    #[ignore = "Affected by agent_view feature flag UI changes"]
    test_copy_on_select_first_to_last_through_ai_simple,
    #[ignore = "Affected by agent_view feature flag UI changes"]
    test_selection_first_to_last_through_ai_semantic,
    #[ignore = "Affected by agent_view feature flag UI changes"]
    test_selection_first_to_last_through_ai_lines,
    #[ignore = "Affected by agent_view feature flag UI changes"]
    test_selection_last_to_first_through_ai_simple,
    #[ignore = "Affected by agent_view feature flag UI changes"]
    test_selection_last_to_first_through_ai_semantic,
    #[ignore = "Affected by agent_view feature flag UI changes"]
    test_selection_last_to_first_through_ai_lines,
    #[ignore = "Affected by agent_view feature flag UI changes"]
    test_selection_first_to_ai_simple,
    #[ignore = "Affected by agent_view feature flag UI changes"]
    test_selection_first_to_ai_semantic,
    #[ignore = "Affected by agent_view feature flag UI changes"]
    test_selection_first_to_ai_lines,
    #[ignore = "Affected by agent_view feature flag UI changes"]
    test_selection_ai_to_first_simple,
    #[ignore = "Affected by agent_view feature flag UI changes"]
    test_selection_ai_to_first_semantic,
    #[ignore = "Affected by agent_view feature flag UI changes"]
    test_selection_ai_to_first_lines,
    #[ignore = "Affected by agent_view feature flag UI changes"]
    test_selection_ai_to_last_simple,
    #[ignore = "Affected by agent_view feature flag UI changes"]
    test_selection_ai_to_last_semantic,
    #[ignore = "Affected by agent_view feature flag UI changes"]
    test_selection_ai_to_last_lines,
    #[ignore = "Affected by agent_view feature flag UI changes"]
    test_selection_last_to_ai_simple,
    #[ignore = "Affected by agent_view feature flag UI changes"]
    test_selection_last_to_ai_semantic,
    #[ignore = "Affected by agent_view feature flag UI changes"]
    test_selection_last_to_ai_lines,
    test_restored_ai_block_renders_mermaid_and_local_images,

    // Middle-click-paste is only implemented for Linux right now.
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    test_middle_click_paste,
    test_agent_mode_pane_minimum_size,

    test_rule_creation,
    test_rule_update,
    test_rule_pane_opening,
    test_undo_close_stack_timeout_cleanup,

    test_file_tree_opens_files_in_warp,
    test_file_tree_open_in_new_pane,
    test_file_tree_open_in_new_tab,
    test_file_tree_keyboard_navigation,
    test_file_tree_non_openable_files,
    test_file_tree_nested_file_opening,

    // Go to Line tests
    test_goto_line_dialog_open_close,
    test_goto_line_jumps_to_line,
    test_goto_line_with_column,
    test_goto_line_clamps_out_of_range,

    // Keyboard protocol tests
    test_keyboard_protocol_disabled_shift_enter,
    test_keyboard_protocol_enabled_shift_enter,
    test_keyboard_protocol_enabled_shifted_symbol_uses_unshifted_keycode,
    test_keyboard_protocol_query_and_apply_modes,
    test_keyboard_protocol_report_all_keys_printable_and_cursor,
    test_keyboard_protocol_event_types,
    test_keyboard_protocol_modifier_key_reporting,
    test_keyboard_protocol_modifier_self_bit,
    test_keyboard_protocol_alternate_keys_and_text,

    // Video recording test — requires real display, run manually
    #[ignore = "Manual test: requires real display for frame capture"]
    test_video_recording,
}
