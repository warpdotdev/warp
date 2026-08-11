use warpui::{AppContext, Entity};

use crate::search::command_palette::agent_sessions::candidate::AgentSessionCandidate;
use crate::search::command_palette::agent_sessions::search::match_agent_session;
use crate::search::command_palette::agent_sessions::search_item::AgentSessionSearchItem;
use crate::search::command_palette::agent_sessions::tiers::{
    NAME_SEPARATOR_TIER, NAME_SEPARATOR_TITLE,
};
use crate::search::command_palette::mixer::CommandPaletteItemAction;
use crate::search::command_palette::separator_search_item::SeparatorSearchItem;
use crate::search::data_source::{Query, QueryResult};
use crate::search::mixer::{DataSourceRunErrorWrapper, SyncDataSource};

/// Data source over the CLI-agent sessions the session-search popup can offer.
///
/// Holds a snapshot taken when the popup opened rather than a workspace handle,
/// for the same reason the Ctrl+Tab tabs source does: the query runs
/// synchronously while the workspace view is borrowed, so nothing here may
/// reach back into it. Assembling on open (not per keystroke) is also what
/// keeps typing free of the cost of walking tabs, handles and the scan.
#[derive(Default)]
pub struct DataSource {
    /// Newest-first, as [`candidate::merge`](super::candidate::merge) returns
    /// them. Position in this list *is* the recency ranking.
    candidates: Vec<AgentSessionCandidate>,
}

impl DataSource {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_candidates(&mut self, candidates: Vec<AgentSessionCandidate>) {
        self.candidates = candidates;
    }

    pub fn candidates(&self) -> &[AgentSessionCandidate] {
        &self.candidates
    }

    /// The tiebreaker added to a row's fuzzy score, from its position in the
    /// newest-first candidate list. Strictly inside `(0, 1)`, so it orders
    /// equally-scored rows without ever outranking a better match.
    fn recency_bonus(index: usize, count: usize) -> f64 {
        (count - index) as f64 / (count + 1) as f64
    }
}

impl SyncDataSource for DataSource {
    type Action = CommandPaletteItemAction;

    fn run_query(
        &self,
        query: &Query,
        _app: &AppContext,
    ) -> Result<Vec<QueryResult<Self::Action>>, DataSourceRunErrorWrapper> {
        let search_term = query.text.trim();
        let count = self.candidates.len();

        let mut results: Vec<QueryResult<Self::Action>> = self
            .candidates
            .iter()
            .enumerate()
            .filter_map(|(index, candidate)| {
                let matched = match_agent_session(candidate, search_term)?;
                Some(QueryResult::from(AgentSessionSearchItem::new(
                    matched,
                    Self::recency_bonus(index, count),
                )))
            })
            .collect();

        // The header exists only to head rows. Emitting it for a query that
        // matched nothing would both caption an empty section and suppress the
        // palette's "No results found" placeholder, which only renders when
        // there are no results at all.
        if !results.is_empty() {
            results.push(
                SeparatorSearchItem::new(NAME_SEPARATOR_TITLE.to_owned())
                    .with_priority_tier(NAME_SEPARATOR_TIER)
                    .into(),
            );
        }

        Ok(results)
    }
}

impl Entity for DataSource {
    type Event = ();
}

#[cfg(test)]
#[path = "data_source_tests.rs"]
mod tests;
