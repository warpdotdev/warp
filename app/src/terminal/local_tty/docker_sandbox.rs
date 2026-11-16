use std::ffi::OsStr;
use std::path::PathBuf;

use futures::FutureExt as _;
use futures::future::BoxFuture;
pub use warp_terminal::local_tty::docker_sandbox::*;
use warpui::{AppContext, SingletonEntity as _};

use crate::terminal::local_shell::LocalShellState;
use crate::util::path::{resolve_executable, resolve_executable_in_path};
/// Resolves `sbx` using the PATH captured from the user's interactive login
/// shell, matching how MCP servers and LSP find binaries.
///
/// Falls back to the process's `PATH` if the interactive PATH capture
/// fails.
#[cfg(feature = "local_tty")]
pub fn resolve_sbx_path_from_user_shell(
    ctx: &mut AppContext,
) -> BoxFuture<'static, Option<PathBuf>> {
    let path_future = LocalShellState::handle(ctx).update(ctx, |shell_state, ctx| {
        shell_state.get_interactive_path_env_var(ctx)
    });
    async move {
        let path_env_var = path_future.await;
        let resolved = match path_env_var.as_deref() {
            Some(path) => resolve_executable_in_path("sbx", OsStr::new(path)),
            None => resolve_executable("sbx"),
        };
        resolved.map(|p| p.into_owned())
    }
    .boxed()
}
