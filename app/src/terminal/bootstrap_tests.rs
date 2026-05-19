use super::*;

struct TestAssetProvider;

impl AssetProvider for TestAssetProvider {
    fn get(&self, path: &str) -> anyhow::Result<Cow<'_, [u8]>> {
        let content = match path {
            "bundled/bootstrap/bash.sh" => "#include hello_world",
            "bundled/bootstrap/fish.sh" => "# this is a comment\nthis_is_a_command",
            "bundled/bootstrap/nu.nu" => "# this is a comment\n$env.config = {}",
            "bundled/bootstrap/nu_init_shell.nu" => {
                r#"$env.WARP_USING_WINDOWS_CON_PTY = "@@USING_CON_PTY_BOOLEAN@@""#
            }
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
    assert_eq!(
        decode_script(&script_for_shell(ShellType::Nushell, &TestAssetProvider)),
        "$env.config = {}\n"
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

#[test]
fn test_nushell_init_shell_script_replaces_conpty_placeholder() {
    assert_eq!(
        init_shell_script_for_shell(ShellType::Nushell, &TestAssetProvider),
        format!(r#"$env.WARP_USING_WINDOWS_CON_PTY = "{}""#, cfg!(windows))
    );
}

#[test]
fn test_nushell_bootstrap_env_mutating_helpers_are_env_commands() {
    let script = decode_script(&script_for_shell(ShellType::Nushell, &crate::ASSETS));
    assert!(script.contains("def --env warp_precmd []"));
    assert!(script.contains("def --env warp_run_generator_command [command_id command]"));
}
fn decode_script(bytes: &[u8]) -> &str {
    std::str::from_utf8(bytes).expect("should not fail to decode")
}
