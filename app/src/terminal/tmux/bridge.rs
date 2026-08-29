use std::collections::{HashMap, VecDeque};
use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use parking_lot::{FairMutex, Mutex};
use warp_core::SessionId;
use warpui::WindowId;

use super::parser::PaneId;
use crate::terminal::TerminalModel;
use crate::terminal::model::ansi;
use crate::terminal::model::terminal_model::TmuxClientEvent;
use crate::terminal::shell::ShellType;
use crate::terminal::tmux::pane_bytes::sink_writer;

const MAX_BUFFERED_PANE_BYTES: usize = 64 * 1024;
const MAX_UNREGISTERED_PANE_COUNT: usize = 64;
const MAX_UNREGISTERED_PANE_BYTES: usize = 256 * 1024;
const MAX_PENDING_CLIENT_EVENT_BYTES: usize = 8 * 1024;
const PENDING_CLIENT_EVENT_OVERHEAD: usize = 64;
pub const APP_BIND_TIMEOUT: Duration = Duration::from_secs(8);
pub const BOOTSTRAP_INFLIGHT_TIMEOUT: Duration = Duration::from_secs(8);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TmuxInstanceId(u64);

impl TmuxInstanceId {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }

    pub fn as_u64(self) -> u64 {
        self.0
    }

    pub fn from_u64(id: u64) -> Self {
        Self(id)
    }
}

struct PaneSink {
    model: Arc<FairMutex<TerminalModel>>,
    processor: ansi::Processor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneBootstrapState {
    Unsent,
    Staging,
    Ready,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneBootstrapClaim {
    pub pane_id: String,
    pub shell_type: ShellType,
    pub session_id: SessionId,
    pub generation: u64,
    pub retired_session_id: Option<SessionId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BootstrapTimeoutResult {
    Retry(PaneBootstrapClaim),
    Failed,
    Stale,
}

type ReadyComplete = (ShellType, SessionId, Option<Arc<FairMutex<TerminalModel>>>);

enum PaneBootstrapEntry {
    Staging {
        generation: u64,
        session_id: SessionId,
        retried: bool,
        timer_armed: bool,
    },
    Ready {
        generation: u64,
        session_id: SessionId,
    },
    Failed {
        generation: u64,
        session_id: SessionId,
    },
}

impl PaneBootstrapEntry {
    fn session_id(&self) -> SessionId {
        match self {
            Self::Staging { session_id, .. }
            | Self::Ready { session_id, .. }
            | Self::Failed { session_id, .. } => *session_id,
        }
    }
}

struct Inner {
    gateway_window: Option<WindowId>,
    presentation_window: Option<WindowId>,
    panes: HashMap<String, PaneSink>,
    buffers: HashMap<String, Vec<u8>>,
    pending_captures: VecDeque<String>,
    pending_client_events: Vec<TmuxClientEvent>,
    pending_client_event_bytes: usize,
    pane_bootstrap: HashMap<String, PaneBootstrapEntry>,
    pending_bootstrap_panes: VecDeque<String>,
    bootstrap_stage_count: HashMap<String, u32>,
    bootstrap_script_count: HashMap<String, u32>,
    next_generation: u64,
    tracked_control_pane: Option<String>,
    control_incarnation: u64,
    tracked_expected_session: Option<SessionId>,
    spawned_expected_session: Option<SessionId>,
    pending_retained_zsh_init: Option<(String, SessionId)>,
    retained_zsh_init: Option<(String, SessionId)>,
    early_init_shell: HashMap<String, (SessionId, u64)>,
    early_stage_complete: HashMap<String, SessionId>,
    pending_silent_bootstrap: VecDeque<(String, ShellType)>,
    shell_type: Option<ShellType>,
    app_bind_deadline: Option<instant::Instant>,
    presentation_ready: bool,
    unregistered_bytes: usize,
}

/// One Warp-managed tmux control-mode session (gateway PTY + presentation window).
pub struct TmuxRuntime {
    id: TmuxInstanceId,
    inner: Mutex<Inner>,
    applying: AtomicBool,
}

struct TmuxRuntimeIndex {
    by_id: HashMap<TmuxInstanceId, Arc<TmuxRuntime>>,
    by_gateway: HashMap<WindowId, TmuxInstanceId>,
    by_presentation: HashMap<WindowId, TmuxInstanceId>,
}

fn index() -> parking_lot::MutexGuard<'static, TmuxRuntimeIndex> {
    static INDEX: OnceLock<Mutex<TmuxRuntimeIndex>> = OnceLock::new();
    INDEX
        .get_or_init(|| {
            Mutex::new(TmuxRuntimeIndex {
                by_id: HashMap::new(),
                by_gateway: HashMap::new(),
                by_presentation: HashMap::new(),
            })
        })
        .lock()
}

impl TmuxRuntime {
    pub fn new() -> Arc<Self> {
        let runtime = Arc::new(Self {
            id: TmuxInstanceId::new(),
            inner: Mutex::new(Inner {
                gateway_window: None,
                presentation_window: None,
                panes: HashMap::new(),
                buffers: HashMap::new(),
                pending_captures: VecDeque::new(),
                pending_client_events: Vec::new(),
                pending_client_event_bytes: 0,
                pane_bootstrap: HashMap::new(),
                pending_bootstrap_panes: VecDeque::new(),
                bootstrap_stage_count: HashMap::new(),
                bootstrap_script_count: HashMap::new(),
                next_generation: 0,
                tracked_control_pane: None,
                control_incarnation: 0,
                tracked_expected_session: None,
                spawned_expected_session: None,
                pending_retained_zsh_init: None,
                retained_zsh_init: None,
                early_init_shell: HashMap::new(),
                early_stage_complete: HashMap::new(),
                pending_silent_bootstrap: VecDeque::new(),
                shell_type: None,
                app_bind_deadline: None,
                presentation_ready: false,
                unregistered_bytes: 0,
            }),
            applying: AtomicBool::new(false),
        });
        index().by_id.insert(runtime.id, runtime.clone());
        runtime
    }

    pub fn id(&self) -> TmuxInstanceId {
        self.id
    }

    pub fn insert(runtime: Arc<Self>) {
        index().by_id.insert(runtime.id, runtime);
    }

    pub fn for_id(id: TmuxInstanceId) -> Option<Arc<Self>> {
        index().by_id.get(&id).cloned()
    }

    pub fn for_gateway(window: WindowId) -> Option<Arc<Self>> {
        let idx = index();
        let id = *idx.by_gateway.get(&window)?;
        idx.by_id.get(&id).cloned()
    }

    pub fn for_presentation(window: WindowId) -> Option<Arc<Self>> {
        let idx = index();
        let id = *idx.by_presentation.get(&window)?;
        idx.by_id.get(&id).cloned()
    }

    pub fn bind_gateway(self: &Arc<Self>, window: WindowId) {
        self.inner.lock().gateway_window = Some(window);
        let mut idx = index();
        idx.by_id.insert(self.id, Arc::clone(self));
        idx.by_gateway.insert(window, self.id);
    }

    pub fn bind_presentation(self: &Arc<Self>, window: WindowId) {
        self.inner.lock().presentation_window = Some(window);
        let mut idx = index();
        idx.by_id.insert(self.id, Arc::clone(self));
        idx.by_presentation.insert(window, self.id);
    }

    pub fn gateway_window(&self) -> Option<WindowId> {
        self.inner.lock().gateway_window
    }

    pub fn presentation_window(&self) -> Option<WindowId> {
        self.inner.lock().presentation_window
    }

    pub fn unregister(&self) {
        let retire = {
            let mut inner = self.inner.lock();
            {
                let mut idx = index();
                idx.by_id.remove(&self.id);
                if let Some(gateway) = inner.gateway_window.take() {
                    idx.by_gateway.remove(&gateway);
                }
                if let Some(presentation) = inner.presentation_window.take() {
                    idx.by_presentation.remove(&presentation);
                }
            }
            self.applying.store(false, Ordering::SeqCst);
            let mut retire = Vec::new();
            let bootstrap = std::mem::take(&mut inner.pane_bootstrap);
            for (pane_id, entry) in bootstrap {
                if let Some(sink) = inner.panes.remove(&pane_id) {
                    retire.push((sink.model, entry.session_id()));
                }
            }
            inner.panes.clear();
            inner.buffers.clear();
            inner.pending_captures.clear();
            inner.pending_client_events.clear();
            inner.pending_client_event_bytes = 0;
            inner.pending_bootstrap_panes.clear();
            inner.bootstrap_stage_count.clear();
            inner.bootstrap_script_count.clear();
            inner.next_generation = 0;
            inner.tracked_control_pane = None;
            inner.control_incarnation = 0;
            inner.tracked_expected_session = None;
            inner.spawned_expected_session = None;
            inner.pending_retained_zsh_init = None;
            Self::clear_retained_zsh_init_locked(&mut inner, None);
            inner.early_init_shell.clear();
            inner.early_stage_complete.clear();
            inner.pending_silent_bootstrap.clear();
            inner.app_bind_deadline = None;
            inner.presentation_ready = false;
            inner.unregistered_bytes = 0;
            retire
        };
        for (model, session_id) in retire {
            model.lock().unregister_session_id(session_id);
        }
    }

    pub fn is_applying(&self) -> bool {
        self.applying.load(Ordering::SeqCst)
    }

    pub fn with_applying<T>(&self, f: impl FnOnce() -> T) -> T {
        self.applying.store(true, Ordering::SeqCst);
        let result = f();
        self.applying.store(false, Ordering::SeqCst);
        result
    }

    pub fn register_pane(&self, pane_id: &str, model: Arc<FairMutex<TerminalModel>>) {
        let mut inner = self.inner.lock();
        let buffered = inner.buffers.remove(pane_id).unwrap_or_default();
        inner.unregistered_bytes = inner.unregistered_bytes.saturating_sub(buffered.len());
        let mut sink = PaneSink {
            model,
            processor: ansi::Processor::new(),
        };
        if !buffered.is_empty() {
            feed_sink(&mut sink, &buffered);
        }
        inner.panes.insert(pane_id.to_owned(), sink);
    }

    pub fn unregister_pane(&self, pane_id: &str) {
        let retire = {
            let mut inner = self.inner.lock();
            let session_id = inner
                .pane_bootstrap
                .remove(pane_id)
                .map(|entry| entry.session_id());
            Self::clear_retained_zsh_init_locked(&mut inner, Some(pane_id));
            inner.early_init_shell.remove(pane_id);
            inner.early_stage_complete.remove(pane_id);
            inner
                .pending_silent_bootstrap
                .retain(|(id, _)| id != pane_id);
            if inner.tracked_control_pane.as_deref() == Some(pane_id) {
                inner.tracked_control_pane = None;
                inner.tracked_expected_session = None;
                inner.spawned_expected_session = None;
                inner.control_incarnation = next_control_incarnation(inner.control_incarnation);
            }
            let model = inner.panes.remove(pane_id).map(|sink| sink.model);
            inner.pending_bootstrap_panes.retain(|id| id != pane_id);
            inner.bootstrap_stage_count.remove(pane_id);
            inner.bootstrap_script_count.remove(pane_id);
            session_id.zip(model)
        };
        if let Some((session_id, model)) = retire {
            model.lock().unregister_session_id(session_id);
        }
    }

    pub fn deliver_output(&self, pane_id: &PaneId, bytes: &[u8]) -> bool {
        {
            let mut inner = self.inner.lock();
            Self::maybe_track_spawned_pane_locked(&mut inner, pane_id.as_str());
        }
        let (bytes, stage_complete) = super::protocol::split_stage_complete(bytes);
        for session_id in stage_complete {
            let _ = self.on_stage_complete(pane_id.as_str(), session_id);
        }
        let bytes = bytes.as_slice();
        let mut inner = self.inner.lock();
        if let Some(sink) = inner.panes.get_mut(pane_id.as_str()) {
            feed_sink(sink, bytes);
            return true;
        }
        let is_new_pane = !inner.buffers.contains_key(pane_id.as_str());
        if is_new_pane && inner.buffers.len() >= MAX_UNREGISTERED_PANE_COUNT {
            clear_unregistered_buffers(&mut inner);
            return false;
        }
        let buffered_len = inner
            .buffers
            .get(pane_id.as_str())
            .map(Vec::len)
            .unwrap_or(0);
        let room = MAX_BUFFERED_PANE_BYTES.saturating_sub(buffered_len);
        if room == 0 {
            return true;
        }
        let take = bytes.len().min(room);
        if inner.unregistered_bytes.saturating_add(take) > MAX_UNREGISTERED_PANE_BYTES {
            clear_unregistered_buffers(&mut inner);
            return false;
        }
        inner
            .buffers
            .entry(pane_id.as_str().to_owned())
            .or_default()
            .extend_from_slice(&bytes[..take]);
        inner.unregistered_bytes += take;
        true
    }

    pub fn note_capture(&self, pane_id: &str) {
        self.inner
            .lock()
            .pending_captures
            .push_back(pane_id.to_owned());
    }

    pub fn take_capture(&self) -> Option<String> {
        self.inner.lock().pending_captures.pop_front()
    }

    pub fn buffer_client_events(&self, events: &[TmuxClientEvent]) -> bool {
        if events.is_empty() {
            return true;
        }
        let mut inner = self.inner.lock();
        for event in events {
            let incoming_bytes = client_event_retained_bytes(event);
            if incoming_bytes > MAX_PENDING_CLIENT_EVENT_BYTES {
                log::warn!(
                    "tmux runtime {} pending client event overflow; rolling back",
                    self.id.as_u64()
                );
                inner.pending_client_events.clear();
                inner.pending_client_event_bytes = 0;
                return false;
            }
            if let TmuxClientEvent::LayoutChange { window_id, .. } = event
                && let Some(TmuxClientEvent::LayoutChange {
                    window_id: last_id, ..
                }) = inner.pending_client_events.last()
                && last_id == window_id
            {
                let previous_bytes =
                    client_event_retained_bytes(inner.pending_client_events.last().unwrap());
                let next_bytes = inner.pending_client_event_bytes - previous_bytes + incoming_bytes;
                if next_bytes > MAX_PENDING_CLIENT_EVENT_BYTES {
                    log::warn!(
                        "tmux runtime {} pending client event overflow; rolling back",
                        self.id.as_u64()
                    );
                    inner.pending_client_events.clear();
                    inner.pending_client_event_bytes = 0;
                    return false;
                }
                *inner.pending_client_events.last_mut().unwrap() = event.clone();
                inner.pending_client_event_bytes = next_bytes;
                continue;
            }
            let next_bytes = inner.pending_client_event_bytes + incoming_bytes;
            if next_bytes > MAX_PENDING_CLIENT_EVENT_BYTES {
                log::warn!(
                    "tmux runtime {} pending client event overflow; rolling back",
                    self.id.as_u64()
                );
                inner.pending_client_events.clear();
                inner.pending_client_event_bytes = 0;
                return false;
            }
            inner.pending_client_events.push(event.clone());
            inner.pending_client_event_bytes = next_bytes;
        }
        log::info!(
            "tmux runtime {} buffering {} client events until presentation binds",
            self.id.as_u64(),
            events.len()
        );
        true
    }

    pub fn take_client_events(&self) -> Vec<TmuxClientEvent> {
        let mut inner = self.inner.lock();
        inner.pending_client_event_bytes = 0;
        std::mem::take(&mut inner.pending_client_events)
    }

    pub fn note_shell_type(&self, shell_type: ShellType) {
        self.inner.lock().shell_type = Some(shell_type);
    }

    pub fn set_authoritative_shell_type(&self, shell_type: ShellType) -> Vec<PaneBootstrapClaim> {
        let mut inner = self.inner.lock();
        inner.shell_type = Some(shell_type);
        let pending = std::mem::take(&mut inner.pending_bootstrap_panes);
        let mut claims = Vec::new();
        for pane_id in pending {
            let session_id = warp_terminal::bootstrap::generate_session_id();
            if let Some(claim) = Self::begin_pane_bootstrap_locked(&mut inner, &pane_id, session_id)
            {
                claims.push(claim);
            }
        }
        claims
    }

    pub fn shell_type(&self) -> Option<ShellType> {
        self.inner.lock().shell_type
    }

    pub fn begin_pane_bootstrap(
        &self,
        pane_id: &str,
        session_id: SessionId,
    ) -> Option<PaneBootstrapClaim> {
        let mut inner = self.inner.lock();
        Self::begin_pane_bootstrap_locked(&mut inner, pane_id, session_id)
    }

    fn begin_pane_bootstrap_locked(
        inner: &mut Inner,
        pane_id: &str,
        session_id: SessionId,
    ) -> Option<PaneBootstrapClaim> {
        inner.pending_bootstrap_panes.retain(|id| id != pane_id);
        match inner.pane_bootstrap.get(pane_id) {
            Some(PaneBootstrapEntry::Ready { .. }) | Some(PaneBootstrapEntry::Staging { .. }) => {
                return None;
            }
            Some(PaneBootstrapEntry::Failed { .. }) | None => {}
        }
        let Some(shell_type) = inner.shell_type else {
            if !inner.pending_bootstrap_panes.iter().any(|id| id == pane_id) {
                inner.pending_bootstrap_panes.push_back(pane_id.to_owned());
            }
            return None;
        };
        let session_id = inner
            .early_init_shell
            .get(pane_id)
            .and_then(|(sid, incarnation)| {
                (*incarnation == inner.control_incarnation).then_some(*sid)
            })
            .unwrap_or(session_id);
        let generation = Self::next_bootstrap_generation(inner);
        inner.pane_bootstrap.insert(
            pane_id.to_owned(),
            PaneBootstrapEntry::Staging {
                generation,
                session_id,
                retried: false,
                timer_armed: false,
            },
        );
        *inner
            .bootstrap_stage_count
            .entry(pane_id.to_owned())
            .or_insert(0) += 1;
        if Self::init_shell_accepted(inner, pane_id, session_id)
            && inner.early_stage_complete.get(pane_id).copied() == Some(session_id)
        {
            let _ = Self::complete_staging_to_ready_locked(inner, pane_id, session_id, true);
        }
        Some(PaneBootstrapClaim {
            pane_id: pane_id.to_owned(),
            shell_type,
            session_id,
            generation,
            retired_session_id: None,
        })
    }

    fn next_bootstrap_generation(inner: &mut Inner) -> u64 {
        inner.next_generation = inner.next_generation.wrapping_add(1);
        if inner.next_generation == 0 {
            inner.next_generation = 1;
        }
        inner.next_generation
    }

    pub fn on_init_shell(&self, pane_id: &str, session_id: SessionId) -> Option<ShellType> {
        let mut inner = self.inner.lock();
        match inner.pane_bootstrap.get(pane_id) {
            Some(PaneBootstrapEntry::Staging {
                session_id: staged_id,
                ..
            }) if *staged_id == session_id => {}
            _ => return None,
        }
        let shell_type = inner.shell_type?;
        let incarnation = inner.control_incarnation;
        inner
            .early_init_shell
            .insert(pane_id.to_owned(), (session_id, incarnation));
        let completed = Self::complete_if_init_and_ack_locked(&mut inner, pane_id, session_id);
        drop(inner);
        if let Some((_, staged_id, model)) = completed {
            Self::apply_ready_session(model, staged_id, session_id);
        }
        Some(shell_type)
    }

    pub fn on_stage_complete(&self, pane_id: &str, session_id: SessionId) -> Option<ShellType> {
        let (shell_type, staged_id, model) = {
            let mut inner = self.inner.lock();
            if !Self::ack_is_admissible(&inner, pane_id, session_id) {
                return None;
            }
            inner
                .early_stage_complete
                .insert(pane_id.to_owned(), session_id);
            Self::complete_if_init_and_ack_locked(&mut inner, pane_id, session_id)?
        };
        Self::apply_ready_session(model, staged_id, session_id);
        Some(shell_type)
    }

    fn ack_is_admissible(inner: &Inner, pane_id: &str, session_id: SessionId) -> bool {
        let matches_expected = inner.tracked_control_pane.as_deref() == Some(pane_id)
            && inner.tracked_expected_session == Some(session_id);
        if !inner.pane_bootstrap.contains_key(pane_id) {
            return matches_expected;
        }
        let matches_staging = matches!(
            inner.pane_bootstrap.get(pane_id),
            Some(PaneBootstrapEntry::Staging {
                session_id: staged_id,
                ..
            }) if *staged_id == session_id
        );
        matches_expected || matches_staging
    }

    fn init_shell_accepted(inner: &Inner, pane_id: &str, session_id: SessionId) -> bool {
        inner
            .early_init_shell
            .get(pane_id)
            .is_some_and(|(sid, inc)| *sid == session_id && *inc == inner.control_incarnation)
    }

    fn complete_if_init_and_ack_locked(
        inner: &mut Inner,
        pane_id: &str,
        session_id: SessionId,
    ) -> Option<ReadyComplete> {
        if !Self::init_shell_accepted(inner, pane_id, session_id) {
            return None;
        }
        if inner.early_stage_complete.get(pane_id).copied() != Some(session_id) {
            return None;
        }
        let matches_expected = inner.tracked_control_pane.as_deref() == Some(pane_id)
            && inner.tracked_expected_session == Some(session_id);
        let retried = matches!(
            inner.pane_bootstrap.get(pane_id),
            Some(PaneBootstrapEntry::Staging { retried: true, .. })
        );
        let (shell_type, staged_id) = Self::complete_staging_to_ready_locked(
            inner,
            pane_id,
            session_id,
            matches_expected && !retried,
        )?;
        inner.early_init_shell.remove(pane_id);
        inner.early_stage_complete.remove(pane_id);
        Self::clear_retained_zsh_init_locked(inner, Some(pane_id));
        if matches_expected {
            inner.tracked_expected_session = None;
        }
        let model = inner.panes.get(pane_id).map(|sink| sink.model.clone());
        Some((shell_type, staged_id, model))
    }

    fn apply_ready_session(
        model: Option<Arc<FairMutex<TerminalModel>>>,
        staged_id: SessionId,
        session_id: SessionId,
    ) {
        let Some(model) = model else {
            return;
        };
        let mut locked = model.lock();
        if staged_id != session_id {
            locked.unregister_session_id(staged_id);
        }
        locked.register_session_id(session_id);
    }

    fn complete_staging_to_ready_locked(
        inner: &mut Inner,
        pane_id: &str,
        session_id: SessionId,
        accept_mismatch: bool,
    ) -> Option<(ShellType, SessionId)> {
        let (generation, staged_id) = match inner.pane_bootstrap.get(pane_id) {
            Some(PaneBootstrapEntry::Staging {
                generation,
                session_id: staged_id,
                ..
            }) => (*generation, *staged_id),
            _ => return None,
        };
        if staged_id != session_id && !accept_mismatch {
            return None;
        }
        let shell_type = inner.shell_type?;
        inner.pane_bootstrap.insert(
            pane_id.to_owned(),
            PaneBootstrapEntry::Ready {
                generation,
                session_id,
            },
        );
        *inner
            .bootstrap_script_count
            .entry(pane_id.to_owned())
            .or_insert(0) += 1;
        inner
            .pending_silent_bootstrap
            .push_back((pane_id.to_owned(), shell_type));
        Some((shell_type, staged_id))
    }

    pub fn take_pending_silent_bootstrap(&self) -> Vec<(String, ShellType)> {
        std::mem::take(&mut self.inner.lock().pending_silent_bootstrap)
            .into_iter()
            .collect()
    }

    pub fn arm_bootstrap_timeout(&self, pane_id: &str) -> Option<(u64, Duration)> {
        let mut inner = self.inner.lock();
        match inner.pane_bootstrap.get_mut(pane_id) {
            Some(PaneBootstrapEntry::Staging {
                generation,
                timer_armed,
                ..
            }) if !*timer_armed => {
                *timer_armed = true;
                Some((*generation, BOOTSTRAP_INFLIGHT_TIMEOUT))
            }
            _ => None,
        }
    }

    pub fn handle_bootstrap_timeout(
        &self,
        pane_id: &str,
        generation: u64,
    ) -> BootstrapTimeoutResult {
        let mut inner = self.inner.lock();
        let (session_id, retried) = match inner.pane_bootstrap.get(pane_id) {
            Some(PaneBootstrapEntry::Staging {
                generation: live,
                session_id,
                retried,
                ..
            }) if *live == generation => (*session_id, *retried),
            _ => return BootstrapTimeoutResult::Stale,
        };
        let Some(shell_type) = inner.shell_type else {
            return BootstrapTimeoutResult::Stale;
        };
        inner.tracked_expected_session = None;
        inner.spawned_expected_session = None;
        inner.pending_retained_zsh_init = None;
        Self::clear_retained_zsh_init_locked(&mut inner, Some(pane_id));
        inner.early_stage_complete.remove(pane_id);
        inner.early_init_shell.remove(pane_id);
        if retried {
            inner.pane_bootstrap.insert(
                pane_id.to_owned(),
                PaneBootstrapEntry::Failed {
                    generation,
                    session_id,
                },
            );
            return BootstrapTimeoutResult::Failed;
        }
        let next_gen = Self::next_bootstrap_generation(&mut inner);
        let next_session = warp_terminal::bootstrap::generate_session_id();
        inner.pane_bootstrap.insert(
            pane_id.to_owned(),
            PaneBootstrapEntry::Staging {
                generation: next_gen,
                session_id: next_session,
                retried: true,
                timer_armed: false,
            },
        );
        *inner
            .bootstrap_stage_count
            .entry(pane_id.to_owned())
            .or_insert(0) += 1;
        BootstrapTimeoutResult::Retry(PaneBootstrapClaim {
            pane_id: pane_id.to_owned(),
            shell_type,
            session_id: next_session,
            generation: next_gen,
            retired_session_id: Some(session_id),
        })
    }

    pub fn pane_bootstrap_state(&self, pane_id: &str) -> PaneBootstrapState {
        match self.inner.lock().pane_bootstrap.get(pane_id) {
            Some(PaneBootstrapEntry::Staging { .. }) => PaneBootstrapState::Staging,
            Some(PaneBootstrapEntry::Ready { .. }) => PaneBootstrapState::Ready,
            Some(PaneBootstrapEntry::Failed { .. }) => PaneBootstrapState::Failed,
            None => PaneBootstrapState::Unsent,
        }
    }

    pub fn pane_bootstrap_ready(&self, pane_id: &str) -> bool {
        self.pane_bootstrap_state(pane_id) == PaneBootstrapState::Ready
    }

    pub fn pane_bootstrap_failed(&self, pane_id: &str) -> bool {
        self.pane_bootstrap_state(pane_id) == PaneBootstrapState::Failed
    }

    pub fn bootstrap_stage_count(&self, pane_id: &str) -> u32 {
        self.inner
            .lock()
            .bootstrap_stage_count
            .get(pane_id)
            .copied()
            .unwrap_or(0)
    }

    pub fn bootstrap_script_count(&self, pane_id: &str) -> u32 {
        self.inner
            .lock()
            .bootstrap_script_count
            .get(pane_id)
            .copied()
            .unwrap_or(0)
    }

    pub fn pane_bootstrap_generation(&self, pane_id: &str) -> Option<u64> {
        match self.inner.lock().pane_bootstrap.get(pane_id) {
            Some(PaneBootstrapEntry::Staging { generation, .. })
            | Some(PaneBootstrapEntry::Ready { generation, .. })
            | Some(PaneBootstrapEntry::Failed { generation, .. }) => Some(*generation),
            None => None,
        }
    }

    pub fn pane_bootstrap_session_id(&self, pane_id: &str) -> Option<SessionId> {
        match self.inner.lock().pane_bootstrap.get(pane_id) {
            Some(PaneBootstrapEntry::Staging { session_id, .. })
            | Some(PaneBootstrapEntry::Ready { session_id, .. })
            | Some(PaneBootstrapEntry::Failed { session_id, .. }) => Some(*session_id),
            None => None,
        }
    }

    pub fn note_tracked_control_pane(&self, pane_id: &str) {
        let mut inner = self.inner.lock();
        if inner.tracked_control_pane.as_deref() == Some(pane_id) {
            return;
        }
        Self::clear_retained_zsh_init_locked(&mut inner, None);
        inner.tracked_control_pane = Some(pane_id.to_owned());
        inner.control_incarnation = next_control_incarnation(inner.control_incarnation);
    }

    pub fn set_tracked_expected_session(&self, session_id: SessionId) {
        let mut inner = self.inner.lock();
        if inner.tracked_control_pane.is_some() {
            if inner
                .retained_zsh_init
                .as_ref()
                .is_some_and(|(_, owned)| *owned != session_id)
            {
                inner.retained_zsh_init = None;
            }
            inner.tracked_expected_session = Some(session_id);
        }
    }

    pub fn set_spawned_expected_session(&self, session_id: SessionId) {
        self.inner.lock().spawned_expected_session = Some(session_id);
    }

    pub fn set_pending_retained_zsh_init(&self, script: String, session_id: SessionId) {
        self.inner.lock().pending_retained_zsh_init = Some((script, session_id));
    }

    pub fn take_retained_init_send_keys(&self, pane_id: &str) -> Vec<Vec<u8>> {
        let Some((script, session_id)) = self.inner.lock().pending_retained_zsh_init.take() else {
            return Vec::new();
        };
        self.note_tracked_control_pane(pane_id);
        self.set_tracked_expected_session(session_id);
        self.note_retained_zsh_init(pane_id, session_id);
        let bytes = super::protocol::zsh_init_bytes(&script, ShellType::Zsh, session_id);
        super::protocol::send_keys_commands(&PaneId::from(pane_id), &bytes)
            .into_iter()
            .map(String::into_bytes)
            .collect()
    }

    pub fn spawned_expected_session(&self) -> Option<SessionId> {
        self.inner.lock().spawned_expected_session
    }

    pub fn tracked_expected_session(&self) -> Option<SessionId> {
        self.inner.lock().tracked_expected_session
    }

    fn maybe_track_spawned_pane_locked(inner: &mut Inner, pane_id: &str) {
        if inner.tracked_control_pane.is_some() {
            return;
        }
        let Some(session_id) = inner.spawned_expected_session else {
            return;
        };
        Self::clear_retained_zsh_init_locked(inner, None);
        inner.tracked_control_pane = Some(pane_id.to_owned());
        inner.control_incarnation = next_control_incarnation(inner.control_incarnation);
        inner.tracked_expected_session = Some(session_id);
    }

    pub fn admit_buffered_init_shell(
        &self,
        pane_id: &str,
        model: &FairMutex<TerminalModel>,
    ) -> Option<ShellType> {
        let (session_id, shell_type) = {
            let locked = model.lock();
            (locked.pending_session_id(), locked.pending_shell_type())
        };
        let session_id = session_id?;
        let shell_type = shell_type?;
        self.note_early_init_shell(pane_id, session_id, shell_type)
    }

    pub fn tracked_control_pane(&self) -> Option<String> {
        self.inner.lock().tracked_control_pane.clone()
    }

    pub fn note_retained_zsh_init(&self, pane_id: &str, session_id: SessionId) {
        let mut inner = self.inner.lock();
        if inner.tracked_control_pane.as_deref() != Some(pane_id) {
            return;
        }
        inner.retained_zsh_init = Some((pane_id.to_owned(), session_id));
    }

    pub fn control_pane_owns_retained_init(&self, pane_id: &str) -> bool {
        let inner = self.inner.lock();
        Self::owns_retained_init_locked(&inner, pane_id)
    }

    fn owns_retained_init_locked(inner: &Inner, pane_id: &str) -> bool {
        let Some((owned_pane, owned_session)) = inner.retained_zsh_init.as_ref() else {
            return false;
        };
        owned_pane == pane_id
            && inner.tracked_control_pane.as_deref() == Some(pane_id)
            && inner.tracked_expected_session == Some(*owned_session)
    }

    fn clear_retained_zsh_init_locked(inner: &mut Inner, pane_id: Option<&str>) {
        match pane_id {
            Some(pane_id) => {
                if inner
                    .retained_zsh_init
                    .as_ref()
                    .is_some_and(|(owned, _)| owned == pane_id)
                {
                    inner.retained_zsh_init = None;
                }
            }
            None => inner.retained_zsh_init = None,
        }
    }

    pub fn bootstrap_failed_client_event() -> TmuxClientEvent {
        TmuxClientEvent::PresentationUnready
    }

    pub fn note_early_init_shell(
        &self,
        pane_id: &str,
        session_id: SessionId,
        shell_type: ShellType,
    ) -> Option<ShellType> {
        let mut inner = self.inner.lock();
        let matches_expected = inner.tracked_control_pane.as_deref() == Some(pane_id)
            && inner.tracked_expected_session == Some(session_id);
        let matches_staging = matches!(
            inner.pane_bootstrap.get(pane_id),
            Some(PaneBootstrapEntry::Staging {
                session_id: staged_id,
                ..
            }) if *staged_id == session_id
        );
        if !matches_expected && !matches_staging {
            return None;
        }
        inner.shell_type = Some(shell_type);
        let incarnation = inner.control_incarnation;
        inner
            .early_init_shell
            .insert(pane_id.to_owned(), (session_id, incarnation));
        let completed = Self::complete_if_init_and_ack_locked(&mut inner, pane_id, session_id);
        drop(inner);
        if let Some((_, staged_id, model)) = completed {
            Self::apply_ready_session(model, staged_id, session_id);
        }
        Some(shell_type)
    }

    pub fn early_init_session_id(&self, pane_id: &str) -> Option<SessionId> {
        self.inner
            .lock()
            .early_init_shell
            .get(pane_id)
            .map(|(session_id, _)| *session_id)
    }

    pub fn take_early_init_shell(&self, pane_id: &str) -> Option<SessionId> {
        self.inner
            .lock()
            .early_init_shell
            .remove(pane_id)
            .map(|(session_id, _)| session_id)
    }

    pub fn complete_if_early_init_shell(&self, pane_id: &str) -> Option<ShellType> {
        let session_id = self.take_early_init_shell(pane_id)?;
        self.on_init_shell(pane_id, session_id)
    }

    pub fn complete_if_early_stage_complete(&self, pane_id: &str) -> Option<ShellType> {
        let session_id = self
            .inner
            .lock()
            .early_stage_complete
            .get(pane_id)
            .copied()?;
        self.on_stage_complete(pane_id, session_id)
    }

    pub fn apply_claim_session(&self, claim: &PaneBootstrapClaim) {
        let Some(model) = self.pane_model(&claim.pane_id) else {
            return;
        };
        let mut locked = model.lock();
        if let Some(retired) = claim.retired_session_id {
            locked.unregister_session_id(retired);
        }
        locked.register_session_id(claim.session_id);
    }

    pub fn pane_model(&self, pane_id: &str) -> Option<Arc<FairMutex<TerminalModel>>> {
        self.inner
            .lock()
            .panes
            .get(pane_id)
            .map(|sink| sink.model.clone())
    }

    pub fn start_app_bind_deadline(&self) {
        let mut inner = self.inner.lock();
        inner.presentation_ready = false;
        inner.app_bind_deadline = Some(instant::Instant::now() + APP_BIND_TIMEOUT);
    }

    pub fn mark_presentation_ready(&self) {
        let mut inner = self.inner.lock();
        inner.presentation_ready = true;
        inner.app_bind_deadline = None;
    }

    pub fn is_presentation_ready(&self) -> bool {
        self.inner.lock().presentation_ready
    }

    pub fn app_bind_deadline_elapsed(&self, now: instant::Instant) -> bool {
        let inner = self.inner.lock();
        if inner.presentation_ready {
            return false;
        }
        inner
            .app_bind_deadline
            .is_some_and(|deadline| now >= deadline)
    }

    #[cfg(test)]
    fn buffered_output(&self, pane_id: &str) -> Option<Vec<u8>> {
        self.inner.lock().buffers.get(pane_id).cloned()
    }

    #[cfg(test)]
    fn live_binding_counts(&self) -> (usize, usize, usize) {
        let inner = self.inner.lock();
        (
            inner.panes.len(),
            inner.pane_bootstrap.len(),
            inner.early_init_shell.len(),
        )
    }
}

fn next_control_incarnation(current: u64) -> u64 {
    let next = current.wrapping_add(1);
    if next == 0 { 1 } else { next }
}

fn client_event_retained_bytes(event: &TmuxClientEvent) -> usize {
    let payload = match event {
        TmuxClientEvent::LayoutChange {
            window_id,
            layout,
            visible_layout,
            flags,
        } => {
            window_id.len()
                + layout.len()
                + visible_layout.as_ref().map(String::len).unwrap_or(0)
                + flags.as_ref().map(String::len).unwrap_or(0)
        }
        TmuxClientEvent::WindowAdd { window_id }
        | TmuxClientEvent::WindowClose { window_id }
        | TmuxClientEvent::SessionWindowChanged { window_id } => window_id.len(),
        TmuxClientEvent::WindowRenamed { window_id, name } => window_id.len() + name.len(),
        TmuxClientEvent::CommandEnd {
            payload,
            capture_pane,
            ..
        } => {
            payload.iter().map(String::len).sum::<usize>()
                + capture_pane.as_ref().map(String::len).unwrap_or(0)
        }
        TmuxClientEvent::PresentationUnready => 0,
    };
    PENDING_CLIENT_EVENT_OVERHEAD.saturating_add(payload)
}

fn clear_unregistered_buffers(inner: &mut Inner) {
    log::warn!("tmux unregistered pane output overflow; rolling back");
    inner.buffers.clear();
    inner.unregistered_bytes = 0;
}

fn feed_sink(sink: &mut PaneSink, bytes: &[u8]) {
    let mut writer = sink_writer();
    let mut model = sink.model.lock();
    sink.processor.parse_bytes(&mut *model, bytes, &mut writer);
    let _ = writer.flush();
    model.wake_for_tmux_output();
}

#[cfg(test)]
#[path = "bridge_tests.rs"]
mod tests;
