use std::borrow::Cow;

use crate::SizeInfo;
use crate::tmux::PaneId;

/// Messages that may be sent to the `EventLoop`.
#[derive(Debug)]
pub enum Message {
    /// Data that should be written to the PTY.
    Input(Cow<'static, [u8]>),

    /// One tmux control-mode command line, written as-is to the `tmux -CC` PTY.
    TmuxControlCommand(Cow<'static, [u8]>),

    /// Bytes destined for a tmux pane; encoded once as `send-keys`.
    TmuxPaneInput {
        pane_id: PaneId,
        bytes: Cow<'static, [u8]>,
    },

    /// Indicates that the `EventLoop` should be shut down.
    Shutdown,

    /// Indicates that the child process has exited.
    ///
    /// Only used on Windows, as we need to pass this information to the
    /// event loop via the channel (and cannot use the child event token).
    #[cfg_attr(not(windows), allow(dead_code))]
    ChildExited,

    /// Instruction to resize the PTY.
    Resize(SizeInfo),
}
