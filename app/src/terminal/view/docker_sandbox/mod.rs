#[cfg(feature = "local_tty")]
use std::sync::mpsc::SyncSender;

#[cfg(feature = "local_tty")]
use warpui::geometry::vector::Vector2F;
#[cfg(feature = "local_tty")]
use warpui::ModelHandle;
use warpui::ViewContext;
#[cfg(not(target_family = "wasm"))]
use warpui::{SingletonEntity, View, ViewHandle};

#[cfg(feature = "local_tty")]
use crate::pane_group::TerminalViewResources;
#[cfg(feature = "local_tty")]
use crate::persistence::ModelEvent;
#[cfg(feature = "local_tty")]
use crate::server::server_api::ServerApiProvider;
#[cfg(feature = "local_tty")]
use crate::terminal::local_tty::docker_sandbox::resolve_sbx_path_from_user_shell;
#[cfg(feature = "local_tty")]
use crate::terminal::TerminalManager;

#[cfg(not(target_family = "wasm"))]
use crate::ai::agent_sdk::driver::{
    environment::prepare_environment, terminal::TerminalDriver, WARP_DRIVE_SYNC_TIMEOUT,
};
#[cfg(not(target_family = "wasm"))]
use crate::ai::cloud_environments::CloudAmbientAgentEnvironment;
use crate::cloud_object::CloudObjectLookup as _;
#[cfg(not(target_family = "wasm"))]
use crate::server::cloud_objects::update_manager::UpdateManager;
#[cfg(not(target_family = "wasm"))]
use crate::server::ids::{ServerId, SyncId};
#[cfg(not(target_family = "wasm"))]
use crate::terminal::local_tty::docker_sandbox::DOCKER_SANDBOX_HOME_DIR;
#[cfg(feature = "remote_tty")]
use crate::terminal::remote_tty::TerminalManager as RemoteTtyTerminalManager;
#[cfg(not(target_family = "wasm"))]
use warp_cli::agent::Harness;
#[cfg(not(target_family = "wasm"))]
use warpui::r#async::FutureExt;

use super::TerminalView;

/// Default base Docker image used for newly created sandbox shells.
///
/// `None` means "let sbx pick its own default template".
///
/// TODO(advait): Replace this with the base image read off the associated
/// `AmbientAgentEnvironment` (see `BaseImage::DockerImage`). Requires moving
/// the environment lookup ahead of `create_and_push_docker_sandbox`, which
/// currently happens asynchronously in `initialize_docker_sandbox_environment`
/// after the PTY is spawned. Tracked in Ben's review comment on PR #24550.
#[cfg(feature = "local_tty")]
pub(crate) const DEFAULT_DOCKER_SANDBOX_BASE_IMAGE: Option<&str> = None;

#[cfg(feature = "local_tty")]
#[allow(unused_variables)]
fn create_docker_sandbox_view(
    resources: TerminalViewResources,
    initial_size: Vector2F,
    model_event_sender: Option<SyncSender<ModelEvent>>,
    #[allow(dead_code)] sbx_path: std::path::PathBuf,
    ctx: &mut ViewContext<TerminalView>,
) -> (
    ViewHandle<TerminalView>,
    ModelHandle<Box<dyn TerminalManager>>,
) {
    cfg_if::cfg_if! {
        if #[cfg(feature = "remote_tty")] {
            let terminal_manager = RemoteTtyTerminalManager::create_model(
                resources,
                initial_size,
                model_event_sender,
                ctx.window_id(),
                None, /* initial_input_config */
                ctx,
            );
        } else if #[cfg(feature = "local_tty")] {
            let user_default_shell_unsupported_banner_model_handle =
                ctx.add_model(|_| crate::banner::BannerState::default());

            let chosen_shell = Some(crate::terminal::available_shells::AvailableShell::new_docker_sandbox_shell(
                sbx_path,
                DEFAULT_DOCKER_SANDBOX_BASE_IMAGE.map(str::to_owned),
            ));

            let terminal_manager = crate::terminal::local_tty::TerminalManager::create_model(
                None,
                std::collections::HashMap::new(),
                crate::terminal::shared_session::IsSharedSessionCreator::No,
                resources,
                None, /* restored_blocks */
                None, /* conversation_restoration */
                user_default_shell_unsupported_banner_model_handle,
                initial_size,
                model_event_sender,
                ctx.window_id(),
                chosen_shell,
                None, /* initial_input_config */
                ctx,
            );
        } else {
            log::info!("USING MOCK TERMINAL MANAGER!!!!!");
            use crate::terminal::shell::{ShellName, ShellType};
            use crate::terminal::ShellLaunchState;

            let terminal_manager = crate::terminal::MockTerminalManager::create_model(
                ShellLaunchState::ShellSpawned {
                    available_shell: None,
                    display_name: ShellName::blank(),
                    shell_type: ShellType::Bash,
                },
                resources,
                None, /* restored_blocks */
                None, /* conversation_restoration */
                initial_size,
                ctx.window_id(),
                ctx,
            );
        }
    }

    let terminal_view = terminal_manager.as_ref(ctx).view();
    (terminal_view, terminal_manager)
}

impl TerminalView {
    pub(crate) fn create_and_push_docker_sandbox(&self, ctx: &mut ViewContext<Self>) {
        // Resolve sbx via the user's interactive shell PATH (same mechanism
        // MCP servers use) before creating the pane. This is async, so we
        // spawn and then build the pane in the completion callback.
        //
        // The sbx resolution and sandbox creation are only meaningful on
        // platforms with a local tty; other builds (e.g. wasm/remote_tty) log
        // and bail.
        #[cfg(feature = "local_tty")]
        {
            let sbx_future = resolve_sbx_path_from_user_shell(ctx);
            ctx.spawn(sbx_future, move |me, sbx_path, ctx| {
                let Some(sbx_path) = sbx_path else {
                    log::error!("sbx binary not found; cannot create Docker sandbox");
                    return;
                };
                me.create_and_push_docker_sandbox_with_sbx(sbx_path, ctx);
            });
        }
        #[cfg(not(feature = "local_tty"))]
        {
            let _ = ctx;
            log::warn!("Docker sandbox requires the `local_tty` feature; ignoring request");
        }
    }

    #[cfg(feature = "local_tty")]
    fn create_and_push_docker_sandbox_with_sbx(
        &self,
        sbx_path: std::path::PathBuf,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(pane_stack) = self
            .pane_stack
            .as_ref()
            .and_then(|stack| stack.upgrade(ctx))
        else {
            log::warn!("Pane stack not available, cannot create docker sandbox session");
            return;
        };

        let resources = TerminalViewResources {
            tips_completed: self.tips_completed.clone(),
            server_api: ServerApiProvider::as_ref(ctx).get(),
            model_event_sender: self.model_event_sender.clone(),
        };
        let pane_configuration = self.pane_configuration().clone();

        let (terminal_view, terminal_manager) = create_docker_sandbox_view(
            resources,
            self.size_info().pane_size_px(),
            self.model_event_sender.clone(),
            sbx_path,
            ctx,
        );

        terminal_view.update(ctx, |view, _| {
            view.set_pane_configuration(pane_configuration);
        });

        #[cfg(not(target_family = "wasm"))]
        let terminal_view_for_init = terminal_view.clone();

        pane_stack.update(ctx, |stack, ctx| {
            stack.push(terminal_manager, terminal_view, ctx);
        });

        #[cfg(not(target_family = "wasm"))]
        Self::initialize_docker_sandbox_environment(&terminal_view_for_init, ctx);

        ctx.notify();
    }

    /// Kick off async environment initialization for a docker sandbox terminal.
    #[cfg(not(target_family = "wasm"))]
    pub(crate) fn initialize_docker_sandbox_environment<V: View>(
        terminal_view: &ViewHandle<TerminalView>,
        ctx: &mut ViewContext<V>,
    ) {
        let terminal_driver = TerminalDriver::create_from_existing_view(terminal_view.clone(), ctx);

        let spawner = terminal_driver.update(ctx, |_, ctx| ctx.spawner());
        let sync_future = UpdateManager::as_ref(ctx).initial_load_complete();
        ctx.spawn(
            async move {
                // Wait for Warp Drive initial sync so environment lookup succeeds.

                if sync_future
                    .with_timeout(WARP_DRIVE_SYNC_TIMEOUT)
                    .await
                    .is_err()
                {
                    return Err("Timed out waiting for Warp Drive to sync for docker sandbox");
                }

                // Wait for the terminal session to bootstrap.
                let bootstrap_future = spawner
                    .spawn(move |driver, _| driver.wait_for_session_bootstrapped())
                    .await
                    .map_err(|_| "view dropped")?;

                if let Err(e) = bootstrap_future.await {
                    log::error!("Docker sandbox bootstrap failed: {e}");
                    return Err("terminal bootstrap failed");
                }

                // Look up the environment by hardcoded ID.
                let environment = spawner
                    .spawn(|_, ctx| {
                        let server_id = ServerId::try_from("SVhg783GBFQHk1OfdPfFU9").ok()?;
                        let sync_id = SyncId::ServerId(server_id);
                        CloudAmbientAgentEnvironment::get_by_id(&sync_id, ctx)
                            .map(|env| env.model().string_model.clone())
                    })
                    .await
                    .map_err(|_| "view dropped")?
                    .ok_or("environment not found")?;

                // Prepare the environment (clone repos, run setup commands, index codebases).
                let prepare_future = spawner
                    .spawn(|_, ctx| {
                        prepare_environment(
                            environment,
                            DOCKER_SANDBOX_HOME_DIR.into(),
                            true, /* is_sandbox */
                            Harness::Oz,
                            ctx,
                        )
                    })
                    .await
                    .map_err(|_| "view dropped")?;

                prepare_future.await.map_err(|e| {
                    log::error!("Docker sandbox environment preparation failed: {e}");
                    "environment preparation failed"
                })?;

                // Keep the TerminalDriver model alive for the entire duration of
                // this async block. The spawner only holds a weak reference to the
                // model; if the ModelHandle is dropped the model is released and
                // all subsequent spawner calls fail with ModelDropped.
                drop(terminal_driver);

                Ok(())
            },
            |_, result, _| match result {
                Ok(()) => {
                    log::info!("Prepared Docker Sandbox environment");
                }
                Err(err) => {
                    log::error!("Docker Sandbox environment setup failed: {err}");
                }
            },
        );
    }
}
