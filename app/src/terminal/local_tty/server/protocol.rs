use std::{
    io::{IoSlice, IoSliceMut},
    os::unix::prelude::*,
};

use anyhow::{anyhow, bail, ensure, Context, Result};
use itertools::Itertools;
use nix::{cmsg_space, errno::Errno, sys::socket};
use serde::{Deserialize, Serialize};

use crate::terminal::local_tty::PtySpawnResult;

use super::api;

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
            return Ok(TryReceiveMessageResult::SocketClosed)
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
