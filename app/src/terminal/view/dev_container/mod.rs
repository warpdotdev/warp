#[cfg(feature = "local_tty")]
use std::sync::mpsc::SyncSender;

#[cfg(feature = "local_tty")]
use warpui::geometry::vector::Vector2F;
#[cfg(feature = "local_tty")]
use warpui::ModelHandle;
use warpui::ViewContext;
#[cfg(feature = "local_tty")]
use warpui::{SingletonEntity, ViewHandle};

#[cfg(feature = "local_tty")]
use crate::pane_group::TerminalViewResources;
#[cfg(feature = "local_tty")]
use crate::persistence::ModelEvent;
#[cfg(feature = "local_tty")]
use crate::server::server_api::ServerApiProvider;
#[cfg(feature = "local_tty")]
use crate::terminal::local_tty::dev_container::{
    find_nearest_devcontainer_configs, resolve_devcontainer_cli_path_from_user_shell,
    DevContainerConfig,
};
#[cfg(feature = "remote_tty")]
use crate::terminal::remote_tty::TerminalManager as RemoteTtyTerminalManager;
#[cfg(feature = "local_tty")]
use crate::terminal::TerminalManager;

use super::TerminalView;

#[cfg(feature = "local_tty")]
#[allow(unused_variables)]
fn create_dev_container_view(
    resources: TerminalViewResources,
    initial_size: Vector2F,
    model_event_sender: Option<SyncSender<ModelEvent>>,
    devcontainer_cli_path: std::path::PathBuf,
    config: DevContainerConfig,
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

            let chosen_shell = Some(crate::terminal::available_shells::AvailableShell::new_dev_container_shell(
                devcontainer_cli_path,
                config.workspace_folder,
                config.config_path,
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
    pub(crate) fn create_and_push_dev_container(&mut self, ctx: &mut ViewContext<Self>) {
        #[cfg(feature = "local_tty")]
        {
            let Some(start_path) = self.active_session_path_if_local(ctx) else {
                self.show_error_toast(
                    "Open a local terminal in a project with a Dev Container config first."
                        .to_owned(),
                    ctx,
                );
                return;
            };

            let mut configs = find_nearest_devcontainer_configs(&start_path);
            let Some(config) = configs.drain(..).next() else {
                self.show_error_toast(
                    "No Dev Container config found for the current workspace.".to_owned(),
                    ctx,
                );
                return;
            };

            if !configs.is_empty() {
                log::info!(
                    "Multiple Dev Container configs found; launching {}",
                    config.display_name()
                );
            }

            let devcontainer_cli_future = resolve_devcontainer_cli_path_from_user_shell(ctx);
            ctx.spawn(
                devcontainer_cli_future,
                move |me, devcontainer_cli_path, ctx| {
                    let Some(devcontainer_cli_path) = devcontainer_cli_path else {
                        log::error!("devcontainer binary not found; cannot create Dev Container");
                        me.show_error_toast(
                            "Could not find the devcontainer CLI in your shell PATH.".to_owned(),
                            ctx,
                        );
                        return;
                    };
                    me.create_and_push_dev_container_with_config(
                        devcontainer_cli_path,
                        config,
                        ctx,
                    );
                },
            );
        }
        #[cfg(not(feature = "local_tty"))]
        {
            let _ = ctx;
            log::warn!("Dev Containers require the `local_tty` feature; ignoring request");
        }
    }

    #[cfg(feature = "local_tty")]
    fn create_and_push_dev_container_with_config(
        &self,
        devcontainer_cli_path: std::path::PathBuf,
        config: DevContainerConfig,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(pane_stack) = self
            .pane_stack
            .as_ref()
            .and_then(|stack| stack.upgrade(ctx))
        else {
            log::warn!("Pane stack not available, cannot create Dev Container session");
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
            devcontainer_cli_path,
            config,
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
}
