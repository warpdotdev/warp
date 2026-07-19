use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_broadcast::InactiveReceiver;
use warpui::r#async::SpawnedFutureHandle;
use warpui::{Entity, ModelContext, SingletonEntity, WindowId};

use crate::settings::{DebugSettings, DebugSettingsChangedEvent};
use crate::view_components::{DismissibleToast, ToastLink};
use crate::workspace::{ToastStack, WorkspaceAction};

/// Subdirectory under the application state directory for PTY recordings.
#[cfg(feature = "local_fs")]
const PTY_RECORDINGS_DIR: &str = "pty_recordings";

/// Per-session PTY recorder. Manages starting and stopping an async
/// recording task that writes PTY bytes to a distinct file.
pub struct PtyRecorder {
    /// Handle to the current recording task, if one is running.
    /// Aborting this handle stops the recording.
    recording_handle: Option<SpawnedFutureHandle>,
    /// Pre-computed file path for this session's recording. Fixed at
    /// construction time so start/stop cycles reuse the same path.
    path: PathBuf,
    /// Whether the per-session recording toggle is enabled.
    is_per_session_recording_enabled: bool,
    /// Inactive receiver for PTY reads. Only `Some` for local TTY sessions.
    #[cfg_attr(not(feature = "local_fs"), expect(dead_code))]
    pty_reads_rx: Option<InactiveReceiver<Arc<Vec<u8>>>>,
    /// Window ID used for showing toasts.
    window_id: WindowId,
}

impl Entity for PtyRecorder {
    type Event = ();
}

impl PtyRecorder {
    /// Creates a new recorder. Recording is enabled for a session if it
    /// is toggled on or if the global recording mode in [`DebugSettings`]
    /// is set.
    pub fn new(
        pty_reads_rx: Option<InactiveReceiver<Arc<Vec<u8>>>>,
        window_id: WindowId,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        ctx.subscribe_to_model(&DebugSettings::handle(ctx), |me, _, event, ctx| {
            if let DebugSettingsChangedEvent::RecordingModeEnabled { .. } = event {
                me.update_recording_state(ctx);
            }
        });

        let path = Self::recording_path(ctx);
        let mut recorder = Self {
            recording_handle: None,
            path,
            is_per_session_recording_enabled: false,
            pty_reads_rx,
            window_id,
        };

        // If the global recording mode is already enabled, start recording immediately.
        if *DebugSettings::as_ref(ctx).recording_mode {
            recorder.update_recording_state(ctx);
        }

        recorder
    }

    #[cfg(feature = "local_fs")]
    fn recording_path(ctx: &ModelContext<Self>) -> PathBuf {
        use chrono::Local;

        let recordings_dir = warp_core::paths::state_dir().join(PTY_RECORDINGS_DIR);
        let timestamp = Local::now().format("%Y-%m-%d_%H-%M-%S");
        recordings_dir.join(format!("{timestamp}-{}.pty.recording", ctx.model_id()))
    }

    #[cfg(not(feature = "local_fs"))]
    fn recording_path(_ctx: &ModelContext<Self>) -> PathBuf {
        PathBuf::new()
    }

    /// Whether recording is currently active.
    pub fn is_recording(&self) -> bool {
        self.recording_handle.is_some()
    }

    /// Toggle per-session recording and update the recording state.
    pub fn toggle_recording(&mut self, ctx: &mut ModelContext<Self>) {
        self.is_per_session_recording_enabled = !self.is_per_session_recording_enabled;
        self.update_recording_state(ctx);
    }

    /// Starts or stops recording based on the combined state of the global
    /// `RecordingMode` debug setting and the per-session toggle.
    fn update_recording_state(&mut self, ctx: &mut ModelContext<Self>) {
        let global_enabled = *DebugSettings::as_ref(ctx).recording_mode;
        let should_record = global_enabled || self.is_per_session_recording_enabled;

        if should_record && !self.is_recording() {
            if let Some(path) = self.start_recording(ctx) {
                let display_path = warp_core::paths::home_relative_path(path);
                let file_path = path.to_owned();
                self.show_toast(
                    format!("PTY recording started: {display_path}"),
                    Some(file_path),
                    ctx,
                );
            }
        } else if !should_record && self.is_recording() {
            let display_path = warp_core::paths::home_relative_path(&self.path);
            self.stop_recording();
            self.show_toast(
                format!("PTY recording stopped: {display_path}"),
                Some(self.path.clone()),
                ctx,
            );
        }
    }

    /// Start recording PTY bytes. Returns the path of the recording file
    /// on success, or `None` if recording could not be started.
    #[cfg(feature = "local_fs")]
    fn start_recording(&mut self, ctx: &mut ModelContext<Self>) -> Option<&Path> {
        use std::fs;

        // Stop any existing recording first.
        self.stop_recording();

        let pty_reads_rx = self.pty_reads_rx.as_ref()?;

        let recordings_dir = self.path.parent()?;
        if let Err(e) = fs::create_dir_all(recordings_dir) {
            log::warn!("Failed to create PTY recordings directory: {e}");
            return None;
        }

        let record_future = record_pty_bytes(pty_reads_rx.activate_cloned(), self.path.clone());
        self.recording_handle = Some(ctx.spawn(record_future, |_, _, _| {}));
        log::info!("Started PTY recording to {}", self.path.display());
        Some(&self.path)
    }

    /// No-op on platforms without local filesystem access.
    #[cfg(not(feature = "local_fs"))]
    fn start_recording(&mut self, _ctx: &mut ModelContext<Self>) -> Option<&Path> {
        None
    }

    /// Shows a toast with the given message. If `recording_path` is provided,
    /// clicking the toast body copies the path to the clipboard, and an
    /// "Open" link is shown that opens the file in the system file explorer.
    fn show_toast(
        &self,
        message: String,
        recording_path: Option<PathBuf>,
        ctx: &mut ModelContext<Self>,
    ) {
        let window_id = self.window_id;
        ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
            let mut toast = DismissibleToast::default(message);
            if let Some(path) = recording_path {
                let path_str = path.to_string_lossy().into_owned();
                toast = toast
                    .with_link(
                        ToastLink::new("Open".to_string())
                            .with_onclick_action(WorkspaceAction::OpenInExplorer { path }),
                    )
                    .with_on_body_click(move |ctx| {
                        ctx.dispatch_typed_action(&WorkspaceAction::CopyTextToClipboard(
                            path_str.clone(),
                        ));
                    });
            }
            toast_stack.add_ephemeral_toast(toast, window_id, ctx);
        });
    }

    /// Stop any active recording.
    fn stop_recording(&mut self) {
        if let Some(handle) = self.recording_handle.take() {
            log::info!("Stopped PTY recording to {}", self.path.display());
            handle.abort();
        }
    }
}

impl Drop for PtyRecorder {
    fn drop(&mut self) {
        self.stop_recording();
    }
}

/// Records all of the PTY reads that are received over the channel
/// to the given file path.
#[cfg(feature = "local_fs")]
async fn record_pty_bytes(
    mut pty_reads_rx: async_broadcast::Receiver<Arc<Vec<u8>>>,
    path: PathBuf,
) {
    use std::fs::OpenOptions;
    use std::io::Write;

    let mut file = None;
    while let Ok(bytes) = pty_reads_rx.recv().await {
        if file.is_none() {
            match OpenOptions::new().create(true).append(true).open(&path) {
                Ok(f) => {
                    file = Some(f);
                }
                Err(e) => {
                    log::info!(
                        "Failed to open file for PTY recording at {}: {e}",
                        path.display()
                    );
                }
            }
        }

        if let Some(file) = file.as_mut() {
            let write_res = file.write_all(bytes.as_slice());
            if let Err(e) = write_res {
                log::info!("Failed to write to PTY recording: {e}");
            }
        }
    }
}
