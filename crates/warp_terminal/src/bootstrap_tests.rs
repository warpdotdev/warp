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

fn fish_sh() -> &'static str {
    static NORMALIZED: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    NORMALIZED.get_or_init(|| {
        include_str!("../../../app/assets/bundled/bootstrap/fish.sh").replace('\r', "")
    })
}

#[test]
fn fish_sh_strips_carriage_returns_so_lf_extraction_markers_match() {
    assert!(!fish_sh().contains('\r'));
    assert!(fish_sh().contains("function warp_external_ctrl_r_widget\n"));
    assert!(fish_sh().contains("function warp_run_external_ctrl_r_widget\n"));
}

fn extract_fish_fn(start_marker: &str, end_marker: &str, what: &str) -> &'static str {
    let start = fish_sh()
        .find(start_marker)
        .unwrap_or_else(|| panic!("{what} start should exist"));
    let end = fish_sh()[start..]
        .find(end_marker)
        .unwrap_or_else(|| panic!("{what} end should exist"));
    &fish_sh()[start..start + end + end_marker.len()]
}

fn fish_ctrl_r_widget_query_fn() -> &'static str {
    extract_fish_fn(
        "function warp_external_ctrl_r_widget\n",
        "\nend\n",
        "fish ctrl-r widget query function",
    )
}

fn fish_ctrl_r_detection_snippet() -> &'static str {
    extract_fish_fn(
        "set -g _WARP_EXTERNAL_CTRL_R_WIDGET \"\"\n  set -l warp_ctrl_r_widget (warp_external_ctrl_r_widget)\n  switch \"$warp_ctrl_r_widget\"",
        "        set -a shell_plugins external_ctrl_r_history\n      end\n  end",
        "fish ctrl-r detection snippet",
    )
}

fn fish_ctrl_r_widget_runner_fn() -> &'static str {
    extract_fish_fn(
        "function warp_run_external_ctrl_r_widget\n",
        "\nend\n",
        "fish ctrl-r widget runner function",
    )
}

fn fish_ctrl_r_detection_script(define_widget: bool) -> String {
    let query = fish_ctrl_r_widget_query_fn();
    let detection = fish_ctrl_r_detection_snippet();
    let widget_fn = if define_widget {
        "function _fzf_search_history\nend\n"
    } else {
        ""
    };
    format!(
        r#"
{query}
{widget_fn}bind \cr _fzf_search_history
set -l shell_plugins
{detection}
printf 'widget=[%s] plugins=[%s]\n' "$_WARP_EXTERNAL_CTRL_R_WIDGET" "$shell_plugins"
"#,
        query = query,
        widget_fn = widget_fn,
        detection = detection,
    )
}

#[test]
fn test_fish_ctrl_r_detection_tags_fzf_fish_search_history_when_function_is_present() {
    let Some(stdout) = run_fish(&fish_ctrl_r_detection_script(true)) else {
        return;
    };
    assert!(stdout.contains("widget=[_fzf_search_history]"), "{stdout}");
    assert!(stdout.contains("external_ctrl_r_history"), "{stdout}");
}

#[test]
fn test_fish_ctrl_r_detection_declines_fzf_fish_search_history_when_function_is_absent() {
    let Some(stdout) = run_fish(&fish_ctrl_r_detection_script(false)) else {
        return;
    };
    assert!(stdout.contains("widget=[]"), "{stdout}");
    assert!(!stdout.contains("external_ctrl_r_history"), "{stdout}");
}

#[test]
fn test_fish_ctrl_r_widget_reports_fzf_fish_search_history_selection() {
    let runner = fish_ctrl_r_widget_runner_fn();
    let script = format!(
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
function _fzf_search_history
  set -g _test_commandline_value 'echo selected_from_fzf_fish'
end
set -g _WARP_EXTERNAL_CTRL_R_WIDGET _fzf_search_history
set -g WARP_SESSION_ID 12345
{runner}
warp_run_external_ctrl_r_widget test-token
"#
    );
    let Some(stdout) = run_fish(&script) else {
        return;
    };
    assert!(
        stdout.contains(r#""buffer": "echo selected_from_fzf_fish""#),
        "{stdout}"
    );
}
