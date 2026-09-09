//! `/devcontainer` flow.
//!
//! Unlike the Docker sandbox (which creates its container as a side effect of
//! spawning the PTY), Dev Container lifecycle is kept off the PTY spawn path:
//! `devcontainer up` can take minutes, so it runs in a focused right split that
//! streams logs. Only after attach setup succeeds is that split permanently
//! replaced with a `ShellStarter::DevContainer` pane (see
//! `crate::terminal::local_tty::dev_container`).

#[cfg(feature = "local_tty")]
mod kill;
#[cfg(feature = "local_tty")]
mod newline;
#[cfg(feature = "local_tty")]
pub(crate) mod operation;
#[cfg(feature = "local_tty")]
pub(crate) mod registry;
#[cfg(feature = "local_tty")]
mod stream;
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
pub(crate) use stream::PtyResizeHandle;
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
pub(crate) fn create_dev_container_view<V: warpui::View>(
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
    ctx: &mut ViewContext<V>,
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
    /// selection re-enters this flow via [`Self::resolve_dev_container_cli_and_bring_up`].
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

    /// Canonicalizes the workspace and config paths, then asks the pane group to open or focus
    /// the Dev Container build split.
    #[cfg(feature = "local_tty")]
    pub(crate) fn resolve_dev_container_cli_and_bring_up(
        &self,
        workspace_folder: PathBuf,
        config_path: PathBuf,
        ctx: &mut ViewContext<Self>,
    ) {
        let workspace_folder = dunce::canonicalize(&workspace_folder).unwrap_or(workspace_folder);
        let config_file = dunce::canonicalize(&config_path).unwrap_or(config_path);
        ctx.emit(super::Event::StartDevContainerBuild {
            workspace_folder,
            config_file,
        });
    }

    #[cfg(feature = "local_tty")]
    pub(crate) fn bind_dev_container_build(
        &mut self,
        operation: warpui::ModelHandle<operation::DevContainerBuildOperation>,
        ctx: &mut ViewContext<Self>,
    ) {
        ctx.subscribe_to_model(&operation, |me, _, _, ctx| {
            me.sync_dev_container_build_header(ctx);
            ctx.notify();
        });
        self.dev_container_build = Some(operation);
        self.model.lock().start_commandless_output_block();
        self.sync_dev_container_build_header(ctx);
        self.dev_container_awaiting_layout = true;
        ctx.notify();
    }

    #[cfg(feature = "local_tty")]
    pub(crate) fn retry_dev_container_build(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(operation) = self.dev_container_build.clone() else {
            return;
        };
        if !operation.read(ctx, |operation, _| operation.shows_retry_and_close()) {
            return;
        }
        let attempt_id = operation.update(ctx, |operation, ctx| operation.begin_retry(ctx));
        let key = operation.read(ctx, |operation, _| operation.key().clone());
        registry::DevContainerBuildRegistry::handle(ctx).update(ctx, |registry, _| {
            registry.set_attempt(&key, attempt_id);
        });
        self.model.lock().reset_commandless_output_block();
        self.sync_dev_container_build_header(ctx);
        self.dev_container_awaiting_layout = false;
        self.start_dev_container_build_attempt(ctx);
        ctx.notify();
    }

    pub(crate) fn cancel_dev_container_build(&mut self, ctx: &mut ViewContext<Self>) {
        #[cfg(feature = "local_tty")]
        {
            let Some(operation) = self.dev_container_build.clone() else {
                return;
            };
            let key = operation.read(ctx, |operation, _| operation.key().clone());
            self.dev_container_awaiting_layout = false;
            self.clear_dev_container_pty_resize();
            operation.update(ctx, |operation, ctx| {
                operation.tombstone(ctx);
                operation.mark_cancelled(ctx);
            });
            registry::DevContainerBuildRegistry::handle(ctx).update(ctx, |registry, _| {
                registry.remove(&key);
            });
        }
        #[cfg(not(feature = "local_tty"))]
        {
            let _ = ctx;
        }
    }

    pub(crate) fn is_dev_container_build_surface(&self) -> bool {
        #[cfg(feature = "local_tty")]
        {
            self.dev_container_build.is_some()
        }
        #[cfg(not(feature = "local_tty"))]
        {
            false
        }
    }

    #[cfg(feature = "local_tty")]
    pub(crate) fn sync_dev_container_build_header(&self, ctx: &mut ViewContext<Self>) {
        let Some(operation) = self.dev_container_build.clone() else {
            return;
        };
        let (title, secondary) = operation.read(ctx, |operation, _| {
            (operation.header_title(), operation.header_secondary())
        });
        self.pane_configuration.update(ctx, |pane_config, ctx| {
            pane_config.set_title(title, ctx);
            pane_config.set_title_secondary(secondary, ctx);
            pane_config.notify_header_content_changed(ctx);
        });
    }

    #[cfg(feature = "local_tty")]
    fn arm_silence_watch(&self, ctx: &mut ViewContext<Self>) {
        let Some(operation) = self.dev_container_build.clone() else {
            return;
        };
        let operation_id = operation.read(ctx, |operation, _| operation.operation_id());
        let attempt_id = operation.read(ctx, |operation, _| operation.attempt_id());
        let delay = operation.read(ctx, |operation, _| {
            operation::silence_watch_delay(operation.output_elapsed())
        });
        let output_rx = operation.read(ctx, |operation, _| operation.output_rx());
        while output_rx.try_recv().is_ok() {}
        ctx.spawn(
            async move {
                futures::future::select(
                    Box::pin(warpui::r#async::Timer::after(delay)),
                    Box::pin(async {
                        let _ = output_rx.recv().await;
                    }),
                )
                .await;
            },
            move |me, _, ctx| {
                if !me.is_current_dev_container_attempt(operation_id, attempt_id, ctx) {
                    return;
                }
                let Some(operation) = &me.dev_container_build else {
                    return;
                };
                if operation.read(ctx, |operation, _| {
                    operation.status() != operation::DevContainerBuildStatus::Running
                }) {
                    return;
                }
                me.sync_dev_container_build_header(ctx);
                me.arm_silence_watch(ctx);
            },
        );
    }

    #[cfg(all(test, feature = "local_tty"))]
    pub(crate) fn fail_dev_container_build_for_test(
        &self,
        phase: operation::DevContainerBuildPhase,
        message: String,
        ctx: &mut ViewContext<Self>,
    ) {
        self.fail_dev_container_build(phase, message, ctx);
    }

    #[cfg(all(test, feature = "local_tty"))]
    pub(crate) fn set_dev_container_build_phase_for_test(
        &self,
        phase: operation::DevContainerBuildPhase,
        ctx: &mut ViewContext<Self>,
    ) {
        if let Some(operation) = &self.dev_container_build {
            operation.update(ctx, |operation, ctx| operation.set_phase(phase, ctx));
        }
    }

    #[cfg(all(test, feature = "local_tty"))]
    pub(crate) fn dev_container_attempt_id(&self, ctx: &warpui::AppContext) -> Option<u64> {
        self.dev_container_build
            .as_ref()
            .map(|operation| operation.read(ctx, |operation, _| operation.attempt_id()))
    }

    #[cfg(all(test, feature = "local_tty"))]
    pub(crate) fn dev_container_shows_retry_and_close(&self, ctx: &warpui::AppContext) -> bool {
        self.dev_container_build.as_ref().is_some_and(|operation| {
            operation.read(ctx, |operation, _| operation.shows_retry_and_close())
        })
    }

    #[cfg(all(test, feature = "local_tty"))]
    pub(crate) fn dev_container_header_title(&self, ctx: &warpui::AppContext) -> Option<String> {
        self.dev_container_build
            .as_ref()
            .map(|operation| operation.read(ctx, |operation, _| operation.header_title()))
    }

    #[cfg(feature = "local_tty")]
    fn is_current_dev_container_attempt(
        &self,
        operation_id: uuid::Uuid,
        attempt_id: u64,
        ctx: &warpui::AppContext,
    ) -> bool {
        let Some(operation) = &self.dev_container_build else {
            return false;
        };
        let key = operation.read(ctx, |operation, _| operation.key().clone());
        operation.read(ctx, |operation, _| {
            operation.is_current_attempt(operation_id, attempt_id)
        }) && registry::DevContainerBuildRegistry::handle(ctx).read(ctx, |registry, _| {
            registry.matches(&key, operation_id, attempt_id)
        })
    }

    #[cfg(feature = "local_tty")]
    pub(crate) fn model_event_sender(
        &self,
    ) -> Option<std::sync::mpsc::SyncSender<crate::persistence::ModelEvent>> {
        self.model_event_sender.clone()
    }

    #[cfg(feature = "local_tty")]
    pub(crate) fn dev_container_build_key(
        &self,
        ctx: &warpui::AppContext,
    ) -> Option<registry::DevContainerBuildKey> {
        self.dev_container_build
            .as_ref()
            .map(|operation| operation.read(ctx, |operation, _| operation.key().clone()))
    }

    #[cfg(feature = "local_tty")]
    pub(crate) fn after_dev_container_layout(
        &mut self,
        size: Vector2F,
        ctx: &mut ViewContext<Self>,
    ) {
        if size.x() == 0. || size.y() == 0. {
            return;
        }
        if self.dev_container_awaiting_layout {
            self.dev_container_awaiting_layout = false;
            self.start_dev_container_build_attempt(ctx);
        }
        self.sync_dev_container_pty_size();
    }

    #[cfg(feature = "local_tty")]
    fn clear_dev_container_pty_resize(&mut self) {
        #[cfg(unix)]
        {
            self.dev_container_pty_resize = None;
        }
    }

    #[cfg(feature = "local_tty")]
    fn sync_dev_container_pty_size(&self) {
        #[cfg(unix)]
        {
            let Some(slot) = &self.dev_container_pty_resize else {
                return;
            };
            let Some(handle) = slot.lock().clone() else {
                return;
            };
            let _ = handle.resize(self.current_dev_container_pty_size());
        }
    }

    #[cfg(feature = "local_tty")]
    fn current_dev_container_pty_size(&self) -> stream::PtySize {
        stream::PtySize::from_grid(self.size_info().columns(), self.size_info().rows())
    }

    #[cfg(feature = "local_tty")]
    fn start_dev_container_build_attempt(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(operation) = self.dev_container_build.clone() else {
            return;
        };
        let operation_id = operation.read(ctx, |operation, _| operation.operation_id());
        let attempt_id = operation.read(ctx, |operation, _| operation.attempt_id());
        let workspace_folder =
            operation.read(ctx, |operation, _| operation.workspace_folder().clone());
        let config_file = operation.read(ctx, |operation, _| operation.config_file().clone());
        let cancel = operation.read(ctx, |operation, _| operation.cancel_handle());
        self.arm_silence_watch(ctx);

        let cli_future = resolve_devcontainer_cli_path(ctx);
        let docker_future = resolve_docker_cli_path(ctx);
        ctx.spawn(
            async move { (cli_future.await, docker_future.await) },
            move |me, (cli, docker), ctx| {
                let Some(operation) = me.dev_container_build.clone() else {
                    return;
                };
                if !operation.read(ctx, |operation, _| {
                    operation.is_current_attempt(operation_id, attempt_id)
                }) {
                    return;
                }
                match (cli, docker) {
                    (Some(cli), Some(docker)) => {
                        me.run_dev_container_up(
                            cli,
                            docker,
                            workspace_folder,
                            config_file,
                            cancel,
                            operation_id,
                            attempt_id,
                            ctx,
                        );
                    }
                    (None, _) => me.fail_dev_container_build(
                        operation::DevContainerBuildPhase::Build,
                        "devcontainer CLI not found on PATH. Install it with `npm install -g \
                         @devcontainers/cli` and try again."
                            .to_owned(),
                        ctx,
                    ),
                    (_, None) => me.fail_dev_container_build(
                        operation::DevContainerBuildPhase::Build,
                        "docker CLI not found on PATH.".to_owned(),
                        ctx,
                    ),
                }
            },
        );
    }

    #[cfg(feature = "local_tty")]
    fn fail_dev_container_build(
        &self,
        phase: operation::DevContainerBuildPhase,
        message: String,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(operation) = self.dev_container_build.clone() else {
            return;
        };
        let key = operation.read(ctx, |operation, _| operation.key().clone());
        operation.update(ctx, |operation, ctx| {
            operation.fail(phase, ctx);
        });
        registry::DevContainerBuildRegistry::handle(ctx).update(ctx, |registry, _| {
            registry.mark_failed(&key);
        });
        self.append_dev_container_failure_to_grid(&message);
        self.sync_dev_container_build_header(ctx);
        #[cfg(unix)]
        if let Some(slot) = &self.dev_container_pty_resize {
            *slot.lock() = None;
        }
        ctx.notify();
    }

    #[cfg(feature = "local_tty")]
    fn append_dev_container_failure_to_grid(&self, message: &str) {
        use warp_terminal::model::ansi::Processor;

        let event_proxy = self.model.lock().event_proxy.clone();
        {
            let mut model = self.model.lock();
            let mut processor = Processor::new();
            let mut normalizer = newline::NewlineNormalizer::new();
            let bytes = normalizer.push(format!("\n{message}\n").as_bytes());
            processor.parse_bytes(&mut *model, &bytes, &mut std::io::sink());
        }
        event_proxy.send_wakeup_event();
    }

    #[cfg(feature = "local_tty")]
    #[allow(clippy::too_many_arguments)]
    fn run_dev_container_up(
        &mut self,
        cli: PathBuf,
        docker_path: PathBuf,
        workspace_folder: PathBuf,
        config_file: PathBuf,
        cancel: operation::DevContainerBuildCancel,
        operation_id: uuid::Uuid,
        attempt_id: u64,
        ctx: &mut ViewContext<Self>,
    ) {
        use std::sync::Arc;

        use parking_lot::Mutex;
        use warp_terminal::model::ansi::Processor;

        let processor = Arc::new(Mutex::new(Processor::new()));
        let model = self.model.clone();
        let event_proxy = self.model.lock().event_proxy.clone();
        let last_output = self
            .dev_container_build
            .as_ref()
            .map(|operation| operation.read(ctx, |operation, _| operation.last_output_clock()));
        let output_tx = self
            .dev_container_build
            .as_ref()
            .map(|operation| operation.read(ctx, |operation, _| operation.output_tx()));
        let pty_size = self.current_dev_container_pty_size();
        let resize_slot = {
            let slot = Arc::new(Mutex::new(None));
            #[cfg(unix)]
            {
                self.dev_container_pty_resize = Some(slot.clone());
            }
            Some(slot)
        };
        let up_future = async move {
            let command =
                stream::dev_container_up_command(&cli, &workspace_folder, &config_file, pty_size);
            let (drain, exit_success) = stream::drain_dev_container_child_with_size_and_resize(
                command,
                Some(&cancel),
                move |chunk| {
                    if !chunk.is_empty() {
                        if let Some(last_output) = &last_output {
                            *last_output.lock() = instant::Instant::now();
                        }
                        if let Some(output_tx) = &output_tx {
                            let _ = output_tx.try_send(());
                        }
                    }
                    processor
                        .lock()
                        .parse_bytes(&mut *model.lock(), chunk, &mut std::io::sink());
                    event_proxy.send_wakeup_event();
                },
                pty_size,
                resize_slot,
            )
            .await?;
            std::io::Result::Ok((drain, exit_success, docker_path, workspace_folder))
        };
        ctx.spawn(up_future, move |me, result, ctx| {
            me.clear_dev_container_pty_resize();
            if !me.is_current_dev_container_attempt(operation_id, attempt_id, ctx) {
                return;
            }
            let Some(operation) = me.dev_container_build.clone() else {
                return;
            };
            match result {
                Ok((drain, exit_success, docker_path, workspace_folder)) => {
                    if drain.stdout.oversized {
                        me.fail_dev_container_build(
                            operation::DevContainerBuildPhase::Build,
                            "Dev container failed to start: stdout exceeded 1 MiB.".to_owned(),
                            ctx,
                        );
                        return;
                    }
                    match interpret_dev_container_up_output(
                        exit_success,
                        &drain.stdout.bytes,
                        &drain.stderr_tail,
                    ) {
                        DevContainerUpOutcome::ReadyToAttach {
                            container_id,
                            remote_user,
                            remote_workspace_folder,
                        } => {
                            operation.update(ctx, |operation, ctx| {
                                operation
                                    .set_phase(operation::DevContainerBuildPhase::Preflight, ctx);
                            });
                            me.preflight_and_attach_dev_container(
                                workspace_folder,
                                docker_path,
                                container_id,
                                remote_user,
                                remote_workspace_folder,
                                generate_sandbox_id(),
                                generate_session_id(),
                                operation_id,
                                attempt_id,
                                ctx,
                            );
                        }
                        DevContainerUpOutcome::Error(message) => {
                            me.fail_dev_container_build(
                                operation::DevContainerBuildPhase::Build,
                                message,
                                ctx,
                            );
                        }
                    }
                }
                Err(error) => me.fail_dev_container_build(
                    operation::DevContainerBuildPhase::Build,
                    format!("Failed to run `devcontainer up`: {error}"),
                    ctx,
                ),
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
        operation_id: uuid::Uuid,
        attempt_id: u64,
        ctx: &mut ViewContext<Self>,
    ) {
        let preflight_future = {
            let docker_path = docker_path.clone();
            let container_id = container_id.clone();
            let remote_user = remote_user.clone();
            let remote_workspace_folder = remote_workspace_folder.clone();
            let cancel = self
                .dev_container_build
                .as_ref()
                .map(|operation| operation.read(ctx, |operation, _| operation.cancel_handle()));
            async move {
                let mut command = Command::new_with_process_group(&docker_path);
                command.args(dev_container_preflight_args(
                    &container_id,
                    remote_user.as_deref(),
                    &remote_workspace_folder,
                ));
                match cancel {
                    Some(cancel) => {
                        stream::run_cancellable_process_group_command(command, &cancel).await
                    }
                    None => command.output().await,
                }
            }
        };

        ctx.spawn(preflight_future, move |me, result, ctx| {
            if !me.is_current_dev_container_attempt(operation_id, attempt_id, ctx) {
                return;
            }
            match result {
                Ok(output) if output.status.success() => {
                    // Staging is several sequential `docker cp`/`exec` round trips (a
                    // default-user probe plus two copy+chown+chmod sequences), so it must
                    // run as its own spawned future rather than be awaited inline here:
                    // this closure runs on the main thread, and blocking it for that long
                    // right after `devcontainer up` would freeze the UI.
                    #[cfg(unix)]
                    let staging_future = {
                        let cancel = me.dev_container_build.as_ref().map(|operation| {
                            operation.read(ctx, |operation, _| operation.cancel_handle())
                        });
                        let docker_path = docker_path.clone();
                        let container_id = container_id.clone();
                        let remote_user = remote_user.clone();
                        let sandbox_id = sandbox_id.clone();
                        async move {
                            crate::terminal::local_tty::prepare_dev_container(
                                docker_path,
                                container_id,
                                remote_user,
                                sandbox_id,
                                session_id,
                                cancel.as_ref().map(|cancel| {
                                    cancel as &dyn crate::terminal::local_tty::ProcessGroupCancel
                                }),
                            )
                            .await
                        }
                    }
                    .boxed();
                    #[cfg(not(unix))]
                    let staging_future = futures::future::ready(Err(anyhow::Error::msg(
                        "Dev Container sessions are only supported on Linux and macOS",
                    )))
                    .boxed();

                    if me.dev_container_build.is_some() {
                        me.dev_container_build
                            .as_ref()
                            .unwrap()
                            .update(ctx, |operation, ctx| {
                                operation
                                    .set_phase(operation::DevContainerBuildPhase::Staging, ctx);
                            });
                    }
                    ctx.spawn(staging_future, move |me, staging_result, ctx| {
                        if !me.is_current_dev_container_attempt(operation_id, attempt_id, ctx) {
                            return;
                        }
                        match staging_result {
                            Ok(()) => {
                                if me.dev_container_build.is_some() {
                                    me.dev_container_build.as_ref().unwrap().update(
                                        ctx,
                                        |operation, ctx| {
                                            operation.set_phase(
                                                operation::DevContainerBuildPhase::Attach,
                                                ctx,
                                            );
                                            operation.complete(ctx);
                                        },
                                    );
                                    ctx.emit(super::Event::ReplaceDevContainerBuildPane {
                                        workspace_folder,
                                        docker_path,
                                        container_id,
                                        remote_user,
                                        remote_workspace_folder,
                                        sandbox_id,
                                        session_id,
                                    });
                                    return;
                                }
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
                                let message =
                                    format!("Failed to prepare the Dev Container session: {e:#}");
                                if me.dev_container_build.is_some() {
                                    me.fail_dev_container_build(
                                        operation::DevContainerBuildPhase::Staging,
                                        message,
                                        ctx,
                                    );
                                } else {
                                    me.show_dev_container_toast(message, ToastFlavor::Error, ctx);
                                }
                            }
                        }
                    });
                }
                Ok(output) => {
                    let detail = tail_lines(&String::from_utf8_lossy(&output.stderr), 5);
                    let message = format!(
                        "Dev container isn't ready to attach: it may be missing `bash`, or its \
                     configured remote user or workspace folder may not exist: {detail}"
                    );
                    if me.dev_container_build.is_some() {
                        me.fail_dev_container_build(
                            operation::DevContainerBuildPhase::Preflight,
                            message,
                            ctx,
                        );
                    } else {
                        me.show_dev_container_toast(message, ToastFlavor::Error, ctx);
                    }
                }
                Err(e) => {
                    let message =
                        format!("Failed to verify the Dev Container is ready to attach: {e}");
                    if me.dev_container_build.is_some() {
                        me.fail_dev_container_build(
                            operation::DevContainerBuildPhase::Preflight,
                            message,
                            ctx,
                        );
                    } else {
                        me.show_dev_container_toast(message, ToastFlavor::Error, ctx);
                    }
                }
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
/// stdout/stderr as a pure function so the partial-result and JSON-fallback
/// cases are unit testable without actually running `devcontainer`.
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

/// Builds the user-facing failure body for a failed (or unparseable) `devcontainer
/// up`, preferring structured `message`/`description` from its final JSON status
/// line and including leftover stdout or useful stderr when they add detail.
#[cfg(feature = "local_tty")]
fn dev_container_up_failure_message(stdout: &[u8], stderr: &[u8]) -> String {
    let structured = parse_dev_container_up_stdout(stdout)
        .and_then(|result| result.message.or(result.description));
    let extra_stdout = leftover_stdout_lines(stdout);
    let extra_stderr = useful_stderr_lines(stderr);

    let mut parts = Vec::new();
    if let Some(structured) = structured {
        parts.push(structured);
    }
    append_unique_failure_part(&mut parts, extra_stdout);
    append_unique_failure_part(&mut parts, extra_stderr);
    if parts.is_empty() {
        "Dev container failed to start.".to_owned()
    } else {
        format!("Dev container failed to start:\n{}", parts.join("\n"))
    }
}

#[cfg(feature = "local_tty")]
fn leftover_stdout_lines(stdout: &[u8]) -> Option<String> {
    let stdout_text = strip_ansi_control_sequences(&String::from_utf8_lossy(stdout));
    let mut lines: Vec<&str> = stdout_text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    if lines
        .last()
        .is_some_and(|line| serde_json::from_str::<serde_json::Value>(line).is_ok())
    {
        lines.pop();
    }
    if lines.is_empty() {
        None
    } else {
        Some(lines[lines.len().saturating_sub(20)..].join("\n"))
    }
}

#[cfg(feature = "local_tty")]
fn useful_stderr_lines(stderr: &[u8]) -> Option<String> {
    let stderr_text = strip_ansi_control_sequences(&String::from_utf8_lossy(stderr));
    let lines: Vec<String> = stderr_text
        .lines()
        .filter_map(display_stderr_line)
        .collect();
    if lines.is_empty() {
        None
    } else {
        Some(lines[lines.len().saturating_sub(20)..].join("\n"))
    }
}

#[cfg(feature = "local_tty")]
fn strip_ansi_control_sequences(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\u{1b}' {
            if ch != '\r' {
                out.push(ch);
            }
            continue;
        }
        match chars.peek() {
            Some('[') => {
                chars.next();
                for next in chars.by_ref() {
                    if next.is_ascii_alphabetic() || next == '~' {
                        break;
                    }
                }
            }
            Some(']') => {
                chars.next();
                while let Some(next) = chars.next() {
                    if next == '\u{7}' || next == '\u{9c}' {
                        break;
                    }
                    if next == '\u{1b}' && chars.peek() == Some(&'\\') {
                        chars.next();
                        break;
                    }
                }
            }
            Some(_) => {
                chars.next();
            }
            None => {}
        }
    }
    out
}

#[cfg(feature = "local_tty")]
fn display_stderr_line(line: &str) -> Option<String> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
        let event_type = value.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if event_type == "progress" || event_type == "stop" {
            return None;
        }
        let text = value.get("text").and_then(|v| v.as_str())?.trim();
        if text.is_empty() || text.contains("@devcontainers/cli") {
            return None;
        }
        return Some(text.to_owned());
    }
    if trimmed.contains("@devcontainers/cli") {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

#[cfg(feature = "local_tty")]
fn append_unique_failure_part(parts: &mut Vec<String>, extra: Option<String>) {
    let Some(extra) = extra else {
        return;
    };
    let mut seen: Vec<&str> = parts
        .iter()
        .flat_map(|part| part.lines())
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    let mut novel = Vec::new();
    for line in extra.lines().map(str::trim).filter(|line| !line.is_empty()) {
        if seen.contains(&line) {
            continue;
        }
        seen.push(line);
        novel.push(line);
    }
    if novel.is_empty() {
        return;
    }
    parts.push(novel.join("\n"));
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
    let line = stdout_text
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())?;
    serde_json::from_str::<DevContainerUpResult>(line)
        .ok()
        .filter(|parsed| !parsed.outcome.is_empty())
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
pub(crate) fn interpret_up_output_for_test(
    exit_success: bool,
    stdout: &[u8],
    stderr: &[u8],
) -> bool {
    matches!(
        interpret_dev_container_up_output(exit_success, stdout, stderr),
        DevContainerUpOutcome::ReadyToAttach { .. }
    )
}

#[cfg(all(test, feature = "local_tty"))]
pub(crate) fn failure_message_for_test(exit_success: bool, stdout: &[u8], stderr: &[u8]) -> String {
    match interpret_dev_container_up_output(exit_success, stdout, stderr) {
        DevContainerUpOutcome::Error(message) => message,
        DevContainerUpOutcome::ReadyToAttach { .. } => {
            panic!("expected a failed Dev Container outcome")
        }
    }
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
