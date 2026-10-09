//! Prototype `/devcontainer` flow.
//!
//! Unlike the Docker sandbox (which creates its container as a side effect of
//! spawning the PTY), Dev Container lifecycle is explicitly kept off the PTY
//! spawn path: `devcontainer up` can take minutes (image pull, build,
//! `postCreateCommand`), so it runs here, before any pane exists, with a
//! real toast showing progress and a real error toast on failure. Only after
//! `devcontainer up` reports success do we create a pane, using a
//! `ShellStarter::DevContainer` that assumes the container is already
//! running (see `crate::terminal::local_tty::dev_container`).
#[cfg(all(feature = "local_tty", not(feature = "remote_tty")))]
use std::collections::HashMap;
#[cfg(feature = "local_tty")]
use std::path::{Path, PathBuf};
#[cfg(feature = "local_tty")]
use std::sync::mpsc::SyncSender;

#[cfg(feature = "local_tty")]
use command::r#async::Command;
#[cfg(feature = "local_tty")]
use futures::FutureExt as _;
#[cfg(feature = "local_tty")]
use serde::Deserialize;
#[cfg(feature = "local_tty")]
use warp_core::SessionId;
#[cfg(feature = "local_tty")]
use warpui::ModelHandle;
use warpui::ViewContext;
#[cfg(feature = "local_tty")]
use warpui::geometry::vector::Vector2F;
#[cfg(not(target_family = "wasm"))]
use warpui::{SingletonEntity, ViewHandle};

use super::TerminalView;
#[cfg(all(feature = "local_tty", not(feature = "remote_tty")))]
use crate::banner::BannerState;
#[cfg(feature = "local_tty")]
use crate::pane_group::TerminalViewResources;
#[cfg(feature = "local_tty")]
use crate::persistence::ModelEvent;
#[cfg(feature = "local_tty")]
use crate::server::server_api::ServerApiProvider;
#[cfg(feature = "local_tty")]
use crate::terminal::TerminalManager;
#[cfg(all(feature = "local_tty", not(feature = "remote_tty")))]
use crate::terminal::available_shells::AvailableShell;
#[cfg(feature = "local_tty")]
use crate::terminal::bootstrap::generate_session_id;
#[cfg(feature = "local_tty")]
use crate::terminal::local_tty::dev_container::{
    generate_sandbox_id, resolve_devcontainer_cli_path, resolve_docker_cli_path,
};
#[cfg(all(feature = "local_tty", not(feature = "remote_tty")))]
use crate::terminal::local_tty::{
    TerminalManager as LocalTtyTerminalManager, TerminalViewSurfaceConfig,
    create_terminal_view_surface,
};
#[cfg(feature = "remote_tty")]
use crate::terminal::remote_tty::TerminalManager as RemoteTtyTerminalManager;
#[cfg(all(feature = "local_tty", not(feature = "remote_tty")))]
use crate::terminal::shared_session::IsSharedSessionCreator;
#[cfg(feature = "local_tty")]
use crate::view_components::{DismissibleToast, ToastFlavor};
#[cfg(feature = "local_tty")]
use crate::workspace::ToastStack;

/// Object ID shared by every toast in a single `/devcontainer` invocation, so
/// the "building" toast is automatically replaced by the eventual
/// success/error toast instead of stacking.
#[cfg(feature = "local_tty")]
const DEV_CONTAINER_TOAST_OBJECT_ID: &str = "dev-container-build";

/// The last line of `devcontainer up --workspace-folder <dir>` stdout is a
/// single JSON object reporting the outcome. See:
/// <https://github.com/devcontainers/cli>
///
/// On success this also carries what we need to attach with plain `docker
/// exec` afterward (`containerId`/`remoteUser`/`remoteWorkspaceFolder`); see
/// [`crate::terminal::shell::ShellLaunchData::DevContainer`] for why we
/// don't use `devcontainer exec` for that step.
#[cfg(feature = "local_tty")]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DevContainerUpResult {
    outcome: String,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    container_id: Option<String>,
    #[serde(default)]
    remote_user: Option<String>,
    #[serde(default)]
    remote_workspace_folder: Option<String>,
}

#[cfg(feature = "local_tty")]
#[allow(unused_variables, clippy::too_many_arguments)]
fn create_dev_container_view(
    resources: TerminalViewResources,
    initial_size: Vector2F,
    model_event_sender: Option<SyncSender<ModelEvent>>,
    #[allow(dead_code)] workspace_folder: PathBuf,
    #[allow(dead_code)] docker_path: PathBuf,
    #[allow(dead_code)] container_id: String,
    #[allow(dead_code)] remote_user: Option<String>,
    #[allow(dead_code)] remote_workspace_folder: String,
    #[allow(dead_code)] sandbox_id: String,
    #[allow(dead_code)] session_id: SessionId,
    ctx: &mut ViewContext<TerminalView>,
) -> (
    ViewHandle<TerminalView>,
    ModelHandle<Box<dyn TerminalManager>>,
) {
    cfg_if::cfg_if! {
        if #[cfg(feature = "remote_tty")] {
            let terminal_init = RemoteTtyTerminalManager::create_model(
                resources,
                initial_size,
                model_event_sender,
                ctx.window_id(),
                None, /* initial_input_config */
                ctx,
            );
            let terminal_manager = terminal_init.manager;
            let terminal_view = terminal_init.view;
        } else {
            let user_default_shell_unsupported_banner_model_handle =
                ctx.add_model(|_| BannerState::default());

            let chosen_shell = Some(AvailableShell::new_dev_container_shell(
                workspace_folder,
                docker_path,
                container_id,
                remote_user,
                remote_workspace_folder,
                sandbox_id,
                session_id,
            ));

            let model_event_sender_for_surface = model_event_sender.clone();
            let window_id = ctx.window_id();
            let terminal_init = LocalTtyTerminalManager::<TerminalView>::create_model(
                None,
                HashMap::new(),
                IsSharedSessionCreator::No,
                None, /* restored_blocks */
                user_default_shell_unsupported_banner_model_handle,
                initial_size,
                model_event_sender,
                chosen_shell,
                ctx,
                |surface_init, ctx| {
                    create_terminal_view_surface(
                        TerminalViewSurfaceConfig {
                            resources,
                            model_event_sender: model_event_sender_for_surface,
                            window_id,
                            initial_input_config: None,
                            conversation_restoration: None,
                            has_conversation_restoration: false,
                            is_historical: false,
                            should_use_live_appearance: false,
                            has_restored_command_blocks: false,
                        },
                        surface_init,
                        ctx,
                    )
                },
            );
            let terminal_manager = terminal_init.manager;
            let terminal_view = terminal_init.surface;
        }
    }

    (terminal_view, terminal_manager)
}

impl TerminalView {
    /// Entry point for the `/devcontainer` slash command.
    ///
    /// Discovers `devcontainer.json` candidates for the active session's directory (see
    /// [`discover_dev_container_configs`]). With none, shows an error toast. With exactly one,
    /// proceeds directly, same as if there were only ever one supported config. With more than
    /// one, opens the inline Dev Container config selector and waits for a choice (see
    /// [`crate::terminal::input::Input::open_dev_container_config_selector`]); the eventual
    /// selection re-enters this flow via [`Self::resolve_dev_container_cli_and_bring_up`]. Never
    /// opens a pane itself: a pane only appears once the container is confirmed running.
    pub(crate) fn find_and_start_dev_container(&self, ctx: &mut ViewContext<Self>) {
        #[cfg(feature = "local_tty")]
        {
            let Some(workspace_folder) =
                self.canonical_session_pwd_if_local(ctx).map(PathBuf::from)
            else {
                self.show_dev_container_toast(
                    "Couldn't determine this session's directory; cd into a local project first."
                        .to_owned(),
                    ToastFlavor::Error,
                    ctx,
                );
                return;
            };

            let mut configs = discover_dev_container_configs(&workspace_folder);
            match configs.len() {
                0 => {
                    self.show_dev_container_toast(
                        format!(
                            "No devcontainer.json found in {} (checked \
                             .devcontainer/devcontainer.json, \
                             .devcontainer/*/devcontainer.json, and .devcontainer.json)",
                            workspace_folder.display()
                        ),
                        ToastFlavor::Error,
                        ctx,
                    );
                }
                1 => {
                    let config_path = configs.pop().expect("len() == 1");
                    self.resolve_dev_container_cli_and_bring_up(workspace_folder, config_path, ctx);
                }
                _ => {
                    self.input.update(ctx, |input, ctx| {
                        input.open_dev_container_config_selector(workspace_folder, configs, ctx);
                    });
                }
            }
        }
        #[cfg(not(feature = "local_tty"))]
        {
            let _ = ctx;
            log::warn!("Dev Container requires the `local_tty` feature; ignoring request");
        }
    }

    /// Resolves the `devcontainer`/`docker` CLI paths and shows the "building" toast, then hands
    /// off to [`Self::bring_up_dev_container`] for `config_path`. Shared by the single-config
    /// path in [`Self::find_and_start_dev_container`] and by the config selector's `Selected`
    /// event (routed here via `InputEvent::DevContainerConfigSelected`).
    #[cfg(feature = "local_tty")]
    pub(crate) fn resolve_dev_container_cli_and_bring_up(
        &self,
        workspace_folder: PathBuf,
        config_path: PathBuf,
        ctx: &mut ViewContext<Self>,
    ) {
        self.show_dev_container_toast(
            format!(
                "Building dev container for {}… this can take a few minutes.",
                workspace_folder.display()
            ),
            ToastFlavor::Default,
            ctx,
        );

        let devcontainer_cli_future = resolve_devcontainer_cli_path(ctx);
        let docker_cli_future = resolve_docker_cli_path(ctx);
        ctx.spawn(
            async move { (devcontainer_cli_future.await, docker_cli_future.await) },
            move |me, (devcontainer_path, docker_path), ctx| {
                let Some(devcontainer_path) = devcontainer_path else {
                    me.show_dev_container_toast(
                        "devcontainer CLI not found on PATH. Install it with \
                         `npm install -g @devcontainers/cli` and try again."
                            .to_owned(),
                        ToastFlavor::Error,
                        ctx,
                    );
                    return;
                };
                let Some(docker_path) = docker_path else {
                    me.show_dev_container_toast(
                        "docker CLI not found on PATH.".to_owned(),
                        ToastFlavor::Error,
                        ctx,
                    );
                    return;
                };
                me.bring_up_dev_container(
                    workspace_folder,
                    config_path,
                    devcontainer_path,
                    docker_path,
                    ctx,
                );
            },
        );
    }

    /// Runs `devcontainer up` for `workspace_folder` against `config_path`. Only opens a pane
    /// once `up` reports success *and* [`Self::preflight_and_attach_dev_container`] has both
    /// verified the container is attachable and staged the init/bootstrap scripts into it; shows
    /// an error toast (never a pane) otherwise.
    #[cfg(feature = "local_tty")]
    fn bring_up_dev_container(
        &self,
        workspace_folder: PathBuf,
        config_path: PathBuf,
        devcontainer_path: PathBuf,
        docker_path: PathBuf,
        ctx: &mut ViewContext<Self>,
    ) {
        let sandbox_id = generate_sandbox_id();
        // Generated here, before the container is even confirmed attachable,
        // so the same ID can be baked into the init/bootstrap scripts staged
        // by the preflight step below *and* into the `DevContainerShellStarter`
        // eventually constructed for the pane; the terminal model validates
        // the `InitShell` hook's session ID against the one it's told to
        // expect, so the two must match.
        let session_id = generate_session_id();

        let up_future = {
            let devcontainer_path = devcontainer_path.clone();
            let workspace_folder = workspace_folder.clone();
            async move {
                Command::new(&devcontainer_path)
                    .arg("up")
                    .arg("--workspace-folder")
                    .arg(&workspace_folder)
                    .arg("--config")
                    .arg(&config_path)
                    .output()
                    .await
            }
        };

        ctx.spawn(up_future, move |me, result, ctx| match result {
            Ok(output) => match interpret_dev_container_up_output(
                output.status.success(),
                &output.stdout,
                &output.stderr,
            ) {
                DevContainerUpOutcome::ReadyToAttach {
                    container_id,
                    remote_user,
                    remote_workspace_folder,
                } => {
                    me.preflight_and_attach_dev_container(
                        workspace_folder,
                        docker_path,
                        container_id,
                        remote_user,
                        remote_workspace_folder,
                        sandbox_id,
                        session_id,
                        ctx,
                    );
                }
                DevContainerUpOutcome::Error(message) => {
                    me.show_dev_container_toast(message, ToastFlavor::Error, ctx);
                }
            },
            Err(e) => {
                me.show_dev_container_toast(
                    format!("Failed to run `devcontainer up`: {e}"),
                    ToastFlavor::Error,
                    ctx,
                );
            }
        });
    }

    /// Checks that the container is actually attachable, then stages the
    /// init and bootstrap scripts into it — *before* creating a pane.
    /// Without this, either failure would only surface once `bash --rcfile`
    /// is already running inside a freshly created pane that then
    /// immediately exits, well past the point where an error toast with no
    /// pane is still possible.
    ///
    /// The attachability check verifies the container has both `script` and
    /// `bash` ([`crate::terminal::local_tty::unix::dev_container_exec_args`]'s
    /// attach mechanism depends on both, and neither is guaranteed present
    /// in every image), and that `remote_user`/`remote_workspace_folder`
    /// (using the exact same `-u`/`-w` the real attach uses) are actually
    /// valid in the container.
    ///
    /// Staging (see [`crate::terminal::local_tty::prepare_dev_container`])
    /// copies both scripts in with `docker cp` and secures their
    /// permissions; the eventual `bash --rcfile` session sources them
    /// directly from the container, so nothing is typed into the live pty.
    #[cfg(feature = "local_tty")]
    #[allow(clippy::too_many_arguments)]
    fn preflight_and_attach_dev_container(
        &self,
        workspace_folder: PathBuf,
        docker_path: PathBuf,
        container_id: String,
        remote_user: Option<String>,
        remote_workspace_folder: String,
        sandbox_id: String,
        session_id: SessionId,
        ctx: &mut ViewContext<Self>,
    ) {
        let preflight_future = {
            let docker_path = docker_path.clone();
            let container_id = container_id.clone();
            let remote_user = remote_user.clone();
            let remote_workspace_folder = remote_workspace_folder.clone();
            async move {
                Command::new(&docker_path)
                    .args(dev_container_preflight_args(
                        &container_id,
                        remote_user.as_deref(),
                        &remote_workspace_folder,
                    ))
                    .output()
                    .await
            }
        };

        ctx.spawn(preflight_future, move |me, result, ctx| match result {
            Ok(output) if output.status.success() => {
                // Staging is several sequential `docker cp`/`exec` round trips (a
                // default-user probe plus two copy+chown+chmod sequences), so it must
                // run as its own spawned future rather than be awaited inline here:
                // this closure runs on the main thread, and blocking it for that long
                // right after `devcontainer up` would freeze the UI.
                #[cfg(unix)]
                let staging_future = crate::terminal::local_tty::prepare_dev_container(
                    docker_path.clone(),
                    container_id.clone(),
                    remote_user.clone(),
                    sandbox_id.clone(),
                    session_id,
                )
                .boxed();
                #[cfg(not(unix))]
                let staging_future = futures::future::ready(Err(anyhow::Error::msg(
                    "Dev Container sessions are only supported on Linux and macOS",
                )))
                .boxed();

                ctx.spawn(
                    staging_future,
                    move |me, staging_result, ctx| match staging_result {
                        Ok(()) => {
                            me.show_dev_container_toast(
                                format!(
                                    "Dev container ready — opening session in {}…",
                                    workspace_folder.display()
                                ),
                                ToastFlavor::Success,
                                ctx,
                            );
                            me.create_and_push_dev_container(
                                workspace_folder,
                                docker_path,
                                container_id,
                                remote_user,
                                remote_workspace_folder,
                                sandbox_id,
                                session_id,
                                ctx,
                            );
                        }
                        Err(e) => {
                            me.show_dev_container_toast(
                                format!("Failed to prepare the Dev Container session: {e:#}"),
                                ToastFlavor::Error,
                                ctx,
                            );
                        }
                    },
                );
            }
            Ok(output) => {
                let detail = tail_lines(&String::from_utf8_lossy(&output.stderr), 5);
                me.show_dev_container_toast(
                    format!(
                        "Dev container isn't ready to attach: it may be missing `bash`, or its \
                         configured remote user or workspace folder may not exist: {detail}"
                    ),
                    ToastFlavor::Error,
                    ctx,
                );
            }
            Err(e) => {
                me.show_dev_container_toast(
                    format!("Failed to verify the Dev Container is ready to attach: {e}"),
                    ToastFlavor::Error,
                    ctx,
                );
            }
        });
    }

    #[cfg(feature = "local_tty")]
    #[allow(clippy::too_many_arguments)]
    fn create_and_push_dev_container(
        &self,
        workspace_folder: PathBuf,
        docker_path: PathBuf,
        container_id: String,
        remote_user: Option<String>,
        remote_workspace_folder: String,
        sandbox_id: String,
        session_id: SessionId,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(pane_stack) = self
            .pane_stack
            .as_ref()
            .and_then(|stack| stack.upgrade(ctx))
        else {
            log::warn!("Pane stack not available, cannot create dev container session");
            return;
        };

        let resources = TerminalViewResources {
            tips_completed: self.tips_completed.clone(),
            server_api: ServerApiProvider::as_ref(ctx).get(),
            model_event_sender: self.model_event_sender.clone(),
        };
        let pane_configuration = self.pane_configuration().clone();

        let (terminal_view, terminal_manager) = create_dev_container_view(
            resources,
            self.size_info().pane_size_px(),
            self.model_event_sender.clone(),
            workspace_folder,
            docker_path,
            container_id,
            remote_user,
            remote_workspace_folder,
            sandbox_id,
            session_id,
            ctx,
        );

        terminal_view.update(ctx, |view, _| {
            view.set_pane_configuration(pane_configuration);
        });

        pane_stack.update(ctx, |stack, ctx| {
            stack.push(terminal_manager, terminal_view, ctx);
        });

        ctx.notify();
    }

    #[cfg(feature = "local_tty")]
    fn show_dev_container_toast(
        &self,
        text: String,
        flavor: ToastFlavor,
        ctx: &mut ViewContext<Self>,
    ) {
        let window_id = ctx.window_id();
        ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
            let toast = DismissibleToast::new(text, flavor)
                .with_object_id(DEV_CONTAINER_TOAST_OBJECT_ID.to_owned());
            toast_stack.add_persistent_toast(toast, window_id, ctx);
        });
    }
}

/// The outcome of a completed `devcontainer up` invocation, once its process
/// exit status and JSON status line have both been interpreted.
#[cfg(feature = "local_tty")]
#[cfg_attr(test, derive(Debug, PartialEq))]
enum DevContainerUpOutcome {
    /// `up` succeeded and reported enough to attach to.
    ReadyToAttach {
        container_id: String,
        remote_user: Option<String>,
        remote_workspace_folder: String,
    },
    /// `up` failed, or succeeded without reporting what's needed to attach.
    /// Carries the user-facing error message.
    Error(String),
}

/// Interprets a completed `devcontainer up` process's exit status and
/// stdout/stderr. Pulled out of [`TerminalView::bring_up_dev_container`] as a
/// pure function so the partial-result and JSON-fallback cases are unit
/// testable without actually running `devcontainer`.
///
/// Takes the exit status and byte streams as plain primitives, rather than a
/// `std::process::Output`, so callers (including tests) don't need a real,
/// platform-specific `ExitStatus` (which has no portable public constructor)
/// just to exercise this logic.
#[cfg(feature = "local_tty")]
fn interpret_dev_container_up_output(
    exit_success: bool,
    stdout: &[u8],
    stderr: &[u8],
) -> DevContainerUpOutcome {
    if !exit_success {
        return DevContainerUpOutcome::Error(dev_container_up_failure_message(stdout, stderr));
    }
    match parse_dev_container_up_stdout(stdout) {
        Some(up_result) if up_result.outcome == "success" => {
            match (up_result.container_id, up_result.remote_workspace_folder) {
                (Some(container_id), Some(remote_workspace_folder)) => {
                    DevContainerUpOutcome::ReadyToAttach {
                        container_id,
                        remote_user: up_result.remote_user,
                        remote_workspace_folder,
                    }
                }
                _ => DevContainerUpOutcome::Error(
                    "Dev container started, but `devcontainer up` didn't report a container ID \
                     or workspace folder to attach to."
                        .to_owned(),
                ),
            }
        }
        _ => DevContainerUpOutcome::Error(dev_container_up_failure_message(stdout, stderr)),
    }
}

/// Builds the error-toast message for a failed (or unparseable) `devcontainer
/// up`, preferring the structured `message`/`description` from its final
/// JSON status line and falling back to the tail of stderr when that's
/// unavailable.
#[cfg(feature = "local_tty")]
fn dev_container_up_failure_message(stdout: &[u8], stderr: &[u8]) -> String {
    let structured_message = parse_dev_container_up_stdout(stdout).and_then(|result| {
        result
            .message
            .or(result.description)
            .map(|detail| format!("Dev container failed to start: {detail}"))
    });
    structured_message.unwrap_or_else(|| {
        let stderr_text = String::from_utf8_lossy(stderr);
        let tail = tail_lines(&stderr_text, 20);
        format!("Dev container failed to start:\n{tail}")
    })
}

/// Args for a preflight `docker exec` that checks the same things the real
/// attach in `crate::terminal::local_tty::unix::dev_container_exec_args`
/// needs to succeed, run with the exact same `-u`/`-w` it uses: that the
/// container has `bash` (not guaranteed present in every image), and that
/// `remote_user` and `remote_workspace_folder` (if set) are actually valid
/// in the container — an unqualified `-u <bad user>` or `-w <missing dir>`
/// fails a `docker exec` the same way `sh` not finding `bash` does: a
/// non-zero exit with no pane created.
#[cfg(feature = "local_tty")]
fn dev_container_preflight_args(
    container_id: &str,
    remote_user: Option<&str>,
    remote_workspace_folder: &str,
) -> Vec<std::ffi::OsString> {
    let mut args = vec![std::ffi::OsString::from("exec")];
    if let Some(remote_user) = remote_user {
        args.push(std::ffi::OsString::from("-u"));
        args.push(std::ffi::OsString::from(remote_user));
    }
    args.extend([
        std::ffi::OsString::from("-w"),
        std::ffi::OsString::from(remote_workspace_folder),
        std::ffi::OsString::from(container_id),
        std::ffi::OsString::from("sh"),
        std::ffi::OsString::from("-c"),
        std::ffi::OsString::from("command -v bash"),
    ]);
    args
}

/// Parses the final JSON status line that `devcontainer up` writes to
/// stdout on completion (both success and failure).
#[cfg(feature = "local_tty")]
fn parse_dev_container_up_stdout(stdout: &[u8]) -> Option<DevContainerUpResult> {
    let stdout_text = String::from_utf8_lossy(stdout);
    let last_line = stdout_text.lines().next_back()?.trim();
    serde_json::from_str(last_line).ok()
}

/// Finds `devcontainer.json` candidates for `workspace_folder`, in the order the
/// [devcontainers spec](https://containers.dev/implementors/spec/#devcontainerjson) lists
/// them: `.devcontainer/devcontainer.json`, then a workspace-root `.devcontainer.json`, then
/// any `.devcontainer/<folder>/devcontainer.json` (sorted by folder name, for a stable
/// selector order).
///
/// A repo with more than one candidate needs the caller to ask the user which one to use (see
/// [`TerminalView::find_and_start_dev_container`]) and to pass the chosen path explicitly via
/// `devcontainer up --config`, since the CLI's own default resolution only ever considers the
/// first two of these three locations.
#[cfg(feature = "local_tty")]
fn discover_dev_container_configs(workspace_folder: &Path) -> Vec<PathBuf> {
    let mut configs = Vec::new();

    let devcontainer_dir = workspace_folder.join(".devcontainer");
    let top_level_config = devcontainer_dir.join("devcontainer.json");
    if top_level_config.is_file() {
        configs.push(top_level_config);
    }

    let root_config = workspace_folder.join(".devcontainer.json");
    if root_config.is_file() {
        configs.push(root_config);
    }

    if let Ok(entries) = std::fs::read_dir(&devcontainer_dir) {
        let mut nested_configs: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .map(|dir| dir.join("devcontainer.json"))
            .filter(|path| path.is_file())
            .collect();
        nested_configs.sort();
        configs.extend(nested_configs);
    }

    configs
}

#[cfg(all(test, feature = "local_tty"))]
#[path = "mod_tests.rs"]
mod tests;

/// Returns the last `max_lines` non-empty lines of `text`, joined by `\n`.
#[cfg(feature = "local_tty")]
fn tail_lines(text: &str, max_lines: usize) -> String {
    let lines: Vec<&str> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    let start = lines.len().saturating_sub(max_lines);
    lines[start..].join("\n")
}
