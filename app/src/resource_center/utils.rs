// Hard coded constants to divide keybindings into their respective categories/sections.
// This should always align with documentation: https://docs.warp.dev/getting-started/keyboard-shortcuts

use warpui::keymap::Keystroke;

use crate::localization;
use crate::util::bindings::CommandBinding;

pub const BLOCKS_KEYBINDINGS: &[&str] = &[
    "terminal:select_bookmark_down",
    "terminal:copy_outputs",
    "terminal:select_bookmark_up",
    "terminal:select_all_blocks",
    "terminal:bookmark_selected_block",
    "terminal:select_next_block",
    "terminal:reinput_commands",
    "terminal:focus_input",
    "terminal:select_previous_block",
    "terminal:open_block_list_context_menu_via_keybinding",
    "terminal:copy_commands",
    "terminal:reinput_commands_with_sudo",
    "terminal:open_share_block_modal",
    "terminal:expand_block_selection_below",
    "terminal:expand_block_selection_above",
    "terminal:clear_blocks",
];

pub const INPUT_EDITOR_KEYBINDINGS: &[&str] = &[
    "input:clear_screen",
    "editor:delete_word_left",
    "editor:delete_word_right",
    "editor:insert_last_word_previous_command",
    "editor:select_to_line_end",
    "editor:select_to_line_start",
    "editor_view:add_cursor_above",
    "editor_view:add_cursor_below",
    "editor_view:add_next_occurrence",
    "editor_view:backspace",
    "editor_view:clear_and_copy_lines",
    "editor_view:clear_buffer",
    "editor_view:clear_lines",
    "editor_view:cmd_down",
    "editor_view:inspect_command",
    "editor_view:cut_all_right",
    "editor_view:cut_word_left",
    "editor_view:cut_word_right",
    "editor_view:delete",
    "editor_view:delete_all_left",
    "editor_view:delete_all_right",
    "editor_view:down",
    "editor_view:end",
    "editor_view:fold",
    "editor_view:fold_selected_ranges",
    "editor_view:home",
    "editor_view:insert_newline",
    "editor_view:left",
    "editor_view:move_backward_one_subword",
    "editor_view:move_backward_one_word",
    "editor_view:move_forward_one_subword",
    "editor_view:move_forward_one_word",
    "editor_view:move_to_buffer_end",
    "editor_view:move_to_buffer_start",
    "editor_view:move_to_line_end",
    "editor_view:move_to_line_start",
    "editor_view:move_to_paragraph_end",
    "editor_view:move_to_paragraph_start",
    "editor_view:right",
    "editor_view:select_all",
    "editor_view:select_down",
    "editor_view:select_left",
    "editor_view:select_left_by_subword",
    "editor_view:select_left_by_word",
    "editor_view:select_right",
    "editor_view:select_right_by_subword",
    "editor_view:select_right_by_word",
    "editor_view:select_up",
    "editor_view:unfold",
    "editor_view:up",
];

pub const TERMINAL_KEYBINDINGS: &[&str] = &[
    "find:find_next_occurrence",
    "find:find_prev_occurrence",
    "workspace:set_a11y_concise_verbosity_level",
    "workspace:set_a11y_verbose_verbosity_level",
    "workspace:show_command_search",
    "workspace:show_keybinding_settings",
    "workspace:show_settings_account_page",
    "workspace:show_settings",
    "workspace:toggle_command_palette",
    "workspace:toggle_launch_config_palette",
    "workspace:toggle_mouse_reporting",
    "workspace:toggle_navigation_palette",
    "workspace:toggle_resource_center",
    "pane_group:add_down",
    "pane_group:navigate_down",
    "pane_group:navigate_left",
    "pane_group:navigate_next",
    "pane_group:navigate_prev",
    "pane_group:navigate_right",
    "pane_group:navigate_up",
    "pane_group:resize_down",
    "pane_group:resize_left",
    "pane_group:resize_right",
    "pane_group:resize_up",
    "pane_group:toggle_maximize_pane",
];

pub const FUNDAMENTALS_KEYBINDINGS: &[&str] = &[
    "workspace:new_window",
    "workspace:hide_warp",
    "workspace:hide_others",
    "workspace:quit_warp",
    "workspace:minimize",
];

/// Returns hard-coded keybindings that are shown in the mac menus but not saved/accessible
/// anywhere else in the code.
pub fn get_additional_keybindings(ctx: &warpui::AppContext) -> Vec<CommandBinding> {
    vec![
        CommandBinding::new(
            "workspace:new_window".into(),
            localization::text_for_app(ctx, "resource_center.additional_keybinding.new_window"),
            Some(Keystroke::parse("cmd-n").expect("Valid keystroke")),
        ),
        CommandBinding::new(
            "workspace:hide_warp".into(),
            localization::text_for_app(ctx, "resource_center.additional_keybinding.hide_warp"),
            Some(Keystroke::parse("cmd-h").expect("Valid keystroke")),
        ),
        CommandBinding::new(
            "workspace:hide_others".into(),
            localization::text_for_app(ctx, "resource_center.additional_keybinding.hide_others"),
            Some(Keystroke::parse("alt-cmd-h").expect("Valid keystroke")),
        ),
        CommandBinding::new(
            "workspace:quit_warp".into(),
            localization::text_for_app(ctx, "resource_center.additional_keybinding.quit_warp"),
            Some(Keystroke::parse("cmd-q").expect("Valid keystroke")),
        ),
        CommandBinding::new(
            "workspace:minimize".into(),
            localization::text_for_app(ctx, "resource_center.additional_keybinding.minimize"),
            Some(Keystroke::parse("cmd-m").expect("Valid keystroke")),
        ),
    ]
}
