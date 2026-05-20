use settings::{RespectUserSyncSetting, SyncToCloud};
use warp::{
    features::FeatureFlag,
    integration_testing::{
        self,
        notebook::{
            assert_cloud_preference_exists, assert_notebook_contents,
            assert_notebook_metadata_revision,
        },
        step::{new_step_with_default_assertions, new_step_with_default_assertions_for_pane},
        tab::assert_pane_title,
        terminal::wait_until_bootstrapped_single_pane_for_tab,
        view_getters::single_terminal_view_for_tab,
        workflow::assert_workflow_metadata_revision,
    },
    settings::Preference,
    settings_view::{SettingsSection, SettingsView},
    sqlite_testing::set_user_and_hostname_for_blocks,
    terminal::{
        model::{session::get_local_hostname, terminal_model::BlockIndex},
        shell::ShellType,
        History, ShellHost, TerminalView,
    },
    workspace::Workspace,
};
use warpui::{
    async_assert_eq,
    integration::{AssertionOutcome, TestStep},
    SingletonEntity, ViewHandle,
};

use crate::util::{get_local_user, tab_title_in_home_dir};

use super::{new_builder, Builder, TEST_ONLY_ASSETS};

pub fn test_session_restoration() -> Builder {
    new_builder()
        .with_setup(|_utils| {
            integration_testing::create_file_from_assets(
                TEST_ONLY_ASSETS,
                // Three tabs is a snapshot with three tabs that have the cwd None.
                "three_tabs.sqlite",
                &integration_testing::persistence::database_file_path_for_scope(
                    &integration_testing::persistence::PersistenceScope::App,
                ),
            );
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(1))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(2))
        .with_step(
            new_step_with_default_assertions("Assert the app state").add_assertion(
                move |app, window_id| {
                    // There should be three tabs.
                    let workspace_views: Vec<ViewHandle<Workspace>> =
                        app.views_of_type(window_id).expect("Workspace must exist");
                    let workspace = workspace_views.first().expect("Workspace must exist");
                    workspace.read(app, |workspace, _| assert_eq!(workspace.tab_count(), 3));

                    // There should be three terminal views.
                    let terminal_views: Vec<ViewHandle<TerminalView>> =
                        app.views_of_type(window_id).expect("Terminals must exist");
                    assert_eq!(terminal_views.len(), 3);

                    // The pwd should be ~ for each one.
                    for terminal_view in terminal_views {
                        terminal_view.read(app, |terminal_view, _| {
                            let model = terminal_view.model.lock();
                            let pwd = model
                                .block_list()
                                .active_block()
                                .user_friendly_pwd()
                                .expect("Should have pwd");
                            assert_eq!(pwd, "~");
                        });
                    }
                    AssertionOutcome::Success
                },
            ),
        )
}

/// Saved blocks run on different hosts/shells should NOT get added to History::session_commands
/// during session restoration. However, if we have NULL for the shell/host information, it should
/// always get added. The mock data for this case looks like this:
/// | command            | output       | shell | user       | host          |
/// | ------------------ | ------------ | ----- | ---------- | ------------- |
/// | echo $TERM_PROGRAM | WarpTerminal | zsh   | local:user | local:host    |
/// | pwd                | /            | bash  | local:user | local:host    |
/// | uname              | Linux        | zsh   | andy       | ubuntu-test   |
/// | mkdir secrets      | secrets      | NULL  | NULL       | NULL          |
/// | echo foobar        | foobar       | pwsh  | local:user | local:host    |
pub fn test_restored_blocks_on_different_hosts() -> Builder {
    new_builder()
        .with_setup(|_utils| {
            integration_testing::create_file_from_assets(
                TEST_ONLY_ASSETS,
                "restored_blocks.sqlite",
                &integration_testing::persistence::database_file_path_for_scope(
                    &integration_testing::persistence::PersistenceScope::App,
                ),
            );

            let local_user = get_local_user();
            let local_hostname = get_local_hostname().expect("Failed to retrieve system hostname.");
            set_user_and_hostname_for_blocks(local_user, local_hostname);
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Assert the app state").add_assertion(
                move |app, window_id| {
                    let terminal_views: Vec<ViewHandle<TerminalView>> = app
                        .views_of_type(window_id)
                        .expect("Should have views of type TerminalView after bootstrapping");
                    assert_eq!(terminal_views.len(), 1);

                    let terminal_view = &terminal_views[0];
                    terminal_view.read(app, |terminal, ctx| {
                        History::handle(ctx).read(app, |history, ctx| {
                            let session = terminal
                                .active_block_session_id()
                                .and_then(|session_id| terminal.sessions(ctx).get(session_id))
                                .expect("terminal should have active session after bootstrap");

                            let local_user = get_local_user();
                            let local_hostname =
                                get_local_hostname().expect("Failed to retrieve system hostname.");

                            let shell_type = session.shell().shell_type();
                            let shell_host = ShellHost {
                                shell_type,
                                user: local_user,
                                hostname: local_hostname,
                            };
                            let hist_list = &history.session_commands()[&shell_host];
                            match shell_type {
                                ShellType::Zsh => {
                                    assert_eq!(hist_list.len(), 2);
                                    assert_eq!(
                                        hist_list[0].command, "echo $TERM_PROGRAM",
                                        "history item 1 for Zsh"
                                    );
                                    async_assert_eq!(
                                        hist_list[1].command,
                                        "mkdir secrets",
                                        "history item 2 for Zsh"
                                    )
                                }
                                ShellType::Bash => {
                                    assert_eq!(hist_list.len(), 2);
                                    assert_eq!(
                                        hist_list[0].command, "pwd",
                                        "history items for Bash"
                                    );
                                    async_assert_eq!(
                                        hist_list[1].command,
                                        "mkdir secrets",
                                        "history item 2 for Bash"
                                    )
                                }
                                ShellType::Fish => {
                                    assert_eq!(hist_list.len(), 1, "fish has no restored commands");
                                    async_assert_eq!(
                                        hist_list[0].command,
                                        "mkdir secrets",
                                        "history items for fish"
                                    )
                                }
                                ShellType::PowerShell => {
                                    assert_eq!(hist_list.len(), 2);
                                    assert_eq!(
                                        hist_list[0].command, "mkdir secrets",
                                        "history items for PowerShell"
                                    );
                                    async_assert_eq!(
                                        hist_list[1].command,
                                        "echo foobar",
                                        "history item 2 for PowerShell"
                                    )
                                }
                            }
                        })
                    })
                },
            ),
        )
}

/// Regression test to ensure we don't ever crash in this scenario.
pub fn test_restore_snapshot_with_deleted_cwd() -> Builder {
    new_builder()
        .with_setup(|_utils| {
            integration_testing::create_file_from_assets(
                TEST_ONLY_ASSETS,
                "deleted_cwd.sqlite",
                &integration_testing::persistence::database_file_path_for_scope(
                    &integration_testing::persistence::PersistenceScope::App,
                ),
            );
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Assert there's one terminal with ~ as the pwd")
                .add_assertion(move |app, window_id| {
                    // There should be one terminal view.
                    let terminal_views: Vec<ViewHandle<TerminalView>> =
                        app.views_of_type(window_id).expect("Terminal must exist");
                    assert_eq!(terminal_views.len(), 1);
                    let terminal_view = terminal_views
                        .first()
                        .expect("There is exactly one terminal view");

                    terminal_view.read(app, |terminal_view, _| {
                        let model = terminal_view.model.lock();
                        let pwd = model
                            .block_list()
                            .active_block()
                            .user_friendly_pwd()
                            .expect("Should have pwd");
                        assert_eq!(pwd, "~");
                    });
                    AssertionOutcome::Success
                }),
        )
}

// Note: this test is brittle b/c it depends on sqlite having accurate paths to
// the bash and zsh executables in the test runner. If we have a mechanism for it,
// it would be nice to be able to modify the sqlite template to include the proper
// paths, rather than having to hardcode them in advance.
pub fn test_session_restoration_with_multiple_shells() -> Builder {
    FeatureFlag::ShellSelector.set_enabled(true);
    new_builder()
        .with_setup(|_utils| {
            integration_testing::create_file_from_assets(
                TEST_ONLY_ASSETS,
                "test_restoring_tabs_with_different_shells.sqlite",
                &integration_testing::persistence::database_file_path_for_scope(
                    &integration_testing::persistence::PersistenceScope::App,
                ),
            );
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(1))
        .with_step(
            new_step_with_default_assertions("Assert that tabs are of different shells")
                .add_assertion(move |app, window_id| {
                    let terminal_views: Vec<ViewHandle<TerminalView>> =
                        app.views_of_type(window_id).expect("Terminals must exist");
                    assert_eq!(terminal_views.len(), 2);

                    let bash_view = &terminal_views[0];
                    let zsh_view = &terminal_views[1];

                    assert_eq!(
                        zsh_view.read(app, |session, ctx| session.active_session_shell_type(ctx)),
                        Some(ShellType::Zsh)
                    );
                    assert_eq!(
                        bash_view.read(app, |session, ctx| session.active_session_shell_type(ctx)),
                        Some(ShellType::Bash)
                    );
                    AssertionOutcome::Success
                }),
        )
}

/// Background output should be restored inline with regular command blocks.
/// The session being restored is:
/// ```shell
/// $ (sleep 5l echo "background output") &
/// [1] 1512
/// $ echo foreground 1
/// foreground 1
/// background output
/// [1]  + done       ( sleep 5; echo "background output"; )
/// $ echo foreground 2
/// foreground 2
/// ```
pub fn test_restore_snapshot_with_background_output() -> Builder {
    new_builder()
        .with_setup(|_utils| {
            integration_testing::create_file_from_assets(
                TEST_ONLY_ASSETS,
                "restored_background_blocks.sqlite",
                &integration_testing::persistence::database_file_path_for_scope(
                    &integration_testing::persistence::PersistenceScope::App,
                ),
            );
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            new_step_with_default_assertions("Assert the background output is restored")
                .add_named_assertion("block list contents", move |app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |terminal, _| {
                        let model = terminal.model.lock();
                        let blocks = model.block_list();

                        let command_block = blocks
                            .block_at(BlockIndex::from(0))
                            .expect("command block should exist");
                        assert!(!command_block.is_background());
                        assert_eq!(
                            &command_block.command_to_string(),
                            r#"(sleep 5; echo "background output") &"#
                        );
                        assert_eq!(&command_block.output_to_string(), "[1] 1512");

                        let foreground_block_1 = blocks
                            .block_at(BlockIndex::from(1))
                            .expect("block should exist");
                        assert!(!foreground_block_1.is_background());
                        assert_eq!(foreground_block_1.command_to_string(), "echo foreground 1");

                        let background_block = blocks
                            .block_at(BlockIndex::from(2))
                            .expect("block should exist");
                        assert!(background_block.is_background());
                        assert!(background_block.command_to_string().is_empty());
                        assert_eq!(
                            background_block.output_to_string(),
                            r#"background output

[1]  + done       ( sleep 5; echo "background output"; )"#
                        );

                        let foreground_block_2 = blocks
                            .block_at(BlockIndex::from(3))
                            .expect("block should exist");
                        assert!(!foreground_block_2.is_background());
                        assert_eq!(foreground_block_2.command_to_string(), "echo foreground 2");

                        AssertionOutcome::Success
                    })
                }),
        )
}

/// Tests restoring a snapshot that includes notebook panes.
///
/// The snapshot has a single window with one tab, containing:
/// * A notebook pane, where the notebook exists
/// * A notebook pane, where the notebook no longer exists
/// * A terminal pane
pub fn test_restore_snapshot_with_notebooks() -> Builder {
    new_builder()
        .with_setup(|_utils| {
            integration_testing::create_file_from_assets(
                TEST_ONLY_ASSETS,
                "restored_notebooks.sqlite",
                &integration_testing::persistence::database_file_path_for_scope(
                    &integration_testing::persistence::PersistenceScope::App,
                ),
            );
        })
        .with_step(
            TestStep::new("Verify that the notebook panes were restored")
                .add_assertion(assert_pane_title(0, 0, "First Notebook"))
                // The missing notebook should be replaced with an empty new notebook.
                .add_assertion(assert_pane_title(0, 1, "Untitled")),
        )
        .with_step(
            new_step_with_default_assertions_for_pane("Wait for terminal pane to bootstrap", 0, 2)
                .add_assertion(assert_pane_title(
                    0,
                    2,
                    tab_title_in_home_dir("test_restore_snapshot_with_notebooks"),
                )),
        )
        .with_step(
            TestStep::new("Verify notebook contents")
                .add_assertion(assert_notebook_contents(0, 0, "Notebook 1 content"))
                .add_assertion(assert_notebook_contents(0, 1, "")),
        )
}

/// Test restoring a snapshot that includes workflow panes - the second pane exists, but the first
/// is for a deleted workflow.
pub fn test_restore_snapshot_with_workflows() -> Builder {
    new_builder()
        .with_setup(|_utils| {
            integration_testing::create_file_from_assets(
                TEST_ONLY_ASSETS,
                "restored_workflows.sqlite",
                &integration_testing::persistence::database_file_path_for_scope(
                    &integration_testing::persistence::PersistenceScope::App,
                ),
            )
        })
        .with_step(
            TestStep::new("Verify that the workflow panes were restored")
                .add_assertion(assert_pane_title(0, 1, "My Workflow"))
                .add_assertion(assert_pane_title(0, 0, "Untitled")),
        )
}

/// Tests restoring a snapshot that includes a test json object.
///
/// The test json object has as its contents the string "egpmggresq"
pub fn test_restore_snapshot_with_test_json_object() -> Builder {
    new_builder()
        .with_setup(|_utils| {
            integration_testing::create_file_from_assets(
                TEST_ONLY_ASSETS,
                "test_json_object.sqlite",
                &integration_testing::persistence::database_file_path_for_scope(
                    &integration_testing::persistence::PersistenceScope::App,
                ),
            );
        })
        .with_step(
            TestStep::new("Verify json object contents").add_assertion(
                assert_cloud_preference_exists(
                    Preference::new(
                        "HonorPS1".to_string(),
                        "false",
                        SyncToCloud::Globally(RespectUserSyncSetting::Yes),
                    )
                    .expect("error creating preference"),
                ),
            ),
        )
}

/// Tests restoring a snapshot that has multiple objects with the same shareable_object_id
/// in the metadata table.  This test verifies a regression introduced in
/// https://github.com/warpdotdev/warp-internal/pull/7406 and fixed in
/// https://github.com/warpdotdev/warp-internal/pull/7480
///
/// The two objects have server ids Workflow-ftv7on4HwTeixO2xF5hmKf and Notebook-Flbu686H9XDCHZlYRriVpB
/// and shareable_object_id 2.
pub fn test_restore_snapshot_with_common_shareable_metadata_ids() -> Builder {
    new_builder()
        .with_setup(|_utils| {
            integration_testing::create_file_from_assets(
                TEST_ONLY_ASSETS,
                "test_duplicate_shareable_ids.sqlite",
                &integration_testing::persistence::database_file_path_for_scope(
                    &integration_testing::persistence::PersistenceScope::App,
                ),
            );
        })
        .with_step(TestStep::new("Verify revision of workflow").add_assertion(
            assert_workflow_metadata_revision("ftv7on4HwTeixO2xF5hmKf", 1676321629559090),
        ))
        .with_step(TestStep::new("Verify revision of notebook").add_assertion(
            assert_notebook_metadata_revision("Flbu686H9XDCHZlYRriVpB", 1690991057168223),
        ))
}

/// Tests restoring a snapshot that includes a Markdown file pane.
///
/// The snapshot has a single window with one tab, containing:
/// * A terminal pane
/// * A Markdown file pane, `test.md` (backed by [`../../tests/data/test.md`]).
///
/// Normally, we store absolute paths in SQLite for restoring Markdown panes. The test uses a
/// relative path for portability, and assumes it's run from the root of the `integration` crate.
pub fn test_restore_snapshot_with_markdown_file() -> Builder {
    new_builder()
        .with_setup(|utils| {
            integration_testing::create_file_from_assets(
                TEST_ONLY_ASSETS,
                "file_notebook.sqlite",
                &integration_testing::persistence::database_file_path_for_scope(
                    &integration_testing::persistence::PersistenceScope::App,
                ),
            );
            integration_testing::create_file_from_assets(
                TEST_ONLY_ASSETS,
                "test.md",
                &utils.test_dir().join("docs/test.md"),
            );
        })
        // Wait for the terminal pane to bootstrap first - we need an active session to resolve the
        // home directory and context for the notebook pane.
        .with_step(
            new_step_with_default_assertions_for_pane("Wait for terminal pane to bootstrap", 0, 0)
                .add_assertion(assert_pane_title(
                    0,
                    0,
                    tab_title_in_home_dir("test_restore_snapshot_with_markdown_file"),
                )),
        )
        .with_step(
            // The pane title isn't set until after the Markdown file is read in, so this verifies
            // that both pieces were successful.
            TestStep::new("Verify that the notebook pane was restored")
                .add_assertion(assert_pane_title(0, 1, "test.md")),
        )
}

/// Tests restoring a snapshot that includes a code pane.
///
/// The snapshot has a single window with one tab, containing:
/// * A terminal pane
/// * A code pane, `test.rs` (backed by [`../../tests/data/test.rs`]).
///
/// Normally, we store absolute paths in SQLite for restoring code panes. The test uses a
/// relative path for portability, and assumes it's run from the root of the `integration` crate.
pub fn test_restore_snapshot_with_code_file() -> Builder {
    new_builder()
        .with_setup(|utils| {
            integration_testing::create_file_from_assets(
                TEST_ONLY_ASSETS,
                "restored_code.sqlite",
                &integration_testing::persistence::database_file_path_for_scope(
                    &integration_testing::persistence::PersistenceScope::App,
                ),
            );
            integration_testing::create_file_from_assets(
                TEST_ONLY_ASSETS,
                "test.rs",
                &utils.test_dir().join("docs/test.rs"),
            );
        })
        // Wait for the terminal pane to bootstrap first - we need an active session to resolve the
        // home directory and context for the notebook pane.
        .with_step(
            new_step_with_default_assertions_for_pane("Wait for terminal pane to bootstrap", 0, 0)
                .add_assertion(assert_pane_title(
                    0,
                    0,
                    tab_title_in_home_dir("test_restore_snapshot_with_code_file"),
                )),
        )
        .with_step(
            // The pane title isn't set until after the file is read in, so this verifies
            // that both pieces were successful.
            TestStep::new("Verify that the code pane was restored")
                .add_assertion(assert_pane_title(0, 1, "./docs/test.rs")),
        )
}

/// Tests restoring a snapshot that includes a settings pane.
///
/// The snapshot has a single window with one tab, containing:
/// * A terminal pane
/// * A settings pane (with page set to "Referrals")
pub fn test_restore_snapshot_with_settings_page() -> Builder {
    new_builder()
        .with_setup(|_utils| {
            integration_testing::create_file_from_assets(
                TEST_ONLY_ASSETS,
                "restored_settings.sqlite",
                &integration_testing::persistence::database_file_path_for_scope(
                    &integration_testing::persistence::PersistenceScope::App,
                ),
            );
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            TestStep::new("Verify settings pane restoration")
                .add_assertion(assert_pane_title(0, 1, "Settings"))
                .add_assertion(move |app, window_id| {
                    // Verify the settings view exists and is on the Referrals page.
                    let settings_views: Vec<ViewHandle<SettingsView>> = app
                        .views_of_type(window_id)
                        .expect("Settings view must exist");
                    assert_eq!(settings_views.len(), 1);

                    let settings_view = settings_views.first().expect("Settings view must exist");
                    settings_view.read(app, |view, _| {
                        async_assert_eq!(
                            view.current_settings_section(),
                            SettingsSection::Referrals
                        )
                    })
                }),
        )
}
