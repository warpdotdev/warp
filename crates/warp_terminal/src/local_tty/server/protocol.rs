use std::collections::VecDeque;
use std::io::{IoSlice, IoSliceMut};
use std::os::unix::prelude::*;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail, ensure};
use itertools::Itertools;
use nix::cmsg_space;
use nix::errno::Errno;
use nix::sys::socket;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use super::api;
use crate::local_tty::PtySpawnResult;

/// The size of a usize, in bytes.
const USIZE_SIZE: usize = std::mem::size_of::<usize>();

/// A structure representing a non-blocking unix socket file descriptor.
#[derive(Copy, Clone)]
pub struct NonblockingSocketFd(RawFd);

impl NonblockingSocketFd {
    pub fn new(fd: RawFd) -> Result<Self> {
        use nix::fcntl;

        let mut flags = fcntl::OFlag::from_bits(
            fcntl::fcntl(fd, fcntl::F_GETFL)
                .context("should be able to read flags from unix socket")?,
        )
        .ok_or_else(|| anyhow!("received invalid flags from fcntl F_GETFL"))?;
        flags.insert(fcntl::OFlag::O_NONBLOCK);
        fcntl::fcntl(fd, fcntl::F_SETFL(flags))
            .context("should be able to set O_NONBLOCK on unix socket")?;

        Ok(Self(fd))
    }
}

impl AsRawFd for NonblockingSocketFd {
    fn as_raw_fd(&self) -> RawFd {
        self.0
    }
}

/// The type of the additional data that we might send as a control message
/// as part of a [`sendmsg`](socket::sendmsg) call.
type AuxData = [RawFd; 1];

/// The data structure that we serialize and send across the socket.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Message {
    /// A sentinel field which we always set to `true` so that we can confirm,
    /// upon receipt, that the message was sent correctly.  If we think we
    /// parsed a message but this field is `false`, we probably just parsed a
    /// zero-initialized buffer.
    initialized: bool,

    /// The actual message data that we wanted to send.
    data: api::Message,
}

impl Message {
    fn new(data: api::Message) -> Self {
        Self {
            initialized: true,
            data,
        }
    }
}

/// Sends a message across the provided Unix domain socket.
///
/// A file descriptor can optionally be provided to send across the socket.  The
/// provided file descriptor is copied across the socket, and is not guaranteed
/// to have the same index on the receiving end.
pub(super) fn send_message(
    socket_fd: impl AsRawFd,
    data: api::Message,
    aux_fd: Option<RawFd>,
) -> Result<()> {
    let msg = Message::new(data);
    let mut serialized_msg = bincode::serialize(&msg)?;

    // Create a buffer to hold the data we want to send over the socket.
    let mut buf = Vec::new();
    // First, add a message header - a usize representing the length of the
    // serialized message, in bytes.
    buf.extend_from_slice(&serialized_msg.len().to_be_bytes());
    // Next, add the serialized message itself.
    buf.append(&mut serialized_msg);
    let iov = IoSlice::new(&buf);

    // If we're sending a file descriptor alongside the message, build the
    // control message that we will send it within.
    let mut fds = vec![];
    let mut control_msgs = vec![];
    if let Some(aux_fd) = aux_fd {
        fds.push(aux_fd);
        control_msgs.push(socket::ControlMessage::ScmRights(fds.as_slice()));
    }

    // Push the message data and any control messages across the socket.
    let _len = socket::sendmsg::<()>(
        socket_fd.as_raw_fd(),
        &[iov],
        control_msgs.as_slice(),
        socket::MsgFlags::empty(),
        None,
    )?;

    Ok(())
}

#[allow(clippy::large_enum_variant)]
pub(super) enum TryReceiveMessageResult {
    Success(api::Message),
    WouldBlock,
    SocketClosed,
}

pub(super) fn receive_message(socket_fd: impl AsRawFd) -> Result<Option<api::Message>> {
    try_receive_message_internal(socket_fd).map(|result| match result {
        TryReceiveMessageResult::Success(message) => Some(message),
        TryReceiveMessageResult::WouldBlock => {
            panic!("should never get EWOULDBLOCK on a blocking socket")
        }
        TryReceiveMessageResult::SocketClosed => None,
    })
}

/// Receives a message sent across the provided non-blocking Unix domain socket.
pub(super) fn try_receive_message(
    socket_fd: NonblockingSocketFd,
) -> Result<TryReceiveMessageResult> {
    try_receive_message_internal(socket_fd)
}

/// Receives a message sent across the provided Unix domain socket.  This
/// supports both blocking and non-blocking sockets.
fn try_receive_message_internal(socket_fd: impl AsRawFd) -> Result<TryReceiveMessageResult> {
    let (payload_size, cmsgs) = match receive_message_header(&socket_fd)? {
        ReceiveMessageHeaderResult::WouldBlock => return Ok(TryReceiveMessageResult::WouldBlock),
        ReceiveMessageHeaderResult::SocketClosed => {
            return Ok(TryReceiveMessageResult::SocketClosed);
        }
        ReceiveMessageHeaderResult::Success {
            payload_size,
            cmsgs,
        } => (payload_size, cmsgs),
    };
    ensure!(payload_size > 0, "Message payload should have a size >0!");

    // Allocate a buffer that is large enough to read the message payload.
    let mut buf = vec![0; payload_size];
    let mut buffers = [IoSliceMut::new(&mut buf)];

    // Grow the initial buffer to a sufficient size and read the rest of the
    // message from the socket.
    let msg: socket::RecvMsg<()> = socket::recvmsg(
        socket_fd.as_raw_fd(),
        &mut buffers,
        None,
        socket::MsgFlags::empty(),
    )
    .expect("should not fail to receive");
    ensure!(
        msg.bytes == payload_size,
        "Received unexpected amount of data in second recvmsg call!"
    );

    // Deserialize the message, and verify that the sentinel bit (`initialized`)
    // was set.
    let mut message: Message = bincode::deserialize(&buf[..])
        .context("Failed to deserialize received data as a Message")?;
    ensure!(message.initialized, "Received uninitialized message!");

    // Extract extra information from Unix domain socket control messages for
    // api::Message variants which require it.
    if let api::Message::SpawnShellResponse {
        spawn_result: api::Result::Ok(PtySpawnResult { leader_fd, .. }),
        ..
    } = &mut message.data
    {
        match &cmsgs[..] {
            [socket::ControlMessageOwned::ScmRights(fds)] => {
                let Some(received_fd) = fds.first() else {
                    bail!(
                        "Received {} fds alongside a SpawnShellResponse message; expected 1",
                        fds.len()
                    );
                };
                // Set the `leader_fd` field in the `TtySpawnResult` to the
                // value we received in the control message.
                *leader_fd = *received_fd;
            }
            _ => {
                bail!(
                    "Received {} cmsgs alongside a SpawnShellResponse message; expected 1",
                    cmsgs.len()
                );
            }
        };
    }

    Ok(TryReceiveMessageResult::Success(message.data))
}

fn receive_message_header(socket_fd: &impl AsRawFd) -> Result<ReceiveMessageHeaderResult> {
    // Allocate a buffer that is only large enough to receive the message
    // header, to ensure we don't accidentally receive multiple messages
    // in a single read.
    let mut buf = vec![0; USIZE_SIZE];
    let mut buffers = [IoSliceMut::new(&mut buf)];

    // Allocate enough space for any control message that might contain
    // auxiliary data (in our case, specifically, a single file descriptor).
    let mut cmsg_buffer = cmsg_space!(AuxData);

    // Read the message header from the socket.
    let msg = socket::recvmsg(
        socket_fd.as_raw_fd(),
        &mut buffers,
        Some(&mut cmsg_buffer),
        socket::MsgFlags::empty(),
    );

    // Check if we successfully received a message, if there was no data
    // available to read, or if there was some actual error.
    let msg: socket::RecvMsg<()> = match msg {
        Ok(msg) => msg,
        Err(err) => match err {
            Errno::EAGAIN => return Ok(ReceiveMessageHeaderResult::WouldBlock),
            #[allow(unreachable_patterns)]
            Errno::EWOULDBLOCK => return Ok(ReceiveMessageHeaderResult::WouldBlock),
            _ => anyhow::bail!("Failed to read data from socket: {err:?}"),
        },
    };

    if msg.bytes == 0 {
        // The other side of the connection has been closed, so we should
        // quit.
        log::info!("Received empty message; assuming the connection has been closed.");
        return Ok(ReceiveMessageHeaderResult::SocketClosed);
    }

    ensure!(
        msg.bytes == USIZE_SIZE,
        "Message too small for message size header!"
    );

    // Extract any control messages from the initial message.  If control
    // messages are sent by the client alongside a payload which is larger than
    // our receive buffer, the control messages will be read out in the initial
    // readmsg() call, and the follow-up readmsg calls will only return payload
    // data.
    let cmsgs = msg.cmsgs().collect_vec();

    // Parse the message header - we convert the bytes back into a usize, which
    // tells us the size of the serialized message, in bytes.  We add the size
    // of a usize to get the total number of bytes we expect to read off the
    // wire.
    let payload_size = {
        let mut bytes: [u8; USIZE_SIZE] = [0; USIZE_SIZE];
        bytes.copy_from_slice(&buf[0..USIZE_SIZE]);
        usize::from_be_bytes(bytes)
    };

    Ok(ReceiveMessageHeaderResult::Success {
        payload_size,
        cmsgs,
    })
}

enum ReceiveMessageHeaderResult {
    WouldBlock,
    SocketClosed,
    Success {
        payload_size: usize,
        cmsgs: Vec<socket::ControlMessageOwned>,
    },
}

/// The maximum number of file descriptors that can accompany a single frame.
pub(super) const MAX_FRAME_FDS: usize = 2;

/// The maximum size of a single frame payload, in bytes.
pub(super) const MAX_FRAME_SIZE: usize = 64 * 1024 * 1024;

/// The maximum size of a frame sent by a client to the daemon. Large enough
/// for a `Spawn` request carrying the client's full environment.
pub(super) const MAX_CLIENT_FRAME_SIZE: usize = 4 * 1024 * 1024;

/// The maximum number of bytes [`FrameReader::fill`] reads in one call.
pub(super) const FILL_BUDGET: usize = 256 * 1024;

/// How long a sender waits for a full socket buffer to drain before giving up.
const SEND_TIMEOUT: Duration = Duration::from_secs(5);

/// The control message buffer type used by generic frames.
type FrameAuxData = [RawFd; MAX_FRAME_FDS];

/// Sends `value` as a single frame: a big-endian `usize` length header followed
/// by the bincode-serialized payload, with up to [`MAX_FRAME_FDS`] file
/// descriptors attached to the first byte via `SCM_RIGHTS`.
///
/// Partial writes are retried until the full frame is sent. Non-blocking
/// sockets are supported by waiting for writability.
pub(super) fn send_frame<T: Serialize>(
    socket_fd: impl AsRawFd,
    value: &T,
    fds: &[RawFd],
) -> Result<()> {
    ensure!(
        fds.len() <= MAX_FRAME_FDS,
        "Cannot send {} fds in a single frame",
        fds.len()
    );
    let buf = encode_frame(value)?;
    let fd = socket_fd.as_raw_fd();
    let rights = [socket::ControlMessage::ScmRights(fds)];
    let mut written = 0;
    while written < buf.len() {
        let control_msgs: &[socket::ControlMessage] = if written == 0 && !fds.is_empty() {
            &rights
        } else {
            &[]
        };
        match socket::sendmsg::<()>(
            fd,
            &[IoSlice::new(&buf[written..])],
            control_msgs,
            send_flags(),
            None,
        ) {
            Ok(n) => written += n,
            Err(Errno::EINTR) => continue,
            Err(Errno::EAGAIN) => wait_for(fd, libc::POLLOUT, SEND_TIMEOUT)?,
            Err(err) => return Err(anyhow::Error::new(err).context("Failed to send frame")),
        }
    }
    Ok(())
}

/// Serializes `value` into a frame: a big-endian `usize` length header
/// followed by the bincode payload.
pub(super) fn encode_frame<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    let payload = bincode::serialize(value).context("Failed to serialize frame")?;
    ensure!(
        !payload.is_empty() && payload.len() <= MAX_FRAME_SIZE,
        "Frame payload has invalid size {}",
        payload.len()
    );
    let mut buf = Vec::with_capacity(USIZE_SIZE + payload.len());
    buf.extend_from_slice(&payload.len().to_be_bytes());
    buf.extend_from_slice(&payload);
    Ok(buf)
}

/// Returns whether `err` means the peer went away: `EPIPE`, `ECONNRESET`,
/// `ENOTCONN`, or a connection closed in the middle of a frame.
pub(super) fn is_disconnect(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| {
        cause.downcast_ref::<Errno>().is_some_and(|errno| {
            matches!(errno, Errno::EPIPE | Errno::ECONNRESET | Errno::ENOTCONN)
        }) || cause.downcast_ref::<std::io::Error>().is_some_and(|io| {
            matches!(
                io.kind(),
                std::io::ErrorKind::BrokenPipe
                    | std::io::ErrorKind::ConnectionReset
                    | std::io::ErrorKind::NotConnected
            )
        })
    })
}

/// Receives a single frame from a socket, blocking until the full frame has
/// arrived. Returns `None` if the peer closed the connection at a frame
/// boundary.
pub(super) fn receive_frame<T: DeserializeOwned>(
    socket_fd: impl AsRawFd,
) -> Result<Option<(T, Vec<OwnedFd>)>> {
    let fd = socket_fd.as_raw_fd();
    let mut fds = Vec::new();

    let mut header = [0u8; USIZE_SIZE];
    if !receive_exact(fd, &mut header, &mut fds)? {
        return Ok(None);
    }
    let payload_size = usize::from_be_bytes(header);
    ensure!(
        payload_size > 0 && payload_size <= MAX_FRAME_SIZE,
        "Received frame with invalid size {payload_size}"
    );

    let mut payload = vec![0; payload_size];
    if !receive_exact(fd, &mut payload, &mut fds)? {
        return Err(anyhow::Error::new(Errno::ECONNRESET)
            .context("Connection closed in the middle of a frame"));
    }
    let value = bincode::deserialize(&payload).context("Failed to deserialize frame")?;
    Ok(Some((value, fds)))
}

/// Fills `buf` completely, collecting any received fds. Returns `false` if the
/// connection was closed before any byte was read.
fn receive_exact(fd: RawFd, buf: &mut [u8], fds: &mut Vec<OwnedFd>) -> Result<bool> {
    let mut filled = 0;
    while filled < buf.len() {
        match receive_some(fd, &mut buf[filled..], fds) {
            Ok(0) if filled == 0 => return Ok(false),
            Ok(0) => {
                return Err(anyhow::Error::new(Errno::ECONNRESET)
                    .context("Connection closed in the middle of a frame"));
            }
            Ok(n) => filled += n,
            Err(Errno::EINTR) => continue,
            Err(Errno::EAGAIN) => wait_for(fd, libc::POLLIN, Duration::MAX)?,
            Err(err) => return Err(anyhow::Error::new(err).context("Failed to receive frame")),
        }
    }
    Ok(true)
}

/// Performs a single `recvmsg`, taking ownership of any received fds.
fn receive_some(fd: RawFd, buf: &mut [u8], fds: &mut Vec<OwnedFd>) -> nix::Result<usize> {
    let mut buffers = [IoSliceMut::new(buf)];
    let mut cmsg_buffer = cmsg_space!(FrameAuxData);
    let msg: socket::RecvMsg<()> =
        socket::recvmsg(fd, &mut buffers, Some(&mut cmsg_buffer), receive_flags())?;
    let bytes = msg.bytes;
    let truncated = msg.flags.contains(socket::MsgFlags::MSG_CTRUNC);
    for cmsg in msg.cmsgs() {
        if let socket::ControlMessageOwned::ScmRights(received) = cmsg {
            for raw in received {
                // SAFETY: the kernel just installed this fd in our table and
                // nothing else owns it.
                let owned = unsafe { OwnedFd::from_raw_fd(raw) };
                let _ = set_cloexec(owned.as_raw_fd());
                fds.push(owned);
            }
        }
    }
    if truncated {
        log::warn!("Control message was truncated; some fds were dropped");
        return Err(Errno::EMSGSIZE);
    }
    Ok(bytes)
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn receive_flags() -> socket::MsgFlags {
    socket::MsgFlags::MSG_CMSG_CLOEXEC
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
fn receive_flags() -> socket::MsgFlags {
    socket::MsgFlags::empty()
}

#[cfg(any(target_os = "linux", target_os = "android"))]
pub(super) fn send_flags() -> socket::MsgFlags {
    socket::MsgFlags::MSG_NOSIGNAL
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
pub(super) fn send_flags() -> socket::MsgFlags {
    socket::MsgFlags::empty()
}

/// Sets `FD_CLOEXEC` on `fd`.
pub(super) fn set_cloexec(fd: RawFd) -> nix::Result<()> {
    use nix::fcntl;
    fcntl::fcntl(fd, fcntl::F_SETFD(fcntl::FdFlag::FD_CLOEXEC)).map(|_| ())
}

/// Sets `O_NONBLOCK` on `fd`.
pub(super) fn set_nonblocking(fd: RawFd) -> nix::Result<()> {
    use nix::fcntl;
    let flags = fcntl::OFlag::from_bits_truncate(fcntl::fcntl(fd, fcntl::F_GETFL)?);
    fcntl::fcntl(fd, fcntl::F_SETFL(flags | fcntl::OFlag::O_NONBLOCK)).map(|_| ())
}

/// Waits until `fd` reports `events`, or fails after `timeout`.
fn wait_for(fd: RawFd, events: libc::c_short, timeout: Duration) -> Result<()> {
    let timeout_ms = if timeout == Duration::MAX {
        -1
    } else {
        timeout.as_millis().min(libc::c_int::MAX as u128) as libc::c_int
    };
    loop {
        let mut pollfd = libc::pollfd {
            fd,
            events,
            revents: 0,
        };
        // SAFETY: `pollfd` is a valid, initialized pollfd for the duration of
        // the call.
        let res = unsafe { libc::poll(&mut pollfd, 1, timeout_ms) };
        match res {
            0 => bail!("Timed out waiting for socket readiness"),
            n if n > 0 => return Ok(()),
            _ => {
                let err = std::io::Error::last_os_error();
                if err.kind() != std::io::ErrorKind::Interrupted {
                    return Err(err).context("poll failed");
                }
            }
        }
    }
}

/// The outcome of [`FrameReader::fill`].
#[derive(Debug, PartialEq, Eq)]
pub(super) enum FillResult {
    /// Every available byte was read; the socket would block.
    WouldBlock,
    /// [`FILL_BUDGET`] bytes were read; more may be available.
    Budget,
    /// The peer closed the connection.
    Closed,
}

/// Incrementally reassembles frames from a non-blocking socket.
///
/// Bytes and fds are accumulated across reads. Each received fd is associated
/// with the absolute stream offset at which it arrived, which is the first byte
/// of the `sendmsg` call that carried it, so it is attributed to the frame that
/// contains that byte.
pub(super) struct FrameReader {
    buf: Vec<u8>,
    /// The absolute stream offset of `buf[0]`.
    consumed: u64,
    fds: VecDeque<(u64, OwnedFd)>,
    max_frame_size: usize,
}

impl Default for FrameReader {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameReader {
    pub fn new() -> Self {
        Self::with_max_frame_size(MAX_FRAME_SIZE)
    }

    pub fn with_max_frame_size(max_frame_size: usize) -> Self {
        Self {
            buf: Vec::new(),
            consumed: 0,
            fds: VecDeque::new(),
            max_frame_size,
        }
    }

    /// Reads available bytes from `fd`, up to [`FILL_BUDGET`].
    pub fn fill(&mut self, fd: RawFd) -> Result<FillResult> {
        let mut chunk = [0u8; 16 * 1024];
        let mut read = 0;
        loop {
            if read >= FILL_BUDGET {
                return Ok(FillResult::Budget);
            }
            let offset = self.consumed + self.buf.len() as u64;
            let mut fds = Vec::new();
            let result = receive_some(fd, &mut chunk, &mut fds);
            self.fds.extend(fds.into_iter().map(|fd| (offset, fd)));
            match result {
                Ok(0) => return Ok(FillResult::Closed),
                Ok(n) => {
                    read += n;
                    self.buf.extend_from_slice(&chunk[..n]);
                    if self.pending_frame_size()? > self.max_frame_size {
                        bail!("Received oversized frame");
                    }
                }
                Err(Errno::EINTR) => continue,
                Err(Errno::EAGAIN) => return Ok(FillResult::WouldBlock),
                Err(Errno::ECONNRESET) => return Ok(FillResult::Closed),
                Err(err) => bail!("Failed to read from socket: {err}"),
            }
        }
    }

    fn pending_frame_size(&self) -> Result<usize> {
        if self.buf.len() < USIZE_SIZE {
            return Ok(0);
        }
        let mut header = [0u8; USIZE_SIZE];
        header.copy_from_slice(&self.buf[..USIZE_SIZE]);
        let size = usize::from_be_bytes(header);
        ensure!(size > 0, "Received frame with invalid size 0");
        Ok(size)
    }

    /// Returns the next complete frame, if one has been fully received.
    pub fn next_frame<T: DeserializeOwned>(&mut self) -> Result<Option<(T, Vec<OwnedFd>)>> {
        let size = self.pending_frame_size()?;
        if size == 0 || self.buf.len() < USIZE_SIZE + size {
            return Ok(None);
        }
        ensure!(size <= self.max_frame_size, "Received oversized frame");
        let frame_len = USIZE_SIZE + size;
        let value = bincode::deserialize(&self.buf[USIZE_SIZE..frame_len])
            .context("Failed to deserialize frame")?;
        let frame_end = self.consumed + frame_len as u64;
        let mut fds = Vec::new();
        while self
            .fds
            .front()
            .is_some_and(|(offset, _)| *offset < frame_end)
        {
            if let Some((_, fd)) = self.fds.pop_front() {
                fds.push(fd);
            }
        }
        self.buf.drain(..frame_len);
        self.consumed = frame_end;
        Ok(Some((value, fds)))
    }
}

/// The outcome of [`FrameWriter::flush`].
#[derive(Debug, PartialEq, Eq)]
pub(super) enum FlushResult {
    /// Every queued frame was written.
    Done,
    /// The socket is full; flush again once it is writable.
    WouldBlock,
}

struct OutboundFrame {
    bytes: Vec<u8>,
    written: usize,
    fds: Vec<OwnedFd>,
}

/// Queues frames for a non-blocking socket and writes them as the socket
/// accepts them, so a slow peer never blocks the sender.
pub(super) struct FrameWriter {
    queue: VecDeque<OutboundFrame>,
    queued_bytes: usize,
    max_queued_bytes: usize,
}

impl FrameWriter {
    pub fn new(max_queued_bytes: usize) -> Self {
        Self {
            queue: VecDeque::new(),
            queued_bytes: 0,
            max_queued_bytes,
        }
    }

    /// Queues `value` with `fds` attached to its first byte. Fails if the
    /// queue would exceed its limit.
    pub fn push<T: Serialize>(&mut self, value: &T, fds: Vec<OwnedFd>) -> Result<()> {
        ensure!(
            fds.len() <= MAX_FRAME_FDS,
            "Cannot send {} fds in a single frame",
            fds.len()
        );
        let bytes = encode_frame(value)?;
        ensure!(
            self.queued_bytes + bytes.len() <= self.max_queued_bytes,
            "Outbound queue is full"
        );
        self.queued_bytes += bytes.len();
        self.queue.push_back(OutboundFrame {
            bytes,
            written: 0,
            fds,
        });
        Ok(())
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// Writes queued frames until the queue is empty or the socket would
    /// block. The fds of every frame whose first byte was written are moved to
    /// `sent_fds`, so the caller can keep them open while they are in flight.
    pub fn flush(&mut self, fd: RawFd, sent_fds: &mut Vec<OwnedFd>) -> Result<FlushResult> {
        while let Some(frame) = self.queue.front_mut() {
            let raw_fds: Vec<RawFd> = if frame.written == 0 {
                frame.fds.iter().map(AsRawFd::as_raw_fd).collect()
            } else {
                Vec::new()
            };
            let rights = [socket::ControlMessage::ScmRights(&raw_fds)];
            let control_msgs: &[socket::ControlMessage] =
                if raw_fds.is_empty() { &[] } else { &rights };
            match socket::sendmsg::<()>(
                fd,
                &[IoSlice::new(&frame.bytes[frame.written..])],
                control_msgs,
                send_flags(),
                None,
            ) {
                Ok(n) => {
                    if frame.written == 0 {
                        sent_fds.append(&mut frame.fds);
                    }
                    frame.written += n;
                    if frame.written == frame.bytes.len() {
                        self.queued_bytes -= frame.bytes.len();
                        self.queue.pop_front();
                    }
                }
                Err(Errno::EINTR) => continue,
                Err(Errno::EAGAIN) => return Ok(FlushResult::WouldBlock),
                Err(err) => return Err(anyhow::Error::new(err).context("Failed to send frame")),
            }
        }
        Ok(FlushResult::Done)
    }
}

#[cfg(test)]
#[path = "protocol_tests.rs"]
mod tests;
