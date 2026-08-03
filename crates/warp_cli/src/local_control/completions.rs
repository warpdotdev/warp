//! Shell completion generation for `warpctrl`.
use clap_complete::aot::{Shell, generate};
use local_control::protocol::{ControlError, ErrorCode};

use crate::local_control::ControlArgs;

pub(super) fn generate_completions_to_stdout(shell: Option<Shell>) -> Result<(), ControlError> {
    let shell = shell.or_else(Shell::from_env).ok_or_else(|| {
        ControlError::new(
            ErrorCode::InvalidParams,
            "could not determine shell from environment; provide a shell argument",
        )
    })?;
    let mut cmd = ControlArgs::clap_command();
    let bin_name = crate::binary_name().unwrap_or_else(|| "warpctrl".to_owned());
    generate(shell, &mut cmd, bin_name, &mut std::io::stdout());
    Ok(())
}

#[cfg(test)]
pub(crate) fn generate_completion_string(shell: Shell) -> Result<String, ControlError> {
    let mut cmd = ControlArgs::clap_command();
    let mut output = Vec::new();
    generate(shell, &mut cmd, "warpctrl", &mut output);
    String::from_utf8(output).map_err(|err| {
        ControlError::with_details(
            ErrorCode::Internal,
            "failed to render local-control completions",
            err.to_string(),
        )
    })
}
