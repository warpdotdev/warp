cfg_if::cfg_if! {
    if #[cfg(feature = "v2")] {
        mod v2;
        pub use v2::*;
    }
}

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use smol_str::SmolStr;
use typed_path::{TypedPath, TypedPathBuf};
use warp_core::command::ExitCode;
use warp_util::path::{EscapeChar, ShellFamily};
use warpui::platform::OperatingSystem;

use crate::{completer::TopLevelCommandCaseSensitivity, signatures::CommandRegistry};

use super::engine::EngineDirEntry;

/// This trait may be implemented to configure behavior of the completions engine.
pub trait CompletionContext: Send + Sync {
    /// If path completions are supported, should return an instance of a `PathCompletionContext`
    /// implementation.
    fn path_completion_context(&self) -> Option<&dyn PathCompletionContext>;

    /// If generators are supported, should return an instance of a `GeneratorContext`
    /// implementation.
    fn generator_context(&self) -> Option<&dyn GeneratorContext>;

    fn command_case_sensitivity(&self) -> TopLevelCommandCaseSensitivity {
        OperatingSystem::get().into()
    }

    fn alias_and_function_case_sensitivity(&self) -> TopLevelCommandCaseSensitivity {
        match self.shell_family() {
            Some(ShellFamily::PowerShell) => TopLevelCommandCaseSensitivity::CaseInsensitive,
            _ => TopLevelCommandCaseSensitivity::CaseSensitive,
        }
    }

    fn escape_char(&self) -> EscapeChar {
        // This is fallback logic. Ultimately, the escape character depends on the shell, _not_ the
        // OS. Use the shell to determine this whenever possible. However, if we are in a context
        // where we don't know/have a running shell, we will go by the default shell per OS.
        match OperatingSystem::get() {
            OperatingSystem::Windows => EscapeChar::Backtick,
            OperatingSystem::Linux | OperatingSystem::Mac | OperatingSystem::Other(_) => {
                EscapeChar::Backslash
            }
        }
    }

    #[cfg(feature = "v2")]
    /// If JS execution is supported, should return an instance of `JsExecutionContext`.
    fn js_context(&self) -> Option<&dyn JsExecutionContext> {
        None
    }

    /// Returns top-level commands to be suggested when completing on an empty buffer.
    fn top_level_commands(&self) -> Box<dyn Iterator<Item = &str> + '_>;

    /// The `CommandRegistry` containing `Signature`s used for completions.
    fn command_registry(&self) -> &CommandRegistry;

    /// All available environment variables if exists in the current context.
    fn environment_variable_names(&self) -> Option<&HashSet<SmolStr>> {
        None
    }

    /// The active shell configuration if exists in the current context.
    fn shell_supports_autocd(&self) -> Option<bool> {
        None
    }

    /// Returns the command that an alias expands to, if one exists.
    ///
    /// It's generally incorrect to implement this and not [`CompletionContext::aliases`]
    fn alias_command(&self, _alias: &str) -> Option<&str> {
        None
    }

    /// Returns an iterator over all aliases and their commands.
    ///
    /// It's generally incorrect to implement this and not [`CompletionContext::alias_command`].
    fn aliases(&self) -> Box<dyn Iterator<Item = (&str, &str)> + '_> {
        Box::new(std::iter::empty())
    }

    /// Returns a map of abbreviations to command.
    fn abbreviations(&self) -> Option<&HashMap<SmolStr, String>> {
        None
    }

    /// Returns a set of functions.
    fn functions(&self) -> Option<&HashSet<SmolStr>> {
        None
    }

    /// Returns a set of shell builtins.
    fn builtins(&self) -> Option<&HashSet<SmolStr>> {
        None
    }

    fn shell_family(&self) -> Option<ShellFamily> {
        None
    }
}

/// Keeps track of which separators characters are relevant in file paths.
///
/// There is a [`std::path::MAIN_SEPARATOR`], but we usually can't read that. We need to be dynamic
/// in order to accommodate for sessions using a different separator from the system the app is
/// running on, e.g. WSL or MSYS2.
#[derive(Clone, Debug)]
pub struct PathSeparators {
    /// Analogous to [`std::path::MAIN_SEPARATOR`].
    pub main: char,
    /// Set of all valid separators, e.g. Windows recognizes both "/" and "\".
    pub all: &'static [char],
}

impl PathSeparators {
    const WINDOWS_SEPARATORS: &[char] = &['/', '\\'];
    const UNIX_SEPARATORS: &[char] = &['/'];

    pub fn for_os() -> Self {
        let main_separator = std::path::MAIN_SEPARATOR;
        Self {
            main: main_separator,
            all: match main_separator {
                '/' => Self::UNIX_SEPARATORS,
                '\\' => Self::WINDOWS_SEPARATORS,
                _ => panic!("unknown main path separator: {main_separator}"),
            },
        }
    }

    pub fn for_unix() -> Self {
        Self {
            main: '/',
            all: Self::UNIX_SEPARATORS,
        }
    }

    pub fn for_windows() -> Self {
        Self {
            main: '\\',
            all: Self::WINDOWS_SEPARATORS,
        }
    }
}

#[async_trait]
pub trait PathCompletionContext: Send + Sync {
    /// Implementations should return a vector of entries (files/subdirectories) in the given
    /// `directory`.
    async fn list_directory_entries(&self, directory: TypedPathBuf) -> Arc<Vec<EngineDirEntry>>;

    /// The "home" directory of the session.
    ///
    /// This is used to expand '~' and '$HOME' in user input.
    fn home_directory(&self) -> Option<&str>;

    /// `CDPATH`/`cdpath` from the shell, colon-separated. Used to extend `cd` argument completions.
    fn cdpath(&self) -> Option<&str> {
        None
    }

    fn shell_family(&self) -> ShellFamily;

    /// The current working directory, which is used to determine how relative path suggestions
    /// should be computed.
    fn pwd(&self) -> TypedPath<'_>;

    fn path_separators(&self) -> PathSeparators;
}

#[async_trait]
pub trait GeneratorContext: Send + Sync {
    /// Execute a given command at the active pwd. If no session exist in the source, return None.
    async fn execute_command_at_pwd(
        &self,
        _shell_command: &str,
        _session_env_vars: Option<HashMap<String, String>>,
    ) -> Result<CommandOutput>;

    /// Whether the implementation allows execution of generators in parallel.
    fn supports_parallel_execution(&self) -> bool;
}

#[derive(Debug)]
pub struct CommandOutput {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub status: CommandExitStatus,
    /// The exit code of the command. On Unix this can be None if the command was
    /// terminated by a signal.
    pub exit_code: Option<ExitCode>,
}

impl CommandOutput {
    pub fn success(&self) -> bool {
        self.status == CommandExitStatus::Success
    }

    // The output of the command, stdout if command was successful, stderr otherwise.
    pub fn output(&self) -> &Vec<u8> {
        if self.success() {
            &self.stdout
        } else {
            &self.stderr
        }
    }

    pub fn exit_code(&self) -> Option<ExitCode> {
        self.exit_code
    }

    pub fn to_string(&self) -> Result<String> {
        String::from_utf8(self.stdout.to_vec()).map_err(anyhow::Error::from)
    }
}

#[cfg(not(target_family = "wasm"))]
impl From<command::Output> for CommandOutput {
    fn from(other: command::Output) -> CommandOutput {
        let status = if other.status.success() {
            CommandExitStatus::Success
        } else {
            CommandExitStatus::Failure
        };
        CommandOutput {
            stdout: other.stdout,
            stderr: other.stderr,
            status,
            exit_code: other.status.code().map(ExitCode::from),
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum CommandExitStatus {
    Success,
    Failure,
}
