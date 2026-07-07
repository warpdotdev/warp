use std::collections::HashMap;

use settings::Setting as _;
use warp::integration_testing::command_search::{
    assert_command_search_has_results, assert_command_search_is_open,
    assert_history_filter_is_active,
};
use warp::integration_testing::input::assert_workflow_info_box_is_open;
use warp::integration_testing::step::new_step_with_default_assertions;
use warp::integration_testing::terminal::{
    assert_input_editor_contents, wait_until_bootstrapped_single_pane_for_tab,
};
use warp::integration_testing::view_getters::single_input_view;
use warp::integration_testing::{self};
use warp::search::command_search::settings::ShowGlobalWorkflowsInUniversalSearch;
use warp::sqlite_testing::set_user_and_hostname_for_commands;
use warp::terminal::input::Input;
use warp::terminal::model::session::get_local_hostname;
use warp::terminal::shell::ShellType;
use warpui_core::{async_assert, ViewHandle};

use super::{new_builder, TEST_ONLY_ASSETS};
use crate::util::{get_local_user, write_histfiles_for_test};
use crate::Builder;

/// The `history_with_metadata.sqlite` table looks like the following:
///
/// |id|command                 |exit_code|start_ts                  |completed_ts              |pwd        |shell|username      |hostname      |session_id     |git_branch|cloud_workflow_id|workflow_command   |
/// |1 |echo "foo"              |0        |2023-07-11 16:29:32.092176|2023-07-11 16:29:33.124078|/Users/user|zsh  |local:user    |local:host    |168911816423351|NULL      |NULL             |NULL               |
/// |2 |[[ -n "foo" ]]          |0        |2023-07-11 16:29:34.837961|2023-07-11 16:29:33.124078|/Users/user|zsh  |local:user    |local:host    |168911816423351|NULL      |NULL             |[[ -n {{string}} ]]|
/// |3 |echo "bar"              |0        |2023-07-11 16:29:42.000000|2023-07-11 16:29:43.000000|/Users/user|zsh  |local:user    |local:host    |168911816423351|NULL      |NULL             |NULL               |
/// |10|sed -i '' '/hello/d' foo|0        |2023-07-12 16:29:42.000000|2023-07-12 16:29:43.000000|/Users/user|zsh  |local:user    |local:host    |168911816423351|NULL      |NULL             |sed -i '' '/{{string}}/d' {{file}}|
///
/// These three rows are duplicated in the table for each `shell` type so history tests can run on all shells with the same data.
///
/// Note that the user and host columns are updated at runtime in test setup to the actual local
/// user and host values for the machine on which this test is running -- this is necessary because
/// app logic depends on user and host values to match persisted commands to live sessions.
const FAKE_HISTORY_SQLITE_FILE: &str = "history_with_metadata.sqlite";

pub fn test_up_arrow_history() -> Builder {
    new_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(new_step_with_default_assertions("Run ls").with_keystrokes(&["l", "s", "enter"]))
        .with_step(
            new_step_with_default_assertions(
                "Run multiline command and verify history menu is not visible",
            )
            .with_keystrokes(&["c", "shift-enter", "n", "enter"])
            .add_assertion(|app, window_id| {
                let views = app.views_of_type(window_id).unwrap();
                let input_view: &ViewHandle<Input> = views.first().unwrap();
                input_view.read(app, |view, ctx| {
                    async_assert!(
                        !view
                            .suggestions_mode_model()
                            .as_ref(ctx)
                            .mode()
                            .is_visible(),
                        "Input suggestion should not be visible right now."
                    )
                })
            }),
        )
        .with_step(
            new_step_with_default_assertions(
                "Enter up key and verify previous command is in buffer",
            )
            .with_keystrokes(&[
                "up", // this should open the history menu
            ])
            .add_assertion(|app, window_id| {
                let views = app.views_of_type(window_id).unwrap();
                let input_view: &ViewHandle<Input> = views.first().unwrap();
                input_view.read(app, |view, ctx| {
                    // The history menu should be visible.
                    assert!(view
                        .suggestions_mode_model()
                        .as_ref(ctx)
                        .mode()
                        .is_visible());

                    // The cursor should be on the last row.
                    assert!(view.editor().as_ref(ctx).single_cursor_on_last_row(ctx));
                    async_assert!(
                        view.buffer_text(ctx) == *"c\nn",
                        "History menu should show the previous multiline command"
                    )
                })
            }),
        )
        .with_step(
            new_step_with_default_assertions(
                "Enter up key again and verify cursor moves to the top row",
            )
            .with_keystrokes(&[
                "up", // this should move the cursor to the top line
            ])
            .add_assertion(|app, window_id| {
                let views = app.views_of_type(window_id).unwrap();
                let input_view: &ViewHandle<Input> = views.first().unwrap();
                input_view.read(app, |view, ctx| {
                    // The history menu should be visible.
                    assert!(view
                        .suggestions_mode_model()
                        .as_ref(ctx)
                        .mode()
                        .is_visible());

                    // The cursor should be on the first row.
                    assert!(view.editor().as_ref(ctx).single_cursor_on_first_row(ctx));
                    async_assert!(
                        view.buffer_text(ctx) == *"c\nn",
                        "History menu should still show the previous multiline command"
                    )
                })
            }),
        )
}

pub fn test_up_arrow_history_enters_shift_tab_for_workflow() -> Builder {
    new_builder()
        .with_setup(|utils| {
            integration_testing::create_file_from_assets(
                TEST_ONLY_ASSETS,
                FAKE_HISTORY_SQLITE_FILE,
                &integration_testing::persistence::database_file_path_for_scope(
                    &integration_testing::persistence::PersistenceScope::App,
                ),
            );

            let local_user = get_local_user();
            let local_hostname = get_local_hostname().expect("Failed to retrieve system hostname.");
            set_user_and_hostname_for_commands(local_user, local_hostname);

            let home_dir = utils.test_dir();
            write_histfiles_for_test(
                home_dir,
                vec![r#"echo "foo""#, r#"sed -i '' '/hello/d' foo"#],
                [
                    ShellType::Zsh,
                    ShellType::Bash,
                    ShellType::Fish,
                    ShellType::PowerShell,
                ],
            );
        })
        .with_user_defaults(HashMap::from([(
            ShowGlobalWorkflowsInUniversalSearch::storage_key().to_owned(),
            "true".to_owned(),
        )]))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions(
                "Enter up key and verify terminal input contains workflow command",
            )
            .with_keystrokes(&[
                "up", // this should move the cursor to the top line
            ])
            .add_assertion(|app, window_id| {
                let input_view = single_input_view(app, window_id);
                input_view.read(app, |view, ctx| {
                    // The history menu should be visible.
                    async_assert!(view
                        .suggestions_mode_model()
                        .as_ref(ctx)
                        .mode()
                        .is_visible())
                })
            })
            .add_named_assertion(
                "Input contains most recent command",
                assert_input_editor_contents(0, "sed -i '' '/hello/d' foo"),
            ),
        )
        .with_step(
            new_step_with_default_assertions("Update \"string\" workflow parameter")
                .with_keystrokes(&[
                    "shift-tab", // this should cause the first argument to be highlighted
                ])
                .with_typed_characters(&[
                    "bye", // this should result in us replacing the first argument
                ])
                .add_named_assertion(
                    "First workflow parameter is substituted",
                    assert_input_editor_contents(0, "sed -i '' '/bye/d' foo"),
                ),
        )
        .with_step(
            new_step_with_default_assertions("Update \"string\" workflow parameter")
                .with_keystrokes(&["shift-tab"])
                .with_typed_characters(&["baz"])
                .add_named_assertion(
                    "Second workflow parameter is substituted",
                    assert_input_editor_contents(0, "sed -i '' '/bye/d' baz"),
                ),
        )
}

/// Tests that history commands are loaded from the shell's histfile.
pub fn test_command_search_loads_history() -> Builder {
    new_builder()
        .with_setup(|utils| {
            integration_testing::create_file_from_assets(
                TEST_ONLY_ASSETS,
                FAKE_HISTORY_SQLITE_FILE,
                &integration_testing::persistence::database_file_path_for_scope(
                    &integration_testing::persistence::PersistenceScope::App,
                ),
            );

            let local_user = get_local_user();
            let local_hostname = get_local_hostname().expect("Failed to retrieve system hostname.");
            set_user_and_hostname_for_commands(local_user, local_hostname);

            let home_dir = utils.test_dir();
            write_histfiles_for_test(
                home_dir,
                vec![r#"echo "foo""#, r#"[[ -n "foo" ]]"#, r#"echo "bar""#],
                [
                    ShellType::Zsh,
                    ShellType::Bash,
                    ShellType::Fish,
                    ShellType::PowerShell,
                ],
            );
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Open command search")
                .with_keystrokes(&["ctrl-r"])
                .add_named_assertion("Command search is open", assert_command_search_is_open()),
        )
        .with_step(
            new_step_with_default_assertions("Select history filter")
                .with_typed_characters(&["h"])
                .with_keystrokes(&["tab"])
                .add_named_assertion(
                    "History filter is active",
                    assert_history_filter_is_active(),
                ),
        )
        .with_step(
            new_step_with_default_assertions("Wait for history results").add_named_assertion(
                "Command search has history results",
                assert_command_search_has_results(),
            ),
        )
        .with_step(
            new_step_with_default_assertions("Loads history from sqlite")
                .with_keystrokes(&["up", "up", "enter"])
                .add_named_assertion(
                    "Input contains selected history command",
                    assert_input_editor_contents(0, r#"echo "foo""#),
                ),
        )
}

/// Tests that history commands are loaded from the shell's histfile.
pub fn test_command_search_loads_history_from_nondefault_histfile_path() -> Builder {
    new_builder()
        .with_setup(|_utils| {
            integration_testing::create_file_from_assets(
                TEST_ONLY_ASSETS,
                FAKE_HISTORY_SQLITE_FILE,
                &integration_testing::persistence::database_file_path_for_scope(
                    &integration_testing::persistence::PersistenceScope::App,
                ),
            );

            let local_user = get_local_user();
            let local_hostname = get_local_hostname().expect("Failed to retrieve system hostname.");
            set_user_and_hostname_for_commands(local_user, local_hostname);

            let base_dirs =
                directories::BaseDirs::new().expect("should be able to determine home directory");

            write_histfiles_for_test(
                base_dirs.home_dir(),
                vec![r#"echo "foo""#, r#"[[ -n "foo" ]]"#, r#"echo "bar""#],
                [ShellType::Zsh, ShellType::Bash, ShellType::Fish],
            );
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Open command search")
                .with_keystrokes(&["ctrl-r"])
                .add_named_assertion("Command search is open", assert_command_search_is_open()),
        )
        .with_step(
            new_step_with_default_assertions("Select history filter")
                .with_typed_characters(&["h"])
                .with_keystrokes(&["tab"])
                .add_named_assertion(
                    "History filter is active",
                    assert_history_filter_is_active(),
                ),
        )
        .with_step(
            new_step_with_default_assertions("Wait for history results").add_named_assertion(
                "Command search has history results",
                assert_command_search_has_results(),
            ),
        )
        .with_step(
            new_step_with_default_assertions("Loads history from sqlite")
                .with_keystrokes(&["up", "up", "enter"])
                .add_named_assertion(
                    "Input contains selected history command",
                    assert_input_editor_contents(0, r#"echo "foo""#),
                ),
        )
}

/// Tests that commands in the histfile are treated as the "source of truth" for shell history, and
/// that the command rows persisted to the sqlite table are only used to join against the list of
/// histfile commands, effectively "enriching" them with metadata.
///
/// Basically, if a user manually deletes a command from their shell histfile, it should not show
/// up in Warp -- so we effectively do a "left join" on commands from the histfile with commands
/// loaded from sqlite.
pub fn test_histfile_left_joined_with_persisted_history() -> Builder {
    new_builder()
        .with_setup(|utils| {
            integration_testing::create_file_from_assets(
                TEST_ONLY_ASSETS,
                FAKE_HISTORY_SQLITE_FILE,
                &integration_testing::persistence::database_file_path_for_scope(
                    &integration_testing::persistence::PersistenceScope::App,
                ),
            );

            let local_user = get_local_user();
            let local_hostname = get_local_hostname().expect("Failed to retrieve system hostname.");
            set_user_and_hostname_for_commands(local_user, local_hostname);

            let home_dir = utils.test_dir();
            write_histfiles_for_test(
                home_dir,
                vec![r#"echo "foo""#, r#"echo "bar""#],
                [
                    ShellType::Zsh,
                    ShellType::Bash,
                    ShellType::Fish,
                    ShellType::PowerShell,
                ],
            );
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Open command search")
                .with_keystrokes(&["ctrl-r"])
                .add_named_assertion("Command search is open", assert_command_search_is_open()),
        )
        .with_step(
            new_step_with_default_assertions("Select history filter")
                .with_typed_characters(&["h"])
                .with_keystrokes(&["tab"])
                .add_named_assertion(
                    "History filter is active",
                    assert_history_filter_is_active(),
                ),
        )
        .with_step(
            new_step_with_default_assertions("Wait for history results").add_named_assertion(
                "Command search has history results",
                assert_command_search_has_results(),
            ),
        )
        .with_step(
            new_step_with_default_assertions("Loads history from sqlite")
                .with_keystrokes(&["up", "enter"])
                .add_named_assertion(
                    "Input contains history command from histfile",
                    assert_input_editor_contents(0, r#"echo "foo""#),
                ),
        )
}

pub fn test_history_command_is_linked_to_local_workflow() -> Builder {
    new_builder()
        .with_setup(|utils| {
            integration_testing::create_file_from_assets(
                TEST_ONLY_ASSETS,
                FAKE_HISTORY_SQLITE_FILE,
                &integration_testing::persistence::database_file_path_for_scope(
                    &integration_testing::persistence::PersistenceScope::App,
                ),
            );

            let local_user = get_local_user();
            let local_hostname = get_local_hostname().expect("Failed to retrieve system hostname.");
            set_user_and_hostname_for_commands(local_user, local_hostname);

            let home_dir = utils.test_dir();
            write_histfiles_for_test(
                home_dir,
                vec![r#"echo "foo""#, r#"[[ -n "foo" ]]"#, r#"echo "bar""#],
                [
                    ShellType::Zsh,
                    ShellType::Bash,
                    ShellType::Fish,
                    ShellType::PowerShell,
                ],
            );
        })
        .with_user_defaults(HashMap::from([(
            ShowGlobalWorkflowsInUniversalSearch::storage_key().to_owned(),
            "true".to_owned(),
        )]))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Open command search")
                .with_keystrokes(&["ctrl-r"])
                .add_named_assertion("Command search is open", assert_command_search_is_open()),
        )
        .with_step(
            new_step_with_default_assertions("Select history filter")
                .with_typed_characters(&["h"])
                .with_keystrokes(&["tab"])
                .add_named_assertion(
                    "History filter is active",
                    assert_history_filter_is_active(),
                ),
        )
        .with_step(
            new_step_with_default_assertions("Wait for history results").add_named_assertion(
                "Command search has history results",
                assert_command_search_has_results(),
            ),
        )
        .with_step(
            new_step_with_default_assertions("Loads history from sqlite")
                .with_keystrokes(&["up", "enter"])
                .add_named_assertion(
                    "Input contains selected history command",
                    assert_input_editor_contents(0, r#"[[ -n "foo" ]]"#),
                )
                .add_named_assertion(
                    "Workflows info box is open",
                    assert_workflow_info_box_is_open(0, 0),
                ),
        )
}
