use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread::JoinHandle;

use anyhow::{Context as _, Result};
use parking_lot::FairMutex;
use settings::Setting as _;
use warpui::{AppContext, SingletonEntity};

use crate::server::telemetry::{PtySpawnMode as TelemetryPtySpawnMode, TelemetryEvent};
use crate::settings::{DebugSettings, PrivacySettings};
use crate::terminal::TerminalModel;
use crate::terminal::event_listener::ChannelEventListener;
use crate::terminal::local_tty::shell::ShellStarter;
use crate::terminal::local_tty::spawner::{PtySpawnHooks, PtySpawnMode};
use crate::terminal::local_tty::{Pty, PtyOptions, mio_channel};
use crate::terminal::session_settings::SessionSettings;
use crate::terminal::tmux::event_loop::{ControlClientEventLoop, SharedControlState};
use crate::terminal::tmux::protocol::{
    DEDICATED_TMUX_CONFIG, PaneBootstrap, cleanup_unspawned_dedicated_files, dedicated_config_path,
    dedicated_socket_path, register_dedicated_server, resolve_tmux_binary, tmux_shell_starter,
};
use crate::terminal::tmux::transport::ControlTransportSpec;
use crate::terminal::writeable_pty::Message;

struct ControlClientSpawnHooks {
    is_crash_reporting_enabled: bool,
}

impl PtySpawnHooks for ControlClientSpawnHooks {
    fn before_spawn(&self) {
        #[cfg(feature = "crash_reporting")]
        crate::crash_reporting::uninit_cocoa_sentry();
    }

    fn after_spawn(&self) {
        if self.is_crash_reporting_enabled {
            #[cfg(feature = "crash_reporting")]
            crate::crash_reporting::init_cocoa_sentry();
        }
    }

    fn spawned(&self, mode: PtySpawnMode, ctx: &mut AppContext) {
        let mode = match mode {
            PtySpawnMode::TerminalServer => TelemetryPtySpawnMode::TerminalServer,
            PtySpawnMode::FallbackToDirect => TelemetryPtySpawnMode::FallbackToDirect,
            PtySpawnMode::Direct => TelemetryPtySpawnMode::Direct,
        };
        crate::send_telemetry_from_app_ctx!(TelemetryEvent::PtySpawned { mode }, ctx);
    }
}

pub struct SpawnedControlClient {
    pub event_loop_handle: JoinHandle<()>,
    pub socket: PathBuf,
}

pub fn spawn_control_client(
    bootstrap: &PaneBootstrap,
    model: Arc<FairMutex<TerminalModel>>,
    channel_event_proxy: ChannelEventListener,
    event_loop_rx: mio_channel::Receiver<Message>,
    shared: Arc<SharedControlState>,
    ctx: &mut AppContext,
) -> Result<SpawnedControlClient> {
    let tmux_path = resolve_tmux_binary().context("tmux binary not found on PATH")?;
    let socket = dedicated_socket_path(bootstrap.session_id);
    let config = dedicated_config_path(bootstrap.session_id);
    if let Some(parent) = socket.parent() {
        std::fs::create_dir_all(parent).context("failed to create tmux socket directory")?;
    }
    std::fs::write(&config, DEDICATED_TMUX_CONFIG).context("failed to write tmux config")?;

    let size = model.lock().block_list().size().to_owned();
    let argv = ControlTransportSpec::LocalDedicated {
        tmux_path: tmux_path.clone(),
        socket: socket.clone(),
        config: config.clone(),
        bootstrap: bootstrap.clone(),
        columns: size.columns(),
        rows: size.rows(),
    }
    .spawn_argv();
    let starter = tmux_shell_starter(argv, bootstrap.session_id)
        .context("failed to construct tmux shell starter")?;

    let is_shell_debug_mode_enabled = *DebugSettings::as_ref(ctx)
        .is_shell_debug_mode_enabled
        .value();
    let is_honor_ps1_enabled = *SessionSettings::as_ref(ctx).honor_ps1;
    let is_crash_reporting_enabled = PrivacySettings::as_ref(ctx).is_crash_reporting_enabled;

    let options = PtyOptions {
        size,
        window_id: None,
        shell_starter: ShellStarter::Direct(starter),
        start_dir: None,
        env_vars: HashMap::new(),
        enable_ssh_wrapper: false,
        reuse_ssh_control_master: false,
        shell_debug_mode: is_shell_debug_mode_enabled,
        honor_ps1: is_honor_ps1_enabled,
        node_version_chip_enabled: false,
        close_fds: true,
    };
    let hooks = ControlClientSpawnHooks {
        is_crash_reporting_enabled,
    };
    let pty = match Pty::new(options, &hooks, ctx) {
        Ok(pty) => pty,
        Err(err) => {
            cleanup_unspawned_dedicated_files(&socket);
            return Err(err).context("failed to spawn tmux control client");
        }
    };

    let zsh_init = bootstrap
        .init_script
        .clone()
        .map(|script| (script, bootstrap.shell_type));
    let event_loop = ControlClientEventLoop::new(
        model,
        channel_event_proxy,
        pty,
        event_loop_rx,
        shared,
        bootstrap.session_id,
        zsh_init,
    );
    register_dedicated_server(socket.clone());
    Ok(SpawnedControlClient {
        event_loop_handle: event_loop.spawn(),
        socket,
    })
}
