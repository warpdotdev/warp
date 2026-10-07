use settings::Setting;
use warpui::integration::{AssertionOutcome, TestStep};
use warpui::windowing::WindowManager;
use warpui::{App, SingletonEntity, WindowId, async_assert};

use crate::integration_testing::step::new_step_with_default_assertions;
use crate::integration_testing::view_getters::{settings_view, theme_chooser_view};
use crate::settings_view::{
    SEARCH_EDITOR_POSITION_ID, SettingsAction, SettingsSection, nav_page_position_id,
    nav_subpage_position_id, nav_umbrella_position_id,
};
use crate::window_settings::WindowSettings;
use crate::workspace::{Workspace, WorkspaceAction};

/// Dispatches a [`WorkspaceAction`] against the active window's workspace view.
fn dispatch_workspace_action(app: &mut App, action: WorkspaceAction) {
    let window_id = app.read(|ctx| {
        WindowManager::as_ref(ctx)
            .active_window()
            .expect("no active window")
    });
    let workspace_view_id = app
        .views_of_type::<Workspace>(window_id)
        .and_then(|views| views.first().map(|view| view.id()))
        .expect("no workspace view");
    app.dispatch_typed_action(window_id, &[workspace_view_id], &action);
}

/// Builds a step that will toggle a setting by [`SettingsAction`]. This can
/// only update settings with a corresponding action on the settings view.
pub fn toggle_setting(action: SettingsAction) -> TestStep {
    new_step_with_default_assertions(&format!("Toggle setting: {action:?}")).with_action(
        move |app, _, _| {
            dispatch_workspace_action(app, WorkspaceAction::DispatchToSettingsTab(action.clone()));
        },
    )
}

/// Opens the settings pane at `section` and waits until it is showing.
pub fn open_settings_page(section: SettingsSection) -> TestStep {
    new_step_with_default_assertions(&format!("Open settings at {section:?}"))
        .with_action(move |app, _, _| {
            dispatch_workspace_action(app, WorkspaceAction::ShowSettingsPage(section));
        })
        .add_named_assertion(
            format!("Settings is showing {section:?}"),
            move |app, window_id| assert_section_selected(app, window_id, section),
        )
}

/// Clicks a top-level sidebar row.
pub fn click_settings_nav_page(section: SettingsSection) -> TestStep {
    new_step_with_default_assertions(&format!("Click settings nav row {section:?}"))
        .with_click_on_saved_position(nav_page_position_id(section))
}

/// Clicks an umbrella header row, toggling it open or closed.
pub fn click_settings_umbrella(label: &'static str) -> TestStep {
    new_step_with_default_assertions(&format!("Click settings umbrella \"{label}\""))
        .with_click_on_saved_position(nav_umbrella_position_id(label))
}

/// Clicks a subpage row nested under an expanded umbrella.
pub fn click_settings_nav_subpage(section: SettingsSection) -> TestStep {
    new_step_with_default_assertions(&format!("Click settings subpage row {section:?}"))
        .with_click_on_saved_position(nav_subpage_position_id(section))
}

/// Types `query` into the settings search input, focusing it first.
pub fn type_settings_search(query: &'static str) -> TestStep {
    new_step_with_default_assertions(&format!("Search settings for {query:?}"))
        .with_click_on_saved_position(SEARCH_EDITOR_POSITION_ID)
        .with_typed_characters(&[query])
        .add_named_assertion(
            format!("Search input contains {query:?}"),
            move |app, window_id| {
                let actual =
                    settings_view(app, window_id).read(app, |view, ctx| view.search_query(ctx));
                async_assert!(
                    actual == query,
                    "Search input should contain {query:?}, was {actual:?}"
                )
            },
        )
}

/// Clears the settings search input by selecting all of it and deleting.
pub fn clear_settings_search() -> TestStep {
    new_step_with_default_assertions("Clear settings search")
        .with_click_on_saved_position(SEARCH_EDITOR_POSITION_ID)
        .with_keystrokes(&["cmdorctrl-a", "backspace"])
        .add_named_assertion("Search input is empty", |app, window_id| {
            let actual =
                settings_view(app, window_id).read(app, |view, ctx| view.search_query(ctx));
            async_assert!(
                actual.is_empty(),
                "Search input should be empty, was {actual:?}"
            )
        })
}

/// Presses Down to move to the next row in the settings sidebar.
pub fn press_settings_nav_down() -> TestStep {
    new_step_with_default_assertions("Press Down in the settings sidebar")
        .with_keystrokes(&["down"])
}

/// Presses Up to move to the previous row in the settings sidebar.
pub fn press_settings_nav_up() -> TestStep {
    new_step_with_default_assertions("Press Up in the settings sidebar").with_keystrokes(&["up"])
}

/// Asserts which settings section is currently selected.
pub fn assert_settings_section(section: SettingsSection) -> TestStep {
    TestStep::new(&format!("Assert settings section is {section:?}")).add_named_assertion(
        format!("Selected section is {section:?}"),
        move |app, window_id| assert_section_selected(app, window_id, section),
    )
}

/// Asserts whether a sidebar row was painted in the most recent frame.
///
/// Nav rows cache their position for a single frame, so a row's presence in
/// the position cache means it is currently rendered. Reading visibility this
/// way means the assertion is checking what was actually drawn, rather than
/// re-deriving the sidebar's filter rules and risking drift from `render`.
fn assert_row_painted(position_id: String, description: String, visible: bool) -> TestStep {
    TestStep::new(&format!("Assert {description} visible is {visible}")).add_named_assertion(
        format!("{description} visible is {visible}"),
        move |app: &mut App, window_id| {
            let painted = app.presenter(window_id).is_some_and(|presenter| {
                presenter
                    .borrow()
                    .position_cache()
                    .get_position(&position_id)
                    .is_some()
            });
            async_assert!(
                painted == visible,
                "{description} visible should be {visible}, was {painted}"
            )
        },
    )
}

/// Asserts whether a top-level sidebar row is currently rendered.
pub fn assert_settings_nav_page_visible(section: SettingsSection, visible: bool) -> TestStep {
    assert_row_painted(
        nav_page_position_id(section),
        format!("nav row {section:?}"),
        visible,
    )
}

/// Asserts whether a subpage row nested under an umbrella is currently rendered.
pub fn assert_settings_nav_subpage_visible(section: SettingsSection, visible: bool) -> TestStep {
    assert_row_painted(
        nav_subpage_position_id(section),
        format!("subpage row {section:?}"),
        visible,
    )
}

/// Asserts whether an umbrella header row is currently rendered.
pub fn assert_settings_umbrella_visible(label: &'static str, visible: bool) -> TestStep {
    assert_row_painted(
        nav_umbrella_position_id(label),
        format!("umbrella header \"{label}\""),
        visible,
    )
}

/// Asserts whether the settings widget with `widget_id` has been rendered in
/// the content pane.
///
/// Unlike nav rows, widgets cache their position indefinitely, so this really
/// asserts "has been painted at least once since the app started". That makes
/// it useful for proving a page's content rendered for the first time, but not
/// for observing that content later disappeared.
pub fn assert_settings_widget_rendered(widget_id: &'static str, rendered: bool) -> TestStep {
    assert_row_painted(
        widget_id.to_string(),
        format!("settings widget {widget_id}"),
        rendered,
    )
}

/// Asserts whether the umbrella labelled `label` is expanded.
pub fn assert_umbrella_expanded(label: &'static str, expanded: bool) -> TestStep {
    TestStep::new(&format!(
        "Assert umbrella \"{label}\" expanded is {expanded}"
    ))
    .add_named_assertion(
        format!("Umbrella \"{label}\" expanded is {expanded}"),
        move |app, window_id| {
            let actual =
                settings_view(app, window_id).read(app, |view, _| view.is_umbrella_expanded(label));
            async_assert!(
                actual == Some(expanded),
                "Umbrella \"{label}\" expanded should be {expanded}, was {actual:?}"
            )
        },
    )
}

fn assert_section_selected(
    app: &mut App,
    window_id: WindowId,
    expected: SettingsSection,
) -> AssertionOutcome {
    let actual = settings_view(app, window_id).read(app, |view, _| view.current_settings_section());
    async_assert!(
        actual == expected,
        "Selected settings section should be {expected:?}, was {actual:?}"
    )
}

pub fn assert_theme_chooser_contains(theme_name: &'static str, count: usize) -> TestStep {
    TestStep::new("Assert the theme chooser contents match our expectations").add_named_assertion(
        format!("The theme chooser contains {count} theme(s) named \"{theme_name}\""),
        move |app, window_id| {
            let theme_chooser = theme_chooser_view(app, window_id);

            let result: usize = theme_chooser.read(app, |theme_chooser, _| {
                theme_chooser
                    .themes()
                    .filter(|theme| theme.matches(theme_name))
                    .count()
            });
            async_assert!(
                result == count,
                "Should have exactly {count} theme(s) named test theme. Instead had {result}"
            )
        },
    )
}

/// Set a custom size for new windows. This updates:
/// * The boolean setting for whether or not to use the custom size
/// * The setting for the window width in rows
/// * The setting for the window height in columns
pub fn set_window_custom_size(rows: u16, columns: u16) -> TestStep {
    TestStep::new("Set custom size for new windows").with_action(move |app, _, _| {
        WindowSettings::handle(app).update(app, |settings, ctx| {
            settings
                .open_windows_at_custom_size
                .set_value(true, ctx)
                .expect("Could not enable custom window sizes");
            settings
                .new_windows_num_rows
                .set_value(rows, ctx)
                .expect("Could not set window width");
            settings
                .new_windows_num_columns
                .set_value(columns, ctx)
                .expect("Could not set window height");
        })
    })
}
