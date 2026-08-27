use std::sync::Arc;

use chrono::Local;
use futures_lite::future::yield_now;
use warpui::{AppContext, SingletonEntity};

use super::HistorySearchItem;
use super::rank::{self, RankInputs};
use crate::search::async_snapshot_data_source::AsyncSnapshotDataSource;
use crate::search::command_search::searcher::CommandSearchItemAction;
use crate::search::data_source::{Query, QueryResult};
use crate::search::mixer::{BoxFuture, DataSourceRunErrorWrapper};
use crate::settings::AISettings;
use crate::terminal;
use crate::terminal::HistoryEntry;
use crate::terminal::model::session::SessionId;

/// A history entry paired with how many times it's been executed, per
/// `History::command_execution_count`. Computed once when the snapshot is built (while a
/// `History` reference is available) rather than per-query, since frequency changes are
/// infrequent relative to keystrokes.
struct HistoryCandidate {
    entry: Arc<HistoryEntry>,
    frequency: u32,
}

pub(crate) struct HistorySnapshot {
    candidates: Arc<[HistoryCandidate]>,
    query_text: String,
    current_session_id: SessionId,
    /// The session's live working directory at the moment history search was opened. This is
    /// *not* derivable from the history entries themselves: `HistoryEntry::pwd` is captured when
    /// a command *starts* (`terminal/history.rs`'s `for_session_command`), so the most recent
    /// entry's `pwd` is stale immediately after a `cd` until the next command runs.
    current_cwd: Option<String>,
}

/// Creates an async data source for shell history commands.
#[cfg(test)]
pub fn history_data_source(
    commands: Vec<HistoryEntry>,
) -> AsyncSnapshotDataSource<HistorySnapshot, CommandSearchItemAction> {
    history_data_source_with_cwd(commands, None)
}

#[cfg(test)]
pub fn history_data_source_with_cwd(
    commands: Vec<HistoryEntry>,
    current_cwd: Option<String>,
) -> AsyncSnapshotDataSource<HistorySnapshot, CommandSearchItemAction> {
    let candidates: Arc<[HistoryCandidate]> = commands
        .into_iter()
        .map(|entry| HistoryCandidate {
            entry: Arc::new(entry),
            frequency: 1,
        })
        .collect();
    history_data_source_from_shared(candidates, SessionId::from(0), current_cwd)
}

fn history_data_source_from_shared(
    candidates: Arc<[HistoryCandidate]>,
    current_session_id: SessionId,
    current_cwd: Option<String>,
) -> AsyncSnapshotDataSource<HistorySnapshot, CommandSearchItemAction> {
    AsyncSnapshotDataSource::new(
        move |query: &Query, _app: &AppContext| HistorySnapshot {
            // Historical commands are all stored as Arcs (with COW semantics and very infrequent writes),
            // so cloning the commands to pass them in to the async sort function is a negligible cost.
            candidates: candidates.clone(),
            query_text: query.text.clone(),
            current_session_id,
            current_cwd: current_cwd.clone(),
        },
        fuzzy_match_history,
    )
}

pub(crate) fn history_data_source_for_session(
    session_id: SessionId,
    current_cwd: Option<String>,
    history_model: &terminal::History,
    app: &AppContext,
) -> AsyncSnapshotDataSource<HistorySnapshot, CommandSearchItemAction> {
    let include_agent_commands = *AISettings::as_ref(app).include_agent_commands_in_history;
    let candidates: Arc<[HistoryCandidate]> = history_model
        .commands_shared(session_id)
        .unwrap_or_default()
        .into_iter()
        .filter(|entry| include_agent_commands || !entry.is_agent_executed)
        .map(|entry| {
            let frequency = history_model.command_execution_count(session_id, &entry.command);
            HistoryCandidate { entry, frequency }
        })
        .collect();
    history_data_source_from_shared(candidates, session_id, current_cwd)
}

pub(crate) fn fuzzy_match_history(
    snapshot: HistorySnapshot,
) -> BoxFuture<'static, Result<Vec<QueryResult<CommandSearchItemAction>>, DataSourceRunErrorWrapper>>
{
    Box::pin(async move {
        let mut results = Vec::new();
        let now = Local::now();
        let is_blank_query = snapshot.query_text.trim().is_empty();
        let tokens = rank::tokenize_query(&snapshot.query_text);
        let total_candidates = snapshot.candidates.len();
        let current_cwd = snapshot.current_cwd.as_deref();

        // History entries are cheap to match (single short string), so we use a large chunk
        // size to reduce yield overhead while still allowing cancellation of stale queries.
        const CHUNK_SIZE: usize = 512;
        for (chunk_index, chunk) in snapshot.candidates.chunks(CHUNK_SIZE).enumerate() {
            let chunk_start = chunk_index * CHUNK_SIZE;
            for (offset, candidate) in chunk.iter().enumerate() {
                let Some((match_result, match_quality)) =
                    rank::match_history_command(candidate.entry.command.as_str(), &tokens)
                else {
                    continue;
                };

                let index = chunk_start + offset;
                let Some(score) = rank::rank(RankInputs {
                    entry: candidate.entry.as_ref(),
                    frequency: candidate.frequency,
                    match_quality,
                    now,
                    current_session_id: snapshot.current_session_id,
                    current_cwd,
                    newer_candidate_count: total_candidates - 1 - index,
                    is_blank_query,
                }) else {
                    continue;
                };

                results.push(
                    HistorySearchItem {
                        entry: candidate.entry.clone(),
                        match_result,
                        score,
                    }
                    .into(),
                );
            }
            yield_now().await;
        }

        Ok(results)
    })
}
