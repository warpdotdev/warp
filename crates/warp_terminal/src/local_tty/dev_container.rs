//! Dev Container-specific shell-starter types and helpers.
//!
//! This module owns everything specific to running a Warp shell inside a
//! container that `devcontainer up` (from `@devcontainers/cli`) has already
//! brought up: the [`DevContainerShellStarter`] that carries per-instance
//! state and the init-script staging/copy layout.
//!
//! Bringing the container up is *not* this module's concern — that happens
//! before a `DevContainerShellStarter` is ever constructed, driven from
//! `crate::terminal::view::dev_container`, with its own progress/failure UI.
//! This module only knows how to attach a shell to a container that is
//! already running, mirroring [`super::docker_sandbox`]'s split between
//! sandbox lifecycle and the [`super::shell::ShellStarter::DockerSandbox`]
//! variant.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use warp_core::SessionId;

use super::shell::DirectShellStarter;
use crate::shell::ShellType;

/// Root directory on the host under which Dev Container scratch files (bash
/// init scripts, staged before `docker cp`) live.
///
/// Lives under the Warp per-user cache directory for the same reasons as
/// [`super::docker_sandbox::docker_sandbox_host_root`]: protected by the
/// user's home-directory permissions. Unlike the Docker sandbox, this
/// directory is never mounted into a container — see
/// [`DevContainerShellStarter::new`] for why a bind mount doesn't work here.
fn dev_container_host_root() -> PathBuf {
    warp_core::paths::cache_dir().join("dev-container")
}

/// Generates a fresh sandbox ID: 8 hex chars (32 bits), plenty for realistic
/// concurrent session counts and keeps paths readable.
pub fn generate_sandbox_id() -> String {
    format!("{:08x}", rand::random::<u32>())
}

/// Host path where Warp stages a Dev Container session's bash init script
/// before copying it into the container with `docker cp`, keyed by
/// `sandbox_id` so concurrent Warp panes don't collide.
pub fn host_init_script_path_for_sandbox_id(sandbox_id: &str) -> PathBuf {
    dev_container_host_root()
        .join("init")
        .join(format!("{sandbox_id}.sh"))
}

/// Path *inside the container* that the init script is copied to, and that
/// gets passed to `bash --rcfile`. Lives under `/tmp`, which is writable in
/// essentially every dev container image regardless of the configured user.
pub fn container_init_script_path_for_sandbox_id(sandbox_id: &str) -> String {
    format!("/tmp/.warp-devcontainer-init-{sandbox_id}.sh")
}

/// Host path where Warp stages the Dev Container bootstrap script (the same
/// script content used for local shells) before copying it into a container
/// with `docker cp`. Keyed by a hash of the script's own contents rather
/// than by session: unlike the init script, the bootstrap script takes no
/// session-specific input, so every session on a given Warp build stages
/// and copies the exact same bytes. Keying by content hash makes that copy
/// write-once-per-build instead of accumulating a fresh ~80KB file per
/// session in a long-lived container.
pub fn host_bootstrap_script_path_for_content_hash(content_hash: &str) -> PathBuf {
    dev_container_host_root()
        .join("init")
        .join(format!("bootstrap-{content_hash}.sh"))
}

/// Path *inside the container* that the bootstrap script is copied to. The
/// init script's `--rcfile` content `source`s this path directly, so the
/// large bootstrap script never has to be typed into the live pty. Keyed
/// the same way as [`host_bootstrap_script_path_for_content_hash`].
pub fn container_bootstrap_script_path_for_content_hash(content_hash: &str) -> String {
    format!("/tmp/.warp-devcontainer-bootstrap-{content_hash}.sh")
}

/// Wraps a [`DirectShellStarter`] and adds Dev Container-specific parameters.
///
/// Each instance carries a unique `sandbox_id` so multiple Warp panes
/// attaching to the same (or different) Dev Containers don't collide on the
/// staged/copied init-script path.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DevContainerShellStarter {
    pub direct: DirectShellStarter,
    /// Host directory containing `.devcontainer/devcontainer.json`, kept for
    /// display purposes only.
    pub workspace_folder: PathBuf,
    /// Container ID reported by `devcontainer up`, passed to `docker exec`.
    pub container_id: String,
    /// Remote user reported by `devcontainer up` (`docker exec -u`), if any.
    pub remote_user: Option<String>,
    /// Workspace folder inside the container reported by `devcontainer up`
    /// (`docker exec -w`).
    pub remote_workspace_folder: String,
    /// Unique per-instance ID used to derive the init-script staging and
    /// in-container paths. Generated at construction time; see [`Self::new`].
    pub sandbox_id: String,
    /// The client-generated session ID injected into this container's init script.
    pub session_id: SessionId,
}

impl DevContainerShellStarter {
    /// Construct a new starter for the given `sandbox_id`.
    ///
    /// `sandbox_id` need not match anything from the `devcontainer up` step: the
    /// init script is delivered via `docker cp` immediately before `docker exec`
    /// runs, not a bind mount configured when the container was created. A bind
    /// mount only takes effect the *first* time a container is created;
    /// `devcontainer up` reuses an existing container (matched by workspace
    /// label) on subsequent invocations without re-applying `--mount`, so a
    /// fresh per-invocation mount path would silently stop existing inside the
    /// container after the first session. `docker cp` has no such lifecycle
    /// coupling — it works against any running container regardless of how it
    /// was created.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        direct: DirectShellStarter,
        workspace_folder: PathBuf,
        container_id: String,
        remote_user: Option<String>,
        remote_workspace_folder: String,
        sandbox_id: String,
    ) -> Self {
        let session_id = direct.session_id();
        Self {
            direct,
            workspace_folder,
            container_id,
            remote_user,
            remote_workspace_folder,
            sandbox_id,
            session_id,
        }
    }

    pub fn shell_type(&self) -> ShellType {
        self.direct.shell_type()
    }

    pub fn logical_shell_path(&self) -> &std::path::Path {
        self.direct.logical_shell_path()
    }

    pub fn display_name(&self) -> &str {
        self.direct.display_name()
    }

    pub fn session_id(&self) -> SessionId {
        self.session_id
    }

    /// Host path where this session's init script is staged before
    /// `docker cp`.
    pub fn host_init_script_path(&self) -> PathBuf {
        host_init_script_path_for_sandbox_id(&self.sandbox_id)
    }

    /// Path inside the container the init script is copied to.
    pub fn container_init_script_path(&self) -> String {
        container_init_script_path_for_sandbox_id(&self.sandbox_id)
    }
}
