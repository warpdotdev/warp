//! Integration tests for native shell completions, where the client asks the user's real shell to
//! compute completions for the input line and renders the answer in the completions menu.
//!
//! These boot a real shell, so they cover the client<->shell seam unit tests cannot reach, and run
//! once per shell the shell-integration suite covers -- zsh, bash, fish and PowerShell. Windows and
//! its conpty stay uncovered, since CI skips shell integration there.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use settings::Setting as _;
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
use warp::settings::{NativeShellCompletionsEnabled, WarpCompletionsEnabled};
use warp::terminal::model::block::TranscriptScope;
use warp::terminal::shell::ShellType;
use warpui_core::async_assert;
use warpui_core::units::Lines;

use super::new_builder;
use crate::Builder;
use crate::util::{ShellRcType, write_rc_files_for_test};

/// File, relative to the hermetic `$HOME`, that an instrumented shell completion appends to when it
/// runs, so a test can tell whether the shell was asked to compute completions.
const SHELL_ASKED_MARKER_FILE: &str = "native_completions_shell_asked_marker";

/// Absolute path to [`SHELL_ASKED_MARKER_FILE`] within the running test's hermetic home directory.
fn shell_asked_marker_path() -> PathBuf {
    let home = std::env::var("HOME").expect("HOME is set for the duration of the integration test");
    Path::new(&home).join(SHELL_ASKED_MARKER_FILE)
}

/// Enables the `NativeShellCompletions` gate for the whole app run. Process-global, which is safe
/// because each integration test runs in its own process, and unlike a scoped guard it is still in
/// effect once the app starts.
fn enable_native_shell_completions_feature() {
    FeatureFlag::NativeShellCompletions.set_enabled(true);
}

/// Warp completions off, native on, resolving to `CompletionSources::NativeOnly`: the shell is
/// asked unconditionally, with no bundled specs and no file-path fallback.
fn native_only_completion_defaults() -> HashMap<String, String> {
    HashMap::from([
        (
            WarpCompletionsEnabled::storage_key().to_string(),
            "false".to_string(),
        ),
        (
            NativeShellCompletionsEnabled::storage_key().to_string(),
            "true".to_string(),
        ),
    ])
}

/// Both toggles on, resolving to `CompletionSources::WarpThenNative`: the bundled specs answer
/// first and the shell is asked only when they come back empty.
fn specs_first_completion_defaults() -> HashMap<String, String> {
    HashMap::from([
        (
            WarpCompletionsEnabled::storage_key().to_string(),
            "true".to_string(),
        ),
        (
            NativeShellCompletionsEnabled::storage_key().to_string(),
            "true".to_string(),
        ),
    ])
}

/// Whether native completions can be exercised against the shell this run is testing.
fn shell_supports_native_completions() -> bool {
    let (starter, _version) = current_shell_starter_and_version();
    matches!(
        starter.shell_type(),
        ShellType::Zsh | ShellType::Bash | ShellType::Fish | ShellType::PowerShell
    )
}

/// Registers a completion for a made-up, spec-less command (`warptool`), so a request for it can
/// only be answered by the shell's own machinery. `apple` and `avocado` match the typed `a` prefix
/// and `banana` does not; two matches sharing no prefix beyond `a` mean Tab opens the menu instead
/// of extending the line. With `with_marker`, the completion also writes the marker file.
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

    // Warp's bootstrap does not initialize zsh's completion system, so `compdef` needs it here.
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

    // pwsh is launched `-NoProfile`, but Warp's bootstrap dot-sources the user profile afterward,
    // so a profile written here is still sourced.
    let pwsh_marker = if with_marker {
        format!("  [System.IO.File]::AppendAllText(\"$env:HOME/{SHELL_ASKED_MARKER_FILE}\", 'x')\n")
    } else {
        String::new()
    };
    write_rc_files_for_test(
        &dir,
        format!(
            "Register-ArgumentCompleter -Native -CommandName warptool -ScriptBlock {{\n  \
               param($wordToComplete, $commandAst, $cursorPosition)\n\
             {pwsh_marker}  \
               @('apple','avocado','banana') | Where-Object {{ $_ -like \"$wordToComplete*\" }} | ForEach-Object {{\n    \
                 [System.Management.Automation.CompletionResult]::new($_, $_, 'ParameterValue', $_)\n  \
               }}\n\
             }}\n"
        ),
        [ShellRcType::PowerShell],
    );
}

/// Overrides `git`'s completion -- a command with a bundled Warp spec -- with an instrumented one
/// offering sentinels the spec would never produce, so a test can tell whether the shell was asked
/// for a spec-backed command. Two sentinels rather than one because a lone match is inserted
/// straight into the buffer instead of opening the menu.
fn write_spec_command_marker_override_rc_files(dir: impl AsRef<Path>) {
    write_rc_files_for_test(
        &dir,
        format!(
            "_git_override_complete() {{\n  \
               printf 'x' >> \"$HOME/{SHELL_ASKED_MARKER_FILE}\"\n  \
               COMPREPLY=( checkzzz checkyyy )\n\
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
             _git_override_complete() {{ printf 'x' >> \"$HOME/{SHELL_ASKED_MARKER_FILE}\"; compadd checkzzz checkyyy }}\n\
             compdef _git_override_complete git\n"
        ),
        [ShellRcType::Zsh],
    );

    write_rc_files_for_test(
        &dir,
        format!(
            "complete -c git -f -a '(printf \"x\" >> $HOME/{SHELL_ASKED_MARKER_FILE}; echo checkzzz; echo checkyyy)'\n"
        ),
        [ShellRcType::Fish],
    );

    write_rc_files_for_test(
        &dir,
        format!(
            "Register-ArgumentCompleter -Native -CommandName git -ScriptBlock {{\n  \
               param($wordToComplete, $commandAst, $cursorPosition)\n  \
               [System.IO.File]::AppendAllText(\"$env:HOME/{SHELL_ASKED_MARKER_FILE}\", 'x')\n  \
               @('checkzzz','checkyyy') | ForEach-Object {{ [System.Management.Automation.CompletionResult]::new($_, $_, 'ParameterValue', $_) }}\n\
             }}\n"
        ),
        [ShellRcType::PowerShell],
    );
}

/// Asserts no user-visible block holds the generator command. Its own block is hidden (zero
/// height), so a non-zero-height block carrying it is the ghost block this guards against. The name
/// is normalized to match both `warp_run_generator_command*` and `Warp-Run-GeneratorCommand*`.
fn assert_no_visible_generator_block()
-> impl Fn(&mut warpui_core::App, warpui_core::WindowId) -> warpui_core::integration::AssertionOutcome
{
    move |app, window_id| {
        let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
        terminal_view.read(app, |view, _ctx| {
            let visible_generator_blocks: Vec<String> = view
                .model
                .lock()
                .block_list()
                .blocks()
                .iter()
                .filter(|block| block.height(&TranscriptScope::Terminal) != Lines::zero())
                .map(|block| block.command_with_secrets_unobfuscated(false))
                .filter(|command| {
                    command
                        .to_ascii_lowercase()
                        .replace(['_', '-'], "")
                        .contains("warprungeneratorcommand")
                })
                .collect();
            async_assert!(
                visible_generator_blocks.is_empty(),
                "the generator command must not appear as a visible block; visible generator \
                 blocks = {visible_generator_blocks:?}"
            )
        })
    }
}

/// The shell's own matches reach the menu, filtered to the typed prefix, without disturbing the
/// input line or leaving a stray block.
pub fn test_native_shell_completions_menu() -> Builder {
    enable_native_shell_completions_feature();
    new_builder()
        .set_should_run_test(shell_supports_native_completions)
        .with_user_defaults(native_only_completion_defaults())
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
                    assert_no_visible_generator_block(),
                ),
        )
}

/// Confirms the generator round trip leaves the pty and session clean.
pub fn test_command_runs_cleanly_after_native_shell_completion() -> Builder {
    enable_native_shell_completions_feature();
    new_builder()
        .set_should_run_test(shell_supports_native_completions)
        .with_user_defaults(native_only_completion_defaults())
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

/// The positive half of the dispatch decision: with no bundled spec to answer, the request falls
/// through to the shell.
pub fn test_native_shell_completions_used_when_no_bundled_spec() -> Builder {
    enable_native_shell_completions_feature();
    new_builder()
        .set_should_run_test(shell_supports_native_completions)
        .with_user_defaults(specs_first_completion_defaults())
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

/// A bundled spec answers without a foreground round trip to the shell. The absence signals only
/// mean "not asked" because `test_native_shell_completions_reach_a_spec_command_native_only` proves
/// the same override does fire when git is dispatched.
pub fn test_native_shell_completions_skipped_when_a_bundled_spec_answers() -> Builder {
    enable_native_shell_completions_feature();
    new_builder()
        .set_should_run_test(shell_supports_native_completions)
        .with_user_defaults(specs_first_completion_defaults())
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

/// Reachability control for `test_native_shell_completions_skipped_when_a_bundled_spec_answers`:
/// under native-only the same override does fire for `git`, so that test's silence means the shell
/// was not asked rather than that the override never registered.
pub fn test_native_shell_completions_reach_a_spec_command_native_only() -> Builder {
    enable_native_shell_completions_feature();
    new_builder()
        .set_should_run_test(shell_supports_native_completions)
        .with_user_defaults(native_only_completion_defaults())
        .with_setup(|utils| write_spec_command_marker_override_rc_files(utils.test_dir()))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(clear_blocklist_to_remove_bootstrapped_blocks())
        .with_step(
            new_step_with_default_assertions("Type 'git ch' and press tab")
                .with_typed_characters(&["git ch"])
                .with_keystrokes(&["tab"])
                .set_timeout(Duration::from_secs(30))
                .add_named_assertion(
                    "the marker shows the git override ran",
                    |_app, _window_id| {
                        async_assert!(
                            shell_asked_marker_path().exists(),
                            "expected the shell to have been asked (marker file should exist)"
                        )
                    },
                )
                .add_named_assertion(
                    "the instrumented git override's sentinel appears because the shell is asked",
                    |app, window_id| {
                        let suggestions = single_input_suggestions_view_for_tab(app, window_id, 0);
                        suggestions.read(app, |view, _ctx| {
                            let texts: Vec<_> =
                                view.items().iter().map(|item| item.text()).collect();
                            async_assert!(
                                view.items().iter().any(|item| item.text() == "checkzzz"),
                                "expected the shell override's 'checkzzz' under native-only, \
                                 got {texts:?}"
                            )
                        })
                    },
                ),
        )
}

/// `(Get-Date).` surfaces real .NET members computed by pwsh's own engine, exercising the
/// zero-length replacement span the shell reports at a member-access position.
pub fn test_native_shell_completions_powershell_member_access() -> Builder {
    enable_native_shell_completions_feature();
    new_builder()
        .set_should_run_test(|| {
            let (starter, _version) = current_shell_starter_and_version();
            matches!(starter.shell_type(), ShellType::PowerShell)
        })
        .with_user_defaults(native_only_completion_defaults())
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(clear_blocklist_to_remove_bootstrapped_blocks())
        .with_step(
            new_step_with_default_assertions("Type '(Get-Date).' and press tab")
                .with_typed_characters(&["(Get-Date)."])
                .with_keystrokes(&["tab"])
                .set_timeout(Duration::from_secs(30))
                .add_named_assertion(
                    "the menu shows a real DateTime member computed by the shell",
                    |app, window_id| {
                        let suggestions = single_input_suggestions_view_for_tab(app, window_id, 0);
                        suggestions.read(app, |view, _ctx| {
                            async_assert!(
                                view.items().iter().any(|item| item.text() == "Year"),
                                "expected a real DateTime member like 'Year' from the shell, got {:?}",
                                view.items().iter().map(|item| item.text()).collect::<Vec<_>>()
                            )
                        })
                    },
                ),
        )
}
