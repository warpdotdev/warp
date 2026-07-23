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
        .filter_map(|suggestion| match suggestion {
            HistoryInputSuggestion::Command { entry } => {
                let text = entry.command.trim();
                (!text.is_empty()).then(|| TuiUpArrowHistoryItem {
                    text: text.to_owned(),
                    kind: TuiUpArrowHistoryItemKind::Command {
                        linked_workflow_data: entry.linked_workflow_data(),
                    },
                })
            }
            HistoryInputSuggestion::AIQuery { entry } => (!entry.query_text.trim().is_empty())
                .then_some(TuiUpArrowHistoryItem {
                    text: entry.query_text,
                    kind: TuiUpArrowHistoryItemKind::Prompt,
                }),
        })
        .collect()
}
