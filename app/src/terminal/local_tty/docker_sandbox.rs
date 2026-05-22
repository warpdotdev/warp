//! Docker-sandbox-specific shell-starter types and helpers.
//!
//! This module owns everything specific to running a Warp shell inside a
//! `sbx`-managed Docker sandbox: the [`DockerSandboxShellStarter`] that
//! carries per-instance state, the host-side mount-point layout, and the
//! `sbx` binary resolution logic.
//!
//! The generic [`super::shell::ShellStarter`] enum references
//! [`DockerSandboxShellStarter`] from its `DockerSandbox` variant, but the
//! bulk of the sandbox-specific surface lives here so `shell.rs` can stay
//! focused on the cross-shell abstraction.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use futures::future::BoxFuture;
use futures::FutureExt as _;
use serde::{Deserialize, Serialize};
use warpui::{AppContext, SingletonEntity as _};

use super::shell::DirectShellStarter;
#[cfg(feature = "local_tty")]
use crate::terminal::local_shell::LocalShellState;
use crate::terminal::shell::ShellType;
use crate::util::path::{resolve_executable, resolve_executable_in_path};

/// Default home directory for the sandbox user inside the shell template.
/// Lives inside the container image and is shared across all sandboxes, so it
/// doesn't need to be per-instance.
pub const DOCKER_SANDBOX_HOME_DIR: &str = "/home/agent";

/// Prefix for generated container names: `warp-sandbox-<id>`.
const DOCKER_SANDBOX_NAME_PREFIX: &str = "warp-sandbox";

/// Root directory on the host under which Docker-sandbox scratch files
/// (bash init scripts, empty workspace mount points) live.
///
/// Lives under the Warp per-user cache directory rather than `/tmp` so:
/// - other users on a multi-user host can't pre-create or symlink-attack the
///   mount path,
/// - file contents are protected by the user's home-directory permissions
///   (the per-sandbox subdirectories below are additionally created with
///   mode 0700).
///
/// Layout: `<cache_dir>/docker-sandbox/{init,workspace}/<sandbox_id>/`.
fn docker_sandbox_host_root() -> PathBuf {
    warp_core::paths::cache_dir().join("docker-sandbox")
}

/// Resolves the absolute path to the `sbx` CLI binary using the Warp
/// process's `PATH`.
///
/// Warp's process `PATH` is minimal and often misses user-shell-installed
/// tools (e.g. homebrew on Apple Silicon when Warp is launched from Finder,
/// or `~/.local/bin`). Prefer [`resolve_sbx_path_from_user_shell`], which
/// captures the PATH from the user's interactive login shell, the same way
/// MCP servers and LSP resolve binaries.
pub fn resolve_sbx_path() -> Option<PathBuf> {
    resolve_executable("sbx").map(|p| p.into_owned())
}

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

/// Wraps a [`DirectShellStarter`] and adds Docker-sandbox-specific parameters.
///
/// Each instance carries a unique `sandbox_id` so multiple Warp panes can run
/// independent sandboxes concurrently without colliding on container name or
/// on the host-side init / workspace mount directories. The base Docker image
/// is threaded down from the AvailableShell used to initialize this starter
/// so that [`super::unix::spawn`] can pass it to `sbx run` via the
/// `--template` flag.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockerSandboxShellStarter {
    pub direct: DirectShellStarter,
    /// Base Docker image for the sandbox. `None` means "use sbx's default
    /// image".
    pub base_image: Option<String>,
    /// Unique per-instance ID used to derive the container name and host mount
    /// paths. Generated at construction time; see [`Self::new`].
    pub sandbox_id: String,
}

impl DockerSandboxShellStarter {
    /// Construct a new starter with a freshly generated `sandbox_id`.
    pub fn new(direct: DirectShellStarter, base_image: Option<String>) -> Self {
        // Short random ID — 8 hex chars (32 bits) is plenty for realistic
        // concurrent sandbox counts and keeps container names readable.
        let sandbox_id = format!("{:08x}", rand::random::<u32>());
        Self {
            direct,
            base_image,
            sandbox_id,
        }
    }

    pub fn shell_type(&self) -> ShellType {
        self.direct.shell_type()
    }

    pub fn logical_shell_path(&self) -> &Path {
        self.direct.logical_shell_path()
    }

    pub fn display_name(&self) -> &str {
        self.direct.display_name()
    }

    pub fn base_image(&self) -> Option<&str> {
        self.base_image.as_deref()
    }

    /// Name passed to `sbx run --name`. Unique per instance.
    pub fn sandbox_name(&self) -> String {
        format!("{DOCKER_SANDBOX_NAME_PREFIX}-{}", self.sandbox_id)
    }

    /// Host directory where Warp writes this sandbox's bash init script.
    /// Mounted read-only into the container at the same absolute path.
    pub fn init_dir(&self) -> PathBuf {
        docker_sandbox_host_root()
            .join("init")
            .join(&self.sandbox_id)
    }

    /// Full path to this sandbox's `init.sh` on the host (also valid inside
    /// the container once mounted).
    pub fn init_path(&self) -> PathBuf {
        self.init_dir().join("init.sh")
    }

    /// Dedicated empty host workspace for this sandbox, used to satisfy
    /// `sbx run shell`'s required primary-workspace positional arg without
    /// exposing the user's current working tree or home directory.
    pub fn workspace_dir(&self) -> PathBuf {
        docker_sandbox_host_root()
            .join("workspace")
            .join(&self.sandbox_id)
    }
}
