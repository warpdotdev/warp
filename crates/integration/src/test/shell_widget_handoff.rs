//! Live fzf ctrl-r / ctrl-t handoff tests. Skipped when fzf is not installed or the current
//! shell is not bash/zsh/fish (the shells fzf ships key-bindings for).

use std::path::{Path, PathBuf};
use std::time::Duration;

use command::blocking::Command;
use warp::features::FeatureFlag;
use warp::integration_testing;
use warp::integration_testing::step::new_step_with_default_assertions;
use warp::integration_testing::terminal::util::{
    ExpectedExitStatus, current_shell_starter_and_version,
};
use warp::integration_testing::terminal::{
    assert_input_editor_contents, execute_command_for_single_terminal_in_tab,
    wait_until_bootstrapped_single_pane_for_tab,
};
use warp::integration_testing::view_getters::{
    single_input_view_for_tab, single_terminal_view_for_tab, workspace_view,
};
use warp::terminal::shell::ShellType;
use warpui_core::async_assert;
use warpui_core::integration::{AssertionCallback, TestStep};

use super::{TEST_ONLY_ASSETS, new_builder};
use crate::Builder;
use crate::util::{
    ShellRcType, set_zsh_histfile_location, should_run_fzf_widget_handoff_tests,
    write_rc_files_for_test,
};

const CTRL_R_HISTORY_COMMAND: &str = "echo fzf_ctrl_r_marker_alpha";
const CTRL_R_HISTORY_OUTPUT: &str = "fzf_ctrl_r_marker_alpha";
const CTRL_R_DRAFT: &str = "draft_before_ctrl_r";
const CTRL_T_FILENAME: &str = "fzf_ctrl_t_marker_file.txt";
const CTRL_T_PREFIX: &str = "echo ";
const FZF_STEP_TIMEOUT: Duration = Duration::from_secs(20);

fn fzf_handoff_builder() -> Builder {
    FeatureFlag::ShellWidgetHandoff.set_enabled(true);
    new_builder()
        .set_should_run_test(should_run_fzf_widget_handoff_tests)
        .with_setup(|utils| {
            let home = utils.test_dir();
            install_fzf_key_bindings(&home);
            std::fs::write(home.join(CTRL_T_FILENAME), b"")
                .expect("should be able to create the ctrl-t marker file");
        })
}

fn install_fzf_key_bindings(home: &Path) {
    write_rc_files_for_test(home, bash_fzf_rc(home), [ShellRcType::Bash]);
    write_rc_files_for_test(home, zsh_fzf_rc(home), [ShellRcType::Zsh]);
    write_rc_files_for_test(home, fish_fzf_rc(home), [ShellRcType::Fish]);
    set_zsh_histfile_location(home);
}

fn bash_fzf_rc(home: &Path) -> String {
    if fzf_dumps_script("--bash") {
        "eval \"$(fzf --bash)\"\n".to_owned()
    } else {
        format!(
            ". '{}'\n",
            ensure_fzf_key_bindings(home, "key-bindings.bash").display()
        )
    }
}

fn zsh_fzf_rc(home: &Path) -> String {
    if fzf_dumps_script("--zsh") {
        "source <(fzf --zsh)\n".to_owned()
    } else {
        format!(
            "source '{}'\n",
            ensure_fzf_key_bindings(home, "key-bindings.zsh").display()
        )
    }
}

fn fish_fzf_rc(home: &Path) -> String {
    if fzf_dumps_script("--fish") {
        "fzf --fish | source\n".to_owned()
    } else if fzf_key_bindings_path("key-bindings.fish").is_some()
        || Path::new("/usr/share/fish/vendor_functions.d/fzf_key_bindings.fish").is_file()
    {
        // Distro packages often install the function without calling it.
        "if functions -q fzf_key_bindings; fzf_key_bindings; end\n".to_owned()
    } else {
        format!(
            "source '{}'\n",
            ensure_fzf_key_bindings(home, "key-bindings.fish").display()
        )
    }
}

fn ensure_fzf_key_bindings(home: &Path, filename: &str) -> PathBuf {
    if let Some(path) = fzf_key_bindings_path(filename) {
        return path;
    }
    let dest = home.join(filename);
    integration_testing::create_file_from_assets(
        TEST_ONLY_ASSETS,
        &format!("fzf/{filename}"),
        &dest,
    );
    dest
}

fn fzf_dumps_script(flag: &str) -> bool {
    Command::new("fzf")
        .arg(flag)
        .output()
        .map(|output| output.status.success() && !output.stdout.is_empty())
        .unwrap_or(false)
}

fn fzf_key_bindings_path(filename: &str) -> Option<PathBuf> {
    let mut candidates = vec![
        PathBuf::from(format!("/usr/share/doc/fzf/examples/{filename}")),
        PathBuf::from(format!("/usr/share/fzf/{filename}")),
        PathBuf::from(format!("/usr/share/fzf/shell/{filename}")),
        PathBuf::from(format!("/opt/homebrew/opt/fzf/shell/{filename}")),
        PathBuf::from(format!("/usr/local/opt/fzf/shell/{filename}")),
    ];
    if let Ok(output) = Command::new("brew").args(["--prefix", "fzf"]).output()
        && output.status.success()
    {
        let prefix = String::from_utf8_lossy(&output.stdout);
        let prefix = prefix.trim();
        if !prefix.is_empty() {
            candidates.push(PathBuf::from(prefix).join("shell").join(filename));
        }
    }
    candidates.into_iter().find(|path| path.is_file())
}

fn assert_command_search_is_closed() -> AssertionCallback {
    Box::new(move |app, window_id| {
        let workspace_view = workspace_view(app, window_id);
        workspace_view.read(app, |workspace, _ctx| {
            async_assert!(
                !workspace.is_command_search_open(),
                "Warp command search should not open when fzf owns the key"
            )
        })
    })
}

fn assert_shell_plugin_tag(tag: &'static str) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
        terminal_view.read(app, |view, ctx| {
            let Some(session_id) = view.active_block_session_id() else {
                return warpui_core::integration::AssertionOutcome::failure(
                    "expected an active session after bootstrap".into(),
                );
            };
            let Some(session) = view.sessions(ctx).get(session_id) else {
                return warpui_core::integration::AssertionOutcome::failure(
                    "expected the active session to be registered".into(),
                );
            };
            let plugins = session.shell().plugins();
            async_assert!(
                plugins.contains(tag),
                "expected shell plugin tag {tag}, have {plugins:?}"
            )
        })
    })
}

fn assert_shell_widget_handoff_enabled() -> AssertionCallback {
    Box::new(move |_app, _window_id| {
        async_assert!(
            FeatureFlag::ShellWidgetHandoff.is_enabled(),
            "ShellWidgetHandoff must be enabled or these tests exercise nothing"
        )
    })
}

fn assert_fzf_shows(text: &'static str) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
        terminal_view.read(app, |view, _| {
            let model = view.model.lock();
            let alt = model.alt_screen().output_to_string();
            let block = model.block_list().active_block().output_to_string();
            async_assert!(
                alt.contains(text) || block.contains(text),
                "fzf should show {text:?}; alt-screen={alt:?} block={block:?}"
            )
        })
    })
}

fn assert_input_contains(text: &'static str) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let input = single_input_view_for_tab(app, window_id, 0);
        input.read(app, |view, ctx| {
            let contents = view.buffer_text(ctx);
            async_assert!(
                contents.contains(text),
                "input {contents:?} should contain {text:?}"
            )
        })
    })
}

fn open_fzf(key: &'static str) -> TestStep {
    TestStep::new("Wait for fzf to take over the PTY")
        .with_keystrokes(&[key])
        .set_timeout(FZF_STEP_TIMEOUT)
        .add_named_assertion(
            "command search stayed closed",
            assert_command_search_is_closed(),
        )
        .add_named_assertion(
            "fzf is running as a long-running command",
            assert_fzf_is_running(),
        )
}

fn assert_fzf_is_running() -> AssertionCallback {
    Box::new(move |app, window_id| {
        let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
        terminal_view.read(app, |view, _ctx| {
            let is_editor_focused = view
                .input()
                .read(app, |input, ctx| input.editor().is_focused(ctx));
            let buffer = view
                .input()
                .read(app, |input, ctx| input.buffer_text(ctx));
            let model = view.model.lock();
            let active_block = model.block_list().active_block();
            let output = active_block.output_to_string();
            let long_running = active_block.is_active_and_long_running();
            // bash may not emit preexec for the leading-space helper invocation, so do not
            // require is_executing(); the editor hiding and long-running block are the handoff.
            async_assert!(
                !is_editor_focused && long_running,
                "expected fzf long-running; editor_focused={is_editor_focused} long_running={long_running} buffer={buffer:?} output={output:?}"
            )
        })
    })
}

/// ctrl-r opens the real fzf history picker and lands the selected command in the editor
/// unexecuted.
pub fn test_fzf_ctrl_r_selects_history_unexecuted() -> Builder {
    fzf_handoff_builder()
        .with_step(
            wait_until_bootstrapped_single_pane_for_tab(0).add_named_assertion(
                "ShellWidgetHandoff is enabled",
                assert_shell_widget_handoff_enabled(),
            ),
        )
        .with_step(
            new_step_with_default_assertions("Bootstrap reported the fzf ctrl-r plugin tag")
                .add_named_assertion(
                    "external_ctrl_r_history is tagged",
                    assert_shell_plugin_tag("external_ctrl_r_history"),
                ),
        )
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            CTRL_R_HISTORY_COMMAND.to_owned(),
            ExpectedExitStatus::Success,
            CTRL_R_HISTORY_OUTPUT,
        ))
        .with_step(open_fzf("ctrl-r"))
        .with_step(
            TestStep::new("Filter to the unique history entry")
                .with_typed_characters(&[CTRL_R_HISTORY_OUTPUT])
                .set_timeout(FZF_STEP_TIMEOUT)
                .add_named_assertion(
                    "fzf lists the unique history entry",
                    assert_fzf_shows(CTRL_R_HISTORY_OUTPUT),
                ),
        )
        .with_step(
            TestStep::new("Accept the fzf selection")
                .with_keystrokes(&["enter"])
                .set_timeout(FZF_STEP_TIMEOUT),
        )
        .with_step(
            new_step_with_default_assertions(
                "Selected history command is in the editor unexecuted",
            )
            .add_named_assertion(
                "command search stayed closed",
                assert_command_search_is_closed(),
            )
            .add_named_assertion(
                "input contains the selected command",
                assert_input_editor_contents(0, CTRL_R_HISTORY_COMMAND),
            ),
        )
}

/// ctrl-t opens the real fzf file picker and lands the selection in the editor. bash/zsh splice
/// at the cursor (prefix preserved); fish applies the widget's own finished buffer.
pub fn test_fzf_ctrl_t_inserts_selection() -> Builder {
    let is_fish = current_shell_starter_and_version().0.shell_type() == ShellType::Fish;
    fzf_handoff_builder()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0).add_named_assertion(
            "ShellWidgetHandoff is enabled",
            assert_shell_widget_handoff_enabled(),
        ))
        .with_step(
            new_step_with_default_assertions("Bootstrap reported the fzf ctrl-t plugin tag")
                .add_named_assertion(
                    "external_ctrl_t_file is tagged",
                    assert_shell_plugin_tag("external_ctrl_t_file"),
                ),
        )
        .with_step(
            new_step_with_default_assertions("Type a prefix so splice vs replace is observable")
                .with_typed_characters(&[CTRL_T_PREFIX])
                .with_keystrokes(&["escape"])
                .add_named_assertion(
                    "prefix is in the input",
                    assert_input_editor_contents(0, CTRL_T_PREFIX),
                ),
        )
        .with_step(open_fzf("ctrl-t"))
        .with_step(
            TestStep::new("Filter to the unique file and accept")
                .with_typed_characters(&[CTRL_T_FILENAME])
                .with_keystrokes(&["enter"])
                .set_timeout(FZF_STEP_TIMEOUT),
        )
        .with_step(
            new_step_with_default_assertions("Selected file landed in the editor unexecuted")
                .add_named_assertion("command search stayed closed", assert_command_search_is_closed())
                .add_named_assertion(
                    "input contains the selected filename",
                    assert_input_contains(CTRL_T_FILENAME),
                )
                .add_named_assertion("prefix is preserved on bash/zsh", move |app, window_id| {
                    if is_fish {
                        return warpui_core::integration::AssertionOutcome::Success;
                    }
                    let input = single_input_view_for_tab(app, window_id, 0);
                    input.read(app, |view, ctx| {
                        let contents = view.buffer_text(ctx);
                        async_assert!(
                            contents.contains(CTRL_T_PREFIX.trim()),
                            "bash/zsh should splice at the cursor, keeping the prefix; got {contents:?}"
                        )
                    })
                }),
        )
}

/// Cancel restores the draft that was in the editor when ctrl-r was pressed.
pub fn test_fzf_ctrl_r_cancel_restores_draft() -> Builder {
    fzf_handoff_builder()
        .with_step(
            wait_until_bootstrapped_single_pane_for_tab(0).add_named_assertion(
                "ShellWidgetHandoff is enabled",
                assert_shell_widget_handoff_enabled(),
            ),
        )
        .with_step(
            new_step_with_default_assertions("Bootstrap reported the fzf ctrl-r plugin tag")
                .add_named_assertion(
                    "external_ctrl_r_history is tagged",
                    assert_shell_plugin_tag("external_ctrl_r_history"),
                ),
        )
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            CTRL_R_HISTORY_COMMAND.to_owned(),
            ExpectedExitStatus::Success,
            CTRL_R_HISTORY_OUTPUT,
        ))
        .with_step(
            new_step_with_default_assertions("Type a draft to restore on cancel")
                .with_typed_characters(&[CTRL_R_DRAFT])
                .add_named_assertion(
                    "draft is in the input",
                    assert_input_editor_contents(0, CTRL_R_DRAFT),
                ),
        )
        .with_step(open_fzf("ctrl-r"))
        .with_step(
            TestStep::new("Cancel fzf")
                .with_keystrokes(&["escape"])
                .set_timeout(FZF_STEP_TIMEOUT),
        )
        .with_step(
            new_step_with_default_assertions("Draft is restored after cancel")
                .add_named_assertion(
                    "command search stayed closed",
                    assert_command_search_is_closed(),
                )
                .add_named_assertion(
                    "input still has the original draft",
                    assert_input_editor_contents(0, CTRL_R_DRAFT),
                ),
        )
}
