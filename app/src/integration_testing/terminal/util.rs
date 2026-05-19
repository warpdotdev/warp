use async_io::block_on;
use command::blocking::Command;
use std::borrow::Cow;
use std::iter;
use std::path::{Path, PathBuf};
use warp_core::command::ExitCode;
#[cfg(windows)]
use warp_core::paths::base_config_dir;

use rand::Rng;
use rand::{distributions::Alphanumeric, thread_rng};
use regex::Regex;

use crate::terminal::shell::ShellType;
use crate::terminal::{
    local_tty::shell::{DirectShellStarter, ShellStarter, ShellStarterSource},
    shell,
};

/// Returns the shell starter along with the version of the shell about to be run.
pub fn current_shell_starter_and_version() -> (DirectShellStarter, String) {
    let shell_starter_or_wsl_name = ShellStarter::init(Default::default())
        .expect("Could not create a shell starter or wsl name");
    let shell_starter_source =
        block_on(async { shell_starter_or_wsl_name.to_shell_starter_source().await })
            .expect("Could not create a shell starter source");
    let starter = match shell_starter_source {
        ShellStarterSource::Override(starter) => match starter {
            ShellStarter::Direct(direct_shell_starter) => direct_shell_starter,
            ShellStarter::Wsl(_) => {
                // TODO(CORE-2302): Support integration tests on Windows (including WSL).
                todo!("We don't yet support integration tests for WSL shells")
            }
            // TODO(CORE-2302): Support integration tests on Windows (including WSL).
            ShellStarter::MSYS2(_) => {
                todo!("We don't yet support integration tests for MSYS2")
            }
            ShellStarter::DockerSandbox(_) => {
                todo!("We don't yet support integration tests for Docker sandbox shells")
            }
        },
        ShellStarterSource::Environment(starter)
        | ShellStarterSource::UserDefault(starter)
        | ShellStarterSource::Fallback { starter, .. } => starter,
    };
    let version = match starter.shell_type() {
        shell::ShellType::Zsh => {
            let stdout = Command::new(starter.logical_shell_path())
                .args(["-c", "echo $ZSH_VERSION"])
                .output()
                .expect("version command should run")
                .stdout;
            String::from_utf8_lossy(&stdout).into_owned()
        }
        shell::ShellType::Bash => {
            let stdout = Command::new(starter.logical_shell_path())
                .args(["-c", "echo $BASH_VERSION"])
                .output()
                .expect("version command should run")
                .stdout;
            String::from_utf8_lossy(&stdout).into_owned()
        }
        shell::ShellType::Fish => {
            let stdout = Command::new(starter.logical_shell_path())
                .args(["-c", "echo $FISH_VERSION"])
                .output()
                .expect("version command should run")
                .stdout;
            String::from_utf8_lossy(&stdout).into_owned()
        }
        shell::ShellType::Nushell => {
            let stdout = Command::new(starter.logical_shell_path())
                .args(["-c", "version | get version"])
                .output()
                .expect("version command should run")
                .stdout;
            String::from_utf8_lossy(&stdout).into_owned()
        }
        shell::ShellType::PowerShell => {
            let stdout = Command::new(starter.logical_shell_path())
                .args(["-Version"])
                .output()
                .expect("version command should run")
                .stdout;
            String::from_utf8_lossy(&stdout).into_owned()
        }
    };
    assert!(!version.is_empty());
    (starter, version)
}

/// Returns the directory for the default histfile location for the ShellType in this
/// ShellStarter based on the given user `home_dir`.
pub fn default_histfile_directory(shell: &ShellType, home_dir: &Path) -> PathBuf {
    match shell {
        ShellType::Fish => home_dir.join(".local/share/fish"),
        ShellType::Nushell => home_dir.join(".local/share/nushell"),
        #[cfg(not(windows))]
        ShellType::PowerShell => home_dir.join(".local/share/powershell/PSReadLine"),
        #[cfg(windows)]
        ShellType::PowerShell => base_config_dir().join("Microsoft/Windows/PowerShell/PSReadLine"),
        _ => home_dir.to_owned(),
    }
}

/// Generates a random nonce to distinguish between commands.
pub fn nonce() -> String {
    let mut rng = thread_rng();
    iter::repeat(())
        .map(|()| rng.sample(Alphanumeric))
        .map(char::from)
        .take(7)
        .collect()
}

/// Different options for asserting the value of the exit code.
pub enum ExpectedExitStatus {
    /// Checks code == 0
    Success,
    /// Checks code != 0
    Failure,
    /// Checks code == expected
    ExactCode(ExitCode),
    /// Any exit status is considered valid.
    Any,
}

/// A representation of the expected output from running a command.
pub trait ExpectedOutput: std::fmt::Debug {
    /// Returns whether the given result matches the expected output.
    fn matches(&self, result: &str) -> bool;
}

#[derive(Debug)]
pub struct ExactLine<'a>(Cow<'a, str>);

impl<'a, T: Into<Cow<'a, str>>> From<T> for ExactLine<'a> {
    fn from(value: T) -> Self {
        ExactLine(value.into())
    }
}

impl ExpectedOutput for str {
    fn matches(&self, result: &str) -> bool {
        self == result
    }
}

impl<T: ExpectedOutput + ?Sized> ExpectedOutput for &T {
    fn matches(&self, result: &str) -> bool {
        (*self).matches(result)
    }
}

impl ExpectedOutput for String {
    fn matches(&self, result: &str) -> bool {
        self == result
    }
}

impl ExpectedOutput for ExactLine<'_> {
    fn matches(&self, result: &str) -> bool {
        result.lines().any(|line| line == self.0)
    }
}

impl ExpectedOutput for Regex {
    fn matches(&self, result: &str) -> bool {
        self.is_match(result)
    }
}

impl ExpectedOutput for Path {
    fn matches(&self, result: &str) -> bool {
        self.to_str() == Some(result)
    }
}

impl ExpectedOutput for PathBuf {
    fn matches(&self, result: &str) -> bool {
        self.as_path().matches(result)
    }
}

impl ExpectedOutput for () {
    fn matches(&self, _result: &str) -> bool {
        true
    }
}

impl<T: ExpectedOutput> ExpectedOutput for Option<T> {
    fn matches(&self, result: &str) -> bool {
        match self {
            Some(expected) => expected.matches(result),
            None => true,
        }
    }
}

#[derive(Debug)]
pub struct JsonEq(pub serde_json::Value);

impl ExpectedOutput for JsonEq {
    fn matches(&self, result: &str) -> bool {
        match serde_json::from_str::<serde_json::Value>(result) {
            Ok(actual) => actual == self.0,
            Err(_) => false,
        }
    }
}
