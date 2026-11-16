/// Who authors the turns of an `AIConversation`, and therefore which client-side behaviours
/// apply to it. Call sites should ask the derived predicates rather than match on variants, so
/// a new driver only has to answer each predicate once.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ConversationDriver {
    /// Warp's agent loop: the MAA server streams events and this client executes tools.
    #[default]
    Native,
    /// Another client's conversation mirrored over a shared session; view-only here.
    SharedSessionViewer,
    /// A restored 3p CLI transcript rendered through a vehicle conversation; view-only here.
    CliAgentTranscript,
    /// Placeholder for a child agent executing on a remote worker, whose own client drives it.
    RemoteChild,
    /// A harness running outside Warp's agent loop in this client's session; it authors the MAA
    /// events itself and runs its own tools.
    ExternalHarness,
}

impl ConversationDriver {
    /// This client sends follow-ups, settles status from action results, and may cancel the turn.
    pub fn owns_turn_lifecycle(self) -> bool {
        match self {
            Self::Native => true,
            Self::SharedSessionViewer
            | Self::CliAgentTranscript
            | Self::RemoteChild
            | Self::ExternalHarness => false,
        }
    }

    /// This client runs the conversation's tool calls through the action model.
    pub fn executes_tool_calls_locally(self) -> bool {
        match self {
            Self::Native => true,
            Self::SharedSessionViewer
            | Self::CliAgentTranscript
            | Self::RemoteChild
            | Self::ExternalHarness => false,
        }
    }

    /// Exchange inputs are rebuilt from streamed `UserQuery` / `ToolCallResult` messages instead
    /// of being inserted by the local executor when the request is sent.
    pub fn reconstructs_inputs_from_messages(self) -> bool {
        match self {
            Self::SharedSessionViewer | Self::ExternalHarness => true,
            Self::Native | Self::CliAgentTranscript | Self::RemoteChild => false,
        }
    }

    /// This client reports the conversation's status to the task sync model.
    pub fn reports_task_status(self) -> bool {
        match self {
            Self::Native | Self::CliAgentTranscript | Self::ExternalHarness => true,
            Self::SharedSessionViewer | Self::RemoteChild => false,
        }
    }

    /// The agent view renders this conversation without an input footer or other affordances
    /// that would let the user act on it.
    pub fn is_read_only_ui(self) -> bool {
        match self {
            Self::SharedSessionViewer | Self::CliAgentTranscript => true,
            Self::Native | Self::RemoteChild | Self::ExternalHarness => false,
        }
    }

    /// The conversation is persisted to the local session database.
    pub fn is_persisted_locally(self) -> bool {
        match self {
            Self::Native | Self::CliAgentTranscript | Self::ExternalHarness => true,
            Self::SharedSessionViewer | Self::RemoteChild => false,
        }
    }
}

#[cfg(test)]
#[path = "conversation_driver_tests.rs"]
mod tests;
