use std::collections::HashMap;

use anyhow::{anyhow, Result};
use async_trait::async_trait;

use super::{CommandExecutor, CommandOutput, ExecuteCommandOptions};
use crate::terminal::shell::Shell;

///  A "no-op" implementation of `CommandExecutor` to be used as a placeholder `CommandExecutor`
///  implementation for remote non-SSH subshell `Session`s when the user has disabled in-band
///  generators. Users may disable in-band generators by setting a 'hidden' user default (not
///  surfaced in Settings), purely as a "killswitch" if there is a particularly bad in-band
///  generators bug.
#[derive(Debug, Default)]
pub struct NoOpCommandExecutor {}

impl NoOpCommandExecutor {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl CommandExecutor for NoOpCommandExecutor {
    async fn execute_command(
        &self,
        _command: &str,
        _shell: &Shell,
        _current_directory_path: Option<&str>,
        _environment_variables: Option<HashMap<String, String>>,
        _execute_command_options: ExecuteCommandOptions,
    ) -> Result<CommandOutput> {
        Err(anyhow!(
            "Did not execute command; using NoOpCommandExecutor"
        ))
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn supports_parallel_command_execution(&self) -> bool {
        false
    }
}
