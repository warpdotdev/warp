use std::collections::HashSet;

use fuzzy_match::FuzzyMatchResult;
use warpui::{AppContext, SingletonEntity};

use super::search_item::CommandSearchItem;
use crate::search::ai_context_menu::mixer::AIContextMenuSearchableAction;
use crate::search::data_source::{Query, QueryResult};
use crate::search::mixer::{DataSourceRunErrorWrapper, SyncDataSource};
use crate::terminal::History;

const MAX_RESULTS: usize = 50;

pub struct CommandDataSource;

impl CommandDataSource {
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self
    }

    /// Get terminal commands from all sessions' history
    fn get_terminal_commands(&self, app: &AppContext) -> Vec<String> {
        let history = History::as_ref(app);
        let mut unique_commands = Vec::new();
        let mut seen = HashSet::new();

        // Get all live session IDs from history
        let session_ids = history.all_live_session_ids();

        // Collect commands from all sessions, prioritizing more recent commands
        let mut all_commands = Vec::new();

        for session_id in session_ids {
            if let Some(commands) = history.commands(session_id) {
                // Add commands with their timestamps for sorting
                for entry in commands.iter() {
                    if !entry.command.trim().is_empty() {
                        all_commands.push((entry.command.clone(), entry.start_ts));
                    }
                }
            }
        }

        // Sort by timestamp (most recent first), using start_ts when available
        all_commands.sort_by(|a, b| {
            match (a.1, b.1) {
                (Some(a_time), Some(b_time)) => b_time.cmp(&a_time),
                (Some(_), None) => std::cmp::Ordering::Less, // timestamped commands first
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            }
        });

        // Deduplicate while preserving order (most recent occurrence wins)
        for (command, _) in all_commands {
            if !seen.contains(&command) {
                seen.insert(command.clone());
                unique_commands.push(command);

                // Limit to reasonable number of commands
                if unique_commands.len() >= MAX_RESULTS {
                    break;
                }
            }
        }

        unique_commands
    }

    /// Performs fuzzy matching on commands
    fn fuzzy_match_command(&self, command: &str, query: &str) -> Option<FuzzyMatchResult> {
        if query.is_empty() {
            return Some(FuzzyMatchResult::no_match());
        }

        fuzzy_match::match_indices_case_insensitive(command, query)
    }
}

impl SyncDataSource for CommandDataSource {
    type Action = AIContextMenuSearchableAction;

    fn run_query(
        &self,
        query: &Query,
        app: &AppContext,
    ) -> Result<Vec<QueryResult<Self::Action>>, DataSourceRunErrorWrapper> {
        let query_text = &query.text;
        let commands = self.get_terminal_commands(app);

        let results: Vec<QueryResult<AIContextMenuSearchableAction>> = if query_text.is_empty() {
            // Zero state: show recent commands without fuzzy matching
            commands
                .into_iter()
                .map(|command| {
                    let search_item = CommandSearchItem {
                        command,
                        match_result: FuzzyMatchResult::no_match(),
                    };
                    QueryResult::from(search_item)
                })
                .collect()
        } else {
            // Non-empty query: use fuzzy matching
            commands
                .into_iter()
                .filter_map(|command| {
                    let match_result = self.fuzzy_match_command(&command, query_text)?;
                    let search_item = CommandSearchItem {
                        command,
                        match_result,
                    };
                    Some(QueryResult::from(search_item))
                })
                .collect()
        };

        Ok(results)
    }
}

impl warpui::Entity for CommandDataSource {
    type Event = ();
}
