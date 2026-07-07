//! This module contains the `Priority` struct, which may be specified on [`Suggestion`]s to
//! influence the order of suggestions returned to users.  

cfg_if::cfg_if! {
    if #[cfg(feature = "v2")] {
        mod v2;
    }
}

use std::cmp::Ordering;

use serde::{Deserialize, Serialize};

/// The lowest number we can represent for a Priority.
const MIN_PRIORITY: i32 = -100;
/// We default to this Priority value if it is not otherwise provided.
const DEFAULT_PRIORITY: i32 = 0;
/// The highest number we can represent for a Priority.
const MAX_PRIORITY: i32 = 100;

/// Priority is part of how we rank completion suggestions. For non-default priority values, we
/// break ties with lexicographic ordering. Higher values are higher priority and appear earlier in
/// lists of suggestsions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Priority(i32);

impl Priority {
    /// Creates a new Priority with its value clamped to the range [-100, 100].
    pub fn new(value: i32) -> Self {
        Self(value.clamp(MIN_PRIORITY, MAX_PRIORITY))
    }

    pub fn value(&self) -> i32 {
        self.0
    }

    pub fn min() -> Self {
        Self::new(MIN_PRIORITY)
    }

    pub fn max() -> Self {
        Self::new(MAX_PRIORITY)
    }
}

impl Default for Priority {
    fn default() -> Self {
        Self::new(DEFAULT_PRIORITY)
    }
}

impl Ord for Priority {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.cmp(&other.0)
    }
}

impl PartialOrd for Priority {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl From<warp_command_signatures::Priority> for Priority {
    fn from(value: warp_command_signatures::Priority) -> Self {
        match value {
            warp_command_signatures::Priority::Global(importance)
            | warp_command_signatures::Priority::Local(importance) => match importance {
                warp_command_signatures::Importance::More(order) => Self::new(order.0 as i32),
                warp_command_signatures::Importance::Less(order) => {
                    Self::new(-(101 - order.0 as i32))
                }
            },
            warp_command_signatures::Priority::Default => Priority::default(),
        }
    }
}

impl From<Priority> for warp_command_signatures::Priority {
    fn from(value: Priority) -> Self {
        match value.cmp(&Priority::default()) {
            Ordering::Less => warp_command_signatures::Priority::Global(
                warp_command_signatures::Importance::Less(warp_command_signatures::Order(
                    101 - value.value().unsigned_abs(),
                )),
            ),
            Ordering::Equal => warp_command_signatures::Priority::default(),
            Ordering::Greater => warp_command_signatures::Priority::Global(
                warp_command_signatures::Importance::More(warp_command_signatures::Order(
                    value.value().unsigned_abs(),
                )),
            ),
        }
    }
}

#[cfg(test)]
#[path = "priority_tests.rs"]
mod tests;
