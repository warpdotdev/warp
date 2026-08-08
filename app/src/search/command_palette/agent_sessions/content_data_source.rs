use warpui::{AppContext, Entity};

use crate::search::command_palette::agent_sessions::content_search_item::ContentSearchItem;
use crate::search::command_palette::agent_sessions::tiers::{
    CONTENT_SEPARATOR_TIER, CONTENT_SEPARATOR_TITLE,
};
use crate::search::command_palette::mixer::CommandPaletteItemAction;
use crate::search::command_palette::separator_search_item::SeparatorSearchItem;
use crate::search::data_source::{Query, QueryResult};
use crate::search::mixer::{DataSourceRunErrorWrapper, SyncDataSource};
use crate::terminal::cli_agent_sessions::transcript_digest::ContentHit;

/// Most content rows shown at once, per `rail-search/plan.md` §7.
///
/// The digest already caps its own hits at the same number; this cap is the one
/// the *screen* needs, and it is stated here so a change to either is a
/// deliberate change to both.
const MAX_CONTENT_ROWS: usize = 50;

/// Data source over the transcript-content hits the digest has **already**
/// published.
///
/// Does no I/O and no matching — it serves a snapshot, nothing else. That is
/// the whole point: the substring pass over the corpus costs tens of
/// milliseconds, and running it inside `run_query` would spend that on every
/// keystroke, on the thread that is drawing the palette.
///
/// It is also why this source is **synchronous**. Registering it as an async
/// source would make the mixer withhold the instant, in-memory *name* results
/// for up to its initial-results timeout while the palette still showed the
/// previous query's rows, and would make Enter a no-op for as long as it was
/// loading. Content search therefore lives in its own model, off to the side,
/// and this source is the letterbox it publishes through.
#[derive(Default)]
pub struct ContentDataSource {
    /// The query [`Self::hits`] answer. Compared against the palette's current
    /// query text on every run, so results for a query the user has already
    /// typed past are shown for exactly zero frames.
    query: String,
    /// Newest-first, as the digest's corpus order gives them.
    hits: Vec<ContentHit>,
}

impl ContentDataSource {
    pub fn new() -> Self {
        Self::default()
    }

    /// Publishes the hits the digest found for `query`.
    pub fn set_results(&mut self, query: String, hits: Vec<ContentHit>) {
        self.query = query;
        self.hits = hits;
    }

    /// Drops everything published, so a newly opened popup cannot flash the
    /// previous one's results.
    pub fn clear(&mut self) {
        self.query.clear();
        self.hits.clear();
    }

    pub fn hits(&self) -> &[ContentHit] {
        &self.hits
    }

    /// The tiebreaker added to a row's score, from its position in the
    /// newest-first hit list. Strictly inside `(0, 1]`, matching the name
    /// source's convention.
    fn recency_bonus(index: usize, count: usize) -> f64 {
        (count - index) as f64 / (count + 1) as f64
    }
}

impl SyncDataSource for ContentDataSource {
    type Action = CommandPaletteItemAction;

    fn run_query(
        &self,
        query: &Query,
        _app: &AppContext,
    ) -> Result<Vec<QueryResult<Self::Action>>, DataSourceRunErrorWrapper> {
        let search_term = query.text.trim();
        // Hits belong to the query they were found for. While the user is
        // typing ahead of the digest, that is not this query, and showing them
        // anyway would put rows on screen that do not contain what is in the
        // search box.
        if search_term.is_empty() || search_term != self.query {
            return Ok(Vec::new());
        }

        let count = self.hits.len().min(MAX_CONTENT_ROWS);
        let mut results: Vec<QueryResult<Self::Action>> = self
            .hits
            .iter()
            .take(MAX_CONTENT_ROWS)
            .enumerate()
            .map(|(index, hit)| {
                QueryResult::from(ContentSearchItem::new(
                    hit.clone(),
                    Self::recency_bonus(index, count),
                ))
            })
            .collect();

        // Same rule as the name section: a header with no rows under it would
        // caption an empty section and, worse, suppress the palette's "No
        // results found" placeholder, which only renders when there are no
        // results at all.
        if !results.is_empty() {
            results.push(
                SeparatorSearchItem::new(CONTENT_SEPARATOR_TITLE.to_owned())
                    .with_priority_tier(CONTENT_SEPARATOR_TIER)
                    .into(),
            );
        }

        Ok(results)
    }
}

impl Entity for ContentDataSource {
    type Event = ();
}

#[cfg(test)]
#[path = "content_data_source_tests.rs"]
mod tests;
