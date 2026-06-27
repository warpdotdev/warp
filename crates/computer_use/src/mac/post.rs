use objc2_core_graphics::{CGEvent, CGEventTapLocation};

/// Describes where synthesized Quartz events are delivered.
///
/// This selects between the legacy whole-screen delivery and background, per-window delivery.
/// `HidTap` reproduces the historical behavior of injecting events as if they came from real
/// hardware, while `Pid` delivers directly to a process for background, non-interfering control.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PostTarget {
    /// Inject at the HID event tap, exactly as real hardware would. This moves the real
    /// cursor and the event is routed to whichever application is frontmost.
    HidTap,
    /// Deliver the event directly to a specific process by PID via `CGEventPostToPid`. This
    /// does not move the global cursor and does not require the target to be frontmost, at
    /// the cost of reduced reliability (especially for mouse events).
    Pid(libc::pid_t),
}

impl PostTarget {
    /// Returns true when events are delivered directly to a process rather than the HID tap.
    pub fn is_pid_targeted(self) -> bool {
        matches!(self, PostTarget::Pid(_))
    }

    /// Returns the target PID, if events are delivered directly to a process.
    pub fn pid(self) -> Option<libc::pid_t> {
        match self {
            PostTarget::Pid(pid) => Some(pid),
            PostTarget::HidTap => None,
        }
    }

    /// Posts the given event according to this target.
    pub fn post(self, event: &CGEvent) {
        match self {
            PostTarget::HidTap => CGEvent::post(CGEventTapLocation::HIDEventTap, Some(event)),
            PostTarget::Pid(pid) => CGEvent::post_to_pid(pid, Some(event)),
        }
    }
}
