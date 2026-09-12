use std::ffi::OsStr;
use std::path::PathBuf;

use futures::FutureExt as _;
use futures::future::BoxFuture;
pub use warp_terminal::local_tty::dev_container::*;
use warpui::{AppContext, SingletonEntity as _};

use crate::terminal::local_shell::LocalShellState;
use crate::util::path::{resolve_executable, resolve_executable_in_path};

/// Name of the `@devcontainers/cli` binary we shell out to for `up`.
const DEVCONTAINER_CLI_BIN: &str = "devcontainer";

/// Name of the `docker` CLI binary we shell out to for the interactive
/// attach step. See [`super::shell::ShellStarter::DevContainer`] for why
/// attach goes through plain `docker exec` rather than `devcontainer exec`.
const DOCKER_CLI_BIN: &str = "docker";

/// Resolves a binary using the PATH captured from the user's interactive
/// login shell, matching how `sbx` is resolved for the Docker sandbox (see
/// [`super::docker_sandbox::resolve_sbx_path_from_user_shell`]).
///
/// Falls back to the process's `PATH` if the interactive PATH capture fails.
#[cfg(feature = "local_tty")]
fn resolve_cli_path_from_user_shell(
    bin_name: &'static str,
    ctx: &mut AppContext,
) -> BoxFuture<'static, Option<PathBuf>> {
    let path_future = LocalShellState::handle(ctx).update(ctx, |shell_state, ctx| {
        shell_state.get_interactive_path_env_var(ctx)
    });
    async move {
        let path_env_var = path_future.await;
        let resolved = match path_env_var.as_deref() {
            Some(path) => resolve_executable_in_path(bin_name, OsStr::new(path)),
            None => resolve_executable(bin_name),
        };
        resolved.map(|p| p.into_owned())
    }
    .boxed()
}

/// Resolves the `devcontainer` CLI (used for `devcontainer up`).
#[cfg(feature = "local_tty")]
pub fn resolve_devcontainer_cli_path(ctx: &mut AppContext) -> BoxFuture<'static, Option<PathBuf>> {
    resolve_cli_path_from_user_shell(DEVCONTAINER_CLI_BIN, ctx)
}

/// Resolves the `docker` CLI (used for the interactive attach step).
#[cfg(feature = "local_tty")]
pub fn resolve_docker_cli_path(ctx: &mut AppContext) -> BoxFuture<'static, Option<PathBuf>> {
    resolve_cli_path_from_user_shell(DOCKER_CLI_BIN, ctx)
}
