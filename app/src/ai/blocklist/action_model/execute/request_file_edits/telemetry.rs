/// Coarse format classification for the edit payload that produced a code diff.
///
/// This distinguishes the legacy search/replace edit format from the structured
/// V4A patch format used by `apply_patch`.
use ai::diff_validation::DiffMatchFailures;
use serde::Serialize;
use serde_json::json;
use strum_macros::{EnumDiscriminants, EnumIter};
use warp_core::telemetry::{EnablementState, TelemetryEvent, TelemetryEventDesc};

use crate::ai::agent::AIIdentifiers;
use crate::ai::blocklist::RequestedEditResolution;

/// Telemetry events associated with the `RequestFileEdits` AI agent action.
#[derive(Serialize, Debug, EnumDiscriminants)]
#[strum_discriminants(derive(EnumIter))]
pub enum RequestFileEditsTelemetryEvent {
    EditResolved(EditResolvedEvent),
    EditAcceptClicked(EditAcceptClickedEvent),
    EditAcceptAndContinueClicked(EditAcceptAndContinueClickedEvent),
    DiffMatchFailed(DiffMatchFailedEvent),
    DiffInvalidFile(DiffInvalidFileEvent),
    EditReceived(EditReceivedEvent),
    MissingLineNumbers(MissingLineNumbersEvent),
    MalformedFinalLineProxy(MalformedFinalLineProxyEvent),
}

/// Emitted when a user Accepts or Rejects a code diff suggestsion from Agent Mode.
#[derive(Serialize, Debug)]
pub struct EditResolvedEvent {
    #[serde(flatten)]
    pub identifiers: AIIdentifiers,
    pub response: RequestedEditResolution,
    /// Information about the resolved edit, only set if it is accepted.
    pub stats: EditStats,
    /// Whether this is a passive diff.
    pub passive_diff: bool,
}

/// Emitted when a user selects Accept for a code diff suggestion.
#[derive(Serialize, Debug)]
pub struct EditAcceptClickedEvent {
    #[serde(flatten)]
    pub identifiers: AIIdentifiers,
    /// Whether this is a passive diff.
    pub passive_diff: bool,
}

/// Emitted when a user selects Accept and start conversation for a code diff suggestion.
#[derive(Serialize, Debug)]
pub struct EditAcceptAndContinueClickedEvent {
    #[serde(flatten)]
    pub identifiers: AIIdentifiers,
}

#[derive(Serialize, Debug)]
pub struct EditStats {
    /// Number of files that were edited.
    pub files_edited: usize,
    /// Number of lines that were added.
    pub lines_added: usize,
    /// Number of lines that were removed.
    pub lines_removed: usize,
}

#[derive(Serialize, Debug)]
pub struct DiffMatchFailedEvent {
    #[serde(flatten)]
    pub identifiers: AIIdentifiers,
    #[serde(flatten)]
    pub failures: DiffMatchFailures,
    /// Whether this is a passive diff.
    pub passive_diff: bool,
}

/// Could not find the file(s) given in a code diff.
#[derive(Serialize, Debug)]
pub struct DiffInvalidFileEvent {
    #[serde(flatten)]
    pub identifiers: AIIdentifiers,
    pub count: usize,
    /// Whether this is a passive diff.
    pub passive_diff: bool,
}

/// Emitted when code edits are displayed to the user.
#[derive(Serialize, Debug)]
pub struct EditReceivedEvent {
    #[serde(flatten)]
    pub identifiers: AIIdentifiers,

    /// Number of unique files in the code diff.
    pub unique_files: usize,

    /// Total number of diffs in the event.
    pub diffs: usize,

    /// Whether this is a passive diff.
    pub passive_diff: bool,
}

/// Emitted when search blocks are missing line numbers (non-fatal warning).
#[derive(Serialize, Debug)]
pub struct MissingLineNumbersEvent {
    #[serde(flatten)]
    pub identifiers: AIIdentifiers,
    /// Number of search blocks missing line numbers.
    pub count: u8,
    /// Whether this is a passive diff.
    pub passive_diff: bool,
}

#[derive(Serialize, Debug, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum RequestFileEditsFormatKind {
    /// Legacy search/replace diff format (`edit_files` style).
    StrReplace,
    /// Structured V4A patch format (`apply_patch` style with Begin/End Patch hunks).
    V4A,
    /// Both formats were present in the same requested edit payload.
    Mixed,
    /// The format could not be determined from the payload.
    Unknown,
}

/// Emitted when accepted diffs indicate a likely malformed trailing-line condition.
///
/// This signal is emitted when final changed lines intersect the model-proposed terminal changed
/// range and the proposed terminal line matches a malformed-line heuristic.
#[derive(Serialize, Debug)]
pub struct MalformedFinalLineProxyEvent {
    #[serde(flatten)]
    pub identifiers: AIIdentifiers,
    /// Number of files included in the accepted edit.
    pub file_count: usize,
    /// Number of files that were edited by the user prior to accepting.
    pub edited_file_count: usize,
    /// Number of files where:
    /// - final changed lines intersect the model-proposed terminal changed range, and
    /// - the proposed terminal line matched the malformed-line heuristic.
    pub correction_count: usize,
    /// Number of `correction_count` detections where `was_edited` was true.
    pub edited_correction_count: usize,
    /// Number of `correction_count` detections where `was_edited` was false.
    pub unedited_correction_count: usize,
    /// Coarse source format for the requested edit payload.
    pub format_kind: RequestFileEditsFormatKind,
    /// Whether this is a passive diff.
    pub passive_diff: bool,
}

impl TelemetryEvent for RequestFileEditsTelemetryEvent {
    fn name(&self) -> &'static str {
        RequestFileEditsTelemetryEventDiscriminants::from(self).name()
    }

    fn payload(&self) -> Option<serde_json::Value> {
        match self {
            RequestFileEditsTelemetryEvent::EditResolved(resolved_edit_event) => {
                Some(json!(resolved_edit_event))
            }
            RequestFileEditsTelemetryEvent::EditAcceptClicked(edit_accept_clicked_event) => {
                Some(json!(edit_accept_clicked_event))
            }
            RequestFileEditsTelemetryEvent::EditAcceptAndContinueClicked(
                edit_accept_and_continue_clicked_event,
            ) => Some(json!(edit_accept_and_continue_clicked_event)),
            RequestFileEditsTelemetryEvent::DiffMatchFailed(diff_application_error_event) => {
                Some(json!(diff_application_error_event))
            }
            RequestFileEditsTelemetryEvent::DiffInvalidFile(diff_invalid_file_event) => {
                Some(json!(diff_invalid_file_event))
            }
            RequestFileEditsTelemetryEvent::EditReceived(edit_received_event) => {
                Some(json!(edit_received_event))
            }
            RequestFileEditsTelemetryEvent::MissingLineNumbers(missing_line_numbers_event) => {
                Some(json!(missing_line_numbers_event))
            }
            RequestFileEditsTelemetryEvent::MalformedFinalLineProxy(
                malformed_final_line_proxy_event,
            ) => Some(json!(malformed_final_line_proxy_event)),
        }
    }

    fn description(&self) -> &'static str {
        RequestFileEditsTelemetryEventDiscriminants::from(self).description()
    }

    fn enablement_state(&self) -> EnablementState {
        RequestFileEditsTelemetryEventDiscriminants::from(self).enablement_state()
    }

    fn contains_ugc(&self) -> bool {
        RequestFileEditsTelemetryEventDiscriminants::from(self).contains_ugc()
    }

    fn event_descs() -> impl Iterator<Item = Box<dyn TelemetryEventDesc>> {
        warp_core::telemetry::enum_events::<Self>()
    }
}

impl RequestFileEditsTelemetryEventDiscriminants {
    pub fn contains_ugc(&self) -> bool {
        false
    }
}

impl TelemetryEventDesc for RequestFileEditsTelemetryEventDiscriminants {
    fn name(&self) -> &'static str {
        match self {
            Self::EditResolved => "AgentMode.Code.SuggestedEditResolved",
            Self::EditAcceptClicked => "AgentMode.Code.SuggestedEditAcceptClicked",
            Self::EditAcceptAndContinueClicked => {
                "AgentMode.Code.SuggestedEditAcceptAndContinueClicked"
            }
            Self::DiffMatchFailed => "AgentMode.Code.DiffMatchFailed",
            Self::DiffInvalidFile => "AgentMode.Code.InvalidFile",
            Self::EditReceived => "AgentMode.Code.SuggestedEditReceived",
            Self::MissingLineNumbers => "AgentMode.Code.MissingLineNumbers",
            Self::MalformedFinalLineProxy => "AgentMode.Code.MalformedFinalLineProxy",
        }
    }

    fn description(&self) -> &'static str {
        match self {
            Self::EditResolved => "Agent Mode pending code edit suggestion resolved",
            Self::EditAcceptClicked => {
                "User selected Accept for a code diff suggestion in Agent Mode"
            }
            Self::EditAcceptAndContinueClicked => {
                "User selected Accept and start conversation for a code diff suggestion in Agent Mode"
            }
            Self::DiffMatchFailed => "Failed to match code diff",
            Self::DiffInvalidFile => "File(s) in code diff could not be found",
            Self::EditReceived => "Agent Mode suggested a code edit",
            Self::MissingLineNumbers => "Code diff was missing line numbers",
            Self::MalformedFinalLineProxy => {
                "Suggested code diff likely required malformed trailing line correction (heuristic)"
            }
        }
    }

    fn enablement_state(&self) -> EnablementState {
        EnablementState::Always
    }
}

warp_core::register_telemetry_event!(RequestFileEditsTelemetryEvent);
