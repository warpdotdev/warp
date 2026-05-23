use std::fmt;

use command_corrections::ExitCode as CommandCorrectionsExitCode;
use serde::{Deserialize, Serialize};

/// List of process exit codes that we consider to be "Success"
///
/// - 0 is the standard success exit code
/// - 130 is the exit code for when a process is quit by Ctrl-C
/// - 141 is for when a process is closed while piping output to a pager (e.g. `git log`)
/// - -1073741510 is exit code for when a process is aborted with `STATUS_CONTROL_C_EXIT` on
///   Windows. We don't gate this on OS because it's impossible to get a negative exit code in
///   Unix environments.
const SUCCESSFUL_EXIT_CODES: &[i32] = &[0, 130, 141, -1073741510];

/// This is a newtype for i32.
/// It is meant to cover
/// - POSIX systems where exit codes are u8
/// - Windows systems where exit codes are i32
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ExitCode(i32);

impl ExitCode {
    pub fn value(&self) -> i32 {
        self.0
    }

    /// Returns true if the command exited due to SIGINT, typically via ctrl-c.
    pub fn is_sigint(&self) -> bool {
        self.0 == 130
    }

    /// Returns true if the exit code indicates "command not found".
    /// - 127: Unix/Linux/macOS
    /// - 9009: Windows CMD
    pub fn was_command_not_found(&self) -> bool {
        self.0 == 127 || self.0 == 9009
    }

    /// Returns true if the error code indicates that the error code
    /// is successful from the perspective of us indicating in the
    /// ui that it is not in error:
    /// - 0 is the standard success exit code
    /// - 130 is the exit code for when a process is quit by Ctrl-C
    /// - 141 is for when a process is closed while piping output to a pager (e.g. `git log`)
    pub fn was_successful(&self) -> bool {
        SUCCESSFUL_EXIT_CODES.contains(&self.0)
    }
}

impl From<i32> for ExitCode {
    fn from(code: i32) -> Self {
        Self(code)
    }
}

impl From<CommandCorrectionsExitCode> for ExitCode {
    fn from(code: CommandCorrectionsExitCode) -> Self {
        Self::from(code.raw())
    }
}

impl From<ExitCode> for CommandCorrectionsExitCode {
    fn from(code: ExitCode) -> Self {
        Self::from(code.0)
    }
}

impl fmt::Display for ExitCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
