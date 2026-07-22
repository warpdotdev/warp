use warp::integration_testing::clipboard::assert_clipboard_contains_string;
use warp::integration_testing::command_palette::open_command_palette_and_run_action;
use warp::integration_testing::terminal::wait_until_bootstrapped_single_pane_for_tab;
use warp::integration_testing::view_getters::pane_group_view;
use warpui_core::integration::{AssertionOutcome, TestStep};

use super::{Builder, new_builder};
use crate::util::write_all_rc_files_for_test;

/// Running the "Copy current path" command-palette action while a regular terminal is focused
/// copies the focused session's working directory to the clipboard.
///
/// The file-viewer branch of `PaneGroup::path_from_focused_pane` (copying the open file's
/// `display_path()`) shares this same dispatch path and reuses the accessors covered by the
/// file-viewer context-menu unit tests, so it is not re-exercised end-to-end here.
pub fn test_copy_current_path_copies_terminal_pwd() -> Builder {
    new_builder()
        .with_setup(|utils| {
            let test_dir = utils.test_dir();
            let dir_string = test_dir
                .to_str()
                .expect("Should be able to convert test dir to str");
            // Start the shell in a known directory so the focused session has a stable pwd.
            write_all_rc_files_for_test(&test_dir, format!("cd {dir_string}"));
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_steps(open_command_palette_and_run_action("Copy current path"))
        .with_step(
            TestStep::new("Clipboard contains the focused terminal pwd").add_assertion(
                |app, window_id| {
                    let pane_group = pane_group_view(app, window_id, 0);
                    let expected = pane_group.read(app, |pane_group, ctx| {
                        pane_group
                            .focused_session_view(ctx)
                            .and_then(|terminal_view| terminal_view.as_ref(ctx).pwd())
                    });
                    let Some(expected) = expected else {
                        return AssertionOutcome::failure(
                            "focused terminal pwd not yet available".to_string(),
                        );
                    };
                    let mut assert_clipboard = assert_clipboard_contains_string(expected);
                    assert_clipboard(app, window_id)
                },
            ),
        )
}
