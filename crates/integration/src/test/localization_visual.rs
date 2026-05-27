use std::time::Duration;

use regex::Regex;
use warp::integration_testing::agent_mode::{enter_agent_view_directly, exit_agent_view};
use warp::integration_testing::command_palette::{close_command_palette, open_command_palette};
use warp::integration_testing::command_search::assert_command_search_is_open;
use warp::integration_testing::context_chips::assert_working_dir_is_present;
use warp::integration_testing::input::input_editor_is_focused;
use warp::integration_testing::localization_visual::{
    assert_localized_app_and_dock_menus, show_localized_launch_config_dialog,
    show_localized_workspace_toast,
};
use warp::integration_testing::step::new_step_with_default_assertions;
use warp::integration_testing::tab::{assert_pane_title, assert_tab_title};
use warp::integration_testing::terminal::wait_until_bootstrapped_single_pane_for_tab;
use warp::integration_testing::view_getters::single_input_view_for_tab;
use warp::settings::{AppLanguage, LanguageSettings};
use warp::settings_view::{SettingsSection, SettingsView};
use warpui::integration::TestStep;
use warpui::{async_assert, SingletonEntity, ViewHandle};

use crate::Builder;

pub fn test_zh_cn_localization_visual_smoke() -> Builder {
    Builder::new()
        .with_real_display()
        .with_timeout(Duration::from_secs(180))
        .with_setup(|utils| {
            let settings_dir = utils.test_dir().join(".warp-integration");
            std::fs::create_dir_all(&settings_dir)
                .expect("Could not create integration settings directory");
            std::fs::write(
                settings_dir.join("settings.toml"),
                "[appearance.interface]\nlanguage = \"simplified_chinese\"\n",
            )
            .expect("Could not write integration settings file");
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(assert_localized_app_and_dock_menus())
        .with_step(open_settings_tab())
        .with_step(open_appearance_page_and_capture())
        .with_step(close_settings_tab())
        .with_step(focus_terminal_input().with_take_screenshot("terminal-input-zh-cn.png"))
        .with_step(assert_context_chips_and_capture())
        .with_step(open_command_search_and_capture())
        .with_step(close_command_search())
        .with_step(enter_agent_view_directly().with_take_screenshot("agent-input-zh-cn.png"))
        .with_step(exit_agent_view())
        .with_step(open_command_palette().with_take_screenshot("command-palette-zh-cn.png"))
        .with_step(close_command_palette())
        .with_step(
            show_localized_workspace_toast("workspace.toast.sync_tab_inputs_enabled")
                .with_take_screenshot("toast-zh-cn.png"),
        )
        .with_step(
            show_localized_launch_config_dialog()
                .with_take_screenshot("dialog-launch-config-zh-cn.png"),
        )
}

fn open_settings_tab() -> TestStep {
    new_step_with_default_assertions("Open settings tab for localization visual smoke")
        .with_keystrokes(&["cmdorctrl-,"])
        .add_assertion(assert_tab_title(
            1,
            Regex::new("^(Settings|设置)$").expect("regex should compile"),
        ))
        .add_assertion(assert_pane_title(
            1,
            0,
            Regex::new("^(Settings|设置)$").expect("regex should compile"),
        ))
        .add_named_assertion("Settings view opened", |app, window_id| {
            let settings_views: Vec<ViewHandle<SettingsView>> = app
                .views_of_type(window_id)
                .expect("Settings view must exist");
            async_assert!(
                settings_views.len() == 1,
                "Expected one SettingsView, got {}",
                settings_views.len()
            )
        })
}

fn open_appearance_page_and_capture() -> TestStep {
    TestStep::new("Open Appearance settings page and capture zh-CN language UI")
        .with_action(|app, window_id, _| {
            let settings_view = app
                .views_of_type::<SettingsView>(window_id)
                .expect("Settings view must exist")
                .first()
                .expect("Settings view must exist")
                .clone();
            settings_view.update(app, |view, ctx| {
                view.set_and_refresh_current_page(SettingsSection::Appearance, ctx);
            });
        })
        .add_assertion(assert_tab_title(
            1,
            Regex::new("^(Settings|设置)$").expect("regex should compile"),
        ))
        .add_assertion(assert_pane_title(
            1,
            0,
            Regex::new("^(Settings|设置)$").expect("regex should compile"),
        ))
        .add_named_assertion("Appearance page is selected in zh-CN", |app, window_id| {
            let language =
                LanguageSettings::handle(app).read(app, |settings, _| *settings.app_language);
            let settings_view = app
                .views_of_type::<SettingsView>(window_id)
                .expect("Settings view must exist")
                .first()
                .expect("Settings view must exist")
                .clone();
            settings_view.read(app, |view, _| {
                let current_section = view.current_settings_section();
                async_assert!(
                    language == AppLanguage::SimplifiedChinese
                        && current_section == SettingsSection::Appearance,
                    "Expected Simplified Chinese Appearance page, got language={language:?}, section={current_section:?}"
                )
            })
        })
        .with_take_screenshot("settings-appearance-language-zh-cn.png")
}

fn close_settings_tab() -> TestStep {
    new_step_with_default_assertions("Close settings tab after zh-CN screenshot")
        .with_hover_over_saved_position("close_tab_button:1")
        .with_click_on_saved_position("close_tab_button:1")
        .add_assertion(assert_tab_title(
            0,
            Regex::new("^(~|bash)$").expect("regex should compile"),
        ))
}

fn focus_terminal_input() -> TestStep {
    TestStep::new("Focus terminal input before Agent view screenshot")
        .with_click_on_saved_position_fn(|app, window_id| {
            let input = single_input_view_for_tab(app, window_id, 0);
            input.read(app, |input, _| input.save_position_id())
        })
        .add_assertion(input_editor_is_focused(0))
}

fn assert_context_chips_and_capture() -> TestStep {
    new_step_with_default_assertions("Capture zh-CN context chips")
        .add_named_assertion(
            "Working directory context chip is present",
            assert_working_dir_is_present(0),
        )
        .with_take_screenshot("context-chips-zh-cn.png")
}

fn open_command_search_and_capture() -> TestStep {
    new_step_with_default_assertions("Open command search for localization visual smoke")
        .with_keystrokes(&["ctrl-r"])
        .add_named_assertion("Command search is open", assert_command_search_is_open())
        .with_take_screenshot("command-search-zh-cn.png")
}

fn close_command_search() -> TestStep {
    new_step_with_default_assertions("Close command search after zh-CN screenshot")
        .with_keystrokes(&["escape"])
        .add_assertion(input_editor_is_focused(0))
}
