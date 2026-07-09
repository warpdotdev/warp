use std::io;
use std::sync::Arc;

use async_channel::Receiver;
use futures_util::SinkExt;
use parking_lot::FairMutex;
use serde::Serialize;
use warp_errors::report_error;
use warpui::{Entity, ModelContext, SingletonEntity};
use websocket::{Message, Sink, Stream, WebSocket, WebsocketMessage as _};

use crate::terminal::bootstrap::init_shell_script_for_shell;
use crate::terminal::event_listener::ChannelEventListener;
use crate::terminal::model::ansi::Processor;
use crate::terminal::session_settings::SessionSettings;
use crate::terminal::shell::ShellType;
use crate::terminal::writeable_pty::Message as EventLoopMessage;
use crate::terminal::{SizeInfo, TerminalModel};

const CREATE_SESSION_ENDPOINT: &str = "ws://127.0.0.1:3030/create";

/// Contains info needed to resize the SSH terminal session. Is serialized and
/// sent over the websocket as text.
///
/// The field names need to be kept the same as the `WindowSizeChange` struct in
/// https://github.com/warpdotdev/ssh-proxy-server/blob/main/src/ssh/session.rs.
#[derive(Serialize, Debug)]
struct WindowSizeChange {
    width: u32,
    height: u32,
    width_px: u32,
    height_px: u32,
}

pub(super) struct EventLoop {
    terminal_model: Arc<FairMutex<TerminalModel>>,
    parser: Processor,
    event_loop_rx: Receiver<EventLoopMessage>,
    channel_event_listener: ChannelEventListener,
}

impl EventLoop {
    /// Starts the [`EventLoop`] by starting a websocket connection with the server and
    /// bootstrapping the PTY.
    pub(super) fn start(
        model: Arc<FairMutex<TerminalModel>>,
        websocket_receiver: Receiver<EventLoopMessage>,
        channel_event_listener: ChannelEventListener,
        size_info: SizeInfo,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        let event_loop = Self::new(model, websocket_receiver, channel_event_listener);

        let url = Self::get_new_session_url(size_info);
        let response = WebSocket::connect(url, None /* protocols */);

        ctx.spawn(response, Self::on_ws_connection);

        event_loop
    }

    fn new(
        terminal_model: Arc<FairMutex<TerminalModel>>,
        websocket_receiver: Receiver<EventLoopMessage>,
        channel_event_listener: ChannelEventListener,
    ) -> Self {
        Self {
            terminal_model,
            parser: Processor::default(),
            event_loop_rx: websocket_receiver,
            channel_event_listener,
        }
    }

    fn get_new_session_url(size_info: SizeInfo) -> String {
        let num_rows = size_info.rows;
        let num_cols = size_info.columns;

        format!("{CREATE_SESSION_ENDPOINT}?num_rows={num_rows}&num_cols={num_cols}")
    }

    /// Starts tasks to listen to and write to the websocket.
    fn start_websocket_listener_and_writer_tasks(
        &mut self,
        mut sink: impl Sink,
        stream: impl Stream,
        ctx: &mut ModelContext<Self>,
    ) {
        // TODO(alokedesai): Add a spawn_stream equivalent that runs on the background executor.
        ctx.spawn_stream_local(
            stream,
            |event_loop, message, _| {
                let message = match message {
                    Ok(message) => message,
                    Err(err) => {
                        report_error!(anyhow::Error::new(err).context("Unable to receive item"));
                        return;
                    }
                };

                let Some(bytes) = message.binary() else {
                    report_error!("Received non binary message");
                    return;
                };

                event_loop.process_pty_bytes(bytes);
            },
            |_, _| {},
        );

        let is_honor_ps1_enabled = *SessionSettings::as_ref(ctx).honor_ps1;

        let terminal_model_for_init = self.terminal_model.clone();
        let receiver = self.event_loop_rx.clone();
        ctx.background_executor()
            .spawn(async move {
                if let Err(e) = Self::write_env_vars(&mut sink, is_honor_ps1_enabled).await {
                    report_error!(e.context("Failed to write env vars to pty"));
                }
                if let Err(e) =
                    Self::write_zsh_init_shell_script(&mut sink, &terminal_model_for_init).await
                {
                    report_error!(e.context("Failed to write zsh bootstrap bytes to pty"));
                }

                while let Ok(message) = receiver.recv().await {
                    match message {
                        EventLoopMessage::Input(bytes) => {
                            if let Err(e) = sink.send(Message::new_binary(bytes.to_vec())).await {
                                report_error!(anyhow::Error::new(e)
                                    .context("Failed to send message to network-backed PTY"));
                            };
                        }
                        EventLoopMessage::Resize(size_info) => {
                            let size_change = WindowSizeChange {
                                width: size_info.columns as u32,
                                height: size_info.rows as u32,
                                width_px: size_info.pane_width_px().as_f32() as u32,
                                height_px: size_info.pane_height_px().as_f32() as u32,
                            };

                            let Ok(serialized) = serde_json::to_string(&size_change) else {
                                report_error!("Error serializing window size change info");
                                continue;
                            };

                            // Sending as a `Text` message implies that this is a
                            // control channel message. The SSH proxy server should
                            // make this distinction.
                            if let Err(e) = sink.send(Message::new_text(serialized)).await {
                                report_error!(anyhow::Error::new(e)
                                    .context("Failed to send message to network-backed PTY"));
                            };
                        }
                        // TODO(alokedesai): Implement shutdown on the network backed PTY.
                        EventLoopMessage::Shutdown | EventLoopMessage::ChildExited => {}
                    }
                }
            })
            .detach();
    }

    /// Writes the ZSH init shell script to the "PTY", mimicking how we send the init shell script
    /// when there is a local pty:
    /// <https://github.com/warpdotdev/warp-internal/blob/747da2df83f2caa97e781ce284ceb226fb97a66c/app/src/terminal/local_tty/unix.rs#L338-L347>.
    async fn write_zsh_init_shell_script(
        sink: &mut impl Sink,
        terminal_model: &parking_lot::FairMutex<crate::terminal::TerminalModel>,
    ) -> anyhow::Result<()> {
        let session_id = crate::terminal::bootstrap::generate_session_id();
        terminal_model.lock().register_session_id(session_id);
        let zsh_init_shell_script =
            init_shell_script_for_shell(ShellType::Zsh, &crate::ASSETS, session_id);
        sink.send(Message::new_binary(
            zsh_init_shell_script.as_bytes().to_vec(),
        ))
        .await?;

        sink.send(Message::new_binary(
            ShellType::Zsh.execute_command_bytes().to_vec(),
        ))
        .await?;

        Ok(())
    }

    /// Writes environment variables that should be defined in the session
    /// before bootstrapping. This is a subset of the environment variables
    /// defined in `app/src/terminal/local_tty/unix.rs` that are necessary in
    /// order to dogfood Warp on Web over the remote tty.
    async fn write_env_vars(
        sink: &mut impl Sink,
        is_honor_ps1_enabled: bool,
    ) -> anyhow::Result<()> {
        let honor_ps1_env_var = format!(r#"WARP_HONOR_PS1="{}";"#, is_honor_ps1_enabled as u8);
        sink.send(Message::new_binary(honor_ps1_env_var.as_bytes().to_vec()))
            .await?;

        Ok(())
    }

    fn on_ws_connection(
        &mut self,
        connection: anyhow::Result<WebSocket>,
        ctx: &mut ModelContext<Self>,
    ) {
        let connection = match connection {
            Ok(connection) => connection,
            Err(e) => {
                report_error!(e.context("Failed to construct websocket connection"));
                return;
            }
        };

        ctx.spawn(connection.split(), |me, (sink, stream), ctx| {
            me.start_websocket_listener_and_writer_tasks(sink, stream, ctx);
        });
    }

    /// Processes a byte slice through the `Processor`.
    fn process_pty_bytes(&mut self, bytes: &[u8]) {
        let mut terminal_model = self.terminal_model.lock();
        self.parser
            .parse_bytes(&mut *terminal_model, bytes, &mut io::sink());
        self.channel_event_listener.send_wakeup_event();
    }
}

impl Entity for EventLoop {
    type Event = ();
}
