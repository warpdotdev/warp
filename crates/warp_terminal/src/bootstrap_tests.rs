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

#[test]
fn test_ssh_wrapper_keeps_windows_shell_guard() {
    const POSIX_GUARD: &str = r#"set -- \`# '\"' <# \" 2>nul\` || %COMSPEC%"#;
    // fish keeps `\`` literally inside double quotes, so its wrapper uses bare backticks.
    const FISH_GUARD: &str = r#"set -- `# '\"' <# \" 2>nul` || %COMSPEC%"#;
    const POWERSHELL_TAIL: [&str; 2] =
        ["exit #>", r#"iex '& (Get-Process -Id \$PID).Path -NoLogo'"#];

    for (shell_type, guard) in [
        (ShellType::Bash, POSIX_GUARD),
        (ShellType::Zsh, POSIX_GUARD),
        (ShellType::Fish, FISH_GUARD),
    ] {
        let script = render_script_for_shell(shell_type, &crate::ASSETS);
        let lines = script.lines().collect_vec();

        let guard_line = lines
            .iter()
            .positions(|line| *line == guard)
            .exactly_one()
            .unwrap_or_else(|_| panic!("{shell_type:?}: expected exactly one guard line"));
        // cmd.exe runs only the first line of the remote command.
        assert_eq!(
            lines[guard_line - 1],
            "\"",
            "{shell_type:?}: the guard must open the remote command"
        );

        let closing_quote = guard_line
            + lines[guard_line..]
                .iter()
                .position(|line| *line == "\"")
                .unwrap_or_else(|| panic!("{shell_type:?}: the remote command is never closed"));
        assert_eq!(
            lines[closing_quote - 2..closing_quote],
            POWERSHELL_TAIL,
            "{shell_type:?}: the remote command must end with the PowerShell tail"
        );
        assert!(
            lines[guard_line + 1..closing_quote - 2]
                .iter()
                .all(|line| !line.contains("#>")),
            "{shell_type:?}: a `#>` in the POSIX body ends PowerShell's block comment early"
        );
    }
}

fn decode_script(bytes: &[u8]) -> &str {
    std::str::from_utf8(bytes).expect("should not fail to decode")
}
