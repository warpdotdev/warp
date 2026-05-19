use futures_lite::future::yield_now;
use std::sync::Arc;
use warpui::{AppContext, SingletonEntity};

use crate::search::async_snapshot_data_source::AsyncSnapshotDataSource;
use crate::search::command_search::searcher::CommandSearchItemAction;
use crate::search::data_source::{Query, QueryResult};
use crate::search::mixer::{BoxFuture, DataSourceRunErrorWrapper};
use crate::settings::AISettings;
use crate::terminal;
use crate::terminal::model::session::SessionId;
use crate::terminal::HistoryEntry;

use super::HistorySearchItem;

pub(crate) struct HistorySnapshot {
    commands: Arc<[Arc<HistoryEntry>]>,
    query_text: String,
}

/// Creates an async data source for shell history commands.
#[cfg(test)]
pub fn history_data_source(
    commands: Vec<HistoryEntry>,
) -> AsyncSnapshotDataSource<HistorySnapshot, CommandSearchItemAction> {
    let commands: Arc<[Arc<HistoryEntry>]> = commands.into_iter().map(Arc::new).collect();
    history_data_source_from_shared(commands)
}

fn history_data_source_from_shared(
    commands: Arc<[Arc<HistoryEntry>]>,
) -> AsyncSnapshotDataSource<HistorySnapshot, CommandSearchItemAction> {
    AsyncSnapshotDataSource::new(
        move |query: &Query, _app: &AppContext| HistorySnapshot {
            // Historical commands are all stored as Arcs (with COW semantics and very infrequent writes),
            // so cloning the commands to pass them in to the async sort function is a negligible cost.
            commands: commands.clone(),
            query_text: query.text.clone(),
        },
        fuzzy_match_history,
    )
}

pub(crate) fn history_data_source_for_session(
    session_id: SessionId,
    history_model: &terminal::History,
    app: &AppContext,
) -> AsyncSnapshotDataSource<HistorySnapshot, CommandSearchItemAction> {
    let include_agent_commands = *AISettings::as_ref(app).include_agent_commands_in_history;
    let commands: Arc<[Arc<HistoryEntry>]> = history_model
        .commands_shared(session_id)
        .unwrap_or_default()
        .into_iter()
        .filter(|entry| include_agent_commands || !entry.is_agent_executed)
        .collect();
    history_data_source_from_shared(commands)
}

pub(crate) fn fuzzy_match_history(
    snapshot: HistorySnapshot,
) -> BoxFuture<'static, Result<Vec<QueryResult<CommandSearchItemAction>>, DataSourceRunErrorWrapper>>
{
    Box::pin(async move {
        let mut results = Vec::new();

        // Reuse a single matcher across all entries so that the internal scoring
        // buffers are allocated once and recycled, instead of creating a new
        // SkimMatcherV2 (with fresh heap allocations) per entry.  This fixes a
        // memory spike where fuzzy-matching tens of thousands of history entries
        // could consume multiple GiB of transient heap (APP-4542).
        let matcher = fuzzy_match::CachedFuzzyMatcher::case_insensitive();

        // History entries are cheap to match (single short string), so we use a large chunk
        // size to reduce yield overhead while still allowing cancellation of stale queries.
        for chunk in snapshot.commands.chunks(512) {
            for entry in chunk {
                if let Some(match_result) = fuzzy_match::match_indices_case_insensitive_with(
                    entry.command.as_str(),
                    snapshot.query_text.as_str(),
                    &matcher,
                ) {
                    results.push(
                        HistorySearchItem {
                            entry: entry.clone(),
                            match_result,
                        }
                        .into(),
                    );
                }
            }
            yield_now().await;
        }

        Ok(results)
    })
}
