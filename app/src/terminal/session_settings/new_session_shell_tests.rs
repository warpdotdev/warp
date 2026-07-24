use warp_util::path::ShellFamily;

use super::NewSessionShell;

// These regression tests pin the contract that
// `AvailableShells::user_preferred_shell_family` relies on: a user who
// has explicitly picked PowerShell (or a `pwsh`/`powershell` executable
// anywhere — pwsh on macOS, pwsh on Linux) must get
// `ShellFamily::PowerShell`, not the OS default. Without this, the
// worktree-path shell-escape in #11144 would silently use the wrong
// family on cross-OS shell setups (Andy Carlson's review on #11144).

#[test]
fn powershell_executable_picks_powershell_family_regardless_of_os() {
    let cases = [
        "pwsh",
        "pwsh.exe",
        "powershell",
        "powershell.exe",
        "/usr/local/bin/pwsh",
        "C:\\Program Files\\PowerShell\\7\\pwsh.exe",
    ];
    for case in cases {
        let setting = NewSessionShell::Executable(case.to_string());
        assert_eq!(
            setting.shell_family(),
            ShellFamily::PowerShell,
            "{case} should resolve to PowerShell family"
        );
    }
}

#[test]
fn non_powershell_executable_picks_posix_family() {
    let cases = ["bash", "zsh", "fish", "/bin/sh", "/opt/homebrew/bin/zsh"];
    for case in cases {
        let setting = NewSessionShell::Executable(case.to_string());
        assert_eq!(
            setting.shell_family(),
            ShellFamily::Posix,
            "{case} should resolve to Posix family"
        );
    }
}

#[test]
fn wsl_is_always_posix() {
    let setting = NewSessionShell::WSL("Ubuntu-22.04".to_string());
    assert_eq!(setting.shell_family(), ShellFamily::Posix);
}

#[test]
fn custom_pwsh_is_powershell_family() {
    let setting = NewSessionShell::Custom("/opt/microsoft/pwsh/7.4.0/pwsh".to_string());
    assert_eq!(setting.shell_family(), ShellFamily::PowerShell);
}

#[test]
fn msys2_pwsh_is_powershell_family() {
    // MSYS2 lets users wrap a PowerShell binary too; the family pick
    // must still respect the executable name.
    let setting = NewSessionShell::MSYS2("pwsh.exe".to_string());
    assert_eq!(setting.shell_family(), ShellFamily::PowerShell);
}
