//! Integration tests for native shell completions, where the client asks the user's real shell to
//! compute completions for the current input line via an in-band generator command and renders the
//! shell's answer in the completions menu (see `native_shell_completions.rs` on the Rust side and
//! the `warp_run_generator_command*` functions in the bootstrap scripts).
//!
//! These boot a real shell and drive it end-to-end, covering the client<->shell seam that unit
//! tests cannot reach. They run against every shell Warp supports native completions on and that
//! the integration runners provide -- zsh, bash, and fish. PowerShell is excluded: it is not
//! installed on the integration runners and the harness writes no PowerShell rc file, so its
//! PSReadLine-driven path cannot be exercised here.
//!
//! Native completions are Tab/keybinding-triggered only; there is deliberately no as-you-type
//! coverage, because as-you-type does not ask the shell.
//!
//! Two switches turn the feature on, and the tests use each for what it proves:
//! - The `ForceNativeShellCompletions` private preference makes the shell's answer win
//!   unconditionally. It needs no cargo feature, so it runs in the standard build, and it
//!   deterministically asks the shell -- ideal for exercising the shell mechanism itself.
//! - Enabling `FeatureFlag::NativeShellCompletions` at runtime (no force pref) selects the
//!   shipping dispatch, which asks the shell only when Warp's bundled specs come back empty. The
//!   `..._when_a_bundled_spec_answers` / `..._when_no_bundled_spec` pair pins that decision by
//!   observing, via a marker file the shell writes only when its completion actually runs, whether
//!   the shell was consulted.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use warp::features::FeatureFlag;
use warp::integration_testing::input::{input_is_empty, tab_completions_menu_is_open};
use warp::integration_testing::step::new_step_with_default_assertions;
use warp::integration_testing::terminal::util::current_shell_starter_and_version;
use warp::integration_testing::terminal::{
    clear_blocklist_to_remove_bootstrapped_blocks, execute_echo,
    wait_until_bootstrapped_single_pane_for_tab,
};
use warp::integration_testing::view_getters::{
    single_input_suggestions_view_for_tab, single_input_view_for_tab, single_terminal_view_for_tab,
};
use warp::terminal::model::block::TranscriptScope;
use warp::terminal::shell::ShellType;
use warpui_core::async_assert;
use warpui_core::units::Lines;

use super::new_builder;
use crate::Builder;
use crate::util::{ShellRcType, write_rc_files_for_test};

/// The private preference that forces native shell completions on and makes the shell's answer win
/// unconditionally, independent of the `native_shell_completions` cargo feature.
const FORCE_NATIVE_COMPLETIONS_PREF: &str = "ForceNativeShellCompletions";

/// File, relative to the hermetic `$HOME`, that an instrumented shell completion appends to when it
/// actually runs. Its presence is the signal for "the shell was asked to compute completions"; its
/// absence for "the shell was not asked". Both the shell (via `$HOME`) and the assertions (via the
/// `HOME` env var) resolve it to the same path.
const SHELL_ASKED_MARKER_FILE: &str = "native_completions_shell_asked_marker";

/// Absolute path to [`SHELL_ASKED_MARKER_FILE`] within the running test's hermetic home directory.
fn shell_asked_marker_path() -> PathBuf {
    let home = std::env::var("HOME").expect("HOME is set for the duration of the integration test");
    Path::new(&home).join(SHELL_ASKED_MARKER_FILE)
}

/// User defaults that force native shell completions on so the shell is always asked.
fn force_native_completions_defaults() -> HashMap<String, String> {
    HashMap::from([(FORCE_NATIVE_COMPLETIONS_PREF.to_owned(), "true".to_owned())])
}

/// Whether native shell completions can be exercised against the current test shell.
fn shell_supports_native_completions() -> bool {
    let (starter, _version) = current_shell_starter_and_version();
    matches!(
        starter.shell_type(),
        ShellType::Zsh | ShellType::Bash | ShellType::Fish
    )
}

/// Registers a completion for a made-up, spec-less command (`warptool`) in each shell's rc file, so
/// a completions request for it can only be answered by the shell's own machinery. `apple` and
/// `avocado` both match the `a` prefix the tests type; `banana` does not. The two matches share no
/// common prefix beyond `a`, so Tab opens the menu without extending the typed line.
///
/// When `with_marker` is set, the completion also appends to [`SHELL_ASKED_MARKER_FILE`] when it
/// runs, so a test can detect whether the shell was actually asked.
fn write_specless_completion_rc_files(dir: impl AsRef<Path>, with_marker: bool) {
    let bash_marker = if with_marker {
        format!("printf 'x' >> \"$HOME/{SHELL_ASKED_MARKER_FILE}\"\n  ")
    } else {
        String::new()
    };
    write_rc_files_for_test(
        &dir,
        format!(
            "_warptool_complete() {{\n  {bash_marker}\
               local cur=${{COMP_WORDS[COMP_CWORD]}}\n  \
               COMPREPLY=( $(compgen -W \"apple avocado banana\" -- \"$cur\") )\n\
             }}\n\
             complete -F _warptool_complete warptool\n"
        ),
        [ShellRcType::Bash],
    );

    // Warp's bootstrap does not initialize zsh's completion system, so do it here before
    // registering the completion with `compdef`.
    let zsh_marker = if with_marker {
        format!("printf 'x' >> \"$HOME/{SHELL_ASKED_MARKER_FILE}\"; ")
    } else {
        String::new()
    };
    write_rc_files_for_test(
        &dir,
        format!(
            "autoload -Uz compinit\n\
             compinit -u\n\
             _warptool_complete() {{ {zsh_marker}compadd apple avocado banana }}\n\
             compdef _warptool_complete warptool\n"
        ),
        [ShellRcType::Zsh],
    );

    let fish_candidates = if with_marker {
        format!(
            "(printf 'x' >> $HOME/{SHELL_ASKED_MARKER_FILE}; echo apple; echo avocado; echo banana)"
        )
    } else {
        "apple avocado banana".to_owned()
    };
    write_rc_files_for_test(
        &dir,
        format!("complete -c warptool -f -a '{fish_candidates}'\n"),
        [ShellRcType::Fish],
    );
}

/// Overrides the completion for `git` -- a command that has a bundled Warp spec -- with an
/// instrumented one that appends to [`SHELL_ASKED_MARKER_FILE`] and offers a sentinel candidate
/// (`checkzzz`) that the bundled spec would never produce. Under the shipping dispatch the shell is
/// not asked for `git`, so neither the marker nor the sentinel should ever appear.
fn write_spec_command_marker_override_rc_files(dir: impl AsRef<Path>) {
    write_rc_files_for_test(
        &dir,
        format!(
            "_git_override_complete() {{\n  \
               printf 'x' >> \"$HOME/{SHELL_ASKED_MARKER_FILE}\"\n  \
               COMPREPLY=( checkzzz )\n\
             }}\n\
             complete -F _git_override_complete git\n"
        ),
        [ShellRcType::Bash],
    );

    write_rc_files_for_test(
        &dir,
        format!(
            "autoload -Uz compinit\n\
             compinit -u\n\
             _git_override_complete() {{ printf 'x' >> \"$HOME/{SHELL_ASKED_MARKER_FILE}\"; compadd checkzzz }}\n\
             compdef _git_override_complete git\n"
        ),
        [ShellRcType::Zsh],
    );

    write_rc_files_for_test(
        &dir,
        format!(
            "complete -c git -f -a '(printf \"x\" >> $HOME/{SHELL_ASKED_MARKER_FILE}; echo checkzzz)'\n"
        ),
        [ShellRcType::Fish],
    );
}

/// Requesting completions for a spec-less command surfaces the real shell's own matches, filtered
/// to the typed prefix, in the completions menu -- without disturbing the input line or leaving a
/// stray block behind.
pub fn test_native_shell_completions_menu() -> Builder {
    new_builder()
        .set_should_run_test(shell_supports_native_completions)
        .with_user_defaults(force_native_completions_defaults())
        .with_setup(|utils| write_specless_completion_rc_files(utils.test_dir(), false))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(clear_blocklist_to_remove_bootstrapped_blocks())
        .with_step(
            new_step_with_default_assertions("Type 'warptool a' and press tab")
                .with_typed_characters(&["warptool a"])
                .with_keystrokes(&["tab"])
                .set_timeout(Duration::from_secs(30))
                .add_named_assertion(
                    "native completions menu opens",
                    tab_completions_menu_is_open(0, true),
                )
                .add_named_assertion(
                    "menu shows the shell's matching completions and omits the non-match",
                    |app, window_id| {
                        let suggestions = single_input_suggestions_view_for_tab(app, window_id, 0);
                        suggestions.read(app, |view, _ctx| {
                            let has = |needle: &str| {
                                view.items().iter().any(|item| item.text() == needle)
                            };
                            let texts: Vec<_> =
                                view.items().iter().map(|item| item.text()).collect();
                            async_assert!(
                                has("apple") && has("avocado") && !has("banana"),
                                "expected the shell to supply 'apple' and 'avocado' and to filter \
                                 out the non-matching 'banana', got {texts:?}"
                            )
                        })
                    },
                )
                .add_named_assertion(
                    "the input line is left exactly as typed",
                    |app, window_id| {
                        let input = single_input_view_for_tab(app, window_id, 0);
                        input.read(app, |view, ctx| {
                            let buffer = view.buffer_text(ctx);
                            async_assert!(
                                buffer == "warptool a",
                                "expected the input to be left as 'warptool a', got {buffer:?}"
                            )
                        })
                    },
                )
                .add_named_assertion(
                    "the generator command leaves no visible block",
                    |app, window_id| {
                        let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                        terminal_view.read(app, |view, _ctx| {
                            let model = view.model.lock();
                            // The generator runs as an in-band command whose block is hidden
                            // (zero height), so it never surfaces to the user. A visible block
                            // (non-zero height, the signal `assertion.rs` uses for "rendered")
                            // carrying the generator command would be the ghost block this guards
                            // against.
                            let visible_generator_blocks: Vec<String> = model
                                .block_list()
                                .blocks()
                                .iter()
                                .filter(|block| {
                                    block.height(&TranscriptScope::Terminal) != Lines::zero()
                                })
                                .map(|block| block.command_with_secrets_unobfuscated(false))
                                .filter(|command| command.contains("warp_run_generator_command"))
                                .collect();
                            async_assert!(
                                visible_generator_blocks.is_empty(),
                                "the generator command must not appear as a visible block; visible \
                                 generator blocks = {visible_generator_blocks:?}"
                            )
                        })
                    },
                ),
        )
}

/// An ordinary command run right after a native completions request produces its normal output,
/// confirming the generator round-trip left the pty and session in a clean state.
pub fn test_command_runs_cleanly_after_native_shell_completion() -> Builder {
    new_builder()
        .set_should_run_test(shell_supports_native_completions)
        .with_user_defaults(force_native_completions_defaults())
        .with_setup(|utils| write_specless_completion_rc_files(utils.test_dir(), false))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(clear_blocklist_to_remove_bootstrapped_blocks())
        .with_step(
            new_step_with_default_assertions("Request completions for a spec-less command")
                .with_typed_characters(&["warptool a"])
                .with_keystrokes(&["tab"])
                .set_timeout(Duration::from_secs(30))
                .add_named_assertion(
                    "native completions menu opens",
                    tab_completions_menu_is_open(0, true),
                ),
        )
        .with_step(
            new_step_with_default_assertions("Dismiss the menu and clear the input")
                .with_action(|app, window_id, _| {
                    let input = single_input_view_for_tab(app, window_id, 0);
                    input.update(app, |input, ctx| {
                        input.close_overlays(false, ctx);
                        input.clear_buffer_and_reset_undo_stack(ctx);
                    });
                })
                .add_named_assertion("input is cleared", input_is_empty(0))
                .add_named_assertion(
                    "completions menu is closed",
                    tab_completions_menu_is_open(0, false),
                ),
        )
        .with_step(execute_echo(0))
}

/// With the shipping dispatch (feature on, force pref off), a command with no bundled spec falls
/// through to the shell, so its native completions appear and the shell is recorded as having been
/// asked. This is the positive half of the dispatch decision and confirms the marker mechanism
/// fires when the shell really is consulted.
pub fn test_native_shell_completions_used_when_no_bundled_spec() -> Builder {
    FeatureFlag::NativeShellCompletions.set_enabled(true);
    new_builder()
        .set_should_run_test(shell_supports_native_completions)
        .with_setup(|utils| write_specless_completion_rc_files(utils.test_dir(), true))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(clear_blocklist_to_remove_bootstrapped_blocks())
        .with_step(
            new_step_with_default_assertions("Type 'warptool a' and press tab")
                .with_typed_characters(&["warptool a"])
                .with_keystrokes(&["tab"])
                .set_timeout(Duration::from_secs(30))
                .add_named_assertion(
                    "the shell's completions appear because no bundled spec answered",
                    |app, window_id| {
                        let suggestions =
                            single_input_suggestions_view_for_tab(app, window_id, 0);
                        suggestions.read(app, |view, _ctx| {
                            let has = |needle: &str| {
                                view.items().iter().any(|item| item.text() == needle)
                            };
                            let texts: Vec<_> =
                                view.items().iter().map(|item| item.text()).collect();
                            async_assert!(
                                has("apple") && has("avocado"),
                                "expected the shell's completions 'apple' and 'avocado', got {texts:?}"
                            )
                        })
                    },
                ),
        )
        .with_step(
            new_step_with_default_assertions("The shell was asked").add_named_assertion(
                "the marker shows the shell computed completions",
                |_app, _window_id| {
                    async_assert!(
                        shell_asked_marker_path().exists(),
                        "expected the shell to have been asked (marker file should exist)"
                    )
                },
            ),
        )
}

/// With the shipping dispatch (feature on, force pref off), a command with a bundled spec is
/// answered by the spec and the shell is never asked. This is the behavior the feature was
/// specifically requested to have: the shell does not pay for a foreground round trip on a
/// keystroke a bundled spec already answers.
pub fn test_native_shell_completions_skipped_when_a_bundled_spec_answers() -> Builder {
    FeatureFlag::NativeShellCompletions.set_enabled(true);
    new_builder()
        .set_should_run_test(shell_supports_native_completions)
        .with_setup(|utils| write_spec_command_marker_override_rc_files(utils.test_dir()))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(clear_blocklist_to_remove_bootstrapped_blocks())
        .with_step(
            new_step_with_default_assertions("Type 'git ch' and press tab")
                .with_typed_characters(&["git ch"])
                .with_keystrokes(&["tab"])
                .set_timeout(Duration::from_secs(30))
                .add_named_assertion(
                    "the bundled git spec answers and the instrumented shell candidate never appears",
                    |app, window_id| {
                        let suggestions =
                            single_input_suggestions_view_for_tab(app, window_id, 0);
                        suggestions.read(app, |view, _ctx| {
                            let has = |needle: &str| {
                                view.items().iter().any(|item| item.text() == needle)
                            };
                            let texts: Vec<_> =
                                view.items().iter().map(|item| item.text()).collect();
                            async_assert!(
                                has("checkout") && !has("checkzzz"),
                                "expected the bundled spec's 'checkout' and never the shell \
                                 override's 'checkzzz', got {texts:?}"
                            )
                        })
                    },
                ),
        )
        .with_step(
            new_step_with_default_assertions("The shell was not asked").add_named_assertion(
                "no marker: the shell's git completion never ran",
                |_app, _window_id| {
                    async_assert!(
                        !shell_asked_marker_path().exists(),
                        "expected the shell not to have been asked (marker file should not exist)"
                    )
                },
            ),
        )
}
