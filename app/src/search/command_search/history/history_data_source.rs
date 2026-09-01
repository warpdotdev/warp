use std::sync::Arc;

use chrono::Local;
use futures_lite::future::yield_now;
use ordered_float::OrderedFloat;
use warp_core::features::FeatureFlag;
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

pub(crate) struct HistorySnapshot {
    commands: Arc<[Arc<HistoryEntry>]>,
    query_text: String,
    current_session_id: SessionId,
}

/// Creates an async data source for shell history commands.
#[cfg(test)]
pub fn history_data_source(
    commands: Vec<HistoryEntry>,
) -> AsyncSnapshotDataSource<HistorySnapshot, CommandSearchItemAction> {
    let commands: Arc<[Arc<HistoryEntry>]> = commands.into_iter().map(Arc::new).collect();
    history_data_source_from_shared(commands, SessionId::from(0))
}

fn history_data_source_from_shared(
    commands: Arc<[Arc<HistoryEntry>]>,
    current_session_id: SessionId,
) -> AsyncSnapshotDataSource<HistorySnapshot, CommandSearchItemAction> {
    AsyncSnapshotDataSource::new(
        move |query: &Query, _app: &AppContext| HistorySnapshot {
            // Historical commands are all stored as Arcs, so cloning the commands to pass them in
            // to the async sort function is a cheap refcount bump, not a deep copy. Because the
            // entries themselves are copy-on-write (`Arc::make_mut` in `mark_command_as_finished`),
            // a snapshot taken before an in-flight command completes keeps pointing at the
            // pre-completion entry rather than seeing the update -- that staleness is expected and
            // resolved by the next query.
            commands: commands.clone(),
            query_text: query.text.clone(),
            current_session_id,
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
    history_data_source_from_shared(commands, session_id)
}

pub(crate) fn fuzzy_match_history(
    snapshot: HistorySnapshot,
) -> BoxFuture<'static, Result<Vec<QueryResult<CommandSearchItemAction>>, DataSourceRunErrorWrapper>>
{
    if !FeatureFlag::HistorySearchRankingV2.is_enabled() {
        return fuzzy_match_history_legacy(snapshot);
    }

    Box::pin(async move {
        let mut results = Vec::new();
        let now = Local::now();
        let is_blank_query = snapshot.query_text.trim().is_empty();
        let tokens = rank::tokenize_query(&snapshot.query_text);
        let total_candidates = snapshot.commands.len();

        // History entries are cheap to match (single short string), so we use a large chunk
        // size to reduce yield overhead while still allowing cancellation of stale queries.
        const CHUNK_SIZE: usize = 512;
        for (chunk_index, chunk) in snapshot.commands.chunks(CHUNK_SIZE).enumerate() {
            let chunk_start = chunk_index * CHUNK_SIZE;
            for (offset, entry) in chunk.iter().enumerate() {
                let Some((match_result, match_quality)) =
                    rank::match_history_command(entry.command.as_str(), &tokens)
                else {
                    continue;
                };

                let index = chunk_start + offset;
                let Some(score) = rank::rank(RankInputs {
                    entry: entry.as_ref(),
                    match_quality,
                    now,
                    current_session_id: snapshot.current_session_id,
                    newer_candidate_count: total_candidates - 1 - index,
                    is_blank_query,
                }) else {
                    continue;
                };

                results.push(
                    HistorySearchItem {
                        entry: entry.clone(),
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

/// The pre-[`FeatureFlag::HistorySearchRankingV2`] matching behavior: the whole query as a
/// single fuzzy pattern against each command (no whitespace tokenization), scored directly by
/// Skim's raw match score with no history priors and no floor. This is the exact code path
/// history search used before APP-5650, not an approximation of it, so disabling the flag is a
/// genuine escape hatch back to the previous behavior.
fn fuzzy_match_history_legacy(
    snapshot: HistorySnapshot,
) -> BoxFuture<'static, Result<Vec<QueryResult<CommandSearchItemAction>>, DataSourceRunErrorWrapper>>
{
    Box::pin(async move {
        let mut results = Vec::new();

        // History entries are cheap to match (single short string), so we use a large chunk
        // size to reduce yield overhead while still allowing cancellation of stale queries.
        const CHUNK_SIZE: usize = 512;
        for chunk in snapshot.commands.chunks(CHUNK_SIZE) {
            for entry in chunk {
                let Some(match_result) = fuzzy_match::match_indices_case_insensitive(
                    entry.command.as_str(),
                    snapshot.query_text.as_str(),
                ) else {
                    continue;
                };
                let score = OrderedFloat(match_result.score as f64);

                results.push(
                    HistorySearchItem {
                        entry: entry.clone(),
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
