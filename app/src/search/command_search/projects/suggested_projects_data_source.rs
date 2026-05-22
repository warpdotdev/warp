use std::collections::{HashMap, HashSet};
use std::path::Path;

use chrono::{Duration, NaiveDateTime, Utc};
use fuzzy_match::match_indices_case_insensitive;
use itertools::Itertools;
use warpui::{AppContext, Entity, ModelContext, SingletonEntity};

use crate::search::command_palette::mixer::CommandPaletteItemAction;
use crate::search::command_search::projects::{os_probably_case_sensitive, ProjectSearchItem};
use crate::search::data_source::{Query, QueryResult};
use crate::search::mixer::{DataSourceRunErrorWrapper, SyncDataSource};
use crate::terminal::model::session::get_local_hostname;
use crate::terminal::History;

const SUGGESTION_TIME_WINDOW: Duration = Duration::days(30);
const SUGGESTION_MAX_COUNT: usize = 20;

/// Data source for suggested projects (derived from recent shell history).
pub struct SuggestedProjectsDataSource {
    suggestions: Vec<ProjectSearchItem>,
    excluded_paths: HashSet<String>,
}

impl SuggestedProjectsDataSource {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        let mut excluded_paths: HashSet<String> = HashSet::new();

        excluded_paths.insert("/".to_string());

        if let Some(home) = dirs::home_dir().and_then(|dir| dir.into_os_string().into_string().ok())
        {
            excluded_paths.insert(home);
        }

        let mut data_source = Self {
            suggestions: Vec::new(),
            excluded_paths,
        };

        data_source.regenerate_suggestions(SUGGESTION_MAX_COUNT, ctx);
        data_source
    }

    fn regenerate_suggestions(&mut self, limit: usize, app: &AppContext) {
        // Look at folders used in the last 30 days
        let since = Utc::now().naive_utc() - SUGGESTION_TIME_WINDOW;

        let candidates = self.candidates_from_history(since, limit, app);

        let filter = |candidate: &ProjectSearchItem| {
            if !Path::new(&candidate.path).is_dir() {
                return false;
            }

            if os_probably_case_sensitive() {
                !self.excluded_paths.contains(&candidate.path)
            } else {
                let result = !(self
                    .excluded_paths
                    .iter()
                    .any(|path| candidate.path.eq_ignore_ascii_case(path)));
                result
            }
        };

        self.suggestions = candidates
            .into_iter()
            .filter(filter)
            .k_largest(limit)
            .collect();
    }

    /// Gathers recently used folders from various sources, filtered by a "since" timestamp.
    /// Returns a vector of candidate folders with usage statistics.
    fn candidates_from_history(
        &self,
        since: NaiveDateTime,
        limit: usize,
        app: &AppContext,
    ) -> Vec<ProjectSearchItem> {
        let mut candidates: HashMap<String, ProjectSearchItem> = HashMap::new();

        let history_model = History::as_ref(app);
        let hostname = get_local_hostname().unwrap_or_default();

        for (count, summary) in history_model.command_summaries(hostname) {
            if summary
                .start_ts
                .map(|ts| ts.naive_local() >= since)
                .unwrap_or(false)
            {
                if let Some(pwd_str) = &summary.pwd {
                    let candidate = candidates.entry(pwd_str.clone()).or_insert_with(|| {
                        ProjectSearchItem::new(
                            pwd_str.clone(),
                            fuzzy_match::FuzzyMatchResult::no_match(),
                            Default::default(),
                        )
                    });
                    candidate.popularity_score += count as i32;
                }
            }
        }

        candidates
            .into_values()
            .filter(|candidate| !self.excluded_paths.contains(&candidate.path))
            .k_largest(limit)
            .collect()
    }

    /// Returns up to `limit` suggested projects for the zero-state.
    pub fn top_n(&self, limit: usize) -> Vec<QueryResult<CommandPaletteItemAction>> {
        self.suggestions
            .iter()
            .k_largest(limit)
            .map(|search_item| QueryResult::from(search_item.clone()))
            .collect()
    }
}

impl Entity for SuggestedProjectsDataSource {
    type Event = ();
}

impl SyncDataSource for SuggestedProjectsDataSource {
    type Action = CommandPaletteItemAction;

    fn run_query(
        &self,
        query: &Query,
        _app: &AppContext,
    ) -> Result<Vec<QueryResult<Self::Action>>, DataSourceRunErrorWrapper> {
        if query.text.is_empty() {
            Ok(self.top_n(SUGGESTION_MAX_COUNT))
        } else {
            let results = self
                .suggestions
                .iter()
                .filter_map(|search_item| {
                    match_indices_case_insensitive(&search_item.name, &query.text).map(|mut m| {
                        let mut item = search_item.clone();
                        // This is a hack to make sure these results have higher priority than other
                        // searchable items in the welcome palette. The tantivy search
                        // implementation tends to score things higher than fuzzy_match.
                        m.score *= 4;
                        item.match_result = m;
                        QueryResult::from(item)
                    })
                })
                .collect_vec();

            Ok(results)
        }
    }
}
