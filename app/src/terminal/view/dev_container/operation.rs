use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use instant::Instant;
use parking_lot::Mutex;
use uuid::Uuid;
use warpui::{Entity, ModelContext};

use super::kill::ProcessGroupKillOnDrop;
use super::registry::DevContainerBuildKey;
#[cfg(unix)]
use crate::terminal::local_tty::{ProcessGroupCancel, StagingProcessGroupKillOnDrop};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DevContainerBuildPhase {
    Build,
    Preflight,
    Staging,
    Attach,
}

pub(crate) const BUILD_SILENCE_THRESHOLD: Duration = Duration::from_secs(120);

impl DevContainerBuildPhase {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Build => "Build",
            Self::Preflight => "Preflight",
            Self::Staging => "Staging",
            Self::Attach => "Attach",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DevContainerBuildStatus {
    Running,
    Failed,
    Cancelling,
    Cancelled,
    Completed,
}

enum ArmedProcessGroupKill {
    Stream(ProcessGroupKillOnDrop),
    #[cfg(unix)]
    Staging(StagingProcessGroupKillOnDrop),
}

impl ArmedProcessGroupKill {
    fn terminate_now(&self) {
        match self {
            Self::Stream(kill) => kill.terminate_now(),
            #[cfg(unix)]
            Self::Staging(kill) => kill.terminate_now(),
        }
    }
}

#[derive(Default)]
struct DevContainerBuildCancelState {
    cancelled: bool,
    kill: Option<ArmedProcessGroupKill>,
}

#[derive(Clone)]
pub(crate) struct DevContainerBuildCancel {
    inner: Arc<Mutex<DevContainerBuildCancelState>>,
}

#[cfg(unix)]
impl ProcessGroupCancel for DevContainerBuildCancel {
    fn register_process_group(&self, kill_group: StagingProcessGroupKillOnDrop) -> bool {
        let mut inner = self.inner.lock();
        if inner.cancelled {
            return false;
        }
        inner.kill = Some(ArmedProcessGroupKill::Staging(kill_group));
        true
    }

    fn is_cancelled(&self) -> bool {
        DevContainerBuildCancel::is_cancelled(self)
    }
}

impl DevContainerBuildCancel {
    pub(crate) fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(DevContainerBuildCancelState::default())),
        }
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.inner.lock().cancelled
    }

    pub(crate) fn register_kill_group(&self, kill_group: ProcessGroupKillOnDrop) -> bool {
        let mut inner = self.inner.lock();
        if inner.cancelled {
            return false;
        }
        inner.kill = Some(ArmedProcessGroupKill::Stream(kill_group));
        true
    }

    pub(crate) fn mark_cancelled(&self) {
        let mut inner = self.inner.lock();
        inner.cancelled = true;
        if let Some(kill) = inner.kill.take() {
            kill.terminate_now();
        }
    }

    #[cfg(test)]
    pub(crate) fn has_armed_kill(&self) -> bool {
        self.inner.lock().kill.is_some()
    }
}

pub(crate) struct DevContainerBuildOperation {
    key: DevContainerBuildKey,
    operation_id: Uuid,
    attempt_id: u64,
    workspace_folder: PathBuf,
    config_file: PathBuf,
    phase: DevContainerBuildPhase,
    status: DevContainerBuildStatus,
    cancel: DevContainerBuildCancel,
    last_output_at: Arc<Mutex<Instant>>,
    output_tx: async_channel::Sender<()>,
    output_rx: async_channel::Receiver<()>,
    remote_server_session_id: Option<warp_core::SessionId>,
}

impl DevContainerBuildOperation {
    pub(crate) fn new(key: DevContainerBuildKey) -> Self {
        let workspace_folder = key.workspace_folder.clone();
        let config_file = key.config_file.clone();
        let (output_tx, output_rx) = async_channel::bounded(1);
        Self {
            key,
            operation_id: Uuid::new_v4(),
            attempt_id: 1,
            workspace_folder,
            config_file,
            phase: DevContainerBuildPhase::Build,
            status: DevContainerBuildStatus::Running,
            cancel: DevContainerBuildCancel::new(),
            last_output_at: Arc::new(Mutex::new(Instant::now())),
            output_tx,
            output_rx,
            remote_server_session_id: None,
        }
    }

    pub(crate) fn key(&self) -> &DevContainerBuildKey {
        &self.key
    }

    pub(crate) fn operation_id(&self) -> Uuid {
        self.operation_id
    }

    pub(crate) fn attempt_id(&self) -> u64 {
        self.attempt_id
    }

    pub(crate) fn workspace_folder(&self) -> &PathBuf {
        &self.workspace_folder
    }

    pub(crate) fn config_file(&self) -> &PathBuf {
        &self.config_file
    }

    pub(crate) fn phase(&self) -> DevContainerBuildPhase {
        self.phase
    }

    pub(crate) fn status(&self) -> DevContainerBuildStatus {
        self.status
    }

    pub(crate) fn header_title(&self) -> String {
        let workspace = self
            .workspace_folder
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.workspace_folder.display().to_string());
        match self.status() {
            DevContainerBuildStatus::Failed => {
                format!("{} · {} failed", workspace, self.phase().label())
            }
            DevContainerBuildStatus::Cancelling | DevContainerBuildStatus::Cancelled => {
                format!("{} · Cancelling", workspace)
            }
            DevContainerBuildStatus::Running | DevContainerBuildStatus::Completed => {
                format!("{} · {}", workspace, self.phase().label())
            }
        }
    }

    pub(crate) fn header_secondary(&self) -> String {
        if self.status != DevContainerBuildStatus::Running {
            return String::new();
        }
        silence_subtitle(self.last_output_at.lock().elapsed()).unwrap_or_default()
    }

    pub(crate) fn last_output_clock(&self) -> Arc<Mutex<Instant>> {
        self.last_output_at.clone()
    }

    pub(crate) fn output_elapsed(&self) -> Duration {
        self.last_output_at.lock().elapsed()
    }

    pub(crate) fn output_tx(&self) -> async_channel::Sender<()> {
        self.output_tx.clone()
    }

    pub(crate) fn output_rx(&self) -> async_channel::Receiver<()> {
        self.output_rx.clone()
    }

    pub(crate) fn shows_retry(&self) -> bool {
        self.status == DevContainerBuildStatus::Failed
    }

    pub(crate) fn shows_close(&self) -> bool {
        matches!(
            self.status,
            DevContainerBuildStatus::Running
                | DevContainerBuildStatus::Failed
                | DevContainerBuildStatus::Cancelling
        )
    }

    pub(crate) fn shows_retry_and_close(&self) -> bool {
        self.shows_retry()
    }

    pub(crate) fn cancel_handle(&self) -> DevContainerBuildCancel {
        self.cancel.clone()
    }

    pub(crate) fn is_current_attempt(&self, operation_id: Uuid, attempt_id: u64) -> bool {
        self.operation_id == operation_id
            && self.attempt_id == attempt_id
            && !matches!(
                self.status,
                DevContainerBuildStatus::Cancelled | DevContainerBuildStatus::Completed
            )
            && !self.cancel.is_cancelled()
    }

    pub(crate) fn set_phase(
        &mut self,
        phase: DevContainerBuildPhase,
        ctx: &mut ModelContext<Self>,
    ) {
        self.phase = phase;
        ctx.notify();
    }

    pub(crate) fn fail(&mut self, phase: DevContainerBuildPhase, ctx: &mut ModelContext<Self>) {
        self.phase = phase;
        self.status = DevContainerBuildStatus::Failed;
        ctx.notify();
    }

    pub(crate) fn complete(&mut self, ctx: &mut ModelContext<Self>) {
        self.status = DevContainerBuildStatus::Completed;
        ctx.notify();
    }

    /// Marks the operation cancelled before the caller terminates processes or
    /// removes the pane, so a late completion is a no-op.
    pub(crate) fn tombstone(&mut self, ctx: &mut ModelContext<Self>) {
        self.cancel.mark_cancelled();
        if self.status == DevContainerBuildStatus::Running {
            self.status = DevContainerBuildStatus::Cancelling;
        }
        ctx.notify();
    }

    pub(crate) fn mark_cancelled(&mut self, ctx: &mut ModelContext<Self>) {
        self.status = DevContainerBuildStatus::Cancelled;
        ctx.notify();
    }

    pub(crate) fn begin_retry(&mut self, ctx: &mut ModelContext<Self>) -> u64 {
        self.cancel.mark_cancelled();
        self.attempt_id += 1;
        self.phase = DevContainerBuildPhase::Build;
        self.status = DevContainerBuildStatus::Running;
        self.cancel = DevContainerBuildCancel::new();
        *self.last_output_at.lock() = Instant::now();
        self.remote_server_session_id = None;
        ctx.notify();
        self.attempt_id
    }

    pub(crate) fn set_remote_server_session_id(
        &mut self,
        session_id: Option<warp_core::SessionId>,
        ctx: &mut ModelContext<Self>,
    ) {
        self.remote_server_session_id = session_id;
        ctx.notify();
    }

    #[cfg(test)]
    pub(crate) fn remote_server_session_id(&self) -> Option<warp_core::SessionId> {
        self.remote_server_session_id
    }
}

pub(crate) fn silence_subtitle(elapsed: Duration) -> Option<String> {
    if elapsed < BUILD_SILENCE_THRESHOLD {
        return None;
    }
    let minutes = elapsed.as_secs() / 60;
    Some(format!("No output for {minutes}m"))
}

pub(crate) fn silence_watch_delay(elapsed: Duration) -> Duration {
    let remaining = BUILD_SILENCE_THRESHOLD.saturating_sub(elapsed);
    if remaining.is_zero() {
        Duration::from_secs(60)
    } else {
        remaining
    }
}

impl Entity for DevContainerBuildOperation {
    type Event = ();
}

#[cfg(test)]
#[path = "operation_tests.rs"]
mod tests;
