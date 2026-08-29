/// A unique identifier for an inline banner.
pub type InlineBannerId = usize;

/// Type of inline banner - determines behavior like visibility in agent view.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum InlineBannerType {
    NotificationsDiscovery,
    NotificationsError,
    PromptSuggestions,
    AliasExpansion,
    SharedSessionStart,
    SharedSessionEnd,
    ShellProcessTerminated,
    OpenInWarp,
    VimMode,
    CodebaseIndexSpeedbump,
    AgentModeSetup,
    AwsBedrockLogin,
    AwsCliNotInstalled,
}

impl InlineBannerType {
    /// Returns whether this banner type should be visible when agent view is active.
    /// Exhaustive match ensures new banner types must define their visibility.
    pub fn is_visible_in_agent_view(&self) -> bool {
        match self {
            // Agent-related banners: visible in agent view
            Self::PromptSuggestions
            | Self::CodebaseIndexSpeedbump
            | Self::AgentModeSetup
            | Self::AwsBedrockLogin
            | Self::AwsCliNotInstalled => true,
            // Terminal-context banners: hidden in agent view
            Self::NotificationsDiscovery
            | Self::NotificationsError
            | Self::AliasExpansion
            | Self::SharedSessionStart
            | Self::SharedSessionEnd
            | Self::ShellProcessTerminated
            | Self::OpenInWarp
            | Self::VimMode => false,
        }
    }
}

/// An inline banner with its unique ID and type metadata.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct InlineBannerItem {
    pub id: InlineBannerId,
    pub banner_type: InlineBannerType,
}

impl InlineBannerItem {
    pub fn new(id: InlineBannerId, banner_type: InlineBannerType) -> Self {
        Self { id, banner_type }
    }
}

/// A unique identifier for a subshell separator.
pub type SeparatorId = usize;
