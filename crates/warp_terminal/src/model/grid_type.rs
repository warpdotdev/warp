use enum_iterator::Sequence;

#[derive(Debug, PartialEq, Copy, Clone, Eq, PartialOrd, Sequence, Hash)]
pub enum GridType {
    Prompt,
    Rprompt,          // Right side prompt
    PromptAndCommand, // Combined prompt/command grid.
    Output,
}

impl From<GridType> for session_sharing_protocol::common::GridType {
    fn from(val: GridType) -> Self {
        match val {
            GridType::Prompt => session_sharing_protocol::common::GridType::Prompt,
            GridType::Rprompt => session_sharing_protocol::common::GridType::Rprompt,
            GridType::Output => session_sharing_protocol::common::GridType::Output,
            GridType::PromptAndCommand => {
                session_sharing_protocol::common::GridType::PromptAndCommand
            }
        }
    }
}

impl From<session_sharing_protocol::common::GridType> for GridType {
    fn from(value: session_sharing_protocol::common::GridType) -> Self {
        match value {
            session_sharing_protocol::common::GridType::Prompt => Self::Prompt,
            session_sharing_protocol::common::GridType::Rprompt => Self::Rprompt,
            session_sharing_protocol::common::GridType::Output => Self::Output,
            session_sharing_protocol::common::GridType::PromptAndCommand => Self::PromptAndCommand,
        }
    }
}
