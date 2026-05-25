use objc2_core_graphics::{CGEvent, CGEventTapLocation};

/// Environment variable used to route computer-use events to a specific process by PID
/// instead of the system-wide HID event tap.
const TARGET_PID_ENV_VAR: &str = "COMPUTER_USE_TARGET_PID";

/// Describes where synthesized Quartz events are delivered.
///
/// This is an experimental knob for evaluating background, non-interfering control. The
/// default (`HidTap`) reproduces the historical behavior of injecting events as if they came
/// from real hardware.
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
    /// Determines the post target from the environment, falling back to the HID event tap.
    ///
    /// Setting `COMPUTER_USE_TARGET_PID` to a valid PID routes all events to that process. An
    /// unset or unparseable value leaves the historical HID behavior in place.
    pub fn from_env() -> Self {
        match std::env::var(TARGET_PID_ENV_VAR) {
            Ok(value) => match value.trim().parse::<libc::pid_t>() {
                Ok(pid) => PostTarget::Pid(pid),
                Err(_) => {
                    log::warn!(
                        "Ignoring invalid {TARGET_PID_ENV_VAR} value {value:?}; \
                         falling back to the HID event tap."
                    );
                    PostTarget::HidTap
                }
            },
            Err(_) => PostTarget::HidTap,
        }
    }

    /// Returns true when events are delivered directly to a process rather than the HID tap.
    pub fn is_pid_targeted(self) -> bool {
        matches!(self, PostTarget::Pid(_))
    }

    /// Posts the given event according to this target.
    pub fn post(self, event: &CGEvent) {
        match self {
            PostTarget::HidTap => CGEvent::post(CGEventTapLocation::HIDEventTap, Some(event)),
            PostTarget::Pid(pid) => CGEvent::post_to_pid(pid, Some(event)),
        }
    }
}
