#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
/// Because it's hard to identify the contents of rich content blocks,
/// we register unique identifiers to make it easier to identify them.
pub enum RichContentType {
    AIBlock,
    EnterAgentView,
    WarpifySuccessBlock,
    InlineAgentViewHeader,
    AgentViewZeroState,
    TerminalViewZeroState,
    PluginInstructionsBlock,
}

impl RichContentType {
    pub fn is_ai_block(&self) -> bool {
        matches!(self, Self::AIBlock)
    }

    pub fn is_agent_view_block(&self) -> bool {
        matches!(self, Self::EnterAgentView)
    }

    pub fn is_inline_agent_view_header(&self) -> bool {
        matches!(self, Self::InlineAgentViewHeader)
    }
}
