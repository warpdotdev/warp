use warpui::{AppContext, EntityId, SingletonEntity};

use crate::input_suggestions::HistoryInputSuggestion;
use crate::terminal::history::{History, LinkedWorkflowData, UpArrowHistoryConfig};
use crate::terminal::model::session::SessionId;

/// An owned history item for the TUI up-arrow menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TuiUpArrowHistoryItem {
    pub text: String,
    pub kind: TuiUpArrowHistoryItemKind,
}

/// The input kind represented by a TUI up-arrow history item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TuiUpArrowHistoryItemKind {
    Prompt,
    Command {
        linked_workflow_data: Option<LinkedWorkflowData>,
    },
}

/// Returns an owned, de-duplicated history snapshot for the TUI up-arrow menu.
pub fn tui_up_arrow_history(
    terminal_surface_id: EntityId,
    session_id: Option<SessionId>,
    config: UpArrowHistoryConfig,
    app: &AppContext,
) -> Vec<TuiUpArrowHistoryItem> {
    History::handle(app)
        .as_ref(app)
        .up_arrow_suggestions_for_terminal_surface(terminal_surface_id, session_id, config, app)
        .into_iter()
        .map(|suggestion| {
            let text = suggestion.normalized_text().to_owned();
            match suggestion {
                HistoryInputSuggestion::Command { entry } => TuiUpArrowHistoryItem {
                    text,
                    kind: TuiUpArrowHistoryItemKind::Command {
                        linked_workflow_data: entry.linked_workflow_data(),
                    },
                },
                HistoryInputSuggestion::AIQuery { .. } => TuiUpArrowHistoryItem {
                    text,
                    kind: TuiUpArrowHistoryItemKind::Prompt,
                },
            }
        })
        .collect()
}
