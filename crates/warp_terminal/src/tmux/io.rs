use std::borrow::Cow;
use std::collections::{HashSet, VecDeque};
use std::time::Duration;

use instant::Instant;

use super::encode::{
    EXIT_EMPTY_OFF_COMMAND, LIST_WINDOWS_LAYOUT_COMMAND, WARP_CONTROL_SOCKET_NAME,
    refresh_client_command, send_keys_command,
};
use super::parser::{ControlEvent, ControlModeParser, DecodeItem, PaneId, WindowId};

const START_PENDING_TIMEOUT: Duration = Duration::from_secs(8);
const PRESENTATION_READY_TIMEOUT: Duration = Duration::from_secs(8);
const DETACH_CLIENT: &[u8] = b"detach-client\n";

pub fn is_tmux_client_command(bytes: &[u8]) -> bool {
    bytes.starts_with(b"split-window")
        || bytes.starts_with(b"select-pane")
        || bytes.starts_with(b"kill-pane")
        || bytes.starts_with(b"resize-pane")
        || bytes.starts_with(b"refresh-client")
        || bytes.starts_with(b"new-window")
        || bytes.starts_with(b"select-window")
        || bytes.starts_with(b"kill-window")
        || bytes.starts_with(b"detach-client")
        || bytes.starts_with(b"pipe-pane")
        || bytes.starts_with(b"capture-pane")
        || bytes.starts_with(b"list-windows")
        || bytes.starts_with(b"list-panes")
        || bytes.starts_with(b"display-message")
        || bytes.starts_with(b"send-keys")
        || bytes.starts_with(b"set -s")
}

fn is_detach_client_command(bytes: &[u8]) -> bool {
    bytes.starts_with(b"detach-client")
}

pub fn is_tmux_cc_start(bytes: &[u8]) -> bool {
    let trimmed = bytes.trim_ascii_start();
    trimmed.starts_with(b"tmux -CC")
}

/// True for Warp-managed `/tmux` (`-L warp-control-v1`), not arbitrary user `tmux -CC`.
pub fn is_managed_isolated_tmux_cc(bytes: &[u8]) -> bool {
    let trimmed = bytes.trim_ascii_start();
    if !is_tmux_cc_start(trimmed) {
        return false;
    }
    let Ok(text) = std::str::from_utf8(trimmed) else {
        return false;
    };
    managed_socket_in_tmux_globals(&tokenize_tmux_cc_args(text.trim_end()))
}

fn tokenize_tmux_cc_args(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    for ch in input.chars() {
        match quote {
            Some(q) if ch == q => quote = None,
            Some(_) => current.push(ch),
            None if ch == '\'' || ch == '"' => quote = Some(ch),
            None if ch.is_whitespace() => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            None => current.push(ch),
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn managed_socket_in_tmux_globals(tokens: &[String]) -> bool {
    let compact = format!("-L{WARP_CONTROL_SOCKET_NAME}");
    let mut i = 0;
    if tokens.first().is_some_and(|token| token == "tmux") {
        i = 1;
    }
    while i < tokens.len() {
        let token = &tokens[i];
        if token == "--" {
            return false;
        }
        if token == &compact {
            return true;
        }
        if token == "-L" {
            return tokens
                .get(i + 1)
                .is_some_and(|value| value == WARP_CONTROL_SOCKET_NAME);
        }
        i += 1;
    }
    false
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TmuxPhaseKind {
    Inactive,
    StartPending,
    InControl,
    OverflowRecovering,
    PresentationRecovering,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CommandIntent {
    Snapshot { generation: u64 },
    Capture { pane_id: PaneId },
    Other,
}

#[allow(clippy::large_enum_variant)]
enum TmuxPhase {
    Inactive,
    StartPending {
        pending_writes: Vec<Cow<'static, [u8]>>,
        pending_control: Vec<Cow<'static, [u8]>>,
        pending_resize: Option<(usize, usize)>,
        started_at: Instant,
        managed_isolated: bool,
    },
    InControl {
        focused: Option<PaneId>,
        known_panes: HashSet<PaneId>,
        pending_writes: Vec<Cow<'static, [u8]>>,
        pending_resize: Option<(usize, usize)>,
        pending_commands: VecDeque<CommandIntent>,
        live_snapshot: Option<u64>,
        next_snapshot_generation: u64,
        pending_bootstrap_window: Option<WindowId>,
        pending_bootstrap_pane: Option<PaneId>,
        bootstrap_blocked: bool,
        layout_ready: bool,
        ready_deadline: Option<Instant>,
    },
    OverflowRecovering {
        pending_writes: Vec<Cow<'static, [u8]>>,
    },
    PresentationRecovering {
        pending_writes: Vec<Cow<'static, [u8]>>,
    },
}

pub struct TmuxIoState {
    parser: ControlModeParser,
    phase: TmuxPhase,
    managed: bool,
}

impl Default for TmuxIoState {
    fn default() -> Self {
        Self::new()
    }
}

impl TmuxIoState {
    pub fn new() -> Self {
        Self {
            parser: ControlModeParser::new(),
            phase: TmuxPhase::Inactive,
            managed: false,
        }
    }

    /// Treat the next control-mode entry as Warp-managed even without `-L warp-control-v1`.
    pub fn with_managed_isolated(mut self) -> Self {
        self.managed = true;
        self
    }

    pub fn phase(&self) -> TmuxPhaseKind {
        match self.phase {
            TmuxPhase::Inactive => TmuxPhaseKind::Inactive,
            TmuxPhase::StartPending { .. } => TmuxPhaseKind::StartPending,
            TmuxPhase::InControl { .. } => TmuxPhaseKind::InControl,
            TmuxPhase::OverflowRecovering { .. } => TmuxPhaseKind::OverflowRecovering,
            TmuxPhase::PresentationRecovering { .. } => TmuxPhaseKind::PresentationRecovering,
        }
    }

    pub fn focused_pane(&self) -> Option<&PaneId> {
        match &self.phase {
            TmuxPhase::InControl { focused, .. } => focused.as_ref(),
            _ => None,
        }
    }

    pub fn in_control(&self) -> bool {
        matches!(self.phase, TmuxPhase::InControl { .. })
    }

    pub fn enqueue_input(&mut self, input: Cow<'static, [u8]>) -> Vec<Cow<'static, [u8]>> {
        match &mut self.phase {
            TmuxPhase::Inactive => {
                let is_start = is_tmux_cc_start(&input);
                if is_start {
                    self.phase = TmuxPhase::StartPending {
                        pending_writes: Vec::new(),
                        pending_control: Vec::new(),
                        pending_resize: None,
                        started_at: Instant::now(),
                        managed_isolated: self.managed || is_managed_isolated_tmux_cc(&input),
                    };
                }
                vec![input]
            }
            TmuxPhase::StartPending { pending_writes, .. }
            | TmuxPhase::OverflowRecovering { pending_writes }
            | TmuxPhase::PresentationRecovering { pending_writes } => {
                if is_detach_client_command(&input) {
                    return Vec::new();
                }
                pending_writes.push(input);
                Vec::new()
            }
            TmuxPhase::InControl { .. } => {
                let focused = match &self.phase {
                    TmuxPhase::InControl { focused, .. } => focused.clone(),
                    _ => None,
                };
                if let Some(pane) = focused {
                    let encoded = send_keys_command(&pane, &input);
                    if encoded.is_empty() {
                        Vec::new()
                    } else {
                        self.note_outgoing_command(&encoded);
                        vec![Cow::Owned(encoded)]
                    }
                } else if let TmuxPhase::InControl { pending_writes, .. } = &mut self.phase {
                    pending_writes.push(input);
                    Vec::new()
                } else {
                    Vec::new()
                }
            }
        }
    }

    pub fn enqueue_control_command(
        &mut self,
        input: Cow<'static, [u8]>,
    ) -> Vec<Cow<'static, [u8]>> {
        let input = command_line(input);
        match &mut self.phase {
            TmuxPhase::StartPending {
                pending_control, ..
            } => {
                pending_control.push(input);
                Vec::new()
            }
            TmuxPhase::InControl { .. } => {
                if is_detach_client_command(&input) {
                    return self.begin_detach_recovery();
                }
                self.note_outgoing_command(&input);
                vec![input]
            }
            TmuxPhase::Inactive
            | TmuxPhase::OverflowRecovering { .. }
            | TmuxPhase::PresentationRecovering { .. } => Vec::new(),
        }
    }

    pub fn enqueue_pane_input(
        &mut self,
        pane_id: &PaneId,
        input: Cow<'static, [u8]>,
    ) -> Vec<Cow<'static, [u8]>> {
        match &mut self.phase {
            TmuxPhase::StartPending { pending_writes, .. }
            | TmuxPhase::OverflowRecovering { pending_writes }
            | TmuxPhase::PresentationRecovering { pending_writes } => {
                pending_writes.push(input);
                Vec::new()
            }
            TmuxPhase::InControl { .. } => self.encode_pane_bytes(pane_id, &input),
            TmuxPhase::Inactive => Vec::new(),
        }
    }

    fn encode_pane_bytes(&mut self, pane_id: &PaneId, input: &[u8]) -> Vec<Cow<'static, [u8]>> {
        let encoded = send_keys_command(pane_id, input);
        if encoded.is_empty() {
            Vec::new()
        } else {
            self.note_outgoing_command(&encoded);
            vec![Cow::Owned(encoded)]
        }
    }

    pub fn enqueue_resize(&mut self, columns: usize, rows: usize) -> Option<Cow<'static, [u8]>> {
        let in_control = matches!(self.phase, TmuxPhase::InControl { .. });
        match &mut self.phase {
            TmuxPhase::Inactive
            | TmuxPhase::OverflowRecovering { .. }
            | TmuxPhase::PresentationRecovering { .. } => return None,
            TmuxPhase::StartPending { pending_resize, .. }
            | TmuxPhase::InControl { pending_resize, .. } => {
                *pending_resize = Some((columns, rows));
            }
        }
        in_control.then(|| {
            let command = Cow::Owned(refresh_client_command(columns, rows).into_bytes());
            self.note_outgoing_command(&command);
            command
        })
    }

    pub fn start_pending_remaining(&self) -> Option<Duration> {
        match &self.phase {
            TmuxPhase::StartPending { started_at, .. } => {
                Some(START_PENDING_TIMEOUT.saturating_sub(started_at.elapsed()))
            }
            _ => None,
        }
    }

    pub fn presentation_ready_remaining(&self) -> Option<Duration> {
        match &self.phase {
            TmuxPhase::InControl {
                layout_ready: false,
                ready_deadline: Some(deadline),
                ..
            } => Some(deadline.saturating_duration_since(Instant::now())),
            _ => None,
        }
    }

    pub fn poll_timeout_remaining(&self) -> Option<Duration> {
        match (
            self.start_pending_remaining(),
            self.presentation_ready_remaining(),
        ) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    pub fn check_start_timeout(&mut self, now: Instant) -> Vec<TmuxFeedItem> {
        let TmuxPhase::StartPending { started_at, .. } = &self.phase else {
            return Vec::new();
        };
        if now.saturating_duration_since(*started_at) < START_PENDING_TIMEOUT {
            return Vec::new();
        }
        self.fail_start_pending()
    }

    pub fn check_timeouts(&mut self, now: Instant) -> Vec<TmuxFeedItem> {
        let start = self.check_start_timeout(now);
        if !start.is_empty() {
            return start;
        }
        self.check_presentation_timeout(now)
    }

    pub fn check_presentation_timeout(&mut self, now: Instant) -> Vec<TmuxFeedItem> {
        if let TmuxPhase::InControl {
            layout_ready: false,
            ready_deadline: Some(deadline),
            ..
        } = &self.phase
            && now >= *deadline
        {
            return self.fail_presentation_ready();
        }
        Vec::new()
    }

    pub fn feed(&mut self, bytes: &[u8]) -> Vec<TmuxFeedItem> {
        let mut items = Vec::new();
        let mut start_failed = false;
        for decoded in self.parser.decode(bytes) {
            if start_failed {
                continue;
            }
            match decoded {
                DecodeItem::Shell(shell) => {
                    let failed = matches!(self.phase, TmuxPhase::StartPending { .. })
                        && looks_like_start_failure(&shell);
                    items.push(TmuxFeedItem::Shell(shell));
                    if failed {
                        items.extend(self.fail_start_pending());
                        start_failed = true;
                    }
                }
                DecodeItem::Control(event) => items.extend(self.apply_control(event)),
            }
        }
        items
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TmuxFeedItem {
    Shell(Vec<u8>),
    EnteredControl {
        refresh_client: Option<String>,
    },
    PaneOutput {
        pane_id: PaneId,
        bytes: Vec<u8>,
    },
    LayoutChange {
        window_id: WindowId,
        layout: String,
        visible_layout: Option<String>,
        flags: Option<String>,
    },
    Focused(PaneId),
    EncodedPending(Vec<u8>),
    WindowAdd {
        window_id: WindowId,
    },
    WindowClose {
        window_id: WindowId,
    },
    WindowRenamed {
        window_id: WindowId,
        name: String,
    },
    SessionWindowChanged {
        window_id: WindowId,
    },
    CommandEnd {
        number: u64,
        error: bool,
        payload: Vec<String>,
        capture_pane: Option<PaneId>,
    },
    Exited {
        replay: Vec<Cow<'static, [u8]>>,
    },
    OverflowRecovering {
        detach: Cow<'static, [u8]>,
    },
    PresentationUnready {
        detach: Cow<'static, [u8]>,
    },
}

impl TmuxIoState {
    fn is_recovering(&self) -> bool {
        matches!(
            self.phase,
            TmuxPhase::OverflowRecovering { .. } | TmuxPhase::PresentationRecovering { .. }
        )
    }

    fn apply_control(&mut self, event: ControlEvent) -> Vec<TmuxFeedItem> {
        if self.is_recovering() {
            return match event {
                ControlEvent::Exit { .. } => self.exit_to_inactive(),
                _ => Vec::new(),
            };
        }
        match event {
            ControlEvent::EnteredControlMode => {
                let (pending_writes, pending_control, pending_resize, from_start, managed_isolated) =
                    match &mut self.phase {
                        TmuxPhase::StartPending {
                            pending_writes,
                            pending_control,
                            pending_resize,
                            managed_isolated,
                            ..
                        } => (
                            std::mem::take(pending_writes),
                            std::mem::take(pending_control),
                            pending_resize.take(),
                            true,
                            *managed_isolated,
                        ),
                        TmuxPhase::Inactive => (Vec::new(), Vec::new(), None, false, self.managed),
                        _ => (Vec::new(), Vec::new(), None, false, false),
                    };
                self.phase = TmuxPhase::InControl {
                    focused: None,
                    known_panes: HashSet::new(),
                    pending_writes,
                    pending_resize,
                    pending_commands: VecDeque::new(),
                    live_snapshot: None,
                    next_snapshot_generation: 0,
                    pending_bootstrap_window: None,
                    pending_bootstrap_pane: None,
                    bootstrap_blocked: false,
                    layout_ready: false,
                    ready_deadline: Some(Instant::now() + PRESENTATION_READY_TIMEOUT),
                };
                if from_start {
                    self.note_outgoing_command(b"tmux -CC");
                }
                let refresh_client =
                    pending_resize.map(|(columns, rows)| refresh_client_command(columns, rows));
                if let Some(command) = refresh_client.as_ref() {
                    self.note_outgoing_command(command.as_bytes());
                }
                let mut items = vec![TmuxFeedItem::EnteredControl { refresh_client }];
                if managed_isolated {
                    self.note_outgoing_command(EXIT_EMPTY_OFF_COMMAND.as_bytes());
                    items.push(TmuxFeedItem::EncodedPending(
                        EXIT_EMPTY_OFF_COMMAND.as_bytes().to_vec(),
                    ));
                }
                self.note_outgoing_command(LIST_WINDOWS_LAYOUT_COMMAND.as_bytes());
                items.push(TmuxFeedItem::EncodedPending(
                    LIST_WINDOWS_LAYOUT_COMMAND.as_bytes().to_vec(),
                ));
                for command in pending_control {
                    self.note_outgoing_command(&command);
                    items.push(TmuxFeedItem::EncodedPending(command.into_owned()));
                }
                items
            }
            ControlEvent::PaneOutput { pane_id, bytes } => {
                self.note_pane(pane_id.clone());
                let mut items = self.bootstrap_layout_from_output(&pane_id);
                items.push(TmuxFeedItem::PaneOutput { pane_id, bytes });
                items.extend(self.flush_pending_if_focused());
                items
            }
            ControlEvent::WindowPaneChanged { pane_id, .. } => {
                self.note_pane(pane_id.clone());
                self.set_focused(pane_id.clone());
                let mut items = vec![TmuxFeedItem::Focused(pane_id)];
                items.extend(self.flush_pending_if_focused());
                items
            }
            ControlEvent::LayoutChange {
                window_id,
                layout,
                visible_layout,
                flags,
            } => {
                self.note_layout(&layout);
                let mut items = vec![TmuxFeedItem::LayoutChange {
                    window_id,
                    layout,
                    visible_layout,
                    flags,
                }];
                items.extend(self.flush_pending_if_focused());
                items
            }
            ControlEvent::WindowAdd { window_id } => {
                let mut items = vec![TmuxFeedItem::WindowAdd {
                    window_id: window_id.clone(),
                }];
                items.extend(self.bootstrap_layout_from_window(window_id));
                items
            }
            ControlEvent::WindowClose { window_id } => {
                vec![TmuxFeedItem::WindowClose { window_id }]
            }
            ControlEvent::WindowRenamed { window_id, name } => {
                vec![TmuxFeedItem::WindowRenamed { window_id, name }]
            }
            ControlEvent::SessionWindowChanged { window_id } => {
                let mut items = vec![TmuxFeedItem::SessionWindowChanged {
                    window_id: window_id.clone(),
                }];
                items.extend(self.bootstrap_layout_from_window(window_id));
                items
            }
            ControlEvent::CommandEnd {
                number,
                error,
                payload,
                ..
            } => self.apply_command_end(number, error, payload),
            ControlEvent::CommandBegin { .. } => Vec::new(),
            ControlEvent::ProtocolOverflow => {
                let pending_writes = self.take_pending_writes();
                self.phase = TmuxPhase::OverflowRecovering { pending_writes };
                vec![TmuxFeedItem::OverflowRecovering {
                    detach: Cow::Borrowed(DETACH_CLIENT),
                }]
            }
            ControlEvent::Exit { .. } => self.exit_to_inactive(),
        }
    }

    fn note_pane(&mut self, pane_id: PaneId) {
        if let TmuxPhase::InControl { known_panes, .. } = &mut self.phase {
            known_panes.insert(pane_id);
        }
    }

    fn set_focused(&mut self, pane_id: PaneId) {
        if let TmuxPhase::InControl {
            focused,
            known_panes,
            ..
        } = &mut self.phase
        {
            known_panes.insert(pane_id.clone());
            *focused = Some(pane_id);
        }
    }

    fn flush_pending_if_focused(&mut self) -> Vec<TmuxFeedItem> {
        let (pane, pending) = match &mut self.phase {
            TmuxPhase::InControl {
                focused,
                pending_writes,
                ..
            } => {
                let Some(pane) = focused.clone() else {
                    return Vec::new();
                };
                (pane, std::mem::take(pending_writes))
            }
            _ => return Vec::new(),
        };
        pending
            .into_iter()
            .filter_map(|input| {
                let encoded = send_keys_command(&pane, &input);
                if encoded.is_empty() {
                    None
                } else {
                    self.note_outgoing_command(&encoded);
                    Some(TmuxFeedItem::EncodedPending(encoded))
                }
            })
            .collect()
    }

    fn take_pending_writes(&mut self) -> Vec<Cow<'static, [u8]>> {
        match &mut self.phase {
            TmuxPhase::StartPending { pending_writes, .. }
            | TmuxPhase::InControl { pending_writes, .. }
            | TmuxPhase::OverflowRecovering { pending_writes }
            | TmuxPhase::PresentationRecovering { pending_writes } => {
                std::mem::take(pending_writes)
            }
            TmuxPhase::Inactive => Vec::new(),
        }
    }

    fn exit_to_inactive(&mut self) -> Vec<TmuxFeedItem> {
        let replay = self.take_pending_writes();
        self.phase = TmuxPhase::Inactive;
        vec![TmuxFeedItem::Exited { replay }]
    }

    fn fail_start_pending(&mut self) -> Vec<TmuxFeedItem> {
        let replay = self.take_pending_writes();
        self.phase = TmuxPhase::Inactive;
        self.parser = ControlModeParser::new();
        vec![TmuxFeedItem::Exited { replay }]
    }

    fn note_outgoing_command(&mut self, bytes: &[u8]) {
        let TmuxPhase::InControl {
            pending_commands,
            live_snapshot,
            next_snapshot_generation,
            ..
        } = &mut self.phase
        else {
            return;
        };
        if let Some(pane_id) = capture_pane_target(bytes) {
            pending_commands.push_back(CommandIntent::Capture { pane_id });
            return;
        }
        if is_snapshot_command(bytes) {
            *next_snapshot_generation += 1;
            let generation = *next_snapshot_generation;
            *live_snapshot = Some(generation);
            pending_commands.push_back(CommandIntent::Snapshot { generation });
            return;
        }
        pending_commands.push_back(CommandIntent::Other);
    }

    fn apply_command_end(
        &mut self,
        number: u64,
        error: bool,
        payload: Vec<String>,
    ) -> Vec<TmuxFeedItem> {
        if payload.len() == 1
            && let Some(pane_id) = payload.first().and_then(|line| parse_pane_id_line(line))
        {
            self.note_pane(pane_id);
        }
        match self.pop_command_intent() {
            Some(CommandIntent::Snapshot { generation }) => {
                let is_live = self.live_snapshot() == Some(generation);
                self.clear_live_snapshot_if(generation);
                if !error && is_live {
                    let has_layout = payload
                        .iter()
                        .any(|line| parse_window_layout_line(line).is_some());
                    if has_layout {
                        return self.apply_snapshot_payload(payload);
                    }
                }
                vec![TmuxFeedItem::CommandEnd {
                    number,
                    error,
                    payload,
                    capture_pane: None,
                }]
            }
            Some(CommandIntent::Capture { pane_id }) => vec![TmuxFeedItem::CommandEnd {
                number,
                error,
                payload,
                capture_pane: Some(pane_id),
            }],
            Some(CommandIntent::Other) | None => vec![TmuxFeedItem::CommandEnd {
                number,
                error,
                payload,
                capture_pane: None,
            }],
        }
    }

    fn apply_snapshot_payload(&mut self, payload: Vec<String>) -> Vec<TmuxFeedItem> {
        let mut items = Vec::new();
        for line in payload {
            let Some((window_id, layout)) = parse_window_layout_line(&line) else {
                continue;
            };
            self.note_layout(&layout);
            items.push(TmuxFeedItem::WindowAdd {
                window_id: window_id.clone(),
            });
            items.push(TmuxFeedItem::LayoutChange {
                window_id,
                layout,
                visible_layout: None,
                flags: None,
            });
        }
        items.extend(self.flush_pending_if_focused());
        items
    }

    fn bootstrap_layout_from_output(&mut self, pane_id: &PaneId) -> Vec<TmuxFeedItem> {
        if let TmuxPhase::InControl {
            layout_ready: false,
            bootstrap_blocked: false,
            pending_bootstrap_pane,
            ..
        } = &mut self.phase
            && pending_bootstrap_pane.is_none()
            && is_canonical_bootstrap_pane(pane_id)
        {
            *pending_bootstrap_pane = Some(pane_id.clone());
        }
        self.maybe_synthesize_bootstrap_layout()
    }

    fn bootstrap_layout_from_window(&mut self, window_id: WindowId) -> Vec<TmuxFeedItem> {
        if let TmuxPhase::InControl {
            layout_ready: false,
            bootstrap_blocked,
            pending_bootstrap_window,
            pending_bootstrap_pane,
            ..
        } = &mut self.phase
        {
            if !is_canonical_bootstrap_window(&window_id) {
                *bootstrap_blocked = true;
                *pending_bootstrap_window = None;
                *pending_bootstrap_pane = None;
            } else if !*bootstrap_blocked && pending_bootstrap_window.is_none() {
                *pending_bootstrap_window = Some(window_id);
            }
        }
        self.maybe_synthesize_bootstrap_layout()
    }

    fn maybe_synthesize_bootstrap_layout(&mut self) -> Vec<TmuxFeedItem> {
        let (window_id, pane_id) = match &self.phase {
            TmuxPhase::InControl {
                layout_ready: false,
                bootstrap_blocked: false,
                pending_bootstrap_window: Some(window_id),
                pending_bootstrap_pane: Some(pane_id),
                ..
            } if is_canonical_bootstrap_window(window_id)
                && is_canonical_bootstrap_pane(pane_id) =>
            {
                (window_id.clone(), pane_id.clone())
            }
            _ => return Vec::new(),
        };
        let layout = dummy_layout_for_pane(&pane_id);
        self.note_layout(&layout);
        let mut items = vec![TmuxFeedItem::LayoutChange {
            window_id,
            layout,
            visible_layout: None,
            flags: None,
        }];
        items.extend(self.flush_pending_if_focused());
        items
    }

    fn note_layout(&mut self, layout: &str) {
        if let Some(parsed) = super::layout::parse_window_layout(layout) {
            let ids = parsed.pane_ids();
            for id in &ids {
                self.note_pane(id.clone());
            }
            if ids.len() == 1
                && let Some(id) = ids.into_iter().next()
            {
                self.set_focused(id);
            }
            self.mark_layout_ready();
        }
    }

    fn mark_layout_ready(&mut self) {
        if let TmuxPhase::InControl {
            layout_ready,
            ready_deadline,
            pending_bootstrap_window,
            pending_bootstrap_pane,
            bootstrap_blocked,
            ..
        } = &mut self.phase
        {
            *layout_ready = true;
            *ready_deadline = None;
            *pending_bootstrap_window = None;
            *pending_bootstrap_pane = None;
            *bootstrap_blocked = false;
        }
    }

    fn fail_presentation_ready(&mut self) -> Vec<TmuxFeedItem> {
        let pending_writes = self.take_pending_writes();
        self.phase = TmuxPhase::PresentationRecovering { pending_writes };
        vec![TmuxFeedItem::PresentationUnready {
            detach: Cow::Borrowed(DETACH_CLIENT),
        }]
    }

    fn begin_detach_recovery(&mut self) -> Vec<Cow<'static, [u8]>> {
        if self.is_recovering() {
            return Vec::new();
        }
        let pending_writes = self.take_pending_writes();
        self.phase = TmuxPhase::PresentationRecovering { pending_writes };
        vec![Cow::Borrowed(DETACH_CLIENT)]
    }

    fn live_snapshot(&self) -> Option<u64> {
        match &self.phase {
            TmuxPhase::InControl { live_snapshot, .. } => *live_snapshot,
            _ => None,
        }
    }

    fn clear_live_snapshot_if(&mut self, generation: u64) {
        if let TmuxPhase::InControl { live_snapshot, .. } = &mut self.phase
            && *live_snapshot == Some(generation)
        {
            *live_snapshot = None;
        }
    }

    fn pop_command_intent(&mut self) -> Option<CommandIntent> {
        match &mut self.phase {
            TmuxPhase::InControl {
                pending_commands, ..
            } => pending_commands.pop_front(),
            _ => None,
        }
    }
}

fn capture_pane_target(bytes: &[u8]) -> Option<PaneId> {
    let text = std::str::from_utf8(bytes).ok()?;
    let mut tokens = text.split_whitespace();
    if tokens.next()? != "capture-pane" {
        return None;
    }
    while let Some(token) = tokens.next() {
        if token == "-t" {
            return parse_pane_id_line(tokens.next()?);
        }
        if let Some(attached) = token.strip_prefix("-t")
            && !attached.is_empty()
        {
            return parse_pane_id_line(attached);
        }
    }
    None
}

fn looks_like_start_failure(shell: &[u8]) -> bool {
    let text = String::from_utf8_lossy(shell);
    let lower = text.to_ascii_lowercase();
    lower.contains("command not found")
        || lower.contains("tmux: unknown option")
        || lower.contains("tmux: invalid option")
        || lower.contains("not installed")
        || lower.contains("error connecting to")
        || lower.contains("no server running")
        || lower.contains("no such file or directory")
}

fn parse_pane_id_line(line: &str) -> Option<PaneId> {
    let line = line.trim();
    let digits = line.strip_prefix('%')?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(PaneId::from(line))
}

fn is_snapshot_command(bytes: &[u8]) -> bool {
    bytes.starts_with(b"list-windows")
}

fn parse_window_layout_line(line: &str) -> Option<(WindowId, String)> {
    let line = line.trim();
    let (id, layout) = line.split_once(' ')?;
    let digits = id.strip_prefix('@')?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) || layout.is_empty() {
        return None;
    }
    Some((WindowId::from(id), layout.to_owned()))
}

fn dummy_layout_for_pane(pane_id: &PaneId) -> String {
    let index = pane_id.as_str().trim_start_matches('%');
    format!("80x24,0,0,{index}")
}

fn is_canonical_bootstrap_window(window_id: &WindowId) -> bool {
    window_id.as_str() == "@0"
}

fn is_canonical_bootstrap_pane(pane_id: &PaneId) -> bool {
    pane_id.as_str() == "%0"
}

fn command_line(input: Cow<'static, [u8]>) -> Cow<'static, [u8]> {
    if input.ends_with(b"\n") {
        input
    } else {
        let mut bytes = input.into_owned();
        bytes.push(b'\n');
        Cow::Owned(bytes)
    }
}

#[cfg(test)]
#[path = "io_tests.rs"]
mod tests;
