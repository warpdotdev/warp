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

/// Regression test for GH-1957.
///
/// `warp_ssh_helper` (defined in each shell's bootstrap body) is the wrapper
/// that spawns the user's interactive `ssh` for a Warp-managed remote
/// session. Without `-o LogLevel=ERROR`, OpenSSH emits INFO-level messages
/// (notably `channel N: open failed: connect failed: open failed`, logged by
/// `channel_input_open_failure` in OpenSSH's `channels.c` whenever a
/// client-initiated port-forward is rejected by the server) directly into
/// the user's terminal, where they appear as terminal noise the user can't
/// silence via `~/.ssh/config` (cli `-o` wins over config).
///
/// This test asserts each shell's wrapper invocation includes the flag, so a
/// future refactor can't accidentally regress the fix.
#[test]
fn test_warp_ssh_helper_suppresses_openssh_chatter() {
    const BASH_BODY: &str = include_str!("../../assets/bundled/bootstrap/bash_body.sh");
    const ZSH_BODY: &str = include_str!("../../assets/bundled/bootstrap/zsh_body.sh");
    const FISH_BODY: &str = include_str!("../../assets/bundled/bootstrap/fish.sh");

    for (shell, body) in [
        ("bash_body.sh", BASH_BODY),
        ("zsh_body.sh", ZSH_BODY),
        ("fish.sh", FISH_BODY),
    ] {
        let helper_idx = body.find("warp_ssh_helper").unwrap_or_else(|| {
            panic!("{shell}: `warp_ssh_helper` definition not found")
        });
        let invocation_offset = body[helper_idx..]
            .find("command ssh -o ControlMaster=yes")
            .unwrap_or_else(|| {
                panic!(
                    "{shell}: `command ssh -o ControlMaster=yes` no longer appears inside \
                     `warp_ssh_helper`. If the wrapper structure has changed, update this \
                     regression test (and verify GH-1957 doesn't regress)."
                )
            });
        let invocation_start = helper_idx + invocation_offset;
        // The invocation is multi-line via `\` continuations. Inspect a wide
        // window after the start of the call so we cover the full arg list
        // regardless of how it's wrapped across lines.
        let window_end = (invocation_start + 600).min(body.len());
        let window = &body[invocation_start..window_end];
        assert!(
            window.contains("-o LogLevel=ERROR"),
            "GH-1957: `{shell}`'s `warp_ssh_helper` is missing `-o LogLevel=ERROR`. \
             Without it, OpenSSH's INFO-level chatter (channel-open-failure noise) \
             leaks into the user's terminal. Window inspected:\n{window}"
        );
    }
}
