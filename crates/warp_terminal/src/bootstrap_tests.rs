use super::*;

struct TestAssetProvider;

impl AssetProvider for TestAssetProvider {
    fn get(&self, path: &str) -> anyhow::Result<Cow<'_, [u8]>> {
        let content = match path {
            "bundled/bootstrap/bash.sh" => "#include hello_world",
            "bundled/bootstrap/fish.sh" => "# this is a comment\nthis_is_a_command",
            "bundled/bootstrap/zsh.sh" => {
                "asdf\n#include whitespace\n    prepended whitespace\n\n\n"
            }
            "bundled/bootstrap/pwsh.ps1" => {
                r#"# This is a comment
                Write-Output 'Testing some output'
                function test1 {
                    [Diagnostics.CodeAnalysis.SuppressMessageAttribute('PSAvoidUsingInvokeExpression', '', Justification = 'We actually need it')]
                    param([string]$command)
                    Invoke-Expression $command
                }"#
            }
            "hello_world" => "hello world!",
            "whitespace" => "no whitespace\n\n\n yes whitespace!",
            _ => anyhow::bail!("path not found in assets"),
        };
        Ok(Cow::Borrowed(content.as_bytes()))
    }
}

#[test]
fn test_include_directive() {
    assert_eq!(
        decode_script(&script_for_shell(ShellType::Bash, &TestAssetProvider)),
        "hello world!\n"
    );
}

#[test]
fn test_trims_comments() {
    assert_eq!(
        decode_script(&script_for_shell(ShellType::Fish, &TestAssetProvider)),
        "this_is_a_command\n"
    );
}

#[test]
fn test_trims_whitespace() {
    assert_eq!(
        decode_script(&script_for_shell(ShellType::Zsh, &TestAssetProvider)),
        "asdf\nno whitespace\n yes whitespace!\n prepended whitespace\n"
    );
}

#[test]
fn test_trims_powershell_specifics() {
    assert_eq!(
        decode_script(&script_for_shell(ShellType::PowerShell, &TestAssetProvider)),
        " Write-Output 'Testing some output'\n function test1 {\n param([string]$command)\n Invoke-Expression $command\n }\n"
    );
}

fn decode_script(bytes: &[u8]) -> &str {
    std::str::from_utf8(bytes).expect("should not fail to decode")
}

fn fish_history_wrapper_installer() -> &'static str {
    const FISH_SH: &str = include_str!("../../../app/assets/bundled/bootstrap/fish.sh");
    let start_marker = "if functions -q fish_should_add_to_history\n  and not functions fish_should_add_to_history";
    let end_marker = "  warp_original_fish_should_add_to_history $argv\nend";
    let start = FISH_SH
        .find(start_marker)
        .expect("fish history wrapper installer start should exist");
    let end = FISH_SH[start..]
        .find(end_marker)
        .expect("fish history wrapper installer end should exist");
    &FISH_SH[start..start + end + end_marker.len()]
}

fn run_fish(script: &str) -> Option<String> {
    let output = match command::blocking::Command::new("fish")
        .args(["--no-config", "-c", script])
        .output()
    {
        Ok(output) => output,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
        Err(error) => panic!("failed to run fish: {error}"),
    };
    assert!(
        output.status.success(),
        "fish exited with {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[test]
fn test_fish_history_wrapper_accepts_normal_commands_across_resourcing() {
    let installer = fish_history_wrapper_installer();
    let script = format!(
        r#"
{installer}
{installer}
fish_should_add_to_history "echo normal"
echo "normal:$status"
fish_should_add_to_history "warp_run_external_ctrl_r_widget token"
echo "helper:$status"
# The real invocation (see trigger_external_ctrl_r_history_search) is prefixed with a leading
# space, so atuin's own "ignorespace" exclusion also catches it; the wrapper must still reject
# this exact shape too.
fish_should_add_to_history " warp_run_external_ctrl_r_widget token"
echo "helper_leading_space:$status"
"#
    );
    let Some(stdout) = run_fish(&script) else {
        return;
    };
    assert!(stdout.contains("normal:0"), "{stdout}");
    assert!(stdout.contains("helper:1"), "{stdout}");
    assert!(stdout.contains("helper_leading_space:1"), "{stdout}");
}

#[test]
fn test_fish_history_wrapper_preserves_user_hook_across_resourcing() {
    let installer = fish_history_wrapper_installer();
    let script = format!(
        r#"
function fish_should_add_to_history
  string match --quiet -- "user_excluded*" $argv[1]; and return 1
  return 0
end
{installer}
{installer}
fish_should_add_to_history "echo normal"
echo "normal:$status"
fish_should_add_to_history "warp_run_external_ctrl_r_widget token"
echo "helper:$status"
fish_should_add_to_history "user_excluded"
echo "user:$status"
"#
    );
    let Some(stdout) = run_fish(&script) else {
        return;
    };
    assert!(stdout.contains("normal:0"), "{stdout}");
    assert!(stdout.contains("helper:1"), "{stdout}");
    assert!(stdout.contains("user:1"), "{stdout}");
}

/// Regression test for a user/plugin hook defined *between* two sourcings of this bootstrap
/// script (e.g. a plugin loaded after Warp's shell integration, followed by a shell reload or
/// nested fish subshell): the second sourcing must capture that hook rather than discarding it
/// in favor of whatever backup (or accept-everything default) an earlier sourcing installed.
#[test]
fn test_fish_history_wrapper_captures_hook_installed_between_resourcing() {
    let installer = fish_history_wrapper_installer();
    let script = format!(
        r#"
{installer}
function fish_should_add_to_history
  string match --quiet -- "user_excluded*" $argv[1]; and return 1
  return 0
end
{installer}
fish_should_add_to_history "echo normal"
echo "normal:$status"
fish_should_add_to_history "warp_run_external_ctrl_r_widget token"
echo "helper:$status"
fish_should_add_to_history "user_excluded"
echo "user:$status"
"#
    );
    let Some(stdout) = run_fish(&script) else {
        return;
    };
    assert!(stdout.contains("normal:0"), "{stdout}");
    assert!(stdout.contains("helper:1"), "{stdout}");
    assert!(stdout.contains("user:1"), "{stdout}");
}

fn bash_ctrl_t_detection_snippet() -> &'static str {
    const BASH_SH: &str = include_str!("../../../app/assets/bundled/bootstrap/bash_body.sh");
    let start_marker = "      _WARP_EXTERNAL_CTRL_T_WIDGET=\"\"\n      warp_ctrl_t_binding=";
    let end_marker = "          fi\n          ;;\n      esac";
    let start = BASH_SH
        .find(start_marker)
        .expect("bash ctrl-t detection snippet start should exist");
    let end = BASH_SH[start..]
        .find(end_marker)
        .expect("bash ctrl-t detection snippet end should exist");
    &BASH_SH[start..start + end + end_marker.len()]
}

fn run_bash(script: &str) -> Option<String> {
    let output = match command::blocking::Command::new("bash")
        .args(["--noprofile", "--norc", "-c", script])
        .output()
    {
        Ok(output) => output,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
        Err(error) => panic!("failed to run bash: {error}"),
    };
    assert!(
        output.status.success(),
        "bash exited with {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn bash_ctrl_t_bind_x_extraction() -> &'static str {
    let snippet = bash_ctrl_t_detection_snippet();
    let start = snippet
        .find("bind -X 2>/dev/null | command -p sed")
        .expect("ctrl-t detection should pipe bind -X through sed");
    let rest = &snippet[start..];
    let quote = rest
        .rfind('\'')
        .expect("ctrl-t bind -X sed program should be single-quoted");
    &rest[..=quote]
}

fn bash_ctrl_t_bind_x_sed_program() -> &'static str {
    let extraction = bash_ctrl_t_bind_x_extraction();
    let prefix = "sed -n '";
    let start = extraction
        .find(prefix)
        .expect("extraction should invoke sed -n")
        + prefix.len();
    let end = extraction[start..]
        .find("'")
        .expect("sed program should be single-quoted");
    &extraction[start..start + end]
}

/// Whether this environment can read a `bind -x` binding back out through the pipeline in
/// `bash_body.sh`, run the way the tests below run it. `None` if bash isn't installed at all,
/// mirroring `run_bash`'s "shell missing" skip convention.
///
/// Skip only when `bind -X` does not list the probe at all: `bind -X` arrived in bash 4.3, and a
/// non-interactive shell need not have line editing enabled, so the binding is never listable.
/// If `bind -X` lists the probe but extraction is empty, panic -- that is a broken product
/// extractor, not a reason to skip.
///
/// Deliberately probes with a sentinel widget name rather than `fzf-file-widget`, so it tests the
/// capability without also asserting the `case` match the tests below exist to check -- otherwise
/// the gate would subsume the assertion and the tests could never fail.
fn bash_can_extract_ctrl_t_binding() -> Option<bool> {
    let extraction = bash_ctrl_t_bind_x_extraction();
    let script = format!(
        r#"bind -x '"\C-t": warp_bind_x_probe' 2>/dev/null; printf 'EXTRACTED:%s\n' "$({extraction})"; bind -X 2>/dev/null"#
    );

    let output = match command::blocking::Command::new("bash")
        .args(["--noprofile", "--norc", "-c", &script])
        .output()
    {
        Ok(output) => output,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
        Err(error) => panic!("failed to run bash: {error}"),
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut lines = stdout.lines();
    let extracted = lines
        .next()
        .unwrap_or("")
        .strip_prefix("EXTRACTED:")
        .unwrap_or("")
        .trim();
    let raw: String = lines.collect();
    if extracted == "warp_bind_x_probe" {
        return Some(true);
    }
    if raw.contains("warp_bind_x_probe") {
        panic!(
            "bind -X listed warp_bind_x_probe but bash_body.sh extraction returned {extracted:?}; raw bind -X: {raw:?}"
        );
    }
    Some(false)
}

/// The ctrl-t `bind -X` sed from `bash_body.sh` must accept both bash 5.2 colon and bash 5.3
/// space layouts. Does not need `bind -X` or an interactive shell.
#[test]
fn test_bash_bind_x_extraction_accepts_colon_and_space_formats() {
    let sed = bash_ctrl_t_bind_x_sed_program();
    let script = format!(
        r#"colon=$(printf '%s\n' '"\C-t": "fzf-file-widget"' | command -p sed -n '{sed}'); space=$(printf '%s\n' '"\C-t" "fzf-file-widget"' | command -p sed -n '{sed}'); printf 'colon=[%s] space=[%s]\n' "$colon" "$space""#
    );

    let Some(stdout) = run_bash(&script) else {
        return;
    };
    assert!(
        stdout.contains("colon=[fzf-file-widget]"),
        "colon layout should extract; got {stdout:?}"
    );
    assert!(
        stdout.contains("space=[fzf-file-widget]"),
        "space layout should extract; got {stdout:?}"
    );
    assert!(
        sed.contains("[ :]"),
        "ctrl-t bind -X sed must accept colon or space; got {sed:?}"
    );
}

/// Regression test for the ctrl-t equivalent of bash's `declare -F __atuin_history` guard on the
/// ctrl-r path: detection must decline (no tag, no interception) when the picker function
/// `warp_run_external_ctrl_t_widget` calls -- `__fzf_select__` -- isn't actually defined, even
/// though `bind -X` reports the wrapper name ("fzf-file-widget") that detection matches against.
/// Without this guard, an fzf version that renamed its picker function would have ctrl-t tagged
/// and intercepted with nothing to invoke, swallowing the key instead of leaving it alone.
#[test]
fn test_bash_ctrl_t_detection_declines_when_picker_function_is_absent() {
    if bash_can_extract_ctrl_t_binding() == Some(false) {
        return;
    }
    let detection = bash_ctrl_t_detection_snippet();
    let script = format!(
        r#"
WARP_IN_MSYS2=false
shell_plugins=()
bind -x '"\C-t": fzf-file-widget'
{detection}
printf 'widget=[%s] plugins=[%s]\n' "$_WARP_EXTERNAL_CTRL_T_WIDGET" "${{shell_plugins[*]}}"
"#
    );
    let Some(stdout) = run_bash(&script) else {
        return;
    };
    assert!(stdout.contains("widget=[]"), "{stdout}");
    assert!(!stdout.contains("external_ctrl_t_file"), "{stdout}");
}

#[test]
fn test_bash_ctrl_t_detection_tags_when_picker_function_is_present() {
    if bash_can_extract_ctrl_t_binding() == Some(false) {
        return;
    }
    let detection = bash_ctrl_t_detection_snippet();
    let script = format!(
        r#"
WARP_IN_MSYS2=false
shell_plugins=()
bind -x '"\C-t": fzf-file-widget'
__fzf_select__() {{ :; }}
{detection}
printf 'widget=[%s] plugins=[%s]\n' "$_WARP_EXTERNAL_CTRL_T_WIDGET" "${{shell_plugins[*]}}"
"#
    );
    let Some(stdout) = run_bash(&script) else {
        return;
    };
    assert!(stdout.contains("widget=[fzf-file-widget]"), "{stdout}");
    assert!(stdout.contains("external_ctrl_t_file"), "{stdout}");
}

fn fish_ctrl_r_widget_runner_fn() -> &'static str {
    const FISH_SH: &str = include_str!("../../../app/assets/bundled/bootstrap/fish.sh");
    let start_marker = "function warp_run_external_ctrl_r_widget\n";
    let start = FISH_SH
        .find(start_marker)
        .expect("fish ctrl-r widget runner function start should exist");
    let end_marker = "\nend\n";
    let end = FISH_SH[start..]
        .find(end_marker)
        .expect("fish ctrl-r widget runner function end should exist");
    &FISH_SH[start..start + end + end_marker.len()]
}

/// Regression test for `warp_run_external_ctrl_r_widget`'s fzf case: it used to hand-build
/// `FZF_DEFAULT_OPTS` with flags (`--wrap-sign`, `--highlight-line`, `--accept-nth`,
/// `--with-shell`) and call a helper function (`__fzf_defaults`) that don't exist on every fzf
/// shell integration -- confirmed to fail outright with "Unknown command: __fzf_defaults" against
/// a real, still-commonly-packaged fzf 0.44.1 install, with the picker that did appear (fzf
/// falling through to a plain invocation once that command failed) reading raw, unformatted
/// history text as its input. It now delegates entirely to the user's own `fzf-history-widget`
/// instead, so this stubs that widget and the interactive-only `commandline` builtin they both
/// call, to verify the wrapper reports whatever the widget leaves on the commandline without
/// depending on any fzf-version-specific option or helper function existing at all -- the kind of
/// test that would have caught the original defect, rather than merely asserting one flag absent.
fn fish_ctrl_r_widget_test_script(runner: &str, widget_body: &str) -> String {
    format!(
        r#"
function warp_escape_json
  string join \n $argv
end
function warp_send_json_message
  echo "$argv"
end
set -g _test_commandline_value ''
function commandline
  echo "$_test_commandline_value"
end
function fzf-history-widget
  {widget_body}
end
set -g _WARP_EXTERNAL_CTRL_R_WIDGET fzf-history-widget
set -g WARP_SESSION_ID 12345
{runner}
warp_run_external_ctrl_r_widget test-token
"#
    )
}

#[test]
fn test_fish_ctrl_r_widget_reports_fzf_history_widget_selection() {
    let runner = fish_ctrl_r_widget_runner_fn();
    let script = fish_ctrl_r_widget_test_script(
        runner,
        "set -g _test_commandline_value 'echo selected_from_widget'",
    );
    let Some(stdout) = run_fish(&script) else {
        return;
    };
    assert!(
        stdout.contains(r#""buffer": "echo selected_from_widget""#),
        "{stdout}"
    );
}

/// `fzf-history-widget` only calls `commandline` on a successful selection, leaving it untouched
/// on cancel -- the wrapper must report that untouched (here: still-empty) state as an empty
/// buffer, matching the existing "nothing selected" convention shared with the plain-path bash/
/// zsh widgets.
#[test]
fn test_fish_ctrl_r_widget_reports_empty_buffer_when_widget_leaves_commandline_untouched() {
    let runner = fish_ctrl_r_widget_runner_fn();
    let script = fish_ctrl_r_widget_test_script(runner, "# cancelled: commandline left as-is");
    let Some(stdout) = run_fish(&script) else {
        return;
    };
    assert!(stdout.contains(r#""buffer": """#), "{stdout}");
}

fn fish_warp_escape_json_fn() -> &'static str {
    const FISH_SH: &str = include_str!("../../../app/assets/bundled/bootstrap/fish.sh");
    let start_marker = "function warp_escape_json\n";
    let start = FISH_SH
        .find(start_marker)
        .expect("fish warp_escape_json function start should exist");
    let end_marker = "\nend\n";
    let end = FISH_SH[start..]
        .find(end_marker)
        .expect("fish warp_escape_json function end should exist");
    &FISH_SH[start..start + end + end_marker.len()]
}

/// Regression test for `set result (commandline | string collect)` above: without `string
/// collect`, a multi-line selection makes that `set`'s own command substitution split it into a
/// list by newline, and the real `warp_escape_json` (used here instead of the plain-join stub the
/// other tests in this section use, since the defect is specifically in how it escapes -- or
/// fails to escape -- what it's given) then quotes that list back down to a single argument by
/// joining with a space instead of preserving the newline as JSON's `\n` escape.
#[test]
fn test_fish_ctrl_r_widget_reports_multiline_selection_with_embedded_newline() {
    let runner = fish_ctrl_r_widget_runner_fn();
    let escape_json = fish_warp_escape_json_fn();
    let script = format!(
        r#"
{escape_json}
function warp_send_json_message
  echo "$argv"
end
set -g _test_commandline_value ''
function commandline
  echo "$_test_commandline_value"
end
function fzf-history-widget
  set -g _test_commandline_value (printf 'echo one\necho two' | string collect)
end
set -g _WARP_EXTERNAL_CTRL_R_WIDGET fzf-history-widget
set -g WARP_SESSION_ID 12345
{runner}
warp_run_external_ctrl_r_widget test-token
"#
    );
    let Some(stdout) = run_fish(&script) else {
        return;
    };
    assert!(
        stdout.contains(r#""buffer": "echo one\necho two""#),
        "{stdout}"
    );
}

fn fish_ctrl_t_widget_query_fn() -> &'static str {
    const FISH_SH: &str = include_str!("../../../app/assets/bundled/bootstrap/fish.sh");
    let start_marker = "function warp_external_ctrl_t_widget\n  set -l widget \"\"\n  for binding in (bind \\ct 2>/dev/null)";
    let end_marker = "  test -n \"$widget\"; or return 1\n  echo \"$widget\"\nend";
    let start = FISH_SH
        .find(start_marker)
        .expect("fish ctrl-t widget query function start should exist");
    let end = FISH_SH[start..]
        .find(end_marker)
        .expect("fish ctrl-t widget query function end should exist");
    &FISH_SH[start..start + end + end_marker.len()]
}

fn fish_ctrl_t_detection_snippet() -> &'static str {
    const FISH_SH: &str = include_str!("../../../app/assets/bundled/bootstrap/fish.sh");
    let start_marker = "set -g _WARP_EXTERNAL_CTRL_T_WIDGET \"\"\n  set -l warp_ctrl_t_widget (warp_external_ctrl_t_widget)\n  switch \"$warp_ctrl_t_widget\"";
    let end_marker = "        set -a shell_plugins external_ctrl_t_file\n      end\n  end";
    let start = FISH_SH
        .find(start_marker)
        .expect("fish ctrl-t detection snippet start should exist");
    let end = FISH_SH[start..]
        .find(end_marker)
        .expect("fish ctrl-t detection snippet end should exist");
    &FISH_SH[start..start + end + end_marker.len()]
}

fn fish_ctrl_t_widget_result_fn() -> &'static str {
    const FISH_SH: &str = include_str!("../../../app/assets/bundled/bootstrap/fish.sh");
    // Locates the function boundary structurally (start of the `function` line to its matching
    // `end` line) rather than by matching the literal body text, so a behavioral mutation to the
    // comparison inside it changes what the test observes instead of breaking extraction itself.
    let start_marker = "function warp_ctrl_t_widget_result\n";
    let start = FISH_SH
        .find(start_marker)
        .expect("fish ctrl-t widget result function start should exist");
    let end_marker = "\nend\n";
    let end = FISH_SH[start..]
        .find(end_marker)
        .expect("fish ctrl-t widget result function end should exist");
    &FISH_SH[start..start + end + end_marker.len()]
}

#[test]
fn test_fish_ctrl_t_widget_result_is_empty_when_widget_leaves_draft_unchanged() {
    let result_fn = fish_ctrl_t_widget_result_fn();
    let script = format!(
        r#"
{result_fn}
set result (warp_ctrl_t_widget_result 'echo START MIDDLE' 'echo START MIDDLE')
printf 'result=[%s]\n' "$result"
"#
    );
    let Some(stdout) = run_fish(&script) else {
        return;
    };
    assert!(stdout.contains("result=[]"), "{stdout}");
}

#[test]
fn test_fish_ctrl_t_widget_result_preserves_changed_line() {
    let result_fn = fish_ctrl_t_widget_result_fn();
    let script = format!(
        r#"
{result_fn}
set result (warp_ctrl_t_widget_result 'echo START MIDDLE' 'echo START nested.rs MIDDLE')
printf 'result=[%s]\n' "$result"
"#
    );
    let Some(stdout) = run_fish(&script) else {
        return;
    };
    assert!(
        stdout.contains("result=[echo START nested.rs MIDDLE]"),
        "{stdout}"
    );
}

/// Hex-encodes `s` the way [`ctrl_t_draft_arg`] does, for building test `{char_cursor}:{hex}`
/// arguments without depending on fish's own `warp_hex_encode_string`.
fn hex_encode(s: &str) -> String {
    s.bytes().map(|b| format!("{b:02x}")).collect()
}

fn fish_hex_decode_string_fn() -> &'static str {
    const FISH_SH: &str = include_str!("../../../app/assets/bundled/bootstrap/fish.sh");
    let start_marker = "function warp_hex_decode_string\n";
    let start = FISH_SH
        .find(start_marker)
        .expect("fish hex decode function start should exist");
    let end_marker = "\nend\n";
    let end = FISH_SH[start..]
        .find(end_marker)
        .expect("fish hex decode function end should exist");
    &FISH_SH[start..start + end + end_marker.len()]
}

/// The `string split` + `warp_hex_decode_string` argument-parsing step inside
/// `warp_run_external_ctrl_t_widget`, extracted on its own (not via the full widget runner) so a
/// dedicated test can assert the decoded `char_cursor`/`original_line` directly. The two
/// full-widget tests below can't catch a corrupted split or decode by themselves: the same
/// corrupted value seeds both sides of `warp_ctrl_t_widget_result`'s equality check and cancels
/// out.
fn fish_ctrl_t_argument_parsing_snippet() -> &'static str {
    const FISH_SH: &str = include_str!("../../../app/assets/bundled/bootstrap/fish.sh");
    // Structural, not literal-text, boundaries (see `fish_ctrl_t_widget_result_fn` above) so a
    // behavioral change to the parsing logic itself changes what the test observes.
    let start_marker = "set -l warp_ctrl_t_parts (string split -m 1 -- ':' \"$argv[2]\")\n";
    let start = FISH_SH
        .find(start_marker)
        .expect("fish ctrl-t argument parsing snippet start should exist");
    let end_marker = "--allow-empty)\n";
    let end = FISH_SH[start..]
        .find(end_marker)
        .expect("fish ctrl-t argument parsing snippet end should exist");
    &FISH_SH[start..start + end + end_marker.len()]
}

/// Regression test for the argument-parsing step alone, asserting the decoded `char_cursor` and
/// `original_line` directly against a draft with both an embedded and a trailing newline -- the
/// case that requires `string collect --no-trim-newlines`, not just `warp_hex_decode_string`
/// itself, to survive intact.
#[test]
fn test_fish_ctrl_t_argument_parsing_decodes_multiline_trailing_newline_draft() {
    let hex_decode_fn = fish_hex_decode_string_fn();
    let parsing_snippet = fish_ctrl_t_argument_parsing_snippet();
    let hex_draft = hex_encode("echo one\ntwo\n");
    let script = format!(
        r#"
{hex_decode_fn}
function warp_ctrl_t_test_parse
  {parsing_snippet}
  printf 'char_cursor=[%s]\n' "$char_cursor"
  printf 'original_line=[%s]\n' "$original_line"
end
warp_ctrl_t_test_parse test-token '8:{hex_draft}'
"#
    );
    let Some(stdout) = run_fish(&script) else {
        return;
    };
    assert!(stdout.contains("char_cursor=[8]"), "{stdout}");
    assert!(
        stdout.contains("original_line=[echo one\ntwo\n]"),
        "{stdout}"
    );
}

fn fish_ctrl_t_widget_runner_fn() -> &'static str {
    const FISH_SH: &str = include_str!("../../../app/assets/bundled/bootstrap/fish.sh");
    let start_marker = "function warp_run_external_ctrl_t_widget\n";
    let start = FISH_SH
        .find(start_marker)
        .expect("fish ctrl-t widget runner function start should exist");
    let end_marker = "\nend\n";
    let end = FISH_SH[start..]
        .find(end_marker)
        .expect("fish ctrl-t widget runner function end should exist");
    &FISH_SH[start..start + end + end_marker.len()]
}

/// Builds a script that runs the full `warp_run_external_ctrl_t_widget` (not just the
/// `warp_ctrl_t_widget_result` comparison helper in isolation) against a real `{char_cursor}:{hex}`
/// argument, so the `(commandline | string collect)` argument at its `fzf-file-widget` call site
/// is exercised too -- unquoted, a multi-line result there would otherwise expand to multiple
/// arguments, silently truncating that comparison to the result's first line alone. `commandline`
/// is stubbed statefully (supporting the `-r --` and `-C --` forms the widget actually calls,
/// plus a plain read) rather than as a fixed value, since the widget both seeds and reads it
/// back. The read stub uses `echo`, matching the real builtin's own bare-read behavior of always
/// terminating its output with a newline regardless of the buffer's actual content -- a stub that
/// used `printf '%s'` instead would silently hide a regression in the comparison's own newline
/// handling. The `-r` stub also distinguishes a *read* (no CMD argument at all, i.e.
/// `$argv[3..]` is empty) from a *write*, matching real fish semantics -- a stub that always
/// wrote, even with nothing to write, would silently hide a regression that fails to seed an
/// empty draft. `_test_cl_value` starts as a non-empty sentinel rather than `''`, so a failure to
/// seed (leaving the sentinel in place) is distinguishable from a correctly-seeded empty draft.
fn fish_ctrl_t_widget_test_script(ctrl_t_arg: &str, widget_body: &str) -> String {
    let runner = fish_ctrl_t_widget_runner_fn();
    let hex_decode_fn = fish_hex_decode_string_fn();
    let widget_result_fn = fish_ctrl_t_widget_result_fn();
    format!(
        r#"
# Unlike the real warp_escape_json (see fish_warp_escape_json_fn above), this stub doesn't
# actually escape a real newline into JSON's `\n` -- piped through `string collect` purely so
# that leaving one in doesn't itself get re-split by the `set` below that captures this
# function's own output, which would otherwise mask the very truncation these tests exist to
# catch behind an unrelated space-joining artifact of the stub.
function warp_escape_json
  string join \n $argv | string collect
end
function warp_send_json_message
  echo "$argv"
end
{hex_decode_fn}
{widget_result_fn}
set -g _test_cl_value 'UNSEEDED-SENTINEL'
function commandline
  if test (count $argv) -ge 1; and test "$argv[1]" = '-r'
    if test (count $argv[3..]) -eq 0
      # No CMD argument at all is a read, not a write -- must leave $_test_cl_value untouched.
      return 0
    end
    set -g _test_cl_value (string collect --no-trim-newlines -- $argv[3..])
    return 0
  end
  if test (count $argv) -ge 1; and test "$argv[1]" = '-C'
    return 0
  end
  echo "$_test_cl_value"
end
function fzf-file-widget
  {widget_body}
end
set -g _WARP_EXTERNAL_CTRL_T_WIDGET fzf-file-widget
set -g WARP_SESSION_ID 12345
{runner}
warp_run_external_ctrl_t_widget test-token '{ctrl_t_arg}'
"#
    )
}

/// Regression test for the `(commandline | string collect)` argument at the widget's
/// `fzf-file-widget` call site: without `string collect`, a multi-line selection is split by that
/// call's own (unquoted) command substitution into multiple arguments, silently truncating
/// `warp_ctrl_t_widget_result`'s second argument -- and therefore the reported buffer -- to the
/// selection's first line alone.
#[test]
fn test_fish_ctrl_t_widget_reports_full_multiline_change_without_truncation() {
    let hex_draft = hex_encode("echo START\nMIDDLE");
    let script = fish_ctrl_t_widget_test_script(
        &format!("10:{hex_draft}"),
        "commandline -r -- (printf 'echo START\\nMIDDLE nested.rs ' | string collect)",
    );
    let Some(stdout) = run_fish(&script) else {
        return;
    };
    assert!(
        stdout.contains("\"buffer\": \"echo START\nMIDDLE nested.rs \""),
        "{stdout}"
    );
}

/// Companion to the test above, for the failure mode the same truncation causes on cancel: a
/// multi-line draft left unchanged gets word-split at the same call site, so
/// `warp_ctrl_t_widget_result` compares the full original line against only its own first line,
/// finds them unequal, and reports that stale first line as if it were a real selection instead
/// of the empty buffer this "unchanged" case is supposed to produce.
#[test]
fn test_fish_ctrl_t_widget_reports_empty_when_multiline_draft_is_left_unchanged() {
    let hex_draft = hex_encode("echo START\nMIDDLE");
    let script = fish_ctrl_t_widget_test_script(&format!("10:{hex_draft}"), "# cancelled");
    let Some(stdout) = run_fish(&script) else {
        return;
    };
    assert!(stdout.contains(r#""buffer": """#), "{stdout}");
}

/// Regression test for a plain, single-line, unchanged draft on cancel: a bare `commandline` read
/// always terminates its own output with a newline regardless of the buffer's actual content, so
/// comparing it against `original_line` with `--no-trim-newlines` (rather than the default,
/// trimming `string collect`) would make an ordinary, single-line cancel always compare unequal
/// to itself, misreporting the cancel as a real selection.
#[test]
fn test_fish_ctrl_t_widget_reports_empty_when_single_line_draft_is_left_unchanged() {
    let hex_draft = hex_encode("echo START MIDDLE");
    let script = fish_ctrl_t_widget_test_script(&format!("11:{hex_draft}"), "# cancelled");
    let Some(stdout) = run_fish(&script) else {
        return;
    };
    assert!(stdout.contains(r#""buffer": """#), "{stdout}");
}

/// Regression test for an empty draft (ctrl-t on a blank line): decoding zero bytes must still
/// seed the commandline with an explicit empty buffer, not skip seeding altogether.
/// `warp_hex_decode_string` on an empty hex string produces no output at all, and piping that
/// through plain `string collect` collapses to zero list elements rather than one empty string --
/// so `commandline -r --` would receive no CMD argument, which fish treats as a *read*, leaving
/// whatever was already on the commandline (here, the sentinel standing in for the synthetic
/// helper invocation itself) in place instead of clearing it.
#[test]
fn test_fish_ctrl_t_widget_seeds_blank_buffer_for_empty_draft() {
    let script = fish_ctrl_t_widget_test_script("0:", "# cancelled");
    let Some(stdout) = run_fish(&script) else {
        return;
    };
    assert!(stdout.contains(r#""buffer": """#), "{stdout}");
    assert!(!stdout.contains("UNSEEDED-SENTINEL"), "{stdout}");
}

/// Regression test for the fish equivalent of bash's picker-function guard: detection must
/// decline (no tag, no interception) when `fzf-file-widget` -- the function
/// `warp_run_external_ctrl_t_widget` now calls directly -- isn't actually defined, even though
/// `bind` reports it as ctrl-t's binding. Without this guard, a rebind to a nonexistent or
/// renamed function would have ctrl-t tagged and intercepted with nothing to invoke, swallowing
/// the key instead of leaving it alone.
#[test]
fn test_fish_ctrl_t_detection_declines_when_picker_function_is_absent() {
    let query_fn = fish_ctrl_t_widget_query_fn();
    let detection = fish_ctrl_t_detection_snippet();
    let script = format!(
        r#"
{query_fn}
bind \ct fzf-file-widget
set -l shell_plugins
{detection}
printf 'widget=[%s] plugins=[%s]\n' "$_WARP_EXTERNAL_CTRL_T_WIDGET" "$shell_plugins"
"#
    );
    let Some(stdout) = run_fish(&script) else {
        return;
    };
    assert!(stdout.contains("widget=[]"), "{stdout}");
    assert!(!stdout.contains("external_ctrl_t_file"), "{stdout}");
}

#[test]
fn test_fish_ctrl_t_detection_tags_when_picker_function_is_present() {
    let query_fn = fish_ctrl_t_widget_query_fn();
    let detection = fish_ctrl_t_detection_snippet();
    let script = format!(
        r#"
{query_fn}
function fzf-file-widget
end
bind \ct fzf-file-widget
set -l shell_plugins
{detection}
printf 'widget=[%s] plugins=[%s]\n' "$_WARP_EXTERNAL_CTRL_T_WIDGET" "$shell_plugins"
"#
    );
    let Some(stdout) = run_fish(&script) else {
        return;
    };
    assert!(stdout.contains("widget=[fzf-file-widget]"), "{stdout}");
    assert!(stdout.contains("external_ctrl_t_file"), "{stdout}");
}
