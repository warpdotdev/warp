//! Integration tests for native shell completions, where the client asks the user's real shell to
//! compute completions for the current input line via an in-band generator command and renders the
//! shell's answer in the completions menu (see `native_shell_completions.rs` on the Rust side and
//! the `warp_run_generator_command*` / `Warp-Run-GeneratorCommand*` functions in the bootstrap
//! scripts).
//!
//! These boot a real shell and drive it end-to-end, covering the client<->shell seam that unit
//! tests cannot reach. They run against every shell CI exercises the shell integration suite
//! against -- zsh, bash, fish, and PowerShell -- which on the unix runners is all four, under
//! xvfb. Each shell registers the test's completions through its own rc file / profile; Warp's
//! bootstrap sources all four, including PowerShell (launched `-NoProfile`, then the bootstrap
//! dot-sources the user profile itself). What stays uncovered is Windows and its conpty, where CI
//! skips the shell integration tests entirely -- and that is the residual that matters, because
//! the PowerShell path is reintroducing command execution and conpty is where that risk lives.
//!
//! Native completions are Tab/keybinding-triggered only; there is deliberately no as-you-type
//! coverage, because as-you-type does not ask the shell. Two further behaviors -- that the
//! generator command stays out of the shell's history and out of the tab title -- are deliberately
//! left to unit tests: at this layer they are only observable through a flaky signal (reading the
//! histfile after a shell exit) or a transient one (a title flash), so a test here would assert
//! weakly while looking like coverage.
//!
//! The `NativeShellCompletions` feature flag is the outer gate, and which channels enable it is not
//! this suite's business, so every test sets it at runtime rather than inheriting a default. Which
//! sources a completion draws on is then chosen by two Appearance -> Input toggles, "Warp
//! completions" and "Native shell completions", which resolve to four states (see
//! `CompletionSources`): both on is specs-first (the shell is asked only when the
//! bundled specs are empty); Warp completions off with native on is native-only (the shell wins
//! outright, with no specs and no file-path fallback); both off disables shell completions. The
//! tests set these toggles explicitly for the behavior they exercise:
//! - The menu, clean-command, and reachability tests use native-only, so the shell is asked
//!   unconditionally -- the honest successor to the removed `ForceNativeShellCompletions` pref.
//! - The `..._when_a_bundled_spec_answers` / `..._when_no_bundled_spec` pair uses specs-first to
//!   pin the dispatch decision, observing via a marker file (written only when the shell's own
//!   completion actually runs) whether the shell was consulted;
//!   `..._reach_a_spec_command_native_only` is the reachability control that keeps the negative
//!   test's silence meaningful.

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
/// actually runs. Its presence is the signal for "the shell was asked to compute completions"; its
/// absence for "the shell was not asked". Both the shell (via `$HOME`) and the assertions (via the
/// `HOME` env var) resolve it to the same path.
const SHELL_ASKED_MARKER_FILE: &str = "native_completions_shell_asked_marker";

/// Absolute path to [`SHELL_ASKED_MARKER_FILE`] within the running test's hermetic home directory.
fn shell_asked_marker_path() -> PathBuf {
    let home = std::env::var("HOME").expect("HOME is set for the duration of the integration test");
    Path::new(&home).join(SHELL_ASKED_MARKER_FILE)
}

/// Enables the `NativeShellCompletions` feature flag at runtime. The flag is the outer gate: with
/// it off the shell is never asked whatever the toggles say, so every test that drives native
/// completions must enable it. Which completion source is then used is chosen by the toggle values
/// passed to `with_user_defaults` (see `native_only_completion_defaults` /
/// `specs_first_completion_defaults`).
///
/// This is a process-global side effect, but each integration test runs in its own process, so it
/// cannot leak across tests. It matches how other integration tests enable a flag (e.g.
/// `settings_private`), and unlike a unit-test scoped guard it stays in effect through the app run
/// rather than being dropped before the app starts.
fn enable_native_shell_completions_feature() {
    FeatureFlag::NativeShellCompletions.set_enabled(true);
}

/// User defaults selecting the native-only completion source: Warp completions off, the shell's
/// native completions on. With the feature flag enabled this resolves to
/// `CompletionSources::NativeOnly` -- the shell's answer wins outright, and bundled specs and the
/// file-path fallback are not consulted. This is the honest successor to the removed
/// `ForceNativeShellCompletions` pref.
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

/// User defaults selecting the specs-first completion source: both toggles on. With the feature
/// flag enabled this resolves to `CompletionSources::WarpThenNative` -- the bundled specs answer
/// first and the shell is asked only when they come back empty. Set explicitly (rather than relying
/// on the defaults) so the dispatch tests state the source they exercise.
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

/// Whether native shell completions can be exercised against the current test shell. CI runs the
/// shell integration suite against all four on the unix runners.
fn shell_supports_native_completions() -> bool {
    let (starter, _version) = current_shell_starter_and_version();
    matches!(
        starter.shell_type(),
        ShellType::Zsh | ShellType::Bash | ShellType::Fish | ShellType::PowerShell
    )
}

/// Registers a completion for a made-up, spec-less command (`warptool`) in each shell's rc file /
/// profile, so a completions request for it can only be answered by the shell's own machinery.
/// `apple` and `avocado` both match the `a` prefix the tests type; `banana` does not. The two
/// matches share no common prefix beyond `a`, so Tab opens the menu without extending the typed
/// line.
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

    // PowerShell: Warp launches pwsh with `-NoProfile` but its bootstrap dot-sources the user
    // profile afterward, so a profile written here is sourced. A `-Native` argument completer
    // fires for an arbitrary command line through the same completion engine the native-completions
    // handler drives.
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

/// Overrides the completion for `git` -- a command that has a bundled Warp spec -- with an
/// instrumented one that appends to [`SHELL_ASKED_MARKER_FILE`] and offers sentinel candidates
/// (`checkzzz`, `checkyyy`) that the bundled spec would never produce. Under the shipping dispatch
/// the shell is not asked for `git`, so neither the marker nor a sentinel should appear; under
/// native-only the shell is asked, so both must (see the reachability-control test).
///
/// Two sentinels are offered rather than one so a menu reliably opens: a lone match is inserted
/// straight into the buffer instead of being shown in the menu (see `test_function_completions`).
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

/// Asserts that no user-visible block holds the native-completions generator command.
///
/// The generator runs as an in-band command whose block is hidden (zero height), so it never
/// surfaces to the user; a visible block (non-zero height, the signal `assertion.rs` uses for
/// "rendered") carrying the generator command would be the ghost block this guards against. The
/// name is normalized before matching so it catches both the POSIX shells'
/// `warp_run_generator_command*` and PowerShell's `Warp-Run-GeneratorCommand*`.
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

/// Requesting completions for a spec-less command surfaces the real shell's own matches, filtered
/// to the typed prefix, in the completions menu -- without disturbing the input line or leaving a
/// stray block behind.
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

/// An ordinary command run right after a native completions request produces its normal output,
/// confirming the generator round-trip left the pty and session in a clean state.
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

/// With specs-first (both toggles on), a command with no bundled spec falls through to the shell,
/// so its native completions appear and the shell is recorded as having been asked. This is the
/// positive half of the dispatch decision and confirms the marker mechanism fires when the shell
/// really is consulted.
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

/// With specs-first (both toggles on), a command with a bundled spec is answered by the spec and
/// the shell is never asked. This is the behavior the feature was specifically requested to have:
/// the shell does not pay for a foreground round trip on a keystroke a bundled spec already answers.
///
/// The absence signals here (no `checkzzz`, no marker) only mean "the shell was not asked" because
/// `test_native_shell_completions_reach_a_spec_command_native_only` proves the same override *does*
/// fire when git is dispatched to the shell -- so this silence is not a silently-failed
/// registration.
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

/// Reachability control for `test_native_shell_completions_skipped_when_a_bundled_spec_answers`.
///
/// Under native-only, even a spec-backed command like `git` is dispatched to the shell, so the
/// same instrumented override must fire: its sentinel appears and the marker is written. This
/// demonstrates the override is reachable, which is what makes the negative test's silence mean
/// "the shell was not asked" rather than "the override never registered" -- git is exactly the
/// command most likely to have a competing completion already loaded, so this control is not
/// hypothetical.
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

/// PowerShell member-access completion: typing `(Get-Date).` and pressing Tab surfaces the real
/// .NET members of the returned value (e.g. `Year`) computed by the shell's own engine. This is
/// PowerShell-specific syntax, so the test is gated to pwsh. It exercises the zero-length
/// replacement span the shell reports at a member-access position, and defends the newly-correct
/// behavior where a punctuation-heavy query returns real members rather than fuzzy-matched junk.
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
