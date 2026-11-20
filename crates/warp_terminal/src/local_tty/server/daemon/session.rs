//! A daemon-owned PTY session and the byte pumping between its leader fd and
//! an attached client's data socket.

use std::collections::VecDeque;
use std::os::fd::{AsRawFd, OwnedFd};
use std::process::Child;

use instant::Instant;
use mio::Interest;
use mio::unix::SourceFd;
use nix::errno::Errno;
use nix::sys::socket;

use super::super::api::{PersistentSessionId, SessionMetadata, SessionSummary};
use super::super::protocol::send_flags;
use super::ring::History;
use crate::event::ObservedExitStatus;

/// Above this many queued bytes for a client, the daemon stops reading the PTY.
pub(super) const OUTBOUND_HIGH_WATER: usize = 4 * 1024 * 1024;
/// Reading resumes once the client queue drains below this many bytes.
pub(super) const OUTBOUND_LOW_WATER: usize = 1024 * 1024;
/// The maximum number of bytes buffered from a client before they are written
/// to the PTY.
const INBOUND_LIMIT: usize = 4 * 1024 * 1024;
const READ_CHUNK: usize = 64 * 1024;
/// The maximum number of PTY bytes read in a single pump, for fairness.
const READ_BUDGET: usize = 1024 * 1024;

/// The client side of a session: the daemon's end of the data socketpair and
/// the queues in each direction.
pub(super) struct Attachment {
    pub data: OwnedFd,
    pub token: mio::Token,
    outbound: VecDeque<u8>,
    inbound: VecDeque<u8>,
    /// Set once the session has exited: after the queue is flushed the data
    /// socket is shut down and the session is dropped. The flag says whether a
    /// `SessionExited` event should be emitted.
    pub close_after_flush: Option<bool>,
}

impl Attachment {
    pub fn new(data: OwnedFd, token: mio::Token, replay: Vec<u8>) -> Self {
        Self {
            data,
            token,
            outbound: VecDeque::from(replay),
            inbound: VecDeque::new(),
            close_after_flush: None,
        }
    }

    /// Writes as much of the outbound queue as the socket accepts. Returns
    /// `Err` if the peer is gone.
    fn flush(&mut self) -> Result<(), Errno> {
        while !self.outbound.is_empty() {
            let (front, _) = self.outbound.as_slices();
            match socket::send(self.data.as_raw_fd(), front, send_flags()) {
                Ok(n) => {
                    self.outbound.drain(..n);
                }
                Err(Errno::EINTR) => continue,
                Err(Errno::EAGAIN) => return Ok(()),
                Err(err) => return Err(err),
            }
        }
        Ok(())
    }
}

/// What the daemon has to do after pumping a session.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum PumpOutcome {
    Idle,
    /// The read budget was exhausted; pump again without waiting for events.
    MoreWork,
    /// The client closed its data socket.
    DataClosed,
    /// An exited session finished flushing and should be dropped.
    Finished {
        emit_event: bool,
    },
}

pub(super) struct Session {
    pub id: PersistentSessionId,
    pub pid: u32,
    pub metadata: SessionMetadata,
    pub child: Option<Child>,
    pub leader: Option<OwnedFd>,
    pub leader_token: mio::Token,
    leader_registered: bool,
    pub history: History,
    pub exit_status: Option<ObservedExitStatus>,
    pub exited_at: Option<Instant>,
    pub attachment: Option<Attachment>,
    paused: bool,
}

impl Session {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: PersistentSessionId,
        metadata: SessionMetadata,
        child: Child,
        leader: OwnedFd,
        leader_token: mio::Token,
        ring_capacity: usize,
        registry: &mio::Registry,
    ) -> std::io::Result<Self> {
        registry.register(
            &mut SourceFd(&leader.as_raw_fd()),
            leader_token,
            Interest::READABLE | Interest::WRITABLE,
        )?;
        Ok(Self {
            id,
            pid: child.id(),
            history: History::new(ring_capacity, metadata.hook_session_id),
            metadata,
            child: Some(child),
            leader: Some(leader),
            leader_token,
            leader_registered: true,
            exit_status: None,
            exited_at: None,
            attachment: None,
            paused: false,
        })
    }

    #[cfg(test)]
    pub fn is_paused(&self) -> bool {
        self.paused
    }

    pub fn summary(&self) -> SessionSummary {
        SessionSummary {
            session_id: self.id,
            pid: self.pid,
            metadata: self.metadata.clone(),
            exit_status: self.exit_status,
            attached: self.attachment.is_some(),
        }
    }

    /// Stops watching and closes the PTY leader.
    pub fn close_leader(&mut self, registry: &mio::Registry) {
        self.unregister_leader(registry);
        self.leader = None;
    }

    fn unregister_leader(&mut self, registry: &mio::Registry) {
        if self.leader_registered
            && let Some(leader) = &self.leader
        {
            let _ = registry.deregister(&mut SourceFd(&leader.as_raw_fd()));
        }
        self.leader_registered = false;
    }

    /// Detaches the client, if any, and returns its attachment so the caller
    /// can unregister it.
    pub fn detach(&mut self) -> Option<Attachment> {
        self.history.mark_detached();
        self.paused = false;
        let attachment = self.attachment.take();
        if self.exit_status.is_some() && self.exited_at.is_none() {
            self.exited_at = Some(Instant::now());
        }
        attachment
    }

    fn on_output(&mut self, bytes: &[u8]) {
        self.history.record(bytes);
        if let Some(attachment) = &mut self.attachment {
            attachment.outbound.extend(bytes);
        }
    }

    fn outbound_len(&self) -> usize {
        self.attachment
            .as_ref()
            .map_or(0, |attachment| attachment.outbound.len())
    }

    /// Reads the leader until it would block, hits EOF, or `budget` bytes have
    /// been read. Returns whether the budget ran out.
    fn read_leader(&mut self, budget: Option<usize>, registry: &mio::Registry) -> bool {
        let Some(leader) = self.leader.as_ref().map(|fd| fd.as_raw_fd()) else {
            return false;
        };
        if !self.leader_registered {
            return false;
        }
        let mut chunk = vec![0u8; READ_CHUNK];
        let mut total = 0;
        loop {
            match nix::unistd::read(leader, &mut chunk) {
                Ok(0) | Err(Errno::EIO) => {
                    self.unregister_leader(registry);
                    return false;
                }
                Ok(n) => {
                    self.on_output(&chunk[..n]);
                    total += n;
                    if budget.is_some() && self.outbound_len() >= OUTBOUND_HIGH_WATER {
                        self.paused = true;
                        return false;
                    }
                    if budget.is_some_and(|budget| total >= budget) {
                        return true;
                    }
                }
                Err(Errno::EINTR) => continue,
                Err(Errno::EAGAIN) => return false,
                Err(err) => {
                    log::warn!("Failed to read from pty leader: {err}");
                    self.unregister_leader(registry);
                    return false;
                }
            }
        }
    }

    /// Reads all remaining output after the child exited, ignoring
    /// backpressure.
    pub fn drain_leader(&mut self, registry: &mio::Registry) {
        self.read_leader(None, registry);
    }

    /// Moves bytes between the PTY leader and the attached client.
    pub fn pump(&mut self, registry: &mio::Registry) -> PumpOutcome {
        if let Some(attachment) = &mut self.attachment {
            if attachment.flush().is_err() {
                return PumpOutcome::DataClosed;
            }
            if let Some(emit_event) = attachment.close_after_flush
                && attachment.outbound.is_empty()
            {
                let _ = socket::shutdown(attachment.data.as_raw_fd(), socket::Shutdown::Write);
                return PumpOutcome::Finished { emit_event };
            }
        }

        if self.paused && self.outbound_len() < OUTBOUND_LOW_WATER {
            self.paused = false;
        }
        let mut more_work = false;
        if !self.paused {
            more_work = self.read_leader(Some(READ_BUDGET), registry);
            if let Some(attachment) = &mut self.attachment
                && attachment.flush().is_err()
            {
                return PumpOutcome::DataClosed;
            }
        }

        if let Some(attachment) = &mut self.attachment {
            let mut chunk = vec![0u8; READ_CHUNK];
            let mut hit_limit = false;
            loop {
                if attachment.inbound.len() >= INBOUND_LIMIT {
                    hit_limit = true;
                    break;
                }
                match nix::unistd::read(attachment.data.as_raw_fd(), &mut chunk) {
                    Ok(0) => return PumpOutcome::DataClosed,
                    Ok(n) => attachment.inbound.extend(&chunk[..n]),
                    Err(Errno::EINTR) => continue,
                    Err(Errno::EAGAIN) => break,
                    Err(_) => return PumpOutcome::DataClosed,
                }
            }

            match &self.leader {
                None => attachment.inbound.clear(),
                Some(leader) => {
                    while !attachment.inbound.is_empty() {
                        let (front, _) = attachment.inbound.as_slices();
                        match nix::unistd::write(leader.as_raw_fd(), front) {
                            Ok(n) => {
                                attachment.inbound.drain(..n);
                            }
                            Err(Errno::EINTR) => continue,
                            Err(Errno::EAGAIN) => break,
                            Err(err) => {
                                log::warn!("Failed to write to pty leader: {err}");
                                attachment.inbound.clear();
                            }
                        }
                    }
                }
            }
            if hit_limit && attachment.inbound.len() < INBOUND_LIMIT {
                more_work = true;
            }
            if attachment.close_after_flush.is_some() && attachment.outbound.is_empty() {
                more_work = true;
            }
        }

        if more_work {
            PumpOutcome::MoreWork
        } else {
            PumpOutcome::Idle
        }
    }
}

/// Converts a child's exit status into the status reported to clients.
pub(super) fn observed_exit_status(status: std::process::ExitStatus) -> ObservedExitStatus {
    use std::os::unix::process::ExitStatusExt;
    status
        .code()
        .map(ObservedExitStatus::Code)
        .or_else(|| status.signal().map(ObservedExitStatus::Signal))
        .unwrap_or(ObservedExitStatus::Unavailable)
}
