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
use crate::terminal::model::session::SessionId;
use crate::terminal::{CommandExecutionStats, HistoryEntry};

const CHUNK_SIZE: usize = 512;

struct HistoryCandidate {
    entry: Arc<HistoryEntry>,
    stats: CommandExecutionStats,
}

pub(crate) struct HistorySnapshot {
    candidates: Arc<[HistoryCandidate]>,
    query_text: String,
    current_session_id: SessionId,
}

/// Creates an async data source for shell history commands.
#[cfg(test)]
pub fn history_data_source(
    commands: Vec<HistoryEntry>,
) -> AsyncSnapshotDataSource<HistorySnapshot, CommandSearchItemAction> {
    let candidates: Arc<[HistoryCandidate]> = commands
        .into_iter()
        .map(|entry| HistoryCandidate {
            entry: Arc::new(entry),
            stats: CommandExecutionStats::default(),
        })
        .collect();
    AsyncSnapshotDataSource::new(
        move |query: &Query, _app: &AppContext| HistorySnapshot {
            candidates: candidates.clone(),
            query_text: query.text.clone(),
            current_session_id: SessionId::from(0),
        },
        fuzzy_match_history,
    )
}

pub(crate) fn history_data_source_for_session(
    session_id: SessionId,
    current_cwd: Option<String>,
) -> AsyncSnapshotDataSource<HistorySnapshot, CommandSearchItemAction> {
    AsyncSnapshotDataSource::new(
        move |query: &Query, app: &AppContext| {
            let include_agent_commands = *AISettings::as_ref(app).include_agent_commands_in_history;
            let history_model = terminal::History::as_ref(app);
            let candidates: Arc<[HistoryCandidate]> = history_model
                .commands_shared(session_id)
                .unwrap_or_default()
                .into_iter()
                .filter(|entry| include_agent_commands || !entry.is_agent_executed)
                .map(|entry| {
                    let stats = history_model.command_execution_stats(
                        session_id,
                        &entry.command,
                        current_cwd.as_deref(),
                    );
                    HistoryCandidate { entry, stats }
                })
                .collect();
            HistorySnapshot {
                candidates,
                query_text: query.text.clone(),
                current_session_id: session_id,
            }
        },
        fuzzy_match_history,
    )
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

        for chunk in snapshot.candidates.chunks(CHUNK_SIZE) {
            for candidate in chunk {
                let Some((match_result, match_quality)) =
                    rank::match_history_command(candidate.entry.command.as_str(), &tokens)
                else {
                    continue;
                };

                let Some(score) = rank::rank(RankInputs {
                    entry: candidate.entry.as_ref(),
                    match_quality,
                    now,
                    current_session_id: snapshot.current_session_id,
                    total_execution_count: candidate.stats.total_count,
                    cwd_execution_count: candidate.stats.cwd_count,
                    pwd_known_execution_count: candidate.stats.pwd_known_count,
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

fn fuzzy_match_history_legacy(
    snapshot: HistorySnapshot,
) -> BoxFuture<'static, Result<Vec<QueryResult<CommandSearchItemAction>>, DataSourceRunErrorWrapper>>
{
    Box::pin(async move {
        let mut results = Vec::new();

        for chunk in snapshot.candidates.chunks(CHUNK_SIZE) {
            for candidate in chunk {
                let Some(match_result) = fuzzy_match::match_indices_case_insensitive(
                    candidate.entry.command.as_str(),
                    snapshot.query_text.as_str(),
                ) else {
                    continue;
                };
                let score = OrderedFloat(match_result.score as f64);

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
