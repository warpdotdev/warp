use std::fmt::Display;
use std::time::Duration;

use serde::Serialize;
use serde_json::json;
use serde_with::SerializeDisplay;
use strum_macros::{EnumDiscriminants, EnumIter};
use warp_core::telemetry::{EnablementState, TelemetryEvent, TelemetryEventDesc};

use crate::code_review::diff_state::DiffMode;
use crate::features::FeatureFlag;
use crate::server::telemetry::CLIAgentType;
use crate::view_components::find::FindDirection;

/// Identifies which git button the user clicked in the code review header.
/// Each variant maps to one of the primary action button / dropdown items.
#[derive(Clone, Copy, Debug, Serialize)]
pub enum GitButtonKind {
    #[serde(rename = "commit")]
    Commit,
    #[serde(rename = "push")]
    Push,
    #[serde(rename = "publish")]
    Publish,
    #[serde(rename = "create_pr")]
    CreatePr,
    #[serde(rename = "view_pr")]
    ViewPr,
}

/// Identifies which git operation actually ran when a `GitDialog` completed.
/// Distinguishes commit-dialog chained intents (e.g. commit-and-push) from
/// standalone push/publish/create-PR dialogs so analytics can tell the user
/// flows apart.
#[derive(Clone, Copy, Debug, Serialize)]
pub enum GitOperationKind {
    /// Commit dialog with the commit-only intent.
    #[serde(rename = "commit_only")]
    CommitOnly,
    /// Commit dialog with the commit-and-push intent.
    #[serde(rename = "commit_and_push")]
    CommitAndPush,
    /// Commit dialog with the commit-and-create-PR intent.
    #[serde(rename = "commit_and_create_pr")]
    CommitAndCreatePr,
    /// Standalone push dialog.
    #[serde(rename = "push")]
    Push,
    /// Standalone publish dialog (push that also sets upstream).
    #[serde(rename = "publish")]
    Publish,
    /// Standalone create-PR dialog.
    #[serde(rename = "create_pr")]
    CreatePr,
}

/// Terminal status of a `GitDialog`. Captures both async-op outcomes and
/// pre-confirmation user cancels in a single enum.
#[derive(Clone, Copy, Debug, Serialize)]
pub enum GitDialogStatus {
    /// User confirmed the dialog and the underlying git operation succeeded.
    #[serde(rename = "succeeded")]
    Succeeded,
    /// User confirmed the dialog and the underlying git operation failed.
    #[serde(rename = "failed")]
    Failed,
    /// User cancelled the dialog (ESC / close button / cancel button) before
    /// the async op ran.
    #[serde(rename = "cancelled")]
    Cancelled,
}

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
        is_local: Option<bool>,
        entrypoint: CodeReviewPaneEntrypoint,
        is_code_mode_v2: bool,
        /// The CLI agent type if opened from a CLI agent footer (e.g., Claude Code).
        cli_agent: Option<CLIAgentType>,
    },
    /// Emitted when a user adds content to AI context from code review.
    AddToContext {
        is_local: Option<bool>,
        origin: AddToContextOrigin,
        destination: CodeReviewContextDestination,
        diff_set_scope: Option<DiffSetContextScope>,
    },
    /// Emitted when a user clicks the revert hunk button.
    RevertHunkClicked { is_local: Option<bool> },
    /// Emitted when a file is saved in the code review pane.
    FileSaved { is_local: Option<bool> },
    /// Emitted when the code review pane is minimized or maximized.
    PaneStateChanged {
        is_local: Option<bool>,
        state_change: PaneStateChange,
    },
    /// Emitted when the diff base is changed (e.g., from uncommitted to main branch).
    BaseChanged {
        is_local: Option<bool>,
        /// The new diff mode.
        mode: DiffMode,
    },
    /// Failure when we are calculating the diff metadata.
    LoadMetadataFailed {
        is_local: Option<bool>,
        mode: DiffMode,
        error: String,
    },
    /// Failure when we are loading the actual diff content.
    LoadDiffFailed {
        is_local: Option<bool>,
        mode: DiffMode,
        error: String,
        /// Time elapsed between when the tracked diff load was requested and
        /// when this failure was observed. `None` if no tracked load was in
        /// flight (e.g. a background invalidation error).
        load_duration: Option<Duration>,
    },
    /// Emitted when a full diff load completes successfully.
    DiffLoadCompleted {
        is_local: Option<bool>,
        mode: DiffMode,
        file_count: usize,
        files_changed: usize,
        total_additions: usize,
        total_deletions: usize,
        /// Time elapsed between when the diff load was requested and when the
        /// diffs were ready. `None` if the load was not initiated through a
        /// tracked entry point (e.g. background refresh).
        load_duration: Option<Duration>,
    },
    /// Emitted when the code review find bar is opened or closed.
    FindBarToggled {
        is_local: Option<bool>,
        /// Whether the find bar is now open.
        is_open: bool,
    },
    /// Emitted when search mode settings are changed.
    FindBarModeChanged {
        is_local: Option<bool>,
        /// Whether case-sensitive search is enabled.
        case_sensitive: bool,
        /// Whether regex search is enabled.
        regex: bool,
    },
    /// Emitted when the user navigates to the next or previous match.
    FindNavigated {
        is_local: Option<bool>,
        /// Direction of navigation.
        direction: FindDirection,
    },
    /// Emitted when the inline comment editor is opened in the code review pane.
    CommentEditorOpened { is_local: Option<bool> },
    /// Emitted when a new comment is added to the inline review.
    CommentAdded { is_local: Option<bool> },
    /// Emitted when an existing comment is edited.
    CommentEdited { is_local: Option<bool> },
    /// Emitted when a comment is deleted from the inline review.
    CommentDeleted {
        is_local: Option<bool>,
        is_imported: bool,
    },
    /// Emitted when the bottom comment list panel is expanded.
    CommentListExpanded {
        is_local: Option<bool>,
        /// Number of comments currently in the list.
        comment_count: usize,
    },
    /// Emitted when the user submits an inline review to the agent.
    ReviewSubmitted {
        is_local: Option<bool>,
        /// Number of comments in the submitted review.
        comment_count: usize,
        /// Number of unique files with comments.
        file_count: usize,
        /// Where the review was submitted.
        destination: CodeReviewContextDestination,
    },
    /// Emitted when a comment in the list view is clicked to jump to its location.
    CommentListItemClicked { is_local: Option<bool> },
    /// Emitted when one or more comments fail to be precisely relocated after code changes.
    CommentRelocationFailed {
        is_local: Option<bool>,
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
        is_local: Option<bool>,
        /// Number of raw InsertReviewComment items from the tool call.
        raw_count: usize,
        /// Number of successfully converted PendingImportedReviewComments.
        converted_count: usize,
        /// Number of AttachedReviewComments after thread flattening.
        thread_count: usize,
    },
    /// Emitted after newly-imported comments are relocated against editor lines.
    CommentsAttached {
        is_local: Option<bool>,
        /// Number of non-outdated imported comments after relocation.
        active_count: usize,
        /// Number of outdated imported comments after relocation.
        outdated_count: usize,
    },
    /// Emitted when a user clicks a git operation button in the code review
    /// header (primary button or dropdown item).
    GitButtonTriggered {
        is_local: Option<bool>,
        button: GitButtonKind,
    },
    /// Emitted when a git dialog reaches a terminal state — either the async
    /// op succeeded / failed, or the user cancelled before confirming.
    GitDialogCompleted {
        is_local: Option<bool>,
        /// The git operation that ran or would have run (e.g. `commit_and_push`
        /// for the commit dialog with that chained intent).
        operation: GitOperationKind,
        /// Whether the dialog succeeded, failed, or was cancelled.
        status: GitDialogStatus,
        /// Raw error string when `status == Failed`, `None` otherwise.
        error: Option<String>,
    },
}

impl TelemetryEvent for CodeReviewTelemetryEvent {
    fn name(&self) -> &'static str {
        CodeReviewTelemetryEventDiscriminants::from(self).name()
    }

    fn payload(&self) -> Option<serde_json::Value> {
        match self {
            CodeReviewTelemetryEvent::PaneOpened {
                is_local,
                entrypoint,
                is_code_mode_v2,
                cli_agent,
            } => Some(json!({
                "is_local": is_local,
                "entrypoint": entrypoint,
                "is_code_mode_v2": is_code_mode_v2,
                "agent_name": cli_agent,
            })),
            CodeReviewTelemetryEvent::AddToContext {
                is_local,
                origin,
                destination,
                diff_set_scope,
            } => Some(json!({
                "is_local": is_local,
                "origin": origin,
                "destination": destination,
                "diff_set_scope": diff_set_scope,
            })),
            CodeReviewTelemetryEvent::RevertHunkClicked { is_local } => {
                Some(json!({ "is_local": is_local }))
            }
            CodeReviewTelemetryEvent::FileSaved { is_local } => {
                Some(json!({ "is_local": is_local }))
            }
            CodeReviewTelemetryEvent::PaneStateChanged {
                is_local,
                state_change,
            } => Some(json!({ "is_local": is_local, "state_change": state_change })),
            CodeReviewTelemetryEvent::BaseChanged { is_local, mode } => {
                Some(json!({ "is_local": is_local, "mode": mode }))
            }
            CodeReviewTelemetryEvent::LoadMetadataFailed {
                is_local,
                mode,
                error,
            } => Some(json!({ "is_local": is_local, "mode": mode, "error": error })),
            CodeReviewTelemetryEvent::LoadDiffFailed {
                is_local,
                mode,
                error,
                load_duration,
            } => Some(json!({
                "is_local": is_local,
                "mode": mode,
                "error": error,
                "load_duration": load_duration,
            })),
            CodeReviewTelemetryEvent::DiffLoadCompleted {
                is_local,
                mode,
                file_count,
                files_changed,
                total_additions,
                total_deletions,
                load_duration,
            } => Some(json!({
                "is_local": is_local,
                "mode": mode,
                "file_count": file_count,
                "files_changed": files_changed,
                "total_additions": total_additions,
                "total_deletions": total_deletions,
                "load_duration": load_duration,
            })),
            CodeReviewTelemetryEvent::FindBarToggled { is_local, is_open } => {
                Some(json!({ "is_local": is_local, "is_open": is_open }))
            }
            CodeReviewTelemetryEvent::FindBarModeChanged {
                is_local,
                case_sensitive,
                regex,
            } => Some(json!({
                "is_local": is_local,
                "case_sensitive": case_sensitive,
                "regex": regex,
            })),
            CodeReviewTelemetryEvent::FindNavigated {
                is_local,
                direction,
            } => Some(json!({ "is_local": is_local, "direction": direction })),
            CodeReviewTelemetryEvent::CommentEditorOpened { is_local } => {
                Some(json!({ "is_local": is_local }))
            }
            CodeReviewTelemetryEvent::CommentAdded { is_local } => {
                Some(json!({ "is_local": is_local }))
            }
            CodeReviewTelemetryEvent::CommentEdited { is_local } => {
                Some(json!({ "is_local": is_local }))
            }
            CodeReviewTelemetryEvent::CommentDeleted {
                is_local,
                is_imported,
            } => Some(json!({ "is_local": is_local, "is_imported": is_imported })),
            CodeReviewTelemetryEvent::CommentListExpanded {
                is_local,
                comment_count,
            } => Some(json!({ "is_local": is_local, "comment_count": comment_count })),
            CodeReviewTelemetryEvent::ReviewSubmitted {
                is_local,
                comment_count,
                file_count,
                destination,
            } => Some(json!({
                "is_local": is_local,
                "comment_count": comment_count,
                "file_count": file_count,
                "destination": destination,
            })),
            CodeReviewTelemetryEvent::CommentListItemClicked { is_local } => {
                Some(json!({ "is_local": is_local }))
            }
            CodeReviewTelemetryEvent::CommentRelocationFailed {
                is_local,
                fallback_count,
            } => Some(json!({ "is_local": is_local, "fallback_count": fallback_count })),
            CodeReviewTelemetryEvent::CommentResolved { resolved_count } => {
                Some(json!({ "resolved_count": resolved_count }))
            }
            CodeReviewTelemetryEvent::CommentsReceived {
                is_local,
                raw_count,
                converted_count,
                thread_count,
            } => Some(json!({
                "is_local": is_local,
                "raw_count": raw_count,
                "converted_count": converted_count,
                "thread_count": thread_count,
            })),
            CodeReviewTelemetryEvent::CommentsAttached {
                is_local,
                active_count,
                outdated_count,
            } => Some(json!({
                "is_local": is_local,
                "active_count": active_count,
                "outdated_count": outdated_count,
            })),
            CodeReviewTelemetryEvent::GitButtonTriggered { is_local, button } => {
                Some(json!({ "is_local": is_local, "button": button }))
            }
            CodeReviewTelemetryEvent::GitDialogCompleted {
                is_local,
                operation,
                status,
                error,
            } => Some(json!({
                "is_local": is_local,
                "operation": operation,
                "status": status,
                "error": error,
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
            Self::LoadMetadataFailed => "CodeReview.LoadMetadataFailed",
            Self::LoadDiffFailed => "CodeReview.LoadDiffFailed",
            Self::DiffLoadCompleted => "CodeReview.DiffLoadCompleted",
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
            Self::GitButtonTriggered => "CodeReview.GitButtonTriggered",
            Self::GitDialogCompleted => "CodeReview.GitDialogCompleted",
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
            Self::LoadMetadataFailed => "Failure when calculating diff metadata",
            Self::LoadDiffFailed => "Failure when loading diff content",
            Self::DiffLoadCompleted => "Diff content loaded successfully",
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
            Self::GitButtonTriggered => {
                "User clicked a git operation button in the code review header"
            }
            Self::GitDialogCompleted => {
                "Git operation dialog reached a terminal state (succeeded, failed, or cancelled)"
            }
        }
    }

    fn enablement_state(&self) -> EnablementState {
        match self {
            Self::CommentsReceived | Self::CommentsAttached => {
                EnablementState::Flag(FeatureFlag::PRCommentsV2)
            }
            Self::GitButtonTriggered | Self::GitDialogCompleted => {
                EnablementState::Flag(FeatureFlag::GitOperationsInCodeReview)
            }
            _ => EnablementState::Always,
        }
    }
}

warp_core::register_telemetry_event!(CodeReviewTelemetryEvent);
