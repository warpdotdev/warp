use crate::server::telemetry::CLIAgentType;
use crate::view_components::find::FindDirection;
use crate::{code_review::diff_state::DiffMode, features::FeatureFlag};
use serde::Serialize;
use serde_json::json;
use serde_with::SerializeDisplay;
use std::fmt::Display;
use strum_macros::{EnumDiscriminants, EnumIter};
use warp_core::telemetry::{EnablementState, TelemetryEvent, TelemetryEventDesc};

/// Entry points for opening the code review pane.
#[derive(Clone, Copy, Debug, SerializeDisplay, Default)]
pub enum CodeReviewPaneEntrypoint {
    /// Opened via the git diff chip (git changes button in AI control panel).
    GitDiffChip,
    /// Opened via the "View changes" button when Agent mode is done running.
    AgentModeCompleted,
    /// Opened via the "Review changes" button when Agent mode is running.
    AgentModeRunning,
    /// Opened via the "/code-review" slash command.
    SlashCommand,
    /// Opened by the agent tool call.
    InvokedByAgent,
    // Force opened when user accepted first diff of a conversation
    ForceOpened,
    // Opened via the agent mode diff header
    CodeDiffHeader,
    // Opened via the pane header
    PaneHeader,
    // Opened via the code mode v2 right panel button
    RightPanel,
    /// Opened via the CLI agent view footer (e.g., Claude Code).
    CLIAgentView,
    /// Opened via other means (unknown entry point).
    #[default]
    Other,
}

impl Display for CodeReviewPaneEntrypoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::GitDiffChip => write!(f, "git_diff_chip"),
            Self::AgentModeCompleted => write!(f, "agent_mode_completed"),
            Self::AgentModeRunning => write!(f, "agent_mode_running"),
            Self::SlashCommand => write!(f, "slash_command"),
            Self::InvokedByAgent => write!(f, "invoked_by_agent"),
            Self::ForceOpened => write!(f, "force_opened"),
            Self::CodeDiffHeader => write!(f, "agent_mode_diff_header"),
            Self::PaneHeader => write!(f, "pane_header"),
            Self::RightPanel => write!(f, "right_panel"),
            Self::CLIAgentView => write!(f, "cli_agent_view"),
            Self::Other => write!(f, "other"),
        }
    }
}

/// Origin of an "Add to context" action.
#[derive(Clone, Copy, Debug, Serialize)]
pub enum AddToContextOrigin {
    /// User selected text and added it to context.
    #[serde(rename = "selected_text")]
    SelectedText,
    /// User clicked the gutter to add a line/hunk to context.
    #[serde(rename = "gutter")]
    Gutter,
    /// User clicked the "Add diff set as context" button in code review header.
    #[serde(rename = "code_review_header")]
    #[allow(unused)]
    CodeReviewHeader,
}

/// Where code review content was sent after the user action.
#[derive(Clone, Copy, Debug, Serialize)]
pub enum CodeReviewContextDestination {
    /// Written directly to the terminal PTY for an active CLI agent.
    #[serde(rename = "pty")]
    Pty,
    /// Inserted into the Warp AI input buffer as plain text.
    #[serde(rename = "agent_input")]
    AgentInput,
    /// Registered as an AI attachment and referenced from the input.
    #[serde(rename = "agent_attachment")]
    AgentAttachment,
    /// Inserted into the active command buffer while a command is running.
    #[serde(rename = "active_command_buffer")]
    ActiveCommandBuffer,
    /// Submitted as an inline code review request through the Warp AI path.
    #[serde(rename = "agent_review")]
    AgentReview,
    /// Inserted into CLI agent rich input.
    #[serde(rename = "rich_input")]
    RichInput,
}

/// Scope of a diff set attachment initiated from code review.
#[derive(Clone, Copy, Debug, Serialize)]
#[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
pub enum DiffSetContextScope {
    /// Attach the full diff set for the current review.
    #[serde(rename = "all")]
    All,
    /// Attach the diff set for a single file.
    #[serde(rename = "file")]
    File,
}

/// Pane state change for minimize/maximize events.
#[derive(Clone, Copy, Debug, Serialize)]
pub enum PaneStateChange {
    /// Pane was minimized.
    #[serde(rename = "minimized")]
    Minimized,
    /// Pane was maximized.
    #[serde(rename = "maximized")]
    Maximized,
}

/// Telemetry events associated with the code review pane.
#[derive(Serialize, Debug, EnumDiscriminants)]
#[strum_discriminants(derive(EnumIter))]
#[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
pub enum CodeReviewTelemetryEvent {
    /// Emitted when the code review pane is opened.
    PaneOpened {
        entrypoint: CodeReviewPaneEntrypoint,
        is_code_mode_v2: bool,
        /// The CLI agent type if opened from a CLI agent footer (e.g., Claude Code).
        cli_agent: Option<CLIAgentType>,
    },
    /// Emitted when a user adds content to AI context from code review.
    AddToContext {
        origin: AddToContextOrigin,
        destination: CodeReviewContextDestination,
        diff_set_scope: Option<DiffSetContextScope>,
    },
    /// Emitted when a user clicks the revert hunk button.
    RevertHunkClicked,
    /// Emitted when a file is saved in the code review pane.
    FileSaved,
    /// Emitted when the code review pane is minimized or maximized.
    PaneStateChanged { state_change: PaneStateChange },
    /// Emitted when the diff base is changed (e.g., from uncommitted to main branch).
    BaseChanged {
        /// The new diff mode.
        mode: DiffMode,
    },
    /// Failure when we are calculating the diff metadata.
    CalculateDiffMetadataFailed { error: String },
    /// Failure when we are loading the actual diff content.
    LoadDiffFailed { error: String },
    /// Emitted when the code review find bar is opened or closed.
    FindBarToggled {
        /// Whether the find bar is now open.
        is_open: bool,
    },
    /// Emitted when search mode settings are changed.
    FindBarModeChanged {
        /// Whether case-sensitive search is enabled.
        case_sensitive: bool,
        /// Whether regex search is enabled.
        regex: bool,
    },
    /// Emitted when the user navigates to the next or previous match.
    FindNavigated {
        /// Direction of navigation.
        direction: FindDirection,
    },
    /// Emitted when the inline comment editor is opened in the code review pane.
    CommentEditorOpened,
    /// Emitted when a new comment is added to the inline review.
    CommentAdded,
    /// Emitted when an existing comment is edited.
    CommentEdited,
    /// Emitted when a comment is deleted from the inline review.
    CommentDeleted { is_imported: bool },
    /// Emitted when the bottom comment list panel is expanded.
    CommentListExpanded {
        /// Number of comments currently in the list.
        comment_count: usize,
    },
    /// Emitted when the user submits an inline review to the agent.
    ReviewSubmitted {
        /// Number of comments in the submitted review.
        comment_count: usize,
        /// Number of unique files with comments.
        file_count: usize,
        /// Where the review was submitted.
        destination: CodeReviewContextDestination,
    },
    /// Emitted when a comment in the list view is clicked to jump to its location.
    CommentListItemClicked,
    /// Emitted when one or more comments fail to be precisely relocated after code changes.
    CommentRelocationFailed {
        /// Number of comments that could not be matched to an exact line and had to fall back.
        fallback_count: usize,
    },
    /// Emitted when one or more comments are resolved.
    CommentResolved {
        /// Number of comments resolved by this operation.
        resolved_count: usize,
    },
    /// Emitted when the agent's insert_code_review_comments tool call is received and processed.
    CommentsReceived {
        /// Number of raw InsertReviewComment items from the tool call.
        raw_count: usize,
        /// Number of successfully converted PendingImportedReviewComments.
        converted_count: usize,
        /// Number of AttachedReviewComments after thread flattening.
        thread_count: usize,
    },
    /// Emitted after newly-imported comments are relocated against editor lines.
    CommentsAttached {
        /// Number of non-outdated imported comments after relocation.
        active_count: usize,
        /// Number of outdated imported comments after relocation.
        outdated_count: usize,
    },
}

impl TelemetryEvent for CodeReviewTelemetryEvent {
    fn name(&self) -> &'static str {
        CodeReviewTelemetryEventDiscriminants::from(self).name()
    }

    fn payload(&self) -> Option<serde_json::Value> {
        match self {
            CodeReviewTelemetryEvent::PaneOpened {
                entrypoint,
                is_code_mode_v2,
                cli_agent,
            } => Some(
                json!({ "entrypoint": entrypoint, "is_code_mode_v2": is_code_mode_v2, "agent_name": cli_agent}),
            ),
            CodeReviewTelemetryEvent::AddToContext {
                origin,
                destination,
                diff_set_scope,
            } => Some(json!({
                "origin": origin,
                "destination": destination,
                "diff_set_scope": diff_set_scope,
            })),
            CodeReviewTelemetryEvent::RevertHunkClicked => None,
            CodeReviewTelemetryEvent::FileSaved => None,
            CodeReviewTelemetryEvent::PaneStateChanged { state_change } => {
                Some(json!({ "state_change": state_change }))
            }
            CodeReviewTelemetryEvent::BaseChanged { mode } => Some(json!({ "mode": mode })),
            CodeReviewTelemetryEvent::CalculateDiffMetadataFailed { error } => {
                Some(json!({ "error": error }))
            }
            CodeReviewTelemetryEvent::LoadDiffFailed { error } => Some(json!({ "error": error })),
            CodeReviewTelemetryEvent::FindBarToggled { is_open } => {
                Some(json!({ "is_open": is_open }))
            }
            CodeReviewTelemetryEvent::FindBarModeChanged {
                case_sensitive,
                regex,
            } => Some(json!({
                "case_sensitive": case_sensitive,
                "regex": regex,
            })),
            CodeReviewTelemetryEvent::FindNavigated { direction } => {
                Some(json!({ "direction": direction }))
            }
            CodeReviewTelemetryEvent::CommentEditorOpened => None,
            CodeReviewTelemetryEvent::CommentAdded => None,
            CodeReviewTelemetryEvent::CommentEdited => None,
            CodeReviewTelemetryEvent::CommentDeleted { is_imported } => {
                Some(json!({ "is_imported": is_imported }))
            }
            CodeReviewTelemetryEvent::CommentListExpanded { comment_count } => {
                Some(json!({ "comment_count": comment_count }))
            }
            CodeReviewTelemetryEvent::ReviewSubmitted {
                comment_count,
                file_count,
                destination,
            } => Some(json!({
                "comment_count": comment_count,
                "file_count": file_count,
                "destination": destination,
            })),
            CodeReviewTelemetryEvent::CommentListItemClicked => None,
            CodeReviewTelemetryEvent::CommentRelocationFailed { fallback_count } => {
                Some(json!({ "fallback_count": fallback_count }))
            }
            CodeReviewTelemetryEvent::CommentResolved { resolved_count } => {
                Some(json!({ "resolved_count": resolved_count }))
            }
            CodeReviewTelemetryEvent::CommentsReceived {
                raw_count,
                converted_count,
                thread_count,
            } => Some(json!({
                "raw_count": raw_count,
                "converted_count": converted_count,
                "thread_count": thread_count,
            })),
            CodeReviewTelemetryEvent::CommentsAttached {
                active_count,
                outdated_count,
            } => Some(json!({
                "active_count": active_count,
                "outdated_count": outdated_count,
            })),
        }
    }

    fn description(&self) -> &'static str {
        CodeReviewTelemetryEventDiscriminants::from(self).description()
    }

    fn enablement_state(&self) -> EnablementState {
        CodeReviewTelemetryEventDiscriminants::from(self).enablement_state()
    }

    fn contains_ugc(&self) -> bool {
        CodeReviewTelemetryEventDiscriminants::from(self).contains_ugc()
    }

    fn event_descs() -> impl Iterator<Item = Box<dyn TelemetryEventDesc>> {
        warp_core::telemetry::enum_events::<Self>()
    }
}

impl CodeReviewTelemetryEventDiscriminants {
    pub fn contains_ugc(&self) -> bool {
        false
    }
}

impl TelemetryEventDesc for CodeReviewTelemetryEventDiscriminants {
    fn name(&self) -> &'static str {
        match self {
            Self::PaneOpened => "CodeReview.PaneOpened",
            Self::AddToContext => "CodeReview.AddToContext",
            Self::RevertHunkClicked => "CodeReview.RevertHunkClicked",
            Self::FileSaved => "CodeReview.FileSaved",
            Self::PaneStateChanged => "CodeReview.PaneStateChanged",
            Self::BaseChanged => "CodeReview.BaseChanged",
            Self::CalculateDiffMetadataFailed => "CodeReview.CalculateDiffMetadataFailed",
            Self::LoadDiffFailed => "CodeReview.LoadDiffFailed",
            Self::FindBarToggled => "CodeReview.FindBarToggled",
            Self::FindBarModeChanged => "CodeReview.FindBarModeChanged",
            Self::FindNavigated => "CodeReview.FindNavigated",
            Self::CommentEditorOpened => "CodeReview.CommentEditorOpened",
            Self::CommentAdded => "CodeReview.CommentAdded",
            Self::CommentEdited => "CodeReview.CommentEdited",
            Self::CommentDeleted => "CodeReview.CommentDeleted",
            Self::CommentListExpanded => "CodeReview.CommentListExpanded",
            Self::ReviewSubmitted => "CodeReview.ReviewSubmitted",
            Self::CommentListItemClicked => "CodeReview.CommentListItemClicked",
            Self::CommentRelocationFailed => "CodeReview.CommentRelocationFailed",
            Self::CommentResolved => "CodeReview.CommentResolved",
            Self::CommentsReceived => "CodeReview.CommentsReceived",
            Self::CommentsAttached => "CodeReview.CommentsAttached",
        }
    }

    fn description(&self) -> &'static str {
        match self {
            Self::PaneOpened => "Code review pane opened",
            Self::AddToContext => "Content added to AI context from code review",
            Self::RevertHunkClicked => "Revert hunk button clicked",
            Self::FileSaved => "File saved in code review pane",
            Self::PaneStateChanged => "Code review pane minimized or maximized",
            Self::BaseChanged => "Diff base changed in code review",
            Self::CalculateDiffMetadataFailed => "Failure when calculating diff metadata",
            Self::LoadDiffFailed => "Failure when loading diff content",
            Self::FindBarToggled => "Code review find bar opened or closed",
            Self::FindBarModeChanged => "Search mode changed in code review find bar",
            Self::FindNavigated => "Navigated to next or previous match in code review find bar",
            Self::CommentEditorOpened => "Inline code review comment editor opened",
            Self::CommentAdded => "Inline code review comment added",
            Self::CommentEdited => "Inline code review comment edited",
            Self::CommentDeleted => "Inline code review comment deleted",
            Self::CommentListExpanded => "Inline code review comment list expanded",
            Self::ReviewSubmitted => "Inline code review submitted to agent",
            Self::CommentListItemClicked => "Inline code review comment list item clicked",
            Self::CommentRelocationFailed => {
                "Inline code review comment relocation fell back to approximate line"
            }
            Self::CommentResolved => "Inline code review comment resolved",
            Self::CommentsReceived => {
                "Agent insert_code_review_comments tool call received and processed"
            }
            Self::CommentsAttached => "Newly-imported comments relocated against editor lines",
        }
    }

    fn enablement_state(&self) -> EnablementState {
        match self {
            Self::CommentsReceived | Self::CommentsAttached => {
                EnablementState::Flag(FeatureFlag::PRCommentsV2)
            }
            _ => EnablementState::Always,
        }
    }
}

warp_core::register_telemetry_event!(CodeReviewTelemetryEvent);
