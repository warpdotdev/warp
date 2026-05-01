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

/// Regression test for GH-1160.
///
/// Until this fix, the MotD-emulation block in each shell-bootstrap body was
/// nested inside an `if test "${SHELL##*/}" != "bash" -a "${SHELL##*/}" !=
/// "zsh"` guard, with a comment claiming MotD was "instead handled by our
/// bootstrap script" for bash and zsh. It wasn't — sshd skips MotD for
/// command-passing invocations and Warp's bash/zsh rcfile bootstrap doesn't
/// reintroduce it, so bash and zsh users silently lost the MotD over Warp SSH.
///
/// This test asserts the structural invariant after the fix: in each of
/// `bash_body.sh`, `zsh_body.sh`, and `fish.sh` the MotD-print branch
/// (identified by `cat /etc/motd` / `cat /run/motd.dynamic`) appears **before**
/// the `!= "bash" -a` shell-type guard, so it runs unconditionally.
#[test]
fn test_motd_emulation_is_not_gated_on_shell_type() {
    const BASH_BODY: &str = include_str!("../../assets/bundled/bootstrap/bash_body.sh");
    const ZSH_BODY: &str = include_str!("../../assets/bundled/bootstrap/zsh_body.sh");
    const FISH_BODY: &str = include_str!("../../assets/bundled/bootstrap/fish.sh");

    for (shell, body) in [
        ("bash_body.sh", BASH_BODY),
        ("zsh_body.sh", ZSH_BODY),
        ("fish.sh", FISH_BODY),
    ] {
        // Marker for the actual MotD-print probe. We deliberately match the
        // *executable* `test -f /etc/motd && test -r /etc/motd` line and not
        // bare `/etc/motd`, because `/etc/motd` also appears in the
        // surrounding explanatory comments — a pre-fix file that kept the
        // comments but removed the executable probe would still pass a
        // `body.find("/etc/motd")` check, defeating the test.
        let motd_marker = "test -f /etc/motd && test -r /etc/motd";
        // Marker for the non-bash/non-zsh guard. The `!= "bash" -a` substring
        // is stable across the three heredocs.
        let guard_marker = "!= \"bash\" -a";

        let motd_idx = body.find(motd_marker).unwrap_or_else(|| {
            panic!(
                "{shell}: executable MotD probe ({motd_marker:?}) not found. \
                 If the probe structure changed, update this regression test \
                 (and verify GH-1160 doesn't regress)."
            )
        });
        let guard_idx = body.find(guard_marker).unwrap_or_else(|| {
            panic!(
                "{shell}: non-bash/non-zsh guard ({guard_marker:?}) not found. \
                 If the heredoc structure changed, update this regression test \
                 (and verify GH-1160 doesn't regress)."
            )
        });

        assert!(
            motd_idx < guard_idx,
            "GH-1160: in `{shell}` the executable MotD probe (offset {motd_idx}) \
             must appear BEFORE the non-bash/non-zsh guard (offset {guard_idx}) \
             so MotD prints for bash and zsh too. Putting the MotD block back \
             inside the guard silently regresses GH-1160 (bash/zsh users will \
             no longer see /etc/motd over Warp SSH)."
        );
    }
}
