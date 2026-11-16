#[derive(thiserror::Error, Debug)]
pub enum ConvertToAPITypeError {
    /// There is no API type for the given value.
    ///
    /// This means the value just be ignored during request construction.
    #[error("Ignoring value when constructing API type.")]
    Ignore,
    #[error("Conversion from type {0} is unimplemented.")]
    Unimplemented(String),
    #[error("Encountered error converting types for MultiAgentApi request: {0:?}")]
    Other(#[from] anyhow::Error),
}

/// Unexpected errors when trying to convert an [`api::message::ToolCall`] to an [`AIAgentAction`].
#[derive(Debug, thiserror::Error)]
pub enum ToolToAIAgentActionError {
    #[error("Missing tool")]
    MissingTool,
    #[error("Could not parse args for MCP tool call: {0}")]
    CallMCPToolArgsError(String),
    #[error("Error converting suggest prompt tool call: {0}")]
    SuggestPromptError(String),
    #[error("Required coordinates for computer use action were missing")]
    MissingComputerUseCoordinates,
    #[error("Required scroll distance for mouse wheel action was missing")]
    MissingComputerUseScrollDistance,
    #[error("Received missing computer use action type")]
    MissingComputerUseActionType,
    #[error("Wait duration must be non-negative")]
    InvalidComputerUseWaitDuration,
    #[error("Required key for KeyDown/KeyUp action was missing")]
    MissingComputerUseKey,
    #[error("Character key was empty")]
    InvalidComputerUseCharKey,
    #[error("Received unexpected tool")]
    UnexpectedTool,
    #[error("Missing required reference for read skill tool call")]
    MissingSkillReference,
    #[error("Missing required file reference for upload artifact tool call")]
    MissingUploadArtifactFileReference,
    #[error("Invalid recording window id: {0:?}")]
    InvalidRecordingWindowId(String),
}
