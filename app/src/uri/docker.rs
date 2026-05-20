use std::{collections::HashMap, fmt::Display};

use crate::{
    send_telemetry_from_app_ctx, server::telemetry::TelemetryEvent, terminal::shell::ShellType,
};
use regex::Regex;
use url::Url;
use warp_util::path::{is_posix_portable_pathname, ShellFamily};
use warpui::AppContext;

use crate::root_view::SubshellCommandArg;

use anyhow::{anyhow, Result};

/// String of hex digits meant to represent a Docker container ID.
#[derive(Debug)]
struct DockerContainerId(String);

impl TryFrom<String> for DockerContainerId {
    type Error = anyhow::Error;

    fn try_from(input: String) -> Result<Self, Self::Error> {
        // Note: We could do a further check to validate that this Docker container ID actually exists and/or is running.

        if input.is_empty() || input.len() > 64 {
            Err(anyhow!(
                "Docker container IDs must be between 1 and 64 bytes long"
            ))
        } else if input.chars().any(|c| !c.is_ascii_hexdigit()) {
            Err(anyhow!(
                "Could not find valid docker container id to open warpified shell"
            ))
        } else {
            Ok(DockerContainerId(input))
        }
    }
}

impl Display for DockerContainerId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Given a Url with query parameters in the correct format, dispatch an action to create a new tab
/// (or open a new window if there is no window), then run a command to open a subshell into the
/// specified Docker container, and then warpify that new subshell.
pub fn open_docker_container(url: &Url, ctx: &mut AppContext) -> Result<()> {
    let query_params: HashMap<String, String> = url
        .query_pairs()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();

    let container_id = query_params
        .get("container_id")
        .and_then(|container_id| DockerContainerId::try_from(container_id.to_owned()).ok())
        .ok_or(anyhow!("no valid container ID parameter found"))?;

    let shell_path = query_params
        .get("shell")
        // TODO(CORE-2658): Make this filter less restrictive without reducing security.
        .filter(|shell_path| is_posix_portable_pathname(shell_path))
        // TODO(CORE-2658): Our Docker extension lets users specify any shell, but we're only accepting
        // shells we can bootstrap. We should probably change the Docker extension to only surface
        // shells we can bootstrap.
        .filter(|shell_path| ShellType::from_name(shell_path).is_some())
        .ok_or(anyhow!("no valid shell parameter found"))?;

    // This NAME_REGEX specifies this format of linux user names. It's a very
    // common pattern, but some systems might have a different configuration.
    let username_pattern =
        Regex::new(r"^[a-z][-a-z0-9_]*\$?$").expect("NAME_REGEX should be valid.");

    let user = match query_params.get("user") {
        Some(user) if username_pattern.is_match(user.as_str()) => Some(user),
        Some(_) => anyhow::bail!("Invalid user parameter found."),
        None => None,
    };

    // Command example: docker exec -it --user 'admin' 'container_id' 'zsh'.
    // TODO(CORE-2658): This [`ShellFamily::shell_escape`] function is built with `bash` in mind but we need to
    // properly escape for all our officially supported shells.
    // Assume MacOS/Linux and therefore POSIX shell. Running Docker on Windows requires WSL anyway.
    let mut docker_exec_command = String::from("docker exec -it");
    if let Some(user) = user {
        docker_exec_command
            .push_str(format!(" --user '{}' ", ShellFamily::Posix.shell_escape(user)).as_str());
    }
    // We don't need to escape the container_id because we already checked that it had no special
    // characters.
    docker_exec_command.push_str(
        format!(
            " '{}' '{}'",
            container_id,
            ShellFamily::Posix.shell_escape(shell_path)
        )
        .as_str(),
    );

    let shell_type = ShellType::from_name(shell_path);

    // Opens a new window if there is none.
    ctx.dispatch_global_action(
        "root_view:open_new_tab_insert_subshell_command_and_bootstrap_if_supported",
        &SubshellCommandArg {
            command: docker_exec_command,
            shell_type,
        },
    );

    send_telemetry_from_app_ctx!(
        TelemetryEvent::OpenAndWarpifyDockerSubshell { shell_type },
        ctx
    );

    Ok(())
}

#[cfg(test)]
#[path = "docker_tests.rs"]
mod tests;
