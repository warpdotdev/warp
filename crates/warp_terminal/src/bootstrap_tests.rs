#[cfg(unix)]
use command::blocking::Command;

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

#[cfg(unix)]
#[test]
fn test_zsh_fzf_history_selection_restores_multiline_entry() {
    assert_eq!(
        run_zsh_fzf_history_selection(
            "echo CORE3811_FIRST\necho CORE3811_SECOND",
            r"    42  echo CORE3811_FIRST\necho CORE3811_SECOND",
        ),
        b"echo CORE3811_FIRST\necho CORE3811_SECOND"
    );
}

#[cfg(unix)]
#[test]
fn test_zsh_fzf_history_selection_preserves_literal_backslash_n() {
    assert_eq!(
        run_zsh_fzf_history_selection(r"printf '%s\n' hi", r"    42  printf '%s\n' hi",),
        br"printf '%s\n' hi"
    );
}

#[cfg(unix)]
fn run_zsh_fzf_history_selection(canonical_history: &str, listed_history: &str) -> Vec<u8> {
    const ZSH_BODY: &str = include_str!("../../../app/assets/bundled/bootstrap/zsh_body.sh");
    const FUNCTION_START: &str = "  function warp_select_fzf_history_entry () {";
    const FUNCTION_END: &str = "\n  }\n\n  # Runs the shell's own ctrl-r";

    let function_start = ZSH_BODY
        .find(FUNCTION_START)
        .expect("fzf history selection function should exist");
    let function_body = &ZSH_BODY[function_start..];
    let function_end = function_body
        .find(FUNCTION_END)
        .expect("fzf history selection function should have a closing brace")
        + "\n  }".len();
    let function_body = &function_body[..function_end];
    let script = format!(
        r#"
{function_body}
function fc() {{
  print -r -- "$LISTED_HISTORY"
}}
function fzf() {{
  command cat
}}
function run_test() {{
  local -A history
  history[42]="$CANONICAL_HISTORY"
  warp_select_fzf_history_entry
  print -rn -- "$REPLY"
}}
run_test
"#
    );

    let output = Command::new("zsh")
        .args(["-dfc", &script])
        .env("CANONICAL_HISTORY", canonical_history)
        .env("LISTED_HISTORY", listed_history)
        .output()
        .expect("zsh should run the fzf history selection fixture");
    assert!(
        output.status.success(),
        "zsh fixture failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

fn decode_script(bytes: &[u8]) -> &str {
    std::str::from_utf8(bytes).expect("should not fail to decode")
}
