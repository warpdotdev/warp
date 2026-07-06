//! Data source for the inline history menu, providing both conversations and commands.
//!
//! Ordering semantics match the legacy up-arrow history menu:
//! - Items from different sessions appear before items from the current session
//! - Within each group, items are sorted by timestamp (oldest first)
//! - Commands are deduplicated, keeping the most recent occurrence
//! - The result is that current session items appear at the bottom (closer to input)

use chrono::{DateTime, Local};
use fuzzy_match::FuzzyMatchResult;
use ordered_float::OrderedFloat;
use warpui::{AppContext, Entity, EntityId, ModelHandle, SingletonEntity};

use crate::ai::agent::conversation::{AIConversationId, ConversationStatus};
use crate::ai::blocklist::agent_view::AgentViewController;
use crate::ai::blocklist::BlocklistAIHistoryModel;
use crate::input_suggestions::{HistoryInputSuggestion, HistoryOrder};
use crate::search::data_source::{Query, QueryFilter, QueryResult};
use crate::search::mixer::DataSourceRunErrorWrapper;
use crate::search::SyncDataSource;
use crate::terminal::history::{History, LinkedWorkflowData, UpArrowHistoryConfig};
use crate::terminal::input::inline_history::search_item::InlineHistoryItem;
use crate::terminal::input::inline_menu::{
    InlineMenuAction, InlineMenuClickBehavior, InlineMenuType,
};
use crate::terminal::model::session::active_session::ActiveSession;
use crate::terminal::model::session::SessionId;

#[derive(Clone, Debug)]
pub enum AcceptHistoryItem {
    Conversation {
        conversation_id: AIConversationId,
        title: String,
    },
    Command {
        command: String,
        linked_workflow_data: Option<LinkedWorkflowData>,
    },
    AIPrompt {
        query_text: String,
    },
}

impl AcceptHistoryItem {
    pub fn buffer_replacement_text(&self) -> Option<&String> {
        match self {
            AcceptHistoryItem::Command { command, .. } => Some(command),
            AcceptHistoryItem::AIPrompt { query_text } => Some(query_text),
            AcceptHistoryItem::Conversation { .. } => None,
        }
    }
}

impl InlineMenuAction for AcceptHistoryItem {
    const MENU_TYPE: InlineMenuType = InlineMenuType::InlineHistoryMenu;

    fn click_behavior(&self) -> InlineMenuClickBehavior {
        match self {
            AcceptHistoryItem::Conversation { .. } => InlineMenuClickBehavior::AcceptOnClick,
            AcceptHistoryItem::Command { .. } | AcceptHistoryItem::AIPrompt { .. } => {
                InlineMenuClickBehavior::SelectOnClick
            }
        }
    }
}

/// Data source that provides both live conversations for a terminal view and command history.
pub struct InlineHistoryMenuDataSource {
    terminal_view_id: EntityId,
    active_session: ModelHandle<ActiveSession>,
    agent_view_controller: ModelHandle<AgentViewController>,
}

impl InlineHistoryMenuDataSource {
    pub fn new(
        terminal_view_id: EntityId,
        active_session: ModelHandle<ActiveSession>,
        agent_view_controller: ModelHandle<AgentViewController>,
    ) -> Self {
        Self {
            terminal_view_id,
            active_session,
            agent_view_controller,
        }
    }

    fn build_agent_view_results(
        &self,
        query: &Query,
        prefix_match_len: usize,
        session_id: Option<SessionId>,
        app: &AppContext,
    ) -> Vec<QueryResult<AcceptHistoryItem>> {
        let trimmed_query = query.text.trim();
        let include_commands =
            query.filters.is_empty() || query.filters.contains(&QueryFilter::Commands);
        let include_prompts =
            query.filters.is_empty() || query.filters.contains(&QueryFilter::PromptHistory);

        let history = History::handle(app).as_ref(app);
        let config = UpArrowHistoryConfig {
            include_commands,
            include_prompts,
        };
        let suggestions = history.up_arrow_suggestions_for_terminal_view(
            self.terminal_view_id,
            session_id,
            config,
            app,
        );

        let mut results: Vec<QueryResult<AcceptHistoryItem>> = Vec::new();
        for suggestion in suggestions {
            if !trimmed_query.is_empty() && !suggestion.text().starts_with(trimmed_query) {
                continue;
            }

            let (search_item, score) = match suggestion {
                HistoryInputSuggestion::Command { entry } => {
                    let command = entry.command.trim();
                    if command.is_empty() {
                        continue;
                    }
                    let timestamp = entry.start_ts.unwrap_or_else(Local::now);
                    (
                        InlineHistoryItem::command(
                            command.to_string(),
                            entry.linked_workflow_data(),
                            timestamp,
                        )
                        .with_prefix_match_len(prefix_match_len),
                        OrderedFloat(results.len() as f64),
                    )
                }
                HistoryInputSuggestion::AIQuery { entry } => {
                    let query_text = entry.query_text.trim();
                    if query_text.is_empty() {
                        continue;
                    }
                    (
                        InlineHistoryItem::ai_prompt(query_text.to_string(), entry.start_time)
                            .with_prefix_match_len(prefix_match_len),
                        OrderedFloat(results.len() as f64),
                    )
                }
            };

            results.push(QueryResult::from(search_item.with_score(score)));
        }

        results
    }

    fn build_conversation_entries(&self, trimmed_query: &str, app: &AppContext) -> Vec<MenuEntry> {
        let mut conversation_entries: Vec<MenuEntry> = Vec::new();
        let history_model = BlocklistAIHistoryModel::handle(app).as_ref(app);
        for conversation in
            history_model.all_live_conversations_for_terminal_surface(self.terminal_view_id)
        {
            if conversation.is_entirely_passive() || conversation.exchange_count() == 0 {
                continue;
            }

            let Some(timestamp) = conversation.last_modified_at() else {
                continue;
            };
            let title = conversation
                .title()
                .unwrap_or_else(|| "Untitled conversation".to_string());
            let match_result = if trimmed_query.is_empty() {
                None
            } else {
                let result = fuzzy_match::match_indices_case_insensitive(&title, trimmed_query);
                if result.is_none() || result.as_ref().is_some_and(|r| r.score < 50) {
                    continue;
                }
                result
            };

            conversation_entries.push(MenuEntry {
                order: HistoryOrder::CurrentSession,
                sort_timestamp: timestamp,
                item: MenuItem::Conversation {
                    conversation_id: conversation.id(),
                    title,
                    status: conversation.status().clone(),
                    match_result,
                    display_timestamp: timestamp,
                },
            });
        }

        conversation_entries.sort_by(|a, b| {
            a.order
                .cmp(&b.order)
                .then(a.sort_timestamp.cmp(&b.sort_timestamp))
        });
        conversation_entries
    }
}

#[derive(Clone)]
struct MenuEntry {
    order: HistoryOrder,
    sort_timestamp: DateTime<Local>,
    item: MenuItem,
}

#[derive(Clone)]
enum MenuItem {
    Conversation {
        conversation_id: AIConversationId,
        title: String,
        status: ConversationStatus,
        match_result: Option<FuzzyMatchResult>,
        display_timestamp: DateTime<Local>,
    },
    Command {
        command: String,
        linked_workflow_data: Option<LinkedWorkflowData>,
        display_timestamp: DateTime<Local>,
        prefix_match_len: usize,
    },
}

fn interleave_conversations(base: Vec<MenuEntry>, conversations: Vec<MenuEntry>) -> Vec<MenuEntry> {
    let current_start_idx = base
        .iter()
        .position(|e| e.order == HistoryOrder::CurrentSession)
        .unwrap_or(base.len());

    let mut merged: Vec<MenuEntry> = Vec::with_capacity(base.len() + conversations.len());
    merged.extend(base.iter().take(current_start_idx).cloned());

    let base_current = base.into_iter().skip(current_start_idx).collect::<Vec<_>>();
    let mut conversations = conversations;
    conversations.sort_by(|a, b| a.sort_timestamp.cmp(&b.sort_timestamp));

    let mut i = 0;
    for conv in conversations {
        while i < base_current.len() && base_current[i].sort_timestamp < conv.sort_timestamp {
            merged.push(base_current[i].clone());
            i += 1;
        }
        merged.push(conv);
    }
    merged.extend(base_current.into_iter().skip(i));

    merged
}

impl SyncDataSource for InlineHistoryMenuDataSource {
    type Action = AcceptHistoryItem;

    fn run_query(
        &self,
        query: &Query,
        app: &AppContext,
    ) -> Result<Vec<QueryResult<Self::Action>>, DataSourceRunErrorWrapper> {
        let trimmed_query = query.text.trim();
        let prefix_match_len = trimmed_query.len();

        let session_id = self.active_session.as_ref(app).session(app).map(|s| s.id());
        let is_agent_view = self.agent_view_controller.as_ref(app).is_active();

        if is_agent_view {
            return Ok(self.build_agent_view_results(query, prefix_match_len, session_id, app));
        }

        let include_commands =
            query.filters.is_empty() || query.filters.contains(&QueryFilter::Commands);
        let include_conversations =
            query.filters.is_empty() || query.filters.contains(&QueryFilter::Conversations);

        let history = History::handle(app).as_ref(app);
        let all_live_session_ids = history.all_live_session_ids();

        let command_entries = if include_commands {
            history
                .up_arrow_suggestions_for_terminal_view(
                    self.terminal_view_id,
                    session_id,
                    UpArrowHistoryConfig {
                        include_commands: true,
                        include_prompts: false,
                    },
                    app,
                )
                .into_iter()
                .filter_map(|suggestion| {
                    let HistoryInputSuggestion::Command { entry } = &suggestion else {
                        return None;
                    };

                    let command = entry.command.trim();
                    if command.is_empty() {
                        return None;
                    }
                    if !trimmed_query.is_empty() && !command.starts_with(trimmed_query) {
                        return None;
                    }

                    let order = suggestion.history_order(session_id, &all_live_session_ids);
                    let sort_timestamp = entry.start_ts.unwrap_or_default();
                    let display_timestamp = entry.start_ts.unwrap_or_else(Local::now);

                    Some(MenuEntry {
                        order,
                        sort_timestamp,
                        item: MenuItem::Command {
                            command: command.to_string(),
                            linked_workflow_data: entry.linked_workflow_data(),
                            display_timestamp,
                            prefix_match_len,
                        },
                    })
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };

        let conversation_entries = if include_conversations {
            self.build_conversation_entries(trimmed_query, app)
        } else {
            Vec::new()
        };
        let merged_entries = interleave_conversations(command_entries, conversation_entries);

        let mut results: Vec<QueryResult<AcceptHistoryItem>> = Vec::new();
        for entry in merged_entries {
            let score = OrderedFloat(results.len() as f64);
            let search_item = match entry.item {
                MenuItem::Conversation {
                    conversation_id,
                    title,
                    status,
                    match_result,
                    display_timestamp,
                } => InlineHistoryItem::conversation(
                    conversation_id,
                    title,
                    status,
                    display_timestamp,
                )
                .with_name_match_result(match_result),
                MenuItem::Command {
                    command,
                    linked_workflow_data,
                    display_timestamp,
                    prefix_match_len,
                } => InlineHistoryItem::command(command, linked_workflow_data, display_timestamp)
                    .with_prefix_match_len(prefix_match_len),
            };

            results.push(QueryResult::from(search_item.with_score(score)));
        }

        Ok(results)
    }
}

impl Entity for InlineHistoryMenuDataSource {
    type Event = ();
}

#[cfg(test)]
#[path = "data_source_tests.rs"]
mod tests;
