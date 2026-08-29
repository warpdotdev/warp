use serde::{Deserialize, Deserializer, Serialize};

#[derive(Debug, Clone, Serialize, PartialEq)]
pub enum UserTakeOverReason {
    Manual,
    /// The user interrupted the command and took control. `should_auto_resume` is `true` for a
    /// live interrupt (e.g. Ctrl-C) that keeps the conversation alive so it resumes once the
    /// command completes, and `false` for teardown flows (stop/rewind) that have cancelled it.
    Stop {
        should_auto_resume: bool,
    },
    /// The agent explicitly transferred control to the user via the
    /// TransferShellCommandControlToUser tool call.
    TransferFromAgent {
        /// The reason the agent gave for transferring control.
        reason: String,
    },
}

impl<'de> Deserialize<'de> for UserTakeOverReason {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        // `Current` mirrors the derived shape so serde can parse it without recursing back into
        // this impl. `LegacyStop` accepts the bare `"Stop"` persisted before `should_auto_resume`
        // existed: an externally tagged enum can't accept `Stop` as both a unit (legacy) and a
        // struct (current) variant, and `#[serde(default)]` can't bridge the two, so the forms are
        // unioned as `untagged`.
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Wire {
            Current(Current),
            LegacyStop(LegacyStop),
        }

        #[derive(Deserialize)]
        enum Current {
            Manual,
            Stop { should_auto_resume: bool },
            TransferFromAgent { reason: String },
        }

        #[derive(Deserialize)]
        enum LegacyStop {
            Stop,
        }

        Ok(match Wire::deserialize(deserializer)? {
            Wire::Current(Current::Manual) => Self::Manual,
            Wire::Current(Current::Stop { should_auto_resume }) => {
                Self::Stop { should_auto_resume }
            }
            Wire::Current(Current::TransferFromAgent { reason }) => {
                Self::TransferFromAgent { reason }
            }
            Wire::LegacyStop(LegacyStop::Stop) => Self::Stop {
                should_auto_resume: false,
            },
        })
    }
}

impl UserTakeOverReason {
    pub fn is_stop(&self) -> bool {
        matches!(self, Self::Stop { .. })
    }

    pub fn is_transfer_from_agent(&self) -> bool {
        matches!(self, Self::TransferFromAgent { .. })
    }

    /// Returns `true` if the conversation should resume once the user-controlled command
    /// completes. Only a teardown `Stop` opts out.
    pub fn should_auto_resume(&self) -> bool {
        match self {
            Self::Manual | Self::TransferFromAgent { .. } => true,
            Self::Stop { should_auto_resume } => *should_auto_resume,
        }
    }

    pub fn transfer_reason(&self) -> Option<&str> {
        match self {
            Self::TransferFromAgent { reason } => Some(reason.as_str()),
            _ => None,
        }
    }
}

/// Represents which party is in control of the active long running command.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum LongRunningCommandControlState {
    /// The agent is in control.
    ///
    /// When the agent has control, the user cannot submit input to the command.
    Agent {
        /// `true` if the agent is blocked on approval from the user for submitting input.
        is_blocked: bool,
        /// `true` if agent responses should be hidden in the UI.
        should_hide_responses: bool,
    },
    /// The user is in control.
    User { reason: UserTakeOverReason },
}

impl LongRunningCommandControlState {
    pub fn is_agent_in_control(&self) -> bool {
        matches!(self, Self::Agent { .. })
    }

    pub fn is_agent_blocked(&self) -> bool {
        matches!(
            self,
            Self::Agent {
                is_blocked: true,
                ..
            }
        )
    }

    pub fn is_user_in_control(&self) -> bool {
        matches!(self, Self::User { .. })
    }

    /// Returns `true` if a completing user-controlled command should auto-resume the conversation.
    pub fn should_auto_resume(&self) -> bool {
        match self {
            Self::Agent { .. } => false,
            Self::User { reason } => reason.should_auto_resume(),
        }
    }

    pub fn should_hide_responses(&self) -> bool {
        matches!(
            self,
            Self::Agent {
                should_hide_responses: true,
                ..
            }
        )
    }

    pub fn user_take_over_reason(&self) -> Option<&UserTakeOverReason> {
        match &self {
            LongRunningCommandControlState::Agent { .. } => None,
            LongRunningCommandControlState::User { reason } => Some(reason),
        }
    }
}

#[cfg(test)]
#[path = "long_running_command_tests.rs"]
mod tests;
