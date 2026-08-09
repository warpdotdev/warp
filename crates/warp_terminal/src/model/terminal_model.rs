use std::ops::RangeInclusive;

use super::Point;
pub trait RangeInModel {
    fn range(&self) -> RangeInclusive<Point>;
}
#[derive(Debug, Copy, Clone)]
pub enum ExitReason {
    /// The shell process exited naturally
    ShellProcessExited,
    /// PTY spawn failed
    PtySpawnFailed,
    /// PTY connection was lost/disconnected
    PtyDisconnected,
    /// Process was killed/terminated
    ProcessKilled,
    /// Shell could not be found/determined
    ShellNotFound,
}
