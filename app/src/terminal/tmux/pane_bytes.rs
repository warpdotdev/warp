use std::io::{self, Write};

use warp_core::SessionId;

use crate::terminal::TerminalModel;
use crate::terminal::model::ansi;
use crate::terminal::model::terminal_model::ExitReason;
use crate::terminal::tmux::parser::{ControlEvent, ControlModeParser, PaneId};
use crate::terminal::tmux::protocol::split_stage_complete;

/// Feed tmux `-CC` bytes into a local TerminalModel.
///
/// Only decoded `%output` pane bytes reach the ANSI processor. Protocol chatter never does.
pub fn feed_control_bytes<H, W>(
    parser: &mut ControlModeParser,
    ansi_parser: &mut ansi::Processor,
    model: &mut H,
    writer: &mut W,
    tracked_pane: &mut Option<PaneId>,
    bytes: &[u8],
) -> FeedResult
where
    H: ansi::Handler,
    W: Write,
{
    let mut result = FeedResult::default();
    for event in parser.push(bytes) {
        match event {
            ControlEvent::EnteredControlMode => {
                result.entered_control_mode = true;
            }
            ControlEvent::PaneOutput { pane_id, bytes } => {
                let (bytes, complete) = split_stage_complete(&bytes);
                for session_id in complete {
                    result.stage_complete.push((pane_id.clone(), session_id));
                }
                if tracked_pane.as_ref() == Some(&pane_id) {
                    ansi_parser.parse_bytes(model, &bytes, writer);
                    result.pane_bytes += bytes.len();
                }
            }
            ControlEvent::WindowPaneChanged { pane_id, .. } => {
                *tracked_pane = Some(pane_id);
            }
            ControlEvent::Exit { .. } => {
                result.exited = true;
            }
            ControlEvent::CommandBegin { .. }
            | ControlEvent::CommandEnd { .. }
            | ControlEvent::LayoutChange { .. }
            | ControlEvent::WindowAdd { .. }
            | ControlEvent::WindowClose { .. }
            | ControlEvent::WindowRenamed { .. }
            | ControlEvent::SessionWindowChanged { .. }
            | ControlEvent::ProtocolOverflow => {}
        }
    }
    result
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FeedResult {
    pub entered_control_mode: bool,
    pub pane_bytes: usize,
    pub exited: bool,
    pub stage_complete: Vec<(PaneId, SessionId)>,
}

pub fn notify_exit(model: &mut TerminalModel) {
    model.exit(ExitReason::ShellProcessExited);
}

/// Discard writer used when the pane has no local PTY to answer terminal queries.
pub fn sink_writer() -> io::Sink {
    io::sink()
}

#[cfg(test)]
#[path = "pane_bytes_tests.rs"]
mod tests;
